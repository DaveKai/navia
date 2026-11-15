//! The Songs page's filters. Each reads one song field, or a value derived from it, and is hidden when no song has it.
//! [`FilterIndex`] holds every song's facts in memory, built once from the library and shared by both apps.
//! Labels are English catalog keys, which `tools/i18n.py` collects through [`n_`].

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::audio_quality::{FileFacts, Tier};
use crate::client::Song;
use crate::client::models::is_explicit;

/// Marks English text for the translation catalog. The apps show it through their own `tr`.
pub const fn n_(key: &'static str) -> &'static str {
    key
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FilterId {
    Genre,
    Artist,
    Year,
    Favorite,
    Plays,
    Added,
    Quality,
    Downloaded,
    AlbumArtist,
    LastPlayed,
    Format,
    Bitrate,
    Length,
    Explicit,
    Bpm,
    Rating,
    /// The Artists page's album count, and album artist or appears on.
    Albums,
    Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    /// Pick any of a list, ORed within the filter. `single` allows one pick at most.
    /// `find` is the find field's placeholder. Short lists without one are drawn as chips.
    List { single: bool, find: Option<&'static str> },
    /// From and To, typed with at most `digits` digits, with quick picks where there are any.
    Range { digits: usize },
    /// On or off, switched by its chip.
    Toggle,
}

/// Lists with at most this many choices and no find field are drawn as chips.
pub const CHIPS_UP_TO: usize = 6;

#[derive(Clone, Copy, Debug)]
pub struct FilterSpec {
    pub id: FilterId,
    /// English catalog key.
    pub label: &'static str,
    /// Shown first. The rest wait under More Filters.
    pub main: bool,
    pub kind: FilterKind,
}

pub(crate) const fn list(id: FilterId, label: &'static str, main: bool, single: bool, find: Option<&'static str>) -> FilterSpec {
    FilterSpec { id, label, main, kind: FilterKind::List { single, find } }
}

/// Every filter, in chip order: the main set, then More Filters.
pub const FILTERS: [FilterSpec; 16] = [
    list(FilterId::Genre, n_("Genre"), true, false, Some(n_("Find Genres"))),
    list(FilterId::Artist, n_("Artist"), true, false, Some(n_("Find Artists"))),
    FilterSpec { id: FilterId::Year, label: n_("Year"), main: true, kind: FilterKind::Range { digits: 4 } },
    FilterSpec { id: FilterId::Favorite, label: n_("Favorites"), main: true, kind: FilterKind::Toggle },
    list(FilterId::Plays, n_("Plays"), true, true, None),
    list(FilterId::Added, n_("Date Added"), true, true, None),
    list(FilterId::Quality, n_("Quality"), true, false, None),
    FilterSpec { id: FilterId::Downloaded, label: n_("Downloaded"), main: true, kind: FilterKind::Toggle },
    list(FilterId::AlbumArtist, n_("Album Artist"), false, false, Some(n_("Find Album Artists"))),
    list(FilterId::LastPlayed, n_("Last Played"), false, true, None),
    list(FilterId::Format, n_("Format"), false, false, None),
    list(FilterId::Bitrate, n_("Bitrate"), false, true, None),
    list(FilterId::Length, n_("Length"), false, false, None),
    list(FilterId::Explicit, n_("Explicit"), false, true, None),
    FilterSpec { id: FilterId::Bpm, label: n_("BPM"), main: false, kind: FilterKind::Range { digits: 3 } },
    list(FilterId::Rating, n_("Rating"), false, true, None),
];

/// A filter's spec from the Songs or Artists catalog.
/// `main` can differ between pages, so read it from the page's own catalog.
pub fn spec(id: FilterId) -> &'static FilterSpec {
    FILTERS.iter().chain(crate::artist_filters::ARTIST_FILTERS.iter()).find(|f| f.id == id).expect("every filter is in a catalog")
}

/// What a page's filter row reads from its index: available filters, list choices with counts and quick picks.
pub trait FilterSource: Send + Sync {
    fn available(&self) -> Vec<FilterId>;
    fn options(&self, id: FilterId, now_ms: u64) -> Vec<FilterOption>;
    fn quick_ranges(&self, _id: FilterId) -> Vec<QuickRange> {
        Vec::new()
    }
}

impl FilterSource for FilterIndex {
    fn available(&self) -> Vec<FilterId> {
        FilterIndex::available(self)
    }
    fn options(&self, id: FilterId, now_ms: u64) -> Vec<FilterOption> {
        FilterIndex::options(self, id, now_ms)
    }
    fn quick_ranges(&self, id: FilterId) -> Vec<QuickRange> {
        FilterIndex::quick_ranges(self, id)
    }
}

/// What a choice says, either the server's own text or English to translate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionLabel {
    Text(String),
    Key(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterOption {
    pub value: String,
    pub label: OptionLabel,
    /// Songs it matches in the whole library.
    pub count: u32,
}

/// A quick pick for a range, such as a decade, with its label and the From and To it sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickRange {
    pub label: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterValue {
    List(Vec<String>),
    Range { from: String, to: String },
    On,
}

impl FilterValue {
    /// Whether it narrows anything.
    pub fn is_set(&self) -> bool {
        match self {
            FilterValue::List(v) => !v.is_empty(),
            FilterValue::Range { from, to } => !from.is_empty() || !to.is_empty(),
            FilterValue::On => true,
        }
    }
}

/// The filters set on the page, in the order they were set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterState {
    values: Vec<(FilterId, FilterValue)>,
}

impl FilterState {
    pub fn get(&self, id: FilterId) -> Option<&FilterValue> {
        self.values.iter().find(|(i, _)| *i == id).map(|(_, v)| v)
    }

    /// Sets a filter. An empty list or range clears it.
    pub fn set(&mut self, id: FilterId, value: Option<FilterValue>) {
        match value.filter(FilterValue::is_set) {
            Some(v) => match self.values.iter_mut().find(|(i, _)| *i == id) {
                Some(slot) => slot.1 = v,
                None => self.values.push((id, v)),
            },
            None => self.values.retain(|(i, _)| *i != id),
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn is_set(&self, id: FilterId) -> bool {
        self.get(id).is_some()
    }

    /// Each set filter with its value, in the order they were set.
    pub fn iter(&self) -> impl Iterator<Item = (FilterId, &FilterValue)> {
        self.values.iter().map(|(i, v)| (*i, v))
    }

    /// The filters that are on, in the order they were set.
    pub fn active(&self) -> Vec<FilterId> {
        self.values.iter().map(|(i, _)| *i).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// A typed range isn't finished, so the filters can't be saved as a smart playlist yet.
    pub fn unfinished(&self) -> bool {
        self.values.iter().any(|(id, v)| matches!(v, FilterValue::Range { from, to } if range_problem(*id, from, to).is_some()))
    }

    /// Picks or unpicks one choice of a list filter. A single-choice list keeps at most one.
    pub fn toggle_choice(&mut self, id: FilterId, value: &str) {
        let single = matches!(spec(id).kind, FilterKind::List { single: true, .. });
        let mut list = match self.get(id) {
            Some(FilterValue::List(v)) => v.clone(),
            _ => Vec::new(),
        };
        if let Some(at) = list.iter().position(|v| v == value) {
            list.remove(at);
        } else if single {
            list = vec![value.to_owned()];
        } else {
            list.push(value.to_owned());
        }
        self.set(id, Some(FilterValue::List(list)));
    }
}

/// Why a typed range isn't usable yet, as an English catalog key, or `None` when it's complete and in order.
/// Years need all four digits, while shorter values like BPM need only one.
pub fn range_problem(id: FilterId, from: &str, to: &str) -> Option<&'static str> {
    let digits = match spec(id).kind {
        FilterKind::Range { digits } => digits,
        _ => return None,
    };
    let partial = |v: &str| !v.is_empty() && v.len() < if digits == 4 { 4 } else { 1 };
    if partial(from) || partial(to) {
        return Some(n_("Enter four-digit years."));
    }
    if let (Ok(a), Ok(b)) = (from.parse::<i64>(), to.parse::<i64>())
        && b < a
    {
        return Some(n_("From comes before To."));
    }
    None
}

/// Keeps only digits, at most as many as the range takes.
pub fn clean_range_input(id: FilterId, text: &str) -> String {
    let digits = match spec(id).kind {
        FilterKind::Range { digits } => digits,
        _ => 0,
    };
    text.chars().filter(char::is_ascii_digit).take(digits).collect()
}

pub(crate) const DAY_MS: u64 = 86_400_000;

pub(crate) const PLAYS: [(&str, &str); 2] = [("never", n_("Never Played")), ("played", n_("Played"))];
pub(crate) const ADDED: [(&str, &str); 4] = [("7", n_("Last 7 Days")), ("30", n_("Last 30 Days")), ("90", n_("Last 3 Months")), ("365", n_("Last Year"))];
const LAST_PLAYED: [(&str, &str); 3] = [("30", n_("In the Last Month")), ("365", n_("In the Last Year")), ("-365", n_("Not in a Year"))];
const QUALITY: [(&str, &str); 3] = [("hires", n_("Hi-Res Lossless")), ("lossless", n_("Lossless")), ("lossy", n_("Lossy"))];
const LENGTH: [(&str, &str); 3] = [("short", n_("Under 3 Minutes")), ("mid", n_("3 to 6 Minutes")), ("long", n_("Over 6 Minutes"))];
const BITRATE: [(&str, &str); 3] = [("320", n_("320 kbps or More")), ("256", n_("256 kbps or More")), ("128", n_("128 kbps or More"))];
const EXPLICIT: [(&str, &str); 2] = [("hide", n_("Hide Explicit")), ("only", n_("Only Explicit"))];
/// Five stars is drawn as stars only, so it isn't text to translate.
const RATING: [(&str, &str); 3] = [("5", "★★★★★"), ("4", n_("★★★★ or More")), ("3", n_("★★★ or More"))];

fn length_seconds(key: &str) -> Option<(i64, i64)> {
    match key {
        "short" => Some((0, 179)),
        "mid" => Some((180, 360)),
        "long" => Some((361, 1_000_000_000)),
        _ => None,
    }
}

fn tier_key(t: Tier) -> &'static str {
    match t {
        Tier::HiRes => "hires",
        Tier::Lossless => "lossless",
        Tier::Lossy => "lossy",
    }
}

/// Interned text, since genres, artists and formats repeat across thousands of songs.
type Name = u32;

/// What the filters read about one song.
#[derive(Clone, Debug, Default)]
struct Facts {
    rowid: u32,
    genres: Vec<Name>,
    artists: Vec<Name>,
    album_artist: Option<Name>,
    year: Option<i32>,
    starred: bool,
    plays: i64,
    added_ms: Option<u64>,
    played_ms: Option<u64>,
    tier: Option<Tier>,
    format: Option<Name>,
    bitrate: i32,
    duration: i64,
    explicit: bool,
    bpm: i32,
    rating: i32,
}

/// Every song's filter facts, from the local library. Build it with [`FilterIndex::from_songs`].
#[derive(Clone, Debug, Default)]
pub struct FilterIndex {
    songs: Vec<Facts>,
    names: Vec<String>,
    ids: HashMap<String, Name>,
}

/// Device state a match reads beside the index, which songs are downloaded and the clock.
pub struct MatchContext<'a> {
    pub downloaded: &'a HashSet<u32>,
    pub now_ms: u64,
}

impl FilterIndex {
    pub fn from_songs<'a>(songs: impl IntoIterator<Item = (u32, &'a Song)>) -> Self {
        let mut index = FilterIndex::default();
        for (rowid, s) in songs {
            index.add(rowid, s);
        }
        index
    }

    fn name(&mut self, name: &str) -> Name {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len() as Name;
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), id);
        id
    }

    /// Adds one song's facts.
    pub fn add(&mut self, rowid: u32, s: &Song) {
        let file = FileFacts::from_song(s);
        let facts = Facts {
            rowid,
            genres: s.genre_names().iter().map(|g| self.name(g)).collect(),
            artists: s.artist_names().iter().map(|a| self.name(a)).collect(),
            album_artist: s.album_artist_line().map(|a| self.name(&a)),
            year: s.year.filter(|&y| y > 0),
            starred: s.starred.as_deref().is_some_and(|v| !v.is_empty()),
            plays: s.play_count.unwrap_or(0),
            added_ms: s.created.as_deref().and_then(crate::queue_sync::parse_iso8601_ms),
            played_ms: s.played.as_deref().and_then(crate::queue_sync::parse_iso8601_ms),
            tier: file.as_ref().map(FileFacts::tier),
            format: file.as_ref().map(|f| f.format_name()).map(|f| self.name(&f)),
            bitrate: s.bit_rate.unwrap_or(0).max(0),
            duration: s.duration.unwrap_or(0).max(0),
            explicit: is_explicit(s.explicit_status.as_deref()),
            bpm: s.bpm.unwrap_or(0).max(0),
            rating: s.user_rating.unwrap_or(0).max(0),
        };
        self.songs.push(facts);
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Whether the library has the data a filter reads, so filters that match nothing stay hidden.
    /// No API says whether ratings are on, so Rating shows once any song has one.
    pub fn has_data(&self, id: FilterId) -> bool {
        let any = |f: &dyn Fn(&Facts) -> bool| self.songs.iter().any(f);
        match id {
            FilterId::Genre => any(&|s| !s.genres.is_empty()),
            FilterId::Year => any(&|s| s.year.is_some()),
            FilterId::Added => any(&|s| s.added_ms.is_some()),
            FilterId::AlbumArtist => any(&|s| s.album_artist.is_some()),
            FilterId::LastPlayed => any(&|s| s.played_ms.is_some()),
            FilterId::Bitrate => any(&|s| s.bitrate > 0),
            FilterId::Explicit => any(&|s| s.explicit),
            FilterId::Bpm => any(&|s| s.bpm > 0),
            FilterId::Rating => any(&|s| s.rating > 0),
            FilterId::Artist | FilterId::Favorite | FilterId::Plays | FilterId::Quality | FilterId::Downloaded | FilterId::Format | FilterId::Length => true,
            FilterId::Albums | FilterId::Role => false,
        }
    }

    /// The filters this library can offer, in chip order.
    pub fn available(&self) -> Vec<FilterId> {
        FILTERS.iter().map(|f| f.id).filter(|&id| self.has_data(id)).collect()
    }

    /// A list filter's choices with their song counts. Names are sorted by count, and fixed choices keep their order
    /// with empty ones left out.
    pub fn options(&self, id: FilterId, now_ms: u64) -> Vec<FilterOption> {
        let fixed = |list: &[(&'static str, &'static str)]| -> Vec<FilterOption> {
            list.iter()
                .map(|&(value, label)| {
                    let count = self.songs.iter().filter(|s| self.fits(s, id, value, now_ms)).count() as u32;
                    let label = if value == "5" && id == FilterId::Rating { OptionLabel::Text(label.to_owned()) } else { OptionLabel::Key(label) };
                    FilterOption { value: value.to_owned(), label, count }
                })
                .filter(|o| o.count > 0)
                .collect()
        };
        match id {
            FilterId::Genre => self.counted(|s| s.genres.clone()),
            FilterId::Artist => self.counted(|s| s.artists.clone()),
            FilterId::AlbumArtist => self.counted(|s| s.album_artist.into_iter().collect()),
            FilterId::Format => self.counted(|s| s.format.into_iter().collect()),
            FilterId::Plays => fixed(&PLAYS),
            FilterId::Added => fixed(&ADDED),
            FilterId::LastPlayed => fixed(&LAST_PLAYED),
            FilterId::Quality => fixed(&QUALITY),
            FilterId::Length => fixed(&LENGTH),
            FilterId::Bitrate => fixed(&BITRATE),
            FilterId::Explicit => fixed(&EXPLICIT),
            FilterId::Rating => fixed(&RATING),
            FilterId::Year | FilterId::Bpm | FilterId::Favorite | FilterId::Downloaded | FilterId::Albums | FilterId::Role => Vec::new(),
        }
    }

    fn counted(&self, of: impl Fn(&Facts) -> Vec<Name>) -> Vec<FilterOption> {
        let mut n: HashMap<Name, u32> = HashMap::new();
        for s in &self.songs {
            for v in of(s) {
                *n.entry(v).or_default() += 1;
            }
        }
        let mut all: Vec<(Name, u32)> = n.into_iter().collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| self.names[a.0 as usize].to_lowercase().cmp(&self.names[b.0 as usize].to_lowercase())));
        all.into_iter()
            .map(|(name, count)| {
                let text = self.names[name as usize].clone();
                FilterOption { value: text.clone(), label: OptionLabel::Text(text), count }
            })
            .collect()
    }

    /// A range's quick picks, the library's decades newest first, at most eight.
    /// Years below 1000 are tags that lost their century, so they get no decade.
    pub fn quick_ranges(&self, id: FilterId) -> Vec<QuickRange> {
        if id != FilterId::Year {
            return Vec::new();
        }
        let mut decades: Vec<i32> = self.songs.iter().filter_map(|s| s.year).filter(|&y| y >= 1000).map(|y| y / 10 * 10).collect::<HashSet<_>>().into_iter().collect();
        decades.sort_unstable_by(|a, b| b.cmp(a));
        decades.truncate(8);
        decades.into_iter().map(|d| QuickRange { label: format!("{d}s"), from: d.to_string(), to: (d + 9).to_string() }).collect()
    }

    fn name_in(&self, name: Name, values: &[String]) -> bool {
        let text = &self.names[name as usize];
        values.iter().any(|v| v == text)
    }

    /// Whether a song fits one fixed choice of a list filter.
    fn fits(&self, s: &Facts, id: FilterId, value: &str, now_ms: u64) -> bool {
        let within = |at: Option<u64>, days: u64| at.is_some_and(|at| now_ms.saturating_sub(at) <= days * DAY_MS);
        let n: i64 = value.parse().unwrap_or(0);
        match id {
            FilterId::Plays => (value == "never") == (s.plays <= 0),
            FilterId::Added => within(s.added_ms, n.max(0) as u64),
            FilterId::LastPlayed => {
                if n > 0 {
                    within(s.played_ms, n as u64)
                } else {
                    s.played_ms.is_some() && !within(s.played_ms, n.unsigned_abs())
                }
            }
            FilterId::Quality => s.tier.is_some_and(|t| tier_key(t) == value),
            FilterId::Length => length_seconds(value).is_some_and(|(a, b)| s.duration >= a && s.duration <= b),
            FilterId::Bitrate => i64::from(s.bitrate) >= n,
            FilterId::Explicit => (value == "only") == s.explicit,
            FilterId::Rating => i64::from(s.rating) >= n,
            _ => false,
        }
    }

    fn song_matches(&self, s: &Facts, state: &FilterState, ctx: &MatchContext) -> bool {
        for (id, value) in &state.values {
            let ok = match (id, value) {
                (FilterId::Favorite, _) => s.starred,
                (FilterId::Downloaded, _) => ctx.downloaded.contains(&s.rowid),
                (FilterId::Genre, FilterValue::List(v)) => s.genres.iter().any(|&g| self.name_in(g, v)),
                (FilterId::Artist, FilterValue::List(v)) => s.artists.iter().any(|&a| self.name_in(a, v)),
                (FilterId::AlbumArtist, FilterValue::List(v)) => s.album_artist.is_some_and(|a| self.name_in(a, v)),
                (FilterId::Format, FilterValue::List(v)) => s.format.is_some_and(|f| self.name_in(f, v)),
                (_, FilterValue::List(v)) => v.iter().any(|choice| self.fits(s, *id, choice, ctx.now_ms)),
                (_, FilterValue::Range { from, to }) => {
                    if range_problem(*id, from, to).is_some() {
                        true
                    } else {
                        let have = match id {
                            FilterId::Year => s.year.map(i64::from),
                            FilterId::Bpm => (s.bpm > 0).then_some(i64::from(s.bpm)),
                            _ => None,
                        };
                        let from: Option<i64> = from.parse().ok();
                        let to: Option<i64> = to.parse().ok();
                        have.is_some_and(|n| from.is_none_or(|f| n >= f) && to.is_none_or(|t| n <= t))
                    }
                }
                (_, FilterValue::On) => true,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// Row ids of the songs every set filter fits.
    pub fn matching(&self, state: &FilterState, ctx: &MatchContext) -> HashSet<u32> {
        self.songs.iter().filter(|s| self.song_matches(s, state, ctx)).map(|s| s.rowid).collect()
    }

    /// Narrows a list of row ids, in its order, to the songs every set filter fits.
    pub fn narrow(&self, rowids: Vec<u32>, state: &FilterState, ctx: &MatchContext) -> Vec<u32> {
        if state.is_empty() {
            return rowids;
        }
        let keep = self.matching(state, ctx);
        rowids.into_iter().filter(|r| keep.contains(r)).collect()
    }
}

fn one_of(field: &str, values: &[String]) -> Value {
    if values.len() > 1 {
        json!({ "any": values.iter().map(|v| json!({ "is": { field: v } })).collect::<Vec<_>>() })
    } else {
        json!({ "is": { field: values.first().cloned().unwrap_or_default() } })
    }
}

fn range_rule(field: &str, from: &str, to: &str) -> Value {
    let from: Option<i64> = from.parse().ok();
    let to: Option<i64> = to.parse().ok();
    match (from, to) {
        (Some(a), Some(b)) => json!({ "inTheRange": { field: [a, b] } }),
        (Some(a), None) => json!({ "gt": { field: a - 1 } }),
        (None, Some(b)) => json!({ "lt": { field: b + 1 } }),
        (None, None) => Value::Null,
    }
}

/// Navidrome's rule for one filter, or `None` when the server has no field for it.
fn rule(id: FilterId, value: &FilterValue) -> Option<Value> {
    let list = match value {
        FilterValue::List(v) => v.as_slice(),
        _ => &[],
    };
    let first = list.first().map(String::as_str).unwrap_or("");
    let n: i64 = first.parse().unwrap_or(0);
    Some(match id {
        FilterId::Genre => one_of("genre", list),
        FilterId::Artist => one_of("artist", list),
        FilterId::AlbumArtist => one_of("albumartist", list),
        FilterId::Year | FilterId::Bpm => match value {
            FilterValue::Range { from, to } => range_rule(if id == FilterId::Year { "year" } else { "bpm" }, from, to),
            _ => return None,
        },
        FilterId::Favorite => json!({ "is": { "loved": true } }),
        FilterId::Plays => {
            if first == "never" {
                json!({ "is": { "playcount": 0 } })
            } else {
                json!({ "gt": { "playcount": 0 } })
            }
        }
        FilterId::Added => json!({ "inTheLast": { "dateadded": n } }),
        FilterId::LastPlayed => {
            if n > 0 {
                json!({ "inTheLast": { "lastplayed": n } })
            } else {
                json!({ "notInTheLast": { "lastplayed": -n } })
            }
        }
        FilterId::Bitrate => json!({ "gt": { "bitrate": n - 1 } }),
        FilterId::Length => json!({ "any": list.iter().filter_map(|k| length_seconds(k)).map(|(a, b)| json!({ "inTheRange": { "duration": [a, b] } })).collect::<Vec<_>>() }),
        FilterId::Explicit => {
            if first == "hide" {
                json!({ "isNot": { "explicitstatus": "e" } })
            } else {
                json!({ "is": { "explicitstatus": "e" } })
            }
        }
        FilterId::Rating => json!({ "gt": { "rating": n - 1 } }),
        FilterId::Quality | FilterId::Format | FilterId::Downloaded | FilterId::Albums | FilterId::Role => return None,
    })
}

/// Builds Navidrome's criteria for the set filters with the page's sort, for Save as Smart Playlist.
/// Also returns the filters the server can't keep, for the editor's note.
pub fn to_rules(state: &FilterState, sort: &str, descending: bool) -> (String, Vec<FilterId>) {
    let mut all = Vec::new();
    let mut dropped = Vec::new();
    for (id, value) in &state.values {
        match rule(*id, value) {
            Some(r) if !r.is_null() => all.push(r),
            _ => dropped.push(*id),
        }
    }
    let mut top = json!({ "all": all, "sort": sort });
    if descending {
        top["order"] = json!("desc");
    }
    (top.to_string(), dropped)
}

/// A song list's sort as a smart playlist's field and direction, for [`to_rules`].
pub fn smart_sort(order: crate::library::SongOrder) -> (&'static str, bool) {
    use crate::library::SongOrder;
    match order {
        SongOrder::MostPlayed => ("playcount", true),
        SongOrder::Added => ("dateadded", true),
        SongOrder::Title => ("title", false),
        SongOrder::Artist => ("artist", false),
        SongOrder::Album => ("album", false),
        SongOrder::Year => ("year", true),
        SongOrder::Duration => ("duration", true),
    }
}

/// Text rule semantics over a multi-valued field, matching Navidrome's tag behavior.
/// A positive operator fits when any value does, and a negative one when none fits its positive twin.
/// Case-insensitive like SQLite `LIKE`.
pub fn text_rule_matches_any(op: &str, values: &[String], want: &str) -> bool {
    let want = want.to_lowercase();
    let fits = |positive: &str, v: &String| {
        let v = v.to_lowercase();
        match positive {
            "is" => v == want,
            "contains" => v.contains(&want),
            "startsWith" => v.starts_with(&want),
            "endsWith" => v.ends_with(&want),
            _ => false,
        }
    };
    match op {
        "isNot" => !values.iter().any(|v| fits("is", v)),
        "notContains" => !values.iter().any(|v| fits("contains", v)),
        _ => values.iter().any(|v| fits(op, v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::models::NamedRef;

    fn song(id: &str, genres: &[&str], year: i32, suffix: &str) -> Song {
        Song {
            id: id.into(),
            title: id.into(),
            artist: Some(format!("{id} artist")),
            genres: genres.iter().map(|g| NamedRef { name: (*g).into() }).collect(),
            genre: genres.first().map(|g| (*g).into()),
            year: Some(year),
            suffix: Some(suffix.into()),
            duration: Some(200),
            ..Default::default()
        }
    }

    fn ctx(downloaded: &HashSet<u32>) -> MatchContext<'_> {
        MatchContext { downloaded, now_ms: 0 }
    }

    #[test]
    fn genre_matches_every_genre_and_counts_each() {
        let songs = [song("a", &["Rock", "Pop"], 1995, "flac"), song("b", &["Pop"], 2012, "mp3"), song("c", &[], 2013, "mp3")];
        let index = FilterIndex::from_songs(songs.iter().enumerate().map(|(i, s)| (i as u32, s)));
        let mut state = FilterState::default();
        state.toggle_choice(FilterId::Genre, "Pop");
        let none = HashSet::new();
        let mut got: Vec<u32> = index.matching(&state, &ctx(&none)).into_iter().collect();
        got.sort();
        assert_eq!(got, vec![0, 1]);
        let options = index.options(FilterId::Genre, 0);
        assert_eq!(options[0].value, "Pop");
        assert_eq!(options[0].count, 2);
        assert!(!index.has_data(FilterId::Rating));
        assert!(!index.has_data(FilterId::Bpm));
        assert!(index.has_data(FilterId::Genre));
    }

    #[test]
    fn ranges_formats_and_downloads() {
        let songs = [song("a", &["Rock"], 1995, "flac"), song("b", &["Pop"], 2012, "mp3"), song("c", &[], 2019, "mp3")];
        let index = FilterIndex::from_songs(songs.iter().enumerate().map(|(i, s)| (i as u32, s)));
        let mut state = FilterState::default();
        state.set(FilterId::Year, Some(FilterValue::Range { from: "2010".into(), to: "2019".into() }));
        let down: HashSet<u32> = [2].into();
        assert_eq!(index.matching(&state, &ctx(&down)).len(), 2);
        state.set(FilterId::Downloaded, Some(FilterValue::On));
        assert_eq!(index.matching(&state, &ctx(&down)).into_iter().collect::<Vec<_>>(), vec![2]);
        // An unfinished year doesn't narrow anything, and can't be saved.
        let mut typing = FilterState::default();
        typing.set(FilterId::Year, Some(FilterValue::Range { from: "19".into(), to: String::new() }));
        assert!(typing.unfinished());
        assert_eq!(index.matching(&typing, &ctx(&down)).len(), 3);
        assert_eq!(index.quick_ranges(FilterId::Year).iter().map(|q| q.label.as_str()).collect::<Vec<_>>(), vec!["2010s", "1990s"]);
        let formats = index.options(FilterId::Format, 0);
        assert_eq!(formats.iter().map(|o| (o.value.as_str(), o.count)).collect::<Vec<_>>(), vec![("MP3", 2), ("FLAC", 1)]);
        let quality = index.options(FilterId::Quality, 0);
        assert_eq!(quality.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), vec!["lossless", "lossy"]);
    }

    #[test]
    fn rules_keep_what_the_server_has_and_name_the_rest() {
        let mut state = FilterState::default();
        state.toggle_choice(FilterId::Genre, "Rock");
        state.toggle_choice(FilterId::Genre, "Pop");
        state.set(FilterId::Year, Some(FilterValue::Range { from: "1990".into(), to: String::new() }));
        state.toggle_choice(FilterId::Quality, "hires");
        state.set(FilterId::Downloaded, Some(FilterValue::On));
        state.toggle_choice(FilterId::Plays, "never");
        let (json, dropped) = to_rules(&state, "dateadded", true);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["all"][0], json!({"any": [{"is": {"genre": "Rock"}}, {"is": {"genre": "Pop"}}]}));
        assert_eq!(v["all"][1], json!({"gt": {"year": 1989}}));
        assert_eq!(v["all"][2], json!({"is": {"playcount": 0}}));
        assert_eq!(v["order"], "desc");
        assert_eq!(dropped, vec![FilterId::Quality, FilterId::Downloaded]);
        assert!(crate::smart_rules::checked_criteria(&json).is_ok());
    }

    #[test]
    fn single_choice_lists_keep_one() {
        let mut state = FilterState::default();
        state.toggle_choice(FilterId::Plays, "never");
        state.toggle_choice(FilterId::Plays, "played");
        assert_eq!(state.get(FilterId::Plays), Some(&FilterValue::List(vec!["played".into()])));
        state.toggle_choice(FilterId::Plays, "played");
        assert!(state.is_empty());
    }

    #[test]
    fn several_values_match_like_navidrome_tags() {
        let g = vec!["Rock".to_string(), "Pop".to_string()];
        assert!(text_rule_matches_any("is", &g, "pop"));
        assert!(!text_rule_matches_any("isNot", &g, "Pop"));
        assert!(text_rule_matches_any("isNot", &g, "Jazz"));
        assert!(text_rule_matches_any("contains", &g, "ock"));
        assert!(!text_rule_matches_any("notContains", &g, "ock"));
        assert!(!text_rule_matches_any("is", &[], "Pop"));
        assert!(text_rule_matches_any("isNot", &[], "Pop"));
    }
}
