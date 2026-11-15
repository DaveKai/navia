//! A missing session bus must not hurt the app. The address points at a socket that doesn't exist, so the real session
//! bus is never contacted.
#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use navia_integrations::media_controls::{MediaControls, MediaControlsConfig};
use navia_integrations::{Integration, PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot};

struct Ignore;

impl PlayerControl for Ignore {
    fn command(&self, _command: PlayerCommand) {}
}

#[test]
fn updates_stop_quietly_without_a_bus() {
    // SAFETY: this file has one test and no other thread has started yet.
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/navia-test-bus") };

    let mut controls = MediaControls::new(
        MediaControlsConfig {
            display_name: "Navia Test".into(),
            dbus_name: "naviatest".into(),
            desktop_entry: "com.example.NaviaTest".into(),
            hwnd: None,
        },
        Arc::new(Ignore),
    )
    .unwrap();

    let snapshot = PlayerSnapshot {
        state: PlaybackState::Playing,
        volume: 1.0,
        ..Default::default()
    };
    let mut position = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !controls.failed() {
        assert!(Instant::now() < deadline, "the failure was never noticed");
        // A new position each time, so every update has something to send.
        position += 10;
        controls.update(&PlayerSnapshot {
            position: Duration::from_secs(position),
            ..snapshot.clone()
        });
        std::thread::sleep(Duration::from_millis(50));
    }
}
