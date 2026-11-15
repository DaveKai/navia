//! Artist, album and genre text in song rows as links. Hovering underlines the text and a
//! click opens that page. Ctrl or Shift clicks still extend the selection. Text the library
//! can't resolve stays plain.

use gpui::prelude::*;
use crate::ui::LinkCursor;
use gpui::{AnyElement, Context, Hsla, MouseButton, SharedString, div, px};

use crate::model::Song;
use crate::pages::Route;
use crate::shell::Shell;

#[derive(Clone, Debug)]
pub enum LinkTarget {
    Artist(SharedString),
    Album(SharedString),
    Genre(SharedString),
    Playlist(SharedString),
    Mix(SharedString),
    Liked,
    Songs,
}

impl From<&navia_core::queue_order::SourceLink> for LinkTarget {
    fn from(link: &navia_core::queue_order::SourceLink) -> Self {
        use navia_core::queue_order::SourceLink as L;
        match link {
            L::Album(id) => Self::Album(id.clone().into()),
            L::Playlist(id) => Self::Playlist(id.clone().into()),
            L::Artist(id) => Self::Artist(id.clone().into()),
            L::Genre(name) => Self::Genre(name.clone().into()),
            L::Mix(id) => Self::Mix(id.clone().into()),
            L::Liked => Self::Liked,
            L::Songs => Self::Songs,
        }
    }
}

impl Shell {
    /// One link, sized to its text so only the text reacts. `key` must be unique in the frame.
    pub fn link(&mut self, key: (&'static str, usize), text: SharedString, target: Option<LinkTarget>, color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        // Padding above and below, canceled by margins, gives a short name a larger hit area
        // without moving the text.
        let pad = self.theme.s(3.0);
        let base = div().min_w(px(0.0)).truncate().text_color(color).py(pad).my(-pad).child(text);
        let Some(target) = target else { return base.into_any_element() };
        base.id(key)
            .link_cursor()
            .hover(|s| s.underline())
            // The row underneath must not take this press as the start of a drag.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                let m = event.modifiers;
                if !(m.shift || m.control || m.platform) {
                    this.link_pressed = Some(std::time::Instant::now());
                }
            }))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                let m = event.modifiers();
                // Ctrl or Shift on a link extends the selection, as anywhere else in the row.
                if m.secondary(cx) || m.shift {
                    return;
                }
                cx.stop_propagation();
                this.link_pressed = None;
                match target.clone() {
                    LinkTarget::Artist(id) => this.navigate(Route::Artist(id), cx),
                    LinkTarget::Album(id) => this.navigate(Route::Album(id), cx),
                    LinkTarget::Genre(name) => this.open_genre(name, cx),
                    LinkTarget::Playlist(id) => this.navigate(Route::Playlist(id), cx),
                    LinkTarget::Mix(id) => this.navigate(Route::Mix(id), cx),
                    LinkTarget::Liked => this.navigate(Route::Liked, cx),
                    LinkTarget::Songs => this.open_all_songs(cx),
                }
                // From the mini player, the full window comes back on that page.
                this.mini_follow_link(cx);
            }))
            .into_any_element()
    }

    /// The song's artists, each a link when the library knows them, separated by commas.
    pub fn artist_links(&mut self, key: (&'static str, usize), song: &Song, color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let names: Vec<SharedString> = if song.artists.len() > 1 { song.artists.clone() } else { vec![song.artist.clone()] };
        let mut row = div().flex().flex_row().min_w(px(0.0)).overflow_hidden().whitespace_nowrap();
        let count = names.len();
        for (k, name) in names.into_iter().enumerate() {
            let target = self.artist_named(&name).map(|a| LinkTarget::Artist(a.id.clone()));
            let last = k + 1 == count;
            // Only the last name truncates, so the line reads as one text cut at its end.
            let link = self.link((key.0, key.1 * 16 + k.min(15)), name, target, color, cx);
            row = if last { row.child(link) } else { row.child(div().flex_none().child(link)) };
            if !last {
                row = row.child(div().flex_none().mr(px(4.0)).text_color(color).child(","));
            }
        }
        row.into_any_element()
    }

    /// A credit line like "A, B & C", with each known artist as its own link and the separators
    /// as plain text. A credit the library knows as one artist is one link.
    pub fn artist_name_links(&mut self, key: (&'static str, usize), credit: &SharedString, color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().flex().flex_row().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().text_color(color);
        if let Some(a) = self.artist_named(credit) {
            let id = a.id.clone();
            return row.child(self.link(key, credit.clone(), Some(LinkTarget::Artist(id)), color, cx)).into_any_element();
        }
        let mut k = 0;
        for (piece, is_name) in split_credit(credit) {
            if !is_name {
                row = row.child(div().flex_none().whitespace_nowrap().child(SharedString::from(piece.replace(' ', "\u{00a0}"))));
                continue;
            }
            let target = self.artist_named(&SharedString::from(piece.clone())).map(|a| LinkTarget::Artist(a.id.clone()));
            row = row.child(self.link((key.0, key.1 * 16 + k.min(15)), piece.into(), target, color, cx));
            k += 1;
        }
        row.into_any_element()
    }

    pub fn album_link(&mut self, key: (&'static str, usize), song: &Song, color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let target = (!song.album_id.is_empty()).then(|| LinkTarget::Album(song.album_id.clone()));
        self.link(key, song.album.clone(), target, color, cx)
    }

    pub fn genre_link(&mut self, key: (&'static str, usize), song: &Song, color: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let target = (!song.genre.trim().is_empty()).then(|| LinkTarget::Genre(song.genre.clone()));
        self.link(key, song.genre.clone(), target, color, cx)
    }
}

/// Splits an artist credit into names and the separators between them, in order.
pub fn split_credit(credit: &str) -> Vec<(String, bool)> {
    const SEPARATORS: [&str; 6] = [", ", " & ", " feat. ", " ft. ", " featuring ", " x "];
    let mut out = Vec::new();
    let mut rest = credit;
    loop {
        let next = SEPARATORS.iter().filter_map(|sep| rest.find(sep).map(|i| (i, *sep))).min_by_key(|(i, _)| *i);
        match next {
            Some((i, sep)) if i > 0 => {
                out.push((rest[..i].to_string(), true));
                out.push((sep.to_string(), false));
                rest = &rest[i + sep.len()..];
            }
            _ => {
                if !rest.is_empty() {
                    out.push((rest.to_string(), true));
                }
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn credits_split_into_names() {
        let parts = super::split_credit("A$AP Rocky, Imogen Heap & Clams Casino");
        let names: Vec<&str> = parts.iter().filter(|p| p.1).map(|p| p.0.as_str()).collect();
        assert_eq!(names, ["A$AP Rocky", "Imogen Heap", "Clams Casino"]);
        assert_eq!(super::split_credit("Sade"), vec![("Sade".to_string(), true)]);
    }
}
