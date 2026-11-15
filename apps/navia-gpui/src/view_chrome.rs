//! The top bar's page-specific parts: titles, the center pill, and page toolbars.

use crate::i18n::tr;
use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, Context, FontWeight, SharedString, div};

use crate::pages::{AlbumScope, DiscoverTab, Route};
use crate::shell::Shell;
use crate::theme::size::*;
use crate::ui::{divider, glass_pill, icon, icon_button};
use crate::ui_controls::TextTarget;
use crate::view_menu::MenuRow;
use crate::view_search::{SearchScope, SearchSort};

impl Shell {
    pub fn page_title(&self) -> Option<SharedString> {
        match &self.route {
            Route::Albums(AlbumScope::All) => Some(tr("Albums").into()),
            Route::Albums(AlbumScope::Artist(_, section)) => Some(crate::view_artist::section_title(*section).into()),
            Route::Liked => Some(tr("Liked Songs").into()),
            Route::Artists => Some(tr("Artists").into()),
            Route::Playlists => Some(tr("Playlists").into()),
            Route::Genres => Some(tr("Genres").into()),
            Route::Equalizer => Some(tr("Equalizer").into()),
            Route::Downloads => Some(tr("Downloads").into()),
            Route::CachedSongs => Some(tr("Cached Songs").into()),
            Route::Shares => Some(tr("Share Links").into()),
            Route::SmartEditor(id) => Some(self.smart_page_title(id)),
            _ => None,
        }
    }

    /// Secondary text after the title, such as whose albums a "See all" page shows.
    pub fn page_title_detail(&self) -> Option<SharedString> {
        match &self.route {
            Route::Albums(AlbumScope::Artist(id, _)) => self.artists.iter().find(|a| &a.id == id).map(|a| a.name.clone()),
            Route::Artists => self.artists_title_detail(),
            _ => None,
        }
    }

    pub fn render_page_center(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.route.clone() {
            Route::Discover(tab) => Some(self.render_discover_tabs(tab, main_tokens, cx)),
            Route::Browse | Route::Search => Some(self.render_search_field(main_tokens, cx)),
            Route::Albums(_) | Route::Artists | Route::Liked | Route::Playlists | Route::Genres | Route::Equalizer | Route::Downloads | Route::CachedSongs | Route::Shares | Route::SmartEditor(_) => None,
            _ => Some(self.render_icon_nav_public(main_tokens, cx)),
        }
    }

    fn render_discover_tabs(&mut self, active: DiscoverTab, main_tokens: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let pad = if main_tokens <= 620.0 {
            10.0
        } else if main_tokens <= 900.0 {
            14.0
        } else {
            24.0
        };
        let tabs = [(DiscoverTab::ForYou, tr("For You")), (DiscoverTab::New, tr("New")), (DiscoverTab::Charts, tr("Charts")), (DiscoverTab::Radio, tr("Radio"))];
        let mut pill = div().flex().flex_row().items_center().flex_none().h(t.s(53.0)).p(t.s(4.0)).rounded_full().map(|d| crate::ui::glass_layers(d, &t, t.s(53.0 / 2.0), GLASS_BLUR, GLASS_SATURATION, t.glass_frost_fill(), t.glass_fill(), crate::ui::edge_and(crate::ui::shadow_pill(&t)))).occlude();
        for (i, (tab, label)) in tabs.into_iter().enumerate() {
            let on = tab == active;
            let ink = crate::ui::ink(("tab", i));
            pill = pill.child(
                div()
                    .id(("tab", i))
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(t.s(44.0))
                    .px(t.s(pad))
                    .rounded_full()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.t_em().into() } else { ink.color(t.t2(), t.hover_ink()) })
                    .when(on, |d| d.bg(t.glass_on()))
                    .when(!on, |d| d.child(crate::ui::hover_track(("tab", i))).opacity(ink.opacity(1.0)))
                    .stop(&t).on_click(cx.listener(move |this, _, _, cx| this.navigate(Route::Discover(tab), cx)))
                    .child(label),
            );
        }
        pill.into_any_element()
    }

    fn render_search_field(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let width = if main_tokens <= 620.0 {
            260.0_f32.min(main_tokens * 0.45)
        } else if main_tokens <= 1000.0 {
            260.0
        } else {
            384.0
        };
        let query = self.search_query.clone();
        let active = self.search_active;
        let text = self.field_line(&TextTarget::Page, &query, tr("Search..."), "search-query", t.text(TEXT_LG));
        crate::ui::search_field(&t, "search-field", crate::ui::SearchSize::Large, width, active, text)
            .on_click(cx.listener(|this, _, _, cx| this.focus_field(TextTarget::Page, cx)))
            .into_any_element()
    }

    pub fn render_page_tools(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let search_w = if main_tokens <= 620.0 {
            120.0
        } else if main_tokens <= 1000.0 {
            168.0
        } else {
            187.0
        };
        match self.route.clone() {
            Route::Albums(_) => {
                let query = self.albums_view.query.clone();
                let sort = icon_button(&t, "albums-sort", "arrows-down-up-regular", false).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    let rows = this.albums_sort_rows();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let filter = icon_button(&t, "albums-filter", "funnel-simple-regular", self.albums_view.genre.is_some()).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    let rows = this.albums_filter_rows();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let views = self.layout_toggle(crate::view_split::SplitPage::Albums, cx);
                let search = self.toolbar_search(tr("Search Albums"), query, search_w, cx);
                self.toolbar(vec![glass_pill(&t).occlude().child(sort).child(divider(&t)).child(filter).into_any_element(), views, search])
            }
            Route::Playlists => {
                let query = self.playlists_view.query.clone();
                let list_mode = self.playlists_view.list;
                let sort = icon_button(&t, "playlists-sort", "arrows-down-up-regular", false).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    let rows = this.playlists_sort_rows();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let filter = icon_button(&t, "playlists-filter", "funnel-simple-regular", self.playlists_view.kind != crate::view_playlists::PlaylistKind::All).stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    let rows = this.playlists_kind_rows();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let list_btn = icon_button(&t, "playlists-list-view", if list_mode { "list-bullets-fill" } else { "list-bullets-regular" }, list_mode).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                    this.playlists_view.list = true;
                    this.invalidate_page();
                    cx.notify();
                }));
                let grid_btn = icon_button(&t, "playlists-grid-view", if list_mode { "grid-four-regular" } else { "grid-four-fill" }, !list_mode).stop(&t).on_click(cx.listener(|this, _, _, cx| {
                    this.playlists_view.list = false;
                    this.invalidate_page();
                    cx.notify();
                }));
                let search = self.toolbar_search(tr("Search Playlists"), query, search_w, cx);
                let new_button = crate::ui_controls::round_button(&t, "playlists-new", "plus-regular").occlude().stop(&t).on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    let rows = this.new_playlist_rows();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                self.toolbar(vec![new_button.into_any_element(), glass_pill(&t).occlude().child(sort).child(divider(&t)).child(filter).into_any_element(), glass_pill(&t).occlude().child(list_btn).child(grid_btn).into_any_element(), search])
            }
            Route::Search => {
                let v = &self.search_view;
                let (sort, favorites, scope) = (v.sort, v.favorites, v.scope);
                let sort_btn = icon_button(&t, "search-sort", "arrows-down-up-regular", false).stop(&t).on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                    let rows = [(SearchSort::Relevance, tr("Relevance")), (SearchSort::Name, tr("Name")), (SearchSort::Year, tr("Year"))]
                        .into_iter()
                        .map(|(s, label)| MenuRow::check(label, sort == s, move |this, cx| {
                            this.search_view.sort = s;
                            this.invalidate_page();
                            cx.notify();
                        }))
                        .collect();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let filter_btn = icon_button(&t, "search-filter", "funnel-simple-regular", favorites).stop(&t).on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                    let rows = [(false, tr("Everything")), (true, tr("Favorites"))]
                        .into_iter()
                        .map(|(f, label)| MenuRow::check(label, favorites == f, move |this, cx| {
                            this.search_view.favorites = f;
                            this.invalidate_page();
                            cx.notify();
                        }))
                        .collect();
                    this.open_toolbar_menu(rows, event, cx);
                }));
                let scope_btn = div()
                    .id("search-scope")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(t.s(8.0))
                    .h(t.s(34.0))
                    .px(t.s(12.0))
                    .rounded_full()
                    .font_weight(FontWeight::MEDIUM)
                    .child(crate::ui::hover_pill(&t))
                    .child(scope.label())
                    .child(icon(&t, "caret-up-down-bold", 15.0, t.t1()))
                    .stop(&t).on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        let rows = [SearchScope::All, SearchScope::Artists, SearchScope::Albums, SearchScope::Songs]
                            .into_iter()
                            .map(|s| MenuRow::check(s.label(), scope == s, move |this, cx| {
                                this.search_view.scope = s;
                                this.invalidate_page();
                                cx.notify();
                            }))
                            .collect();
                        this.open_toolbar_menu(rows, event, cx);
                    }));
                self.toolbar(vec![glass_pill(&t).occlude().child(sort_btn).child(divider(&t)).child(filter_btn).into_any_element(), glass_pill(&t).occlude().child(scope_btn).into_any_element()])
            }
            Route::Artists if !self.artists.is_empty() => {
                let query = self.artists_view.query.clone();
                let views = self.layout_toggle(crate::view_split::SplitPage::Artists, cx);
                let search = self.toolbar_search(tr("Search Artists"), query, search_w, cx);
                self.toolbar(vec![views, search])
            }
            Route::Genres if !self.genres_view.page.all.is_empty() => {
                let query = self.genres_view.query.clone();
                self.toolbar_search(tr("Search Genres"), query, search_w, cx)
            }
            Route::SmartEditor(_) => self.render_smart_tools(cx),
            _ => div().into_any_element(),
        }
    }

    fn toolbar(&mut self, children: Vec<AnyElement>) -> AnyElement {
        let t = self.theme;
        div().flex().flex_row().items_center().flex_none().gap(t.s(10.0)).children(children).into_any_element()
    }

    fn toolbar_search(&mut self, placeholder: &'static str, query: String, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let line = self.field_line(&TextTarget::Page, &query, placeholder, "toolbar-query", t.text(TEXT_MD));
        crate::ui::search_field(&t, "toolbar-search", crate::ui::SearchSize::Toolbar, width, self.search_active, line)
            .on_click(cx.listener(|this, _, _, cx| this.focus_field(TextTarget::Page, cx)))
            .into_any_element()
    }
}
