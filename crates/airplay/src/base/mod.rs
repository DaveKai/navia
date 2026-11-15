//! Foundation types: devices, features, codecs, stream config, errors (upstream `airplay-core`).
pub mod codec;
pub mod device;
pub mod error;
pub mod features;
pub mod stream;

pub use codec::{AudioCodec, AudioFormat, SampleRate};
pub use device::{Device, DeviceId, DeviceInfo, Version};
pub use error::{Error, PairingError, Result, RtspError, StreamingError};
pub use features::{AuthMethod, Features};
pub use stream::{PtpMode, StreamConfig, StreamType, TimingProtocol};
