//! The audio thread of an AirPlay output. It sends the engine's output in 352-frame packets at
//! 44.1 kHz, paced by the clock, with sync packets every second and answers to retransmit requests.
//! The receiver plays each frame `SYNC_LATENCY` after it is sent, so position reports lag by that much.

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use navia_airplay::audio::cipher::ChaChaPacketCipher;
use navia_airplay::audio::{AlacEncoder, AudioEncoder, RetransmitRequest, RtpSender};
use navia_airplay::base::{AudioCodec, AudioFormat, SampleRate};
use navia_airplay::crypto::AudioCipher;
use navia_airplay::timing::unix_to_ntp;
use tokio::sync::mpsc::UnboundedSender;

use super::session::{FRAMES_PER_PACKET, SAMPLE_RATE, Streaming};
use crate::audio::render::{RenderCore, RenderShared};
use crate::error::{Error, Result};

/// Frames the receiver is told to buffer: 1.5 s, as pyatv uses.
/// A sync packet says frame `rtp - SYNC_LATENCY` plays now while frame `rtp` is being sent.
pub const SYNC_LATENCY: u32 = 66_150;
/// The realtime audio payload type.
const PT_AUDIO: u8 = 0x60;
/// Retransmit request (receiver to sender).
const PT_RETRANSMIT_REQUEST: u8 = 0x55;
/// Packets of PCM kept to replay what a pause flushed from the receiver.
const KEEP_PACKETS: usize = (SYNC_LATENCY / FRAMES_PER_PACKET) as usize + 8;

/// Requests from the audio thread to the RTSP side.
pub(crate) enum Ctl {
    /// Drop what the receiver buffered. Audio continues at `seq`/`rtptime`.
    Flush { seq: u16, rtptime: u32 },
    /// Playing again after a pause.
    Resume { seq: u16, rtptime: u32 },
}

/// State the handle and the RTSP side read.
#[derive(Default)]
pub(crate) struct SenderState {
    pub stop: AtomicBool,
    pub failed: AtomicBool,
    pub packets: AtomicU64,
    pub retransmits: AtomicU64,
    /// Packets the receiver sent to our control port, which proves its packets get through.
    pub inbound: AtomicU64,
    /// RTP timestamp of the next packet.
    pub next_rtp: AtomicU32,
}

impl SenderState {
    /// The RTP time being heard now.
    pub fn heard_rtp(&self) -> u32 {
        self.next_rtp.load(Ordering::Acquire).wrapping_sub(SYNC_LATENCY)
    }
}

pub(crate) struct Sender {
    pub state: Arc<SenderState>,
    threads: Vec<JoinHandle<()>>,
}

impl Sender {
    pub fn start(streaming: Streaming, pcm: bool, core: Arc<Mutex<RenderCore>>, shared: Arc<RenderShared>, ctl: UnboundedSender<Ctl>) -> Result<Self> {
        let Streaming { data_dest, control_dest, stream_key, control_socket, data_port } = streaming;
        let mut rtp = RtpSender::new(data_dest, 0);
        rtp.set_control_dest(control_dest);
        // Like the other fixed ports, wait a moment for a session that just ended.
        let deadline = Instant::now() + Duration::from_millis(1500);
        while let Err(e) = rtp.bind(data_port) {
            if data_port == 0 {
                return Err(Error::AirPlay(format!("data socket: {e}")));
            }
            if Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            tracing::warn!("airplay: data port {data_port} is taken ({e}), using any port");
            rtp.bind(0).map_err(|e| Error::AirPlay(format!("data socket: {e}")))?;
            break;
        }
        rtp.set_control_socket(control_socket.try_clone()?);
        rtp.set_cipher(Box::new(ChaChaPacketCipher::new(AudioCipher::new(stream_key))));
        let rtp = Arc::new(Mutex::new(rtp));
        let state = Arc::new(SenderState::default());
        let encoder: Box<dyn AudioEncoder> = if pcm {
            Box::new(Pcm)
        } else {
            Box::new(AlacEncoder::new(AudioFormat { codec: AudioCodec::Alac, sample_rate: SampleRate::Hz44100, bit_depth: 16, channels: 2, frames_per_packet: FRAMES_PER_PACKET }).map_err(|e| Error::AirPlay(e.to_string()))?)
        };
        let sync_socket = control_socket.try_clone()?;
        let mut threads = Vec::new();
        {
            let (state, rtp) = (state.clone(), rtp.clone());
            threads.push(
                thread::Builder::new()
                    .name("navia-airplay-send".into())
                    .spawn(move || {
                        let mut run = Run { rtp, encoder, core, shared, ctl, sync_socket, control_dest, state: state.clone(), keep: VecDeque::with_capacity(KEEP_PACKETS), replay: VecDeque::new() };
                        run.run();
                    })
                    .map_err(Error::Io)?,
            );
        }
        {
            let (state, rtp) = (state.clone(), rtp.clone());
            control_socket.set_read_timeout(Some(Duration::from_millis(100)))?;
            threads.push(
                thread::Builder::new()
                    .name("navia-airplay-ctl".into())
                    .spawn(move || retransmits(control_socket, rtp, state))
                    .map_err(Error::Io)?,
            );
        }
        Ok(Self { state, threads })
    }

    pub fn stop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Raw 16-bit big-endian PCM, the fallback codec.
struct Pcm;

impl AudioEncoder for Pcm {
    fn encode(&mut self, samples: &[i16]) -> navia_airplay::Result<navia_airplay::audio::EncodedPacket> {
        Ok(navia_airplay::audio::EncodedPacket { data: samples.iter().flat_map(|s| s.to_be_bytes()).collect(), samples: (samples.len() / 2) as u32, timestamp: 0 })
    }
    fn codec(&self) -> AudioCodec {
        AudioCodec::Pcm
    }
    fn frames_per_packet(&self) -> u32 {
        FRAMES_PER_PACKET
    }
    fn flush(&mut self) -> navia_airplay::Result<Option<navia_airplay::audio::EncodedPacket>> {
        Ok(None)
    }
}

struct Run {
    rtp: Arc<Mutex<RtpSender>>,
    encoder: Box<dyn AudioEncoder>,
    core: Arc<Mutex<RenderCore>>,
    shared: Arc<RenderShared>,
    ctl: UnboundedSender<Ctl>,
    sync_socket: UdpSocket,
    control_dest: SocketAddr,
    state: Arc<SenderState>,
    /// The last packets sent, as PCM, to replay after a pause.
    keep: VecDeque<Vec<i16>>,
    /// Packets to send before reading more from the engine.
    replay: VecDeque<Vec<i16>>,
}

impl Run {
    fn run(&mut self) {
        let packet = Duration::from_secs_f64(f64::from(FRAMES_PER_PACKET) / f64::from(SAMPLE_RATE));
        let mut rtp_time: u32 = 0;
        let mut next = Instant::now();
        let mut first = true;
        let mut last_sync: Option<u32> = None;
        let mut paused = false;
        let mut discontinuity = self.shared.discontinuity.load(Ordering::Acquire);
        let mut block = vec![0f32; FRAMES_PER_PACKET as usize * 2];
        while !self.state.stop.load(Ordering::Acquire) {
            let now_paused = self.shared.paused.load(Ordering::Acquire);
            if now_paused && !paused {
                paused = true;
                // The receiver drops its buffer at once. What it hadn't played is sent again on resume.
                let seq = self.rtp.lock().expect("rtp").sequence();
                let _ = self.ctl.send(Ctl::Flush { seq, rtptime: rtp_time });
                let unheard = self.keep.len().min((SYNC_LATENCY / FRAMES_PER_PACKET) as usize);
                let tail: Vec<Vec<i16>> = self.keep.drain(self.keep.len() - unheard..).collect();
                self.keep.clear();
                tail.into_iter().rev().for_each(|p| self.replay.push_front(p));
            }
            if paused {
                if now_paused {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                paused = false;
                let replay_frames = self.replay.len() as u64 * u64::from(FRAMES_PER_PACKET);
                self.core.lock().expect("core").begin_replay(replay_frames);
                first = true;
                last_sync = None;
                next = Instant::now();
                let seq = self.rtp.lock().expect("rtp").sequence();
                let _ = self.ctl.send(Ctl::Resume { seq, rtptime: rtp_time });
            }
            let now = Instant::now();
            if now < next {
                thread::sleep((next - now).min(Duration::from_millis(4)));
                continue;
            }
            // Fell far behind, as after sleep. Start over from now rather than burst.
            if now.duration_since(next) > Duration::from_millis(250) {
                next = now;
            }
            let pcm = match self.replay.pop_front() {
                Some(p) => {
                    self.core.lock().expect("core").replayed(u64::from(FRAMES_PER_PACKET));
                    p
                }
                None => {
                    self.core.lock().expect("core").render(&mut block);
                    let d = self.shared.discontinuity.load(Ordering::Acquire);
                    if d != discontinuity {
                        // A seek or skip, so what the receiver holds is stale.
                        discontinuity = d;
                        let seq = self.rtp.lock().expect("rtp").sequence();
                        let _ = self.ctl.send(Ctl::Flush { seq, rtptime: rtp_time });
                        self.keep.clear();
                        first = true;
                        last_sync = None;
                    }
                    block.iter().map(|&s| to_i16(s)).collect()
                }
            };
            if last_sync.is_none_or(|t| rtp_time.wrapping_sub(t) >= SAMPLE_RATE) {
                self.send_sync(rtp_time, first);
                last_sync = Some(rtp_time);
            }
            match self.encoder.encode(&pcm) {
                Ok(encoded) => {
                    if let Err(e) = self.rtp.lock().expect("rtp").send_audio(PT_AUDIO, rtp_time, &encoded.data, first) {
                        tracing::warn!("airplay: send failed: {e}");
                    }
                }
                Err(e) => tracing::warn!("airplay: encode failed: {e}"),
            }
            if self.keep.len() == KEEP_PACKETS {
                self.keep.pop_front();
            }
            self.keep.push_back(pcm);
            first = false;
            rtp_time = rtp_time.wrapping_add(FRAMES_PER_PACKET);
            self.state.next_rtp.store(rtp_time, Ordering::Release);
            self.state.packets.fetch_add(1, Ordering::Relaxed);
            next += packet;
        }
    }

    /// A RAOP sync packet (PT 0x54) as Apple senders and pyatv build it.
    /// It carries the frame playing now (`rtp - latency`), its NTP time, and the frame being sent.
    fn send_sync(&self, rtp: u32, first: bool) {
        let mut p = [0u8; 20];
        p[0] = if first { 0x90 } else { 0x80 };
        p[1] = 0xd4;
        p[2..4].copy_from_slice(&7u16.to_be_bytes());
        p[4..8].copy_from_slice(&rtp.wrapping_sub(SYNC_LATENCY).to_be_bytes());
        let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
        p[8..16].copy_from_slice(&unix_to_ntp(wall).to_be_bytes());
        p[16..20].copy_from_slice(&rtp.to_be_bytes());
        if let Err(e) = self.sync_socket.send_to(&p, self.control_dest) {
            tracing::debug!("airplay: sync failed: {e}");
        }
    }
}

/// Same conversion as the cpal outputs, so a 16-bit source arrives bit for bit.
fn to_i16(s: f32) -> i16 {
    cpal::Sample::to_sample::<i16>(s)
}

/// Answers the receiver's retransmit requests from the kept packets.
fn retransmits(socket: UdpSocket, rtp: Arc<Mutex<RtpSender>>, state: Arc<SenderState>) {
    let mut buf = [0u8; 2048];
    while !state.stop.load(Ordering::Acquire) {
        let Ok((n, _)) = socket.recv_from(&mut buf) else { continue };
        state.inbound.fetch_add(1, Ordering::Relaxed);
        let data = &buf[..n];
        if n < 8 || data[1] & 0x7f != PT_RETRANSMIT_REQUEST {
            continue;
        }
        let req = if n == 8 {
            Some(RetransmitRequest { first_sequence: u16::from_be_bytes([data[4], data[5]]), count: u16::from_be_bytes([data[6], data[7]]) })
        } else {
            RetransmitRequest::parse(data).ok()
        };
        if let Some(req) = req
            && let Ok(sent) = rtp.lock().expect("rtp").handle_retransmit(&req)
        {
            state.retransmits.fetch_add(u64::from(sent), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::render::Marker;
    use ringbuf::HeapRb;
    use ringbuf::traits::{Producer as _, Split as _};

    #[test]
    fn sixteen_bit_conversion_is_exact_for_sixteen_bit_sources() {
        for v in [-32768i16, -32767, -1, 0, 1, 12345, 32767] {
            assert_eq!(to_i16(f32::from(v) / 32768.0), v);
        }
        assert_eq!(to_i16(2.0), 32767);
    }

    /// Frame `f` as the engine hands it over, with values that don't repeat nearby.
    fn frame(f: usize) -> [i16; 2] {
        [(((f * 7) % 65_536) as i32 - 32_768) as i16, (((f * 13 + 5) % 65_536) as i32 - 32_768) as i16]
    }

    struct Packet {
        seq: u16,
        rtp: u32,
        samples: Vec<i16>,
    }

    fn receive(data: &UdpSocket, key: [u8; 32], out: &mut Vec<Packet>) {
        let cipher = AudioCipher::new(key);
        let mut buf = [0u8; 4096];
        let until = Instant::now() + Duration::from_millis(50);
        while Instant::now() < until
            && let Ok(n) = data.recv(&mut buf)
        {
            let p = &buf[..n];
            let (seq, rtp) = (u16::from_be_bytes([p[2], p[3]]), u32::from_be_bytes([p[4], p[5], p[6], p[7]]));
            let body = &p[12..n - 24];
            let tag: [u8; 16] = p[n - 24..n - 8].try_into().unwrap();
            let nonce: [u8; 8] = p[n - 8..].try_into().unwrap();
            let plain = cipher.decrypt(body, &nonce, &tag, rtp, 0).expect("decrypts with the session key");
            out.push(Packet { seq, rtp, samples: plain.chunks_exact(2).map(|b| i16::from_be_bytes([b[0], b[1]])).collect() });
        }
    }

    /// The sender over loopback in PCM: every frame arrives bit for bit and numbered without gaps.
    /// A pause flushes, and the resume resends the unheard tail before new audio.
    #[test]
    fn loopback_is_exact_paced_and_replays_after_a_pause() {
        let frames = 44_100 * 3;
        let (mut prod, cons) = HeapRb::<f32>::new(frames * 2).split();
        let (_mprod, mcons) = HeapRb::<Marker>::new(8).split();
        let (mut core, shared) = RenderCore::new(cons, mcons);
        core.set_stream_rate(SAMPLE_RATE);
        let audio: Vec<f32> = (0..frames).flat_map(|f| frame(f).map(|v| f32::from(v) / 32_768.0)).collect();
        prod.push_slice(&audio);
        let core = Arc::new(Mutex::new(core));

        let data = UdpSocket::bind("127.0.0.1:0").unwrap();
        data.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let sync = UdpSocket::bind("127.0.0.1:0").unwrap();
        sync.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let key = [7u8; 32];
        let streaming = Streaming { data_dest: data.local_addr().unwrap(), control_dest: sync.local_addr().unwrap(), stream_key: key, control_socket: UdpSocket::bind("127.0.0.1:0").unwrap(), data_port: 0 };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sender = Sender::start(streaming, true, core.clone(), shared.clone(), tx).unwrap();

        let mut got = Vec::new();
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(600) {
            receive(&data, key, &mut got);
        }
        let before = got.len();
        // At real time, 0.6 s is about 75 packets.
        assert!((60..=90).contains(&before), "{before} packets in 0.6 s");
        shared.paused.store(true, Ordering::Release);
        thread::sleep(Duration::from_millis(100));
        receive(&data, key, &mut got);
        let paused_at = got.len();
        match rx.try_recv() {
            Ok(Ctl::Flush { seq, rtptime }) => {
                assert_eq!(seq, got.last().unwrap().seq.wrapping_add(1));
                assert_eq!(rtptime, got.last().unwrap().rtp + FRAMES_PER_PACKET);
            }
            _ => panic!("a pause flushes the receiver"),
        }
        thread::sleep(Duration::from_millis(100));
        receive(&data, key, &mut got);
        assert_eq!(got.len(), paused_at, "nothing is sent while paused");
        shared.paused.store(false, Ordering::Release);
        let resumed = Instant::now();
        while resumed.elapsed() < Duration::from_millis(1200) {
            receive(&data, key, &mut got);
        }
        assert!(matches!(rx.try_recv(), Ok(Ctl::Resume { .. })));
        sender.stop();

        // No gap in the numbering anywhere, through the pause.
        for w in got.windows(2) {
            assert_eq!(w[1].seq, w[0].seq.wrapping_add(1));
            assert_eq!(w[1].rtp, w[0].rtp + FRAMES_PER_PACKET);
        }
        // Frames up to the pause, then the unheard ones again, then on from where it stopped.
        // All of them are resent here, since less than the 1.5 s latency had gone out.
        let flat: Vec<i16> = got.iter().flat_map(|p| p.samples.iter().copied()).collect();
        let sent = paused_at * FRAMES_PER_PACKET as usize;
        let replay = paused_at.min((SYNC_LATENCY / FRAMES_PER_PACKET) as usize) * FRAMES_PER_PACKET as usize;
        let expected: Vec<i16> = (0..sent).chain(sent - replay..sent).chain(sent..).take(flat.len() / 2).flat_map(frame).collect();
        assert_eq!(flat.len(), expected.len());
        let first_wrong = flat.iter().zip(&expected).position(|(a, b)| a != b);
        assert_eq!(first_wrong, None, "every sample arrives exactly");

        // The first sync packet is marked, then one comes every second.
        let mut buf = [0u8; 64];
        let mut syncs = Vec::new();
        while let Ok(n) = sync.recv(&mut buf) {
            syncs.push(buf[..n].to_vec());
        }
        assert!(syncs.len() >= 2, "{} sync packets", syncs.len());
        assert_eq!(syncs[0][0], 0x90);
        for p in &syncs {
            assert_eq!(p.len(), 20);
            assert_eq!(p[1], 0xd4);
            let now_minus = u32::from_be_bytes(p[4..8].try_into().unwrap());
            let now = u32::from_be_bytes(p[16..20].try_into().unwrap());
            assert_eq!(now.wrapping_sub(now_minus), SYNC_LATENCY);
        }
    }
}
