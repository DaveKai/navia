use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Paused,
    Playing,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct TrackInfo {
    /// Stable song id, used to notice track changes.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<Duration>,
    /// MusicBrainz release id (Navidrome's album `musicBrainzId`), if tagged.
    pub release_mbid: Option<String>,
    /// MusicBrainz release group id, if tagged.
    pub release_group_mbid: Option<String>,
    /// The server's album id, for the server's own public cover link.
    pub album_id: Option<String>,
    /// The recording's ISRC (OpenSubsonic `isrc`), for an exact store lookup.
    pub isrc: Option<String>,
    /// Artwork for the OS media controls only, as a `file://` path or a URL the local machine can read. Never sent to
    /// Discord.
    pub cover_url: Option<String>,
}

/// Repeat as MPRIS `LoopStatus` names it: None, Track (repeat one) or Playlist (repeat all).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LoopMode {
    #[default]
    None,
    Track,
    Playlist,
}

/// Everything an integration needs to know about the player at one moment.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PlayerSnapshot {
    pub state: PlaybackState,
    pub track: Option<TrackInfo>,
    pub position: Duration,
    /// 0.0 to 1.0.
    pub volume: f64,
    /// When set, nothing about the current playback may be shown publicly.
    pub private_mode: bool,
    pub loop_mode: LoopMode,
    pub shuffle: bool,
    /// False while repeat and shuffle can't change, like during a station.
    pub can_change_modes: bool,
}

/// Read side of the player, implemented by whatever owns playback.
pub trait PlayerState: Send + Sync {
    fn snapshot(&self) -> PlayerSnapshot;
}

/// A request coming from the OS, such as a media key.
#[derive(Clone, PartialEq, Debug)]
pub enum PlayerCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekForward(Duration),
    SeekBackward(Duration),
    SeekTo(Duration),
    /// 0.0 to 1.0 is the expected range. Other values may arrive and should be clamped.
    SetVolume(f64),
    SetLoop(LoopMode),
    SetShuffle(bool),
    /// Bring the window to the front.
    Raise,
    Quit,
}

/// Command side of the player. Called from the integration's own thread.
pub trait PlayerControl: Send + Sync {
    fn command(&self, command: PlayerCommand);
}

/// Something that wants to follow the player.
pub trait Integration {
    fn update(&mut self, snapshot: &PlayerSnapshot);
}
