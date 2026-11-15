//! Discord "Listening to" status for the Swift app, through `navia-integrations`. Does nothing until enabled, and never
//! in a test run.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use navia_integrations::art::{ArtSource, CoverSource, NoArt, PublicAddress, PublicAddressError, PublicArt, ServerLookup, parse_public_address, public_server_address};
use navia_integrations::discord::{DiscordConfig, DiscordPresence};
use navia_integrations::{PlaybackState, PlayerSnapshot, TrackInfo};

use crate::core_api::NaviaCore;

/// Navia's Discord application, which supplies the name and `navia` image Rich Presence shows.
const DISCORD_APP_ID: &str = "1399802340940185642";
const DISCORD_DEFAULT_IMAGE: &str = "navia";

#[derive(uniffi::Record)]
pub struct DiscordTrackRow {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    /// The server's album id, for the server's own public cover.
    pub album_id: Option<String>,
}

/// The "Public server address" setting, validated.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PublicAddressCheck {
    Empty,
    Usable,
    NotHttps,
    Invalid,
}

/// Validates a typed "Public server address". Only `https`, with just a host and a base path.
#[uniffi::export]
pub fn check_public_address(text: String) -> PublicAddressCheck {
    match parse_public_address(&text) {
        Ok(None) => PublicAddressCheck::Empty,
        Ok(Some(_)) => PublicAddressCheck::Usable,
        Err(PublicAddressError::NotHttps) => PublicAddressCheck::NotHttps,
        Err(_) => PublicAddressCheck::Invalid,
    }
}

/// Settings > Discord "Covers from".
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiscordCoverSource {
    /// The server when its address looks public, else the external sources.
    Automatic,
    /// Always the server's own cover link.
    Server,
    /// Always the external sources.
    Search,
}

/// Whether the connected server address counts as public for Discord covers under Automatic. It must be `https`, not
/// private, loopback, link-local, Tailscale or `.local`, and its name must resolve to a public address. Blocks on DNS,
/// so call it off the main thread.
#[uniffi::export]
pub fn server_address_is_public(server_url: String) -> bool {
    public_server_address(&server_url).is_some()
}

#[derive(uniffi::Object)]
pub struct NaviaDiscord {
    presence: Mutex<Option<DiscordPresence>>,
    /// The signed-in server, used for its public cover links and for ISRCs the library lacks.
    server: Mutex<Option<Arc<NaviaCore>>>,
    address: PublicAddress,
    /// The last song's ISRC from the library, so updates don't read it again.
    isrc: Mutex<Option<(String, Option<String>)>>,
}

/// Off until [`NaviaDiscord::set_enabled`].
#[uniffi::export]
pub fn make_discord_status() -> Arc<NaviaDiscord> {
    Arc::new(NaviaDiscord { presence: Mutex::new(None), server: Mutex::new(None), address: PublicAddress::default(), isrc: Mutex::new(None) })
}

impl NaviaDiscord {
    fn start(&self, presence: &mut Option<DiscordPresence>) {
        let server = self.server.lock().unwrap().clone();
        let art: Arc<dyn ArtSource> = match PublicArt::new() {
            Ok(a) => match server {
                Some(core) => a.with_server(core as Arc<dyn ServerLookup>, self.address.clone()).map_or_else(|_| Arc::new(NoArt) as Arc<dyn ArtSource>, |a| Arc::new(a)),
                None => Arc::new(a),
            },
            Err(_) => Arc::new(NoArt),
        };
        let mut config = DiscordConfig::new(DISCORD_APP_ID);
        config.default_image = Some(DISCORD_DEFAULT_IMAGE.to_string());
        *presence = DiscordPresence::new(config, art).ok();
    }
}

#[uniffi::export]
impl NaviaDiscord {
    /// Starts or stops the status. Stopping can wait briefly for the connection to close.
    pub fn set_enabled(&self, on: bool) {
        let mut presence = self.presence.lock().unwrap();
        if !on {
            presence.take();
            return;
        }
        if presence.is_none() {
            self.start(&mut presence);
        }
    }

    /// The signed-in server, or None after signing out. A running status restarts with it.
    pub fn set_server(&self, core: Option<Arc<NaviaCore>>) {
        let same = match (&*self.server.lock().unwrap(), &core) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.address.set_server(core.as_ref().map(|c| c.server_url()));
        *self.server.lock().unwrap() = core;
        *self.isrc.lock().unwrap() = None;
        let mut presence = self.presence.lock().unwrap();
        if presence.take().is_some() {
            self.start(&mut presence);
        }
    }

    /// Applies the "Public server address" setting once entered. Anything that isn't a usable `https` address counts as
    /// none.
    pub fn set_public_address(&self, text: String) {
        self.address.set_custom(parse_public_address(&text).ok().flatten());
    }

    /// Settings > Discord "Covers from".
    pub fn set_cover_source(&self, source: DiscordCoverSource) {
        self.address.set_source(match source {
            DiscordCoverSource::Automatic => CoverSource::Automatic,
            DiscordCoverSource::Server => CoverSource::Server,
            DiscordCoverSource::Search => CoverSource::Search,
        });
    }

    /// What's playing now. Paused, stopped or private clears the status.
    pub fn update(&self, track: Option<DiscordTrackRow>, playing: bool, position_ms: u64, private_mode: bool) {
        let presence = self.presence.lock().unwrap();
        let Some(presence) = presence.as_ref() else { return };
        let track = track.map(|t| {
            let isrc = self.library_isrc(&t.id);
            TrackInfo { id: t.id, title: t.title, artist: t.artist, album: t.album, duration: t.duration_ms.map(Duration::from_millis), album_id: t.album_id.filter(|id| !id.is_empty()), isrc, ..Default::default() }
        });
        let snapshot = PlayerSnapshot {
            state: if playing { PlaybackState::Playing } else { PlaybackState::Paused },
            track,
            position: Duration::from_millis(position_ms),
            volume: 1.0,
            private_mode,
            // Discord shows none of these.
            loop_mode: Default::default(),
            shuffle: false,
            can_change_modes: false,
        };
        presence.set_snapshot(&snapshot);
    }
}

impl NaviaDiscord {
    fn library_isrc(&self, song_id: &str) -> Option<String> {
        let mut last = self.isrc.lock().unwrap();
        if let Some((id, isrc)) = last.as_ref()
            && id == song_id
        {
            return isrc.clone();
        }
        let isrc = self.server.lock().unwrap().as_ref().and_then(|core| core.song_isrc_local(song_id));
        *last = Some((song_id.to_owned(), isrc.clone()));
        isrc
    }
}
