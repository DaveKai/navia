//! HomeKit pair-setup and pair-verify (upstream `airplay-pairing`).
mod controller;
mod pair_setup;
mod pair_verify;

pub use controller::ControllerIdentity;
pub use pair_setup::PairSetup;
pub use pair_verify::PairVerify;
