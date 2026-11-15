//! A local stand-in Lanyard for testing Spotify friends. It serves REST and the socket from a
//! fixture file that is re-read every half second, so a test moves a friend by rewriting it.
//!
//! cargo run -p navia-core --example lanyard_stand_in -- 127.0.0.1:4001 presences.json
//!
//! The file maps Discord ids to presence objects. `"start": "now-30"` means 30 s ago and
//! `"end": "+200"` means 200 s after the start, so fixtures don't go stale.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

type Presences = Arc<Mutex<HashMap<String, Value>>>;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Reads the fixture and resolves relative timestamps. They stay fixed while the song is
/// unchanged, so positions don't jump.
fn load(path: &str, old: &HashMap<String, Value>) -> HashMap<String, Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let map: HashMap<String, Value> = serde_json::from_str(&text).unwrap_or_default();
    map.into_iter()
        .map(|(id, mut p)| {
            let raw = p["spotify"].clone();
            if let Some(stamps) = p.get_mut("spotify").and_then(|s| s.get_mut("timestamps")) {
                let prior = old.get(&id).filter(|o| o["spotify_raw"] == raw).map(|o| o["spotify"]["timestamps"].clone());
                if let Some(prior) = prior {
                    *stamps = prior;
                } else {
                    let start = match &stamps["start"] {
                        Value::String(s) => now_ms().saturating_sub(s.trim_start_matches("now-").parse::<u64>().unwrap_or(0) * 1000),
                        v => v.as_u64().unwrap_or(0),
                    };
                    let end = match &stamps["end"] {
                        Value::String(s) => start + s.trim_start_matches('+').parse::<u64>().unwrap_or(0) * 1000,
                        v => v.as_u64().unwrap_or(0),
                    };
                    *stamps = json!({ "start": start, "end": end });
                }
            }
            p["spotify_raw"] = raw;
            (id, p)
        })
        .collect()
}

fn public(p: &Value) -> Value {
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("spotify_raw");
    }
    p
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:4001".into());
    let path = args.next().expect("a presences.json fixture");
    let presences: Presences = Arc::new(Mutex::new(load(&path, &HashMap::new())));
    let (tx, _) = broadcast::channel::<(String, Value)>(64);
    {
        let (presences, tx, path) = (presences.clone(), tx.clone(), path.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let old = presences.lock().unwrap().clone();
                let new = load(&path, &old);
                for (id, p) in &new {
                    if old.get(id) != Some(p) {
                        let _ = tx.send((id.clone(), public(p)));
                    }
                }
                *presences.lock().unwrap() = new;
            }
        });
    }
    let listener = TcpListener::bind(&addr).await.expect("bind");
    eprintln!("lanyard stand-in on http://{addr}");
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let (presences, rx) = (presences.clone(), tx.subscribe());
        tokio::spawn(async move { serve(stream, presences, rx).await });
    }
}

async fn serve(mut stream: TcpStream, presences: Presences, mut rx: broadcast::Receiver<(String, Value)>) {
    let mut head = [0u8; 2048];
    let Ok(n) = stream.peek(&mut head).await else { return };
    let request = String::from_utf8_lossy(&head[..n]).to_lowercase();
    if !request.contains("upgrade: websocket") {
        // REST, GET /v1/users/<id>.
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let id = request.split_whitespace().nth(1).and_then(|p| p.strip_prefix("/v1/users/")).unwrap_or_default().to_string();
        let found = presences.lock().unwrap().get(&id).map(public);
        let (status, body) = match found {
            Some(p) => ("200 OK", json!({ "success": true, "data": p })),
            None => ("404 Not Found", json!({ "success": false, "error": { "code": "user_not_monitored", "message": "User is not being monitored by Lanyard" } })),
        };
        let body = body.to_string();
        let _ = stream.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
        eprintln!("REST {id}: {status}");
        return;
    }
    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
    let _ = ws.send(Message::text(json!({ "op": 1, "d": { "heartbeat_interval": 30000 } }).to_string())).await;
    let mut ids: Vec<String> = Vec::new();
    let mut seq = 0u64;
    loop {
        tokio::select! {
            msg = ws.next() => {
                let Some(Ok(msg)) = msg else { return };
                let Ok(v) = serde_json::from_str::<Value>(msg.to_text().unwrap_or_default()) else { continue };
                if v["op"] == 2 {
                    ids = v["d"]["subscribe_to_ids"].as_array().map(|a| a.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    let init: serde_json::Map<String, Value> = {
                        let map = presences.lock().unwrap();
                        ids.iter().filter_map(|id| map.get(id).map(|p| (id.clone(), public(p)))).collect()
                    };
                    seq += 1;
                    eprintln!("socket subscribed to {ids:?}");
                    let _ = ws.send(Message::text(json!({ "op": 0, "seq": seq, "t": "INIT_STATE", "d": init }).to_string())).await;
                }
            }
            update = rx.recv() => {
                let Ok((id, p)) = update else { continue };
                if ids.contains(&id) {
                    seq += 1;
                    eprintln!("socket update {id}");
                    let _ = ws.send(Message::text(json!({ "op": 0, "seq": seq, "t": "PRESENCE_UPDATE", "d": p }).to_string())).await;
                }
            }
        }
    }
}
