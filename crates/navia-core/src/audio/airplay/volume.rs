//! The receiver's own volume: read on connect, followed while playing, and set only when the
//! listener moves the slider, at most every [`SEND_EVERY`] while dragging.
//!
//! Navia's scale is 0 to 1. The receiver's is -30 to 0 dB, with -144 for mute.
//! Reports go to the player through [`Reported`], never back to the receiver.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::session::Session;

/// Minimum gap between sends while dragging. The last position always goes out.
pub(crate) const SEND_EVERY: Duration = Duration::from_millis(125);
/// Reports this soon after a send are about our own change and may be stale mid-drag.
const QUIET_AFTER_SEND: Duration = Duration::from_millis(1500);
/// Changes smaller than this (0.25 dB) aren't worth moving the slider for.
const REPORT_STEP: f32 = 0.25 / 30.0;

/// The receiver's last reported volume, with a counter that moves on each report.
#[derive(Default)]
pub(crate) struct Reported(Mutex<Option<(u64, f32)>>);

impl Reported {
    fn publish(&self, level: f32) {
        let mut r = self.0.lock().expect("reported volume");
        let n = r.map_or(1, |(n, _)| n + 1);
        *r = Some((n, level));
    }

    pub fn get(&self) -> Option<(u64, f32)> {
        *self.0.lock().expect("reported volume")
    }
}

/// One session's volume traffic, run from the session task.
pub(crate) struct VolumeLink {
    reported: std::sync::Arc<Reported>,
    /// Our best idea of the receiver's volume: read, reported, or last sent.
    known: Option<f32>,
    /// The listener's latest position, not sent yet.
    pending: Option<f32>,
    last_send: Option<Instant>,
    /// The receiver answered `GET_PARAMETER volume`, so it's asked again now and then.
    pollable: bool,
}

impl VolumeLink {
    /// Reads the receiver's volume and tells the player.
    /// A receiver that refuses or doesn't say stays unknown and isn't asked again.
    pub async fn start(session: &mut Session, reported: std::sync::Arc<Reported>) -> Self {
        let read = match session.read_volume().await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("airplay: reading the volume failed: {e}");
                None
            }
        };
        if let Some(level) = read {
            reported.publish(level);
        }
        Self { reported, known: read, pending: None, last_send: None, pollable: read.is_some() }
    }

    /// The listener moved the slider, or muted with 0.
    pub fn request(&mut self, level: f32) {
        self.pending = Some(level.clamp(0.0, 1.0));
    }

    /// When the pending volume may go out.
    pub fn due(&self) -> Option<Instant> {
        self.pending?;
        Some(self.last_send.map_or_else(Instant::now, |t| t + SEND_EVERY))
    }

    /// Send the pending volume if its time has come.
    pub async fn send_due(&mut self, session: &mut Session) {
        if self.due().is_none_or(|d| d > Instant::now()) {
            return;
        }
        let Some(level) = self.pending.take() else { return };
        if self.known.is_some_and(|k| (k - level).abs() < 1e-4) {
            return;
        }
        self.last_send = Some(Instant::now());
        self.known = Some(level);
        if let Err(e) = session.volume(level).await {
            tracing::warn!("airplay volume: {e}");
        }
    }

    /// The receiver reports its volume, through DACP or a poll answer.
    pub fn report(&mut self, level: f32) {
        if self.busy() {
            return;
        }
        if self.known.is_none_or(|k| (k - level).abs() >= REPORT_STEP) {
            self.known = Some(level);
            self.reported.publish(level);
        }
    }

    /// Asks the receiver again to follow its own remote, but never while the listener is changing it.
    pub async fn poll(&mut self, session: &mut Session) {
        if !self.pollable || self.busy() {
            return;
        }
        match session.read_volume().await {
            Ok(Some(level)) => self.report(level),
            _ => self.pollable = false,
        }
    }

    /// The listener is changing it, or just did.
    fn busy(&self) -> bool {
        self.pending.is_some() || self.last_send.is_some_and(|t| t.elapsed() < QUIET_AFTER_SEND)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;
    use crate::audio::airplay::device::AirPlayDevice;
    use crate::audio::airplay::session::level_to_db;

    /// What a fake receiver got: method, body and time.
    type Log = Arc<Mutex<Vec<(String, String, Instant)>>>;

    /// A localhost receiver that answers `GET_PARAMETER` with `get` and everything else with 200.
    async fn fake_receiver(get: (u16, &'static str)) -> (AirPlayDevice, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log: Log = Arc::default();
        let l = log.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut head = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    head.push(line);
                }
                let header = |name: &str| head.iter().find_map(|h| h.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
                let len = header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).await.unwrap();
                let method = head[0].split_whitespace().next().unwrap_or("").to_string();
                l.lock().unwrap().push((method.clone(), String::from_utf8_lossy(&body).into_owned(), Instant::now()));
                let cseq = header("CSeq").unwrap_or_default();
                let (status, text) = if method == "GET_PARAMETER" { get } else { (200, "") };
                let reply = format!("RTSP/1.0 {status} X\r\nCSeq: {cseq}\r\nContent-Length: {}\r\n\r\n{text}", text.len());
                reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let txt: HashMap<String, String> = [("deviceid", "AA:BB:CC:DD:EE:10"), ("features", "0x445F8A00,0x1C340"), ("flags", "0x4"), ("srcvers", "366.0")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        (AirPlayDevice::from_txt("Fake", &txt, vec!["127.0.0.1".parse().unwrap()], port).unwrap(), log)
    }

    fn sets(log: &Log) -> Vec<(String, Instant)> {
        log.lock().unwrap().iter().filter(|(m, _, _)| m == "SET_PARAMETER").map(|(_, b, t)| (b.clone(), *t)).collect()
    }

    #[tokio::test]
    async fn reads_the_receivers_volume_on_connect() {
        for (answer, level) in [("volume: -15.000000\r\n", 0.5), ("volume: -144.000000\r\n", 0.0), ("volume: 0.000000\r\n", 1.0)] {
            let (dev, log) = fake_receiver((200, answer)).await;
            let mut s = Session::plain(&dev).await.unwrap();
            let reported = Arc::new(Reported::default());
            let link = VolumeLink::start(&mut s, reported.clone()).await;
            assert_eq!(reported.get(), Some((1, level)), "{answer}");
            assert!(link.pollable);
            let log = log.lock().unwrap();
            assert_eq!(log.len(), 1);
            assert_eq!((log[0].0.as_str(), log[0].1.as_str()), ("GET_PARAMETER", "volume\r\n"));
        }
    }

    #[tokio::test]
    async fn a_receiver_that_refuses_stays_unknown_and_gets_nothing() {
        for answer in [(501, ""), (400, ""), (200, ""), (200, "volume: loud\r\n")] {
            let (dev, log) = fake_receiver(answer).await;
            let mut s = Session::plain(&dev).await.unwrap();
            let reported = Arc::new(Reported::default());
            let mut link = VolumeLink::start(&mut s, reported.clone()).await;
            assert_eq!(reported.get(), None, "{answer:?}");
            // Not asked again, and no volume goes out on its own.
            link.poll(&mut s).await;
            link.send_due(&mut s).await;
            let log = log.lock().unwrap();
            assert_eq!(log.iter().map(|(m, _, _)| m.as_str()).collect::<Vec<_>>(), ["GET_PARAMETER"], "{answer:?}");
        }
    }

    /// A drag: 60 positions over about 600 ms, driven the way the session task drives it.
    #[tokio::test]
    async fn a_drag_is_rate_limited_and_ends_on_the_last_position() {
        let (dev, log) = fake_receiver((200, "volume: -24.000000\r\n")).await;
        let mut s = Session::plain(&dev).await.unwrap();
        let reported = Arc::new(Reported::default());
        let mut link = VolumeLink::start(&mut s, reported.clone()).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f32>();
        tokio::spawn(async move {
            for i in 0..60 {
                let _ = tx.send(0.21 + 0.005 * i as f32);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let started = Instant::now();
        loop {
            let due = link.due();
            tokio::select! {
                v = rx.recv() => match v {
                    Some(v) => {
                        link.request(v);
                        link.send_due(&mut s).await;
                        // Mid-drag, the receiver isn't asked and its reports are ignored.
                        link.poll(&mut s).await;
                        link.report(0.9);
                    }
                    None if due.is_none() => break,
                    None => tokio::time::sleep_until(due.unwrap().into()).await,
                },
                _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now).into()), if due.is_some() => link.send_due(&mut s).await,
            }
        }
        let elapsed = started.elapsed();
        let sent = sets(&log);
        let most = (elapsed.as_millis() / SEND_EVERY.as_millis()) as usize + 1;
        assert!(sent.len() >= 3 && sent.len() <= most, "{} sends in {elapsed:?}", sent.len());
        for w in sent.windows(2) {
            assert!(w[1].1 - w[0].1 >= SEND_EVERY - Duration::from_millis(5), "sends {:?} apart", w[1].1 - w[0].1);
        }
        assert_eq!(sent.last().unwrap().0, format!("volume: {:.6}\r\n", level_to_db(0.21 + 0.005 * 59.0)));
        assert_eq!(log.lock().unwrap().iter().filter(|(m, _, _)| m == "GET_PARAMETER").count(), 1);
        // The reports during the drag never reached the player.
        assert_eq!(reported.get(), Some((1, 0.2)));
    }

    #[tokio::test]
    async fn the_receivers_own_changes_reach_the_player_once_the_listener_stops() {
        let (dev, _log) = fake_receiver((200, "volume: -6.000000\r\n")).await;
        let mut s = Session::plain(&dev).await.unwrap();
        let reported = Arc::new(Reported::default());
        let mut link = VolumeLink::start(&mut s, reported.clone()).await;
        assert_eq!(reported.get(), Some((1, 0.8)));
        // A poll that finds the same volume changes nothing.
        link.poll(&mut s).await;
        assert_eq!(reported.get(), Some((1, 0.8)));
        // The receiver's remote (DACP) turns it down.
        link.report(0.3);
        assert_eq!(reported.get(), Some((2, 0.3)));
        // The listener sets it, and a report right after is about that change, so it's ignored.
        link.request(0.5);
        link.send_due(&mut s).await;
        link.report(0.45);
        assert_eq!(reported.get(), Some((2, 0.3)));
    }
}
