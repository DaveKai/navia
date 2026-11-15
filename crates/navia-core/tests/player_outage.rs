//! The idle timer, losing the server mid-playback and live streams that end, against local fakes.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::audio::{synth, to_f32, wav_bytes};
use common::http::{Faults, Library, TestServer};
use navia_core::audio::output::{CaptureBackend, CaptureBuffer};
use navia_core::audio::player::{OutputPolicy, Player, PlayerConfig, PlayerEvent, PlayerStatus, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};

fn client(url: &str) -> Client {
    Client::new(ClientConfig::new(url.to_string(), Credentials::new("t".to_string(), "t".to_string()))).unwrap()
}

fn items(ids: &[(&str, u64)]) -> Vec<QueueItem> {
    ids.iter().map(|(id, ms)| QueueItem::new(id.to_string(), Some(*ms))).collect()
}

struct Rig {
    p: Player,
    capture: Arc<CaptureBuffer>,
    events: Arc<Mutex<Vec<String>>>,
}

fn rig(url: &str, speed: f64, idle: Option<Duration>) -> Rig {
    let backend = CaptureBackend { speed, rates: (8_000, 192_000), ..Default::default() };
    let capture = backend.capture.clone();
    let resolver = ServerResolver::new(client(url)).unwrap();
    let mut config = PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() };
    if let Some(idle) = idle {
        config.idle_close_after = idle;
    }
    let p = Player::with_backend(Arc::new(resolver), Box::new(backend), config).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    p.add_listener(move |e| {
        if matches!(e, PlayerEvent::TrackChanged { .. } | PlayerEvent::StatusChanged(_) | PlayerEvent::Connection { .. } | PlayerEvent::QueueEnded | PlayerEvent::Error(_)) {
            sink.lock().unwrap().push(format!("{e:?}"));
        }
    });
    Rig { p, capture, events }
}

fn wait_for(limit: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn wait_status(p: &Player, status: PlayerStatus, limit: Duration) {
    p.sync();
    assert!(wait_for(limit, || p.snapshot().status == status), "never reached {status:?}, snapshot {:?}", p.snapshot());
}

/// Drops every silent frame. Outages and the start add silence, and that's all they may add.
fn sounding(samples: &[f32]) -> Vec<f32> {
    samples.chunks_exact(2).filter(|f| f[0] != 0.0 || f[1] != 0.0).flatten().copied().collect()
}

fn assert_contains_exact(haystack: &[f32], needle: &[f32], what: &str) {
    assert!(haystack.len() >= needle.len(), "{what}: captured {} samples, expected at least {}", haystack.len(), needle.len());
    let bad = haystack.iter().zip(needle).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(bad, None, "{what}: differs at sample {bad:?}");
}

fn tone(secs: f64, seed: f64) -> Vec<i16> {
    synth((44_100.0 * secs) as usize, 44_100, seed)
}

/// `synth` never starts on two zero samples for these seeds, so `sounding` keeps every frame.
fn expect_all(tracks: &[&Vec<i16>]) -> Vec<f32> {
    tracks.iter().flat_map(|t| to_f32(t)).collect()
}

// Idle timer.

fn restart_after(setup: impl FnOnce(&Rig, &TestServer)) {
    let a = tone(0.3, 0.2);
    let b = tone(6.0, 1.2);
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    library.raw.insert("b".into(), wav_bytes(&b, 44_100));
    let server = TestServer::start(library, Faults::default());
    let r = rig(&server.url, 1.0, Some(Duration::from_millis(1500)));
    setup(&r, &server);
    // Well past the idle limit counted from the earlier stop.
    r.p.set_queue(items(&[("b", 6000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(3000));
    let before = r.capture.len();
    let position = r.p.snapshot().position_ms;
    std::thread::sleep(Duration::from_millis(500));
    assert!(r.capture.len() > before, "the output closed under new playback (position {position} ms, status {:?})", r.p.snapshot().status);
    assert!(position > 2000, "position stalled at {position} ms");
    assert_eq!(r.capture.opens().len(), r.capture.opens().iter().map(|o| o.1).collect::<std::collections::HashSet<_>>().len());
    r.p.stop();
}

#[test]
fn playback_after_the_queue_ended_is_not_closed_by_the_old_idle_timer() {
    restart_after(|r, _| {
        r.p.set_queue(items(&[("a", 300)]), 0, 0, false);
        wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(10));
    });
}

#[test]
fn playback_after_stop_is_not_closed_by_the_old_idle_timer() {
    restart_after(|r, _| {
        r.p.set_queue(items(&[("b", 6000)]), 0, 0, false);
        wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
        r.p.stop();
        wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(2));
    });
}

#[test]
fn the_output_closes_when_idle_but_only_when_idle() {
    let a = tone(4.0, 0.7);
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    let server = TestServer::start(library, Faults::default());
    let r = rig(&server.url, 1.0, Some(Duration::from_millis(400)));
    r.p.set_queue(items(&[("a", 4000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    r.p.pause();
    wait_status(&r.p, PlayerStatus::Paused, Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(900));
    let closed = r.capture.len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(r.capture.len(), closed, "paused past the limit: the device should be closed");
    r.p.play();
    assert!(wait_for(Duration::from_secs(3), || r.capture.len() > closed), "resume reopens the device");
    r.p.stop();
}

#[test]
fn listeners_hear_a_status_that_set_queue_wrote_early() {
    let a = tone(4.0, 0.2);
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    let server = TestServer::start(library, Faults::default());
    let r = rig(&server.url, 1.0, None);
    r.p.set_queue(items(&[("a", 4000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    r.p.stop();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(2));
    r.events.lock().unwrap().clear();
    r.p.set_queue(items(&[("a", 4000)]), 0, 0, true);
    r.p.sync();
    assert!(wait_for(Duration::from_secs(2), || r.events.lock().unwrap().iter().any(|e| e.contains("StatusChanged(Paused)"))), "{:?}", r.events.lock().unwrap());
    r.p.stop();
}

// Losing the server.

#[test]
fn losing_the_server_mid_track_holds_the_place_and_resumes_bit_exact() {
    let a = tone(9.0, 0.4);
    let b = tone(3.0, 1.7);
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    library.raw.insert("b".into(), wav_bytes(&b, 44_100));
    // Slow enough that the file is still arriving when the server goes away.
    let server = TestServer::start(library, Faults { throttle: 400_000, ..Default::default() });
    let r = rig(&server.url, 4.0, None);
    r.p.set_queue(items(&[("a", 9000), ("b", 3000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    assert!(wait_for(Duration::from_secs(5), || r.p.snapshot().position_ms > 300), "no audio");
    server.set_down(true);

    assert!(wait_for(Duration::from_secs(8), || r.p.snapshot().waiting_for_server), "never noticed the outage: {:?}", r.p.snapshot());
    wait_status(&r.p, PlayerStatus::Buffering, Duration::from_secs(6));
    let held = r.p.snapshot();
    std::thread::sleep(Duration::from_millis(1500));
    let later = r.p.snapshot();
    assert_eq!(later.position_ms, held.position_ms, "position must not move while there's no audio");
    assert_eq!(later.item.as_ref().map(|i| i.id.clone()), Some("a".to_string()), "must stay on the same track");

    server.set_down(false);
    r.p.retry_now();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(30));
    assert!(!r.p.snapshot().waiting_for_server);
    let captured = sounding(&r.capture.samples());
    assert_contains_exact(&captured, &expect_all(&[&a, &b]), "whole queue after the outage");
    let events = r.events.lock().unwrap().join("\n");
    assert!(events.contains("Connection { waiting: true") && events.contains("Connection { waiting: false"), "{events}");
    assert!(!events.contains("Error"), "an outage isn't a track failure: {events}");
}

#[test]
fn an_outage_between_tracks_does_not_run_through_the_queue() {
    let (a, b, c, d) = (tone(6.0, 0.3), tone(6.0, 0.9), tone(6.0, 1.4), tone(6.0, 2.1));
    let mut library = Library::default();
    for (id, pcm) in [("a", &a), ("b", &b), ("c", &c), ("d", &d)] {
        library.raw.insert(id.into(), wav_bytes(pcm, 44_100));
    }
    let server = TestServer::start(library, Faults::default());
    let r = rig(&server.url, 6.0, None);
    r.p.set_queue(items(&[("a", 6000), ("b", 6000), ("c", 6000), ("d", 6000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    // All of a is fetched, and b's fetch fails.
    server.set_down(true);
    assert!(wait_for(Duration::from_secs(20), || r.p.snapshot().status == PlayerStatus::Buffering), "never waited: {:?}", r.p.snapshot());
    std::thread::sleep(Duration::from_secs(4));
    let snap = r.p.snapshot();
    assert_eq!(snap.index.unwrap_or(0).max(1), 1, "ran ahead into the queue during the outage: {snap:?}");
    assert!(snap.waiting_for_server);

    server.set_down(false);
    r.p.retry_now();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(30));
    let captured = sounding(&r.capture.samples());
    assert_contains_exact(&captured, &expect_all(&[&a, &b, &c, &d]), "every track played, in order");
}

#[test]
fn next_during_an_outage_moves_the_place_and_keeps_waiting() {
    let (a, b, c) = (tone(8.0, 0.3), tone(1.0, 0.9), tone(2.0, 1.4));
    let mut library = Library::default();
    for (id, pcm) in [("a", &a), ("b", &b), ("c", &c)] {
        library.raw.insert(id.into(), wav_bytes(pcm, 44_100));
    }
    let server = TestServer::start(library, Faults { throttle: 300_000, ..Default::default() });
    let r = rig(&server.url, 1.0, None);
    r.p.set_queue(items(&[("a", 8000), ("b", 1000), ("c", 2000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    server.set_down(true);
    assert!(wait_for(Duration::from_secs(8), || r.p.snapshot().waiting_for_server), "never waited");
    let before = Instant::now();
    r.p.next();
    r.p.next();
    r.p.sync();
    std::thread::sleep(Duration::from_millis(400));
    assert!(before.elapsed() < Duration::from_secs(3), "commands waited on a blocked read");
    assert!(r.p.snapshot().waiting_for_server, "still waiting for the server, on the next track");
    server.set_down(false);
    r.p.retry_now();
    assert!(wait_for(Duration::from_secs(10), || r.p.snapshot().item.as_ref().is_some_and(|i| i.id == "c")), "{:?}", r.p.snapshot());
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(10));
}

#[test]
fn stop_and_seek_during_an_outage_are_prompt() {
    let a = tone(6.0, 0.6);
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    let server = TestServer::start(library, Faults { throttle: 250_000, ..Default::default() });
    let r = rig(&server.url, 1.0, None);
    r.p.set_queue(items(&[("a", 6000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    server.set_down(true);
    assert!(wait_for(Duration::from_secs(8), || r.p.snapshot().waiting_for_server), "never waited");
    // A seek interrupts the blocked read and stays on the track.
    r.p.seek_ms(3000);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(r.p.snapshot().item.as_ref().map(|i| i.id.as_str()), Some("a"));
    let before = Instant::now();
    r.p.stop();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(3));
    assert!(before.elapsed() < Duration::from_secs(2));
    server.set_down(false);
}

#[test]
fn a_missing_track_still_skips_at_once() {
    let (a, c) = (tone(1.0, 0.3), tone(1.0, 1.4));
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    library.raw.insert("c".into(), wav_bytes(&c, 44_100));
    let server = TestServer::start(library, Faults::default());
    let r = rig(&server.url, 8.0, None);
    r.p.set_queue(items(&[("a", 1000), ("missing", 1000), ("c", 1000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(15));
    let captured = sounding(&r.capture.samples());
    assert_contains_exact(&captured, &expect_all(&[&a, &c]), "a and c, with the 404 skipped");
    assert!(!r.p.snapshot().waiting_for_server);
}

// Interrupts while a track is opening or waiting.
// `play_track` can return `Seek` or `Resume` in each of these, and the run loop must handle it.

fn one_track(id: &str, pcm: &[i16], faults: Faults) -> TestServer {
    let mut library = Library::default();
    library.raw.insert(id.into(), wav_bytes(pcm, 44_100));
    TestServer::start(library, faults)
}

fn frames_from(pcm: &[i16], ms: u64) -> Vec<f32> {
    to_f32(&pcm[(ms * 44_100 / 1000) as usize * 2..])
}

fn assert_ends_with(haystack: &[f32], needle: &[f32], what: &str) {
    assert!(haystack.len() >= needle.len(), "{what}: captured {} samples, expected at least {}", haystack.len(), needle.len());
    assert_contains_exact(&haystack[haystack.len() - needle.len()..], needle, what);
}

#[test]
fn a_seek_while_the_open_waits_for_the_server_restarts_there() {
    let a = tone(6.0, 0.4);
    let server = one_track("a", &a, Faults::default());
    server.set_down(true);
    let r = rig(&server.url, 8.0, None);
    // A restored session, loaded and paused with the server already gone.
    r.p.set_queue(items(&[("a", 6000)]), 0, 1000, true);
    r.p.sync();
    assert!(wait_for(Duration::from_secs(8), || r.p.snapshot().waiting_for_server), "never waited: {:?}", r.p.snapshot());
    assert_eq!(r.p.snapshot().position_ms, 1000, "the restored place is held while waiting");

    r.p.seek_ms(2500);
    assert!(wait_for(Duration::from_secs(2), || r.p.snapshot().position_ms == 2500), "the seek isn't reflected: {:?}", r.p.snapshot());
    std::thread::sleep(Duration::from_millis(300));
    assert!(r.p.snapshot().waiting_for_server, "still waiting for the server: {:?}", r.p.snapshot());

    server.set_down(false);
    r.p.retry_now();
    assert!(wait_for(Duration::from_secs(10), || !r.p.snapshot().waiting_for_server), "never recovered: {:?}", r.p.snapshot());
    r.p.play();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(30));
    // Resuming from pause ramps in over 256 frames, and everything after that is bit-exact.
    let expected = sounding(&frames_from(&a, 2500));
    assert_ends_with(&sounding(&r.capture.samples()), &expected[1024..], "from the seek to the end");
    let events = r.events.lock().unwrap().join("\n");
    assert!(!events.contains("Error"), "an outage isn't a track failure: {events}");
}

#[test]
fn a_seek_while_a_transcode_reopen_waits_restarts_there() {
    let a = tone(8.0, 0.4);
    let mut library = Library::default();
    library.raw.insert("a".into(), b"this is not audio".to_vec());
    library.transcoded.insert(("a".into(), "mp3".into()), wav_bytes(&a, 44_100));
    let server = TestServer::start(library, Faults { throttle: 400_000, ..Default::default() });
    let r = rig(&server.url, 1.0, None);
    r.p.set_queue(items(&[("a", 8000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    assert!(wait_for(Duration::from_secs(5), || r.p.snapshot().position_ms > 100), "no audio");
    // The stream can't wait in place, so the engine reopens it at `decoded_ms` and waits there.
    server.set_down(true);
    assert!(wait_for(Duration::from_secs(10), || r.p.snapshot().waiting_for_server), "never noticed the outage: {:?}", r.p.snapshot());
    r.p.seek_ms(2000);
    assert!(wait_for(Duration::from_secs(2), || r.p.snapshot().position_ms == 2000), "the seek isn't reflected: {:?}", r.p.snapshot());
    server.set_down(false);
    r.p.retry_now();
    assert!(wait_for(Duration::from_secs(10), || !r.p.snapshot().waiting_for_server), "never recovered: {:?}", r.p.snapshot());
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    assert!(wait_for(Duration::from_secs(5), || r.p.snapshot().position_ms >= 2000 && r.p.snapshot().position_ms < 8000), "playing past the seek: {:?}", r.p.snapshot());
    let last = server.requests().into_iter().rev().find(|q| q.format.as_deref() == Some("mp3")).unwrap();
    assert_eq!(last.time_offset, Some(2), "the transcode restarts at the seek");
    r.p.stop();
}

#[test]
fn a_seek_while_opening_a_slow_track_restarts_there() {
    let a = tone(6.0, 0.4);
    let server = one_track("a", &a, Faults { stall: Duration::from_millis(1500), ..Default::default() });
    let r = rig(&server.url, 8.0, None);
    r.p.set_queue(items(&[("a", 6000)]), 0, 0, false);
    r.p.sync();
    // While the first request is still in flight it's a slow server, not an outage.
    std::thread::sleep(Duration::from_millis(200));
    r.p.seek_ms(3000);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(10));
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(30));
    assert_ends_with(&sounding(&r.capture.samples()), &sounding(&frames_from(&a, 3000)), "from the seek to the end");
    let events = r.events.lock().unwrap().join("\n");
    assert!(!events.contains("Error"), "nothing failed: {events}");
}

#[test]
fn a_seek_while_the_next_track_opens_applies_to_the_track_being_heard() {
    // Short enough that the decoder has finished `a` and is waiting on `b` while `a` is heard.
    let (a, b) = (tone(1.4, 0.3), tone(1.0, 0.9));
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&a, 44_100));
    library.raw.insert("b".into(), wav_bytes(&b, 44_100));
    let server = TestServer::start(library, Faults { stall: Duration::from_millis(1500), ..Default::default() });
    let r = rig(&server.url, 1.0, None);
    r.p.set_queue(items(&[("a", 1400), ("b", 1000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(10));
    assert_eq!(r.p.snapshot().index, Some(0));
    r.p.seek_ms(700);
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(30));
    let mut expected = sounding(&frames_from(&a, 700));
    expected.extend(sounding(&to_f32(&b)));
    assert_ends_with(&sounding(&r.capture.samples()), &expected, "the rest of a after the seek, then b");
}

#[test]
fn a_second_seek_during_a_blocked_flac_seek_stays_on_the_track() {
    if !common::audio::have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = tone(30.0, 0.4);
    let b = tone(1.0, 0.9);
    let wav = dir.path().join("a.wav");
    common::audio::write_wav(&wav, &a, 44_100);
    let flac = dir.path().join("a.flac");
    common::audio::encode(&wav, &flac, &["-c:a", "flac"]);
    let mut library = Library::default();
    library.raw.insert("a".into(), std::fs::read(&flac).unwrap());
    library.raw.insert("b".into(), wav_bytes(&b, 44_100));
    let server = TestServer::start(library, Faults { throttle: 150_000, ..Default::default() });
    let r = rig(&server.url, 6.0, None);
    r.p.set_queue(items(&[("a", 30_000), ("b", 1000)]), 0, 0, false);
    wait_status(&r.p, PlayerStatus::Playing, Duration::from_secs(5));
    server.set_down(true);
    assert!(wait_for(Duration::from_secs(10), || r.p.snapshot().waiting_for_server), "never noticed the outage: {:?}", r.p.snapshot());

    // The end of the file isn't here, so the FLAC seek itself blocks on the read.
    r.p.seek_ms(25_000);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(r.p.snapshot().item.as_ref().map(|i| i.id.as_str()), Some("a"));
    // A drag sends several seeks in a row, and the next one interrupts the blocked first.
    r.p.seek_ms(20_000);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(r.p.snapshot().item.as_ref().map(|i| i.id.as_str()), Some("a"), "the interrupted seek skipped the track");
    assert_eq!(r.p.snapshot().position_ms, 20_000);

    server.set_down(false);
    r.p.retry_now();
    wait_status(&r.p, PlayerStatus::Stopped, Duration::from_secs(40));
    let mut expected = sounding(&frames_from(&a, 20_000));
    expected.extend(sounding(&to_f32(&b)));
    assert_ends_with(&sounding(&r.capture.samples()), &expected, "the rest of a after the second seek, then b");
    let events = r.events.lock().unwrap().join("\n");
    assert!(!events.contains("Error"), "a seek during an outage isn't a track failure: {events}");
}

#[test]
fn a_panicking_listener_stops_the_player_with_an_error_not_silently() {
    let a = tone(4.0, 0.2);
    let server = one_track("a", &a, Faults::default());
    let r = rig(&server.url, 1.0, None);
    r.p.add_listener(|e| {
        if matches!(e, PlayerEvent::QueueChanged) {
            panic!("listener failed");
        }
    });
    let events = r.events.clone();
    r.p.set_queue(items(&[("a", 4000)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(5), || r.p.snapshot().error.is_some()), "no error state: {:?}", r.p.snapshot());
    let snap = r.p.snapshot();
    assert_eq!(snap.status, PlayerStatus::Stopped, "{snap:?}");
    assert!(events.lock().unwrap().iter().any(|e| e.contains("Error")), "the error reached listeners");
    // Later commands report the failure instead of going nowhere.
    let before = events.lock().unwrap().len();
    r.p.play();
    assert!(wait_for(Duration::from_secs(2), || events.lock().unwrap().len() > before), "a command after the player died must say so");
}
