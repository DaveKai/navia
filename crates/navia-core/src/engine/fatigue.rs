//! Skip fatigue: a skip means "not now", never "never". Early and late skips fill a leaky bucket
//! that halves every [`HALF_LIFE_DAYS`]. Finishing or choosing the song clears it at once.
//! A second bucket counts recent full plays so over-played songs rotate out a little.
//!
//! Fatigue only scales the Navia mode's score, by about the tie-break band, so relatedness
//! still leads. Everything is read from the local play log and nothing is stored.

use std::collections::HashMap;

use super::playlog::{EARLY_SKIP_MS, Ended, Origin, PlayEvent, events_since};
use crate::library::Library;

const DAY_MS: f64 = 86_400_000.0;
/// Both buckets halve in this many days, so a week of skips is mostly gone two weeks later.
pub const HALF_LIFE_DAYS: f64 = 5.0;
/// Events older than this weigh under 1% and aren't read.
pub const WINDOW_DAYS: i64 = 35;
/// What an early skip adds, and a late one.
pub const EARLY: f32 = 1.0;
pub const LATE: f32 = 0.2;
/// Skips this close together count once, as one occasion.
pub const SPELL_MS: i64 = 12 * 3600 * 1000;
/// The most skip fatigue a song holds, however often it was skipped.
pub const CAP: f32 = 3.0;
/// What a song at the cap loses, as a fraction of its score.
pub const SKIP_DROP: f32 = 0.25;
/// Recent full plays above this start to count, fully at [`OVERPLAY_FULL`], where the song
/// loses [`OVERPLAY_DROP`].
pub const OVERPLAY_FROM: f32 = 3.0;
pub const OVERPLAY_FULL: f32 = 8.0;
pub const OVERPLAY_DROP: f32 = 0.12;
/// The lowest combined multiplier. A little beyond the tie-break band (`plan::BAND`), never
/// enough to put a clearly less related song first.
pub const FLOOR: f32 = 0.75;
/// The most recency fatigue Fewer Repeats gets from skips. A song heard this session is 1.
pub const RECENCY_MAX: f32 = 0.4;
/// A song starting within this of the previous one being replaced was started by the listener.
/// It can take a few seconds on a slow connection for the new song to be heard.
pub const EXPLICIT_GAP_MS: i64 = 15_000;

/// One song's fatigue now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fatigue {
    /// Skip fatigue, 0 to [`CAP`].
    pub skips: f32,
    /// Recent listened-through plays, decayed.
    pub plays: f32,
}

impl Fatigue {
    /// What the song's score is multiplied by: 1 when fresh, never below [`FLOOR`].
    pub fn multiplier(&self) -> f32 {
        let skip = 1.0 - SKIP_DROP * (self.skips / CAP).clamp(0.0, 1.0);
        let over = 1.0 - OVERPLAY_DROP * ((self.plays - OVERPLAY_FROM) / (OVERPLAY_FULL - OVERPLAY_FROM)).clamp(0.0, 1.0);
        (skip * over).max(FLOOR)
    }

    /// Fewer Repeats' recency from skip fatigue, 0 to [`RECENCY_MAX`]. Over-play is already in
    /// the server's last-played date.
    pub fn recency(&self) -> f32 {
        RECENCY_MAX * (self.skips / CAP).clamp(0.0, 1.0)
    }
}

/// Whether a play was started by the listener, meaning the one before was replaced within
/// [`EXPLICIT_GAP_MS`].
pub fn explicit_start(previous: Option<(Ended, i64)>, started_ms: i64) -> bool {
    matches!(previous, Some((Ended::Replaced, at)) if (0..=EXPLICIT_GAP_MS).contains(&(started_ms - at)))
}

fn decay(from_ms: i64, to_ms: i64) -> f32 {
    0.5f64.powf((to_ms - from_ms).max(0) as f64 / DAY_MS / HALF_LIFE_DAYS) as f32
}

#[derive(Default)]
struct Bucket {
    skips: f32,
    skips_at: i64,
    /// The spell under way: when it started and what it has added.
    spell: Option<(i64, f32)>,
    plays: f32,
    plays_at: i64,
}

impl Bucket {
    fn skip(&mut self, w: f32, at: i64) {
        if let Some((start, added)) = self.spell
            && at - start < SPELL_MS
        {
            if w > added {
                self.skips = (self.skips * decay(self.skips_at, at) + w - added).min(CAP);
                self.skips_at = at;
                self.spell = Some((start, w));
            }
            return;
        }
        self.skips = (self.skips * decay(self.skips_at, at) + w).min(CAP);
        self.skips_at = at;
        self.spell = Some((at, w));
    }

    fn clear(&mut self) {
        self.skips = 0.0;
        self.spell = None;
    }

    fn play(&mut self, at: i64) {
        self.plays = self.plays * decay(self.plays_at, at) + 1.0;
        self.plays_at = at;
    }

    fn now(&self, now: i64) -> Fatigue {
        Fatigue { skips: self.skips * decay(self.skips_at, now), plays: self.plays * decay(self.plays_at, now) }
    }
}

/// Each song's fatigue at `now` from the play log, oldest first. Songs with none are left out.
pub fn from_events(events: &[PlayEvent], now: i64) -> HashMap<String, Fatigue> {
    let mut buckets: HashMap<&str, Bucket> = HashMap::new();
    let mut previous: Option<(Ended, i64)> = None;
    for e in events {
        let explicit = explicit_start(previous, e.started_ms);
        previous = e.ended.zip(e.ended_ms);
        if e.origin == Origin::Along || e.started_ms > now {
            continue;
        }
        let b = buckets.entry(e.song_id.as_str()).or_default();
        let end = e.ended_ms.unwrap_or(e.started_ms);
        if explicit {
            b.clear();
        }
        if e.listened_through() {
            b.clear();
            b.play(end);
        } else if !explicit && e.ended == Some(Ended::Skipped) {
            b.skip(if e.heard_ms.unwrap_or(0) < EARLY_SKIP_MS { EARLY } else { LATE }, end);
        }
    }
    buckets.into_iter().map(|(id, b)| (id.to_string(), b.now(now))).filter(|(_, f)| f.skips > 1e-3 || f.plays > 1e-3).collect()
}

/// Each song's fatigue now, from the last [`WINDOW_DAYS`] of the play log.
pub fn load(db: &Library, now: i64) -> HashMap<String, Fatigue> {
    let since = now - WINDOW_DAYS * DAY_MS as i64;
    from_events(&events_since(db, since).unwrap_or_default(), now)
}

/// [`load`] as score multipliers, only for songs under 1.
pub fn multipliers(db: &Library, now: i64) -> HashMap<String, f32> {
    load(db, now).into_iter().map(|(id, f)| (id, f.multiplier())).filter(|(_, m)| *m < 1.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000_000;
    const DAY: i64 = 86_400_000;

    struct Log(Vec<PlayEvent>);

    impl Log {
        fn new() -> Self {
            Self(Vec::new())
        }

        fn add(&mut self, id: &str, at: i64, heard_ms: u64, ended: Ended, origin: Origin) -> &mut Self {
            let n = self.0.len() as i64;
            self.0.push(PlayEvent {
                id: n + 1,
                song_id: id.into(),
                artist_id: None,
                session: 1,
                started_ms: at,
                ended_ms: Some(at + heard_ms as i64),
                heard_ms: Some(heard_ms),
                duration_ms: Some(200_000),
                ended: Some(ended),
                origin,
            });
            self
        }

        /// A song reached by advancing, skipped after `heard` ms.
        fn skip(&mut self, id: &str, at: i64, heard: u64) -> &mut Self {
            self.add("before", at - 200_000, 200_000, Ended::Finished, Origin::User).add(id, at, heard, Ended::Skipped, Origin::User)
        }

        fn at(&self, id: &str, now: i64) -> Fatigue {
            from_events(&self.0, now).get(id).copied().unwrap_or_default()
        }
    }

    #[test]
    fn an_early_skip_fades_by_half_every_few_days() {
        let mut log = Log::new();
        log.skip("s", T0, 5_000);
        let f0 = log.at("s", T0 + 10_000);
        assert!((f0.skips - EARLY).abs() < 1e-3);
        let m0 = f0.multiplier();
        assert!(m0 < 1.0 && m0 > 0.9, "one skip is a nudge: {m0}");
        let half = log.at("s", T0 + (HALF_LIFE_DAYS * DAY as f64) as i64);
        assert!((half.skips - EARLY / 2.0).abs() < 0.01, "{half:?}");
        assert!(log.at("s", T0 + 30 * DAY).multiplier() > 0.995, "a month on it's gone");
    }

    #[test]
    fn late_skips_barely_count_and_explicit_or_along_ones_not_at_all() {
        let mut log = Log::new();
        log.skip("late", T0, 90_000);
        assert!((log.at("late", T0 + 100_000).skips - LATE).abs() < 1e-3);
        // Started by the listener, then skipped early. They changed their mind, so it doesn't count.
        log.add("x", T0 + DAY, 20_000, Ended::Replaced, Origin::User).add("picked", T0 + DAY + 22_000, 4_000, Ended::Skipped, Origin::User);
        assert_eq!(log.at("picked", T0 + DAY + 30_000), Fatigue::default());
        // Heard along with someone.
        log.add("along", T0 + 2 * DAY, 3_000, Ended::Skipped, Origin::Along);
        assert_eq!(log.at("along", T0 + 2 * DAY + 10_000), Fatigue::default());
        // Autoplay's pick, skipped early: counts.
        log.add("y", T0 + 3 * DAY, 200_000, Ended::Finished, Origin::User).add("auto", T0 + 3 * DAY + 200_000, 6_000, Ended::Skipped, Origin::Autoplay(super::super::AutoplayMode::Navia));
        assert!(log.at("auto", T0 + 3 * DAY + 210_000).skips > 0.99);
    }

    #[test]
    fn skips_on_separate_days_build_up_to_the_cap_and_one_day_counts_once() {
        let mut log = Log::new();
        for k in 0..4 {
            log.skip("same", T0 + k * 600_000, 3_000);
        }
        let one = log.at("same", T0 + DAY / 2).skips;
        assert!(one <= EARLY && one > 0.9, "four skips in an hour are one occasion: {one}");
        let mut log = Log::new();
        for d in 0..10 {
            log.skip("daily", T0 + d * DAY, 3_000);
        }
        let f = log.at("daily", T0 + 9 * DAY + 10_000);
        assert!(f.skips > 2.5 && f.skips <= CAP, "{f:?}");
        assert!(f.multiplier() >= FLOOR && f.multiplier() < 0.8);
        // Never a ban. Three weeks after the last skip it's back.
        assert!(log.at("daily", T0 + 23 * DAY).multiplier() > 0.95);
        assert!(log.at("daily", T0 + 30 * DAY).multiplier() > 0.98);
    }

    #[test]
    fn listening_through_or_picking_it_clears_at_once() {
        let mut log = Log::new();
        log.skip("s", T0, 3_000).skip("s", T0 + DAY, 3_000);
        assert!(log.at("s", T0 + DAY + 10_000).skips > 1.5);
        log.add("s", T0 + 2 * DAY, 200_000, Ended::Finished, Origin::User);
        assert_eq!(log.at("s", T0 + 2 * DAY + 300_000).skips, 0.0, "finished");
        log.skip("s", T0 + 3 * DAY, 3_000);
        assert!(log.at("s", T0 + 3 * DAY + 10_000).skips > 0.99);
        // Started from Search and skipped late in the same play. The pick itself still clears it.
        log.add("other", T0 + 4 * DAY, 10_000, Ended::Replaced, Origin::User).add("s", T0 + 4 * DAY + 11_000, 40_000, Ended::Replaced, Origin::User);
        assert_eq!(log.at("s", T0 + 4 * DAY + 60_000).skips, 0.0, "picked");
    }

    #[test]
    fn over_play_rotates_a_little_and_recovers() {
        let mut log = Log::new();
        for k in 0..10 {
            log.add("hit", T0 + k * DAY / 2, 200_000, Ended::Finished, Origin::User);
        }
        let now = T0 + 5 * DAY;
        let f = log.at("hit", now);
        assert!(f.plays > OVERPLAY_FROM && f.skips == 0.0, "{f:?}");
        let m = f.multiplier();
        assert!((1.0 - OVERPLAY_DROP..1.0).contains(&m), "{m}");
        assert!(log.at("hit", now + 14 * DAY).multiplier() == 1.0, "two weeks off and it's back");
        // A few plays are just liking it.
        let mut log = Log::new();
        log.add("liked", T0, 200_000, Ended::Finished, Origin::User).add("liked", T0 + DAY, 200_000, Ended::Finished, Origin::User);
        assert_eq!(log.at("liked", T0 + DAY + 300_000).multiplier(), 1.0);
    }

    #[test]
    fn the_floor_holds_with_both() {
        let f = Fatigue { skips: CAP, plays: 50.0 };
        assert_eq!(f.multiplier(), FLOOR);
        assert!((Fatigue { skips: CAP, plays: 0.0 }.recency() - RECENCY_MAX).abs() < 1e-6);
    }

    #[test]
    fn explicit_starts_need_a_replaced_play_just_before() {
        assert!(explicit_start(Some((Ended::Replaced, T0)), T0 + 2_000));
        assert!(!explicit_start(Some((Ended::Replaced, T0)), T0 + EXPLICIT_GAP_MS + 1));
        assert!(!explicit_start(Some((Ended::Finished, T0)), T0 + 100));
        assert!(!explicit_start(Some((Ended::Skipped, T0)), T0 + 100));
        assert!(!explicit_start(None, T0));
    }
}
