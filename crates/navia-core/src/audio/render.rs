//! The output side of the engine, which pulls samples for a device callback or a capture.
//! Nothing here blocks or allocates. The decode side shares a sample ring and a marker ring
//! with it, and a flush bumps an epoch so stale frames and markers are dropped without locks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ringbuf::HeapCons;
use ringbuf::traits::{Consumer as _, Observer as _};

use crate::audio::processing::{EqSettings, EqTarget, Equalizer};

/// Frames over which a pause, resume or volume change ramps, to avoid clicks.
const RAMP_FRAMES: f32 = 256.0;
/// Frames to have buffered before playing after a start, seek or skip.
const PREBUFFER_FRAMES: usize = 8192;
/// How many (output frame, read frame) pairs are kept to map output back to what is heard.
/// One is taken per render call.
const HISTORY: usize = 1024;
/// Markers that can wait to be heard. Past this many, they apply at once.
const DEFERRED: usize = 64;

#[derive(Clone, Copy, Debug)]
pub(crate) enum MarkerKind {
    /// `run` counts plays from the decoder. A seek keeps it and a replay of the same entry doesn't.
    TrackStart { token: usize, base_ms: u64, rate: u32, run: u64 },
    RateChange { rate: u32 },
    QueueEnd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Marker {
    pub frame: u64,
    pub epoch: u64,
    pub kind: MarkerKind,
}

/// State shared between the decode side, the output side and whoever observes playback.
pub struct RenderShared {
    pub(crate) epoch: AtomicU64,
    pub(crate) flush_until: AtomicU64,
    /// Where a flush starts dropping frames, or [`NO_CUT`] to drop everything unheard.
    pub(crate) cut_from: AtomicU64,
    /// Frames consumed from the ring, including ones skipped by a flush.
    pub read_frames: AtomicU64,
    /// Frames actually delivered to the output.
    pub heard_frames: AtomicU64,
    pub paused: AtomicBool,
    volume_bits: AtomicU32,
    pub track_token: AtomicUsize,
    pub track_start_frame: AtomicU64,
    pub track_base_ms: AtomicU64,
    pub track_rate: AtomicU32,
    /// Bumped when a track start is reached, so observers notice a seek within the same track.
    pub track_generation: AtomicU64,
    /// The decoder's play count for the track heard. A new value at the same token is a replay.
    pub track_run: AtomicU64,
    /// Frames of silence caused by the decoder not keeping up, excluding seeks and skips.
    pub underrun_frames: AtomicU64,
    /// Frames of silence while waiting for audio after a seek or skip.
    pub rebuffer_frames: AtomicU64,
    /// Non-zero when the output must reopen at this rate before more audio can play.
    pub rate_request: AtomicU32,
    /// Silence frames written while waiting for that reopen.
    pub drain_frames: AtomicU64,
    pub queue_ended: AtomicBool,
    /// Asks the output to wait for a cushion before playing again after an outage.
    pub rebuffer_request: AtomicBool,
    /// Where a pending load or seek will start, shown until its first audio arrives.
    pub held_position: AtomicU64,
    /// Frames between what the output takes and what is heard, such as an AirPlay receiver's
    /// buffer. 0 for local devices and captures.
    pub output_latency: AtomicU64,
    /// The read frame being heard now.
    pub heard_read: AtomicU64,
    /// Bumped by a flush that drops audio, so an output that buffers far ahead drops what it sent.
    pub discontinuity: AtomicU64,
    /// The equalizer settings for the output, swapped in without blocking it.
    eq_target: Mutex<EqTarget>,
    eq_generation: AtomicU64,
}

pub const NO_TRACK: usize = usize::MAX;
/// `held_position` when nothing is pending.
pub const NO_HELD_POSITION: u64 = u64::MAX;
/// `cut_from` for an ordinary flush.
pub(crate) const NO_CUT: u64 = u64::MAX;

impl RenderShared {
    fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            flush_until: AtomicU64::new(0),
            cut_from: AtomicU64::new(NO_CUT),
            read_frames: AtomicU64::new(0),
            heard_frames: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            track_token: AtomicUsize::new(NO_TRACK),
            track_start_frame: AtomicU64::new(0),
            track_base_ms: AtomicU64::new(0),
            track_rate: AtomicU32::new(0),
            track_generation: AtomicU64::new(0),
            track_run: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            rebuffer_frames: AtomicU64::new(0),
            rate_request: AtomicU32::new(0),
            drain_frames: AtomicU64::new(0),
            queue_ended: AtomicBool::new(false),
            rebuffer_request: AtomicBool::new(false),
            held_position: AtomicU64::new(NO_HELD_POSITION),
            output_latency: AtomicU64::new(0),
            heard_read: AtomicU64::new(0),
            discontinuity: AtomicU64::new(0),
            eq_target: Mutex::new(EqTarget::off()),
            eq_generation: AtomicU64::new(0),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume_bits.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Set the equalizer, or None for unprocessed. Applies at once without clicks.
    pub fn set_eq(&self, settings: Option<&EqSettings>) {
        let target = EqTarget::from_settings(settings);
        let mut slot = self.eq_target.lock().expect("eq target");
        *slot = target;
        self.eq_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    /// Current position within the playing track, in milliseconds.
    pub fn position_ms(&self) -> u64 {
        let held = self.held_position.load(Ordering::Acquire);
        if held != NO_HELD_POSITION {
            return held;
        }
        let rate = u64::from(self.track_rate.load(Ordering::Acquire));
        if rate == 0 {
            return 0;
        }
        let base = self.track_base_ms.load(Ordering::Acquire);
        let start = self.track_start_frame.load(Ordering::Acquire);
        let read = if self.output_latency.load(Ordering::Acquire) == 0 { self.read_frames.load(Ordering::Acquire) } else { self.heard_read.load(Ordering::Acquire) };
        base + read.saturating_sub(start) * 1000 / rate
    }
}

/// Pull side of the engine. Hold it inside the audio callback.
pub struct RenderCore {
    ring: HeapCons<f32>,
    markers: HeapCons<Marker>,
    shared: Arc<RenderShared>,
    local_epoch: u64,
    read: u64,
    pending: Option<Marker>,
    stream_rate: u32,
    waiting: bool,
    gain: f32,
    active: bool,
    rebuffering: bool,
    eq: Equalizer,
    eq_seen: u64,
    /// A cut still ahead: play up to the first frame, then skip to the second.
    cut: Option<(u64, u64)>,
    /// Frames handed to the output, silence included.
    out: u64,
    /// Recent (output frame, read frame) pairs, oldest first once full.
    history: Box<[(u64, u64)]>,
    hist_len: usize,
    hist_next: usize,
    /// Track starts and queue ends read but not heard yet.
    deferred: VecDeque<Marker>,
}

impl RenderCore {
    pub(crate) fn new(ring: HeapCons<f32>, markers: HeapCons<Marker>) -> (Self, Arc<RenderShared>) {
        let shared = Arc::new(RenderShared::new());
        let core = Self {
            ring,
            markers,
            shared: shared.clone(),
            local_epoch: 0,
            read: 0,
            pending: None,
            stream_rate: 0,
            waiting: false,
            gain: 1.0,
            active: false,
            rebuffering: true,
            eq: Equalizer::new(),
            eq_seen: 0,
            cut: None,
            out: 0,
            history: vec![(0, 0); HISTORY].into_boxed_slice(),
            hist_len: 0,
            hist_next: 0,
            deferred: VecDeque::with_capacity(DEFERRED),
        };
        (core, shared)
    }

    pub fn shared(&self) -> &Arc<RenderShared> {
        &self.shared
    }

    /// Set the stream rate after opening or reopening a device.
    pub fn set_stream_rate(&mut self, rate: u32) {
        self.stream_rate = rate;
        self.waiting = false;
        self.sync_eq();
        self.eq.configure(rate);
    }

    /// Pick up new equalizer settings if any were published. Never blocks.
    fn sync_eq(&mut self) {
        let generation = self.shared.eq_generation.load(Ordering::Acquire);
        if generation == self.eq_seen {
            return;
        }
        if let Ok(target) = self.shared.eq_target.try_lock() {
            let target = *target;
            self.eq.set_target(target);
            self.eq_seen = generation;
        }
    }

    pub fn stream_rate(&self) -> u32 {
        self.stream_rate
    }

    /// Discard everything before the latest flush. Also used when no device is running.
    pub(crate) fn handle_flush(&mut self) {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if epoch == self.local_epoch {
            return;
        }
        self.local_epoch = epoch;
        self.pending = None;
        // Held events won't be heard now, but they still fire.
        while let Some(m) = self.deferred.pop_front() {
            self.observe(m);
        }
        let until = self.shared.flush_until.load(Ordering::Acquire);
        let from = self.shared.cut_from.load(Ordering::Acquire);
        self.cut = None;
        if from != NO_CUT && from > self.read {
            // The song being heard plays to its end. Markers from before the cut belong to the
            // dropped audio, and the next song's arrive with the new epoch.
            if from < until {
                self.cut = Some((from, until));
            }
            return;
        }
        self.shared.discontinuity.fetch_add(1, Ordering::AcqRel);
        self.hist_len = 0;
        let stale = until.saturating_sub(self.read);
        let dropped = self.ring.skip((stale * 2) as usize) as u64 / 2;
        self.read += dropped;
        self.shared.read_frames.store(self.read, Ordering::Release);
        self.waiting = false;
        self.shared.rate_request.store(0, Ordering::Release);
        self.rebuffering = true;
    }

    fn poll_markers(&mut self) {
        loop {
            if self.pending.is_none() {
                self.pending = self.markers.try_pop();
            }
            let Some(m) = self.pending else { return };
            if m.epoch < self.local_epoch {
                self.pending = None;
                continue;
            }
            if m.epoch > self.local_epoch || m.frame > self.read {
                return;
            }
            self.pending = None;
            self.apply(m);
            if self.waiting {
                return;
            }
        }
    }

    fn apply(&mut self, marker: Marker) {
        match marker.kind {
            MarkerKind::TrackStart { .. } => {
                self.active = true;
                self.defer(marker);
            }
            MarkerKind::RateChange { rate } => {
                if rate != self.stream_rate {
                    self.waiting = true;
                    self.shared.drain_frames.store(0, Ordering::Release);
                    self.shared.rate_request.store(rate, Ordering::Release);
                }
            }
            MarkerKind::QueueEnd => {
                self.active = false;
                // Waiting for audio from idle isn't an underrun.
                self.rebuffering = true;
                self.defer(marker);
            }
        }
    }

    /// Hold a marker until its frame is heard, or apply it now if nothing is held.
    fn defer(&mut self, marker: Marker) {
        if self.shared.output_latency.load(Ordering::Acquire) == 0 || self.deferred.len() >= DEFERRED {
            self.observe(marker);
        } else {
            self.deferred.push_back(marker);
        }
    }

    /// Publish a marker to observers.
    fn observe(&mut self, marker: Marker) {
        match marker.kind {
            MarkerKind::TrackStart { token, base_ms, rate, run } => {
                self.shared.track_token.store(token, Ordering::Release);
                self.shared.track_run.store(run, Ordering::Release);
                self.shared.track_start_frame.store(marker.frame, Ordering::Release);
                self.shared.track_base_ms.store(base_ms, Ordering::Release);
                self.shared.track_rate.store(rate, Ordering::Release);
                self.shared.track_generation.fetch_add(1, Ordering::AcqRel);
                self.shared.held_position.store(NO_HELD_POSITION, Ordering::Release);
                self.shared.queue_ended.store(false, Ordering::Release);
            }
            MarkerKind::QueueEnd => self.shared.queue_ended.store(true, Ordering::Release),
            MarkerKind::RateChange { .. } => {}
        }
    }

    /// Record where output and reading stand, and release the markers that are now heard.
    fn track_heard(&mut self, out_before: u64, read_before: u64) {
        self.push_history(out_before, read_before);
        self.update_heard();
    }

    fn push_history(&mut self, out: u64, read: u64) {
        if self.hist_len > 0 {
            let last = (self.hist_next + HISTORY - 1) % HISTORY;
            if self.history[last] == (out, read) {
                return;
            }
            if self.history[last].0 == out {
                self.history[last] = (out, read);
                return;
            }
        }
        self.history[self.hist_next] = (out, read);
        self.hist_next = (self.hist_next + 1) % HISTORY;
        self.hist_len = (self.hist_len + 1).min(HISTORY);
    }

    /// The read frame that was at output frame `at`.
    fn read_at(&self, at: u64) -> u64 {
        let mut best: Option<(u64, u64)> = None;
        let mut next_after: Option<(u64, u64)> = None;
        for i in 0..self.hist_len {
            let e = self.history[(self.hist_next + HISTORY - self.hist_len + i) % HISTORY];
            if e.0 <= at {
                best = Some(e);
            } else {
                next_after = Some(e);
                break;
            }
        }
        match best {
            // Reading advanced frame for frame within a span, capped by the next sample's read
            // (which can be lower after a replay) and by how far output went.
            Some((o, r)) => {
                let span = match next_after {
                    Some((no, nr)) if nr >= r => (nr - r).min(no - o),
                    Some((no, _)) => no - o,
                    None => self.read.saturating_sub(r),
                };
                r + (at - o).min(span)
            }
            // Older than anything kept, so use the oldest known point.
            None => self.history.get((self.hist_next + HISTORY - self.hist_len) % HISTORY).map(|e| e.1).unwrap_or(0).min(self.read),
        }
    }

    /// The output dropped its last `frames` unheard and will send them again via
    /// [`Self::replayed`]. An AirPlay pause does this.
    pub fn begin_replay(&mut self, frames: u64) {
        let cut = self.out.saturating_sub(frames);
        let from = self.read_at(cut);
        // Forget what went out after `cut`, since it was dropped unheard.
        while self.hist_len > 0 && self.history[(self.hist_next + HISTORY - 1) % HISTORY].0 > cut {
            self.hist_next = (self.hist_next + HISTORY - 1) % HISTORY;
            self.hist_len -= 1;
        }
        self.push_history(cut, from);
        self.push_history(self.out, from);
        self.update_heard();
    }

    /// The output resent `frames` from what it kept, without reading.
    pub fn replayed(&mut self, frames: u64) {
        self.out += frames;
        self.update_heard();
    }

    fn update_heard(&mut self) {
        let latency = self.shared.output_latency.load(Ordering::Acquire);
        if latency > 0 && self.out < latency {
            // Nothing has reached the listener yet.
            return;
        }
        let heard = if latency == 0 { self.read } else { self.read_at(self.out - latency) };
        self.shared.heard_read.store(heard, Ordering::Release);
        while let Some(m) = self.deferred.front()
            && m.frame <= heard
        {
            let m = self.deferred.pop_front().expect("front");
            self.observe(m);
        }
    }

    /// Fill `out` with interleaved stereo and return the frames taken from the ring.
    /// The rest of `out` is silence.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        self.handle_flush();
        let (out_before, read_before) = (self.out, self.read);
        self.sync_eq();
        if self.shared.rebuffer_request.swap(false, Ordering::AcqRel) {
            self.rebuffering = true;
        }
        let want = out.len() / 2;
        let paused = self.shared.paused.load(Ordering::Acquire);
        let target = if paused { 0.0 } else { self.shared.volume() };
        if paused && !self.active {
            // Nothing heard yet, so start silent instead of ramping down.
            self.gain = 0.0;
        }
        let mut filled = 0;
        while filled < want {
            self.poll_markers();
            if self.waiting || (paused && self.gain <= 0.0) {
                break;
            }
            // Wait for a small cushion after a start, seek or skip so playback doesn't starve.
            // A queued marker means no more audio is coming, so very short tracks still play.
            if self.rebuffering && self.ring.occupied_len() / 2 < PREBUFFER_FRAMES && self.markers.occupied_len() == 0 && self.pending.is_none() {
                break;
            }
            if let Some((from, until)) = self.cut
                && self.read >= from
            {
                let dropped = self.ring.skip((until.saturating_sub(self.read) * 2) as usize) as u64 / 2;
                self.read += dropped;
                self.shared.read_frames.store(self.read, Ordering::Release);
                if self.read >= until {
                    self.cut = None;
                } else if dropped == 0 {
                    break;
                }
                continue;
            }
            // Stop at the next marker so it applies at its exact frame.
            let mut room = want - filled;
            if let Some((from, _)) = self.cut {
                room = room.min(from.saturating_sub(self.read) as usize);
            }
            if let Some(m) = self.pending
                && m.epoch == self.local_epoch
            {
                room = room.min(m.frame.saturating_sub(self.read) as usize);
            }
            if room == 0 {
                continue;
            }
            let span = &mut out[filled * 2..(filled + room) * 2];
            let got = self.ring.pop_slice(span) / 2;
            if got == 0 {
                break;
            }
            self.eq.process(&mut out[filled * 2..(filled + got) * 2]);
            apply_gain(&mut self.gain, target, &mut out[filled * 2..(filled + got) * 2]);
            filled += got;
            self.read += got as u64;
        }
        self.shared.read_frames.store(self.read, Ordering::Release);
        self.shared.heard_frames.fetch_add(filled as u64, Ordering::AcqRel);
        out[filled * 2..].fill(0.0);
        if filled < want {
            let short = (want - filled) as u64;
            if self.waiting {
                self.shared.drain_frames.fetch_add(short, Ordering::AcqRel);
            } else if !paused && self.active {
                if self.rebuffering {
                    self.shared.rebuffer_frames.fetch_add(short, Ordering::AcqRel);
                } else {
                    self.shared.underrun_frames.fetch_add(short, Ordering::AcqRel);
                }
            }
        } else {
            self.rebuffering = false;
        }
        self.out += want as u64;
        self.track_heard(out_before, read_before);
        filled
    }

    /// Apply markers that are due without playing audio, for when no device is running.
    pub(crate) fn render_markers_only(&mut self, _scratch: &mut [f32]) {
        self.poll_markers();
        while let Some(m) = self.deferred.pop_front() {
            self.observe(m);
        }
        self.shared.heard_read.store(self.read, Ordering::Release);
    }

    /// Frames waiting in the ring.
    pub fn buffered_frames(&self) -> usize {
        self.ring.occupied_len() / 2
    }
}

/// Ramp `gain` toward `target` over the block. A steady gain of 1.0 leaves samples untouched,
/// which keeps unprocessed playback bit-exact.
fn apply_gain(gain: &mut f32, target: f32, samples: &mut [f32]) {
    if *gain == target {
        if target != 1.0 {
            samples.iter_mut().for_each(|s| *s *= target);
        }
        return;
    }
    let step = 1.0 / RAMP_FRAMES;
    for frame in samples.chunks_exact_mut(2) {
        if (*gain - target).abs() <= step {
            *gain = target;
        } else if *gain < target {
            *gain += step;
        } else {
            *gain -= step;
        }
        frame[0] *= *gain;
        frame[1] *= *gain;
    }
    if *gain == target && target == 1.0 {
        *gain = 1.0;
    }
}

#[cfg(test)]
mod latency_tests {
    use super::*;
    use ringbuf::HeapRb;
    use ringbuf::traits::{Producer as _, Split as _};

    fn core_with(frames: usize) -> (RenderCore, Arc<RenderShared>, ringbuf::HeapProd<f32>, ringbuf::HeapProd<Marker>) {
        let (mut prod, cons) = HeapRb::<f32>::new(frames * 2 + 1024).split();
        let (mprod, mcons) = HeapRb::<Marker>::new(16).split();
        let (mut core, shared) = RenderCore::new(cons, mcons);
        core.set_stream_rate(1000);
        let audio: Vec<f32> = (0..frames * 2).map(|i| (i % 7) as f32 * 0.01).collect();
        prod.push_slice(&audio);
        (core, shared, prod, mprod)
    }

    fn start(mprod: &mut ringbuf::HeapProd<Marker>, frame: u64, token: usize) {
        let _ = mprod.try_push(Marker { frame, epoch: 0, kind: MarkerKind::TrackStart { token, base_ms: 0, rate: 1000, run: 1 } });
    }

    /// With 2,000 frames of output latency, track starts and position follow what is heard.
    #[test]
    fn track_start_waits_until_heard() {
        let (mut core, shared, _p, mut m) = core_with(20_000);
        shared.output_latency.store(2_000, Ordering::Release);
        start(&mut m, 0, 1);
        start(&mut m, 9_000, 2);
        let mut block = vec![0.0f32; 1_000 * 2];
        for _ in 0..10 {
            core.render(&mut block); // read 10,000 frames
        }
        // Read past 9,000, but only 8,000 heard: still track 1, at 8 s (1,000 Hz).
        assert_eq!(shared.track_token.load(Ordering::Acquire), 1);
        assert_eq!(shared.position_ms(), 8_000);
        core.render(&mut block); // 11,000 read, 9,000 heard
        assert_eq!(shared.track_token.load(Ordering::Acquire), 2);
        assert_eq!(shared.position_ms(), 0);
        core.render(&mut block);
        assert_eq!(shared.position_ms(), 1_000);
    }

    /// Without latency, events and position follow the read frame.
    #[test]
    fn no_latency_is_unchanged() {
        let (mut core, shared, _p, mut m) = core_with(5_000);
        start(&mut m, 0, 1);
        start(&mut m, 2_500, 2);
        let mut block = vec![0.0f32; 1_000 * 2];
        core.render(&mut block);
        core.render(&mut block);
        core.render(&mut block);
        assert_eq!(shared.track_token.load(Ordering::Acquire), 2);
        assert_eq!(shared.position_ms(), 500);
    }

    /// A paused AirPlay output replays what the receiver dropped, and the position holds
    /// through the pause.
    #[test]
    fn rewind_and_replay_follow_the_receiver() {
        let (mut core, shared, _p, mut m) = core_with(20_000);
        shared.output_latency.store(2_000, Ordering::Release);
        start(&mut m, 0, 1);
        let mut block = vec![0.0f32; 1_000 * 2];
        for _ in 0..6 {
            core.render(&mut block); // 6,000 read and sent, 4,000 heard
        }
        assert_eq!(shared.position_ms(), 4_000);
        // Paused. The receiver dropped the unheard 2,000 and heard holds.
        assert_eq!(shared.position_ms(), 4_000);
        core.begin_replay(2_000);
        assert_eq!(shared.position_ms(), 4_000);
        core.replayed(1_000); // the receiver is still filling its buffer
        assert_eq!(shared.position_ms(), 4_000);
        core.replayed(1_000);
        assert_eq!(shared.position_ms(), 4_000);
        core.render(&mut block); // 7,000 read; heard: the first replayed 1,000
        assert_eq!(shared.position_ms(), 5_000);
        core.render(&mut block);
        assert_eq!(shared.position_ms(), 6_000);
        core.render(&mut block); // past the replay, into new reads
        assert_eq!(shared.position_ms(), 7_000);
    }

    /// A seek's flush applies held events at once, so none is lost.
    #[test]
    fn flush_releases_held_markers() {
        let (mut core, shared, _p, mut m) = core_with(20_000);
        shared.output_latency.store(5_000, Ordering::Release);
        start(&mut m, 0, 1);
        let mut block = vec![0.0f32; 1_000 * 2];
        core.render(&mut block);
        assert_eq!(shared.track_token.load(Ordering::Acquire), NO_TRACK);
        let before = shared.discontinuity.load(Ordering::Acquire);
        shared.flush_until.store(1_000, Ordering::Release);
        shared.epoch.fetch_add(1, Ordering::AcqRel);
        core.render(&mut block);
        assert_eq!(shared.track_token.load(Ordering::Acquire), 1);
        assert_eq!(shared.discontinuity.load(Ordering::Acquire), before + 1);
    }
}
