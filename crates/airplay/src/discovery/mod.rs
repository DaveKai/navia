//! mDNS TXT parsing for AirPlay receivers (upstream `airplay-discovery`).
mod parser;

pub use parser::TxtRecordParser;

/// AirPlay 2 service type for mDNS discovery.
pub const AIRPLAY_SERVICE_TYPE: &str = "_airplay._tcp.local.";
/// RAOP (AirPlay 1) service type.
pub const RAOP_SERVICE_TYPE: &str = "_raop._tcp.local.";
