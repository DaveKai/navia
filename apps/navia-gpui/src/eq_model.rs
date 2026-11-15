//! The equalizer's saved settings and curve math. `EqState::plan_for` turns one output's
//! settings into the core's chain, or `None` when off so the output stays bit-exact.

use crate::i18n::{tr, n_};
use std::collections::HashMap;

use serde_json::{Map, Value, json};

pub const BANDS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const BAND_LABELS: [(&str, &str); 10] = [("31 Hz", "31"), ("62 Hz", "62"), ("125 Hz", "125"), ("250 Hz", "250"), ("500 Hz", "500"), ("1 kHz", "1k"), ("2 kHz", "2k"), ("4 kHz", "4k"), ("8 kHz", "8k"), ("16 kHz", "16k")];
pub const EQ_RANGE: f32 = 12.0;
pub const FREQ_MIN: f32 = 20.0;
pub const FREQ_MAX: f32 = 20000.0;
pub const MAX_FILTERS: usize = 10;
pub const NAME_MAX: usize = 30;
const SAMPLES: usize = 180;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EqMode {
    Graphic,
    Parametric,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EqFilter {
    pub id: u64,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct EqProfile {
    pub mode: EqMode,
    /// The preset in use for graphic mode, or empty once the curve has been edited.
    pub preset: String,
    pub parametric_preset: String,
    pub gains: [f32; 10],
    pub filters: Vec<EqFilter>,
    pub preamp: f32,
    pub limiter: bool,
    /// The headphone correction by name, or empty for none.
    pub headphone: String,
    pub balance: f32,
    pub mono: bool,
    pub crossfeed: bool,
    pub crossfeed_amount: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CustomPreset {
    pub name: String,
    pub mode: EqMode,
    pub gains: Option<[f32; 10]>,
    pub filters: Option<Vec<EqFilter>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

/// One filter of a loaded headphone correction, in AutoEQ's terms.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CorrectionFilter {
    pub kind: FilterKind,
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Correction {
    /// Ten graphic bands, used only by the sample list.
    Bands([f32; 10]),
    /// A published AutoEQ profile, with its filters and preamp.
    Filters { preamp: f32, filters: Vec<CorrectionFilter> },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Headphone {
    pub name: String,
    pub group: String,
    pub correction: Correction,
}

pub struct BuiltIn {
    pub name: &'static str,
    pub gains: [f32; 10],
}

pub const BUILT_IN_PRESETS: [BuiltIn; 10] = [
    BuiltIn { name: n_("Flat"), gains: [0.0; 10] },
    BuiltIn { name: n_("Bass Boost"), gains: [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0] },
    BuiltIn { name: n_("Treble Boost"), gains: [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 3.5, 5.0, 6.0] },
    BuiltIn { name: n_("Vocal"), gains: [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.5, 2.0, 0.0, -1.0] },
    BuiltIn { name: n_("Loudness"), gains: [6.0, 4.5, 2.0, 0.0, -1.0, -1.5, -1.0, 1.0, 3.5, 5.0] },
    BuiltIn { name: n_("Rock"), gains: [4.0, 3.0, 1.5, -1.0, -1.5, -0.5, 1.5, 3.0, 4.0, 4.0] },
    BuiltIn { name: n_("Electronic"), gains: [5.0, 4.0, 1.0, 0.0, -1.5, 1.0, 0.5, 1.5, 4.0, 4.5] },
    BuiltIn { name: n_("Acoustic"), gains: [3.0, 3.0, 2.0, 0.5, 1.0, 1.0, 2.0, 2.5, 2.0, 1.5] },
    BuiltIn { name: n_("Reduced Bass"), gains: [-5.0, -4.0, -3.0, -1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    BuiltIn { name: n_("Spoken Word"), gains: [-4.0, -3.0, -1.0, 1.0, 3.0, 3.5, 3.0, 1.5, -1.0, -3.0] },
];

/// Models with invented curves, for `NAVIA_SAMPLE` runs only. Real runs load AutoEQ files.
pub fn sample_headphones() -> Vec<Headphone> {
    let row = |name: &str, group: &str, g: [f32; 10]| Headphone { name: name.to_string(), group: group.to_string(), correction: Correction::Bands(g) };
    vec![
        row("Sennheiser HD 600", "Over-ear (sample)", [4.0, 2.5, 0.5, -0.5, 0.0, 0.5, -1.0, 1.5, -2.0, 2.0]),
        row("Sennheiser HD 650", "Over-ear (sample)", [5.0, 3.0, 0.0, -1.0, 0.0, 1.0, -0.5, 2.0, -1.5, 3.0]),
        row("Beyerdynamic DT 770 Pro", "Over-ear (sample)", [1.0, 0.0, -1.5, -1.0, 0.5, 1.5, 2.0, -1.0, -4.5, 1.0]),
        row("Audio-Technica ATH-M50x", "Over-ear (sample)", [-1.5, -2.0, -2.5, -1.0, 1.0, 2.0, 1.5, 1.0, -3.0, 2.5]),
        row("Sony WH-1000XM4", "Over-ear (sample)", [-3.0, -3.5, -3.0, -1.0, 1.0, 1.5, 3.0, 2.0, 0.5, 3.5]),
        row("Moondrop Aria", "In-ear (sample)", [-1.0, -1.0, -0.5, 0.0, 0.5, 1.0, 1.0, -1.5, 1.0, 4.0]),
        row("Moondrop Blessing 2", "In-ear (sample)", [1.0, 0.5, 0.0, 0.0, 0.0, 0.5, -0.5, -1.0, 1.5, 3.5]),
        row("Sennheiser IE 600", "In-ear (sample)", [-1.5, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5, 0.0, -2.0, 1.0]),
    ]
}

pub fn clamp_gain(v: f32) -> f32 {
    v.clamp(-EQ_RANGE, EQ_RANGE)
}

pub fn clamp_freq(f: f32) -> f32 {
    f.clamp(FREQ_MIN, FREQ_MAX)
}

pub fn freq_to_x(f: f32) -> f32 {
    (f / FREQ_MIN).ln() / (FREQ_MAX / FREQ_MIN).ln()
}

pub fn x_to_freq(x: f32) -> f32 {
    FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf(x)
}

pub fn gain_to_y(g: f32) -> f32 {
    (EQ_RANGE - g) / (2.0 * EQ_RANGE)
}

pub fn y_to_gain(y: f32) -> f32 {
    EQ_RANGE - y * 2.0 * EQ_RANGE
}

/// Keeps drags to useful values: whole Hz below 1 kHz, 10 Hz steps above.
pub fn snap_freq(f: f32) -> f32 {
    clamp_freq(if f < 1000.0 { f.round() } else { (f / 10.0).round() * 10.0 })
}

pub fn snap_gain(g: f32) -> f32 {
    clamp_gain((g * 2.0).round() / 2.0)
}

/// `0`, or a signed value with one decimal and a real minus sign.
pub fn format_db(g: f32) -> String {
    if g == 0.0 {
        "0".to_string()
    } else {
        format!("{}{}", if g > 0.0 { '+' } else { '\u{2212}' }, crate::i18n::fmt_dec(g.abs() as f64, 1))
    }
}

pub fn format_freq(f: f32) -> String {
    if f < 1000.0 {
        return format!("{} Hz", crate::i18n::fmt_int(f.round() as i64));
    }
    let text = format!("{:.*}", if f < 10000.0 { 2 } else { 1 }, f / 1000.0);
    format!("{} kHz", crate::i18n::decimal_text(text.trim_end_matches('0').trim_end_matches('.')))
}

pub fn flat_profile(next_id: &mut u64) -> EqProfile {
    let mut filter = |freq: f32| {
        *next_id += 1;
        EqFilter { id: *next_id, freq, gain: 0.0, q: 1.0 }
    };
    let filters = vec![filter(100.0), filter(1000.0), filter(8000.0)];
    EqProfile { mode: EqMode::Graphic, preset: "Flat".to_string(), parametric_preset: String::new(), gains: [0.0; 10], filters, preamp: 0.0, limiter: true, headphone: String::new(), balance: 0.0, mono: false, crossfeed: false, crossfeed_amount: 35.0 }
}

/// A smooth line through the band gains that never overshoots a band next to a peak.
fn band_curve(gains: &[f32; 10]) -> impl Fn(f32) -> f32 + use<> {
    let gains = *gains;
    let xs: Vec<f32> = BANDS.iter().map(|f| freq_to_x(*f)).collect();
    let n = gains.len();
    let slopes: Vec<f32> = (0..n - 1).map(|i| (gains[i + 1] - gains[i]) / (xs[i + 1] - xs[i])).collect();
    let tangents: Vec<f32> = (0..n)
        .map(|i| {
            if i == 0 || i == n - 1 {
                return 0.0;
            }
            let (a, b) = (slopes[i - 1], slopes[i]);
            if a * b <= 0.0 {
                return 0.0;
            }
            a.signum() * ((a + b).abs() / 2.0).min(3.0 * a.abs().min(b.abs()))
        })
        .collect();
    move |x: f32| {
        if x <= xs[0] {
            return gains[0];
        }
        if x >= xs[n - 1] {
            return gains[n - 1];
        }
        let i = (0..n - 1).find(|k| x >= xs[*k] && x <= xs[*k + 1]).unwrap_or(0);
        let h = xs[i + 1] - xs[i];
        let t = (x - xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * gains[i] + (t3 - 2.0 * t2 + t) * h * tangents[i] + (-2.0 * t3 + 3.0 * t2) * gains[i + 1] + (t3 - t2) * h * tangents[i + 1]
    }
}

const SAMPLE_RATE: f32 = 48000.0;

/// Gain in dB of one filter at a frequency, from the standard biquad formulas.
pub fn filter_db(kind: FilterKind, freq: f32, gain: f32, q: f32, f: f32) -> f32 {
    let fs = SAMPLE_RATE;
    let a = 10f32.powf(gain / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * freq.min(fs * 0.45) / fs;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q.max(0.01));
    let (b0, b1, b2, a0, a1, a2) = match kind {
        FilterKind::Peaking => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
        FilterKind::LowShelf => {
            let s = 2.0 * a.sqrt() * alpha;
            (a * ((a + 1.0) - (a - 1.0) * cos + s), 2.0 * a * ((a - 1.0) - (a + 1.0) * cos), a * ((a + 1.0) - (a - 1.0) * cos - s), (a + 1.0) + (a - 1.0) * cos + s, -2.0 * ((a - 1.0) + (a + 1.0) * cos), (a + 1.0) + (a - 1.0) * cos - s)
        }
        FilterKind::HighShelf => {
            let s = 2.0 * a.sqrt() * alpha;
            (a * ((a + 1.0) + (a - 1.0) * cos + s), -2.0 * a * ((a - 1.0) + (a + 1.0) * cos), a * ((a + 1.0) + (a - 1.0) * cos - s), (a + 1.0) - (a - 1.0) * cos + s, 2.0 * ((a - 1.0) - (a + 1.0) * cos), (a + 1.0) - (a - 1.0) * cos - s)
        }
    };
    let w = 2.0 * std::f32::consts::PI * f.min(fs * 0.45) / fs;
    let (c1, s1) = (w.cos(), w.sin());
    let (c2, s2) = ((2.0 * w).cos(), (2.0 * w).sin());
    let mag = |x0: f32, x1: f32, x2: f32| (x0 + x1 * c1 + x2 * c2).hypot(x1 * s1 + x2 * s2);
    20.0 * (mag(b0, b1, b2) / mag(a0, a1, a2)).log10()
}

pub type CurvePoint = (f32, f32);

pub fn sample_bands(gains: &[f32; 10]) -> Vec<CurvePoint> {
    let at = band_curve(gains);
    (0..=SAMPLES)
        .map(|i| {
            let x = i as f32 / SAMPLES as f32;
            (x, at(x))
        })
        .collect()
}

pub fn sample_filters(filters: &[EqFilter]) -> Vec<CurvePoint> {
    (0..=SAMPLES)
        .map(|i| {
            let x = i as f32 / SAMPLES as f32;
            (x, filters.iter().map(|f| filter_db(FilterKind::Peaking, f.freq, f.gain, f.q, x_to_freq(x))).sum())
        })
        .collect()
}

/// The curve a correction draws, for the dashed line and the peak estimate.
pub fn correction_curve(c: &Correction) -> Vec<CurvePoint> {
    match c {
        Correction::Bands(g) => sample_bands(g),
        Correction::Filters { filters, .. } => (0..=SAMPLES)
            .map(|i| {
                let x = i as f32 / SAMPLES as f32;
                (x, filters.iter().map(|f| filter_db(f.kind, f.freq, f.gain, f.q, x_to_freq(x))).sum())
            })
            .collect(),
    }
}

pub fn profile_curve(p: &EqProfile) -> Vec<CurvePoint> {
    match p.mode {
        EqMode::Graphic => sample_bands(&p.gains),
        EqMode::Parametric => sample_filters(&p.filters),
    }
}

/// The highest level the curve, correction and preamps reach, in dB.
pub fn peak_level(p: &EqProfile, correction: Option<&Correction>) -> f32 {
    let curve = profile_curve(p);
    let extra = correction.map(correction_curve);
    let preamp = p.preamp + correction.map_or(0.0, |c| if let Correction::Filters { preamp, .. } = c { *preamp } else { 0.0 });
    curve.iter().enumerate().map(|(i, (_, g))| g + extra.as_ref().map_or(0.0, |e| e[i].1)).fold(f32::MIN, f32::max) + preamp
}

/// A spot for a new filter, in the middle of the widest empty stretch of the frequency axis.
pub fn emptiest_freq(filters: &[EqFilter]) -> f32 {
    let mut xs: Vec<f32> = vec![0.0];
    let mut inner: Vec<f32> = filters.iter().map(|f| freq_to_x(f.freq)).collect();
    inner.sort_by(f32::total_cmp);
    xs.extend(inner);
    xs.push(1.0);
    let mut best = 0;
    for i in 0..xs.len() - 1 {
        if xs[i + 1] - xs[i] > xs[best + 1] - xs[best] {
            best = i;
        }
    }
    snap_freq(x_to_freq((xs[best] + xs[best + 1]) / 2.0))
}

/// Parses an AutoEQ `ParametricEQ.txt` in Equalizer APO's format, a `Preamp: -6.4 dB` line and
/// then lines like `Filter 1: ON PK Fc 105 Hz Gain -1.5 dB Q 0.70`.
pub fn parse_autoeq(name: &str, text: &str) -> Result<Headphone, String> {
    let mut preamp = 0.0;
    let mut filters = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Preamp:") {
            preamp = rest.trim().trim_end_matches("dB").trim().parse().map_err(|_| tr("The preamp line isn't a number.").to_string())?;
            continue;
        }
        if !line.starts_with("Filter") {
            continue;
        }
        let Some((_, rest)) = line.split_once(':') else { continue };
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.first().is_some_and(|t| !t.eq_ignore_ascii_case("on")) {
            continue;
        }
        let Some(kind) = tokens.get(1).and_then(|t| match t.to_ascii_uppercase().as_str() {
            "PK" | "PEQ" => Some(FilterKind::Peaking),
            "LS" | "LSC" => Some(FilterKind::LowShelf),
            "HS" | "HSC" => Some(FilterKind::HighShelf),
            _ => None,
        }) else {
            continue;
        };
        let number = |key: &str| tokens.iter().position(|t| t.eq_ignore_ascii_case(key)).and_then(|i| tokens.get(i + 1)).and_then(|t| t.parse::<f32>().ok());
        if let (Some(freq), Some(gain), Some(q)) = (number("Fc"), number("Gain"), number("Q")) {
            filters.push(CorrectionFilter { kind, freq, gain, q });
        }
    }
    if filters.is_empty() {
        return Err(tr("No filters were found. Pick an AutoEQ ParametricEQ.txt file.").to_string());
    }
    Ok(Headphone { name: name.to_string(), group: n_("Loaded AutoEQ profiles").to_string(), correction: Correction::Filters { preamp, filters } })
}

/// The name to show for an imported file. `HD 600 ParametricEQ.txt` becomes `HD 600`.
pub fn headphone_name_from_path(path: &std::path::Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    stem.trim_end_matches("ParametricEQ").trim_end().to_string()
}

/// One filter for the audio chain.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PlanFilter {
    pub kind: FilterKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// What the audio chain does, built by `EqState::plan_for`. The core runs preamp, filters, mono,
/// crossfeed and balance, then the limiter last so its ceiling holds.
#[derive(Clone, PartialEq, Debug)]
pub struct EqPlan {
    pub preamp_db: f32,
    pub filters: Vec<PlanFilter>,
    pub limiter: bool,
    /// -1 is full left, 1 is full right.
    pub balance: f32,
    pub mono: bool,
    /// Strength from 0 to 1, when on.
    pub crossfeed: Option<f32>,
}

#[cfg(test)]
impl EqPlan {
    /// Whether the samples would pass through unchanged, so the chain can stay bypassed.
    pub fn is_identity(&self) -> bool {
        self.preamp_db == 0.0 && self.filters.iter().all(|f| f.gain_db == 0.0) && self.balance == 0.0 && !self.mono && self.crossfeed.is_none()
    }
}

#[derive(Clone, Debug)]
pub struct EqState {
    pub enabled: bool,
    /// The output whose profile is being edited.
    pub output: String,
    pub profiles: HashMap<String, EqProfile>,
    pub custom: Vec<CustomPreset>,
    /// AutoEQ profiles the user loaded from files.
    pub headphones: Vec<Headphone>,
    pub next_id: u64,
}

impl Default for EqState {
    fn default() -> Self {
        Self { enabled: false, output: OUTPUT_DEFAULT.to_string(), profiles: HashMap::new(), custom: Vec::new(), headphones: Vec::new(), next_id: 0 }
    }
}

/// The key for the system's default output.
pub const OUTPUT_DEFAULT: &str = "default";

/// Q for each graphic band when built as a peaking filter, about an octave wide.
const GRAPHIC_Q: f32 = 1.41;

fn path() -> Option<std::path::PathBuf> {
    crate::persist::config_dir().map(|d| d.join("equalizer.json"))
}

impl EqState {
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default() };
        serde_json::from_str::<Value>(&text).ok().map(|v| Self::from_json(&v)).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, serde_json::to_vec_pretty(&self.to_json()).unwrap_or_default());
    }

    /// The profile of the output being edited, created flat on first use.
    pub fn profile(&mut self) -> &mut EqProfile {
        let output = self.output.clone();
        if !self.profiles.contains_key(&output) {
            let flat = flat_profile(&mut self.next_id);
            self.profiles.insert(output.clone(), flat);
        }
        self.profiles.get_mut(&output).expect("just inserted")
    }

    /// The profile, or a flat one, without creating anything.
    pub fn profile_ref(&self) -> EqProfile {
        self.profiles.get(&self.output).cloned().unwrap_or_else(|| flat_profile(&mut 0))
    }

    pub fn new_filter(&mut self, freq: f32) -> EqFilter {
        self.next_id += 1;
        EqFilter { id: self.next_id, freq, gain: 0.0, q: 1.0 }
    }

    pub fn save_preset(&mut self, entry: CustomPreset) {
        let (name, mode) = (entry.name.clone(), entry.mode);
        let profile = self.profile();
        match mode {
            EqMode::Graphic => profile.preset = name.clone(),
            EqMode::Parametric => profile.parametric_preset = name.clone(),
        }
        self.custom.retain(|p| !(p.name == name && p.mode == mode));
        self.custom.push(entry);
        self.save();
    }

    /// Deletes a preset and clears it from every output that used it.
    pub fn delete_preset(&mut self, name: &str, mode: EqMode) {
        self.custom.retain(|p| !(p.name == name && p.mode == mode));
        for p in self.profiles.values_mut() {
            match mode {
                EqMode::Graphic if p.preset == name => p.preset.clear(),
                EqMode::Parametric if p.parametric_preset == name => p.parametric_preset.clear(),
                _ => {}
            }
        }
        self.save();
    }

    /// The settings of the output being edited, as an audio chain.
    #[cfg(test)]
    pub fn plan(&self, correction: Option<&Correction>) -> Option<EqPlan> {
        self.plan_for(&self.output, correction)
    }

    /// The saved profile for an output, or a flat one, without creating anything.
    pub fn profile_for(&self, output: &str) -> EqProfile {
        self.profiles.get(output).cloned().unwrap_or_else(|| flat_profile(&mut 0))
    }

    /// The settings for one output as an audio chain, or `None` when the equalizer is off.
    pub fn plan_for(&self, output: &str, correction: Option<&Correction>) -> Option<EqPlan> {
        if !self.enabled {
            return None;
        }
        let p = self.profile_for(output);
        let mut filters: Vec<PlanFilter> = match p.mode {
            EqMode::Graphic => BANDS.iter().zip(p.gains).map(|(f, g)| PlanFilter { kind: FilterKind::Peaking, freq: *f, gain_db: g, q: GRAPHIC_Q }).collect(),
            EqMode::Parametric => p.filters.iter().map(|f| PlanFilter { kind: FilterKind::Peaking, freq: f.freq, gain_db: f.gain, q: f.q }).collect(),
        };
        let mut preamp = p.preamp;
        match correction {
            Some(Correction::Bands(g)) => filters.extend(BANDS.iter().zip(g).map(|(f, g)| PlanFilter { kind: FilterKind::Peaking, freq: *f, gain_db: *g, q: GRAPHIC_Q })),
            Some(Correction::Filters { preamp: amount, filters: list }) => {
                preamp += amount;
                filters.extend(list.iter().map(|f| PlanFilter { kind: f.kind, freq: f.freq, gain_db: f.gain, q: f.q }));
            }
            None => {}
        }
        Some(EqPlan { preamp_db: preamp, filters, limiter: p.limiter, balance: p.balance / 100.0, mono: p.mono, crossfeed: p.crossfeed.then_some(p.crossfeed_amount / 100.0) })
    }

    pub fn to_json(&self) -> Value {
        let profiles: Map<String, Value> = self.profiles.iter().map(|(k, p)| (k.clone(), profile_json(p))).collect();
        json!({
            "enabled": self.enabled,
            "output": self.output,
            "profiles": profiles,
            "custom": self.custom.iter().map(preset_json).collect::<Vec<_>>(),
            "headphones": self.headphones.iter().map(headphone_json).collect::<Vec<_>>(),
            "next_id": self.next_id,
        })
    }

    pub fn from_json(v: &Value) -> Self {
        let mut state = Self { enabled: bool_at(v, "enabled", false), output: str_at(v, "output", OUTPUT_DEFAULT), next_id: v.get("next_id").and_then(Value::as_u64).unwrap_or(0), ..Default::default() };
        if let Some(obj) = v.get("profiles").and_then(Value::as_object) {
            for (k, p) in obj {
                if let Some(p) = profile_from(p) {
                    state.profiles.insert(k.clone(), p);
                }
            }
        }
        state.custom = v.get("custom").and_then(Value::as_array).map(|a| a.iter().filter_map(preset_from).collect()).unwrap_or_default();
        state.headphones = v.get("headphones").and_then(Value::as_array).map(|a| a.iter().filter_map(headphone_from).collect()).unwrap_or_default();
        // Saved files from before the counter can already hold filter ids.
        let highest = state.profiles.values().flat_map(|p| p.filters.iter().map(|f| f.id)).max().unwrap_or(0);
        state.next_id = state.next_id.max(highest);
        state
    }
}

fn f32_at(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(Value::as_f64).map_or(default, |x| x as f32)
}

fn bool_at(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn str_at(v: &Value, key: &str, default: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
}

fn gains_from(v: Option<&Value>) -> Option<[f32; 10]> {
    let list = v?.as_array()?;
    if list.len() != 10 {
        return None;
    }
    let mut out = [0.0; 10];
    for (i, x) in list.iter().enumerate() {
        out[i] = clamp_gain(x.as_f64()? as f32);
    }
    Some(out)
}

fn filter_json(f: &EqFilter) -> Value {
    json!({ "id": f.id, "freq": f.freq, "gain": f.gain, "q": f.q })
}

fn filter_from(v: &Value) -> Option<EqFilter> {
    Some(EqFilter { id: v.get("id")?.as_u64()?, freq: clamp_freq(v.get("freq")?.as_f64()? as f32), gain: clamp_gain(v.get("gain")?.as_f64()? as f32), q: (v.get("q")?.as_f64()? as f32).clamp(0.2, 8.0) })
}

fn mode_str(m: EqMode) -> &'static str {
    if m == EqMode::Parametric { "parametric" } else { "graphic" }
}

fn mode_from(v: Option<&Value>) -> EqMode {
    if v.and_then(Value::as_str) == Some("parametric") { EqMode::Parametric } else { EqMode::Graphic }
}

fn profile_json(p: &EqProfile) -> Value {
    json!({
        "mode": mode_str(p.mode),
        "preset": p.preset,
        "parametricPreset": p.parametric_preset,
        "gains": p.gains.to_vec(),
        "filters": p.filters.iter().map(filter_json).collect::<Vec<_>>(),
        "preamp": p.preamp,
        "limiter": p.limiter,
        "headphone": p.headphone,
        "balance": p.balance,
        "mono": p.mono,
        "crossfeed": p.crossfeed,
        "crossfeedAmount": p.crossfeed_amount,
    })
}

fn profile_from(v: &Value) -> Option<EqProfile> {
    let filters: Vec<EqFilter> = v.get("filters")?.as_array()?.iter().filter_map(filter_from).collect();
    Some(EqProfile {
        mode: mode_from(v.get("mode")),
        preset: str_at(v, "preset", ""),
        parametric_preset: str_at(v, "parametricPreset", ""),
        gains: gains_from(v.get("gains"))?,
        filters,
        preamp: clamp_gain(f32_at(v, "preamp", 0.0)),
        limiter: bool_at(v, "limiter", true),
        headphone: str_at(v, "headphone", ""),
        balance: f32_at(v, "balance", 0.0).clamp(-100.0, 100.0),
        mono: bool_at(v, "mono", false),
        crossfeed: bool_at(v, "crossfeed", false),
        crossfeed_amount: f32_at(v, "crossfeedAmount", 35.0).clamp(0.0, 100.0),
    })
}

fn preset_json(p: &CustomPreset) -> Value {
    json!({
        "name": p.name,
        "mode": mode_str(p.mode),
        "gains": p.gains.map(|g| g.to_vec()),
        "filters": p.filters.as_ref().map(|f| f.iter().map(filter_json).collect::<Vec<_>>()),
    })
}

fn preset_from(v: &Value) -> Option<CustomPreset> {
    Some(CustomPreset {
        name: v.get("name")?.as_str()?.to_string(),
        mode: mode_from(v.get("mode")),
        gains: gains_from(v.get("gains")),
        filters: v.get("filters").and_then(Value::as_array).map(|a| a.iter().filter_map(filter_from).collect()),
    })
}

fn kind_str(k: FilterKind) -> &'static str {
    match k {
        FilterKind::Peaking => "pk",
        FilterKind::LowShelf => "ls",
        FilterKind::HighShelf => "hs",
    }
}

fn headphone_json(h: &Headphone) -> Value {
    let filters = match &h.correction {
        Correction::Filters { filters, .. } => filters.iter().map(|f| json!({ "kind": kind_str(f.kind), "freq": f.freq, "gain": f.gain, "q": f.q })).collect::<Vec<_>>(),
        Correction::Bands(_) => Vec::new(),
    };
    let preamp = if let Correction::Filters { preamp, .. } = &h.correction { *preamp } else { 0.0 };
    json!({ "name": h.name, "group": h.group, "preamp": preamp, "filters": filters })
}

fn headphone_from(v: &Value) -> Option<Headphone> {
    let filters: Vec<CorrectionFilter> = v
        .get("filters")?
        .as_array()?
        .iter()
        .filter_map(|f| {
            let kind = match f.get("kind")?.as_str()? {
                "ls" => FilterKind::LowShelf,
                "hs" => FilterKind::HighShelf,
                _ => FilterKind::Peaking,
            };
            Some(CorrectionFilter { kind, freq: f32_at(f, "freq", 1000.0), gain: f32_at(f, "gain", 0.0), q: f32_at(f, "q", 1.0) })
        })
        .collect();
    Some(Headphone { name: v.get("name")?.as_str()?.to_string(), group: str_at(v, "group", "Loaded AutoEQ profiles"), correction: Correction::Filters { preamp: f32_at(v, "preamp", 0.0), filters } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_and_gain_axes_are_inverses() {
        assert!((freq_to_x(1000.0) - 0.5663).abs() < 0.001);
        assert!((x_to_freq(freq_to_x(440.0)) - 440.0).abs() < 0.01);
        assert_eq!(gain_to_y(12.0), 0.0);
        assert_eq!(gain_to_y(-12.0), 1.0);
        assert_eq!(y_to_gain(0.5), 0.0);
    }

    #[test]
    fn snapping_matches_the_reference() {
        assert_eq!(snap_freq(99.6), 100.0);
        assert_eq!(snap_freq(1234.0), 1230.0);
        assert_eq!(snap_gain(1.3), 1.5);
        assert_eq!(snap_gain(50.0), 12.0);
    }

    #[test]
    fn formats() {
        assert_eq!(format_db(0.0), "0");
        assert_eq!(format_db(1.5), "+1.5");
        assert_eq!(format_db(-3.0), "\u{2212}3.0");
        assert_eq!(format_freq(100.0), "100 Hz");
        assert_eq!(format_freq(1500.0), "1.5 kHz");
        assert_eq!(format_freq(12000.0), "12 kHz");
    }

    #[test]
    fn band_curve_passes_through_each_band_without_overshoot() {
        let gains = [0.0, 0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let at = band_curve(&gains);
        assert!((at(freq_to_x(125.0)) - 12.0).abs() < 0.01);
        let curve = sample_bands(&gains);
        assert!(curve.iter().all(|(_, g)| *g <= 12.01 && *g >= -0.01), "curve overshoots");
    }

    #[test]
    fn peaking_gain_at_its_own_frequency_matches_the_setting() {
        let db = filter_db(FilterKind::Peaking, 1000.0, 6.0, 1.0, 1000.0);
        assert!((db - 6.0).abs() < 0.05, "{db}");
    }

    #[test]
    fn shelves_reach_their_gain_at_the_far_end() {
        let low = filter_db(FilterKind::LowShelf, 105.0, -3.0, 0.7, 20.0);
        assert!((low + 3.0).abs() < 0.3, "{low}");
        let high = filter_db(FilterKind::HighShelf, 10000.0, 4.0, 0.7, 19000.0);
        assert!((high - 4.0).abs() < 0.8, "{high}");
    }

    #[test]
    fn autoeq_file_parses() {
        let text = "Preamp: -6.4 dB\nFilter 1: ON LSC Fc 105 Hz Gain -1.5 dB Q 0.70\nFilter 2: ON PK Fc 1000 Hz Gain 2 dB Q 1.20\nFilter 3: OFF PK Fc 2000 Hz Gain 2 dB Q 1.20\nFilter 4: ON HSC Fc 10000 Hz Gain -2.9 dB Q 0.70\n";
        let h = parse_autoeq("HD 600", text).unwrap();
        let Correction::Filters { preamp, filters } = h.correction else { panic!("not filters") };
        assert_eq!(preamp, -6.4);
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].kind, FilterKind::LowShelf);
        assert_eq!(filters[2], CorrectionFilter { kind: FilterKind::HighShelf, freq: 10000.0, gain: -2.9, q: 0.7 });
        assert!(parse_autoeq("x", "nothing here").is_err());
    }

    #[test]
    fn profile_name_from_path() {
        assert_eq!(headphone_name_from_path(std::path::Path::new("/tmp/HD 600 ParametricEQ.txt")), "HD 600");
    }

    #[test]
    fn plan_is_none_when_off_and_identity_when_flat() {
        let mut s = EqState::default();
        assert!(s.plan(None).is_none());
        s.enabled = true;
        let plan = s.plan(None).unwrap();
        assert!(plan.is_identity());
        s.profile().gains[3] = 4.0;
        s.profile().limiter = true;
        let plan = s.plan(None).unwrap();
        assert!(!plan.is_identity());
        assert_eq!(plan.filters.len(), 10);
        assert_eq!(plan.filters[3].gain_db, 4.0);
    }

    #[test]
    fn peak_level_adds_preamp_and_correction() {
        let mut p = flat_profile(&mut 0);
        p.gains[5] = 3.0;
        p.preamp = -1.0;
        assert!((peak_level(&p, None) - 2.0).abs() < 0.01);
        let bands = Correction::Bands([0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((peak_level(&p, Some(&bands)) - 4.0).abs() < 0.02);
    }

    #[test]
    fn json_round_trip_keeps_everything() {
        let mut s = EqState::default();
        s.enabled = true;
        s.output = "speakers".to_string();
        s.profile().gains = [1.0; 10];
        s.profile().crossfeed = true;
        s.profile().crossfeed_amount = 20.0;
        s.profile().balance = -30.0;
        s.custom.push(CustomPreset { name: "Mine".into(), mode: EqMode::Graphic, gains: Some([2.0; 10]), filters: None });
        s.headphones.push(parse_autoeq("IEM", "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1.0").unwrap());
        let back = EqState::from_json(&s.to_json());
        assert_eq!(back.enabled, true);
        assert_eq!(back.output, "speakers");
        assert_eq!(back.profiles["speakers"], s.profiles["speakers"]);
        assert_eq!(back.custom, s.custom);
        assert_eq!(back.headphones, s.headphones);
        assert_eq!(back.next_id, s.next_id);
    }

    #[test]
    fn deleting_a_preset_clears_it_everywhere() {
        let mut s = EqState::default();
        s.profile().preset = "Mine".to_string();
        s.custom.push(CustomPreset { name: "Mine".into(), mode: EqMode::Graphic, gains: Some([0.0; 10]), filters: None });
        s.delete_preset("Mine", EqMode::Graphic);
        assert!(s.custom.is_empty());
        assert_eq!(s.profiles[OUTPUT_DEFAULT].preset, "");
    }

    #[test]
    fn emptiest_freq_picks_the_widest_gap() {
        let filters = [EqFilter { id: 1, freq: 100.0, gain: 0.0, q: 1.0 }, EqFilter { id: 2, freq: 1000.0, gain: 0.0, q: 1.0 }, EqFilter { id: 3, freq: 8000.0, gain: 0.0, q: 1.0 }];
        let f = emptiest_freq(&filters);
        // The widest gap is 100 Hz to 1 kHz, and its log-scale midpoint is about 316 Hz.
        assert_eq!(f, 316.0);
    }
}
