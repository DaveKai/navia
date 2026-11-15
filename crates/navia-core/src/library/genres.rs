//! The Genres page, read from the synced library so it works offline. Genre names are the
//! server's tag values, never split or merged.

use std::collections::{HashMap, HashSet};

use super::Library;
use crate::error::Result;

/// An album a genre can show as a cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreCover {
    pub rowid: u32,
    pub id: String,
    pub cover_art: String,
}

/// One genre on the Genres page.
#[derive(Clone, Debug, PartialEq)]
pub struct GenreOverview {
    /// Exactly as the server tags it.
    pub name: String,
    /// Albums tagged with it.
    pub albums: u32,
    /// The recent-taste score of its albums' plays, then their last-played score.
    pub taste: f64,
    pub taste_fallback: f64,
    /// Whether any of its albums has ever been played.
    pub played: bool,
    /// Its albums with cover art, most played lately first, then newest. Pick with
    /// [`pick_genre_covers`].
    pub covers: Vec<GenreCover>,
}

/// "Your Top Genres" shows at most this many, and only played ones.
pub const TOP_GENRES: usize = 12;
/// Covers fanned on a top genre's tile.
pub const TILE_COVERS: usize = 3;

/// A genre as the page shows it: its name, album count and the covers picked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreEntry {
    pub name: String,
    pub albums: u32,
    pub covers: Vec<GenreCover>,
}

/// The whole Genres page: the top tiles, then every genre as a row. `browse` is Search's
/// "Browse by Genre" shelf, holding the genres the top tiles don't show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenrePage {
    pub top: Vec<GenreEntry>,
    pub all: Vec<GenreEntry>,
    pub browse: Vec<GenreEntry>,
}

impl Library {
    /// The Genres page from the synced library. Cheap enough to call once per visit or sync,
    /// off the UI thread, without caching.
    pub fn genre_page(&self) -> Result<GenrePage> {
        Ok(genre_page_of(&self.genre_overview()?))
    }

    /// Every genre on the albums, in page order.
    pub fn genre_overview(&self) -> Result<Vec<GenreOverview>> {
        struct Row {
            rowid: u32,
            id: String,
            cover: Option<String>,
            created: String,
            genre: String,
        }
        let rows: Vec<Row> = self.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT DISTINCT a.rowid, a.id, json_extract(a.data, '$.coverArt'), coalesce(a.created, ''), g.name FROM album a JOIN (
                    SELECT a2.rowid AS rowid, json_extract(j.value, '$.name') AS name FROM album a2, json_each(a2.data, '$.genres') j
                    UNION ALL
                    SELECT a3.rowid, json_extract(a3.data, '$.genre') FROM album a3 WHERE json_extract(a3.data, '$.genres') IS NULL
                 ) g ON g.rowid = a.rowid WHERE g.name IS NOT NULL AND g.name != ''",
            )?;
            let rows = st.query_map([], |r| {
                Ok(Row { rowid: r.get::<_, i64>(0)? as u32, id: r.get(1)?, cover: r.get(2)?, created: r.get(3)?, genre: r.get(4)? })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })?;
        let taste = self.taste()?;
        let scores: HashMap<u32, (f64, f64)> = taste.albums.iter().map(|a| (a.rowid, (a.score, a.fallback))).collect();

        struct Acc<'a> {
            albums: u32,
            taste: f64,
            fallback: f64,
            played: bool,
            covers: Vec<(&'a Row, f64, f64)>,
        }
        let mut by_name: HashMap<&str, Acc> = HashMap::new();
        for row in &rows {
            let acc = by_name.entry(row.genre.as_str()).or_insert(Acc { albums: 0, taste: 0.0, fallback: 0.0, played: false, covers: Vec::new() });
            acc.albums += 1;
            let (score, fallback) = scores.get(&row.rowid).copied().unwrap_or((0.0, 0.0));
            if scores.contains_key(&row.rowid) {
                acc.played = true;
                acc.taste += score;
                acc.fallback += fallback;
            }
            if row.cover.as_deref().is_some_and(|c| !c.is_empty()) {
                acc.covers.push((row, score, fallback));
            }
        }
        let mut out: Vec<GenreOverview> = by_name
            .into_iter()
            .map(|(name, mut acc)| {
                acc.covers.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.total_cmp(&a.2)).then_with(|| b.0.created.cmp(&a.0.created)).then(a.0.rowid.cmp(&b.0.rowid)));
                GenreOverview {
                    name: name.to_string(),
                    albums: acc.albums,
                    taste: acc.taste,
                    taste_fallback: acc.fallback,
                    played: acc.played,
                    covers: acc.covers.into_iter().map(|(r, _, _)| GenreCover { rowid: r.rowid, id: r.id.clone(), cover_art: r.cover.clone().unwrap_or_default() }).collect(),
                }
            })
            .collect();
        sort_genres(&mut out);
        Ok(out)
    }
}

/// The covers each genre shows, in page order, preferring albums no earlier genre has shown so
/// one album tagged with many genres doesn't front every tile. With `unique` an album never repeats.
pub fn pick_genre_covers<'a>(genres: impl IntoIterator<Item = &'a GenreOverview>, limit: usize, unique: bool) -> Vec<Vec<GenreCover>> {
    pick_genre_covers_after(&mut HashSet::new(), genres, limit, unique)
}

/// [`pick_genre_covers`], given the albums already shown in `used`. Adds the ones it picks.
fn pick_genre_covers_after<'a>(used: &mut HashSet<u32>, genres: impl IntoIterator<Item = &'a GenreOverview>, limit: usize, unique: bool) -> Vec<Vec<GenreCover>> {
    genres
        .into_iter()
        .map(|g| {
            let fresh = g.covers.iter().filter(|c| !used.contains(&c.rowid));
            let seen = g.covers.iter().filter(|c| used.contains(&c.rowid)).take(if unique { 0 } else { limit });
            let picked: Vec<GenreCover> = fresh.chain(seen).take(limit).cloned().collect();
            used.extend(picked.iter().map(|c| c.rowid));
            picked
        })
        .collect()
}

/// The page for genres already in page order, so a shell with its own data uses the same rules.
pub fn genre_page_of(all: &[GenreOverview]) -> GenrePage {
    let top = top_genres(all);
    let entry = |g: &GenreOverview, covers: Vec<GenreCover>| GenreEntry { name: g.name.clone(), albums: g.albums, covers };
    let tiles = pick_genre_covers(top.iter().copied(), TILE_COVERS, true);
    let rows = pick_genre_covers(all.iter(), 1, false);
    let mut used: HashSet<u32> = tiles.iter().flatten().map(|c| c.rowid).collect();
    let mut rest: Vec<&GenreOverview> = all.iter().filter(|g| !top.iter().any(|t| t.name == g.name)).collect();
    rest.sort_by(|a, b| b.albums.cmp(&a.albums).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name)));
    let browse = pick_genre_covers_after(&mut used, rest.iter().copied(), TILE_COVERS, false);
    GenrePage {
        top: top.iter().zip(tiles).map(|(g, c)| entry(g, c)).collect(),
        all: all.iter().zip(rows).map(|(g, c)| entry(g, c)).collect(),
        browse: rest.iter().zip(browse).map(|(g, c)| entry(g, c)).collect(),
    }
}

/// The order of the page: played genres by recent taste, then the rest by album count, then by name.
pub fn sort_genres(all: &mut [GenreOverview]) {
    all.sort_by(|a, b| {
        b.played
            .cmp(&a.played)
            .then(b.taste.total_cmp(&a.taste))
            .then(b.taste_fallback.total_cmp(&a.taste_fallback))
            .then(b.albums.cmp(&a.albums))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// The genres "Your Top Genres" shows: played ones, best first, at most [`TOP_GENRES`].
pub fn top_genres(all: &[GenreOverview]) -> Vec<&GenreOverview> {
    all.iter().filter(|g| g.played).take(TOP_GENRES).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Album, MusicFolder, NamedRef, Song};
    use crate::library::LibraryConfig;

    fn album(id: &str, created: &str, genres: &[&str], art: bool) -> Album {
        Album {
            id: id.into(),
            name: id.into(),
            artist: Some("Ann".into()),
            artist_id: Some("ann".into()),
            created: Some(created.into()),
            cover_art: art.then(|| format!("al-{id}")),
            genres: genres.iter().map(|g| NamedRef { name: (*g).into() }).collect(),
            ..Default::default()
        }
    }

    fn setup() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        let albums = [
            album("old", "2001-01-01", &["Rock", "Pop"], true),
            album("new", "2024-01-01", &["Rock"], true),
            album("mid", "2010-01-01", &["Rock", "Hip Hop"], true),
            album("bare", "2025-01-01", &["Rock"], false),
            album("jazz", "2020-01-01", &["Jazz"], true),
            album("jazz2", "2021-01-01", &["Jazz"], true),
            album("none", "2020-01-01", &[], true),
        ];
        lib.apply_albums("1", &cur, albums.len() as u32, &albums).unwrap();
        // One played song, on "old" (Rock and Pop), played yesterday.
        let yesterday = crate::library::taste::now_unix() - 86_400;
        let played = Song { id: "s1".into(), title: "S".into(), album_id: Some("old".into()), play_count: Some(4), played: Some(crate::library::extras::iso8601_ms(yesterday as u64 * 1000)), ..Default::default() };
        lib.apply_songs("1", &cur, 1, &[played]).unwrap();
        (dir, lib)
    }

    #[test]
    fn overview_orders_played_first_then_album_count() {
        let (_d, lib) = setup();
        let all = lib.genre_overview().unwrap();
        let names: Vec<&str> = all.iter().map(|g| g.name.as_str()).collect();
        // Rock and Pop hold the played album; Rock has more albums so wins the tie on taste.
        assert_eq!(names, ["Rock", "Pop", "Jazz", "Hip Hop"]);
        let rock = &all[0];
        assert_eq!(rock.albums, 4);
        assert!(rock.played && !all[2].played);
        // The played album first, then newest; the one without art is left out.
        assert_eq!(rock.covers.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["old", "new", "mid"]);
        assert_eq!(top_genres(&all).iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Rock", "Pop"]);
    }

    #[test]
    fn covers_never_repeat_on_tiles_and_fall_back_on_rows() {
        let (_d, lib) = setup();
        let all = lib.genre_overview().unwrap();
        let tiles = pick_genre_covers(all.iter(), TILE_COVERS, true);
        assert_eq!(tiles[0].iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["old", "new", "mid"]);
        // Pop's only album is on Rock's tile already.
        assert!(tiles[1].is_empty());
        let rows = pick_genre_covers(all.iter(), 1, false);
        assert_eq!(rows.iter().map(|r| r[0].id.as_str()).collect::<Vec<_>>(), ["old", "old", "jazz2", "mid"]);
        let page = lib.genre_page().unwrap();
        assert_eq!(page.top.iter().map(|g| (g.name.as_str(), g.covers.len())).collect::<Vec<_>>(), [("Rock", 3), ("Pop", 0)]);
        assert_eq!(page.all.len(), 4);
        assert!(page.all.iter().all(|g| g.covers.len() == 1));
        // Browse by Genre: the genres the top tiles don't show, by album count.
        assert_eq!(page.browse.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Jazz", "Hip Hop"]);
        assert_eq!(page.browse[0].covers.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["jazz2", "jazz"]);
        assert_eq!(page.browse[1].covers.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["mid"]);
    }
}
