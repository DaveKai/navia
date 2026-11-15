//! The editor's live preview over the songs stored on this device, read in chunks so memory stays flat.
//! Fields the device doesn't store are reported and skipped, never guessed.

use std::collections::{HashMap, HashSet};

use super::fields::{SORT_RANDOM, Shape, field_by_id, shape_of};
use super::rules::{EditorState, LimitUnit, Row, RuleValue, SimpleRule, limit_value, parse_rule, ready_rows};
use super::{English, Say};
use crate::client::models::{Album, Song};
use crate::library::browse::AlbumOrder;
use crate::library::{Library, SongOrder};

/// A value a song has for a field, in the form rules compare against.
#[derive(Clone, Debug, PartialEq)]
pub enum Have {
    Null,
    Num(f64),
    Str(String),
    Bool(bool),
    /// Several values, like a song's genres. Text rules match them like Navidrome's tags.
    List(Vec<String>),
}

/// What's known about one song, for the preview. Dates are `YYYY-MM-DD...` text.
#[derive(Clone, Debug, Default)]
pub struct PreviewSong {
    pub rowid: u32,
    pub id: String,
    pub title: String,
    pub album: String,
    pub artist: String,
    pub album_artist: String,
    pub genres: Vec<String>,
    pub year: Option<f64>,
    pub track: Option<f64>,
    pub disc: Option<f64>,
    pub plays: Option<f64>,
    pub last_played: Option<String>,
    pub date_added: Option<String>,
    pub loved: Option<bool>,
    pub filetype: Option<String>,
    pub duration: Option<f64>,
    pub bitrate: Option<f64>,
    pub bitdepth: Option<f64>,
    pub samplerate: Option<f64>,
    pub channels: Option<f64>,
    pub bpm: Option<f64>,
    pub size: Option<f64>,
    pub comment: Option<String>,
    /// "e", "c" or nothing.
    pub explicit: Option<String>,
    pub rating: Option<f64>,
    pub filepath: Option<String>,
    pub lyrics: Option<bool>,
    pub album_plays: Option<f64>,
    pub album_songs: Option<f64>,
    pub album_duration: Option<f64>,
    pub album_added: Option<String>,
    pub album_last_played: Option<String>,
    pub album_loved: Option<bool>,
}

/// An album's numbers the preview reads for its songs.
#[derive(Clone, Debug, Default)]
pub struct AlbumFacts {
    pub plays: Option<f64>,
    pub songs: Option<f64>,
    pub duration: Option<f64>,
    pub added: Option<String>,
    pub last_played: Option<String>,
    pub loved: bool,
}

impl AlbumFacts {
    pub fn of(a: &Album) -> Self {
        Self { plays: Some(a.play_count.unwrap_or(0) as f64), songs: a.song_count.map(|n| n as f64), duration: a.duration.map(|n| n as f64), added: a.created.clone(), last_played: a.played.clone(), loved: a.starred.is_some() }
    }
}

impl PreviewSong {
    /// A song from the local library, with its album's numbers when known.
    pub fn from_song(rowid: u32, s: &Song, album: Option<&AlbumFacts>) -> Self {
        PreviewSong {
            rowid,
            id: s.id.clone(),
            title: s.title.clone(),
            album: s.album.clone().unwrap_or_default(),
            artist: s.artist_line().unwrap_or_default(),
            album_artist: s.display_album_artist.clone().unwrap_or_default(),
            genres: s.genre_names(),
            year: s.year.map(f64::from),
            track: s.track.map(f64::from),
            disc: s.disc_number.map(f64::from),
            plays: Some(s.play_count.unwrap_or(0) as f64),
            last_played: s.played.clone(),
            date_added: s.created.clone(),
            loved: Some(s.starred.is_some()),
            filetype: s.suffix.clone(),
            duration: s.duration.map(|d| d as f64),
            bitrate: s.bit_rate.map(f64::from),
            bitdepth: s.bit_depth.map(f64::from),
            samplerate: s.sampling_rate.map(f64::from),
            channels: s.channel_count.map(f64::from),
            bpm: s.bpm.map(f64::from),
            size: s.size.map(|v| v as f64),
            comment: s.comment.clone(),
            explicit: s.explicit_code().map(str::to_string),
            rating: s.user_rating.map(f64::from),
            filepath: s.path.clone(),
            lyrics: None,
            album_plays: album.and_then(|a| a.plays),
            album_songs: album.and_then(|a| a.songs),
            album_duration: album.and_then(|a| a.duration),
            album_added: album.and_then(|a| a.added.clone()),
            album_last_played: album.and_then(|a| a.last_played.clone()),
            album_loved: album.map(|a| a.loved),
        }
    }

    /// The value for a field, or `None` when this song's source doesn't carry it at all.
    pub fn get(&self, field: &str) -> Option<Have> {
        let s = |v: &Option<String>| v.as_ref().map_or(Have::Null, |s| Have::Str(s.clone()));
        let n = |v: Option<f64>| v.map_or(Have::Null, Have::Num);
        let b = |v: Option<bool>| v.map_or(Have::Null, Have::Bool);
        let text = |v: &str| if v.is_empty() { Have::Null } else { Have::Str(v.to_string()) };
        Some(match field {
            "title" => Have::Str(self.title.clone()),
            "album" => text(&self.album),
            "artist" => text(&self.artist),
            "albumartist" => text(&self.album_artist),
            "genre" => if self.genres.is_empty() { Have::Null } else { Have::List(self.genres.clone()) },
            "year" => n(self.year),
            "tracknumber" => n(self.track),
            "discnumber" => n(self.disc),
            "playcount" => n(self.plays),
            "lastplayed" => s(&self.last_played),
            "dateadded" => s(&self.date_added),
            "loved" => b(self.loved),
            "filetype" => s(&self.filetype),
            "duration" => n(self.duration),
            "bitrate" => n(self.bitrate),
            "bitdepth" => n(self.bitdepth),
            "samplerate" => n(self.samplerate),
            "channels" => n(self.channels),
            "bpm" => n(self.bpm),
            "size" => n(self.size),
            "comment" => s(&self.comment),
            "explicitstatus" => s(&self.explicit),
            "rating" => n(self.rating),
            "filepath" => s(&self.filepath),
            "albumplaycount" => n(self.album_plays),
            "albumsongcount" => n(self.album_songs),
            "albumduration" => n(self.album_duration),
            "albumdateadded" => s(&self.album_added),
            "albumlastplayed" => s(&self.album_last_played),
            "albumloved" => b(self.album_loved),
            _ => return None,
        })
    }
}

/// Which fields a source can answer. `Have::Null` means the source has the field but this song has no value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The local library.
    Core,
    /// An app's built-in sample data, which only has the common fields.
    Sample,
}

const CORE_FIELDS: &[&str] = &["title", "album", "artist", "albumartist", "genre", "year", "tracknumber", "discnumber", "playcount", "lastplayed", "dateadded", "loved", "filetype", "duration", "bitrate", "bitdepth", "samplerate", "channels", "bpm", "size", "comment", "explicitstatus", "rating", "filepath", "albumplaycount", "albumsongcount", "albumduration", "albumdateadded", "albumlastplayed", "albumloved"];
const SAMPLE_FIELDS: &[&str] = &["title", "album", "artist", "albumartist", "genre", "year", "playcount", "loved", "duration", "albumplaycount", "albumsongcount", "albumduration", "albumdateadded", "albumlastplayed", "albumloved"];

impl Source {
    fn can_read(self, field: &str, op: &str) -> bool {
        match field {
            "lyrics" => self == Source::Sample && (op == "isMissing" || op == "isPresent"),
            "id" => true,
            _ => match self {
                Source::Core => CORE_FIELDS.contains(&field),
                Source::Sample => SAMPLE_FIELDS.contains(&field),
            },
        }
    }

    /// Whether the preview can sort by this field.
    pub fn can_sort(self, field: &str) -> bool {
        field != "lyrics" && field != "id" && self.can_read(field, "is")
    }
}

/// Song ids in each playlist, for playlist rules, only for playlists whose songs are known here.
pub type PlaylistMembers = HashMap<String, HashSet<String>>;

/// Text rules over several values, as Navidrome matches tags.
/// Any value must match for positive operators, and none for negative ones.
fn text_matches_any(op: &str, values: &[String], want: &str) -> bool {
    crate::song_filters::text_rule_matches_any(op, values, want)
}

fn compare(s: Shape, op: &str, have: &Have, want: &RuleValue, now_days: i64) -> bool {
    if op == "isMissing" || op == "isPresent" {
        let missing = matches!(have, Have::Null) || matches!(have, Have::Str(x) if x.is_empty());
        return missing == (op == "isMissing");
    }
    if let Have::List(values) = have {
        let want = if let RuleValue::One(v) = want { v.as_str() } else { "" };
        return text_matches_any(op, values, want);
    }
    if s == Shape::Bool {
        let RuleValue::Flag(want) = want else { return false };
        return (matches!(have, Have::Bool(true)) == *want) == (op == "is");
    }
    if matches!(s, Shape::Number | Shape::Year | Shape::RangeNumber | Shape::RangeYear) {
        let n = match have {
            Have::Num(n) => *n,
            Have::Str(s) => match s.parse() {
                Ok(n) => n,
                Err(_) => return false,
            },
            _ => return false,
        };
        return match want {
            RuleValue::Range(a, b) => n >= a.parse().unwrap_or(f64::NAN) && n <= b.parse().unwrap_or(f64::NAN),
            RuleValue::One(v) => {
                let v: f64 = v.parse().unwrap_or(f64::NAN);
                match op {
                    "is" => n == v,
                    "isNot" => n != v,
                    "gt" => n > v,
                    _ => n < v,
                }
            }
            _ => false,
        };
    }
    if s == Shape::Days {
        let days: i64 = if let RuleValue::One(v) = want { v.parse().unwrap_or(0) } else { 0 };
        let in_last = matches!(have, Have::Str(d) if days_from_iso(d).is_some_and(|d| d >= now_days - days));
        return if op == "inTheLast" { in_last } else { !in_last };
    }
    if matches!(s, Shape::Date | Shape::RangeDate) {
        let Have::Str(have) = have else { return false };
        let d = &have[..have.len().min(10)];
        return match want {
            RuleValue::Range(a, b) => d >= a.as_str() && d <= b.as_str(),
            RuleValue::One(v) => match op {
                "is" => d == v,
                "isNot" => d != v,
                "before" => d < v.as_str(),
                _ => d > v.as_str(),
            },
            _ => false,
        };
    }
    let have_text = match have {
        Have::Str(s) => s.to_lowercase(),
        Have::Num(n) => n.to_string(),
        Have::Bool(b) => b.to_string(),
        Have::Null => String::new(),
        Have::List(l) => l.join(", ").to_lowercase(),
    };
    let want_text = if let RuleValue::One(v) = want { v.to_lowercase() } else { String::new() };
    match op {
        "is" => have_text == want_text,
        "isNot" => have_text != want_text,
        "contains" => have_text.contains(&want_text),
        "notContains" => !have_text.contains(&want_text),
        "startsWith" => have_text.starts_with(&want_text),
        _ => have_text.ends_with(&want_text),
    }
}

/// Days since the Unix epoch of a `YYYY-MM-DD...` string.
pub fn days_from_iso(s: &str) -> Option<i64> {
    let y: i64 = s.get(..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// `YYYY-MM-DD` of Unix seconds, for dates an app stores as seconds.
pub fn iso_from_unix(secs: i64) -> String {
    let z = secs.div_euclid(86400) + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today, in days since the Unix epoch.
pub fn today_days() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) / 86400
}

/// Playlist ids the rules name, at any depth, so their members can be looked up once.
pub fn playlist_ids(rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows {
        match r {
            Row::Simple(s) if s.field == "id" => {
                if let RuleValue::One(v) = &s.value {
                    out.push(v.clone());
                }
            }
            Row::Group { rows, .. } => out.extend(playlist_ids(rows)),
            _ => {}
        }
    }
    out
}

struct Ctx<'a> {
    source: Source,
    playlists: &'a PlaylistMembers,
    now_days: i64,
}

fn check_simple(r: &SimpleRule, t: &PreviewSong, c: &Ctx) -> Option<bool> {
    if r.field == "id" {
        let RuleValue::One(id) = &r.value else { return None };
        return Some(c.playlists.get(id)?.contains(&t.id) == (r.op == "inPlaylist"));
    }
    if !c.source.can_read(&r.field, &r.op) {
        return None;
    }
    let f = field_by_id(&r.field)?;
    // A song with lyrics has "yes", one without has nothing.
    let have = if r.field == "lyrics" { if t.lyrics == Some(true) { Have::Str("yes".to_string()) } else { Have::Null } } else { t.get(&r.field)? };
    Some(compare(shape_of(f, &r.op), &r.op, &have, &r.value, c.now_days))
}

/// All or any over results, skipping rules that can't be checked here. `None` when none could.
fn combine(any: bool, results: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let ok: Vec<bool> = results.flatten().collect();
    if ok.is_empty() {
        return None;
    }
    Some(if any { ok.iter().any(|b| *b) } else { ok.iter().all(|b| *b) })
}

fn check_row(row: &Row, t: &PreviewSong, c: &Ctx) -> Option<bool> {
    match row {
        Row::Simple(r) => check_simple(r, t, c),
        Row::Group { any, rows, .. } => combine(*any, rows.iter().map(|r| check_row(r, t, c))),
        Row::Kept { raw, .. } => match parse_rule(raw, Say(&English)) {
            Row::Kept { .. } => None,
            parsed => check_row(&parsed, t, c),
        },
    }
}

fn uncheckable(row: &Row, out: &mut Vec<String>, c: &Ctx) {
    let mut note = |text: String| {
        if !out.contains(&text) {
            out.push(text);
        }
    };
    match row {
        Row::Simple(r) => {
            let ok = if r.field == "id" { matches!(&r.value, RuleValue::One(id) if c.playlists.contains_key(id)) } else { c.source.can_read(&r.field, &r.op) };
            if !ok {
                note(field_by_id(&r.field).map_or(r.field.clone(), |f| f.label.to_string()));
            }
        }
        Row::Group { rows, .. } => rows.iter().for_each(|r| uncheckable(r, out, c)),
        Row::Kept { raw, text, .. } => match parse_rule(raw, Say(&English)) {
            Row::Kept { .. } => note(text.clone()),
            parsed => uncheckable(&parsed, out, c),
        },
    }
}

/// What the preview found. `unchecked` and `sort_unchecked` name what couldn't be checked, for the apps' note.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub rowids: Vec<u32>,
    pub matched: usize,
    pub sampled: usize,
    pub unchecked: Vec<String>,
    pub checkable: bool,
    pub sort_unchecked: Vec<String>,
}

/// One matched song's sort values.
#[derive(Clone, Debug, PartialEq)]
enum SortValue {
    Null,
    Num(f64),
    Str(String),
}

/// Runs the rules over songs fed one at a time. Matched songs keep only their sort values.
pub struct Evaluator {
    rows: Vec<Row>,
    any: bool,
    source: Source,
    playlists: PlaylistMembers,
    now_days: i64,
    random: bool,
    /// The sort fields the source can read, with their direction.
    sort_fields: Vec<(String, bool)>,
    sort_unchecked: Vec<String>,
    pub sampled: usize,
    any_checkable: bool,
    matched: Vec<(u32, Vec<SortValue>, u64)>,
}

impl Evaluator {
    pub fn new(st: &EditorState, source: Source, playlists: PlaylistMembers, now_days: i64) -> Self {
        let random = st.sorts.first().is_some_and(|s| s.field == SORT_RANDOM);
        let sort_fields = st.sorts.iter().filter(|s| s.field != SORT_RANDOM && source.can_sort(&s.field)).map(|s| (s.field.clone(), s.desc)).collect();
        let sort_unchecked = st.sorts.iter().filter(|s| s.field != SORT_RANDOM && !source.can_sort(&s.field)).map(|s| field_by_id(&s.field).map_or(s.field.clone(), |f| f.label.to_string())).collect();
        Self { rows: ready_rows(&st.rows), any: st.match_any, source, playlists, now_days, random, sort_fields, sort_unchecked, sampled: 0, any_checkable: false, matched: Vec::new() }
    }

    pub fn feed(&mut self, t: &PreviewSong) {
        self.sampled += 1;
        let c = Ctx { source: self.source, playlists: &self.playlists, now_days: self.now_days };
        let hit = combine(self.any, self.rows.iter().map(|r| check_row(r, t, &c)));
        let Some(hit) = hit else { return };
        self.any_checkable = true;
        if !hit {
            return;
        }
        let keys = self
            .sort_fields
            .iter()
            .map(|(f, _)| match t.get(f) {
                Some(Have::Num(n)) => SortValue::Num(n),
                Some(Have::Str(s)) => SortValue::Str(s.to_lowercase()),
                Some(Have::Bool(b)) => SortValue::Num(if b { 1.0 } else { 0.0 }),
                Some(Have::List(l)) => l.first().map_or(SortValue::Null, |s| SortValue::Str(s.to_lowercase())),
                _ => SortValue::Null,
            })
            .collect();
        self.matched.push((t.rowid, keys, fnv(&t.id)));
    }

    /// Sorts, applies the limit, keeps at most `max` rows and reports what couldn't be checked.
    pub fn finish(mut self, st: &EditorState, max: usize) -> Preview {
        use std::cmp::Ordering::*;
        let c = Ctx { source: self.source, playlists: &self.playlists, now_days: self.now_days };
        let mut unchecked = Vec::new();
        for r in &self.rows {
            uncheckable(r, &mut unchecked, &c);
        }
        let checkable = !self.rows.is_empty() && self.any_checkable;
        let matched = if checkable { self.matched.len() } else { 0 };
        if self.random {
            self.matched.sort_by_key(|(_, _, h)| *h);
        } else if !self.sort_fields.is_empty() {
            let dirs: Vec<bool> = self.sort_fields.iter().map(|(_, d)| *d).collect();
            self.matched.sort_by(|a, b| {
                for (i, desc) in dirs.iter().enumerate() {
                    let o = match (&a.1[i], &b.1[i]) {
                        (SortValue::Null, SortValue::Null) => Equal,
                        // Songs with no value go last either way.
                        (SortValue::Null, _) => return Greater,
                        (_, SortValue::Null) => return Less,
                        (SortValue::Num(x), SortValue::Num(y)) => x.total_cmp(y),
                        (SortValue::Str(x), SortValue::Str(y)) => x.cmp(y),
                        (SortValue::Num(_), SortValue::Str(_)) => Less,
                        (SortValue::Str(_), SortValue::Num(_)) => Greater,
                    };
                    let o = if *desc { o.reverse() } else { o };
                    if o != Equal {
                        return o;
                    }
                }
                Equal
            });
        }
        let n = limit_value(st) as usize;
        let limited = match (n, st.unit) {
            (0, _) => matched,
            (n, LimitUnit::Songs) => n,
            (p, LimitUnit::Percent) => ((matched * p) / 100).max(1),
        };
        let rowids = if checkable { self.matched.iter().take(limited.min(max)).map(|m| m.0).collect() } else { Vec::new() };
        Preview { rowids, matched, sampled: self.sampled, unchecked, checkable, sort_unchecked: self.sort_unchecked }
    }
}

/// A stable hash for the random order, so it doesn't reshuffle on every keystroke.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Songs read from the database at once, so memory stays flat however big the library is.
const SCAN_CHUNK: usize = 2000;

/// Runs the preview over the whole local library, playlist rules included where their songs are stored.
pub fn preview_library(lib: &Library, st: &EditorState, max: usize) -> Preview {
    let mut members = PlaylistMembers::new();
    for id in playlist_ids(&st.rows) {
        if let Ok(Some(pl)) = lib.playlist(&id) {
            if !pl.entry.is_empty() || pl.song_count.unwrap_or(0) == 0 {
                members.insert(id, pl.entry.into_iter().map(|s| s.id).collect());
            }
        }
    }
    let order = lib.album_order(AlbumOrder::Name, None, None, None).map(|o| o.rowids).unwrap_or_default();
    let albums: HashMap<String, AlbumFacts> = lib.albums_by_rowid(&order).unwrap_or_default().into_iter().map(|(_, a)| (a.id.clone(), AlbumFacts::of(&a))).collect();
    let mut eval = Evaluator::new(st, Source::Core, members, today_days());
    let rows = lib.song_order(SongOrder::Title, None).unwrap_or_default();
    for chunk in rows.chunks(SCAN_CHUNK) {
        for (rowid, song) in lib.songs_by_rowid(chunk).unwrap_or_default() {
            let album = song.album_id.as_ref().and_then(|id| albums.get(id));
            eval.feed(&PreviewSong::from_song(rowid, &song, album));
        }
    }
    eval.finish(st, max)
}

#[cfg(test)]
mod tests {
    use super::super::rules::{next_key, sort_key};
    use super::*;

    fn song(title: &str, genre: &str, year: f64, plays: f64) -> PreviewSong {
        PreviewSong { rowid: title.len() as u32, id: title.to_string(), title: title.to_string(), genres: if genre.is_empty() { Vec::new() } else { genre.split(';').map(str::to_string).collect() }, year: Some(year), plays: Some(plays), ..Default::default() }
    }

    fn run(st: &EditorState, songs: &[PreviewSong]) -> Preview {
        let mut e = Evaluator::new(st, Source::Core, PlaylistMembers::new(), 20000);
        for s in songs {
            e.feed(s);
        }
        e.finish(st, 500)
    }

    fn rule(field: &str, op: &str, value: RuleValue) -> Row {
        Row::Simple(SimpleRule { key: next_key(), field: field.into(), op: op.into(), value })
    }

    fn one(v: &str) -> RuleValue {
        RuleValue::One(v.into())
    }

    #[test]
    fn filters_sorts_and_limits() {
        let songs = vec![song("A", "Rock", 1999.0, 3.0), song("BB", "Jazz", 1990.0, 9.0), song("CCC", "Rock", 2005.0, 5.0)];
        let mut st = EditorState { sorts: vec![sort_key("playcount", true)], ..Default::default() };
        st.rows = vec![rule("genre", "is", one("rock"))];
        let p = run(&st, &songs);
        assert_eq!((p.matched, p.rowids.clone()), (2, vec![3, 1]));
        st.limit = "1".into();
        assert_eq!(run(&st, &songs).rowids, vec![3]);
        st.unit = LimitUnit::Percent;
        st.limit = "50".into();
        assert_eq!(run(&st, &songs).rowids, vec![3]);
    }

    #[test]
    fn several_sort_fields_break_ties() {
        let songs = vec![song("A", "", 2000.0, 3.0), song("BB", "", 2000.0, 9.0), song("CCC", "", 1990.0, 5.0)];
        let mut st = EditorState { sorts: vec![sort_key("year", true), sort_key("playcount", true)], ..Default::default() };
        st.rows = vec![rule("year", "gt", one("1000"))];
        assert_eq!(run(&st, &songs).rowids, vec![2, 1, 3]);
        st.sorts.push(sort_key("dateadded", false));
        st.sorts.push(sort_key("composer", false));
        assert_eq!(run(&st, &songs).sort_unchecked, vec!["Composer"]);
    }

    #[test]
    fn groups_combine_at_any_depth() {
        let songs = vec![song("A", "Rock", 1999.0, 3.0), song("BB", "Jazz", 1990.0, 9.0), song("CCC", "Soul", 2005.0, 0.0)];
        let mut st = EditorState::default();
        st.rows = vec![
            Row::Group { key: next_key(), any: true, rows: vec![rule("genre", "is", one("Jazz")), rule("genre", "is", one("Soul"))] },
            Row::Group { key: next_key(), any: true, rows: vec![rule("playcount", "gt", one("5")), Row::Group { key: next_key(), any: false, rows: vec![rule("year", "gt", one("2000")), rule("playcount", "lt", one("1"))] }] },
        ];
        assert_eq!(run(&st, &songs).rowids, vec![2, 3]);
        // An empty group, or one whose rules are unfinished, is skipped.
        st.rows.push(Row::Group { key: next_key(), any: false, rows: vec![rule("year", "is", one(""))] });
        assert_eq!(run(&st, &songs).matched, 2);
    }

    #[test]
    fn genre_rules_read_every_genre() {
        let songs = vec![song("A", "Pop;Rock", 1999.0, 3.0), song("BB", "Jazz", 1990.0, 9.0)];
        let mut st = EditorState::default();
        st.rows = vec![rule("genre", "is", one("rock"))];
        assert_eq!(run(&st, &songs).rowids, vec![1]);
        st.rows = vec![rule("genre", "isNot", one("Rock"))];
        assert_eq!(run(&st, &songs).rowids, vec![2]);
    }

    #[test]
    fn unchecked_fields_are_reported_and_skipped() {
        let songs = vec![song("A", "Rock", 1999.0, 3.0)];
        let mut st = EditorState::default();
        st.rows = vec![rule("composer", "is", one("Bach")), rule("genre", "is", one("Rock"))];
        let p = run(&st, &songs);
        assert_eq!((p.unchecked.clone(), p.matched), (vec!["Composer".to_string()], 1));
        st.rows = vec![rule("composer", "is", one("Bach"))];
        let p = run(&st, &songs);
        assert!(!p.checkable);
        assert_eq!(p.matched, 0);
    }

    #[test]
    fn dates_and_playlists() {
        let mut a = song("A", "", 2000.0, 1.0);
        a.date_added = Some("2021-05-01T10:00:00Z".to_string());
        let mut b = song("BB", "", 2000.0, 1.0);
        b.date_added = Some("2019-05-01T10:00:00Z".to_string());
        let st = EditorState { rows: vec![rule("dateadded", "inTheLast", one("30"))], ..Default::default() };
        let mut e = Evaluator::new(&st, Source::Core, PlaylistMembers::new(), days_from_iso("2021-05-11").unwrap());
        e.feed(&a);
        e.feed(&b);
        assert_eq!(e.finish(&st, 10).rowids, vec![1]);
        let st = EditorState { rows: vec![rule("id", "inPlaylist", one("p1"))], ..Default::default() };
        let mut members = PlaylistMembers::new();
        members.insert("p1".into(), ["A".to_string()].into_iter().collect());
        let mut e = Evaluator::new(&st, Source::Core, members, 0);
        e.feed(&a);
        assert_eq!(e.finish(&st, 10).rowids, vec![1]);
        assert_eq!(run(&st, &[a]).unchecked, vec!["Playlist"]);
        assert_eq!(iso_from_unix(1_700_000_000), "2023-11-14");
    }
}
