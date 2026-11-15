//! Streaming resampler with an exact output length of ceil(input frames * ratio).
//! It trims the filter's startup delay and pads the tail.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::error::{Error, Result};

const CHUNK: usize = 1024;
const CHANNELS: usize = 2;

pub struct StreamResampler {
    inner: Fft<f32>,
    pub source_rate: u32,
    pub target_rate: u32,
    pending: Vec<f32>,
    output: Vec<f32>,
    delay_left: usize,
    total_in: u64,
    emitted: u64,
}

impl StreamResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Result<Self> {
        let inner = Fft::new(
            source_rate as usize,
            target_rate as usize,
            CHUNK,
            CHANNELS,
            FixedSync::Input,
        )
        .map_err(|e| Error::Audio(format!("resampler: {e}")))?;
        let delay_left = inner.output_delay();
        let output = vec![0.0; inner.output_frames_max() * CHANNELS];
        Ok(Self {
            inner,
            source_rate,
            target_rate,
            pending: Vec::new(),
            output,
            delay_left,
            total_in: 0,
            emitted: 0,
        })
    }

    /// Input frames still in flight, expressed as output frames not yet emitted.
    pub fn pending_output(&self) -> u64 {
        self.expected_output().saturating_sub(self.emitted)
    }

    fn expected_output(&self) -> u64 {
        (self.total_in * self.target_rate as u64).div_ceil(self.source_rate as u64)
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.total_in += (input.len() / CHANNELS) as u64;
        self.pending.extend_from_slice(input);
        while self.pending.len() >= CHUNK * CHANNELS {
            self.process_chunk(out);
        }
    }

    /// Flush the filter tail. After this, output frames == ceil(input frames * ratio).
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        while self.emitted < self.expected_output() {
            self.pending.resize(CHUNK * CHANNELS, 0.0);
            self.process_chunk(out);
        }
        let excess = (self.emitted - self.expected_output()) as usize * CHANNELS;
        out.truncate(out.len() - excess);
        self.emitted = self.expected_output();
    }

    fn process_chunk(&mut self, out: &mut Vec<f32>) {
        let input = InterleavedSlice::new(&self.pending, CHANNELS, CHUNK).expect("input size");
        let frames_out = self.output.len() / CHANNELS;
        let mut output =
            InterleavedSlice::new_mut(&mut self.output, CHANNELS, frames_out).expect("output size");
        let (consumed, produced) =
            self.inner.process_into_buffer(&input, &mut output, None).expect("resample");
        self.pending.drain(..consumed * CHANNELS);

        let skip = self.delay_left.min(produced);
        self.delay_left -= skip;
        let usable = &self.output[skip * CHANNELS..produced * CHANNELS];
        out.extend_from_slice(usable);
        self.emitted += usable.len() as u64 / CHANNELS as u64;
    }
}
