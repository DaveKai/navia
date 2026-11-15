//! The lyrics panel, used by the dock and the expanded player. Lines are pre-rendered and blurred and dimmed by
//! distance from the current line.

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, Bounds, Context, FontWeight, Pixels, RenderImage, SharedString, div, px, relative};

use crate::lyrics::{BLUR_LEVELS, LineSprite, LyricJob, LyricKey, LyricLayout, row_fractions};
use crate::shell::{Layout, Shell};
use crate::theme::motion;
use crate::theme::size::*;
use crate::ui::icon;

/// Room above the current line in CSS pixels.
const LYRIC_LEAD: f32 = 36.0;
const LYRIC_LEAD_LARGE: f32 = 64.0;

/// Room above the lyrics. The first line starts this far below the panel top, clear of the fade, and a synced current
/// line scrolls here.
fn lyric_lead(large: bool) -> f32 {
    if large { LYRIC_LEAD_LARGE } else { LYRIC_LEAD }
}

/// Rasterizing is expensive, so a resize waits for the size to settle.
const REWRAP_DEBOUNCE: Duration = Duration::from_millis(140);

/// The sprites for one layout of one song, plus the animation state that goes with it.
pub struct LyricSet {
    pub key: LyricKey,
    pub layout: LyricLayout,
    pub sprites: Vec<Option<LineSprite>>,
    /// Where each line's text starts, in device pixels, from the top of the first line.
    pub tops: Vec<f32>,
    pub total: f32,
}

#[derive(Default)]
pub struct LyricsState {
    pub set: Option<LyricSet>,
    /// Images taken off screen, to free from the GPU atlas on the next frame.
    pub garbage: Vec<Arc<RenderImage>>,
    wanted: Option<LyricKey>,
    requested: Option<LyricKey>,
    generation: u64,
    /// Blur distance currently shown per line, easing toward the true distance.
    displayed: Vec<f32>,
    pub scroll: f32,
    scroll_set: bool,
    active: Option<usize>,
    last_frame: Option<Instant>,
    /// The hovered line, drawn sharp.
    pub hovered: Option<usize>,
    pub bounds: Rc<RefCell<Bounds<Pixels>>>,
    clock: LyricClock,
}

/// Playback position between the player's reports, so the sweep moves every frame.
#[derive(Default)]
struct LyricClock {
    anchor: f32,
    at: Option<Instant>,
    playing: bool,
    last_report: f32,
}

impl LyricClock {
    /// Capped at 0.75 s past the last report so a stalled player can't run ahead.
    fn estimate(&self, now: Instant) -> f32 {
        match self.at {
            Some(at) if self.playing => self.anchor + now.duration_since(at).as_secs_f32().min(0.75),
            _ => self.anchor,
        }
    }

    /// Takes a new report. Small differences are blended so the sweep never steps back, and seeks land at once.
    fn report(&mut self, position: f32, playing: bool, now: Instant) {
        if position == self.last_report && playing == self.playing && self.at.is_some() {
            return;
        }
        let predicted = self.estimate(now);
        self.anchor = if playing && self.playing && (position - predicted).abs() < 0.25 { predicted + (position - predicted) * 0.3 } else { position };
        self.at = Some(now);
        self.playing = playing;
        self.last_report = position;
    }
}

impl LyricsState {
    /// After the window was hidden, jump to the current line on the next frame instead of easing.
    pub fn restart_clock(&mut self) {
        self.last_frame = None;
        self.clock = LyricClock::default();
    }

    /// Takes the sprites off screen. Their images are freed from the GPU atlas with the garbage.
    fn retire_set(&mut self) {
        if let Some(old) = self.set.take() {
            for sprite in old.sprites.into_iter().flatten() {
                self.garbage.extend(sprite.levels);
                self.garbage.push(sprite.glow);
            }
        }
    }

    /// Drops every sprite and the record of what was requested, so the next draw lays the lyrics out again. They must
    /// go together, or the panel stays empty until the song or size changes.
    pub fn release(&mut self) -> Vec<Arc<RenderImage>> {
        self.retire_set();
        self.wanted = None;
        self.requested = None;
        // Any pending or in-flight request is for what was just released.
        self.generation += 1;
        std::mem::take(&mut self.garbage)
    }

    /// Whether sprites for `key` still have to be asked for.
    fn needs_request(&self, key: &LyricKey) -> bool {
        self.requested.as_ref() != Some(key) && self.wanted.as_ref() != Some(key)
    }
}

impl Shell {
    pub fn apply_lyric_layout(&mut self, key: LyricKey, layout: LyricLayout) {
        if self.lyrics_view.requested.as_ref() != Some(&key) {
            return;
        }
        self.lyrics_view.retire_set();
        let gap = gap_device(&key);
        let mut tops = Vec::with_capacity(layout.heights.len());
        let mut y = 0.0;
        for h in &layout.heights {
            tops.push(y);
            y += h + gap;
        }
        let n = layout.heights.len();
        self.lyrics_view.displayed = vec![BLUR_LEVELS as f32; n];
        self.lyrics_view.set = Some(LyricSet { key, layout, sprites: vec![None; n], tops, total: y });
        self.lyrics_view.scroll_set = false;
    }

    pub fn apply_lyric_line(&mut self, key: LyricKey, ix: usize, sprite: LineSprite) {
        if let Some(set) = self.lyrics_view.set.as_mut() {
            if set.key == key {
                if let Some(slot) = set.sprites.get_mut(ix) {
                    *slot = Some(sprite);
                }
            }
        }
    }

    pub fn drop_lyric_garbage(&mut self, window: &mut gpui::Window) {
        for image in std::mem::take(&mut self.lyrics_view.garbage) {
            let _ = window.drop_image(image);
        }
    }

    /// Asks for sprites when the song or size changes. A size change waits until it settles.
    fn request_lyric_sprites(&mut self, key: LyricKey, immediate: bool, cx: &mut Context<Self>) {
        if !self.lyrics_view.needs_request(&key) {
            return;
        }
        self.lyrics_view.wanted = Some(key.clone());
        self.lyrics_view.generation += 1;
        let generation = self.lyrics_view.generation;
        let near = self.current_lyric_line().unwrap_or(0);
        if immediate {
            self.start_lyric_job(key, near);
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REWRAP_DEBOUNCE).await;
            let _ = this.update(cx, |this, _| {
                if this.lyrics_view.generation == generation {
                    this.start_lyric_job(key, near);
                }
            });
        })
        .detach();
    }

    fn start_lyric_job(&mut self, key: LyricKey, near: usize) {
        let lines: Vec<SharedString> = self.lyrics.iter().map(|l| l.text.clone()).collect();
        self.lyrics_view.requested = Some(key.clone());
        self.lyric_raster.request(LyricJob { key, lines, near });
    }

    /// Whether the lyrics have timings to follow: every line synced and more than one start time.
    pub(crate) fn lyrics_synced(&self) -> bool {
        self.lyrics.iter().all(|l| l.synced) && self.lyrics.iter().map(|l| l.start_ms).collect::<std::collections::HashSet<_>>().len() > 1
    }

    /// The synced line being sung, or the first line before any has started.
    pub(crate) fn current_lyric_line(&self) -> Option<usize> {
        let position_ms = (self.player.position * 1000.0) as u32;
        // Before the first line the first line is shown as current.
        if self.lyrics.is_empty() { None } else { Some(self.lyrics.iter().rposition(|l| l.start_ms <= position_ms).unwrap_or(0)) }
    }

    /// `width` is the panel's width and `top_inset` is the dock head it must clear.
    pub fn render_lyrics_panel(&mut self, large: bool, width: Pixels, top_inset: Pixels, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if self.lyrics.is_empty() {
            return if self.lyrics_loading { div().into_any_element() } else { self.render_lyrics_empty(top_inset) };
        }
        // Lyrics without timings scroll as plain text.
        if !self.lyrics_synced() {
            return self.render_unsynced_lyrics(large, top_inset, layout);
        }
        let scale = layout.scale;
        let tokens_w = layout.tokens_w;
        let font_tokens = if large { (0.024 * tokens_w).clamp(26.0, 38.0) } else { TEXT_SECTION };
        let pad_x = if large { 0.0 } else { 32.0 };
        let gap = if large { 44.0 } else { 20.0 };
        let wrap = f32::from(width) - 2.0 * f32::from(t.s(pad_x));
        let key = LyricKey {
            song: self.current_song().map(|s| s.id.clone()).unwrap_or_default(),
            font_px_x100: (f32::from(t.s(font_tokens)) * scale * 100.0).round() as u32,
            wrap_px: (wrap * scale).round().max(40.0) as u32,
            css_scale_x1000: ((t.token_scale() * scale) * 1000.0).round() as u32,
            gap_px_x100: (f32::from(t.s(gap)) * scale * 100.0).round() as u32,
        };
        // During a dock drag the sprites are scaled, and laid out again once it ends.
        let resizing = self.dock_resizing() && self.lyrics_view.set.is_some();
        if !resizing {
            let immediate = self.lyrics_view.set.as_ref().is_none_or(|s| s.key.song != key.song || s.key.font_px_x100 != key.font_px_x100);
            self.request_lyric_sprites(key, immediate, cx);
        }

        let active = self.current_lyric_line().unwrap_or(0);
        let bounds = *self.lyrics_view.bounds.borrow();
        let view_h = f32::from(bounds.size.height).max(1.0);
        let now = Instant::now();
        let dt = self.lyrics_view.last_frame.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
        self.lyrics_view.last_frame = Some(now);
        let active_changed = self.lyrics_view.active != Some(active);
        self.lyrics_view.active = Some(active);

        // The expanded layout's text starts at the panel edge, so the clip extends past it by the sprites' widest halo.
        // The dock's padding already leaves that room.
        let bleed = if large { t.s(36.0) } else { px(0.0) };
        let mut body = div().relative().size_full();
        let Some(set) = self.lyrics_view.set.as_ref() else {
            return self.lyrics_frame(body.into_any_element(), top_inset, bleed, scale, cx);
        };
        // Sprites are scaled during a drag, so their positions scale with them.
        let draw_scale = (wrap * scale / set.key.wrap_px.max(1) as f32).max(0.1);
        let to_logical = |device: f32| px(device * draw_scale / scale);

        // Glide between lines, but jump the first time the view opens. The current line sits `LEAD` below the panel
        // top, so the line before it fades in above.
        let lead = f32::from(t.s(lyric_lead(large))) * scale;
        let (pad_top, align) = (lead, lead);
        let view_device = view_h * scale;
        let target = (set.tops.get(active).copied().unwrap_or(0.0) * draw_scale + pad_top - align).max(0.0);
        let max_scroll = ((set.total * draw_scale + view_device * if large { 0.70 } else { 0.62 }) - view_device).max(0.0);
        let target = target.min(max_scroll);
        self.lyrics_max_scroll = max_scroll;
        let snap = !self.lyrics_view.scroll_set || active_changed && dt == 0.0;
        if snap {
            self.lyrics_view.scroll = target;
            self.lyrics_view.scroll_set = true;
        }
        if active_changed {
            self.lyrics_view.hovered = None;
            self.lyrics_user_scrolled = false;
        }
        let mut animating = false;
        // Follow the current line unless the user is scrolling.
        if !self.lyrics_user_scrolled {
            let k = motion::follow(dt, motion::LYRIC_GLIDE);
            self.lyrics_view.scroll += (target - self.lyrics_view.scroll) * k;
            if (self.lyrics_view.scroll - target).abs() > 0.5 {
                animating = true;
            } else {
                self.lyrics_view.scroll = target;
            }
        }
        let scroll = self.lyrics_view.scroll.clamp(0.0, max_scroll);
        self.lyrics_view.scroll = scroll;
        let k_blur = motion::follow(dt, motion::LYRIC_DIM);
        let hovered = self.lyrics_view.hovered;
        let playing = self.player.playing && !self.player.waiting_for_server;
        self.lyrics_view.clock.report(self.player.position, playing, now);
        let position_ms = self.lyrics_view.clock.estimate(now) * 1000.0;
        let (line_start, line_end) = (
            self.lyrics.get(active).map_or(0, |l| l.start_ms) as f32,
            self.lyrics.get(active + 1).map_or_else(|| self.lyrics.get(active).map_or(0, |l| l.start_ms) as f32 + 4000.0, |l| l.start_ms as f32),
        );
        // Real word timing where the server has it (`songLyrics` v2), otherwise estimated from the line's span.
        let timed = self.lyrics.get(active).and_then(|l| l.timed.as_ref()).and_then(|l| l.sung_fraction(position_ms.max(0.0) as u32));
        let sweep = timed.unwrap_or_else(|| ((position_ms - line_start) / (line_end - line_start).min(6000.0).max(1.0)).clamp(0.0, 1.0));
        // Keep drawing frames only while the current line is still filling.
        if playing && sweep < 1.0 {
            animating = true;
        }

        let n = set.tops.len();
        let blur_by_distance = self.prefs.lyric_blur;
        let mut lines = Vec::new();
        for i in 0..n {
            let top_device = set.tops[i] * draw_scale + pad_top - scroll;
            let height_device = set.layout.heights[i] * draw_scale;
            if top_device + height_device < -400.0 || top_device > view_device + 400.0 {
                self.lyrics_view.displayed[i] = i.abs_diff(active).min(BLUR_LEVELS - 1) as f32;
                continue;
            }
            let goal = i.abs_diff(active).min(BLUR_LEVELS - 1) as f32;
            let shown = &mut self.lyrics_view.displayed[i];
            if snap {
                *shown = goal;
            } else {
                *shown += (goal - *shown) * k_blur;
                if (*shown - goal).abs() > 0.01 {
                    animating = true;
                } else {
                    *shown = goal;
                }
            }
            let shown = *shown;
            // GPUI has no masks, so each line fades whole near the panel's top and bottom. At the top the fade is clear
            // for `LYRIC_TOP_CLEAR` of the lead room, then ramps to full at the current line, using the value at each
            // line's middle.
            let center = (top_device + height_device / 2.0) / view_device.max(1.0);
            let clear = lead * LYRIC_TOP_CLEAR;
            let top_edge = ((top_device + height_device / 2.0 - clear) / (lead - clear).max(1.0)).clamp(0.0, 1.0);
            let edge = top_edge.min(((1.0 - center) / 0.12).clamp(0.0, 1.0));
            if edge <= 0.01 {
                continue;
            }
            let is_active = i == active;
            let is_hovered = hovered == Some(i) && !is_active;
            let opacity = if is_active {
                1.0
            } else if is_hovered {
                LYRIC_HOVER
            } else {
                (LYRIC_DIM_NEAR - shown * LYRIC_DIM_STEP).max(LYRIC_DIM_MIN).max(if shown < 1.0 { LYRIC_DIM_NEAR + (1.0 - LYRIC_DIM_NEAR) * (1.0 - shown) } else { 0.0 })
            };
            let (top, height) = (px(top_device / scale), to_logical(set.layout.heights[i]));
            let mut line = div()
                .id(("lyric", i))
                .absolute()
                .left(t.s(pad_x))
                .top(top)
                .w(px(wrap))
                .h(height)
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    let lv = &mut this.lyrics_view;
                    if *hovered {
                        lv.hovered = Some(i);
                    } else if lv.hovered == Some(i) {
                        lv.hovered = None;
                    }
                    cx.notify();
                }))
                .stop(&t).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ms) = this.lyrics.get(i).map(|l| l.start_ms) {
                        if this.core.is_some() {
                            this.core_seek_ms(u64::from(ms), cx);
                        } else {
                            this.player.position = ms as f32 / 1000.0;
                        }
                        this.lyrics_user_scrolled = false;
                        cx.notify();
                    }
                }));
            if let Some(sprite) = set.sprites[i].as_ref() {
                let draw_w = to_logical(sprite.width);
                let draw_h = to_logical(sprite.height);
                let pad = to_logical(sprite.padding);
                // GPUI's atlas packs images edge to edge and samples linearly, so a sprite drawn scaled or off the
                // pixel grid picks up its neighbor's outer pixels. Sprite margins are empty, so each one is clipped a
                // few device pixels inside its edge.
                let inset = px(SPRITE_EDGE / scale);
                let image = |level: usize, alpha: f32| clipped_sprite(sprite.levels[level].clone(), -pad, -pad, draw_w, draw_h, inset, alpha * edge);
                if is_active {
                    // A line that's becoming current eases in from blurred.
                    let blur_mix = shown.clamp(0.0, 1.0);
                    let sharp = 1.0 - blur_mix;
                    // The glow follows what's drawn. The unsung part glows at its own strength and the sung part fully.
                    let glow_pad = to_logical(sprite.glow_padding);
                    let glow_size = (to_logical(sprite.width + (sprite.glow_padding - sprite.padding) * 2.0), to_logical(sprite.height + (sprite.glow_padding - sprite.padding) * 2.0));
                    let lit = LYRIC_UNSUNG + (1.0 - LYRIC_UNSUNG) * sweep;
                    line = line.child(clipped_sprite(sprite.glow.clone(), -glow_pad, -glow_pad, glow_size.0, glow_size.1, inset, lit * sharp * edge));
                    if blur_mix > 0.02 {
                        line = line.child(image(1, blur_mix));
                    }
                    // The base text at 58%, then each visual line fills in reading order with a soft edge 14% of the
                    // row wide. The edge is drawn as strips on whole device pixels so they meet exactly.
                    line = line.child(image(0, LYRIC_UNSUNG * sharp));
                    let w = f32::from(draw_w);
                    let h = f32::from(draw_h);
                    let p = f32::from(pad);
                    let lo = f32::from(inset);
                    let snap = |x: f32| (x * scale).round() / scale;
                    let band = |x0: f32, x1: f32, y0: f32, y1: f32, alpha: f32| {
                        let (x0, x1) = (snap(x0.clamp(lo, w - lo)), snap(x1.clamp(lo, w - lo)));
                        div().absolute().left(-pad + px(x0)).top(-pad + px(y0)).w(px((x1 - x0).max(0.0))).h(px((y1 - y0).max(0.0))).overflow_hidden().child(
                            crate::ui::image(sprite.levels[0].clone()).absolute().left(px(-x0)).top(px(-y0)).w(draw_w).h(draw_h).opacity(alpha * edge),
                        )
                    };
                    let rows = &sprite.rows;
                    for (r, (row, f)) in rows.iter().zip(row_fractions(rows, sweep)).enumerate() {
                        if f <= 0.0 {
                            continue;
                        }
                        // The first and last rows reach the sprite's edges, so nothing in the padding is cut.
                        let y0 = snap(if r == 0 { lo } else { p + f32::from(to_logical(row.top)) });
                        let y1 = snap(if r + 1 == rows.len() { h - lo } else { p + f32::from(to_logical(row.top + row.height)) });
                        let (x0, x1) = (p + f32::from(to_logical(row.x0)), p + f32::from(to_logical(row.x1)));
                        let soft = (x1 - x0) * LYRIC_SWEEP_SOFT;
                        let cut = if row.rtl { x1 - (x1 - x0) * f } else { x0 + (x1 - x0) * f };
                        let steps = ((soft * scale / LYRIC_SWEEP_STEP_PX).round() as usize).clamp(1, LYRIC_SWEEP_STEPS_MAX);
                        // The sung part, out to the row's padding so no glyph is cut.
                        line = line.child(if row.rtl { band(cut, w, y0, y1, sharp) } else { band(0.0, cut, y0, y1, sharp) });
                        for k in 0..steps {
                            let (a, b) = (soft * k as f32 / steps as f32, soft * (k + 1) as f32 / steps as f32);
                            let alpha = 1.0 - (k as f32 + 0.5) / steps as f32;
                            line = line.child(if row.rtl { band(cut - b, cut - a, y0, y1, alpha * sharp) } else { band(cut + a, cut + b, y0, y1, alpha * sharp) });
                        }
                    }
                } else if is_hovered {
                    line = line.child(image(0, opacity));
                } else if !blur_by_distance {
                    line = line.child(image(0, opacity));
                } else {
                    let level_f = shown.clamp(0.0, BLUR_LEVELS as f32 - 1.001);
                    let low = level_f.floor() as usize;
                    let mix = level_f.fract();
                    for (level, alpha) in [(low, 1.0 - mix), (low + 1, mix)] {
                        if alpha > 0.02 && level < BLUR_LEVELS {
                            line = line.child(image(level, alpha * opacity));
                        }
                    }
                }
            } else {
                // The sprite is still loading, so plain text holds the line's place.
                let text = self.lyrics.get(i).map(|l| l.text.clone()).unwrap_or_default();
                line = line.child(
                    div()
                        .absolute()
                        .inset_0()
                        .text_size(px(set.key.font_px() / scale * draw_scale))
                        .line_height(relative(1.22))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.t_on_art())
                        .opacity(opacity * edge)
                        .child(text),
                );
            }
            lines.push(line.into_any_element());
        }
        if animating {
            self.lyrics_animating = true;
        }
        body = body.children(lines);
        self.lyrics_frame(body.into_any_element(), top_inset, bleed, scale, cx)
    }

    fn lyrics_frame(&mut self, body: AnyElement, top_inset: Pixels, bleed: Pixels, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let bounds_store = self.lyrics_view.bounds.clone();
        div()
            .absolute()
            .left(px(0.0))
            .right(px(0.0))
            .top(top_inset)
            .bottom(px(0.0))
            .child(crate::view_topbar::track_bounds(bounds_store))
            .child(
                div()
                    .id("lyrics-scroll")
                    .absolute()
                    .left(-bleed)
                    .right(-bleed)
                    .top(-bleed)
                    .bottom(-bleed)
                    .overflow_hidden()
                    .on_scroll(cx.listener(|this, event: &gpui::ScrollWheelEvent, window, cx| {
                        let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
                        this.lyrics_user_scrolled = true;
                        this.lyrics_view.scroll -= dy * window.scale_factor();
                        cx.notify();
                    }))
                    .child(div().absolute().left(bleed).right(bleed).top(bleed).bottom(bleed).child(body)),
            )
            .child(self.render_scrollbar("lyrics", crate::ui_scroll::ScrollSource::Lyrics, px(0.0), px(0.0), scale, cx))
            .into_any_element()
    }

    fn render_unsynced_lyrics(&mut self, large: bool, top_inset: Pixels, layout: &Layout) -> AnyElement {
        let t = self.theme;
        let font_tokens = if large { (0.024 * layout.tokens_w).clamp(26.0, 38.0) * 0.72 } else { TEXT_LG };
        let mut column = div().flex().flex_col().gap(t.s(if large { 16.0 } else { 10.0 })).px(t.s(if large { 0.0 } else { 32.0 })).pt(t.s(lyric_lead(large))).pb(t.s(40.0));
        for line in &self.lyrics {
            column = column.child(div().text_size(t.text(font_tokens)).font_weight(FontWeight::MEDIUM).line_height(relative(1.35)).text_color(gpui::Hsla::from(t.t_on_art()).opacity(LYRIC_PLAIN)).child(line.text.clone()));
        }
        div()
            .id("lyrics-plain")
            .absolute()
            .left(px(0.0))
            .right(px(0.0))
            .top(top_inset)
            .bottom(px(0.0))
            .overflow_y_scroll()
            .child(column)
            .into_any_element()
    }

    fn render_lyrics_empty(&mut self, top_inset: Pixels) -> AnyElement {
        let t = self.theme;
        div()
            .absolute()
            .left(px(0.0))
            .right(px(0.0))
            .top(top_inset)
            .bottom(px(0.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(t.s(8.0))
            .p(t.s(32.0))
            .text_center()
            .child(icon(&t, "quotes-regular", 26.0, t.t3()))
            .child(div().text_size(t.text(TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.t1()).child(tr("Lyrics aren't available")))
            .child(div().text_color(t.t2()).child(tr("The server has no synced lines for this song.")))
            .into_any_element()
    }
}

fn gap_device(key: &LyricKey) -> f32 {
    key.gap_px_x100 as f32 / 100.0
}

/// How far inside its edge a lyric sprite is clipped, in device pixels. Enough to stay clear of the atlas's linear
/// sampling into a neighbor at the quarter-size glow's scale.
const SPRITE_EDGE: f32 = 3.0;
/// The sweep's soft edge as a fraction of the row, then the strip width in device pixels and the most strips drawn.
const LYRIC_SWEEP_SOFT: f32 = 0.14;
const LYRIC_SWEEP_STEP_PX: f32 = 2.0;
const LYRIC_SWEEP_STEPS_MAX: usize = 40;

/// A lyric sprite at its place, clipped `inset` inside its edge (see `SPRITE_EDGE`).
fn clipped_sprite(image: Arc<RenderImage>, left: Pixels, top: Pixels, w: Pixels, h: Pixels, inset: Pixels, alpha: f32) -> gpui::Div {
    div()
        .absolute()
        .left(left + inset)
        .top(top + inset)
        .w((w - inset * 2.0).max(px(0.0)))
        .h((h - inset * 2.0).max(px(0.0)))
        .overflow_hidden()
        .child(crate::ui::image(image).absolute().left(-inset).top(-inset).w(w).h(h).opacity(alpha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::into_render_image;

    fn key() -> LyricKey {
        LyricKey { song: "song".into(), font_px_x100: 2400, wrap_px: 600, css_scale_x1000: 1000, gap_px_x100: 2000 }
    }

    fn sprite() -> LineSprite {
        let image = || into_render_image(image::RgbaImage::new(2, 2));
        LineSprite { levels: vec![image(), image()], width: 2.0, height: 2.0, padding: 0.0, glow: image(), glow_padding: 0.0, rows: Vec::new() }
    }

    /// Lyrics laid out and drawn for `key`, as before the window is hidden.
    fn laid_out() -> LyricsState {
        let mut state = LyricsState { requested: Some(key()), wanted: Some(key()), ..Default::default() };
        state.set = Some(LyricSet { key: key(), layout: LyricLayout { heights: vec![10.0, 10.0] }, sprites: vec![Some(sprite()), None], tops: vec![0.0, 20.0], total: 40.0 });
        state
    }

    #[test]
    fn drawn_lyrics_are_not_asked_for_again() {
        assert!(!laid_out().needs_request(&key()));
    }

    /// Minimizing and restoring must not leave the lyrics panel empty. When the sprites go, the request record must go
    /// too, or the panel never asks again.
    #[test]
    fn released_lyrics_are_asked_for_again() {
        let mut state = laid_out();
        let generation = state.generation;
        let images = state.release();
        assert_eq!(images.len(), 3, "both levels and the glow of the one drawn line");
        assert!(state.set.is_none() && state.garbage.is_empty());
        assert!(state.needs_request(&key()));
        assert_ne!(state.generation, generation, "a debounced request from before is dropped");
    }
}
