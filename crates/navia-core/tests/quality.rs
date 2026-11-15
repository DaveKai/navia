//! Streaming and download quality against a local fake of Navidrome, into an in-memory capture device.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::audio::{encode, have_ffmpeg, synth, wav_bytes, write_wav};
use common::http::{FakeExtension, Faults, Library as Files, TestServer};
use navia_core::audio::output::CaptureBackend;
use navia_core::audio::player::{OutputPolicy, Player, PlayerConfig, PlayerStatus, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};
use navia_core::downloads::{DownloadManager, DownloadRequest};
use navia_core::library::{Library, LibraryConfig};
use navia_core::quality::{Codec, Delivered, Leveling, Quality, QualityControl};

fn client(url: &str) -> Client {
    Client::new(ClientConfig::new(url.to_string(), Credentials::new("t".to_string(), "t".to_string()))).unwrap()
}

fn wait_for(limit: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn wait_status(p: &Player, status: PlayerStatus, limit: Duration) {
    assert!(wait_for(limit, || p.snapshot().status == status), "wanted {status:?}, got {:?}", p.snapshot().status);
}

struct Rig {
    _dir: tempfile::TempDir,
    server: TestServer,
    quality: Arc<QualityControl>,
    player: Player,
}

fn rig(files: Files, faults: Faults, extension: Option<FakeExtension>) -> Rig {
    let server = TestServer::start(files, faults);
    *server.state.extension.lock().unwrap() = extension;
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    let quality = QualityControl::new(library, client(&server.url), None).unwrap();
    let resolver = Arc::new(ServerResolver::new(client(&server.url)).unwrap().with_quality(quality.clone()));
    let backend = CaptureBackend { speed: 8.0, rates: (8_000, 192_000), ..Default::default() };
    let player = Player::with_backend(resolver, Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
    Rig { _dir: dir, server, quality, player }
}

fn mp3(dir: &std::path::Path, name: &str, seconds: usize) -> Vec<u8> {
    let wav = dir.join(format!("{name}.wav"));
    write_wav(&wav, &synth(44_100 * seconds, 44_100, 0.5), 44_100);
    let out = dir.join(format!("{name}.mp3"));
    encode(&wav, &out, &["-c:a", "libmp3lame", "-b:a", "128k"]);
    std::fs::read(&out).unwrap()
}

fn file_with_transcode(ids: &[&str]) -> Files {
    files_of_length(ids, 2)
}

fn files_of_length(ids: &[&str], seconds: usize) -> Files {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Files::default();
    for id in ids {
        files.raw.insert((*id).into(), wav_bytes(&synth(44_100 * seconds, 44_100, 0.3), 44_100));
        files.transcoded.insert(((*id).into(), "mp3".into()), mp3(dir.path(), id, seconds));
    }
    files
}

fn play(p: &Player, ids: &[&str]) {
    let items = ids.iter().map(|id| QueueItem::new((*id).to_string(), Some(2000))).collect();
    p.set_queue(items, 0, 0, false);
}

#[test]
fn original_streams_the_file_untouched() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(file_with_transcode(&["a"]), Faults::default(), None);
    assert_eq!(rig.quality.streaming(), Quality::Original);
    play(&rig.player, &["a"]);
    wait_status(&rig.player, PlayerStatus::Stopped, Duration::from_secs(15));
    let reqs = rig.server.requests();
    assert!(reqs.iter().all(|r| r.format.as_deref() == Some("raw") && r.max_bit_rate.is_none()), "{reqs:?}");
    assert!(matches!(rig.player.snapshot().delivered, None | Some(Delivered::Original)));
}

#[test]
fn a_chosen_format_is_requested_and_reported() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(file_with_transcode(&["a"]), Faults::default(), None);
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    play(&rig.player, &["a"]);
    assert!(wait_for(Duration::from_secs(10), || rig.player.snapshot().delivered == Some(Delivered::Transcoded { codec: Codec::Mp3, kbps: 192 })), "delivered {:?}", rig.player.snapshot().delivered);
    let reqs = rig.server.requests();
    assert!(reqs.iter().any(|r| r.format.as_deref() == Some("mp3") && r.max_bit_rate == Some(192)), "{reqs:?}");
    rig.player.stop();
}

#[test]
fn a_format_the_server_cannot_make_falls_back_to_the_original_and_keeps_playing() {
    let mut files = Files::default();
    files.raw.insert("a".into(), wav_bytes(&synth(44_100 * 2, 44_100, 0.3), 44_100));
    // No "opus" entry, so the fake answers 404 for it.
    let rig = rig(files, Faults::default(), None);
    if !Codec::Opus.decodable() {
        return;
    }
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Opus, kbps: 128 }).unwrap();
    play(&rig.player, &["a"]);
    assert!(wait_for(Duration::from_secs(10), || matches!(rig.player.snapshot().delivered, Some(Delivered::Fallback { .. }))), "delivered {:?}", rig.player.snapshot().delivered);
    assert!(rig.player.snapshot().delivered.unwrap().fallback_reason().is_some());
    assert!(rig.server.requests().iter().any(|r| r.format.as_deref() == Some("raw")));
    rig.player.stop();
}

#[test]
fn a_server_substituting_another_format_is_not_believed() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(file_with_transcode(&["a"]), Faults { transcode_mime: Some("audio/ogg"), ..Default::default() }, None);
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    play(&rig.player, &["a"]);
    assert!(wait_for(Duration::from_secs(10), || matches!(rig.player.snapshot().delivered, Some(Delivered::Fallback { .. }))));
    rig.player.stop();
}

#[test]
fn the_extension_is_used_when_the_server_has_it() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(files_of_length(&["a"], 20), Faults::default(), Some(FakeExtension { can_direct_play: false, can_transcode: true }));
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    play(&rig.player, &["a"]);
    wait_status(&rig.player, PlayerStatus::Playing, Duration::from_secs(10));
    rig.player.seek_ms(1000);
    assert!(wait_for(Duration::from_secs(10), || rig.server.requests().iter().any(|r| r.endpoint == "getTranscodeStream" && r.time_offset == Some(1))), "{:?}", rig.server.requests());
    assert_eq!(rig.server.state.decisions.lock().unwrap().len(), 1, "the decision is reused for seeks");
    let decision = rig.server.state.decisions.lock().unwrap()[0].clone();
    assert!(decision.contains(r#""maxAudioBitrate":192000"#), "{decision}");
    rig.player.stop();
}

#[test]
fn the_extension_saying_direct_play_streams_the_original() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(file_with_transcode(&["a"]), Faults::default(), Some(FakeExtension { can_direct_play: true, can_transcode: false }));
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 320 }).unwrap();
    play(&rig.player, &["a"]);
    wait_status(&rig.player, PlayerStatus::Stopped, Duration::from_secs(15));
    assert!(rig.server.requests().iter().all(|r| r.format.as_deref() == Some("raw")), "{:?}", rig.server.requests());
}

#[test]
fn the_extension_refusing_falls_back_to_stream() {
    if !have_ffmpeg() {
        return;
    }
    let rig = rig(file_with_transcode(&["a", "b"]), Faults::default(), Some(FakeExtension { can_direct_play: false, can_transcode: false }));
    rig.quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    play(&rig.player, &["a", "b"]);
    assert!(wait_for(Duration::from_secs(20), || rig.server.requests().iter().any(|r| r.id == "b" && r.endpoint == "stream" && r.format.as_deref() == Some("mp3"))), "{:?}", rig.server.requests());
    // Remembered, so the second song doesn't ask the extension again.
    assert_eq!(rig.server.state.decisions.lock().unwrap().len(), 1);
    rig.player.stop();
}

#[test]
fn a_metered_connection_changes_the_next_track_and_not_the_one_playing() {
    if !have_ffmpeg() {
        return;
    }
    // Long enough that the next track isn't preloaded yet when the signal changes.
    let rig = rig(files_of_length(&["a", "b"], 40), Faults::default(), None);
    rig.quality.set_streaming(Quality::Original).unwrap();
    rig.quality.set_metered_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 128 }).unwrap();
    rig.quality.set_metered(false);
    let items = vec![QueueItem::new("a".into(), Some(40_000)), QueueItem::new("b".into(), Some(40_000))];
    rig.player.set_queue(items, 0, 0, false);
    wait_status(&rig.player, PlayerStatus::Playing, Duration::from_secs(10));
    assert!(wait_for(Duration::from_secs(5), || rig.server.requests().iter().any(|r| r.id == "a")), "first song requested");
    rig.quality.set_metered(true);
    wait_status(&rig.player, PlayerStatus::Stopped, Duration::from_secs(20));
    let reqs = rig.server.requests();
    assert!(reqs.iter().filter(|r| r.id == "a").all(|r| r.format.as_deref() == Some("raw")), "playing song restarted or changed: {reqs:?}");
    assert!(reqs.iter().any(|r| r.id == "b" && r.format.as_deref() == Some("mp3") && r.max_bit_rate == Some(128)), "next song not metered: {reqs:?}");
}

#[test]
fn downloads_record_what_they_were_made_as_and_ignore_metered() {
    if !have_ffmpeg() {
        return;
    }
    let server = TestServer::start(file_with_transcode(&["a"]), Faults::default());
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    let manager = Arc::new(DownloadManager::new(library, client(&server.url), dir.path().join("downloads")).unwrap());
    manager.quality_control().set_download(Quality::Transcode { codec: Codec::Mp3, kbps: 128 }).unwrap();
    manager.quality_control().set_metered(true);
    manager.quality_control().set_metered_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 320 }).unwrap();
    manager.enqueue(vec![DownloadRequest { id: "a".to_string(), suffix: Some("flac".to_string()), size: None }]);
    assert!(wait_for(Duration::from_secs(15), || manager.local_path("a").is_some()), "download never finished");
    assert_eq!(manager.delivered_quality("a"), Some((Codec::Mp3, 128)));
    let path = manager.local_path("a").unwrap();
    assert_eq!(path.extension().unwrap(), "mp3");
    let req = server.requests();
    assert!(req.iter().all(|r| r.format.as_deref() == Some("mp3") && r.max_bit_rate == Some(128)), "{req:?}");
}

#[test]
fn a_finished_transcode_is_replayed_from_the_cache_keyed_by_rate() {
    if !have_ffmpeg() {
        return;
    }
    let server = TestServer::start(file_with_transcode(&["a"]), Faults::default());
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    let quality = QualityControl::new(library, client(&server.url), Some(dir.path().join("cache"))).unwrap();
    quality.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    let resolver = Arc::new(ServerResolver::new(client(&server.url)).unwrap().with_quality(quality.clone()));
    let backend = CaptureBackend { speed: 8.0, rates: (8_000, 192_000), ..Default::default() };
    let player = Player::with_backend(resolver, Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
    play(&player, &["a"]);
    wait_status(&player, PlayerStatus::Stopped, Duration::from_secs(15));
    let count = server.requests().len();
    let mp3_192 = Quality::Transcode { codec: Codec::Mp3, kbps: 192 };
    assert!(wait_for(Duration::from_secs(5), || quality.cache().unwrap().lookup("a", mp3_192, 0).is_some()), "the transcode wasn't kept");
    play(&player, &["a"]);
    let mut from_cache = false;
    assert!(wait_for(Duration::from_secs(5), || {
        from_cache |= player.snapshot().delivered == Some(Delivered::Cached { quality: Some((Codec::Mp3, 192)) });
        from_cache && player.snapshot().status == PlayerStatus::Stopped
    }), "replay didn't come from the cache: {:?}", player.snapshot().delivered);
    // At most a one-byte look at the original to see it hasn't changed, and nothing transcoded again.
    let later: Vec<_> = server.requests().into_iter().skip(count).collect();
    assert!(later.iter().all(|r| r.format.as_deref() == Some("raw")), "replay asked for a transcode again: {later:?}");
    // A different rate doesn't reuse it.
    assert!(quality.cache().unwrap().lookup("a", Quality::Transcode { codec: Codec::Mp3, kbps: 128 }, 0).is_none());
}

/// Puts a song straight into the library, as a sync would have.
fn insert_song(path: &std::path::Path, id: &str, extra: serde_json::Value) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    let mut data = serde_json::json!({ "id": id, "title": id });
    data.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    conn.execute(
        "INSERT INTO song(id, library_id, title, sort_name, album_id, artist_id, track, disc, year, duration, created, play_count, played, starred, generation, content_hash, data) VALUES (?1,'l',?1,?1,'a','a',1,1,0,0,'',0,NULL,NULL,0,'h',?2)",
        rusqlite::params![id, data.to_string()],
    )
    .unwrap();
}

/// Two tracks into a capture device, with library rows carrying the given fields.
fn leveling_rig(tracks: &[(&str, serde_json::Value, Vec<i16>)], mode: Leveling) -> (TestServer, Arc<QualityControl>, Player, Arc<navia_core::audio::output::CaptureBuffer>, tempfile::TempDir) {
    let mut files = Files::default();
    for (id, _, pcm) in tracks {
        files.raw.insert((*id).into(), wav_bytes(pcm, 44_100));
    }
    let server = TestServer::start(files, Faults::default());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.sqlite");
    let library = Library::open(&LibraryConfig { path: path.clone() }).unwrap();
    for (id, extra, _) in tracks {
        insert_song(&path, id, extra.clone());
    }
    let quality = QualityControl::new(library, client(&server.url), None).unwrap();
    quality.set_leveling(mode).unwrap();
    let resolver = Arc::new(ServerResolver::new(client(&server.url)).unwrap().with_quality(quality.clone()));
    let backend = CaptureBackend { speed: 8.0, rates: (8_000, 192_000), ..Default::default() };
    let capture = backend.capture.clone();
    let player = Player::with_backend(resolver, Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
    (server, quality, player, capture, dir)
}

fn rg(track: f64, track_peak: f64) -> serde_json::Value {
    serde_json::json!({ "replayGain": { "trackGain": track, "trackPeak": track_peak } })
}

#[test]
fn leveling_off_is_bit_exact_and_leveling_applies_per_track_at_the_join() {
    let (a, b) = (synth(44_100, 44_100, 0.4), synth(44_100, 44_100, 0.7));
    let tracks = vec![("a", rg(-6.0, 0.5), a.clone()), ("b", rg(-12.0, 0.5), b.clone())];

    let (_s, _q, p, capture, _d) = leveling_rig(&tracks, Leveling::Off);
    p.set_queue(vec![QueueItem::new("a".into(), Some(1000)), QueueItem::new("b".into(), Some(1000))], 0, 0, false);
    wait_status(&p, PlayerStatus::Stopped, Duration::from_secs(15));
    let reference: Vec<f32> = common::audio::to_f32(&a).into_iter().chain(common::audio::to_f32(&b)).collect();
    let audio = capture.samples();
    let start = audio.iter().position(|v| *v != 0.0).unwrap() / 2 * 2;
    assert!(audio[start..].starts_with(&reference[..reference.len() - 2]), "off wasn't bit-exact");

    let (_s, _q, p, capture, _d) = leveling_rig(&tracks, Leveling::Track);
    p.set_queue(vec![QueueItem::new("a".into(), Some(1000)), QueueItem::new("b".into(), Some(1000))], 0, 0, false);
    wait_status(&p, PlayerStatus::Stopped, Duration::from_secs(15));
    let audio = capture.samples();
    let start = audio.iter().position(|v| *v != 0.0).unwrap();
    let got = &audio[start..start + reference.len() - 2];
    let (gain_a, gain_b) = (10f32.powf(-6.0 / 20.0), 10f32.powf(-12.0 / 20.0));
    let split = 44_100 * 2;
    for (i, (g, r)) in got.iter().zip(&reference).enumerate() {
        let want = r * if i < split { gain_a } else { gain_b };
        assert!((g - want).abs() < 1e-5, "sample {i} ({}) got {g} want {want}", if i < split { "a" } else { "b" });
    }
}

#[test]
fn songs_with_no_replaygain_play_unchanged() {
    let a = synth(44_100, 44_100, 0.4);
    let tracks = vec![("a", serde_json::json!({}), a.clone())];
    let (_s, _q, p, capture, _d) = leveling_rig(&tracks, Leveling::Album);
    p.set_queue(vec![QueueItem::new("a".into(), Some(1000))], 0, 0, false);
    wait_status(&p, PlayerStatus::Stopped, Duration::from_secs(15));
    let reference = common::audio::to_f32(&a);
    let audio = capture.samples();
    let start = audio.iter().position(|v| *v != 0.0).unwrap();
    assert_eq!(&audio[start..start + reference.len() - 2], &reference[..reference.len() - 2]);
}

#[test]
fn a_file_at_or_below_the_limit_is_not_transcoded() {
    let tracks = vec![("a", serde_json::json!({ "bitRate": 128, "suffix": "mp3" }), synth(44_100, 44_100, 0.4))];
    let (s, q, p, _capture, _d) = leveling_rig(&tracks, Leveling::Off);
    q.set_streaming(Quality::Transcode { codec: Codec::Mp3, kbps: 192 }).unwrap();
    p.set_queue(vec![QueueItem::new("a".into(), Some(1000))], 0, 0, false);
    wait_status(&p, PlayerStatus::Stopped, Duration::from_secs(15));
    assert!(s.requests().iter().all(|r| r.format.as_deref() == Some("raw")), "{:?}", s.requests());
}
