//! An output with its own volume, like AirPlay. The slider follows the device, starts quiet when it
//! won't say, and nothing is sent unless the listener changes the volume.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::audio::{synth, wav_bytes};
use common::http::{Faults, Library, TestServer};
use navia_core::Result;
use navia_core::audio::output::{CaptureBackend, DeviceCaps, DeviceInfo, OutputBackend, OutputHandle};
use navia_core::audio::player::{ASSUMED_DEVICE_VOLUME, DeviceVolume, OutputPolicy, Player, PlayerConfig, PlayerEvent, QueueItem, ServerResolver};
use navia_core::audio::render::RenderCore;
use navia_core::client::{Client, ClientConfig, Credentials};

type Report = Arc<Mutex<Option<(u64, f32)>>>;
type Sent = Arc<Mutex<Vec<f32>>>;
type Events = Arc<Mutex<Vec<Option<DeviceVolume>>>>;

/// The capture device, with a volume of its own that reports `report` and logs what's sent.
struct RemoteBackend {
    inner: CaptureBackend,
    report: Report,
    sent: Sent,
}

struct RemoteHandle {
    inner: Box<dyn OutputHandle>,
    report: Report,
    sent: Sent,
}

impl OutputHandle for RemoteHandle {
    fn latency_frames(&self) -> u64 {
        self.inner.latency_frames()
    }
    fn failed(&self) -> bool {
        false
    }
    fn rate(&self) -> u32 {
        self.inner.rate()
    }
    fn xruns(&self) -> u64 {
        0
    }
    fn remote_volume(&self) -> bool {
        true
    }
    fn set_volume(&self, volume: f32) {
        self.sent.lock().unwrap().push(volume);
    }
    fn reported_volume(&self) -> Option<(u64, f32)> {
        *self.report.lock().unwrap()
    }
}

impl OutputBackend for RemoteBackend {
    fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        self.inner.list_devices()
    }
    fn caps(&self, device: Option<&str>) -> Result<DeviceCaps> {
        self.inner.caps(device)
    }
    fn open(&mut self, device: Option<&str>, rate: u32, core: Arc<Mutex<RenderCore>>) -> Result<Box<dyn OutputHandle>> {
        let inner = self.inner.open(device, rate, core)?;
        Ok(Box::new(RemoteHandle { inner, report: self.report.clone(), sent: self.sent.clone() }))
    }
}

fn wait_for(limit: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// A player on the remote-volume device, playing a 20 s song, with its `DeviceVolume` events.
fn playing(report: Option<(u64, f32)>) -> (TestServer, Player, Report, Sent, Events) {
    let mut library = Library::default();
    library.raw.insert("a".into(), wav_bytes(&synth(44_100 * 20, 44_100, 0.3), 44_100));
    let server = TestServer::start(library, Faults::default());
    let (report, sent): (Report, Sent) = (Arc::new(Mutex::new(report)), Arc::default());
    let backend = RemoteBackend { inner: CaptureBackend { speed: 1.0, rates: (8_000, 192_000), ..Default::default() }, report: report.clone(), sent: sent.clone() };
    let client = Client::new(ClientConfig::new(server.url.clone(), Credentials::new("t".to_string(), "t".to_string()))).unwrap();
    let p = Player::with_backend(Arc::new(ServerResolver::new(client).unwrap()), Box::new(backend), PlayerConfig { policy: OutputPolicy::FollowTrack, ..Default::default() }).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    p.add_listener(move |e| {
        if let PlayerEvent::DeviceVolume(v) = e {
            sink.lock().unwrap().push(*v);
        }
    });
    p.set_queue(vec![QueueItem::new("a".into(), Some(20_000))], 0, 0, false);
    assert!(wait_for(Duration::from_secs(5), || p.snapshot().device_volume.is_some()), "the output never opened");
    (server, p, report, sent, events)
}

#[test]
fn the_slider_starts_where_the_device_is_and_follows_it() {
    let (_server, p, report, sent, events) = playing(Some((1, 0.3)));
    let known = |level| Some(DeviceVolume { level, known: true });
    assert_eq!(p.snapshot().device_volume, known(0.3));
    // The device turns itself up, which is shown but not sent back.
    *report.lock().unwrap() = Some((2, 0.6));
    assert!(wait_for(Duration::from_secs(1), || p.snapshot().device_volume == known(0.6)));
    std::thread::sleep(Duration::from_millis(100));
    assert!(sent.lock().unwrap().is_empty(), "sent {:?} unasked", sent.lock().unwrap());
    assert_eq!(*events.lock().unwrap(), vec![known(0.3), known(0.6)]);

    // The listener moves it, so the device's volume changes and the player's own doesn't.
    p.set_volume(0.5);
    p.set_volume(0.5);
    p.set_muted(true);
    p.set_muted(false);
    p.sync();
    assert_eq!(*sent.lock().unwrap(), vec![0.5, 0.0, 0.5]);
    let s = p.snapshot();
    assert_eq!((s.volume, s.device_volume), (1.0, known(0.5)));
}

#[test]
fn a_device_that_wont_say_starts_quiet_and_gets_nothing_unasked() {
    let (_server, p, _report, sent, _events) = playing(None);
    assert_eq!(p.snapshot().device_volume, Some(DeviceVolume { level: ASSUMED_DEVICE_VOLUME, known: false }));
    std::thread::sleep(Duration::from_millis(300));
    assert!(sent.lock().unwrap().is_empty());
    // The first move goes from the quiet assumption, as the slider shows it.
    p.set_volume(ASSUMED_DEVICE_VOLUME + 0.05);
    p.sync();
    assert_eq!(*sent.lock().unwrap(), vec![ASSUMED_DEVICE_VOLUME + 0.05]);
    assert_eq!(p.snapshot().device_volume, Some(DeviceVolume { level: ASSUMED_DEVICE_VOLUME + 0.05, known: true }));
}
