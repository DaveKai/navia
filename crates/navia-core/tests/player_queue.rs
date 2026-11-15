//! Queue edits and repeat while playing, against a local fake of `/stream` and an in-memory device.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::audio::{synth, to_f32, wav_bytes};
use common::http::{Faults, Library, TestServer};
use navia_core::audio::output::{CaptureBackend, CaptureBuffer};
use navia_core::audio::player::{OutputPolicy, Player, PlayerConfig, PlayerEvent, PlayerStatus, QueueItem, Repeat, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};

const RATE: u32 = 44_100;

fn items(ids: &[(&str, u64)]) -> Vec<QueueItem> {
    ids.iter().map(|(id, ms)| QueueItem::new(id.to_string(), Some(*ms))).collect()
}

fn player(url: &str, speed: f64, ring_frames: Option<usize>) -> (Player, Arc<CaptureBuffer>, Arc<Mutex<Vec<(usize, String)>>>) {
    let backend = CaptureBackend { speed, rates: (8_000, 192_000), ..Default::default() };
    let capture = backend.capture.clone();
    let client = Client::new(ClientConfig::new(url.to_string(), Credentials::new("t".to_string(), "t".to_string()))).unwrap();
    let mut config = PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() };
    if let Some(frames) = ring_frames {
        config.ring_frames = frames;
    }
    let player = Player::with_backend(Arc::new(ServerResolver::new(client).unwrap()), Box::new(backend), config).unwrap();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let sink = changes.clone();
    player.add_listener(move |e| {
        if let PlayerEvent::TrackChanged { index, item, .. } = e {
            sink.lock().unwrap().push((*index, item.id.clone()));
        }
    });
    (player, capture, changes)
}

fn server(tracks: &[(&str, &[i16])]) -> TestServer {
    let mut library = Library::default();
    for (id, pcm) in tracks {
        library.raw.insert(id.to_string(), wav_bytes(pcm, RATE));
    }
    TestServer::start(library, Faults::default())
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

fn audio_from_first(samples: &[f32]) -> &[f32] {
    let first = samples.chunks_exact(2).position(|f| f[0] != 0.0 || f[1] != 0.0).unwrap_or(0);
    &samples[first * 2..]
}

fn assert_exact(haystack: &[f32], needle: &[f32], what: &str) {
    assert!(haystack.len() >= needle.len(), "{what}: captured {} samples, expected at least {}", haystack.len(), needle.len());
    let bad = haystack.iter().zip(needle).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(bad, None, "{what}: differs at sample {bad:?} of {}", needle.len());
}

fn joined(parts: &[&[i16]]) -> Vec<f32> {
    parts.iter().flat_map(|p| to_f32(p)).collect()
}

fn ms(pcm: &[i16]) -> u64 {
    (pcm.len() / 2) as u64 * 1000 / u64::from(RATE)
}

/// Waits until the capture has heard this much audio, or panics.
fn wait_heard(capture: &CaptureBuffer, frames: usize) {
    assert!(
        wait_for(Duration::from_secs(10), || audio_from_first(&capture.samples()).len() >= frames * 2),
        "never heard {frames} frames"
    );
}

#[test]
fn moving_up_next_while_the_decoder_is_ahead_stays_gapless() {
    // The ring holds more than a whole song, so the decoder is well into the next one at once.
    let (a, b, c) = (synth(RATE as usize * 2 + 311, RATE, 0.3), synth(RATE as usize * 2 + 97, RATE, 1.1), synth(RATE as usize * 2 + 5, RATE, 2.4));
    let server = server(&[("a", &a), ("b", &b), ("c", &c)]);
    let (p, capture, changes) = player(&server.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b)), ("c", ms(&c))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    // Move c in front of b while a plays.
    p.move_items(vec![2], 1);
    p.sync();
    let ids: Vec<String> = p.snapshot().queue.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, ["a", "c", "b"]);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let samples = capture.samples();
    let audio = audio_from_first(&samples);
    let expected = joined(&[&a, &c, &b]);
    assert_exact(audio, &expected, "a then c then b, with no gap");
    assert!(audio[expected.len()..].iter().all(|s| *s == 0.0), "nothing after the queue");
    let order: Vec<String> = changes.lock().unwrap().iter().map(|(_, id)| id.clone()).collect();
    assert_eq!(order, ["a", "c", "b"], "track changes");
    assert_eq!(p.snapshot().underrun_frames, 0);
}

#[test]
fn moving_up_next_replaces_a_stale_preload() {
    // A short ring keeps the decoder on a, with b already preloading, when the move lands.
    let (a, b, c) = (synth(RATE as usize * 2 + 11, RATE, 0.7), synth(RATE as usize + 300, RATE, 1.9), synth(RATE as usize + 77, RATE, 2.9));
    let server = server(&[("a", &a), ("b", &b), ("c", &c)]);
    let (p, capture, _) = player(&server.url, 1.0, Some(RATE as usize / 2));
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b)), ("c", ms(&c))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    assert!(server.requests().iter().any(|r| r.id == "b"), "b preloads early, so the move makes it stale");
    p.move_items(vec![2], 1);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let samples = capture.samples();
    assert_exact(audio_from_first(&samples), &joined(&[&a, &c, &b]), "a then c then b");
    assert_eq!(p.snapshot().underrun_frames, 0, "c was preloaded in time");
}

#[test]
fn several_rows_move_together_and_the_playing_song_keeps_its_place() {
    let songs: Vec<Vec<i16>> = (0..5).map(|k| synth(RATE as usize / 2 + k * 31, RATE, 0.5 + k as f64)).collect();
    let tracks: Vec<(String, &[i16])> = songs.iter().enumerate().map(|(k, s)| (format!("s{k}"), s.as_slice())).collect();
    let named: Vec<(&str, &[i16])> = tracks.iter().map(|(id, s)| (id.as_str(), *s)).collect();
    let server = server(&named);
    let (p, capture, _) = player(&server.url, 1.0, None);
    let queue: Vec<(&str, u64)> = named.iter().map(|(id, s)| (*id, ms(s))).collect();
    p.set_queue(items(&queue), 0, 0, false);
    wait_heard(&capture, RATE as usize / 8);
    // s3 and s4 to the front of Up Next.
    p.move_items(vec![3, 4], 1);
    p.sync();
    let ids: Vec<String> = p.snapshot().queue.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, ["s0", "s3", "s4", "s1", "s2"]);
    assert_eq!(p.snapshot().index, Some(0), "the playing song stays first");
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let samples = capture.samples();
    assert_exact(audio_from_first(&samples), &joined(&[&songs[0], &songs[3], &songs[4], &songs[1], &songs[2]]), "moved order, gapless");
}

#[test]
fn repeat_one_replays_the_song_and_next_and_previous_still_move() {
    let (a, b) = (synth(RATE as usize + 123, RATE, 0.4), synth(RATE as usize + 45, RATE, 1.4));
    let server = server(&[("a", &a), ("b", &b)]);
    let (p, capture, changes) = player(&server.url, 1.0, None);
    p.set_repeat(Repeat::One);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b))]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || changes.lock().unwrap().len() >= 2), "a never played again");
    assert_eq!(changes.lock().unwrap()[..2], [(0, "a".to_string()), (0, "a".to_string())], "a again, not b");
    assert_eq!(p.snapshot().repeat, Repeat::One);
    wait_heard(&capture, a.len() + RATE as usize / 10);
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &a]), "a replays with no gap");

    // Next moves on even with repeat one, and switches to repeat all, saying so.
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = said.clone();
    p.add_listener(move |e| {
        if let PlayerEvent::RepeatChanged(r) = e {
            sink.lock().unwrap().push(*r);
        }
    });
    p.next();
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().item.as_ref().is_some_and(|i| i.id == "b")), "next didn't reach b");
    assert_eq!(p.snapshot().repeat, Repeat::All);
    assert_eq!(*said.lock().unwrap(), vec![Repeat::All]);
    // Previous at the start of the song goes back, and from the first song wraps to the last.
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().item.as_ref().is_some_and(|i| i.id == "a")), "previous didn't reach a");
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().index == Some(1)), "previous from the first song with repeat on wraps to the last");
    // At the end of the queue, Next with repeat one goes back to the start.
    p.next();
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().index == Some(0)), "next from the last song wraps with repeat one");
    p.stop();
}

#[test]
fn turning_repeat_one_on_late_replays_the_song_gaplessly() {
    // The decoder is already into b when repeat one is chosen, but a must play again, not b.
    let (a, b) = (synth(RATE as usize * 2 + 3, RATE, 0.9), synth(RATE as usize + 8, RATE, 2.0));
    let server = server(&[("a", &a), ("b", &b)]);
    let (p, capture, changes) = player(&server.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    p.set_repeat(Repeat::One);
    assert!(wait_for(Duration::from_secs(10), || changes.lock().unwrap().len() >= 2), "nothing followed a");
    // Then off again: after this run of a, b follows and the queue ends.
    p.set_repeat(Repeat::Off);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let order: Vec<String> = changes.lock().unwrap().iter().map(|(_, id)| id.clone()).collect();
    assert_eq!(order, ["a", "a", "b"]);
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &a, &b]), "a, a again, then b, with no gaps");
}

#[test]
fn repeat_all_wraps_and_off_ends() {
    let (a, b) = (synth(RATE as usize / 2, RATE, 0.1), synth(RATE as usize / 2 + 9, RATE, 1.3));
    let server = server(&[("a", &a), ("b", &b)]);
    let (p, _capture, changes) = player(&server.url, 2.0, None);
    p.set_repeat(Repeat::All);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b))]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || changes.lock().unwrap().len() >= 3), "repeat all didn't wrap");
    let order: Vec<String> = changes.lock().unwrap().iter().take(3).map(|(_, id)| id.clone()).collect();
    assert_eq!(order, ["a", "b", "a"]);
    p.set_repeat(Repeat::Off);
    assert!(wait_for(Duration::from_secs(10), || p.snapshot().status == PlayerStatus::Stopped), "off didn't end the queue");
}

#[test]
fn replacing_up_next_for_shuffle_stays_gapless_while_the_decoder_is_ahead() {
    // Turning shuffle on or off replaces everything after the song being heard.
    let (a, b, c) = (synth(RATE as usize * 2 + 41, RATE, 0.6), synth(RATE as usize + 10, RATE, 1.6), synth(RATE as usize + 3, RATE, 2.6));
    let server = server(&[("a", &a), ("b", &b), ("c", &c)]);
    let (p, capture, _) = player(&server.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b)), ("c", ms(&c))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    p.replace_upcoming(items(&[("c", ms(&c)), ("b", ms(&b))]));
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &c, &b]), "a, then the new order, with no gap");
    assert_eq!(p.snapshot().underrun_frames, 0);
}

/// Queued flags survive the engine, Clear Queue removes only the queued run, section moves flip a flag,
/// and the source shows in the snapshot.
#[test]
fn queued_flags_and_source_travel_with_the_queue() {
    use navia_core::queue_order::{QueueSource, SourceLink};
    let a = synth(RATE as usize * 2, RATE, 0.6);
    let srv = server(&[("a", &a), ("b", &a), ("c", &a), ("q1", &a), ("q2", &a)]);
    let (p, _capture, _) = player(&srv.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&a)), ("c", ms(&a))]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().index == Some(0)));
    let source = QueueSource { name: "Rumours".into(), radio: false, link: Some(SourceLink::Album("al-1".into())) };
    p.set_source(Some(source.clone()));
    p.insert_next(items(&[("q1", ms(&a)), ("q2", ms(&a))]).into_iter().map(|i| i.queued(true)).collect());
    p.sync();
    let snap = p.snapshot();
    let flags: Vec<(String, bool)> = snap.queue.iter().map(|i| (i.id.clone(), i.queued)).collect();
    assert_eq!(flags, vec![("a".into(), false), ("q1".into(), true), ("q2".into(), true), ("b".into(), false), ("c".into(), false)]);
    assert_eq!(p.queued_run(), 2);
    assert_eq!(snap.source, Some(source));
    // A section move puts q2 into the list.
    p.set_queued(vec![2], false);
    p.sync();
    assert_eq!(p.queued_run(), 1);
    p.set_queued(vec![2], true);
    p.clear_queued();
    p.sync();
    let ids: Vec<String> = p.snapshot().queue.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
    assert_eq!(p.queued_run(), 0);
    p.set_source(None);
    assert_eq!(p.snapshot().source, None);
}

/// Removing songs from Up Next while the decoder is already into the next one.
/// The removed audio is cut at the join and what follows plays with no gap, in list order.
#[test]
fn removing_several_songs_while_the_decoder_is_ahead_stays_gapless() {
    let (a, b, c, d) = (
        synth(RATE as usize * 2 + 311, RATE, 0.3),
        synth(RATE as usize / 2 + 97, RATE, 1.1),
        synth(RATE as usize / 2 + 5, RATE, 2.4),
        synth(RATE as usize + 63, RATE, 3.2),
    );
    let server = server(&[("a", &a), ("b", &b), ("c", &c), ("d", &d)]);
    let (p, capture, changes) = player(&server.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b)), ("c", ms(&c)), ("d", ms(&d))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    // b and c are decoded already since the ring holds more than both. Remove them while a plays.
    p.remove_items(vec![1, 2]);
    p.sync();
    let ids: Vec<String> = p.snapshot().queue.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, ["a", "d"]);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let samples = capture.samples();
    let audio = audio_from_first(&samples);
    let expected = joined(&[&a, &d]);
    assert_exact(audio, &expected, "a then d, with no gap");
    assert!(audio[expected.len()..].iter().all(|s| *s == 0.0), "nothing after the queue");
    let order: Vec<String> = changes.lock().unwrap().iter().map(|(_, id)| id.clone()).collect();
    assert_eq!(order, ["a", "d"], "track changes");
    assert_eq!(p.snapshot().underrun_frames, 0);
}

/// Removing the next song while it only preloads swaps the preload for the song after it.
#[test]
fn removing_the_next_song_replaces_a_stale_preload() {
    let (a, b, c) = (synth(RATE as usize * 2 + 11, RATE, 0.7), synth(RATE as usize + 300, RATE, 1.9), synth(RATE as usize + 77, RATE, 2.9));
    let server = server(&[("a", &a), ("b", &b), ("c", &c)]);
    let (p, capture, _) = player(&server.url, 1.0, Some(RATE as usize / 2));
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b)), ("c", ms(&c))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    assert!(server.requests().iter().any(|r| r.id == "b"), "b preloads early, so the removal makes it stale");
    p.remove_items(vec![1]);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &c]), "a then c");
    assert_eq!(p.snapshot().underrun_frames, 0, "c was preloaded in time");
}

/// The song being heard can be among the removed ones. The rest go and play moves on.
#[test]
fn removing_the_playing_song_with_others_moves_on() {
    let a = synth(RATE as usize * 3, RATE, 0.6);
    let srv = server(&[("a", &a), ("b", &a), ("c", &a), ("d", &a)]);
    let (p, _capture, changes) = player(&srv.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&a)), ("c", ms(&a)), ("d", ms(&a))]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().index == Some(0)));
    p.remove_items(vec![0, 1, 9]);
    assert!(wait_for(Duration::from_secs(5), || changes.lock().unwrap().last().is_some_and(|(_, id)| id == "c")), "moved on to c");
    let ids: Vec<String> = p.snapshot().queue.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, ["c", "d"]);
}

/// Track changes as (index, id, left).
type TrackLog = Arc<Mutex<Vec<(usize, String, Option<usize>)>>>;

/// Every track change, for the queue rules below.
fn track_log(p: &Player) -> TrackLog {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    p.add_listener(move |e| {
        if let PlayerEvent::TrackChanged { index, item, left } = e {
            sink.lock().unwrap().push((*index, item.id.clone(), left.map(|l| l.index)));
        }
    });
    log
}

fn queue_ids(p: &Player) -> Vec<String> {
    p.sync();
    p.snapshot().queue.iter().map(|i| i.id.clone()).collect()
}

fn queued(ids: &[(&str, u64, bool)]) -> Vec<QueueItem> {
    ids.iter().map(|(id, ms, q)| QueueItem::new(id.to_string(), Some(*ms)).queued(*q)).collect()
}

fn playing(p: &Player, id: &str) -> bool {
    p.snapshot().item.as_ref().is_some_and(|i| i.id == id)
}

/// A queued song leaves the queue once it has played, so Previous goes to the list song before it.
/// Removing it while the decoder is ahead changes nothing audible.
#[test]
fn a_queued_song_leaves_once_it_has_played() {
    let (a, q, b, c) = (synth(RATE as usize / 2 + 17, RATE, 0.3), synth(RATE as usize / 2 + 5, RATE, 1.2), synth(RATE as usize * 3 + 9, RATE, 2.1), synth(RATE as usize / 2, RATE, 2.9));
    let srv = server(&[("a", &a), ("q", &q), ("b", &b), ("c", &c)]);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    let log = track_log(&p);
    p.set_queue(queued(&[("a", ms(&a), false), ("q", ms(&q), true), ("b", ms(&b), false), ("c", ms(&c), false)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "b")), "never reached b");
    assert_eq!(queue_ids(&p), ["a", "b", "c"], "q played and left");
    assert_eq!(p.snapshot().index, Some(1));
    let log_now = log.lock().unwrap().clone();
    assert_eq!(log_now, vec![(0, "a".into(), None), (1, "q".into(), None), (1, "b".into(), Some(1))]);
    // a, q and the start of b, joined with no gap.
    wait_heard(&capture, (a.len() + q.len()) / 2 + RATE as usize / 4);
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &q, &b[..RATE as usize / 2]]), "a, q, b gapless");
    assert_eq!(p.snapshot().underrun_frames, 0);
    // Previous at the start of b goes to a, the list song before it, not back to q.
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "a")), "previous didn't reach a");
    assert_eq!(queue_ids(&p), ["a", "b", "c"], "list songs stay for Previous");
    p.stop();
}

/// Repeat one replays a queued song and keeps it. Next moves on and it leaves.
#[test]
fn repeat_one_keeps_a_queued_song_until_next() {
    let (a, q, b) = (synth(RATE as usize / 4, RATE, 0.5), synth(RATE as usize / 4 + 3, RATE, 1.5), synth(RATE as usize * 4, RATE, 2.5));
    let srv = server(&[("a", &a), ("q", &q), ("b", &b)]);
    let (p, _capture, _) = player(&srv.url, 1.0, None);
    let log = track_log(&p);
    p.set_repeat(Repeat::One);
    p.set_queue(queued(&[("a", ms(&a), false), ("q", ms(&q), true), ("b", ms(&b), false)]), 1, 0, false);
    assert!(wait_for(Duration::from_secs(10), || log.lock().unwrap().len() >= 2), "q never replayed");
    assert_eq!(log.lock().unwrap()[..2], [(1, "q".to_string(), None), (1, "q".to_string(), None)]);
    assert_eq!(queue_ids(&p), ["a", "q", "b"], "a replay isn't leaving");
    p.next();
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "b")), "next didn't reach b");
    assert_eq!(queue_ids(&p), ["a", "b"]);
    assert_eq!(log.lock().unwrap().last().cloned(), Some((1, "b".to_string(), Some(1))));
    p.stop();
}

/// When the queue ends on a queued song, it stays as the current song.
/// Once something else plays it leaves.
#[test]
fn a_queued_last_song_stays_until_something_else_plays() {
    let (a, q) = (synth(RATE as usize / 4, RATE, 0.5), synth(RATE as usize / 4 + 7, RATE, 1.7));
    let srv = server(&[("a", &a), ("q", &q)]);
    let (p, _capture, _) = player(&srv.url, 1.0, None);
    p.set_queue(queued(&[("a", ms(&a), false), ("q", ms(&q), true)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || p.snapshot().status == PlayerStatus::Stopped && playing(&p, "q")), "queue didn't end on q");
    assert_eq!(queue_ids(&p), ["a", "q"]);
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "a")), "previous didn't reach a");
    assert_eq!(queue_ids(&p), ["a"]);
    p.stop();
}

/// With repeat all, queued songs play once, then the list repeats without them.
#[test]
fn repeat_all_plays_queued_songs_once() {
    let (a, q, b) = (synth(RATE as usize / 4, RATE, 0.5), synth(RATE as usize / 4 + 3, RATE, 1.5), synth(RATE as usize / 4 + 9, RATE, 2.5));
    let srv = server(&[("a", &a), ("q", &q), ("b", &b)]);
    let (p, _capture, _) = player(&srv.url, 2.0, None);
    let log = track_log(&p);
    p.set_repeat(Repeat::All);
    p.set_queue(queued(&[("a", ms(&a), false), ("q", ms(&q), true), ("b", ms(&b), false)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || log.lock().unwrap().len() >= 5), "repeat all didn't wrap twice");
    p.stop();
    let order: Vec<String> = log.lock().unwrap().iter().take(5).map(|(_, id, _)| id.clone()).collect();
    assert_eq!(order, ["a", "q", "b", "a", "b"]);
    assert_eq!(queue_ids(&p), ["a", "b"]);
}

/// Double-clicking a queued song further down Next in Queue drops the queued songs above it.
/// After it, Previous goes back to the list.
#[test]
fn jumping_into_next_in_queue_drops_the_songs_passed_over() {
    use navia_core::queue_order::queue_jump;
    let long = synth(RATE as usize * 4, RATE, 0.6);
    let short = synth(RATE as usize / 4, RATE, 1.6);
    let srv = server(&[("a", &long), ("q1", &long), ("q2", &long), ("q3", &short), ("b", &long)]);
    let (p, _capture, _) = player(&srv.url, 1.0, None);
    p.set_queue(queued(&[("a", 4000, false), ("q1", 4000, true), ("q2", 4000, true), ("q3", ms(&short), true), ("b", 4000, false)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "a")));
    let flags: Vec<bool> = p.snapshot().queue.iter().map(|i| i.queued).collect();
    let jump = queue_jump(&flags, 0, 3);
    assert_eq!((jump.dropped.clone(), jump.at), (vec![1, 2], 1));
    p.remove_items(jump.dropped);
    p.jump_to(jump.at);
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "q3")), "q3 didn't play");
    assert_eq!(queue_ids(&p), ["a", "q3", "b"]);
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "b")), "b didn't follow q3");
    assert_eq!(queue_ids(&p), ["a", "b"]);
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "a")), "previous didn't go back to a");
    p.stop();
}

/// Starting a new list keeps the songs waiting in Next in Queue, and a later radio joins after them.
/// A queue restored paused on a queued song lets it leave too.
#[test]
fn a_new_list_and_its_radio_play_after_the_kept_queued_songs() {
    use navia_core::queue_order::{apply_order, start_keeping_queued};
    let tone = |k: usize| synth(RATE as usize / 4 + k * 13, RATE, 0.4 + k as f64);
    let songs: Vec<Vec<i16>> = (0..5).map(tone).collect();
    let names = ["s", "q1", "q2", "r1", "r2"];
    let named: Vec<(&str, &[i16])> = names.iter().zip(&songs).map(|(n, s)| (*n, s.as_slice())).collect();
    let srv = server(&named);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    let log = track_log(&p);
    // A Search song played with q1 and q2 waiting, so only the song itself is the list for now.
    let all = queued(&[("s", ms(&songs[0]), false), ("q1", ms(&songs[1]), true), ("q2", ms(&songs[2]), true)]);
    let order = start_keeping_queued(1, 0, 2);
    p.set_queue(apply_order(&all, &order), 0, 0, false);
    // Its radio arrives and goes after the queued songs.
    p.append(queued(&[("r1", ms(&songs[3]), false), ("r2", ms(&songs[4]), false)]));
    assert!(wait_for(Duration::from_secs(10), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let order: Vec<String> = log.lock().unwrap().iter().map(|(_, id, _)| id.clone()).collect();
    assert_eq!(order, ["s", "q1", "q2", "r1", "r2"]);
    assert_eq!(queue_ids(&p), ["s", "r1", "r2"], "the queued songs left as they played");
    let parts: Vec<&[i16]> = songs.iter().map(Vec::as_slice).collect();
    assert_exact(audio_from_first(&capture.samples()), &joined(&parts), "gapless all the way");

    // Restored paused on a queued song, which leaves once the next one plays.
    p.set_queue(queued(&[("s", ms(&songs[0]), false), ("q1", ms(&songs[1]), true), ("r1", ms(&songs[3]), false)]), 1, 0, true);
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "q1")));
    p.play();
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "r1")), "r1 didn't follow");
    assert_eq!(queue_ids(&p), ["s", "r1"]);
    p.stop();
}

/// Clear <source> while the decoder is already into the list. The queued song and Autoplay's song stay,
/// Undo puts the list back, and it all plays sample-exact with no gap.
#[test]
fn clearing_the_list_and_undoing_it_stays_gapless() {
    let (a, q, b, c, auto) = (
        synth(RATE as usize * 2 + 31, RATE, 0.4),
        synth(RATE as usize / 4 + 7, RATE, 1.2),
        synth(RATE as usize / 4 + 3, RATE, 2.0),
        synth(RATE as usize / 4 + 11, RATE, 2.8),
        synth(RATE as usize / 4 + 5, RATE, 3.6),
    );
    let srv = server(&[("a", &a), ("q", &q), ("b", &b), ("c", &c), ("auto", &auto)]);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    let all = queued(&[("a", ms(&a), false), ("q", ms(&q), true), ("b", ms(&b), false), ("c", ms(&c), false), ("auto", ms(&auto), false)]);
    p.set_queue(all.clone(), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    // The ring holds more than q, b and c, so they're decoded already when the list goes.
    p.clear_list(1);
    assert_eq!(queue_ids(&p), ["a", "q", "auto"], "the list left; the queued song and Autoplay's stay");
    // Undo puts the list back right after Next in Queue.
    p.insert_at(2, vec![all[2].clone(), all[3].clone()]);
    assert_eq!(queue_ids(&p), ["a", "q", "b", "c", "auto"]);
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    assert_exact(audio_from_first(&capture.samples()), &joined(&[&a, &q, &b, &c, &auto]), "everything, in place, gapless");
    assert_eq!(p.snapshot().underrun_frames, 0);
}

/// Clearing the list with nothing after it and nothing queued, while the decoder is ahead.
/// The song being heard plays out and the queue ends there.
#[test]
fn clearing_the_whole_list_ends_after_the_current_song() {
    let (a, b) = (synth(RATE as usize * 2 + 9, RATE, 0.7), synth(RATE as usize / 2, RATE, 1.7));
    let srv = server(&[("a", &a), ("b", &b)]);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    p.set_queue(items(&[("a", ms(&a)), ("b", ms(&b))]), 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    p.clear_list(0);
    assert_eq!(queue_ids(&p), ["a"]);
    p.clear_list(0);
    assert_eq!(queue_ids(&p), ["a"], "nothing left to clear");
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let samples = capture.samples();
    let audio = audio_from_first(&samples);
    assert_exact(audio, &to_f32(&a), "a alone");
    assert!(audio[a.len()..].iter().all(|s| *s == 0.0), "b's decoded audio was cut");
}

/// Shuffle Queue mixes the queued songs exactly as `shuffled_queued_order` does with the same seed,
/// and the new order plays with no gap.
#[test]
fn shuffle_queue_matches_the_shared_order_and_stays_gapless() {
    use navia_core::queue_order::{apply_order, shuffled_queued_order};
    let names = ["a", "q1", "q2", "q3", "b"];
    let songs: Vec<Vec<i16>> = (0..5).map(|k| synth(if k == 0 { RATE as usize * 2 } else { RATE as usize / 4 + k * 17 }, RATE, 0.3 + k as f64 * 0.7)).collect();
    let named: Vec<(&str, &[i16])> = names.iter().zip(&songs).map(|(n, s)| (*n, s.as_slice())).collect();
    let srv = server(&named);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    let flags = [false, true, true, true, false];
    let all: Vec<QueueItem> = names.iter().zip(&songs).zip(flags).map(|((n, s), f)| QueueItem::new(n.to_string(), Some(ms(s))).queued(f)).collect();
    p.set_queue(all, 0, 0, false);
    wait_heard(&capture, RATE as usize / 4);
    // The queued songs are decoded already, and the mix replaces them at the join.
    let seed = 42;
    let order = shuffled_queued_order(&flags, Some(0), seed).unwrap();
    p.shuffle_queued(seed);
    let expected: Vec<&str> = apply_order(&names, &order);
    assert_eq!(queue_ids(&p), expected);
    assert_eq!(p.queued_run(), 3, "still Next in Queue");
    assert!(wait_for(Duration::from_secs(20), || p.snapshot().status == PlayerStatus::Stopped), "queue never ended");
    let parts: Vec<&[i16]> = order.iter().map(|&i| songs[i].as_slice()).collect();
    assert_exact(audio_from_first(&capture.samples()), &joined(&parts), "the mixed order, gapless");
    assert_eq!(p.snapshot().underrun_frames, 0);
}

/// An edit the app made before it saw a queued song leave is rebased onto the song meant.
/// After `ack_left`, indexes count from the shorter queue again.
#[test]
fn edits_made_before_a_queued_song_left_land_on_the_songs_meant() {
    let short = synth(RATE as usize / 4, RATE, 0.4);
    let long = synth(RATE as usize * 4, RATE, 1.4);
    let srv = server(&[("q", &short), ("b", &long), ("c", &long), ("d", &long), ("e", &long), ("x", &long)]);
    let (p, _capture, _) = player(&srv.url, 1.0, None);
    let seqs = Arc::new(Mutex::new(Vec::new()));
    let sink = seqs.clone();
    p.add_listener(move |e| {
        if let PlayerEvent::TrackChanged { left: Some(l), .. } = e {
            sink.lock().unwrap().push(*l);
        }
    });
    // Playing the queued q, then the list b..e.
    p.set_queue(queued(&[("q", ms(&short), true), ("b", 4000, false), ("c", 4000, false), ("d", 4000, false), ("e", 4000, false)]), 0, 0, false);
    assert!(wait_for(Duration::from_secs(10), || playing(&p, "b")), "b never followed q");
    assert_eq!(queue_ids(&p), ["b", "c", "d", "e"], "q left");
    let left = seqs.lock().unwrap()[0];
    assert_eq!(left.index, 0);
    // The app still has [q, b, c, d, e] and removes d (3) and q (0), which is already gone.
    // Then in its [q, b, c, e] it moves e (3) before c (2) and inserts x before e.
    p.remove_items(vec![3, 0]);
    p.move_items(vec![3], 2);
    p.insert_at(2, queued(&[("x", 4000, false)]));
    assert_eq!(queue_ids(&p), ["b", "x", "e", "c"]);
    // Caught up, so indexes count from the queue as it is.
    p.ack_left(left.seq);
    p.remove_items(vec![1]);
    assert_eq!(queue_ids(&p), ["b", "e", "c"]);
    p.stop();
}

/// Previous with Repeat one. Moving back switches to repeat all, while restarting the same song keeps repeat one.
#[test]
fn previous_with_repeat_one_switches_only_when_it_moves() {
    let long = synth(RATE as usize * 6, RATE, 0.9);
    let srv = server(&[("a", &long), ("b", &long)]);
    let (p, capture, _) = player(&srv.url, 1.0, None);
    p.set_repeat(Repeat::One);
    p.set_queue(items(&[("a", 6000), ("b", 6000)]), 1, 0, false);
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "b")));
    wait_heard(&capture, RATE as usize * 7 / 2);
    // Past 3 s, Previous restarts b and stays on repeat one.
    p.previous();
    p.sync();
    assert!(playing(&p, "b"));
    assert_eq!(p.snapshot().repeat, Repeat::One);
    // At the start, Previous goes to a and switches to repeat all.
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().position_ms < 1000), "b didn't restart");
    p.previous();
    assert!(wait_for(Duration::from_secs(5), || playing(&p, "a")), "previous didn't reach a");
    assert_eq!(p.snapshot().repeat, Repeat::All);
    p.stop();
}
