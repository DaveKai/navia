//! The play queue on the server, so another device can pick up where this one left off.
//! Saves are debounced and skipped while private or read-only. `savePlayQueueByIndex` is used when available.
//! At launch, [`QueueSync::remote_offer`] can offer a newer server queue, but never replaces the local one by itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::capabilities::ServerFeatures;
use crate::client::{Client, Song};
use crate::error::{Error, Result};
use crate::library::Library;
use crate::reporting::PlayReporter;

/// Changes within this long of each other are saved together.
pub const SAVE_AFTER: Duration = Duration::from_secs(5);
/// Most songs saved at once as a form. A larger queue is cut down around the current song.
const MAX_SAVED_FORM: usize = 1000;
/// Most songs saved when the ids have to fit in a URL.
const MAX_SAVED_URL: usize = 200;
/// Songs kept before the current one when a queue is cut down.
const KEEP_BEFORE: usize = 20;
/// A position moved less than this since the last save isn't worth another call.
const POSITION_SLACK_MS: u64 = 2000;

/// Listening this much of the queue at launch counts as choosing it.
const LISTENED_MS: u64 = 10_000;
/// How long [`QueueSync::remote_offer`] waits for the app's own queue to be restored.
const RESTORE_WAIT: Duration = Duration::from_secs(3);
/// How long [`QueueSync::remote_offer`] waits for the server's extension list.
const FEATURES_WAIT: Duration = Duration::from_secs(20);

const META_LOCAL: &str = "queue.local";
const META_SAVED: &str = "queue.saved";
const META_DISMISSED: &str = "queue.dismissed";

/// The listener's queue as the app has it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueState {
    pub song_ids: Vec<String>,
    /// The current song's place in `song_ids`.
    pub index: usize,
    pub position_ms: u64,
}

impl QueueState {
    fn fingerprint(&self) -> String {
        fingerprint(&self.song_ids, self.index)
    }
}

fn fingerprint(ids: &[String], index: usize) -> String {
    let mut text = index.to_string();
    for id in ids {
        text.push('\n');
        text.push_str(id);
    }
    format!("{:x}", md5::compute(text.as_bytes()))
}

/// A queue the server holds from another client or device, newer than this one's.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteQueue {
    /// The client that saved it, as it named itself in `changedBy`. May be empty.
    pub changed_by: String,
    /// When it was saved, in Unix ms.
    pub changed_ms: u64,
    pub songs: Vec<Song>,
    pub index: usize,
    pub position_ms: u64,
    /// The server's own stamp, which identifies this save.
    changed: String,
}

impl RemoteQueue {
    /// The queue as this device's library rows, leaving out songs it doesn't know.
    /// `None` when the current song itself isn't known.
    pub fn local_rows(&self, library: &Library) -> Option<(Vec<u32>, usize)> {
        let mut rows = Vec::with_capacity(self.songs.len());
        let mut index = None;
        for (i, song) in self.songs.iter().enumerate() {
            if let Some(row) = library.song_rowids(std::slice::from_ref(&song.id)).ok()?.first() {
                if i == self.index {
                    index = Some(rows.len());
                }
                rows.push(*row);
            }
        }
        Some((rows, index?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Sent {
    fingerprint: String,
    position_ms: u64,
}

struct Inner {
    client: Client,
    library: Library,
    reporter: Arc<PlayReporter>,
    features: Mutex<Option<ServerFeatures>>,
    pending: Mutex<Option<QueueState>>,
    /// The fingerprint last stored as this device's queue, so the database isn't written for every position.
    noted: Mutex<Option<String>>,
    sent: Mutex<Option<Sent>>,
    /// The rows last looked up by [`QueueSync::note_rows`], with their song ids.
    rows: Mutex<(Vec<u32>, Vec<Option<String>>)>,
    /// Nothing is saved at launch until the server's queue has been checked, so a restored local queue
    /// can't overwrite one the listener may want to continue.
    held: AtomicBool,
    /// The first queue noted this run and its position.
    launch: Mutex<Option<(String, u64)>>,
    flushing: tokio::sync::Mutex<()>,
    scheduled: AtomicBool,
    runtime: tokio::runtime::Handle,
}

/// One account's queue on the server. Cheap to clone.
#[derive(Clone)]
pub struct QueueSync {
    inner: Arc<Inner>,
}

impl QueueSync {
    /// `runtime` runs the delayed saves.
    pub fn new(client: Client, library: Library, reporter: Arc<PlayReporter>, runtime: tokio::runtime::Handle) -> Self {
        let noted = library.meta(META_LOCAL).ok().flatten().and_then(|v| v.split_once(' ').map(|(_, fp)| fp.to_string()));
        Self {
            inner: Arc::new(Inner {
                client,
                library,
                reporter,
                features: Mutex::new(None),
                pending: Mutex::new(None),
                noted: Mutex::new(noted),
                sent: Mutex::new(None),
                rows: Mutex::default(),
                held: AtomicBool::new(true),
                launch: Mutex::new(None),
                flushing: tokio::sync::Mutex::new(()),
                scheduled: AtomicBool::new(false),
                runtime,
            }),
        }
    }

    /// Sets what the server offers. Nothing is saved or offered until this is known.
    pub fn set_features(&self, features: ServerFeatures) {
        *self.inner.features.lock().unwrap() = Some(features);
    }

    fn features(&self) -> Option<ServerFeatures> {
        *self.inner.features.lock().unwrap()
    }

    /// Notes a change to the queue, song or position. Song changes save a few seconds later.
    /// A position change alone waits for [`Self::flush`].
    pub fn note(&self, state: QueueState) {
        if state.song_ids.is_empty() {
            return;
        }
        let fp = state.fingerprint();
        {
            let mut noted = self.inner.noted.lock().unwrap();
            if noted.as_deref() != Some(fp.as_str()) {
                // The first queue ever noted has no known time, so a server queue isn't hidden by it.
                let at = if noted.is_none() { 0 } else { now_ms() };
                let _ = self.inner.library.set_meta(META_LOCAL, Some(&format!("{at} {fp}")));
                *noted = Some(fp.clone());
            }
        }
        {
            let mut launch = self.inner.launch.lock().unwrap();
            match launch.as_ref() {
                None => *launch = Some((fp.clone(), state.position_ms)),
                // The listener changed the queue, so it's theirs whatever the server holds.
                Some((first, _)) if *first != fp => self.release(),
                _ => {}
            }
        }
        let changed = self.inner.sent.lock().unwrap().as_ref().is_none_or(|s| s.fingerprint != fp);
        *self.inner.pending.lock().unwrap() = Some(state);
        if changed && !self.inner.scheduled.swap(true, Ordering::AcqRel) {
            let this = self.clone();
            self.inner.runtime.spawn(async move {
                tokio::time::sleep(SAVE_AFTER).await;
                this.inner.scheduled.store(false, Ordering::Release);
                let _ = this.flush().await;
            });
        }
    }

    /// [`Self::note`] for an app that holds the queue as library row ids.
    /// Rows are only looked up when they change, so calling this often is cheap.
    pub fn note_rows(&self, rowids: &[u32], index: usize, position_ms: u64) {
        if rowids.is_empty() {
            return;
        }
        let (rows, index) = window(rowids, index, MAX_SAVED_FORM);
        let mut cache = self.inner.rows.lock().unwrap();
        if cache.0 != rows {
            let found: std::collections::HashMap<u32, String> = self.inner.library.songs_by_rowid(rows).unwrap_or_default().into_iter().map(|(r, s)| (r, s.id)).collect();
            *cache = (rows.to_vec(), rows.iter().map(|r| found.get(r).cloned()).collect());
        }
        let mut ids = Vec::with_capacity(rows.len());
        let mut current = 0;
        for (i, id) in cache.1.iter().enumerate() {
            if i == index {
                current = ids.len();
            }
            if let Some(id) = id {
                ids.push(id.clone());
            }
        }
        drop(cache);
        if !ids.is_empty() {
            let index = current.min(ids.len() - 1);
            self.note(QueueState { song_ids: ids, index, position_ms });
        }
    }

    /// Saves the latest queue now if it differs from what was last saved, for pause and quit.
    pub async fn flush(&self) -> Result<()> {
        // One save at a time, so a delayed save and a pause can't both send the same queue.
        let _one = self.inner.flushing.lock().await;
        let Some(state) = self.inner.pending.lock().unwrap().clone() else { return Ok(()) };
        let fp = state.fingerprint();
        self.note_listening(&fp, state.position_ms);
        if self.inner.held.load(Ordering::Acquire) {
            return Ok(());
        }
        let same = self.inner.sent.lock().unwrap().as_ref().is_some_and(|s| s.fingerprint == fp && s.position_ms.abs_diff(state.position_ms) < POSITION_SLACK_MS);
        if same {
            return Ok(());
        }
        let Some(features) = self.features() else { return Ok(()) };
        if self.inner.reporter.is_private() {
            return Ok(());
        }
        let result = if features.index_queue {
            let max = if features.form_post { MAX_SAVED_FORM } else { MAX_SAVED_URL };
            let (ids, index) = window(&state.song_ids, state.index, max);
            self.inner.client.save_play_queue_by_index(ids, index, state.position_ms, features.form_post).await
        } else {
            let (ids, index) = window(&state.song_ids, state.index, MAX_SAVED_URL);
            self.inner.client.save_play_queue(ids, ids.get(index).map(String::as_str), Some(state.position_ms)).await
        };
        match result {
            Ok(()) => {
                *self.inner.sent.lock().unwrap() = Some(Sent { fingerprint: fp.clone(), position_ms: state.position_ms });
                let (ids, index) = window(&state.song_ids, state.index, if features.index_queue && features.form_post { MAX_SAVED_FORM } else { MAX_SAVED_URL });
                let _ = self.inner.library.set_meta(META_SAVED, Some(&fingerprint(ids, index)));
                Ok(())
            }
            Err(Error::WritesDisabled { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// The server's queue, when it's worth offering. Read-only.
    /// It must come from another client, be newer than the local queue and not have been turned down.
    pub async fn remote_offer(&self) -> Result<Option<RemoteQueue>> {
        let found = self.fetch_offer().await;
        // With nothing to offer or no answer, this device's queue is the one to keep saving.
        if !matches!(found, Ok(Some(_))) {
            self.release();
        }
        found
    }

    async fn fetch_offer(&self) -> Result<Option<RemoteQueue>> {
        let mut waited = Duration::ZERO;
        while self.features().is_none() && waited < FEATURES_WAIT {
            tokio::time::sleep(Duration::from_millis(250)).await;
            waited += Duration::from_millis(250);
        }
        // Give the app a moment to restore its own queue, so the two can be compared.
        let mut waited = Duration::ZERO;
        while self.inner.pending.lock().unwrap().is_none() && waited < RESTORE_WAIT {
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += Duration::from_millis(100);
        }
        let Some(features) = self.features() else { return Ok(None) };
        let remote = if features.index_queue {
            let Some(q) = self.inner.client.play_queue_by_index().await? else { return Ok(None) };
            let index = q.current_index.unwrap_or(0).max(0) as usize;
            (q.entry, index, q.position, q.changed, q.changed_by)
        } else {
            let Some(q) = self.inner.client.play_queue().await? else { return Ok(None) };
            let index = q.current.as_deref().and_then(|c| q.entry.iter().position(|s| s.id == c)).unwrap_or(0);
            (q.entry, index, q.position, q.changed, q.changed_by)
        };
        let (songs, index, position, changed, changed_by) = remote;
        if songs.is_empty() || index >= songs.len() {
            return Ok(None);
        }
        let changed = changed.unwrap_or_default();
        let Some(changed_ms) = parse_iso8601_ms(&changed) else { return Ok(None) };
        let ids: Vec<String> = songs.iter().map(|s| s.id.clone()).collect();
        let fp = fingerprint(&ids, index);
        let lib = &self.inner.library;
        let (local_ms, local_fp) = match lib.meta(META_LOCAL)?.and_then(|v| v.split_once(' ').map(|(a, b)| (a.parse::<u64>().unwrap_or(0), b.to_string()))) {
            Some((ms, fp)) => (ms, Some(fp)),
            None => (0, None),
        };
        let ours = lib.meta(META_SAVED)?.as_deref() == Some(fp.as_str());
        let here = self.inner.pending.lock().unwrap().as_ref().map(QueueState::fingerprint);
        let same_as_here = local_fp.as_deref() == Some(fp.as_str()) || here.as_deref() == Some(fp.as_str());
        let dismissed = lib.meta(META_DISMISSED)?.as_deref() == Some(changed.as_str());
        if ours || same_as_here || dismissed || changed_ms <= local_ms {
            return Ok(None);
        }
        Ok(Some(RemoteQueue { changed_by: changed_by.unwrap_or_default(), changed_ms, songs, index, position_ms: position.unwrap_or(0).max(0) as u64, changed }))
    }

    /// Records the listener's answer to an offer, so it isn't offered again, and starts saving.
    pub fn answered(&self, offer: &RemoteQueue) {
        let _ = self.inner.library.set_meta(META_DISMISSED, Some(&offer.changed));
        let ids: Vec<String> = offer.songs.iter().map(|s| s.id.clone()).collect();
        *self.inner.sent.lock().unwrap() = Some(Sent { fingerprint: fingerprint(&ids, offer.index), position_ms: offer.position_ms });
        self.release();
    }

    /// Lets saving start once there's no offer, it's answered, or the listener took over the local queue.
    fn release(&self) {
        if self.inner.held.swap(false, Ordering::AcqRel) {
            let this = self.clone();
            self.inner.runtime.spawn(async move {
                let _ = this.flush().await;
            });
        }
    }

    /// Listening to this device's queue makes it the newest session and the listener's choice over any offer.
    fn note_listening(&self, fp: &str, position_ms: u64) {
        let moved = self.inner.launch.lock().unwrap().as_ref().is_some_and(|(first, at)| first == fp && at.abs_diff(position_ms) > LISTENED_MS);
        if moved {
            let _ = self.inner.library.set_meta(META_LOCAL, Some(&format!("{} {fp}", now_ms())));
            *self.inner.launch.lock().unwrap() = Some((fp.to_string(), position_ms));
            self.release();
        }
    }
}

/// At most `max` ids around `index`, a few before it and the rest after.
fn window<T>(ids: &[T], index: usize, max: usize) -> (&[T], usize) {
    if ids.len() <= max {
        return (ids, index.min(ids.len().saturating_sub(1)));
    }
    let index = index.min(ids.len() - 1);
    let start = index.saturating_sub(KEEP_BEFORE).min(ids.len() - max);
    (&ids[start..start + max], index - start)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Parses an RFC 3339 timestamp as Unix ms.
/// `None` for anything else and for Go's zero time, which Navidrome sends for an empty queue.
pub fn parse_iso8601_ms(text: &str) -> Option<u64> {
    let t = text.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = t.split_once('T')?;
    let mut d = date.split('-');
    let (y, mo, da) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    if y < 1970 {
        return None;
    }
    let (clock, offset_s) = if let Some(c) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
        (c, 0)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (c, off) = rest.split_at(at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':').unwrap_or((&off[1..3.min(off.len())], off.get(3..).unwrap_or("0")));
        (c, sign * (num(oh)? * 3600 + num(om).unwrap_or(0) * 60))
    };
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, ""));
    let mut h = hms.split(':');
    let (hh, mm, ss) = (num(h.next()?)?, num(h.next()?)?, num(h.next().unwrap_or("0"))?);
    let ms = frac.chars().chain("000".chars()).take(3).collect::<String>().parse::<i64>().unwrap_or(0);
    // Days from the civil date (Howard Hinnant's algorithm).
    let (yy, m) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * m + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3600 + mm * 60 + ss - offset_s;
    u64::try_from(secs * 1000 + ms).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_times() {
        assert_eq!(parse_iso8601_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601_ms("2023-03-10T02:19:35.784818075Z"), Some(1_678_414_775_784));
        assert_eq!(parse_iso8601_ms("2023-03-10T03:19:35.5+01:00"), Some(1_678_414_775_500));
        assert_eq!(parse_iso8601_ms("0001-01-01T00:00:00Z"), None);
        assert_eq!(parse_iso8601_ms("yesterday"), None);
    }

    #[test]
    fn long_queues_keep_the_part_around_the_current_song() {
        let ids: Vec<String> = (0..5000).map(|i| i.to_string()).collect();
        let (w, i) = window(&ids, 3000, 1000);
        assert_eq!((w.len(), i, w[i].as_str()), (1000, KEEP_BEFORE, "3000"));
        let (w, i) = window(&ids, 4990, 1000);
        assert_eq!((w.len(), w[i].as_str()), (1000, "4990"));
        let (w, i) = window(&ids[..10], 4, 1000);
        assert_eq!((w.len(), i), (10, 4));
    }
}
