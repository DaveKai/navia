//! "Because you've been playing X" against a fake server: how often it asks `getArtistInfo2`.

mod common;

use common::{FakeServer, State, song};
use navia_core::client::native::ScrobbleEntry;
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::taste::{Because, refresh_because};
use navia_core::library::{Library, LibraryConfig};
use serde_json::json;
use wiremock::matchers::path_regex;
use wiremock::{Mock, ResponseTemplate};

const DAY: i64 = 86_400;

async fn setup(similar: serde_json::Value) -> (tempfile::TempDir, FakeServer, Library) {
    let mut songs = Vec::new();
    for i in 1..=5 {
        songs.push(song(&format!("t{i}"), "Top", &format!("T{i}"), &format!("Top {i}"), "1"));
        songs.push(song(&format!("s{i}"), "Similar", &format!("S{i}"), &format!("Similar {i}"), "1"));
    }
    let state = State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, last_scan: Some("2026-01-01T00:00:00Z".into()), songs, ..Default::default() };
    let server = FakeServer::start(state).await;
    let body = json!({"subsonic-response": {"status": "ok", "version": "1.16.1", "openSubsonic": true, "artistInfo2": similar["artistInfo2"].clone()}});
    Mock::given(path_regex("getArtistInfo2")).respond_with(ResponseTemplate::new(200).set_body_json(body)).with_priority(1).mount(&server.server).await;
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    Syncer::new(server.client(), library.clone(), SyncOptions::default()).full_sync(&|_| {}).await.unwrap();
    let now = navia_core::library::taste::now_unix();
    let plays: Vec<ScrobbleEntry> = (1..=5).flat_map(|i| (0..i).map(move |n| ScrobbleEntry { id: i * 10 + n, song_id: format!("t{i}"), played_at: now - DAY - n * 60 })).collect();
    library.replace_recent_plays(&plays).unwrap();
    (dir, server, library)
}

async fn asks(server: &FakeServer) -> usize {
    server.server.received_requests().await.unwrap().iter().filter(|r| r.url.path().contains("getArtistInfo2")).count()
}

fn similar(ids: &[&str]) -> serde_json::Value {
    json!({"artistInfo2": {"similarArtist": ids.iter().map(|id| json!({"id": id, "name": id})).collect::<Vec<_>>()}})
}

#[tokio::test]
async fn asks_once_per_seed_per_day() {
    let (_dir, server, library) = setup(similar(&["ar-S1", "ar-S2", "ar-S3", "ar-S4", "ar-S5", "ar-T1", "not-in-library"])).await;
    let client = server.client();
    let day = 20_000;
    let Because::Ready { seed, artists } = refresh_because(&client, &library, day).await.unwrap() else { panic!("no row") };
    assert_eq!(asks(&server).await, 1);
    let names: Vec<String> = library.artists_by_rowid(&artists).unwrap().into_iter().map(|(_, a)| a.name).collect();
    assert_eq!(names, ["S1", "S2", "S3", "S4", "S5"]);
    assert!(library.artists_by_rowid(&[seed]).unwrap()[0].1.name.starts_with('T'));
    // Later the same day it's all from the database.
    assert!(matches!(library.because(day).unwrap(), Because::Ready { .. }));
    refresh_because(&client, &library, day).await.unwrap();
    assert_eq!(asks(&server).await, 1);
    // A new day asks again.
    refresh_because(&client, &library, day + 1).await.unwrap();
    assert_eq!(asks(&server).await, 2);
}

#[tokio::test]
async fn no_similar_artists_means_no_row_and_a_bounded_number_of_asks() {
    let (_dir, server, library) = setup(json!({"artistInfo2": {}})).await;
    let client = server.client();
    assert_eq!(refresh_because(&client, &library, 7).await.unwrap(), Because::Nothing);
    // Every seed asked once, then nothing more that day.
    let first = asks(&server).await;
    assert!((1..=5).contains(&first), "{first}");
    refresh_because(&client, &library, 7).await.unwrap();
    assert_eq!(asks(&server).await, first);
}
