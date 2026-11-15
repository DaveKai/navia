//! The expanded player. It fills the whole window, sidebar included, over a baked backdrop.

use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, BoxShadow, Context, div, point, px};

use crate::color::{glow_scrim_alpha, player_fg, player_fg_dim};
use crate::persist::DockView;
use crate::shell::{Layout, Shell};
use crate::theme::size::*;
use crate::view_window_buttons::Side;

impl Shell {
    pub fn render_expanded(&mut self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let Some(song) = self.current_song().cloned() else { return div().into_any_element() };
        let tint = self.tint();
        let fg: gpui::Hsla = player_fg(tint).hsla(1.0);
        let dim: gpui::Hsla = player_fg_dim(tint).hsla(1.0);
        // The text scrim is as dark as the brightest baked pixel under the controls needs. It uses last frame's bounds,
        // and a change redraws.
        let [root_b, controls_b] = *self.expanded_controls.borrow();
        let scrim = self.expanded_peaks.as_ref().filter(|_| backdrop_ready(&self.expanded_bg, &song.album_id)).map_or(0.0, |grid| {
            let s = layout.scale;
            let (x0, y0) = (f32::from(controls_b.origin.x - root_b.origin.x) * s, f32::from(controls_b.origin.y - root_b.origin.y) * s);
            glow_scrim_alpha(grid.max_in(x0, y0, x0 + f32::from(controls_b.size.width) * s, y0 + f32::from(controls_b.size.height) * s), tint)
        });
        let backdrop = self.expanded_bg.clone().filter(|(album, _, _, _)| *album == song.album_id).map(|(_, _, _, image)| image);
        // CSS vw units inside the page zoom, as a fraction of the real window width in tokens.
        let vw = f32::from(layout.viewport.width);
        let side_pad = (vw * 0.0825).clamp(36.0, 132.0);
        let gap = (vw * 0.12).clamp(36.0, 193.0);
        let view = self.shown_view();
        let solo = view == DockView::Artwork;

        let col_w = if solo { 420.0 } else { 412.0 };
        let art_px = (f32::from(t.s(420.0)) * layout.scale).round() as u32;
        let art = self.thumb_faded(&song.album_id, art_px);
        // Square by its own size. GPUI's img gives an auto height the image's pixel height, which overrides
        // `aspect_ratio` and makes the art tall at display scales above 1.
        let art_el: AnyElement = match art {
            Some((image, fade)) => crate::ui::image(image)
                .size(t.s(col_w))
                .rounded(t.s(R_LG))
                .opacity(fade)
                .shadow(vec![crate::ui::shadow_player_art(&t)])
                .into_any_element(),
            None => div().size(t.s(col_w)).rounded(t.s(R_LG)).bg(t.on_art_placeholder()).into_any_element(),
        };
        let player_col = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(t.s(col_w))
            .text_color(fg)
            .child(art_el)
            .child(
                div()
                    .relative()
                    .child(bounds_slot(self.expanded_controls.clone(), 1))
                    // A blurred black box past the controls at the sides and below.
                    .when(scrim > 0.0, |d| {
                        d.child(div().absolute().top(px(0.0)).bottom(t.s(-GLOW_SCRIM_REACH)).left(t.s(-GLOW_SCRIM_REACH)).right(t.s(-GLOW_SCRIM_REACH)).rounded(t.s(GLOW_SCRIM_REACH)).shadow(vec![BoxShadow {
                            color: gpui::black().opacity(scrim),
                            offset: point(px(0.0), px(0.0)),
                            // A shadow's blur radius is about twice CSS blur()'s deviation.
                            blur_radius: t.s(GLOW_SCRIM_BLUR * 2.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }]))
                    })
                    .child(div().pt(t.s(22.0)).when_some(self.render_player_chip(false, dim, fg, cx), |d, offer| d.child(offer)).child(self.render_meta_row(&song, fg, dim, cx)))
                    .child(self.render_scrubber(crate::view_dock::ScrubVariant::Stacked, t.player_track(), fg, dim, cx))
                    .child(self.render_transport(fg, cx))
                    .child(self.render_volume_row(dim, fg, cx)),
            );

        // Wide windows put the player left and lyrics or queue right. The 820px breakpoint is in window pixels. Narrow
        // windows stack, and the panel takes the rest.
        let stacked = !solo && vw <= 820.0;
        let vh_tokens = f32::from(layout.viewport.height) / t.token_scale();
        let mut layout_row = if stacked {
            div().relative().flex_1().min_h(px(0.0)).flex().flex_col().items_start().gap(t.s(18.0)).pt(t.s(60.0)).px(t.s(26.0)).pb(t.s(20.0)).child(player_col)
        } else {
            div().relative().flex_1().min_h(px(0.0)).flex().flex_row().items_center().justify_center().gap(t.s(gap)).px(t.s(side_pad)).child(player_col)
        };
        if !solo {
            let tokens_w = layout.tokens_w;
            // The panel's width, so lyrics wrap at the right measure.
            let panel_tokens = if stacked { tokens_w - 52.0 } else { (tokens_w - side_pad * 2.0 - gap - 412.0).max(240.0) };
            let top_inset = t.s(if stacked { 0.0 } else { 54.0 });
            let panel: AnyElement = match view {
                DockView::Lyrics => self.render_lyrics_panel(true, t.s(panel_tokens), top_inset, layout, cx),
                _ => self.render_queue_panel(top_inset, layout.scale, cx),
            };
            let area = if stacked {
                // The switcher heads the panel here, leaving the top-right corner for the account button and Collapse.
                let switcher = self.render_view_switcher(50.0, fg, cx);
                div()
                    .relative()
                    .w_full()
                    .h(t.s((vh_tokens * 0.42).max(160.0)))
                    .flex()
                    .flex_col()
                    .child(div().flex().flex_row().justify_center().mb(t.s(10.0)).child(div().occlude().child(switcher)))
                    .child(div().relative().flex_1().min_h(px(0.0)).child(panel))
            } else {
                div().relative().flex_1().min_w(px(0.0)).h_full().pt(t.s(178.0)).pb(t.s(40.0)).child(div().relative().size_full().child(panel))
            };
            layout_row = layout_row.child(area);
        }

        let close = crate::ui::ink_button(&t, "collapse", "nv-collapse", 24.0, t.t_on_art(), t.t_on_art())
            .occlude()
            .stop(&t).on_click(cx.listener(|this, _, _, cx| {
                this.expanded = false;
                cx.notify();
            }));

        let switcher = if stacked {
            div()
        } else if solo {
            let switcher_inner = self.render_view_switcher(50.0, fg, cx);
            div().absolute().top(t.s(24.0)).left(px(0.0)).right(px(0.0)).flex().justify_center().child(div().occlude().child(switcher_inner))
        } else {
            let switcher_inner = self.render_view_switcher(50.0, fg, cx);
            // Over the panel. Its top is 178 tokens down and the switcher sits 26 above that.
            let panel_left = (layout.tokens_w - 412.0 - gap - side_pad * 2.0).max(0.0);
            div().absolute().top(t.s(152.0)).right(t.s(side_pad)).w(t.s(panel_left)).flex().justify_center().child(div().occlude().child(switcher_inner))
        };
        // Window buttons sit in the desktop's corner, beside Collapse on the right. There's no account button here, but
        // Ctrl+, still opens Settings.
        let top_right = div().absolute().top(t.s(24.0)).right(t.s(32.0)).flex().flex_row().items_center().gap(t.s(12.0)).child(close).child(self.render_window_buttons(Side::Right, true));
        let top_left = div().absolute().top(t.s(24.0)).left(t.s(32.0)).flex().flex_row().items_center().child(self.render_window_buttons(Side::Left, true));
        let top_strip = div().absolute().top(px(0.0)).left(px(0.0)).right(px(0.0)).h(t.s(76.0)).map(|d| Shell::drag_area(d, cx));

        let corners = self.edge.corners;
        crate::chrome::round_corners(div().id("expanded-player").absolute().inset_0().occlude().flex().flex_col().bg(gpui::rgb(crate::theme::EXPANDED_BG)).child(bounds_slot(self.expanded_controls.clone(), 0)), corners)
            .when_some(backdrop, |d, image| {
                d.child(crate::chrome::round_corners(crate::ui::image(image).absolute().inset_0().size_full(), corners))
            })
            .child(layout_row)
            .child(top_strip)
            .child(switcher)
            .child(top_left)
            .child(top_right)
            .into_any_element()
    }
}

fn backdrop_ready(bg: &Option<(gpui::SharedString, u32, u32, std::sync::Arc<gpui::RenderImage>)>, album: &gpui::SharedString) -> bool {
    bg.as_ref().is_some_and(|(a, ..)| a == album)
}

/// Keeps one element's bounds in `store[slot]` and redraws when they move, so the scrim follows.
fn bounds_slot(store: std::rc::Rc<std::cell::RefCell<[gpui::Bounds<gpui::Pixels>; 2]>>, slot: usize) -> impl IntoElement {
    gpui::canvas(
        move |bounds, window, _| {
            let mut s = store.borrow_mut();
            if s[slot] != bounds {
                s[slot] = bounds;
                window.refresh();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}
