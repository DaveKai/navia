//! Plays through the real player to a real AirPlay receiver. Read-only on the server, and it
//! never sets the receiver's volume. Pairing data lives in `AIRPLAY_DIR` (default `./airplay-config`).
//!
//! `cargo run -p navia-core --example airplay_check -- discover [secs]`
//! `... -- pair <deviceid> <pin-file>` (write the PIN the receiver shows to the file)
//! `NAVIDROME_URL=... NAVIDROME_USER=... NAVIDROME_PASSWORD=... cargo run -p navia-core --example
//! airplay_check -- play <deviceid> <song id>[,<song id>...] [step...]`
//!
//! Each step is `<seconds>:<action>`, timed from the previous one. Actions are `seek=<ms>`, `pause`,
//! `play`, `next`, `previous`, `crossfade=<ms>` and `stop`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use navia_core::audio::airplay::{AirPlay, AirPlayConfig, ClientNowPlaying, DEFAULT_PORTS, ID_PREFIX};
use navia_core::audio::player::{Player, PlayerConfig, PlayerStatus, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials, WritePolicy};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::env::var("AIRPLAY_DIR").unwrap_or_else(|_| "airplay-config".into());
    let client = client().ok();
    let now_playing = client.clone().map(|c| Arc::new(ClientNowPlaying::new(c)) as _);
    let remote: Arc<std::sync::Mutex<Option<Arc<Player>>>> = Arc::new(std::sync::Mutex::new(None));
    let r = remote.clone();
    let handler: navia_core::audio::airplay::RemoteHandler = Arc::new(move |cmd| {
        println!("remote: {cmd:?}");
        if let Some(p) = r.lock().unwrap().as_ref() {
            use navia_core::audio::airplay::RemoteCommand as C;
            match cmd {
                C::PlayPause => p.toggle(),
                C::Play => p.play(),
                C::Pause => p.pause(),
                C::Stop => p.stop(),
                C::Next => p.next(),
                C::Previous => p.previous(),
                // The test never sets a volume, so these are only printed.
                C::VolumeUp | C::VolumeDown => {}
            }
        }
    });
    let airplay = AirPlay::new(AirPlayConfig { config_dir: dir.into(), secrets: None, ports: DEFAULT_PORTS, now_playing, remote: Some(handler) })?;
    airplay.start_discovery(|| {})?;
    match args.first().map(String::as_str) {
        Some("discover") => {
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            for o in airplay.outputs() {
                println!("{} | {} | {} | {:?} | paired {} | needs PIN {}", o.id, o.name, o.model, o.support, o.paired, o.needs_pin);
            }
        }
        Some("pair") => {
            let (id, pin_file) = (args.get(1).ok_or("pair <deviceid> <pin-file>")?, args.get(2).ok_or("pair <deviceid> <pin-file>")?);
            let _ = std::fs::remove_file(pin_file);
            let ap = airplay.clone();
            let out = format!("{ID_PREFIX}{id}");
            let o2 = out.clone();
            tokio::task::spawn_blocking(move || ap.pair_begin(&o2)).await??;
            println!("the receiver shows a PIN; waiting for it in {pin_file}");
            let pin = loop {
                if let Ok(s) = std::fs::read_to_string(pin_file)
                    && s.trim().len() == 4
                {
                    break s.trim().to_string();
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            };
            let ap = airplay.clone();
            tokio::task::spawn_blocking(move || ap.pair_finish(&out, &pin)).await??;
            println!("paired");
        }
        Some("play") => {
            let id = args.get(1).ok_or("play <deviceid> <songs> [steps]")?;
            let songs = args.get(2).ok_or("play <deviceid> <songs> [steps]")?;
            let client = client.ok_or("NAVIDROME_URL, NAVIDROME_USER and NAVIDROME_PASSWORD must be set")?;
            let mut items = Vec::new();
            for s in songs.split(',') {
                let song = client.song(s).await?;
                println!("queue: {} - {} ({} s)", song.artist.clone().unwrap_or_default(), song.title, song.duration.unwrap_or(0));
                items.push(QueueItem::new(s.to_string(), song.duration.map(|d| d as u64 * 1000)));
            }
            let resolver = Arc::new(ServerResolver::new(client.clone())?);
            let player = Arc::new(Player::with_backend(resolver, Box::new(airplay.router()), PlayerConfig::default())?);
            *remote.lock().unwrap() = Some(player.clone());
            player.add_listener(|e| match e {
                navia_core::audio::player::PlayerEvent::Error(m) => println!("player error: {m}"),
                navia_core::audio::player::PlayerEvent::TrackChanged { index, .. } => println!("heard: track {index}"),
                _ => {}
            });
            player.set_output_device(Some(format!("{ID_PREFIX}{id}")));
            player.set_queue(items, 0, 0, false);
            player.play();
            let mut steps: Vec<(u64, String)> = args[3..].iter().filter_map(|s| s.split_once(':').map(|(t, a)| (t.parse().unwrap_or(0), a.to_string()))).collect();
            steps.reverse();
            let started = Instant::now();
            let mut next_step = Instant::now() + Duration::from_secs(steps.last().map(|s| s.0).unwrap_or(0));
            let mut end = if steps.is_empty() { None } else { Some(Instant::now() + Duration::from_secs(36_000)) };
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let s = player.snapshot();
                println!("t={:>4.0}s {:?} track {:?} at {:>6.1}s rate {} {}", started.elapsed().as_secs_f64(), s.status, s.index, s.position_ms as f64 / 1000.0, s.output_rate, airplay.outputs().iter().find(|o| o.id.ends_with(id.as_str())).and_then(|o| o.state.clone()).map(|st| format!("{st:?}")).unwrap_or_default());
                if let Some(e) = s.error.as_ref() {
                    println!("  error: {e}");
                }
                if !steps.is_empty() && Instant::now() >= next_step {
                    let (_, action) = steps.pop().expect("step");
                    println!("step: {action}");
                    match action.split_once('=') {
                        Some(("seek", ms)) => player.seek_ms(ms.parse()?),
                        Some(("crossfade", ms)) => player.set_crossfade_ms(ms.parse()?),
                        _ => match action.as_str() {
                            "pause" => player.pause(),
                            "play" => player.play(),
                            "next" => player.next(),
                            "previous" => player.previous(),
                            "stop" => player.stop(),
                            other => println!("unknown step {other}"),
                        },
                    }
                    match steps.last() {
                        Some((secs, _)) => next_step = Instant::now() + Duration::from_secs(*secs),
                        None => end = Some(Instant::now() + Duration::from_secs(10)),
                    }
                }
                if end.is_some_and(|e| Instant::now() >= e) || (s.status == PlayerStatus::Stopped && started.elapsed() > Duration::from_secs(10)) {
                    break;
                }
            }
            player.shutdown();
        }
        _ => eprintln!("usage: airplay_check discover [secs] | pair <deviceid> <pin-file> | play <deviceid> <songs> [steps]"),
    }
    Ok(())
}

fn client() -> Result<Client, String> {
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set"));
    let mut cfg = ClientConfig::new(env("NAVIDROME_URL")?, Credentials::new(env("NAVIDROME_USER")?, env("NAVIDROME_PASSWORD")?));
    cfg.write_policy = WritePolicy::ReadOnly;
    Client::new(cfg).map_err(|e| e.to_string())
}
