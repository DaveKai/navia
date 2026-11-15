//! Times the Artists filters on a copy of a library and prints what each offers.
//! `cargo run --release -p navia-core --example artist_filters_check -- <library.sqlite copy>`

use navia_core::library::{Library, LibraryConfig};
use navia_core::song_filters::{FilterId, FilterState, FilterValue};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("a library copy");
    let lib = Library::open(&LibraryConfig { path: path.into() }).expect("open the library");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let t = Instant::now();
    let index = lib.artist_filters().expect("index");
    println!("index: {} artists in {:?}", index.len(), t.elapsed());
    for id in index.available() {
        let t = Instant::now();
        let options = index.options(id, now);
        println!("{id:?}: {} choices in {:?}, first {:?}", options.len(), t.elapsed(), options.iter().take(4).map(|o| (o.value.as_str(), o.count)).collect::<Vec<_>>());
    }
    let all = lib.filtered_artists(None, &FilterState::default(), now).unwrap().len();
    let mut state = FilterState::default();
    state.toggle_choice(FilterId::Plays, "played");
    state.toggle_choice(FilterId::Albums, "many");
    let t = Instant::now();
    let some = lib.filtered_artists(None, &state, now).unwrap().len();
    println!("artists {all}; played with 6+ albums {some} in {:?}", t.elapsed());
    state.set(FilterId::Downloaded, Some(FilterValue::On));
    let t = Instant::now();
    println!("and downloaded {} in {:?}", lib.filtered_artists(None, &state, now).unwrap().len(), t.elapsed());
}
