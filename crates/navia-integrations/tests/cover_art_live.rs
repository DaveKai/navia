//! A few real requests to the Cover Art Archive and MusicBrainz. Ignored by default. Run with `cargo test --test
//! cover_art_live -- --ignored`.

use std::sync::OnceLock;

use navia_integrations::TrackInfo;
use navia_integrations::art::{ArtSource, CoverArtArchive};

/// One shared client, so MusicBrainz's one-request-per-second limit holds across these tests. Parallel requests got
/// rate-limited.
fn source() -> &'static CoverArtArchive {
    static SOURCE: OnceLock<CoverArtArchive> = OnceLock::new();
    SOURCE.get_or_init(|| CoverArtArchive::new().unwrap())
}

fn assert_public_image(url: &str) {
    assert!(url.starts_with("https://"), "{url}");
    assert!(url.len() <= 256, "{url}");
    assert!(url.contains("archive.org"), "{url}");
    println!("found: {url}");
}

#[test]
#[ignore = "uses the network"]
fn finds_a_cover_from_a_release_group_id() {
    let track = TrackInfo {
        // The Dark Side of the Moon.
        release_group_mbid: Some("f5093c06-23e3-404f-aeaa-40f72885ee3a".into()),
        ..Default::default()
    };
    let url = source().find(&track).unwrap().unwrap();
    assert_public_image(&url);
}

#[test]
#[ignore = "uses the network"]
fn finds_a_cover_by_searching() {
    let track = TrackInfo {
        artist: "Pink Floyd".into(),
        album: "The Dark Side of the Moon".into(),
        ..Default::default()
    };
    let url = source().find(&track).unwrap().unwrap();
    assert_public_image(&url);
}

#[test]
#[ignore = "uses the network"]
fn unknown_albums_find_nothing() {
    let track = TrackInfo {
        artist: "Zzqx Nonexistent Artist".into(),
        album: "Zzqx Nonexistent Album".into(),
        ..Default::default()
    };
    assert_eq!(source().find(&track).unwrap(), None);
}
