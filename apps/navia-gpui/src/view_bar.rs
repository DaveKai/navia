//! The floating bottom bar.

use crate::i18n::tr;
use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, Context, FontWeight, MouseButton, div, point, px};

use crate::persist::{DockView, PlayerMode};
use crate::shell::{Layout, Shell};
use crate::theme::size::*;

/// Bar measurements in tokens: side padding, gaps around the play controls, right-group spacers, and the play glyphs
/// and their gap.
const PAD_L: f32 = 12.0;
const PAD_R: f32 = 14.0;
const GAP: f32 = 18.0;
const SPACER: f32 = 16.0;
const SKIP: f32 = 23.0;
const PLAY: f32 = 32.0;
const TRANSPORT_GAP: f32 = 6.0;
/// Shuffle and repeat glyph size. Their box is 12 wider.
const MODE: f32 = 20.0;
const MODE_W: f32 = MODE + 12.0;

/// What a bar `width` tokens wide shows. The play controls stay centered with equal sides, so the right group must fit
/// in half the remaining width. Extras drop in order: Output and More, then heart and volume, then shuffle and repeat,
/// then the whole right group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarFit {
    pub extras: bool,
    pub optional: bool,
    pub modes: bool,
    pub side: bool,
}

pub fn bar_fit(width: f32, can_dock: bool) -> BarFit {
    let dock = if can_dock { 1.0 } else { 0.0 };
    // Lyrics, Queue, Mini player, Expand, and Dock when the window can dock.
    let base = (4.0 + dock) * ICON_W;
    let right = |extras: bool, optional: bool| {
        let optional_w = if optional { 2.0 * ICON_W + 2.0 * SPACER } else { 0.0 };
        let extras_w = if extras { 2.0 * ICON_W } else { 0.0 };
        base + optional_w + extras_w + PAD_R
    };
    let transport = |modes: bool| SKIP + PLAY + SKIP + 2.0 * TRANSPORT_GAP + if modes { 2.0 * (MODE_W + TRANSPORT_GAP) } else { 0.0 };
    let fits = |right_w: f32, modes: bool| right_w <= (width - transport(modes) - 2.0 * GAP) / 2.0;
    let modes = width > 600.0 && fits(right(false, false), true);
    BarFit {
        extras: width > 880.0 && fits(right(true, true), modes),
        optional: width > 760.0 && fits(right(false, true), modes),
        modes,
        side: width > 430.0 && fits(right(false, false), modes),
    }
}

impl Shell {
    pub fn render_bar(&mut self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let Some(song) = self.current_song().cloned() else { return div().into_any_element() };
        let width_tokens = (f32::from(layout.main_width) / t.token_scale() - 28.0).clamp(240.0, BAR_W);
        let width = t.s(width_tokens);
        // Narrowing drops extras in order. The expanded player keeps all of them.
        let can_dock = self.can_dock(layout);
        let fit = bar_fit(width_tokens, can_dock);
        // `NAVIA_BAR_LOG=1` logs the play controls' center against the bar's.
        let measure = std::env::var_os("NAVIA_BAR_LOG").is_some().then(|| (self.bounds_store("bar-measure"), self.bounds_store("bar-measure-transport")));
        if let Some((bar, transport)) = &measure {
            let (bar, transport) = (*bar.borrow(), *transport.borrow());
            if bar.size.width > px(0.0) {
                eprintln!("navia: bar {:.1} wide, transport centre {:+.2} from the bar's, {fit:?}", f32::from(bar.size.width), f32::from(transport.center().x - bar.center().x));
            }
        }
        let (show_extras, show_optional, show_modes, show_side) = (fit.extras, fit.optional, fit.modes, fit.side);
        let art_px = (f32::from(t.s(38.0)) * layout.scale).round() as u32;
        let art = self.thumb_faded(&song.album_id, art_px);
        let art_el: AnyElement = match art {
            Some((image, fade)) => crate::ui::fade_in(crate::ui::image(image).size(t.s(38.0)).rounded(t.s(R_SM)), div().size(t.s(38.0)).rounded(t.s(R_SM)).bg(t.on_art_placeholder()), fade),
            None => div().size(t.s(38.0)).rounded(t.s(R_SM)).bg(t.on_art_placeholder()).into_any_element(),
        };
        let playing = self.player.playing;
        // The bar always uses its neutral colors, never the cover tint.
        let fg: gpui::Hsla = t.bar_ink().into();
        let dim = fg;
        let title_color: gpui::Hsla = t.bar_title().into();
        let plain_button = |id: &'static str, name: &'static str, size: f32| crate::ui::ink_button(&t, id, name, size, t.bar_ink().into(), t.hover_ink());
        let transport_button = |id: &'static str, name: &'static str, size: f32, enabled: bool| crate::ui::transport_button(&t, id, name, size, fg, enabled);
        let starred = self.player.song_ix.map(|i| self.is_starred(i as u32)).unwrap_or(false);
        // The AirPlay glyph while a receiver is the output, in the accent once it plays.
        let (output_glyph, output_accent) = self.output_glyph();
        let output_button = if output_accent { crate::ui::ink_button(&t, "bar-output", output_glyph, 20.0, t.accent_hsla(), t.accent_hsla()) } else { plain_button("bar-output", output_glyph, 20.0) };
        let star_button = |name: &'static str, enabled: bool| {
            crate::ui::ink_button(&t, "bar-star", name, 20.0, t.bar_ink().into(), if enabled { t.hover_ink() } else { t.bar_ink().into() })
        };
        let station = self.player.station.is_some();
        let mode_color = fg;
        let shuffle = show_modes.then(|| self.render_mode_button(crate::view_dock::ModeButton::Shuffle, "bar-shuffle", MODE, mode_color, cx));
        let repeat = show_modes.then(|| self.render_mode_button(crate::view_dock::ModeButton::Repeat, "bar-repeat", MODE, mode_color, cx));
        let volume_name = self.volume_icon();
        let volume = show_optional.then(|| self.render_bar_volume(volume_name, cx));
        // The right half, as wide as the left, with its buttons at the end.
        let mut side = div().flex().flex_row().items_center().justify_end().w_full().min_w(px(0.0)).pr(t.s(PAD_R));
        if show_optional {
            side = side
                .when(show_extras, |d| d.child(plain_button("bar-more", "dots-three-bold", 20.0).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| this.open_player_menu(event.position() - point(px(0.0), this.theme.s(320.0)), true, cx)))))
                .child(star_button(if starred { "heart-fill" } else { "heart-regular" }, !station).when(!station, |d| d.stop(&t).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(song) = this.player.song_ix {
                        this.toggle_song_star(song as u32, cx);
                    }
                }))).when(station, |d| crate::ui::locked(d).on_click(cx.listener(|this, _, _, cx| this.show_toast(tr(crate::ui::STATION_NO_STAR), cx)))))
                .child(div().w(t.s(SPACER)).flex_none())
                .when(show_extras, |d| d.child(output_button.stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| this.open_output_menu(event.position() - point(px(0.0), this.theme.s(70.0)), cx)))))
                .children(volume)
                .child(div().w(t.s(SPACER)).flex_none());
        }
        if show_side {
            side = side
                .child(plain_button("bar-lyrics", "nv-lyrics", 20.0).stop(&t).on_click(cx.listener(move |this, _, _, cx| this.open_side_view(DockView::Lyrics, can_dock, cx))))
                // Songs dropped on the queue button go to the end of Next in Queue.
                .child(div().relative().children(self.drop_mark("bar-queue", crate::song_drop::SongTarget::QueueEnd, R_MD, 0.0)).child(plain_button("bar-queue", "list-bullets-regular", 20.0).stop(&t).on_click(cx.listener(move |this, _, _, cx| this.open_side_view(DockView::Queue, can_dock, cx)))))
                .child(plain_button("bar-mini", "picture-in-picture-regular", 20.0).stop(&t).on_click(cx.listener(|this, _, _, cx| this.open_mini_player(cx))))
                .child(plain_button("bar-expand", "nv-expand", 20.0).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                    this.expanded = true;
                    cx.notify();
                })));
            if can_dock {
                side = side.child(plain_button("bar-dock", "nv-dock", 20.0).stop(&t).on_click(cx.listener(|this, _, _, cx| this.set_mode(PlayerMode::Dock, cx))));
            }
        }
        // The two sides of the play controls, always equal in width.
        let half = || div().flex().flex_row().items_center().flex_1().flex_basis(px(0.0)).min_w(px(0.0));
        // The title links to its album and each artist to their page.
        let station = self.player.station.is_some();
        let (bar_title, bar_artists): (AnyElement, AnyElement) = if station {
            (div().font_weight(FontWeight::MEDIUM).flex_shrink(1.0).min_w(px(0.0)).truncate().text_color(title_color).child(song.title.clone()).into_any_element(), div().truncate().child(song.artist.clone()).into_any_element())
        } else {
            let target = (!song.album_id.is_empty()).then(|| crate::song_links::LinkTarget::Album(song.album_id.clone()));
            let title = self.link(("bar-title", 0), song.title.clone(), target, title_color, cx);
            let title = crate::ui::titled(&t, div().font_weight(FontWeight::MEDIUM).flex().min_w(px(0.0)).overflow_hidden().child(title), song.explicit, TEXT_MD).flex_shrink(1.0).into_any_element();
            (title, self.artist_links(("bar-artist", 0), &song, dim, cx))
        };
        let title_block = div()
            .id("bar-title-block")
            // Right-click on the song opens the same menu as More.
            .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| this.open_player_menu(event.position, false, cx)))
            .flex()
            .flex_row()
            .items_baseline()
            .gap(t.s(5.0))
            .mb(px(1.0))
            .child(bar_title)
            .child(div().text_color(dim).flex().flex_row().flex_grow(1.0).flex_shrink(1.0).min_w(px(0.0)).overflow_hidden().child(div().flex_none().mr(t.s(5.0)).child("–")).child(bar_artists));
        let badge_narrow = width_tokens < OUTPUT_BADGE_NARROW;
        let title_row = div().flex().flex_row().items_center().gap(t.s(8.0)).w_full().min_w(px(0.0)).text_color(dim).child(title_block.flex_1().min_w(px(0.0))).when_some(self.private_badge(), |d, badge| d.child(badge)).when_some(self.output_badge(true, badge_narrow, dim), |d, badge| d.child(badge));

        div()
            .id("player-bar")
            .absolute()
            .left(px((f32::from(layout.main_width) - f32::from(width)) / 2.0))
            .bottom(t.s(WINDOW_INSET))
            .w(width)
            .h(t.s(BAR_H))
            .flex()
            .flex_row()
            .items_center()
            .rounded(t.s(R_LG))
            .map(|d| crate::ui::glass_layers(d, &t, t.s(R_LG), BAR_BLUR, BAR_SATURATION, t.bar_frost_fill(), t.bar_fill(), crate::ui::edge_and(crate::ui::shadow_float(&t))))
            .occlude()
            .children(self.drop_mark("bar", crate::song_drop::SongTarget::Surface, R_LG, 0.0))
            .when_some(measure.as_ref().map(|m| m.0.clone()), |d, store| d.child(crate::ui_controls::track_bounds(store)))
            // The left half: art, title and progress, which shrink to fit. Padding stays inside each half so both stay
            // equal.
            .child(half().child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(t.s(GAP))
                    .w_full()
                    .min_w(px(0.0))
                    .pl(t.s(PAD_L))
                    .child(
                        div()
                            .id("bar-art")
                            .flex_none()
                            .stop(&t).on_click(cx.listener(|this, _, _, cx| {
                                this.expanded = true;
                                cx.notify();
                            }))
                            .child(art_el),
                    )
                    .child(div().flex_1().min_w(px(0.0)).flex().flex_col().child(title_row).child(self.render_scrubber(crate::view_dock::ScrubVariant::Inline, t.bar_track(), fg, dim, cx))),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_none()
                    .gap(t.s(TRANSPORT_GAP))
                    // The play controls fill the bar's height and carry the player chip above them.
                    .relative()
                    .h_full()
                    .when_some(self.render_player_chip(true, dim, fg, cx), |d, chip| d.child(chip))
                    .when_some(measure.as_ref().map(|m| m.1.clone()), |d, store| d.child(crate::ui_controls::track_bounds(store)))
                    // Exactly centered, since the halves are equal.
                    .mx(t.s(GAP))
                    .when_some(shuffle, |d, b| d.child(b))
                    .child(transport_button("bar-prev", "rewind-fill", SKIP, !station).when(!station, |d| d.stop(&t).on_click(cx.listener(|this, _, _, cx| this.previous(cx)))).when(station, |d| crate::ui::locked(d).on_click(cx.listener(|this, _, _, cx| this.show_toast(tr(crate::ui::STATION_NO_SKIP), cx)))))
                    .child(transport_button("bar-play", if playing { "pause-fill" } else { "play-fill" }, PLAY, true).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_play(cx);
                    })))
                    .child(transport_button("bar-next", "fast-forward-fill", SKIP, !station).when(!station, |d| d.stop(&t).on_click(cx.listener(|this, _, _, cx| this.next(cx)))).when(station, |d| crate::ui::locked(d).on_click(cx.listener(|this, _, _, cx| this.show_toast(tr(crate::ui::STATION_NO_SKIP), cx)))))
                    .when_some(repeat, |d, b| d.child(b)),
            )
            .child(half().child(side))
            .into_any_element()
    }

    /// The speaker button. Hovering it or its panel shows the slider, and clicking mutes.
    fn render_bar_volume(&mut self, name: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let panel_w = t.s(170.0);
        let open = self.bar_volume.open;
        let speaker = crate::ui::ink_button(&t, "bar-volume", name, 20.0, t.bar_ink().into(), t.hover_ink())
            .stop(&t).on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx)))
            .on_hover(cx.listener(|this, on: &bool, _, cx| this.bar_volume_hover(false, *on, cx)));
        let mut wrap = div().relative().flex_none().child(speaker);
        if open {
            let slider = self.render_volume_slider(t.bar_ink().into(), t.bar_track(), cx);
            wrap = wrap.child(
                crate::ui::float_panel(&t)
                    .id("bar-volume-panel")
                    .absolute()
                    .bottom(t.s(34.0 + 14.0))
                    .left(px((f32::from(t.s(ICON_W)) - f32::from(panel_w)) / 2.0))
                    .w(panel_w)
                    .px(t.s(16.0))
                    .py(t.s(12.0))
                    .flex()
                    .items_center()
                    .on_hover(cx.listener(|this, on: &bool, _, cx| this.bar_volume_hover(true, *on, cx)))
                    .child(slider),
            );
        }
        wrap.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_right_group_always_fits_its_half() {
        for can_dock in [false, true] {
            let mut last = None;
            for width in (240..=BAR_W as i32).step_by(10) {
                let fit = bar_fit(width as f32, can_dock);
                // Extras only come back as the bar widens, in order.
                assert!(!fit.extras || fit.optional);
                assert!(!fit.optional || fit.side);
                if let Some(prev) = last {
                    let prev: BarFit = prev;
                    assert!(!prev.extras || fit.extras, "{width}");
                    assert!(!prev.side || fit.side, "{width}");
                }
                last = Some(fit);
            }
        }
        // The full width (`BAR_W`, 1010) fits the whole right group and the dock button, but 930 doesn't.
        let all = BarFit { extras: true, optional: true, modes: true, side: true };
        assert_eq!(bar_fit(BAR_W, true), all);
        assert_eq!(bar_fit(930.0, true), BarFit { extras: false, ..all });
        assert_eq!(bar_fit(930.0, false), all);
    }

    /// The widths where each group drops. Keep in step with `bar_fit`.
    #[test]
    fn steps_match_the_reference() {
        let step = |width: f32, dock: bool, pick: fn(BarFit) -> bool| (pick(bar_fit(width, dock)), pick(bar_fit(width + 1.0, dock)));
        for (width, dock, pick) in [
            (901.0, false, (|f: BarFit| f.extras) as fn(BarFit) -> bool),
            (977.0, true, |f| f.extras),
            (760.0, false, |f| f.optional),
            (825.0, true, |f| f.optional),
            (600.0, false, |f| f.modes),
            (609.0, true, |f| f.modes),
            (457.0, false, |f| f.side),
            (533.0, true, |f| f.side),
        ] {
            assert_eq!(step(width, dock, pick), (false, true), "{width} dock={dock}");
        }
    }
}
