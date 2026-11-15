//! The equalizer, measured against the cookbook formulas and ffmpeg's own biquads, plus bypass,
//! smoothing, limiter and stereo stages through the real engine. Nothing here produces sound.

use std::f64::consts::{PI, TAU};
use std::path::Path;
use std::process::{Command, Stdio};

use navia_core::audio::capture;
use navia_core::audio::engine::{EngineConfig, OutputMode, spawn};
use navia_core::audio::processing::{EqBand, EqSettings, Equalizer, FilterShape};

const RATES: [u32; 4] = [44_100, 48_000, 96_000, 192_000];

fn sine(freq: f64, rate: u32, frames: usize, amp: f64) -> Vec<f32> {
    (0..frames).flat_map(|n| {
        let v = (amp * (TAU * freq * n as f64 / f64::from(rate)).sin()) as f32;
        [v, v]
    }).collect()
}

/// Amplitude of `freq` in a stereo channel after skipping the first `skip` frames.
fn tone_amp(samples: &[f32], rate: u32, freq: f64, skip: usize) -> f64 {
    let (mut re, mut im, mut n) = (0f64, 0f64, 0usize);
    for (i, frame) in samples.chunks_exact(2).enumerate().skip(skip) {
        let t = TAU * freq * i as f64 / f64::from(rate);
        re += f64::from(frame[0]) * t.cos();
        im += f64::from(frame[0]) * t.sin();
        n += 1;
    }
    2.0 * (re * re + im * im).sqrt() / n as f64
}

fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

fn band(shape: FilterShape, freq: f32, gain: f32, q: f32) -> EqBand {
    EqBand { shape, freq_hz: freq, gain_db: gain, q }
}

/// Runs a settled equalizer over `samples`.
fn run(eq: &mut Equalizer, mut samples: Vec<f32>) -> Vec<f32> {
    eq.process(&mut samples);
    samples
}

fn settled(settings: &EqSettings, rate: u32) -> Equalizer {
    let mut eq = Equalizer::new();
    eq.set_settings(Some(settings));
    eq.configure(rate);
    eq
}

fn only(bands: Vec<EqBand>) -> EqSettings {
    EqSettings { bands, ..Default::default() }
}

/// Magnitude of a cookbook biquad at `f`, written out from the Audio EQ Cookbook definitions
/// independently of the crate's code.
fn cookbook_db(kind: &str, f0: f64, gain: f64, q: f64, rate: f64, f: f64) -> f64 {
    let a = 10f64.powf(gain / 40.0);
    let w0 = TAU * f0 / rate;
    let (sn, cs) = w0.sin_cos();
    let alpha = sn / (2.0 * q);
    let (b0, b1, b2, a0, a1, a2) = match kind {
        "peak" => (1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
        "low" => {
            let s = 2.0 * a.sqrt() * alpha;
            (a * ((a + 1.0) - (a - 1.0) * cs + s), 2.0 * a * ((a - 1.0) - (a + 1.0) * cs), a * ((a + 1.0) - (a - 1.0) * cs - s), (a + 1.0) + (a - 1.0) * cs + s, -2.0 * ((a - 1.0) + (a + 1.0) * cs), (a + 1.0) + (a - 1.0) * cs - s)
        }
        _ => {
            let s = 2.0 * a.sqrt() * alpha;
            (a * ((a + 1.0) + (a - 1.0) * cs + s), -2.0 * a * ((a - 1.0) + (a + 1.0) * cs), a * ((a + 1.0) + (a - 1.0) * cs - s), (a + 1.0) - (a - 1.0) * cs + s, 2.0 * ((a - 1.0) - (a + 1.0) * cs), (a + 1.0) - (a - 1.0) * cs - s)
        }
    };
    let w = TAU * f / rate;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let mag = |x0: f64, x1: f64, x2: f64| (x0 + x1 * c1 + x2 * c2).hypot(x1 * s1 + x2 * s2);
    20.0 * (mag(b0, b1, b2) / mag(a0, a1, a2)).log10()
}

#[test]
fn filter_shapes_match_the_cookbook_by_measurement() {
    let mut worst = 0f64;
    for rate in RATES {
        let cases = [("peak", FilterShape::Peaking, 1000.0, 6.0, 1.0), ("peak", FilterShape::Peaking, 125.0, -9.0, 2.0), ("low", FilterShape::LowShelf, 200.0, 8.0, 0.7), ("high", FilterShape::HighShelf, 8000.0, -6.0, 0.7)];
        for (kind, shape, f0, gain, q) in cases {
            let settings = only(vec![band(shape, f0 as f32, gain as f32, q as f32)]);
            for probe in [40.0, 90.0, 400.0, 1000.0, 3000.0, 8000.0, 15_000.0] {
                if probe >= f64::from(rate) * 0.4 {
                    continue;
                }
                let frames = (f64::from(rate) * 1.0) as usize;
                let mut eq = settled(&settings, rate);
                let out = run(&mut eq, sine(probe, rate, frames, 0.1));
                let measured = db(tone_amp(&out, rate, probe, frames / 2) / 0.1);
                let expected = cookbook_db(kind, f0, gain, q, f64::from(rate), probe);
                worst = worst.max((measured - expected).abs());
                assert!((measured - expected).abs() < 0.05, "{kind} {f0} Hz at {rate}: {measured:.3} dB vs {expected:.3} dB at {probe} Hz");
            }
        }
    }
    eprintln!("worst deviation from the cookbook: {worst:.4} dB");
}

#[test]
fn matches_ffmpeg_biquads_sample_for_sample() {
    if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let rate = 48_000u32;
    // A long logarithmic sweep exercises the whole band.
    let frames = 48_000 * 4;
    let mut src = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let t = n as f64 / f64::from(rate);
        let f = 30.0 * (18_000.0f64 / 30.0).powf(t / 4.0);
        let phase = TAU * 30.0 * 4.0 / (18_000.0f64 / 30.0).ln() * ((18_000.0f64 / 30.0).powf(t / 4.0) - 1.0);
        let _ = f;
        let v = (0.4 * phase.sin()) as f32;
        src.extend_from_slice(&[v, (v * 0.8)]);
    }
    let cases = [
        ("equalizer=f=1000:t=q:w=1:g=6", band(FilterShape::Peaking, 1000.0, 6.0, 1.0)),
        ("lowshelf=f=200:t=q:w=0.7:g=8", band(FilterShape::LowShelf, 200.0, 8.0, 0.7)),
        ("highshelf=f=8000:t=q:w=0.7:g=-6", band(FilterShape::HighShelf, 8000.0, -6.0, 0.7)),
    ];
    for (filter, eq_band) in cases {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("in.raw");
        std::fs::write(&raw, src.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(&raw)
            .args(["-af", filter, "-f", "f32le", "-"])
            .output()
            .expect("ffmpeg");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let reference: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(reference.len(), src.len());
        let mut eq = settled(&only(vec![eq_band]), rate);
        let ours = run(&mut eq, src.clone());
        let (mut err, mut sig) = (0f64, 0f64);
        // Skip the first 50 ms, where start transients differ. The steady state must not.
        for i in 4800..ours.len() {
            let (a, b) = (f64::from(ours[i]), f64::from(reference[i]));
            err += (a - b) * (a - b);
            sig += b * b;
        }
        let snr = 10.0 * (sig / err.max(1e-40)).log10();
        eprintln!("ffmpeg comparison {filter}: {snr:.1} dB");
        assert!(snr > 80.0, "{filter} differs from ffmpeg: {snr:.1} dB");
    }
}

#[test]
fn off_and_identity_leave_samples_untouched_bit_for_bit() {
    let src = sine(997.0, 48_000, 20_000, 0.9);
    // Never configured, off, and a flat plan with the limiter on all bypass.
    let mut off = Equalizer::new();
    off.configure(48_000);
    assert!(off.is_bypassed());
    let mut flat = Equalizer::new();
    flat.set_settings(Some(&EqSettings { bands: vec![band(FilterShape::Peaking, 1000.0, 0.0, 1.0)], limiter: true, ..Default::default() }));
    flat.configure(48_000);
    assert!(flat.is_bypassed(), "a flat plan changes nothing");
    let mut none = Equalizer::new();
    none.configure(48_000);
    none.set_settings(None);
    for eq in [&mut off, &mut flat, &mut none] {
        let out = run(eq, src.clone());
        assert!(out.iter().zip(&src).all(|(a, b)| a.to_bits() == b.to_bits()), "changed while bypassed");
    }
}

#[test]
fn turning_off_settles_back_to_bit_exact() {
    let src = sine(440.0, 48_000, 48_000 * 2, 0.5);
    let mut eq = settled(&only(vec![band(FilterShape::Peaking, 1000.0, 8.0, 1.0)]), 48_000);
    let (first, rest) = src.split_at(48_000);
    let mut played = first.to_vec();
    eq.process(&mut played);
    assert!(!eq.is_bypassed());
    eq.set_settings(None);
    let mut rest = rest.to_vec();
    eq.process(&mut rest);
    assert!(eq.is_bypassed(), "should settle back to bypass");
    // The last second, after settling, is the source exactly.
    let exact_from = rest.len() - 24_000;
    assert!(rest[exact_from..].iter().zip(&src[48_000 + exact_from..]).all(|(a, b)| a.to_bits() == b.to_bits()), "not bit-exact after settling");
}

/// The largest change between consecutive samples, the crude measure of a click.
fn max_step(samples: &[f32]) -> f32 {
    samples.chunks_exact(2).collect::<Vec<_>>().windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0.0, f32::max)
}

#[test]
fn parameter_changes_do_not_click() {
    let rate = 48_000u32;
    let freq = 700.0;
    // The steepest sample-to-sample change a pure tone of this level has once settled.
    let changes: Vec<(&str, EqSettings)> = vec![
        ("big boost", only(vec![band(FilterShape::Peaking, 1000.0, 12.0, 1.0)])),
        ("shelf shape swap", only(vec![band(FilterShape::HighShelf, 1000.0, 6.0, 0.7)])),
        ("preamp", EqSettings { preamp_db: 12.0, bands: vec![band(FilterShape::Peaking, 1000.0, 12.0, 1.0)], ..Default::default() }),
        ("mono and balance", EqSettings { balance: -0.9, mono: true, bands: vec![band(FilterShape::Peaking, 1000.0, 0.5, 1.0)], ..Default::default() }),
    ];
    for (name, change) in changes {
        let mut eq = settled(&only(vec![band(FilterShape::Peaking, 1000.0, -12.0, 1.0)]), rate);
        // Warm up on the first second, at the settled low level.
        let mut warm = sine(freq, rate, rate as usize, 0.25);
        eq.process(&mut warm);
        // Change while audio runs, then check every sample of the next stretch,
        // split into device-sized callbacks of uneven length.
        eq.set_settings(Some(&change));
        let mut signal = sine(freq, rate, rate as usize, 0.25);
        let mut start = 0;
        for (i, size) in [512usize, 1, 333, 7, 4096, 90].iter().cycle().enumerate() {
            if start >= signal.len() {
                break;
            }
            let end = (start + size * 2).min(signal.len());
            eq.process(&mut signal[start..end]);
            start = end;
            let _ = i;
        }
        let stepped = max_step(&signal);
        // A settled tone's largest step, at its final amplitude, plus room for the transition.
        let final_amp = tone_amp(&signal[signal.len() / 2..], rate, freq, 0);
        let allowed = (2.0 * PI * freq / f64::from(rate)).sin() * final_amp * 1.25;
        assert!(stepped < allowed as f32, "{name}: step {stepped:.4} over {allowed:.4}");
        eprintln!("{name}: largest sample step {stepped:.5}");

        // Control check. A hard switch between the same two settings must trip this same measure,
        // or the test proves nothing.
        let mut before = settled(&only(vec![band(FilterShape::Peaking, 1000.0, -12.0, 1.0)]), rate);
        let mut after = settled(&change, rate);
        let a = run(&mut before, sine(freq, rate, rate as usize, 0.25));
        let b = run(&mut after, sine(freq, rate, rate as usize, 0.25));
        let mut hard = a[..a.len() / 2].to_vec();
        hard.extend_from_slice(&b[b.len() / 2..]);
        let abrupt = max_step(&hard);
        assert!(abrupt > allowed as f32, "{name}: the measure misses a hard switch: {abrupt:.4} vs {allowed:.4}");
        eprintln!("{name}: hard switch steps {abrupt:.5}");
    }
}

#[test]
fn limiter_never_exceeds_full_scale_and_is_transparent_when_quiet() {
    let boost = EqSettings { bands: vec![band(FilterShape::Peaking, 1000.0, 12.0, 1.0)], limiter: true, ..Default::default() };
    let mut eq = settled(&boost, 48_000);
    let out = run(&mut eq, sine(1000.0, 48_000, 96_000, 0.99));
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "peak {peak} over full scale");
    assert!(peak > 0.9, "the tone should be limited, not gone: {peak}");

    let mut quiet = settled(&EqSettings { preamp_db: -0.5, limiter: true, ..Default::default() }, 48_000);
    let src = sine(1000.0, 48_000, 48_000, 0.5);
    let out = run(&mut quiet, src.clone());
    let want = 10f32.powf(-0.5 / 20.0);
    let worst = out.iter().skip(4800).zip(src.iter().skip(4800)).map(|(o, s)| (o - s * want).abs()).fold(0f32, f32::max);
    assert!(worst < 1e-5, "limiter touched a quiet signal: {worst}");
}

#[test]
fn balance_mono_and_crossfeed_measure_as_designed() {
    let rate = 48_000u32;
    let frames = 96_000;
    let left_only: Vec<f32> = (0..frames).flat_map(|n| {
        let v = (0.5 * (TAU * 500.0 * n as f64 / f64::from(rate)).sin()) as f32;
        [v, 0.0]
    }).collect();
    // Balance fully right silences the left channel and leaves the right alone.
    let mut balance = settled(&EqSettings { balance: 1.0, ..Default::default() }, rate);
    let stereo: Vec<f32> = (0..frames).flat_map(|n| {
        let v = (0.5 * (TAU * 500.0 * n as f64 / f64::from(rate)).sin()) as f32;
        [v * 0.2, v]
    }).collect();
    let out = run(&mut balance, stereo.clone());
    let l: Vec<f32> = out.chunks_exact(2).map(|c| c[0]).collect();
    let r: Vec<f32> = out.chunks_exact(2).map(|c| c[1]).collect();
    assert!(l.iter().skip(4800).all(|v| v.abs() < 1e-6), "left should be silent");
    assert!(r.iter().zip(stereo.chunks_exact(2).map(|c| c[1])).skip(4800).all(|(a, b)| (a - b).abs() < 1e-6), "right should be unchanged");

    let mut mono = settled(&EqSettings { mono: true, ..Default::default() }, rate);
    let out = run(&mut mono, left_only.clone());
    let (ml, mr) = (tone_amp(&out, rate, 500.0, 24_000), tone_amp(&out.iter().copied().skip(1).step_by(2).flat_map(|v| [v, 0.0]).collect::<Vec<_>>(), rate, 500.0, 24_000));
    assert!((ml - 0.25).abs() < 0.01 && (mr - 0.25).abs() < 0.01, "mono halves a hard-panned tone into both sides: {ml} {mr}");

    let mut cross = settled(&EqSettings { crossfeed: Some(1.0), ..Default::default() }, rate);
    let out = run(&mut cross, left_only);
    let right: Vec<f32> = out.iter().copied().skip(1).step_by(2).flat_map(|v| [v, 0.0]).collect();
    let right_amp = tone_amp(&right, rate, 500.0, 24_000);
    assert!(right_amp > 0.08 && right_amp < 0.2, "right gets a low-passed copy of a hard-left 500 Hz tone: {right_amp}");

    // A centered tone stays close to level, through any amount.
    for freq in [60.0, 1000.0, 12_000.0] {
        let mut eq = settled(&EqSettings { crossfeed: Some(1.0), ..Default::default() }, rate);
        let out = run(&mut eq, sine(freq, rate, frames, 0.5));
        let level = db(tone_amp(&out, rate, freq, 24_000) / 0.5);
        assert!(level.abs() < 1.6, "centered tone at {freq} Hz moved by {level:.2} dB");
    }
}

#[test]
fn survives_a_rate_change_and_keeps_every_track_correct() {
    let dir = tempfile::tempdir().unwrap();
    let (rate_a, rate_b) = (44_100u32, 96_000u32);
    let a = sine(1000.0, rate_a, rate_a as usize * 2, 0.2);
    let b = sine(1000.0, rate_b, rate_b as usize * 2, 0.2);
    let (pa, pb) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
    write_wav(&pa, &a, rate_a);
    write_wav(&pb, &b, rate_b);
    let settings = only(vec![band(FilterShape::Peaking, 1000.0, 6.0, 1.0)]);
    let output = spawn(vec![pa, pb], EngineConfig::new(OutputMode::FollowTrack));
    output.shared().set_eq(Some(&settings));
    let c = capture::run(output);
    let cut = (rate_a as usize) * 2;
    assert_eq!(c.samples.len() / 2, cut + rate_b as usize * 2, "no frames gained or lost");
    assert!(c.samples.iter().all(|s| s.is_finite()), "nothing blew up across the change");
    let first = tone_amp(&c.samples[..cut * 2], rate_a, 1000.0, rate_a as usize / 2);
    let second = tone_amp(&c.samples[cut * 2..], rate_b, 1000.0, rate_b as usize / 2);
    let boost = 10f64.powf(6.0 / 20.0) * 0.2;
    assert!((first - boost).abs() / boost < 0.01, "first track {first:.4} vs {boost:.4}");
    assert!((second - boost).abs() / boost < 0.01, "second track {second:.4} vs {boost:.4}");
}

#[test]
fn stays_gapless_across_joins_and_correct_through_a_crossfade() {
    let dir = tempfile::tempdir().unwrap();
    let rate = 48_000u32;
    let a = sine(440.0, rate, rate as usize, 0.3);
    let b = sine(440.0, rate, rate as usize + 999, 0.3);
    let (pa, pb) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
    write_wav(&pa, &a, rate);
    write_wav(&pb, &b, rate);
    let settings = only(vec![band(FilterShape::Peaking, 440.0, 6.0, 1.0)]);

    // A gapless join through the filter gives one continuous tone with no blip where the tracks meet.
    let output = spawn(vec![pa.clone(), pb.clone()], EngineConfig::new(OutputMode::FollowTrack));
    output.shared().set_eq(Some(&settings));
    let c = capture::run(output);
    assert_eq!(c.samples.len() / 2, rate as usize * 2 + 999, "no frames gained or lost");
    let join = rate as usize;
    let around: Vec<f32> = c.samples[(join - 2000) * 2..(join + 2000) * 2].to_vec();
    let steady = (2.0 * PI * 440.0 / f64::from(rate)).sin() * 0.6 * 1.05;
    assert!(f64::from(max_step(&around)) < steady, "a click at the join: {} vs {steady}", max_step(&around));

    // With crossfade on, the mix goes through the same filters.
    let mut cfg = EngineConfig::new(OutputMode::FollowTrack);
    cfg.crossfade_frames = 12_000;
    let output = spawn(vec![pa, pb], cfg);
    output.shared().set_eq(Some(&settings));
    let c = capture::run(output);
    assert_eq!(c.samples.len() / 2, rate as usize * 2 + 999 - 12_000);
    let level = tone_amp(&c.samples, rate, 440.0, rate as usize / 2);
    let boost = 10f64.powf(6.0 / 20.0) * 0.3;
    assert!(level > boost * 0.8 && level < boost * 1.2, "the filter ran over the mix: {level:.4}");
}

fn write_wav(path: &Path, pcm: &[f32], rate: u32) {
    let data: Vec<u8> = pcm.iter().flat_map(|s| ((s * 32767.0) as i16).to_le_bytes()).collect();
    let mut f = Vec::with_capacity(44 + data.len());
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&rate.to_le_bytes());
    f.extend_from_slice(&(rate * 4).to_le_bytes());
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    std::fs::write(path, f).unwrap();
}
