//! Cover art lookups against a small local HTTP server, with no real network.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use navia_integrations::TrackInfo;
use navia_integrations::art::{ArtSource, CoverArtArchive};
use serde_json::json;

const RELEASE: &str = "11111111-1111-1111-1111-111111111111";
const GROUP: &str = "22222222-2222-2222-2222-222222222222";

struct Reply {
    status: u16,
    body: String,
}

fn reply(status: u16) -> Reply {
    Reply {
        status,
        body: String::new(),
    }
}

/// Answers each request in order and records the request lines.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

fn start(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let mut replies = VecDeque::from(replies);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream: TcpStream = stream;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                    break;
                }
            }
            log.lock().unwrap().push(request_line.trim().to_owned());
            let Reply { status, body } = replies.pop_front().unwrap_or_else(|| reply(404));
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Server { base, requests }
}

fn source(server: &Server) -> CoverArtArchive {
    CoverArtArchive::with_bases(&server.base, &server.base).unwrap()
}

fn tagged() -> TrackInfo {
    TrackInfo {
        release_mbid: Some(RELEASE.into()),
        release_group_mbid: Some(GROUP.into()),
        ..Default::default()
    }
}

#[test]
fn uses_the_release_first() {
    let server = start(vec![reply(200)]);
    let url = source(&server).find(&tagged()).unwrap().unwrap();
    assert_eq!(url, format!("{}/release/{RELEASE}/front-500", server.base));
}

#[test]
fn falls_back_to_the_release_group() {
    let server = start(vec![reply(404), reply(200)]);
    let url = source(&server).find(&tagged()).unwrap().unwrap();
    assert_eq!(url, format!("{}/release-group/{GROUP}/front-500", server.base));
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("HEAD /release/"), "{}", requests[0]);
}

#[test]
fn a_brief_server_error_is_retried_once() {
    let server = start(vec![reply(500), reply(200)]);
    assert!(source(&server).find(&tagged()).unwrap().is_some());
    assert_eq!(server.requests.lock().unwrap().len(), 2);
}

#[test]
fn a_rate_limit_is_an_error_and_not_retried() {
    let server = start(vec![reply(503)]);
    assert!(source(&server).find(&tagged()).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn searches_by_name_and_probes_the_match() {
    let body = json!({"release-groups": [{
        "id": GROUP, "score": 100, "title": "Some \"Quoted\" Album", "primary-type": "Album",
        "artist-credit": [{"name": "Some Artist"}],
    }]})
    .to_string();
    let server = start(vec![Reply { status: 200, body }, reply(200)]);
    let track = TrackInfo {
        artist: "Some Artist".into(),
        album: "Some \"Quoted\" Album".into(),
        ..Default::default()
    };
    let url = source(&server).find(&track).unwrap().unwrap();
    assert_eq!(url, format!("{}/release-group/{GROUP}/front-500", server.base));

    let requests = server.requests.lock().unwrap();
    assert!(requests[0].starts_with("GET /ws/2/release-group?"), "{}", requests[0]);
    assert!(requests[0].contains("fmt=json"), "{}", requests[0]);
}

#[test]
fn no_tags_and_no_names_makes_no_requests() {
    let server = start(vec![]);
    assert_eq!(source(&server).find(&TrackInfo::default()).unwrap(), None);
    assert!(server.requests.lock().unwrap().is_empty());
}
