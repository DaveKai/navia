//! Runs local searches on a library copy and prints top hits with timings.
//!
//! cargo run --release -p navia-core --example search_check -- <library.sqlite copy> "query" ...
//!
//! With no queries it runs a built-in set. Each query is also timed one keystroke at a time.

use navia_core::library::{Library, LibraryConfig};

const DEFAULT: &[&str] = &[
    "drake", "fleetwod mac", "fleetwoodmac", "fleetwood", "beyonse", "beyonce", "kendrik lamr", "wanna be urs",
    "rythm", "weekend", "weeknd", "landslide fleetwood", "arctic monkeys", "bjork", "天使", "雪の華", "한결", "テイラー",
    "love", "a", "the", "qqqqq", "mamas and the papas", "mamas & papas",
];

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a library.sqlite copy");
    let lib = Library::open(&LibraryConfig { path: path.into() }).expect("open the library");
    let queries: Vec<String> = {
        let given: Vec<String> = args.collect();
        if given.is_empty() { DEFAULT.iter().map(|s| s.to_string()).collect() } else { given }
    };
    let started = std::time::Instant::now();
    lib.search("warm", 20, None).unwrap();
    println!("first search (builds anything lazy): {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    let rss = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find(|l| l.starts_with("VmRSS")).map(str::to_string)).unwrap_or_default();
    println!("{rss}\n");
    let mut all = Vec::new();
    for q in &queries {
        let mut times = Vec::new();
        let mut r = Default::default();
        for _ in 0..5 {
            let t = std::time::Instant::now();
            r = lib.search(q, 20, None).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let r: navia_core::library::SearchResults = r;
        let ms = median(times);
        all.push(ms);
        println!("{q:?}  ({ms:.2} ms)");
        let artists: Vec<&str> = r.artists.iter().take(3).map(|a| a.name.as_str()).collect();
        let albums: Vec<String> = r.albums.iter().take(3).map(|a| format!("{} - {}", a.name, a.artist.as_deref().unwrap_or(""))).collect();
        let songs: Vec<String> = r.songs.iter().take(5).map(|s| format!("{} - {}", s.title, s.artist.as_deref().unwrap_or(""))).collect();
        println!("  artists: {artists:?}\n  albums:  {albums:?}\n  songs:   {songs:?}");
    }
    // Every prefix of every query, as the UI asks while typing.
    let mut keys = Vec::new();
    for q in &queries {
        let chars: Vec<char> = q.chars().collect();
        for n in 1..=chars.len() {
            let prefix: String = chars[..n].iter().collect();
            let t = std::time::Instant::now();
            lib.search(&prefix, 20, None).unwrap();
            keys.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    keys.sort_by(f64::total_cmp);
    let pct = |p: f64| keys[((keys.len() - 1) as f64 * p) as usize];
    println!("\nkeystrokes: {} searches, median {:.2} ms, p95 {:.2} ms, max {:.2} ms", keys.len(), pct(0.5), pct(0.95), pct(1.0));
}
