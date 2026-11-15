//! Keeps Spotify friends' presences from Lanyard, read over REST at once and then live over its socket.
//! Sockets can go quiet without closing, so REST is checked periodically and stale sockets are replaced.
//! Apps poll [`FriendsWatch::friends`] cheaply, since [`FriendsWatch::generation`] moves on every change.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;

use super::{Friend, FriendNow, Presence, QUIET_GRACE, Reach, SpotifyTrack, matching, now_ms};
use crate::client::Song;
use crate::library::Library;

/// How often friends are asked without the socket, and while following one.
pub const POLL: Duration = Duration::from_secs(15);
pub const POLL_FOLLOWING: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The watch's pace. [`Timing::default`] is what the apps use, and tests shrink it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    poll: Duration,
    poll_following: Duration,
    /// The first wait before retrying a failed socket, doubling up to the max.
    socket_retry_min: Duration,
    socket_retry_max: Duration,
    /// A socket that stayed up this long resets that wait.
    socket_stable: Duration,
    /// How often the socket resubscribes while Lanyard doesn't see a friend yet.
    /// Lanyard only learns about people who joined later on a new subscription.
    resubscribe: Duration,
    /// While the socket is up, everyone is read over REST this often too.
    rest_check: Duration,
    /// How often the clocks are compared, to notice the machine slept.
    watchdog: Duration,
    /// How much further the wall clock may run than the monotonic one before it counts as a sleep.
    sleep_jump: Duration,
    /// Longest a socket may stay silent after a ping, on top of the heartbeat interval.
    pong_slack: Duration,
    /// The shortest heartbeat interval taken from Lanyard's hello.
    heartbeat_min: Duration,
    /// How long past its end a song still counts as playing without a newer presence.
    end_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: POLL,
            poll_following: POLL_FOLLOWING,
            socket_retry_min: Duration::from_secs(5),
            socket_retry_max: Duration::from_secs(120),
            socket_stable: Duration::from_secs(120),
            resubscribe: Duration::from_secs(300),
            rest_check: Duration::from_secs(60),
            watchdog: Duration::from_secs(5),
            sleep_jump: Duration::from_secs(15),
            pong_slack: Duration::from_secs(15),
            heartbeat_min: Duration::from_secs(5),
            end_grace: QUIET_GRACE,
        }
    }
}

/// Notices the machine sleeping, since the monotonic clock stops during sleep and the wall clock doesn't.
struct SleepCheck {
    mono: Instant,
    wall: std::time::SystemTime,
}

impl SleepCheck {
    fn new() -> Self {
        Self { mono: Instant::now(), wall: std::time::SystemTime::now() }
    }

    /// Whether it slept by more than `jump` since the last look.
    fn slept(&mut self, jump: Duration) -> bool {
        let now = Self::new();
        let mono = now.mono.duration_since(self.mono);
        let wall = now.wall.duration_since(self.wall).unwrap_or_default();
        *self = now;
        slept_between(mono, wall, jump)
    }
}

fn slept_between(mono: Duration, wall: Duration, jump: Duration) -> bool {
    wall.saturating_sub(mono) > jump
}

/// Whether a song's end has passed by more than `grace`, which makes it over whatever the last presence said.
fn ended(track: &SpotifyTrack, now_ms: u64, grace: Duration) -> bool {
    track.end_ms.is_some_and(|end| now_ms > end + grace.as_millis() as u64)
}
/// A missed match is looked up again after this, in case the library gained the song.
const MISS_RETRY: Duration = Duration::from_secs(300);
const MATCH_CACHE: usize = 512;

/// How Lanyard is being read now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    /// No friends, so nothing contacts Lanyard.
    #[default]
    Idle,
    Socket,
    Polling,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    /// Last read over REST, and the socket hasn't reported this friend since.
    from_rest: bool,
    reach: Reach,
    user: Option<super::DiscordUser>,
    track: Option<SpotifyTrack>,
    quiet_since_ms: Option<u64>,
}

#[derive(Default)]
struct State {
    timing: Timing,
    address: String,
    friends: Vec<Friend>,
    /// Moves when the address or the friends' ids change.
    config: u64,
    entries: HashMap<String, Entry>,
    generation: u64,
    transport: Transport,
    following: bool,
}

impl State {
    /// Ends songs whose end passed without a newer presence, so the friend goes quiet.
    fn expire(&mut self, now: u64) {
        let mut changed = false;
        for e in self.entries.values_mut() {
            if let Some(track) = e.track.as_ref().filter(|t| ended(t, now, self.timing.end_grace)) {
                e.quiet_since_ms = Some(track.end_ms.unwrap_or(now));
                e.track = None;
                changed = true;
            }
        }
        if changed {
            self.generation += 1;
        }
    }
}

impl Entry {
    /// The same presence, wherever it was read.
    fn same_presence(&self, other: &Entry) -> bool {
        self.reach == other.reach && self.user == other.user && self.track == other.track
    }
}

type MatchCache = HashMap<String, (Option<(u32, Song)>, Instant)>;

struct Shared {
    state: Mutex<State>,
    wake: Notify,
    matches: Mutex<MatchCache>,
}

impl Shared {
    fn config(&self) -> (u64, String, Vec<String>) {
        let s = self.state.lock().unwrap();
        (s.config, s.address.clone(), s.friends.iter().map(|f| f.id.clone()).collect())
    }

    fn current(&self, config: u64) -> bool {
        self.state.lock().unwrap().config == config
    }

    fn set_transport(&self, t: Transport) {
        let mut s = self.state.lock().unwrap();
        if s.transport != t {
            s.transport = t;
            s.generation += 1;
        }
    }

    fn timing(&self) -> Timing {
        self.state.lock().unwrap().timing
    }

    /// Records what Lanyard said about `id` under configuration `config`.
    /// Returns true when a REST read found an update the socket missed.
    fn apply(&self, config: u64, id: &str, reach: Reach, presence: Option<Presence>, from_rest: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.config != config || !s.friends.iter().any(|f| f.id == id) {
            return false;
        }
        let old = s.entries.get(id).cloned().unwrap_or_default();
        // The socket's INIT_STATE leaves out people it doesn't push for, so keep what REST read.
        if !from_rest && reach == Reach::NotMonitored && old.from_rest && old.reach == Reach::Readable {
            return false;
        }
        let now = now_ms();
        let (user, track) = match presence {
            Some(p) => (p.user.or(old.user.clone()), p.spotify),
            None => (old.user.clone(), None),
        };
        // A song that's long over isn't playing, whatever an old presence says.
        let track = track.filter(|t| !ended(t, now, s.timing.end_grace));
        let quiet_since_ms = match (&track, old.quiet_since_ms) {
            (Some(_), _) => None,
            (None, Some(at)) => Some(at),
            (None, None) => Some(now),
        };
        let mut new = Entry { from_rest, reach, user, track, quiet_since_ms };
        let socket_fresh = !old.from_rest && old.reach == Reach::Readable;
        // REST agreeing with what the socket said leaves it the socket's.
        if from_rest && socket_fresh && new.same_presence(&old) {
            new.from_rest = false;
        }
        let missed = from_rest && socket_fresh && !new.same_presence(&old);
        if new != old {
            s.entries.insert(id.to_string(), new);
            s.generation += 1;
        }
        missed
    }

    fn poll_every(&self) -> Duration {
        let s = self.state.lock().unwrap();
        if s.following { s.timing.poll_following } else { s.timing.poll }
    }

    fn any_unseen(&self) -> bool {
        !self.unseen().is_empty()
    }

    /// Friends the socket hasn't reported as readable. REST keeps these fresh meanwhile.
    fn unseen(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        s.friends.iter().filter(|f| s.entries.get(&f.id).is_none_or(|e| e.reach != Reach::Readable || e.from_rest)).map(|f| f.id.clone()).collect()
    }
}

/// Spotify friends' presences, kept fresh in the background on the core's runtime.
pub struct FriendsWatch {
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FriendsWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FriendsWatch {
    /// Starts idle. Nothing is asked until [`Self::configure`] adds a friend.
    pub fn start(rt: &tokio::runtime::Handle) -> Self {
        Self::start_with(rt, Timing::default())
    }

    pub(crate) fn start_with(rt: &tokio::runtime::Handle, timing: Timing) -> Self {
        let state = State { timing, ..State::default() };
        let shared = Arc::new(Shared { state: Mutex::new(state), wake: Notify::new(), matches: Mutex::new(HashMap::new()) });
        let task = rt.spawn(run(shared.clone()));
        Self { shared, task }
    }

    /// Sets the Lanyard address and the friends. Unchanged ids and address keep the connection.
    pub fn configure(&self, address: &str, friends: Vec<Friend>) {
        let mut friends = friends;
        let mut seen = std::collections::HashSet::new();
        friends.retain(|f| seen.insert(f.id.clone()));
        let mut s = self.shared.state.lock().unwrap();
        let moved = s.address != address;
        let ids_changed = moved || s.friends.iter().map(|f| &f.id).ne(friends.iter().map(|f| &f.id));
        if !ids_changed && s.friends == friends {
            return;
        }
        s.friends = friends;
        s.generation += 1;
        if !ids_changed {
            return;
        }
        s.address = address.to_string();
        s.config += 1;
        if moved {
            s.entries.clear();
        } else {
            let keep: std::collections::HashSet<String> = s.friends.iter().map(|f| f.id.clone()).collect();
            s.entries.retain(|id, _| keep.contains(id));
        }
        drop(s);
        self.shared.wake.notify_one();
    }

    /// Polls faster while following a friend, when the socket can't be used.
    pub fn set_following(&self, on: bool) {
        let mut s = self.shared.state.lock().unwrap();
        if s.following != on {
            s.following = on;
            drop(s);
            self.shared.wake.notify_one();
        }
    }

    /// Moves whenever a presence, a friend or the transport changed, or a song's end passed.
    pub fn generation(&self) -> u64 {
        let mut s = self.shared.state.lock().unwrap();
        s.expire(now_ms());
        s.generation
    }

    pub fn transport(&self) -> Transport {
        self.shared.state.lock().unwrap().transport
    }

    /// Every friend in the order added, as Lanyard last said, with songs not matched.
    pub fn friends(&self) -> Vec<FriendNow> {
        let mut s = self.shared.state.lock().unwrap();
        s.expire(now_ms());
        s.friends
            .iter()
            .map(|f| {
                let e = s.entries.get(&f.id).cloned().unwrap_or_default();
                FriendNow { friend: f.clone(), reach: e.reach, user: e.user, track: e.track, quiet_since_ms: e.quiet_since_ms, song: None }
            })
            .collect()
    }

    /// [`Self::friends`] with each song found in `library` and remembered per song.
    pub fn friends_matched(&self, library: &Library) -> Vec<FriendNow> {
        let mut friends = self.friends();
        for f in &mut friends {
            f.song = f.track.as_ref().and_then(|t| self.matched(library, t)).map(|(_, s)| s);
        }
        friends
    }

    /// The library's song for a Spotify song, remembered. Misses are retried after [`MISS_RETRY`].
    pub fn matched(&self, library: &Library, track: &SpotifyTrack) -> Option<(u32, Song)> {
        let key = track.key();
        if let Some((hit, at)) = self.shared.matches.lock().unwrap().get(&key)
            && (hit.is_some() || at.elapsed() < MISS_RETRY)
        {
            return hit.clone();
        }
        let hit = matching::find(library, track);
        let mut cache = self.shared.matches.lock().unwrap();
        if cache.len() >= MATCH_CACHE {
            cache.clear();
        }
        cache.insert(key, (hit.clone(), Instant::now()));
        hit
    }

    /// Forgets the remembered matches, for when the library changed a lot or another one opened.
    pub fn forget_matches(&self) {
        self.shared.matches.lock().unwrap().clear();
    }
}

async fn run(shared: Arc<Shared>) {
    let timing = shared.timing();
    let mut retry = timing.socket_retry_min;
    loop {
        let (config, address, ids) = shared.config();
        if ids.is_empty() {
            shared.set_transport(Transport::Idle);
            shared.wake.notified().await;
            continue;
        }
        // Read everyone over REST first, so they show without waiting for the socket.
        let Some(http) = rest_client() else { return };
        fetch_all(&shared, &http, config, &address, &ids).await;
        match socket(&shared, &http, config, &address, &ids).await {
            SocketEnd::Again => {
                retry = timing.socket_retry_min;
                continue;
            }
            SocketEnd::Failed(lasted) => {
                if lasted >= timing.socket_stable {
                    retry = timing.socket_retry_min;
                }
            }
        }
        shared.set_transport(Transport::Polling);
        poll(&shared, config, &address, &ids, retry).await;
        retry = (retry * 2).min(timing.socket_retry_max);
    }
}

/// How a socket ended.
enum SocketEnd {
    /// Start over now because the friends changed, the machine slept or the socket missed an update.
    Again,
    /// It failed or stopped answering after being up this long. Poll, then try it again.
    Failed(Duration),
}

/// The socket's address for a Lanyard address.
pub fn socket_url(address: &str) -> Option<String> {
    let rest = address.strip_prefix("https://").map(|r| ("wss://", r)).or_else(|| address.strip_prefix("http://").map(|r| ("ws://", r)))?;
    Some(format!("{}{}/socket", rest.0, rest.1.trim_end_matches('/')))
}

/// TLS for the socket, rustls with aws-lc-rs and the platform's verifier, as reqwest uses.
fn tls() -> Option<Arc<rustls::ClientConfig>> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().ok()?.with_platform_verifier().ok()?.with_no_client_auth();
    Some(Arc::new(config))
}

/// Reads presences over the socket until it should start over or it fails.
async fn socket(shared: &Shared, http: &reqwest::Client, config: u64, address: &str, ids: &[String]) -> SocketEnd {
    let failed = SocketEnd::Failed(Duration::ZERO);
    let Some(url) = socket_url(address) else { return failed };
    let connector = if url.starts_with("wss://") { tls().map(tokio_tungstenite::Connector::Rustls) } else { Some(tokio_tungstenite::Connector::Plain) };
    let Some(connector) = connector else { return failed };
    let connect = tokio_tungstenite::connect_async_tls_with_config(url.as_str(), None, false, Some(connector));
    let Ok(Ok((mut ws, _))) = tokio::time::timeout(CONNECT_TIMEOUT, connect).await else {
        tracing::debug!("lanyard socket: no connection");
        return failed;
    };
    // Hello first, with the heartbeat interval.
    let hello = match tokio::time::timeout(CONNECT_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text).ok(),
        _ => None,
    };
    let Some(beat_ms) = hello.filter(|v| v["op"] == 1).and_then(|v| v["d"]["heartbeat_interval"].as_u64()) else { return failed };
    if ws.send(Message::text(json!({ "op": 2, "d": { "subscribe_to_ids": ids } }).to_string())).await.is_err() {
        return failed;
    }
    shared.set_transport(Transport::Socket);
    let timing = shared.timing();
    let beat_every = Duration::from_millis(beat_ms).clamp(timing.heartbeat_min, Duration::from_secs(120));
    let mut beat = tokio::time::interval(beat_every);
    beat.tick().await;
    let opened = Instant::now();
    let failed = || SocketEnd::Failed(opened.elapsed());
    let mut recheck = tokio::time::interval(timing.resubscribe);
    recheck.tick().await;
    let mut rest_check = tokio::time::interval(timing.rest_check);
    rest_check.tick().await;
    let mut watchdog = tokio::time::interval(timing.watchdog);
    watchdog.tick().await;
    let mut sleep = SleepCheck::new();
    // Lanyard doesn't answer heartbeats, but a live connection answers pings.
    let mut heard = Instant::now();
    // Friends the socket didn't report are read over REST meanwhile, as often as without it.
    let mut unseen_at = Instant::now() + shared.poll_every();
    loop {
        tokio::select! {
            msg = ws.next() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    tracing::debug!("lanyard socket closed after {:?}", opened.elapsed());
                    return failed();
                }
                Some(Ok(message)) => {
                    heard = Instant::now();
                    if let Message::Text(text) = message
                        && let Ok(v) = serde_json::from_str::<Value>(&text)
                    {
                        on_event(shared, config, ids, &v);
                    }
                }
            },
            _ = beat.tick() => {
                if heard.elapsed() > beat_every + timing.pong_slack {
                    tracing::debug!("lanyard socket stopped answering after {:?}", opened.elapsed());
                    return failed();
                }
                if ws.send(Message::text(json!({ "op": 3 }).to_string())).await.is_err() || ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return failed();
                }
            }
            _ = recheck.tick() => {
                if shared.any_unseen() {
                    let _ = ws.close(None).await;
                    return SocketEnd::Again;
                }
            }
            _ = rest_check.tick() => {
                // A socket that missed an update can't be trusted with the next one, so subscribe again.
                if fetch_all(shared, http, config, address, ids).await {
                    tracing::debug!("lanyard socket missed an update");
                    let _ = ws.close(None).await;
                    return SocketEnd::Again;
                }
            }
            _ = watchdog.tick() => {
                if sleep.slept(timing.sleep_jump) {
                    tracing::debug!("lanyard: the machine slept; starting over");
                    let _ = ws.close(None).await;
                    return SocketEnd::Again;
                }
            }
            _ = tokio::time::sleep_until(unseen_at.into()) => {
                let unseen = shared.unseen();
                fetch_all(shared, http, config, address, &unseen).await;
                unseen_at = Instant::now() + shared.poll_every();
            }
            _ = shared.wake.notified() => {
                if !shared.current(config) {
                    let _ = ws.close(None).await;
                    return SocketEnd::Again;
                }
            }
        }
    }
}

/// Handles one socket event, either `INIT_STATE` with every presence or `PRESENCE_UPDATE` with one.
/// Anyone missing from `INIT_STATE` isn't monitored.
fn on_event(shared: &Shared, config: u64, ids: &[String], v: &Value) {
    if v["op"] != 0 {
        return;
    }
    let d = &v["d"];
    match v["t"].as_str() {
        Some("INIT_STATE") => {
            for id in ids {
                let _ = match d.get(id).and_then(Presence::from_json) {
                    Some(p) => shared.apply(config, id, Reach::Readable, Some(p), false),
                    None => shared.apply(config, id, Reach::NotMonitored, None, false),
                };
            }
        }
        Some("PRESENCE_UPDATE") => {
            if let (Some(id), Some(p)) = (Presence::id_in(d), Presence::from_json(d)) {
                let _ = shared.apply(config, &id, Reach::Readable, Some(p), false);
            }
        }
        _ => {}
    }
}

fn rest_client() -> Option<reqwest::Client> {
    reqwest::Client::builder().timeout(CONNECT_TIMEOUT).user_agent(crate::client::user_agent()).build().ok()
}

/// Reads these friends over REST, all at once.
/// Returns true when one differed from what the socket had said.
async fn fetch_all(shared: &Shared, http: &reqwest::Client, config: u64, address: &str, ids: &[String]) -> bool {
    let answers = futures_util::future::join_all(ids.iter().map(|id| fetch(http, address, id))).await;
    let mut missed = false;
    for (id, (reach, presence)) in ids.iter().zip(answers) {
        missed |= shared.apply(config, id, reach, presence, true);
    }
    missed
}

/// Polls each friend over REST until the configuration changes, `wait` passes or the machine sleeps.
async fn poll(shared: &Shared, config: u64, address: &str, ids: &[String], wait: Duration) {
    let Some(http) = rest_client() else { return };
    let until = Instant::now() + wait;
    let mut sleep = SleepCheck::new();
    loop {
        let left = until.saturating_duration_since(Instant::now());
        let _ = tokio::time::timeout(shared.poll_every().min(left), shared.wake.notified()).await;
        if Instant::now() >= until || !shared.current(config) || sleep.slept(shared.timing().sleep_jump) {
            return;
        }
        fetch_all(shared, &http, config, address, ids).await;
    }
}

/// `GET /v1/users/<id>`, giving the presence, a 404 for unmonitored friends, or unreachable.
pub async fn fetch(http: &reqwest::Client, address: &str, id: &str) -> (Reach, Option<Presence>) {
    let Ok(resp) = http.get(format!("{address}/v1/users/{id}")).send().await else { return (Reach::Unreachable, None) };
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_default();
    if status.as_u16() == 404 || body["error"]["code"] == "user_not_monitored" {
        return (Reach::NotMonitored, None);
    }
    match (status.is_success() && body["success"] != false).then(|| Presence::from_json(&body["data"])).flatten() {
        Some(p) => (Reach::Readable, Some(p)),
        None => (Reach::Unreachable, None),
    }
}

/// An avatar's image bytes, a small PNG from Discord. Blocking, so call it from an image worker thread.
pub fn fetch_image(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("https://") {
        return None;
    }
    let resp = reqwest::blocking::Client::builder().timeout(CONNECT_TIMEOUT).build().ok()?.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    (bytes.len() < 4 << 20).then(|| bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(id: &str, song: Option<&str>) -> Value {
        json!({
            "discord_user": { "id": id, "username": format!("user{id}"), "avatar": null },
            "listening_to_spotify": song.is_some(),
            "spotify": song.map(|s| json!({ "song": s, "artist": "Artist", "album": "Album", "timestamps": { "start": now_ms() - 1_000, "end": now_ms() + 200_000 } })),
        })
    }

    /// A stand-in Lanyard on loopback, serving REST from `init` and a socket with updates pushed by the test.
    async fn stand_in(updates: tokio::sync::mpsc::UnboundedReceiver<Value>, init: Value) -> String {
        stand_in_with(updates, init.clone(), init).await
    }

    /// The same, with the socket's INIT_STATE different from what REST answers.
    async fn stand_in_with(updates: tokio::sync::mpsc::UnboundedReceiver<Value>, init: Value, socket_init: Value) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let updates = Arc::new(tokio::sync::Mutex::new(updates));
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut head = [0u8; 2048];
                let n = stream.peek(&mut head).await.unwrap();
                let request = String::from_utf8_lossy(&head[..n]).to_lowercase();
                let (init, socket_init, updates) = (init.clone(), socket_init.clone(), updates.clone());
                if !request.contains("upgrade: websocket") {
                    let _ = stream.read(&mut [0u8; 4096]).await;
                    let id = request.split_whitespace().nth(1).and_then(|p| p.strip_prefix("/v1/users/")).unwrap_or_default().to_string();
                    let (status, body) = match init.get(&id) {
                        Some(p) => ("200 OK", json!({ "success": true, "data": p })),
                        None => ("404 Not Found", json!({ "success": false, "error": { "code": "user_not_monitored" } })),
                    };
                    let body = body.to_string();
                    let _ = stream.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                    continue;
                }
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    ws.send(Message::text(json!({ "op": 1, "d": { "heartbeat_interval": 30000 } }).to_string())).await.unwrap();
                    let Some(Ok(sub)) = ws.next().await else { return };
                    let sub: Value = serde_json::from_str(sub.to_text().unwrap()).unwrap();
                    assert_eq!(sub["op"], 2);
                    ws.send(Message::text(json!({ "op": 0, "seq": 1, "t": "INIT_STATE", "d": socket_init }).to_string())).await.unwrap();
                    while let Some(d) = updates.lock().await.recv().await {
                        ws.send(Message::text(json!({ "op": 0, "seq": 2, "t": "PRESENCE_UPDATE", "d": d }).to_string())).await.unwrap();
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    async fn wait_for(watch: &FriendsWatch, mut ok: impl FnMut(&[FriendNow]) -> bool) -> Vec<FriendNow> {
        for _ in 0..300 {
            let f = watch.friends();
            if ok(&f) {
                return f;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never: {:?}", watch.friends());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_socket_keeps_friends_live() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let a = "111111111111111111";
        let b = "222222222222222222";
        let address = stand_in(rx, json!({ a: presence(a, Some("One")) })).await;
        let watch = FriendsWatch::start(&tokio::runtime::Handle::current());
        assert_eq!(watch.transport(), Transport::Idle);
        watch.configure(&address, vec![Friend { id: a.into(), name: None }, Friend { id: b.into(), name: Some("Bea".into()) }]);
        // REST answers first, then the socket takes over.
        wait_for(&watch, |_| watch.transport() == Transport::Socket).await;
        let f = wait_for(&watch, |f| f.iter().all(|f| f.reach != Reach::Waiting)).await;
        assert_eq!(f[0].track.as_ref().map(|t| t.song.as_str()), Some("One"));
        assert_eq!(f[0].display_name(), format!("user{a}"));
        // Missing from INIT_STATE, so Lanyard doesn't see them.
        assert_eq!((f[1].reach, f[1].display_name().as_str()), (Reach::NotMonitored, "Bea"));
        // A new song, then the activity is gone while paused, so they're quiet from then on.
        tx.send(presence(a, Some("Two"))).unwrap();
        wait_for(&watch, |f| f[0].track.as_ref().is_some_and(|t| t.song == "Two")).await;
        let before = now_ms();
        tx.send(presence(a, None)).unwrap();
        let f = wait_for(&watch, |f| f[0].track.is_none()).await;
        assert!(f[0].quiet_since_ms.is_some_and(|at| at >= before));
        // A name alone changes what's shown, not the connection.
        let config = watch.shared.config().0;
        watch.configure(&address, vec![Friend { id: a.into(), name: Some("Alex".into()) }, Friend { id: b.into(), name: Some("Bea".into()) }]);
        assert_eq!((watch.shared.config().0, watch.friends()[0].display_name().as_str()), (config, "Alex"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rest_reads_presences_and_unmonitored_users() {
        let server = wiremock::MockServer::start().await;
        let a = "333333333333333333";
        wiremock::Mock::given(wiremock::matchers::path(format!("/v1/users/{a}")))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": presence(a, Some("Three")) })))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/v1/users/444444444444444444"))
            .respond_with(wiremock::ResponseTemplate::new(404).set_body_json(json!({ "success": false, "error": { "code": "user_not_monitored", "message": "User is not being monitored by Lanyard" } })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let (reach, p) = fetch(&http, &server.uri(), a).await;
        assert_eq!((reach, p.and_then(|p| p.spotify).map(|t| t.song)), (Reach::Readable, Some("Three".to_string())));
        assert_eq!(fetch(&http, &server.uri(), "444444444444444444").await.0, Reach::NotMonitored);
        assert_eq!(fetch(&http, "http://127.0.0.1:9", a).await.0, Reach::Unreachable);
        // The watch falls back to polling when there is no socket.
        let watch = FriendsWatch::start(&tokio::runtime::Handle::current());
        watch.configure(&server.uri(), vec![Friend { id: a.into(), name: None }]);
        let f = wait_for(&watch, |f| f[0].reach == Reach::Readable).await;
        wait_for(&watch, |_| watch.transport() == Transport::Polling).await;
        assert_eq!(f[0].track.as_ref().map(|t| t.song.as_str()), Some("Three"));
    }

    /// A real Lanyard REST answer with the id, names and avatar replaced.
    /// It has Discord's newer user fields, a "dnd" status and the raw Spotify activity.
    const REAL: &str = include_str!("fixtures/rest_presence.json");

    #[test]
    fn a_real_presence_parses() {
        let body: Value = serde_json::from_str(REAL).unwrap();
        let p = Presence::from_json(&body["data"]).unwrap();
        let t = p.spotify.unwrap();
        assert!(!t.song.is_empty() && !t.artist.is_empty() && !t.album.is_empty());
        assert!(t.duration_ms().is_some_and(|d| d > 30_000));
        assert_eq!(t.track_id.as_deref().map(str::len), Some(22));
        let u = p.user.unwrap();
        assert_eq!((u.id.as_str(), u.global_name.as_deref()), ("100000000000000042", Some("Friend")));
        assert!(u.avatar_url().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn friends_show_at_once_from_rest_even_when_the_socket_leaves_them_out() {
        // INIT_STATE lacks the friend, since they joined after subscribing, but REST has them.
        // Their song is under way now, since the fixture's own times are long past.
        let mut body: Value = serde_json::from_str(REAL).unwrap();
        body["data"]["spotify"]["timestamps"] = json!({ "start": now_ms() - 30_000, "end": now_ms() + 200_000 });
        let id = "100000000000000042";
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let address = stand_in_with(rx, json!({ id: body["data"].clone() }), json!({})).await;
        let watch = FriendsWatch::start(&tokio::runtime::Handle::current());
        let started = Instant::now();
        watch.configure(&address, vec![Friend { id: id.into(), name: None }]);
        let f = wait_for(&watch, |f| f[0].listening()).await;
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(f[0].display_name(), "Friend");
        // The socket's INIT_STATE then leaves them out, and what REST read stays.
        wait_for(&watch, |_| watch.transport() == Transport::Socket).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(watch.friends()[0].listening() && watch.friends()[0].reach == Reach::Readable);
    }

    /// How a fake Lanyard's socket behaves for its next connections.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Sock {
        /// Hello, INIT_STATE from REST's presences, then the pushed updates. Answers pings.
        Normal,
        /// Hello and INIT_STATE, then it closes.
        Drop,
        /// Hello and INIT_STATE, then nothing at all, like a dead connection that never closed.
        Deaf,
    }

    /// A fake Lanyard on loopback whose REST answers and socket behavior the test changes,
    /// counting socket connections.
    struct Fake {
        address: String,
        rest: Arc<Mutex<Value>>,
        modes: Arc<Mutex<Vec<Sock>>>,
        sockets: Arc<std::sync::atomic::AtomicUsize>,
        push: tokio::sync::broadcast::Sender<Value>,
    }

    impl Fake {
        /// `modes` has one entry per connection in order, and the last repeats.
        async fn start(rest: Value, modes: Vec<Sock>) -> Self {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let rest = Arc::new(Mutex::new(rest));
            let modes = Arc::new(Mutex::new(modes));
            let sockets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (push, _) = tokio::sync::broadcast::channel(16);
            let (r, m, n, p) = (rest.clone(), modes.clone(), sockets.clone(), push.clone());
            tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut head = [0u8; 2048];
                    let k = stream.peek(&mut head).await.unwrap();
                    let request = String::from_utf8_lossy(&head[..k]).to_lowercase();
                    if !request.contains("upgrade: websocket") {
                        let _ = stream.read(&mut [0u8; 4096]).await;
                        let id = request.split_whitespace().nth(1).and_then(|p| p.strip_prefix("/v1/users/")).unwrap_or_default().to_string();
                        let presence = r.lock().unwrap().get(&id).cloned();
                        let (status, body) = match presence {
                            Some(p) => ("200 OK", json!({ "success": true, "data": p })),
                            None => ("404 Not Found", json!({ "success": false, "error": { "code": "user_not_monitored" } })),
                        };
                        let body = body.to_string();
                        let _ = stream.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                        continue;
                    }
                    let count = n.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    let mode = {
                        let modes = m.lock().unwrap();
                        modes.get(count).or(modes.last()).copied().unwrap_or(Sock::Normal)
                    };
                    let init = r.lock().unwrap().clone();
                    let mut updates = p.subscribe();
                    tokio::spawn(async move {
                        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                        let _ = ws.send(Message::text(json!({ "op": 1, "d": { "heartbeat_interval": 100 } }).to_string())).await;
                        let Some(Ok(_subscribe)) = ws.next().await else { return };
                        let _ = ws.send(Message::text(json!({ "op": 0, "seq": 1, "t": "INIT_STATE", "d": init }).to_string())).await;
                        match mode {
                            Sock::Drop => {
                                let _ = ws.close(None).await;
                            }
                            Sock::Deaf => {
                                // Hold the connection open and never read it, so pings go unanswered.
                                tokio::time::sleep(Duration::from_secs(30)).await;
                            }
                            Sock::Normal => loop {
                                tokio::select! {
                                    msg = ws.next() => if !matches!(msg, Some(Ok(_))) { return },
                                    d = updates.recv() => {
                                        let Ok(d) = d else { return };
                                        if ws.send(Message::text(json!({ "op": 0, "seq": 2, "t": "PRESENCE_UPDATE", "d": d }).to_string())).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            },
                        }
                    });
                }
            });
            Fake { address, rest, modes, sockets, push }
        }

        fn sockets(&self) -> usize {
            self.sockets.load(std::sync::atomic::Ordering::Acquire)
        }

        /// What REST says for `id` from now on. The socket isn't told.
        fn set_rest(&self, id: &str, presence: Value) {
            self.rest.lock().unwrap()[id] = presence;
        }
    }

    /// Quick timings, so a test sees reconnects, checks and ends in well under a second each.
    fn quick() -> Timing {
        Timing {
            poll: Duration::from_millis(150),
            poll_following: Duration::from_millis(100),
            socket_retry_min: Duration::from_millis(200),
            socket_retry_max: Duration::from_millis(800),
            socket_stable: Duration::from_secs(5),
            resubscribe: Duration::from_secs(60),
            rest_check: Duration::from_millis(300),
            watchdog: Duration::from_millis(100),
            sleep_jump: Duration::from_secs(15),
            pong_slack: Duration::from_millis(250),
            heartbeat_min: Duration::from_millis(100),
            end_grace: Duration::from_millis(300),
        }
    }

    fn song_of(f: &[FriendNow]) -> Option<String> {
        f[0].track.as_ref().map(|t| t.song.clone())
    }

    fn playing(id: &str, song: &str, end_in_ms: i64) -> Value {
        let now = now_ms() as i64;
        json!({
            "discord_user": { "id": id, "username": "friend" },
            "listening_to_spotify": true,
            "spotify": { "song": song, "artist": "Artist", "album": "Album", "timestamps": { "start": now - 60_000, "end": now + end_in_ms } },
        })
    }

    async fn wait_long(watch: &FriendsWatch, what: &str, mut ok: impl FnMut(&[FriendNow]) -> bool) {
        for _ in 0..500 {
            if ok(&watch.friends()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never {what}: {:?} ({:?})", watch.friends(), watch.transport());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_socket_comes_back_and_reads_what_changed_meanwhile() {
        let id = "555555555555555555";
        let fake = Fake::start(json!({ id: playing(id, "Family Affair", 200_000) }), vec![Sock::Drop, Sock::Normal]).await;
        let watch = FriendsWatch::start_with(&tokio::runtime::Handle::current(), quick());
        watch.configure(&fake.address, vec![Friend { id: id.into(), name: None }]);
        wait_long(&watch, "the first song", |f| song_of(f).as_deref() == Some("Family Affair")).await;
        // The socket dropped right after INIT_STATE, and the song changes while it's down.
        fake.set_rest(id, playing(id, "After Hours", 300_000));
        wait_long(&watch, "the new song", |f| song_of(f).as_deref() == Some("After Hours")).await;
        // It reconnects on its own, and the new socket's updates arrive.
        wait_long(&watch, "a new socket", |_| fake.sockets() >= 2 && watch.transport() == Transport::Socket).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = fake.push.send(playing(id, "Alone Again", 250_000));
        wait_long(&watch, "the pushed song", |f| song_of(f).as_deref() == Some("Alone Again")).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_socket_that_missed_an_update_is_corrected_and_replaced() {
        let id = "666666666666666666";
        let fake = Fake::start(json!({ id: playing(id, "Family Affair", 200_000) }), vec![Sock::Normal]).await;
        let watch = FriendsWatch::start_with(&tokio::runtime::Handle::current(), quick());
        watch.configure(&fake.address, vec![Friend { id: id.into(), name: None }]);
        wait_long(&watch, "the socket", |_| watch.transport() == Transport::Socket).await;
        let before = watch.generation();
        // Lanyard has a new song, but the socket never says so.
        fake.set_rest(id, playing(id, "After Hours", 300_000));
        wait_long(&watch, "the REST check's song", |f| song_of(f).as_deref() == Some("After Hours")).await;
        assert!(watch.generation() > before);
        wait_long(&watch, "a fresh subscription", |_| fake.sockets() >= 2).await;
        // REST agreeing with the socket doesn't churn it.
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert!(fake.sockets() <= 3, "resubscribed {} times", fake.sockets());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_socket_that_stops_answering_is_dropped() {
        let id = "777777777777777777";
        let fake = Fake::start(json!({ id: playing(id, "Family Affair", 200_000) }), vec![Sock::Deaf, Sock::Normal]).await;
        let watch = FriendsWatch::start_with(&tokio::runtime::Handle::current(), quick());
        watch.configure(&fake.address, vec![Friend { id: id.into(), name: None }]);
        wait_long(&watch, "the deaf socket", |_| fake.sockets() == 1 && watch.transport() == Transport::Socket).await;
        // No pong comes back, so it's given up on and polled, then a new socket opens.
        wait_long(&watch, "polling", |_| watch.transport() == Transport::Polling).await;
        wait_long(&watch, "a new socket", |_| fake.sockets() >= 2 && watch.transport() == Transport::Socket).await;
        fake.set_rest(id, playing(id, "After Hours", 300_000));
        let _ = fake.push.send(playing(id, "After Hours", 300_000));
        wait_long(&watch, "the new song", |f| song_of(f).as_deref() == Some("After Hours")).await;
        let _ = &fake.modes;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_song_past_its_end_is_over_without_a_newer_presence() {
        let id = "888888888888888888";
        let other = "999999999999999999";
        // One ended long ago, from a stale presence, and one ends in a moment.
        let fake = Fake::start(json!({ id: playing(id, "Family Affair", -60_000), other: playing(other, "Short", 200) }), vec![Sock::Normal]).await;
        let watch = FriendsWatch::start_with(&tokio::runtime::Handle::current(), quick());
        watch.configure(&fake.address, vec![Friend { id: id.into(), name: None }, Friend { id: other.into(), name: None }]);
        wait_long(&watch, "both read", |f| f.iter().all(|f| f.reach == Reach::Readable)).await;
        let f = watch.friends();
        assert!(!f[0].listening() && f[0].quiet_since_ms.is_some(), "an ended song isn't listed: {:?}", f[0]);
        assert!(f[1].listening());
        let before = watch.generation();
        // Nothing else comes. Past its end and the grace the second is over too,
        // and the generation moves so the apps redraw.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(watch.generation() > before);
        let f = watch.friends();
        assert!(!f[1].listening(), "{:?}", f[1]);
        // Following them ends after the quiet grace, as when their activity goes.
        let followed = super::super::Followed { user: "x".into(), player: super::super::friend_player(other) };
        let local = super::super::Local { song_id: "s".into(), position_ms: 0, playing: true, ..Default::default() };
        let quiet = f[1].quiet_since_ms.unwrap();
        assert_eq!(super::super::friend_step(&followed, &f, &local, quiet + super::super::QUIET_GRACE.as_millis() as u64 + 1), super::super::FollowStep::Ended);
        // A newer presence brings them back.
        let _ = fake.push.send(playing(other, "Next", 200_000));
        wait_long(&watch, "the next song", |f| f[1].listening()).await;
    }

    #[test]
    fn sleeping_is_told_from_the_clocks() {
        let jump = Duration::from_secs(15);
        assert!(!slept_between(Duration::from_secs(5), Duration::from_secs(5), jump));
        assert!(!slept_between(Duration::from_secs(5), Duration::from_secs(19), jump));
        assert!(slept_between(Duration::from_secs(5), Duration::from_secs(600), jump));
        // A wall clock set back isn't a sleep.
        assert!(!slept_between(Duration::from_secs(5), Duration::ZERO, jump));
    }

    #[test]
    fn socket_addresses() {
        assert_eq!(socket_url("https://api.lanyard.rest").as_deref(), Some("wss://api.lanyard.rest/socket"));
        assert_eq!(socket_url("http://127.0.0.1:4001/").as_deref(), Some("ws://127.0.0.1:4001/socket"));
        assert_eq!(socket_url("ftp://x"), None);
    }

    #[test]
    fn tls_builds() {
        assert!(tls().is_some());
    }
}
