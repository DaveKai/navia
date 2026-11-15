//! The play log from a real player, recording how each play ended and how much was heard.
//! Runs against a local fake of `/stream` and an in-memory capture device.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::audio::{synth, wav_bytes};
use common::http::{Faults, Library as Songs, TestServer};
use navia_core::audio::output::CaptureBackend;
use navia_core::audio::player::{OutputPolicy, PlayEnd, Player, PlayerConfig, PlayerEvent, PlayerStatus, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};
use navia_core::engine::playlog::{self, Ended, Origin};
use navia_core::library::{Library, LibraryConfig};

const RATE: u32 = 44_100;

type Ends = Arc<Mutex<Vec<(String, u64, PlayEnd)>>>;

fn setup(tracks: &[(&str, f64)]) -> (TestServer, Player, Ends, Vec<QueueItem>) {
    let mut songs = Songs::default();
    let mut items = Vec::new();
    for (k, (id, secs)) in tracks.iter().enumerate() {
        let frames = (RATE as f64 * secs) as usize;
        songs.raw.insert(id.to_string(), wav_bytes(&synth(frames, RATE, 0.4 + k as f64), RATE));
        items.push(QueueItem::new(id.to_string(), Some((secs * 1000.0) as u64)));
    }
    let server = TestServer::start(songs, Faults::default());
    let backend = CaptureBackend { speed: 1.0, rates: (8_000, 192_000), ..Default::default() };
    let client = Client::new(ClientConfig::new(server.url.clone(), Credentials::new("t".to_string(), "t".to_string()))).unwrap();
    let player = Player::with_backend(Arc::new(ServerResolver::new(client).unwrap()), Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
    let ends: Ends = Default::default();
    let sink = ends.clone();
    player.add_listener(move |e| {
        if let PlayerEvent::PlayEnded { item, heard_ms, end, .. } = e {
            sink.lock().unwrap().push((item.id.clone(), *heard_ms, *end));
        }
    });
    (server, player, ends, items)
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

fn playing(p: &Player, id: &str) -> bool {
    p.snapshot().item.is_some_and(|i| i.id == id) && p.snapshot().status == PlayerStatus::Playing
}

fn near(got: u64, want: u64, slack: u64) -> bool {
    got + slack >= want && got <= want + slack
}

#[test]
fn every_play_ends_finished_skipped_or_replaced_with_what_was_heard() {
    let (_server, p, ends, items) = setup(&[("a", 1.0), ("b", 3.0), ("c", 3.0), ("d", 1.0)]);
    let dir = tempfile::tempdir().unwrap();
    let db = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    playlog::attach_play_log(&p, db.clone());

    p.set_queue(items, 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "b")), "a never moved on to b");
    std::thread::sleep(Duration::from_millis(600));
    p.next();
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "c")), "Next never reached c");
    std::thread::sleep(Duration::from_millis(400));
    p.jump_to(3);
    assert!(wait_for(Duration::from_secs(10), || p.snapshot().status == PlayerStatus::Stopped), "the queue never ended");

    let ends = ends.lock().unwrap().clone();
    let shape: Vec<(&str, PlayEnd)> = ends.iter().map(|(id, _, end)| (id.as_str(), *end)).collect();
    assert_eq!(shape, [("a", PlayEnd::Finished), ("b", PlayEnd::Skipped), ("c", PlayEnd::Replaced), ("d", PlayEnd::Finished)], "{ends:?}");
    let heard: Vec<u64> = ends.iter().map(|(_, ms, _)| *ms).collect();
    assert!(near(heard[0], 1000, 60), "a heard whole: {heard:?}");
    assert!(near(heard[1], 600, 250), "b heard about 0.6 s: {heard:?}");
    assert!(near(heard[2], 400, 250), "c heard about 0.4 s: {heard:?}");
    assert!(near(heard[3], 1000, 60), "d heard whole: {heard:?}");

    // The log has the same four, written on its own thread.
    assert!(wait_for(Duration::from_secs(5), || playlog::events_since(&db, 0).unwrap().iter().filter(|e| e.ended.is_some()).count() == 4), "the log never caught up");
    let events = playlog::events_since(&db, 0).unwrap();
    let logged: Vec<(&str, Option<Ended>, Origin)> = events.iter().map(|e| (e.song_id.as_str(), e.ended, e.origin)).collect();
    assert_eq!(logged, [("a", Some(Ended::Finished), Origin::User), ("b", Some(Ended::Skipped), Origin::User), ("c", Some(Ended::Replaced), Origin::User), ("d", Some(Ended::Finished), Origin::User)]);
    assert!(events[1].early_skip());
    assert!(events.iter().all(|e| e.session == events[0].session));
    assert_eq!(playlog::song_stat(&db, "a").completions, 1);
    assert_eq!(playlog::song_stat(&db, "b").early_skips, 1);
    assert_eq!(playlog::seed_ids(&db, playlog::now_ms(), false, 5), ["d", "a"]);
}

#[test]
fn a_queue_rebuilt_around_the_playing_song_is_one_play_and_stop_replaces() {
    let (_server, p, ends, items) = setup(&[("x", 3.0), ("y", 1.0)]);
    p.set_queue(items.clone(), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "x")));
    std::thread::sleep(Duration::from_millis(300));
    // Rebuilt around x where it is, as when a station or listen-along ends.
    let at = p.snapshot().position_ms.max(1);
    p.set_queue(items, 0, at, false);
    std::thread::sleep(Duration::from_millis(300));
    assert!(ends.lock().unwrap().is_empty(), "carrying on isn't an end: {:?}", ends.lock().unwrap());
    p.stop();
    assert!(wait_for(Duration::from_secs(5), || !ends.lock().unwrap().is_empty()));
    let ends = ends.lock().unwrap().clone();
    assert_eq!(ends.len(), 1);
    assert_eq!((ends[0].0.as_str(), ends[0].2), ("x", PlayEnd::Replaced));
    assert!(near(ends[0].1, 600, 250), "both parts counted: {ends:?}");
}

#[test]
fn songs_heard_while_following_are_logged_as_along() {
    let (_server, p, _ends, items) = setup(&[("f", 1.0), ("own", 1.0)]);
    let following: Arc<Mutex<Vec<(String, bool)>>> = Default::default();
    let sink = following.clone();
    p.add_listener(move |e| {
        if let PlayerEvent::PlayEnded { item, following, .. } = e {
            sink.lock().unwrap().push((item.id.clone(), *following));
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let db = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    playlog::attach_play_log(&p, db.clone());
    // Following plays the host's song alone, then the listener's own queue comes back.
    p.set_following(true);
    p.set_queue(items[..1].to_vec(), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || p.snapshot().status == PlayerStatus::Stopped), "the followed song never ended");
    p.set_following(false);
    p.set_queue(items[1..].to_vec(), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || following.lock().unwrap().len() == 2), "the own song never ended");
    assert_eq!(*following.lock().unwrap(), [("f".to_string(), true), ("own".to_string(), false)]);
    assert!(wait_for(Duration::from_secs(5), || playlog::events_since(&db, 0).unwrap().iter().filter(|e| e.ended.is_some()).count() == 2));
    let origins: Vec<(String, Origin)> = playlog::events_since(&db, 0).unwrap().into_iter().map(|e| (e.song_id, e.origin)).collect();
    assert_eq!(origins, [("f".to_string(), Origin::Along), ("own".to_string(), Origin::User)]);
    assert_eq!(playlog::song_stat(&db, "f").completions, 0, "not the listener's own");
    assert_eq!(playlog::song_stat(&db, "own").completions, 1);
}
