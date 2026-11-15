//! Tests against a fake Discord socket in a private folder. They never look at the standard socket locations, so the
//! real Discord can't be reached.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use navia_integrations::art::{ArtError, ArtSource, NoArt};
use navia_integrations::discord::{DiscordConfig, DiscordPresence, StatusText};
use navia_integrations::{PlaybackState, PlayerSnapshot, TrackInfo};
use serde_json::{Value, json};

const APP_ID: &str = "123456789012345678";
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

#[derive(Default)]
struct Log {
    handshakes: Vec<Value>,
    commands: Vec<Value>,
    pongs: Vec<Value>,
    closes: usize,
}

struct Fake {
    log: Mutex<Log>,
    changed: Condvar,
    connection: Mutex<Option<UnixStream>>,
    reject: AtomicBool,
}

struct FakeDiscord {
    state: Arc<Fake>,
    dir: tempfile::TempDir,
}

fn read_frame(stream: &mut UnixStream) -> Option<(u32, Value)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).ok()?;
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).ok()?;
    Some((opcode, serde_json::from_slice(&body).ok()?))
}

fn write_frame(stream: &mut UnixStream, opcode: u32, body: &Value) {
    let json = body.to_string();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&opcode.to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(json.as_bytes());
    let _ = stream.write_all(&bytes);
}

impl FakeDiscord {
    fn start() -> Self {
        // Socket paths are limited to about 108 bytes, so avoid a long TMPDIR.
        let base = if std::env::temp_dir().as_os_str().len() > 60 {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = tempfile::Builder::new().prefix("nv-ipc").tempdir_in(base).unwrap();
        let listener = UnixListener::bind(dir.path().join("discord-ipc-0")).unwrap();
        let state = Arc::new(Fake {
            log: Mutex::default(),
            changed: Condvar::new(),
            connection: Mutex::new(None),
            reject: AtomicBool::new(false),
        });
        let server = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                *server.connection.lock().unwrap() = stream.try_clone().ok();
                let server = server.clone();
                thread::spawn(move || serve(server, stream));
            }
        });
        Self { state, dir }
    }

    fn config(&self) -> DiscordConfig {
        DiscordConfig {
            socket_dirs: Some(vec![self.dir.path().to_path_buf()]),
            reconnect_min: Duration::from_millis(50),
            reconnect_max: Duration::from_millis(200),
            ..DiscordConfig::new(APP_ID)
        }
    }

    fn wait<T>(&self, what: &str, mut check: impl FnMut(&Log) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut log = self.state.log.lock().unwrap();
        loop {
            if let Some(value) = check(&log) {
                return value;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}");
            log = self.state.changed.wait_timeout(log, left).unwrap().0;
        }
    }

    fn command_count(&self) -> usize {
        self.state.log.lock().unwrap().commands.len()
    }

    /// Waits for command number `index` (from zero) and returns its activity.
    fn command(&self, index: usize) -> Value {
        self.wait(&format!("command {index}"), |log| {
            log.commands.get(index).map(|c| c["args"]["activity"].clone())
        })
    }

    fn drop_connection(&self) {
        if let Some(stream) = self.state.connection.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn serve(state: Arc<Fake>, mut stream: UnixStream) {
    while let Some((opcode, body)) = read_frame(&mut stream) {
        let mut log = state.log.lock().unwrap();
        match opcode {
            OP_HANDSHAKE => {
                log.handshakes.push(body);
                if state.reject.load(Ordering::SeqCst) {
                    write_frame(&mut stream, OP_CLOSE, &json!({"code": 4000, "message": "Invalid Client ID"}));
                } else {
                    write_frame(&mut stream, OP_FRAME, &json!({"evt": "READY", "cmd": "DISPATCH", "data": {"v": 1}}));
                }
            }
            OP_FRAME => {
                let nonce = body["nonce"].clone();
                log.commands.push(body);
                write_frame(&mut stream, OP_FRAME, &json!({"cmd": "SET_ACTIVITY", "data": {}, "evt": null, "nonce": nonce}));
            }
            OP_PONG => log.pongs.push(body),
            OP_CLOSE => log.closes += 1,
            _ => {}
        }
        drop(log);
        state.changed.notify_all();
    }
}

fn track(id: &str) -> TrackInfo {
    TrackInfo {
        id: id.into(),
        title: format!("Song {id}"),
        artist: "Artist".into(),
        album: format!("Album {id}"),
        duration: Some(Duration::from_secs(200)),
        ..Default::default()
    }
}

fn playing(id: &str, position: u64) -> PlayerSnapshot {
    PlayerSnapshot {
        state: PlaybackState::Playing,
        track: Some(track(id)),
        position: Duration::from_secs(position),
        volume: 1.0,
        private_mode: false,
        loop_mode: Default::default(),
        shuffle: false,
        can_change_modes: false,
    }
}

fn with_state(mut snapshot: PlayerSnapshot, state: PlaybackState) -> PlayerSnapshot {
    snapshot.state = state;
    snapshot
}

fn with_private(mut snapshot: PlayerSnapshot) -> PlayerSnapshot {
    snapshot.private_mode = true;
    snapshot
}

fn presence(config: DiscordConfig) -> DiscordPresence {
    DiscordPresence::new(config, Arc::new(NoArt)).unwrap()
}

fn settle() {
    thread::sleep(Duration::from_millis(300));
}

#[test]
fn publishes_song_artist_album_and_elapsed_time() {
    let fake = FakeDiscord::start();
    let config = DiscordConfig {
        default_image: Some("navia_default".into()),
        status_text: StatusText::Artist,
        ..fake.config()
    };
    let client = presence(config);
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    client.set_snapshot(&playing("a", 30));

    let handshake = fake.wait("handshake", |log| log.handshakes.first().cloned());
    assert_eq!(handshake, json!({"v": 1, "client_id": APP_ID}));

    let command = fake.wait("command", |log| log.commands.first().cloned());
    assert_eq!(command["cmd"], "SET_ACTIVITY");
    assert_eq!(command["args"]["pid"], json!(std::process::id()));
    assert!(command["nonce"].is_string());

    let activity = &command["args"]["activity"];
    assert_eq!(activity["type"], 2);
    assert_eq!(activity["status_display_type"], 1);
    assert_eq!(activity["details"], "Song a");
    assert_eq!(activity["state"], "Artist");
    assert_eq!(activity["assets"]["large_image"], "navia_default");
    assert_eq!(activity["assets"]["large_text"], "Album a");

    let start = activity["timestamps"]["start"].as_i64().unwrap();
    let end = activity["timestamps"]["end"].as_i64().unwrap();
    assert_eq!(end - start, 200_000);
    assert!((start - (before - 30_000)).abs() < 2_000, "start {start} before {before}");
}

#[test]
fn pause_and_stop_clear_and_resume_sets_it_again() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());

    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(0)["details"], "Song a");

    client.set_snapshot(&with_state(playing("a", 11), PlaybackState::Paused));
    assert_eq!(fake.command(1), Value::Null);

    client.set_snapshot(&playing("a", 12));
    assert_eq!(fake.command(2)["details"], "Song a");

    client.set_snapshot(&with_state(playing("a", 13), PlaybackState::Stopped));
    assert_eq!(fake.command(3), Value::Null);
    settle();
    assert_eq!(fake.command_count(), 4, "no repeated clears");
}

#[test]
fn private_mode_clears_and_stays_clear() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());

    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(0)["details"], "Song a");

    client.set_snapshot(&with_private(playing("a", 11)));
    assert_eq!(fake.command(1), Value::Null);

    client.set_snapshot(&with_private(playing("b", 0)));
    settle();
    assert_eq!(fake.command_count(), 2, "nothing is sent while private");
}

#[test]
fn starting_in_private_mode_never_connects() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());
    client.set_snapshot(&with_private(playing("a", 10)));
    settle();
    let log = fake.state.log.lock().unwrap();
    assert!(log.handshakes.is_empty());
    assert!(log.commands.is_empty());
}

#[test]
fn normal_progress_does_not_resend_but_a_seek_does() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());

    let mut snapshot = playing("a", 10);
    client.set_snapshot(&snapshot);
    let first = fake.command(0);

    for _ in 0..3 {
        thread::sleep(Duration::from_millis(100));
        snapshot.position += Duration::from_millis(100);
        client.set_snapshot(&snapshot);
    }
    settle();
    assert_eq!(fake.command_count(), 1, "playing on doesn't send updates");

    snapshot.position = Duration::from_secs(150);
    client.set_snapshot(&snapshot);
    let moved = fake.command(1);
    let difference = first["timestamps"]["start"].as_i64().unwrap() - moved["timestamps"]["start"].as_i64().unwrap();
    assert!((difference - 140_000).abs() < 2_000, "difference {difference}");
}

#[test]
fn reconnects_and_republishes_after_losing_discord() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());

    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(0)["details"], "Song a");

    fake.drop_connection();
    client.set_snapshot(&playing("a", 11));
    assert_eq!(fake.command(1)["details"], "Song a");
    assert_eq!(fake.state.log.lock().unwrap().handshakes.len(), 2);
}

#[test]
fn waits_for_discord_to_appear() {
    let dir = tempfile::Builder::new().prefix("nv-ipc").tempdir_in("/tmp").unwrap();
    let path = dir.path().join("discord-ipc-3");
    let config = DiscordConfig {
        socket_dirs: Some(vec![dir.path().to_path_buf()]),
        reconnect_min: Duration::from_millis(50),
        reconnect_max: Duration::from_millis(100),
        ..DiscordConfig::new(APP_ID)
    };
    let client = presence(config);
    client.set_snapshot(&playing("a", 10));
    settle();

    let listener = UnixListener::bind(&path).unwrap();
    let (mut stream, _) = listener.accept().unwrap();
    let (opcode, _) = read_frame(&mut stream).unwrap();
    assert_eq!(opcode, OP_HANDSHAKE);
    write_frame(&mut stream, OP_FRAME, &json!({"evt": "READY"}));
    let (_, command) = read_frame(&mut stream).unwrap();
    assert_eq!(command["args"]["activity"]["details"], "Song a");
}

#[test]
fn a_refused_handshake_is_retried_without_sending_anything() {
    let fake = FakeDiscord::start();
    fake.state.reject.store(true, Ordering::SeqCst);
    let client = presence(fake.config());
    client.set_snapshot(&playing("a", 10));
    fake.wait("two refused handshakes", |log| (log.handshakes.len() >= 2).then_some(()));
    assert_eq!(fake.command_count(), 0);
}

#[test]
fn answers_ping_with_pong() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());
    client.set_snapshot(&playing("a", 10));
    fake.command(0);
    {
        let guard = fake.state.connection.lock().unwrap();
        let mut stream = guard.as_ref().unwrap().try_clone().unwrap();
        write_frame(&mut stream, OP_PING, &json!({"hello": "there"}));
    }
    let pong = fake.wait("pong", |log| log.pongs.first().cloned());
    assert_eq!(pong, json!({"hello": "there"}));
}

#[test]
fn clears_and_closes_on_shutdown() {
    let fake = FakeDiscord::start();
    let client = presence(fake.config());
    client.set_snapshot(&playing("a", 10));
    fake.command(0);
    drop(client);
    assert_eq!(fake.command(1), Value::Null);
    fake.wait("close frame", |log| (log.closes == 1).then_some(()));
}

#[test]
fn bursts_are_rate_limited_and_the_last_track_wins() {
    let fake = FakeDiscord::start();
    let config = DiscordConfig {
        rate_limit: 3,
        rate_window: Duration::from_millis(800),
        ..fake.config()
    };
    let client = presence(config);

    let started = Instant::now();
    for index in 0..12 {
        client.set_snapshot(&playing(&index.to_string(), 0));
        thread::sleep(Duration::from_millis(20));
    }
    let details = fake.wait("final track", |log| {
        log.commands
            .iter()
            .any(|c| c["args"]["activity"]["details"] == "Song 11")
            .then_some(log.commands.len())
    });
    let took = started.elapsed();
    assert!(details < 12, "{details} updates should be fewer than 12");
    if details > 3 {
        assert!(took >= Duration::from_millis(800), "{details} updates in {took:?}");
    }
}

#[test]
fn clears_are_not_held_back_by_the_rate_limit() {
    let fake = FakeDiscord::start();
    let config = DiscordConfig {
        rate_limit: 2,
        rate_window: Duration::from_secs(60),
        ..fake.config()
    };
    let client = presence(config);

    client.set_snapshot(&playing("a", 0));
    fake.command(0);
    client.set_snapshot(&playing("b", 0));
    fake.command(1);
    client.set_snapshot(&playing("c", 0));
    settle();
    assert_eq!(fake.command_count(), 2, "third update waits");

    client.set_snapshot(&with_state(playing("c", 1), PlaybackState::Paused));
    assert_eq!(fake.command(2), Value::Null);
}

struct FixedArt(&'static str);

impl ArtSource for FixedArt {
    fn find(&self, _track: &TrackInfo) -> Result<Option<String>, ArtError> {
        Ok(Some(self.0.to_owned()))
    }
}

#[test]
fn shows_default_image_then_found_art_once_per_album() {
    let fake = FakeDiscord::start();
    let config = DiscordConfig {
        default_image: Some("navia_default".into()),
        ..fake.config()
    };
    let client = DiscordPresence::new(config, Arc::new(FixedArt("https://art.example/cover.jpg"))).unwrap();

    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(0)["assets"]["large_image"], "navia_default");
    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(1)["assets"]["large_image"], "https://art.example/cover.jpg");

    // A second song on the same album goes straight to the found art.
    let mut same_album = playing("b", 0);
    same_album.track.as_mut().unwrap().album = "Album a".into();
    client.set_snapshot(&same_album);
    let third = fake.command(2);
    assert_eq!(third["details"], "Song b");
    assert_eq!(third["assets"]["large_image"], "https://art.example/cover.jpg");
}

/// Answers with whichever cover is set now. A new cover is a new generation, like a new public address for `PublicArt`.
struct SwitchingArt(Mutex<(&'static str, u64)>);

impl ArtSource for SwitchingArt {
    fn find(&self, _track: &TrackInfo) -> Result<Option<String>, ArtError> {
        Ok(Some(self.0.lock().unwrap().0.to_owned()))
    }

    fn generation(&self) -> u64 {
        self.0.lock().unwrap().1
    }
}

#[test]
fn a_new_art_source_generation_drops_cached_covers() {
    let fake = FakeDiscord::start();
    let art = Arc::new(SwitchingArt(Mutex::new(("https://art.example/store.jpg", 0))));
    let client = DiscordPresence::new(DiscordConfig { default_image: Some("navia_default".into()), ..fake.config() }, art.clone()).unwrap();
    client.set_snapshot(&playing("a", 10));
    fake.command(0);
    client.set_snapshot(&playing("a", 10));
    assert_eq!(fake.command(1)["assets"]["large_image"], "https://art.example/store.jpg");

    *art.0.lock().unwrap() = ("https://music.example.org/share/img/t", 1);
    client.set_snapshot(&playing("a", 11));
    settle();
    client.set_snapshot(&playing("a", 12));
    let last = fake.command(fake.command_count() - 1);
    assert_eq!(last["assets"]["large_image"], "https://music.example.org/share/img/t");
}

struct GatedArt {
    gate: Mutex<bool>,
    open: Condvar,
    calls: Mutex<Vec<String>>,
}

impl ArtSource for GatedArt {
    fn find(&self, track: &TrackInfo) -> Result<Option<String>, ArtError> {
        self.calls.lock().unwrap().push(track.id.clone());
        let mut open = self.gate.lock().unwrap();
        while !*open {
            open = self.open.wait(open).unwrap();
        }
        Ok(Some("https://art.example/late.jpg".into()))
    }
}

#[test]
fn late_art_never_escapes_after_private_mode() {
    let fake = FakeDiscord::start();
    let art = Arc::new(GatedArt {
        gate: Mutex::new(false),
        open: Condvar::new(),
        calls: Mutex::default(),
    });
    let client = DiscordPresence::new(fake.config(), art.clone()).unwrap();

    client.set_snapshot(&playing("a", 10));
    fake.command(0);

    client.set_snapshot(&with_private(playing("a", 11)));
    assert_eq!(fake.command(1), Value::Null);

    *art.gate.lock().unwrap() = true;
    art.open.notify_all();
    settle();
    assert_eq!(fake.command_count(), 2, "late art must not republish while private");
}

#[test]
fn private_mode_cancels_queued_lookups() {
    let fake = FakeDiscord::start();
    let art = Arc::new(GatedArt {
        gate: Mutex::new(false),
        open: Condvar::new(),
        calls: Mutex::default(),
    });
    let client = DiscordPresence::new(fake.config(), art.clone()).unwrap();

    client.set_snapshot(&playing("a", 10));
    fake.command(0);
    // The art thread is busy with "a", so this one waits in the queue.
    client.set_snapshot(&playing("b", 0));
    fake.command(1);

    client.set_snapshot(&with_private(playing("b", 1)));
    assert_eq!(fake.command(2), Value::Null);

    *art.gate.lock().unwrap() = true;
    art.open.notify_all();
    settle();
    assert_eq!(*art.calls.lock().unwrap(), vec!["a".to_owned()]);
}

#[test]
fn rejects_an_invalid_application_id() {
    assert!(DiscordPresence::new(DiscordConfig::new("not-a-number"), Arc::new(NoArt)).is_err());
    assert!(DiscordPresence::new(DiscordConfig::new(""), Arc::new(NoArt)).is_err());
}
