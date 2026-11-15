//! Seeking must land on the requested frame. Each codec's decode after a seek is compared with
//! the reference decode at that position.

mod common;

use common::audio::{best_lag, encode, have_ffmpeg, synth, write_wav};
use navia_core::audio::Decoder;
use std::path::Path;

/// (name, ffmpeg args, extension, must be bit-exact)
const CODECS: &[(&str, &[&str], &str, bool)] = &[
    ("flac", &["-c:a", "flac"], "flac", true),
    ("alac", &["-c:a", "alac"], "m4a", true),
    ("wav", &["-c:a", "pcm_s16le"], "wav", true),
    ("mp3", &["-c:a", "libmp3lame", "-b:a", "192k"], "mp3", false),
    ("aac", &["-c:a", "aac", "-b:a", "192k"], "m4a", false),
    ("opus", &["-c:a", "libopus", "-b:a", "128k"], "opus", false),
    ("vorbis", &["-c:a", "libvorbis", "-b:a", "160k"], "ogg", false),
];

fn decode_all(path: &Path) -> (Vec<f32>, u32) {
    let mut d = Decoder::open(path).unwrap();
    let mut all = Vec::new();
    let mut b = Vec::new();
    while d.next_block(&mut b).unwrap() {
        all.extend_from_slice(&b);
    }
    (all, d.sample_rate)
}

fn decode_from(path: &Path, frame: u64, frames: usize) -> Vec<f32> {
    let mut d = Decoder::open(path).unwrap();
    d.seek(frame).unwrap();
    let mut out = Vec::new();
    let mut b = Vec::new();
    while out.len() / 2 < frames && d.next_block(&mut b).unwrap() {
        out.extend_from_slice(&b);
    }
    out.truncate(frames * 2);
    out
}

#[test]
fn seek_lands_on_the_requested_frame_for_every_codec() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let rate = 44_100u32;
    let frames = rate as usize * 30;
    let wav = dir.path().join("src.wav");
    write_wav(&wav, &synth(frames, rate, 0.3), rate);

    let targets_ms = [0u64, 137, 4_000, 12_345, 25_000];
    let mut report = String::new();
    for (name, args, ext, exact) in CODECS {
        let path = dir.path().join(format!("{name}.{ext}"));
        encode(&wav, &path, args);
        let (reference, file_rate) = decode_all(&path);
        for ms in targets_ms {
            let target = ms * file_rate as u64 / 1000;
            let probe = 4096;
            let got = decode_from(&path, target, probe);
            assert_eq!(got.len(), probe * 2, "{name} at {ms} ms returned too little");
            let start = (target as usize * 2).min(reference.len());
            let expected = &reference[start..(start + probe * 2).min(reference.len())];
            if *exact {
                assert!(got.iter().zip(expected).all(|(a, b)| a.to_bits() == b.to_bits()), "{name} at {ms} ms is not bit-exact");
                report.push_str(&format!("{name:7} {ms:6} ms: bit-exact\n"));
            } else {
                // Skip the first 2048 frames, where lossy decoders are still settling their overlap.
                let skip = 2048 * 2;
                let (lag, worst) = best_lag(&got[skip..], &expected[skip..], 2500, 1000);
                report.push_str(&format!("{name:7} {ms:6} ms: lag {lag:+} frames, worst sample diff {worst:.4}\n"));
                assert_eq!(lag, 0, "{name} at {ms} ms landed {lag} frames off");
            }
        }
    }
    println!("{report}");
}

#[test]
fn seek_past_the_end_fails_cleanly_or_ends_the_track() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.wav");
    write_wav(&path, &synth(44_100, 44_100, 0.1), 44_100);
    let mut d = Decoder::open(&path).unwrap();
    let mut b = Vec::new();
    match d.seek(44_100 * 60) {
        Err(_) => {}
        Ok(()) => assert!(!d.next_block(&mut b).unwrap(), "no audio after seeking past the end"),
    }
}
