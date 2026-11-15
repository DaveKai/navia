//! Synced downloads of playlists and albums, against a local fake of Navidrome.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::audio::{synth, wav_bytes};
use common::{FakeServer, State, song};
use navia_core::downloads::{CollectionEvent, CollectionPhase, DownloadManager, DownloadRequest, RemovalPlan};
use navia_core::library::collections::CollectionKind::{Album, Playlist};
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::json;
use tokio::runtime::Runtime;

struct Rig {
    rt: Runtime,
    server: FakeServer,
    library: Library,
    events: Arc<Mutex<Vec<CollectionEvent>>>,
    dir: tempfile::TempDir,
}

const ALBUM_X: &str = "al-X";

/// Songs a, b, c on album X and d, e on album Y, each a small WAV, with the library synced.
fn rig() -> Rig {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let mut state = State { folders: vec![("1".into(), "Music".into())], last_scan: Some("2026-01-01T00:00:00Z".into()), ..State::default() };
    for (id, album) in [("a", "X"), ("b", "X"), ("c", "X"), ("d", "Y"), ("e", "Y")] {
        state.songs.push(song(id, &format!("Song {id}"), "Artist", album, "1"));
        state.stream.insert(id.into(), wav_bytes(&synth(4_800, 48_000, id.as_bytes()[0] as f64 / 100.0), 48_000));
    }
    let server = rt.block_on(FakeServer::start(state));
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
    let syncer = Syncer::new(server.read_only_client(), library.clone(), SyncOptions::default());
    rt.block_on(syncer.full_sync(&|_| {})).unwrap();
    Rig { rt, server, library, events: Arc::default(), dir }
}

impl Rig {
    fn manager(&self) -> Arc<DownloadManager> {
        let manager = Arc::new(DownloadManager::new(self.library.clone(), self.server.read_only_client(), self.dir.path().join("downloads")).unwrap());
        let sink = self.events.clone();
        manager.add_collection_listener(move |e| sink.lock().unwrap().push(e.clone()));
        manager
    }

    /// Puts a playlist on the fake server with these songs and fetches it, as opening it does.
    fn playlist(&self, id: &str, songs: &[&str]) {
        self.set_playlist(id, songs, None);
        self.rt.block_on(Syncer::new(self.server.read_only_client(), self.library.clone(), SyncOptions::default()).refresh_playlist(id)).unwrap();
    }

    fn set_playlist(&self, id: &str, songs: &[&str], valid_until: Option<&str>) {
        let entries: Vec<_> = songs.iter().map(|s| json!({"id": s, "title": format!("Song {s}")})).collect();
        let mut body = json!({"id": id, "name": format!("List {id}"), "owner": "tester", "readonly": valid_until.is_some(), "songCount": songs.len(), "changed": format!("2026-01-0{}T00:00:00Z", songs.len()), "entry": entries});
        if let Some(v) = valid_until {
            body["validUntil"] = json!(v);
        }
        let mut header = body.clone();
        header.as_object_mut().unwrap().remove("entry");
        let mut st = self.server.state.lock().unwrap();
        st.canned.insert(format!("getPlaylist:{id}"), json!({"playlist": body}));
        st.canned.insert("getPlaylists".into(), json!({"playlists": {"playlist": [header]}}));
    }

    fn stream_calls(&self) -> usize {
        self.server.calls("stream")
    }
}

fn wait_for(limit: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    check()
}

fn on_device(m: &DownloadManager, ids: &[&str]) -> bool {
    ids.iter().all(|id| m.local_path(id).is_some())
}

fn off_device(m: &DownloadManager, ids: &[&str]) -> bool {
    ids.iter().all(|id| m.local_path(id).is_none())
}

fn files(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            n += if e.path().is_dir() { files(&e.path()) } else { 1 };
        }
    }
    n
}

const LIMIT: Duration = Duration::from_secs(10);

#[test]
fn a_kept_playlist_downloads_added_songs_and_drops_removed_ones() {
    let rig = rig();
    let m = rig.manager();
    rig.playlist("p1", &["a", "b"]);
    m.collection_download_on(Playlist, "p1").unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["a", "b"])));
    assert!(wait_for(LIMIT, || m.collection_status(Playlist, "p1").unwrap().is_some_and(|s| s.phase == CollectionPhase::Done)));
    let status = m.collection_status(Playlist, "p1").unwrap().unwrap();
    assert_eq!((status.have, status.total, status.missing), (2, 2, 0));
    assert!(status.bytes > 0 && status.progress == 1.0);
    assert!(!m.is_single_download("a"), "held by the playlist, not on its own");
    assert!(rig.events.lock().unwrap().iter().any(|e| e.kind == Playlist && e.id == "p1"));

    // The server's playlist changes, with a leaving and c arriving. Storing it is enough to sync.
    rig.playlist("p1", &["b", "c"]);
    assert!(wait_for(LIMIT, || on_device(&m, &["b", "c"]) && off_device(&m, &["a"])), "a: {:?} c: {:?}", m.local_path("a"), m.local_path("c"));
    let mut members = rig.library.collection_members(Playlist, "p1").unwrap();
    members.sort();
    assert_eq!(members, ["b", "c"]);
    assert_eq!(m.collection_songs_on_device(Playlist, "p1").unwrap(), ["b", "c"]);
    assert_eq!(files(&rig.dir.path().join("downloads")), 2);
    // Syncing again changes nothing and fetches nothing.
    let before = rig.stream_calls();
    m.sync_collections().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.stream_calls(), before);
}

#[test]
fn songs_another_download_holds_stay_and_removal_plans_count_them() {
    let rig = rig();
    let m = rig.manager();
    rig.playlist("p1", &["a", "b"]);
    m.collection_download_on(Playlist, "p1").unwrap();
    m.collection_download_on(Album, ALBUM_X).unwrap();
    m.enqueue(vec![DownloadRequest { id: "d".into(), suffix: None, size: None }]);
    assert!(wait_for(LIMIT, || on_device(&m, &["a", "b", "c", "d"])));
    assert_eq!(m.synced_collections().unwrap().iter().map(|c| (c.kind, c.id.as_str())).collect::<Vec<_>>(), [(Album, ALBUM_X), (Playlist, "p1")], "newest first");
    assert!(m.is_single_download("d") && !m.is_single_download("c"));

    // The album holds a and b too, so only c is the album's own, and the playlist has none of its own.
    assert_eq!(m.removal_plan(Album, ALBUM_X).unwrap().remove, 1);
    assert_eq!(m.removal_plan(Album, ALBUM_X).unwrap().keep, 2);
    assert_eq!(m.removal_plan(Playlist, "p1").unwrap(), RemovalPlan { remove: 0, remove_bytes: 0, keep: 2 });

    // Removing a single download a collection holds keeps the file.
    m.enqueue(vec![DownloadRequest { id: "a".into(), suffix: None, size: None }]);
    assert!(m.is_single_download("a"));
    m.remove(&["a".to_string()]).unwrap();
    assert!(on_device(&m, &["a"]) && !m.is_single_download("a"));

    m.collection_download_off(Album, ALBUM_X, false).unwrap();
    assert!(off_device(&m, &["c"]) && on_device(&m, &["a", "b", "d"]), "only c went");
    assert!(m.collection_status(Album, ALBUM_X).unwrap().is_none());
    m.collection_download_off(Playlist, "p1", false).unwrap();
    assert!(off_device(&m, &["a", "b"]) && on_device(&m, &["d"]), "the single download stays");
    // A single download nothing else holds is deleted.
    m.remove(&["d".to_string()]).unwrap();
    assert_eq!(files(&rig.dir.path().join("downloads")), 0);
}

#[test]
fn keep_songs_turns_them_into_single_downloads() {
    let rig = rig();
    let m = rig.manager();
    m.collection_download_on(Album, ALBUM_X).unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["a", "b", "c"])));
    let plan = m.removal_plan(Album, ALBUM_X).unwrap();
    assert_eq!((plan.remove, plan.keep), (3, 0));
    assert!(plan.remove_bytes > 0);
    m.collection_download_off(Album, ALBUM_X, true).unwrap();
    assert!(on_device(&m, &["a", "b", "c"]));
    assert!(["a", "b", "c"].iter().all(|id| m.is_single_download(id)));
    assert!(m.synced_collections().unwrap().is_empty());
    let mut singles = m.single_download_ids().unwrap();
    singles.sort();
    assert_eq!(singles, ["a", "b", "c"]);
}

#[test]
fn metered_pauses_automatic_sync_unless_allowed_and_manual_runs_anyway() {
    let rig = rig();
    let m = rig.manager();
    m.set_metered(true);
    assert!(!m.download_on_metered(), "off by default");
    rig.playlist("p1", &["a"]);
    // Turning the switch on is the user's own download, so it runs on a metered connection.
    m.collection_download_on(Playlist, "p1").unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["a"])));
    // A song a single download asks for runs too.
    m.enqueue(vec![DownloadRequest { id: "e".into(), suffix: None, size: None }]);
    assert!(wait_for(LIMIT, || on_device(&m, &["e"])));

    // A song added on the server is an automatic download, so it waits.
    rig.playlist("p1", &["a", "b"]);
    assert!(wait_for(LIMIT, || m.in_progress().contains(&"b".to_string())));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(off_device(&m, &["b"]));
    let status = m.collection_status(Playlist, "p1").unwrap().unwrap();
    assert_eq!((status.phase, status.have, status.missing), (CollectionPhase::Metered, 1, 1));

    // The setting lets it run.
    rig.events.lock().unwrap().clear();
    m.set_download_on_metered(true).unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["b"])));
    assert!(wait_for(LIMIT, || rig.events.lock().unwrap().iter().any(|e| e.id == "p1")), "paused state change announced");
    assert_eq!(m.collection_status(Playlist, "p1").unwrap().unwrap().phase, CollectionPhase::Done);

    // With the setting off again, it waits until the connection isn't metered.
    m.set_download_on_metered(false).unwrap();
    rig.playlist("p1", &["a", "b", "c"]);
    assert!(wait_for(LIMIT, || m.collection_status(Playlist, "p1").unwrap().unwrap().phase == CollectionPhase::Metered));
    // It was complete before, so once resumed it reads as syncing new songs.
    m.set_metered(false);
    assert!(wait_for(LIMIT, || on_device(&m, &["c"])));

    // Offline holds automatic downloads the same way, and reconnecting resumes them.
    m.set_online(false);
    rig.playlist("p1", &["a", "b", "c", "d"]);
    assert!(wait_for(LIMIT, || m.collection_status(Playlist, "p1").unwrap().unwrap().phase == CollectionPhase::Offline));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(off_device(&m, &["d"]));
    m.set_online(true);
    assert!(wait_for(LIMIT, || on_device(&m, &["d"])));
}

#[test]
fn kept_collections_and_the_metered_setting_survive_a_restart() {
    let rig = rig();
    let m = rig.manager();
    rig.playlist("p1", &["a"]);
    m.collection_download_on(Playlist, "p1").unwrap();
    m.set_download_on_metered(true).unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["a"])));
    let since = m.synced_collections().unwrap()[0].since_ms;
    drop(m);

    // The playlist grows while Navia is closed.
    rig.playlist("p1", &["a", "b"]);
    let m = rig.manager();
    assert!(m.download_on_metered());
    let kept = m.synced_collections().unwrap();
    assert_eq!((kept.len(), kept[0].id.as_str(), kept[0].since_ms), (1, "p1", since));
    // Opening syncs at once.
    assert!(wait_for(LIMIT, || on_device(&m, &["b"])));
    assert!(wait_for(LIMIT, || m.collection_status(Playlist, "p1").unwrap().unwrap().phase == CollectionPhase::Done));
}

#[test]
fn a_removed_playlist_releases_its_songs() {
    let rig = rig();
    let m = rig.manager();
    rig.playlist("p1", &["a", "b"]);
    m.collection_download_on(Playlist, "p1").unwrap();
    assert!(wait_for(LIMIT, || on_device(&m, &["a", "b"])));
    // The server no longer lists it.
    rig.server.state.lock().unwrap().canned.insert("getPlaylists".into(), json!({"playlists": {}}));
    rig.rt.block_on(Syncer::new(rig.server.read_only_client(), rig.library.clone(), SyncOptions::default()).sync_playlists()).unwrap();
    assert!(wait_for(LIMIT, || off_device(&m, &["a", "b"])));
    assert!(m.synced_collections().unwrap().is_empty());
}

#[test]
fn kept_smart_playlists_are_fetched_on_playlist_sync_unless_a_test_run_says_not() {
    let rig = rig();
    let m = rig.manager();
    // Listed only, as background sync does for smart playlists, so its songs aren't known yet.
    rig.set_playlist("s1", &["a", "b"], Some("2026-01-01T00:00:05Z"));
    let quiet = Syncer::new(rig.server.read_only_client(), rig.library.clone(), SyncOptions { synced_smart: false, ..SyncOptions::default() });
    rig.rt.block_on(quiet.sync_playlists()).unwrap();
    assert!(rig.library.collection_needs_fetch(Playlist, "s1").unwrap());
    m.collection_download_on(Playlist, "s1").unwrap();
    let status = m.collection_status(Playlist, "s1").unwrap().unwrap();
    assert_eq!((status.phase, status.total, status.have), (CollectionPhase::Waiting, 2, 0));

    // A test run's sync leaves it alone.
    let fetches = rig.server.calls("getPlaylist");
    rig.rt.block_on(quiet.sync_playlists()).unwrap();
    assert_eq!(rig.server.calls("getPlaylist"), fetches, "no evaluation on the server");

    // A normal sync fetches it every time, so its downloads follow the server's evaluation.
    let normal = Syncer::new(rig.server.read_only_client(), rig.library.clone(), SyncOptions::default());
    rig.rt.block_on(normal.sync_playlists()).unwrap();
    assert_eq!(rig.server.calls("getPlaylist"), fetches + 1);
    assert!(wait_for(LIMIT, || on_device(&m, &["a", "b"])));
    rig.set_playlist("s1", &["b"], Some("2026-01-01T00:00:09Z"));
    rig.rt.block_on(normal.sync_playlists()).unwrap();
    assert!(wait_for(LIMIT, || off_device(&m, &["a"]) && on_device(&m, &["b"])));
}
