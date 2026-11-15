//! Offline replay harness. For each held-out session it asks a ranker what comes next and
//! compares how often finished songs and early-skipped songs land in the top [`TOP`].
//! A ranker only sees the session up to that point.

use std::collections::HashSet;

use super::playlog::{PlayEvent, seeds_from};

/// How deep into the ranking counts as a hit.
pub const TOP: usize = 20;

/// What a ranker is given at each step.
pub struct Moment<'a> {
    /// The session so far, oldest first.
    pub before: &'a [PlayEvent],
    /// The seeds the engine would use now: finished songs, newest first, never skipped ones.
    pub seeds: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Sessions replayed.
    pub sessions: usize,
    /// Times the ranker was asked.
    pub asks: usize,
    pub finished_next: usize,
    pub finished_in_top: usize,
    pub skipped_next: usize,
    pub skipped_in_top: usize,
}

impl Report {
    /// Share of the next finished songs found in the top [`TOP`].
    pub fn hit_rate(&self) -> f64 {
        share(self.finished_in_top, self.finished_next)
    }

    /// Share of the next early-skipped songs found in the top [`TOP`].
    pub fn skip_rate(&self) -> f64 {
        share(self.skipped_in_top, self.skipped_next)
    }

    /// How much more often finished songs rank in the top than skipped ones.
    pub fn separation(&self) -> f64 {
        self.hit_rate() - self.skip_rate()
    }
}

fn share(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 / of as f64 }
}

/// Every [`HELD_OUT_EVERY`]th session is held out for evaluation. Models may train on the rest.
pub const HELD_OUT_EVERY: i64 = 5;

pub fn held_out(session: i64) -> bool {
    session % HELD_OUT_EVERY == 0
}

/// Replays `events` (oldest first, any number of sessions) for the sessions `evaluate` picks.
/// `rank` returns song ids, best first.
pub fn replay(events: &[PlayEvent], evaluate: impl Fn(i64) -> bool, mut rank: impl FnMut(&Moment) -> Vec<String>) -> Report {
    let mut report = Report::default();
    let mut start = 0;
    while start < events.len() {
        let session = events[start].session;
        let end = start + events[start..].iter().take_while(|e| e.session == session).count();
        if evaluate(session) {
            report.sessions += 1;
            let played = &events[start..end];
            for i in 1..played.len() {
                let next = &played[i];
                // Songs heard while listening along weren't the listener's choice.
                if next.origin == super::playlog::Origin::Along {
                    continue;
                }
                let (finished, skipped) = (next.completed(), next.early_skip());
                if !finished && !skipped {
                    continue;
                }
                let seeds = seeds_from(&played[..i], true, super::ANCHORS);
                if seeds.is_empty() {
                    continue;
                }
                let ranked = rank(&Moment { before: &played[..i], seeds });
                report.asks += 1;
                let top: HashSet<&str> = ranked.iter().take(TOP).map(String::as_str).collect();
                let hit = top.contains(next.song_id.as_str());
                if finished {
                    report.finished_next += 1;
                    report.finished_in_top += hit as usize;
                } else {
                    report.skipped_next += 1;
                    report.skipped_in_top += hit as usize;
                }
            }
        }
        start = end;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::playlog::{Ended, Origin};
    use std::collections::HashMap;

    fn event(id: i64, session: i64, song: &str, ended: Ended, heard_ms: u64) -> PlayEvent {
        PlayEvent {
            id,
            song_id: song.into(),
            artist_id: None,
            session,
            started_ms: id * 1000,
            ended_ms: Some(id * 1000 + heard_ms as i64),
            heard_ms: Some(heard_ms),
            duration_ms: Some(200_000),
            ended: Some(ended),
            origin: Origin::User,
        }
    }

    /// Sessions of a listener who finishes "rock" songs and skips "jazz" ones early, mixed in.
    fn history() -> Vec<PlayEvent> {
        let mut out = Vec::new();
        let mut n = 0;
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for session in 1..=20 {
            for _ in 0..12 {
                n += 1;
                if next() % 4 == 0 {
                    out.push(event(n, session, &format!("jazz{}", next() % 30), Ended::Skipped, 8_000));
                } else {
                    out.push(event(n, session, &format!("rock{}", next() % 30), Ended::Finished, 200_000));
                }
            }
        }
        out
    }

    #[test]
    fn replay_rewards_a_ranker_that_knows_the_listener() {
        let events = history();
        let genre: HashMap<String, &str> = (0..30).flat_map(|i| [(format!("rock{i}"), "rock"), (format!("jazz{i}"), "jazz")]).collect();
        let all: Vec<String> = genre.keys().cloned().collect();
        // Knows the seeds' genre and offers it, in a fixed order.
        let knowing = replay(&events, held_out, |m| {
            let g = genre[&m.seeds[0]];
            let mut same: Vec<String> = all.iter().filter(|s| genre[*s] == g).cloned().collect();
            same.sort();
            same
        });
        // Ignores the listener.
        let blind = replay(&events, held_out, |_| {
            let mut every = all.clone();
            every.sort();
            every
        });
        assert_eq!(knowing.sessions, 4, "sessions 5, 10, 15, 20");
        assert!(knowing.asks > 30 && knowing.asks == blind.asks);
        assert!(knowing.hit_rate() > 0.5, "{knowing:?}");
        assert_eq!(knowing.skip_rate(), 0.0, "a skipped jazz song is never offered after rock seeds");
        assert!(knowing.separation() > blind.separation() + 0.3, "{knowing:?} {blind:?}");
    }

    #[test]
    fn the_ranker_never_sees_the_song_it_is_judged_on() {
        let events = history();
        replay(&events, |_| true, |m| {
            let next = m.before.len();
            assert!(m.before.iter().all(|e| e.id <= m.before[next - 1].id));
            Vec::new()
        });
        let report = replay(&events, |s| s == 1, |m| m.seeds.clone());
        assert_eq!(report.sessions, 1);
    }
}
