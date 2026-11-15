//! Sample-accurate checks of the audio engine against real files. Audio goes to memory, never
//! to a device.

mod library;
mod reference;

use std::path::{Path, PathBuf};

use navia_core::audio::capture::{self, Capture};
use navia_core::audio::Decoder;
use navia_core::audio::engine::{self, EngineConfig, EngineEvent, OutputMode, crossfade_gains};

pub type Result<T> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["compare-file", f] => compare_file(Path::new(f)),
        ["survey"] => survey(),
        ["survey-lossy"] => survey_lossy(),
        ["gapless", album, artist] => gapless_album(album, artist),
        ["rate-change", album, artist] => rate_change(album, artist),
        ["crossfade", album, artist, secs] => crossfade(album, artist, secs.parse().unwrap()),
        ["resample", album, artist, rate] => resample_album(album, artist, rate.parse().unwrap()),
        ["join", original, parts @ ..] => join_check(Path::new(original), parts),
        _ => Err("usage: survey | gapless ALBUM ARTIST | rate-change ALBUM ARTIST | crossfade ALBUM ARTIST SECS | resample ALBUM ARTIST RATE | join ORIGINAL PARTS...".to_string()),
    };
    if let Err(e) = result {
        eprintln!("FAILED: {e}");
        std::process::exit(1);
    }
}

fn config(output: OutputMode, crossfade_frames: usize) -> EngineConfig {
    { let mut c = EngineConfig::new(output); c.crossfade_frames = crossfade_frames; c }
}

fn play(queue: Vec<PathBuf>, cfg: EngineConfig) -> Capture {
    capture::run(engine::spawn(queue, cfg))
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().chars().take(44).collect()
}

/// Compares frames from the engine with independent tools, per format.
fn survey() -> Result<()> {
    println!("{:<20} {:<44} {:>10} {:>10} {:>10} {:>7} {:>6} {:>6}  gapless", "bucket", "file", "engine", "ffmpeg", "metaflac", "lag", "snr", "codec");
    for (bucket, path) in library::sample_tracks()? {
        let label = name(&path);
        match Decoder::open(&path) {
            Err(e) => println!("{bucket:<20} {label:<44} NOT DECODED: {e}"),
            Ok(dec) => {
                let (gapless, codec) = (dec.gapless, dec.codec.clone());
                let capture = play(vec![path.clone()], config(OutputMode::FollowTrack, 0));
                let frames = capture.samples.len() as u64 / 2;
                let reference = reference::ffmpeg_decode(&path)?;
                let ref_frames = reference.len() as u64 / 2;
                let flac = reference::flac_info(&path).ok().map(|i| i.total_samples.to_string());
                let (lag, snr) = alignment(&capture.samples, &reference);
                println!(
                    "{bucket:<20} {label:<44} {frames:>10} {ref_frames:>10} {:>10} {lag:>7} {snr:>6.1} {codec:>6}  {gapless:?}{}",
                    flac.unwrap_or_else(|| "-".to_string()),
                    if frames != ref_frames { format!("  DIFF {}", frames as i64 - ref_frames as i64) } else { String::new() }
                );
            }
        }
    }
    Ok(())
}

/// Best lag of `a` against `b` in frames, and the SNR in dB of the aligned difference.
fn alignment(a: &[f32], b: &[f32]) -> (i64, f64) {
    let frames = (a.len().min(b.len())) / 2;
    if frames < 96_000 {
        return (0, 0.0);
    }
    let (window, search) = (48_000, 6_000i64);
    let mid = frames / 2;
    let start = mid - window / 2;
    let mono = |s: &[f32], f: usize| (s[f * 2] + s[f * 2 + 1]) as f64;
    let snr_at = |lag: i64| {
        let usable = frames - lag.unsigned_abs() as usize;
        let (mut sig, mut err) = (0f64, 0f64);
        for i in 0..usable {
            let (ai, bi) = if lag >= 0 { (i + lag as usize, i) } else { (i, i + (-lag) as usize) };
            for ch in 0..2 {
                let (x, y) = (a[ai * 2 + ch] as f64, b[bi * 2 + ch] as f64);
                sig += y * y;
                err += (x - y) * (x - y);
            }
        }
        10.0 * (sig / err.max(1e-30)).log10()
    };
    // A periodic tone matches well at many lags, so a clean match at zero wins.
    let zero = snr_at(0);
    if zero > 60.0 {
        return (0, zero);
    }
    let (mut best_lag, mut best) = (0i64, f64::NEG_INFINITY);
    for lag in -search..=search {
        let corr: f64 = (0..window)
            .step_by(3)
            .map(|i| mono(a, (start as i64 + i as i64 + lag) as usize) * mono(b, start + i))
            .sum();
        if corr > best {
            (best, best_lag) = (corr, lag);
        }
    }
    (best_lag, snr_at(best_lag))
}

/// Every lossy file in the library against ffmpeg, summarized.
fn survey_lossy() -> Result<()> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<String, (u64, u64, u64, i64, i64, f64, f64, Vec<String>)> = BTreeMap::new();
    for (_bucket, path) in library::lossy_tracks()? {
        let Ok(dec) = Decoder::open(&path) else { continue };
        let label = format!("{}/{:?}", dec.codec, dec.gapless);
        let capture = play(vec![path.clone()], config(OutputMode::FollowTrack, 0));
        let reference = reference::ffmpeg_decode(&path)?;
        let (got, want) = (capture.samples.len() as i64 / 2, reference.len() as i64 / 2);
        let (lag, snr) = alignment(&capture.samples, &reference);
        let errors: u64 = capture.events.iter().map(|e| if let EngineEvent::TrackEnd { decode_errors, .. } = e { *decode_errors } else { 0 }).sum();
        let e = buckets.entry(label).or_insert((0, 0, 0, 0, 0, f64::MAX, f64::MIN, Vec::new()));
        e.0 += 1;
        e.1 += u64::from(got == want);
        e.2 += u64::from(lag == 0);
        e.3 = e.3.min(got - want);
        e.4 = e.4.max(got - want);
        e.5 = e.5.min(snr);
        e.6 = e.6.max(snr);
        if errors > 0 { eprintln!("errors {errors} in {}", path.display()); }
        if (got != want || lag != 0) && e.7.len() < 3 {
            e.7.push(format!("{} diff {} lag {lag} snr {snr:.0} errors {errors}", name(&path), got - want));
        }
    }
    for (k, v) in buckets {
        println!("{k:<26} files {:>3}  same-length {:>3}  lag-0 {:>3}  diff {:>+5}..{:>+5}  snr {:.0}..{:.0} dB", v.0, v.1, v.2, v.3, v.4, v.5, v.6);
        for x in v.7 {
            println!("    e.g. {x}");
        }
    }
    Ok(())
}

/// Where Symphonia and ffmpeg disagree for one file.
fn compare_file(path: &Path) -> Result<()> {
    let capture = play(vec![path.to_path_buf()], config(OutputMode::FollowTrack, 0));
    let reference = reference::ffmpeg_decode(path)?;
    let (a, b) = (&capture.samples, &reference);
    println!("engine {} frames, ffmpeg {} frames, stalls {}", a.len() / 2, b.len() / 2, capture.stalls);
    let n = a.len().min(b.len());
    let window = |slice: &[f32]| (slice.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / slice.len().max(1) as f64).sqrt();
    let first_big = (0..n).find(|&i| (a[i] - b[i]).abs() > 1e-3);
    println!("first sample differing >1e-3 from the start: {first_big:?}");
    let aligned_end = (0..n).rev().find(|&i| (a[i + a.len() - n] - b[i + b.len() - n]).abs() > 1e-3);
    println!("last sample differing >1e-3 from the end-aligned: {aligned_end:?} of {n}");
    println!("rms last 2048 frames: engine {:.5} ffmpeg {:.5}", window(&a[a.len().saturating_sub(4096)..]), window(&b[b.len().saturating_sub(4096)..]));
    Ok(())
}

fn track_starts(capture: &Capture) -> Vec<(usize, u64, EngineEvent)> {
    capture.events.iter().filter_map(|e| match e {
        EngineEvent::TrackStart { track, output_frame, .. } => Some((*track, *output_frame, e.clone())),
        _ => None,
    }).collect()
}

fn report_errors(capture: &Capture) -> usize {
    capture.events.iter().filter(|e| match e {
        EngineEvent::Error { track, message } => { println!("  ENGINE ERROR track {track}: {message}"); true }
        _ => false,
    }).count()
}

/// A gapless album through the real engine against FLAC ground truth.
fn gapless_album(album: &str, artist: &str) -> Result<()> {
    let tracks = library::album_tracks(album, artist)?;
    println!("album: {album} by {artist}, {} tracks", tracks.len());
    let infos: Vec<_> = tracks.iter().map(|t| reference::flac_info(t)).collect::<std::result::Result<_, _>>()?;
    let rate = infos[0].sample_rate;
    let bits = infos[0].bits_per_sample;
    let expected: u64 = infos.iter().map(|i| i.total_samples).sum();
    println!("format {rate} Hz / {bits}-bit; expected frames from STREAMINFO: {expected}");

    let capture = play(tracks.clone(), config(OutputMode::FollowTrack, 0));
    let captured = capture.samples.len() as u64 / 2;
    let seconds = capture.samples.len() as f64 / 2.0 / rate as f64;
    println!("captured frames: {captured} ({seconds:.2} s) in {:.2?} ({:.0}x real time), stalls {}", capture.elapsed, seconds / capture.elapsed.as_secs_f64(), capture.stalls);
    println!("frame count difference: {}", captured as i64 - expected as i64);
    report_errors(&capture);

    let starts = track_starts(&capture);
    let mut offset = 0u64;
    let (mut md5_ok, mut md5_absent, mut md5_bad, mut ref_diffs) = (0, 0, 0, 0u64);
    let mut boundary_rows = Vec::new();
    for (i, track) in tracks.iter().enumerate() {
        let (frames, info) = (infos[i].total_samples, &infos[i]);
        let start_ev = starts.iter().find(|s| s.0 == i).map(|s| s.1);
        if start_ev != Some(offset) {
            println!("  BOUNDARY MARKER MISMATCH track {i}: engine {:?} expected {offset}", start_ev);
        }
        let slice = &capture.samples[offset as usize * 2..(offset + frames) as usize * 2];
        // FLAC stores an MD5 of the original PCM, which no decoder can influence.
        if info.md5 == [0; 16] {
            md5_absent += 1;
        } else if reference::flac_md5_of(slice, info.bits_per_sample) == info.md5 {
            md5_ok += 1;
        } else {
            md5_bad += 1;
            println!("  MD5 MISMATCH {}", name(track));
        }
        let reference = reference::ffmpeg_decode(track)?;
        if reference.len() != slice.len() {
            println!("  LENGTH DIFF vs ffmpeg {} : {} vs {}", name(track), slice.len() / 2, reference.len() / 2);
        }
        let diff = slice.iter().zip(&reference).filter(|(a, b)| a.to_bits() != b.to_bits()).count() as u64;
        ref_diffs += diff;
        if i > 0 {
            boundary_rows.push(boundary_row(&capture.samples, offset as usize, rate));
        }
        offset += frames;
    }
    println!("per-track MD5 (STREAMINFO): {md5_ok} match, {md5_bad} mismatch, {md5_absent} absent");
    println!("differing samples vs independent ffmpeg decode: {ref_diffs}");
    println!("junctions (window 2048 frames each side):");
    println!("{:>4} {:>12} {:>10} {:>10} {:>12} {:>12}  note", "#", "frame", "rms_before", "rms_after", "jump@join", "max_jump_near");
    for (i, row) in boundary_rows.iter().enumerate() {
        let continuous = if row.0 > 0.01 && row.1 > 0.01 { "audio crosses join" } else { "fade or silence at join" };
        println!("{:>4} {:>12} {:>10.4} {:>10.4} {:>12.5} {:>12.5}  {continuous}", i + 1, row.2, row.0, row.1, row.3, row.4);
    }
    if captured != expected || md5_bad > 0 || ref_diffs > 0 {
        return Err("sample accuracy check failed".to_string());
    }
    println!("RESULT: exact sample count and bit-identical audio through {} joins", tracks.len() - 1);
    Ok(())
}

/// Returns RMS before and after, the frame, the jump at the join, and the largest jump within 512 frames.
fn boundary_row(samples: &[f32], frame: usize, rate: u32) -> (f64, f64, usize, f64, f64) {
    let w = (rate as usize / 25).min(2048);
    let rms = |from: usize, to: usize| {
        let s = &samples[from * 2..to * 2];
        (s.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
    };
    let jump = |i: usize| (samples[i * 2] - samples[(i - 1) * 2]).abs().max((samples[i * 2 + 1] - samples[(i - 1) * 2 + 1]).abs()) as f64;
    let near = (frame - 512..frame + 512).filter(|i| *i != frame).map(jump).fold(0.0, f64::max);
    (rms(frame - w, frame), rms(frame, frame + w), frame, jump(frame), near)
}

/// A queue with sample-rate changes, in both output modes.
fn rate_change(album: &str, artist: &str) -> Result<()> {
    let tracks = library::album_tracks(album, artist)?;
    let infos: Vec<_> = tracks.iter().map(|t| reference::flac_info(t)).collect::<std::result::Result<_, _>>()?;
    let rates: Vec<u32> = infos.iter().map(|i| i.sample_rate).collect();
    println!("album: {album}; rates: {rates:?}");

    let capture = play(tracks.clone(), config(OutputMode::FollowTrack, 0));
    let expected: u64 = infos.iter().map(|i| i.total_samples).sum();
    println!("FollowTrack: captured {} frames, expected {expected} (diff {})", capture.samples.len() / 2, capture.samples.len() as i64 / 2 - expected as i64);
    report_errors(&capture);
    let mut offset = 0;
    for (i, info) in infos.iter().enumerate() {
        let slice = &capture.samples[offset * 2..(offset + info.total_samples as usize) * 2];
        let md5 = if info.md5 == [0; 16] { "absent" } else if reference::flac_md5_of(slice, info.bits_per_sample) == info.md5 { "ok" } else { "MISMATCH" };
        let start = track_starts(&capture).into_iter().find(|s| s.0 == i).unwrap();
        if let EngineEvent::TrackStart { output_rate, reopen, .. } = start.2 {
            println!("  track {i:>2} {} Hz at output frame {:>10} reopen={reopen:<5} md5 {md5}", output_rate, start.1);
        }
        offset += info.total_samples as usize;
    }
    let target = 48_000;
    let capture = play(tracks.clone(), config(OutputMode::FixedRate(target), 0));
    let mut expected = 0u64;
    // Runs of equal rate are resampled as one stream, so round per run.
    let mut i = 0;
    while i < rates.len() {
        let mut j = i;
        let mut sum = 0u64;
        while j < rates.len() && rates[j] == rates[i] {
            sum += infos[j].total_samples;
            j += 1;
        }
        expected += if rates[i] == target { sum } else { (sum * target as u64).div_ceil(rates[i] as u64) };
        i = j;
    }
    println!("FixedRate({target}): captured {} frames, expected {expected} (diff {})", capture.samples.len() / 2, capture.samples.len() as i64 / 2 - expected as i64);
    report_errors(&capture);
    println!("  every track delivered at one device rate, so no reopen happens");
    Ok(())
}

/// Resamples an album to a fixed rate and checks length and joins.
fn resample_album(album: &str, artist: &str, rate: u32) -> Result<()> {
    let tracks = library::album_tracks(album, artist)?;
    let infos: Vec<_> = tracks.iter().map(|t| reference::flac_info(t)).collect::<std::result::Result<_, _>>()?;
    let src: u32 = infos[0].sample_rate;
    let total: u64 = infos.iter().map(|i| i.total_samples).sum();
    let expected = (total * rate as u64).div_ceil(src as u64);
    let capture = play(tracks.clone(), config(OutputMode::FixedRate(rate), 0));
    println!("{src} -> {rate}: captured {} frames, expected {expected} (diff {}), stalls {}", capture.samples.len() / 2, capture.samples.len() as i64 / 2 - expected as i64, capture.stalls);
    report_errors(&capture);
    println!("{:>12} {:>10} {:>10} {:>10} {:>12}", "join frame", "rms_before", "rms_after", "jump@join", "max_jump_near");
    for s in track_starts(&capture).iter().skip(1) {
        let r = boundary_row(&capture.samples, s.1 as usize, rate);
        println!("{:>12} {:>10.4} {:>10.4} {:>10.5} {:>12.5}", r.2, r.0, r.1, r.3, r.4);
    }
    Ok(())
}

/// Checks crossfade length and that each overlap matches an independently mixed reference.
fn crossfade(album: &str, artist: &str, secs: f32) -> Result<()> {
    let tracks = library::album_tracks(album, artist)?;
    let tracks: Vec<PathBuf> = tracks.into_iter().take(4).collect();
    let infos: Vec<_> = tracks.iter().map(|t| reference::flac_info(t)).collect::<std::result::Result<_, _>>()?;
    let rate = infos[0].sample_rate;
    let xf = (secs * rate as f32) as usize;
    let capture = play(tracks.clone(), config(OutputMode::FollowTrack, xf));
    let refs: Vec<Vec<f32>> = tracks.iter().map(|t| reference::ffmpeg_decode(t)).collect::<std::result::Result<_, _>>()?;
    report_errors(&capture);
    let starts = track_starts(&capture);
    let total_src: u64 = infos.iter().map(|i| i.total_samples).sum();
    let overlaps: u64 = starts.iter().map(|s| if let EngineEvent::TrackStart { overlap, .. } = s.2 { overlap as u64 } else { 0 }).sum();
    let captured = capture.samples.len() as u64 / 2;
    println!("crossfade {secs}s = {xf} frames, {} tracks at {rate} Hz", tracks.len());
    println!("captured {captured}, expected {} (diff {})", total_src - overlaps, captured as i64 - (total_src - overlaps) as i64);
    for w in starts.windows(2) {
        let EngineEvent::TrackStart { overlap, .. } = w[1].2 else { unreachable!() };
        let (a, b) = (&refs[w[0].0], &refs[w[1].0]);
        let at = w[1].1 as usize;
        let mut max_err = 0f32;
        for k in 0..overlap {
            let t = (k as f32 + 0.5) / overlap as f32;
            let (gout, gin) = crossfade_gains(t);
            for ch in 0..2 {
                let aidx = (a.len() / 2 - overlap + k) * 2 + ch;
                let want = a[aidx] * gout + b[k * 2 + ch] * gin;
                max_err = max_err.max((capture.samples[(at + k) * 2 + ch] - want).abs());
            }
        }
        println!("  join at {at}: overlap {overlap} frames, max error vs independent mix {max_err:.2e}");
    }
    Ok(())
}

/// Plays encoded parts as a gapless queue and compares them with the original audio.
fn join_check(original: &Path, parts: &[&str]) -> Result<()> {
    let original_pcm = reference::ffmpeg_decode(original)?;
    let queue: Vec<PathBuf> = parts.iter().map(PathBuf::from).collect();
    for p in &queue {
        let d = Decoder::open(p).map_err(|e| e.to_string())?;
        println!("  {}: codec {}, gapless {:?}, delay {:?}, padding {:?}, smpb {:?}", name(p), d.codec, d.gapless, d.delay, d.padding, d.smpb);
    }
    let capture = play(queue.clone(), config(OutputMode::FollowTrack, 0));
    report_errors(&capture);
    let got = capture.samples.len() / 2;
    let want = original_pcm.len() / 2;
    println!("captured {got} frames, original {want} (diff {})", got as i64 - want as i64);
    let starts = track_starts(&capture);
    let join = starts.get(1).map(|s| s.1 as usize).ok_or("need 2 parts")?;
    let (lag, snr) = alignment(&capture.samples, &original_pcm);
    println!("whole-file lag {lag} frames, SNR {snr:.1} dB");
    let r = 1024;
    let residual = |c: usize| {
        let mut err = 0f64;
        let lag0 = lag.max(0) as usize;
        for i in c.saturating_sub(r)..c + r {
            err += (capture.samples[(i + lag0) * 2] as f64 - original_pcm[i * 2] as f64).powi(2);
        }
        (err / (2 * r) as f64).sqrt()
    };
    println!("residual RMS vs original at join: {:.6}; at mid-part1: {:.6}", residual(join), residual(join / 2));
    Ok(())
}
