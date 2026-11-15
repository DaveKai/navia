//! Prints the play-ranked rows by all-time counts and by recent taste, for a real library.
//!
//! `NAVIDROME_URL=... NAVIDROME_USER=... NAVIDROME_PASSWORD=... cargo run --release -p navia-core
//! --example home_rows -- <library.sqlite>`
//!
//! `getArtistInfo2` can refresh an artist's cached info on the server. `NAVIA_FALLBACK=1` skips
//! the dated plays, to see what a server without play history gets.

use navia_core::client::{Client, ClientConfig, Credentials, WritePolicy};
use navia_core::library::browse::AlbumOrder;
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::taste::{self, Because};
use navia_core::library::{Library, LibraryConfig};

fn names(list: impl IntoIterator<Item = String>) -> String {
    list.into_iter().collect::<Vec<_>>().join(", ")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: home_rows <library.sqlite>")?;
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set"));
    let mut cfg = ClientConfig::new(env("NAVIDROME_URL")?, Credentials::new(env("NAVIDROME_USER")?, env("NAVIDROME_PASSWORD")?));
    cfg.write_policy = WritePolicy::ReadOnly;
    let client = Client::new(cfg)?;
    let library = Library::open(&LibraryConfig { path: path.into() })?;
    let syncer = Syncer::new(client.clone(), library.clone(), SyncOptions { play_history: true, ..Default::default() });
    if library.counts()?.songs == 0 {
        syncer.full_sync(&|_| {}).await?;
    } else {
        syncer.refresh(false, &|_| {}).await?;
    }
    syncer.refresh_annotations().await?;
    if std::env::var_os("NAVIA_FALLBACK").is_some() {
        library.replace_recent_plays(&[])?;
    }

    println!("== All-time (before)");
    let ranked: Vec<_> = library.ranked_artists(None, Some(200))?.into_iter().filter(|a| a.plays > 0).collect();
    println!("Top artists: {}", names(ranked.iter().take(12).map(|a| format!("{} {}", a.artist.name, a.plays))));
    println!("World of: {}", ranked.iter().find(|a| a.own_albums >= 4).map_or("-".into(), |a| a.artist.name.clone()));
    let most = library.album_order(AlbumOrder::MostPlayed, None, None, None)?;
    let most: Vec<_> = library.albums_by_rowid(&most.rowids[..most.rowids.len().min(10)])?.into_iter().filter(|(_, a)| a.play_count.unwrap_or(0) > 0).collect();
    println!("Most Played: {}", names(most.iter().map(|(_, a)| format!("{} ({})", a.name, a.play_count.unwrap_or(0)))));

    let started = std::time::Instant::now();
    let t = library.taste()?;
    println!("\n== Recent taste (after), {:?}, computed in {:.1} ms", t.source, started.elapsed().as_secs_f64() * 1000.0);
    let artists = library.taste_artists(Some(12))?;
    println!("Top artists: {}", names(artists.iter().map(|a| format!("{} {:.1}", a.artist.name, a.score))));
    let day = taste::local_day(taste::now_unix());
    let candidates: Vec<String> = t.world_of_candidates().iter().map(|a| a.id.clone()).collect();
    let artist_name = |id: &str| library.artist(id).ok().flatten().map_or(id.to_string(), |a| a.name);
    println!("World of candidates: {}", names(candidates.iter().map(|id| artist_name(id))));
    println!("World of, today and the next 5 days: {}", names((0..6).map(|d| t.world_of(day + d).map_or("-".into(), |a| artist_name(&a.id)))));
    let albums = library.taste_albums(Some(10))?;
    println!("Most Played: {}", names(albums.iter().map(|(_, a)| a.name.clone())));
    let song_titles = |rows: &[u32]| names(library.songs_by_rowid(rows).unwrap_or_default().into_iter().map(|(_, s)| format!("{} - {}", s.title, s.artist.unwrap_or_default())));
    println!("Your Top Songs: {}", song_titles(&t.top_songs(10)));
    println!("On Repeat: {}", song_titles(&t.on_repeat));
    println!("Back in Rotation: {}", names(library.artists_by_rowid(&t.back_in_rotation)?.into_iter().map(|(_, a)| a.name)));
    println!("From Your Top Artists: {}", names(library.deep_cuts(40, 8)?.into_iter().map(|a| format!("{} ({})", a.name, a.artist.unwrap_or_default()))));
    println!("Because seeds today: {}", names(t.because_seeds(day).iter().map(|a| artist_name(&a.id))));
    match taste::refresh_because(&client, &library, day).await? {
        Because::Ready { seed, artists } => {
            let seed = library.artists_by_rowid(&[seed])?.into_iter().next().map(|(_, a)| a.name).unwrap_or_default();
            println!("Because you've been playing {seed}: {}", names(library.artists_by_rowid(&artists)?.into_iter().map(|(_, a)| a.name)));
        }
        other => println!("Because: {other:?}"),
    }
    Ok(())
}
