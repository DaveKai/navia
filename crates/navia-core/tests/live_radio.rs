//! Internet radio against fake stations, covering ICY metadata, status lines, playlists and failures.
//! Audio goes to an in-memory backend, and no real station is contacted.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use navia_core::audio::output::CaptureBackend;
use navia_core::audio::player::{Player, PlayerConfig, PlayerEvent, PlayerStatus, QueueItem, TrackResolver};
use navia_core::audio::{TrackOpener, player::FileResolver};
use navia_core::error::Error;

struct NoFiles;
impl TrackResolver for NoFiles {
    fn opener(&self, _: &QueueItem) -> Arc<dyn TrackOpener> {
        struct No;
        impl TrackOpener for No {
            fn open(&self, _: u64) -> Result<(navia_core::audio::Decoder, u64), Error> {
                Err(Error::Audio("no file".to_string()))
            }
        }
        Arc::new(No)
    }
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
        && Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("libmp3lame"))
}

fn mp3_bytes(dir: &Path, secs: u32) -> Vec<u8> {
    let out = dir.join("s.mp3");
    let status = Command::new("ffmpeg").args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", &format!("sine=frequency=800:sample_rate=44100:duration={secs}"), "-ac", "2", "-c:a", "libmp3lame", "-b:a", "128k"]).arg(&out).status().unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
}

/// Serves one TCP connection at a time with `response(request_line)`. Returns its address.
fn serve(responder: impl Fn(&str, &mut std::net::TcpStream) + Send + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => return,
            };
            let mut req = Vec::new();
            let mut byte = [0u8; 1];
            while !req.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => req.push(byte[0]),
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&req).to_string();
            let line = text.lines().next().unwrap_or("").to_string();
            responder(&line, &mut stream);
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// An MP3 stream paced to real time, with ICY metadata every `metaint` bytes, for `seconds`.
fn write_icy_stream(stream: &mut std::net::TcpStream, mp3: &[u8], metaint: usize, title: &str, seconds: f64) {
    let block: Vec<u8> = {
        let text = format!("StreamTitle='{title}';StreamUrl='';");
        let mut v = text.into_bytes();
        let pad = (16 - v.len() % 16) % 16;
        v.extend(std::iter::repeat_n(0u8, pad));
        v
    };
    let start = Instant::now();
    let rate = 128_000.0 / 8.0;
    let mut sent = 0usize;
    let mut since_meta = 0usize;
    for chunk in mp3.chunks(1024) {
        let mut chunk = chunk;
        while !chunk.is_empty() {
            let take = (metaint - since_meta).min(chunk.len());
            if stream.write_all(&chunk[..take]).is_err() {
                return;
            }
            chunk = &chunk[take..];
            since_meta += take;
            sent += take;
            if since_meta == metaint {
                since_meta = 0;
                if stream.write_all(&[(block.len() / 16) as u8]).is_err() || stream.write_all(&block).is_err() {
                    return;
                }
            }
        }
        let due = Duration::from_secs_f64(sent as f64 / rate);
        if let Some(wait) = due.checked_sub(start.elapsed()) {
            thread::sleep(wait);
        }
        if start.elapsed().as_secs_f64() > seconds {
            return;
        }
    }
}

fn new_player() -> (Arc<Player>, Arc<navia_core::audio::output::CaptureBuffer>, Arc<Mutex<Vec<PlayerEvent>>>) {
    let backend = CaptureBackend { speed: 1.0, ..Default::default() };
    let capture = backend.capture.clone();
    let player = Arc::new(Player::with_backend(Arc::new(NoFiles), Box::new(backend), PlayerConfig::default()).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    player.add_listener(move |e| sink.lock().unwrap().push(e.clone()));
    (player, capture, events)
}

fn wait_for(p: &Player, status: PlayerStatus, limit: Duration) -> bool {
    let start = Instant::now();
    p.sync();
    while start.elapsed() < limit {
        if p.snapshot().status == status {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn item() -> QueueItem {
    QueueItem::new("station".to_string(), None)
}

/// The loudest sample, since a capture also holds the silence before a station starts.
fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0f32, |m, s| m.max(s.abs()))
}

fn play_url(url: String) -> (Arc<Player>, Arc<navia_core::audio::output::CaptureBuffer>, Arc<Mutex<Vec<PlayerEvent>>>) {
    let (p, c, e) = new_player();
    p.play_live(item(), url);
    (p, c, e)
}

#[test]
fn old_shoutcast_status_line_and_stream_titles_work() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mp3 = mp3_bytes(dir.path(), 20);
    let url = serve(move |_, stream| {
        let _ = stream.write_all(b"ICY 200 OK\r\nicy-name:Test\r\nicy-metaint:4096\r\ncontent-type:audio/mpeg\r\n\r\n");
        write_icy_stream(stream, &mp3, 4096, "Artist - A Song", 14.0);
    });
    let (player, capture, events) = play_url(url);
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "never started: {:?}", events.lock().unwrap());
    assert!(player.snapshot().live);
    // Seeking does nothing on a live stream.
    let before = player.snapshot().position_ms;
    player.seek_ms(90_000);
    thread::sleep(Duration::from_millis(800));
    let after = player.snapshot().position_ms;
    assert!(after < 30_000 && after >= before, "seek must be ignored: {before} -> {after}");
    // Titles and audio both come through the same connection.
    let deadline = Instant::now() + Duration::from_secs(6);
    while player.stream_title().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(player.stream_title().as_deref(), Some("Artist - A Song"));
    player.stop();
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(5)));
    assert!(!player.snapshot().live);
    let samples = capture.samples();
    assert!(peak(&samples) > 0.05, "no audio in the capture");
    assert_eq!(player.snapshot().underrun_frames, 0);
}

#[test]
fn normal_http_station_with_metaint_header_and_pause_rejoins_live() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Arc::new(mp3_bytes(dir.path(), 30));
    let connections = Arc::new(Mutex::new(0usize));
    let count = connections.clone();
    let url = serve(move |_, stream| {
        *count.lock().unwrap() += 1;
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nicy-metaint: 2048\r\n\r\n");
        write_icy_stream(stream, &mp3, 2048, "Title", 20.0);
    });
    let (player, capture, events) = play_url(url);
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "never started: {:?}", events.lock().unwrap());
    thread::sleep(Duration::from_millis(1500));
    player.pause();
    assert!(wait_for(&player, PlayerStatus::Paused, Duration::from_secs(3)));
    // The connection is dropped on pause and the queue-end from that is not an error.
    thread::sleep(Duration::from_millis(600));
    assert_eq!(player.snapshot().status, PlayerStatus::Paused);
    assert_eq!(player.snapshot().live, true);
    player.play();
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "{:?}", events.lock().unwrap());
    thread::sleep(Duration::from_millis(800));
    assert!(*connections.lock().unwrap() >= 2, "the stream reconnected at live");
    player.stop();
    assert!(peak(&capture.samples()) > 0.05);
    let bad: Vec<_> = events.lock().unwrap().iter().filter(|e| matches!(e, PlayerEvent::Error(_))).cloned().collect();
    assert!(bad.is_empty(), "{bad:?}");
}

#[test]
fn playlist_urls_resolve_and_hls_is_refused() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Arc::new(mp3_bytes(dir.path(), 5));
    let mp3_for = mp3.clone();
    let stream_url = serve(move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n");
        write_icy_stream(stream, &mp3_for, 1_000_000_000, "", 4.0);
    });
    let target = stream_url.clone();
    let pls = serve(move |_, stream| {
        let body = format!("[playlist]\nFile1={target}\nLength1=-1\n");
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: audio/x-scpls\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    let (player, _, events) = play_url(format!("{pls}/radio.pls"));
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "{:?}", events.lock().unwrap());
    player.stop();

    let hls = serve(|_, stream| {
        let body = "#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:6,\nchunk0.ts\n";
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    let (player, _, events) = play_url(format!("{hls}/live.m3u8"));
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(5)));
    let events = events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e, PlayerEvent::Error(m) if m.contains("HLS"))), "{events:?}");
}

fn expect_failure(url: String, want: &str) {
    let (player, _, events) = play_url(url);
    let started = Instant::now();
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(12)), "should give up, got {:?}", events.lock().unwrap());
    assert!(started.elapsed() < Duration::from_secs(12));
    let ev = events.lock().unwrap();
    assert!(ev.iter().any(|e| matches!(e, PlayerEvent::Error(m) if m.contains(want))), "wanted {want:?} in {ev:?}");
    assert!(!player.snapshot().live, "the failed station is not left marked live");
}

#[test]
fn dead_stations_fail_cleanly_and_quickly() {
    // Nothing listens here.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    expect_failure(format!("http://127.0.0.1:{port}/stream"), "responding");

    let not_found = serve(|_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    });
    expect_failure(format!("{not_found}/stream"), "HTTP 404");

    let html = serve(|_, stream| {
        let body = "<html>hello</html>";
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    });
    expect_failure(format!("{html}/stream"), "web page");

    let garbage = serve(|_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\r\n");
        let _ = stream.write_all(&[0xAAu8; 100_000]);
    });
    expect_failure(format!("{garbage}/stream"), "can't be played");
}

#[test]
fn a_stalled_station_does_not_hold_the_engine_when_stopped() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    Command::new("ffmpeg").args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-ac", "2"]).arg(&wav).status().unwrap();
    let (tx, rx) = channel();
    let mp3 = Arc::new(mp3_bytes(dir.path(), 3));
    let stall = serve(move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n");
        let _ = stream.write_all(&mp3[..30_000]);
        let _ = rx.recv_timeout(Duration::from_secs(60));
    });
    let backend = CaptureBackend { speed: 0.0, ..Default::default() };
    let capture = backend.capture.clone();
    let map = std::collections::HashMap::from([("0".to_string(), PathBuf::from(&wav))]);
    let player = Player::with_backend(Arc::new(FileResolver(map)), Box::new(backend), PlayerConfig::default()).unwrap();
    player.play_live(item(), stall);
    thread::sleep(Duration::from_millis(1200));
    let before = Instant::now();
    player.set_queue(vec![QueueItem::new("0".into(), None)], 0, 0, false);
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(10)), "file after stalled stream");
    assert!(before.elapsed() < Duration::from_secs(8), "stalled read held the engine for {:?}", before.elapsed());
    assert!(capture.samples().len() > 44_100);
    let _ = tx.send(());
}

/// A station whose connection closes and that then refuses to come back.
#[test]
fn a_station_that_ends_stops_instead_of_playing_silence() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Arc::new(mp3_bytes(dir.path(), 20));
    let dead = Arc::new(Mutex::new(false));
    let flag = dead.clone();
    let url = serve(move |_, stream| {
        if *flag.lock().unwrap() {
            return; // Accepts and closes, so the station is gone.
        }
        *flag.lock().unwrap() = true;
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n");
        write_icy_stream(stream, &mp3, 1_000_000_000, "", 4.0);
    });
    let (player, _capture, events) = play_url(url);
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "{:?}", events.lock().unwrap());
    let started = Instant::now();
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(20)), "still {:?} after the stream ended", player.snapshot().status);
    assert!(started.elapsed() < Duration::from_secs(15), "took {:?}", started.elapsed());
    assert!(!player.snapshot().live);
    let log = events.lock().unwrap();
    assert!(log.iter().any(|e| matches!(e, PlayerEvent::Error(_))), "no error told the user: {log:?}");
    assert!(log.iter().any(|e| matches!(e, PlayerEvent::QueueEnded)), "{log:?}");
}

/// A station that keeps its connection open but sends nothing more.
#[test]
fn a_silent_station_gives_up_within_seconds() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg with libmp3lame not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mp3 = Arc::new(mp3_bytes(dir.path(), 5));
    let url = serve(move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n");
        write_icy_stream(stream, &mp3, 1_000_000_000, "", 4.0);
        thread::sleep(Duration::from_secs(120));
    });
    let (player, _capture, events) = play_url(url);
    assert!(wait_for(&player, PlayerStatus::Playing, Duration::from_secs(5)), "{:?}", events.lock().unwrap());
    let started = Instant::now();
    assert!(wait_for(&player, PlayerStatus::Stopped, Duration::from_secs(45)), "still {:?}", player.snapshot().status);
    println!("gave up after {:?}", started.elapsed());
    assert!(started.elapsed() < Duration::from_secs(30), "took {:?}", started.elapsed());
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, PlayerEvent::Error(_))));
}
