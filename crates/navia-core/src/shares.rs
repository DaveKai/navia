//! Share links through Subsonic's share endpoints.
//! `getShares` doesn't say a share's kind or whether it allows downloads, so links made here are remembered locally.
//! Only the signed-in user's own links are listed, even for an admin.

use serde::{Deserialize, Serialize};

use crate::client::{Client, Share, ShareEntry};
use crate::error::{Error, Result};
use crate::library::Library;
use crate::queue_sync::parse_iso8601_ms;

pub const DAY_MS: u64 = 86_400_000;
/// The ready-made end dates the share dialogs offer, as (id, days from now).
pub const EXPIRY_PRESETS: [(&str, u64); 3] = [("1d", 1), ("1w", 7), ("1m", 30)];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareKind {
    Song,
    Album,
    Playlist,
    Artist,
}

/// What a share holds, songs or albums or one playlist or artist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareTarget {
    pub kind: ShareKind,
    pub ids: Vec<String>,
}

/// What the share dialogs ask for. A `None` end date means the server's default on a new link
/// and the current date on an existing one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareChoice {
    pub description: String,
    pub expires_ms: Option<u64>,
    pub downloadable: bool,
}

/// One of the user's links.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareLink {
    /// The server's id, also the link's last part.
    pub id: String,
    pub url: String,
    pub description: String,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub last_visited_ms: Option<u64>,
    pub visits: u64,
    pub target: ShareTarget,
    /// Whether it allows downloads, when this device made or edited it.
    pub downloadable: Option<bool>,
    /// The albums or songs the server lists in it.
    pub entries: Vec<ShareEntry>,
}

impl ShareLink {
    pub fn expired(&self, now_ms: u64) -> bool {
        self.expires_ms <= now_ms
    }
}

/// What this device remembers of a link it made or edited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Remembered {
    kind: ShareKind,
    ids: Vec<String>,
    downloadable: bool,
}

fn meta_key(id: &str) -> String {
    format!("share:{id}")
}

/// The server's share as a link, with dates as Unix ms.
/// The kind comes from what this device remembers, else from its entries.
pub fn link_from(share: Share, remembered: Option<(ShareTarget, bool)>) -> ShareLink {
    let ms = |s: &Option<String>| s.as_deref().and_then(parse_iso8601_ms);
    let (target, downloadable) = match remembered {
        Some((target, downloadable)) => (target, Some(downloadable)),
        None => {
            let albums = share.entry.iter().any(|e| e.is_dir == Some(true));
            let kind = if albums { ShareKind::Album } else { ShareKind::Song };
            (ShareTarget { kind, ids: share.entry.iter().map(|e| e.id.clone()).collect() }, None)
        }
    };
    ShareLink {
        url: share.url.clone().unwrap_or_default(),
        description: share.description.clone().unwrap_or_default(),
        created_ms: ms(&share.created).unwrap_or(0),
        expires_ms: ms(&share.expires).unwrap_or(0),
        last_visited_ms: ms(&share.last_visited),
        visits: share.visit_count.unwrap_or(0).max(0) as u64,
        id: share.id,
        target,
        downloadable,
        entries: share.entry,
    }
}

/// Whether a description is only the server's stand-in for an empty one, the title cut to 30 characters.
/// Such descriptions aren't shown or edited.
pub fn is_server_label(description: &str, title: &str) -> bool {
    let d = description.trim().trim_end_matches("...").trim_end_matches('\u{2026}').trim_end();
    !d.is_empty() && (title.trim() == d || (description.chars().count() >= 30 && title.trim().starts_with(d)))
}

/// The end of a calendar day, 23:59:59 local time, as Unix ms. `offset_s` is the zone's offset east of UTC.
pub fn end_of_day_ms(year: i32, month: u32, day: u32, offset_s: i64) -> Option<u64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    // Days from the epoch in the proleptic Gregorian calendar.
    let (y, m) = if month <= 2 { (year as i64 - 1, month as i64 + 9) } else { (year as i64, month as i64 - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + 86_399 - offset_s;
    u64::try_from(secs).ok().map(|s| s * 1000)
}

/// The user's links, newest first, with what this device remembers of them.
pub async fn list(client: &Client, library: &Library) -> Result<Vec<ShareLink>> {
    let username = client.credentials().username().to_string();
    let shares: Vec<Share> = client.shares().await?.into_iter().filter(|s| s.username.as_deref().is_none_or(|u| u == username)).collect();
    let ids: Vec<String> = shares.iter().map(|s| s.id.clone()).collect();
    let remembered = library.blocking(move |l| Ok(ids.iter().map(|id| l.meta(&meta_key(id)).ok().flatten()).collect::<Vec<_>>())).await?;
    let mut links: Vec<ShareLink> = shares
        .into_iter()
        .zip(remembered)
        .map(|(share, memo)| {
            let memo = memo.and_then(|m| serde_json::from_str::<Remembered>(&m).ok()).map(|r| (ShareTarget { kind: r.kind, ids: r.ids }, r.downloadable));
            link_from(share, memo)
        })
        .collect();
    links.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
    Ok(links)
}

/// Makes a link. `downloads` says whether the server allows downloads at all.
pub async fn create(client: &Client, library: &Library, target: ShareTarget, choice: ShareChoice, downloads: bool) -> Result<ShareLink> {
    if target.ids.is_empty() {
        return Err(Error::Decode { endpoint: "createShare".to_string(), detail: "nothing to share".to_string() });
    }
    let downloadable = downloads && choice.downloadable;
    let share = client.create_share(&target.ids, &choice.description, choice.expires_ms, downloads.then_some(downloadable)).await?;
    remember(library, &share.id, &target, downloadable).await;
    Ok(link_from(share, Some((target, downloadable))))
}

/// Edits a link's description, its end date when one is chosen and downloads where allowed.
pub async fn update(client: &Client, library: &Library, link: &ShareLink, choice: ShareChoice, downloads: bool) -> Result<()> {
    let downloadable = downloads && choice.downloadable;
    client.update_share(&link.id, &choice.description, choice.expires_ms, downloads.then_some(downloadable)).await?;
    remember(library, &link.id, &link.target, downloadable).await;
    Ok(())
}

/// Deletes a link, which stops working for everyone who has it.
pub async fn delete(client: &Client, library: &Library, id: &str) -> Result<()> {
    client.delete_share(id).await?;
    let key = meta_key(id);
    let _ = library.blocking(move |l| l.set_meta(&key, None)).await;
    Ok(())
}

async fn remember(library: &Library, id: &str, target: &ShareTarget, downloadable: bool) {
    let memo = Remembered { kind: target.kind, ids: target.ids.clone(), downloadable };
    let (key, value) = (meta_key(id), serde_json::to_string(&memo).unwrap_or_default());
    let _ = library.blocking(move |l| l.set_meta(&key, Some(&value))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_labels() {
        assert!(is_server_label("MC-101", "MC-101"));
        assert!(is_server_label("HIT ME HARD AND SOFT (Walmart e...", "HIT ME HARD AND SOFT (Walmart exclusive)"));
        assert!(!is_server_label("For the drive", "MC-101"));
        assert!(!is_server_label("", "MC-101"));
        assert!(!is_server_label("MC", "MC-101"));
    }

    #[test]
    fn dates() {
        // 2026-11-12 23:59:59 UTC, then in UTC+1, an hour earlier.
        assert_eq!(end_of_day_ms(2026, 11, 12, 0), Some(1_794_527_999_000));
        assert_eq!(end_of_day_ms(2026, 11, 12, 3600), Some(1_794_524_399_000));
        assert_eq!(end_of_day_ms(2024, 2, 29, 0), Some(1_709_251_199_000));
        assert_eq!(end_of_day_ms(2026, 13, 1, 0), None);
    }

    #[test]
    fn kinds_from_entries_or_memory() {
        let share = Share {
            id: "abc".into(),
            created: Some("2026-11-01T10:00:00Z".into()),
            expires: Some("2027-11-01T10:00:00Z".into()),
            visit_count: Some(3),
            entry: vec![ShareEntry { id: "al1".into(), is_dir: Some(true), ..Default::default() }],
            ..Default::default()
        };
        let link = link_from(share.clone(), None);
        assert_eq!(link.target, ShareTarget { kind: ShareKind::Album, ids: vec!["al1".into()] });
        assert_eq!(link.downloadable, None);
        assert_eq!(link.visits, 3);
        assert!(!link.expired(link.created_ms));
        assert!(link.expired(link.expires_ms));
        let artist = ShareTarget { kind: ShareKind::Artist, ids: vec!["ar1".into()] };
        let link = link_from(share, Some((artist.clone(), true)));
        assert_eq!((link.target, link.downloadable), (artist, Some(true)));
    }
}
