//! Recent taste: the play score behind every row ranked by plays. Each dated play decays with a
//! [`HALF_LIFE_DAYS`] half-life, and servers without dated history fall back to last-played dates
//! and damped all-time counts. Nothing never played is ranked.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use super::Library;
use super::browse::RankedArtist;
use super::recent::{META_FETCHED_AT, TOP_SONGS_WINDOW_DAYS};
use crate::client::{Album, Artist, Client};
use crate::error::Result;

/// A play this old weighs half as much as one today: a quarter at 3 months, a sixteenth at 6.
pub const HALF_LIFE_DAYS: f64 = 45.0;
/// A song counts at most this many times in one local day.
pub const SONG_PLAYS_PER_DAY: u32 = 3;
/// "World of" rotates among this many top artists.
pub const WORLD_OF_CANDIDATES: usize = 5;
/// An artist needs this many albums of their own for a "World of" row.
pub const WORLD_OF_MIN_ALBUMS: u32 = 4;
/// Scores only move with new plays, which are writes, and the window's edge, so an hour is plenty.
const CACHE_SECS: i64 = 3600;
/// On Repeat: songs played at least this often in the last `ON_REPEAT_DAYS`.
pub const ON_REPEAT_DAYS: i64 = 28;
pub const ON_REPEAT_MIN_PLAYS: u32 = 3;
pub const ON_REPEAT_LIMIT: usize = 8;
/// Back in Rotation: among the `ROTATION_POOL` artists most played before `ROTATION_OLD_DAYS`
/// ago, with under a tenth of that lately. Ratios rather than counts suit any amount of listening.
pub const ROTATION_OLD_DAYS: i64 = 183;
pub const ROTATION_LATELY_DAYS: i64 = 90;
pub const ROTATION_POOL: usize = 25;
pub const ROTATION_MIN_OLD: u32 = 5;
pub const ROTATION_LIMIT: usize = 12;
/// Because you've been playing X: seeds come from the top artists, and the row needs this many
/// similar library artists.
pub const BECAUSE_SEEDS: usize = 5;
pub const BECAUSE_MIN: usize = 4;
pub const BECAUSE_LIMIT: usize = 14;
const DAY: i64 = 86_400;
/// Home's rotation: "<turn>:<local day>".
const META_HOME_TURN: &str = "home_turn";
const WINDOW_SECS: i64 = TOP_SONGS_WINDOW_DAYS * DAY;

/// Where the scores came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TasteSource {
    /// Dated plays from the server's scrobble history.
    History,
    /// Last-played dates and all-time counts, because there's no dated history.
    LastPlayed,
}

/// One scored song, album or artist.
#[derive(Clone, Debug, PartialEq)]
pub struct Scored {
    pub rowid: u32,
    /// The server id for albums and artists. Empty for songs, which go by row.
    pub id: String,
    /// What it's ranked by: the weighted dated plays, or the fallback score without history.
    pub score: f64,
    /// The last-played score, which orders everything after the dated plays.
    pub fallback: f64,
    /// All-time plays, as the server counts them.
    pub plays: u64,
    /// The newest play known, Unix seconds, 0 when there's no date.
    pub last_played: i64,
    /// Albums whose main artist this is. Artists only.
    pub own_albums: u32,
}

/// Songs, albums and artists with real plays, best first.
#[derive(Clone, Debug)]
pub struct Taste {
    pub source: TasteSource,
    pub computed_at: i64,
    version: u64,
    pub songs: Vec<Scored>,
    pub albums: Vec<Scored>,
    pub artists: Vec<Scored>,
    /// Song rows for On Repeat, most played first, one per title and artist.
    pub on_repeat: Vec<u32>,
    /// Artist rows for Back in Rotation, by their old plays.
    pub back_in_rotation: Vec<u32>,
}

/// What "Because you've been playing X" can show right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Because {
    /// Seed artist row and the similar library artists' rows.
    Ready { seed: u32, artists: Vec<u32> },
    /// This artist's similar artists haven't been asked for today.
    Ask(String),
    /// No seed has enough similar library artists, or nothing has been played.
    Nothing,
}

impl Taste {
    /// "Your Top Songs": songs with a play inside the window. Never padded with older ones.
    pub fn top_songs(&self, limit: usize) -> Vec<u32> {
        let cutoff = self.computed_at - WINDOW_SECS;
        let inside = |s: &&Scored| match self.source {
            TasteSource::History => s.score > 0.0,
            TasteSource::LastPlayed => s.last_played >= cutoff,
        };
        self.songs.iter().filter(inside).take(limit).map(|s| s.rowid).collect()
    }

    /// The "World of" candidates: the best artists with enough albums of their own.
    pub fn world_of_candidates(&self) -> Vec<&Scored> {
        self.artists.iter().filter(|a| a.own_albums >= WORLD_OF_MIN_ALBUMS).take(WORLD_OF_CANDIDATES).collect()
    }

    /// The "World of" artist for a turn. Candidates go in id order, so scores trading places in
    /// the top five don't repeat an artist.
    pub fn world_of(&self, turn: i64) -> Option<&Scored> {
        let mut candidates = self.world_of_candidates();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        let n = candidates.len() as i64;
        (n > 0).then(|| candidates[turn.rem_euclid(n) as usize])
    }
}

impl Taste {
    /// The top artists and Back in Rotation's, so Because never repeats an artist on Home.
    fn because_excluded(&self) -> std::collections::HashSet<u32> {
        self.artists.iter().take(BECAUSE_SEEDS).map(|a| a.rowid).chain(self.back_in_rotation.iter().copied()).collect()
    }

    /// Seeds in the order to try this turn: the top artists except World of's, one step on each turn.
    pub fn because_seeds(&self, turn: i64) -> Vec<&Scored> {
        let world = self.world_of(turn).map(|w| w.rowid);
        let mut seeds: Vec<&Scored> = self.artists.iter().take(BECAUSE_SEEDS).filter(|a| Some(a.rowid) != world).collect();
        seeds.sort_by(|a, b| a.id.cmp(&b.id));
        if seeds.is_empty() {
            return seeds;
        }
        let start = turn.rem_euclid(seeds.len() as i64) as usize;
        seeds.rotate_left(start);
        seeds
    }

    /// Picks today's row. `similar` gives a seed's similar library artists, or `None` when it
    /// hasn't been asked today.
    pub fn because(&self, turn: i64, similar: impl Fn(&Scored) -> Option<Vec<u32>>) -> Because {
        let excluded = self.because_excluded();
        for seed in self.because_seeds(turn) {
            let Some(found) = similar(seed) else { return Because::Ask(seed.id.clone()) };
            let mut seen = std::collections::HashSet::new();
            let artists: Vec<u32> = found.into_iter().filter(|r| *r != seed.rowid && !excluded.contains(r) && seen.insert(*r)).take(BECAUSE_LIMIT).collect();
            if artists.len() >= BECAUSE_MIN {
                return Because::Ready { seed: seed.rowid, artists };
            }
        }
        Because::Nothing
    }
}

/// How much a play `age_secs` old counts.
pub fn decay(age_secs: i64) -> f64 {
    0.5f64.powf(age_secs.max(0) as f64 / (HALF_LIFE_DAYS * DAY as f64))
}

/// The score of something with only a last-played date and an all-time count.
pub fn fallback_score(age_secs: Option<i64>, count: u64) -> f64 {
    match age_secs {
        Some(age) if count > 0 => decay(age) * (1.0 + (count as f64).ln()),
        _ => 0.0,
    }
}

/// Today's local date, as days since 1970-01-01.
pub fn local_day(now: i64) -> i64 {
    (now + utc_offset(now)).div_euclid(DAY)
}

pub fn now_unix() -> i64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    now + clock_offset()
}

/// Seconds added to the day clock from `NAVIA_TEST_CLOCK_OFFSET` in test runs, so a run can
/// cross local midnight on cue. Zero otherwise.
fn clock_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| std::env::var("NAVIA_TEST_CLOCK_OFFSET").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Seconds the local time zone is ahead of UTC at `now`.
#[cfg(unix)]
pub fn utc_offset(now: i64) -> i64 {
    // SAFETY: localtime_r only writes the `tm` it's given.
    unsafe {
        let t = now as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i64 }
    }
}

#[cfg(not(unix))]
pub fn utc_offset(_now: i64) -> i64 {
    0
}

impl Library {
    /// The current scores, computed at most once per write or hour.
    pub fn taste(&self) -> Result<Arc<Taste>> {
        let now = now_unix();
        let mut slot = self.inner.taste.lock().expect("taste cache poisoned");
        if let Some(t) = slot.as_ref()
            && t.version == self.version()
            && (0..CACHE_SECS).contains(&(now - t.computed_at))
        {
            return Ok(t.clone());
        }
        let fresh = Arc::new(self.taste_at(now)?);
        *slot = Some(fresh.clone());
        Ok(fresh)
    }

    /// Scores as of `now`, never cached.
    pub fn taste_at(&self, now: i64) -> Result<Taste> {
        let version = self.version();
        let fetched_at: Option<i64> = self.meta(META_FETCHED_AT)?.and_then(|v| v.parse().ok());
        let offset = utc_offset(now);
        self.read(|c| compute(c, now, offset, fetched_at, version))
    }

    /// Artists by recent taste, best first, only ones with real plays.
    pub fn taste_artists(&self, limit: Option<u32>) -> Result<Vec<RankedArtist>> {
        let taste = self.taste()?;
        let picked: Vec<&Scored> = taste.artists.iter().take(limit.map_or(usize::MAX, |l| l as usize)).collect();
        self.ranked_from(&picked)
    }

    /// Albums by recent taste, best first, only ones with real plays.
    pub fn taste_albums(&self, limit: Option<u32>) -> Result<Vec<(u32, Album)>> {
        let taste = self.taste()?;
        let rowids: Vec<u32> = taste.albums.iter().take(limit.map_or(usize::MAX, |l| l as usize)).map(|a| a.rowid).collect();
        self.albums_by_rowid(&rowids)
    }

    /// Home's rotation step for "World of" and "Because". It moves by one on the first ask after
    /// launch and at local midnight, and stays put in between so Home doesn't jump while browsing.
    pub fn home_turn(&self, local_day: i64) -> Result<i64> {
        let mut slot = self.inner.home_turn.lock().expect("home turn poisoned");
        match *slot {
            Some((day, turn)) if day >= local_day => Ok(turn),
            _ => {
                let saved: i64 = self.meta(META_HOME_TURN)?.and_then(|v| v.split(':').next()?.parse().ok()).unwrap_or(0);
                let turn = saved + 1;
                self.write_quiet(|tx| {
                    tx.execute(
                        "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![META_HOME_TURN, format!("{turn}:{local_day}")],
                    )?;
                    Ok(())
                })?;
                *slot = Some((local_day, turn));
                Ok(turn)
            }
        }
    }

    /// The "World of" artist for this turn, `None` until some artist qualifies.
    pub fn world_of(&self, local_day: i64) -> Result<Option<RankedArtist>> {
        let taste = self.taste()?;
        let turn = self.home_turn(local_day)?;
        let Some(pick) = taste.world_of(turn) else { return Ok(None) };
        Ok(self.ranked_from(&[pick])?.into_iter().next())
    }

    /// This turn's "Because you've been playing X" from what's stored, without asking the server.
    pub fn because(&self, local_day: i64) -> Result<Because> {
        let taste = self.taste()?;
        let turn = self.home_turn(local_day)?;
        let mut known: HashMap<String, Vec<u32>> = HashMap::new();
        for seed in taste.because_seeds(turn) {
            if let Some(ids) = crate::similar_artists::stored_for_day(self, &seed.id, local_day) {
                known.insert(seed.id.clone(), self.artist_rowids(&ids)?);
            }
        }
        Ok(taste.because(turn, |seed| known.get(&seed.id).cloned()))
    }

    /// Similar library artist ids stored for this artist on this local day.
    pub fn similar_for_day(&self, artist_id: &str, local_day: i64) -> Result<Option<Vec<String>>> {
        let stored: Option<(i64, String)> = self.read(|c| {
            Ok(c.query_row("SELECT day, similar FROM similar_artist WHERE artist_id = ?1", [artist_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
        })?;
        Ok(stored.filter(|(day, _)| *day == local_day).map(|(_, ids)| serde_json::from_str(&ids).unwrap_or_default()))
    }

    /// Similar library artist ids stored for any of these artists, from any day. Never asks the
    /// server.
    pub fn similar_stored(&self, artist_ids: &[String]) -> Result<HashMap<String, Vec<String>>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT similar FROM similar_artist WHERE artist_id = ?1")?;
            let mut out = HashMap::new();
            for id in artist_ids {
                if let Some(json) = st.query_row([id], |r| r.get::<_, String>(0)).optional()? {
                    out.insert(id.clone(), serde_json::from_str(&json).unwrap_or_default());
                }
            }
            Ok(out)
        })
    }

    /// Keeps the server's answer for the day. Nothing is derived from it, so the taste cache stays.
    pub fn store_similar(&self, artist_id: &str, local_day: i64, ids: &[String]) -> Result<()> {
        let json = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
        self.write_quiet(|tx| {
            tx.execute(
                "INSERT INTO similar_artist (artist_id, day, similar) VALUES (?1, ?2, ?3)
                 ON CONFLICT(artist_id) DO UPDATE SET day = excluded.day, similar = excluded.similar",
                params![artist_id, local_day, json],
            )?;
            Ok(())
        })
    }

    /// Artist rows for ids, in the order given, leaving out ones not in the library.
    pub fn artist_rowids(&self, ids: &[String]) -> Result<Vec<u32>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT rowid FROM artist WHERE id = ?1")?;
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(r) = st.query_row([id], |r| r.get::<_, i64>(0)).optional()? {
                    out.push(r as u32);
                }
            }
            Ok(out)
        })
    }

    fn ranked_from(&self, picked: &[&Scored]) -> Result<Vec<RankedArtist>> {
        let rowids: Vec<u32> = picked.iter().map(|a| a.rowid).collect();
        let found = self.artists_by_rowid(&rowids)?;
        let by_rowid: HashMap<u32, &&Scored> = picked.iter().map(|a| (a.rowid, a)).collect();
        Ok(found
            .into_iter()
            .map(|(rowid, artist)| {
                let s = by_rowid[&rowid];
                // Dated plays the local counts haven't caught up with still count as played.
                RankedArtist { rowid, artist, plays: s.plays.max(1), own_albums: s.own_albums, score: s.score }
            })
            .collect())
    }
}

#[derive(Default)]
struct Acc {
    score: f64,
    fallback: f64,
    plays: u64,
    last: i64,
}

impl Acc {
    fn into_scored(self, rowid: u32) -> Scored {
        Scored { rowid, id: String::new(), score: self.score, fallback: self.fallback, plays: self.plays, last_played: self.last, own_albums: 0 }
    }
}

struct SongFacts {
    album: Option<u32>,
    artist: Option<u32>,
    plays: u64,
    played: Option<i64>,
}

/// Songs with plays, plus any the window has that the local count hasn't caught up with.
const PLAYED_SONGS: &str = "SELECT s.rowid, al.rowid, ar.rowid, s.play_count, unixepoch(s.played)
     FROM song s LEFT JOIN album al ON al.id = s.album_id
     LEFT JOIN artist ar ON ar.id = coalesce(s.artist_id, al.artist_id)
     WHERE s.play_count > 0
     UNION ALL
     SELECT s.rowid, al.rowid, ar.rowid, s.play_count, unixepoch(s.played)
     FROM (SELECT DISTINCT song_id FROM recent_play WHERE played_at >= ?1) p JOIN song s ON s.id = p.song_id
     LEFT JOIN album al ON al.id = s.album_id
     LEFT JOIN artist ar ON ar.id = coalesce(s.artist_id, al.artist_id)
     WHERE s.play_count <= 0";
const WINDOW_PLAYS: &str = "SELECT s.rowid, p.played_at FROM recent_play p JOIN song s ON s.id = p.song_id
     WHERE p.played_at >= ?1 ORDER BY s.rowid, p.played_at";

fn compute(c: &Connection, now: i64, offset: i64, fetched_at: Option<i64>, version: u64) -> Result<Taste> {
    let cutoff = now - WINDOW_SECS;
    let mut facts: HashMap<u32, SongFacts> = HashMap::new();
    {
        let mut st = c.prepare_cached(PLAYED_SONGS)?;
        let mut rows = st.query(params![cutoff])?;
        while let Some(r) = rows.next()? {
            let album = r.get::<_, Option<i64>>(1)?.map(|v| v as u32);
            let artist = r.get::<_, Option<i64>>(2)?.map(|v| v as u32);
            let plays = r.get::<_, i64>(3)?.max(0) as u64;
            facts.insert(r.get::<_, i64>(0)? as u32, SongFacts { album, artist, plays, played: r.get(4)? });
        }
    }

    // Dated plays, a song at a time in time order, capped per local day.
    let mut dated: HashMap<u32, (f64, i64)> = HashMap::new();
    {
        let mut st = c.prepare_cached(WINDOW_PLAYS)?;
        let mut rows = st.query(params![cutoff])?;
        let (mut current, mut day, mut today) = (u32::MAX, i64::MIN, 0u32);
        while let Some(r) = rows.next()? {
            let (rowid, at) = (r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)?);
            let d = (at + offset).div_euclid(DAY);
            if rowid != current || d != day {
                (current, day, today) = (rowid, d, 0);
            }
            today += 1;
            if today > SONG_PLAYS_PER_DAY {
                continue;
            }
            let e = dated.entry(rowid).or_insert((0.0, 0));
            e.0 += decay(now - at);
            e.1 = e.1.max(at);
        }
    }
    let source = if dated.is_empty() { TasteSource::LastPlayed } else { TasteSource::History };
    if source == TasteSource::History {
        // Played since the history was fetched: the last-played date is all there is.
        let since = fetched_at.unwrap_or(i64::MAX);
        for (rowid, f) in &facts {
            if let Some(played) = f.played.filter(|p| *p > since && *p <= now) {
                let e = dated.entry(*rowid).or_insert((0.0, 0));
                if played > e.1 + 30 {
                    e.0 += decay(now - played);
                    e.1 = played;
                }
            }
        }
    }

    let mut songs: HashMap<u32, Acc> = HashMap::new();
    let mut albums: HashMap<u32, Acc> = HashMap::new();
    let mut artists: HashMap<u32, Acc> = HashMap::new();
    for (rowid, f) in facts {
        let fallback = fallback_score(f.played.map(|p| now - p), f.plays);
        let (recent, newest) = dated.get(&rowid).copied().unwrap_or((0.0, 0));
        let score = if source == TasteSource::History { recent } else { fallback };
        let last = newest.max(f.played.unwrap_or(0));
        let add = |acc: &mut Acc| {
            acc.score += score;
            acc.fallback += fallback;
            acc.plays += f.plays;
            acc.last = acc.last.max(last);
        };
        add(songs.entry(rowid).or_default());
        if let Some(r) = f.album {
            add(albums.entry(r).or_default());
        }
        if let Some(r) = f.artist {
            add(artists.entry(r).or_default());
        }
    }

    let finish = |table: &str, map: HashMap<u32, Acc>| -> Result<Vec<Scored>> {
        let mut list: Vec<Scored> = map.into_iter().filter(|(_, a)| a.score > 0.0 || a.plays > 0).map(|(rowid, a)| a.into_scored(rowid)).collect();
        list.sort_by(rank);
        if !table.is_empty() {
            let ids = ids_of(c, table, &list)?;
            for s in &mut list {
                s.id = ids.get(&s.rowid).cloned().unwrap_or_default();
            }
        }
        Ok(list)
    };
    let songs = finish("", songs)?;
    let albums = finish("album", albums)?;
    let mut artists = finish("artist", artists)?;
    let own = own_album_counts(c)?;
    for a in &mut artists {
        a.own_albums = own.get(&a.rowid).copied().unwrap_or(0);
    }
    let on_repeat = on_repeat(c, now)?;
    let back_in_rotation = back_in_rotation(c, now)?;
    Ok(Taste { source, computed_at: now, version, songs, albums, artists, on_repeat, back_in_rotation })
}

/// Best first: dated score, then the last-played score, then all-time plays, then the row.
fn rank(a: &Scored, b: &Scored) -> Ordering {
    b.score.total_cmp(&a.score).then(b.fallback.total_cmp(&a.fallback)).then(b.plays.cmp(&a.plays)).then(a.rowid.cmp(&b.rowid))
}

fn on_repeat(c: &Connection, now: i64) -> Result<Vec<u32>> {
    let mut st = c.prepare_cached(
        "SELECT s.rowid, lower(trim(s.title)), coalesce(s.artist_id, ''), count(*) AS n FROM recent_play p JOIN song s ON s.id = p.song_id
         WHERE p.played_at >= ?1 AND p.played_at <= ?2 GROUP BY s.rowid HAVING n >= ?3
         ORDER BY n DESC, max(p.played_at) DESC, s.rowid",
    )?;
    let rows = st.query_map(params![now - ON_REPEAT_DAYS * DAY, now, ON_REPEAT_MIN_PLAYS], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        let (rowid, title, artist) = row?;
        // The same song on two albums counts once.
        if seen.insert((title, artist)) {
            out.push(rowid);
            if out.len() == ON_REPEAT_LIMIT {
                break;
            }
        }
    }
    Ok(out)
}

fn back_in_rotation(c: &Connection, now: i64) -> Result<Vec<u32>> {
    let mut st = c.prepare_cached(
        "SELECT ar.rowid, sum(p.played_at < ?1) AS old, sum(p.played_at >= ?2) FROM recent_play p JOIN song s ON s.id = p.song_id
         LEFT JOIN album al ON al.id = s.album_id JOIN artist ar ON ar.id = coalesce(s.artist_id, al.artist_id)
         GROUP BY ar.rowid HAVING old > 0 ORDER BY old DESC, ar.rowid LIMIT ?3",
    )?;
    let rows = st.query_map(params![now - ROTATION_OLD_DAYS * DAY, now - ROTATION_LATELY_DAYS * DAY, ROTATION_POOL as i64], |r| {
        Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (rowid, old, lately) = row?;
        if old >= i64::from(ROTATION_MIN_OLD) && (lately as f64) < old as f64 * 0.1 {
            out.push(rowid);
        }
    }
    out.truncate(ROTATION_LIMIT);
    Ok(out)
}

/// Albums of their own per artist row.
fn own_album_counts(c: &Connection) -> Result<HashMap<u32, u32>> {
    let mut st = c.prepare_cached("SELECT ar.rowid, count(*) FROM album al JOIN artist ar ON ar.id = al.artist_id GROUP BY ar.rowid")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)?.max(0) as u32)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Ids for the scored rows, from the id index alone.
fn ids_of(c: &Connection, table: &str, list: &[Scored]) -> Result<HashMap<u32, String>> {
    let mut out = HashMap::with_capacity(list.len());
    for chunk in list.chunks(500) {
        let holders = vec!["?"; chunk.len()].join(",");
        let mut st = c.prepare_cached(&format!("SELECT rowid, id FROM {table} WHERE rowid IN ({holders})"))?;
        let rows = st.query_map(rusqlite::params_from_iter(chunk.iter().map(|s| s.rowid)), |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (rowid, id) = row?;
            out.insert(rowid, id);
        }
    }
    Ok(out)
}

/// At most this many seeds are asked about in one go; the rest wait for the next refresh.
const BECAUSE_ASKS: usize = BECAUSE_SEEDS;

/// Asks the server about today's seeds until the Because row is decided. Each seed is asked once
/// per local day, since each ask can make the server refresh that artist's info. Failures wait
/// until tomorrow.
pub async fn refresh_because(client: &Client, library: &Library, local_day: i64) -> Result<Because> {
    for _ in 0..BECAUSE_ASKS {
        let state = library.blocking(move |l| l.because(local_day)).await?;
        let Because::Ask(seed) = state else { return Ok(state) };
        let ids: Vec<String> = match client.artist_info2(&seed, Some(30)).await {
            Ok(info) => info.similar_artist.into_iter().map(|a: Artist| a.id).filter(|id| !id.is_empty()).collect(),
            Err(e) => {
                tracing::warn!("similar artists for {seed}: {e}");
                Vec::new()
            }
        };
        library.store_similar(&seed, local_day, &ids)?;
        // Deezer's related artists join the server's while the listener has Deezer on.
        if let Some(artist) = library.artist(&seed)? {
            crate::similar_artists::deezer_similar(library, crate::deezer::Deezer::shared(), &artist.name).await;
        }
    }
    library.blocking(move |l| l.because(local_day)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Artist, MusicFolder, Song};
    use crate::client::native::ScrobbleEntry;
    use crate::library::LibraryConfig;

    const NOW: i64 = 1_800_000_000;

    fn iso(unix: i64) -> String {
        crate::library::extras::iso8601_ms(unix as u64 * 1000)
    }

    /// Song `id` by artist `artist` on album `al-<artist>`, played `plays` times, last at `played`.
    fn song(id: &str, artist: &str, plays: i64, played: Option<i64>) -> Song {
        Song {
            id: id.into(),
            title: id.to_uppercase(),
            artist_id: Some(artist.into()),
            album_id: Some(format!("al-{artist}")),
            play_count: Some(plays),
            played: played.map(iso),
            ..Default::default()
        }
    }

    fn album(id: &str, artist: &str) -> Album {
        Album { id: id.into(), name: id.into(), artist: Some(artist.into()), artist_id: Some(artist.into()), ..Default::default() }
    }

    /// Artists `a`..`z` as given, each with `albums[i]` albums of their own (`al-<artist>` first).
    fn library(dir: &tempfile::TempDir, songs: &[Song], artists: &[(&str, u32)]) -> Library {
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        let list: Vec<Artist> = artists.iter().map(|(id, _)| Artist { id: (*id).into(), name: id.to_uppercase(), ..Default::default() }).collect();
        lib.apply_artists("1", &cur, list.len() as u32, &list).unwrap();
        let mut albums = Vec::new();
        for (artist, count) in artists {
            for i in 0..*count {
                albums.push(album(&if i == 0 { format!("al-{artist}") } else { format!("al-{artist}-{i}") }, artist));
            }
        }
        lib.apply_albums("1", &cur, albums.len() as u32, &albums).unwrap();
        lib.apply_songs("1", &cur, songs.len() as u32, songs).unwrap();
        lib
    }

    fn plays(lib: &Library, list: &[(&str, i64)]) {
        let entries: Vec<ScrobbleEntry> = list.iter().enumerate().map(|(i, (s, at))| ScrobbleEntry { id: i as i64 + 1, song_id: (*s).into(), played_at: *at }).collect();
        lib.replace_recent_plays(&entries).unwrap();
    }

    fn artist_ids(t: &Taste) -> Vec<&str> {
        t.artists.iter().map(|a| a.id.as_str()).collect()
    }

    fn song_ids(lib: &Library, t: &Taste, limit: usize) -> Vec<String> {
        lib.songs_by_rowid(&t.top_songs(limit)).unwrap().into_iter().map(|(_, s)| s.id).collect()
    }

    #[test]
    fn decay_halves_every_half_life() {
        assert_eq!(decay(0), 1.0);
        assert!((decay(45 * DAY) - 0.5).abs() < 1e-12);
        assert!((decay(90 * DAY) - 0.25).abs() < 1e-12);
        assert!((decay(180 * DAY) - 0.0625).abs() < 1e-12);
        // A clock a little ahead of the server never makes a play count more than one.
        assert_eq!(decay(-500), 1.0);
    }

    #[test]
    fn recent_plays_beat_a_bigger_older_habit() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("old", "a", 900, Some(NOW - 150 * DAY)), song("new", "b", 3, Some(NOW - DAY))];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1)]);
        // Eight plays five months ago against two this week.
        let mut list: Vec<(&str, i64)> = (0..8).map(|d| ("old", NOW - 150 * DAY - d * DAY)).collect();
        list.extend([("new", NOW - DAY), ("new", NOW - 2 * DAY)]);
        plays(&lib, &list);
        let t = lib.taste_at(NOW).unwrap();
        assert_eq!(t.source, TasteSource::History);
        assert_eq!(artist_ids(&t), ["b", "a"]);
        let b = &t.artists[0];
        assert!((b.score - (decay(DAY) + decay(2 * DAY))).abs() < 1e-9);
        assert_eq!(song_ids(&lib, &t, 10), ["new", "old"]);
        assert_eq!(t.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["al-b", "al-a"]);
    }

    #[test]
    fn one_song_on_repeat_counts_three_times_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("loop", "a", 40, Some(NOW - DAY)), song("x", "b", 2, None), song("y", "b", 2, None)];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1)]);
        // Forty plays of one song in a single afternoon...
        let mut list: Vec<(&str, i64)> = (0..40).map(|i| ("loop", NOW - DAY + i * 60)).collect();
        // ...against four plays of two songs over two days.
        list.extend([("x", NOW - DAY), ("y", NOW - DAY), ("x", NOW - 2 * DAY), ("y", NOW - 2 * DAY)]);
        plays(&lib, &list);
        let t = lib.taste_at(NOW).unwrap();
        assert_eq!(artist_ids(&t), ["b", "a"]);
        let a = t.artists.iter().find(|a| a.id == "a").unwrap();
        assert!(a.score < 3.0 && a.score > 2.9, "{}", a.score);
        // The same song on three different days counts every day.
        let lib2 = library(&tempfile::tempdir().unwrap(), &songs, &[("a", 1), ("b", 1)]);
        plays(&lib2, &[("loop", NOW - DAY), ("loop", NOW - 2 * DAY), ("loop", NOW - 3 * DAY), ("loop", NOW - 4 * DAY)]);
        assert!(lib2.taste_at(NOW).unwrap().artists[0].score > 3.5);
    }

    #[test]
    fn fallback_without_history_puts_recency_first() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [
            song("habit", "a", 500, Some(NOW - 200 * DAY)),
            song("lately", "b", 3, Some(NOW - 2 * DAY)),
            song("month", "c", 40, Some(NOW - 30 * DAY)),
            song("never", "d", 0, None),
            song("undated", "e", 7, None),
        ];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1), ("c", 1), ("d", 1), ("e", 1)]);
        let t = lib.taste_at(NOW).unwrap();
        assert_eq!(t.source, TasteSource::LastPlayed);
        // 500 plays seven months ago rank below a month-old habit and three plays this week.
        // An undated count comes last, and nothing unplayed appears.
        assert_eq!(artist_ids(&t), ["c", "b", "a", "e"]);
        assert!((t.artists[1].score - decay(2 * DAY) * (1.0 + 3f64.ln())).abs() < 1e-9);
        // Top Songs keeps its window: the seven-month-old habit and the undated song stay out.
        assert_eq!(song_ids(&lib, &t, 10), ["month", "lately"]);
    }

    #[test]
    fn history_ranks_first_and_old_plays_follow() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("old", "a", 300, Some(NOW - 400 * DAY)), song("new", "b", 1, Some(NOW - DAY))];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1)]);
        plays(&lib, &[("new", NOW - DAY)]);
        let t = lib.taste_at(NOW).unwrap();
        assert_eq!(artist_ids(&t), ["b", "a"]);
        assert_eq!(t.artists[1].score, 0.0);
        // Top Songs only has songs played inside the window.
        assert_eq!(song_ids(&lib, &t, 10), ["new"]);
    }

    #[test]
    fn plays_since_the_last_fetch_count_from_the_last_played_date() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("fetched", "a", 2, Some(NOW - 10 * DAY)), song("since", "b", 1, Some(NOW - 600))];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1)]);
        plays(&lib, &[("fetched", NOW - 10 * DAY)]);
        lib.set_meta(META_FETCHED_AT, Some(&(NOW - 3600).to_string())).unwrap();
        let t = lib.taste_at(NOW).unwrap();
        assert_eq!(artist_ids(&t), ["b", "a"]);
        assert!((t.artists[0].score - decay(600)).abs() < 1e-9);
    }

    #[test]
    fn nothing_played_means_nothing_ranked() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(&dir, &[song("x", "a", 0, None)], &[("a", 6)]);
        let t = lib.taste_at(NOW).unwrap();
        assert!(t.songs.is_empty() && t.albums.is_empty() && t.artists.is_empty());
        assert!(t.top_songs(16).is_empty());
        assert!(t.world_of(20_000).is_none());
        assert!(lib.taste_artists(None).unwrap().is_empty());
        assert!(lib.world_of(20_000).unwrap().is_none());
    }

    #[test]
    fn songs_without_an_artist_count_for_the_album_artist() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = song("x", "a", 1, Some(NOW - DAY));
        s.artist_id = None;
        let lib = library(&dir, &[s], &[("a", 1)]);
        assert_eq!(artist_ids(&lib.taste_at(NOW).unwrap()), ["a"]);
    }

    /// Artists `a`..`g`, `a` best. `c` and `f` have too few albums.
    fn world(dir: &tempfile::TempDir) -> Library {
        let names = ["a", "b", "c", "d", "e", "f", "g"];
        let songs: Vec<Song> = names.iter().enumerate().map(|(i, n)| song(&format!("s{n}"), n, 10, Some(NOW - (i as i64 + 1) * DAY))).collect();
        let albums = [("a", 4), ("b", 5), ("c", 3), ("d", 4), ("e", 9), ("f", 1), ("g", 4)];
        library(dir, &songs, &albums)
    }

    #[test]
    fn world_of_takes_turns_among_the_top_five_with_four_albums() {
        let dir = tempfile::tempdir().unwrap();
        let lib = world(&dir);
        let t = lib.taste_at(NOW).unwrap();
        let candidates: Vec<&str> = t.world_of_candidates().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(candidates, ["a", "b", "d", "e", "g"]);
        let day = local_day(NOW);
        // Five turns in a row show all five, then it starts over.
        let week: Vec<&str> = (0..6).map(|d| t.world_of(day + d).unwrap().id.as_str()).collect();
        let mut seen = week[..5].to_vec();
        seen.sort();
        assert_eq!(seen, ["a", "b", "d", "e", "g"]);
        assert_eq!(week[5], week[0]);
        for pair in week.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        // The library's pick is the one for its first turn this launch.
        assert_eq!(lib.world_of(day).unwrap().unwrap().artist.id, t.world_of(1).unwrap().id);
        assert!(lib.world_of(day).unwrap().unwrap().own_albums >= WORLD_OF_MIN_ALBUMS);
    }

    #[test]
    fn world_of_works_with_fewer_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("x", "a", 3, Some(NOW - DAY)), song("y", "b", 3, Some(NOW - 2 * DAY)), song("z", "c", 9, Some(NOW - DAY))];
        let lib = library(&dir, &songs, &[("a", 4), ("b", 6), ("c", 2)]);
        let t = lib.taste_at(NOW).unwrap();
        let picks: Vec<&str> = (0..4).map(|d| t.world_of(d).unwrap().id.as_str()).collect();
        assert_eq!(picks, ["a", "b", "a", "b"]);
        let one = library(&tempfile::tempdir().unwrap(), &songs[..1], &[("a", 4)]);
        let t = one.taste_at(NOW).unwrap();
        assert!((0..3).all(|d| t.world_of(d).unwrap().id == "a"));
        // Negative days (before 1970) still pick something.
        assert!(t.world_of(-3).is_some());
    }

    #[test]
    fn local_day_follows_the_offset() {
        // Whatever the zone, a day number moves by one every 24 hours.
        assert_eq!(local_day(NOW + DAY) - local_day(NOW), 1);
        assert_eq!(local_day(NOW) - local_day(NOW - DAY), 1);
    }

    #[test]
    fn cache_refreshes_after_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let now = now_unix();
        let lib = library(&dir, &[song("x", "a", 1, Some(now - DAY)), song("y", "b", 1, Some(now - 2 * DAY))], &[("a", 1), ("b", 1)]);
        let first = lib.taste().unwrap();
        assert!(Arc::ptr_eq(&first, &lib.taste().unwrap()));
        assert_eq!(artist_ids(&first), ["a", "b"]);
        plays(&lib, &[("y", now - 60), ("y", now - 120)]);
        let second = lib.taste().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(artist_ids(&second), ["b", "a"]);
    }

    #[test]
    fn deep_cuts_follow_recent_taste() {
        let dir = tempfile::tempdir().unwrap();
        // `a` has the bigger all-time count, `b` the recent plays.
        let now = now_unix();
        let songs = [song("x", "a", 400, Some(now - 300 * DAY)), song("y", "b", 2, Some(now - DAY))];
        let lib = library(&dir, &songs, &[("a", 3), ("b", 3)]);
        let cuts: Vec<String> = lib.deep_cuts(1, 10).unwrap().into_iter().map(|a| a.artist_id.unwrap()).collect();
        assert_eq!(cuts, ["b", "b", "b"]);
    }

    /// `cargo test --release -p navia-core --lib taste_stays_fast -- --ignored --nocapture`.
    #[test]
    #[ignore = "builds a 300,000-song library with 200,000 dated plays"]
    fn taste_stays_fast_on_a_big_library_and_long_history() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("big.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        let artists: Vec<Artist> = (0..20_000).map(|i| Artist { id: format!("ar{i}"), name: format!("Artist {i}"), ..Default::default() }).collect();
        for chunk in artists.chunks(500) {
            lib.apply_artists("1", &cur, 0, chunk).unwrap();
        }
        let albums: Vec<Album> = (0..30_000).map(|i| album(&format!("al{i}"), &format!("ar{}", i % 20_000))).collect();
        for chunk in albums.chunks(500) {
            lib.apply_albums("1", &cur, 0, chunk).unwrap();
        }
        let mut batch = Vec::with_capacity(500);
        for i in 0..300_000i64 {
            let played = (i % 3 == 0).then(|| NOW - (i % 400) * DAY);
            let mut s = song(&format!("s{i}"), &format!("ar{}", i % 20_000), if played.is_some() { 1 + i % 50 } else { 0 }, played);
            s.album_id = Some(format!("al{}", i % 30_000));
            batch.push(s);
            if batch.len() == 500 {
                lib.apply_songs("1", &cur, 0, &batch).unwrap();
                batch.clear();
            }
        }
        let history: Vec<ScrobbleEntry> = (0..200_000i64).map(|i| ScrobbleEntry { id: i, song_id: format!("s{}", ((i * 7919) % 100_000) * 3), played_at: NOW - (i % (180 * DAY)) }).collect();
        lib.replace_recent_plays(&history).unwrap();
        let mut times = Vec::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let taste = lib.taste_at(NOW).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(taste.source, TasteSource::History);
        }
        let taste = lib.taste_at(NOW).unwrap();
        println!("songs {}, albums {}, artists {}; computed in {:?} ms", taste.songs.len(), taste.albums.len(), taste.artists.len(), times.iter().map(|t| t.round()).collect::<Vec<_>>());
        assert!(times.iter().all(|ms| *ms < 2000.0));
    }

    #[test]
    fn on_repeat_needs_three_plays_in_four_weeks_and_counts_a_song_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut twin = song("twin", "a", 3, Some(NOW - DAY));
        twin.title = "LOOP ".into();
        let songs = [song("loop", "a", 9, Some(NOW - DAY)), twin, song("two", "b", 2, Some(NOW - DAY)), song("old", "c", 5, Some(NOW - 40 * DAY)), song("ok", "c", 3, Some(NOW - DAY))];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1), ("c", 1)]);
        let mut list: Vec<(&str, i64)> = (0..5).map(|d| ("loop", NOW - d * DAY)).collect();
        list.extend((0..4).map(|d| ("twin", NOW - d * DAY - 60)));
        list.extend([("two", NOW - DAY), ("two", NOW - 2 * DAY)]);
        list.extend((0..5).map(|d| ("old", NOW - 30 * DAY - d * DAY)));
        list.extend([("ok", NOW - 27 * DAY), ("ok", NOW - DAY), ("ok", NOW - 2 * DAY)]);
        plays(&lib, &list);
        let t = lib.taste_at(NOW).unwrap();
        let ids: Vec<String> = lib.songs_by_rowid(&t.on_repeat).unwrap().into_iter().map(|(_, s)| s.id).collect();
        assert_eq!(ids, ["loop", "ok"]);
        // Without dated plays there's no telling, so the row hides.
        let none = library(&tempfile::tempdir().unwrap(), &songs, &[("a", 1)]);
        assert!(none.taste_at(NOW).unwrap().on_repeat.is_empty());
    }

    #[test]
    fn back_in_rotation_wants_old_favourites_gone_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let songs = [song("a1", "a", 30, Some(NOW - 200 * DAY)), song("b1", "b", 30, Some(NOW - DAY)), song("c1", "c", 4, Some(NOW - 300 * DAY)), song("d1", "d", 21, Some(NOW - 10 * DAY))];
        let lib = library(&dir, &songs, &[("a", 1), ("b", 1), ("c", 1), ("d", 1)]);
        let mut list: Vec<(&str, i64)> = Vec::new();
        // a: 20 old plays, none lately. b: 20 old, 10 lately. c: only 4 old. d: 20 old, 1 lately.
        list.extend((0..20).map(|d| ("a1", NOW - 200 * DAY - d * DAY)));
        list.extend((0..20).map(|d| ("b1", NOW - 220 * DAY - d * DAY)));
        list.extend((0..10).map(|d| ("b1", NOW - d * DAY)));
        list.extend((0..4).map(|d| ("c1", NOW - 300 * DAY - d * DAY)));
        list.extend((0..20).map(|d| ("d1", NOW - 250 * DAY - d * DAY)));
        list.push(("d1", NOW - 10 * DAY));
        plays(&lib, &list);
        let t = lib.taste_at(NOW).unwrap();
        let ids: Vec<String> = lib.artists_by_rowid(&t.back_in_rotation).unwrap().into_iter().map(|(_, a)| a.id).collect();
        assert_eq!(ids, ["a", "d"]);
    }

    fn rowid_of(lib: &Library, id: &str) -> u32 {
        lib.artist_rowids(&[id.to_string()]).unwrap()[0]
    }

    #[test]
    fn because_rotates_skips_world_of_and_top_artists_and_needs_four() {
        let dir = tempfile::tempdir().unwrap();
        // Top five a..e (each 4 albums, so all World of candidates too), then similar ones.
        let names = ["a", "b", "c", "d", "e"];
        let songs: Vec<Song> = names.iter().enumerate().map(|(i, n)| song(&format!("s{n}"), n, 10, Some(NOW - (i as i64 + 1) * DAY))).collect();
        let mut artists: Vec<(&str, u32)> = names.iter().map(|n| (*n, 4)).collect();
        artists.extend([("s1", 1), ("s2", 1), ("s3", 1), ("s4", 1), ("s5", 1)]);
        let lib = library(&dir, &songs, &artists);
        let t = lib.taste_at(NOW).unwrap();
        let day = 1000;
        let world = t.world_of(day).unwrap().id.clone();
        let seeds: Vec<String> = t.because_seeds(day).iter().map(|s| s.id.clone()).collect();
        assert_eq!(seeds.len(), 4);
        assert!(!seeds.contains(&world));
        // Stable all day, and tomorrow starts somewhere else.
        assert_eq!(seeds, t.because_seeds(day).iter().map(|s| s.id.clone()).collect::<Vec<_>>());
        assert_ne!(seeds[0], t.because_seeds(day + 1)[0].id);
        // Nothing asked yet: ask the first seed.
        assert_eq!(t.because(day, |_| None), Because::Ask(seeds[0].clone()));
        let row = |ids: &[&str]| ids.iter().map(|i| rowid_of(&lib, i)).collect::<Vec<u32>>();
        // First seed's similar artists are mostly top artists: too few left, so try the next.
        let first = row(&["a", "b", "c", "s1", "s2"]);
        let second = row(&["s1", "s2", "s3", "s4", "e", "s5"]);
        let answer = t.because(day, |s| Some(if s.id == seeds[0] { first.clone() } else { second.clone() }));
        assert_eq!(answer, Because::Ready { seed: rowid_of(&lib, &seeds[1]), artists: row(&["s1", "s2", "s3", "s4", "s5"]) });
        // Nobody with enough similar artists: no row.
        assert_eq!(t.because(day, |_| Some(first.clone())), Because::Nothing);
    }

    #[test]
    fn home_turn_moves_on_each_launch_and_at_midnight_only() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(&dir, &[song("x", "a", 1, Some(NOW - DAY))], &[("a", 1)]);
        // First ask this launch: one step on. Then steady for the session.
        assert_eq!(lib.home_turn(10).unwrap(), 1);
        assert_eq!(lib.home_turn(10).unwrap(), 1);
        assert_eq!(lib.clone().home_turn(10).unwrap(), 1);
        // Local midnight while running: one more.
        assert_eq!(lib.home_turn(11).unwrap(), 2);
        assert_eq!(lib.home_turn(11).unwrap(), 2);
        // The next launch (the library opened again) moves on from the saved turn.
        drop(lib);
        let again = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        assert_eq!(again.home_turn(11).unwrap(), 3);
        assert_eq!(again.home_turn(11).unwrap(), 3);
    }

    #[test]
    fn world_of_and_because_follow_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let lib = world(&dir);
        let t = lib.taste_at(NOW).unwrap();
        let first = lib.world_of(5).unwrap().unwrap().artist.id;
        assert_eq!(first, t.world_of(1).unwrap().id);
        let next = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        let second = next.world_of(5).unwrap().unwrap().artist.id;
        assert_eq!(second, t.world_of(2).unwrap().id);
        assert_ne!(first, second);
        assert!(!t.because_seeds(2).iter().any(|s| s.id == second));
    }

    #[test]
    fn similar_answers_last_one_day() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(&dir, &[song("x", "a", 1, Some(NOW - DAY))], &[("a", 1), ("b", 1)]);
        let before = lib.version();
        lib.store_similar("a", 10, &["b".into(), "gone".into()]).unwrap();
        assert_eq!(lib.version(), before, "the taste cache stays");
        assert_eq!(lib.similar_for_day("a", 10).unwrap(), Some(vec!["b".to_string(), "gone".to_string()]));
        assert_eq!(lib.similar_for_day("a", 11).unwrap(), None);
        assert_eq!(lib.artist_rowids(&["b".into(), "gone".into()]).unwrap().len(), 1);
    }
}
