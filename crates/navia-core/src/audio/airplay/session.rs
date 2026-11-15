//! The RTSP side of one AirPlay session: pairing, SETUP, RECORD, FLUSH, parameters, keep-alive
//! and TEARDOWN. Audio goes out on its own thread in `sender`.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use navia_airplay::base::{AudioCodec, AudioFormat, PtpMode, StreamConfig, StreamType, TimingProtocol};
use navia_airplay::crypto::ControlCipher;
use navia_airplay::crypto::ed25519::IdentityKeyPair;
use navia_airplay::crypto::keys::SessionKeys;
use navia_airplay::pairing::{ControllerIdentity, PairSetup, PairVerify};
use navia_airplay::rtsp::dmap::TrackMetadata;
use navia_airplay::rtsp::{RtspConnection, RtspMethod, RtspRequest, RtspSession, SessionState};
use navia_airplay::timing::NtpTimingServer;
use tokio::net::TcpStream;

use super::dacp::RemoteIds;
use super::device::{AirPlayDevice, Pairing, Route};
use super::store::{PairedDevice, PairingStore, hex, unhex};
use crate::error::{Error, Result};

/// Frames per packet, the AirPlay realtime standard.
pub const FRAMES_PER_PACKET: u32 = 352;
pub const SAMPLE_RATE: u32 = 44_100;
/// Frames the receiver is told to buffer (0.5 s to 2 s), as Apple senders and pyatv do.
const LATENCY_MIN: u32 = 11_025;
const LATENCY_MAX: u32 = 88_200;
/// The PIN for transient pairing, fixed by the protocol.
const TRANSIENT_PIN: &str = "3939";
const RTSP_TIMEOUT: Duration = Duration::from_secs(5);

/// Local ports for receiver traffic. 0 picks any free port.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ports {
    pub timing: u16,
    pub control: u16,
    pub data: u16,
    /// TCP, for the receiver's remote buttons (DACP).
    pub dacp: u16,
}

/// What the audio sender needs from SETUP.
pub(crate) struct Streaming {
    pub data_dest: SocketAddr,
    pub control_dest: SocketAddr,
    pub stream_key: [u8; 32],
    /// Our control port. Sync packets go out on it and retransmit requests come in.
    pub control_socket: UdpSocket,
    pub data_port: u16,
}

pub(crate) struct Session {
    rtsp: RtspConnection,
    session: RtspSession,
    timing: Option<NtpTimingServer>,
    events: Option<TcpStream>,
    addr: IpAddr,
}

pub(crate) fn stream_config(route: Route) -> StreamConfig {
    let audio_format = AudioFormat {
        codec: if route.pcm { AudioCodec::Pcm } else { AudioCodec::Alac },
        sample_rate: navia_airplay::base::SampleRate::Hz44100,
        bit_depth: 16,
        channels: 2,
        frames_per_packet: FRAMES_PER_PACKET,
    };
    let asc = (!route.pcm).then(|| navia_airplay::audio::AlacEncoder::new(audio_format.clone()).map(|e| e.magic_cookie()).ok()).flatten();
    StreamConfig {
        stream_type: StreamType::Realtime,
        audio_format,
        timing_protocol: TimingProtocol::Ntp,
        ptp_mode: PtpMode::Master,
        latency_min: LATENCY_MIN,
        latency_max: LATENCY_MAX,
        supports_dynamic_stream_id: true,
        asc,
    }
}

fn controller(store: &PairingStore) -> Result<ControllerIdentity> {
    let id = store.identity()?;
    Ok(ControllerIdentity::with_id(IdentityKeyPair::from_seed(&id.seed), id.id))
}

fn random_device_id() -> String {
    let b: [u8; 6] = rand::random();
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":")
}

fn ap(e: impl std::fmt::Display) -> Error {
    Error::AirPlay(e.to_string())
}

/// Opens the RTSP connection with the headers every request carries.
async fn open(dev: &AirPlayDevice, cfg: StreamConfig, ids: &RemoteIds) -> Result<(RtspConnection, RtspSession, IpAddr, String)> {
    let addr = *dev.addresses.first().ok_or_else(|| ap("the receiver has no address"))?;
    let mut rtsp = RtspConnection::new(SocketAddr::new(addr, dev.port));
    tokio::time::timeout(RTSP_TIMEOUT, rtsp.connect()).await.map_err(|_| ap("the receiver didn't answer"))?.map_err(ap)?;
    let client_id = random_device_id();
    let mut session = RtspSession::new(dev.raw.clone(), cfg);
    session.set_client_device_id(client_id.clone());
    session.set_connected().map_err(ap)?;
    session.set_request_host(addr.to_string());
    rtsp.add_session_header("User-Agent", "AirPlay/745.83");
    rtsp.add_session_header("X-Apple-Client-Name", "Navia");
    rtsp.add_session_header("X-Apple-Device-ID", client_id.clone());
    rtsp.add_session_header("DACP-ID", ids.dacp_id.clone());
    rtsp.add_session_header("Client-Instance", ids.dacp_id.clone());
    rtsp.add_session_header("Active-Remote", ids.active_remote.clone());
    Ok((rtsp, session, addr, client_id))
}

async fn send(rtsp: &mut RtspConnection, req: RtspRequest) -> Result<navia_airplay::rtsp::RtspResponse> {
    tokio::time::timeout(RTSP_TIMEOUT, rtsp.send(req)).await.map_err(|_| ap("the receiver stopped answering"))?.map_err(ap)
}

impl Session {
    /// Connects and authenticates, with pair-verify for a stored pairing or else transient pairing.
    pub async fn connect(dev: &AirPlayDevice, route: Route, store: &PairingStore, ids: &RemoteIds) -> Result<Self> {
        let (mut rtsp, mut session, addr, client_id) = open(dev, stream_config(route), ids).await?;
        let keys = match route.pairing {
            Pairing::Pin => {
                let paired = store.device(&dev.id).ok_or_else(|| Error::AirPlay("needs pairing".into()))?;
                verify(&mut rtsp, &controller(store)?, &paired).await?
            }
            Pairing::Transient => {
                send(&mut rtsp, RtspRequest::get_info()).await?;
                transient(&mut rtsp, &client_id).await?
            }
            Pairing::Password => return Err(ap("password-protected receivers aren't supported yet")),
        };
        rtsp.set_cipher(ControlCipher::new(*keys.write_key.as_bytes(), *keys.read_key.as_bytes()));
        session.set_paired().map_err(ap)?;
        Ok(Self { rtsp, session, timing: None, events: None, addr })
    }

    /// An unpaired, unencrypted session, for tests against a fake receiver.
    #[cfg(test)]
    pub async fn plain(dev: &AirPlayDevice) -> Result<Self> {
        let (rtsp, session, addr, _) = open(dev, stream_config(Route { pairing: Pairing::Transient, pcm: false }), &RemoteIds::new()).await?;
        Ok(Self { rtsp, session, timing: None, events: None, addr })
    }

    /// SETUP for timing and events, then for the audio stream, then RECORD.
    pub async fn setup(&mut self, ports: Ports) -> Result<Streaming> {
        // A fixed port that's taken falls back to any free port, which works where the firewall allows.
        let timing = start_timing(ports.timing).await?;
        let local = self.rtsp.local_addr().map(|a| vec![a.ip().to_string()]);
        let timing_port = timing.port();
        // Kept from here on, so a failed setup still says whether the receiver ever asked.
        self.timing = Some(timing);
        let body = self.session.build_setup_phase1(timing_port, local).map_err(ap)?;
        let resp = send(&mut self.rtsp, RtspRequest::setup(self.session.request_uri(), body)).await?;
        if resp.status_code != 200 {
            return Err(ap(format!("SETUP was refused ({})", resp.status_code)));
        }
        self.session.process_setup_phase1_response(resp.body.as_deref().unwrap_or(&[])).map_err(ap)?;
        let sid = resp.headers.get("Session").cloned().unwrap_or_else(|| "1".into());
        self.rtsp.add_session_header("Session", sid);
        let event_port = self.session.ports().map(|p| p.event_port).unwrap_or(0);
        if event_port != 0 {
            // Some receivers answer 500 to the stream SETUP without this connection.
            self.events = tokio::time::timeout(RTSP_TIMEOUT, TcpStream::connect((self.addr, event_port))).await.ok().and_then(|r| r.ok());
        }
        let control_socket = bind_udp(ports.control, "control")?;
        let control_port = control_socket.local_addr()?.port();
        self.session.set_local_control_port(control_port);
        // Receivers start timing right after the first SETUP and may refuse a request that overtakes it.
        // Not every receiver queries, so this only waits a little.
        if let Some(t) = self.timing.as_ref() {
            t.wait_for_requests(1, Duration::from_millis(1500)).await;
        }
        let body = self.session.build_setup_phase2().map_err(ap)?;
        let resp = send(&mut self.rtsp, RtspRequest::setup(self.session.request_uri(), body)).await?;
        if resp.status_code != 200 {
            return Err(ap(format!("the audio SETUP was refused ({})", resp.status_code)));
        }
        self.session.process_setup_phase2_response(resp.body.as_deref().unwrap_or(&[])).map_err(ap)?;
        let p = self.session.ports().ok_or_else(|| ap("SETUP gave no ports"))?;
        let (data_dest, control_dest) = (SocketAddr::new(self.addr, p.data_port), SocketAddr::new(self.addr, p.control_port));
        let resp = send(&mut self.rtsp, RtspRequest::record_with_info(self.session.request_uri(), 0, 0)).await?;
        if resp.status_code != 200 {
            tracing::warn!("airplay: RECORD answered {}", resp.status_code);
        }
        self.session.start_playing().ok();
        Ok(Streaming { data_dest, control_dest, stream_key: *self.session.stream_key(), control_socket, data_port: ports.data })
    }

    /// Timing requests from the receiver, or `None` before SETUP sent our timing port.
    /// 0 after that usually means a firewall.
    pub fn timing_requests(&self) -> Option<u64> {
        self.timing.as_ref().map(|t| t.requests_served())
    }

    /// Drops what the receiver has buffered. The next audio starts at `seq`/`rtptime`.
    pub async fn flush(&mut self, seq: u16, rtptime: u32) -> Result<()> {
        send(&mut self.rtsp, RtspRequest::flush_with_info(self.session.request_uri(), seq, rtptime)).await.map(|_| ())
    }

    /// Starts playing again after a FLUSH, from `seq`/`rtptime`.
    pub async fn record(&mut self, seq: u16, rtptime: u32) -> Result<()> {
        send(&mut self.rtsp, RtspRequest::record_with_info(self.session.request_uri(), seq, rtptime)).await.map(|_| ())
    }

    /// Sets the receiver's volume, 0 to 1. Only called when the listener asked.
    pub async fn volume(&mut self, volume: f32) -> Result<()> {
        let body = format!("volume: {:.6}\r\n", level_to_db(volume)).into_bytes();
        send(&mut self.rtsp, RtspRequest::set_parameter_text(self.session.request_uri(), body)).await.map(|_| ())
    }

    /// The receiver's volume, 0 to 1, from `GET_PARAMETER volume`.
    /// `None` when it refuses or doesn't say, and an error when it doesn't answer.
    pub async fn read_volume(&mut self) -> Result<Option<f32>> {
        let resp = send(&mut self.rtsp, RtspRequest::get_parameter_text(self.session.request_uri(), b"volume\r\n".to_vec())).await?;
        if resp.status_code != 200 {
            return Ok(None);
        }
        Ok(parse_volume(resp.body.as_deref().unwrap_or(&[])).map(db_to_level))
    }

    pub async fn metadata(&mut self, meta: &TrackMetadata, rtptime: u32) -> Result<()> {
        let body = navia_airplay::rtsp::dmap::encode_track_metadata(meta);
        send(&mut self.rtsp, RtspRequest::set_parameter_dmap(self.session.request_uri(), rtptime, body)).await.map(|_| ())
    }

    /// Cover art. An empty image with `image/none` clears it.
    pub async fn artwork(&mut self, image: Vec<u8>, content_type: &str, rtptime: u32) -> Result<()> {
        send(&mut self.rtsp, RtspRequest::set_parameter_artwork(self.session.request_uri(), rtptime, content_type, image)).await.map(|_| ())
    }

    pub async fn progress(&mut self, start: u32, current: u32, end: u32) -> Result<()> {
        send(&mut self.rtsp, RtspRequest::set_parameter_progress(self.session.request_uri(), start, current, end)).await.map(|_| ())
    }

    /// Keep-alive. Receivers drop a session without it.
    pub async fn feedback(&mut self) -> Result<()> {
        send(&mut self.rtsp, RtspRequest::feedback(self.session.request_uri())).await.map(|_| ())
    }

    pub async fn teardown(&mut self) {
        if self.session.state() != SessionState::Disconnected {
            let _ = self.session.start_teardown();
            let _ = tokio::time::timeout(Duration::from_secs(2), self.rtsp.send(RtspRequest::teardown(self.session.request_uri()))).await;
        }
        if let Some(mut t) = self.timing.take() {
            t.stop().await;
        }
        self.events = None;
        let _ = self.rtsp.close().await;
    }
}

/// Navia's 0 to 1 volume as the receiver's dB: -30 to 0, and -144 for silence.
pub(crate) fn level_to_db(level: f32) -> f32 {
    if level.is_nan() || level <= 0.0 { -144.0 } else { -30.0 + 30.0 * level.min(1.0) }
}

/// The receiver's dB on Navia's scale. -30 and below is 0, and 0 dB is 1.
pub(crate) fn db_to_level(db: f32) -> f32 {
    if db <= -30.0 { 0.0 } else { ((db + 30.0) / 30.0).clamp(0.0, 1.0) }
}

/// The dB in a `volume: -11.25` line, or `None` for any other answer.
fn parse_volume(body: &[u8]) -> Option<f32> {
    let text = std::str::from_utf8(body).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("volume") {
            return None;
        }
        value.trim().parse::<f32>().ok().filter(|v| v.is_finite())
    })
}

/// How long to wait for a fixed port the last session still holds before taking any port.
/// A random port wouldn't be covered by a narrow firewall rule.
const PORT_RETRY: Duration = Duration::from_millis(1500);

/// A UDP socket on `port`, or on any port if that one stays taken.
pub(crate) fn bind_udp(port: u16, what: &str) -> Result<UdpSocket> {
    let deadline = std::time::Instant::now() + PORT_RETRY;
    loop {
        match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => return Ok(s),
            Err(e) if port != 0 && std::time::Instant::now() < deadline && e.kind() == std::io::ErrorKind::AddrInUse => std::thread::sleep(Duration::from_millis(50)),
            Err(e) if port != 0 => {
                tracing::warn!("airplay: {what} port {port} is taken ({e}), using any port");
                return Ok(UdpSocket::bind(("0.0.0.0", 0))?);
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// The timing server on `port`, after a short wait for one just ended, else on any port.
async fn start_timing(port: u16) -> Result<NtpTimingServer> {
    let deadline = std::time::Instant::now() + PORT_RETRY;
    loop {
        match NtpTimingServer::start_on(SAMPLE_RATE, port).await {
            Ok(t) => return Ok(t),
            Err(_) if port != 0 && std::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(e) if port != 0 => {
                tracing::warn!("airplay: timing port {port} is taken ({e}), using any port");
                return NtpTimingServer::start_on(SAMPLE_RATE, 0).await.map_err(ap);
            }
            Err(e) => return Err(ap(e)),
        }
    }
}

/// Pair-verify with a stored pairing.
async fn verify(rtsp: &mut RtspConnection, controller: &ControllerIdentity, paired: &PairedDevice) -> Result<SessionKeys> {
    send(rtsp, RtspRequest::get_info()).await?;
    let mut pv = PairVerify::new_with_controller(controller);
    if let Some(ltpk) = unhex(&paired.ltpk).and_then(|v| <[u8; 32]>::try_from(v).ok()) {
        pv.set_server_ltpk(ltpk);
    }
    let m2 = send(rtsp, RtspRequest::pair_verify(pv.generate_m1().map_err(ap)?, 3)).await?;
    pv.process_m2(m2.body.as_deref().unwrap_or(&[])).map_err(|e| ap(format!("pairing was refused, pair again ({e})")))?;
    let m4 = send(rtsp, RtspRequest::pair_verify(pv.generate_m3().map_err(ap)?, 3)).await?;
    pv.process_m4(m4.body.as_deref().unwrap_or(&[])).map_err(|e| ap(format!("pairing was refused, pair again ({e})")))
}

/// HomeKit transient pairing: SRP with the fixed PIN, M1 to M4, nothing stored.
async fn transient(rtsp: &mut RtspConnection, client_id: &str) -> Result<SessionKeys> {
    let mut ps = PairSetup::new_transient_with_pin(TRANSIENT_PIN);
    let m2 = send(rtsp, RtspRequest::pair_setup(ps.generate_m1().map_err(ap)?, client_id, 4)).await?;
    ps.process_m2(m2.body.as_deref().unwrap_or(&[])).map_err(ap)?;
    let m4 = send(rtsp, RtspRequest::pair_setup(ps.generate_m3().map_err(ap)?, client_id, 4)).await?;
    ps.process_m4(m4.body.as_deref().unwrap_or(&[])).map_err(ap)?;
    let secret = ps.complete_transient().map_err(ap)?;
    SessionKeys::derive_control_keys(&secret).map_err(ap)
}

/// A PIN pairing in progress, while the receiver shows its PIN.
pub struct PinPairing {
    rtsp: RtspConnection,
    setup: PairSetup,
    m2: Vec<u8>,
    client_id: String,
    device_id: String,
    name: String,
}

/// Asks the receiver to show a PIN. Like pyatv, sends `/pair-pin-start` and pair-setup M1 on one
/// connection, since some receivers only show the PIN then.
pub async fn pair_begin(dev: &AirPlayDevice, ids: &RemoteIds) -> Result<PinPairing> {
    let (mut rtsp, _session, _addr, client_id) = open(dev, stream_config(Route { pairing: Pairing::Pin, pcm: false }), ids).await?;
    let start = RtspRequest::new(RtspMethod::Post, "/pair-pin-start").header("Content-Type", "application/octet-stream").header("X-Apple-HKP", "3");
    let resp = send(&mut rtsp, start).await?;
    if resp.status_code != 200 {
        return Err(ap(format!("the receiver wouldn't show a PIN ({})", resp.status_code)));
    }
    let mut setup = PairSetup::new("0000");
    let m2 = send(&mut rtsp, RtspRequest::pair_setup(setup.generate_m1().map_err(ap)?, &client_id, 3)).await?;
    Ok(PinPairing { rtsp, setup, m2: m2.body.unwrap_or_default(), client_id, device_id: dev.id.clone(), name: dev.name.clone() })
}

/// Finishes pairing with the PIN read off the receiver, and stores it.
pub async fn pair_finish(mut p: PinPairing, pin: &str, store: &PairingStore) -> Result<()> {
    let controller = controller(store)?;
    p.setup.set_pin(pin);
    p.setup.process_m2(&p.m2).map_err(ap)?;
    let m4 = send(&mut p.rtsp, RtspRequest::pair_setup(p.setup.generate_m3().map_err(ap)?, &p.client_id, 3)).await?;
    p.setup.process_m4(m4.body.as_deref().unwrap_or(&[])).map_err(|_| Error::AirPlay("wrong PIN".into()))?;
    let m6 = send(&mut p.rtsp, RtspRequest::pair_setup(p.setup.generate_m5_with_controller(&controller).map_err(ap)?, &p.client_id, 3)).await?;
    p.setup.process_m6(m6.body.as_deref().unwrap_or(&[])).map_err(ap)?;
    let ltpk = p.setup.server_ltpk().ok_or_else(|| ap("the receiver sent no key"))?;
    let identifier = p.setup.server_identifier().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    let _ = p.rtsp.close().await;
    store.save_device(&p.device_id, PairedDevice { name: p.name, ltpk: hex(&ltpk), identifier })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed port still held by the last session is waited for, not swapped for a random one.
    #[test]
    fn a_fixed_port_held_briefly_is_kept() {
        let held = UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let s = bind_udp(port, "test").unwrap();
        assert_eq!(s.local_addr().unwrap().port(), port);
        t.join().unwrap();
    }

    /// A port held for good falls back to any port rather than failing.
    #[test]
    fn a_fixed_port_held_for_good_falls_back() {
        let held = UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let s = bind_udp(port, "test").unwrap();
        assert_ne!(s.local_addr().unwrap().port(), port);
        drop(held);
    }

    #[tokio::test]
    async fn the_timing_server_keeps_its_fixed_port_across_a_reconnect() {
        let first = start_timing(0).await.unwrap();
        let port = first.port();
        drop(first);
        let second = start_timing(port).await.unwrap();
        assert_eq!(second.port(), port);
    }
}
