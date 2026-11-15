//! Opt-in listen-along between users of the same server.
//! Sharing Navias mark their client name with [`SHARE_MARK`], and following needs the `playbackReport` extension.
//! Everything here is pure, and the apps call [`fetch`] on their own timers.

use std::time::Duration;

use serde_json::Value;

use crate::client::{Client, Song};
use crate::error::Result;

/// Ends the client name of a sharing Navia. A zero-width space, so it's invisible where the name is shown.
pub const SHARE_MARK: char = '\u{200B}';
/// Follows the share mark when this Navia lets listeners control playback. A zero-width non-joiner.
pub const CONTROL_MARK: char = '\u{200C}';
/// Follows the share mark when this Navia is listening along with someone. A zero-width joiner.
pub const FOLLOW_MARK: char = '\u{200D}';

/// How often Listening Now is checked, and how often while following.
pub const LIST_POLL: Duration = Duration::from_secs(15);
pub const FOLLOW_POLL: Duration = Duration::from_secs(5);
/// Following seeks only when the two players are further apart than this.
pub const RESYNC_DRIFT: Duration = Duration::from_secs(2);
/// How long the last good Listening Now is kept while checks fail, about two missed checks.
pub const LISTING_KEEP: Duration = Duration::from_secs(45);
/// How long "<user> stopped listening" shows after following ends by itself.
pub const ENDED_NOTICE: Duration = Duration::from_secs(6);

/// A session's state, where the server has `playbackReport`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Playing,
    Paused,
}

impl SessionState {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "starting" => Some(Self::Starting),
            "playing" => Some(Self::Playing),
            "paused" => Some(Self::Paused),
            _ => None,
        }
    }

    /// Starting counts as playing.
    pub fn is_playing(self) -> bool {
        self != Self::Paused
    }
}

/// One `getNowPlaying` entry. `state` and `position_ms` are `None` without `playbackReport`.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub user: String,
    pub player: String,
    pub song: Song,
    pub state: Option<SessionState>,
    pub position_ms: Option<u64>,
    pub minutes_ago: u32,
}

impl NowPlaying {
    /// Parses one entry, skipping entries without a song or user.
    /// `live` says whether the server has `playbackReport`. Without it, state and position are ignored.
    pub fn from_entry(entry: &Value, live: bool) -> Option<Self> {
        let song: Song = serde_json::from_value(entry.clone()).ok().filter(|s: &Song| !s.id.is_empty())?;
        let user = entry.get("username").and_then(Value::as_str)?.to_string();
        let player = entry.get("playerName").and_then(Value::as_str).unwrap_or_default().to_string();
        let state = if live { entry.get("state").and_then(Value::as_str).and_then(SessionState::parse) } else { None };
        let position_ms = if live && state.is_some() { entry.get("positionMs").and_then(Value::as_u64) } else { None };
        let minutes_ago = entry.get("minutesAgo").and_then(Value::as_u64).unwrap_or(0) as u32;
        Some(Self { user, player, song, state, position_ms, minutes_ago })
    }

    /// Shared from a Navia with "Share what I'm playing" on.
    pub fn shared(&self) -> bool {
        self.player.contains(SHARE_MARK)
    }

    /// Shared from a Navia with "Let listeners control playback" on.
    pub fn allows_control(&self) -> bool {
        self.shared() && self.player.contains(CONTROL_MARK)
    }

    /// Shared from a Navia that is listening along with someone.
    pub fn follows(&self) -> bool {
        self.shared() && self.player.contains(FOLLOW_MARK)
    }

    /// Whether this person can be followed, which needs the server to say what plays and where.
    pub fn can_listen_along(&self) -> bool {
        self.state.is_some() && self.position_ms.is_some()
    }
}

/// Fetches and parses `getNowPlaying`. `live` says whether the server has `playbackReport`.
pub async fn fetch(client: &Client, live: bool) -> Result<Vec<NowPlaying>> {
    Ok(client.now_playing().await?.iter().filter_map(|e| NowPlaying::from_entry(e, live)).collect())
}

/// Reads `getNowPlaying` entries from a JSON array, so tests can check following without real people.
pub fn from_json(bytes: &[u8]) -> Vec<NowPlaying> {
    let entries: Vec<Value> = serde_json::from_slice(bytes).unwrap_or_default();
    entries.iter().filter_map(|e| NowPlaying::from_entry(e, true)).collect()
}

fn same_user(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Listening Now for `viewer`, with one row per other person who shares, newest first.
/// Empty unless `active` is true.
pub fn people(entries: &[NowPlaying], viewer: &str, active: bool) -> Vec<NowPlaying> {
    if !active {
        return Vec::new();
    }
    let mut out: Vec<NowPlaying> = Vec::new();
    for e in entries.iter().filter(|e| e.shared() && !same_user(&e.user, viewer)) {
        match out.iter().position(|o| same_user(&o.user, &e.user)) {
            None => out.push(e.clone()),
            Some(at) => {
                let have_playing = out[at].state.is_some_and(SessionState::is_playing);
                if !have_playing && e.state.is_some_and(SessionState::is_playing) {
                    out[at] = e.clone();
                }
            }
        }
    }
    out
}

/// Who is being followed, and the player last followed since they may have two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Followed {
    pub user: String,
    pub player: String,
}

/// What the listener's own player is doing while following.
///
/// Play and pause are shared both ways, and whoever changed last wins. Song changes, skips and
/// seeks stay with the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Local {
    pub song_id: String,
    pub position_ms: u64,
    pub playing: bool,
    /// The listener paused it themselves. It stays paused until they play again or the host changes state after them.
    pub held: bool,
    /// When the listener last pressed play or pause while following, in Unix ms.
    pub own_change_ms: Option<u64>,
    /// The followed player's playing state at the previous check, and when it was read.
    pub seen: Option<(bool, u64)>,
}

/// What one check while following asks of the listener's player.
#[derive(Clone, Debug, PartialEq)]
pub enum FollowStep {
    /// They stopped, went quiet or can't be followed anymore.
    /// Keep the song, pause and end following with "<user> stopped listening".
    Ended,
    /// Play `entry`'s song from its position, matching its play state.
    /// Sent for a song change, a pause or resume, or drift past [`RESYNC_DRIFT`].
    Resync { entry: NowPlaying, player: String },
    /// In step. `drift_ms` is how far ahead the listener is, negative when behind.
    Keep { drift_ms: i64, player: String },
    /// A followed Spotify friend plays a song that isn't in the library.
    /// Pause, say it was skipped once per `key`, and keep following.
    NotInLibrary { key: String, song: String, artist: String },
}

/// The followed person's shared session in a `getNowPlaying` answer.
/// Prefers the same player, then one that's playing, then the newest.
pub fn followed_entry<'a>(followed: &Followed, entries: &'a [NowPlaying]) -> Option<&'a NowPlaying> {
    let theirs: Vec<&NowPlaying> = entries.iter().filter(|e| e.shared() && same_user(&e.user, &followed.user)).collect();
    theirs
        .iter()
        .find(|e| e.player == followed.player)
        .or_else(|| theirs.iter().find(|e| e.state.is_some_and(SessionState::is_playing)))
        .or_else(|| theirs.first())
        .copied()
}

/// One check while following `followed`, from a fresh `getNowPlaying` answer.
///
/// When play states differ, the newer of the listener's change and the host's change wins.
pub fn follow_step(followed: &Followed, entries: &[NowPlaying], local: &Local) -> FollowStep {
    let Some(entry) = followed_entry(followed, entries).filter(|e| e.can_listen_along()) else { return FollowStep::Ended };
    let keep = |drift_ms| FollowStep::Keep { drift_ms, player: entry.player.clone() };
    let at = entry.position_ms.unwrap_or(0);
    let playing = entry.state.is_some_and(SessionState::is_playing);
    let drift_ms = local.position_ms as i64 - at as i64;
    let differs = entry.song.id != local.song_id || playing != local.playing || drift_ms.unsigned_abs() > RESYNC_DRIFT.as_millis() as u64;
    // After the listener's own change, wait for the host to mirror it.
    // Songs and seeks are only mirrored where the host allows control.
    let mine_newer = local.own_change_ms.is_some_and(|c| local.seen.is_none_or(|(_, at)| c > at));
    if differs && mine_newer {
        return keep(0);
    }
    if entry.song.id != local.song_id {
        return FollowStep::Resync { entry: entry.clone(), player: entry.player.clone() };
    }
    if playing != local.playing {
        let host_changed = local.seen.is_some_and(|(was, _)| was != playing);
        if local.held && !host_changed {
            return keep(0);
        }
        return FollowStep::Resync { entry: entry.clone(), player: entry.player.clone() };
    }
    if drift_ms.unsigned_abs() > RESYNC_DRIFT.as_millis() as u64 {
        return FollowStep::Resync { entry: entry.clone(), player: entry.player.clone() };
    }
    keep(drift_ms)
}

/// Someone listening along with this Navia, as the host sees them in `getNowPlaying`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    pub user: String,
    pub player: String,
    pub song_id: String,
    pub position_ms: u64,
    pub playing: bool,
    /// On the host's song at this look.
    pub in_step: bool,
    /// When this was read, in Unix ms.
    pub seen_ms: u64,
}

/// The host's own player, for [`listeners`] and [`host_mirror`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub song_id: String,
    pub position_ms: u64,
    pub playing: bool,
    /// When the host's user last pressed play or pause, seeked or played something, in Unix ms.
    /// Mirroring a listener doesn't count.
    pub own_change_ms: u64,
}

/// What the host does to mirror a listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAction {
    Pause,
    Play,
    Seek(u64),
    /// Play this song from this position, then the host's queue carries on.
    Song { song_id: String, position_ms: u64, playing: bool },
}

impl Listener {
    fn same(&self, other: &Listener) -> bool {
        self.player == other.player && same_user(&self.user, &other.user)
    }
}

/// Who listens along with `viewer` now.
/// That's other people's shared sessions marked as following and on the host's song, now or at the last look.
pub fn listeners(entries: &[NowPlaying], viewer: &str, host: &Host, prev: &[Listener], now_ms: u64) -> Vec<Listener> {
    let slack = (RESYNC_DRIFT + FOLLOW_POLL).as_millis() as u64;
    entries
        .iter()
        .filter(|e| e.follows() && !same_user(&e.user, viewer) && e.can_listen_along())
        .filter_map(|e| {
            let playing = e.state.is_some_and(SessionState::is_playing);
            let position_ms = e.position_ms.unwrap_or(0);
            let on_song = e.song.id == host.song_id;
            let in_step = on_song && playing == host.playing && position_ms.abs_diff(host.position_ms) <= slack;
            let known = prev.iter().any(|p| p.player == e.player && same_user(&p.user, &e.user) && (p.in_step || on_song));
            (known || in_step).then(|| Listener { user: e.user.clone(), player: e.player.clone(), song_id: e.song.id.clone(), position_ms, playing, in_step: on_song, seen_ms: now_ms })
        })
        .collect()
}

/// What the host does to mirror its listeners, if anything.
///
/// The first listener who changed since the last look wins, unless the host changed after that look.
/// Nothing is mirrored unless `control` is true.
pub fn host_mirror(host: &Host, prev: &[Listener], now: &[Listener], control: bool) -> Option<HostAction> {
    if !control {
        return None;
    }
    let drift = RESYNC_DRIFT.as_millis() as u64;
    for n in now {
        let Some(p) = prev.iter().find(|p| p.same(n)) else { continue };
        if host.own_change_ms > p.seen_ms {
            continue;
        }
        if n.song_id != p.song_id {
            if n.song_id != host.song_id {
                return Some(HostAction::Song { song_id: n.song_id.clone(), position_ms: n.position_ms, playing: n.playing });
            }
            continue;
        }
        if n.song_id != host.song_id {
            continue;
        }
        // How far plain play could have taken them since the last look. Anything beyond is a seek.
        let elapsed = if p.playing || n.playing { n.seen_ms.saturating_sub(p.seen_ms) } else { 0 };
        let jumped = n.position_ms + drift < p.position_ms || n.position_ms > p.position_ms + elapsed + drift;
        if jumped && n.position_ms.abs_diff(host.position_ms) > drift {
            return Some(HostAction::Seek(n.position_ms));
        }
        if p.playing != n.playing && n.playing != host.playing {
            return Some(if n.playing { HostAction::Play } else { HostAction::Pause });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn marked() -> String {
        format!("navia{SHARE_MARK}")
    }

    fn entry(user: &str, player: &str, song: &str, state: Option<&str>, pos: Option<u64>) -> NowPlaying {
        let mut v = json!({ "id": song, "title": format!("Song {song}"), "username": user, "playerName": player, "minutesAgo": 0 });
        if let Some(s) = state {
            v["state"] = json!(s);
        }
        if let Some(p) = pos {
            v["positionMs"] = json!(p);
        }
        NowPlaying::from_entry(&v, true).unwrap()
    }

    fn local(song: &str, pos: u64, playing: bool) -> Local {
        Local { song_id: song.into(), position_ms: pos, playing, ..Default::default() }
    }

    #[test]
    fn entries_parse_with_and_without_playback_report() {
        let v = json!({ "id": "s1", "title": "One", "albumId": "al", "coverArt": "al", "username": "Alex", "playerName": marked(), "minutesAgo": 3, "state": "paused", "positionMs": 4100 });
        let e = NowPlaying::from_entry(&v, true).unwrap();
        assert_eq!((e.user.as_str(), e.state, e.position_ms, e.minutes_ago), ("Alex", Some(SessionState::Paused), Some(4100), 3));
        assert!(e.shared() && e.can_listen_along());
        assert_eq!(e.song.album_id.as_deref(), Some("al"));
        // A server without playbackReport is still listed, but without a live mark or Listen Along.
        let legacy = NowPlaying::from_entry(&v, false).unwrap();
        assert_eq!((legacy.state, legacy.position_ms), (None, None));
        assert!(legacy.shared() && !legacy.can_listen_along());
        // Entries with no user or no song are skipped.
        assert!(NowPlaying::from_entry(&json!({ "id": "s", "title": "t" }), true).is_none());
        assert!(NowPlaying::from_entry(&json!({ "username": "x" }), true).is_none());
    }

    #[test]
    fn listening_now_shows_other_sharers_once_each() {
        let m = marked();
        let entries = vec![
            entry("Alex", &m, "a1", Some("paused"), Some(5)),
            entry("Jo", "navia", "j1", Some("playing"), Some(5)),
            entry("Sam", "symfonium", "x1", Some("playing"), Some(5)),
            entry("me", &m, "m1", Some("playing"), Some(5)),
            entry("ME", &m, "m2", Some("playing"), Some(5)),
            entry("alex", &m, "a2", Some("playing"), Some(9)),
            entry("Mira", &m, "r1", Some("paused"), Some(5)),
            entry("Mira", &m, "r2", Some("paused"), Some(5)),
        ];
        let shown = people(&entries, "Me", true);
        let rows: Vec<(&str, &str)> = shown.iter().map(|e| (e.user.as_str(), e.song.id.as_str())).collect();
        // Only marked players of other people show, one row each, preferring the playing player.
        assert_eq!(rows, vec![("alex", "a2"), ("Mira", "r1")]);
        // Nobody shows with the viewer's own sharing off or in Private mode.
        assert!(people(&entries, "Me", false).is_empty());
        assert!(people(&[], "Me", true).is_empty());
    }

    #[test]
    fn following_resyncs_on_changes_and_drift_only() {
        let m = marked();
        let f = Followed { user: "Alex".into(), player: m.clone() };
        let now = |song: &str, state: &str, pos: u64| vec![entry("Alex", &m, song, Some(state), Some(pos))];
        // In step, a second apart, so no jump.
        assert_eq!(follow_step(&f, &now("a1", "playing", 61_000), &local("a1", 62_000, true)), FollowStep::Keep { drift_ms: 1000, player: m.clone() });
        // More than 2 s apart either way resyncs.
        assert!(matches!(follow_step(&f, &now("a1", "playing", 61_000), &local("a1", 63_500, true)), FollowStep::Resync { .. }));
        assert!(matches!(follow_step(&f, &now("a1", "playing", 61_000), &local("a1", 58_000, true)), FollowStep::Resync { .. }));
        // A new song, a pause or a resume resyncs at once, whatever the drift.
        assert!(matches!(follow_step(&f, &now("a2", "playing", 1000), &local("a1", 1000, true)), FollowStep::Resync { .. }));
        assert!(matches!(follow_step(&f, &now("a1", "paused", 61_000), &local("a1", 61_000, true)), FollowStep::Resync { .. }));
        assert!(matches!(follow_step(&f, &now("a1", "playing", 61_000), &local("a1", 61_000, false)), FollowStep::Resync { .. }));
        // Starting counts as playing.
        assert!(matches!(follow_step(&f, &now("a1", "starting", 0), &local("a1", 500, true)), FollowStep::Keep { .. }));
        match follow_step(&f, &now("a2", "paused", 7000), &local("a1", 1000, true)) {
            FollowStep::Resync { entry, .. } => assert_eq!((entry.song.id.as_str(), entry.position_ms, entry.state), ("a2", Some(7000), Some(SessionState::Paused))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_listeners_pause_holds_and_waits_for_the_host_to_mirror_it() {
        let m = marked();
        let f = Followed { user: "Alex".into(), player: m.clone() };
        let playing = |pos| vec![entry("Alex", &m, "a1", Some("playing"), Some(pos))];
        let paused = |pos| vec![entry("Alex", &m, "a1", Some("paused"), Some(pos))];
        // The listener paused at t=110 after last seeing the host playing at t=100, so wait for the mirror.
        let mine = Local { held: true, own_change_ms: Some(110), seen: Some((true, 100)), ..local("a1", 30_000, false) };
        assert!(matches!(follow_step(&f, &playing(31_000), &mine), FollowStep::Keep { .. }));
        // The host hasn't mirrored by the next check at 115, so the listener's pause still stands.
        let later = Local { seen: Some((true, 115)), ..mine.clone() };
        assert!(matches!(follow_step(&f, &playing(36_000), &later), FollowStep::Keep { .. }));
        // The host mirrored, so they're in step and paused.
        assert!(matches!(follow_step(&f, &paused(30_500), &later), FollowStep::Keep { .. }));
        // The host plays again after that, seen paused at 120, so the host's change wins.
        let host_resumed = Local { seen: Some((false, 120)), ..later.clone() };
        match follow_step(&f, &playing(30_600), &host_resumed) {
            FollowStep::Resync { entry, .. } => assert_eq!(entry.state, Some(SessionState::Playing)),
            other => panic!("{other:?}"),
        }
        // A host song change after the listener's last change is followed, held or not.
        assert!(matches!(follow_step(&f, &[entry("Alex", &m, "a2", Some("playing"), Some(0))], &later), FollowStep::Resync { .. }));
        // The listener's own song change with control on waits for the host to mirror it.
        let picked = Local { held: false, own_change_ms: Some(130), seen: Some((true, 125)), ..local("b9", 1_000, true) };
        assert!(matches!(follow_step(&f, &playing(40_000), &picked), FollowStep::Keep { .. }));
        // The listener plays at 130 while the host is paused, so wait for the host to resume.
        let resume = Local { held: false, own_change_ms: Some(130), seen: Some((false, 125)), ..local("a1", 30_500, true) };
        assert!(matches!(follow_step(&f, &paused(30_500), &resume), FollowStep::Keep { .. }));
        // If they stop, following still ends.
        assert_eq!(follow_step(&f, &[], &mine), FollowStep::Ended);
    }

    fn host(song: &str, pos: u64, playing: bool, changed: u64) -> Host {
        Host { song_id: song.into(), position_ms: pos, playing, own_change_ms: changed }
    }

    fn follower() -> String {
        format!("navia{SHARE_MARK}{FOLLOW_MARK}")
    }

    #[test]
    fn marks_read_back() {
        let e = entry("Jo", &format!("navia{SHARE_MARK}{CONTROL_MARK}"), "a1", Some("playing"), Some(0));
        assert!(e.shared() && e.allows_control() && !e.follows());
        assert!(entry("Jo", &follower(), "a1", Some("playing"), Some(0)).follows());
        assert!(!entry("Jo", "navia", "a1", Some("playing"), Some(0)).shared());
        assert_eq!(crate::client::client_display_name(&format!("navia{SHARE_MARK}{CONTROL_MARK}{FOLLOW_MARK}")), "Navia");
    }

    /// Players named by the current build ("Navia") and by older ones ("navia") are read alike.
    #[test]
    fn old_and_new_client_names_share_alike() {
        let old = entry("Jo", &format!("navia{SHARE_MARK}{CONTROL_MARK}"), "a1", Some("playing"), Some(0));
        let new = entry("Sam", &format!("Navia{SHARE_MARK}{CONTROL_MARK}"), "b1", Some("playing"), Some(0));
        for e in [&old, &new] {
            assert!(e.shared() && e.allows_control());
            assert!(crate::client::is_navia_client(&e.player));
        }
        assert_eq!(people(&[old, new], "me", true).len(), 2);
        assert!(!entry("Jo", "Navia", "a1", Some("playing"), Some(0)).shared());
    }

    #[test]
    fn the_host_mirrors_a_listeners_pause_and_resume() {
        let m = follower();
        let h = host("a1", 60_000, true, 10);
        // Jo follows and plays the host's song in step, so Jo is a listener. Sam plays another song,
        // and Kim plays the same song without following anyone, so neither is one.
        let first = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(59_000)), entry("Sam", &m, "b1", Some("playing"), Some(60_000)), entry("Kim", &marked(), "a1", Some("playing"), Some(60_000))], "Me", &h, &[], 100_000);
        assert_eq!(first.iter().map(|l| l.user.as_str()).collect::<Vec<_>>(), ["Jo"]);
        assert_eq!(host_mirror(&h, &[], &first, true), None);
        // Jo pauses, and the host pauses too only with control on.
        let h5 = host("a1", 65_000, true, 10);
        let second = listeners(&[entry("Jo", &m, "a1", Some("paused"), Some(62_000))], "Me", &h5, &first, 105_000);
        assert_eq!(second.len(), 1, "still a listener while paused, though out of step");
        assert_eq!(host_mirror(&h5, &first, &second, false), None);
        assert_eq!(host_mirror(&h5, &first, &second, true), Some(HostAction::Pause));
        // The host is paused now by a mirror, so its own change time stays. Jo resumes and the host resumes.
        let h_paused = host("a1", 65_000, false, 10);
        let third = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(62_500))], "Me", &h_paused, &second, 110_000);
        assert_eq!(host_mirror(&h_paused, &second, &third, true), Some(HostAction::Play));
    }

    #[test]
    fn the_host_mirrors_a_listeners_song_and_seek() {
        let m = follower();
        let h = host("a1", 60_000, true, 10);
        let first = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(60_000))], "Me", &h, &[], 100_000);
        // Jo plays another song, and the host plays it from Jo's position.
        let h2 = host("a1", 65_000, true, 10);
        let song = listeners(&[entry("Jo", &m, "b7", Some("playing"), Some(3_000))], "Me", &h2, &first, 105_000);
        assert_eq!(song.len(), 1, "a listener who changed song is still one");
        assert_eq!(host_mirror(&h2, &first, &song, true), Some(HostAction::Song { song_id: "b7".into(), position_ms: 3_000, playing: true }));
        // Jo seeks to 2:00 on the host's song, and the host seeks.
        let seek = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(120_000))], "Me", &h2, &first, 105_000);
        assert_eq!(host_mirror(&h2, &first, &seek, true), Some(HostAction::Seek(120_000)));
        // Plain play moving on 5 s is no seek.
        let on = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(65_000))], "Me", &h2, &first, 105_000);
        assert_eq!(host_mirror(&h2, &first, &on, true), None);
        // Someone off the host's song at two looks in a row isn't a listener anymore.
        let drifted = listeners(&[entry("Jo", &m, "c1", Some("playing"), Some(0))], "Me", &h2, &[Listener { in_step: false, ..song[0].clone() }], 110_000);
        assert!(drifted.is_empty());
    }

    #[test]
    fn two_listeners_in_one_check_have_one_winner_and_nothing_loops() {
        let m = follower();
        let h = host("a1", 60_000, true, 10);
        let first = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(60_000)), entry("Ana", &m, "a1", Some("playing"), Some(60_000))], "Me", &h, &[], 100_000);
        // Jo plays b7 and Ana pauses within the same check. Only the first listener's change is mirrored.
        let h2 = host("a1", 65_000, true, 10);
        let both = listeners(&[entry("Jo", &m, "b7", Some("playing"), Some(1_000)), entry("Ana", &m, "a1", Some("paused"), Some(62_000))], "Me", &h2, &first, 105_000);
        assert_eq!(host_mirror(&h2, &first, &both, true), Some(HostAction::Song { song_id: "b7".into(), position_ms: 1_000, playing: true }));
        // At the next look the host plays b7. Ana's pause is old news and she follows the host to b7,
        // so there's nothing more to mirror.
        let h3 = host("b7", 5_000, true, 10);
        let after = listeners(&[entry("Jo", &m, "b7", Some("playing"), Some(5_000)), entry("Ana", &m, "a1", Some("paused"), Some(62_000))], "Me", &h3, &both, 110_000);
        assert_eq!(host_mirror(&h3, &both, &after, true), None);
        // On Ana's side, her pause was before her last look and the host changed song since, so she follows.
        let ana = Local { held: true, own_change_ms: Some(104_000), seen: Some((true, 106_000)), ..local("a1", 62_000, false) };
        let f = Followed { user: "Me".into(), player: marked() };
        assert!(matches!(follow_step(&f, &[entry("Me", &marked(), "b7", Some("playing"), Some(5_000))], &ana), FollowStep::Resync { .. }));
    }

    #[test]
    fn nothing_bounces_back_and_the_last_change_wins() {
        let m = follower();
        let prev = vec![Listener { user: "Jo".into(), player: m.clone(), song_id: "a1".into(), position_ms: 60_000, playing: true, in_step: true, seen_ms: 100_000 }];
        // The host's user paused at 102 and Jo paused too, so the state matches and nothing happens.
        let h = host("a1", 60_000, false, 102_000);
        let now = listeners(&[entry("Jo", &m, "a1", Some("paused"), Some(60_000))], "Me", &h, &prev, 105_000);
        assert_eq!(host_mirror(&h, &prev, &now, true), None);
        // Both toggled within one check from the same state, so both are paused and nothing is mirrored.
        let jo = Local { held: true, own_change_ms: Some(103_000), seen: Some((true, 100_000)), ..local("a1", 60_000, false) };
        let f = Followed { user: "Me".into(), player: marked() };
        assert!(matches!(follow_step(&f, &[entry("Me", &marked(), "a1", Some("paused"), Some(60_000))], &jo), FollowStep::Keep { .. }));
        // Jo resumed at 104 but the host's user paused at 106, after the host last looked at Jo at 105.
        // The host's change is newer, so Jo follows the host's pause.
        let h2 = host("a1", 60_000, false, 106_000);
        let jo_seen_paused = vec![Listener { playing: false, seen_ms: 105_000, ..prev[0].clone() }];
        let jo_now = listeners(&[entry("Jo", &m, "a1", Some("playing"), Some(60_500))], "Me", &h2, &jo_seen_paused, 110_000);
        assert_eq!(host_mirror(&h2, &jo_seen_paused, &jo_now, true), None);
        let jo_local = Local { held: false, own_change_ms: Some(104_000), seen: Some((true, 105_000)), ..local("a1", 60_500, true) };
        match follow_step(&f, &[entry("Me", &marked(), "a1", Some("paused"), Some(60_000))], &jo_local) {
            FollowStep::Resync { entry, .. } => assert_eq!(entry.state, Some(SessionState::Paused)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn following_ends_when_they_stop_or_cant_be_followed() {
        let m = marked();
        let f = Followed { user: "Alex".into(), player: m.clone() };
        let here = local("a1", 1000, true);
        // Gone from the answer, stopped or quiet too long.
        assert_eq!(follow_step(&f, &[], &here), FollowStep::Ended);
        // Still playing but no longer shared, or another app's session, so not theirs to follow.
        assert_eq!(follow_step(&f, &[entry("Alex", "navia", "a1", Some("playing"), Some(1000))], &here), FollowStep::Ended);
        // A legacy answer has no state or position, so it can't be followed.
        let legacy = NowPlaying { state: None, position_ms: None, ..entry("Alex", &m, "a1", Some("playing"), Some(1000)) };
        assert_eq!(follow_step(&f, &[legacy], &here), FollowStep::Ended);
    }

    #[test]
    fn following_keeps_to_the_same_player_or_the_one_playing() {
        let m = marked();
        let other = format!("Navia{SHARE_MARK}");
        let f = Followed { user: "Alex".into(), player: m.clone() };
        // With two shared players, the followed one wins even when paused.
        let both = vec![entry("Alex", &other, "b1", Some("playing"), Some(1000)), entry("Alex", &m, "a1", Some("paused"), Some(1000))];
        assert!(matches!(follow_step(&f, &both, &local("a1", 1000, false)), FollowStep::Keep { .. }));
        // When the followed player is gone, the one that plays takes over.
        let moved = vec![entry("Alex", &other, "b1", Some("paused"), Some(1000)), entry("alex", &other, "b2", Some("playing"), Some(3000))];
        match follow_step(&f, &moved, &local("a1", 1000, true)) {
            FollowStep::Resync { entry, player } => assert_eq!((entry.song.id.as_str(), player), ("b2", other)),
            other => panic!("{other:?}"),
        }
    }
}
