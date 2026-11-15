//! Artist Radio from Deezer with the server's radio as fallback and padding, against fakes of both.

mod common;

use std::time::Duration;

use common::{FakeServer, FakeSong, State, song};
use navia_core::artist_radio::{Deezer, DeezerError, DeezerUse, Gate, RADIO_LEN, artist_radio};
use navia_core::deezer;
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SEED: &str = "ar-Seed Band";

fn with(mut s: FakeSong, extra: Value) -> FakeSong {
    s.extra = extra;
    s
}

fn library_songs() -> Vec<FakeSong> {
    vec![
        with(song("alpha", "Alpha", "Seed Band", "First", "1"), json!({"duration": 200})),
        with(song("beta", "Beta", "Seed Band", "First", "1"), json!({"duration": 210})),
        // Two versions of Epsilon, where the length picks the one Deezer means.
        with(song("eps-short", "Epsilon", "Seed Band", "Second", "1"), json!({"duration": 180})),
        with(song("eps-long", "Epsilon", "Seed Band", "Live Tapes", "1"), json!({"duration": 300})),
        with(song("zeta", "Zeta", "Seed Band", "Second", "1"), json!({"duration": 400})),
        with(song("gamma", "Gamma", "Other Act", "Third", "1"), json!({"duration": 240})),
        // Only its ISRC says it's Deezer's "Delta".
        with(song("delta", "デルタ", "Other Act", "Third", "1"), json!({"duration": 230, "isrc": ["USABC1234567"]})),
        with(song("srv1", "Server One", "Far Away", "Fourth", "1"), json!({"duration": 190})),
        with(song("srv2", "Server Two", "Far Away", "Fourth", "1"), json!({"duration": 190})),
    ]
}

fn track(id: u64, title: &str, artist: &str, album: &str, secs: u64) -> Value {
    json!({"id": id, "title": title, "duration": secs, "artist": {"id": 7, "name": artist, "picture": "https://example.invalid/p.jpg"}, "album": {"id": 8, "title": album, "cover": "https://example.invalid/c.jpg"}})
}

fn radio_tracks() -> Value {
    json!({"data": [
        track(9001, "Alpha", "Seed Band", "First", 201),
        track(9002, "Beta (feat. Guest)", "Seed Band", "First", 211),
        track(9003, "Gamma - Remastered 2011", "Other Act", "Third (Deluxe)", 242),
        track(9004, "Delta", "Other Act", "Third", 230),
        track(9005, "Epsilon", "Seed Band", "Live Tapes", 298),
        track(9006, "Zeta", "Seed Band", "Second", 100),
        track(9007, "Nobody Owns This", "Stranger", "Elsewhere", 200),
        track(9001, "Alpha", "Seed Band", "First", 201),
    ]})
}

struct Setup {
    server: FakeServer,
    deezer: MockServer,
    library: Library,
    _dir: tempfile::TempDir,
}

/// A synced library, the server's own radio of srv1, alpha and srv2, and a Deezer
/// that knows "Seed Band" with id 1 and its radio.
async fn setup() -> Setup {
    let state = State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, last_scan: Some("2026-01-01T00:00:00Z".into()), songs: library_songs(), ..Default::default() };
    let server = FakeServer::start(state).await;
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    Syncer::new(server.client(), library.clone(), SyncOptions::default()).full_sync(&|_| {}).await.unwrap();
    // The listener opted in. Tests of the gate change it.
    deezer::set_setting(&library, Some(true)).unwrap();
    server.state.lock().unwrap().canned.insert(
        "getSimilarSongs2".to_string(),
        json!({"similarSongs2": {"song": [{"id": "srv1", "title": "Server One", "artist": "Far Away"}, {"id": "alpha", "title": "Alpha", "artist": "Seed Band"}, {"id": "srv2", "title": "Server Two", "artist": "Far Away"}]}}),
    );
    let deezer = MockServer::start().await;
    Mock::given(method("GET")).and(path("/search/artist")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
        {"id": 2, "name": "Seed Band Tribute", "nb_fan": 50},
        {"id": 1, "name": "Seed Band", "nb_fan": 10, "picture": "https://example.invalid/a.jpg"}
    ]}))).mount(&deezer).await;
    Mock::given(method("GET")).and(path("/artist/1/radio")).respond_with(ResponseTemplate::new(200).set_body_json(radio_tracks())).mount(&deezer).await;
    Mock::given(method("GET")).and(path("/track/9004")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 9004, "isrc": "usabc1234567"}))).mount(&deezer).await;
    Mock::given(method("GET")).and(path("/track/9006")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 9006, "isrc": "ZZ0000000000"}))).mount(&deezer).await;
    Setup { server, deezer, library, _dir: dir }
}

fn fast(deezer: &MockServer) -> Deezer {
    Deezer::new(deezer.uri()).with_timing(Duration::from_millis(1), Duration::from_secs(5))
}

fn ids(radio: &navia_core::artist_radio::ArtistRadio) -> Vec<&str> {
    radio.songs.iter().map(|s| s.id.as_str()).collect()
}

async fn deezer_paths(deezer: &MockServer) -> Vec<String> {
    deezer.received_requests().await.unwrap().iter().map(|r| r.url.path().to_string()).collect()
}

#[tokio::test]
async fn deezer_songs_come_first_matched_by_name_and_isrc_then_the_server_fills_in() {
    let s = setup().await;
    // The Server Autoplay mode keeps this order, and the Navia mode spaces it below.
    navia_core::engine::set_mode(&s.library, navia_core::engine::AutoplayMode::Server).unwrap();
    let deezer = fast(&s.deezer);
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &deezer, SEED, "Seed Band").await;

    // Each Deezer track matches its own way, Zeta is too far off in length and the stranger isn't in
    // the library. Then comes the server's radio without its Alpha.
    assert_eq!(ids(&radio), ["alpha", "beta", "gamma", "delta", "eps-long", "srv1", "srv2"]);
    assert_eq!(radio.from_deezer, 5);
    let DeezerUse::Used(used) = &radio.deezer else { panic!("{:?}", radio.deezer) };
    assert_eq!((used.tracks, used.by_isrc), (7, 1));

    let asked = deezer_paths(&s.deezer).await;
    // ISRCs only for misses by artists the library has (Delta, Zeta), never the stranger's.
    assert!(asked.contains(&"/track/9004".to_string()) && asked.contains(&"/track/9006".to_string()), "{asked:?}");
    assert!(!asked.contains(&"/track/9007".to_string()), "{asked:?}");
    // Too few matched, so the radio was asked twice. The second call added nothing new here.
    assert_eq!(asked.iter().filter(|p| *p == "/artist/1/radio").count(), 2, "{asked:?}");
    // Every Deezer request carries Navia's User-Agent and nothing about the user or server.
    for request in s.deezer.received_requests().await.unwrap() {
        assert!(request.headers.get("user-agent").unwrap().to_str().unwrap().starts_with("Navia/"));
        let query = request.url.query().unwrap_or_default();
        assert!(!query.contains("tester") && !query.contains("127.0.0.1"), "{query}");
        assert!(request.headers.get("authorization").is_none() && request.headers.get("cookie").is_none());
    }
    assert!(s.server.calls("getSimilarSongs2") == 1);
    assert!(radio.songs.len() <= RADIO_LEN);

    // The Navia mode gives the same songs, spaced by artist and album.
    navia_core::engine::set_mode(&s.library, navia_core::engine::AutoplayMode::Navia).unwrap();
    let spaced = artist_radio(&s.server.read_only_client(), &s.library, &deezer, SEED, "Seed Band").await;
    assert_eq!(ids(&spaced), ["alpha", "gamma", "srv1", "eps-long", "delta", "srv2", "beta"]);
}

#[tokio::test]
async fn a_second_start_comes_from_the_cache() {
    let s = setup().await;
    let deezer = fast(&s.deezer);
    let client = s.server.read_only_client();
    let first = artist_radio(&client, &s.library, &deezer, SEED, "Seed Band").await;
    let before = deezer_paths(&s.deezer).await.len();
    let second = artist_radio(&client, &s.library, &deezer, SEED, "Seed Band").await;
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(deezer_paths(&s.deezer).await.len(), before, "no new Deezer requests");
}

#[tokio::test]
async fn deezer_waits_for_the_listener_to_opt_in() {
    let s = setup().await;
    let client = s.server.read_only_client();
    for (choice, gate) in [(None, Gate::NotChosen), (Some(false), Gate::Off)] {
        deezer::set_setting(&s.library, choice).unwrap();
        assert_eq!(deezer::setting(&s.library), choice);
        assert_eq!(deezer::gate(&s.library), gate);
        let radio = artist_radio(&client, &s.library, &fast(&s.deezer), SEED, "Seed Band").await;
        assert_eq!(radio.deezer, DeezerUse::NotAllowed(gate));
        assert_eq!(ids(&radio), ["srv1", "alpha", "srv2"]);
    }
    assert!(deezer_paths(&s.deezer).await.is_empty());
    deezer::set_setting(&s.library, Some(true)).unwrap();
    assert_eq!(deezer::gate(&s.library), Gate::Open);
    // The switch alone decides, and the server's external services setting is never asked.
    let radio = artist_radio(&client, &s.library, &fast(&s.deezer), SEED, "Seed Band").await;
    assert!(matches!(radio.deezer, DeezerUse::Used(_)));
    assert!(!s.server.server.received_requests().await.unwrap().iter().any(|r| r.url.path() == "/app/"));
}

#[tokio::test]
async fn a_failing_deezer_leaves_the_server_radio() {
    let s = setup().await;
    s.deezer.reset().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).mount(&s.deezer).await;
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &fast(&s.deezer), SEED, "Seed Band").await;
    assert_eq!(radio.deezer, DeezerUse::Failed(DeezerError::Http(500)));
    assert_eq!(ids(&radio), ["srv1", "alpha", "srv2"]);
    assert_eq!(radio.from_deezer, 0);
}

#[tokio::test]
async fn an_unreachable_deezer_leaves_the_server_radio() {
    let s = setup().await;
    // Nothing listens on port 9 of the loopback.
    let deezer = Deezer::new("http://127.0.0.1:9").with_timing(Duration::from_millis(1), Duration::from_secs(5));
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &deezer, SEED, "Seed Band").await;
    assert!(matches!(radio.deezer, DeezerUse::Failed(DeezerError::Connection(_))), "{:?}", radio.deezer);
    assert_eq!(ids(&radio), ["srv1", "alpha", "srv2"]);
}

#[tokio::test]
async fn a_slow_deezer_times_out_quickly() {
    let s = setup().await;
    s.deezer.reset().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})).set_delay(Duration::from_secs(10))).mount(&s.deezer).await;
    let deezer = Deezer::new(s.deezer.uri()).with_timing(Duration::from_millis(1), Duration::from_millis(400));
    let started = std::time::Instant::now();
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &deezer, SEED, "Seed Band").await;
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(radio.deezer, DeezerUse::Failed(DeezerError::Timeout));
    assert_eq!(ids(&radio), ["srv1", "alpha", "srv2"]);
}

#[tokio::test]
async fn the_quota_backs_off_without_asking_again() {
    let s = setup().await;
    s.deezer.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error": {"type": "Exception", "message": "Quota limit exceeded", "code": 4}})))
        .mount(&s.deezer)
        .await;
    let deezer = fast(&s.deezer);
    let client = s.server.read_only_client();
    let first = artist_radio(&client, &s.library, &deezer, SEED, "Seed Band").await;
    assert_eq!(first.deezer, DeezerUse::Failed(DeezerError::Quota));
    assert_eq!(ids(&first), ["srv1", "alpha", "srv2"]);
    let second = artist_radio(&client, &s.library, &deezer, SEED, "Other Act").await;
    assert_eq!(second.deezer, DeezerUse::Failed(DeezerError::BackingOff));
    assert_eq!(deezer_paths(&s.deezer).await.len(), 1, "no request while backing off");
}

#[tokio::test]
async fn an_artist_deezer_doesnt_know_leaves_the_server_radio() {
    let s = setup().await;
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &fast(&s.deezer), "ar-Far Away", "Far Away").await;
    // This Deezer only has "Seed Band" and a tribute, with no exact name match, so no radio from it.
    let DeezerUse::Used(used) = &radio.deezer else { panic!("{:?}", radio.deezer) };
    assert!(!used.found_artist);
    assert_eq!(radio.from_deezer, 0);
    assert_eq!(ids(&radio), ["srv1", "alpha", "srv2"]);
    assert!(!deezer_paths(&s.deezer).await.iter().any(|p| p.contains("/radio")));
}

#[tokio::test]
async fn of_two_deezer_artists_with_the_name_the_one_the_library_has_wins() {
    let s = setup().await;
    s.deezer.reset().await;
    // Two "Seed Band"s, and the more popular one isn't the library's.
    Mock::given(method("GET")).and(path("/search/artist")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
        {"id": 50, "name": "Seed Band", "nb_fan": 9000},
        {"id": 1, "name": "Seed Band", "nb_fan": 10}
    ]}))).mount(&s.deezer).await;
    Mock::given(method("GET")).and(path("/artist/50/top")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [track(1, "Unknown Hit", "Seed Band", "X", 200)]}))).mount(&s.deezer).await;
    Mock::given(method("GET")).and(path("/artist/1/top")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [track(9001, "Alpha", "Seed Band", "First", 201)]}))).mount(&s.deezer).await;
    Mock::given(method("GET")).and(path("/artist/1/radio")).respond_with(ResponseTemplate::new(200).set_body_json(radio_tracks())).mount(&s.deezer).await;
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &fast(&s.deezer), SEED, "Seed Band").await;
    assert!(radio.from_deezer >= 4, "{:?}", ids(&radio));
    assert!(!deezer_paths(&s.deezer).await.iter().any(|p| p == "/artist/50/radio"));
}

#[tokio::test]
async fn a_full_deezer_radio_needs_no_padding() {
    let s = setup().await;
    // Sixty owned songs by the seed artist, all in Deezer's radio.
    let mut songs = library_songs();
    let mut tracks = Vec::new();
    for i in 0..60u64 {
        songs.push(with(song(&format!("own{i}"), &format!("Tune {i}"), "Seed Band", "Many", "1"), json!({"duration": 200})));
        tracks.push(track(10_000 + i, &format!("Tune {i}"), "Seed Band", "Many", 200));
    }
    s.server.state.lock().unwrap().songs = songs;
    Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default()).full_sync(&|_| {}).await.unwrap();
    s.deezer.reset().await;
    Mock::given(method("GET")).and(path("/search/artist")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": 1, "name": "Seed Band", "nb_fan": 10}]}))).mount(&s.deezer).await;
    Mock::given(method("GET")).and(path("/artist/1/radio")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": tracks}))).mount(&s.deezer).await;
    let radio = artist_radio(&s.server.read_only_client(), &s.library, &fast(&s.deezer), SEED, "Seed Band").await;
    assert_eq!(radio.songs.len(), RADIO_LEN);
    assert_eq!(radio.from_deezer, RADIO_LEN);
    // Enough matched on the first call, so one radio request and no ISRC lookups.
    assert_eq!(deezer_paths(&s.deezer).await, ["/search/artist", "/artist/1/radio"]);
}
