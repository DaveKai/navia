//! Saving a playlist's new song order against the local fake server in one POST form.
//! A playlist changed elsewhere is refused, and smart, read-only and test runs send nothing.

mod common;

use common::{FakeServer, State, song};
use navia_core::library::sync::{PlaylistEditError, SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::{Value, json};

fn state() -> State {
    State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, songs: vec![song("s1", "One", "Art", "Alb", "1")], ..Default::default() }
}

fn open_library() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    (dir, library)
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn playlist(id: &str, songs: &[&str], extra: Value) -> Value {
    let mut p = json!({"id": id, "name": "Mix", "owner": "tester", "songCount": songs.len(), "changed": "2026-01-01T00:00:00Z", "readonly": false,
        "entry": songs.iter().map(|s| json!({"id": s, "title": format!("Song {s}")})).collect::<Vec<_>>()});
    if let (Some(m), Value::Object(e)) = (p.as_object_mut(), extra) {
        m.extend(e);
    }
    json!({"playlist": p})
}

/// A fake with these playlists listed and fetchable, synced into a fresh library.
async fn setup(lists: &[(&str, Value)]) -> (FakeServer, tempfile::TempDir, Library) {
    let server = FakeServer::start(state()).await;
    {
        let mut st = server.state.lock().unwrap();
        let headers: Vec<Value> = lists.iter().map(|(_, p)| {
            let mut h = p["playlist"].clone();
            h.as_object_mut().unwrap().remove("entry");
            h
        }).collect();
        st.canned.insert("getPlaylists".into(), json!({"playlists": {"playlist": headers}}));
        for (id, body) in lists {
            st.canned.insert(format!("getPlaylist:{id}"), body.clone());
        }
    }
    let (dir, library) = open_library();
    Syncer::new(server.client(), library.clone(), SyncOptions::default()).sync_playlists().await.unwrap();
    (server, dir, library)
}

fn stored(library: &Library, id: &str) -> Vec<String> {
    library.playlist(id).unwrap().unwrap().entry.into_iter().map(|s| s.id).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_order_is_saved_as_one_post_form_and_stored() {
    let songs: Vec<String> = (0..400).map(|i| format!("song-{i}")).collect();
    let refs: Vec<&str> = songs.iter().map(String::as_str).collect();
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &refs, json!({})))]).await;
    let mut order = songs.clone();
    let moved = order.remove(399);
    order.insert(0, moved);
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    sync.save_playlist_order("p1", &songs, &order).await.unwrap();
    let saves = server.state.lock().unwrap().playlist_saves.clone();
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].0, "POST", "a long list goes in the body, not the URL");
    assert_eq!((saves[0].1.as_str(), &saves[0].2), ("p1", &order));
    assert_eq!(stored(&library, "p1"), order, "the saved order is stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_playlist_changed_elsewhere_is_refused_and_refreshed() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a", "b", "c"], json!({})))]).await;
    // Someone adds a song on another client after the page was shown.
    server.state.lock().unwrap().canned.insert("getPlaylist:p1".into(), playlist("p1", &["a", "b", "c", "d"], json!({})));
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let result = sync.save_playlist_order("p1", &ids(&["a", "b", "c"]), &ids(&["c", "a", "b"])).await;
    assert_eq!(result, Err(PlaylistEditError::Changed));
    assert!(server.state.lock().unwrap().playlist_saves.is_empty(), "nothing written");
    assert_eq!(stored(&library, "p1"), ids(&["a", "b", "c", "d"]), "the server's copy is stored for the page");
}

#[tokio::test(flavor = "multi_thread")]
async fn smart_read_only_and_test_runs_send_nothing() {
    let (server, _dir, library) = setup(&[
        ("own", playlist("own", &["a", "b"], json!({}))),
        ("smart", playlist("smart", &["a", "b"], json!({"readonly": true, "validUntil": "2026-01-02T00:00:00Z"}))),
        ("theirs", playlist("theirs", &["a", "b"], json!({"readonly": true, "owner": "friend"}))),
    ])
    .await;
    let fetched = server.calls("getPlaylist");
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let (old, new) = (ids(&["a", "b"]), ids(&["b", "a"]));
    assert_eq!(sync.save_playlist_order("smart", &old, &new).await, Err(PlaylistEditError::ReadOnly));
    assert_eq!(sync.save_playlist_order("theirs", &old, &new).await, Err(PlaylistEditError::ReadOnly));
    assert_eq!(server.calls("getPlaylist"), fetched, "a smart playlist is never fetched to save it");
    let read_only = Syncer::new(server.read_only_client(), library.clone(), SyncOptions::default());
    assert_eq!(read_only.save_playlist_order("own", &old, &new).await, Err(PlaylistEditError::WritesDisabled));
    assert_eq!(server.calls("getPlaylist"), fetched, "a read-only run doesn't even fetch");
    assert_eq!(
        sync.save_playlist_order("own", &old, &ids(&["b", "x"])).await,
        Err(PlaylistEditError::Failed("the new order doesn't hold the same songs".into()))
    );
    assert!(server.state.lock().unwrap().playlist_saves.is_empty());
    assert_eq!(server.calls("createPlaylist"), 0);
}
