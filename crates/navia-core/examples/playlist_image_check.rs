//! Tests the playlist image path against a real server. It only touches a playlist named
//! "Navia cover test" whose id matches `NAVIA_ALLOW_COVER_TEST`.
//!
//! `cargo run --example playlist_image_check -- <playlist-id> <work-dir>`

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{ImageFormat, Rgba, RgbaImage};
use navia_core::client::native::PlaylistImageError;
use navia_core::client::{Client, ClientConfig, Credentials};
use navia_core::library::sync::{SyncOptions, Syncer};
use navia_core::library::{Library, LibraryConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (id, work, mode) = match args.as_slice() {
        [_, id, work] => (id, work, "all"),
        [_, id, work, mode] => (id, work, mode.as_str()),
        _ => {
            eprintln!("usage: playlist_image_check <playlist-id> <work-dir> [all|follow]");
            std::process::exit(2);
        }
    };
    let allowed = std::env::var("NAVIA_ALLOW_COVER_TEST").unwrap_or_default();
    assert!(!allowed.is_empty() && &allowed == id, "NAVIA_ALLOW_COVER_TEST must equal the playlist id");
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is required"));
    let config = ClientConfig::new(env("NAVIDROME_URL"), Credentials::new(env("NAVIDROME_USER"), env("NAVIDROME_PASSWORD")));
    let client = Client::new(config).unwrap();

    let work = PathBuf::from(work);
    std::fs::create_dir_all(&work).unwrap();
    let support = client.playlist_image_support().await.unwrap();
    println!("support: {support:?}");

    let list = client.playlists().await.unwrap();
    let playlist = list.iter().find(|p| &p.id == id).expect("no such playlist");
    assert_eq!(playlist.name, "Navia cover test", "only the test playlist may be changed");
    let before = playlist.cover_art.clone();
    println!("start: cover id {before:?}");

    let db = work.join("check.sqlite");
    let library = Library::open(&LibraryConfig { path: db }).unwrap();
    let syncer = Syncer::new(client.clone(), library.clone(), SyncOptions::default());
    syncer.refresh_playlist(id).await.unwrap();

    if mode == "follow" {
        follow_mode(&syncer, &client, id, &work).await;
        return;
    }

    for (name, color, format) in [("red.png", [220, 30, 30, 255], ImageFormat::Png), ("green.jpg", [30, 200, 60, 255], ImageFormat::Jpeg), ("blue.gif", [30, 60, 220, 255], ImageFormat::Gif), ("gold.webp", [230, 170, 20, 255], ImageFormat::WebP)] {
        let file = work.join(name);
        let img = RgbaImage::from_pixel(640, 640, Rgba(color));
        if format == ImageFormat::Gif {
            // `image` can't encode GIF here, so the caller makes this file with ffmpeg.
            assert!(file.exists(), "make {file:?} first");
        } else if format == ImageFormat::Jpeg {
            image::DynamicImage::ImageRgba8(img).into_rgb8().save_with_format(&file, format).unwrap();
        } else {
            img.save_with_format(&file, format).unwrap();
        }
        let start = Instant::now();
        let cover = syncer.set_playlist_image(id, &file).await;
        let ms = start.elapsed().as_millis();
        match cover {
            Ok(Some(cover_id)) => {
                let (full_type, full) = client.http_get_cover(&cover_id, None).await;
                let (small_type, small) = client.http_get_cover(&cover_id, Some(300)).await;
                println!("upload {name}: ok in {ms} ms, cover id {cover_id:?}, full {} bytes {full_type:?} mean {:?}, size 300 {} bytes {small_type:?} mean {:?}, wanted {color:?}", full.len(), mean_color(&full), small.len(), mean_color(&small));
            }
            other => println!("upload {name}: {other:?} after {ms} ms"),
        }
    }

    // The server must refuse this file.
    let bad = work.join("not-an-image.txt");
    std::fs::write(&bad, "not an image").unwrap();
    report("text file", syncer.set_playlist_image(id, &bad).await);

    // Larger than the server's default 10 MB limit.
    let big = work.join("big.bin");
    std::fs::write(&big, vec![0xAAu8; 11 * 1000 * 1000]).unwrap();
    report("11 MB file", syncer.set_playlist_image(id, &big).await);

    report("missing file", syncer.set_playlist_image(id, Path::new("/nonexistent.png")).await);

    let start = Instant::now();
    let removed = syncer.remove_playlist_image(id).await;
    println!("remove: {removed:?} in {} ms", start.elapsed().as_millis());
    let list = client.playlists().await.unwrap();
    let after = list.iter().find(|p| &p.id == id).map(|p| p.cover_art.clone());
    println!("end: cover id {after:?}");
}

/// One upload and the checks the rate limit cut off. The sleeps outlast Navidrome's login
/// window (`AuthWindowLength`, 20 s by default).
async fn follow_mode(syncer: &Syncer, client: &Client, id: &str, work: &Path) {
    let file = work.join("red.png");
    let start = Instant::now();
    let cover = syncer.set_playlist_image(id, &file).await;
    println!("upload red.png: {cover:?} in {} ms", start.elapsed().as_millis());
    if let Ok(Some(cover_id)) = &cover {
        let (kind, small) = client.http_get_cover(cover_id, Some(300)).await;
        println!("served {kind:?} mean {:?}", mean_color(&small));
    }
    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
    let big = work.join("big.bin");
    if !big.exists() {
        std::fs::write(&big, vec![0xAAu8; 11 * 1000 * 1000]).unwrap();
    }
    let result = syncer.set_playlist_image(id, &big).await;
    report("11 MB file", result);
    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
    let removed = syncer.remove_playlist_image(id).await;
    println!("remove: {removed:?}");
}

fn report(label: &str, result: Result<Option<String>, PlaylistImageError>) {
    match result {
        Ok(cover) => println!("{label}: unexpectedly accepted, cover {cover:?}"),
        Err(error) => println!("{label}: refused with {error:?}: \"{error}\""),
    }
}

fn mean_color(bytes: &[u8]) -> Option<[u8; 3]> {
    let img = image::load_from_memory(bytes).ok()?.into_rgb8();
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in img.pixels() {
        r += p[0] as u64;
        g += p[1] as u64;
        b += p[2] as u64;
        n += 1;
    }
    Some([(r / n) as u8, (g / n) as u8, (b / n) as u8])
}

trait CoverFetch {
    async fn http_get_cover(&self, cover: &str, size: Option<u32>) -> (Option<String>, Vec<u8>);
}

impl CoverFetch for Client {
    async fn http_get_cover(&self, cover: &str, size: Option<u32>) -> (Option<String>, Vec<u8>) {
        let url = self.cover_art_url(cover, size).unwrap();
        let response = reqwest::get(url).await.unwrap();
        let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        (kind, response.bytes().await.unwrap().to_vec())
    }
}
