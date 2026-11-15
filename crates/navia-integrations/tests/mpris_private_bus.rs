//! Runs against a private `dbus-daemon` started by this test, so the real session bus is never contacted. This file
//! holds a single test because it changes `DBUS_SESSION_BUS_ADDRESS` for the process.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use navia_integrations::media_controls::{MediaControls, MediaControlsConfig};
use navia_integrations::{Integration, LoopMode, PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot, TrackInfo};
use zbus::blocking::{Connection, MessageIterator, Proxy, connection::Builder};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::MatchRule;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.naviatest";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const ROOT: &str = "org.mpris.MediaPlayer2";

struct PrivateBus {
    daemon: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    fn start() -> Self {
        let dir = tempfile::Builder::new().prefix("nv-bus").tempdir_in("/tmp").unwrap();
        let address = format!("unix:path={}/bus", dir.path().display());
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
            .arg(format!("--address={address}"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon");
        let stdout = daemon.stdout.take().unwrap();
        let printed = BufReader::new(stdout).lines().next().unwrap().unwrap();
        assert!(printed.starts_with(&address), "unexpected address {printed}");
        Self {
            daemon,
            address,
            _dir: dir,
        }
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

struct Commands(Mutex<Sender<PlayerCommand>>);

impl PlayerControl for Commands {
    fn command(&self, command: PlayerCommand) {
        let _ = self.0.lock().unwrap().send(command);
    }
}

fn expect(received: &Receiver<PlayerCommand>, wanted: PlayerCommand) {
    let got = received.recv_timeout(Duration::from_secs(5)).expect("command");
    assert_eq!(got, wanted);
}

fn snapshot(state: PlaybackState, position: u64) -> PlayerSnapshot {
    PlayerSnapshot {
        state,
        track: Some(TrackInfo {
            id: "1".into(),
            title: "A Song".into(),
            artist: "An Artist".into(),
            album: "An Album".into(),
            duration: Some(Duration::from_secs(200)),
            cover_url: Some("file:///tmp/cover.jpg".into()),
            ..Default::default()
        }),
        position: Duration::from_secs(position),
        volume: 0.5,
        private_mode: false,
        loop_mode: LoopMode::None,
        shuffle: false,
        can_change_modes: true,
    }
}

fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn mpris_on_a_private_bus() {
    let bus = PrivateBus::start();
    let original = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    assert_ne!(original.as_deref(), Some(bus.address.as_str()));
    // SAFETY: this file has one test and no other thread has started yet.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus.address) };

    let client: Connection = Builder::address(bus.address.as_str()).unwrap().build().unwrap();
    let dbus = Proxy::new(&client, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").unwrap();

    let (sender, received) = mpsc::channel();
    let mut controls = MediaControls::new(
        MediaControlsConfig {
            display_name: "Navia Test".into(),
            dbus_name: "naviatest".into(),
            desktop_entry: "com.example.NaviaTest".into(),
            hwnd: None,
        },
        Arc::new(Commands(Mutex::new(sender))),
    )
    .unwrap();

    wait_until("the bus name", || {
        let names: Vec<String> = dbus.call("ListNames", &()).unwrap();
        names.iter().any(|name| name == BUS_NAME)
    });

    let player = Proxy::new(&client, BUS_NAME, PATH, PLAYER).unwrap();
    let root = Proxy::new(&client, BUS_NAME, PATH, ROOT).unwrap();
    assert_eq!(root.get_property::<String>("Identity").unwrap(), "Navia Test");
    assert_eq!(root.get_property::<String>("DesktopEntry").unwrap(), "com.example.NaviaTest");
    assert!(!root.get_property::<bool>("CanRaise").unwrap());
    assert!(!root.get_property::<bool>("HasTrackList").unwrap());

    // Every signal from the player's path, to check what is and isn't sent.
    let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).path(PATH).unwrap().build();
    let signals = MessageIterator::for_match_rule(rule, &client, Some(64)).unwrap();
    let (sig_tx, sig_rx) = mpsc::channel::<(String, String)>();
    std::thread::spawn(move || {
        for message in signals {
            let Ok(message) = message else { break };
            let header = message.header();
            let member = header.member().map(|m| m.to_string()).unwrap_or_default();
            let detail = if member == "Seeked" {
                message.body().deserialize::<i64>().map(|p| p.to_string()).unwrap_or_default()
            } else {
                message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    .map(|(_, changed, _)| {
                        let mut keys: Vec<_> = changed.into_keys().collect();
                        keys.sort();
                        keys.join(",")
                    })
                    .unwrap_or_default()
            };
            if sig_tx.send((member, detail)).is_err() {
                break;
            }
        }
    });
    let next_signal = || sig_rx.recv_timeout(Duration::from_secs(5)).expect("signal");
    let no_signal = |what: &str| {
        if let Ok(signal) = sig_rx.recv_timeout(Duration::from_millis(400)) {
            panic!("{what}: unexpected {signal:?}");
        }
    };

    // Properties are read with GetAll because Proxy caches single properties.
    let all = || -> HashMap<String, OwnedValue> {
        let properties = Proxy::new(&client, BUS_NAME, PATH, "org.freedesktop.DBus.Properties").unwrap();
        properties.call("GetAll", &(PLAYER)).unwrap()
    };
    let status = |map: &HashMap<String, OwnedValue>| -> String {
        String::try_from(map["PlaybackStatus"].try_clone().unwrap()).unwrap()
    };
    let position = |map: &HashMap<String, OwnedValue>| -> i64 {
        i64::try_from(&map["Position"]).unwrap()
    };

    controls.update(&snapshot(PlaybackState::Playing, 30));
    wait_until("Playing", || status(&all()) == "Playing");
    let first = next_signal();
    assert_eq!(first.0, "PropertiesChanged");
    assert_eq!(first.1, "CanGoNext,CanGoPrevious,CanPause,CanPlay,CanSeek,LoopStatus,Metadata,PlaybackStatus,Shuffle,Volume");

    // Position advances by itself while playing, without updates or signals.
    let properties = all();
    let at = position(&properties);
    assert!((30_000_000..31_000_000).contains(&at), "{at}");
    std::thread::sleep(Duration::from_millis(600));
    let later = position(&all());
    assert!(later >= at + 500_000, "{at} then {later}");
    // A report that agrees with the clock sends nothing.
    controls.update(&snapshot(PlaybackState::Playing, 31));
    no_signal("a steady position");
    assert_eq!(f64::try_from(&properties["Volume"]).unwrap(), 0.5);

    let metadata: HashMap<String, OwnedValue> = properties["Metadata"].try_clone().unwrap().try_into().unwrap();
    let text = |key: &str| String::try_from(metadata[key].try_clone().unwrap()).unwrap();
    assert_eq!(text("xesam:title"), "A Song");
    assert_eq!(text("xesam:album"), "An Album");
    assert_eq!(text("mpris:artUrl"), "file:///tmp/cover.jpg");
    let artists: Vec<String> = metadata["xesam:artist"].try_clone().unwrap().try_into().unwrap();
    assert_eq!(artists, vec!["An Artist".to_owned()]);
    assert_eq!(i64::try_from(&metadata["mpris:length"]).unwrap(), 200_000_000);

    controls.update(&snapshot(PlaybackState::Paused, 31));
    wait_until("Paused", || status(&all()) == "Paused");
    assert_eq!(next_signal(), ("PropertiesChanged".into(), "PlaybackStatus".into()));
    assert_eq!(position(&all()), 31_000_000);

    // A jump is a seek. It sends Seeked with the new place and no PropertiesChanged.
    controls.update(&snapshot(PlaybackState::Paused, 120));
    assert_eq!(next_signal(), ("Seeked".into(), "120000000".into()));
    assert_eq!(position(&all()), 120_000_000);
    no_signal("after a seek");

    for (method, wanted) in [
        ("Play", PlayerCommand::Play),
        ("Pause", PlayerCommand::Pause),
        ("PlayPause", PlayerCommand::PlayPause),
        ("Stop", PlayerCommand::Stop),
        ("Next", PlayerCommand::Next),
        ("Previous", PlayerCommand::Previous),
    ] {
        let _: () = player.call(method, &()).unwrap();
        expect(&received, wanted);
    }

    let _: () = player.call("Seek", &(5_000_000i64)).unwrap();
    expect(&received, PlayerCommand::SeekForward(Duration::from_secs(5)));
    let _: () = player.call("Seek", &(-3_000_000i64)).unwrap();
    expect(&received, PlayerCommand::SeekBackward(Duration::from_secs(3)));

    let metadata: HashMap<String, OwnedValue> = all()["Metadata"].try_clone().unwrap().try_into().unwrap();
    let track = OwnedObjectPath::try_from(metadata["mpris:trackid"].try_clone().unwrap()).unwrap();
    // Per the spec, another track's id or a position past the end is ignored.
    let other = ObjectPath::try_from("/org/navia/track/tsomethingelse").unwrap();
    let _: () = player.call("SetPosition", &(other, 42_000_000i64)).unwrap();
    let _: () = player.call("SetPosition", &(track.clone(), 900_000_000i64)).unwrap();
    let _: () = player.call("SetPosition", &(track, 42_000_000i64)).unwrap();
    expect(&received, PlayerCommand::SeekTo(Duration::from_secs(42)));

    player.set_property("Volume", 0.25f64).unwrap();
    expect(&received, PlayerCommand::SetVolume(0.25));

    // Repeat as LoopStatus (None, Track for repeat one, Playlist for repeat all) and Shuffle, both ways. A change is
    // signaled, and an unknown value or a station's lock is ignored.
    let text_of = |key: &str| String::try_from(all()[key].try_clone().unwrap()).unwrap();
    assert_eq!(text_of("LoopStatus"), "None");
    assert!(!bool::try_from(&all()["Shuffle"]).unwrap());
    for (status, mode) in [("Track", LoopMode::Track), ("Playlist", LoopMode::Playlist), ("None", LoopMode::None)] {
        player.set_property("LoopStatus", status).unwrap();
        expect(&received, PlayerCommand::SetLoop(mode));
    }
    player.set_property("Shuffle", true).unwrap();
    expect(&received, PlayerCommand::SetShuffle(true));
    let _ = player.set_property("LoopStatus", "Sometimes");
    // Setting a property through zbus signals that property by itself, so drain those first.
    while sig_rx.recv_timeout(Duration::from_millis(300)).is_ok() {}
    controls.update(&PlayerSnapshot { loop_mode: LoopMode::Track, shuffle: true, ..snapshot(PlaybackState::Paused, 120) });
    assert_eq!(next_signal(), ("PropertiesChanged".into(), "LoopStatus,Shuffle".into()));
    assert_eq!(text_of("LoopStatus"), "Track");
    assert!(bool::try_from(&all()["Shuffle"]).unwrap());
    controls.update(&PlayerSnapshot { loop_mode: LoopMode::Track, shuffle: true, can_change_modes: false, ..snapshot(PlaybackState::Paused, 120) });
    player.set_property("LoopStatus", "None").unwrap();
    player.set_property("Shuffle", false).unwrap();
    assert!(received.recv_timeout(Duration::from_millis(400)).is_err(), "a station keeps repeat and shuffle");

    let _: () = root.call("Raise", &()).unwrap();
    expect(&received, PlayerCommand::Raise);

    controls.update(&PlayerSnapshot {
        state: PlaybackState::Stopped,
        volume: 0.5,
        ..Default::default()
    });
    wait_until("Stopped", || status(&all()) == "Stopped");
    // With no track, nothing can be played or sought.
    assert!(!bool::try_from(&all()["CanSeek"]).unwrap());
    assert!(!bool::try_from(&all()["CanPlay"]).unwrap());

    // A second instance can't take the name, runs without media controls, and leaves the first one working.
    let (other_tx, _other_rx) = mpsc::channel();
    let second = MediaControls::new(
        MediaControlsConfig {
            display_name: "Navia Two".into(),
            dbus_name: "naviatest".into(),
            desktop_entry: "com.example.NaviaTest".into(),
            hwnd: None,
        },
        Arc::new(Commands(Mutex::new(other_tx))),
    )
    .unwrap();
    wait_until("the second instance to give up", || second.failed());
    assert_eq!(root.get_property::<String>("Identity").unwrap(), "Navia Test");
    drop(second);
    let _: () = player.call("Next", &()).unwrap();
    expect(&received, PlayerCommand::Next);

    drop(controls);
    wait_until("the name to go away", || {
        let names: Vec<String> = dbus.call("ListNames", &()).unwrap();
        !names.iter().any(|name| name == BUS_NAME)
    });
}
