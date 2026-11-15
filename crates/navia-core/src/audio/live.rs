//! Internet radio: an endless HTTP stream fed straight to the decoder. Handles Shoutcast and
//! Icecast, including `ICY` status lines, `StreamTitle` metadata, and `.pls` and `.m3u` playlists.

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use symphonia::core::io::MediaSource;

use crate::audio::decode::{Decoder, TrackSource};
use crate::audio::engine::TrackOpener;
use crate::error::{Error, Result};

/// Bytes kept ahead of the decoder. Enough to ride out jitter, small enough to stay near live.
const BUFFER_CAP: usize = 128 * 1024;
/// Bytes to buffer before starting, so playback doesn't stall at once.
const START_BYTES: usize = 48 * 1024;
const START_WAIT: Duration = Duration::from_secs(8);
/// A stall this long with no bytes is treated as the station going away.
const READ_STALL: Duration = Duration::from_secs(10);
/// Dropped connections are retried this many times before giving up.
const RECONNECTS: u32 = 3;
/// Bytes a connection must deliver before the retry count resets, so a station that connects
/// and drops at once can't keep the player reconnecting forever.
const STEADY_BYTES: u64 = 64 * 1024;
const CHUNK: usize = 16 * 1024;

fn station_err(message: impl Into<String>) -> Error {
    Error::Audio(message.into())
}

/// Plays one station. Opening happens on the engine's open thread.
pub struct LiveOpener {
    url: String,
    client: Client,
    /// Bumped to abandon any connection in progress or running.
    session: Arc<AtomicU64>,
    current: Mutex<Option<Arc<LiveShared>>>,
}

impl LiveOpener {
    pub fn new(url: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent(crate::client::user_agent())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .http1_only()
            .http09_responses()
            .build()?;
        Ok(Self { url, client, session: Arc::new(AtomicU64::new(0)), current: Mutex::new(None) })
    }

    /// Stops any connection in progress and any blocked read at once.
    pub fn cancel(&self) {
        self.session.fetch_add(1, Ordering::AcqRel);
        if let Some(shared) = self.current.lock().expect("live").as_ref() {
            shared.cancel();
        }
    }

    /// The song the station says is playing, when it says.
    pub fn title(&self) -> Option<String> {
        self.current.lock().expect("live").as_ref().and_then(|s| s.state.lock().expect("live state").title.clone())
    }
}

impl TrackOpener for LiveOpener {
    fn open(&self, _start_ms: u64) -> Result<(Decoder, u64)> {
        let session = self.session.fetch_add(1, Ordering::AcqRel) + 1;
        let alive = || self.session.load(Ordering::Acquire) == session;
        let (resp, direct_url) = connect(&self.client, &self.url, &alive)?;
        let metaint = header_metaint(&resp);
        let shared = Arc::new(LiveShared::default());
        *self.current.lock().expect("live") = Some(shared.clone());
        let pump_shared = shared.clone();
        let (client, url) = (self.client.clone(), direct_url);
        let session_counter = self.session.clone();
        thread::Builder::new()
            .name("navia-live".into())
            .spawn(move || pump(pump_shared, client, url, resp, metaint, session_counter, session))
            .map_err(Error::Io)?;
        let deadline = Instant::now() + START_WAIT;
        loop {
            let state = shared.state.lock().expect("live state");
            if state.buf.len() >= START_BYTES || state.finished() {
                break;
            }
            drop(state);
            if !alive() {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let failed = {
            let state = shared.state.lock().expect("live state");
            state.error.clone().filter(|_| state.buf.is_empty())
        };
        if let Some(message) = failed {
            return Err(station_err(message));
        }
        let opened = Decoder::open_source(&TrackSource::Live(shared.clone()));
        if !alive() {
            return Err(Error::Cancelled);
        }
        Ok((opened.map_err(|_| station_err("the station's stream format can't be played"))?, 0))
    }
}

/// True for a playlist or page, which the caller resolves, rather than an audio stream.
fn looks_like_playlist(content_type: &str, url: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    let lower = url.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or("");
    ct.contains("scpls") || ct.contains("pls+xml") || ct.contains("mpegurl") || ct.contains("x-m3u") || path.ends_with(".pls") || path.ends_with(".m3u") || path.ends_with(".m3u8")
}

/// The first playable address in a `.pls` or `.m3u` body. HLS playlists are refused.
fn playlist_first_url(body: &str) -> Result<String> {
    if body.lines().any(|l| l.trim_start().starts_with("#EXT-X-")) {
        return Err(station_err("the station uses HLS, which isn't supported"));
    }
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let candidate = line.split_once('=').filter(|(k, _)| k.to_ascii_lowercase().starts_with("file")).map_or(line, |(_, v)| v.trim());
        if candidate.starts_with("http://") || candidate.starts_with("https://") {
            return Ok(candidate.to_string());
        }
    }
    Err(station_err("the station's playlist had no address to play"))
}

/// Follows playlists until an audio response is found.
fn connect(client: &Client, start: &str, alive: &dyn Fn() -> bool) -> Result<(Response, String)> {
    let mut url = start.to_string();
    for _ in 0..4 {
        if !alive() {
            return Err(Error::Cancelled);
        }
        let resp = client.get(&url).header("Icy-MetaData", "1").send().map_err(friendly_net)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(station_err(format!("the station answered HTTP {}", status.as_u16())));
        }
        let content_type = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if looks_like_playlist(&content_type, &url) {
            let body = read_text(resp)?;
            url = playlist_first_url(&body)?;
            continue;
        }
        if content_type.to_ascii_lowercase().starts_with("text/html") {
            return Err(station_err("the address is a web page, not a stream"));
        }
        return Ok((resp, url));
    }
    Err(station_err("the station's playlist chain was too deep"))
}

fn friendly_net(e: reqwest::Error) -> Error {
    // Never `Error::Network`, which means the Navidrome server is gone and waits forever.
    if e.is_builder() {
        station_err("the station has no valid address")
    } else {
        station_err("the station isn't responding")
    }
}

fn read_text(mut resp: Response) -> Result<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = resp.read(&mut chunk).map_err(|e| station_err(e.to_string()))?;
        if n == 0 || buf.len() >= 64 * 1024 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn header_metaint(resp: &Response) -> Option<usize> {
    resp.headers().get("icy-metaint").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok())
}

#[derive(Default)]
struct LiveState {
    buf: VecDeque<u8>,
    error: Option<String>,
    eof: bool,
    title: Option<String>,
    /// Set once a reader is attached. A second `reader()` gets nothing.
    attached: bool,
}

impl LiveState {
    fn finished(&self) -> bool {
        self.eof || self.error.is_some()
    }
}

/// Shared between the network thread and the decoder's reader.
#[derive(Default)]
pub struct LiveShared {
    state: Mutex<LiveState>,
    cond: Condvar,
    cancelled: AtomicBool,
}

impl std::fmt::Debug for LiveShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveShared").finish_non_exhaustive()
    }
}

impl LiveShared {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cond.notify_all();
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn fail(&self, message: String) {
        self.state.lock().expect("live state").error = Some(message);
        self.cond.notify_all();
    }

    /// Queues bytes, waiting while the buffer is full. Returns false when canceled.
    fn push(&self, data: &[u8]) -> bool {
        let mut state = self.state.lock().expect("live state");
        loop {
            if self.cancelled() {
                return false;
            }
            if state.buf.len() + data.len() <= BUFFER_CAP {
                state.buf.extend(data);
                self.cond.notify_all();
                return true;
            }
            let (next, _) = self.cond.wait_timeout(state, Duration::from_millis(50)).expect("live state");
            state = next;
        }
    }

    fn set_title(&self, title: String) {
        self.state.lock().expect("live state").title = Some(title);
    }

    /// Hands the decoder its reader. A live stream has only one read position.
    pub(crate) fn take_reader(self: &Arc<Self>) -> Result<Box<dyn MediaSource>> {
        let mut state = self.state.lock().expect("live state");
        if state.attached {
            return Err(Error::Audio("a live stream can only be read once".to_string()));
        }
        state.attached = true;
        Ok(Box::new(LiveReader { shared: self.clone() }))
    }
}

/// The network side. Reads the stream, strips ICY metadata and queues the audio until canceled.
fn pump(shared: Arc<LiveShared>, client: Client, url: String, first: Response, metaint: Option<usize>, session: Arc<AtomicU64>, id: u64) {
    let mut demux = Demux::new(metaint);
    let mut resp = first;
    let mut buf = vec![0u8; CHUNK];
    let mut reconnects = 0u32;
    let mut delivered = 0u64;
    loop {
        if shared.cancelled() || session.load(Ordering::Acquire) != id {
            return;
        }
        let outcome = resp.read(&mut buf);
        match outcome {
            Ok(n) if n > 0 => {
                delivered += n as u64;
                if delivered >= STEADY_BYTES {
                    reconnects = 0;
                }
                let mut audio = Vec::with_capacity(n);
                demux.feed(&buf[..n], &mut audio, &*shared);
                if !audio.is_empty() && !shared.push(&audio) {
                    return;
                }
            }
            // A clean end or a dropped connection, so try to pick the stream back up.
            other => {
                let error = if matches!(other, Ok(_)) { "the station ended the stream".to_string() } else { "the connection to the station was lost".to_string() };
                if reconnects >= RECONNECTS {
                    shared.fail(error);
                    return;
                }
                reconnects += 1;
                delivered = 0;
                thread::sleep(Duration::from_millis(600 * u64::from(reconnects)));
                if shared.cancelled() {
                    return;
                }
                match client.get(&url).header("Icy-MetaData", "1").send() {
                    Ok(next) if next.status().is_success() => {
                        demux = Demux::new(header_metaint(&next));
                        resp = next;
                    }
                    Ok(next) => {
                        shared.fail(format!("the station answered HTTP {}", next.status().as_u16()));
                        return;
                    }
                    Err(e) => {
                        if reconnects >= RECONNECTS {
                            shared.fail(friendly_net(e).to_string());
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Separates audio from ICY metadata blocks. It also strips an `ICY 200 OK` header block,
/// which HTTP libraries leave in the body of an old Shoutcast response.
struct Demux {
    metaint: Option<usize>,
    /// The first bytes, to detect an `ICY` status line.
    header: Option<Vec<u8>>,
    until_meta: usize,
    meta_remaining: usize,
    meta: Vec<u8>,
    /// True while the next byte is a metadata length.
    expecting_len: bool,
}

impl Demux {
    fn new(metaint: Option<usize>) -> Self {
        Self { metaint, header: Some(Vec::new()), until_meta: metaint.unwrap_or(0), meta_remaining: 0, meta: Vec::new(), expecting_len: false }
    }

    fn feed(&mut self, data: &[u8], audio: &mut Vec<u8>, shared: &LiveShared) {
        // An HTTP/0.9 body starts with `ICY 200 OK` and a header block. A normal response never does.
        if let Some(mut head) = self.header.take() {
            head.extend_from_slice(data);
            if head.starts_with(b"ICY") {
                let Some(end) = find(&head, b"\r\n\r\n").map(|p| p + 4).or_else(|| find(&head, b"\n\n").map(|p| p + 2)) else {
                    if head.len() < 16 * 1024 {
                        self.header = Some(head);
                    }
                    return;
                };
                if let Some(interval) = header_block_metaint(&head[..end]) {
                    self.metaint = Some(interval);
                    self.until_meta = interval;
                }
                self.demux(&head[end..], audio, shared);
                return;
            }
            self.demux(&head, audio, shared);
            return;
        }
        self.demux(data, audio, shared);
    }

    fn demux(&mut self, mut data: &[u8], audio: &mut Vec<u8>, shared: &LiveShared) {
        let Some(interval) = self.metaint else {
            audio.extend_from_slice(data);
            return;
        };
        while !data.is_empty() {
            if self.expecting_len {
                self.meta_remaining = data[0] as usize * 16;
                self.expecting_len = false;
                data = &data[1..];
                self.meta.clear();
                if self.meta_remaining == 0 {
                    self.until_meta = interval;
                }
                continue;
            }
            if self.meta_remaining > 0 {
                let take = self.meta_remaining.min(data.len());
                self.meta.extend_from_slice(&data[..take]);
                self.meta_remaining -= take;
                data = &data[take..];
                if self.meta_remaining == 0 {
                    if let Some(title) = parse_stream_title(&self.meta) {
                        shared.set_title(title);
                    }
                    self.until_meta = interval;
                }
                continue;
            }
            let take = self.until_meta.min(data.len());
            audio.extend_from_slice(&data[..take]);
            self.until_meta -= take;
            data = &data[take..];
            if self.until_meta == 0 {
                self.expecting_len = true;
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn header_block_metaint(block: &[u8]) -> Option<usize> {
    String::from_utf8_lossy(block).lines().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("icy-metaint") { value.trim().parse().ok() } else { None }
    })
}

/// Takes the title out of a block like `StreamTitle='Artist - Song';StreamUrl='';`.
fn parse_stream_title(block: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(block);
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let end = rest.find("';").unwrap_or(rest.trim_end_matches('\0').len());
    let title = rest[..end].trim();
    (!title.is_empty()).then(|| title.to_string())
}

struct LiveReader {
    shared: Arc<LiveShared>,
}

impl Read for LiveReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut state = self.shared.state.lock().expect("live state");
        let waited = Instant::now();
        loop {
            if !state.buf.is_empty() {
                let n = state.buf.len().min(buf.len());
                for (slot, byte) in buf[..n].iter_mut().zip(state.buf.drain(..n)) {
                    *slot = byte;
                }
                drop(state);
                self.shared.cond.notify_all();
                return Ok(n);
            }
            if self.shared.cancelled() {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cancelled"));
            }
            if let Some(message) = state.error.clone() {
                return Err(io::Error::other(message));
            }
            if state.eof {
                return Ok(0);
            }
            if waited.elapsed() > READ_STALL {
                return Err(io::Error::other("the station stopped sending audio"));
            }
            let (next, _) = self.shared.cond.wait_timeout(state, Duration::from_millis(100)).expect("live state");
            state = next;
        }
    }
}

impl Seek for LiveReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("a live stream can't seek"))
    }
}

impl MediaSource for LiveReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_titles() {
        assert_eq!(parse_stream_title(b"StreamTitle='Artist - Song';StreamUrl='x';").as_deref(), Some("Artist - Song"));
        assert_eq!(parse_stream_title(b"StreamTitle='';").as_deref(), None);
        assert_eq!(parse_stream_title(b"StreamUrl='x';"), None);
    }

    #[test]
    fn playlists_give_the_first_address_and_refuse_hls() {
        assert_eq!(playlist_first_url("[playlist]\nFile1=http://a/b\nTitle1=x").unwrap(), "http://a/b");
        assert_eq!(playlist_first_url("#EXTM3U\n#EXTINF:0,x\nhttps://a/stream\n").unwrap(), "https://a/stream");
        assert!(playlist_first_url("#EXTM3U\n#EXT-X-VERSION:3\nchunk.ts").is_err());
        assert!(playlist_first_url("nothing here").is_err());
    }

    #[test]
    fn demux_strips_icy_blocks_and_finds_titles() {
        let shared = Arc::new(LiveShared::default());
        let mut demux = Demux::new(Some(8));
        // A metadata length of 1 (16 bytes) twice: audio 1..8, a block, then audio 9..12 across chunks.
        let mut data: Vec<u8> = (1..=8).collect();
        data.push(2);
        let meta = b"StreamTitle='A';\0\0\0\0";
        let mut block = meta.to_vec();
        block.resize(32, 0);
        let mut out = Vec::new();
        demux.feed(&data, &mut out, &shared);
        demux.feed(&block[..10], &mut out, &shared);
        demux.feed(&block[10..], &mut out, &shared);
        demux.feed(&[9, 10, 11, 12], &mut out, &shared);
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(shared.state.lock().unwrap().title.as_deref(), Some("A"));
    }

    #[test]
    fn demux_handles_icy_status_lines_in_the_body() {
        let shared = Arc::new(LiveShared::default());
        let mut demux = Demux::new(None);
        let mut out = Vec::new();
        let mut body = b"ICY 200 OK\r\nicy-metaint:4\r\nicy-name:X\r\n\r\n".to_vec();
        body.extend_from_slice(&[1, 2, 3, 4, 0, 5]);
        demux.feed(&body, &mut out, &shared);
        assert_eq!(out, vec![1, 2, 3, 4, 5]);
    }
}
