//! Generated audio shared by the audio tests. Every frame is unique so any lost, added or
//! shifted frame shows up, and nothing here makes sound.

use std::path::Path;
use std::process::{Command, Stdio};

pub fn synth(frames: usize, rate: u32, seed: f64) -> Vec<i16> {
    let mut out = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let t = n as f64 / rate as f64;
        let f = 220.0 + 180.0 * (t * 0.37 + seed).sin();
        let l = 0.35 * (2.0 * std::f64::consts::PI * f * t).sin() + 0.1 * (2.0 * std::f64::consts::PI * 1733.0 * t + seed).sin();
        let r = 0.30 * (2.0 * std::f64::consts::PI * (f * 1.5) * t + 0.7).sin() + 0.1 * (2.0 * std::f64::consts::PI * 997.0 * t).sin();
        out.push((l * 32767.0) as i16);
        out.push((r * 32767.0) as i16);
    }
    out
}

pub fn wav_bytes(pcm: &[i16], rate: u32) -> Vec<u8> {
    let mut f = Vec::with_capacity(44 + pcm.len() * 2);
    let data_len = (pcm.len() * 2) as u32;
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(36 + data_len).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * 4).to_le_bytes());
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        f.extend_from_slice(&s.to_le_bytes());
    }
    f
}

pub fn write_wav(path: &Path, pcm: &[i16], rate: u32) {
    std::fs::write(path, wav_bytes(pcm, rate)).unwrap();
}

pub fn to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|s| *s as f32 / 32768.0).collect()
}

pub fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

pub fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg").args(["-v", "error", "-nostdin", "-y"]).args(args).status().expect("ffmpeg");
    assert!(status.success(), "ffmpeg failed: {args:?}");
}

/// Encodes `wav` to `out` with the given ffmpeg codec arguments.
pub fn encode(wav: &Path, out: &Path, codec_args: &[&str]) {
    let mut args = vec!["-i", wav.to_str().unwrap()];
    args.extend_from_slice(codec_args);
    args.push(out.to_str().unwrap());
    ffmpeg(&args);
}

/// How far `a` is shifted against `b`, and the worst sample error at that shift over `window` frames.
pub fn best_lag(a: &[f32], b: &[f32], max_shift: i64, window: usize) -> (i64, f32) {
    let mut best = (i64::MAX, f32::MAX, f64::MAX);
    for shift in -max_shift..=max_shift {
        let mut sum = 0.0f64;
        let mut worst = 0.0f32;
        let mut count = 0;
        for i in 0..window {
            let ai = (i as i64 + shift) * 2;
            if ai < 0 || ai as usize + 1 >= a.len() || i * 2 + 1 >= b.len() {
                continue;
            }
            let (x, y) = (a[ai as usize] - b[i * 2], a[ai as usize + 1] - b[i * 2 + 1]);
            sum += (x * x + y * y) as f64;
            worst = worst.max(x.abs()).max(y.abs());
            count += 1;
        }
        if count > window / 2 && sum < best.2 {
            best = (shift, worst, sum);
        }
    }
    (best.0, best.1)
}
