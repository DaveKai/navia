//! OS media controls on Linux: an MPRIS server on zbus.
//!
//! Implements `org.mpris.MediaPlayer2` and `org.mpris.MediaPlayer2.Player`
//! (<https://specifications.freedesktop.org/mpris-spec/latest/>). Nothing polls. zbus serves D-Bus calls on its own
//! executor thread, and [`MediaControls::update`] only signals when something visible changed. `Position` is computed
//! on read from the last known position and elapsed time, and a jump sends `Seeked` as the spec asks.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

use crate::{Integration, LoopMode, PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot, TrackInfo};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// The spec's id for "no track".
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
/// How far the reported position may drift from the expected one before it counts as a seek, which clients hear about
/// through `Seeked`.
const SEEK_TOLERANCE: Duration = Duration::from_millis(1500);

pub struct MediaControlsConfig {
    /// The name shown by the desktop (`Identity`).
    pub display_name: String,
    /// The last part of the MPRIS bus name, like `navia`.
    pub dbus_name: String,
    /// The desktop file's name without `.desktop` (`DesktopEntry`), which is the app ID.
    pub desktop_entry: String,
    /// The window handle. Required on Windows and unused here.
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

/// What clients read, shared with the D-Bus interfaces.
struct State {
    track: Option<TrackInfo>,
    status: PlaybackState,
    /// Position at `at`. While playing it advances with the clock.
    position: Duration,
    at: Instant,
    volume: f64,
    loop_mode: LoopMode,
    shuffle: bool,
    can_change_modes: bool,
}

impl State {
    fn position_now(&self) -> Duration {
        let position = match self.status {
            PlaybackState::Playing => self.position + self.at.elapsed(),
            _ => self.position,
        };
        match self.track.as_ref().and_then(|t| t.duration) {
            Some(length) => position.min(length),
            None => position,
        }
    }

    fn track_id(&self) -> OwnedObjectPath {
        let path = self.track.as_ref().map_or_else(|| NO_TRACK.to_string(), |t| track_path(&t.id));
        ObjectPath::try_from(path).map(Into::into).unwrap_or_else(|_| ObjectPath::from_static_str_unchecked(NO_TRACK).into())
    }

    fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
        let mut map = HashMap::new();
        map.insert("mpris:trackid", Value::from(self.track_id()));
        let Some(track) = &self.track else { return map };
        if let Some(length) = track.duration {
            map.insert("mpris:length", Value::from(i64::try_from(length.as_micros()).unwrap_or(i64::MAX)));
        }
        if let Some(url) = &track.cover_url {
            map.insert("mpris:artUrl", Value::from(url.clone()));
        }
        if !track.title.is_empty() {
            map.insert("xesam:title", Value::from(track.title.clone()));
        }
        if !track.artist.is_empty() {
            map.insert("xesam:artist", Value::from(vec![track.artist.clone()]));
        }
        if !track.album.is_empty() {
            map.insert("xesam:album", Value::from(track.album.clone()));
        }
        map
    }
}

/// A D-Bus object path for a song id. ASCII letters and digits stay as they are, and every other byte, `_` included,
/// becomes `_xx` hex, so every id gets its own valid path.
fn track_path(id: &str) -> String {
    let mut path = String::from("/org/navia/track/t");
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            path.push(byte as char);
        } else {
            path.push_str(&format!("_{byte:02x}"));
        }
    }
    path
}

fn loop_text(mode: LoopMode) -> &'static str {
    match mode {
        LoopMode::None => "None",
        LoopMode::Track => "Track",
        LoopMode::Playlist => "Playlist",
    }
}

fn status_text(status: PlaybackState) -> &'static str {
    match status {
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Stopped => "Stopped",
    }
}

fn micros(d: Duration) -> i64 {
    i64::try_from(d.as_micros()).unwrap_or(i64::MAX)
}

struct Root {
    identity: String,
    desktop_entry: String,
    control: Arc<dyn PlayerControl>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    /// The app doesn't act on Raise yet, so `CanRaise` is false, but the call is still passed on.
    fn raise(&self) {
        self.control.command(PlayerCommand::Raise);
    }

    fn quit(&self) {
        self.control.command(PlayerCommand::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        self.desktop_entry.clone()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    state: Arc<Mutex<State>>,
    control: Arc<dyn PlayerControl>,
}

impl Player {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn has_track(&self) -> bool {
        self.state().track.is_some()
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.control.command(PlayerCommand::Next);
    }

    fn previous(&self) {
        self.control.command(PlayerCommand::Previous);
    }

    fn pause(&self) {
        self.control.command(PlayerCommand::Pause);
    }

    fn play_pause(&self) {
        self.control.command(PlayerCommand::PlayPause);
    }

    fn stop(&self) {
        self.control.command(PlayerCommand::Stop);
    }

    fn play(&self) {
        self.control.command(PlayerCommand::Play);
    }

    /// Relative, in microseconds.
    fn seek(&self, offset: i64) {
        if !self.has_track() {
            return;
        }
        let by = Duration::from_micros(offset.unsigned_abs());
        self.control.command(if offset >= 0 { PlayerCommand::SeekForward(by) } else { PlayerCommand::SeekBackward(by) });
    }

    /// Absolute, in microseconds. Ignored unless it names the current track and lands inside it.
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let state = self.state();
        if state.track.is_none() || track_id.as_str() != state.track_id().as_str() || position < 0 {
            return;
        }
        let to = Duration::from_micros(position as u64);
        if state.track.as_ref().and_then(|t| t.duration).is_some_and(|length| to > length) {
            return;
        }
        drop(state);
        self.control.command(PlayerCommand::SeekTo(to));
    }

    /// Navia doesn't open URIs, and `SupportedUriSchemes` is empty, so this does nothing.
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        status_text(self.state().status).to_string()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
        self.state().metadata()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state().volume
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        loop_text(self.state().loop_mode).to_string()
    }

    /// Unknown values are ignored, and so is any change while a station plays.
    #[zbus(property)]
    fn set_loop_status(&self, status: String) {
        let mode = match status.as_str() {
            "None" => LoopMode::None,
            "Track" => LoopMode::Track,
            "Playlist" => LoopMode::Playlist,
            _ => return,
        };
        if self.state().can_change_modes {
            self.control.command(PlayerCommand::SetLoop(mode));
        }
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.state().shuffle
    }

    #[zbus(property)]
    fn set_shuffle(&self, on: bool) {
        if self.state().can_change_modes {
            self.control.command(PlayerCommand::SetShuffle(on));
        }
    }

    #[zbus(property)]
    fn set_volume(&self, volume: f64) {
        self.control.command(PlayerCommand::SetVolume(volume.clamp(0.0, 1.0)));
    }

    /// Read on demand and never signaled, per the spec. Clients follow it through `Seeked`.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        micros(self.state().position_now())
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.has_track()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

pub struct MediaControls {
    state: Arc<Mutex<State>>,
    connection: Arc<OnceLock<Connection>>,
    failed: Arc<AtomicBool>,
    bus_name: String,
    /// What clients were last told, so only changes are sent.
    sent: Option<PlayerSnapshot>,
}

impl MediaControls {
    /// Connects to the session bus on a background thread. If there's no bus, or another instance owns the name,
    /// updates stop quietly and a warning is logged.
    pub fn new(config: MediaControlsConfig, control: Arc<dyn PlayerControl>) -> Result<Self, MediaControlsError> {
        let state = Arc::new(Mutex::new(State { track: None, status: PlaybackState::Stopped, position: Duration::ZERO, at: Instant::now(), volume: 1.0, loop_mode: LoopMode::None, shuffle: false, can_change_modes: false }));
        let connection = Arc::new(OnceLock::new());
        let failed = Arc::new(AtomicBool::new(false));
        let bus_name = format!("org.mpris.MediaPlayer2.{}", config.dbus_name);
        let root = Root { identity: config.display_name, desktop_entry: config.desktop_entry, control: control.clone() };
        let player = Player { state: state.clone(), control };
        let (slot, flag, name) = (connection.clone(), failed.clone(), bus_name.clone());
        std::thread::Builder::new()
            .name("navia-mpris-setup".into())
            .spawn(move || match connect(&name, root, player) {
                Ok(c) => {
                    let _ = slot.set(c);
                }
                Err(e) => {
                    log::warn!("media controls unavailable: {e}");
                    flag.store(true, Ordering::Release);
                }
            })
            .map_err(|e| MediaControlsError(e.to_string()))?;
        Ok(Self { state, connection, failed, bus_name, sent: None })
    }

    /// True once the D-Bus service has failed and updates have stopped.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    fn apply(&mut self, snapshot: &PlayerSnapshot) -> zbus::Result<()> {
        let mut changed: HashMap<&'static str, Value<'static>> = HashMap::new();
        let mut seeked = None;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let expected = state.position_now();
            let track_changed = state.track != snapshot.track;
            let had_track = state.track.is_some();
            if track_changed {
                state.track = snapshot.track.clone();
            }
            let status_changed = state.status != snapshot.state;
            let jumped = !track_changed && !status_changed && expected.abs_diff(snapshot.position) > SEEK_TOLERANCE;
            // Refresh the anchor on every report so the extrapolation never drifts.
            state.status = snapshot.state;
            state.position = snapshot.position;
            state.at = Instant::now();
            let volume_changed = state.volume != snapshot.volume;
            state.volume = snapshot.volume;
            if self.sent.is_none() || track_changed {
                changed.insert("Metadata", Value::from(state.metadata()));
            }
            if self.sent.is_none() || had_track != state.track.is_some() {
                let can = Value::from(state.track.is_some());
                for name in ["CanPlay", "CanPause", "CanSeek", "CanGoNext", "CanGoPrevious"] {
                    changed.insert(name, can.try_clone().expect("plain value"));
                }
            }
            if self.sent.is_none() || status_changed {
                changed.insert("PlaybackStatus", Value::from(status_text(state.status)));
            }
            if self.sent.is_none() || volume_changed {
                changed.insert("Volume", Value::from(state.volume));
            }
            state.can_change_modes = snapshot.can_change_modes;
            if self.sent.is_none() || state.loop_mode != snapshot.loop_mode {
                state.loop_mode = snapshot.loop_mode;
                changed.insert("LoopStatus", Value::from(loop_text(state.loop_mode)));
            }
            if self.sent.is_none() || state.shuffle != snapshot.shuffle {
                state.shuffle = snapshot.shuffle;
                changed.insert("Shuffle", Value::from(state.shuffle));
            }
            if jumped {
                seeked = Some(micros(state.position_now()));
            }
        }
        let Some(connection) = self.connection.get() else {
            // Not connected yet. Clients read the state when they connect, so nothing is lost.
            return Ok(());
        };
        if self.sent.is_none() {
            self.sent = Some(snapshot.clone());
        }
        if !changed.is_empty() {
            connection.emit_signal(None::<()>, PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged", &(PLAYER, changed, Vec::<&str>::new()))?;
        }
        if let Some(position) = seeked {
            connection.emit_signal(None::<()>, PATH, PLAYER, "Seeked", &position)?;
        }
        self.sent = Some(snapshot.clone());
        Ok(())
    }
}

fn connect(name: &str, root: Root, player: Player) -> zbus::Result<Connection> {
    let connection = Builder::session()?.serve_at(PATH, root)?.serve_at(PATH, player)?.build()?;
    // A second instance doesn't take over the name or wait for it. It runs without media controls.
    match connection.request_name_with_flags(name, RequestNameFlags::DoNotQueue.into())? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(connection),
        _ => Err(zbus::Error::Failure(format!("{name} is already taken, probably by another Navia"))),
    }
}

impl Integration for MediaControls {
    fn update(&mut self, snapshot: &PlayerSnapshot) {
        if self.failed() {
            return;
        }
        if self.sent.as_ref() == Some(snapshot) {
            return;
        }
        if let Err(error) = self.apply(snapshot) {
            log::warn!("media controls stopped: {error}");
            self.failed.store(true, Ordering::Release);
        }
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.get() {
            let _ = connection.release_name(self.bus_name.as_str());
            let _ = connection.object_server().remove::<Player, _>(PATH);
            let _ = connection.object_server().remove::<Root, _>(PATH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_can_move_to_the_runner_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<MediaControls>();
    }

    #[test]
    fn every_song_id_makes_a_valid_distinct_path() {
        for id in ["1", "abc-DEF", "ö/ü", "", "a_b", "a-b"] {
            ObjectPath::try_from(track_path(id)).unwrap();
        }
        assert_ne!(track_path("a_b"), track_path("a-b"));
        assert_ne!(track_path("a_2d"), track_path("a-"));
    }
}
