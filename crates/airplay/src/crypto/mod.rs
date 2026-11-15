//! Cryptography for pairing and the encrypted channels (upstream `airplay-crypto`).
pub mod aes;
pub mod chacha;
pub mod curve25519;
pub mod digest;
pub mod ed25519;
pub mod hkdf;
pub mod keys;
pub mod srp;
pub mod tlv;

pub use chacha::{AudioCipher, ControlCipher};
pub use keys::{EncryptionKey, SessionKeys, SharedSecret};
pub use tlv::{Tlv8, TlvType};
