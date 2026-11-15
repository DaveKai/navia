//! AirPlay 2 sender protocol pieces, vendored and trimmed from airplay2-rs
//! (https://github.com/lmcgartland/airplay2-rs, commit e2e63e2, GPL-3.0-or-later; see README.md
//! for what was kept, removed and changed). Navia's sender, which streams its engine's output,
//! lives in `navia-core` (`audio::airplay`).
#![allow(clippy::all)]
// Upstream code, kept close to its source so fixes can be compared; its warnings aren't ours to chase.
#![allow(deprecated, dead_code, unused_imports, unused_variables, unused_mut, unexpected_cfgs, unused_assignments)]

pub mod audio;
pub mod base;
pub mod crypto;
pub mod discovery;
pub mod pairing;
pub mod rtsp;
pub mod timing;

pub use base::{Device, DeviceId, Error, Features, Result};
