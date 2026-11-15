//! Bringing the local library in step with the server. A full sync pages `search3` per library,
//! writing each page with its resume cursor so a crash loses at most one page. Removal waits until
//! every page was seen and the server's scan state held still, since offset paging can skip rows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::client::native::{LibraryTotals, PlaylistImageError, SmartSaveError, SmartSaved, read_image_file};
use crate::smart_rules::{checked_criteria, checked_name};
use crate::client::{Album, AlbumListOptions, Artist, Client, MusicFolder, Playlist, PlaylistDetails, SearchKind, Song};
use crate::error::{Error, Result};
use crate::library::{Library, LibraryCounts};
use crate::reporting::PlayReporter;

const META_LAST_MARKER: &str = "last_scan_marker";
const META_LAST_COMPLETED: &str = "last_completed_at";
const META_NEEDS_RECONCILE: &str = "needs_reconcile";
/// How long a sync waits for the server's library totals before it starts without them.
const TOTALS_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    Artists,
    Albums,
    Songs,
    Done,
}

impl SyncPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SyncPhase::Artists => "artists",
            SyncPhase::Albums => "albums",
            SyncPhase::Songs => "songs",
            SyncPhase::Done => "done",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "artists" => SyncPhase::Artists,
            "albums" => SyncPhase::Albums,
            "songs" => SyncPhase::Songs,
            _ => SyncPhase::Done,
        }
    }
}

/// Persisted position of a library's sync.
#[derive(Clone, Debug)]
pub struct SyncCursor {
    pub generation: i64,
    pub active: bool,
    pub phase: SyncPhase,
    pub offset: u32,
    /// `lastScan` when this generation began.
    pub scan_marker: Option<String>,
    /// A scan was already running when this generation began.
    pub hold_removal: bool,
    pub pending_shrink: bool,
}

/// What finishing a library did.
#[derive(Clone, Debug, Default)]
pub struct LibraryFinish {
    pub removed: LibraryCounts,
    /// Why removal was held back, if it was.
    pub held: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SyncOptions {
    pub page_size: u32,
    /// Pause between pages, to be gentle on a shared server.
    pub page_delay: Duration,
    /// Set to stop a running sync between pages. The next run resumes where it stopped.
    pub cancel: Arc<AtomicBool>,
    /// Also fetch the dated play history behind Top Songs. It logs in to the native API, so it's
    /// off unless the app asks. Test runs must leave it off.
    pub play_history: bool,
    /// Fetch kept smart playlists on every playlist sync, so their downloads follow the server.
    /// Fetching one re-evaluates it on the server, so test runs turn this off.
    pub synced_smart: bool,
    /// Where the app reads a running sync's progress. A first sync runs on through the playlist
    /// pass that follows it.
    pub watch: Option<Arc<SyncWatch>>,
    /// Ask Navidrome's native API for each library's totals, so the bar is determinate. It logs
    /// in, so it's off unless the app asks and test runs must leave it off.
    pub server_totals: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self { page_size: 500, page_delay: Duration::from_millis(20), cancel: Arc::new(AtomicBool::new(false)), play_history: false, synced_smart: true, watch: None, server_totals: false }
    }
}

#[derive(Clone, Debug)]
pub struct SyncProgress {
    pub library_id: String,
    pub library_name: String,
    pub phase: SyncPhase,
    /// Items processed so far in this library's phase, including resumed ones.
    pub offset: u32,
    /// What to show: the stage across every library, how far it is, and the total when known.
    pub meter: SyncMeter,
}

/// What a sync is doing, for a progress bar. Playlists come after the library's pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStage {
    Artists,
    Albums,
    Songs,
    Playlists,
}

/// A sync's progress. `done` counts the current stage across every library. `total` is only what
/// the server reported, and is dropped when smaller than what's already stored, so a bar never
/// claims more than it knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncMeter {
    pub stage: SyncStage,
    pub done: u32,
    pub total: Option<u32>,
    /// `(done, total)` across the artist, album and song stages together.
    pub overall: Option<(u32, u32)>,
}

impl SyncMeter {
    fn new(stage: SyncStage, done: u32, total: Option<u32>) -> Self {
        Self { stage, done, total: total.filter(|t| *t >= done && *t > 0), overall: None }
    }

    fn with_overall(mut self, done: u32, total: Option<u32>) -> Self {
        self.overall = total.filter(|t| *t >= done && *t > 0).map(|t| (done, t));
        self
    }

    /// 0 to 1 for the bar: the whole sync when every total is known, else the current stage.
    pub fn fraction(&self) -> Option<f32> {
        let (done, total) = self.overall.or(self.total.map(|t| (self.done, t)))?;
        Some((done as f32 / total as f32).clamp(0.0, 1.0))
    }
}

/// The sync the app is showing, shared between the sync task and the interface. A first sync shows
/// at once, and a later pass only once it has run a while, so quick refreshes stay silent.
#[derive(Default)]
pub struct SyncWatch {
    state: std::sync::Mutex<Option<SyncShown>>,
    listener: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for SyncWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncWatch").field("state", &self.current()).finish()
    }
}

/// A running sync as the interface sees it.
#[derive(Clone, Copy, Debug)]
pub struct SyncShown {
    pub meter: Option<SyncMeter>,
    /// The library was empty when it started, so nothing is browsable until rows arrive.
    pub first: bool,
    pub started: Instant,
}

impl SyncWatch {
    pub fn new(listener: impl Fn() + Send + Sync + 'static) -> Self {
        Self { state: Default::default(), listener: std::sync::Mutex::new(Some(Box::new(listener))) }
    }

    fn changed(&self) {
        if let Some(listener) = self.listener.lock().unwrap().as_ref() {
            listener();
        }
    }

    fn begin(&self, first: bool) {
        *self.state.lock().unwrap() = Some(SyncShown { meter: None, first, started: Instant::now() });
        self.changed();
    }

    /// Updates a sync that's showing. One that isn't stays silent.
    fn update(&self, meter: SyncMeter) {
        let showing = match self.state.lock().unwrap().as_mut() {
            Some(shown) => {
                shown.meter = Some(meter);
                true
            }
            None => false,
        };
        if showing {
            self.changed();
        }
    }

    fn finish(&self) {
        let was = self.state.lock().unwrap().take().is_some();
        if was {
            self.changed();
        }
    }

    pub fn current(&self) -> Option<SyncShown> {
        *self.state.lock().unwrap()
    }

    /// The sync to show now: a first sync, or one that has run at least `show_after`.
    pub fn visible(&self, show_after: Duration) -> Option<SyncShown> {
        self.current().filter(|s| s.first || s.started.elapsed() >= show_after)
    }
}

/// Items already counted in each library phase by earlier libraries in this run, and the totals.
#[derive(Default)]
struct RunTally {
    before: [u32; 3],
    /// `getScanStatus`'s count, when there are no per-library totals.
    songs_total: Option<u32>,
    /// Each library's totals in folder order, only when the server gave every one.
    libraries: Option<Vec<LibraryTotals>>,
}

impl RunTally {
    fn library_total(t: &LibraryTotals, phase: SyncPhase) -> u32 {
        match phase {
            SyncPhase::Artists => t.artists,
            SyncPhase::Albums => t.albums,
            _ => t.songs,
        }
    }

    fn stage_total(&self, phase: SyncPhase) -> Option<u32> {
        match &self.libraries {
            Some(libs) => Some(libs.iter().map(|t| Self::library_total(t, phase)).fold(0u32, u32::saturating_add)),
            None if phase == SyncPhase::Songs => self.songs_total,
            None => None,
        }
    }

    fn grand_total(&self) -> Option<u32> {
        let libs = self.libraries.as_ref()?;
        Some(libs.iter().map(|t| t.artists.saturating_add(t.albums).saturating_add(t.songs)).fold(0u32, u32::saturating_add))
    }

    fn meter(&self, phase: SyncPhase, offset: u32) -> SyncMeter {
        let before_all = self.before.iter().fold(0u32, |a, b| a.saturating_add(*b));
        SyncMeter::new(stage_of(phase), self.before[phase_index(phase)].saturating_add(offset), self.stage_total(phase)).with_overall(before_all.saturating_add(offset), self.grand_total())
    }
}

fn phase_index(phase: SyncPhase) -> usize {
    match phase {
        SyncPhase::Artists => 0,
        SyncPhase::Albums => 1,
        _ => 2,
    }
}

fn stage_of(phase: SyncPhase) -> SyncStage {
    match phase {
        SyncPhase::Artists => SyncStage::Artists,
        SyncPhase::Albums => SyncStage::Albums,
        _ => SyncStage::Songs,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// Every library's pages were seen to the end.
    Complete,
    /// Stopped early. The next full sync resumes.
    Cancelled,
}

#[derive(Clone, Debug, Default)]
pub struct LibrarySyncResult {
    pub library_id: String,
    pub removed: LibraryCounts,
    /// Why removal was held back, if it was.
    pub removal_held: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub status: SyncStatus,
    pub libraries: Vec<LibrarySyncResult>,
    pub counts: LibraryCounts,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub enum RefreshOutcome {
    /// The server's last scan hasn't changed since the last complete sync.
    Unchanged,
    /// The server is scanning; try again later.
    ServerScanning,
    /// Another sync is already running.
    AlreadyRunning,
    Synced(SyncOutcome),
}

pub struct Syncer {
    client: Client,
    library: Library,
    options: SyncOptions,
}

impl Syncer {
    pub fn new(client: Client, library: Library, options: SyncOptions) -> Self {
        Self { client, library, options }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.options.cancel.clone()
    }

    /// A full sync, resuming any interrupted generation.
    pub async fn full_sync(&self, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<SyncOutcome> {
        let _gate = self.library.sync_gate().await;
        self.run(on_progress).await
    }

    /// Sync only if the server's library changed since the last complete sync, unless `force`.
    pub async fn refresh(&self, force: bool, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<RefreshOutcome> {
        let Some(_gate) = self.library.try_sync_gate() else {
            return Ok(RefreshOutcome::AlreadyRunning);
        };
        let status = match self.client.scan_status().await {
            Ok(s) => Some(s),
            Err(e) if e.is_transient() => return Err(e),
            // Not implemented here, so sync on schedule instead.
            Err(_) => None,
        };
        if status.as_ref().is_some_and(|s| s.scanning) {
            return Ok(RefreshOutcome::ServerScanning);
        }
        if !force && self.is_current(status.as_ref().and_then(|s| s.last_scan.as_deref()))? {
            return Ok(RefreshOutcome::Unchanged);
        }
        Ok(RefreshOutcome::Synced(self.run(on_progress).await?))
    }

    fn is_current(&self, marker: Option<&str>) -> Result<bool> {
        let lib = &self.library;
        Ok(marker.is_some()
            && lib.meta(META_NEEDS_RECONCILE)?.is_none()
            && lib.meta(META_LAST_COMPLETED)?.is_some()
            && lib.meta(META_LAST_MARKER)?.as_deref() == marker)
    }

    async fn run(&self, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<SyncOutcome> {
        let Some(watch) = self.options.watch.clone() else {
            return self.run_pages(on_progress).await;
        };
        // First means no pass has ever finished here, even if an interrupted first sync resumes.
        let first = self.library.meta(META_LAST_COMPLETED).map(|m| m.is_none()).unwrap_or(false);
        watch.begin(first);
        let shown = |p: SyncProgress| {
            watch.update(p.meter);
            on_progress(p);
        };
        let result = self.run_pages(&shown).await;
        match &result {
            // The playlist pass that follows a first sync keeps the bar until it ends.
            Ok(outcome) if first && outcome.status == SyncStatus::Complete => watch.update(SyncMeter::new(SyncStage::Playlists, 0, None)),
            _ => watch.finish(),
        }
        result
    }

    async fn run_pages(&self, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<SyncOutcome> {
        // A cancel request applies to the run in progress, not to future runs.
        self.options.cancel.store(false, Ordering::Release);
        let started = Instant::now();
        let folders = self.client.music_folders().await?;
        if folders.is_empty() {
            return Err(Error::Sync("the server lists no music libraries for this user".to_string()));
        }
        let start_status = self.client.scan_status().await.ok();
        let marker = start_status.as_ref().and_then(|s| s.last_scan.clone());
        let scanning = start_status.as_ref().is_some_and(|s| s.scanning);

        {
            let f = folders.clone();
            self.library.blocking(move |l| l.replace_libraries(&f)).await?;
        }

        let mut results = Vec::with_capacity(folders.len());
        let mut any_held = false;
        let libraries = self.library_totals(&folders).await;
        let mut tally = RunTally { songs_total: start_status.as_ref().and_then(|s| s.count).map(|c| c.min(u32::MAX as u64) as u32), libraries, ..Default::default() };
        for (index, folder) in folders.iter().enumerate() {
            let Some(result) = self.sync_library(folder, index, marker.as_deref(), scanning, &mut tally, on_progress).await? else {
                return self.outcome(SyncStatus::Cancelled, results, started);
            };
            any_held |= result.removal_held.is_some();
            results.push(result);
        }

        let lib = self.library.clone();
        let marker_to_store = marker.clone();
        lib.blocking(move |l| {
            l.set_meta(META_LAST_COMPLETED, Some(&unix_seconds().to_string()))?;
            if any_held {
                l.set_meta(META_NEEDS_RECONCILE, Some("1"))?;
            } else {
                l.set_meta(META_NEEDS_RECONCILE, None)?;
                l.set_meta(META_LAST_MARKER, marker_to_store.as_deref())?;
            }
            Ok(())
        })
        .await?;

        self.outcome(SyncStatus::Complete, results, started)
    }

    /// Each library's totals for the bar, or `None` when not asked for or any is unavailable.
    /// With one library it asks without a filter, so older servers answer too.
    async fn library_totals(&self, folders: &[MusicFolder]) -> Option<Vec<LibraryTotals>> {
        if !self.options.server_totals || self.options.watch.is_none() {
            return None;
        }
        let ask = async {
            let mut out = Vec::with_capacity(folders.len());
            for folder in folders {
                let filter = (folders.len() > 1).then_some(folder.id.as_str());
                out.push(self.client.library_totals(filter).await?);
            }
            Ok::<_, Error>(out)
        };
        // A slow answer mustn't hold up the pages. The bar falls back instead.
        match tokio::time::timeout(TOTALS_WAIT, ask).await {
            Ok(Ok(out)) => Some(out),
            Ok(Err(e)) => {
                tracing::debug!("no library totals for the sync bar: {e}");
                None
            }
            Err(_) => {
                tracing::debug!("no library totals for the sync bar: no answer in {TOTALS_WAIT:?}");
                None
            }
        }
    }

    fn outcome(&self, status: SyncStatus, libraries: Vec<LibrarySyncResult>, started: Instant) -> Result<SyncOutcome> {
        Ok(SyncOutcome { status, libraries, counts: self.library.counts()?, elapsed: started.elapsed() })
    }

    /// Returns `None` if canceled.
    async fn sync_library(&self, folder: &MusicFolder, folder_index: usize, marker: Option<&str>, scanning: bool, tally: &mut RunTally, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<Option<LibrarySyncResult>> {
        let cursor = {
            let id = folder.id.clone();
            let m = marker.map(str::to_string);
            self.library.blocking(move |l| l.begin_sync(&id, m.as_deref(), scanning)).await?
        };

        let phases = [SyncPhase::Artists, SyncPhase::Albums, SyncPhase::Songs];
        for (index, phase) in phases.into_iter().enumerate() {
            let order = |p: SyncPhase| phases.iter().position(|x| *x == p).unwrap_or(phases.len());
            if order(cursor.phase) > index {
                // Stored by the run this one resumes, so counted as done when the total is known.
                if let Some(t) = tally.libraries.as_ref().and_then(|l| l.get(folder_index)) {
                    tally.before[phase_index(phase)] += RunTally::library_total(t, phase);
                }
                continue;
            }
            let start_offset = if cursor.phase == phase { cursor.offset } else { 0 };
            let tally_ref = &*tally;
            let reached = match phase {
                SyncPhase::Artists => self.sync_phase::<Artist>(folder, &cursor, phase, start_offset, tally_ref, on_progress, |l, f, c, n, items: Vec<Artist>| l.apply_artists(f, c, n, &items)).await?,
                SyncPhase::Albums => self.sync_phase::<Album>(folder, &cursor, phase, start_offset, tally_ref, on_progress, |l, f, c, n, items: Vec<Album>| l.apply_albums(f, c, n, &items)).await?,
                _ => self.sync_phase::<Song>(folder, &cursor, phase, start_offset, tally_ref, on_progress, |l, f, c, n, items: Vec<Song>| l.apply_songs(f, c, n, &items)).await?,
            };
            let Some(reached) = reached else {
                return Ok(None);
            };
            tally.before[phase_index(phase)] += reached;
        }

        // Compare the scan state now against when this generation began.
        let end_status = self.client.scan_status().await.ok();
        let hold = removal_hold(&cursor, end_status.as_ref());
        let finish = {
            let id = folder.id.clone();
            let c = cursor.clone();
            self.library.blocking(move |l| l.finish_library(&id, &c, hold)).await?
        };
        Ok(Some(LibrarySyncResult { library_id: folder.id.clone(), removed: finish.removed, removal_held: finish.held }))
    }

    /// Pages one phase to its end. Returns how many items it reached, or `None` if canceled.
    #[allow(clippy::too_many_arguments)]
    async fn sync_phase<T>(
        &self,
        folder: &MusicFolder,
        cursor: &SyncCursor,
        phase: SyncPhase,
        mut offset: u32,
        tally: &RunTally,
        on_progress: &(dyn Fn(SyncProgress) + Send + Sync),
        apply: impl Fn(&Library, &str, &SyncCursor, u32, Vec<T>) -> Result<()> + Send + Sync + Copy + 'static,
    ) -> Result<Option<u32>>
    where
        T: serde::de::DeserializeOwned + Send + 'static,
    {
        let kind = match phase {
            SyncPhase::Artists => SearchKind::Artist,
            SyncPhase::Albums => SearchKind::Album,
            _ => SearchKind::Song,
        };
        let meter = |offset: u32| tally.meter(phase, offset);
        // Shown before the first page, so a bar has its stage and total at once.
        on_progress(SyncProgress { library_id: folder.id.clone(), library_name: folder.name.clone(), phase, offset, meter: meter(offset) });
        loop {
            if self.options.cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            let page = self.client.search_page::<T>(kind, Some(&folder.id), offset, self.options.page_size).await?;
            if page.raw_count == 0 {
                let id = folder.id.clone();
                let next = next_phase(phase);
                self.library.blocking(move |l| l.advance_phase(&id, next)).await?;
                return Ok(Some(offset));
            }
            let next_offset = offset + page.raw_count as u32;
            {
                let id = folder.id.clone();
                let cursor = cursor.clone();
                let items = page.items;
                self.library.blocking(move |l| apply(&l, &id, &cursor, next_offset, items)).await?;
            }
            offset = next_offset;
            on_progress(SyncProgress { library_id: folder.id.clone(), library_name: folder.name.clone(), phase, offset, meter: meter(offset) });
            if !self.options.page_delay.is_zero() {
                tokio::time::sleep(self.options.page_delay).await;
            }
        }
    }
}

/// Whether a cover id ends with Navidrome's image hash (`pl-<id>_<hash>`).
fn has_image_hash(cover: &str) -> bool {
    cover.contains('_')
}

/// Whether removal must be held. If the library changed while we paged, a row we didn't see may
/// just have shifted past a page boundary.
fn removal_hold(cursor: &SyncCursor, end: Option<&crate::client::ScanStatus>) -> Option<String> {
    if cursor.hold_removal {
        return Some("the server was scanning when this sync began".to_string());
    }
    let end = end?;
    if end.scanning {
        return Some("the server started scanning during this sync".to_string());
    }
    if cursor.scan_marker.is_some() && end.last_scan.is_some() && cursor.scan_marker != end.last_scan {
        return Some("the server finished a scan during this sync".to_string());
    }
    None
}

fn next_phase(phase: SyncPhase) -> SyncPhase {
    match phase {
        SyncPhase::Artists => SyncPhase::Albums,
        SyncPhase::Albums => SyncPhase::Songs,
        _ => SyncPhase::Done,
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// How many playlists a sync touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaylistSync {
    pub listed: usize,
    pub refetched: usize,
    pub removed: u64,
}

/// What a play-count and stars refresh found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnnotationRefresh {
    pub stars_changed: u64,
    pub albums_scanned: usize,
    pub albums_refetched: usize,
    pub songs_changed: u64,
}

const META_ANNOTATION_MARK: &str = "annotation_played_mark";
/// Recently played albums to walk in one pass. The walk stops sooner at the last checkpoint.
const MAX_ANNOTATION_SCAN: usize = 1000;
/// Pause between per-item fetches, to keep load modest on shared servers.
const FETCH_PAUSE: Duration = Duration::from_millis(20);
/// How long to watch for a playlist's new cover id after an image change, in steps. Navidrome
/// can take a few seconds to publish it.
const COVER_FOLLOW_TRIES: usize = 30;
const COVER_FOLLOW_PAUSE: Duration = Duration::from_millis(500);
/// Songs or positions per playlist write sent in the URL, on servers without `formPost`. Small
/// enough for any server's URL limit with long ids.
const URL_CHUNK: usize = 100;

impl Syncer {
    /// Bring the local playlists in step, fetching each in full only if its `changed` stamp or
    /// song count moved. Smart and read-only playlists keep only their list entry, because
    /// `getPlaylist` re-evaluates a smart playlist for everyone. They're fetched when opened.
    pub async fn sync_playlists(&self) -> Result<PlaylistSync> {
        self.sync_playlists_with(&|_| {}).await
    }

    /// [`Self::sync_playlists`], reporting each playlist checked against the list's length.
    pub async fn sync_playlists_with(&self, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<PlaylistSync> {
        let list = match self.client.playlists().await {
            Ok(list) => list,
            Err(e) => {
                if let Some(watch) = &self.options.watch {
                    watch.finish();
                }
                return Err(e);
            }
        };
        let listed = list.len();
        let report = |done: usize| {
            let meter = SyncMeter::new(SyncStage::Playlists, done as u32, Some(listed as u32));
            if let Some(watch) = &self.options.watch {
                watch.update(meter);
            }
            on_progress(SyncProgress { library_id: String::new(), library_name: String::new(), phase: SyncPhase::Done, offset: done as u32, meter });
        };
        report(0);
        let result = self.sync_playlist_list(list, listed, &report).await;
        if let Some(watch) = &self.options.watch {
            watch.finish();
        }
        result
    }

    async fn sync_playlist_list(&self, list: Vec<crate::client::models::Playlist>, listed: usize, report: &(dyn Fn(usize) + Send + Sync)) -> Result<PlaylistSync> {
        let mut refetched = 0;
        // Kept playlists need their songs even when smart or another user's, so downloads follow.
        let kept: std::collections::HashSet<String> = self
            .library
            .blocking(|l| l.synced_collections())
            .await?
            .into_iter()
            .filter(|c| c.kind == crate::library::collections::CollectionKind::Playlist)
            .map(|c| c.id)
            .collect();
        for (index, item) in list.iter().enumerate() {
            if index > 0 {
                report(index);
            }
            let id = item.id.clone();
            let marker = self.library.blocking({ let id = id.clone(); move |l| l.playlist_marker(&id) }).await?;
            let (mut current, mut same_cover) = (false, false);
            if let Some((changed, count, cover)) = marker {
                current = changed == item.changed && count == item.song_count.unwrap_or(count);
                same_cover = cover == item.cover_art;
            }
            if kept.contains(&id) {
                let smart = item.valid_until.is_some();
                let unfetched = self.library.blocking({ let id = id.clone(); move |l| l.collection_needs_fetch(crate::library::collections::CollectionKind::Playlist, &id) }).await?;
                // A smart playlist only changes when a fetch evaluates it. One never evaluated
                // looks `readonly`, so with `synced_smart` off those aren't fetched either.
                let may_be_smart = item.readonly.unwrap_or(true);
                if (smart && self.options.synced_smart) || (!smart && (!current || unfetched) && (self.options.synced_smart || !may_be_smart)) {
                    let full = self.client.playlist(&id).await?;
                    self.library.blocking(move |l| l.store_playlist(&full)).await?;
                    refetched += 1;
                    tokio::time::sleep(FETCH_PAUSE).await;
                    continue;
                }
            }
            if current {
                // A regenerated cover gets a new id without moving `changed`. Only the list entry
                // holds the id, so the songs needn't be fetched again.
                if !same_cover {
                    let header = item.clone();
                    self.library.blocking(move |l| l.store_playlist_header(&header)).await?;
                }
                continue;
            }
            if item.valid_until.is_some() || item.readonly.unwrap_or(false) {
                let header = item.clone();
                self.library.blocking(move |l| l.store_playlist_header(&header)).await?;
                continue;
            }
            let full = self.client.playlist(&id).await?;
            self.library.blocking(move |l| l.store_playlist(&full)).await?;
            refetched += 1;
            tokio::time::sleep(FETCH_PAUSE).await;
        }
        let keep: Vec<String> = list.iter().map(|p| p.id.clone()).collect();
        let removed = self.library.blocking(move |l| l.delete_playlists_except(&keep)).await?;
        if refetched > 0 {
            // Fetching a smart playlist moves its `changed` stamp, and a list's song count can
            // differ from its entries. Remember the list as it is now so the next pass only
            // refetches real changes.
            let current = self.client.playlists().await?;
            self.library.blocking(move |l| l.set_playlist_markers(&current)).await?;
        }
        Ok(PlaylistSync { listed, refetched, removed })
    }

    /// Create a playlist on the server and store it, so it shows without a full sync. Returns
    /// its id, or `None` if the server didn't say.
    pub async fn create_playlist(&self, name: &str) -> Result<Option<String>> {
        let Some(created) = self.client.create_playlist(name).await? else { return Ok(None) };
        let id = created.id.clone();
        self.library.blocking(move |l| l.store_playlist(&created)).await?;
        Ok(Some(id))
    }

    /// Saves a new song order for a regular playlist this user can edit. The playlist is fetched
    /// first and the save refused if it no longer matches `expected`, so another client's edit
    /// is never overwritten. Smart and read-only playlists are refused before any request.
    pub async fn save_playlist_order(&self, id: &str, expected: &[String], order: &[String]) -> std::result::Result<(), PlaylistEditError> {
        // Refused before reading, so a read-only run never even fetches.
        self.editable_header(id).await?;
        let mut a = expected.to_vec();
        let mut b = order.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        if a != b {
            return Err(PlaylistEditError::Failed("the new order doesn't hold the same songs".to_string()));
        }
        let current = self.client.playlist(id).await.map_err(PlaylistEditError::from)?;
        if !order_editable(&current) {
            let header = Playlist { entry: Vec::new(), ..current };
            let _ = self.library.blocking(move |l| l.store_playlist_header(&header)).await;
            return Err(PlaylistEditError::ReadOnly);
        }
        let ids: Vec<String> = current.entry.iter().map(|s| s.id.clone()).collect();
        if ids != expected {
            let _ = self.library.blocking(move |l| l.store_playlist(&current)).await;
            return Err(PlaylistEditError::Changed);
        }
        let answered = self.client.replace_playlist_songs(id, order).await.map_err(PlaylistEditError::from)?;
        let saved = match answered {
            Some(p) if p.entry.len() == order.len() => p,
            _ => self.client.playlist(id).await.map_err(PlaylistEditError::from)?,
        };
        let saved_ids: Vec<String> = saved.entry.iter().map(|s| s.id.clone()).collect();
        self.library.blocking(move |l| l.store_playlist(&saved)).await.map_err(PlaylistEditError::from)?;
        if saved_ids != order {
            return Err(PlaylistEditError::Failed("the server kept a different order".to_string()));
        }
        Ok(())
    }

    /// Creates a regular playlist holding these songs and stores it. Without `formPost`, a long
    /// list goes in chunks through `updatePlaylist`. Returns the new id if the server gave one.
    pub async fn create_playlist_with(&self, name: &str, song_ids: &[String], form: bool) -> Result<Option<String>> {
        let first = if form { song_ids.len() } else { song_ids.len().min(URL_CHUNK) };
        let Some(created) = self.client.create_playlist_with(name, &song_ids[..first], form).await? else { return Ok(None) };
        let id = created.id.clone();
        for chunk in song_ids[first..].chunks(URL_CHUNK) {
            self.client.update_playlist_songs(&id, chunk, &[], false).await?;
        }
        let stored = if first < song_ids.len() { self.client.playlist(&id).await? } else { created };
        self.library.blocking(move |l| l.store_playlist(&stored)).await?;
        Ok(Some(id))
    }

    /// Appends songs to a regular playlist this user owns, then stores the server's copy. `form`
    /// is whether the server offers `formPost`. Without it a long list goes in chunks.
    pub async fn add_to_playlist(&self, id: &str, song_ids: &[String], form: bool) -> std::result::Result<(), PlaylistEditError> {
        self.editable_header(id).await?;
        if song_ids.is_empty() {
            return Ok(());
        }
        if form {
            self.client.update_playlist_songs(id, song_ids, &[], true).await?;
        } else {
            for chunk in song_ids.chunks(URL_CHUNK) {
                self.client.update_playlist_songs(id, chunk, &[], false).await?;
            }
        }
        self.store_fresh(id).await
    }

    /// Removes the entries at these positions of a regular playlist this user owns. Nothing is
    /// removed if the server's order differs from `expected`, so a position never hits another song.
    pub async fn remove_from_playlist(&self, id: &str, expected: &[String], indexes: &[usize], form: bool) -> std::result::Result<(), PlaylistEditError> {
        self.editable_header(id).await?;
        let mut indexes: Vec<usize> = indexes.iter().copied().filter(|&i| i < expected.len()).collect();
        indexes.sort_unstable_by(|a, b| b.cmp(a));
        indexes.dedup();
        if indexes.is_empty() {
            return Ok(());
        }
        let current = self.client.playlist(id).await?;
        if !order_editable(&current) {
            let header = Playlist { entry: Vec::new(), ..current };
            let _ = self.library.blocking(move |l| l.store_playlist_header(&header)).await;
            return Err(PlaylistEditError::ReadOnly);
        }
        if !current.entry.iter().map(|s| s.id.as_str()).eq(expected.iter().map(String::as_str)) {
            let _ = self.library.blocking(move |l| l.store_playlist(&current)).await;
            return Err(PlaylistEditError::Changed);
        }
        if form {
            self.client.update_playlist_songs(id, &[], &indexes, true).await?;
        } else {
            // Highest positions first, so each chunk's positions still point where they did.
            for chunk in indexes.chunks(URL_CHUNK) {
                self.client.update_playlist_songs(id, &[], chunk, false).await?;
            }
        }
        self.store_fresh(id).await
    }

    /// The stored header of a playlist this user may change songs in, or why not.
    async fn editable_header(&self, id: &str) -> std::result::Result<Playlist, PlaylistEditError> {
        let header = {
            let id = id.to_string();
            self.library.blocking(move |l| l.playlist_header(&id)).await?
        };
        let Some(header) = header else { return Err(PlaylistEditError::NotFound) };
        if !order_editable(&header) {
            return Err(PlaylistEditError::ReadOnly);
        }
        if self.client.write_policy() == crate::client::WritePolicy::ReadOnly {
            return Err(PlaylistEditError::WritesDisabled);
        }
        Ok(header)
    }

    /// After a change, fetch the regular playlist again and store it.
    async fn store_fresh(&self, id: &str) -> std::result::Result<(), PlaylistEditError> {
        let fresh = self.client.playlist(id).await?;
        self.library.blocking(move |l| l.store_playlist(&fresh)).await?;
        Ok(())
    }

    /// Replaces the playlist's image with this file and picks up its new cover. Returns the new
    /// cover id if the server has published one yet.
    pub async fn set_playlist_image(&self, id: &str, path: &std::path::Path) -> std::result::Result<Option<String>, PlaylistImageError> {
        let path = path.to_path_buf();
        let bytes = tokio::task::spawn_blocking(move || read_image_file(&path)).await.map_err(|e| PlaylistImageError::File(e.to_string()))??;
        let before = self.stored_cover(id).await;
        self.client.set_playlist_image(id, bytes).await?;
        Ok(self.follow_cover(id, before).await)
    }

    /// Removes the playlist's uploaded image and picks up the server's own cover in its place.
    pub async fn remove_playlist_image(&self, id: &str) -> std::result::Result<Option<String>, PlaylistImageError> {
        let before = self.stored_cover(id).await;
        self.client.remove_playlist_image(id).await?;
        Ok(self.follow_cover(id, before).await)
    }

    async fn stored_cover(&self, id: &str) -> Option<String> {
        let id = id.to_string();
        self.library.blocking(move |l| l.playlist_header(&id)).await.ok().flatten().and_then(|p| p.cover_art)
    }

    /// Watches the playlist list until the cover id moves, then stores the header. Navidrome
    /// hashes the image into the id in the background, which is what busts caches keyed on the
    /// old one. `getPlaylists` is used because it doesn't re-evaluate smart playlists.
    async fn follow_cover(&self, id: &str, before: Option<String>) -> Option<String> {
        let mut latest = None;
        let mut moved = false;
        for _ in 0..COVER_FOLLOW_TRIES {
            tokio::time::sleep(COVER_FOLLOW_PAUSE).await;
            let Ok(list) = self.client.playlists().await else { continue };
            let Some(item) = list.into_iter().find(|p| p.id == id) else { break };
            let cover = item.cover_art.clone();
            let header = item.clone();
            if self.library.blocking(move |l| l.store_playlist_header(&header)).await.is_err() {
                break;
            }
            let settled = cover.as_deref().is_some_and(has_image_hash);
            moved |= cover != before;
            latest = cover.clone();
            // Just after a change the id briefly has no hash and the server reuses it for older
            // art, so only a hashed id is safe to cache under. Re-uploading the same image gives
            // the same hash, which still counts once it has moved.
            if moved && (settled || cover.is_none()) {
                break;
            }
        }
        latest
    }

    /// Saves a smart playlist's name and rules, creating it when `id` is `None`, then fetches it
    /// once to evaluate the new rules. Refused before any request in a read-only run, for invalid
    /// input, or for a playlist this user can't change.
    pub async fn save_smart_playlist(&self, id: Option<&str>, name: &str, criteria: &str, admin: Option<bool>) -> std::result::Result<SmartSaved, SmartSaveError> {
        if self.client.write_policy() == crate::client::WritePolicy::ReadOnly {
            return Err(SmartSaveError::WritesDisabled);
        }
        let name = checked_name(name).map_err(|p| SmartSaveError::Invalid(p.0))?;
        let criteria = checked_criteria(criteria).map_err(|p| SmartSaveError::Invalid(p.0))?;
        if let Some(id) = id {
            let header = {
                let id = id.to_string();
                self.library.blocking(move |l| l.playlist_header(&id)).await.map_err(|e| SmartSaveError::Server(e.to_string()))?
            };
            let Some(header) = header else { return Err(SmartSaveError::NotFound) };
            if order_editable(&header) {
                return Err(SmartSaveError::NotSmart);
            }
            let me = self.client.credentials().username();
            let theirs = header.owner.as_deref().is_some_and(|o| !o.is_empty() && !o.eq_ignore_ascii_case(me));
            if theirs && admin != Some(true) {
                return Err(SmartSaveError::NotAllowed);
            }
        }
        let saved = self.client.save_smart_playlist(id, &name, &criteria, admin).await?;
        // The save stands even if this fails; opening the playlist fetches it again.
        if let Ok(full) = self.client.playlist(&saved.id).await {
            let _ = self.library.blocking(move |l| l.store_playlist(&full)).await;
        }
        Ok(saved)
    }

    /// Change a playlist's name, description or Public setting, then patch the stored copy.
    /// Patched, not fetched, because `getPlaylist` would re-evaluate a smart playlist.
    pub async fn update_playlist_details(&self, id: &str, details: PlaylistDetails) -> Result<()> {
        self.client.update_playlist_details(id, &details).await?;
        let id = id.to_string();
        self.library
            .blocking(move |l| {
                let Some(mut stored) = l.playlist_header(&id)? else { return Ok(()) };
                if let Some(name) = details.name {
                    stored.name = name;
                }
                if let Some(comment) = details.comment {
                    stored.comment = (!comment.is_empty()).then_some(comment);
                }
                if let Some(public) = details.public {
                    stored.public = Some(public);
                }
                l.store_playlist_header(&stored)?;
                l.contents_changed();
                Ok(())
            })
            .await
    }

    /// Fetch one playlist now, when it's opened. Smart playlists are only re-evaluated when
    /// fetched, so their stored copy can lag.
    pub async fn refresh_playlist(&self, id: &str) -> Result<()> {
        let full = self.client.playlist(id).await?;
        self.library.blocking(move |l| l.store_playlist(&full)).await
    }

    /// Update stars and play counts without a catalog sync. Costs one `getStarred2` plus a page
    /// or two of recently played albums, then one `getAlbum` per album whose plays moved.
    pub async fn refresh_annotations(&self) -> Result<AnnotationRefresh> {
        let starred = self.client.starred2().await?;
        let stars = self.library.blocking(move |l| l.apply_starred(&starred)).await?;
        let mut out = AnnotationRefresh { stars_changed: stars.songs + stars.albums + stars.artists, ..Default::default() };

        let mark = self.library.meta(META_ANNOTATION_MARK)?;
        let mut newest: Option<String> = None;
        let mut offset = 0u32;
        let mut reached_mark = false;
        'pages: while out.albums_scanned < MAX_ANNOTATION_SCAN {
            let page = self.client.album_list2(&AlbumListOptions::new("recent", 200, offset)).await?;
            if page.is_empty() {
                reached_mark = true;
                break;
            }
            let count = page.len() as u32;
            for album in page {
                out.albums_scanned += 1;
                let played = album.played.clone();
                if newest.is_none() {
                    newest.clone_from(&played);
                }
                // The list is newest first, so older albums were covered by the last pass.
                if let (Some(p), Some(m)) = (&played, &mark)
                    && p <= m
                {
                    reached_mark = true;
                    break 'pages;
                }
                let check = album.clone();
                if !self.library.blocking(move |l| l.album_annotation_differs(&check)).await? {
                    continue;
                }
                let full = self.client.album(&album.id).await?;
                out.songs_changed += self.library.blocking(move |l| l.apply_album_annotations(&full)).await?;
                out.albums_refetched += 1;
                tokio::time::sleep(FETCH_PAUSE).await;
            }
            offset += count;
        }
        // The first pass may stop at the scan limit, and a full sync covers anything older. With
        // a checkpoint set, hitting the limit leaves it there so the next pass catches up.
        if let Some(n) = newest
            && (reached_mark || mark.is_none())
        {
            self.library.set_meta(META_ANNOTATION_MARK, Some(&n))?;
        }
        if self.options.play_history
            && let Err(e) = super::recent::refresh_play_history(&self.client, &self.library).await
        {
            tracing::warn!("play history refresh failed: {e}");
        }
        Ok(out)
    }
}

/// Periodic refresh in the background. Dropping the handle stops it.
pub struct BackgroundRefresh {
    stop: Arc<Notify>,
    task: JoinHandle<()>,
    pub results: mpsc::UnboundedReceiver<Result<RefreshOutcome>>,
}

impl BackgroundRefresh {
    /// Refresh every `interval`, starting after the first interval.
    pub fn spawn(syncer: Syncer, interval: Duration) -> Self {
        Self::spawn_with(syncer, interval, None)
    }

    /// Like [`Self::spawn`], also refreshing playlists and annotations, and retrying plays on
    /// `reporter`. Failures in those extras are logged and retried on the next tick.
    pub fn spawn_with(syncer: Syncer, interval: Duration, reporter: Option<Arc<PlayReporter>>) -> Self {
        let stop = Arc::new(Notify::new());
        let stopped = stop.clone();
        let (tx, results) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopped.notified() => break,
                    _ = tokio::time::sleep(interval) => {}
                }
                let outcome = syncer.refresh(false, &|_| {}).await;
                let done = tx.send(outcome).is_err();
                if let Err(e) = syncer.sync_playlists().await {
                    tracing::warn!("playlist refresh failed: {e}");
                }
                if let Err(e) = syncer.refresh_annotations().await {
                    tracing::warn!("annotation refresh failed: {e}");
                }
                if let Some(reporter) = &reporter {
                    let _ = reporter.flush().await;
                }
                if done {
                    break;
                }
            }
        });
        Self { stop, task, results }
    }

    pub async fn stop(mut self) {
        self.stop.notify_one();
        let _ = (&mut self.task).await;
    }
}

impl Drop for BackgroundRefresh {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Whether this user can reorder a playlist's songs: a regular playlist not marked read-only.
/// A server that doesn't say counts as read-only.
pub fn order_editable(p: &Playlist) -> bool {
    p.valid_until.is_none() && p.readonly == Some(false)
}

/// Why a playlist order wasn't saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaylistEditError {
    #[error("test and read-only runs can't change playlists")]
    WritesDisabled,
    #[error("this playlist can't be edited")]
    ReadOnly,
    #[error("the playlist changed on the server")]
    Changed,
    #[error("the playlist isn't in the library")]
    NotFound,
    #[error("{0}")]
    Failed(String),
}

impl From<Error> for PlaylistEditError {
    fn from(e: Error) -> Self {
        match e {
            Error::WritesDisabled { .. } => Self::WritesDisabled,
            e => Self::Failed(e.to_string()),
        }
    }
}
