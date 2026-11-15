//! Album, artist and genre lists by row: a list's order as row ids, then rows on demand.
//! Song lists are in `rows.rs`.

use rusqlite::params_from_iter;
use rusqlite::types::Value;

use super::Library;
use super::rows::SongOrder;
use super::text::fts_query;
use crate::client::{Album, Artist};
use crate::error::{Error, Result};

/// The order of an album list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumOrder {
    MostPlayed,
    RecentlyAdded,
    Name,
    Year,
}

impl AlbumOrder {
    fn clause(self) -> &'static str {
        match self {
            AlbumOrder::MostPlayed => "a.play_count DESC, a.sort_name",
            AlbumOrder::RecentlyAdded => "a.created DESC, a.sort_name",
            AlbumOrder::Name => "a.sort_name",
            AlbumOrder::Year => "a.year DESC, a.sort_name",
        }
    }
}

/// One album list: the row ids in order and how many songs they hold together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlbumList {
    pub rowids: Vec<u32>,
    pub total_songs: u64,
}

/// An artist with what comes from play counts rather than the catalog.
#[derive(Clone, Debug)]
pub struct RankedArtist {
    pub rowid: u32,
    pub artist: Artist,
    pub plays: u64,
    /// Albums whose main artist this is. `Artist::album_count` also counts appearances.
    pub own_albums: u32,
    /// The recent-taste score (`taste.rs`) where the list is ranked by it, otherwise 0.
    pub score: f64,
}

/// A genre and how many songs carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreCount {
    pub name: String,
    pub songs: u64,
}

const GENRE_MATCH_SONG: &str = "(json_extract(s.data, '$.genre') = ? OR EXISTS (SELECT 1 FROM json_each(s.data, '$.genres') g WHERE json_extract(g.value, '$.name') = ?))";
const GENRE_MATCH_ALBUM: &str = "(json_extract(a.data, '$.genre') = ? OR EXISTS (SELECT 1 FROM json_each(a.data, '$.genres') g WHERE json_extract(g.value, '$.name') = ?))";

fn parse<T: serde::de::DeserializeOwned>(data: &str) -> Result<T> {
    serde_json::from_str(data).map_err(|e| Error::Sync(e.to_string()))
}

impl Library {
    /// Song row ids in order, narrowed by a search string, a genre or favorites when given.
    pub fn song_order_filtered(&self, order: SongOrder, query: Option<&str>, genre: Option<&str>, starred_only: bool) -> Result<Vec<u32>> {
        let fts = query.filter(|q| !q.trim().is_empty()).and_then(fts_query);
        let genre = genre.filter(|g| !g.is_empty());
        let mut sql = String::from("SELECT s.rowid FROM ");
        let mut values: Vec<Value> = Vec::new();
        sql.push_str(if fts.is_some() { "fts_song f JOIN song s ON s.rowid = f.rowid WHERE fts_song MATCH ?" } else { "song s WHERE 1" });
        if let Some(fts) = &fts {
            values.push(Value::Text(fts.clone()));
        }
        if let Some(genre) = genre {
            sql.push_str(" AND ");
            sql.push_str(GENRE_MATCH_SONG);
            values.push(Value::Text(genre.to_string()));
            values.push(Value::Text(genre.to_string()));
        }
        if starred_only {
            sql.push_str(" AND s.starred IS NOT NULL");
        }
        sql.push_str(" ORDER BY ");
        sql.push_str(order.clause());
        self.read(|c| {
            let mut st = c.prepare_cached(&sql)?;
            let rows = st.query_map(params_from_iter(values.iter()), |r| r.get::<_, i64>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row? as u32);
            }
            Ok(out)
        })
    }

    /// Album row ids in order, narrowed by a search string, a genre or a main artist when given.
    pub fn album_order(&self, order: AlbumOrder, query: Option<&str>, genre: Option<&str>, artist_id: Option<&str>) -> Result<AlbumList> {
        let fts = query.filter(|q| !q.trim().is_empty()).and_then(fts_query);
        let genre = genre.filter(|g| !g.is_empty());
        let mut sql = String::from("SELECT a.rowid, coalesce(json_extract(a.data, '$.songCount'), 0) FROM ");
        let mut values: Vec<Value> = Vec::new();
        sql.push_str(if fts.is_some() { "fts_album f JOIN album a ON a.rowid = f.rowid WHERE fts_album MATCH ?" } else { "album a WHERE 1" });
        if let Some(fts) = &fts {
            values.push(Value::Text(fts.clone()));
        }
        if let Some(genre) = genre {
            sql.push_str(" AND ");
            sql.push_str(GENRE_MATCH_ALBUM);
            values.push(Value::Text(genre.to_string()));
            values.push(Value::Text(genre.to_string()));
        }
        if let Some(artist) = artist_id {
            sql.push_str(" AND a.artist_id = ?");
            values.push(Value::Text(artist.to_string()));
        }
        sql.push_str(" ORDER BY ");
        sql.push_str(order.clause());
        self.read(|c| {
            let mut st = c.prepare_cached(&sql)?;
            let rows = st.query_map(params_from_iter(values.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            let mut out = AlbumList::default();
            for row in rows {
                let (rowid, songs) = row?;
                out.rowids.push(rowid as u32);
                out.total_songs += songs.max(0) as u64;
            }
            Ok(out)
        })
    }

    /// Albums for these row ids, in the order given. Unknown ids are left out.
    pub fn albums_by_rowid(&self, rowids: &[u32]) -> Result<Vec<(u32, Album)>> {
        self.by_rowid("album", rowids)
    }

    /// Artists for these row ids, in the order given. Unknown ids are left out.
    pub fn artists_by_rowid(&self, rowids: &[u32]) -> Result<Vec<(u32, Artist)>> {
        self.by_rowid("artist", rowids)
    }

    fn by_rowid<T: serde::de::DeserializeOwned>(&self, table: &str, rowids: &[u32]) -> Result<Vec<(u32, T)>> {
        let mut found = std::collections::HashMap::with_capacity(rowids.len());
        self.read(|c| {
            for chunk in rowids.chunks(500) {
                let holders = vec!["?"; chunk.len()].join(",");
                let sql = format!("SELECT rowid, data FROM {table} WHERE rowid IN ({holders})");
                let mut st = c.prepare_cached(&sql)?;
                let rows = st.query_map(params_from_iter(chunk.iter().map(|r| *r as i64)), |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?;
                for row in rows {
                    let (rowid, data) = row?;
                    found.insert(rowid, parse::<T>(&data)?);
                }
            }
            Ok(())
        })?;
        Ok(rowids.iter().filter_map(|r| found.remove(r).map(|t| (*r, t))).collect())
    }

    /// Artists by the plays of their albums, most played first, then by name. `None` for
    /// `limit` returns all of them.
    pub fn ranked_artists(&self, query: Option<&str>, limit: Option<u32>) -> Result<Vec<RankedArtist>> {
        let fts = query.filter(|q| !q.trim().is_empty()).and_then(fts_query);
        let from = if fts.is_some() { "fts_artist f JOIN artist a ON a.rowid = f.rowid" } else { "artist a" };
        let filter = if fts.is_some() { "WHERE fts_artist MATCH ?" } else { "" };
        let tail = if limit.is_some() { " LIMIT ?" } else { "" };
        let sql = format!(
            "SELECT a.rowid, a.data, coalesce(sum(al.play_count), 0) AS plays, count(al.id)
             FROM {from} LEFT JOIN album al ON al.artist_id = a.id {filter}
             GROUP BY a.rowid ORDER BY plays DESC, a.sort_name{tail}"
        );
        let mut values: Vec<Value> = Vec::new();
        if let Some(fts) = &fts {
            values.push(Value::Text(fts.clone()));
        }
        if let Some(limit) = limit {
            values.push(Value::Integer(i64::from(limit)));
        }
        self.read(|c| {
            let mut st = c.prepare_cached(&sql)?;
            let rows = st.query_map(params_from_iter(values.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (rowid, data, plays, albums) = row?;
                out.push(RankedArtist { rowid: rowid as u32, artist: parse(&data)?, plays: plays.max(0) as u64, own_albums: albums.max(0) as u32, score: 0.0 });
            }
            Ok(out)
        })
    }

    /// One artist by name, ignoring case.
    pub fn artist_by_name(&self, name: &str) -> Result<Option<Artist>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT data FROM artist WHERE name = ?1 COLLATE NOCASE LIMIT 1")?;
            let mut rows = st.query_map([name], |r| r.get::<_, String>(0))?;
            rows.next().transpose()?.map(|d| parse(&d)).transpose()
        })
    }

    /// Albums played at least `min_plays` times but not since `played_before` (ISO 8601), the
    /// longest forgotten first.
    pub fn forgotten_albums(&self, min_plays: u32, played_before: &str, limit: u32) -> Result<Vec<Album>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT data FROM album WHERE play_count >= ?1 AND played IS NOT NULL AND played < ?2 ORDER BY played LIMIT ?3")?;
            let rows = st.query_map(rusqlite::params![min_plays, played_before, limit], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(parse(&row?)?);
            }
            Ok(out)
        })
    }

    /// The least-played albums of the artists played most lately. Unplayed artists never count
    /// as top artists.
    pub fn deep_cuts(&self, top_artists: u32, limit: u32) -> Result<Vec<Album>> {
        let taste = self.taste()?;
        let ids: Vec<Value> = taste.artists.iter().take(top_artists as usize).map(|a| Value::Text(a.id.clone())).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let holders = vec!["?"; ids.len()].join(",");
        let sql = format!("SELECT data FROM album WHERE artist_id IN ({holders}) ORDER BY play_count, sort_name LIMIT ?");
        let mut values = ids;
        values.push(Value::Integer(i64::from(limit)));
        self.read(|c| {
            let mut st = c.prepare(&sql)?;
            let rows = st.query_map(params_from_iter(values.iter()), |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(parse(&row?)?);
            }
            Ok(out)
        })
    }

    /// Genres with their song counts, largest first. Read from the songs rather than
    /// `getGenres`, so it works offline and matches the genre lists.
    pub fn genre_counts(&self) -> Result<Vec<GenreCount>> {
        self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT name, count(*) AS songs FROM (
                    SELECT json_extract(g.value, '$.name') AS name FROM song s, json_each(s.data, '$.genres') g
                    UNION ALL
                    SELECT json_extract(s.data, '$.genre') FROM song s WHERE json_extract(s.data, '$.genres') IS NULL
                 ) WHERE name IS NOT NULL AND name != '' GROUP BY name ORDER BY songs DESC, name",
            )?;
            let rows = st.query_map([], |r| Ok(GenreCount { name: r.get(0)?, songs: r.get::<_, i64>(1)?.max(0) as u64 }))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Ids of everything this user has starred, newest first when the server gave dates.
    pub fn starred_ids(&self, table: super::StarTable) -> Result<Vec<String>> {
        let sql = format!("SELECT id FROM {} WHERE starred IS NOT NULL ORDER BY starred DESC, rowid", table.name());
        self.read(|c| {
            let mut st = c.prepare_cached(&sql)?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MusicFolder, NamedRef, Song};
    use crate::library::{LibraryConfig, StarTable};

    fn album(id: &str, name: &str, artist: &str, plays: i64, created: &str) -> Album {
        Album { id: id.into(), name: name.into(), artist: Some(artist.into()), artist_id: Some(artist.to_lowercase()), song_count: Some(10), play_count: Some(plays), created: Some(created.into()), genres: vec![NamedRef { name: "Rock".into() }], ..Default::default() }
    }

    fn setup() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        lib.apply_albums("1", &cur, 2, &[album("a", "Zed", "Ann", 5, "2024-01-02"), album("b", "Aye", "Bob", 9, "2025-06-01"), album("c", "Moo", "Ann", 1, "2020-01-01")]).unwrap();
        lib.apply_artists("1", &cur, 3, &[Artist { id: "ann".into(), name: "Ann".into(), album_count: Some(2), ..Default::default() }, Artist { id: "bob".into(), name: "Bob".into(), album_count: Some(1), ..Default::default() }, Artist { id: "cal".into(), name: "Cal".into(), ..Default::default() }]).unwrap();
        let s = |id: &str, title: &str, genre: &str| Song { id: id.into(), title: title.into(), album_id: Some("a".into()), genres: vec![NamedRef { name: genre.into() }], ..Default::default() };
        lib.apply_songs("1", &cur, 3, &[s("s1", "One", "Rock"), s("s2", "Two", "Jazz"), s("s3", "Three", "Rock")]).unwrap();
        (dir, lib)
    }

    #[test]
    fn album_orders_and_filters() {
        let (_d, lib) = setup();
        let plays = lib.album_order(AlbumOrder::MostPlayed, None, None, None).unwrap();
        assert_eq!(plays.total_songs, 30);
        let rows = lib.albums_by_rowid(&plays.rowids).unwrap();
        assert_eq!(rows.iter().map(|(_, a)| a.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
        let by_ann = lib.album_order(AlbumOrder::Name, None, None, Some("ann")).unwrap();
        assert_eq!(lib.albums_by_rowid(&by_ann.rowids).unwrap().iter().map(|(_, a)| a.name.as_str()).collect::<Vec<_>>(), ["Moo", "Zed"]);
        assert_eq!(lib.album_order(AlbumOrder::Name, None, Some("Jazz"), None).unwrap().rowids.len(), 0);
    }

    #[test]
    fn genre_counts_and_order() {
        let (_d, lib) = setup();
        assert_eq!(lib.genre_counts().unwrap(), vec![GenreCount { name: "Rock".into(), songs: 2 }, GenreCount { name: "Jazz".into(), songs: 1 }]);
        assert_eq!(lib.song_order_filtered(SongOrder::Title, None, Some("Rock"), false).unwrap().len(), 2);
    }

    #[test]
    fn ranked_artists_use_album_plays() {
        let (_d, lib) = setup();
        let ranked = lib.ranked_artists(None, None).unwrap();
        assert_eq!(ranked[0].artist.name, "Bob");
        assert_eq!(ranked[0].plays, 9);
        assert_eq!(ranked[1].own_albums, 2);
        assert_eq!(ranked.len(), 3);
        assert_eq!(lib.artist_by_name("ANN").unwrap().unwrap().id, "ann");
        assert_eq!(lib.ranked_artists(Some("bo"), None).unwrap().len(), 1);
        assert!(lib.starred_ids(StarTable::Artist).unwrap().is_empty());
    }

    #[test]
    fn starred_ids_put_the_newest_like_first() {
        let (_d, lib) = setup();
        let stamp = |id: &str, when: &str| lib.write(|tx| { tx.execute("UPDATE song SET starred = ?1 WHERE id = ?2", [when, id])?; Ok(()) }).unwrap();
        stamp("s1", "2026-02-01T10:00:00Z");
        stamp("s2", "2026-03-01T10:00:00Z");
        assert_eq!(lib.starred_ids(StarTable::Song).unwrap(), ["s2", "s1"]);
        assert_eq!(lib.starred_song_rowids().unwrap().len(), 2);
        assert_eq!(lib.starred_song_rowids().unwrap()[0], lib.song_rowids(&["s2".to_string()]).unwrap()[0]);
    }

    #[test]
    fn shelves_from_play_counts() {
        let (_d, lib) = setup();
        assert!(lib.deep_cuts(2, 5).unwrap().is_empty(), "no song has been played");
        let cur = lib.begin_sync("1", None, false).unwrap();
        let now = crate::library::taste::now_unix();
        let played = |id: &str, album: &str, plays: i64, ago: i64| Song { id: id.into(), title: id.into(), album_id: Some(album.into()), play_count: Some(plays), played: Some(crate::library::extras::iso8601_ms((now - ago) as u64 * 1000)), ..Default::default() };
        lib.apply_songs("1", &cur, 2, &[played("p1", "b", 9, 3600), played("p2", "c", 1, 90 * 86_400)]).unwrap();
        let ids = |cuts: Vec<Album>| cuts.into_iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(lib.deep_cuts(2, 5).unwrap()), ["c", "a", "b"]);
        // Bob was played lately, so he's the one top artist.
        assert_eq!(ids(lib.deep_cuts(1, 5).unwrap()), ["b"]);
    }
}
