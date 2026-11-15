//! Plays songs from the server at a chosen quality through a real device. Run it with
//! `PIPEWIRE_NODE` set to a private null sink, and keep runs short since transcoding uses
//! the server's CPU. Login comes from `NAVIDROME_URL`, `NAVIDROME_USER` and `NAVIDROME_PASSWORD`.
//!
//! `transcode_check <original|mp3|opus|aac> <kbps> <seconds to play> <song ids...> [--seek ms]`

use std::sync::Arc;
use std::time::{Duration, Instant};

use navia_core::audio::player::{OutputPolicy, Player, PlayerConfig, QueueItem, ServerResolver};
use navia_core::client::{Client, ClientConfig, Credentials};
use navia_core::library::{Library, LibraryConfig};
use navia_core::quality::{Quality, QualityControl};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: transcode_check <format> <kbps> <seconds> <song ids...> [--seek ms]");
        std::process::exit(2);
    }
    let quality = Quality::new(&args[0], args[1].parse().unwrap_or(0)).expect("quality");
    let seconds: f64 = args[2].parse().expect("seconds");
    let mut ids = Vec::new();
    let mut seek = None;
    let mut cache = false;
    let mut leveling: Option<String> = None;
    let mut rg: Option<String> = None;
    let mut rest = args[3..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--cache" {
            cache = true;
        } else if arg == "--level" {
            leveling = rest.next().cloned();
        } else if arg == "--rg" {
            rg = rest.next().cloned();
        } else if arg == "--seek" {
            seek = rest.next().and_then(|v| v.parse::<u64>().ok());
        } else {
            ids.push(arg.clone());
        }
    }
    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let mut config = ClientConfig::new(env("NAVIDROME_URL"), Credentials::new(env("NAVIDROME_USER"), env("NAVIDROME_PASSWORD")));
    config = config.read_only();
    let client = Client::new(config).expect("client");
    let dir = tempfile::tempdir().expect("temp dir");
    let library = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).expect("library");
    let control = QualityControl::new(library, client.clone(), cache.then(|| dir.path().join("cache"))).expect("quality");
    control.set_streaming(quality).expect("set quality");
    if let Some(mode) = leveling {
        let mode = navia_core::quality::Leveling::from_id(&mode).expect("leveling mode");
        control.set_leveling(mode).expect("set leveling");
        // The server's ReplayGain for the song, stored as a sync would.
        let rg = rg.expect("--rg with --level");
        let conn = rusqlite::Connection::open(dir.path().join("lib.sqlite")).expect("sqlite");
        conn.execute_batch("PRAGMA foreign_keys=OFF").expect("pragma");
        let data = format!(r#"{{"id":"{}","title":"x","replayGain":{rg}}}"#, ids[0]);
        conn.execute("INSERT INTO song(id, library_id, title, sort_name, album_id, artist_id, track, disc, year, duration, created, play_count, played, starred, generation, content_hash, data) VALUES (?1,'l',?1,?1,'a','a',1,1,0,0,'',0,NULL,NULL,0,'h',?2)", rusqlite::params![ids[0], data]).expect("insert");
    }

    let resolver = Arc::new(ServerResolver::new(client).expect("resolver").with_quality(control));
    let player = Player::new(resolver, PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).expect("player");
    player.set_volume(1.0);
    let items = ids.iter().map(|id| QueueItem::new(id.clone(), None)).collect();
    player.set_queue(items, 0, 0, false);
    let start = Instant::now();
    let mut sought = seek.is_none();
    while start.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(Duration::from_millis(50));
        if !sought && player.snapshot().position_ms > (if cache { 4000 } else { 1500 }) {
            sought = true;
            player.seek_ms(seek.unwrap());
            eprintln!("seek {} ms at {:.3}s", seek.unwrap(), start.elapsed().as_secs_f64());
        }
    }
    let snap = player.snapshot();
    println!("status {:?} position {} delivered {:?} rate {} underruns {} rebuffer {} xruns {}", snap.status, snap.position_ms, snap.delivered, snap.output_rate, snap.underrun_frames, snap.rebuffer_frames, snap.xruns);
    player.stop();
}
