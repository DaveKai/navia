//! Prints Discover's For You rows for a real library. Read-only on the server.
//!
//! `NAVIDROME_URL=... NAVIDROME_USER=... NAVIDROME_PASSWORD=... cargo run --release -p navia-core
//! --example for_you -- <copy of library.sqlite>`

use navia_core::client::{Client, ClientConfig, Credentials, WritePolicy};
use navia_core::library::for_you::{self, civil};
use navia_core::library::taste;
use navia_core::library::{Library, LibraryConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: for_you <library.sqlite>")?;
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set"));
    let mut cfg = ClientConfig::new(env("NAVIDROME_URL")?, Credentials::new(env("NAVIDROME_USER")?, env("NAVIDROME_PASSWORD")?));
    cfg.write_policy = WritePolicy::ReadOnly;
    let client = Client::new(cfg)?;
    let library = Library::open(&LibraryConfig { path: path.into() })?;
    let day = taste::local_day(taste::now_unix());
    println!("day {day} = {:?}", civil(day));
    let before = library.for_you(day)?;
    println!("before refresh: needs_refresh {}, mixes {}", before.needs_refresh, before.mixes.len());
    let t = std::time::Instant::now();
    for_you::refresh_for_you(&client, &library, day).await?;
    println!("refresh took {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let page = library.for_you(day)?;
    println!("page took {:?}, needs_refresh {}", t.elapsed(), page.needs_refresh);
    for m in &page.mixes {
        println!("Mix {} ({:?}): {} songs; {}", m.name, m.kind, m.songs.len(), m.artists.iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    let albums = |rows: &[u32]| -> Result<String, Box<dyn std::error::Error>> {
        Ok(library.albums_by_rowid(rows)?.into_iter().map(|(_, a)| format!("{} ({})", a.name, a.artist.unwrap_or_default())).collect::<Vec<_>>().join("; "))
    };
    println!("New to You {}: {}", page.new_to_you.len(), albums(&page.new_to_you)?);
    println!("Unplayed {}: {}", page.unplayed.len(), albums(&page.unplayed)?);
    let songs = library.songs_by_rowid(&page.forgotten)?;
    println!("Forgotten {}: {}", songs.len(), songs.iter().map(|(_, s)| format!("{} ({})", s.title, s.played.clone().unwrap_or_default())).collect::<Vec<_>>().join("; "));
    println!("On This Day {}: {}", page.on_this_day.len(), albums(&page.on_this_day)?);
    println!("Not Played Recently {}: {}", page.not_played_recently.len(), albums(&page.not_played_recently[..page.not_played_recently.len().min(5)])?);
    Ok(())
}
