//! Compares Autoplay's Server and Navia modes on the same seeds. Read-only on the server.
//! It switches the mode in the database, so pass a copy.
//!
//! `NAVIDROME_URL=... NAVIDROME_USER=... NAVIDROME_PASSWORD=... cargo run -p navia-core --example
//! autoplay_compare -- <copy of library.sqlite> [song id,song id,...]`
//!
//! Without ids, the seeds are the last five songs the server says were played.

use std::collections::HashSet;
use std::time::Instant;

use navia_core::autoplay::{Autoplay, fetch_count, main_artist};
use navia_core::client::{Client, ClientConfig, Credentials, Song, WritePolicy};
use navia_core::engine::{self, AutoplayMode};
use navia_core::library::{Library, LibraryConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: autoplay_compare <library.sqlite> [ids]")?;
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set"));
    let mut cfg = ClientConfig::new(env("NAVIDROME_URL")?, Credentials::new(env("NAVIDROME_USER")?, env("NAVIDROME_PASSWORD")?));
    cfg.write_policy = WritePolicy::ReadOnly;
    let client = Client::new(cfg)?;
    let db = Library::open(&LibraryConfig { path: path.clone().into() })?;
    let chosen: Vec<String> = match std::env::args().nth(2) {
        Some(ids) => ids.split(',').map(str::to_string).collect(),
        None => {
            let c = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut st = c.prepare("SELECT id FROM song WHERE played IS NOT NULL ORDER BY played DESC LIMIT 5")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?
        }
    };
    // Oldest first, like the queue.
    let chosen: Vec<String> = chosen.into_iter().rev().collect();
    println!("seeds (oldest first):");
    for id in &chosen {
        if let Some(s) = db.song(id)? {
            println!("  {} - {} [{}]", main_artist(&s), s.title, s.genre.as_deref().unwrap_or(""));
        }
    }
    for mode in [AutoplayMode::Server, AutoplayMode::Navia] {
        engine::set_mode(&db, mode)?;
        let mut a = Autoplay::new();
        a.attach(db.clone());
        a.set_chosen(chosen.iter().map(String::as_str));
        let started = Instant::now();
        let mut asks = 0;
        while let Some(seeds) = a.wanted_fetch() {
            a.begin_fetch(&seeds);
            asks += seeds.len();
            let answers = fetch_count(&client, &seeds, a.per_seed()).await;
            a.finish_fetch(seeds, answers);
        }
        let took = started.elapsed();
        let queue: Vec<_> = chosen.iter().filter_map(|id| db.song(id).ok().flatten()).map(|s| navia_core::autoplay::Taken::of(&s)).collect();
        let offer = a.songs(&queue, 20);
        println!("\n{mode:?}: {asks} asks of {} in {:.1} s", a.per_seed(), took.as_secs_f64());
        for s in &offer {
            println!("  {:<28} {:<40} plays {:>3}{}", main_artist(s), s.title.chars().take(40).collect::<String>(), s.play_count.unwrap_or(0), if s.starred.is_some() { " *" } else { "" });
        }
        report(&offer, &chosen, &db);
    }
    Ok(())
}

fn report(offer: &[Song], chosen: &[String], db: &Library) {
    let first10 = &offer[..offer.len().min(10)];
    let artists: Vec<&str> = first10.iter().map(main_artist).collect();
    let distinct: HashSet<&&str> = artists.iter().collect();
    let close = (1..artists.len()).filter(|&i| artists[i.saturating_sub(4)..i].contains(&artists[i])).count();
    let seed_artists: HashSet<String> = chosen.iter().filter_map(|id| db.song(id).ok().flatten()).map(|s| main_artist(&s).to_string()).collect();
    let same_as_seed = first10.iter().filter(|s| seed_artists.contains(main_artist(s))).count();
    let played = first10.iter().filter(|s| s.play_count.unwrap_or(0) > 0).count();
    let starred = first10.iter().filter(|s| s.starred.is_some()).count();
    println!("  first 10: {} artists, {close} within 4 of the same artist, {same_as_seed} by a seed's artist, {played} played before, {starred} starred", distinct.len());
}
