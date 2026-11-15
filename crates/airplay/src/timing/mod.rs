//! NTP-style timing for realtime streams (upstream `airplay-timing`, without PTP for now).
mod clock;
mod ntp;
mod traits;

pub use clock::{Clock, ClockOffset, NTP_EPOCH_OFFSET, TimestampPair, ntp_to_unix, unix_to_ntp};
pub use ntp::{NtpTimingClient, NtpTimingServer};
pub use traits::TimingProtocol;
