//! Ground truth from outside Symphonia, using `metaflac` and `ffmpeg`.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::Result;

pub struct FlacInfo {
    pub total_samples: u64,
    pub bits_per_sample: u32,
    pub sample_rate: u32,
    /// All zeros when the encoder didn't store one.
    pub md5: [u8; 16],
}

pub fn flac_info(path: &Path) -> Result<FlacInfo> {
    let out = Command::new("metaflac")
        .args(["--show-total-samples", "--show-bps", "--show-sample-rate", "--show-md5sum"])
        .arg(path)
        .output()
        .map_err(|e| format!("metaflac: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    if !out.status.success() || lines.len() != 4 {
        return Err(format!("metaflac failed on {path:?}"));
    }
    let mut md5 = [0u8; 16];
    for (i, b) in md5.iter_mut().enumerate() {
        *b = u8::from_str_radix(&lines[3][i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(FlacInfo {
        total_samples: lines[0].parse().map_err(|_| "total samples")?,
        bits_per_sample: lines[1].parse().map_err(|_| "bps")?,
        sample_rate: lines[2].parse().map_err(|_| "rate")?,
        md5,
    })
}

/// Decodes with ffmpeg to interleaved stereo f32, without resampling.
pub fn ffmpeg_decode(path: &Path) -> Result<Vec<f32>> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-ac", "2", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut bytes = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    child.wait().map_err(|e| e.to_string())?;
    Ok(bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect())
}

/// FLAC's MD5 is over little-endian interleaved integers at the file's bit depth.
pub fn flac_md5_of(samples: &[f32], bits: u32) -> [u8; 16] {
    let scale = (1i64 << (bits - 1)) as f32;
    let bytes_per_sample = bits.div_ceil(8) as usize;
    let mut ctx = md5::Context::new();
    let mut buf = Vec::with_capacity(1 << 16);
    for &s in samples {
        let v = (s * scale).round() as i64;
        buf.extend_from_slice(&v.to_le_bytes()[..bytes_per_sample]);
        if buf.len() >= (1 << 16) - 8 {
            ctx.consume(&buf);
            buf.clear();
        }
    }
    ctx.consume(&buf);
    ctx.finalize().0
}
