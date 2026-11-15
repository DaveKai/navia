//! ALAC encoding, RTP packets and the audio cipher (upstream `airplay-audio`, without its decoder,
//! streamer, equalizer and spatial code: Navia's engine and sender replace them).
pub mod cipher;
pub mod encoder;
pub mod rtp;

pub use encoder::{AlacEncoder, AudioEncoder, EncodedPacket, create_encoder};
pub use rtp::{RetransmitRequest, RtpHeader, RtpPacket, RtpReceiver, RtpSender, build_retransmit_response};
