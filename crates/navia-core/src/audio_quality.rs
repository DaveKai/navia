//! What a song's file is, for the player's quality badge. Fields the server leaves out are skipped.

use crate::client::Song;
use crate::quality::{Codec, Delivered};

/// The original file, as far as the server says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// File suffix as the server reports it, any case. Empty when unknown.
    pub suffix: String,
    pub kbps: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

/// Lossless above CD and DVD-video quality, lossless, or lossy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    HiRes,
    Lossless,
    Lossy,
}

const LOSSLESS: [&str; 7] = ["flac", "alac", "wav", "aiff", "aif", "ape", "wv"];

fn positive(v: Option<i32>) -> Option<u32> {
    v.filter(|&v| v > 0).map(|v| v as u32)
}

impl FileFacts {
    /// `None` when the server didn't say what kind of file it is.
    pub fn from_song(song: &Song) -> Option<FileFacts> {
        let suffix = song.suffix.as_deref().map(str::trim).filter(|s| !s.is_empty())?.to_string();
        Some(FileFacts { suffix, kbps: positive(song.bit_rate), bit_depth: positive(song.bit_depth), sample_rate: positive(song.sampling_rate), channels: positive(song.channel_count) })
    }

    /// The format's short name. Above 500 kbps an M4A must be ALAC, since AAC tops out well below that.
    pub fn format_name(&self) -> String {
        let s = self.suffix.to_ascii_lowercase();
        let name = match s.as_str() {
            "m4a" if self.kbps.unwrap_or(0) > 500 => "ALAC",
            "flac" => "FLAC",
            "alac" => "ALAC",
            "wav" => "WAV",
            "aiff" | "aif" => "AIFF",
            "ape" => "Monkey's Audio",
            "wv" => "WavPack",
            "mp3" => "MP3",
            "aac" | "m4a" | "mp4" => "AAC",
            "opus" => "Opus",
            "ogg" | "oga" => "Vorbis",
            "wma" => "WMA",
            _ => return self.suffix.to_ascii_uppercase(),
        };
        name.to_string()
    }

    pub fn is_lossless(&self) -> bool {
        self.format_name() == "ALAC" || LOSSLESS.contains(&self.suffix.to_ascii_lowercase().as_str())
    }

    /// Above CD and DVD-video quality: more than 16 bits or more than 48 kHz. Unknown values don't count.
    pub fn is_hi_res(&self) -> bool {
        self.is_lossless() && (self.bit_depth.unwrap_or(0) > 16 || self.sample_rate.unwrap_or(0) > 48_000)
    }

    pub fn tier(&self) -> Tier {
        if self.is_hi_res() {
            Tier::HiRes
        } else if self.is_lossless() {
            Tier::Lossless
        } else {
            Tier::Lossy
        }
    }
}

/// Where the playing bytes come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Stream,
    Cache,
    Download,
}

/// What the player is decoding: where from, and the server's conversion when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub source: Source,
    /// The server's transcode format and kbps, or `None` for the original file.
    pub transcode: Option<(Codec, u32)>,
}

impl Delivery {
    pub fn of(delivered: &Delivered) -> Delivery {
        match delivered {
            Delivered::Original | Delivered::Fallback { .. } => Delivery { source: Source::Stream, transcode: None },
            Delivered::Transcoded { codec, kbps } => Delivery { source: Source::Stream, transcode: Some((*codec, *kbps)) },
            Delivered::Cached { quality } => Delivery { source: Source::Cache, transcode: *quality },
            Delivered::Downloaded { quality } => Delivery { source: Source::Download, transcode: *quality },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(suffix: &str, kbps: u32, depth: u32, rate: u32) -> FileFacts {
        let opt = |v: u32| (v > 0).then_some(v);
        FileFacts { suffix: suffix.into(), kbps: opt(kbps), bit_depth: opt(depth), sample_rate: opt(rate), channels: Some(2) }
    }

    #[test]
    fn tiers() {
        assert_eq!(facts("flac", 1011, 16, 44_100).tier(), Tier::Lossless);
        assert_eq!(facts("FLAC", 2304, 24, 96_000).tier(), Tier::HiRes);
        assert_eq!(facts("flac", 2304, 0, 96_000).tier(), Tier::HiRes);
        assert_eq!(facts("flac", 912, 0, 44_100).tier(), Tier::Lossless);
        assert_eq!(facts("mp3", 320, 0, 44_100).tier(), Tier::Lossy);
        // A lossy file at a high rate is still lossy.
        assert_eq!(facts("opus", 510, 0, 96_000).tier(), Tier::Lossy);
    }

    #[test]
    fn m4a_holds_aac_or_alac() {
        assert_eq!(facts("m4a", 256, 0, 44_100).format_name(), "AAC");
        assert_eq!(facts("m4a", 256, 0, 44_100).tier(), Tier::Lossy);
        assert_eq!(facts("m4a", 900, 16, 44_100).format_name(), "ALAC");
        assert_eq!(facts("m4a", 900, 16, 44_100).tier(), Tier::Lossless);
        assert_eq!(facts("xyz", 0, 0, 0).format_name(), "XYZ");
    }

    #[test]
    fn unknown_suffix_and_zeroes() {
        let song = Song { suffix: Some("flac".into()), bit_rate: Some(0), bit_depth: Some(24), sampling_rate: Some(0), channel_count: None, ..Default::default() };
        let f = FileFacts::from_song(&song).expect("facts");
        assert_eq!((f.kbps, f.bit_depth, f.sample_rate, f.channels), (None, Some(24), None, None));
        assert!(FileFacts::from_song(&Song { suffix: Some(" ".into()), ..Default::default() }).is_none());
    }

    #[test]
    fn deliveries() {
        assert_eq!(Delivery::of(&Delivered::Original), Delivery { source: Source::Stream, transcode: None });
        assert_eq!(Delivery::of(&Delivered::Fallback { wanted: Codec::Opus, kbps: 128, reason: String::new() }).transcode, None);
        assert_eq!(Delivery::of(&Delivered::Transcoded { codec: Codec::Mp3, kbps: 192 }).transcode, Some((Codec::Mp3, 192)));
        assert_eq!(Delivery::of(&Delivered::Cached { quality: None }).source, Source::Cache);
        assert_eq!(Delivery::of(&Delivered::Downloaded { quality: Some((Codec::Aac, 256)) }), Delivery { source: Source::Download, transcode: Some((Codec::Aac, 256)) });
    }
}
