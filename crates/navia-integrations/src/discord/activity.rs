//! Builds the activity object, keeping to Discord's field limits.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::StatusText;
use crate::TrackInfo;

const ACTIVITY_TYPE_LISTENING: u64 = 2;
/// Discord's docs no longer state these limits. The 2 to 128 range is what other Rich Presence clients work around, and
/// it hasn't been checked against a real Discord client here.
const MAX_TEXT_UNITS: usize = 128;
const MIN_TEXT_CHARS: usize = 2;
const MAX_IMAGE_LENGTH: usize = crate::art::MAX_IMAGE_URL_LEN;

/// What the status should show right now.
#[derive(Clone, Debug)]
pub struct Presence {
    pub track: TrackInfo,
    pub position: Duration,
    pub captured_at: SystemTime,
}

impl Presence {
    pub fn started_at_ms(&self) -> i64 {
        let captured = self
            .captured_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        captured - self.position.as_millis() as i64
    }
}

/// Fields that require a new activity when they change. Elapsed time is compared separately, with a tolerance.
#[derive(Clone, PartialEq, Debug)]
pub struct Signature {
    pub track_id: String,
    pub image: Option<String>,
    pub duration: Option<Duration>,
}

pub fn build(presence: &Presence, image: Option<&str>, status_text: StatusText) -> (Value, Signature) {
    let track = &presence.track;
    let image = image.filter(|url| url.len() <= MAX_IMAGE_LENGTH);
    let start = presence.started_at_ms();

    let mut activity = Map::new();
    activity.insert("type".into(), json!(ACTIVITY_TYPE_LISTENING));
    if let Some(value) = status_text.wire_value() {
        activity.insert("status_display_type".into(), json!(value));
    }
    activity.insert(
        "details".into(),
        json!(text_field(&track.title).unwrap_or_else(|| "Unknown song".to_owned())),
    );
    if let Some(artist) = text_field(&track.artist) {
        activity.insert("state".into(), json!(artist));
    }

    let mut timestamps = Map::new();
    timestamps.insert("start".into(), json!(start));
    if let Some(duration) = track.duration {
        let end = start + duration.max(presence.position).as_millis() as i64;
        timestamps.insert("end".into(), json!(end));
    }
    activity.insert("timestamps".into(), Value::Object(timestamps));

    let mut assets = Map::new();
    if let Some(image) = image {
        assets.insert("large_image".into(), json!(image));
        if let Some(album) = text_field(&track.album) {
            assets.insert("large_text".into(), json!(album));
        }
    }
    if !assets.is_empty() {
        activity.insert("assets".into(), Value::Object(assets));
    }

    let signature = Signature {
        track_id: track.id.clone(),
        image: image.map(str::to_owned),
        duration: track.duration,
    };
    (Value::Object(activity), signature)
}

/// Trims and fits a string to Discord's limits, or `None` if it is empty.
pub fn text_field(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut units = 0;
    let truncated = text.encode_utf16().count() > MAX_TEXT_UNITS;
    let limit = if truncated { MAX_TEXT_UNITS - 1 } else { MAX_TEXT_UNITS };
    for character in text.chars() {
        units += character.len_utf16();
        if units > limit {
            break;
        }
        out.push(character);
    }
    if truncated {
        out.push('\u{2026}');
    }
    if out.chars().count() < MIN_TEXT_CHARS {
        out.push('\u{200b}');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(position: u64, duration: Option<u64>) -> Presence {
        Presence {
            track: TrackInfo {
                id: "1".into(),
                title: "Song".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration: duration.map(Duration::from_secs),
                ..Default::default()
            },
            position: Duration::from_secs(position),
            captured_at: UNIX_EPOCH + Duration::from_secs(1000),
        }
    }

    #[test]
    fn elapsed_time_gives_start_and_end() {
        let (activity, _) = build(&presence(30, Some(200)), None, StatusText::AppName);
        assert_eq!(activity["timestamps"]["start"], json!(970_000));
        assert_eq!(activity["timestamps"]["end"], json!(1_170_000));
        assert_eq!(activity["type"], json!(2));
    }

    #[test]
    fn missing_duration_omits_the_end() {
        let (activity, _) = build(&presence(30, None), None, StatusText::AppName);
        assert!(activity["timestamps"].get("end").is_none());
    }

    #[test]
    fn album_is_only_sent_with_an_image() {
        let (without, _) = build(&presence(0, None), None, StatusText::AppName);
        assert!(without.get("assets").is_none());
        let (with, _) = build(&presence(0, None), Some("https://x/y.jpg"), StatusText::AppName);
        assert_eq!(with["assets"]["large_text"], "Album");
    }

    #[test]
    fn overlong_image_urls_are_dropped() {
        let url = format!("https://x/{}", "y".repeat(300));
        let (activity, signature) = build(&presence(0, None), Some(&url), StatusText::AppName);
        assert!(activity.get("assets").is_none());
        assert_eq!(signature.image, None);
    }

    #[test]
    fn long_text_is_truncated_in_utf16_units() {
        let result = text_field(&"\u{1F3B5}".repeat(100)).unwrap();
        assert!(result.encode_utf16().count() <= 128);
        assert!(result.ends_with('\u{2026}'));
        let result = text_field(&"a".repeat(128)).unwrap();
        assert_eq!(result.chars().count(), 128);
        assert!(!result.ends_with('\u{2026}'));
    }

    #[test]
    fn short_text_is_padded_and_blank_is_none() {
        assert_eq!(text_field("X").unwrap(), "X\u{200b}");
        assert_eq!(text_field("OK").unwrap(), "OK");
        assert_eq!(text_field("   "), None);
    }
}
