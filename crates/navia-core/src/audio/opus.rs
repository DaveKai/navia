//! Safe wrapper over libopus, since Symphonia has no Opus decoder.
//! The `opus-bundled` feature builds libopus from source (needs cmake) instead of linking the system one.

use std::ffi::CStr;
use std::os::raw::c_int;

use opusic_sys as ffi;

use crate::error::{Error, Result};

/// Opus always decodes at 48 kHz.
pub const OPUS_RATE: u32 = 48_000;
/// 120 ms at 48 kHz, the longest frame a packet can hold.
pub const MAX_FRAMES: usize = 5760;

pub struct OpusDecoder {
    state: *mut ffi::OpusDecoder,
    pub channels: usize,
}

// The decoder state is used from one thread at a time, through `&mut self`.
unsafe impl Send for OpusDecoder {}

fn strerror(code: c_int) -> String {
    // SAFETY: opus_strerror returns a static string for any code.
    unsafe { CStr::from_ptr(ffi::opus_strerror(code)) }.to_string_lossy().into_owned()
}

impl OpusDecoder {
    pub fn new(channels: usize) -> Result<Self> {
        if channels != 1 && channels != 2 {
            return Err(Error::Audio(format!("Opus with {channels} channels isn't supported")));
        }
        let mut error: c_int = 0;
        // SAFETY: arguments are valid and error is a valid out pointer.
        let state = unsafe { ffi::opus_decoder_create(OPUS_RATE as ffi::opus_int32, channels as c_int, &mut error) };
        if error != ffi::OPUS_OK || state.is_null() {
            return Err(Error::Audio(format!("opus decoder: {}", strerror(error))));
        }
        Ok(Self { state, channels })
    }

    /// Clear the decoder state, as needed after a seek.
    pub fn reset(&mut self) {
        // SAFETY: state is valid, and OPUS_RESET_STATE takes no argument.
        unsafe { ffi::opus_decoder_ctl(self.state, ffi::OPUS_RESET_STATE as c_int) };
    }

    /// Decode one packet as interleaved f32. Returns frames per channel.
    pub fn decode_float(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize> {
        let capacity = (out.len() / self.channels).min(MAX_FRAMES);
        // SAFETY: state is valid, packet and out are valid for the given lengths.
        let n = unsafe {
            ffi::opus_decode_float(self.state, packet.as_ptr(), packet.len() as ffi::opus_int32, out.as_mut_ptr(), capacity as c_int, 0)
        };
        if n < 0 {
            return Err(Error::Audio(format!("opus decode: {}", strerror(n))));
        }
        Ok(n as usize)
    }
}

impl Drop for OpusDecoder {
    fn drop(&mut self) {
        // SAFETY: state came from opus_decoder_create and is destroyed once.
        unsafe { ffi::opus_decoder_destroy(self.state) };
    }
}
