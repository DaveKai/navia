//! Deezer as a source for the Navia mode, only when [`crate::deezer::gate`] is open. All calls
//! go through the shared [`Deezer`] client and its budget, and answers are cached in the
//! library's `deezer_cache`. When Deezer fails or is over budget, older cached answers serve
//! and the server's answers stand alone.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::artist_radio::{match_by_name, owned_artists};
use crate::client::Song;
use crate::deezer::{Deezer, DeezerTrack, store, stored, touch};
use crate::lanyard::matching::key;
use crate::library::Library;
use crate::recommend::artist_key;

const DAY_MS: i64 = 86_400_000;
const ARTIST_KEEP_MS: i64 = 90 * DAY_MS;
const ARTIST_MISS_KEEP_MS: i64 = 7 * DAY_MS;
const RELATED_KEEP_MS: i64 = 7 * DAY_MS;
const TOP_KEEP_MS: i64 = 7 * DAY_MS;
const RADIO_FRESH_MS: i64 = 3_600_000;
const RADIO_DRAWS: usize = 3;
/// Top tracks asked per artist.
pub const TOP_LIMIT: u32 = 50;
/// Neighbor artists whose top tracks are looked up per fetch, besides the anchors' own.
pub const TOP_ARTISTS: usize = 8;
/// Time allowed for the seeds' radios and related artists.
const SEED_TIME: Duration = Duration::from_secs(6);

/// What Deezer added for some seeds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeezerAnswers {
    /// Per seed song id, the library songs from its artist's radio draws, in radio order.
    pub radio: Vec<(String, Vec<Song>)>,
    /// Per seed artist id, the library artist ids Deezer relates to it.
    pub related: HashMap<String, Vec<String>>,
    /// Library song id → its place among its artist's top tracks (0 = most popular).
    pub popular: HashMap<String, usize>,
}

fn now_ms() -> i64 {
    super::playlog::now_ms()
}

/// A cached answer younger than `keep_ms`, else `Err` with any older one.
fn kept(db: &Library, key: &str, keep_ms: i64, now: i64) -> Result<String, Option<String>> {
    match stored(db, key) {
        Some((at, value)) if now - at < keep_ms => {
            touch(db, key, now);
            Ok(value)
        }
        Some((_, value)) => Err(Some(value)),
        None => Err(None),
    }
}

/// The Deezer artist for a library artist's name. Hits are cached 90 days, misses 7.
pub async fn artist(db: &Library, deezer: &Deezer, name: &str, deadline: Instant) -> Option<u64> {
    let name_key = format!("artist:{}", key(name));
    let now = now_ms();
    if let Some((at, value)) = stored(db, &name_key) {
        let id = value.parse::<u64>().ok();
        let keep = if id.is_some() { ARTIST_KEEP_MS } else { ARTIST_MISS_KEEP_MS };
        if now - at < keep {
            touch(db, &name_key, now);
            return id;
        }
    }
    match deezer.artist_id(db, name, deadline).await {
        Ok(found) => {
            store(db, &name_key, &found.map(|id| id.to_string()).unwrap_or_default(), now);
            found
        }
        Err(_) => stored(db, &name_key).and_then(|(_, v)| v.parse().ok()),
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Draw {
    at_ms: i64,
    tracks: Vec<DeezerTrack>,
}

/// The artist's last [`RADIO_DRAWS`] radio draws, newest first, deduplicated. Draws again when
/// the newest is over an hour old.
pub async fn radio(db: &Library, deezer: &Deezer, artist: u64, deadline: Instant) -> Vec<DeezerTrack> {
    let draw_key = format!("radio:{artist}");
    let now = now_ms();
    let mut draws: Vec<Draw> = stored(db, &draw_key).and_then(|(_, v)| serde_json::from_str(&v).ok()).unwrap_or_default();
    if draws.first().is_none_or(|d| now - d.at_ms >= RADIO_FRESH_MS)
        && let Ok(tracks) = deezer.radio_draw(artist, deadline).await
    {
        draws.insert(0, Draw { at_ms: now, tracks });
        draws.truncate(RADIO_DRAWS);
        store(db, &draw_key, &serde_json::to_string(&draws).unwrap_or_default(), now);
    } else if !draws.is_empty() {
        touch(db, &draw_key, now);
    }
    let mut seen = HashSet::new();
    draws.into_iter().flat_map(|d| d.tracks).filter(|t| seen.insert(t.id)).collect()
}

/// Names of the artists Deezer relates to this one, cached 7 days.
pub async fn related(db: &Library, deezer: &Deezer, artist: u64, deadline: Instant) -> Vec<String> {
    let k = format!("related:{artist}");
    let now = now_ms();
    match kept(db, &k, RELATED_KEEP_MS, now) {
        Ok(v) => serde_json::from_str(&v).unwrap_or_default(),
        Err(stale) => match deezer.related_names(artist, deadline).await {
            Ok(names) => {
                store(db, &k, &serde_json::to_string(&names).unwrap_or_default(), now);
                names
            }
            Err(_) => stale.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default(),
        },
    }
}

/// Related names from the cache only, at any age, for rows built without waiting.
pub fn kept_related(db: &Library, artist_name: &str) -> Vec<String> {
    let Some(id) = stored(db, &format!("artist:{}", key(artist_name))).and_then(|(_, v)| v.parse::<u64>().ok()) else { return Vec::new() };
    stored(db, &format!("related:{id}")).and_then(|(_, v)| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// An artist's top tracks, cached 7 days.
pub async fn top(db: &Library, deezer: &Deezer, artist: u64, deadline: Instant) -> Vec<DeezerTrack> {
    let k = format!("top:{artist}");
    let now = now_ms();
    match kept(db, &k, TOP_KEEP_MS, now) {
        Ok(v) => serde_json::from_str(&v).unwrap_or_default(),
        Err(stale) => match deezer.top_tracks(artist, TOP_LIMIT, deadline).await {
            Ok(tracks) => {
                store(db, &k, &serde_json::to_string(&tracks).unwrap_or_default(), now);
                tracks
            }
            Err(_) => stale.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default(),
        },
    }
}

/// A seed's main artist name.
fn artist_name(song: &Song) -> String {
    crate::autoplay::main_artist(song).to_string()
}

/// Each seed's artist radio as library songs, and the library artists related to its artist.
/// Seeds by the same artist share one lookup.
pub async fn seed_sources(db: &Library, deezer: &Deezer, seeds: &[Song]) -> DeezerAnswers {
    let deadline = Instant::now() + SEED_TIME;
    let owned = owned_artists(db);
    let mut out = DeezerAnswers::default();
    let mut by_artist: HashMap<String, (Vec<Song>, Vec<String>)> = HashMap::new();
    for seed in seeds {
        let name = artist_name(seed);
        if name.trim().is_empty() {
            continue;
        }
        if !by_artist.contains_key(&name) {
            let Some(id) = artist(db, deezer, &name, deadline).await else {
                by_artist.insert(name.clone(), Default::default());
                continue;
            };
            let tracks = radio(db, deezer, id, deadline).await;
            let mut taken = HashSet::new();
            let songs: Vec<Song> = match_by_name(db, &tracks, &owned).into_iter().flatten().filter(|s| taken.insert(s.id.clone())).collect();
            let names = related(db, deezer, id, deadline).await;
            let artists: Vec<String> = names.iter().filter_map(|n| db.artist_by_name(n).ok().flatten()).map(|a| a.id).collect();
            by_artist.insert(name.clone(), (songs, artists));
        }
        let (songs, artists) = &by_artist[&name];
        if !songs.is_empty() {
            out.radio.push((seed.id.clone(), songs.clone()));
        }
        if !artists.is_empty() {
            out.related.entry(artist_key(seed)).or_default().extend(artists.iter().cloned());
        }
    }
    for list in out.related.values_mut() {
        let mut seen = HashSet::new();
        list.retain(|a| seen.insert(a.clone()));
    }
    out
}

/// Each library song's rank among these artists' Deezer top tracks, 0 being most popular.
/// Stops at `deadline`.
pub async fn top_places(db: &Library, deezer: &Deezer, names: &[String], deadline: Instant) -> HashMap<String, usize> {
    let mut seen = HashSet::new();
    let owned = owned_artists(db);
    let mut popular = HashMap::new();
    for name in names.iter().filter(|n| !n.trim().is_empty() && seen.insert(key(n))) {
        if Instant::now() >= deadline {
            break;
        }
        let Some(id) = artist(db, deezer, name, deadline).await else { continue };
        let tracks = top(db, deezer, id, deadline).await;
        for (rank, song) in match_by_name(db, &tracks, &owned).into_iter().enumerate() {
            if let Some(song) = song {
                popular.entry(song.id).or_insert(rank);
            }
        }
    }
    popular
}

/// A song's BPM from Deezer by ISRC, cached for good, for songs without a BPM tag. Deezer often
/// says 0, which counts as unknown.
pub async fn bpm(db: &Library, deezer: &Deezer, song: &Song) -> Option<u32> {
    if song.bpm.is_some_and(|b| b > 0) {
        return None;
    }
    let k = format!("bpm:{}", song.id);
    if let Some((_, v)) = stored(db, &k) {
        return v.parse().ok().filter(|b| *b > 0);
    }
    let isrc = song.isrc.first()?;
    let body = deezer.track_by_isrc(isrc, Instant::now() + Duration::from_secs(3)).await.ok()?;
    let value = body.get("bpm").and_then(serde_json::Value::as_f64).map(|b| b.round() as u32).filter(|b| *b > 0);
    store(db, &k, &value.map(|b| b.to_string()).unwrap_or_default(), now_ms());
    value
}

/// A BPM kept by [`bpm`].
pub fn kept_bpm(db: &Library, song_id: &str) -> Option<u32> {
    stored(db, &format!("bpm:{song_id}")).and_then(|(_, v)| v.parse().ok()).filter(|b| *b > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Album, ArtistRef, MusicFolder};
    use crate::library::LibraryConfig;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn song(id: &str, title: &str, artist: &str, secs: i64) -> Song {
        Song { id: id.into(), title: title.into(), artist: Some(artist.into()), artists: vec![ArtistRef { id: format!("ar-{artist}"), name: artist.into() }], artist_id: Some(format!("ar-{artist}")), album_id: Some("al".into()), duration: Some(secs), ..Default::default() }
    }

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        lib.apply_artists("1", &cur, 2, &[crate::client::Artist { id: "ar-Seed Band".into(), name: "Seed Band".into(), ..Default::default() }, crate::client::Artist { id: "ar-Other Act".into(), name: "Other Act".into(), ..Default::default() }]).unwrap();
        lib.apply_albums("1", &cur, 1, &[Album { id: "al".into(), name: "First".into(), ..Default::default() }]).unwrap();
        lib.apply_songs("1", &cur, 3, &[song("alpha", "Alpha", "Seed Band", 200), song("beta", "Beta", "Seed Band", 210), song("gamma", "Gamma", "Other Act", 240)]).unwrap();
        (dir, lib)
    }

    fn track(id: u64, title: &str, artist: &str, secs: u64) -> serde_json::Value {
        json!({"id": id, "title": title, "duration": secs, "artist": {"id": 1, "name": artist}, "album": {"title": "First"}})
    }

    async fn deezer() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/search/artist")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": 1, "name": "Seed Band", "nb_fan": 10}]}))).mount(&server).await;
        Mock::given(method("GET")).and(path("/artist/1/radio")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [track(9002, "Beta", "Seed Band", 211), track(9003, "Gamma - Remastered", "Other Act", 241), track(9009, "Not Here", "Stranger", 200)]}))).mount(&server).await;
        Mock::given(method("GET")).and(path("/artist/1/related")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": 5, "name": "Other Act"}, {"id": 6, "name": "Not In The Library"}]}))).mount(&server).await;
        Mock::given(method("GET")).and(path("/artist/1/top")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [track(9002, "Beta", "Seed Band", 210), track(9001, "Alpha", "Seed Band", 200)]}))).mount(&server).await;
        Mock::given(method("GET")).and(path("/track/isrc:USAAA0000001")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 9001, "bpm": 121.6}))).mount(&server).await;
        server
    }

    fn client(server: &MockServer) -> Deezer {
        Deezer::new(server.uri()).with_timing(Duration::from_millis(1), Duration::from_secs(5))
    }

    async fn paths(server: &MockServer) -> Vec<String> {
        server.received_requests().await.unwrap().iter().map(|r| r.url.path().to_string()).collect()
    }

    #[tokio::test]
    async fn radio_related_and_top_tracks_land_on_library_songs() {
        let (_d, db) = library();
        let server = deezer().await;
        let dz = client(&server);
        let seeds = [song("alpha", "Alpha", "Seed Band", 200)];
        let found = seed_sources(&db, &dz, &seeds).await;
        assert_eq!(found.radio.len(), 1);
        let radio: Vec<&str> = found.radio[0].1.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(radio, ["beta", "gamma"], "by name, the edition note ignored, the stranger left out");
        assert_eq!(found.related["ar-Seed Band"], ["ar-Other Act"], "only library artists");
        let popular = top_places(&db, &dz, &["Seed Band".to_string()], Instant::now() + Duration::from_secs(5)).await;
        assert_eq!((popular["beta"], popular["alpha"]), (0, 1));
    }

    #[tokio::test]
    async fn kept_answers_ask_nothing_again_and_an_old_radio_gets_a_new_draw() {
        let (_d, db) = library();
        let server = deezer().await;
        let dz = client(&server);
        let seeds = [song("alpha", "Alpha", "Seed Band", 200)];
        seed_sources(&db, &dz, &seeds).await;
        let first = paths(&server).await.len();
        assert_eq!(first, 3, "artist, radio, related");
        seed_sources(&db, &dz, &seeds).await;
        assert_eq!(paths(&server).await.len(), first, "all kept");
        // The draw is now over an hour old, so one more is made and the last three kept.
        for _ in 0..4 {
            let (_, v) = stored(&db, "radio:1").unwrap();
            let mut draws: Vec<Draw> = serde_json::from_str(&v).unwrap();
            for d in &mut draws {
                d.at_ms -= 2 * RADIO_FRESH_MS;
            }
            store(&db, "radio:1", &serde_json::to_string(&draws).unwrap(), now_ms());
            seed_sources(&db, &dz, &seeds).await;
        }
        let draws: Vec<Draw> = serde_json::from_str(&stored(&db, "radio:1").unwrap().1).unwrap();
        assert_eq!(draws.len(), RADIO_DRAWS);
        assert_eq!(paths(&server).await.iter().filter(|p| p.ends_with("/radio")).count(), 5);
    }

    #[tokio::test]
    async fn a_failing_deezer_serves_what_was_kept() {
        let (_d, db) = library();
        let server = deezer().await;
        let seeds = [song("alpha", "Alpha", "Seed Band", 200)];
        let kept_radio = seed_sources(&db, &client(&server), &seeds).await;
        // Everything cached is now stale, and Deezer is unreachable.
        let _ = db.write_quiet(|tx| {
            tx.execute("UPDATE deezer_cache SET fetched_ms = fetched_ms - 100 * 86400000", [])?;
            Ok(())
        });
        drop(server);
        let gone = Deezer::new("http://127.0.0.1:9").with_timing(Duration::from_millis(1), Duration::from_secs(2));
        let again = seed_sources(&db, &gone, &seeds).await;
        assert_eq!(again, kept_radio);
    }

    #[tokio::test]
    async fn the_hourly_allowance_and_one_request_for_the_same_question() {
        let (_d, db) = library();
        let server = deezer().await;
        let dz = Deezer::new(server.uri()).with_timing(Duration::from_millis(1), Duration::from_secs(5)).with_hourly_cap(2);
        let deadline = Instant::now() + Duration::from_secs(5);
        // Two concurrent asks make one request.
        let (a, b) = tokio::join!(dz.related_names(1, deadline), dz.related_names(1, deadline));
        assert_eq!(a, b);
        assert_eq!(paths(&server).await.len(), 1);
        assert!(dz.top_tracks(1, 50, deadline).await.is_ok());
        assert_eq!(dz.top_tracks(1, 50, deadline).await, Err(crate::deezer::DeezerError::HourlyCap));
        assert_eq!(paths(&server).await.len(), 2);
        let _ = db;
    }

    #[tokio::test]
    async fn bpm_only_for_a_song_without_a_tag_and_kept_for_good() {
        let (_d, db) = library();
        let server = deezer().await;
        let dz = client(&server);
        let mut tagged = song("alpha", "Alpha", "Seed Band", 200);
        tagged.bpm = Some(98);
        tagged.isrc = vec!["USAAA0000001".into()];
        assert_eq!(bpm(&db, &dz, &tagged).await, None, "has a tag");
        tagged.bpm = None;
        assert_eq!(bpm(&db, &dz, &tagged).await, Some(122));
        assert_eq!(bpm(&db, &dz, &tagged).await, Some(122));
        assert_eq!(kept_bpm(&db, "alpha"), Some(122));
        assert_eq!(paths(&server).await.len(), 1);
        assert_eq!(bpm(&db, &dz, &song("beta", "Beta", "Seed Band", 210)).await, None, "no ISRC: not asked");
        assert_eq!(paths(&server).await.len(), 1);
    }
}
