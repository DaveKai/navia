//! The local play log: one `play_event` row per play, with how much was heard, how it ended,
//! who chose it and its session. Nothing here is sent to the server.
//!
//! A row left open when the app quit counts for nothing. Plays heard while listening along are
//! kept as [`Origin::Along`] and say nothing about the listener's own taste.

use std::collections::HashSet;
use std::sync::mpsc;

use rusqlite::{OptionalExtension, params};

use super::AutoplayMode;
use crate::audio::player::{PlayEnd, Player, PlayerEvent};
use crate::library::Library;

/// Silence that starts a new session.
pub const SESSION_GAP_MS: i64 = 30 * 60 * 1000;
/// A skip before this much was heard is an early skip, meaning "not this".
pub const EARLY_SKIP_MS: u64 = 30_000;
/// The share of a song that must be heard to count as listened through.
pub const SEED_HEARD: f64 = 0.5;

/// Who chose a song.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    User,
    /// Autoplay offered it, in this mode.
    Autoplay(AutoplayMode),
    /// Heard while listening along with someone.
    Along,
}

/// How a play ended, as the log keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Finished,
    Skipped,
    Replaced,
}

impl Ended {
    fn as_str(self) -> &'static str {
        match self {
            Ended::Finished => "finished",
            Ended::Skipped => "skipped",
            Ended::Replaced => "replaced",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "finished" => Some(Ended::Finished),
            "skipped" => Some(Ended::Skipped),
            "replaced" => Some(Ended::Replaced),
            _ => None,
        }
    }
}

impl From<PlayEnd> for Ended {
    fn from(end: PlayEnd) -> Self {
        match end {
            PlayEnd::Finished => Ended::Finished,
            PlayEnd::Skipped => Ended::Skipped,
            PlayEnd::Replaced => Ended::Replaced,
        }
    }
}

/// One play.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayEvent {
    pub id: i64,
    pub song_id: String,
    pub artist_id: Option<String>,
    pub session: i64,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub heard_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// None while playing, or when the app quit during it.
    pub ended: Option<Ended>,
    pub origin: Origin,
}

impl PlayEvent {
    /// Played to its end.
    pub fn completed(&self) -> bool {
        self.ended == Some(Ended::Finished)
    }

    /// Played to its end or replaced near it, with at least [`SEED_HEARD`] heard, so it may
    /// seed. Seeking to the end doesn't count.
    pub fn listened_through(&self) -> bool {
        let heard = match (self.heard_ms, self.duration_ms) {
            (Some(h), Some(d)) if d > 0 => h as f64 >= SEED_HEARD * d as f64,
            _ => return self.completed(),
        };
        heard && matches!(self.ended, Some(Ended::Finished | Ended::Replaced))
    }

    /// Skipped with Next before [`EARLY_SKIP_MS`].
    pub fn early_skip(&self) -> bool {
        self.ended == Some(Ended::Skipped) && self.heard_ms.unwrap_or(0) < EARLY_SKIP_MS
    }

    /// Skipped with Next after [`EARLY_SKIP_MS`]. Barely counts.
    pub fn late_skip(&self) -> bool {
        self.ended == Some(Ended::Skipped) && self.heard_ms.unwrap_or(0) >= EARLY_SKIP_MS
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The session for a play starting at `now`. A new one unless something played within
/// [`SESSION_GAP_MS`].
fn session_at(c: &rusqlite::Connection, now: i64) -> rusqlite::Result<i64> {
    let last: Option<(i64, i64)> = c
        .query_row("SELECT session, max(started_ms, coalesce(ended_ms, started_ms + coalesce(duration_ms, 0))) FROM play_event ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match last {
        Some((session, last_ms)) if now - last_ms <= SESSION_GAP_MS => session,
        Some((session, _)) => session + 1,
        None => 1,
    })
}

/// The current session, if one is still going at `now`.
pub fn current_session(db: &Library, now: i64) -> Option<i64> {
    db.read(|c| {
        let last: Option<(i64, i64)> = c
            .query_row("SELECT session, max(started_ms, coalesce(ended_ms, started_ms + coalesce(duration_ms, 0))) FROM play_event ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(last.filter(|(_, at)| now - at <= SESSION_GAP_MS).map(|(s, _)| s))
    })
    .ok()
    .flatten()
}

/// Writes the start of a play and returns its row.
pub fn record_start(db: &Library, song_id: &str, duration_ms: Option<u64>, origin: Origin, now: i64) -> crate::Result<i64> {
    let artist_id = db.song(song_id).ok().flatten().map(|s| crate::recommend::artist_key(&s));
    let (origin_text, mode) = match origin {
        Origin::User => ("user", None),
        Origin::Autoplay(mode) => ("autoplay", Some(mode.as_str())),
        Origin::Along => ("along", None),
    };
    db.write_quiet(|tx| {
        let session = session_at(tx, now)?;
        tx.execute(
            "INSERT INTO play_event(song_id, artist_id, session, started_ms, duration_ms, origin, mode) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![song_id, artist_id, session, now, duration_ms.map(|d| d as i64), origin_text, mode],
        )?;
        Ok(tx.last_insert_rowid())
    })
}

/// Completes a play's row and updates its song and artist counts. Plays heard `along` are marked
/// and don't count.
pub fn record_end(db: &Library, row: i64, heard_ms: u64, ended: Ended, along: bool, now: i64) -> crate::Result<()> {
    db.write_quiet(|tx| {
        let changed = tx.execute("UPDATE play_event SET ended_ms=?2, heard_ms=?3, ended=?4 WHERE id=?1 AND ended IS NULL", params![row, now, heard_ms as i64, ended.as_str()])?;
        if changed == 0 {
            return Ok(());
        }
        if along {
            tx.execute("UPDATE play_event SET origin='along', mode=NULL WHERE id=?1", [row])?;
            return Ok(());
        }
        let (song, artist): (String, Option<String>) = tx.query_row("SELECT song_id, artist_id FROM play_event WHERE id=?1", [row], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let early = ended == Ended::Skipped && heard_ms < EARLY_SKIP_MS;
        let late = ended == Ended::Skipped && !early;
        let done = ended == Ended::Finished;
        if !(done || early || late) {
            return Ok(());
        }
        tx.execute(
            "INSERT INTO song_stat(song_id, completions, early_skips, late_skips, last_completed_ms, last_early_skip_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(song_id) DO UPDATE SET completions = completions + excluded.completions, early_skips = early_skips + excluded.early_skips,
               late_skips = late_skips + excluded.late_skips, last_completed_ms = coalesce(excluded.last_completed_ms, last_completed_ms),
               last_early_skip_ms = coalesce(excluded.last_early_skip_ms, last_early_skip_ms)",
            params![song, done as i64, early as i64, late as i64, done.then_some(now), early.then_some(now)],
        )?;
        if let Some(artist) = artist.filter(|a| !a.is_empty()) {
            tx.execute(
                "INSERT INTO artist_stat(artist_id, completions, early_skips, late_skips) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(artist_id) DO UPDATE SET completions = completions + excluded.completions, early_skips = early_skips + excluded.early_skips, late_skips = late_skips + excluded.late_skips",
                params![artist, done as i64, early as i64, late as i64],
            )?;
        }
        Ok(())
    })
}

fn event_from_row(r: &rusqlite::Row) -> rusqlite::Result<PlayEvent> {
    let origin: String = r.get(9)?;
    let mode: Option<String> = r.get(10)?;
    let ended: Option<String> = r.get(8)?;
    Ok(PlayEvent {
        id: r.get(0)?,
        song_id: r.get(1)?,
        artist_id: r.get(2)?,
        session: r.get(3)?,
        started_ms: r.get(4)?,
        ended_ms: r.get(5)?,
        heard_ms: r.get::<_, Option<i64>>(6)?.map(|v| v.max(0) as u64),
        duration_ms: r.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
        ended: ended.as_deref().and_then(Ended::parse),
        origin: match origin.as_str() {
            "autoplay" => Origin::Autoplay(mode.as_deref().and_then(AutoplayMode::parse).unwrap_or_default()),
            "along" => Origin::Along,
            _ => Origin::User,
        },
    })
}

const EVENT_COLUMNS: &str = "id, song_id, artist_id, session, started_ms, ended_ms, heard_ms, duration_ms, ended, origin, mode";

/// Every play from `since_ms` on, oldest first.
pub fn events_since(db: &Library, since_ms: i64) -> crate::Result<Vec<PlayEvent>> {
    db.read(|c| {
        let mut st = c.prepare(&format!("SELECT {EVENT_COLUMNS} FROM play_event WHERE started_ms >= ?1 ORDER BY id"))?;
        let rows = st.query_map([since_ms], event_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// The plays of one session, oldest first.
pub fn session_events(db: &Library, session: i64) -> crate::Result<Vec<PlayEvent>> {
    db.read(|c| {
        let mut st = c.prepare(&format!("SELECT {EVENT_COLUMNS} FROM play_event WHERE session = ?1 ORDER BY id"))?;
        let rows = st.query_map([session], event_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Seeds from the current session: songs listened through, newest first, without repeats or
/// early skips. Autoplay's own picks only when `with_autoplay`.
pub fn seed_ids(db: &Library, now: i64, with_autoplay: bool, limit: usize) -> Vec<String> {
    let Some(session) = current_session(db, now) else { return Vec::new() };
    let events = session_events(db, session).unwrap_or_default();
    seeds_from(&events, with_autoplay, limit)
}

/// [`seed_ids`] over a session's events, oldest first.
pub fn seeds_from(events: &[PlayEvent], with_autoplay: bool, limit: usize) -> Vec<String> {
    let skipped: HashSet<&str> = events.iter().filter(|e| e.early_skip() && e.origin != Origin::Along).map(|e| e.song_id.as_str()).collect();
    let mut out: Vec<String> = Vec::new();
    for e in events.iter().rev() {
        if out.len() >= limit {
            break;
        }
        let chosen = match e.origin {
            Origin::User => true,
            Origin::Autoplay(_) => with_autoplay,
            Origin::Along => false,
        };
        if e.listened_through() && chosen && !skipped.contains(e.song_id.as_str()) && !out.contains(&e.song_id) {
            out.push(e.song_id.clone());
        }
    }
    out
}

/// Songs skipped early in the current session.
pub fn session_skipped(db: &Library, now: i64) -> HashSet<String> {
    let Some(session) = current_session(db, now) else { return HashSet::new() };
    session_events(db, session).unwrap_or_default().into_iter().filter(|e| e.early_skip() && e.origin != Origin::Along).map(|e| e.song_id).collect()
}

/// Songs heard while listening along that the listener never played themselves, newest first.
pub fn along_discoveries(db: &Library, limit: usize) -> Vec<String> {
    db.read(|c| {
        let mut st = c.prepare(
            "SELECT a.song_id FROM play_event a
             LEFT JOIN song s ON s.id = a.song_id
             WHERE a.origin = 'along' AND a.heard_ms >= ?2
               AND NOT EXISTS (SELECT 1 FROM play_event o WHERE o.song_id = a.song_id AND o.origin <> 'along')
               AND coalesce(s.play_count, 0) <= (SELECT count(*) FROM play_event n WHERE n.song_id = a.song_id AND n.origin = 'along')
             GROUP BY a.song_id ORDER BY max(a.started_ms) DESC LIMIT ?1",
        )?;
        let ids = st.query_map(params![limit as i64, EARLY_SKIP_MS as i64], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    })
    .unwrap_or_default()
}

/// A song's running counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongStat {
    pub completions: u32,
    pub early_skips: u32,
    pub late_skips: u32,
}

pub fn song_stat(db: &Library, song_id: &str) -> SongStat {
    db.read(|c| {
        Ok(c.query_row("SELECT completions, early_skips, late_skips FROM song_stat WHERE song_id=?1", [song_id], |r| Ok(SongStat { completions: r.get(0)?, early_skips: r.get(1)?, late_skips: r.get(2)? }))
            .optional()?)
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

pub fn artist_stat(db: &Library, artist_id: &str) -> SongStat {
    db.read(|c| {
        Ok(c.query_row("SELECT completions, early_skips, late_skips FROM artist_stat WHERE artist_id=?1", [artist_id], |r| Ok(SongStat { completions: r.get(0)?, early_skips: r.get(1)?, late_skips: r.get(2)? }))
            .optional()?)
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

enum Note {
    Started { song_id: String, duration_ms: Option<u64> },
    Ended { song_id: String, heard_ms: u64, end: PlayEnd, following: bool },
}

/// Records every play of `player` into the play log. Writes run on their own thread, since
/// listeners run on the player thread.
pub fn attach_play_log(player: &Player, db: Library) {
    let (tx, rx) = mpsc::channel::<Note>();
    player.add_listener(move |event: &PlayerEvent| {
        let note = match event {
            PlayerEvent::TrackChanged { item, .. } if item.duration_ms.is_some() => Note::Started { song_id: item.id.clone(), duration_ms: item.duration_ms },
            PlayerEvent::PlayEnded { item, heard_ms, end, following } => Note::Ended { song_id: item.id.clone(), heard_ms: *heard_ms, end: *end, following: *following },
            _ => return,
        };
        let _ = tx.send(note);
    });
    let spawned = std::thread::Builder::new().name("navia-playlog".into()).spawn(move || {
        let mut recorder = Recorder::new(db);
        while let Ok(note) = rx.recv() {
            recorder.note(note, now_ms());
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("play log thread didn't start: {e}");
    }
}

struct Recorder {
    db: Library,
    /// The row of the song playing, and its id.
    open: Option<(i64, String)>,
    /// Who chose the playing song, and whether the listener started it.
    open_origin: Option<(Origin, bool)>,
    /// How the last play ended, and when (for [`super::fatigue::explicit_start`]).
    last_end: Option<(Ended, i64)>,
}

impl Recorder {
    fn new(db: Library) -> Self {
        Self { db, open: None, open_origin: None, last_end: None }
    }

    fn note(&mut self, note: Note, now: i64) {
        match note {
            Note::Started { song_id, duration_ms } => {
                // The same song continuing while its queue is rebuilt is the same play.
                if self.open.as_ref().is_some_and(|(_, id)| *id == song_id) {
                    return;
                }
                let origin = self.db.engine_shared().take_offered(&song_id).map_or(Origin::User, Origin::Autoplay);
                self.open_origin = Some((origin, super::fatigue::explicit_start(self.last_end, now)));
                match record_start(&self.db, &song_id, duration_ms, origin, now) {
                    Ok(row) => self.open = Some((row, song_id)),
                    Err(e) => {
                        tracing::warn!("play log: {e}");
                        self.open = None;
                    }
                }
            }
            Note::Ended { song_id, heard_ms, end, following } => {
                let ended = Ended::from(end);
                self.last_end = Some((ended, now));
                let Some((row, id)) = self.open.take() else { return };
                if id != song_id {
                    return;
                }
                // Counts Autoplay picks skipped early in a row. Anything else the listener does ends the run.
                if let Some((origin, explicit)) = self.open_origin.take()
                    && !following
                {
                    let artist = self.db.song(&id).ok().flatten().map(|s| crate::recommend::artist_key(&s)).unwrap_or_default();
                    let picked_skip = origin == Origin::Autoplay(AutoplayMode::Navia) && !explicit && ended == Ended::Skipped && heard_ms < EARLY_SKIP_MS;
                    self.db.engine_shared().note_play_end(picked_skip.then_some(artist.as_str()));
                }
                if let Err(e) = record_end(&self.db, row, heard_ms, end.into(), following, now) {
                    tracing::warn!("play log: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::LibraryConfig;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        (dir, lib)
    }

    fn play(r: &mut Recorder, id: &str, at: i64, heard_ms: u64, end: PlayEnd) {
        r.note(Note::Started { song_id: id.into(), duration_ms: Some(200_000) }, at);
        r.note(Note::Ended { song_id: id.into(), heard_ms, end, following: false }, at + heard_ms as i64);
    }

    #[test]
    fn records_each_play_with_how_it_ended_and_who_chose_it() {
        let (_dir, lib) = library();
        lib.engine_shared().note_offered(["auto"].into_iter(), AutoplayMode::Navia);
        let mut r = Recorder::new(lib.clone());
        let t = 1_000_000_000_000;
        play(&mut r, "a", t, 200_000, PlayEnd::Finished);
        play(&mut r, "b", t + 200_000, 10_000, PlayEnd::Skipped);
        play(&mut r, "c", t + 210_000, 60_000, PlayEnd::Skipped);
        play(&mut r, "auto", t + 270_000, 5_000, PlayEnd::Replaced);
        // A song still playing when the app quits stays open.
        r.note(Note::Started { song_id: "d".into(), duration_ms: Some(200_000) }, t + 280_000);
        let events = events_since(&lib, 0).unwrap();
        let summary: Vec<(&str, Option<Ended>, Origin)> = events.iter().map(|e| (e.song_id.as_str(), e.ended, e.origin)).collect();
        assert_eq!(summary, [
            ("a", Some(Ended::Finished), Origin::User),
            ("b", Some(Ended::Skipped), Origin::User),
            ("c", Some(Ended::Skipped), Origin::User),
            ("auto", Some(Ended::Replaced), Origin::Autoplay(AutoplayMode::Navia)),
            ("d", None, Origin::User),
        ]);
        assert!(events[1].early_skip() && !events[2].early_skip() && events[2].late_skip());
        assert!(events.iter().all(|e| e.session == 1));
        assert_eq!(song_stat(&lib, "a"), SongStat { completions: 1, early_skips: 0, late_skips: 0 });
        assert_eq!(song_stat(&lib, "b"), SongStat { completions: 0, early_skips: 1, late_skips: 0 });
        assert_eq!(song_stat(&lib, "c"), SongStat { completions: 0, early_skips: 0, late_skips: 1 });
        assert_eq!(song_stat(&lib, "auto"), SongStat::default(), "replaced counts for nothing");
        // Autoplay's offer is used once.
        assert_eq!(lib.engine_shared().take_offered("auto"), None);
    }

    #[test]
    fn autoplays_picks_skipped_early_in_a_row_hold_their_artists_back() {
        use crate::client::{ArtistRef, MusicFolder, Song};
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let songs: Vec<Song> = ["p1", "p2", "p3", "p4", "mine"].iter().map(|id| Song { id: (*id).into(), title: (*id).into(), artists: vec![ArtistRef { id: format!("ar-{id}"), name: (*id).into() }], ..Default::default() }).collect();
        let cursor = lib.begin_sync("1", None, false).unwrap();
        lib.apply_songs("1", &cursor, songs.len() as u32, &songs).unwrap();
        let shared = lib.engine_shared();
        shared.set_current(&["anchor".to_string()]);
        lib.engine_shared().note_offered(["p1", "p2", "p3", "p4"].into_iter(), AutoplayMode::Navia);
        let mut r = Recorder::new(lib.clone());
        let t = now_ms() - 3_000_000;
        play(&mut r, "mine", t, 200_000, PlayEnd::Finished);
        play(&mut r, "p1", t + 200_000, 5_000, PlayEnd::Skipped);
        play(&mut r, "mine", t + 205_000, 200_000, PlayEnd::Finished);
        play(&mut r, "p2", t + 405_000, 5_000, PlayEnd::Skipped);
        assert!(shared.held_artists().is_empty(), "a song listened to in between ends the run");
        // p3 was picked by the listener and then skipped, so the run starts again at p4.
        play(&mut r, "mine", t + 410_000, 4_000, PlayEnd::Replaced);
        play(&mut r, "p3", t + 414_500, 4_000, PlayEnd::Skipped);
        assert!(shared.held_artists().is_empty());
        play(&mut r, "p4", t + 420_000, 6_000, PlayEnd::Skipped);
        assert!(shared.held_artists().is_empty());
        lib.engine_shared().note_offered(["p1"].into_iter(), AutoplayMode::Navia);
        play(&mut r, "p1", t + 427_000, 3_000, PlayEnd::Skipped);
        assert_eq!(shared.held_artists(), HashSet::from(["ar-p4".to_string(), "ar-p1".to_string()]));
    }

    #[test]
    fn a_carried_on_song_is_one_play_and_a_repeat_is_two() {
        let (_dir, lib) = library();
        let mut r = Recorder::new(lib.clone());
        let t = 1_000_000_000_000;
        r.note(Note::Started { song_id: "a".into(), duration_ms: Some(200_000) }, t);
        r.note(Note::Started { song_id: "a".into(), duration_ms: Some(200_000) }, t + 50_000);
        r.note(Note::Ended { song_id: "a".into(), heard_ms: 200_000, end: PlayEnd::Finished, following: false }, t + 200_000);
        play(&mut r, "a", t + 200_000, 200_000, PlayEnd::Finished);
        assert_eq!(events_since(&lib, 0).unwrap().len(), 2);
        assert_eq!(song_stat(&lib, "a").completions, 2);
    }

    #[test]
    fn half_an_hour_of_silence_starts_a_new_session() {
        let (_dir, lib) = library();
        let mut r = Recorder::new(lib.clone());
        let t = 1_000_000_000_000;
        play(&mut r, "a", t, 200_000, PlayEnd::Finished);
        play(&mut r, "b", t + 200_000 + SESSION_GAP_MS - 1, 200_000, PlayEnd::Finished);
        play(&mut r, "c", t + 400_000 + 2 * SESSION_GAP_MS, 200_000, PlayEnd::Finished);
        let sessions: Vec<i64> = events_since(&lib, 0).unwrap().iter().map(|e| e.session).collect();
        assert_eq!(sessions, [1, 1, 2]);
        assert_eq!(current_session(&lib, t + 600_000 + 2 * SESSION_GAP_MS), Some(2));
        assert_eq!(current_session(&lib, t + 600_000 + 3 * SESSION_GAP_MS + 1), None);
    }

    #[test]
    fn seeds_are_songs_listened_through_newest_first_never_skipped_ones() {
        let (_dir, lib) = library();
        lib.engine_shared().note_offered(["x"].into_iter(), AutoplayMode::Navia);
        let mut r = Recorder::new(lib.clone());
        let t = now_ms() - 3_000_000;
        play(&mut r, "a", t, 200_000, PlayEnd::Finished);
        play(&mut r, "b", t + 200_000, 200_000, PlayEnd::Finished);
        play(&mut r, "c", t + 400_000, 200_000, PlayEnd::Finished);
        play(&mut r, "b", t + 600_000, 5_000, PlayEnd::Skipped);
        play(&mut r, "x", t + 605_000, 200_000, PlayEnd::Finished);
        play(&mut r, "d", t + 805_000, 90_000, PlayEnd::Replaced);
        play(&mut r, "e", t + 895_000, 9_000, PlayEnd::Finished);
        play(&mut r, "g", t + 904_000, 190_000, PlayEnd::Replaced);
        let now = t + 1_200_000;
        assert_eq!(seed_ids(&lib, now, false, 5), ["g", "c", "a"], "b was skipped later, x is Autoplay's, d was replaced early, e sought to its end; g replaced near its end counts");
        assert_eq!(seed_ids(&lib, now, true, 5), ["g", "x", "c", "a"]);
        assert_eq!(seed_ids(&lib, now, true, 1), ["g"]);
        assert_eq!(session_skipped(&lib, now), HashSet::from(["b".to_string()]));
    }

    #[test]
    fn plays_heard_along_are_kept_but_never_the_listeners_taste() {
        let (_dir, lib) = library();
        let mut r = Recorder::new(lib.clone());
        let t = now_ms() - 3_000_000;
        let along = |r: &mut Recorder, id: &str, at: i64, heard: u64, end: PlayEnd| {
            r.note(Note::Started { song_id: id.into(), duration_ms: Some(200_000) }, at);
            r.note(Note::Ended { song_id: id.into(), heard_ms: heard, end, following: true }, at + heard as i64);
        };
        play(&mut r, "mine", t, 200_000, PlayEnd::Finished);
        along(&mut r, "theirs", t + 200_000, 200_000, PlayEnd::Finished);
        along(&mut r, "skipped", t + 400_000, 5_000, PlayEnd::Skipped);
        along(&mut r, "both", t + 405_000, 200_000, PlayEnd::Finished);
        play(&mut r, "both", t + 605_000, 200_000, PlayEnd::Finished);
        along(&mut r, "brief", t + 805_000, 10_000, PlayEnd::Replaced);
        let now = t + 1_000_000;
        let events = events_since(&lib, 0).unwrap();
        assert_eq!(events.iter().filter(|e| e.origin == Origin::Along).count(), 4);
        assert_eq!(seed_ids(&lib, now, true, 5), ["both", "mine"], "heard along: never a seed");
        assert!(session_skipped(&lib, now).is_empty(), "nor a skip of theirs");
        assert_eq!(song_stat(&lib, "theirs"), SongStat::default(), "nor counted");
        assert_eq!(along_discoveries(&lib, 10), ["theirs"], "new to the listener, and heard for a while");
    }
}
