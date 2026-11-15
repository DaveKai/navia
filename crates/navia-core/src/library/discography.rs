//! An artist's albums grouped by release type. Membership comes from Navidrome's list of albums
//! the artist is credited on, or from the library's songs on servers without it.

use std::collections::HashSet;

use rusqlite::params_from_iter;

use super::Library;
use crate::client::{Album, Client};
use crate::error::{Error, Result};

/// One part of the artist page, in page order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReleaseSection {
    Albums,
    SinglesAndEps,
    Compilations,
    /// Albums the artist is credited on without being one of their album artists.
    AppearsOn,
}

impl ReleaseSection {
    pub const ALL: [ReleaseSection; 4] = [ReleaseSection::Albums, ReleaseSection::SinglesAndEps, ReleaseSection::Compilations, ReleaseSection::AppearsOn];
}

/// Where the list of the artist's albums came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscographySource {
    /// The server's list of albums the artist is credited on.
    Server,
    /// The library's own songs and album artists, used when the server list is missing or failed.
    Library,
}

/// One album in a section: its library row and id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscographyAlbum {
    pub rowid: u32,
    pub id: String,
}

/// An artist's albums by section, each newest first. Sections without albums are left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discography {
    pub sections: Vec<(ReleaseSection, Vec<DiscographyAlbum>)>,
    pub source: DiscographySource,
    /// Albums the server listed that this library hasn't stored yet (the next sync brings them).
    pub not_synced: usize,
}

impl Discography {
    pub fn section(&self, section: ReleaseSection) -> &[DiscographyAlbum] {
        self.sections.iter().find(|(s, _)| *s == section).map_or(&[], |(_, a)| a.as_slice())
    }
}

/// Which section an album goes in for this artist. First match wins: Appears On, Compilations,
/// Singles & EPs, then Albums.
pub fn section_for(artist_id: &str, album: &Album) -> ReleaseSection {
    let album_artist = if album.artists.is_empty() { album.artist_id.as_deref() == Some(artist_id) } else { album.artists.iter().any(|a| a.id == artist_id) };
    if !album_artist {
        return ReleaseSection::AppearsOn;
    }
    let has = |kind: &str| album.release_types.iter().any(|t| t.trim().eq_ignore_ascii_case(kind));
    if album.is_compilation == Some(true) || has("compilation") {
        ReleaseSection::Compilations
    } else if has("single") || has("ep") {
        ReleaseSection::SinglesAndEps
    } else {
        ReleaseSection::Albums
    }
}

/// Newest first: the release date's year, else the album year, then the release date, then name.
fn newest_first(a: &Album, b: &Album) -> std::cmp::Ordering {
    let year = |x: &Album| x.release_date.as_ref().and_then(|d| d.year).or(x.year).unwrap_or(0);
    let date = |x: &Album| x.release_date.as_ref().map(|d| (d.month.unwrap_or(0), d.day.unwrap_or(0))).unwrap_or((0, 0));
    let name = |x: &Album| x.sort_name.clone().unwrap_or_else(|| x.name.clone()).to_lowercase();
    year(b).cmp(&year(a)).then(date(b).cmp(&date(a))).then_with(|| name(a).cmp(&name(b)))
}

/// Groups stored albums for `artist_id`, each section newest first.
pub fn group(artist_id: &str, albums: Vec<(u32, Album)>, source: DiscographySource, not_synced: usize) -> Discography {
    let mut buckets: Vec<(ReleaseSection, Vec<(u32, Album)>)> = ReleaseSection::ALL.iter().map(|s| (*s, Vec::new())).collect();
    for (rowid, album) in albums {
        let section = section_for(artist_id, &album);
        buckets.iter_mut().find(|(s, _)| *s == section).expect("every section has a bucket").1.push((rowid, album));
    }
    let sections = buckets
        .into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(section, mut list)| {
            list.sort_by(|a, b| newest_first(&a.1, &b.1));
            (section, list.into_iter().map(|(rowid, a)| DiscographyAlbum { rowid, id: a.id }).collect())
        })
        .collect();
    Discography { sections, source, not_synced }
}

/// Albums the "More by" shelf under an album shows at most.
pub const MORE_BY_SHOWN: usize = 20;
/// The album artist name Navidrome gives compilations without one (its `consts.VariousArtists`).
pub const VARIOUS_ARTISTS: &str = "Various Artists";

/// "More by <album artist>" under an album.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoreBy {
    pub artist_id: String,
    pub artist_name: String,
    /// The album artist's other albums in the library, newest first, at most [`MORE_BY_SHOWN`].
    pub albums: Vec<DiscographyAlbum>,
    /// The artist section "See Discography" opens: the first of theirs these albums are in.
    pub section: ReleaseSection,
}

/// The shelf for an album, newest first and without the album itself. `None` for Various
/// Artists or when nothing else is there.
pub fn more_by(album: &Album, artist_name: &str, others: Vec<(u32, Album)>) -> Option<MoreBy> {
    let artist_id = album.artist_id.clone().filter(|id| !id.is_empty()).or_else(|| album.artists.first().map(|a| a.id.clone()))?;
    if artist_name.trim().eq_ignore_ascii_case(VARIOUS_ARTISTS) {
        return None;
    }
    let mut list: Vec<(u32, Album, ReleaseSection)> = others
        .into_iter()
        .filter(|(_, a)| a.id != album.id)
        .map(|(r, a)| {
            let section = section_for(&artist_id, &a);
            (r, a, section)
        })
        .filter(|(_, _, s)| *s != ReleaseSection::AppearsOn)
        .collect();
    let section = ReleaseSection::ALL.into_iter().find(|s| list.iter().any(|(_, _, x)| x == s))?;
    list.sort_by(|a, b| newest_first(&a.1, &b.1));
    list.truncate(MORE_BY_SHOWN);
    Some(MoreBy { artist_id, artist_name: artist_name.to_string(), albums: list.into_iter().map(|(rowid, a, _)| DiscographyAlbum { rowid, id: a.id }).collect(), section })
}

impl Library {
    /// The artist's albums from the server's id list, plus every stored album naming them as
    /// album artist. Ids not stored yet are counted in `not_synced`.
    pub fn discography_from_ids(&self, artist_id: &str, ids: &[String]) -> Result<Discography> {
        let mut wanted: Vec<String> = Vec::with_capacity(ids.len());
        let mut seen = HashSet::new();
        for id in ids.iter().cloned().chain(self.album_artist_album_ids(artist_id)?) {
            if seen.insert(id.clone()) {
                wanted.push(id);
            }
        }
        let albums = self.albums_by_id(&wanted)?;
        let found: HashSet<&str> = albums.iter().map(|(_, a)| a.id.as_str()).collect();
        let not_synced = ids.iter().filter(|id| !found.contains(id.as_str())).collect::<HashSet<_>>().len();
        Ok(group(artist_id, albums, DiscographySource::Server, not_synced))
    }

    /// "More by" for an album, from the library alone. `None` for an unknown album, Various
    /// Artists, or an album artist with nothing else stored.
    pub fn more_by(&self, album_id: &str) -> Result<Option<MoreBy>> {
        let Some(album) = self.album(album_id)? else { return Ok(None) };
        let Some(artist_id) = album.artist_id.clone().filter(|id| !id.is_empty()).or_else(|| album.artists.first().map(|a| a.id.clone())) else { return Ok(None) };
        let name = self.artist(&artist_id)?.map(|a| a.name).or_else(|| album.artist.clone()).unwrap_or_default();
        let others = self.albums_by_id(&self.album_artist_album_ids(&artist_id)?)?;
        Ok(more_by(&album, &name, others))
    }

    /// The artist's albums from the library alone, matching the same credits Navidrome's list
    /// does: album artists and song `artists`.
    pub fn library_discography(&self, artist_id: &str) -> Result<Discography> {
        let mut ids = self.album_artist_album_ids(artist_id)?;
        let mut seen: HashSet<String> = ids.iter().cloned().collect();
        let from_songs: Vec<String> = self.read(|c| {
            // `instr` skips the JSON parse for the many songs that can't name the artist.
            let mut st = c.prepare_cached(
                "SELECT DISTINCT s.album_id FROM song s, json_each(s.data, '$.artists') j
                 WHERE instr(s.data, ?1) > 0 AND json_extract(j.value, '$.id') = ?1 AND s.album_id IS NOT NULL",
            )?;
            let rows = st.query_map([artist_id], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Error::from)
        })?;
        ids.extend(from_songs.into_iter().filter(|id| seen.insert(id.clone())));
        let albums = self.albums_by_id(&ids)?;
        Ok(group(artist_id, albums, DiscographySource::Library, 0))
    }

    /// Ids of stored albums with the artist among their album artists.
    fn album_artist_album_ids(&self, artist_id: &str) -> Result<Vec<String>> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT a.id FROM album a WHERE a.artist_id = ?1
                 UNION SELECT a.id FROM album a, json_each(a.data, '$.artists') j
                 WHERE instr(a.data, ?1) > 0 AND json_extract(j.value, '$.id') = ?1",
            )?;
            let rows = st.query_map([artist_id], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Error::from)
        })
    }

    /// Stored albums with these ids, with their rows. Unknown ids are left out.
    fn albums_by_id(&self, ids: &[String]) -> Result<Vec<(u32, Album)>> {
        self.read(|c| {
            let mut out = Vec::with_capacity(ids.len());
            for chunk in ids.chunks(500) {
                let holders = vec!["?"; chunk.len()].join(",");
                let mut st = c.prepare_cached(&format!("SELECT rowid, data FROM album WHERE id IN ({holders})"))?;
                let rows = st.query_map(params_from_iter(chunk.iter()), |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?;
                for row in rows {
                    let (rowid, data) = row?;
                    out.push((rowid, serde_json::from_str::<Album>(&data).map_err(|e| Error::Sync(e.to_string()))?));
                }
            }
            Ok(out)
        })
    }
}

/// Whether a run may ask the server's list. Asking logs in and moves the account's last-login
/// time, so test runs skip it unless `NAVIA_TEST_NATIVE=1` is set.
pub fn server_list_allowed(test_run: bool) -> bool {
    !test_run || std::env::var_os("NAVIA_TEST_NATIVE").is_some_and(|v| v == "1")
}

/// The artist's discography from the server's list when allowed, falling back to the library.
pub async fn artist_discography(client: &Client, library: &Library, artist_id: &str, use_server: bool) -> Result<Discography> {
    if use_server {
        match client.credited_album_ids(artist_id).await {
            Ok(ids) => return library.discography_from_ids(artist_id, &ids),
            Err(e) => {
                if client.lists_credited_albums().await.unwrap_or(false) {
                    eprintln!("navia: the server's album list for an artist failed, using the library's: {e}");
                }
            }
        }
    }
    library.library_discography(artist_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_by_leaves_out_the_album_and_appearances_newest_first() {
        let this = album("this", &["me"], &["album"], None, 2019);
        let others = vec![
            (1, this.clone()),
            (2, album("old", &["me"], &["album"], None, 2001)),
            (3, album("single", &["me"], &["single"], None, 2024)),
            (4, album("guest", &["other"], &["album"], None, 2023)),
        ];
        let m = more_by(&this, "Me", others).unwrap();
        assert_eq!(m.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["single", "old"]);
        assert_eq!(m.section, ReleaseSection::Albums);
        let only_singles = vec![(3, album("single", &["me"], &["single"], None, 2024))];
        assert_eq!(more_by(&this, "Me", only_singles).unwrap().section, ReleaseSection::SinglesAndEps);
    }

    #[test]
    fn more_by_hides_for_various_artists_and_when_alone() {
        let this = album("this", &["va"], &["compilation"], Some(true), 2019);
        let others = vec![(2, album("other", &["va"], &["compilation"], Some(true), 2020))];
        assert!(more_by(&this, "Various Artists", others).is_none());
        let solo = album("solo", &["me"], &["album"], None, 2019);
        assert!(more_by(&solo, "Me", vec![(1, solo.clone())]).is_none());
    }

    fn album(id: &str, artists: &[&str], types: &[&str], compilation: Option<bool>, year: i32) -> Album {
        let value = serde_json::json!({
            "id": id, "name": id, "year": year,
            "artistId": artists.first().copied(),
            "artists": artists.iter().map(|a| serde_json::json!({"id": a, "name": a})).collect::<Vec<_>>(),
            "releaseTypes": types, "isCompilation": compilation,
        });
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn first_match_wins() {
        assert_eq!(section_for("me", &album("a", &["other"], &["single"], Some(true), 2020)), ReleaseSection::AppearsOn);
        assert_eq!(section_for("me", &album("a", &["me", "other"], &["album"], Some(true), 2020)), ReleaseSection::Compilations);
        assert_eq!(section_for("me", &album("a", &["me"], &["album", "compilation"], None, 2020)), ReleaseSection::Compilations);
        assert_eq!(section_for("me", &album("a", &["me"], &["Single"], None, 2020)), ReleaseSection::SinglesAndEps);
        assert_eq!(section_for("me", &album("a", &["me"], &["ep", "live"], Some(false), 2020)), ReleaseSection::SinglesAndEps);
        assert_eq!(section_for("me", &album("a", &["me"], &["single", "album"], None, 2020)), ReleaseSection::SinglesAndEps);
        assert_eq!(section_for("me", &album("a", &["me"], &[], None, 2020)), ReleaseSection::Albums);
        assert_eq!(section_for("me", &album("a", &["me"], &["album", "soundtrack"], None, 2020)), ReleaseSection::Albums);
    }

    #[test]
    fn album_artist_falls_back_to_the_single_field() {
        let mut a = album("a", &[], &["album"], None, 2020);
        a.artist_id = Some("me".to_string());
        assert_eq!(section_for("me", &a), ReleaseSection::Albums);
        a.artist_id = Some("other".to_string());
        assert_eq!(section_for("me", &a), ReleaseSection::AppearsOn);
    }

    #[test]
    fn sections_are_in_page_order_newest_first_and_empty_ones_hide() {
        let albums = vec![
            (1, album("old", &["me"], &["album"], None, 2001)),
            (2, album("feat", &["other"], &["album"], None, 2015)),
            (3, album("new", &["me"], &[], None, 2020)),
            (4, album("one", &["me"], &["single"], None, 2010)),
        ];
        let d = group("me", albums, DiscographySource::Library, 0);
        let order: Vec<ReleaseSection> = d.sections.iter().map(|(s, _)| *s).collect();
        assert_eq!(order, vec![ReleaseSection::Albums, ReleaseSection::SinglesAndEps, ReleaseSection::AppearsOn]);
        let ids: Vec<&str> = d.section(ReleaseSection::Albums).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert!(d.section(ReleaseSection::Compilations).is_empty());
    }

    fn stored(albums: Vec<Album>, songs: Vec<crate::client::Song>) -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&super::super::LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[crate::client::MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        lib.apply_albums("1", &cur, albums.len() as u32, &albums).unwrap();
        lib.apply_songs("1", &cur, songs.len() as u32, &songs).unwrap();
        (dir, lib)
    }

    fn song(id: &str, album: &str, artists: &[&str]) -> crate::client::Song {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "albumId": album, "artistId": artists[0],
            "artists": artists.iter().map(|a| serde_json::json!({"id": a, "name": a})).collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    fn sample() -> (tempfile::TempDir, Library) {
        stored(
            vec![
                album("mine", &["me"], &["album"], None, 2020),
                album("duet", &["me", "friend"], &["single"], None, 2022),
                album("hits", &["me"], &["compilation"], Some(true), 2018),
                album("theirs", &["friend"], &["album"], None, 2019),
                album("unrelated", &["friend"], &["album"], None, 2021),
            ],
            vec![song("s1", "mine", &["me"]), song("s2", "theirs", &["friend", "me"]), song("s3", "unrelated", &["friend"])],
        )
    }

    fn ids(d: &Discography, section: ReleaseSection) -> Vec<&str> {
        d.section(section).iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn library_discography_takes_album_artists_and_song_credits() {
        let (_d, lib) = sample();
        let d = lib.library_discography("me").unwrap();
        assert_eq!(d.source, DiscographySource::Library);
        assert_eq!(ids(&d, ReleaseSection::Albums), ["mine"]);
        assert_eq!(ids(&d, ReleaseSection::SinglesAndEps), ["duet"]);
        assert_eq!(ids(&d, ReleaseSection::Compilations), ["hits"]);
        assert_eq!(ids(&d, ReleaseSection::AppearsOn), ["theirs"]);
        let all: Vec<u32> = d.sections.iter().flat_map(|(_, a)| a.iter().map(|x| x.rowid)).collect();
        assert_eq!(all.len(), all.iter().collect::<HashSet<_>>().len(), "no album twice");
    }

    #[test]
    fn server_ids_add_to_own_albums_and_unsynced_ones_are_counted() {
        let (_d, lib) = sample();
        let d = lib.discography_from_ids("me", &["theirs".to_string(), "unrelated".to_string(), "later".to_string(), "later".to_string()]).unwrap();
        assert_eq!(d.source, DiscographySource::Server);
        assert_eq!(ids(&d, ReleaseSection::AppearsOn), ["unrelated", "theirs"], "newest first");
        assert_eq!(ids(&d, ReleaseSection::Albums), ["mine"], "own albums stay even when the server's list leaves them out");
        assert_eq!(d.not_synced, 1);
    }

    mod server {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

        fn ping(version: &str) -> ResponseTemplate {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"subsonic-response": {"status": "ok", "version": "1.16.1", "type": "navidrome", "serverVersion": version, "openSubsonic": true}}))
        }

        /// Native album list in pages of two. The first token is refused once to act as expired.
        struct Albums {
            ids: Vec<&'static str>,
            expire_first: Arc<AtomicUsize>,
        }

        impl Respond for Albums {
            fn respond(&self, request: &Request) -> ResponseTemplate {
                let q = |k: &str| request.url.query_pairs().find(|(a, _)| a == k).map(|(_, v)| v.into_owned());
                assert_eq!(q("artist_id").as_deref(), Some("me"));
                assert_eq!(q("missing").as_deref(), Some("false"));
                let auth = request.headers.get("x-nd-authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                if auth == "Bearer t1" && self.expire_first.fetch_add(1, Ordering::SeqCst) == 0 {
                    return ResponseTemplate::new(401);
                }
                let start: usize = q("_start").unwrap().parse().unwrap();
                let end: usize = q("_end").unwrap().parse().unwrap();
                let page: Vec<serde_json::Value> = self.ids.iter().skip(start).take(end - start).map(|i| serde_json::json!({"id": i, "name": i})).collect();
                ResponseTemplate::new(200).insert_header("x-total-count", self.ids.len().to_string().as_str()).insert_header("x-nd-authorization", "t2").set_body_json(page)
            }
        }

        async fn mock(version: &str, expire: bool) -> (MockServer, Arc<AtomicUsize>) {
            let server = MockServer::start().await;
            Mock::given(path("/rest/ping")).respond_with(ping(version)).mount(&server).await;
            let logins = Arc::new(AtomicUsize::new(0));
            let counter = logins.clone();
            Mock::given(method("POST"))
                .and(path("/auth/login"))
                .respond_with(move |_: &Request| {
                    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": format!("t{n}")}))
                })
                .mount(&server)
                .await;
            let expire_first = Arc::new(AtomicUsize::new(if expire { 0 } else { 1 }));
            Mock::given(path("/api/album")).respond_with(Albums { ids: vec!["theirs", "mine", "later"], expire_first }).mount(&server).await;
            (server, logins)
        }

        fn client(server: &MockServer) -> Client {
            Client::new(crate::client::ClientConfig::new(server.uri(), crate::client::Credentials::new("tester", "pw")).read_only()).unwrap()
        }

        #[tokio::test]
        async fn the_server_list_is_used_with_one_login_kept() {
            let (server, logins) = mock("0.64.2 (abc)", false).await;
            let (_d, lib) = sample();
            let c = client(&server);
            let d = artist_discography(&c, &lib, "me", true).await.unwrap();
            assert_eq!(d.source, DiscographySource::Server);
            assert_eq!(ids(&d, ReleaseSection::AppearsOn), ["theirs"]);
            assert_eq!(d.not_synced, 1);
            artist_discography(&c, &lib, "me", true).await.unwrap();
            assert_eq!(logins.load(Ordering::SeqCst), 1, "the token is kept between pages and visits");
        }

        #[tokio::test]
        async fn an_expired_token_logs_in_again() {
            let (server, logins) = mock("0.64.2", true).await;
            let (_d, lib) = sample();
            let d = artist_discography(&client(&server), &lib, "me", true).await.unwrap();
            assert_eq!(d.source, DiscographySource::Server);
            assert_eq!(logins.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn older_servers_and_test_runs_use_the_library_without_logging_in() {
            for (version, use_server) in [("0.54.5", true), ("0.64.2", false)] {
                let (server, logins) = mock(version, false).await;
                let (_d, lib) = sample();
                let d = artist_discography(&client(&server), &lib, "me", use_server).await.unwrap();
                assert_eq!(d.source, DiscographySource::Library);
                assert_eq!(ids(&d, ReleaseSection::AppearsOn), ["theirs"]);
                assert_eq!(logins.load(Ordering::SeqCst), 0);
                let native = server.received_requests().await.unwrap().iter().filter(|r| r.url.path().starts_with("/api/")).count();
                assert_eq!(native, 0);
            }
        }
    }
}
