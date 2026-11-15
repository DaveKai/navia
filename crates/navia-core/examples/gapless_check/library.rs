//! Read-only lookups in a Navidrome database. `NAVIA_ND_DB` is the database file and
//! `NAVIA_ND_ROOTS` maps library ids to folders, such as `1=/music/main,2=/music/other`.

use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};

use crate::Result;

fn db_path() -> String {
    format!("file:{}?mode=ro", std::env::var("NAVIA_ND_DB").expect("set NAVIA_ND_DB"))
}

fn library_root(id: i64) -> String {
    let roots = std::env::var("NAVIA_ND_ROOTS").expect("set NAVIA_ND_ROOTS");
    roots
        .split(',')
        .find_map(|e| e.split_once('=').filter(|(k, _)| k.parse::<i64>() == Ok(id)).map(|(_, v)| v.to_string()))
        .unwrap_or_else(|| panic!("no root for library {id} in NAVIA_ND_ROOTS"))
}

fn open() -> Result<Connection> {
    Connection::open_with_flags(
        db_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| e.to_string())
}

/// Tracks of one album, in play order.
pub fn album_tracks(album: &str, album_artist: &str) -> Result<Vec<PathBuf>> {
    let c = open()?;
    let mut stmt = c
        .prepare(
            "SELECT library_id, path FROM media_file
             WHERE album = ?1 AND album_artist = ?2 AND missing = 0
             ORDER BY disc_number, track_number",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([album, album_artist], |r| {
            Ok(PathBuf::from(library_root(r.get(0)?)).join(r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())
}

/// All MP3 and AAC tracks, whatever their file extension.
pub fn lossy_tracks() -> Result<Vec<(String, PathBuf)>> {
    let c = open()?;
    let mut stmt = c
        .prepare(
            "SELECT library_id, path, codec FROM media_file
             WHERE missing=0 AND (suffix='mp3' OR codec='MP3' OR suffix='aac' OR codec='AAC')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(2)?, PathBuf::from(library_root(r.get(0)?)).join(r.get::<_, String>(1)?)))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<std::result::Result<_, _>>().map_err(|e| e.to_string())
}

/// One track path per named bucket, for the format survey.
pub fn sample_tracks() -> Result<Vec<(String, PathBuf)>> {
    let c = open()?;
    let buckets = [
        ("FLAC 16/44.1", "suffix='flac' AND codec='' AND sample_rate=44100 AND bit_depth=16"),
        ("FLAC 24/48", "suffix='flac' AND codec='' AND sample_rate=48000 AND bit_depth=24"),
        ("FLAC 24/96", "suffix='flac' AND codec='' AND sample_rate=96000 AND bit_depth=24"),
        ("FLAC 24/192", "suffix='flac' AND codec='' AND sample_rate=192000"),
        ("WAV", "suffix='wav'"),
        ("MP3 44.1", "suffix='mp3' AND sample_rate=44100"),
        ("MP3 48", "suffix='mp3' AND sample_rate=48000"),
        (".flac holding MP3", "suffix='flac' AND codec='MP3'"),
        ("AAC m4a 44.1", "suffix='m4a' AND codec='AAC' AND sample_rate=44100"),
        ("AAC m4a 48", "suffix='m4a' AND codec='AAC' AND sample_rate=48000"),
        ("AAC raw .aac", "suffix='aac'"),
        ("ALAC", "suffix='m4a' AND codec='ALAC'"),
        ("Opus", "suffix='opus'"),
    ];
    let mut out = Vec::new();
    for (label, cond) in buckets {
        let q = format!(
            "SELECT library_id, path FROM media_file WHERE missing=0 AND {cond} ORDER BY id LIMIT 3"
        );
        let mut stmt = c.prepare(&q).map_err(|e| e.to_string())?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        for (lib, p) in rows {
            out.push((label.to_string(), PathBuf::from(library_root(lib)).join(p)));
        }
    }
    Ok(out)
}
