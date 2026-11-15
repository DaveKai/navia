//! Blur-by-distance lyrics. GPUI can't blur an element's content, so text is rasterized and
//! blurred here, one sprite per line per blur level, on a background thread and cached.

use std::sync::{Arc, Condvar, Mutex};

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap};
use futures::channel::mpsc::UnboundedSender;
use gpui::{RenderImage, SharedString};
use image::{GrayImage, RgbaImage, imageops};

use crate::art::{ArtEvent, into_render_image};

/// Blur levels: 0 is sharp, then one per line of distance from the current line.
pub const BLUR_LEVELS: usize = 6;
/// Levels from this one up are stored at half resolution. The GPU scales them up and the blur hides it.
const HALF_RES_FROM: usize = 1;

/// The blur in logical pixels at a distance from the current line.
fn blur_css(level: usize) -> f32 {
    use crate::theme::size::{LYRIC_BLUR_MAX, LYRIC_BLUR_NEAR, LYRIC_BLUR_STEP};
    if level == 0 { 0.0 } else { (LYRIC_BLUR_NEAR + LYRIC_BLUR_STEP * level as f32).min(LYRIC_BLUR_MAX) }
}

/// A text line's identity. The text stays the same and only its layout varies.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LyricKey {
    pub song: SharedString,
    /// Device pixels, scaled to integers so keys compare exactly.
    pub font_px_x100: u32,
    pub wrap_px: u32,
    /// Device pixels per logical pixel.
    pub css_scale_x1000: u32,
    pub gap_px_x100: u32,
}

impl LyricKey {
    pub fn font_px(&self) -> f32 {
        self.font_px_x100 as f32 / 100.0
    }
}

#[derive(Clone)]
pub struct LineSprite {
    pub levels: Vec<Arc<RenderImage>>,
    /// Full sprite size, including the padding the blur spreads into, in device pixels.
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    /// Soft halo around the current line.
    pub glow: Arc<RenderImage>,
    pub glow_padding: f32,
    /// The wrapped text's visual lines, top to bottom, so the sweep can fill them one at a time.
    pub rows: Vec<SweepRow>,
}

/// One visual line of a wrapped lyric, in device pixels from the text's origin inside the padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepRow {
    pub top: f32,
    pub height: f32,
    pub x0: f32,
    pub x1: f32,
    /// Glyphs on this row. The sweep's progress is shared out by glyph in reading order.
    pub glyphs: usize,
    pub rtl: bool,
}

/// How much of each row is lit at `progress`, from 0 to 1 over the whole lyric.
pub fn row_fractions(rows: &[SweepRow], progress: f32) -> Vec<f32> {
    let total: usize = rows.iter().map(|r| r.glyphs).sum();
    let mut filled = progress.clamp(0.0, 1.0) * total.max(1) as f32;
    rows.iter()
        .map(|r| {
            let f = if r.glyphs == 0 { 1.0 } else { (filled / r.glyphs as f32).clamp(0.0, 1.0) };
            filled -= r.glyphs as f32;
            f
        })
        .collect()
}

fn sweep_rows(buffer: &Buffer) -> Vec<SweepRow> {
    buffer
        .layout_runs()
        .map(|run| {
            let x0 = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let x1 = run.glyphs.iter().map(|g| g.x + g.w).fold(f32::NEG_INFINITY, f32::max);
            let (x0, x1) = if run.glyphs.is_empty() { (0.0, 0.0) } else { (x0, x1) };
            SweepRow { top: run.line_top, height: run.line_height, x0, x1, glyphs: run.glyphs.len(), rtl: run.rtl }
        })
        .collect()
}

pub struct LyricLayout {
    /// Height of each line's text in device pixels, known before any sprite is ready.
    pub heights: Vec<f32>,
}

pub struct LyricJob {
    pub key: LyricKey,
    pub lines: Vec<SharedString>,
    /// Lines near this one are rasterized first.
    pub near: usize,
}

struct Slot {
    job: Mutex<Option<LyricJob>>,
    ready: Condvar,
}

/// A single worker. A new request replaces any waiting one and stops one in progress.
pub struct LyricRaster {
    slot: Arc<Slot>,
}

impl LyricRaster {
    pub fn start(events: UnboundedSender<ArtEvent>) -> Self {
        let slot = Arc::new(Slot { job: Mutex::new(None), ready: Condvar::new() });
        let worker = slot.clone();
        std::thread::Builder::new()
            .name("navia-lyrics".into())
            .spawn(move || worker_loop(&worker, &events))
            .expect("spawn lyric worker");
        Self { slot }
    }

    pub fn request(&self, job: LyricJob) {
        *self.slot.job.lock().unwrap() = Some(job);
        self.slot.ready.notify_one();
    }
}

fn load_fonts() -> FontSystem {
    let mut fonts = FontSystem::new();
    for bytes in [
        include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice(),
    ] {
        fonts.db_mut().load_font_data(bytes.to_vec());
    }
    fonts
}

fn worker_loop(slot: &Slot, events: &UnboundedSender<ArtEvent>) {
    // Loading system fonts is slow and costly, so it waits for the first lyrics.
    let mut fonts: Option<FontSystem> = None;
    let mut cache = SwashCache::new();
    loop {
        let job = {
            let mut guard = slot.job.lock().unwrap();
            loop {
                if let Some(job) = guard.take() {
                    break job;
                }
                guard = slot.ready.wait(guard).unwrap();
            }
        };
        run_job(&job, slot, fonts.get_or_insert_with(load_fonts), &mut cache, events);
    }
}

fn line_height(key: &LyricKey) -> f32 {
    (key.font_px() * 1.22).ceil()
}

fn run_job(job: &LyricJob, slot: &Slot, fonts: &mut FontSystem, cache: &mut SwashCache, events: &UnboundedSender<ArtEvent>) {
    let key = &job.key;
    // Shape everything first, since heights decide where every line goes.
    let mut buffers = Vec::with_capacity(job.lines.len());
    let mut heights = Vec::with_capacity(job.lines.len());
    for line in &job.lines {
        let (buffer, height) = shape(line, key, fonts);
        heights.push(height);
        buffers.push(buffer);
    }
    if events.unbounded_send(ArtEvent::LyricLayout { key: key.clone(), layout: LyricLayout { heights } }).is_err() {
        return;
    }
    let started = std::time::Instant::now();
    let mut order: Vec<usize> = (0..job.lines.len()).collect();
    order.sort_by_key(|i| i.abs_diff(job.near));
    for ix in order {
        if slot.job.lock().unwrap().is_some() {
            return;
        }
        let sprite = rasterize(&buffers[ix], key, fonts, cache);
        if events.unbounded_send(ArtEvent::LyricLine { key: key.clone(), ix, sprite }).is_err() {
            return;
        }
    }
    eprintln!("navia: lyrics rasterized {} lines in {:.0} ms", job.lines.len(), started.elapsed().as_secs_f64() * 1000.0);
}

fn shape(text: &str, key: &LyricKey, fonts: &mut FontSystem) -> (Buffer, f32) {
    let line_h = line_height(key);
    let attrs = Attrs::new().family(Family::Name(crate::assets::FONT_FAMILY)).weight(Weight::BOLD).letter_spacing(-0.01);
    let mut buffer = Buffer::new(fonts, Metrics::new(key.font_px(), line_h));
    {
        let mut b = buffer.borrow_with(fonts);
        b.set_wrap(Wrap::WordOrGlyph);
        b.set_size(Some(key.wrap_px as f32), None);
        b.set_text(text, &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(false);
    }
    let lines = buffer.layout_runs().count().max(1);
    (buffer, lines as f32 * line_h)
}

fn rasterize(buffer: &Buffer, key: &LyricKey, fonts: &mut FontSystem, cache: &mut SwashCache) -> LineSprite {
    let css_scale = key.css_scale_x1000 as f32 / 1000.0;
    let line_h = line_height(key);
    let lines = buffer.layout_runs().count().max(1) as u32;
    // Enough room that the widest blur fades out before the sprite's edge.
    let padding = (3.0 * blur_css(BLUR_LEVELS - 1) * css_scale).ceil() as u32 + 2;
    // The glow is a 16px drop shadow. As with a box shadow, its deviation is half the blur. The
    // margin reaches three deviations so the halo fades out before the sprite's edge.
    let glow_sigma = crate::theme::size::LYRIC_GLOW_BLUR / 2.0 * css_scale;
    let glow_padding = (3.0 * glow_sigma).ceil() as u32 + 4;
    // Only as wide as the text, since lines are often much shorter than the column.
    let text_w = buffer.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max).ceil().max(1.0) as u32;
    let width = text_w + padding * 2;
    let height = lines * line_h as u32 + padding * 2;

    let mut coverage = GrayImage::new(width, height);
    let mut borrowed = buffer.clone();
    let mut b = borrowed.borrow_with(fonts);
    b.draw(cache, Color::rgb(255, 255, 255), |x, y, w, h, color| {
        let alpha = color.a();
        if alpha == 0 {
            return;
        }
        for py in y..y + h as i32 {
            for px in x..x + w as i32 {
                let (cx, cy) = (px + padding as i32, py + padding as i32);
                if cx >= 0 && cy >= 0 && (cx as u32) < width && (cy as u32) < height {
                    let existing = coverage.get_pixel(cx as u32, cy as u32).0[0];
                    coverage.put_pixel(cx as u32, cy as u32, image::Luma([existing.max(alpha)]));
                }
            }
        }
    });

    let mut levels = Vec::with_capacity(BLUR_LEVELS);
    for level in 0..BLUR_LEVELS {
        let sigma = blur_css(level) * css_scale;
        let image = if level == 0 {
            coverage_to_image(&coverage, 1.0)
        } else if level >= HALF_RES_FROM {
            let half = imageops::resize(&coverage, width / 2, height / 2, imageops::FilterType::Triangle);
            coverage_to_image(&imageops::blur(&half, sigma / 2.0), 1.0)
        } else {
            coverage_to_image(&imageops::blur(&coverage, sigma), 1.0)
        };
        levels.push(image);
    }

    // The glow is very soft, so a quarter size is enough. It gets its own wider margin.
    let (gw, gh) = (width + (glow_padding - padding.min(glow_padding)) * 2, height + (glow_padding - padding.min(glow_padding)) * 2);
    let mut glow_source = GrayImage::new(gw, gh);
    let offset = glow_padding - padding.min(glow_padding);
    for y in 0..height {
        for x in 0..width {
            glow_source.put_pixel(x + offset, y + offset, *coverage.get_pixel(x, y));
        }
    }
    let quarter = imageops::resize(&glow_source, (gw / 4).max(1), (gh / 4).max(1), imageops::FilterType::Triangle);
    let glow = coverage_to_image(&imageops::blur(&quarter, glow_sigma / 4.0), crate::theme::size::LYRIC_GLOW_ALPHA);

    LineSprite { levels, width: width as f32, height: height as f32, padding: padding as f32, glow, glow_padding: (padding + offset) as f32, rows: sweep_rows(buffer) }
}

fn coverage_to_image(coverage: &GrayImage, gain: f32) -> Arc<RenderImage> {
    let mut rgba = RgbaImage::new(coverage.width(), coverage.height());
    for (src, dst) in coverage.pixels().zip(rgba.pixels_mut()) {
        dst.0 = [255, 255, 255, (f32::from(src.0[0]) * gain).round().clamp(0.0, 255.0) as u8];
    }
    into_render_image(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(glyphs: usize) -> SweepRow {
        SweepRow { top: 0.0, height: 10.0, x0: 0.0, x1: 100.0, glyphs, rtl: false }
    }

    #[test]
    fn rows_fill_in_reading_order() {
        let rows = [row(30), row(10)];
        assert_eq!(row_fractions(&rows, 0.0), vec![0.0, 0.0]);
        assert_eq!(row_fractions(&rows, 0.375), vec![0.5, 0.0]);
        assert_eq!(row_fractions(&rows, 0.75), vec![1.0, 0.0]);
        assert_eq!(row_fractions(&rows, 0.875), vec![1.0, 0.5]);
        assert_eq!(row_fractions(&rows, 1.0), vec![1.0, 1.0]);
    }

    #[test]
    fn real_text_wraps_into_rows_with_direction() {
        let mut fonts = FontSystem::new();
        let key = LyricKey { song: "s".into(), font_px_x100: 3000, wrap_px: 300, css_scale_x1000: 1000, gap_px_x100: 0 };
        let (buffer, _) = shape("In the stillness of the evening light, remembering what you had", &key, &mut fonts);
        let rows = sweep_rows(&buffer);
        assert!(rows.len() >= 2, "{rows:?}");
        assert!(rows.windows(2).all(|w| w[1].top > w[0].top));
        assert!(rows.iter().all(|r| !r.rtl && r.x1 > r.x0));
        let (buffer, _) = shape("في سكون الليل الطويل نتذكر كل ما كان لدينا", &key, &mut fonts);
        assert!(sweep_rows(&buffer).iter().all(|r| r.rtl));
    }
}
