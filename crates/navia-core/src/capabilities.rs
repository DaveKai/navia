//! What this server can do. Each answer says how it was detected, and [`Support::Unknown`] means we couldn't tell.

use serde::{Deserialize, Serialize};

use crate::client::{Client, Extension, MusicFolder, ServerInfo, TopSongsBy, User};
use crate::error::{Error, Result};
use crate::library::Library;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Support {
    Available,
    Unavailable,
    /// No way to tell from what the server exposes, or the check itself failed.
    Unknown,
}

/// One detected feature, with the evidence for the answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub support: Support,
    pub evidence: String,
}

impl Capability {
    fn yes(evidence: impl Into<String>) -> Self {
        Self { support: Support::Available, evidence: evidence.into() }
    }
    fn no(evidence: impl Into<String>) -> Self {
        Self { support: Support::Unavailable, evidence: evidence.into() }
    }
    fn unknown(evidence: impl Into<String>) -> Self {
        Self { support: Support::Unknown, evidence: evidence.into() }
    }
    pub fn is_available(&self) -> bool {
        self.support == Support::Available
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub server: ServerInfo,
    pub extensions: Vec<Extension>,
    pub libraries: Vec<MusicFolder>,
    pub user: Option<User>,
    pub sharing: Capability,
    pub synced_lyrics: Capability,
    pub top_songs_by_id: Capability,
    pub artist_info: Capability,
    pub top_songs: Capability,
    pub similar_songs: Capability,
    pub star_ratings: Capability,
    pub favorites: Capability,
}

/// The OpenSubsonic extensions Navia uses, from `getOpenSubsonicExtensions`.
/// Anything missing keeps the older behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerFeatures {
    /// `songLyrics`: `getLyricsBySongId` with line timing.
    pub synced_lyrics: bool,
    /// `songLyrics` version 2: word timing with `enhanced=true`.
    pub word_lyrics: bool,
    /// `topSongsByArtistId`: `getTopSongs` by artist id.
    pub top_songs_by_id: bool,
    /// `playbackReport`: `reportPlayback`.
    pub playback_report: bool,
    /// `indexBasedQueue`: `savePlayQueueByIndex` and `getPlayQueueByIndex`.
    pub index_queue: bool,
    /// `formPost`: parameters may go in a POST form, so long lists fit.
    pub form_post: bool,
}

impl ServerFeatures {
    pub fn from_extensions(extensions: &[Extension]) -> Self {
        let find = |n: &str| extensions.iter().find(|e| e.name == n);
        let lyrics = find("songLyrics");
        Self {
            synced_lyrics: lyrics.is_some(),
            word_lyrics: lyrics.is_some_and(|e| e.versions.contains(&2)),
            top_songs_by_id: find("topSongsByArtistId").is_some(),
            playback_report: find("playbackReport").is_some(),
            index_queue: find("indexBasedQueue").is_some(),
            form_post: find("formPost").is_some(),
        }
    }

    /// Asks the server. `None` when the list couldn't be read, which callers treat differently from an empty list.
    pub async fn detect(client: &Client) -> Option<Self> {
        match client.open_subsonic_extensions().await {
            Ok(list) => Some(Self::from_extensions(&list)),
            Err(e) => {
                tracing::debug!("could not list OpenSubsonic extensions: {e}");
                None
            }
        }
    }
}

/// An artist known to exist, used to check whether external data comes back at all.
#[derive(Clone, Debug)]
pub struct ExternalProbe {
    pub artist_id: String,
    pub artist_name: String,
}

impl ExternalProbe {
    /// The artist with the most albums, the likeliest to have external data. `None` before the first sync.
    pub fn from_library(library: &Library) -> Result<Option<Self>> {
        Ok(library.top_artist()?.map(|a| Self { artist_id: a.id, artist_name: a.name }))
    }
}

impl Capabilities {
    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e.name == name)
    }

    /// Detects capabilities. `ping` must succeed, and any other failed check becomes `Unknown`.
    pub async fn detect(client: &Client, probe: Option<&ExternalProbe>) -> Result<Self> {
        let server = client.ping().await?;

        let (extensions, extensions_known) = if server.open_subsonic {
            match client.open_subsonic_extensions().await {
                Ok(e) => (e, true),
                Err(e) => {
                    tracing::warn!("could not list OpenSubsonic extensions: {e}");
                    (Vec::new(), false)
                }
            }
        } else {
            (Vec::new(), true)
        };
        let has = |n: &str| extensions.iter().any(|e| e.name == n);

        let libraries = client.music_folders().await.unwrap_or_default();
        let user = client.user().await.ok();

        let sharing = detect_sharing(client, user.as_ref()).await;
        let synced_lyrics = extension_capability(extensions_known, has("songLyrics"), "songLyrics");
        let top_songs_by_id = extension_capability(extensions_known, has("topSongsByArtistId"), "topSongsByArtistId");

        let (artist_info, top_songs) = match probe {
            Some(p) => (probe_artist_info(client, p).await, probe_top_songs(client, p).await),
            None => {
                let c = Capability::unknown("no artist available to probe yet, usually before the first sync");
                (c.clone(), c)
            }
        };

        // No read API reports these, and a write probe would change server data.
        let star_ratings = Capability::unknown("no read API reports whether star ratings are enabled");
        let favorites = Capability::unknown("no read API reports whether favorites are enabled");
        // Not probed because the first call for an artist can take seconds. Features try it on demand.
        let similar_songs = Capability::unknown("not probed, because it can be slow; try it on demand and handle an empty result");

        Ok(Self { server, extensions, libraries, user, sharing, synced_lyrics, top_songs_by_id, artist_info, top_songs, similar_songs, star_ratings, favorites })
    }
}

fn extension_capability(known: bool, present: bool, name: &str) -> Capability {
    if !known {
        Capability::unknown(format!("could not read the OpenSubsonic extension list to check {name}"))
    } else if present {
        Capability::yes(format!("server advertises the {name} extension"))
    } else {
        Capability::no(format!("server does not advertise the {name} extension"))
    }
}

async fn detect_sharing(client: &Client, user: Option<&User>) -> Capability {
    match client.shares().await {
        Ok(_) => match user.and_then(|u| u.share_role) {
            Some(false) => Capability::no("getShares answers, but this user's shareRole is off"),
            _ => Capability::yes("getShares answered"),
        },
        Err(Error::HttpStatus { status: 501 | 404 | 410, .. }) => Capability::no("getShares reports it is not implemented, so EnableSharing is off"),
        Err(Error::Subsonic { code, message, .. }) => Capability::no(format!("getShares refused with error {code}: {message}")),
        Err(e) => Capability::unknown(format!("getShares could not be checked: {e}")),
    }
}

async fn probe_artist_info(client: &Client, probe: &ExternalProbe) -> Capability {
    match client.artist_info2(&probe.artist_id, Some(1)).await {
        Ok(info) if info.biography.as_deref().is_some_and(|b| !b.trim().is_empty()) || !info.similar_artist.is_empty() || info.large_image_url.as_deref().is_some_and(|u| !u.is_empty()) => {
            Capability::yes("getArtistInfo2 returned data for a sample artist")
        }
        Ok(_) => Capability::unknown("getArtistInfo2 returned nothing for one artist, which can mean no integrations or just no data for them"),
        Err(e) => unknown_or_no("getArtistInfo2", e),
    }
}

async fn probe_top_songs(client: &Client, probe: &ExternalProbe) -> Capability {
    // The count applies before matching against the library, so count=1 can return nothing
    // even for an artist with plenty of songs. Ask for a normal page.
    match client.top_songs(&TopSongsBy::Name(probe.artist_name.clone()), 20).await {
        Ok(songs) if !songs.is_empty() => Capability::yes("getTopSongs returned songs for a sample artist"),
        Ok(_) => Capability::unknown("getTopSongs returned nothing for one artist, which can mean no integrations or just no match"),
        Err(e) => unknown_or_no("getTopSongs", e),
    }
}

fn unknown_or_no(endpoint: &str, e: Error) -> Capability {
    match e {
        Error::Subsonic { .. } | Error::HttpStatus { status: 501 | 404 | 410, .. } => Capability::no(format!("{endpoint} refused the request: {e}")),
        other => Capability::unknown(format!("{endpoint} could not be checked: {other}")),
    }
}
