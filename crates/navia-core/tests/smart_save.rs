//! Saving smart playlists through the native API against the local fake server.
//! Every refusal must happen before any request is sent.

mod common;

use std::sync::{Arc, Mutex};

use common::{FakeServer, State, song};
use navia_core::client::native::SmartSaveError;
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

const RULES: &str = r#"{"all":[{"is":{"loved":true}}],"sort":"playCount","order":"desc","limit":25.0}"#;

fn header(id: &str, owner: &str, smart: bool) -> Value {
    let mut p = json!({"id": id, "name": format!("List {id}"), "owner": owner, "songCount": 0, "changed": "2026-01-01T00:00:00Z", "readonly": smart || owner != "tester"});
    if smart {
        p["validUntil"] = json!("2026-01-01T00:00:05Z");
    }
    p
}

/// What the native API was sent, as (method, path, JSON body).
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// Answers the native playlist routes from a map of id to (owner name, rules), like Navidrome.
struct Native {
    seen: Seen,
    lists: Arc<Mutex<Vec<(String, String, Value)>>>,
    put_status: u16,
}

impl Respond for Native {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let route = request.url.path().to_string();
        self.seen.lock().unwrap().push((request.method.to_string(), route.clone(), body.clone()));
        if request.headers.get("x-nd-authorization").and_then(|v| v.to_str().ok()) != Some("Bearer abc") {
            return ResponseTemplate::new(401);
        }
        let mut lists = self.lists.lock().unwrap();
        let reply = |(id, owner, rules): &(String, String, Value)| json!({"id": id, "ownerName": owner, "rules": rules, "name": format!("List {id}")});
        match (request.method.as_str(), route.trim_start_matches("/api/playlist").trim_matches('/')) {
            ("POST", "") => {
                assert_eq!(request.headers.get("content-type").unwrap(), "application/json", "Navidrome only takes exactly this type");
                let id = format!("new{}", lists.len());
                lists.push((id.clone(), "tester".into(), body["rules"].clone()));
                ResponseTemplate::new(200).set_body_json(json!({"id": id}))
            }
            ("GET", id) => match lists.iter().find(|l| l.0 == id) {
                Some(l) => ResponseTemplate::new(200).set_body_json(reply(l)),
                None => ResponseTemplate::new(404),
            },
            ("PUT", id) => {
                if self.put_status != 200 {
                    return ResponseTemplate::new(self.put_status).set_body_json(json!({"error": "Invalid request payload"}));
                }
                let Some(l) = lists.iter_mut().find(|l| l.0 == id) else { return ResponseTemplate::new(404) };
                l.2 = body["rules"].clone();
                ResponseTemplate::new(200).set_body_json(reply(l))
            }
            _ => ResponseTemplate::new(405),
        }
    }
}

struct Setup {
    server: FakeServer,
    library: Library,
    seen: Seen,
    _dir: tempfile::TempDir,
}

/// The fake server with these Subsonic playlist headers and native playlists.
async fn setup(headers: &[Value], native: &[(&str, &str, Value)], put_status: u16) -> Setup {
    let state = State { folders: vec![("1".into(), "Music".into())], open_subsonic: true, songs: vec![song("s1", "One", "Art", "Alb", "1")], ..Default::default() };
    let server = FakeServer::start(state).await;
    {
        let mut st = server.state.lock().unwrap();
        st.canned.insert("getPlaylists".into(), json!({"playlists": {"playlist": headers}}));
        for h in headers {
            let id = h["id"].as_str().unwrap();
            let mut full = h.clone();
            full["entry"] = json!([{"id": "s1", "title": "One"}]);
            full["songCount"] = json!(1);
            st.canned.insert(format!("getPlaylist:{id}"), json!({"playlist": full}));
        }
        // What `getPlaylist` answers for a playlist created in the test.
        st.canned.insert("getPlaylist:new1".into(), json!({"playlist": {"id": "new1", "name": "Fresh", "owner": "tester", "readonly": true, "validUntil": "2026-01-01T00:00:05Z", "songCount": 1, "entry": [{"id": "s1", "title": "One"}]}}));
    }
    Mock::given(method("POST")).and(path("/auth/login")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "abc", "id": "u"}))).mount(&server.server).await;
    let seen: Seen = Arc::default();
    let lists = Arc::new(Mutex::new(native.iter().map(|(i, o, r)| (i.to_string(), o.to_string(), r.clone())).collect()));
    Mock::given(path_regex(r"^/api/playlist(/.*)?$")).respond_with(Native { seen: seen.clone(), lists, put_status }).mount(&server.server).await;
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    Syncer::new(server.client(), library.clone(), SyncOptions::default()).sync_playlists().await.unwrap();
    server.state.lock().unwrap().calls.clear();
    Setup { server, library, seen, _dir: dir }
}

async fn logins(s: &Setup) -> usize {
    s.server.server.received_requests().await.unwrap_or_default().iter().filter(|r| r.url.path() == "/auth/login").count()
}

fn old_rules() -> Value {
    json!({"all": [{"contains": {"title": "old"}}]})
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_sends_only_name_and_rules_then_fetches_that_playlist_once() {
    let s = setup(&[header("p1", "tester", true), header("p2", "tester", true)], &[("p1", "tester", old_rules())], 200).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    let saved = sync.save_smart_playlist(Some("p1"), "  Loved Most ", RULES, None).await.unwrap();
    assert_eq!(saved.id, "p1");
    let put = s.seen.lock().unwrap().iter().find(|r| r.0 == "PUT").cloned().unwrap();
    assert_eq!(put.1, "/api/playlist/p1");
    assert_eq!(put.2, json!({"name": "Loved Most", "rules": {"all": [{"is": {"loved": true}}], "sort": "playCount", "order": "desc", "limit": 25}}), "only the fields to change, the limit as a whole number");
    let stored: Value = serde_json::from_str(saved.rules.as_deref().unwrap()).unwrap();
    assert_eq!(stored["limit"], json!(25), "the rules come back as the server stored them");
    let calls = s.server.state.lock().unwrap().calls.clone();
    assert_eq!(calls.iter().filter(|c| c.starts_with("getPlaylist")).count(), 1, "one getPlaylist: {calls:?}");
    assert_eq!(s.server.state.lock().unwrap().calls.iter().filter(|c| *c == "getPlaylists").count(), 0);
    assert_eq!(s.library.playlist("p1").unwrap().unwrap().entry.len(), 1, "the evaluated songs are stored");
    assert!(s.library.playlist("p2").unwrap().unwrap().entry.is_empty(), "other smart playlists aren't fetched");
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_posts_name_and_rules_and_stores_the_new_playlist() {
    let s = setup(&[], &[("p0", "tester", old_rules())], 200).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    let saved = sync.save_smart_playlist(None, "Fresh", RULES, None).await.unwrap();
    assert_eq!(saved.id, "new1");
    let seen = s.seen.lock().unwrap().clone();
    assert_eq!((seen[0].0.as_str(), seen[0].1.as_str()), ("POST", "/api/playlist"));
    assert_eq!(seen[0].2["name"], json!("Fresh"));
    assert_eq!(seen[0].2.as_object().unwrap().len(), 2, "only name and rules");
    assert_eq!((seen[1].0.as_str(), seen[1].1.as_str()), ("GET", "/api/playlist/new1"), "the stored rules are read back");
    assert!(saved.rules.as_deref().unwrap().contains("loved"));
    assert_eq!(s.library.playlist("new1").unwrap().unwrap().name, "Fresh");
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_send_nothing() {
    let headers = [header("mine", "tester", true), header("regular", "tester", false), header("theirs", "friend", true)];
    let native = [("mine", "tester", old_rules()), ("theirs", "friend", old_rules())];
    let s = setup(&headers, &native, 200).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    let read_only = Syncer::new(s.server.read_only_client(), s.library.clone(), SyncOptions::default());
    assert_eq!(read_only.save_smart_playlist(Some("mine"), "A", RULES, None).await, Err(SmartSaveError::WritesDisabled));
    assert_eq!(read_only.save_smart_playlist(None, "A", RULES, None).await, Err(SmartSaveError::WritesDisabled));
    assert!(matches!(sync.save_smart_playlist(Some("mine"), "A", r#"{"all":[]}"#, None).await, Err(SmartSaveError::Invalid(_))));
    assert!(matches!(sync.save_smart_playlist(Some("mine"), "A", r#"{"all":[{"is":{"title":"a","album":"b"}}]}"#, None).await, Err(SmartSaveError::Invalid(_))));
    assert!(matches!(sync.save_smart_playlist(None, "  ", RULES, None).await, Err(SmartSaveError::Invalid(_))));
    assert_eq!(sync.save_smart_playlist(Some("regular"), "A", RULES, None).await, Err(SmartSaveError::NotSmart));
    assert_eq!(sync.save_smart_playlist(Some("theirs"), "A", RULES, Some(false)).await, Err(SmartSaveError::NotAllowed));
    assert_eq!(sync.save_smart_playlist(Some("theirs"), "A", RULES, None).await, Err(SmartSaveError::NotAllowed));
    assert_eq!(sync.save_smart_playlist(Some("gone"), "A", RULES, None).await, Err(SmartSaveError::NotFound));
    assert_eq!(logins(&s).await, 0, "no refusal signs in");
    assert!(s.seen.lock().unwrap().is_empty());
    assert!(s.server.state.lock().unwrap().calls.is_empty(), "and no Subsonic call either");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_may_edit_someone_elses() {
    let s = setup(&[header("theirs", "friend", true)], &[("theirs", "friend", old_rules())], 200).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    sync.save_smart_playlist(Some("theirs"), "Theirs", RULES, Some(true)).await.unwrap();
    assert!(s.seen.lock().unwrap().iter().any(|r| r.0 == "PUT"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_servers_copy_decides_before_anything_changes() {
    // Stored as smart and owned, but the server has no rules for it or another owner, so no PUT.
    let s = setup(&[header("a", "tester", true), header("b", "tester", true)], &[("a", "tester", Value::Null), ("b", "friend", old_rules())], 200).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    assert_eq!(sync.save_smart_playlist(Some("a"), "A", RULES, None).await, Err(SmartSaveError::NotSmart));
    assert_eq!(sync.save_smart_playlist(Some("b"), "B", RULES, None).await, Err(SmartSaveError::NotAllowed));
    assert!(s.seen.lock().unwrap().iter().all(|r| r.0 == "GET"));
    assert_eq!(s.server.state.lock().unwrap().calls.iter().filter(|c| c.starts_with("getPlaylist")).count(), 0, "a refused save never evaluates anything");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_refusal_keeps_its_reason() {
    let s = setup(&[header("p1", "tester", true)], &[("p1", "tester", old_rules())], 422).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    let err = sync.save_smart_playlist(Some("p1"), "A", RULES, None).await.unwrap_err();
    assert_eq!(err, SmartSaveError::Invalid("The server didn't accept these rules: Invalid request payload".into()));
    let s = setup(&[header("p1", "tester", true)], &[("p1", "tester", old_rules())], 403).await;
    let sync = Syncer::new(s.server.client(), s.library.clone(), SyncOptions::default());
    assert_eq!(sync.save_smart_playlist(Some("p1"), "A", RULES, None).await, Err(SmartSaveError::NotAllowed));
}

/// One round on the real server, which must allow temporary test playlists. Creates, edits, checks and
/// deletes its own smart playlist and touches nothing else. Run with
/// `NAVIA_ALLOW_REAL_WRITE=1 cargo test -p navia-core --test smart_save real_server -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_server_temporary_smart_playlist() {
    use navia_core::client::{Client, ClientConfig, Credentials};
    assert_eq!(std::env::var("NAVIA_ALLOW_REAL_WRITE").as_deref(), Ok("1"));
    let env: std::collections::HashMap<String, String> = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.local"))
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect();
    let client = Client::new(ClientConfig::new(env["NAVIDROME_URL"].clone(), Credentials::new(env["NAVIDROME_USER"].clone(), env["NAVIDROME_PASSWORD"].clone()))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&LibraryConfig { path: dir.path().join("l.sqlite") }).unwrap();
    let sync = Syncer::new(client.clone(), library.clone(), SyncOptions::default());
    let name = format!("Navia test smart {}", std::process::id());
    let year = |s: &navia_core::client::Song| s.year.unwrap_or(0);

    let created = sync.save_smart_playlist(None, &name, r#"{"all":[{"inTheRange":{"year":[1990,1999]}}],"sort":"title","limit":3}"#, None).await.unwrap();
    let id = created.id.clone();
    println!("created {id}, stored rules {:?}", created.rules);
    // Nothing may fail between creating and deleting, so the checks are only noted until then.
    let stored = library.playlist(&id).ok().flatten().unwrap_or_default();
    let stored_ok = stored.name == name && stored.owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&env["NAVIDROME_USER"]));
    println!("first rules: {} songs, years {:?}", stored.entry.len(), stored.entry.iter().map(year).collect::<Vec<_>>());
    let first_ok = !stored.entry.is_empty() && stored.entry.len() <= 3 && stored.entry.iter().all(|s| (1990..=1999).contains(&year(s)));

    let edited = sync.save_smart_playlist(Some(&id), &format!("{name} edited"), r#"{"any":[{"inTheRange":{"year":[1980,1989]}},{"is":{"loved":true}}],"sort":"year","order":"asc","limit":5.0}"#, None).await;
    let back = client.playlist_rules(&id).await;
    let after = library.playlist(&id).ok().flatten().unwrap_or_default();
    println!("edited: {edited:?}\nread back: {back:?}\nsecond rules: {} songs, years {:?}, name {}", after.entry.len(), after.entry.iter().map(year).collect::<Vec<_>>(), after.name);

    // Deleted whatever happened above, and only after checking it's this test's own playlist.
    let listed = client.playlists().await.unwrap();
    let mine = listed.iter().find(|p| p.id == id).unwrap();
    assert!(mine.name.starts_with("Navia test smart "));
    client.delete_playlist(&id).await.unwrap();
    assert!(client.playlists().await.unwrap().iter().all(|p| p.id != id), "deleted");

    assert!(stored_ok, "stored with its name and this user as owner");
    assert!(first_ok, "the first rules evaluated to 1-3 songs from the 1990s");
    edited.unwrap();
    let back: Value = serde_json::from_str(&back.unwrap().unwrap()).unwrap();
    assert_eq!((back["limit"].clone(), back["sort"].clone(), back["order"].clone()), (json!(5), json!("year"), json!("asc")));
    assert_eq!(back["any"].as_array().map(Vec::len), Some(2));
    assert_eq!(after.name, format!("{name} edited"));
    assert!(!after.entry.is_empty() && after.entry.len() <= 5);
    assert!(after.entry.windows(2).all(|w| year(&w[0]) <= year(&w[1])), "sorted by year");
}

/// Lists this user's "Navia test smart…" playlists and, with `NAVIA_ALLOW_REAL_WRITE=1`, deletes them.
/// `cargo test -p navia-core --test smart_save real_server_test_playlists -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_server_test_playlists() {
    use navia_core::client::{Client, ClientConfig, Credentials};
    let env: std::collections::HashMap<String, String> = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.local"))
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect();
    let client = Client::new(ClientConfig::new(env["NAVIDROME_URL"].clone(), Credentials::new(env["NAVIDROME_USER"].clone(), env["NAVIDROME_PASSWORD"].clone()))).unwrap();
    let delete = std::env::var("NAVIA_ALLOW_REAL_WRITE").as_deref() == Ok("1");
    let me = env["NAVIDROME_USER"].to_lowercase();
    for p in client.playlists().await.unwrap() {
        let own = p.owner.as_deref().is_some_and(|o| o.to_lowercase() == me);
        if !own || !p.name.starts_with("Navia test smart") {
            continue;
        }
        let rules = client.playlist_rules(&p.id).await;
        println!("{} | {} | songs {:?} | rules {:?}", p.id, p.name, p.song_count, rules);
        if delete {
            client.delete_playlist(&p.id).await.unwrap();
            println!("deleted {}", p.id);
        }
    }
}
