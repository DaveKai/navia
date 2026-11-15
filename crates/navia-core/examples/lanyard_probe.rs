//! Prints what a Lanyard reports for the given friends, every second.
//! cargo run -p navia-core --example lanyard_probe -- <address or ""> <id> [<id> ...]
fn main() {
    let mut args = std::env::args().skip(1);
    let address = navia_core::lanyard::parse_address(&args.next().unwrap_or_default()).expect("an address");
    let friends = args.map(|id| navia_core::lanyard::Friend { id, name: None }).collect();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let watch = navia_core::lanyard::watch::FriendsWatch::start(rt.handle());
    watch.configure(&address, friends);
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        for f in watch.friends() {
            println!("{:?} {:?} listening={} track={:?}", watch.transport(), f.reach, f.listening(), f.track.as_ref().map(|t| (&t.song, &t.artist)));
        }
    }
}
