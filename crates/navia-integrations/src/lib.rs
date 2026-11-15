//! Desktop integrations for Navia: Discord "Listening to" status and OS media controls (MPRIS on Linux, SMTC through
//! souvlaki on Windows).
//!
//! This crate doesn't depend on the player. It reads a [`PlayerSnapshot`] through the [`PlayerState`] trait and sends
//! commands back through [`PlayerControl`]. [`spawn_runner`] wires the two together.

pub mod art;
pub mod discord;
#[cfg(any(target_os = "linux", windows))]
pub mod metered;
#[cfg(target_os = "linux")]
#[path = "mpris.rs"]
pub mod media_controls;
#[cfg(target_os = "windows")]
pub mod media_controls;
mod state;

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub use state::{Integration, LoopMode, PlaybackState, PlayerCommand, PlayerControl, PlayerSnapshot, PlayerState, TrackInfo};

/// Polls a [`PlayerState`] and feeds the integrations from a background thread.
pub struct Runner {
    wake: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

/// Starts feeding `integrations` from `player` at least once per `interval`. Call [`Runner::notify`] on changes so
/// updates don't wait for the next tick.
pub fn spawn_runner(
    player: Arc<dyn PlayerState>,
    mut integrations: Vec<Box<dyn Integration + Send>>,
    interval: Duration,
) -> Runner {
    let (wake, rx) = mpsc::channel::<()>();
    let thread = thread::spawn(move || {
        loop {
            match rx.recv_timeout(interval) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {
                    let snapshot = player.snapshot();
                    for integration in &mut integrations {
                        integration.update(&snapshot);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    Runner {
        wake: Some(wake),
        thread: Some(thread),
    }
}

impl Runner {
    /// Wakes the runner so it reads the player state now.
    pub fn notify(&self) {
        if let Some(wake) = &self.wake {
            let _ = wake.send(());
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.wake.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    struct Counter(AtomicU64);

    impl PlayerState for Counter {
        fn snapshot(&self) -> PlayerSnapshot {
            PlayerSnapshot {
                position: Duration::from_secs(self.0.fetch_add(1, Ordering::SeqCst)),
                ..Default::default()
            }
        }
    }

    struct Recorder(Arc<Mutex<Vec<Duration>>>);

    impl Integration for Recorder {
        fn update(&mut self, snapshot: &PlayerSnapshot) {
            self.0.lock().unwrap().push(snapshot.position);
        }
    }

    fn wait_for_updates(seen: &Mutex<Vec<Duration>>, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < count {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn runner_feeds_on_notify_and_stops_on_drop() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = spawn_runner(
            Arc::new(Counter(AtomicU64::new(0))),
            vec![Box::new(Recorder(seen.clone()))],
            Duration::from_secs(3600),
        );
        runner.notify();
        wait_for_updates(&seen, 1);
        runner.notify();
        wait_for_updates(&seen, 2);
        drop(runner);
        let count = seen.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(seen.lock().unwrap().len(), count);
    }

    #[test]
    fn runner_polls_on_its_interval() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let _runner = spawn_runner(
            Arc::new(Counter(AtomicU64::new(0))),
            vec![Box::new(Recorder(seen.clone()))],
            Duration::from_millis(10),
        );
        wait_for_updates(&seen, 3);
    }
}
