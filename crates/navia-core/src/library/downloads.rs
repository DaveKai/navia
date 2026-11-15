//! The record of which songs are kept on disk for offline play.

use rusqlite::{OptionalExtension, params};

use super::Library;
use crate::error::Result;

impl Library {
    /// Mark a finished download. Its path is relative to the downloads folder.
    pub fn download_record(&self, song_id: &str, relative_path: &str, size: u64) -> Result<()> {
        self.download_record_as(song_id, relative_path, size, None)
    }

    /// Mark a finished download along with the format and rate it was made as.
    pub fn download_record_as(&self, song_id: &str, relative_path: &str, size: u64, quality: Option<(&str, u32)>) -> Result<()> {
        self.download_record_held(song_id, relative_path, size, quality, true)
    }

    /// Mark a finished download. `single` means it was asked for on its own, and once set it stays.
    pub(crate) fn download_record_held(&self, song_id: &str, relative_path: &str, size: u64, quality: Option<(&str, u32)>, single: bool) -> Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        self.write(|tx| {
            tx.execute(
                "INSERT INTO download(song_id, path, size, added_at, quality_format, quality_kbps, single) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(song_id) DO UPDATE SET path=excluded.path, size=excluded.size, added_at=excluded.added_at, quality_format=excluded.quality_format, quality_kbps=excluded.quality_kbps, single=max(single, excluded.single)",
                params![song_id, relative_path, size as i64, now, quality.map(|q| q.0), quality.map(|q| q.1), single],
            )?;
            Ok(())
        })
    }

    pub fn download_forget(&self, song_id: &str) -> Result<()> {
        self.write(|tx| {
            tx.execute("DELETE FROM download WHERE song_id = ?1", [song_id])?;
            Ok(())
        })
    }

    /// The relative path of a downloaded song.
    pub fn download_path(&self, song_id: &str) -> Result<Option<String>> {
        self.read(|c| c.query_row("SELECT path FROM download WHERE song_id = ?1", [song_id], |r| r.get(0)).optional().map_err(Into::into))
    }

    /// The format and rate a download was made as, or `None` for the original file.
    pub fn download_quality(&self, song_id: &str) -> Result<Option<(String, u32)>> {
        self.read(|c| c.query_row("SELECT quality_format, quality_kbps FROM download WHERE song_id = ?1", [song_id], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<u32>>(1)?))).optional().map(|row| row.and_then(|(f, k)| Some((f?, k.unwrap_or(0))))).map_err(Into::into))
    }

    /// Every downloaded song as (id, relative path, size).
    pub fn downloads(&self) -> Result<Vec<(String, String, u64)>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT song_id, path, size FROM download")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Row ids of the downloaded songs still in the library.
    pub fn downloaded_rowids(&self) -> Result<Vec<u32>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT s.rowid FROM download d JOIN song s ON s.id = d.song_id")?;
            let rows = st.query_map([], |r| r.get::<_, i64>(0).map(|v| v as u32))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}
