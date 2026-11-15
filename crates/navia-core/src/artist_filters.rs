//! Filters for the Artists page, built on the same row and state as [`crate::song_filters`].
//! [`ArtistFilterIndex`] holds every artist's facts, built once per library version, so narrowing is an in-memory scan.
//! There's no Save as Smart Playlist because Navidrome's smart playlists are rules over songs.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::client::models::{ArtistRef, NamedRef, lenient_vec};
use crate::client::{Album, Artist};
use crate::song_filters::{ADDED, DAY_MS, FilterId, FilterKind, FilterOption, FilterSource, FilterSpec, FilterState, FilterValue, MatchContext, OptionLabel, PLAYS, list, n_};

/// Every Artists filter, in chip order: the main set, then More Filters.
pub const ARTIST_FILTERS: [FilterSpec; 7] = [
    list(FilterId::Genre, n_("Genre"), true, false, Some(n_("Find Genres"))),
    FilterSpec { id: FilterId::Favorite, label: n_("Favorites"), main: true, kind: FilterKind::Toggle },
    list(FilterId::Plays, n_("Plays"), true, true, None),
    FilterSpec { id: FilterId::Downloaded, label: n_("Downloaded"), main: true, kind: FilterKind::Toggle },
    list(FilterId::Albums, n_("Albums"), false, false, None),
    list(FilterId::Added, n_("Date Added"), false, true, None),
    list(FilterId::Role, n_("Role"), false, true, None),
];

/// "1" is a number, not text to translate.
const ALBUMS: [(&str, &str); 3] = [("1", "1"), ("few", n_("2 to 5")), ("many", n_("6 or More"))];
const ROLES: [(&str, &str); 2] = [("albumartist", n_("Album Artist")), ("appears", n_("Appears On"))];

fn album_range(key: &str) -> Option<(u32, u32)> {
    match key {
        "1" => Some((1, 1)),
        "few" => Some((2, 5)),
        "many" => Some((6, u32::MAX)),
        _ => None,
    }
}

type Name = u32;

/// What the filters read about one artist.
#[derive(Clone, Debug, Default)]
struct Facts {
    id: String,
    /// Every genre of every album they're credited on.
    genres: Vec<Name>,
    starred: bool,
    /// Their songs' play counts, summed.
    plays: i64,
    albums: u32,
    /// The songs crediting them, including the songs on their album-artist albums.
    songs: u32,
    /// When their first album reached the server.
    first_added_ms: Option<u64>,
    /// Whether they have any OpenSubsonic roles, and whether "albumartist" is one.
    has_roles: bool,
    album_artist: bool,
}

/// The song fields the index reads.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SongCredits {
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub artists: Vec<ArtistRef>,
    #[serde(deserialize_with = "lenient_vec")]
    pub album_artists: Vec<ArtistRef>,
    pub play_count: Option<i64>,
}

/// An album as the index reads it.
#[derive(Debug, Default)]
struct AlbumFacts {
    genres: Vec<Name>,
    added_ms: Option<u64>,
    /// Its album artists (artist indexes).
    artists: Vec<u32>,
}

/// Every artist's filter facts. Build it with [`ArtistFilterBuilder`].
#[derive(Clone, Debug, Default)]
pub struct ArtistFilterIndex {
    artists: Vec<Facts>,
    /// Artist id -> index into `artists`.
    by_id: HashMap<String, u32>,
    names: Vec<String>,
    /// Each song's credited artists (indexes), for Downloaded.
    song_artists: HashMap<u32, Box<[u32]>>,
}

/// Builds an [`ArtistFilterIndex`]. Add the artists first, then the albums, then the songs.
#[derive(Default)]
pub struct ArtistFilterBuilder {
    index: ArtistFilterIndex,
    name_ids: HashMap<String, Name>,
    by_id: HashMap<String, u32>,
    albums: HashMap<String, AlbumFacts>,
    /// Each artist's album ids (indexes into `album_ids`), deduplicated at the end.
    artist_albums: Vec<Vec<u32>>,
    album_ids: HashMap<String, u32>,
    album_list: Vec<String>,
}

/// An album's genres, from OpenSubsonic `genres` or else its single genre.
fn album_genres(a: &Album) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in a.genres.iter().map(|g: &NamedRef| g.name.trim()) {
        if !g.is_empty() && !out.iter().any(|n| n == g) {
            out.push(g.to_owned());
        }
    }
    if out.is_empty()
        && let Some(g) = a.genre.as_deref().map(str::trim).filter(|g| !g.is_empty())
    {
        out.push(g.to_owned());
    }
    out
}

impl ArtistFilterBuilder {
    fn name(&mut self, name: &str) -> Name {
        if let Some(&id) = self.name_ids.get(name) {
            return id;
        }
        let id = self.index.names.len() as Name;
        self.index.names.push(name.to_owned());
        self.name_ids.insert(name.to_owned(), id);
        id
    }

    fn album_index(&mut self, id: &str) -> u32 {
        if let Some(&i) = self.album_ids.get(id) {
            return i;
        }
        let i = self.album_list.len() as u32;
        self.album_list.push(id.to_owned());
        self.album_ids.insert(id.to_owned(), i);
        i
    }

    pub fn add_artist(&mut self, a: &Artist) {
        if a.id.is_empty() || self.by_id.contains_key(&a.id) {
            return;
        }
        self.by_id.insert(a.id.clone(), self.index.artists.len() as u32);
        self.index.by_id.insert(a.id.clone(), self.index.artists.len() as u32);
        let roles: Vec<&str> = a.roles.iter().map(|r| r.trim()).filter(|r| !r.is_empty()).collect();
        self.index.artists.push(Facts {
            id: a.id.clone(),
            starred: a.starred.as_deref().is_some_and(|v| !v.is_empty()),
            has_roles: !roles.is_empty(),
            album_artist: roles.iter().any(|r| r.eq_ignore_ascii_case("albumartist")),
            ..Default::default()
        });
        self.artist_albums.push(Vec::new());
    }

    pub fn add_album(&mut self, a: &Album) {
        let genres = album_genres(a).iter().map(|g| self.name(g)).collect();
        let ids: Vec<&str> = if a.artists.is_empty() { a.artist_id.as_deref().into_iter().collect() } else { a.artists.iter().map(|r| r.id.as_str()).collect() };
        let artists: Vec<u32> = ids.into_iter().filter_map(|id| self.by_id.get(id).copied()).collect();
        let at = self.album_index(&a.id);
        for &artist in &artists {
            self.artist_albums[artist as usize].push(at);
        }
        self.albums.insert(a.id.clone(), AlbumFacts { genres, added_ms: a.created.as_deref().and_then(crate::queue_sync::parse_iso8601_ms), artists });
    }

    pub fn add_song(&mut self, rowid: u32, s: &SongCredits) {
        let mut credited: Vec<u32> = Vec::new();
        let ids = if s.artists.is_empty() { s.artist_id.as_deref().into_iter().collect::<Vec<_>>() } else { s.artists.iter().map(|r| r.id.as_str()).collect() };
        for id in ids.into_iter().chain(s.album_artists.iter().map(|r| r.id.as_str())) {
            if let Some(&i) = self.by_id.get(id)
                && !credited.contains(&i)
            {
                credited.push(i);
            }
        }
        if let Some(album) = s.album_id.as_deref().and_then(|id| self.albums.get(id)) {
            for &i in &album.artists {
                if !credited.contains(&i) {
                    credited.push(i);
                }
            }
        }
        if credited.is_empty() {
            return;
        }
        let album = s.album_id.as_deref().filter(|id| self.albums.contains_key(*id)).map(|id| self.album_index(id));
        let plays = s.play_count.unwrap_or(0).max(0);
        for &i in &credited {
            self.index.artists[i as usize].plays += plays;
            self.index.artists[i as usize].songs += 1;
            if let Some(album) = album {
                self.artist_albums[i as usize].push(album);
            }
        }
        self.index.song_artists.insert(rowid, credited.into_boxed_slice());
    }

    pub fn finish(mut self) -> ArtistFilterIndex {
        for (i, mut albums) in std::mem::take(&mut self.artist_albums).into_iter().enumerate() {
            albums.sort_unstable();
            albums.dedup();
            let mut genres: Vec<Name> = Vec::new();
            let mut first: Option<u64> = None;
            for &a in &albums {
                let Some(facts) = self.albums.get(&self.album_list[a as usize]) else { continue };
                for &g in &facts.genres {
                    if !genres.contains(&g) {
                        genres.push(g);
                    }
                }
                if let Some(at) = facts.added_ms {
                    first = Some(first.map_or(at, |f| f.min(at)));
                }
            }
            let f = &mut self.index.artists[i];
            f.albums = albums.len() as u32;
            f.genres = genres;
            f.first_added_ms = first;
        }
        self.index
    }
}

/// The album and song counts shown under an artist in the Artists list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArtistCounts {
    pub albums: u32,
    pub songs: u32,
}

impl ArtistFilterIndex {
    /// An artist's album and song counts, or `None` for an artist the index doesn't know.
    pub fn counts(&self, id: &str) -> Option<ArtistCounts> {
        self.by_id.get(id).map(|&i| {
            let f = &self.artists[i as usize];
            ArtistCounts { albums: f.albums, songs: f.songs }
        })
    }

    /// Every artist's counts, by artist id.
    pub fn all_counts(&self) -> impl Iterator<Item = (&str, ArtistCounts)> {
        self.artists.iter().map(|f| (f.id.as_str(), ArtistCounts { albums: f.albums, songs: f.songs }))
    }

    pub fn len(&self) -> usize {
        self.artists.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artists.is_empty()
    }

    /// Whether any artist has the data a filter reads.
    pub fn has_data(&self, id: FilterId) -> bool {
        let any = |f: &dyn Fn(&Facts) -> bool| self.artists.iter().any(f);
        match id {
            FilterId::Genre => any(&|a| !a.genres.is_empty()),
            FilterId::Albums => any(&|a| a.albums > 0),
            FilterId::Added => any(&|a| a.first_added_ms.is_some()),
            FilterId::Role => any(&|a| a.has_roles),
            FilterId::Favorite | FilterId::Plays | FilterId::Downloaded => !self.artists.is_empty(),
            _ => false,
        }
    }

    /// The filters this library can offer, in chip order.
    pub fn available(&self) -> Vec<FilterId> {
        ARTIST_FILTERS.iter().map(|f| f.id).filter(|&id| self.has_data(id)).collect()
    }

    /// A list filter's choices with their artist counts. Genres are sorted by count, and fixed choices keep their order
    /// with empty ones left out.
    pub fn options(&self, id: FilterId, now_ms: u64) -> Vec<FilterOption> {
        let fixed = |list: &[(&'static str, &'static str)]| -> Vec<FilterOption> {
            list.iter()
                .map(|&(value, label)| {
                    let count = self.artists.iter().filter(|a| fits(a, id, value, now_ms)).count() as u32;
                    let label = if id == FilterId::Albums && value == "1" { OptionLabel::Text(label.to_owned()) } else { OptionLabel::Key(label) };
                    FilterOption { value: value.to_owned(), label, count }
                })
                .filter(|o| o.count > 0)
                .collect()
        };
        match id {
            FilterId::Genre => {
                let mut n: HashMap<Name, u32> = HashMap::new();
                for a in &self.artists {
                    for &g in &a.genres {
                        *n.entry(g).or_default() += 1;
                    }
                }
                let mut all: Vec<(Name, u32)> = n.into_iter().collect();
                all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| self.names[a.0 as usize].to_lowercase().cmp(&self.names[b.0 as usize].to_lowercase())));
                all.into_iter().map(|(g, count)| FilterOption { value: self.names[g as usize].clone(), label: OptionLabel::Text(self.names[g as usize].clone()), count }).collect()
            }
            FilterId::Plays => fixed(&PLAYS),
            FilterId::Albums => fixed(&ALBUMS),
            FilterId::Added => fixed(&ADDED),
            FilterId::Role => fixed(&ROLES),
            _ => Vec::new(),
        }
    }

    /// Ids of the artists every set filter fits. `ctx.downloaded` holds downloaded songs' row ids.
    pub fn matching(&self, state: &FilterState, ctx: &MatchContext) -> HashSet<String> {
        let with_downloads: HashSet<u32> = if state.is_set(FilterId::Downloaded) { ctx.downloaded.iter().filter_map(|r| self.song_artists.get(r)).flat_map(|a| a.iter().copied()).collect() } else { HashSet::new() };
        self.artists
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                state.iter().all(|(id, value)| match (id, value) {
                    (FilterId::Favorite, _) => a.starred,
                    (FilterId::Downloaded, _) => with_downloads.contains(&(*i as u32)),
                    (FilterId::Genre, FilterValue::List(v)) => a.genres.iter().any(|&g| v.iter().any(|x| *x == self.names[g as usize])),
                    (_, FilterValue::List(v)) => v.iter().any(|choice| fits(a, id, choice, ctx.now_ms)),
                    _ => true,
                })
            })
            .map(|(_, a)| a.id.clone())
            .collect()
    }
}

/// Whether an artist fits one fixed choice of a list filter.
fn fits(a: &Facts, id: FilterId, value: &str, now_ms: u64) -> bool {
    match id {
        FilterId::Plays => (value == "never") == (a.plays <= 0),
        FilterId::Albums => album_range(value).is_some_and(|(lo, hi)| a.albums >= lo && a.albums <= hi),
        FilterId::Added => {
            let days: u64 = value.parse().unwrap_or(0);
            a.first_added_ms.is_some_and(|at| now_ms.saturating_sub(at) <= days * DAY_MS)
        }
        FilterId::Role => a.has_roles && a.album_artist == (value == "albumartist"),
        _ => false,
    }
}

impl FilterSource for ArtistFilterIndex {
    fn available(&self) -> Vec<FilterId> {
        ArtistFilterIndex::available(self)
    }
    fn options(&self, id: FilterId, now_ms: u64) -> Vec<FilterOption> {
        ArtistFilterIndex::options(self, id, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artist(id: &str, roles: &[&str], starred: bool) -> Artist {
        serde_json::from_value(json!({"id": id, "name": id, "roles": roles, "starred": if starred { Some("2026-01-01T00:00:00Z") } else { None }})).unwrap()
    }
    fn album(id: &str, artist: &str, genres: &[&str], created: &str) -> Album {
        serde_json::from_value(json!({"id": id, "name": id, "artistId": artist, "artists": [{"id": artist, "name": artist}], "genres": genres.iter().map(|g| json!({"name": g})).collect::<Vec<_>>(), "created": created})).unwrap()
    }
    fn song(album: &str, artists: &[&str], plays: i64) -> SongCredits {
        SongCredits { album_id: Some(album.into()), artists: artists.iter().map(|a| ArtistRef { id: (*a).into(), name: (*a).into() }).collect(), play_count: Some(plays), ..Default::default() }
    }

    /// Artist a is album artist of two rock/pop albums and played. Artist b is only featured on a's album and starred.
    fn index() -> ArtistFilterIndex {
        let mut b = ArtistFilterBuilder::default();
        b.add_artist(&artist("a", &["albumartist", "artist"], false));
        b.add_artist(&artist("b", &["artist"], true));
        b.add_artist(&artist("c", &[], false));
        b.add_album(&album("x", "a", &["Rock", "Pop"], "2026-01-01T00:00:00Z"));
        b.add_album(&album("y", "a", &["Rock"], "2020-01-01T00:00:00Z"));
        b.add_song(1, &song("x", &["a"], 3));
        b.add_song(2, &song("x", &["a", "b"], 0));
        b.add_song(3, &song("y", &["a"], 0));
        b.finish()
    }

    fn ids(set: HashSet<String>) -> Vec<String> {
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    }

    #[test]
    fn genres_albums_and_plays_come_from_credits() {
        let index = index();
        let none = HashSet::new();
        let ctx = MatchContext { downloaded: &none, now_ms: crate::queue_sync::parse_iso8601_ms("2026-01-10T00:00:00Z").unwrap() };
        let mut s = FilterState::default();
        s.toggle_choice(FilterId::Genre, "Pop");
        assert_eq!(ids(index.matching(&s, &ctx)), ["a", "b"], "b appears on the Pop album");
        assert_eq!(index.options(FilterId::Genre, 0).iter().map(|o| (o.value.as_str(), o.count)).collect::<Vec<_>>(), [("Pop", 2), ("Rock", 2)]);
        let mut s = FilterState::default();
        s.toggle_choice(FilterId::Plays, "played");
        assert_eq!(ids(index.matching(&s, &ctx)), ["a"]);
        let mut s = FilterState::default();
        s.toggle_choice(FilterId::Albums, "few");
        assert_eq!(ids(index.matching(&s, &ctx)), ["a"]);
        s.toggle_choice(FilterId::Albums, "1");
        assert_eq!(ids(index.matching(&s, &ctx)), ["a", "b"]);
        // a's oldest album is from 2020, b's only one is from 2026.
        let mut s = FilterState::default();
        s.toggle_choice(FilterId::Added, "30");
        assert_eq!(ids(index.matching(&s, &ctx)), ["b"]);
    }

    #[test]
    fn counts_follow_the_credits() {
        let index = index();
        assert_eq!(index.counts("a"), Some(ArtistCounts { albums: 2, songs: 3 }));
        assert_eq!(index.counts("b"), Some(ArtistCounts { albums: 1, songs: 1 }), "featured on one song of one album");
        assert_eq!(index.counts("c"), Some(ArtistCounts { albums: 0, songs: 0 }));
        assert_eq!(index.counts("nobody"), None);
    }

    #[test]
    fn favorites_downloads_and_roles() {
        let index = index();
        let down: HashSet<u32> = [2].into();
        let ctx = MatchContext { downloaded: &down, now_ms: 0 };
        let mut s = FilterState::default();
        s.set(FilterId::Favorite, Some(FilterValue::On));
        assert_eq!(ids(index.matching(&s, &ctx)), ["b"]);
        let mut s = FilterState::default();
        s.set(FilterId::Downloaded, Some(FilterValue::On));
        assert_eq!(ids(index.matching(&s, &ctx)), ["a", "b"]);
        let mut s = FilterState::default();
        s.toggle_choice(FilterId::Role, "appears");
        assert_eq!(ids(index.matching(&s, &ctx)), ["b"], "c has no roles, so neither choice fits it");
        assert_eq!(index.available(), [FilterId::Genre, FilterId::Favorite, FilterId::Plays, FilterId::Downloaded, FilterId::Albums, FilterId::Added, FilterId::Role]);
    }

    #[test]
    fn missing_data_hides_its_filter() {
        let mut b = ArtistFilterBuilder::default();
        b.add_artist(&artist("a", &[], false));
        let index = b.finish();
        assert_eq!(index.available(), [FilterId::Favorite, FilterId::Plays, FilterId::Downloaded]);
    }
}
