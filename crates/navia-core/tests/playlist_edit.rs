//! Adding and removing playlist songs against the local fake server with one `updatePlaylist`.
//! Removals are checked against the order shown, and smart, others' and read-only playlists are refused.

mod common;

use common::{FakeServer, State, song};
use navia_core::library::sync::{PlaylistEditError, SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::{Value, json};

fn state() -> State {
    State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, songs: vec![song("s1", "One", "Art", "Alb", "1")], ..Default::default() }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn playlist(id: &str, songs: &[&str], changed: &str, extra: Value) -> Value {
    let mut p = json!({"id": id, "name": format!("List {id}"), "owner": "tester", "songCount": songs.len(), "changed": changed, "readonly": false,
        "entry": songs.iter().map(|s| json!({"id": s, "title": format!("Song {s}")})).collect::<Vec<_>>()});
    if let (Some(m), Value::Object(e)) = (p.as_object_mut(), extra) {
        m.extend(e);
    }
    json!({"playlist": p})
}

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
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    Syncer::new(server.client(), library.clone(), SyncOptions::default()).sync_playlists().await.unwrap();
    (server, dir, library)
}

fn stored(library: &Library, id: &str) -> Vec<String> {
    library.playlist(id).unwrap().unwrap().entry.into_iter().map(|s| s.id).collect()
}

const T1: &str = "2026-01-01T00:00:00Z";

#[tokio::test(flavor = "multi_thread")]
async fn adding_appends_in_one_form_and_stores_the_new_songs() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a", "b"], T1, json!({})))]).await;
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let many: Vec<String> = (0..250).map(|i| format!("n{i}")).collect();
    sync.add_to_playlist("p1", &many, true).await.unwrap();
    let updates = server.state.lock().unwrap().playlist_updates.clone();
    assert_eq!(updates.len(), 1);
    assert_eq!((updates[0].0.as_str(), updates[0].1.as_str(), &updates[0].2), ("POST", "p1", &many));
    let mut want = ids(&["a", "b"]);
    want.extend(many);
    assert_eq!(stored(&library, "p1"), want, "the server's copy is stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_form_post_a_long_list_goes_in_url_chunks() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a"], T1, json!({})))]).await;
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let many: Vec<String> = (0..250).map(|i| format!("n{i}")).collect();
    sync.add_to_playlist("p1", &many, false).await.unwrap();
    let updates = server.state.lock().unwrap().playlist_updates.clone();
    assert_eq!(updates.iter().map(|u| u.2.len()).collect::<Vec<_>>(), vec![100, 100, 50]);
    assert!(updates.iter().all(|u| u.0 == "GET"));
    assert_eq!(stored(&library, "p1").len(), 251);
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_by_position_checks_the_order_first() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a", "b", "a", "c"], T1, json!({})))]).await;
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    // Only the second "a". A song in a playlist twice is removed by its place, not its id.
    sync.remove_from_playlist("p1", &ids(&["a", "b", "a", "c"]), &[2, 1], true).await.unwrap();
    let updates = server.state.lock().unwrap().playlist_updates.clone();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].3, vec![2, 1]);
    assert_eq!(stored(&library, "p1"), ids(&["a", "c"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_playlist_changed_elsewhere_is_not_removed_from() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a", "b", "c"], T1, json!({})))]).await;
    server.state.lock().unwrap().canned.insert("getPlaylist:p1".into(), playlist("p1", &["x", "a", "b", "c"], T1, json!({})));
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let result = sync.remove_from_playlist("p1", &ids(&["a", "b", "c"]), &[0], true).await;
    assert_eq!(result, Err(PlaylistEditError::Changed));
    assert!(server.state.lock().unwrap().playlist_updates.is_empty(), "nothing removed");
    assert_eq!(stored(&library, "p1"), ids(&["x", "a", "b", "c"]), "the server's copy is stored for the page");
}

#[tokio::test(flavor = "multi_thread")]
async fn smart_others_and_read_only_runs_send_nothing() {
    let (server, _dir, library) = setup(&[
        ("own", playlist("own", &["a"], T1, json!({}))),
        ("smart", playlist("smart", &["a"], T1, json!({"readonly": true, "validUntil": "2026-01-02T00:00:00Z"}))),
        ("theirs", playlist("theirs", &["a"], T1, json!({"readonly": true, "owner": "friend"}))),
    ])
    .await;
    let fetched = server.calls("getPlaylist");
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let add = ids(&["b"]);
    assert_eq!(sync.add_to_playlist("smart", &add, true).await, Err(PlaylistEditError::ReadOnly));
    assert_eq!(sync.add_to_playlist("theirs", &add, true).await, Err(PlaylistEditError::ReadOnly));
    assert_eq!(sync.remove_from_playlist("smart", &ids(&["a"]), &[0], true).await, Err(PlaylistEditError::ReadOnly));
    assert_eq!(sync.add_to_playlist("gone", &add, true).await, Err(PlaylistEditError::NotFound));
    let read_only = Syncer::new(server.read_only_client(), library.clone(), SyncOptions::default());
    assert_eq!(read_only.add_to_playlist("own", &add, true).await, Err(PlaylistEditError::WritesDisabled));
    assert_eq!(read_only.remove_from_playlist("own", &ids(&["a"]), &[0], true).await, Err(PlaylistEditError::WritesDisabled));
    assert_eq!(server.calls("getPlaylist"), fetched, "a smart playlist is never fetched, and a read-only run fetches nothing");
    assert_eq!(server.calls("updatePlaylist"), 0);
    assert!(read_only.create_playlist_with("New", &add, true).await.is_err());
    assert_eq!(server.calls("createPlaylist"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_write_changes_nothing_locally() {
    let (server, _dir, library) = setup(&[("p1", playlist("p1", &["a"], T1, json!({})))]).await;
    // The server loses the playlist between the list and the write.
    server.state.lock().unwrap().canned.remove("getPlaylist:p1");
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    assert!(matches!(sync.add_to_playlist("p1", &ids(&["b"]), true).await, Err(PlaylistEditError::Failed(_))));
    assert_eq!(stored(&library, "p1"), ids(&["a"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_playlist_is_created_with_its_songs_and_stored() {
    let (server, _dir, library) = setup(&[]).await;
    let sync = Syncer::new(server.client(), library.clone(), SyncOptions::default());
    let id = sync.create_playlist_with("Road Trip", &ids(&["a", "b"]), true).await.unwrap().unwrap();
    let creates = server.state.lock().unwrap().playlist_creates.clone();
    assert_eq!(creates, vec![("POST".to_string(), "Road Trip".to_string(), ids(&["a", "b"]))]);
    assert_eq!(stored(&library, &id), ids(&["a", "b"]));
    // Without formPost, the first chunk goes with the create and the rest after it.
    let many: Vec<String> = (0..150).map(|i| format!("n{i}")).collect();
    let id = sync.create_playlist_with("Long", &many, false).await.unwrap().unwrap();
    assert_eq!(server.state.lock().unwrap().playlist_creates[1].2.len(), 100);
    assert_eq!(stored(&library, &id), many);
}

#[tokio::test(flavor = "multi_thread")]
async fn targets_are_own_regular_playlists_newest_change_first() {
    let (_server, _dir, library) = setup(&[
        ("old", playlist("old", &["a"], "2025-05-01T00:00:00Z", json!({}))),
        ("new", playlist("new", &["a"], "2026-03-01T00:00:00Z", json!({}))),
        ("smart", playlist("smart", &["a"], "2026-04-01T00:00:00Z", json!({"readonly": true, "validUntil": "2026-04-02T00:00:00Z"}))),
        ("theirs", playlist("theirs", &["a"], "2026-04-01T00:00:00Z", json!({"readonly": true, "owner": "friend"}))),
    ])
    .await;
    let targets: Vec<String> = library.playlist_targets().unwrap().into_iter().map(|p| p.id).collect();
    assert_eq!(targets, ids(&["new", "old"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn songs_already_in_a_playlist_are_found_in_the_order_asked() {
    let (_server, _dir, library) = setup(&[("p1", playlist("p1", &["a", "b"], T1, json!({})))]).await;
    assert_eq!(library.already_in_playlist("p1", &ids(&["c", "b", "a"])).unwrap(), ids(&["b", "a"]));
    assert!(library.already_in_playlist("p1", &ids(&["c"])).unwrap().is_empty());
    assert!(library.already_in_playlist("missing", &ids(&["a"])).unwrap().is_empty());
}
