//! Small building blocks that replace framework defaults, such as icons, glass pills and icon buttons.

use crate::i18n::n_;
use gpui::prelude::*;
use gpui::{AnyElement, BoxShadow, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Svg, div, point, px, relative, svg};

use crate::assets::icon as icon_path;
use crate::theme::{Theme, motion};
use crate::theme::size::*;

/// Whether the renderer can blur what's beneath an element, as reported by the main window's
/// renderer. Windows assumes not until then, since its DirectX blur pipeline can fail to build.
static BACKDROP_BLUR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(cfg!(not(target_os = "windows")));

pub fn has_backdrop_blur() -> bool {
    BACKDROP_BLUR.load(std::sync::atomic::Ordering::Relaxed)
}

/// Records the answer from a newly opened main window's renderer and logs it once.
pub fn note_backdrop_blur(window: &gpui::Window) {
    static SAID: std::sync::Once = std::sync::Once::new();
    let on = window.supports_backdrop_blur();
    BACKDROP_BLUR.store(on, std::sync::atomic::Ordering::Relaxed);
    SAID.call_once(|| {
        if on {
            eprintln!("navia: backdrop blur on");
        } else {
            eprintln!("navia: backdrop blur unavailable, using solid stand-in fills");
        }
    });
}

/// Blurs what has already been painted beneath this element's rounded bounds. Later children
/// composite over the blur, so put the fill and contents after it.
pub fn backdrop_blur(corners: gpui::Corners<gpui::Pixels>, radius: gpui::Pixels, saturation: f32, bottom_fade: gpui::Pixels) -> impl IntoElement {
    backdrop_blur_faded(corners, radius, saturation, gpui::Edges { bottom: bottom_fade, ..Default::default() })
}

/// [`backdrop_blur`] fading out at its edges. `fades` is how far in from the top, right, bottom
/// and left the blur thins to nothing, with zero for a hard edge.
pub fn backdrop_blur_faded(corners: gpui::Corners<gpui::Pixels>, radius: gpui::Pixels, saturation: f32, fades: gpui::Edges<gpui::Pixels>) -> impl IntoElement {
    gpui::canvas(|_, _, _| {}, move |bounds, _, window, _| {
        if has_backdrop_blur() {
            window.paint_backdrop_blur_faded(bounds, corners, radius, saturation, fades);
        }
    })
    // Pin to all four edges. An absolute child with no insets sits inside the parent's padding,
    // which would shift the blur over the right and bottom edges.
    .absolute()
    .inset_0()
}

/// The layers of a glass surface, the blur of what's beneath and then its fill and edge. Call
/// it before the element's children. Without backdrop blur it falls back to a nearly opaque fill.
pub fn glass_layers<E: gpui::ParentElement + gpui::Styled>(el: E, t: &Theme, radius: gpui::Pixels, blur: f32, saturation: f32, frosted_fill: gpui::Rgba, stand_in_fill: gpui::Rgba, edge: Vec<BoxShadow>) -> E {
    if !has_backdrop_blur() {
        return el.bg(stand_in_fill).shadow(edge);
    }
    el.child(backdrop_blur(gpui::Corners::all(radius), t.s(blur), saturation, px(0.0)))
        .child(div().absolute().inset_0().rounded(radius).bg(frosted_fill).shadow(edge))
}

pub fn icon(t: &Theme, name: &str, token_size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(icon_path(&format!("{name}.svg"))).flex_none().size(t.s(token_size)).text_color(color)
}

/// Draws every image in the app, always with `Cover` fit so images never stretch. A bake drawn
/// while its box has a new size is cropped until the new bake arrives. Don't call GPUI's `img()`
/// directly, since it defaults to another fit.
pub fn image(source: std::sync::Arc<gpui::RenderImage>) -> gpui::Img {
    gpui::img(gpui::ImageSource::Render(source)).object_fit(gpui::ObjectFit::Cover)
}

/// The inset light edge every glass surface has.
pub fn glass_edge() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui::rgba(crate::theme::GLASS_EDGE).into(),
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(1.0),
            inset: true,
        },
        BoxShadow {
            color: gpui::rgba(crate::theme::GLASS_EDGE_TOP).into(),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
            inset: true,
        },
    ]
}

/// The inset light hairline of a filled button.
pub fn button_edge() -> Vec<BoxShadow> {
    vec![BoxShadow { color: gpui::rgba(crate::theme::BUTTON_EDGE).into(), offset: point(px(0.0), px(0.0)), blur_radius: px(0.0), spread_radius: px(1.0), inset: true }]
}

/// The glass edge followed by a drop shadow.
pub fn edge_and(shadow: BoxShadow) -> Vec<BoxShadow> {
    glass_edge().into_iter().chain([shadow]).collect()
}

fn drop(t: &Theme, color: u32, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow { color: gpui::rgba(color).into(), offset: point(px(0.0), t.s(y)), blur_radius: t.s(blur), spread_radius: t.s(spread), inset: false }
}

/// The shadow for floating panels, dialogs and the bottom bar.
pub fn shadow_float(t: &Theme) -> BoxShadow {
    let (color, y, blur, spread) = SHADOW_FLOAT;
    drop(t, color, y, blur, spread)
}

/// How far the float shadow reaches past its element on its longest side, in tokens. A window
/// of its own, like the mini player, keeps this much room. GPUI treats a shadow's blur radius
/// as the gaussian's sigma and draws out to three of them.
pub fn shadow_float_reach() -> f32 {
    let (_, y, blur, spread) = SHADOW_FLOAT;
    y + blur * 3.0 + spread
}

/// The shadow for large cover art on album, playlist and Discover pages.
pub fn shadow_art(t: &Theme) -> BoxShadow {
    let (color, y, blur, spread) = SHADOW_ART;
    drop(t, color, y, blur, spread)
}

/// The shadow for the top bar's glass pills.
pub fn shadow_pill(t: &Theme) -> BoxShadow {
    drop(t, 0x00000080, 8.0, 26.0, -10.0)
}

/// The shadow for the expanded player's cover.
pub fn shadow_player_art(t: &Theme) -> BoxShadow {
    drop(t, 0x00000099, 30.0, 70.0, -22.0)
}

/// The dock's shadow onto the page, to its left.
pub fn shadow_dock(t: &Theme) -> BoxShadow {
    BoxShadow { color: gpui::rgba(0x0000004f).into(), offset: point(t.s(-11.0), px(0.0)), blur_radius: t.s(22.0), spread_radius: t.s(-7.0), inset: false }
}

/// An inset 1px ring for edges drawn as an overlay above artwork, since inset shadows paint under children.
pub fn inset_ring(color: gpui::Rgba) -> BoxShadow {
    BoxShadow { color: color.into(), offset: point(px(0.0), px(0.0)), blur_radius: px(0.0), spread_radius: px(1.0), inset: true }
}

/// The shadow for a sidebar row being dragged.
pub fn shadow_ghost(t: &Theme) -> BoxShadow {
    drop(t, 0x00000066, 6.0, 18.0, 0.0)
}

/// The thumb over a seek or volume bar's playhead, centered on `x`. Hidden at rest, it fades and
/// grows in as `amount` goes from 0 to 1. `size` is `SCRUB_KNOB` or `SCRUB_THIN_KNOB`.
pub fn scrub_knob(t: &Theme, size: f32, x: gpui::Pixels, amount: f32) -> Div {
    use crate::theme::size::SCRUB_KNOB_REST_SCALE;
    let d = t.s(size * (SCRUB_KNOB_REST_SCALE + (1.0 - SCRUB_KNOB_REST_SCALE) * amount));
    div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .left(x - d / 2.0)
        .flex()
        .items_center()
        .opacity(amount)
        .child(div().size(d).rounded_full().bg(t.knob()).shadow(vec![shadow_knob(t)]))
}

/// The shadow for slider thumbs.
pub fn shadow_knob(t: &Theme) -> BoxShadow {
    drop(t, 0x00000066, 1.0, 4.0, 0.0)
}

/// The shadow for the EQ graph's handles.
pub fn shadow_eq_handle(t: &Theme) -> BoxShadow {
    drop(t, 0x00000073, 2.0, 8.0, 0.0)
}

/// The accent pill with the count on lifted songs, like "3 songs".
pub fn count_badge(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().h(t.s(20.0)).px(t.s(8.0)).flex().items_center().rounded(t.s(10.0)).bg(t.accent_hsla()).text_color(t.on_accent.hsla(1.0)).text_size(t.s(12.0)).font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().child(text.into())
}

/// The shadow for a row lifted by a drag.
pub fn shadow_lift(t: &Theme) -> BoxShadow {
    drop(t, 0x00000073, 12.0, 30.0, 0.0)
}

/// A pill-shaped glass container for one or more controls. Falls back to a mostly opaque fill
/// without backdrop blur.
pub fn glass_pill(t: &Theme) -> Div {
    let pill = div().flex().flex_row().items_center().flex_none().h(t.s(CONTROL_H)).px(t.s(3.0)).rounded_full();
    glass_layers(pill, t, t.s(CONTROL_H / 2.0), GLASS_BLUR, GLASS_SATURATION, t.glass_frost_fill(), t.glass_fill(), glass_edge())
}

/// A small single-line float chip, like the mini player's time chip. It moves with the playhead
/// every frame, so it uses the stand-in fill instead of its own backdrop blur.
pub fn float_chip(t: &Theme) -> Div {
    div().flex_none().px(t.s(8.0)).py(t.s(3.0)).rounded_full().bg(t.float_fill()).shadow(glass_edge()).text_size(t.text(TEXT_XS)).line_height(relative(1.2)).text_color(t.t1()).whitespace_nowrap()
}

/// A hover tip shown by GPUI after its hover delay. Pass `tip(&t, text)` to `.tooltip(...)`.
pub struct Tip {
    theme: Theme,
    text: SharedString,
}

impl gpui::Render for Tip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        float_chip(&self.theme).child(self.text.clone())
    }
}

pub fn tip(t: &Theme, text: impl Into<SharedString>) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let (theme, text) = (*t, text.into());
    move |_, cx| cx.new(|_| Tip { theme, text: text.clone() }).into()
}

/// A floating panel for menus, selects, popovers and the bar's volume panel. Add the contents as
/// children. It's `relative` so its glass layers fill it, but a later `absolute()` still wins.
pub fn float_panel(t: &Theme) -> Div {
    let radius = t.s(R_FLOAT);
    glass_layers(div().relative().rounded(radius).occlude(), t, radius, FLOAT_BLUR, FLOAT_SATURATION, t.float_glass(), t.float_fill(), edge_and(shadow_float(t)))
}

/// The filter field at the top of a floating list, for long selects and the Add to Playlist
/// submenu. `line` is the field's text line.
pub fn float_filter(t: &Theme, id: &'static str, line: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .gap(t.s(8.0))
        .h(t.s(34.0))
        .mb(t.s(4.0))
        .px(t.s(11.0))
        .rounded(t.s(10.0))
        .bg(t.row())
        .text_size(t.text(TEXT_SM))
        .child(icon(t, "magnifying-glass-regular", 15.0, t.t2()))
        .child(line)
}

/// A dialog's or toast's glass surface with the given corner radius.
pub fn dialog_surface(t: &Theme, radius: gpui::Pixels) -> Div {
    glass_layers(div().relative().rounded(radius), t, radius, FLOAT_BLUR, FLOAT_SATURATION, t.dialog_glass(), t.dialog_fill(), edge_and(shadow_float(t)))
}

/// A modal dialog's scrim over the window, centering its panel. The caller handles closing on
/// an outside click, and the panel comes from `dialog_panel`.
pub fn dialog_scrim(t: &Theme, id: &'static str) -> Stateful<Div> {
    div().absolute().inset_0().id(id).occlude().flex().items_center().justify_center().bg(t.scrim())
}

/// A modal dialog's panel, which keeps clicks inside it from reaching the scrim. Add
/// `dialog_title`, text, fields and `dialog_actions` as children.
pub fn dialog_panel(t: &Theme, id: &'static str) -> Stateful<Div> {
    dialog_surface(t, t.s(R_LG))
        .id(id)
        .w(t.s(DIALOG_W))
        .max_w_full()
        .p(t.s(DIALOG_PAD))
        .flex()
        .flex_col()
        .gap(t.s(DIALOG_GAP))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The dialog's sheet variant, as used by Get Info. Its height is capped by a token and the
/// window. Children are a head, a scrolling body (`flex_1`, `min_h(0)`) and `dialog_sheet_actions`.
pub fn dialog_sheet(t: &Theme, id: &'static str, window_h: gpui::Pixels) -> Stateful<Div> {
    dialog_panel(t, id).w(t.s(SHEET_W)).max_h(t.s(SHEET_MAX_H).min(window_h - t.s(40.0))).pb(t.s(DIALOG_PAD - DIALOG_ACTIONS_TOP))
}

/// The dialog's cover variant, as used by an artist's About. The sheet's size with no padding
/// so its picture runs to the edges. Children are a scrolling body and a round close button.
pub fn dialog_cover(t: &Theme, id: &'static str, window_h: gpui::Pixels) -> Stateful<Div> {
    dialog_surface(t, t.s(R_LG))
        .id(id)
        .w(t.s(SHEET_W))
        .max_w_full()
        .max_h(t.s(SHEET_MAX_H).min(window_h - t.s(40.0)))
        .flex()
        .flex_col()
        .overflow_hidden()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A cover sheet's picture at the top of its body, already faded out, with the text below
/// rising `SHEET_COVER_OVERLAP` over the fade. Its space is held while it bakes.
pub fn dialog_cover_picture(t: &Theme, image: Option<std::sync::Arc<gpui::RenderImage>>) -> Div {
    div()
        .h(t.s(SHEET_COVER_H))
        .mx(-t.s(DIALOG_PAD))
        .mb(-t.s(SHEET_COVER_OVERLAP))
        .when_some(image, |d, image| d.child(self::image(image).size_full()))
}

/// A sheet's buttons, right-aligned under a hairline across the sheet's full width.
pub fn dialog_sheet_actions(t: &Theme) -> Div {
    dialog_actions(t).mt(px(0.0)).mx(-t.s(DIALOG_PAD)).px(t.s(DIALOG_PAD)).pt(t.s(DIALOG_GAP)).border_t_1().border_color(t.line())
}

/// A dialog's title.
pub fn dialog_title(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(t.text(TEXT_SECTION)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.t1()).child(text.into())
}

/// Small danger text explaining why something failed, shown under the form it belongs to.
pub fn error_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(t.text(TEXT_SM)).text_color(gpui::rgb(crate::theme::DANGER)).line_height(gpui::relative(1.35)).child(text.into())
}

/// A dialog's body text.
pub fn dialog_text(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_color(t.t2()).line_height(gpui::relative(1.4)).child(text.into())
}

/// A dialog's small print.
pub fn dialog_note(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(t.text(TEXT_SM)).text_color(t.t3()).line_height(gpui::relative(1.4)).child(text.into())
}

/// A labeled control in a dialog, with the label above it.
pub fn dialog_block(t: &Theme, label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div().flex().flex_col().gap(t.s(6.0)).text_size(t.text(TEXT_SM)).text_color(t.t2()).child(label.into()).child(control)
}

/// A dialog's switch row, with its label and hint and the switch at the end.
pub fn dialog_switch(t: &Theme, label: impl Into<SharedString>, hint: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(t.s(16.0))
        .child(div().min_w(px(0.0)).child(div().font_weight(gpui::FontWeight::MEDIUM).child(label.into())).child(div().text_size(t.text(TEXT_SM)).text_color(t.t2()).child(hint.into())))
        .child(control)
}

/// A dialog's buttons, right-aligned.
pub fn dialog_actions(t: &Theme) -> Div {
    div().flex().flex_row().justify_end().gap(t.s(DIALOG_ACTIONS_GAP)).mt(t.s(DIALOG_ACTIONS_TOP))
}

/// A dialog's text field. A glass pill with the focus ring, an optional icon, then `line`.
pub fn dialog_field(t: &Theme, id: &'static str, focused: bool, glyph: Option<&str>, line: impl IntoElement) -> Stateful<Div> {
    let edge = if focused { crate::ui_controls::with_ring(glass_edge(), t.accent_hsla()) } else { glass_edge() };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(t.s(9.0))
        .h(t.s(CONTROL_H))
        .px(t.s(FIELD_PAD))
        .rounded_full()
        .bg(t.glass_fill())
        .shadow(edge)
        .text_color(t.t2())
        .cursor_text()
        .when_some(glyph, |d, g| d.child(icon(t, g, 16.0, t.t2())))
        .child(line)
}

/// The 2px accent focus ring as a separate outline around a translucent control. As a shadow it
/// would show through under a frosted fill.
pub fn focus_ring(t: &Theme, radius: gpui::Pixels) -> Div {
    outline_ring(radius, t.accent_hsla(), px(2.0))
}

/// An outer ring in `color` around a translucent control, drawn as its own outline. Used for the
/// focus ring and the invalid ring.
pub fn outline_ring(radius: gpui::Pixels, color: impl Into<Hsla>, w: gpui::Pixels) -> Div {
    div().absolute().top(-w).bottom(-w).left(-w).right(-w).rounded(radius + w).border(w).border_color(color.into())
}

/// A search field's size, a page toolbar's or the Search page's large one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SearchSize {
    Toolbar,
    Large,
}

/// A glass search field with its magnifying glass, then `line`. Focus shows as an outer ring,
/// not a border, so nothing inside moves.
pub fn search_field(t: &Theme, id: &'static str, size: SearchSize, width: f32, focused: bool, line: impl IntoElement) -> Stateful<Div> {
    let (h, pad, gap, glyph) = match size {
        SearchSize::Toolbar => (CONTROL_H, 15.0, 9.0, 18.0),
        SearchSize::Large => (53.0, 19.0, 11.0, 22.0),
    };
    let edge = if size == SearchSize::Large { edge_and(shadow_pill(t)) } else { glass_edge() };
    let el = div().id(id).relative().flex().flex_row().items_center().gap(t.s(gap)).w(t.s(width)).h(t.s(h)).px(t.s(pad)).rounded_full();
    glass_layers(el, t, t.s(h / 2.0), GLASS_BLUR, GLASS_SATURATION, t.glass_frost_fill(), t.glass_fill(), edge)
        .when(focused, |d| d.child(focus_ring(t, t.s(h / 2.0))))
        .occlude()
        .cursor_text()
        .child(icon(t, "magnifying-glass-regular", glyph, t.t2()))
        .child(line)
}

pub fn divider(t: &Theme) -> Div {
    div().w(px(1.0)).h(t.s(20.0)).mx(t.s(3.0)).bg(t.glass_divider()).flex_none()
}

/// An icon button that brightens on hover and sits on glass when active.
pub fn icon_button(t: &Theme, id: impl Into<ElementId>, name: &str, active: bool) -> Stateful<Div> {
    icon_button_colored(t, id, name, active, t.t1().into(), t.hover_ink())
}

/// A small icon button in `color` that brightens to `hover` on hover, like a song row's heart.
pub fn icon_button_colored(t: &Theme, id: impl Into<ElementId>, name: &str, active: bool, color: Hsla, hover: Hsla) -> Stateful<Div> {
    let id = id.into();
    if active {
        return icon_button_shell(t, id, true).child(icon(t, name, 20.0, color));
    }
    ink_button(t, id, name, 20.0, color, hover)
}

/// A small icon button whose glyph rests at `ICON_REST` in `rest` and brightens to `hover`. The
/// player's and bar's buttons pass their own colors.
pub fn ink_button(t: &Theme, id: impl Into<ElementId>, name: &str, size: f32, rest: Hsla, hover: Hsla) -> Stateful<Div> {
    let id = id.into();
    let ink = ink(id.clone());
    icon_button_shell(t, id.clone(), false).child(hover_track(id)).opacity(ink.opacity(ICON_REST)).child(icon(t, name, size, ink.color(rest, hover)))
}

/// The margin around an icon drawn to be scaled (`assets::GROW_PREFIX`), as a fraction of its side.
pub const GROW_MARGIN: f32 = 0.1;

/// Play, pause and skip, with no fill. The glyph grows on hover and shrinks while pressed.
pub fn transport_button(t: &Theme, id: impl Into<ElementId>, name: &str, size: f32, color: Hsla, enabled: bool) -> Stateful<Div> {
    let id = id.into();
    let ink = ink(id.clone());
    let scale = if enabled { ink.scale() } else { 1.0 };
    // Rasterize once at the grown size and scale down on the GPU, so the glyph moves smoothly by
    // fractions of a pixel. Resizing would snap to whole pixels. The raster's transparent margin keeps
    // scaling from sampling a neighboring glyph in the atlas.
    let grown = size * HOVER_GROW;
    let side = t.s(grown * (1.0 + 2.0 * GROW_MARGIN));
    let glyph = svg()
        .path(format!("{}{name}.svg", crate::assets::GROW_PREFIX))
        .absolute()
        .size(side)
        .left((t.s(size) - side) / 2.0)
        .top((t.s(size) - side) / 2.0)
        .text_color(color)
        .with_transformation(gpui::Transformation::scale(gpui::size(scale / HOVER_GROW, scale / HOVER_GROW)));
    div()
        .id(id.clone())
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .when(enabled, |d| d.child(hover_track(id)))
        .child(div().relative().size(t.s(size)).child(glyph))
}

/// `icon_button` with a drawn glyph instead of an icon, like the Download switch's progress ring.
pub fn icon_button_with(t: &Theme, id: impl Into<ElementId>, active: bool, glyph: impl IntoElement) -> Stateful<Div> {
    let id = id.into();
    icon_button_shell(t, id, active).child(glyph)
}

/// The download mark. Filled when downloaded, a filling ring while downloading, gray while
/// waiting. `fetching` is a quarter ring and `queued` a bare ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadGlyph {
    Done,
    Running,
    Paused,
    Queued,
    Fetching,
}

/// Draws a download glyph `size` tokens square. `progress` from 0 to 1 fills the ring, and `ink`
/// colors the arrow until done.
pub fn download_glyph(t: &Theme, glyph: DownloadGlyph, progress: f32, size: f32, ink: Hsla) -> impl IntoElement {
    let (done_fill, done_ink, track, paused) = (t.downloaded(), t.downloaded_ink(), t.download_track(), t.download_paused());
    let arc_color: Hsla = if glyph == DownloadGlyph::Paused { paused.into() } else { done_fill };
    let arrow_color: Hsla = match glyph {
        DownloadGlyph::Done => done_ink,
        DownloadGlyph::Queued => paused.into(),
        _ => ink,
    };
    let arc = match glyph {
        DownloadGlyph::Fetching => 0.25,
        DownloadGlyph::Done | DownloadGlyph::Queued => 0.0,
        _ => progress.clamp(0.04, 1.0),
    };
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // The glyph's 24-unit design grid, scaled to the bounds.
            let u = bounds.size.width / 24.0;
            let at = |x: f32, y: f32| bounds.origin + point(u * x, u * y);
            let ring = |r: f32, stroke: Option<f32>, sweep_to: f32| {
                let mut b = match stroke {
                    Some(w) => gpui::PathBuilder::stroke(u * w),
                    None => gpui::PathBuilder::fill(),
                };
                let angle = |f: f32| (f * std::f32::consts::TAU) - std::f32::consts::FRAC_PI_2;
                let p = |f: f32| at(12.0 + r * angle(f).cos(), 12.0 + r * angle(f).sin());
                b.move_to(p(0.0));
                // At most two halves, so each arc is unambiguous.
                let mid = sweep_to.min(0.5);
                b.arc_to(point(u * r, u * r), px(0.0), false, true, p(mid));
                if sweep_to > 0.5 {
                    b.arc_to(point(u * r, u * r), px(0.0), false, true, p(sweep_to.min(0.9999)));
                }
                b.build().ok()
            };
            if glyph == DownloadGlyph::Done {
                if let Some(path) = ring(11.0, None, 0.9999) {
                    window.paint_path(path, done_fill);
                }
            } else {
                if let Some(path) = ring(10.0, Some(2.0), 0.9999) {
                    window.paint_path(path, track);
                }
                if arc > 0.0
                    && let Some(path) = ring(10.0, Some(2.0), arc)
                {
                    window.paint_path(path, arc_color);
                }
            }
            let w = if glyph == DownloadGlyph::Done { 2.4 } else { 2.0 };
            let mut arrow = gpui::PathBuilder::stroke(u * w);
            arrow.move_to(at(12.0, 6.5));
            arrow.line_to(at(12.0, 17.0));
            arrow.move_to(at(7.5, 12.5));
            arrow.line_to(at(12.0, 17.0));
            arrow.line_to(at(16.5, 12.5));
            if let Ok(path) = arrow.build() {
                window.paint_path(path, arrow_color);
            }
        },
    )
    .flex_none()
    .size(t.s(size))
}

fn icon_button_shell(t: &Theme, id: ElementId, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(t.s(ICON_W))
        .h(t.s(34.0))
        .rounded_full()
        .when(active, |d| d.bg(t.glass_on()))
}

/// The small "E" after a title the server flags explicit, sized from the text it follows.
pub fn explicit_mark(t: &Theme, text: f32) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .ml(t.s(text * 0.27))
        .size(t.s(text * 0.85))
        .rounded(t.s(3.0))
        .bg(gpui::rgba(crate::theme::EXPLICIT_FILL))
        .text_color(gpui::rgba(crate::theme::EXPLICIT_INK))
        .text_size(t.s(text * 0.68))
        .font_weight(gpui::FontWeight::BOLD)
        .line_height(gpui::relative(1.0))
        .child("E")
}

/// A one-line title followed by its explicit mark when it has one. `title` should truncate itself.
pub fn titled(t: &Theme, title: gpui::Div, explicit: bool, text: f32) -> gpui::Div {
    div().flex().flex_row().items_center().min_w(gpui::px(0.0)).child(title.min_w(gpui::px(0.0))).when(explicit, |d| d.child(explicit_mark(t, text)))
}

/// The pointing hand cursor, for text links only, like artist and album names or a sortable
/// column heading. Buttons, rows, cards and other controls keep the arrow.
pub trait LinkCursor: Styled + Sized {
    fn link_cursor(self) -> Self {
        self.cursor(gpui::CursorStyle::PointingHand)
    }
}

impl<E: Styled> LinkCursor for E {}

/// Dims a tile, card or other large link while pressed. Rows and small controls use their own hover.
pub fn press_dim(el: Stateful<Div>) -> Stateful<Div> {
    el.active(|s| s.opacity(PRESS_DIM))
}

/// The usual hover for a button or a segment that can't be selected.
pub fn hover_pill(t: &Theme) -> impl IntoElement {
    hover_fill(HoverShape::Pill, t.hover(), t.press())
}

/// A small control's hover. There's no fill, the ink brightens on hover and dims while pressed,
/// and play and skip grow instead. Read it while building the control with the same key as its
/// `hover_track` child. It lags the pointer by a frame.
#[derive(Clone, Copy, Default)]
pub struct Ink {
    pub amount: f32,
    pub pressed: bool,
    /// Press progress from 0 to 1, over `HOVER_PRESS` going down and `HOVER_IN` coming back up.
    pub press: f32,
}

impl Ink {
    /// The ink between its resting color and the hover color.
    pub fn color(self, rest: impl Into<Hsla>, hover: impl Into<Hsla>) -> Hsla {
        let (a, b) = (rest.into(), hover.into());
        let t = self.amount;
        let (ra, ba) = (a.to_rgb(), b.to_rgb());
        gpui::Rgba { r: ra.r + (ba.r - ra.r) * t, g: ra.g + (ba.g - ra.g) * t, b: ra.b + (ba.b - ra.b) * t, a: ra.a + (ba.a - ra.a) * t }.into()
    }
    /// The control's opacity, from `rest` up to 1, dimmed while pressed.
    pub fn opacity(self, rest: f32) -> f32 {
        if self.pressed { PRESS_DIM } else { rest + (1.0 - rest) * self.amount }
    }
    /// The scale for play and skip, grown on hover and shrunk while pressed.
    pub fn scale(self) -> f32 {
        let hover = 1.0 + (motion::hover_grow() - 1.0) * self.amount;
        let shrink = if motion::reduced() { 1.0 } else { PRESS_SHRINK };
        hover + (shrink - hover) * self.press
    }
}

thread_local! {
    static INKS: std::cell::RefCell<std::collections::HashMap<ElementId, HoverFade>> = Default::default();
}

/// The hover state `hover_track(key)` last painted.
pub fn ink(key: impl Into<ElementId>) -> Ink {
    let key = key.into();
    let now = std::time::Instant::now();
    INKS.with(|m| m.borrow().get(&key).map(|f| Ink { amount: f.amount(now), pressed: f.pressed.get(), press: f.press(now) }).unwrap_or_default())
}

/// Tracks the pointer over a small control for `ink(key)`. Add it as the control's first child.
/// It redraws the view while the ink fades and on enter, leave and press.
pub fn hover_track(key: impl Into<ElementId>) -> impl IntoElement {
    track(key.into(), 0.0)
}

/// Tracks the pointer over an area that reveals controls on hover, for `ink(key).amount`. Add it
/// as the area's last child, so its hitbox sits over the area's own controls and they stay while
/// hovered. Anything drawn over the area from outside still hides it. After the pointer leaves,
/// the controls wait a grace period before fading.
pub fn reveal_track(key: impl Into<ElementId>) -> impl IntoElement {
    track(key.into(), motion::REVEAL_GRACE)
}

thread_local! {
    /// When the pointer left each `reveal_track` still within its grace period.
    static LEFT: std::cell::RefCell<std::collections::HashMap<ElementId, std::time::Instant>> = Default::default();
}

fn track(key: ElementId, grace: f32) -> impl IntoElement {
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            let now = std::time::Instant::now();
            // The hit test keeps the last position once the pointer leaves the window, so check the window too.
            let mut hovered = window.is_window_hovered() && hitbox.is_hovered(window);
            if grace > 0.0 {
                let was = INKS.with(|m| m.borrow().get(&key).is_some_and(|f| f.hovered));
                hovered = LEFT.with(|l| {
                    let mut l = l.borrow_mut();
                    if hovered || !was {
                        l.remove(&key);
                        return hovered;
                    }
                    let left = *l.entry(key.clone()).or_insert(now);
                    let held = now.saturating_duration_since(left).as_secs_f32() < grace;
                    if held {
                        window.request_animation_frame();
                    } else {
                        l.remove(&key);
                    }
                    held
                });
            }
            let (amount, pressed, press) = INKS.with(|m| {
                let mut m = m.borrow_mut();
                let state = m.entry(key.clone()).or_insert_with(|| HoverFade { hovered: false, since: now, from: 0.0, pressed: Default::default(), press: Default::default() });
                if state.hovered != hovered {
                    let from = state.amount(now);
                    *state = HoverFade { hovered, since: now, from, pressed: state.pressed.clone(), press: state.press.clone() };
                }
                state.follow_press(now);
                (state.amount(now), state.pressed.clone(), state.press(now))
            });
            if !hovered && pressed.get() {
                pressed.set(false);
            }
            let pressing = if pressed.get() { press < 1.0 } else { press > 0.0 };
            if (hovered && amount < 1.0) || (!hovered && amount > 0.0) || pressing {
                window.request_animation_frame();
            }
            if hover_log() {
                eprintln!("navia: hover {key:?} {:.1} ms amount {amount:.4} press {press:.4}", crate::testing::since_start_ms());
            }
            track_pointer(window, &hitbox, hovered, pressed);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Redraws the current view when the pointer crosses `hitbox` or presses and releases on it.
fn track_pointer(window: &mut gpui::Window, hitbox: &gpui::Hitbox, hovered: bool, pressed: std::rc::Rc<std::cell::Cell<bool>>) {
    let view = window.current_view();
    let moved = hitbox.clone();
    window.on_mouse_event(move |_: &gpui::MouseMoveEvent, phase, window, cx| {
        if phase == gpui::DispatchPhase::Capture && moved.is_hovered(window) != hovered {
            cx.notify(view);
        }
    });
    let down = pressed.clone();
    let downbox = hitbox.clone();
    window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
        if phase == gpui::DispatchPhase::Capture && event.button == gpui::MouseButton::Left && downbox.is_hovered(window) {
            down.set(true);
            cx.notify(view);
        }
    });
    window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture && pressed.get() {
            pressed.set(false);
            cx.notify(view);
        }
    });
}

/// Where a button's hover fill sits inside its bounds.
#[derive(Clone, Copy)]
pub enum HoverShape {
    /// The whole control with fully rounded ends, like a pill segment or round button.
    Pill,
    /// A circle of this diameter centered on the control, for row icons wider than their glyph.
    Circle(gpui::Pixels),
    /// The whole control with this corner radius, like a small square button.
    Rounded(gpui::Pixels),
}

#[derive(Clone)]
struct HoverFade {
    hovered: bool,
    since: std::time::Instant,
    from: f32,
    pressed: std::rc::Rc<std::cell::Cell<bool>>,
    /// The press's own fade as (down, since, from), restarted when the button goes down or up.
    press: std::rc::Rc<std::cell::Cell<(bool, Option<std::time::Instant>, f32)>>,
}

/// One step of a transition from `from` toward `to` over `secs`, `dt` seconds in. As in CSS, a
/// change partway starts a new full-length transition from the current value.
fn hover_step(from: f32, to: f32, secs: f32, dt: f32) -> f32 {
    from + (to - from) * motion::hover_ease(motion::progress(dt, secs))
}

impl HoverFade {
    /// A quick fade in and a gentler fade out.
    fn amount(&self, now: std::time::Instant) -> f32 {
        let dt = now.saturating_duration_since(self.since).as_secs_f32();
        if self.hovered { hover_step(self.from, 1.0, motion::HOVER_IN, dt) } else { hover_step(self.from, 0.0, motion::HOVER_OUT, dt) }
    }

    /// The press, going down over the press time and back up over the fade-in time.
    fn press(&self, now: std::time::Instant) -> f32 {
        let (down, since, from) = self.press.get();
        let Some(since) = since else { return if down { 1.0 } else { 0.0 } };
        let dt = now.saturating_duration_since(since).as_secs_f32();
        if down { hover_step(from, 1.0, motion::HOVER_PRESS, dt) } else { hover_step(from, 0.0, motion::HOVER_IN, dt) }
    }

    /// Starts the press's fade when the button has gone down or come up since the last frame.
    fn follow_press(&self, now: std::time::Instant) {
        let down = self.pressed.get();
        let (was, _, _) = self.press.get();
        if down != was {
            let from = self.press(now);
            self.press.set((down, Some(now), from));
        }
    }
}

/// With `NAVIA_HOVER_LOG=1`, logs every frame of a small control's hover and press.
fn hover_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NAVIA_HOVER_LOG").is_some())
}

/// The hover fill for a button. Add it as the first child so the glyph paints over it, and only
/// while the button is enabled and not selected. It fades on its own and shows `press` while held.
/// The fade lives in element state, so the button needs an id.
pub fn hover_fill(shape: HoverShape, hover: gpui::Rgba, press: gpui::Rgba) -> impl IntoElement {
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |bounds, hitbox, window, _| {
            let hovered = hitbox.is_hovered(window);
            let now = std::time::Instant::now();
            let (amount, pressed) = window.with_global_id("hover-fill".into(), |id, window| {
                window.with_element_state::<HoverFade, _>(id, |state, _| {
                    let mut state = state.unwrap_or(HoverFade { hovered: false, since: now, from: 0.0, pressed: Default::default(), press: Default::default() });
                    if state.hovered != hovered {
                        let from = state.amount(now);
                        state = HoverFade { hovered, since: now, from, pressed: state.pressed.clone(), press: state.press.clone() };
                    }
                    let amount = state.amount(now);
                    ((amount, state.pressed.clone()), state)
                })
            });
            if !hovered {
                pressed.set(false);
            }
            if amount > 0.0 {
                let color = if pressed.get() { press } else { gpui::Rgba { a: hover.a * amount, ..hover } };
                let (rect, radius) = match shape {
                    HoverShape::Pill => (bounds, bounds.size.height.min(bounds.size.width) / 2.0),
                    HoverShape::Circle(d) => (gpui::Bounds::centered_at(bounds.center(), gpui::size(d, d)), d / 2.0),
                    HoverShape::Rounded(r) => (bounds, r),
                };
                window.paint_quad(gpui::fill(rect, color).corner_radii(radius));
            }
            if (hovered && amount < 1.0) || (!hovered && amount > 0.0) {
                window.request_animation_frame();
            }
            track_pointer(window, &hitbox, hovered, pressed);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Art fading in over its placeholder, or just the art once fully opaque.
pub fn fade_in(art: gpui::Img, placeholder: impl IntoElement, opacity: f32) -> AnyElement {
    if opacity >= 1.0 {
        return art.into_any_element();
    }
    div().relative().flex_none().child(placeholder).child(art.absolute().left(px(0.0)).top(px(0.0)).opacity(opacity)).into_any_element()
}

pub const STATION_NO_STAR: &str = n_("Stations can't be favorited");
pub const STATION_NO_SKIP: &str = n_("Stations can't be skipped");
pub const STATION_NO_MODES: &str = n_("Stations can't be shuffled or repeated");

/// A control that can't be used right now, dimmed and inert. Don't set a hover style before
/// this, since GPUI panics if hover is set twice.
pub fn locked(el: Stateful<Div>) -> Stateful<Div> {
    el.opacity(0.35)
}

pub fn clock(seconds: u32) -> SharedString {
    SharedString::from(format!("{}:{:02}", seconds / 60, seconds % 60))
}

/// The mark and wordmark side by side, `mark` tall. Art from tools/brand_icons.py.
pub fn brand_lockup(t: &Theme, mark: f32, color: impl Into<Hsla>) -> Div {
    let color = color.into();
    let word = mark * BRAND_WORD_RATIO;
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .gap(t.s(mark * BRAND_GAP_RATIO))
        .child(svg().path(icon_path("nv-brand-mark.svg")).flex_none().size(t.s(mark)).text_color(color))
        .child(svg().path(icon_path("nv-brand-word.svg")).flex_none().h(t.s(word)).w(t.s(word * crate::assets::BRAND_WORD_ASPECT)).text_color(color))
}

/// A glass card on its own backdrop, as on the sign-in form.
pub fn glass_card(t: &Theme) -> Div {
    let radius = t.s(R_LG);
    glass_layers(div().relative().rounded(radius), t, radius, FLOAT_BLUR, FLOAT_SATURATION, t.float_glass(), t.float_fill(), edge_and(shadow_float(t)))
}

/// A one-time question inside a panel, like the queue's Deezer card. A title, one fact beside its
/// glyph, then the answers side by side.
pub fn prompt_card(t: &Theme, title: SharedString, glyph: &str, fact: SharedString, answers: Vec<AnyElement>) -> Div {
    glass_card(t).p(t.s(PROMPT_CARD_PAD)).child(
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(t.s(PROMPT_CARD_GAP))
            .child(div().text_size(t.text(TEXT_MD)).font_weight(FontWeight::SEMIBOLD).text_color(t.t_em()).line_height(relative(1.3)).child(title))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(t.s(PROMPT_CARD_GAP))
                    .text_size(t.text(TEXT_SM))
                    .text_color(t.t2())
                    .line_height(relative(1.35))
                    .child(icon(t, glyph, 16.0, t.t2()))
                    .child(div().flex_1().min_w(px(0.0)).child(fact)),
            )
            .child(div().flex().flex_row().gap(t.s(DIALOG_ACTIONS_GAP)).children(answers)),
    )
}

/// A labeled text field, with the label above `field` and an optional message below it.
pub fn text_field_block(t: &Theme, label: impl Into<SharedString>, field: impl IntoElement, message: Option<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(t.s(FIELD_MESSAGE_GAP))
        .child(div().flex().flex_col().gap(t.s(FIELD_LABEL_GAP)).child(div().text_size(t.text(TEXT_SM)).text_color(t.t2()).font_weight(FontWeight::MEDIUM).child(label.into())).child(field))
        .children(message)
}

/// The glass pill of a labeled text field, with the focus ring or the invalid ring. Add the text
/// line, then an optional `field_action` at its right end.
pub fn text_field_box(t: &Theme, id: impl Into<ElementId>, focused: bool, invalid: bool) -> Stateful<Div> {
    let edge = if invalid { crate::ui_controls::with_invalid_ring(glass_edge()) } else if focused { crate::ui_controls::with_ring(glass_edge(), t.accent_hsla()) } else { glass_edge() };
    div().id(id.into()).relative().flex().flex_row().items_center().h(t.s(CONTROL_H)).pl(t.s(FIELD_PAD)).pr(t.s(FIELD_INSET)).rounded_full().bg(t.glass_fill()).shadow(edge).cursor_text()
}

/// An action at a text field's right end, like Show password. Quiet ink that brightens on hover.
pub fn field_action(t: &Theme, id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    let id = id.into();
    let ink = ink(id.clone());
    div()
        .id(id.clone())
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .flex_none()
        .min_w(t.s(ICON_W))
        .h(t.s(CONTROL_H - FIELD_INSET * 2.0))
        .px(t.s(FIELD_PAD / 2.0))
        .rounded_full()
        .text_size(t.text(TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .cursor_default()
        .when(enabled, |d| d.child(hover_track(id)).text_color(ink.color(Hsla::from(t.t2()), t.hover_ink())))
        .when(!enabled, |d| d.text_color(t.t2()).opacity(DISABLED_OPACITY))
}

/// The busy glyph, an arc turning once per `SPIN_PERIOD_MS`, still while motion is reduced.
/// `key` gives each spinner its own animation.
pub fn spinner(t: &Theme, key: impl Into<ElementId>, size: f32, color: impl Into<Hsla>) -> AnyElement {
    use gpui::{Animation, AnimationExt, Transformation, percentage};
    let glyph = icon(t, "circle-notch-regular", size, color);
    if motion::reduced() {
        return glyph.into_any_element();
    }
    glyph
        .with_animation(key, Animation::new(std::time::Duration::from_millis(SPIN_PERIOD_MS)).repeat(), |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))))
        .into_any_element()
}

/// A dialog's small print in the error color.
pub fn dialog_note_error(t: &Theme, text: impl Into<SharedString>) -> Div {
    dialog_note(t, text).text_color(crate::theme::danger_ink())
}

/// A short code typed digit by digit, like an AirPlay PIN. Shows `len` boxes with the next one
/// ringed while `focused` and all ringed in red when `invalid`. The caller handles the keys.
pub fn dialog_pin(t: &Theme, id: &'static str, value: &str, len: usize, focused: bool, invalid: bool, disabled: bool) -> Stateful<Div> {
    let digits: Vec<char> = value.chars().collect();
    let at = digits.len().min(len.saturating_sub(1));
    div().id(id).flex().flex_row().justify_center().gap(t.s(PIN_GAP)).cursor_text().children((0..len).map(|i| {
        let edge = if invalid {
            crate::ui_controls::with_invalid_ring(glass_edge())
        } else if focused && !disabled && i == at {
            crate::ui_controls::with_ring(glass_edge(), t.accent_hsla())
        } else {
            glass_edge()
        };
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(t.s(PIN_BOX_W))
            .h(t.s(PIN_BOX_H))
            .rounded(t.s(R_MD))
            .bg(t.glass_fill())
            .shadow(edge)
            .text_size(t.text(PIN_TEXT))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.t1())
            .when(disabled, |d| d.opacity(DISABLED_OPACITY))
            .child(digits.get(i).map(|c| c.to_string()).unwrap_or_default())
    }))
}

/// A collapsible part of a dialog with a caret and label. The caller adds the content while `open`.
pub fn dialog_disclosure(t: &Theme, id: &'static str, label: impl Into<SharedString>, open: bool) -> Stateful<Div> {
    let go = ink(id);
    div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(t.s(6.0))
        .text_size(t.text(TEXT_SM))
        .text_color(go.color(t.t2(), t.t1()))
        .child(hover_track(id))
        .child(icon(t, if open { "caret-down-bold" } else { "caret-right-bold" }, 12.0, go.color(t.t2(), t.t1())))
        .child(label.into())
}

/// A command to run, in wrapping monospace text on glass, with a `copy` button at its bottom right.
pub fn dialog_code(t: &Theme, code: impl Into<SharedString>, copy: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(t.s(8.0))
        .p(t.s(CODE_PAD))
        .rounded(t.s(R_MD))
        .bg(t.glass_fill())
        .shadow(glass_edge())
        .child(div().w_full().font_family(crate::assets::font_mono()).text_size(t.text(TEXT_XS)).line_height(relative(1.5)).text_color(t.t1()).child(code.into()))
        .child(copy)
}

/// The filter chip, `CHIP_H` tall, glass at rest and filled when on. Add the label and glyphs as
/// children. It's the whole chip unless there's an X ([`filter_chip_remove`]).
pub fn filter_chip(t: &Theme, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let chip = div()
        .id(id)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .gap(t.s(CHIP_GAP))
        .h(t.s(CHIP_H))
        .px(t.s(CHIP_PAD))
        .rounded_full()
        .text_size(t.text(TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap();
    if on {
        chip.bg(t.chip_on()).text_color(t.on_chip())
    } else {
        chip.bg(t.glass_fill()).shadow(glass_edge()).text_color(t.t1()).child(hover_fill(HoverShape::Pill, t.glass_hover(), t.glass_press()))
    }
}

/// A chip's glyph, such as its caret, the More Filters plus or the X.
pub fn chip_glyph(t: &Theme, name: &str, on: bool) -> Svg {
    icon(t, name, CHIP_GLYPH, if on { Hsla::from(t.on_chip()) } else { Hsla::from(t.t1()) })
}

/// An active chip's X, a separate button at the chip's end. Put [`filter_chip`] with
/// `pr(CHIP_GAP)` and this in [`filter_chip_pair`].
pub fn filter_chip_remove(t: &Theme, id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .h_full()
        .pl(t.s(CHIP_GAP))
        .pr(t.s(CHIP_PAD - CHIP_GAP))
        .rounded_full()
        .opacity(0.7)
        .hover(|s| s.opacity(1.0))
        .child(chip_glyph(t, "x-bold", true))
}

/// An active chip with its X, the on fill spanning both buttons.
pub fn filter_chip_pair(t: &Theme, main: Stateful<Div>, remove: Stateful<Div>) -> Div {
    div().flex().flex_row().items_center().flex_none().h(t.s(CHIP_H)).rounded_full().bg(t.chip_on()).text_color(t.on_chip()).child(main.pr(t.s(CHIP_GAP))).child(remove)
}

/// The width of `text` in the app's font as a line of it draws. [`fold_chips`] needs it before layout.
pub fn text_width(window: &mut gpui::Window, text: &str, size: gpui::Pixels, weight: FontWeight) -> gpui::Pixels {
    let mut font = window.text_style().font();
    font.weight = weight;
    let run = gpui::TextRun { len: text.len(), font, color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(SharedString::from(text.to_string()), size, &[run], None).width
}

/// How wide [`filter_chip`] draws a chip, with the caret when it opens a popover.
pub fn filter_chip_width(t: &Theme, window: &mut gpui::Window, label: &str, menu: bool) -> gpui::Pixels {
    let glyph = if menu { t.s(CHIP_GAP + CHIP_GLYPH) } else { px(0.0) };
    t.s(CHIP_PAD * 2.0) + text_width(window, label, t.text(TEXT_SM), FontWeight::MEDIUM) + glyph
}

/// How wide [`filter_chip_pair`] draws an active chip, with its name, value and X.
pub fn filter_chip_pair_width(t: &Theme, window: &mut gpui::Window, name: &str, value: &str) -> gpui::Pixels {
    let size = t.text(TEXT_SM);
    let value = text_width(window, value, size, FontWeight::MEDIUM).min(t.s(CHIP_VALUE_MAX_W));
    t.s(CHIP_PAD * 2.0 + CHIP_GAP * 2.0 + CHIP_GLYPH) + text_width(window, name, size, FontWeight::MEDIUM) + value
}

/// An active chip's value, truncated with an ellipsis past `CHIP_VALUE_MAX_W`.
pub fn chip_value(t: &Theme, value: impl Into<SharedString>) -> Div {
    div().max_w(t.s(CHIP_VALUE_MAX_W)).truncate().child(value.into())
}

/// Which chips of a one-line row show, so the row never wraps or overflows. Kept chips show first,
/// then the others in order until one doesn't fit, and the rest fold into More. The end group
/// (`end_w`) sits at the right and moves into More's menu when there's no room for it. `pin`
/// always shows. `rest` means filters only More can add, so More shows even when all chips fit.
pub struct ChipFold {
    pub shown: Vec<bool>,
    pub more: bool,
    pub end_in_more: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn fold_chips(widths: &[f32], keep: &[bool], pin: Option<usize>, more_w: f32, end_w: f32, gap: f32, room: f32, rest: bool) -> ChipFold {
    let end_room = if end_w > 0.0 { gap + end_w } else { 0.0 };
    let all = widths.iter().map(|w| w + gap).sum::<f32>() - if widths.is_empty() { 0.0 } else { gap };
    if !rest && all + end_room <= room {
        return ChipFold { shown: vec![true; widths.len()], more: false, end_in_more: false };
    }
    let must = |i: usize| keep[i] || pin == Some(i);
    let kept: f32 = widths.iter().enumerate().filter(|(i, _)| must(*i)).map(|(_, w)| w + gap).sum();
    let budget = room - end_room - more_w;
    if kept <= budget {
        let mut used = kept;
        let mut folding = false;
        let shown = widths
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if must(i) {
                    return true;
                }
                if !folding && used + w + gap <= budget {
                    used += w + gap;
                    return true;
                }
                folding = true;
                false
            })
            .collect();
        return ChipFold { shown, more: true, end_in_more: false };
    }
    // No room for the end group, so it goes into More and the kept chips show while they fit.
    let mut used = pin.map_or(0.0, |p| widths[p] + gap);
    let mut folding = false;
    let shown = widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if pin == Some(i) {
                return true;
            }
            if keep[i] && !folding && used + w + gap <= room - more_w {
                used += w + gap;
                return true;
            }
            if keep[i] {
                folding = true;
            }
            false
        })
        .collect();
    ChipFold { shown, more: true, end_in_more: end_w > 0.0 }
}

/// A typed-number field, like Year or BPM. Shows the focus ring while typing, or the invalid ring
/// when its value is the problem and it isn't focused.
pub fn number_field(t: &Theme, id: impl Into<ElementId>, focused: bool, bad: bool, line: impl IntoElement) -> Stateful<Div> {
    let ring = if focused { Some((Hsla::from(t.accent_hsla()), px(2.0))) } else if bad { Some((Hsla::from(gpui::rgba(crate::theme::INVALID_RING)), px(crate::theme::INVALID_RING_W))) } else { None };
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_none()
        .w(t.s(NUMBER_FIELD_W))
        .h(t.s(CHIP_H))
        .px(t.s(CHIP_PAD))
        .rounded_full()
        .bg(t.row())
        .text_size(t.text(TEXT_SM))
        .relative()
        .cursor_text()
        .child(line)
        .when_some(ring, |d, (c, w)| d.child(outline_ring(t.s(CHIP_H / 2.0), c, w)))
}

#[cfg(test)]
mod tests {
    use super::fold_chips;

    #[test]
    fn chips_fold_from_the_first_that_does_not_fit_and_keep_active_ones() {
        // Everything fits and there's nothing else to add, so no More.
        let f = fold_chips(&[50.0, 50.0], &[false, false], None, 40.0, 0.0, 5.0, 200.0, false);
        assert_eq!((f.shown, f.more, f.end_in_more), (vec![true, true], false, false));
        // Room for two plus More. The third and later chips fold, even one that would fit.
        let f = fold_chips(&[50.0, 50.0, 80.0, 10.0], &[false; 4], None, 40.0, 0.0, 5.0, 160.0, false);
        assert_eq!((f.shown, f.more), (vec![true, true, false, false], true));
        // An active chip stays, and the end group takes its share.
        let f = fold_chips(&[50.0, 50.0, 80.0], &[false, false, true], None, 40.0, 60.0, 5.0, 200.0, false);
        assert_eq!((f.shown, f.more, f.end_in_more), (vec![false, false, true], true, false));
        // Filters only More can add keep it on the line.
        let f = fold_chips(&[50.0], &[false], None, 40.0, 0.0, 5.0, 500.0, true);
        assert_eq!((f.shown, f.more), (vec![true], true));
    }

    #[test]
    fn no_room_moves_the_end_group_into_more_and_folds_active_chips_in_order() {
        // Three active chips leave no room for Clear All, so it goes into More and the third one folds.
        let f = fold_chips(&[60.0, 60.0, 60.0], &[true, true, true], None, 40.0, 80.0, 5.0, 180.0, false);
        assert_eq!((f.shown, f.more, f.end_in_more), (vec![true, true, false], true, true));
        // The one being opened stays, ahead of the others.
        let f = fold_chips(&[60.0, 60.0, 60.0], &[true, true, true], Some(2), 40.0, 80.0, 5.0, 180.0, false);
        assert_eq!(f.shown, vec![true, false, true]);
    }
}

/// Which corner a [`cover_label`] sits in. Bottom-left, or top-left when the bottom has a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelAt {
    Top,
    Bottom,
}

/// The quiet label chip on art the app made, with white text over a scrim.
pub fn cover_label(t: &Theme, label: &'static str, at: LabelAt) -> Div {
    let chip = div().absolute().left(t.s(COVER_LABEL_INSET));
    let chip = match at {
        LabelAt::Top => chip.top(t.s(COVER_LABEL_INSET)),
        LabelAt::Bottom => chip.bottom(t.s(COVER_LABEL_INSET)),
    };
    chip.px(t.s(COVER_LABEL_PAD_X))
        .py(t.s(COVER_LABEL_PAD_Y))
        .rounded_full()
        .bg(t.scrim())
        .shadow(glass_edge())
        .text_size(t.text(TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .line_height(relative(1.3))
        .text_color(t.t_on_art())
        .child(label)
}

/// The background for a tile with light text on a tint, like a genre tile. The `tile_fill` ramp
/// along `TILE_FILL_ANGLE`, or a neutral panel while the tint is unknown.
pub fn tile_fill_bg(t: &Theme, tint: Option<crate::color::Srgb>) -> gpui::Background {
    match tint {
        Some(tint) => {
            let (top, bottom) = crate::color::tile_fill(tint);
            gpui::linear_gradient(TILE_FILL_ANGLE, gpui::linear_color_stop(top.hsla(1.0), 0.0), gpui::linear_color_stop(bottom.hsla(1.0), 1.0))
        }
        None => t.panel_2().into(),
    }
}

/// How wide one line of the app's font draws, from glyph advances, for layout outside paint.
/// Kerning isn't counted, so it errs slightly wide.
pub fn text_width_app(cx: &gpui::App, text: &str, size: gpui::Pixels, weight: FontWeight) -> gpui::Pixels {
    let mut font = gpui::font(crate::assets::FONT_FAMILY);
    font.weight = weight;
    let system = cx.text_system();
    let id = system.resolve_font(&font);
    text.chars().map(|c| system.advance(id, size, c).map(|s| s.width).unwrap_or(px(0.0))).fold(px(0.0), |a, b| a + b)
}
