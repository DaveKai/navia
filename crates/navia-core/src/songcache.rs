//! Keeps streamed songs on disk at the quality they played at, so replays start at once and work offline.
//! Only complete, verified files are indexed, so nothing partial can play as complete.
//! The cache is per account, evicts least recently played first and never touches downloads.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::audio::Decoder;
use crate::error::Result;
use crate::library::Library;
use crate::quality::{Codec, Quality};

pub const GB: u64 = 1_000_000_000;
/// The limit a new account starts with.
pub const DEFAULT_LIMIT: u64 = 2 * GB;
/// The limits Settings offers. 0 is Off.
pub const LIMIT_CHOICES: [u64; 5] = [0, GB, 2 * GB, 5 * GB, 10 * GB];
/// Stored in the account's library, so the limit is per server and user.
const LIMIT_KEY: &str = "cache.songs.limit";
/// Free space always left on the disk, 5% of it, at least 1 GB and at most 10 GB.
const RESERVE_MIN: u64 = GB;
const RESERVE_MAX: u64 = 10 * GB;
const COPY_CHUNK: usize = 1024 * 1024;
/// How far a transcode's decoded length may fall short of the song's before it counts as cut off.
/// The library's duration is whole seconds.
const SHORT_BY_MS: u64 = 2_000;

/// Bytes on disk that can be read at any offset, like a stream's spool file.
pub trait Spool {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;
}

/// What a finished stream is, for keeping it.
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub song_id: String,
    pub quality: Quality,
    /// The original file's size as the library knows it, 0 when unknown.
    /// A changed file almost always has a changed size, so a stale copy is never played.
    pub source_size: u64,
    /// The song's length, which a transcode must reach to count as complete.
    pub expected_ms: Option<u64>,
}

/// A cached song ready to play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheHit {
    pub path: PathBuf,
    pub quality: Quality,
    /// The original file's `Last-Modified` when it was cached, if the server sent one.
    pub stamp: Option<String>,
    pub bytes: u64,
    pub source_size: u64,
    song_id: String,
}

/// A song in the cache, for listing what's kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedSong {
    pub song_id: String,
    /// All its copies together.
    pub bytes: u64,
    /// When it was last played or cached, in ms since the epoch.
    pub last_played_ms: i64,
    /// How many qualities it's kept at, usually 1.
    pub copies: u32,
}

/// What happened to a finished stream offered to the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored,
    /// The cache is off.
    Disabled,
    /// Bigger than the whole limit.
    TooLarge,
    /// The disk is nearly full, so caching is paused.
    LowDisk,
    /// The bytes didn't check out because they were cut short, unreadable or the wrong length.
    Invalid(String),
    Failed(String),
}

type SpaceProbe = Box<dyn Fn(&Path) -> Option<(u64, u64)> + Send + Sync>;

/// The cache for one account.
pub struct SongCache {
    dir: PathBuf,
    db: Mutex<Connection>,
    library: Option<Library>,
    limit: AtomicU64,
    /// Set when a song wasn't kept because the disk is nearly full, and cleared once one is.
    low_disk: AtomicBool,
    /// One store at a time, so eviction and the limit see every write.
    writing: Mutex<()>,
    /// Last-played stamps only move forward, so two plays in the same millisecond still order.
    clock: AtomicI64,
    /// Free and total bytes on the disk holding `dir`. Replaceable for tests.
    space: Mutex<SpaceProbe>,
}

impl std::fmt::Debug for SongCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SongCache").field("dir", &self.dir).field("limit", &self.limit()).finish()
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn mtime_ns(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64)
}

fn variant(quality: Quality) -> (&'static str, u32) {
    (quality.format_id(), quality.kbps())
}

fn quality_of(format: &str, kbps: u32) -> Option<Quality> {
    if format == "original" {
        return Some(Quality::Original);
    }
    Some(Quality::Transcode { codec: Codec::from_id(format)?, kbps })
}

impl SongCache {
    /// Opens or creates the cache in `dir`, with the limit from the account's library when there is one.
    /// Crash leftovers are tidied on a background thread, so this returns at once.
    pub fn open(dir: PathBuf, library: Option<Library>) -> Result<Arc<Self>> {
        fs::create_dir_all(&dir)?;
        let conn = Connection::open(dir.join("index.sqlite"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS entry(
                song_id TEXT NOT NULL,
                format TEXT NOT NULL,
                kbps INTEGER NOT NULL,
                source_size INTEGER NOT NULL,
                file TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                md5 TEXT NOT NULL,
                mtime_ns INTEGER NOT NULL,
                stamp TEXT,
                last_played INTEGER NOT NULL,
                PRIMARY KEY(song_id, format, kbps));
             CREATE INDEX IF NOT EXISTS entry_lru ON entry(last_played);",
        )?;
        let last: i64 = conn.query_row("SELECT COALESCE(MAX(last_played), 0) FROM entry", [], |r| r.get(0))?;
        let limit = library.as_ref().and_then(|l| l.meta(LIMIT_KEY).ok().flatten()).and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_LIMIT);
        let cache = Arc::new(Self {
            dir,
            db: Mutex::new(conn),
            library,
            limit: AtomicU64::new(limit),
            low_disk: AtomicBool::new(false),
            writing: Mutex::new(()),
            clock: AtomicI64::new(last),
            space: Mutex::new(Box::new(disk_space)),
        });
        let tidy = cache.clone();
        std::thread::Builder::new().name("navia-songcache".into()).spawn(move || tidy.reconcile()).map_err(crate::Error::Io)?;
        Ok(cache)
    }

    /// Replaces how free space is measured, for tests of a nearly full disk.
    #[doc(hidden)]
    pub fn set_space_probe(&self, probe: impl Fn(&Path) -> Option<(u64, u64)> + Send + Sync + 'static) {
        *self.space.lock().expect("space probe") = Box::new(probe);
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The size limit in bytes. 0 means the cache is off.
    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Acquire)
    }

    pub fn enabled(&self) -> bool {
        self.limit() > 0
    }

    /// Changes the limit, saved for this account. A lower limit evicts at once, and 0 empties the cache.
    pub fn set_limit(&self, bytes: u64) -> Result<()> {
        self.limit.store(bytes, Ordering::Release);
        if let Some(library) = &self.library {
            library.set_meta(LIMIT_KEY, Some(&bytes.to_string()))?;
        }
        let _writing = self.writing.lock().expect("song cache writer");
        self.evict_to(bytes, 0);
        Ok(())
    }

    /// True when caching stopped because the disk is nearly full.
    pub fn low_disk(&self) -> bool {
        self.low_disk.load(Ordering::Acquire)
    }

    /// Bytes the cached songs take, and how many there are.
    pub fn usage(&self) -> (u64, u64) {
        let db = self.db.lock().expect("song cache index");
        db.query_row("SELECT COALESCE(SUM(bytes), 0), COUNT(*) FROM entry", [], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))).unwrap_or((0, 0))
    }

    /// Whether any copy of a song is cached. Index only, so it's cheap enough for the UI thread.
    pub fn contains(&self, song_id: &str) -> bool {
        let db = self.db.lock().expect("song cache index");
        db.query_row("SELECT 1 FROM entry WHERE song_id=?1 LIMIT 1", [song_id], |_| Ok(())).optional().ok().flatten().is_some()
    }

    /// Ids of every cached song, for showing what plays offline.
    pub fn song_ids(&self) -> Vec<String> {
        let db = self.db.lock().expect("song cache index");
        let Ok(mut st) = db.prepare("SELECT DISTINCT song_id FROM entry") else { return Vec::new() };
        st.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Every cached song, most recently played first, with all its copies together.
    /// Reads only the index, and every song listed plays in full.
    pub fn songs(&self) -> Vec<CachedSong> {
        let db = self.db.lock().expect("song cache index");
        let Ok(mut st) = db.prepare("SELECT song_id, SUM(bytes), MAX(last_played), COUNT(*) FROM entry GROUP BY song_id ORDER BY MAX(last_played) DESC") else { return Vec::new() };
        st.query_map([], |r| Ok(CachedSong { song_id: r.get(0)?, bytes: r.get::<_, i64>(1)? as u64, last_played_ms: r.get(2)?, copies: r.get::<_, i64>(3)? as u32 }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Forgets every copy of these songs and deletes their files. Returns the bytes freed.
    pub fn remove_songs(&self, song_ids: &[String]) -> u64 {
        let _writing = self.writing.lock().expect("song cache writer");
        let mut freed = 0;
        for id in song_ids {
            let bytes: u64 = {
                let db = self.db.lock().expect("song cache index");
                db.query_row("SELECT COALESCE(SUM(bytes), 0) FROM entry WHERE song_id=?1", [id], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64
            };
            self.forget_song(id);
            freed += bytes;
        }
        freed
    }

    /// The cached copy of a song at exactly this quality, checked on disk.
    /// A copy of a file whose size has since changed is stale and goes. `source_size` 0 skips that check.
    pub fn lookup(&self, song_id: &str, quality: Quality, source_size: u64) -> Option<CacheHit> {
        let (format, kbps) = variant(quality);
        let row = {
            let db = self.db.lock().expect("song cache index");
            db.query_row(
                "SELECT file, bytes, md5, mtime_ns, stamp, source_size FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3",
                params![song_id, format, kbps],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)? as u64)),
            )
            .optional()
            .ok()
            .flatten()?
        };
        let (file, bytes, md5, mtime, stamp, cached_size) = row;
        if source_size > 0 && cached_size > 0 && source_size != cached_size {
            tracing::info!("the server's file changed since it was cached; dropping the old copy");
            self.remove(song_id, quality);
            return None;
        }
        let path = self.dir.join(&file);
        if !self.intact(&path, bytes, &md5, mtime, song_id, quality) {
            self.remove(song_id, quality);
            return None;
        }
        Some(CacheHit { path, quality, stamp, bytes, source_size: cached_size, song_id: song_id.to_string() })
    }

    /// Any cached copy of a song, the original first, then the highest bitrate. For playing offline.
    pub fn lookup_any(&self, song_id: &str) -> Option<CacheHit> {
        let variants: Vec<(String, u32)> = {
            let db = self.db.lock().expect("song cache index");
            let mut st = db.prepare("SELECT format, kbps FROM entry WHERE song_id=?1 ORDER BY (format='original') DESC, kbps DESC").ok()?;
            st.query_map([song_id], |r| Ok((r.get(0)?, r.get(1)?))).ok()?.flatten().collect()
        };
        variants.into_iter().filter_map(|(f, k)| quality_of(&f, k)).find_map(|q| self.lookup(song_id, q, 0))
    }

    /// Notes that a cached song was just played, for eviction.
    pub fn mark_played(&self, hit: &CacheHit) {
        let stamp = self.tick();
        let (format, kbps) = variant(hit.quality);
        let db = self.db.lock().expect("song cache index");
        let _ = db.execute("UPDATE entry SET last_played=?1 WHERE song_id=?2 AND format=?3 AND kbps=?4", params![stamp, hit.song_id, format, kbps]);
    }

    /// Forgets one cached copy and deletes its file.
    pub fn remove(&self, song_id: &str, quality: Quality) {
        let (format, kbps) = variant(quality);
        let file: Option<String> = {
            let db = self.db.lock().expect("song cache index");
            let file = db.query_row("SELECT file FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3", params![song_id, format, kbps], |r| r.get(0)).optional().ok().flatten();
            let _ = db.execute("DELETE FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3", params![song_id, format, kbps]);
            file
        };
        if let Some(file) = file {
            let _ = fs::remove_file(self.dir.join(file));
        }
    }

    /// Forgets every cached copy of a song, for example once it's downloaded.
    pub fn forget_song(&self, song_id: &str) {
        let variants: Vec<(String, u32)> = {
            let db = self.db.lock().expect("song cache index");
            let Ok(mut st) = db.prepare("SELECT format, kbps FROM entry WHERE song_id=?1") else { return };
            st.query_map([song_id], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        for (format, kbps) in variants {
            if let Some(q) = quality_of(&format, kbps) {
                self.remove(song_id, q);
            }
        }
    }

    /// Moves a cached copy to `dest` and forgets it, so a downloaded song isn't stored twice.
    /// Returns the bytes moved, or `None` when there's no intact copy.
    pub fn take(&self, song_id: &str, quality: Quality, source_size: u64, dest: &Path) -> Option<u64> {
        let _writing = self.writing.lock().expect("song cache writer");
        let hit = self.lookup(song_id, quality, source_size)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        let moved = fs::rename(&hit.path, dest).is_ok() || (fs::copy(&hit.path, dest).ok() == Some(hit.bytes));
        if !moved {
            let _ = fs::remove_file(dest);
            return None;
        }
        let (format, kbps) = variant(quality);
        let db = self.db.lock().expect("song cache index");
        let _ = db.execute("DELETE FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3", params![song_id, format, kbps]);
        drop(db);
        let _ = fs::remove_file(&hit.path);
        Some(hit.bytes)
    }

    /// Deletes every cached song. A file still playing stays readable where the system allows it,
    /// and is tidied at the next launch elsewhere.
    pub fn clear(&self) {
        let _writing = self.writing.lock().expect("song cache writer");
        let files: Vec<String> = {
            let db = self.db.lock().expect("song cache index");
            let files = db.prepare("SELECT file FROM entry").and_then(|mut st| st.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect())).unwrap_or_default();
            let _ = db.execute("DELETE FROM entry", []);
            files
        };
        for file in files {
            let _ = fs::remove_file(self.dir.join(file));
        }
        self.low_disk.store(false, Ordering::Release);
    }

    /// Keeps a finished stream. The caller guarantees `spool` holds every byte from 0 to `len`.
    /// Runs on the stream's own thread, never the audio thread.
    pub fn store(&self, entry: &NewEntry, stamp: Option<String>, spool: &dyn Spool, len: u64) -> StoreOutcome {
        let limit = self.limit();
        if limit == 0 {
            return StoreOutcome::Disabled;
        }
        if len == 0 {
            return StoreOutcome::Invalid("empty".to_string());
        }
        if len > limit {
            return StoreOutcome::TooLarge;
        }
        let _writing = self.writing.lock().expect("song cache writer");
        if let Some((free, total)) = (self.space.lock().expect("space probe"))(&self.dir) {
            let reserve = (total / 20).clamp(RESERVE_MIN, RESERVE_MAX);
            if free < reserve.saturating_add(len) {
                if !self.low_disk.swap(true, Ordering::AcqRel) {
                    tracing::info!("the disk is nearly full, so songs aren't being cached");
                }
                return StoreOutcome::LowDisk;
            }
        }
        // Make room first, so the limit holds the whole time the file is being written.
        self.remove(&entry.song_id, entry.quality);
        self.evict_to(limit, len);
        let (format, kbps) = variant(entry.quality);
        let name = format!("{:x}", md5::compute(format!("{}\0{format}\0{kbps}", entry.song_id)));
        let ext = match entry.quality {
            Quality::Original => "orig",
            Quality::Transcode { codec, .. } => codec.extension(),
        };
        let relative = format!("{}/{name}.{ext}", &name[..2]);
        let path = self.dir.join(&relative);
        let part = path.with_extension(format!("{ext}.part"));
        let digest = match copy_out(spool, len, &part) {
            Ok(digest) => digest,
            Err(e) => {
                let _ = fs::remove_file(&part);
                if e.raw_os_error() == Some(28) || e.kind() == io::ErrorKind::StorageFull {
                    self.low_disk.store(true, Ordering::Release);
                    return StoreOutcome::LowDisk;
                }
                return StoreOutcome::Failed(e.to_string());
            }
        };
        if let Err(reason) = check_audio(&part, entry) {
            let _ = fs::remove_file(&part);
            tracing::info!("not caching a song that didn't check out: {reason}");
            return StoreOutcome::Invalid(reason);
        }
        if let Err(e) = fs::rename(&part, &path) {
            let _ = fs::remove_file(&part);
            return StoreOutcome::Failed(e.to_string());
        }
        let mtime = fs::metadata(&path).map(|m| mtime_ns(&m)).unwrap_or(0);
        let played = self.tick();
        let db = self.db.lock().expect("song cache index");
        let written = db.execute(
            "INSERT OR REPLACE INTO entry(song_id, format, kbps, source_size, file, bytes, md5, mtime_ns, stamp, last_played) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![entry.song_id, format, kbps, entry.source_size as i64, relative, len as i64, digest, mtime, stamp, played],
        );
        drop(db);
        if let Err(e) = written {
            let _ = fs::remove_file(&path);
            return StoreOutcome::Failed(e.to_string());
        }
        self.low_disk.store(false, Ordering::Release);
        StoreOutcome::Stored
    }

    fn tick(&self) -> i64 {
        let now = now_ms();
        let mut last = self.clock.load(Ordering::Acquire);
        loop {
            let next = now.max(last + 1);
            match self.clock.compare_exchange(last, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return next,
                Err(actual) => last = actual,
            }
        }
    }

    /// Evicts least recently played songs until `incoming` more bytes fit under `limit`.
    fn evict_to(&self, limit: u64, incoming: u64) {
        let (mut used, _) = self.usage();
        if used.saturating_add(incoming) <= limit {
            return;
        }
        let oldest: Vec<(String, String, u32, String, u64)> = {
            let db = self.db.lock().expect("song cache index");
            let Ok(mut st) = db.prepare("SELECT song_id, format, kbps, file, bytes FROM entry ORDER BY last_played ASC") else { return };
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? as u64))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        for (song_id, format, kbps, file, bytes) in oldest {
            if used.saturating_add(incoming) <= limit {
                break;
            }
            match fs::remove_file(self.dir.join(&file)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                // Windows blocks deleting a file in use, so leave it for now.
                Err(_) => continue,
            }
            let db = self.db.lock().expect("song cache index");
            let _ = db.execute("DELETE FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3", params![song_id, format, kbps]);
            used = used.saturating_sub(bytes);
        }
    }

    /// Whether the file is the one stored, with the right length and, if touched since, the same content.
    fn intact(&self, path: &Path, bytes: u64, md5: &str, mtime: i64, song_id: &str, quality: Quality) -> bool {
        let Ok(meta) = fs::metadata(path) else { return false };
        if meta.len() != bytes {
            return false;
        }
        if mtime_ns(&meta) == mtime {
            return true;
        }
        let Ok(digest) = file_md5(path) else { return false };
        if digest != md5 {
            tracing::warn!("a cached song was damaged on disk; dropping it");
            return false;
        }
        let (format, kbps) = variant(quality);
        let db = self.db.lock().expect("song cache index");
        let _ = db.execute("UPDATE entry SET mtime_ns=?1 WHERE song_id=?2 AND format=?3 AND kbps=?4", params![mtime_ns(&meta), song_id, format, kbps]);
        true
    }

    /// Repairs the cache after a crash or outside change, dropping partial and orphaned files and records.
    /// Then holds the limit.
    pub fn reconcile(&self) {
        let _writing = self.writing.lock().expect("song cache writer");
        let rows: Vec<(String, String, u32, String, u64)> = {
            let db = self.db.lock().expect("song cache index");
            let Ok(mut st) = db.prepare("SELECT song_id, format, kbps, file, bytes FROM entry") else { return };
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? as u64))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        let mut known = std::collections::HashSet::new();
        for (song_id, format, kbps, file, bytes) in rows {
            let ok = fs::metadata(self.dir.join(&file)).is_ok_and(|m| m.len() == bytes);
            if ok {
                known.insert(file);
            } else {
                let db = self.db.lock().expect("song cache index");
                let _ = db.execute("DELETE FROM entry WHERE song_id=?1 AND format=?2 AND kbps=?3", params![song_id, format, kbps]);
            }
        }
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !path.is_dir() || name.len() != 2 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            for file in fs::read_dir(&path).into_iter().flatten().flatten() {
                let relative = format!("{name}/{}", file.file_name().to_string_lossy());
                if !known.contains(&relative) {
                    let _ = fs::remove_file(file.path());
                }
            }
        }
        self.evict_to(self.limit(), 0);
    }
}

/// Copies `len` bytes out of the spool into `part`, synced, and returns their md5.
fn copy_out(spool: &dyn Spool, len: u64, part: &Path) -> io::Result<String> {
    if let Some(parent) = part.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = fs::File::create(part)?;
    let mut digest = md5::Context::new();
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut offset = 0u64;
    while offset < len {
        let want = ((len - offset) as usize).min(buf.len());
        let n = spool.read_at(offset, &mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the stream's file ended early"));
        }
        out.write_all(&buf[..n])?;
        digest.consume(&buf[..n]);
        offset += n as u64;
    }
    out.sync_all()?;
    Ok(format!("{:x}", digest.finalize()))
}

fn file_md5(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = md5::Context::new();
    io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

/// Checks the file opens as audio. A transcode has no length to check, so it's decoded in full
/// and must last about as long as the song.
fn check_audio(path: &Path, entry: &NewEntry) -> std::result::Result<(), String> {
    let mut decoder = match entry.quality {
        Quality::Original => return Decoder::open(path).map(|_| ()).map_err(|e| format!("can't be opened: {e}")),
        Quality::Transcode { .. } => Decoder::open_spooled(path).map_err(|e| format!("can't be opened: {e}"))?,
    };
    let Some(expected) = entry.expected_ms else { return Ok(()) };
    let rate = u64::from(decoder.sample_rate.max(1));
    let mut frames = 0u64;
    let mut block = Vec::new();
    loop {
        match decoder.next_block(&mut block) {
            // Blocks are always interleaved stereo.
            Ok(true) => frames += block.len() as u64 / 2,
            Ok(false) => break,
            Err(e) => return Err(format!("doesn't decode to the end: {e}")),
        }
    }
    let got = frames * 1000 / rate;
    if got + SHORT_BY_MS < expected {
        return Err(format!("{got} ms of a {expected} ms song"));
    }
    Ok(())
}

/// Free and total bytes on the disk holding `path`.
#[cfg(unix)]
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statvfs` only writes into the struct it's given, and the path is NUL-terminated.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let unit = stat.f_frsize as u64;
    Some((stat.f_bavail as u64 * unit, stat.f_blocks as u64 * unit))
}

/// Not measured here. A full disk shows up as a failed write, which pauses caching the same way.
#[cfg(not(unix))]
fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes(Vec<u8>);

    impl Spool for Bytes {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
            let start = (offset as usize).min(self.0.len());
            let n = buf.len().min(self.0.len() - start);
            buf[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    /// A tiny valid WAV, which the decoder opens.
    fn wav(frames: usize) -> Vec<u8> {
        let data = frames * 4;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&44_100u32.to_le_bytes());
        v.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data as u32).to_le_bytes());
        v.extend((0..data).map(|i| (i % 251) as u8));
        v
    }

    fn entry(id: &str) -> NewEntry {
        NewEntry { song_id: id.to_string(), quality: Quality::Original, source_size: 0, expected_ms: None }
    }

    fn cache(dir: &Path, limit: u64) -> Arc<SongCache> {
        let cache = SongCache::open(dir.to_path_buf(), None).unwrap();
        cache.set_space_probe(|_| None);
        cache.set_limit(limit).unwrap();
        cache
    }

    #[test]
    fn stores_and_finds_a_song() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let bytes = wav(1000);
        assert_eq!(cache.store(&entry("a"), Some("stamp".into()), &Bytes(bytes.clone()), bytes.len() as u64), StoreOutcome::Stored);
        let hit = cache.lookup("a", Quality::Original, 0).unwrap();
        assert_eq!(fs::read(&hit.path).unwrap(), bytes);
        assert_eq!(hit.stamp.as_deref(), Some("stamp"));
        assert!(cache.lookup("a", Quality::Transcode { codec: Codec::Mp3, kbps: 192 }, 0).is_none());
        assert_eq!(cache.usage(), (bytes.len() as u64, 1));
    }

    #[test]
    fn evicts_least_recently_played_first() {
        let dir = tempfile::tempdir().unwrap();
        let one = wav(1000);
        let size = one.len() as u64;
        let cache = cache(dir.path(), size * 2);
        for id in ["a", "b"] {
            assert_eq!(cache.store(&entry(id), None, &Bytes(one.clone()), size), StoreOutcome::Stored);
        }
        // Playing "a" again makes "b" the oldest.
        let a = cache.lookup("a", Quality::Original, 0).unwrap();
        cache.mark_played(&a);
        assert_eq!(cache.store(&entry("c"), None, &Bytes(one.clone()), size), StoreOutcome::Stored);
        assert!(cache.lookup("b", Quality::Original, 0).is_none());
        assert!(cache.lookup("a", Quality::Original, 0).is_some());
        assert!(cache.lookup("c", Quality::Original, 0).is_some());
        assert!(cache.usage().0 <= size * 2);
    }

    #[test]
    fn a_changed_file_is_never_served() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let bytes = wav(1000);
        let e = NewEntry { source_size: 5000, ..entry("a") };
        assert_eq!(cache.store(&e, None, &Bytes(bytes.clone()), bytes.len() as u64), StoreOutcome::Stored);
        assert!(cache.lookup("a", Quality::Original, 6000).is_none());
        assert_eq!(cache.usage(), (0, 0));
    }

    #[test]
    fn damaged_files_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let bytes = wav(1000);
        let len = bytes.len() as u64;
        cache.store(&entry("cut"), None, &Bytes(bytes.clone()), len);
        cache.store(&entry("flipped"), None, &Bytes(bytes.clone()), len);
        let cut = cache.lookup("cut", Quality::Original, 0).unwrap().path;
        fs::OpenOptions::new().write(true).open(&cut).unwrap().set_len(len / 2).unwrap();
        let flipped = cache.lookup("flipped", Quality::Original, 0).unwrap().path;
        let mut data = fs::read(&flipped).unwrap();
        data[100] ^= 0xff;
        fs::write(&flipped, data).unwrap();
        // Same length, new content. The write moved its time, so the content is checked.
        let f = fs::File::options().write(true).open(&flipped).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        assert!(cache.lookup("cut", Quality::Original, 0).is_none());
        assert!(cache.lookup("flipped", Quality::Original, 0).is_none());
        assert!(!cut.exists() && !flipped.exists());
    }

    #[test]
    fn unreadable_or_short_streams_are_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let junk = vec![7u8; 5000];
        assert!(matches!(cache.store(&entry("junk"), None, &Bytes(junk), 5000), StoreOutcome::Invalid(_)));
        // A transcode that stops at a hundredth of the song.
        let bytes = wav(4410);
        let short = NewEntry { quality: Quality::Transcode { codec: Codec::Mp3, kbps: 192 }, expected_ms: Some(10_000), ..entry("short") };
        assert!(matches!(cache.store(&short, None, &Bytes(bytes.clone()), bytes.len() as u64), StoreOutcome::Invalid(_)));
        let whole = NewEntry { expected_ms: Some(100), ..short };
        assert_eq!(cache.store(&whole, None, &Bytes(bytes.clone()), bytes.len() as u64), StoreOutcome::Stored);
        assert_eq!(cache.usage().1, 1);
        // Nothing half-written is left behind.
        let mut files = Vec::new();
        for sub in fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.path().is_dir()) {
            files.extend(fs::read_dir(sub.path()).unwrap().flatten().map(|e| e.path()));
        }
        assert_eq!(files.len(), 1, "{files:?}");
    }

    #[test]
    fn respects_the_limit_and_off() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = wav(1000);
        let len = bytes.len() as u64;
        let cache = cache(dir.path(), len - 1);
        assert_eq!(cache.store(&entry("a"), None, &Bytes(bytes.clone()), len), StoreOutcome::TooLarge);
        cache.set_limit(len * 3).unwrap();
        for id in ["a", "b", "c"] {
            cache.store(&entry(id), None, &Bytes(bytes.clone()), len);
        }
        cache.set_limit(len).unwrap();
        assert_eq!(cache.usage().1, 1);
        cache.set_limit(0).unwrap();
        assert_eq!(cache.usage(), (0, 0));
        assert_eq!(cache.store(&entry("d"), None, &Bytes(bytes), len), StoreOutcome::Disabled);
    }

    #[test]
    fn a_nearly_full_disk_pauses_caching() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let bytes = wav(1000);
        let len = bytes.len() as u64;
        // A 100 GB disk with 4 GB free is under the 5 GB reserve.
        cache.set_space_probe(|_| Some((4 * GB, 100 * GB)));
        assert_eq!(cache.store(&entry("a"), None, &Bytes(bytes.clone()), len), StoreOutcome::LowDisk);
        assert!(cache.low_disk());
        cache.set_space_probe(|_| Some((50 * GB, 100 * GB)));
        assert_eq!(cache.store(&entry("a"), None, &Bytes(bytes), len), StoreOutcome::Stored);
        assert!(!cache.low_disk());
    }

    #[test]
    fn reconcile_drops_orphans_and_partials() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = wav(1000);
        let len = bytes.len() as u64;
        {
            let cache = cache(dir.path(), GB);
            cache.store(&entry("a"), None, &Bytes(bytes.clone()), len);
            cache.store(&entry("b"), None, &Bytes(bytes.clone()), len);
            fs::remove_file(cache.lookup("b", Quality::Original, 0).unwrap().path).unwrap();
        }
        fs::create_dir_all(dir.path().join("ab")).unwrap();
        fs::write(dir.path().join("ab/orphan.orig"), b"x").unwrap();
        fs::write(dir.path().join("ab/half.orig.part"), b"x").unwrap();
        let cache = SongCache::open(dir.path().to_path_buf(), None).unwrap();
        cache.reconcile();
        assert_eq!(cache.usage(), (len, 1));
        assert!(!dir.path().join("ab/orphan.orig").exists());
        assert!(!dir.path().join("ab/half.orig.part").exists());
    }

    #[test]
    fn take_moves_the_file_out() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), GB);
        let bytes = wav(1000);
        cache.store(&entry("a"), None, &Bytes(bytes.clone()), bytes.len() as u64);
        let dest = dir.path().join("downloads/a.flac");
        assert_eq!(cache.take("a", Quality::Original, 0, &dest), Some(bytes.len() as u64));
        assert_eq!(fs::read(dest).unwrap(), bytes);
        assert_eq!(cache.usage(), (0, 0));
    }
}
