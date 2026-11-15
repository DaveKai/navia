//! The top bar: back and forward, title, center pill, and the Songs page controls.

use crate::i18n::{tr, trn};
use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, Context, FontWeight, Pixels, canvas, div, px};

use crate::persist::PlayerMode;
use crate::shell::{Layout, Route, Shell, SortKey};
use crate::theme::size::*;
use crate::ui::{divider, glass_pill, icon, icon_button};
use crate::view_window_buttons::Side;

/// Distance from the window edge to right-hand window buttons, in tokens.
const CORNER_GAP: f32 = 12.0;

impl Shell {
    pub fn render_topbar(&mut self, layout: &Layout, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let tokens_w = layout.tokens_w;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        // Narrow windows give the left group its real width so the center pill can't cover it.
        let tight = main_tokens <= 620.0;
        let mut left = div().flex().flex_row().items_center().gap(t.s(10.0)).min_w(px(0.0)).when(!tight, |d| d.flex_1()).when(tight, |d| d.flex_none());
        if !layout.sidebar_visible || layout.sidebar_overlay {
            let controls = self.render_window_buttons(Side::Left, false);
            left = left.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_none()
                    .mr(t.s(8.0))
                    .child(controls)
                    .child(icon_button(&t, "show-sidebar", "sidebar-simple-regular", false).occlude().stop(&t).on_click(cx.listener(move |this, _, _, cx| this.toggle_sidebar(tokens_w, cx)))),
            );
        }
        if !self.nav_pill_in_sidebar(layout) {
            left = left.child(self.render_nav_pill(cx));
        }
        let songs_page = self.route == Route::Songs;
        let settings_page = self.route == Route::Settings;
        if settings_page && main_tokens > 620.0 {
            left = left.child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.t_em()).flex_none().child(tr("Settings")));
        }
        let page_title = self.page_title();
        if let Some(title) = page_title.filter(|_| main_tokens > 620.0) {
            left = left.child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.t_em()).flex_none().truncate().min_w(px(0.0)).child(title));
        }
        if let Some(detail) = self.page_title_detail().filter(|_| main_tokens > 900.0) {
            left = left.child(div().ml(t.s(10.0)).text_color(t.t2()).truncate().child(detail));
        }
        if self.route == Route::Liked && main_tokens > 900.0 {
            left = left.child(div().ml(t.s(10.0)).text_color(t.t2()).truncate().child(self.liked_summary()));
        }
        if songs_page && main_tokens > 620.0 {
            let count = if self.query.is_empty() && !self.song_filters.is_filtering() {
                trn("{count} song", "{count} songs", self.total)
            } else {
                trn("{count} match", "{count} matches", self.order.len())
            };
            let mut title = div().flex().flex_row().items_baseline().min_w(px(0.0)).child(
                div().font_weight(FontWeight::SEMIBOLD).text_color(t.t_em()).flex_none().child(tr("Songs")),
            );
            if main_tokens > 900.0 {
                title = title.child(div().ml(t.s(10.0)).text_color(t.t2()).truncate().child(count));
            }
            left = left.child(title);
        }

        let center: Option<AnyElement> = if songs_page || settings_page { None } else { self.render_page_center(main_tokens, cx) };

        let right: AnyElement = if songs_page {
            let search_w = if main_tokens <= 620.0 {
                120.0
            } else if main_tokens <= 1000.0 {
                168.0
            } else {
                187.0
            };
            let compact = self.compact;
            let query = self.query.clone();
            let query_line = self.field_line(&crate::ui_controls::TextTarget::Page, &query, tr("Search Songs"), "songs-query", t.text(TEXT_MD));
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .gap(t.s(10.0))
                .child(
                    glass_pill(&t)
                        .occlude()
                        .child(icon_button(&t, "sort", "arrows-down-up-regular", false).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                            let position = event.position() + gpui::point(px(0.0), this.theme.s(14.0));
                            this.open_sort_menu(position, cx);
                        }))),
                )
                .when(main_tokens > 620.0, |d| d.child(
                    glass_pill(&t)
                        .occlude()
                        .child(icon_button(&t, "comfortable", if compact { "list-bullets-regular" } else { "list-bullets-fill" }, !compact).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                            this.compact = false;
                            cx.notify();
                        })))
                        .child(icon_button(&t, "compact", if compact { "rows-fill" } else { "rows-regular" }, compact).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                            this.compact = true;
                            cx.notify();
                        }))),
                ))
                .child(
                    crate::ui::search_field(&t, "search-pill", crate::ui::SearchSize::Toolbar, search_w, self.search_active, query_line)
                        .on_click(cx.listener(|this, _, _, cx| this.focus_field(crate::ui_controls::TextTarget::Page, cx))),
                )
                .into_any_element()
        } else {
            self.render_page_tools(main_tokens, cx)
        };

        let mut bar = div()
            .id("topbar")
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .h(t.s(TOPBAR_H))
            .pl(t.s(if layout.sidebar_visible && !layout.sidebar_overlay || self.has_window_buttons(Side::Left) { GUTTER } else { CORNER_GAP }))
            .pr(t.s(GUTTER))
            .flex()
            .flex_row()
            .items_center()
            .gap(t.s(12.0))
            // The bar's empty space belongs to the window, not to what has scrolled under it.
            .block_mouse_except_scroll()
            .map(|d| Shell::drag_area(d, cx))
            .child(left);
        if let Some(c) = center {
            bar = bar.child(c);
        }
        // The account button ends the right group, left of any corner window buttons.
        let account = self.render_account_button(crate::view_account::AccountPlace::TopBar, cx);
        let right = div().flex().flex_row().items_center().flex_none().gap(t.s(10.0)).child(right).child(account);
        let mut group = div().flex().flex_row().justify_end().flex_1().min_w(px(0.0)).when(tight, |d| d.overflow_hidden()).child(right);
        // With no dock drawn, the top bar carries the window buttons in the top-right corner and pushes the pills in.
        let corner_buttons = (layout.mode != PlayerMode::Dock || !layout.player) && self.has_window_buttons(Side::Right);
        if corner_buttons {
            bar = bar.pr(t.s(self.window_buttons_width(Side::Right) + CORNER_GAP * 2.0));
            group = group.pr(t.s(4.0));
        }
        let mut bar = bar.child(group);
        if corner_buttons {
            bar = bar.child(div().absolute().top(px(0.0)).bottom(px(0.0)).right(t.s(CORNER_GAP)).flex().items_center().child(self.render_window_buttons(Side::Right, false)));
        }
        bar.into_any_element()
    }

    /// Whether the nav pill sits in the sidebar header's left corner. That happens when the sidebar shows and no window
    /// buttons take that corner. Otherwise it stays in the top bar.
    pub fn nav_pill_in_sidebar(&self, layout: &Layout) -> bool {
        layout.sidebar_visible && !layout.sidebar_overlay && !self.has_window_buttons(Side::Left)
    }

    pub fn render_nav_pill(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        glass_pill(&t)
            .occlude()
            .child(icon_button(&t, "back", "caret-left-bold", false).stop(&t).on_click(cx.listener(|this, _, _, cx| this.go_back(cx))))
            .child(divider(&t))
            .child(icon_button(&t, "forward", "caret-right-bold", false).stop(&t).on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))))
            .into_any_element()
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.history_pos > 0 {
            self.history_pos -= 1;
            self.route = self.history[self.history_pos].clone();
            self.focus_arrival();
            self.route_arrival(cx);
            cx.notify();
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.history_pos + 1 < self.history.len() {
            self.history_pos += 1;
            self.route = self.history[self.history_pos].clone();
            self.focus_arrival();
            self.route_arrival(cx);
            cx.notify();
        }
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        if self.route != route {
            self.history.truncate(self.history_pos + 1);
            self.history.push(route.clone());
            self.history_pos = self.history.len() - 1;
            self.route = route;
            self.reset_page_state();
            self.focus_arrival();
            self.route_arrival(cx);
            cx.notify();
        }
    }

    /// Pages reload each time they're opened.
    fn route_arrival(&mut self, cx: &mut Context<Self>) {
        if self.route == Route::CachedSongs {
            self.refresh_cached(cx);
        }
    }

    /// Browse and Search open with focus in their field.
    fn focus_arrival(&mut self) {
        self.search_active = matches!(self.route, Route::Browse | Route::Search);
    }

    pub fn open_sort_menu(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let current = self.sort;
        let rows = SortKey::ALL
            .into_iter()
            .map(|(key, label)| {
                crate::view_menu::MenuRow::check(tr(label), current == key, move |this, cx| {
                    this.sort = key;
                    this.sort_desc = false;
                    this.songs_added_head = false;
                    this.rebuild_order();
                    cx.notify();
                })
            })
            .collect();
        self.open_menu(rows, position, true, cx);
    }

    pub fn render_icon_nav_public(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let width = if main_tokens <= 620.0 {
            42.0
        } else if main_tokens <= 900.0 {
            54.0
        } else {
            75.0
        };
        let items: [(&str, &str, Route); 5] = [
            ("house-regular", "house-fill", Route::Home),
            ("flame-regular", "flame-fill", Route::Discover(crate::pages::DiscoverTab::New)),
            ("star-four-regular", "star-four-fill", Route::Discover(crate::pages::DiscoverTab::ForYou)),
            ("nv-library", "nv-library", Route::Albums(crate::pages::AlbumScope::All)),
            ("magnifying-glass-regular", "magnifying-glass-regular", Route::Browse),
        ];
        let mut pill = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(t.s(PILL_H))
            .p(t.s(4.0))
            .rounded_full()
            .map(|d| crate::ui::glass_layers(d, &t, t.s(PILL_H / 2.0), GLASS_BLUR, GLASS_SATURATION, t.glass_frost_fill(), t.glass_fill(), crate::ui::edge_and(crate::ui::shadow_pill(&t))))
            .occlude();
        for (i, (regular, fill, route)) in items.into_iter().enumerate() {
            let active = match regular {
                "nv-library" => matches!(self.route, Route::Albums(_) | Route::Album(_) | Route::Songs | Route::Liked | Route::Artists | Route::Artist(_) | Route::Playlists | Route::Playlist(_)),
                "magnifying-glass-regular" => matches!(self.route, Route::Browse | Route::Search),
                "star-four-regular" => matches!(self.route, Route::Discover(tab) if tab != crate::pages::DiscoverTab::New),
                _ => self.route == route,
            };
            let name = if active { fill } else { regular };
            pill = pill.child(
                {
                let ink = crate::ui::ink(("nav", i));
                div()
                    .id(("nav", i))
                    .w(t.s(width))
                    .h(t.s(48.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(active, |d| d.bg(t.glass_on()))
                    .when(!active, |d| d.child(crate::ui::hover_track(("nav", i))).opacity(ink.opacity(1.0)))
                    .stop(&t).on_click(cx.listener(move |this, _, _, cx| this.navigate(route.clone(), cx)))
                    .child(icon(&t, name, 24.0, if active { t.t_em().into() } else { ink.color(t.t2(), t.hover_ink()) }))
                },
            );
        }
        pill.into_any_element()
    }

    pub fn render_bar_glow(&mut self, layout: &Layout) -> AnyElement {
        let t = self.theme;
        let Some((_, _, _, image)) = self.bar_glow.clone() else { return div().into_any_element() };
        crate::ui::image(image).absolute().left(px(0.0)).right(px(0.0)).bottom(px(0.0)).w(layout.main_width).h(t.s(150.0)).into_any_element()
    }
}


/// Records an element's window bounds so mouse x can be turned into a fraction.
pub fn track_bounds(store: std::rc::Rc<std::cell::RefCell<gpui::Bounds<Pixels>>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| {
            *store.borrow_mut() = bounds;
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}
