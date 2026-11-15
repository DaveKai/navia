//! AirPlay 2 output on every platform: mDNS discovery, HomeKit pairing, and a realtime ALAC
//! stream with NTP timing. The protocol comes from `navia-airplay`, a trimmed airplay2-rs.

mod backend;
mod dacp;
mod device;
mod discovery;
pub mod firewall;
mod now_playing;
mod sender;
mod session;
mod store;
mod volume;

pub use backend::{AirPlay, AirPlayConfig, AirPlayOutput, ID_PREFIX, NowPlayingSource, OutputRouter, OutputState, TrackInfo};
pub use now_playing::ClientNowPlaying;
pub use dacp::{RemoteCommand, RemoteHandler};
pub use device::{AirPlayDevice, Pairing, Route, Support};
pub use session::Ports;
pub use store::{IDENTITY_KEY, PairedDevice, PairingStore, SecretStore};

/// Default local ports for receiver traffic, so one firewall rule covers them.
pub const DEFAULT_PORTS: Ports = Ports { timing: 51310, control: 51311, data: 51312, dacp: 51313 };
