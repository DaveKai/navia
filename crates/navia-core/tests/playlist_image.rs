//! Playlist images against a fake server shaped like Navidrome 0.64.2's native API.

mod common;

use common::{FakeServer, State};
use navia_core::client::native::PlaylistImageError;
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, ResponseTemplate};

const AUTH: &str = "x-nd-authorization";
const PLAYLIST_PATH: &str = "/api/playlist/pl1/image";

fn state() -> State {
    State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, ..Default::default() }
}

fn open_library() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    (dir, library)
}

fn list_with_cover(server: &FakeServer, cover: &str) {
    server.state.lock().unwrap().canned.insert("getPlaylists".to_string(), json!({"playlists": {"playlist": [{"id": "pl1", "name": "Mine", "owner": "tester", "songCount": 1, "duration": 10, "coverArt": cover}]}}));
}

async fn login_ok(server: &FakeServer) {
    Mock::given(method("POST")).and(path("/auth/login")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "jwt"}))).mount(&server.server).await;
}

fn temp_image() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cover.png");
    std::fs::write(&file, b"PNGfake").unwrap();
    (dir, file)
}

#[tokio::test]
async fn upload_sends_the_image_and_picks_up_the_new_cover() {
    let server = FakeServer::start(state()).await;
    login_ok(&server).await;
    Mock::given(method("POST"))
        .and(path(PLAYLIST_PATH))
        .and(header(AUTH, "Bearer jwt"))
        .and(body_string_contains("name=\"image\""))
        .and(body_string_contains("fake"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"ok\"}"))
        .expect(1)
        .mount(&server.server)
        .await;
    list_with_cover(&server, "pl-pl1_0000000000000002");
    let (_db, library) = open_library();
    let syncer = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let (_dir, file) = temp_image();
    let cover = syncer.set_playlist_image("pl1", &file).await.unwrap();
    assert_eq!(cover.as_deref(), Some("pl-pl1_0000000000000002"));
    assert_eq!(library.playlist_header("pl1").unwrap().unwrap().cover_art.as_deref(), Some("pl-pl1_0000000000000002"));
}

#[tokio::test]
async fn remove_calls_delete() {
    let server = FakeServer::start(state()).await;
    login_ok(&server).await;
    Mock::given(method("DELETE")).and(path(PLAYLIST_PATH)).and(header(AUTH, "Bearer jwt")).respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"ok\"}")).expect(1).mount(&server.server).await;
    list_with_cover(&server, "pl-pl1_0000000000000003");
    let (_db, library) = open_library();
    let syncer = Syncer::new(server.client(), library, SyncOptions::default());
    assert_eq!(syncer.remove_playlist_image("pl1").await.unwrap().as_deref(), Some("pl-pl1_0000000000000003"));
}

#[tokio::test]
async fn read_only_client_never_reaches_the_network() {
    let server = FakeServer::start(state()).await;
    Mock::given(method("POST")).and(path("/auth/login")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "jwt"}))).expect(0).mount(&server.server).await;
    Mock::given(path(PLAYLIST_PATH)).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server.server).await;
    let (_db, library) = open_library();
    let syncer = Syncer::new(server.read_only_client(), library, SyncOptions::default());
    let (_dir, file) = temp_image();
    assert_eq!(syncer.set_playlist_image("pl1", &file).await.unwrap_err(), PlaylistImageError::WritesDisabled);
    assert_eq!(syncer.remove_playlist_image("pl1").await.unwrap_err(), PlaylistImageError::WritesDisabled);
}

#[tokio::test]
async fn refusals_become_clear_errors() {
    let server = FakeServer::start(state()).await;
    login_ok(&server).await;
    Mock::given(method("POST")).and(path(PLAYLIST_PATH)).respond_with(ResponseTemplate::new(403).set_body_string("artwork upload is disabled\n")).mount(&server.server).await;
    let (_db, library) = open_library();
    let syncer = Syncer::new(server.client(), library, SyncOptions::default());
    let (_dir, file) = temp_image();
    assert_eq!(syncer.set_playlist_image("pl1", &file).await.unwrap_err(), PlaylistImageError::UploadsDisabled);
    assert!(matches!(syncer.set_playlist_image("pl1", std::path::Path::new("/nonexistent/x.png")).await.unwrap_err(), PlaylistImageError::File(_)));
}

#[tokio::test]
async fn support_is_read_from_the_public_page_with_no_login() {
    let server = FakeServer::start(state()).await;
    let config = serde_json::to_string(&json!({"version": "0.64.2 (abc)", "enableArtworkUpload": true})).unwrap();
    let page = format!("<html><script>window.__APP_CONFIG__ = {};</script></html>", serde_json::Value::String(config));
    Mock::given(method("GET")).and(path("/app/")).respond_with(ResponseTemplate::new(200).set_body_string(page)).mount(&server.server).await;
    Mock::given(method("POST")).and(path("/auth/login")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server.server).await;
    let support = server.read_only_client().playlist_image_support().await.unwrap();
    assert!(support.supported);
    assert_eq!(support.uploads_enabled, Some(true));
}

#[tokio::test]
async fn servers_older_than_the_route_are_not_offered_it() {
    let server = FakeServer::start(state()).await;
    let config = serde_json::to_string(&json!({"version": "0.60.3 (abc)"})).unwrap();
    let page = format!("<script>window.__APP_CONFIG__ = {};</script>", serde_json::Value::String(config));
    Mock::given(method("GET")).and(path("/app/")).respond_with(ResponseTemplate::new(200).set_body_string(page)).mount(&server.server).await;
    let support = server.read_only_client().playlist_image_support().await.unwrap();
    assert!(!support.supported);
}
