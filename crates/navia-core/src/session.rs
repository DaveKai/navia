//! One signed-in connection to one server. The UI shells call straight into a `Session`,
//! so they carry no networking or database logic.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::artwork::{ArtworkCache, ArtworkConfig};
use crate::audio::player::{Player, PlayerConfig, PlayerEvent, QueueItem, ServerResolver};
use crate::capabilities::{Capabilities, ExternalProbe};
use crate::client::{Client, ClientConfig, Credentials, Song, WritePolicy};
use crate::error::{Error, Result};
use crate::library::sync::{AnnotationRefresh, BackgroundRefresh, PlaylistSync, RefreshOutcome, SyncOptions, SyncOutcome, SyncProgress, Syncer};
use crate::library::{Library, LibraryConfig};
use crate::reporting::{LiveEvent, PlayReporter, TrackPlay};

/// Default artwork cache size, plenty for a large library's covers.
const DEFAULT_ARTWORK_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub struct SessionConfig {
    pub server_url: String,
    pub credentials: Credentials,
    pub write_policy: WritePolicy,
    /// Database file. `None` uses the normal per-user data directory for this account.
    pub library_path: Option<PathBuf>,
    /// Where cover art is cached. `None` uses the normal per-user cache directory for this account.
    pub artwork_dir: Option<PathBuf>,
}

pub struct Session {
    client: Client,
    library: Library,
    reporter: Arc<PlayReporter>,
    capabilities: Capabilities,
    artwork: ArtworkCache,
}

impl Session {
    /// Connects and detects what the server supports. Fails only if the server can't be reached
    /// or refuses the login.
    pub async fn connect(config: SessionConfig) -> Result<Self> {
        let mut client_config = ClientConfig::new(config.server_url.clone(), config.credentials.clone());
        client_config.write_policy = config.write_policy;
        let client = Client::new(client_config)?;
        let path = match config.library_path {
            Some(p) => LibraryConfig { path: p },
            None => LibraryConfig::default_for(&config.credentials, &config.server_url)?,
        };
        let library = Library::open(&path)?;
        let probe = ExternalProbe::from_library(&library)?;
        let capabilities = Capabilities::detect(&client, probe.as_ref()).await?;
        let artwork_dir = match config.artwork_dir {
            Some(dir) => dir,
            None => default_artwork_dir(&config.credentials, &config.server_url)?,
        };
        let artwork = ArtworkCache::new(client.clone(), ArtworkConfig { dir: artwork_dir, max_bytes: DEFAULT_ARTWORK_CACHE_BYTES })?;
        // Plays that didn't reach the server last time, including across a crash, go out now.
        let reporter = Arc::new(PlayReporter::with_store(Arc::new(client.clone()), Arc::new(library.clone())));
        let pending = reporter.clone();
        tokio::spawn(async move {
            let _ = pending.flush().await;
        });
        Ok(Self { client, library, reporter, capabilities, artwork })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn reporter(&self) -> Arc<PlayReporter> {
        self.reporter.clone()
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn artwork(&self) -> &ArtworkCache {
        &self.artwork
    }

    /// A player streaming from this server, with reporting wired up. Call it inside a tokio runtime.
    pub fn player(&self, config: PlayerConfig) -> Result<Player> {
        let resolver = Arc::new(ServerResolver::new(self.client.clone())?);
        let player = Player::new(resolver, config)?;
        attach_reporting(&player, self.reporter.clone());
        Ok(player)
    }

    /// A queue item for a song in the local library or from the server.
    pub fn queue_item(song: &Song) -> QueueItem {
        QueueItem::new(song.id.clone(), song.duration.map(|d| (d as u64).saturating_mul(1000)))
    }

    fn syncer(&self, options: SyncOptions) -> Syncer {
        Syncer::new(self.client.clone(), self.library.clone(), options)
    }

    /// Syncs now, resuming an interrupted sync if there is one.
    pub async fn sync(&self, on_progress: &(dyn Fn(SyncProgress) + Send + Sync)) -> Result<SyncOutcome> {
        self.syncer(SyncOptions::default()).full_sync(on_progress).await
    }

    /// Syncs only if the server's library changed since the last complete sync.
    pub async fn refresh(&self, force: bool) -> Result<RefreshOutcome> {
        self.syncer(SyncOptions::default()).refresh(force, &|_| {}).await
    }

    /// Refreshes the stored playlists.
    pub async fn sync_playlists(&self) -> Result<PlaylistSync> {
        self.syncer(SyncOptions::default()).sync_playlists().await
    }

    /// Fetches one playlist now. Smart playlists only update when fetched.
    pub async fn refresh_playlist(&self, id: &str) -> Result<()> {
        self.syncer(SyncOptions::default()).refresh_playlist(id).await
    }

    /// Refreshes stars and play counts without a catalog sync.
    pub async fn refresh_annotations(&self) -> Result<AnnotationRefresh> {
        self.syncer(SyncOptions::default()).refresh_annotations().await
    }

    /// Keeps the library, playlists, stars and play counts current in the background, and retries
    /// unsent plays. Drop the returned handle to stop.
    pub fn refresh_in_background(&self, interval: Duration) -> BackgroundRefresh {
        BackgroundRefresh::spawn_with(self.syncer(SyncOptions::default()), interval, Some(self.reporter.clone()))
    }
}

fn default_artwork_dir(credentials: &Credentials, server_url: &str) -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "Navia").ok_or(Error::NoDataDirectory)?;
    Ok(dirs.cache_dir().join("artwork").join(credentials.account_key(server_url)))
}

/// Feeds player events to the reporter. Listeners run on the player thread, so reports go through
/// a channel to the tokio runtime.
pub fn attach_reporting(player: &Player, reporter: Arc<PlayReporter>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    player.add_listener(move |event: &PlayerEvent| {
        if matches!(event, PlayerEvent::TrackChanged { .. } | PlayerEvent::Heard { .. } | PlayerEvent::QueueEnded | PlayerEvent::StatusChanged(_) | PlayerEvent::Position { .. }) {
            let _ = tx.send(event.clone());
        }
    });
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                PlayerEvent::TrackChanged { item, .. } => {
                    reporter.track_ended();
                    let duration = Duration::from_millis(item.duration_ms.unwrap_or(0));
                    // Stations have no length and aren't songs the server knows.
                    let id = item.duration_ms.is_some().then(|| item.id.clone());
                    reporter.track_started(TrackPlay { song_id: item.id, duration, started_at_ms: now_ms() }).await;
                    reporter.live_event(LiveEvent::Track(id)).await;
                }
                PlayerEvent::Heard { since_last } => reporter.advance(since_last).await,
                PlayerEvent::QueueEnded => {
                    reporter.track_ended();
                    reporter.live_event(LiveEvent::Ended).await;
                }
                PlayerEvent::StatusChanged(status) => reporter.live_event(LiveEvent::Status(status)).await,
                PlayerEvent::Position { ms } => reporter.live_event(LiveEvent::Position(ms)).await,
                _ => {}
            }
        }
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
