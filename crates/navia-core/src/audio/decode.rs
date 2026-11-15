//! Track decoding with Symphonia, and libopus for Opus. Produces interleaved stereo f32 blocks
//! with gapless trims applied, so joins are sample-accurate.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};
use symphonia::default::{get_codecs, get_probe};

#[cfg(feature = "opus")]
use crate::audio::opus::{MAX_FRAMES, OPUS_RATE, OpusDecoder};
use crate::audio::live::LiveShared;
use crate::audio::stream::HttpStream;
use crate::error::{Error, Result};

fn audio_err(context: &str, detail: impl std::fmt::Display) -> Error {
    Error::Audio(format!("{context}: {detail}"))
}

/// Where gapless trimming for this file comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaplessSource {
    /// The container or codec defines exact lengths (FLAC, WAV, ALAC with a sample table).
    Lossless,
    /// Delay and padding Symphonia read from the container, such as a LAME tag.
    CodecDelay,
    /// iTunSMPB parsed here, because Symphonia's MP4 reader ignores it.
    ITunSmpb,
    /// An MP4 edit list parsed here, for the same reason.
    EditList,
    /// Ogg Opus pre-skip and end trim, applied here because libopus doesn't.
    OpusPreSkip,
    /// No delay or padding information is available.
    Unknown,
}

/// An iTunSMPB tag: priming, padding and valid frames.
#[derive(Clone, Copy, Debug)]
pub struct ITunSmpb {
    pub priming: u64,
    #[allow(dead_code)] // Implied by the valid frame count. Kept for reporting.
    pub padding: u64,
    pub valid: u64,
}

impl ITunSmpb {
    pub fn parse(text: &str) -> Option<Self> {
        let f: Vec<&str> = text.split_whitespace().collect();
        if f.len() < 4 {
            return None;
        }
        Some(Self {
            priming: u64::from_str_radix(f[1], 16).ok()?,
            padding: u64::from_str_radix(f[2], 16).ok()?,
            valid: u64::from_str_radix(f[3], 16).ok()?,
        })
    }
}

/// Where a track's bytes come from.
#[derive(Clone, Debug)]
pub enum TrackSource {
    File(PathBuf),
    Stream(Arc<HttpStream>),
    Live(Arc<LiveShared>),
    /// A file received as a stream with no length, such as a cached transcode. It's probed as
    /// unseekable, like the stream was, so the audio is identical. Seeks work once it's open.
    Spooled(PathBuf),
}

/// A file that says it can't seek until the probe is done.
struct ProbeBlind {
    file: File,
    len: u64,
    seekable: Arc<std::sync::atomic::AtomicBool>,
}

impl std::io::Read for ProbeBlind {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl std::io::Seek for ProbeBlind {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl MediaSource for ProbeBlind {
    fn is_seekable(&self) -> bool {
        self.seekable.load(std::sync::atomic::Ordering::Acquire)
    }

    fn byte_len(&self) -> Option<u64> {
        self.is_seekable().then_some(self.len)
    }
}

fn stream_of(source: &TrackSource) -> Option<Arc<HttpStream>> {
    match source {
        TrackSource::Stream(stream) => Some(stream.clone()),
        _ => None,
    }
}

impl TrackSource {
    /// A new independent reader over the source.
    pub fn reader(&self) -> Result<Box<dyn MediaSource>> {
        match self {
            TrackSource::File(path) | TrackSource::Spooled(path) => Ok(Box::new(File::open(path).map_err(|e| audio_err("open", e))?)),
            TrackSource::Stream(stream) => Ok(Box::new(stream.reader())),
            TrackSource::Live(live) => live.take_reader(),
        }
    }
}

enum Backend {
    Symphonia(Box<dyn AudioDecoder>),
    #[cfg(feature = "opus")]
    Opus(OpusDecoder),
}

pub struct Decoder {
    /// The network stream being read, so a blocked read can be interrupted.
    stream: Option<Arc<HttpStream>>,
    reader: Box<dyn FormatReader>,
    backend: Backend,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: usize,
    pub codec: String,
    pub delay: Option<u32>,
    pub padding: Option<u32>,
    pub gapless: GaplessSource,
    pub smpb: Option<ITunSmpb>,
    /// Frames still to drop from the start of the stream.
    skip_frames: u64,
    /// True when the source can seek, through byte ranges or as a local file.
    pub seekable: bool,
    /// Frames of the decoded timeline trimmed before the first audible frame.
    head_trim: u64,
    /// Total audible frames when the file states them.
    total_frames: Option<u64>,
    time_base: Option<TimeBase>,
    /// After a seek, the timeline frame audio must resume at. Packets before it are dropped.
    seek_target: Option<u64>,
    /// Frames still to emit, when the file states an exact length.
    frames_left: Option<u64>,
    scratch: Vec<f32>,
    pub decode_errors: u64,
}

impl Decoder {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, true)
    }

    /// `apply_mp4_trim` applies MP4 gapless data. False gives raw decoder output.
    pub fn open_with(path: &Path, apply_mp4_trim: bool) -> Result<Self> {
        Self::open_source_with(&TrackSource::File(path.to_path_buf()), apply_mp4_trim)
    }

    /// Opens a file that arrived as a stream with no length, decoding it exactly as that stream.
    pub fn open_spooled(path: &Path) -> Result<Self> {
        Self::open_source_with(&TrackSource::Spooled(path.to_path_buf()), true)
    }

    /// Opens a local file or an HTTP stream.
    pub fn open_source(source: &TrackSource) -> Result<Self> {
        Self::open_source_with(source, true)
    }

    fn open_source_with(source: &TrackSource, apply_mp4_trim: bool) -> Result<Self> {
        let (media, unblind): (Box<dyn MediaSource>, _) = match source {
            TrackSource::Spooled(path) => {
                let file = File::open(path).map_err(|e| audio_err("open", e))?;
                let len = file.metadata().map_err(|e| audio_err("open", e))?.len();
                let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
                (Box::new(ProbeBlind { file, len, seekable: flag.clone() }), Some(flag))
            }
            _ => (source.reader()?, None),
        };
        let seekable = unblind.is_some() || media.is_seekable();
        let mss = MediaSourceStream::new(media, Default::default());
        // No extension hint on purpose, since libraries can hold MP3s named .flac.
        let reader = get_probe()
            .probe(&Hint::new(), mss, Default::default(), MetadataOptions::default())
            .map_err(|e| audio_err("probe", e))?;
        if let Some(flag) = unblind {
            flag.store(true, std::sync::atomic::Ordering::Release);
        }

        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| Error::Audio("no audio track".to_string()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| Error::Audio("no audio parameters".to_string()))?;

        let track_id = track.id;
        let delay = track.delay;
        let padding = track.padding;
        let time_base = track.time_base;
        let num_frames = track.num_frames;
        // Streams cut from a longer one start at a non-zero timestamp, and packet times include it.
        let start_ts = track.start_ts.get().max(0) as u64;
        let sample_rate = params.sample_rate.ok_or_else(|| Error::Audio("no sample rate".to_string()))?;
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2);

        if params.codec == CODEC_ID_OPUS {
            #[cfg(not(feature = "opus"))]
            return Err(Error::Audio("no decoder for this Opus file in this build".to_string()));
            #[cfg(feature = "opus")]
            let opus = OpusDecoder::new(channels)?;
            #[cfg(feature = "opus")]
            return Ok(Self {
                stream: stream_of(source),
                reader,
                backend: Backend::Opus(opus),
                track_id,
                sample_rate: OPUS_RATE,
                channels,
                codec: "opus".to_string(),
                delay,
                padding,
                gapless: GaplessSource::OpusPreSkip,
                smpb: None,
                skip_frames: delay.unwrap_or(0) as u64,
                seekable,
                head_trim: delay.unwrap_or(0) as u64 + start_ts,
                total_frames: num_frames.map(|n| n.saturating_sub(delay.unwrap_or(0) as u64)),
                time_base,
                seek_target: None,
                frames_left: None,
                scratch: vec![0.0; MAX_FRAMES * channels],
                decode_errors: 0,
            });
        }

        let opts = AudioDecoderOptions::default();
        let decoder = get_codecs().make_audio_decoder(params, &opts).map_err(|e| audio_err("decoder", e))?;
        let codec = decoder.codec_info().short_name.to_string();

        // Symphonia 0.6 reports neither edit lists nor iTunSMPB for MP4, so read them here.
        let mp4 = if codec == "aac" {
            source.reader().ok().and_then(|mut r| crate::audio::mp4::read_from(r.as_mut()))
        } else {
            None
        };
        let smpb = mp4.and_then(|m| m.smpb);
        let (gapless, skip_frames, frames_left) = match (mp4.and_then(|m| m.trim()), apply_mp4_trim, delay) {
            (Some((skip, left)), true, _) => {
                let source = if smpb.is_some() { GaplessSource::ITunSmpb } else { GaplessSource::EditList };
                (source, skip, left)
            }
            (_, _, Some(_)) => (GaplessSource::CodecDelay, 0, None),
            _ if matches!(codec.as_str(), "flac" | "alac") || codec.starts_with("pcm") => (GaplessSource::Lossless, 0, None),
            _ => (GaplessSource::Unknown, 0, None),
        };

        Ok(Self {
            stream: stream_of(source),
            reader,
            backend: Backend::Symphonia(decoder),
            track_id,
            sample_rate,
            channels,
            codec,
            delay,
            padding,
            gapless,
            smpb,
            skip_frames,
            seekable,
            head_trim: skip_frames,
            total_frames: match frames_left {
                Some(n) => Some(n),
                None => num_frames.map(|n| n.saturating_sub(skip_frames)),
            },
            time_base,
            seek_target: None,
            frames_left,
            scratch: Vec::new(),
            decode_errors: 0,
        })
    }

    /// Total audible frames at this decoder's sample rate, when the container says.
    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    /// Frames of decoder output the codec's state needs to settle after a seek.
    fn preroll_frames(&self) -> u64 {
        match self.codec.as_str() {
            "opus" => 3840,
            "aac" => 2048,
            "mp3" | "mp2" | "mp1" => 2304,
            "vorbis" => 8192,
            _ => 0,
        }
    }

    /// The HTTP stream behind this decoder, for interrupting a blocked read.
    pub(crate) fn stream(&self) -> Option<Arc<HttpStream>> {
        self.stream.clone()
    }

    pub fn seek_ms(&mut self, ms: u64) -> Result<()> {
        self.seek(ms * u64::from(self.sample_rate) / 1000)
    }

    /// Drops this much of the coming audio, to land inside the second a transcode started at.
    pub(crate) fn skip_ms(&mut self, ms: u64) {
        self.skip_frames += ms * u64::from(self.sample_rate) / 1000;
    }

    /// Treats this many leading frames as not music, such as a transcode's encoder delay.
    /// Unlike `skip_ms`, this also moves the timeline seeks land on.
    pub(crate) fn trim_head(&mut self, frames: u64) {
        self.skip_frames += frames;
        self.head_trim += frames;
    }

    /// Seeks so the next frame decoded is frame `target` of the trimmed timeline.
    /// It decodes and discards from just before the target, so the result is sample-accurate.
    pub fn seek(&mut self, target: u64) -> Result<()> {
        if !self.seekable {
            return Err(Error::Audio("this source can't seek".to_string()));
        }
        // MP4 and Opus containers run on the untrimmed timeline.
        let timeline_target = target + self.head_trim;
        let preroll = self.preroll_frames();
        let want = timeline_target.saturating_sub(preroll);
        let to = SeekTo::Timestamp { ts: self.ts_for_frame(want), track_id: self.track_id };
        // FLAC's seek reads frames, so it can block and be interrupted like any other read.
        let seeked = self.reader.seek(SeekMode::Accurate, to).map_err(|e| match e {
            SymError::IoError(ref io) if io.kind() == std::io::ErrorKind::ConnectionAborted => Error::Cancelled,
            SymError::IoError(ref io) if io.kind() == std::io::ErrorKind::NotConnected => Error::Network(io.to_string()),
            e => audio_err("seek", e),
        })?;
        match &mut self.backend {
            Backend::Symphonia(decoder) => decoder.reset(),
            #[cfg(feature = "opus")]
            Backend::Opus(opus) => opus.reset(),
        }
        let actual = self.frame_for_ts(seeked.actual_ts);
        // Drop everything decoded before the target, whether or not the codec counts frames from
        // the seek point. Vorbis emits nothing for its first packet.
        self.skip_frames = 0;
        self.seek_target = Some(timeline_target.max(actual));
        self.frames_left = self.total_frames.map(|t| t.saturating_sub(target)).filter(|_| self.frames_left.is_some());
        Ok(())
    }

    fn ts_for_frame(&self, frame: u64) -> Timestamp {
        Timestamp::new(self.frames_to_ts(frame) as i64)
    }

    fn frames_to_ts(&self, frame: u64) -> u64 {
        match self.time_base {
            Some(tb) => {
                // ticks = frames / rate * denom / numer
                let (n, d) = (tb.numer.get() as u128, tb.denom.get() as u128);
                ((frame as u128 * d) / (self.sample_rate as u128 * n)) as u64
            }
            None => frame,
        }
    }

    fn frame_for_ts(&self, ts: Timestamp) -> u64 {
        match self.time_base {
            Some(tb) => {
                let (n, d) = (tb.numer.get() as u128, tb.denom.get() as u128);
                ((ts.get().max(0) as u128 * n * self.sample_rate as u128) / d) as u64
            }
            None => ts.get().max(0) as u64,
        }
    }

    /// Decodes the next block into `out` as interleaved stereo. Returns false at end of stream.
    pub fn next_block(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        out.clear();
        loop {
            if self.frames_left == Some(0) {
                return Ok(false);
            }
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                // A canceled live stream. The engine has been told what to do next.
                Err(SymError::IoError(ref e)) if e.kind() == std::io::ErrorKind::ConnectionAborted => return Err(Error::Cancelled),
                // The server went away under a stream that can't wait in place.
                Err(SymError::IoError(ref e)) if e.kind() == std::io::ErrorKind::NotConnected => return Err(Error::Network(e.to_string())),
                Err(e) => return Err(audio_err("read packet", e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }

            let (start_trim, end_trim, decoded_frames) = match &mut self.backend {
                Backend::Symphonia(decoder) => match decoder.decode(&packet) {
                    Ok(buf) => {
                        self.channels = buf.spec().channels().count();
                        buf.copy_to_vec_interleaved(&mut self.scratch);
                        (0, 0, None)
                    }
                    Err(SymError::DecodeError(e)) => {
                        self.decode_errors += 1;
                        tracing::warn!("decode error at packet ts {:?}: {e}", packet.pts);
                        continue;
                    }
                    Err(e) => return Err(audio_err("decode", e)),
                },
                #[cfg(feature = "opus")]
                Backend::Opus(opus) => match opus.decode_float(&packet.data, &mut self.scratch) {
                    Ok(frames) => (packet.trim_start.get() as usize, packet.trim_end.get() as usize, Some(frames)),
                    Err(e) => {
                        self.decode_errors += 1;
                        tracing::warn!("opus decode error at packet ts {:?}: {e}", packet.pts);
                        continue;
                    }
                },
            };

            let ch = self.channels.max(1);
            let total = decoded_frames.unwrap_or(self.scratch.len() / ch);
            let mut frames = &self.scratch[..total * ch];

            // After a seek, resume exactly at the target using this packet's timestamp.
            if let Some(target) = self.seek_target {
                let pts = self.frame_for_ts(packet.pts);
                let packet_frames = frames.len() as u64 / ch as u64;
                if pts + packet_frames <= target {
                    continue;
                }
                let skip = target.saturating_sub(pts) as usize;
                frames = &frames[skip * ch..];
                self.seek_target = None;
                if frames.is_empty() {
                    continue;
                }
            }

            // Ogg Opus packets carry any end trim. The head trim is packet-level or the pre-skip,
            // and `skip_frames` counts whichever applies.
            if end_trim > 0 {
                let keep = frames.len() / ch - end_trim.min(frames.len() / ch);
                frames = &frames[..keep * ch];
            }
            if start_trim > 0 {
                let skip = start_trim.min(frames.len() / ch);
                frames = &frames[skip * ch..];
                self.skip_frames = self.skip_frames.saturating_sub(skip as u64);
            }

            let skip = (self.skip_frames as usize).min(frames.len() / ch);
            self.skip_frames -= skip as u64;
            frames = &frames[skip * ch..];
            if frames.is_empty() {
                continue;
            }
            if let Some(left) = &mut self.frames_left {
                let take = (*left as usize).min(frames.len() / ch);
                frames = &frames[..take * ch];
                *left -= take as u64;
            }
            append_stereo(frames, ch, out);
            return Ok(true);
        }
    }
}

fn append_stereo(input: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        2 => out.extend_from_slice(input),
        1 => input.iter().for_each(|&s| out.extend_from_slice(&[s, s])),
        n => input.chunks_exact(n).for_each(|f| out.extend_from_slice(&f[..2])),
    }
}
