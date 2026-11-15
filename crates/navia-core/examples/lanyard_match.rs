//! Looks up Spotify songs in a library copy the way Spotify friends are matched.
//!
//! cargo run -p navia-core --example lanyard_match -- <library.sqlite copy> "Song|Artist; Artist|Album|seconds" ...

use navia_core::lanyard::{SpotifyTrack, matching};
use navia_core::library::{Library, LibraryConfig};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a library.sqlite copy");
    let lib = Library::open(&LibraryConfig { path: path.into() }).expect("open the library");
    for case in args {
        let parts: Vec<&str> = case.split('|').collect();
        let secs: u64 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let track = SpotifyTrack {
            song: parts[0].into(),
            artist: parts.get(1).copied().unwrap_or_default().into(),
            album: parts.get(2).copied().unwrap_or_default().into(),
            start_ms: Some(1),
            end_ms: (secs > 0).then_some(1 + secs * 1000),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let hit = matching::find(&lib, &track);
        let took = started.elapsed();
        match hit {
            Some((_, s)) => println!("{case} -> {} | {} | {} | {}s ({took:?})", s.title, s.artist.unwrap_or_default(), s.album.unwrap_or_default(), s.duration.unwrap_or(0)),
            None => println!("{case} -> not in library ({took:?})"),
        }
    }
}
