//! Following Spotify friends through [Lanyard](https://github.com/Phineas/lanyard).
//! Navia reads only a friend's Discord presence from Lanyard and matches the song in the local library.
//! No Spotify API is used, and nothing contacts Lanyard until a friend is added.

pub mod matching;
pub mod watch;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::Song;
use crate::listen_along::{FollowStep, Followed, Local, NowPlaying, RESYNC_DRIFT, SessionState};

/// Lanyard's public instance. A self-hosted one is set in Settings instead.
pub const PUBLIC_ADDRESS: &str = "https://api.lanyard.rest";
/// Where a friend joins for Lanyard to see their presence.
pub const PROJECT_URL: &str = "https://github.com/Phineas/lanyard";
/// How long a friend's Spotify activity may be gone before following ends.
/// Spotify drops it while paused and between some songs.
pub const QUIET_GRACE: Duration = Duration::from_secs(10);
/// The player name a followed Spotify friend gets in [`Followed`], with their Discord id after it.
const PLAYER_PREFIX: &str = "lanyard:";

/// A friend the listener added, with their Discord user id and an optional display name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friend {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A Discord user id as typed, 17 to 20 digits after trimming spaces. `None` otherwise.
pub fn parse_friend_id(text: &str) -> Option<String> {
    let id = text.trim();
    (17..=20).contains(&id.len()).then_some(id).filter(|id| id.bytes().all(|b| b.is_ascii_digit())).map(str::to_string)
}

/// The Lanyard address as typed. Empty means the public one, otherwise an `http` or `https` address
/// with a host. `None` when it isn't valid.
pub fn parse_address(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return Some(PUBLIC_ADDRESS.to_string());
    }
    let url = url::Url::parse(text).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) || url.query().is_some() {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_string())
}

/// Whether a Lanyard address is on this machine, the only kind test runs contact.
pub fn is_loopback(address: &str) -> bool {
    url::Url::parse(address).ok().is_some_and(|u| match u.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    })
}

/// The [`Followed::player`] of a Spotify friend.
pub fn friend_player(id: &str) -> String {
    format!("{PLAYER_PREFIX}{id}")
}

/// The friend's Discord id when `player` comes from [`friend_player`].
pub fn friend_of(player: &str) -> Option<&str> {
    player.strip_prefix(PLAYER_PREFIX)
}

/// The friend's Discord account, as Lanyard gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    /// The display name set in Discord, when there is one.
    pub global_name: Option<String>,
    /// The avatar's hash, `None` for Discord's default avatar.
    pub avatar: Option<String>,
}

impl DiscordUser {
    fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        Some(Self { id: text("id")?, username: text("username").unwrap_or_default(), global_name: text("global_name").or_else(|| text("display_name")), avatar: text("avatar") })
    }

    /// The avatar URL from Discord's image server, at a size that suits a sidebar row.
    pub fn avatar_url(&self) -> Option<String> {
        let hash = self.avatar.as_deref()?;
        if !hash.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        Some(format!("https://cdn.discordapp.com/avatars/{}/{hash}.png?size=128", self.id))
    }
}

/// What a friend plays on Spotify, from Lanyard's `spotify` object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotifyTrack {
    pub track_id: Option<String>,
    pub song: String,
    /// Spotify's artists, joined by "; ".
    pub artist: String,
    pub album: String,
    pub album_art_url: Option<String>,
    /// When the song started and will end, in Unix ms.
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

impl SpotifyTrack {
    fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        let song = text("song");
        if song.is_empty() {
            return None;
        }
        let stamps = v.get("timestamps");
        let stamp = |k: &str| stamps.and_then(|t| t.get(k)).and_then(Value::as_u64);
        let opt = |k: &str| Some(text(k)).filter(|s| !s.is_empty());
        Some(Self { track_id: opt("track_id"), song, artist: text("artist"), album: text("album"), album_art_url: opt("album_art_url"), start_ms: stamp("start"), end_ms: stamp("end") })
    }

    /// The song's length, from its timestamps.
    pub fn duration_ms(&self) -> Option<u64> {
        Some(self.end_ms?.checked_sub(self.start_ms?)?).filter(|d| *d > 0)
    }

    /// How far into the song the friend is at `now_ms`, within the song.
    pub fn position_ms(&self, now_ms: u64) -> u64 {
        let at = self.start_ms.map_or(0, |s| now_ms.saturating_sub(s));
        self.duration_ms().map_or(at, |d| at.min(d))
    }

    /// Spotify's artists, one each.
    pub fn artists(&self) -> Vec<&str> {
        self.artist.split(';').map(str::trim).filter(|a| !a.is_empty()).collect()
    }

    /// The first artist, for "Title – Artist".
    pub fn first_artist(&self) -> &str {
        self.artists().first().copied().unwrap_or_default()
    }

    /// Spotify's artists as one line, like "A, B".
    pub fn artist_line(&self) -> String {
        self.artists().join(", ")
    }

    /// Identifies this song for the match cache and "skipped" notices.
    pub fn key(&self) -> String {
        match &self.track_id {
            Some(id) => id.clone(),
            None => format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", self.song, self.artist, self.album, self.duration_ms().unwrap_or(0) / 1000),
        }
    }
}

/// One Lanyard presence, with the Spotify song when `listening_to_spotify`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub user: Option<DiscordUser>,
    pub spotify: Option<SpotifyTrack>,
}

impl Presence {
    /// Parses Lanyard's presence object, the REST `data` or a socket event's `d`.
    pub fn from_json(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let user = v.get("discord_user").and_then(DiscordUser::from_json);
        let listening = v.get("listening_to_spotify").and_then(Value::as_bool).unwrap_or(false);
        let spotify = if listening { v.get("spotify").and_then(SpotifyTrack::from_json) } else { None };
        Some(Self { user, spotify })
    }

    /// The Discord id this presence belongs to, `discord_user.id` or else the event's `user_id`.
    pub fn id_in(v: &Value) -> Option<String> {
        v.get("discord_user").and_then(|u| u.get("id")).or_else(|| v.get("user_id")).and_then(Value::as_str).map(str::to_string)
    }
}

/// Whether Lanyard can tell Navia about a friend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    /// Not asked yet.
    #[default]
    Waiting,
    /// Lanyard knows them and their presence is read.
    Readable,
    /// Lanyard doesn't monitor them because they haven't joined its Discord server.
    NotMonitored,
    /// Lanyard couldn't be reached.
    Unreachable,
}

/// A friend as Navia knows them now.
#[derive(Clone, Debug, PartialEq)]
pub struct FriendNow {
    pub friend: Friend,
    pub reach: Reach,
    pub user: Option<DiscordUser>,
    /// Their Spotify song, while their activity shows one.
    pub track: Option<SpotifyTrack>,
    /// When their Spotify activity went away, in Unix ms, while it is gone.
    pub quiet_since_ms: Option<u64>,
    /// The song in the listener's library, filled in by the caller.
    pub song: Option<Song>,
}

impl FriendNow {
    /// The name to show, the one the listener gave, else their Discord display name, username or id.
    pub fn display_name(&self) -> String {
        let given = self.friend.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let user = self.user.as_ref();
        given
            .or_else(|| user.and_then(|u| u.global_name.as_deref()))
            .or_else(|| user.map(|u| u.username.as_str()).filter(|n| !n.is_empty()))
            .unwrap_or(&self.friend.id)
            .to_string()
    }

    pub fn avatar_url(&self) -> Option<String> {
        self.user.as_ref()?.avatar_url()
    }

    /// Whether they're listed in Listening Now, which needs a Spotify song in their activity.
    pub fn listening(&self) -> bool {
        self.track.is_some()
    }

    /// Their song as a session to follow, playing from where they are.
    /// `None` when they aren't listening or the song isn't in the library.
    pub fn session(&self, now_ms: u64) -> Option<NowPlaying> {
        let track = self.track.as_ref()?;
        let song = self.song.clone()?;
        Some(NowPlaying {
            user: self.display_name(),
            player: friend_player(&self.friend.id),
            song,
            state: Some(SessionState::Playing),
            position_ms: Some(track.position_ms(now_ms)),
            minutes_ago: 0,
        })
    }
}

/// Initials for a friend without an avatar, from up to two words.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let pick: Vec<&str> = if words.len() > 1 { vec![words[0], words[words.len() - 1]] } else { words };
    pick.iter().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).flat_map(char::to_uppercase).collect()
}

/// The current time in Unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Closer to the end than this, a song the friend is finishing isn't started again here.
const ENDING: Duration = Duration::from_millis(1500);

/// One check while following a Spotify friend, in the same terms as same-server listen-along.
///
/// Songs in the library resync on change or drift, and songs not in it report [`FollowStep::NotInLibrary`].
/// Activity gone longer than [`QUIET_GRACE`] ends following.
pub fn friend_step(followed: &Followed, friends: &[FriendNow], local: &Local, now_ms: u64) -> FollowStep {
    let keep = || FollowStep::Keep { drift_ms: 0, player: followed.player.clone() };
    let Some(id) = friend_of(&followed.player) else { return FollowStep::Ended };
    let Some(friend) = friends.iter().find(|f| f.friend.id == id) else { return FollowStep::Ended };
    if friend.reach == Reach::NotMonitored {
        return FollowStep::Ended;
    }
    let grace = QUIET_GRACE.as_millis() as u64;
    let Some(track) = friend.track.as_ref() else {
        let quiet_for = friend.quiet_since_ms.map_or(0, |at| now_ms.saturating_sub(at));
        return if quiet_for >= grace { FollowStep::Ended } else { keep() };
    };
    if track.end_ms.is_some_and(|end| now_ms > end + grace) {
        return FollowStep::Ended;
    }
    if local.held {
        return keep();
    }
    let Some(entry) = friend.session(now_ms) else {
        return FollowStep::NotInLibrary { key: track.key(), song: track.song.clone(), artist: track.first_artist().to_string() };
    };
    let at = entry.position_ms.unwrap_or(0);
    let same = entry.song.id == local.song_id;
    let ending = track.duration_ms().is_some_and(|d| at + ENDING.as_millis() as u64 >= d);
    if same && ending {
        return keep();
    }
    let drift_ms = local.position_ms as i64 - at as i64;
    if !same || !local.playing || drift_ms.unsigned_abs() > RESYNC_DRIFT.as_millis() as u64 {
        return FollowStep::Resync { player: entry.player.clone(), entry };
    }
    FollowStep::Keep { drift_ms, player: followed.player.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "100000000000000007";

    fn presence(listening: bool) -> Value {
        json!({
            "discord_user": { "id": ID, "username": "alex.r", "global_name": "Alex", "avatar": "a_1269e74af4df7417b13759eae50c83dc" },
            "listening_to_spotify": listening,
            "spotify": if listening { json!({
                "track_id": "4cOdK2wGLETKBW3PvgPWqT", "song": "Song One", "artist": "Main; Guest", "album": "The Album",
                "album_art_url": "https://i.scdn.co/image/x", "timestamps": { "start": 1_000_000, "end": 1_200_000 }
            }) } else { Value::Null },
        })
    }

    fn friend(track: Option<SpotifyTrack>, song: Option<&str>, quiet: Option<u64>) -> FriendNow {
        FriendNow {
            friend: Friend { id: ID.into(), name: None },
            reach: Reach::Readable,
            user: Presence::from_json(&presence(false)).unwrap().user,
            track,
            quiet_since_ms: quiet,
            song: song.map(|id| Song { id: id.into(), title: "Song One".into(), ..Default::default() }),
        }
    }

    fn track(start: u64) -> SpotifyTrack {
        Presence::from_json(&presence(true)).unwrap().spotify.map(|t| SpotifyTrack { start_ms: Some(start), end_ms: Some(start + 200_000), ..t }).unwrap()
    }

    fn followed() -> Followed {
        Followed { user: "Alex".into(), player: friend_player(ID) }
    }

    fn local(song: &str, pos: u64, playing: bool) -> Local {
        Local { song_id: song.into(), position_ms: pos, playing, ..Default::default() }
    }

    #[test]
    fn ids_and_addresses() {
        assert_eq!(parse_friend_id(" 100000000000000007 ").as_deref(), Some(ID));
        assert_eq!(parse_friend_id("1234"), None);
        assert_eq!(parse_friend_id("9449051068879257a"), None);
        assert_eq!(parse_address("").as_deref(), Some(PUBLIC_ADDRESS));
        assert_eq!(parse_address("https://lanyard.example.org/").as_deref(), Some("https://lanyard.example.org"));
        assert_eq!(parse_address("http://127.0.0.1:4001").as_deref(), Some("http://127.0.0.1:4001"));
        assert_eq!(parse_address("ftp://x.org"), None);
        assert_eq!(parse_address("not a url"), None);
        assert_eq!(friend_of(&friend_player(ID)), Some(ID));
        assert!(is_loopback("http://127.0.0.1:4001") && is_loopback("http://localhost:1") && is_loopback("http://[::1]:2"));
        assert!(!is_loopback(PUBLIC_ADDRESS));
        assert_eq!(friend_of("navia\u{200B}"), None);
    }

    #[test]
    fn presence_parses_spotify_and_user() {
        let p = Presence::from_json(&presence(true)).unwrap();
        let t = p.spotify.unwrap();
        assert_eq!((t.song.as_str(), t.artists(), t.album.as_str(), t.duration_ms()), ("Song One", vec!["Main", "Guest"], "The Album", Some(200_000)));
        assert_eq!(t.position_ms(1_050_000), 50_000);
        assert_eq!(t.position_ms(9_000_000), 200_000);
        let u = p.user.unwrap();
        assert_eq!(u.avatar_url().as_deref(), Some("https://cdn.discordapp.com/avatars/100000000000000007/a_1269e74af4df7417b13759eae50c83dc.png?size=128"));
        assert_eq!(Presence::id_in(&presence(true)).as_deref(), Some(ID));
        // When not listening, the leftover object is ignored.
        let mut v = presence(true);
        v["listening_to_spotify"] = json!(false);
        assert_eq!(Presence::from_json(&v).unwrap().spotify, None);
    }

    #[test]
    fn names_and_initials() {
        let mut f = friend(None, None, None);
        assert_eq!(f.display_name(), "Alex");
        f.friend.name = Some("  Alex R ".into());
        assert_eq!(f.display_name(), "Alex R");
        f.friend.name = None;
        f.user = None;
        assert_eq!(f.display_name(), ID);
        assert_eq!(initials("alex rivera"), "AR");
        assert_eq!(initials("Mira"), "M");
        assert_eq!(initials("Jean Paul Sartre"), "JS");
    }

    #[test]
    fn following_plays_from_their_position_and_resyncs_on_changes() {
        let start = 1_000_000;
        let f = friend(Some(track(start)), Some("s1"), None);
        // Starting plays s1 from 30 s in.
        match friend_step(&followed(), std::slice::from_ref(&f), &local("old", 0, true), start + 30_000) {
            FollowStep::Resync { entry, player } => {
                assert_eq!((entry.song.id.as_str(), entry.position_ms, entry.user.as_str()), ("s1", Some(30_000), "Alex"));
                assert_eq!(player, friend_player(ID));
            }
            other => panic!("{other:?}"),
        }
        // In step, so keep.
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 31_000, true), start + 30_000), FollowStep::Keep { drift_ms: 1000, .. }));
        // They seeked past the drift, so resync.
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 90_000, true), start + 30_000), FollowStep::Resync { .. }));
        // Paused here while they play, so play again.
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 30_000, false), start + 30_000), FollowStep::Resync { .. }));
        // The song ended here before their next one arrived, so wait and don't restart it.
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 200_000, false), start + 199_500), FollowStep::Keep { .. }));
    }

    #[test]
    fn a_song_not_in_the_library_is_skipped() {
        let f = friend(Some(track(1_000_000)), None, None);
        match friend_step(&followed(), &[f], &local("s1", 0, true), 1_010_000) {
            FollowStep::NotInLibrary { song, artist, key } => assert_eq!((song.as_str(), artist.as_str(), key.as_str()), ("Song One", "Main", "4cOdK2wGLETKBW3PvgPWqT")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn waiting_for_a_song_in_the_library_then_playing_it_in_sync() {
        // Started on a song not in the library, so nothing of theirs plays.
        let waiting = local("", 0, false);
        let missing = friend(Some(track(1_000_000)), None, None);
        assert!(matches!(friend_step(&followed(), &[missing], &waiting, 1_010_000), FollowStep::NotInLibrary { .. }));
        // Still waiting while they're quiet between songs, within the grace.
        let between = friend(None, None, Some(1_015_000));
        assert!(matches!(friend_step(&followed(), &[between], &waiting, 1_016_000), FollowStep::Keep { .. }));
        // Their next song is in the library, so it plays at once from where they are.
        let next = friend(Some(track(1_020_000)), Some("s2"), None);
        match friend_step(&followed(), std::slice::from_ref(&next), &waiting, 1_027_000) {
            FollowStep::Resync { entry, .. } => assert_eq!((entry.song.id.as_str(), entry.position_ms, entry.state), ("s2", Some(7_000), Some(SessionState::Playing))),
            other => panic!("{other:?}"),
        }
        // Then in step, and a later song not in the library is skipped again.
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&next), &local("s2", 7_500, true), 1_027_000), FollowStep::Keep { .. }));
        let later = friend(Some(track(1_220_000)), None, None);
        assert!(matches!(friend_step(&followed(), &[later], &local("s2", 199_000, false), 1_221_000), FollowStep::NotInLibrary { .. }));
    }

    #[test]
    fn the_listeners_own_pause_holds_a_friend_too() {
        let start = 1_000_000;
        let f = friend(Some(track(start)), Some("s1"), None);
        let held = Local { held: true, ..local("s1", 10_000, false) };
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &held, start + 60_000), FollowStep::Keep { .. }));
        // A song not in the library while held gives no skip notice.
        let missing = friend(Some(track(start)), None, None);
        assert!(matches!(friend_step(&followed(), &[missing], &held, start + 60_000), FollowStep::Keep { .. }));
        // Playing again starts from where they are now.
        match friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 10_000, false), start + 60_000) {
            FollowStep::Resync { entry, .. } => assert_eq!(entry.position_ms, Some(60_000)),
            other => panic!("{other:?}"),
        }
        // Removed while held ends following.
        assert_eq!(friend_step(&followed(), &[], &held, start), FollowStep::Ended);
    }

    #[test]
    fn quiet_friends_end_following_after_the_grace() {
        let quiet_at = 5_000_000;
        let f = friend(None, None, Some(quiet_at));
        let grace = QUIET_GRACE.as_millis() as u64;
        assert!(matches!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 0, true), quiet_at + grace - 1), FollowStep::Keep { .. }));
        assert_eq!(friend_step(&followed(), std::slice::from_ref(&f), &local("s1", 0, true), quiet_at + grace), FollowStep::Ended);
        // Removed, or no longer seen by Lanyard, ends following at once.
        assert_eq!(friend_step(&followed(), &[], &local("s1", 0, true), quiet_at), FollowStep::Ended);
        let gone = FriendNow { reach: Reach::NotMonitored, ..friend(Some(track(1)), Some("s1"), None) };
        assert_eq!(friend_step(&followed(), &[gone], &local("s1", 0, true), 2), FollowStep::Ended);
        // A song long over with nothing after it, from a stale presence.
        let stale = friend(Some(track(1_000_000)), Some("s1"), None);
        assert_eq!(friend_step(&followed(), &[stale], &local("s1", 0, false), 1_200_000 + grace + 1), FollowStep::Ended);
    }
}
