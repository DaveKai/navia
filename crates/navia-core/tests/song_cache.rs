//! The song cache through the real player and streaming code, against a local fake of `/stream`.
//! Cached replays must be bit-identical, gapless and offline-capable, and nothing partial or stale may play.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::audio::{encode, have_ffmpeg, synth, to_f32, wav_bytes, write_wav};
use common::http::{Faults, Library as Files, TestServer};
use navia_core::audio::output::{CaptureBackend, CaptureBuffer};
use navia_core::audio::player::{OutputPolicy, Player, PlayerConfig, PlayerStatus, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};
use navia_core::downloads::{DownloadManager, DownloadRequest};
use navia_core::library::{Library, LibraryConfig};
use navia_core::quality::{Delivered, Quality};
use navia_core::songcache::{GB, SongCache};

fn client(url: &str) -> Client {
    Client::new(ClientConfig::new(url.to_string(), Credentials::new("t".to_string(), "t".to_string()))).unwrap()
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

struct Rig {
    server: TestServer,
    downloads: Arc<DownloadManager>,
    cache: Arc<SongCache>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new(files: Files, faults: Faults) -> Self {
        let server = TestServer::start(files, faults);
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        let downloads = Arc::new(DownloadManager::new(library, client(&server.url), dir.path().join("downloads")).unwrap());
        let cache = downloads.song_cache().expect("a song cache");
        // The test machine's real free space doesn't decide these tests.
        cache.set_space_probe(|_| None);
        Self { server, downloads, cache, _dir: dir }
    }

    fn player(&self, speed: f64) -> (Player, Arc<CaptureBuffer>) {
        let backend = CaptureBackend { speed, rates: (8_000, 192_000), ..Default::default() };
        let capture = backend.capture.clone();
        let resolver = ServerResolver::new(client(&self.server.url)).unwrap().with_downloads(self.downloads.clone());
        let player = Player::with_backend(Arc::new(resolver), Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
        (player, capture)
    }

    /// Plays a queue to its end and returns what was heard and every `delivered` seen.
    fn play(&self, ids: &[(&str, u64)]) -> (Vec<f32>, Vec<Delivered>) {
        let (player, capture) = self.player(25.0);
        player.set_queue(ids.iter().map(|(id, ms)| QueueItem::new(id.to_string(), Some(*ms))).collect(), 0, 0, false);
        player.sync();
        let mut seen = Vec::new();
        assert!(
            wait_for(Duration::from_secs(30), || {
                let snap = player.snapshot();
                if let Some(d) = snap.delivered.clone()
                    && seen.last() != Some(&d)
                {
                    seen.push(d);
                }
                snap.status == PlayerStatus::Stopped && snap.index.is_none_or(|i| i + 1 >= ids.len())
            }),
            "queue never finished: {:?}",
            player.snapshot()
        );
        assert_eq!(player.snapshot().underrun_frames, 0, "underruns");
        (capture.samples(), seen)
    }

    fn cached(&self, id: &str) -> bool {
        self.cache.lookup(id, Quality::Original, 0).is_some()
    }

    fn wait_cached(&self, id: &str) {
        assert!(wait_for(Duration::from_secs(10), || self.cached(id)), "{id} was never cached");
    }

    /// Requests made since `from` that fetched more than the one-byte freshness check.
    fn real_fetches_since(&self, from: usize) -> Vec<common::http::Req> {
        self.server.requests().into_iter().skip(from).filter(|r| r.range.as_deref() != Some("bytes=0-0")).collect()
    }
}

/// Drops the silence the capture recorded before the first track began.
fn audio_from_first(samples: &[f32]) -> &[f32] {
    let first = samples.chunks_exact(2).position(|f| f[0] != 0.0 || f[1] != 0.0).unwrap_or(0);
    &samples[first * 2..]
}

fn assert_exact(heard: &[f32], expected: &[f32], what: &str) {
    let audio = audio_from_first(heard);
    assert!(audio.len() >= expected.len(), "{what}: heard {} samples, expected {}", audio.len(), expected.len());
    let bad = audio.iter().zip(expected).position(|(a, b)| a.to_bits() != b.to_bits());
    assert_eq!(bad, None, "{what}: differs at sample {bad:?}");
    assert!(audio[expected.len()..].iter().all(|s| *s == 0.0), "{what}: sound after the queue ended");
}

fn wavs(songs: &[(&str, usize, f64)]) -> (Files, Vec<Vec<f32>>) {
    let mut files = Files::default();
    let mut pcm = Vec::new();
    for (id, frames, seed) in songs {
        let s = synth(*frames, 44_100, *seed);
        files.raw.insert(id.to_string(), wav_bytes(&s, 44_100));
        pcm.push(to_f32(&s));
    }
    (files, pcm)
}

#[test]
fn a_replay_from_the_cache_is_bit_identical_and_fetches_nothing() {
    let (files, pcm) = wavs(&[("a", 44_100 * 3 + 137, 0.2), ("b", 44_100 * 2 + 971, 1.7)]);
    let rig = Rig::new(files, Faults::default());
    let expected: Vec<f32> = pcm.concat();

    let (streamed, delivered) = rig.play(&[("a", 3003), ("b", 2022)]);
    assert_exact(&streamed, &expected, "streamed");
    assert!(delivered.iter().all(|d| *d == Delivered::Original), "{delivered:?}");
    rig.wait_cached("a");
    rig.wait_cached("b");

    let before = rig.server.requests().len();
    let (replayed, delivered) = rig.play(&[("a", 3003), ("b", 2022)]);
    assert_exact(&replayed, &expected, "from the cache");
    assert_eq!(audio_from_first(&streamed)[..expected.len()], audio_from_first(&replayed)[..expected.len()]);
    assert!(delivered.iter().all(|d| *d == Delivered::Cached { quality: None }), "{delivered:?}");
    let fetched = rig.real_fetches_since(before);
    assert!(fetched.is_empty(), "a cached replay fetched audio: {fetched:?}");
}

#[test]
fn flac_from_the_cache_matches_the_stream_and_joins_streamed_songs_gaplessly() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut files = Files::default();
    let mut pcm = Vec::new();
    for (i, seed) in [0.5, 1.1, 2.3].iter().enumerate() {
        let s = synth(44_100 * 3 + i * 911, 44_100, *seed);
        let wav = dir.path().join(format!("{i}.wav"));
        write_wav(&wav, &s, 44_100);
        let flac = dir.path().join(format!("{i}.flac"));
        encode(&wav, &flac, &["-c:a", "flac"]);
        files.raw.insert(format!("s{i}"), std::fs::read(&flac).unwrap());
        pcm.push(to_f32(&s));
    }
    let rig = Rig::new(files, Faults::default());
    // Only the middle song is cached.
    let (heard, _) = rig.play(&[("s1", 3021)]);
    assert_exact(&heard, &pcm[1], "s1 streamed");
    rig.wait_cached("s1");

    // Streamed into cached into streamed, gaplessly and bit for bit.
    let (heard, delivered) = rig.play(&[("s0", 3000), ("s1", 3021), ("s2", 3041)]);
    assert_exact(&heard, &pcm.concat(), "streamed, cached, streamed");
    assert!(delivered.contains(&Delivered::Cached { quality: None }) && delivered.contains(&Delivered::Original), "{delivered:?}");

    // All three are cached now, and the whole album from disk is the same audio.
    rig.wait_cached("s0");
    rig.wait_cached("s2");
    let (heard, delivered) = rig.play(&[("s0", 3000), ("s1", 3021), ("s2", 3041)]);
    assert_exact(&heard, &pcm.concat(), "all cached");
    assert!(delivered.iter().all(|d| *d == Delivered::Cached { quality: None }), "{delivered:?}");
}

#[test]
fn cached_songs_play_while_the_server_is_down() {
    let (files, pcm) = wavs(&[("a", 44_100 * 2 + 5, 0.9)]);
    let rig = Rig::new(files, Faults::default());
    rig.play(&[("a", 2000)]);
    rig.wait_cached("a");
    rig.server.set_down(true);
    let (heard, delivered) = rig.play(&[("a", 2000)]);
    assert_exact(&heard, &pcm[0], "offline");
    assert_eq!(delivered, vec![Delivered::Cached { quality: None }]);
}

#[test]
fn a_song_the_user_seeked_through_is_completed_unless_metered() {
    let (files, _) = wavs(&[("a", 44_100 * 40, 0.4)]);
    let source = files.raw["a"].clone();
    for metered in [false, true] {
        let mut files = Files::default();
        files.raw.insert("a".into(), source.clone());
        // 7 MB at 3 MB/s, so the seek lands well before the file has arrived, leaving a hole.
        let rig = Rig::new(files, Faults { throttle: 3_000_000, ..Default::default() });
        rig.downloads.quality_control().set_metered(metered);
        let (player, _) = rig.player(1.0);
        player.set_queue(vec![QueueItem::new("a".into(), Some(40_000))], 0, 0, false);
        player.sync();
        assert!(wait_for(Duration::from_secs(10), || player.snapshot().status == PlayerStatus::Playing));
        // Straight to the last part, so the head of the file is never read.
        player.seek_ms(30_000);
        assert!(wait_for(Duration::from_secs(20), || player.snapshot().status == PlayerStatus::Stopped), "never finished");
        if metered {
            std::thread::sleep(Duration::from_millis(300));
            assert!(!rig.cached("a"), "a partial stream was completed on a metered connection");
        } else {
            rig.wait_cached("a");
            let hit = rig.cache.lookup("a", Quality::Original, 0).unwrap();
            assert_eq!(std::fs::read(hit.path).unwrap(), source, "the completed copy isn't the file");
        }
        drop(player);
    }
}

#[test]
fn a_skipped_song_leaves_nothing_behind() {
    let (files, _) = wavs(&[("a", 44_100 * 30, 0.4), ("b", 44_100, 1.2)]);
    // Slow enough that the first song can't finish arriving before it's skipped.
    let rig = Rig::new(files, Faults { throttle: 400_000, ..Default::default() });
    let (player, _) = rig.player(1.0);
    player.set_queue(vec![QueueItem::new("a".into(), Some(30_000)), QueueItem::new("b".into(), Some(1_000))], 0, 0, false);
    player.sync();
    assert!(wait_for(Duration::from_secs(10), || player.snapshot().status == PlayerStatus::Playing));
    player.next();
    assert!(wait_for(Duration::from_secs(20), || player.snapshot().status == PlayerStatus::Stopped));
    rig.wait_cached("b");
    std::thread::sleep(Duration::from_millis(500));
    assert!(!rig.cached("a"), "a skipped, partial song was cached");
    assert_eq!(rig.cache.usage().1, 1);
    // Nothing half-written anywhere in the cache.
    let mut stray = Vec::new();
    for sub in std::fs::read_dir(rig.cache.dir()).unwrap().flatten().filter(|e| e.path().is_dir()) {
        stray.extend(std::fs::read_dir(sub.path()).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "part")));
    }
    assert!(stray.is_empty(), "{stray:?}");
}

#[test]
fn the_limit_evicts_the_least_recently_played() {
    let (files, _) = wavs(&[("a", 44_100, 0.1), ("b", 44_100, 0.2), ("c", 44_100, 0.3)]);
    let size = files.raw["a"].len() as u64;
    let rig = Rig::new(files, Faults::default());
    rig.cache.set_limit(size * 2).unwrap();
    rig.play(&[("a", 1000)]);
    rig.wait_cached("a");
    rig.play(&[("b", 1000)]);
    rig.wait_cached("b");
    // Playing "a" again from the cache makes "b" the least recently played.
    let (_, delivered) = rig.play(&[("a", 1000)]);
    assert_eq!(delivered, vec![Delivered::Cached { quality: None }]);
    rig.play(&[("c", 1000)]);
    rig.wait_cached("c");
    assert!(rig.cached("a") && !rig.cached("b"), "wrong song evicted");
    assert!(rig.cache.usage().0 <= size * 2);
    // Off empties it and keeps nothing new.
    rig.cache.set_limit(0).unwrap();
    assert_eq!(rig.cache.usage(), (0, 0));
    rig.play(&[("b", 1000)]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.cache.usage(), (0, 0));
}

#[test]
fn cached_songs_list_newest_first_and_remove_one_by_one() {
    let (files, _) = wavs(&[("a", 44_100, 0.1), ("b", 44_100, 0.2), ("c", 44_100, 0.3)]);
    let size = files.raw["a"].len() as u64;
    let rig = Rig::new(files, Faults::default());
    for id in ["a", "b", "c"] {
        rig.play(&[(id, 1000)]);
        rig.wait_cached(id);
    }
    // A replay from the cache counts as the most recent.
    rig.play(&[("a", 1000)]);
    let order: Vec<String> = rig.cache.songs().into_iter().map(|s| s.song_id).collect();
    assert_eq!(order, ["a", "c", "b"]);
    assert!(rig.cache.songs().iter().all(|s| s.bytes == size && s.copies == 1));
    assert_eq!(rig.cache.remove_songs(&["c".into(), "missing".into()]), size);
    assert!(!rig.cached("c") && rig.cached("a") && rig.cached("b"));
    assert_eq!(rig.cache.usage(), (size * 2, 2));
}

#[test]
fn a_file_changed_on_the_server_is_never_played_stale() {
    let (mut files, _) = wavs(&[("a", 44_100 * 2, 0.6)]);
    files.stamps.insert("a".into(), "Mon, 01 Jan 2024 00:00:00 GMT".into());
    let rig = Rig::new(files, Faults::default());
    rig.play(&[("a", 2000)]);
    rig.wait_cached("a");
    assert_eq!(rig.cache.lookup("a", Quality::Original, 0).unwrap().stamp.as_deref(), Some("Mon, 01 Jan 2024 00:00:00 GMT"));
    // Re-tagged in place, with the same length, new content and a new date.
    let new = synth(44_100 * 2, 44_100, 3.3);
    {
        let mut lib = rig.server.state.library.lock().unwrap();
        lib.raw.insert("a".into(), wav_bytes(&new, 44_100));
        lib.stamps.insert("a".into(), "Tue, 02 Jan 2024 00:00:00 GMT".into());
    }
    let (heard, delivered) = rig.play(&[("a", 2000)]);
    assert_exact(&heard, &to_f32(&new), "after the change");
    assert_eq!(delivered, vec![Delivered::Original]);
    assert!(wait_for(Duration::from_secs(10), || rig.cache.lookup("a", Quality::Original, 0).is_some_and(|h| h.stamp.as_deref() == Some("Tue, 02 Jan 2024 00:00:00 GMT"))), "the new file wasn't cached");
}

#[test]
fn a_damaged_cached_file_is_dropped_and_streamed_instead() {
    let (files, pcm) = wavs(&[("a", 44_100 * 2, 0.6)]);
    let rig = Rig::new(files, Faults::default());
    rig.play(&[("a", 2000)]);
    rig.wait_cached("a");
    let path = rig.cache.lookup("a", Quality::Original, 0).unwrap().path;
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() / 3);
    std::fs::write(&path, bytes).unwrap();
    let (heard, delivered) = rig.play(&[("a", 2000)]);
    assert_exact(&heard, &pcm[0], "after damage");
    assert_eq!(delivered, vec![Delivered::Original]);
}

#[test]
fn downloading_a_cached_song_moves_it_instead_of_fetching_again() {
    let (files, _) = wavs(&[("a", 44_100 * 2, 0.6)]);
    let source = files.raw["a"].clone();
    let rig = Rig::new(files, Faults::default());
    rig.play(&[("a", 2000)]);
    rig.wait_cached("a");
    let before = rig.server.requests().len();
    rig.downloads.enqueue(vec![DownloadRequest { id: "a".into(), suffix: Some("wav".into()), size: None }]);
    assert!(wait_for(Duration::from_secs(10), || rig.downloads.local_path("a").is_some()), "download never finished");
    assert_eq!(std::fs::read(rig.downloads.local_path("a").unwrap()).unwrap(), source);
    assert_eq!(rig.server.requests().len(), before, "the download fetched the song again");
    assert_eq!(rig.cache.usage(), (0, 0), "stored twice");
    // Downloads don't count toward the cache, and they play first.
    let (_, delivered) = rig.play(&[("a", 2000)]);
    assert_eq!(delivered, vec![Delivered::Downloaded { quality: None }]);
    assert_eq!(rig.cache.usage(), (0, 0));
}

#[test]
fn a_nearly_full_disk_stops_caching_but_not_playing() {
    let (files, pcm) = wavs(&[("a", 44_100 * 2, 0.6)]);
    let rig = Rig::new(files, Faults::default());
    rig.cache.set_space_probe(|_| Some((GB / 2, 100 * GB)));
    let (heard, _) = rig.play(&[("a", 2000)]);
    assert_exact(&heard, &pcm[0], "with a full disk");
    assert!(wait_for(Duration::from_secs(5), || rig.cache.low_disk()), "no note that caching stopped");
    assert!(!rig.cached("a"));
    rig.cache.set_space_probe(|_| None);
    rig.play(&[("a", 2000)]);
    rig.wait_cached("a");
    assert!(!rig.cache.low_disk());
}

/// The same song as raw WAV plus server transcodes, for a transcode test.
fn transcoded(ids: &[(&str, f64)], format: &str, args: &[&str], ext: &str) -> Files {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Files::default();
    for (i, (id, seed)) in ids.iter().enumerate() {
        let s = synth(44_100 * 2 + i * 777, 44_100, *seed);
        let wav = dir.path().join(format!("{id}.wav"));
        write_wav(&wav, &s, 44_100);
        let out = dir.path().join(format!("{id}.{ext}"));
        encode(&wav, &out, args);
        files.raw.insert(id.to_string(), wav_bytes(&s, 44_100));
        files.transcoded.insert((id.to_string(), format.to_string()), std::fs::read(&out).unwrap());
    }
    files
}

#[test]
fn transcodes_from_the_cache_are_bit_identical_to_the_stream_and_join_the_same() {
    if !have_ffmpeg() {
        return;
    }
    use navia_core::quality::Codec;
    // MP3 as a pipe sends it without a Xing header and as a file would have it, and Opus.
    let mut cases = vec![
        (Codec::Mp3, 192, transcoded(&[("t0", 0.3), ("t1", 1.4)], "mp3", &["-c:a", "libmp3lame", "-b:a", "192k", "-write_xing", "0"], "mp3")),
        (Codec::Mp3, 192, transcoded(&[("t0", 0.3), ("t1", 1.4)], "mp3", &["-c:a", "libmp3lame", "-b:a", "192k"], "mp3")),
    ];
    if cfg!(feature = "opus") {
        cases.push((Codec::Opus, 128, transcoded(&[("t0", 0.3), ("t1", 1.4)], "opus", &["-c:a", "libopus", "-b:a", "128k", "-f", "ogg"], "ogg")));
    }
    for (codec, kbps, files) in cases {
        let rig = Rig::new(files, Faults::default());
        let quality = Quality::Transcode { codec, kbps };
        rig.downloads.quality_control().set_streaming(quality).unwrap();
        let queue = [("t0", 2000), ("t1", 2017)];
        let (streamed, delivered) = rig.play(&queue);
        assert!(delivered.iter().all(|d| *d == Delivered::Transcoded { codec, kbps }), "{delivered:?}");
        for id in ["t0", "t1"] {
            assert!(wait_for(Duration::from_secs(10), || rig.cache.lookup(id, quality, 0).is_some()), "{id} as {codec:?} wasn't cached");
        }
        let (cached, delivered) = rig.play(&queue);
        assert!(delivered.iter().all(|d| *d == Delivered::Cached { quality: Some((codec, kbps)) }), "{delivered:?}");
        let (a, b) = (audio_from_first(&streamed), audio_from_first(&cached));
        assert!(a.len() > 44_100 * 4 * 2, "{codec:?}: too little audio");
        let len = a.len().min(b.len());
        let bad = a[..len].iter().zip(&b[..len]).position(|(x, y)| x.to_bits() != y.to_bits());
        assert_eq!(bad, None, "{codec:?}: cached play differs from the stream at sample {bad:?}");
        assert!(a[len..].iter().chain(&b[len..]).all(|s| *s == 0.0), "{codec:?}: lengths differ: {} vs {}", a.len(), b.len());
        // A cached first song into a streamed second gives the same join as streaming both.
        rig.cache.remove("t1", quality);
        let (mixed, _) = rig.play(&queue);
        let m = audio_from_first(&mixed);
        let len = a.len().min(m.len());
        let bad = a[..len].iter().zip(&m[..len]).position(|(x, y)| x.to_bits() != y.to_bits());
        assert_eq!(bad, None, "{codec:?}: cached-then-streamed join differs at sample {bad:?}");
        // Downloading the cached first song moves it over, and it still plays the same.
        rig.downloads.quality_control().set_download(quality).unwrap();
        let before = rig.server.requests().len();
        rig.downloads.enqueue(vec![DownloadRequest { id: "t0".into(), suffix: Some("wav".into()), size: None }]);
        assert!(wait_for(Duration::from_secs(10), || rig.downloads.local_path("t0").is_some()), "download never finished");
        assert!(rig.real_fetches_since(before).iter().all(|r| r.id != "t0"), "{codec:?}: the download fetched it again");
        let (downloaded, delivered) = rig.play(&queue);
        assert_eq!(delivered.first(), Some(&Delivered::Downloaded { quality: Some((codec, kbps)) }));
        let d = audio_from_first(&downloaded);
        let len = a.len().min(d.len());
        let bad = a[..len].iter().zip(&d[..len]).position(|(x, y)| x.to_bits() != y.to_bits());
        assert_eq!(bad, None, "{codec:?}: downloaded-then-streamed differs at sample {bad:?}");
    }
}
