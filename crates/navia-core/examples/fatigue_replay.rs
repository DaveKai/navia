//! Replays play logs through the skip-fatigue model, offline. Reads only the given copies.
//!
//! `cargo run -p navia-core --example fatigue_replay -- <library copy>...`

use std::collections::HashMap;

use navia_core::engine::fatigue::{self, Fatigue};
use navia_core::engine::playlog::{self, Ended, Origin};
use navia_core::library::{Library, LibraryConfig};

const DAY: i64 = 86_400_000;

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    assert!(!paths.is_empty(), "usage: fatigue_replay <library copy>...");
    for path in paths {
        let db = Library::open(&LibraryConfig { path: path.clone().into() }).expect("open the copy");
        let events = playlog::events_since(&db, 0).expect("play log");
        let name = |id: &str| db.song(id).ok().flatten().map(|s| format!("{} – {}", s.title, s.artist.unwrap_or_default())).unwrap_or_else(|| id.to_string());
        println!("== {}", std::path::Path::new(&path).file_name().unwrap().to_string_lossy());
        let mut previous = None;
        let (mut explicit, mut counted, mut late, mut picked, mut along) = (0, 0, 0, 0, 0);
        for e in &events {
            let ex = fatigue::explicit_start(previous, e.started_ms);
            previous = e.ended.zip(e.ended_ms);
            explicit += ex as u32;
            if e.ended != Some(Ended::Skipped) {
                continue;
            }
            if e.origin == Origin::Along {
                along += 1;
            } else if ex {
                picked += 1;
            } else if e.early_skip() {
                counted += 1;
            } else {
                late += 1;
            }
        }
        println!(
            "{} plays in {} sessions; {} started by the listener; skips: {} early counted, {} late (light), {} after the listener's own start (not counted), {} heard along (not counted)",
            events.len(),
            events.iter().map(|e| e.session).collect::<std::collections::HashSet<_>>().len(),
            explicit,
            counted,
            late,
            picked,
            along
        );
        let last = events.iter().filter_map(|e| e.ended_ms).max().unwrap_or(0);
        let now = fatigue::from_events(&events, last + 1);
        let mut tired: Vec<(&String, &Fatigue)> = now.iter().filter(|(_, f)| f.multiplier() < 1.0).collect();
        tired.sort_by(|a, b| a.1.multiplier().total_cmp(&b.1.multiplier()));
        println!("tired at the last play: {} songs", tired.len());
        for (id, f) in tired.iter().take(8) {
            let later = |d: i64| fatigue::from_events(&events, last + d * DAY).get(*id).map_or(1.0, Fatigue::multiplier);
            println!("  ×{:.3} (skips {:.2}, plays {:.2}) → ×{:.3} in 7 days, ×{:.3} in 14, ×{:.3} in 21: {}", f.multiplier(), f.skips, f.plays, later(7), later(14), later(21), name(id));
        }
        // Returns of a song after a counted skip.
        let mut skipped_at: HashMap<&str, i64> = HashMap::new();
        let mut previous = None;
        let mut returns = Vec::new();
        for (i, e) in events.iter().enumerate() {
            let ex = fatigue::explicit_start(previous, e.started_ms);
            previous = e.ended.zip(e.ended_ms);
            if e.origin == Origin::Along {
                continue;
            }
            if let Some(at) = skipped_at.get(e.song_id.as_str()) {
                let m = fatigue::from_events(&events[..i], e.started_ms).get(&e.song_id).map_or(1.0, Fatigue::multiplier);
                returns.push(((e.started_ms - at) as f64 / 3_600_000.0, m, ex, e.ended, e.heard_ms.unwrap_or(0), e.song_id.clone()));
            }
            if !ex && e.early_skip() {
                skipped_at.insert(&e.song_id, e.ended_ms.unwrap_or(e.started_ms));
            }
        }
        println!("returns after a counted skip: {}", returns.len());
        for (hours, m, ex, ended, heard, id) in &returns {
            println!("  after {hours:.1} h: ×{m:.3}, {} start, {:?} after {} s: {}", if *ex { "listener's" } else { "advanced" }, ended, heard / 1000, name(id));
        }
        // The multiplier each Autoplay song had when it started.
        let mut by_end: HashMap<&str, (u32, f32)> = HashMap::new();
        for (i, e) in events.iter().enumerate() {
            if !matches!(e.origin, Origin::Autoplay(_)) {
                continue;
            }
            let m = fatigue::from_events(&events[..i], e.started_ms).get(&e.song_id).map_or(1.0, Fatigue::multiplier);
            let kind = if e.early_skip() { "early skip" } else if e.listened_through() { "listened through" } else { "other" };
            let v = by_end.entry(kind).or_default();
            v.0 += 1;
            v.1 += m;
        }
        for (kind, (n, sum)) in by_end {
            println!("Autoplay plays, {kind}: {n}, mean multiplier at start ×{:.3}", sum / n as f32);
        }
        println!();
    }
}
