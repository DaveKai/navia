//! Measures Artist Radio from Deezer on a real library: how many tracks match library songs, how,
//! and how long it takes. Read-only on the server. It turns Deezer on in the database, so pass a copy.
//!
//! `NAVIDROME_URL=... NAVIDROME_USER=... NAVIDROME_PASSWORD=... cargo run -p navia-core --example
//! artist_radio_check -- <copy of library.sqlite> "Artist|Artist|..."`
//!
//! `NAVIA_RADIO_VERIFY=1` also checks name matches against Deezer's ISRCs.

use std::time::{Duration, Instant};

use navia_core::artist_radio::{Deezer, DeezerUse, artist_radio};
use navia_core::client::{Client, ClientConfig, Credentials, WritePolicy};
use navia_core::library::{Library, LibraryConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: artist_radio_check <library.sqlite> \"A|B\"")?;
    let artists = std::env::args().nth(2).ok_or("usage: artist_radio_check <library.sqlite> \"A|B\"")?;
    let verify = std::env::var_os("NAVIA_RADIO_VERIFY").is_some();
    let env = |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set"));
    let mut cfg = ClientConfig::new(env("NAVIDROME_URL")?, Credentials::new(env("NAVIDROME_USER")?, env("NAVIDROME_PASSWORD")?));
    cfg.write_policy = WritePolicy::ReadOnly;
    let client = Client::new(cfg)?;
    let library = Library::open(&LibraryConfig { path: path.into() })?;
    let deezer = Deezer::shared();
    navia_core::deezer::set_setting(&library, Some(true))?;
    println!("gate: {:?}", navia_core::deezer::gate(&library));
    let (mut all_tracks, mut all_matched, mut all_isrc, mut agree, mut differ, mut no_isrc) = (0, 0, 0, 0, 0, 0);
    for name in artists.split('|').map(str::trim).filter(|n| !n.is_empty()) {
        let Some(artist) = library.artist_by_name(name)? else {
            println!("{name}: not in the library");
            continue;
        };
        let started = Instant::now();
        let radio = artist_radio(&client, &library, deezer, &artist.id, &artist.name).await;
        let took = started.elapsed();
        match &radio.deezer {
            DeezerUse::Used(used) => {
                all_tracks += used.tracks;
                all_matched += used.songs.len();
                all_isrc += used.by_isrc;
                let rate = if used.tracks > 0 { 100.0 * used.songs.len() as f64 / used.tracks as f64 } else { 0.0 };
                println!(
                    "{name}: artist found {}, {} tracks, {} matched ({rate:.0}%), {} by ISRC; radio {} songs = {} Deezer + {} server; {:.1} s",
                    used.found_artist,
                    used.tracks,
                    used.songs.len(),
                    used.by_isrc,
                    radio.songs.len(),
                    radio.from_deezer,
                    radio.songs.len() - radio.from_deezer,
                    took.as_secs_f64()
                );
                if verify {
                    let mut checked = 0;
                    for (song, track) in used.songs.iter().zip(&used.track_ids) {
                        if checked == 15 || song.isrc.is_empty() {
                            continue;
                        }
                        checked += 1;
                        match deezer.track_isrc(*track).await {
                            Some(isrc) if song.isrc.iter().any(|i| i.eq_ignore_ascii_case(&isrc)) => agree += 1,
                            Some(_) => {
                                differ += 1;
                                println!("    ISRC differs: \"{}\" ({})", song.title, song.album.as_deref().unwrap_or_default());
                            }
                            None => no_isrc += 1,
                        }
                    }
                }
            }
            other => println!("{name}: {other:?}; radio {} songs from the server; {:.1} s", radio.songs.len(), took.as_secs_f64()),
        }
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    if all_tracks > 0 {
        println!("\nall: {all_tracks} tracks, {all_matched} matched ({:.0}%), {all_isrc} by ISRC", 100.0 * all_matched as f64 / all_tracks as f64);
    }
    if verify {
        println!("name matches checked by ISRC: {agree} same, {differ} different (often another release of the recording), {no_isrc} without an ISRC on Deezer");
    }
    Ok(())
}
