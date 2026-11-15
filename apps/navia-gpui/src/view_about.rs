//! An artist's About sheet, opened from the (i) in the artist header. Every part hides when the server has no data for
//! it.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, MouseButton, ScrollHandle, SharedString, Window, div, px, relative};

use crate::art::BakeKind;
use crate::focus::FocusStop;
use crate::i18n::{fmt_int, tr};
use crate::shell::{Layout, Shell};
use crate::theme::size::*;
use crate::ui_controls::{round_button, small_glass_action};

/// How many of the artist's most common album genres the sheet shows.
const GENRES_SHOWN: usize = 3;

/// The open sheet's artist and scroll state.
pub struct AboutSheet {
    artist: SharedString,
    scroll: ScrollHandle,
}

/// What the sheet shows.
pub struct ArtistAbout {
    facts: Vec<(&'static str, String)>,
    genres: Vec<SharedString>,
    bio: SharedString,
}

impl Shell {
    /// The artist's About, or None when the server has nothing for it.
    pub fn artist_about(&mut self, id: &SharedString) -> Option<ArtistAbout> {
        let info = self.core.as_ref().and_then(|c| c.lib.db.artist_info(id).ok().flatten());
        let mut facts = Vec::new();
        if let Some(info) = info {
            if let Some(n) = info.album_count.filter(|n| *n > 0) {
                facts.push((tr("Albums"), fmt_int(n as i64)));
            }
            if info.song_count > 0 {
                facts.push((tr("Songs"), fmt_int(info.song_count as i64)));
            }
            let mut roles: Vec<&String> = info.roles.iter().filter(|r| r.as_str() != "maincredit").collect();
            roles.sort_by_key(|r| crate::view_info::role_rank(r));
            if !roles.is_empty() {
                let names: Vec<String> = roles.iter().map(|r| crate::view_info::role_name(r, 1)).collect();
                facts.push((if roles.len() > 1 { tr("Roles") } else { tr("Role") }, names.join(", ")));
            }
            if info.play_count > 0 {
                facts.push((tr("Plays"), fmt_int(info.play_count as i64)));
            }
        }
        let mut count: Vec<(SharedString, usize)> = Vec::new();
        for i in self.artist_albums(id) {
            let Some(genre) = self.albums.get(i as usize).map(|a| a.genre.trim().to_string()).filter(|g| !g.is_empty()) else { continue };
            match count.iter_mut().find(|(g, _)| g.as_ref() == genre) {
                Some((_, n)) => *n += 1,
                None => count.push((genre.into(), 1)),
            }
        }
        // Most common first. The stable sort keeps album order among equals.
        count.sort_by(|a, b| b.1.cmp(&a.1));
        let genres = count.into_iter().take(GENRES_SHOWN).map(|(g, _)| g).collect::<Vec<_>>();
        let bio = self.library.as_ref().map(|l| l.artist_info(id).biography).unwrap_or_default();
        (!facts.is_empty() || !genres.is_empty() || !bio.is_empty()).then_some(ArtistAbout { facts, genres, bio })
    }

    pub fn open_artist_about(&mut self, artist: SharedString, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.about = Some(AboutSheet { artist, scroll: ScrollHandle::new() });
        cx.notify();
    }

    pub fn close_artist_about(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.about.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// The close button goes on the same side as the window's close button.
    fn close_on_left(&self) -> bool {
        self.button_layout.left.contains(&Some(gpui::WindowButton::Close))
    }

    pub fn render_artist_about(&mut self, layout: &Layout, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.about.as_ref()?;
        let (id, scroll) = (sheet.artist.clone(), sheet.scroll.clone());
        let artist = self.artists.iter().find(|a| a.id == id).cloned()?;
        let about = self.artist_about(&id).unwrap_or(ArtistAbout { facts: Vec::new(), genres: Vec::new(), bio: SharedString::default() });
        let t = self.theme;
        let device = |tokens: f32| (f32::from(t.s(tokens)) * layout.scale).round() as u32;
        let cover = artist.image.as_ref().filter(|i| !i.is_empty()).and_then(|i| self.bake_for(BakeKind::CoverFade, &i.clone(), device(SHEET_W), device(SHEET_COVER_H), cx));
        let has_cover = artist.image.as_ref().is_some_and(|i| !i.is_empty());
        let left = self.close_on_left();
        // Without a picture the name starts the sheet, so it must clear the close button.
        let clear = DIALOG_GAP * 2.0 + CONTROL_H - DIALOG_PAD;
        let name = div()
            .text_size(t.text(TEXT_TITLE))
            .font_weight(FontWeight::BOLD)
            .line_height(relative(1.1))
            .text_color(t.t_em())
            .when(!has_cover, |d| d.flex().items_center().min_h(t.s(CONTROL_H)).when(left, |d| d.pl(t.s(clear))).when(!left, |d| d.pr(t.s(clear))))
            .child(artist.name.clone());
        let mut content = div().relative().flex().flex_col().gap(t.s(SHEET_SECTION_GAP)).child(name);
        if !about.facts.is_empty() {
            let mut grid = div().flex().flex_col().gap(t.s(DIALOG_GAP));
            for pair in about.facts.chunks(2) {
                let mut row = div().flex().flex_row().gap(t.s(DIALOG_PAD));
                for (label, value) in pair {
                    row = row.child(div().flex_1().min_w(px(0.0)).child(div().text_size(t.text(TEXT_SM)).text_color(t.t2()).child(*label)).child(div().mt(t.s(2.0)).text_color(t.t1()).child(value.clone())));
                }
                if pair.len() == 1 {
                    row = row.child(div().flex_1());
                }
                grid = grid.child(row);
            }
            content = content.child(grid);
        }
        if !about.genres.is_empty() {
            let mut chips = div().flex().flex_row().flex_wrap().gap(t.s(DIALOG_ACTIONS_GAP));
            for (i, genre) in about.genres.iter().enumerate() {
                let g = genre.clone();
                chips = chips.child(small_glass_action(&t, ("about-genre", i)).child(genre.clone()).stop(&t).on_click(cx.listener(move |this, _, _, cx| {
                    this.close_artist_about(cx);
                    this.open_genre(g.clone(), cx);
                })));
            }
            let label = if about.genres.len() > 1 { tr("Genres") } else { tr("Genre") };
            content = content.child(div().flex().flex_col().gap(t.s(SHEET_LABEL_GAP)).child(div().text_size(t.text(TEXT_SM)).text_color(t.t2()).child(label)).child(chips));
        }
        if !about.bio.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(t.s(SHEET_LABEL_GAP))
                    .child(div().text_size(t.text(TEXT_SECTION)).font_weight(FontWeight::SEMIBOLD).text_color(t.t1()).child(tr("About")))
                    .child(div().line_height(relative(1.55)).text_color(t.t1()).child(about.bio.clone())),
            );
        }
        let body = div()
            .id("about-body")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px(t.s(DIALOG_PAD))
            .pb(t.s(DIALOG_PAD))
            .when(!has_cover, |d| d.pt(t.s(DIALOG_GAP)))
            .when(has_cover, |d| d.child(crate::ui::dialog_cover_picture(&t, cover)))
            .child(content);
        let close = round_button(&t, "about-close", "x-regular")
            .absolute()
            .top(t.s(DIALOG_GAP))
            .when(left, |d| d.left(t.s(DIALOG_GAP)))
            .when(!left, |d| d.right(t.s(DIALOG_GAP)))
            .stop_top(&t)
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_artist_about(cx);
            }));
        Some(
            crate::ui::dialog_scrim(&t, "about-scrim")
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.close_artist_about(cx);
                }))
                .child(crate::ui::dialog_cover(&t, "about", window.viewport_size().height).child(body).child(close))
                .into_any_element(),
        )
    }
}
