//! Headless capture: drains the engine exactly like a device callback, but into memory.

use crate::audio::engine::{EngineEvent, EngineOutput};
use std::thread;
use std::time::{Duration, Instant};

/// Block sizes cycled through, so track joins land at arbitrary points in a block.
const BLOCK_SIZES: [usize; 7] = [64, 480, 1024, 4096, 17, 2048, 333];

pub struct Capture {
    pub samples: Vec<f32>,
    pub events: Vec<EngineEvent>,
    /// Times the ring buffer was empty before the engine finished.
    pub stalls: u64,
    pub elapsed: Duration,
}

pub fn run(mut output: EngineOutput) -> Capture {
    let start = Instant::now();
    let mut samples = Vec::with_capacity(1 << 26);
    let mut block = vec![0.0f32; 8192];
    let (mut i, mut stalls) = (0usize, 0u64);
    loop {
        let want = BLOCK_SIZES[i % BLOCK_SIZES.len()] * 2;
        i += 1;
        let n = output.fill(&mut block[..want]);
        samples.extend_from_slice(&block[..n]);
        if n < want {
            if output.is_finished() {
                let n = output.fill(&mut block);
                if n == 0 {
                    break;
                }
                samples.extend_from_slice(&block[..n]);
            } else {
                stalls += 1;
                thread::sleep(Duration::from_micros(200));
            }
        }
    }
    let elapsed = start.elapsed();
    let events = output.events.try_iter().collect();
    output.join();
    Capture { samples, events, stalls, elapsed }
}
