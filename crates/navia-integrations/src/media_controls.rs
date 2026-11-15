//! OS media controls on Windows through `souvlaki` (SMTC). Linux uses its own MPRIS server in `mpris.rs`.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use souvlaki::{
    MediaControlEvent, MediaControls as Controls, MediaMetadata, MediaPlayback, MediaPosition,
    PlatformConfig, SeekDirection,
};

use crate::{Integration, PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot, TrackInfo};

/// Windows' skip buttons don't say how far to seek.
const DEFAULT_SEEK: Duration = Duration::from_secs(10);
/// How far off the extrapolated position counts as a seek.
const SEEK_TOLERANCE: Duration = Duration::from_millis(1500);
/// How often to refresh the system's position.
const POSITION_REFRESH: Duration = Duration::from_secs(5);

pub struct MediaControlsConfig {
    /// The name shown by the desktop.
    pub display_name: String,
    /// The last part of the MPRIS bus name, like `navia`.
    pub dbus_name: String,
    /// MPRIS's `DesktopEntry` on Linux. Unused here.
    pub desktop_entry: String,
    /// The window handle. Required on Windows and unused elsewhere.
    pub hwnd: Option<*mut std::ffi::c_void>,
}

#[derive(Debug)]
pub struct MediaControlsError(String);

impl fmt::Display for MediaControlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MediaControlsError {}

struct Synced {
    state: PlaybackState,
    position: Duration,
    at: Instant,
}

pub struct MediaControls {
    controls: Controls,
    track: Option<TrackInfo>,
    synced: Option<Synced>,
    failed: bool,
}

impl MediaControls {
    /// On Linux this connects to the session bus on a background thread. Without a bus, updates stop quietly and a
    /// warning is logged.
    pub fn new(
        config: MediaControlsConfig,
        control: Arc<dyn PlayerControl>,
    ) -> Result<Self, MediaControlsError> {
        let platform = PlatformConfig {
            display_name: &config.display_name,
            dbus_name: &config.dbus_name,
            hwnd: config.hwnd,
        };
        let mut controls = Controls::new(platform).map_err(describe)?;
        controls
            .attach(move |event| {
                if let Some(command) = to_command(event) {
                    control.command(command);
                }
            })
            .map_err(describe)?;
        Ok(Self {
            controls,
            track: None,
            synced: None,
            failed: false,
        })
    }

    /// True once the OS service has failed and updates have stopped.
    pub fn failed(&self) -> bool {
        self.failed
    }

    fn apply(&mut self, snapshot: &PlayerSnapshot) -> Result<(), souvlaki::Error> {
        if snapshot.track != self.track {
            let metadata = snapshot.track.as_ref().map(to_metadata).unwrap_or_default();
            self.controls.set_metadata(metadata)?;
            self.track = snapshot.track.clone();
            self.synced = None;
        }
        if self.needs_playback_sync(snapshot) {
            self.controls.set_playback(to_playback(snapshot))?;
            self.synced = Some(Synced {
                state: snapshot.state,
                position: snapshot.position,
                at: Instant::now(),
            });
        }
        Ok(())
    }

    fn needs_playback_sync(&self, snapshot: &PlayerSnapshot) -> bool {
        let Some(synced) = &self.synced else {
            return true;
        };
        if synced.state != snapshot.state {
            return true;
        }
        let elapsed = synced.at.elapsed();
        let expected = match snapshot.state {
            PlaybackState::Playing => synced.position + elapsed,
            _ => synced.position,
        };
        let drift = expected.abs_diff(snapshot.position);
        drift > SEEK_TOLERANCE
            || (snapshot.state == PlaybackState::Playing && elapsed >= POSITION_REFRESH)
    }
}

impl Integration for MediaControls {
    fn update(&mut self, snapshot: &PlayerSnapshot) {
        if self.failed {
            return;
        }
        if let Err(error) = self.apply(snapshot) {
            log::warn!("media controls stopped: {}", describe(error));
            self.failed = true;
        }
    }
}

fn describe(error: souvlaki::Error) -> MediaControlsError {
    MediaControlsError(error.to_string())
}

fn to_metadata(track: &TrackInfo) -> MediaMetadata<'_> {
    MediaMetadata {
        title: non_empty(&track.title),
        album: non_empty(&track.album),
        artist: non_empty(&track.artist),
        cover_url: track.cover_url.as_deref(),
        duration: track.duration,
    }
}

fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

fn to_playback(snapshot: &PlayerSnapshot) -> MediaPlayback {
    let progress = Some(MediaPosition(snapshot.position));
    match snapshot.state {
        PlaybackState::Stopped => MediaPlayback::Stopped,
        PlaybackState::Paused => MediaPlayback::Paused { progress },
        PlaybackState::Playing => MediaPlayback::Playing { progress },
    }
}

fn to_command(event: MediaControlEvent) -> Option<PlayerCommand> {
    Some(match event {
        MediaControlEvent::Play => PlayerCommand::Play,
        MediaControlEvent::Pause => PlayerCommand::Pause,
        MediaControlEvent::Toggle => PlayerCommand::PlayPause,
        MediaControlEvent::Next => PlayerCommand::Next,
        MediaControlEvent::Previous => PlayerCommand::Previous,
        MediaControlEvent::Stop => PlayerCommand::Stop,
        MediaControlEvent::Seek(SeekDirection::Forward) => PlayerCommand::SeekForward(DEFAULT_SEEK),
        MediaControlEvent::Seek(SeekDirection::Backward) => PlayerCommand::SeekBackward(DEFAULT_SEEK),
        MediaControlEvent::SeekBy(SeekDirection::Forward, by) => PlayerCommand::SeekForward(by),
        MediaControlEvent::SeekBy(SeekDirection::Backward, by) => PlayerCommand::SeekBackward(by),
        MediaControlEvent::SetPosition(MediaPosition(position)) => PlayerCommand::SeekTo(position),
        MediaControlEvent::SetVolume(volume) => PlayerCommand::SetVolume(volume),
        MediaControlEvent::Raise => PlayerCommand::Raise,
        MediaControlEvent::Quit => PlayerCommand::Quit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_can_move_to_the_runner_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<MediaControls>();
    }
}
