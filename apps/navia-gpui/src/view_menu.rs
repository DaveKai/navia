//! Floating glass menus and their submenus. One menu is open at a time, drawn above everything.

use crate::i18n::{tr, trf};
use std::rc::Rc;

use gpui::prelude::*;
use std::cell::RefCell;

use gpui::{AnyElement, Bounds, Context, FontWeight, Pixels, Point, ScrollHandle, SharedString, Window, anchored, deferred, div, point, px, relative};

use crate::pages::{Block, Route};
use crate::persist::PlayerMode;
use crate::shell::{Layout, Shell};
use crate::ui_controls::track_bounds;
use crate::theme::size::*;
use crate::ui::{glass_edge, icon};

pub type MenuAction = Rc<dyn Fn(&mut Shell, &mut Context<Shell>)>;

#[derive(Clone)]
pub enum MenuRow {
    Item {
        label: SharedString,
        icon: Option<&'static str>,
        checked: Option<bool>,
        /// A smaller second line under the label: why the row is disabled, or a detail like a receiver's model.
        reason: Option<SharedString>,
        /// Shows a spinner where the check would be.
        busy: bool,
        action: Option<MenuAction>,
        /// Drawn as a switch, and the menu stays open when it's flipped. Needs `checked`.
        toggle: bool,
        /// A keyboard shortcut shown dim at the end.
        shortcut: Option<SharedString>,
        /// How many songs a choice matches, shown dim before its check.
        count: Option<SharedString>,
        /// Opens this submenu beside the row on hover, click, Enter, Space or →.
        submenu: Option<Rc<Submenu>>,
    },
    Divider,
    /// Who is signed in, above the account menu's rows.
    Header { initial: SharedString, name: SharedString, host: SharedString },
    /// A section heading, like the Output menu's AirPlay, with a spinner while it fills. Not pressable.
    Heading { label: SharedString, icon: Option<&'static str>, busy: bool },
}

/// A submenu's content: lead rows the filter never hides, like New Playlist, then a filterable list.
pub struct Submenu {
    pub lead: Vec<MenuRow>,
    pub items: Vec<MenuRow>,
    /// The filter field's placeholder. Typing anywhere in the submenu goes to it.
    pub find: Option<&'static str>,
    /// Shown in place of the list when the filter matches nothing.
    pub empty: Option<&'static str>,
}

/// The open submenu: its parent row, its filter, and keyboard position.
pub struct SubmenuState {
    pub row: usize,
    pub query: String,
    /// The focused row among the shown ones, lead rows first. None when the field has focus or the pointer opened it.
    pub focused: Option<usize>,
    /// The pointer is inside, so crossing other rows doesn't close it.
    pub pointer_inside: bool,
    pub scroll: ScrollHandle,
}

impl MenuRow {
    fn new_item(label: SharedString, icon: Option<&'static str>, checked: Option<bool>, reason: Option<SharedString>, action: Option<MenuAction>, toggle: bool) -> Self {
        MenuRow::Item { label, icon, checked, reason, busy: false, action, toggle, shortcut: None, count: None, submenu: None }
    }

    pub fn item(label: impl Into<SharedString>, icon: &'static str, action: impl Fn(&mut Shell, &mut Context<Shell>) + 'static) -> Self {
        Self::new_item(label.into(), Some(icon), None, None, Some(Rc::new(action)), false)
    }

    /// A row without an icon, such as a playlist in Add to Playlist's submenu.
    pub fn plain(label: impl Into<SharedString>, action: impl Fn(&mut Shell, &mut Context<Shell>) + 'static) -> Self {
        Self::new_item(label.into(), None, None, None, Some(Rc::new(action)), false)
    }

    pub fn submenu(label: impl Into<SharedString>, icon: &'static str, submenu: Submenu) -> Self {
        // The action never runs. It only marks the row as enabled for the keyboard.
        MenuRow::Item { label: label.into(), icon: Some(icon), checked: None, reason: None, busy: false, action: Some(Rc::new(|_, _| {})), toggle: false, shortcut: None, count: None, submenu: Some(Rc::new(submenu)) }
    }

    /// A switch row that flips without closing the menu.
    pub fn toggle(label: impl Into<SharedString>, icon: &'static str, on: bool, action: impl Fn(&mut Shell, &mut Context<Shell>) + 'static) -> Self {
        Self::new_item(label.into(), Some(icon), Some(on), None, Some(Rc::new(action)), true)
    }

    fn label(&self) -> Option<&SharedString> {
        match self {
            MenuRow::Item { label, .. } => Some(label),
            _ => None,
        }
    }

    fn enabled(&self) -> bool {
        matches!(self, MenuRow::Item { action: Some(_), .. })
    }

    /// Shows how many songs a choice matches, before its check.
    pub fn with_count(mut self, n: impl Into<SharedString>) -> Self {
        if let MenuRow::Item { count, .. } = &mut self {
            *count = Some(n.into());
        }
        self
    }

    /// Shows a keyboard shortcut at the row's end.
    pub fn with_shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        if let MenuRow::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys.into());
        }
        self
    }

    /// A disabled row without an icon.
    pub fn plain_off(label: impl Into<SharedString>) -> Self {
        Self::new_item(label.into(), None, None, None, None, false)
    }

    pub fn disabled(label: impl Into<SharedString>, icon: &'static str, reason: &'static str) -> Self {
        Self::new_item(label.into(), Some(icon), None, Some(reason.into()), None, false)
    }

    /// A row with an optional `detail` line, a check, and a spinner while `busy`. Without `action` the row is disabled
    /// and the detail says why.
    pub fn detailed(label: impl Into<SharedString>, detail: Option<SharedString>, checked: bool, busy: bool, action: Option<MenuAction>) -> Self {
        MenuRow::Item { label: label.into(), icon: None, checked: Some(checked), reason: detail, busy, action, toggle: false, shortcut: None, count: None, submenu: None }
    }

    /// A radio-style row for sorts and filters.
    pub fn check(label: impl Into<SharedString>, on: bool, action: impl Fn(&mut Shell, &mut Context<Shell>) + 'static) -> Self {
        Self::new_item(label.into(), None, Some(on), None, Some(Rc::new(action)), false)
    }
}

pub struct MenuState {
    pub rows: Vec<MenuRow>,
    pub position: Point<Pixels>,
    /// Aligns the menu's right edge to `position`, for buttons at the right of a toolbar.
    pub align_end: bool,
    pub focused: Option<usize>,
    /// The highlight only shows after a key press. The first row still has focus when the menu opens, so Enter works
    /// right away.
    pub keyboard: bool,
    /// Read-only text above the rows, like a smart playlist's rules. It wraps and scrolls.
    pub panel: Option<MenuPanel>,
    pub sub: Option<SubmenuState>,
    /// The output device menu. Its rows are rebuilt in place when the device list changes.
    pub outputs: bool,
    /// Bumped when the pointer moves to another row, so only the latest close timer acts.
    sub_close: u64,
    /// Where the menu and each row were last drawn, for placing submenus.
    bounds: Rc<RefCell<Bounds<Pixels>>>,
    row_bounds: Vec<Rc<RefCell<Bounds<Pixels>>>>,
}

pub struct MenuPanel {
    /// The rules as one sentence, shown first.
    pub sentence: Option<SharedString>,
    pub heading: Option<SharedString>,
    pub lines: Vec<SharedString>,
    pub footnote: Option<SharedString>,
    pub scroll: ScrollHandle,
}

/// A menu row's height, then the filter field's height and the gap below it.
const ROW_H: f32 = 36.0;
const FILTER_H: f32 = 34.0;
const FILTER_GAP: f32 = 4.0;
const MENU_PAD: f32 = 6.0;
/// Gap kept from the window's edges.
const EDGE: f32 = 10.0;
/// How long the pointer may cross other rows on its way into an open submenu before it closes.
const SUB_CLOSE_MS: u64 = 250;

impl Shell {
    /// How far floating panels stay inside the window. `EDGE` past the window frame's shadow and resize border, so
    /// nothing runs past the visible edge.
    pub fn float_margin(&self) -> gpui::Edges<Pixels> {
        gpui::Edges { top: self.edge.top + px(EDGE), right: self.edge.right + px(EDGE), bottom: self.edge.bottom + px(EDGE), left: self.edge.left + px(EDGE) }
    }

    pub fn open_menu(&mut self, rows: Vec<MenuRow>, position: Point<Pixels>, align_end: bool, cx: &mut Context<Self>) {
        let focused = rows.iter().position(MenuRow::enabled);
        let row_bounds = rows.iter().map(|_| Rc::default()).collect();
        self.menu = Some(MenuState { rows, position, align_end, focused, keyboard: false, panel: None, sub: None, outputs: false, sub_close: 0, bounds: Rc::default(), row_bounds });
        self.leave_menu_find();
        cx.notify();
    }

    /// Opens a menu led by read-only text, left-aligned under `position`.
    pub fn open_panel_menu(&mut self, panel: MenuPanel, rows: Vec<MenuRow>, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.open_menu(rows, position, false, cx);
        if let Some(menu) = self.menu.as_mut() {
            menu.panel = Some(panel);
        }
    }

    /// Arrow, Page and Home/End keys scroll a menu's panel, since its text has nothing to focus.
    fn panel_scroll(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let step = self.theme.s(40.0);
        let Some(panel) = self.menu.as_ref().and_then(|m| m.panel.as_ref()) else { return false };
        let page = panel.scroll.bounds().size.height - step;
        let max = panel.scroll.max_offset().y;
        if max <= px(0.0) {
            return false;
        }
        let y = -panel.scroll.offset().y;
        let next = match key {
            "down" => y + step,
            "up" => y - step,
            "pagedown" => y + page,
            "pageup" => y - page,
            "home" => px(0.0),
            "end" => max,
            _ => return false,
        };
        panel.scroll.set_offset(point(px(0.0), -next.clamp(px(0.0), max)));
        cx.notify();
        true
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.leave_menu_find();
        cx.notify();
    }

    /// The submenu's filter gives the keyboard back.
    fn leave_menu_find(&mut self) {
        if self.controls.text == Some(crate::ui_controls::TextTarget::MenuFind) {
            self.controls.text = None;
        }
    }

    pub fn menu_find_focused(&self) -> bool {
        self.controls.text == Some(crate::ui_controls::TextTarget::MenuFind)
    }

    /// Opens row `index`'s submenu. From the keyboard or a click, focus moves into it, to the filter if it has one.
    pub fn open_submenu(&mut self, index: usize, keyboard: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        let Some(MenuRow::Item { submenu: Some(spec), action: Some(_), .. }) = menu.rows.get(index) else { return };
        let has_find = spec.find.is_some();
        menu.sub_close += 1;
        if menu.sub.as_ref().is_some_and(|s| s.row == index) && !keyboard {
            return;
        }
        let first = (!has_find).then(|| Self::sub_rows(spec, "").iter().position(MenuRow::enabled)).flatten();
        menu.sub = Some(SubmenuState { row: index, query: String::new(), focused: if keyboard { first } else { None }, pointer_inside: false, scroll: ScrollHandle::new() });
        menu.focused = Some(index);
        if keyboard {
            menu.keyboard = true;
        }
        self.leave_menu_find();
        if keyboard && has_find {
            self.focus_field(crate::ui_controls::TextTarget::MenuFind, cx);
        }
        cx.notify();
    }

    /// ← or Esc closes a submenu and puts focus back on its row.
    pub fn submenu_back(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.menu.as_mut() else { return false };
        let Some(sub) = menu.sub.take() else { return false };
        menu.focused = Some(sub.row);
        menu.keyboard = true;
        self.leave_menu_find();
        cx.notify();
        true
    }

    /// The pointer moved onto another row. Close the submenu soon unless the pointer reaches it first.
    fn submenu_close_soon(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        if menu.sub.is_none() {
            return;
        }
        menu.sub_close += 1;
        let generation = menu.sub_close;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(SUB_CLOSE_MS)).await;
            let _ = this.update(cx, |this, cx| {
                let Some(menu) = this.menu.as_mut() else { return };
                if menu.sub_close == generation && menu.sub.as_ref().is_some_and(|s| !s.pointer_inside) {
                    menu.sub = None;
                    this.leave_menu_find();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The submenu's rows as shown: lead rows, then items matching the filter.
    fn sub_rows(spec: &Submenu, query: &str) -> Vec<MenuRow> {
        let needle = query.trim().to_lowercase();
        let mut rows = spec.lead.clone();
        rows.extend(spec.items.iter().filter(|r| needle.is_empty() || r.label().is_some_and(|l| l.to_lowercase().contains(&needle))).cloned());
        rows
    }

    /// How many shown rows are lead rows, and how many matched the filter.
    fn sub_counts(spec: &Submenu, query: &str) -> (usize, usize) {
        let shown = Self::sub_rows(spec, query).len();
        (spec.lead.len(), shown - spec.lead.len())
    }

    fn open_sub_spec(&self) -> Option<(Rc<Submenu>, String)> {
        let menu = self.menu.as_ref()?;
        let sub = menu.sub.as_ref()?;
        match menu.rows.get(sub.row) {
            Some(MenuRow::Item { submenu: Some(spec), .. }) => Some((spec.clone(), sub.query.clone())),
            _ => None,
        }
    }

    pub fn menu_find_text(&self) -> String {
        self.menu.as_ref().and_then(|m| m.sub.as_ref()).map(|s| s.query.clone()).unwrap_or_default()
    }

    pub fn set_menu_find_text(&mut self, value: String, cx: &mut Context<Self>) {
        if let Some(sub) = self.menu.as_mut().and_then(|m| m.sub.as_mut()) {
            sub.query = value;
            sub.focused = None;
            sub.scroll.set_offset(point(px(0.0), px(0.0)));
        }
        cx.notify();
    }

    /// Arrow keys in a submenu move through the filter field and enabled rows, wrapping.
    fn submenu_move(&mut self, delta: i32, last: Option<bool>, cx: &mut Context<Self>) {
        let Some((spec, query)) = self.open_sub_spec() else { return };
        let rows = Self::sub_rows(&spec, &query);
        let mut stops: Vec<Option<usize>> = Vec::new();
        if spec.find.is_some() {
            stops.push(None);
        }
        stops.extend(rows.iter().enumerate().filter(|(_, r)| r.enabled()).map(|(i, _)| Some(i)));
        if stops.is_empty() {
            return;
        }
        let current = if self.menu_find_focused() { Some(None) } else { self.menu.as_ref().and_then(|m| m.sub.as_ref()).and_then(|s| s.focused).map(Some) };
        let at = current.and_then(|c| stops.iter().position(|s| *s == c));
        let next = match (last, at) {
            (Some(true), _) => stops.len() - 1,
            (Some(false), _) => 0,
            (None, Some(i)) => (i as i32 + delta).rem_euclid(stops.len() as i32) as usize,
            (None, None) if delta >= 0 => 0,
            (None, None) => stops.len() - 1,
        };
        let target = stops[next];
        if let Some(sub) = self.menu.as_mut().and_then(|m| m.sub.as_mut()) {
            sub.focused = target;
            if let Some(i) = target {
                Self::scroll_sub_to(sub, &spec, i, self.theme.s(ROW_H));
            }
        }
        if let Some(menu) = self.menu.as_mut() {
            menu.keyboard = true;
        }
        match target {
            None => self.focus_field(crate::ui_controls::TextTarget::MenuFind, cx),
            Some(_) => self.leave_menu_find(),
        }
        cx.notify();
    }

    /// Keeps a focused list row in view inside the submenu's scrolling list.
    fn scroll_sub_to(sub: &SubmenuState, spec: &Submenu, shown_ix: usize, row_h: Pixels) {
        let Some(list_ix) = shown_ix.checked_sub(spec.lead.len()) else { return };
        let top = row_h * list_ix as f32;
        let view = sub.scroll.bounds().size.height;
        let y = -sub.scroll.offset().y;
        let next = if top < y { top } else if top + row_h > y + view { top + row_h - view } else { return };
        sub.scroll.set_offset(point(px(0.0), -next));
    }

    /// Runs the shown submenu row `shown_ix`. Every submenu row closes the whole menu.
    fn run_sub_row(&mut self, shown_ix: usize, cx: &mut Context<Self>) {
        let Some((spec, query)) = self.open_sub_spec() else { return };
        let Some(MenuRow::Item { action: Some(action), .. }) = Self::sub_rows(&spec, &query).get(shown_ix).cloned() else { return };
        self.close_menu(cx);
        action(self, cx);
        cx.notify();
    }

    /// Enter in a submenu runs the focused row, or the first match when the filter has focus.
    fn submenu_activate(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((spec, query)) = self.open_sub_spec() else { return false };
        if self.menu_find_focused() {
            let (lead, matched) = Self::sub_counts(&spec, &query);
            if !query.trim().is_empty() && matched > 0 {
                self.run_sub_row(lead, cx);
            }
            return true;
        }
        match self.menu.as_ref().and_then(|m| m.sub.as_ref()).and_then(|s| s.focused) {
            Some(i) => {
                self.run_sub_row(i, cx);
                true
            }
            None => false,
        }
    }

    /// A song menu. `remove` is "Remove from This Playlist" for rows of an editable playlist.
    pub fn song_rows(&self, songs: &[u32], remove: Option<MenuRow>) -> Vec<MenuRow> {
        if songs.len() > 1 {
            let (a, b, c, n) = (songs.to_vec(), songs.to_vec(), songs.to_vec(), songs.len());
            let downloaded = songs.iter().all(|s| self.is_downloaded(*s));
            let d = songs.to_vec();
            let mut rows = vec![
                MenuRow::item(trf("Play {n} Next", &[("n", &crate::i18n::fmt_int(n as i64))]), "queue-regular", move |this, cx| this.play_next(&a, cx)),
                MenuRow::item(trf("Add {n} to Queue", &[("n", &crate::i18n::fmt_int(n as i64))]), "queue-regular", move |this, cx| this.add_to_queue(&b, cx)),
                self.add_to_playlist_row(trf("Add {n} to Playlist", &[("n", &crate::i18n::fmt_int(n as i64))]), c, SharedString::default()),
            ];
            rows.extend(remove);
            rows.push(self.share_songs_row(songs));
            rows.push(if downloaded {
                MenuRow::item(tr("Remove Downloads"), "x-regular", move |this, cx| this.set_downloaded(&d, false, cx))
            } else {
                MenuRow::item(tr("Download Selected"), "arrow-down-regular", move |this, cx| this.set_downloaded(&d, true, cx))
            });
            return rows;
        }
        let Some(&song_ix) = songs.first() else { return Vec::new() };
        let Some(song) = self.song_peek(song_ix) else { return Vec::new() };
        let mut rows = vec![
            MenuRow::item(tr("Play Next"), "arrow-bend-up-right-regular", move |this, cx| this.play_next(&[song_ix], cx)),
            MenuRow::item(tr("Add to Queue"), "queue-regular", move |this, cx| this.add_to_queue(&[song_ix], cx)),
            self.add_to_playlist_row(tr("Add to Playlist"), vec![song_ix], song.title.clone()),
        ];
        rows.extend(remove);
        if self.is_downloaded(song_ix) {
            rows.push(MenuRow::item(tr("Remove Download"), "x-regular", move |this, cx| this.set_downloaded(&[song_ix], false, cx)));
        }
        rows.push(MenuRow::Divider);
        let album = song.album_id.clone();
        rows.push(MenuRow::item(tr("Go to Album"), "disc-regular", move |this, cx| this.navigate(Route::Album(album.clone()), cx)));
        // A joined credit like "A & B" isn't an artist, so use the song's artist list.
        let artist = song.artists.iter().find_map(|a| self.artist_named(a)).or_else(|| self.artist_named(&song.artist)).cloned();
        if let Some(artist) = artist {
            let id = artist.id.clone();
            rows.push(MenuRow::item(tr("Go to Artist"), "microphone-stage-regular", move |this, cx| this.navigate(Route::Artist(id.clone()), cx)));
        }
        // Hidden once the server says it has nothing for this song. Unknown stays offered, since asking is slow and
        // only happens when the row is chosen.
        if !self.no_song_radio.contains(&song.id) {
            rows.push(MenuRow::item(tr("Start Song Radio"), "radio-regular", move |this, cx| this.start_song_radio(song_ix, cx)));
        }
        rows.push(self.share_songs_row(&[song_ix]));
        rows.push(self.info_row(crate::view_info::InfoTarget::Song(song.id.clone())));
        rows
    }

    /// Add to Playlist: a filterable submenu with New Playlist, then the user's own regular playlists, most recently
    /// changed first. `name` is the new playlist's starting name.
    pub fn add_to_playlist_row(&self, label: impl Into<SharedString>, songs: Vec<u32>, name: SharedString) -> MenuRow {
        if songs.is_empty() {
            return MenuRow::disabled(label, "list-plus-regular", tr("No songs to add"));
        }
        let lead_songs = songs.clone();
        let lead = vec![MenuRow::item(tr("New Playlist"), "plus-regular", move |this, cx| this.open_new_playlist_with(Some(lead_songs.clone()), name.to_string(), cx))];
        let items = self
            .playlist_targets()
            .into_iter()
            .map(|(id, title)| {
                let songs = songs.clone();
                let target = title.clone();
                MenuRow::plain(title, move |this, cx| this.add_songs_to_playlist(id.clone(), target.clone(), songs.clone(), cx))
            })
            .collect();
        MenuRow::submenu(label, "list-plus-regular", Submenu { lead, items, find: Some(tr("Find a playlist")), empty: Some(tr("No playlists match")) })
    }

    /// Playlists that can take songs: the user's own regular ones, most recently changed first.
    fn playlist_targets(&self) -> Vec<(SharedString, SharedString)> {
        if let Some(core) = self.core.as_ref() {
            return core.lib.playlist_targets();
        }
        self.playlists_full.iter().filter(|p| p.mine && !p.readonly).map(|p| (p.id.clone(), p.name.clone())).collect()
    }

    /// "Remove from This Playlist" for these positions in the playlist's shown list.
    pub fn remove_row(&self, places: Vec<usize>) -> Option<MenuRow> {
        let target = self.playlist_remove.clone()?;
        let label = if places.len() > 1 { trf("Remove {n} from This Playlist", &[("n", &crate::i18n::fmt_int(places.len() as i64))]) } else { tr("Remove from This Playlist").to_string() };
        Some(MenuRow::item(label, "minus-circle-regular", move |this, cx| {
            let shown: Vec<usize> = places.iter().filter_map(|&p| target.places.get(p).copied()).collect();
            match this.core.as_ref() {
                Some(core) => core.lib.remove_from_playlist(target.id.to_string(), target.name.to_string(), shown),
                None => this.show_toast(tr("Playlists can't be changed without the server."), cx),
            }
            this.selection.clear();
        }))
    }

    /// "Remove from Queue" for these rows of the queue view.
    pub fn queue_remove_row(&self, positions: Vec<usize>) -> MenuRow {
        let label = if positions.len() > 1 { trf("Remove {n} from Queue", &[("n", &crate::i18n::fmt_int(positions.len() as i64))]) } else { tr("Remove from Queue").to_string() };
        MenuRow::item(label, "minus-circle-regular", move |this, cx| this.queue_remove(&positions, cx))
    }

    pub fn open_song_menu(&mut self, songs: Vec<u32>, position: Point<Pixels>, align_end: bool, cx: &mut Context<Self>) {
        let rows = self.song_rows(&songs, None);
        self.open_menu(rows, position, align_end, cx);
    }

    /// A song menu for rows of a page list. On an editable playlist it also offers Remove from This Playlist.
    pub fn open_list_song_menu(&mut self, list: usize, places: Vec<usize>, position: Point<Pixels>, align_end: bool, cx: &mut Context<Self>) {
        let Some(all) = self.page.lists.get(list).cloned() else { return };
        let songs: Vec<u32> = places.iter().filter_map(|&p| all.get(p).copied()).collect();
        let remove = if self.playlist_remove.as_ref().is_some_and(|r| r.list == list) { self.remove_row(places) } else { None };
        let rows = self.song_rows(&songs, remove);
        self.open_menu(rows, position, align_end, cx);
    }

    /// The player menu for the dock, bar and expanded player: the song's actions, then the player's. Opened from More
    /// (`align_end`) or by right-clicking the title.
    pub fn open_player_menu(&mut self, position: Point<Pixels>, align_end: bool, cx: &mut Context<Self>) {
        let Some(song_ix) = self.player.song_ix else { return };
        let mut rows = if self.player.station.is_some() {
            vec![MenuRow::item(tr("Stop Station"), "stop-regular", |this, cx| this.core_stop_station(cx))]
        } else {
            self.song_rows(&[song_ix as u32], None)
        };
        rows.push(MenuRow::Divider);
        rows.push(MenuRow::item(tr("Mini Player"), "picture-in-picture-regular", |this, cx| this.open_mini_player(cx)).with_shortcut(Shell::mini_shortcut_label()));
        let on = self.private_mode;
        rows.push(MenuRow::new_item(tr("Private Mode").into(), Some("eye-slash-regular"), Some(on), None, Some(Rc::new(move |this: &mut Shell, cx: &mut Context<Shell>| this.set_private_mode(!on, cx))), false));
        self.open_menu(rows, position, align_end, cx);
    }

    pub fn open_output_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.ensure_outputs(cx);
        self.airplay_menu_opened(cx);
        let rows = self.output_menu_rows();
        self.open_menu(rows, position, true, cx);
        if let Some(menu) = self.menu.as_mut() {
            menu.outputs = true;
        }
    }

    /// Updates an open output menu with the new device list without closing it.
    pub fn refresh_output_menu(&mut self, cx: &mut Context<Self>) {
        if !self.menu.as_ref().is_some_and(|m| m.outputs) {
            return;
        }
        let rows = self.output_menu_rows();
        if let Some(menu) = self.menu.as_mut() {
            menu.row_bounds = rows.iter().map(|_| Rc::default()).collect();
            menu.focused = menu.focused.filter(|&i| i < rows.len()).or_else(|| rows.iter().position(MenuRow::enabled));
            menu.rows = rows;
        }
        cx.notify();
    }

    fn output_menu_rows(&self) -> Vec<MenuRow> {
        let current = self.prefs.output_device.clone();
        let mut rows = vec![MenuRow::check(tr("System default"), current.is_none(), |this, cx| this.set_output_device(None, cx))];
        for (id, name) in self.outputs.clone() {
            let on = current.as_deref() == Some(id.as_str());
            let chosen = id.clone();
            rows.push(MenuRow::check(name, on, move |this, cx| this.set_output_device(Some(chosen.clone()), cx)));
        }
        // AirPlay receivers come after this computer's outputs, under their own heading.
        rows.extend(self.airplay_menu_rows());
        rows
    }

    /// Inserts songs at `offset` after the playing song, into Next in Queue (`queued`) or Next from. Used by Play Next,
    /// Add to Queue and drops.
    pub fn queue_songs_at(&mut self, songs: &[u32], offset: usize, queued: bool, cx: &mut Context<Self>) {
        if self.core.is_some() {
            self.core_queue_at(songs, offset, queued, cx);
            self.core_save_session();
            cx.notify();
            return;
        }
        if self.player.queue.is_empty() {
            self.player.queue = self.order.clone();
        }
        let at = self.core_queue_at(songs, offset, queued, cx);
        self.player.queue.splice(at..at, songs.iter().copied());
        self.core_save_session();
        cx.notify();
    }

    pub fn play_next(&mut self, songs: &[u32], cx: &mut Context<Self>) {
        self.queue_songs_at(songs, 0, true, cx);
        self.show_toast(if songs.len() == 1 { tr("Playing next").to_string() } else { trf("{n} songs playing next", &[("n", &crate::i18n::fmt_int(songs.len() as i64))]) }, cx);
    }

    pub fn add_to_queue(&mut self, songs: &[u32], cx: &mut Context<Self>) {
        let run = self.player.queued_run;
        self.queue_songs_at(songs, run, true, cx);
        self.show_toast(if songs.len() == 1 { tr("Added to queue").to_string() } else { trf("{n} songs added to queue", &[("n", &crate::i18n::fmt_int(songs.len() as i64))]) }, cx);
    }

    /// A row's running download, showing its percent. Click to cancel.
    pub fn download_progress_badge(&self, key: (&'static str, usize), song_ix: u32, fraction: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        div()
            .id(key)
            .w(t.s(ACTION_W))
            .flex_none()
            .flex()
            .justify_center()
            .text_size(t.text(TEXT_SM))
            .text_color(t.t2())
            .child(format!("{}%", (fraction * 100.0).round() as u32))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.set_downloaded(&[song_ix], false, cx);
            }))
            .into_any_element()
    }

    pub fn set_downloaded(&mut self, songs: &[u32], on: bool, cx: &mut Context<Self>) {
        if self.core.is_some() {
            self.core_set_downloaded(songs, on, cx);
            return;
        }
        for s in songs {
            if on {
                self.downloaded.insert(*s);
            } else {
                self.downloaded.remove(s);
            }
        }
        cx.notify();
    }

    pub fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_toast_for(message, std::time::Duration::from_millis(TOAST_MS), cx);
    }

    pub fn clear_toast(&mut self) {
        self.toast = None;
        self.toast_generation += 1;
    }

    /// A toast for slow work. It stays until `clear_toast` or the duration runs out.
    pub fn show_toast_for(&mut self, message: impl Into<SharedString>, duration: std::time::Duration, cx: &mut Context<Self>) {
        self.toast = Some(Toast { text: message.into(), action: None, duration });
        self.hold_toast(false, cx);
    }

    /// A toast with an action button like Undo, which runs `run` and closes it. It stays longer (`TOAST_ACTION_MS`) so
    /// there's time to click.
    pub fn show_toast_action(&mut self, message: impl Into<SharedString>, label: impl Into<SharedString>, run: impl Fn(&mut Shell, &mut Context<Shell>) + 'static, cx: &mut Context<Self>) {
        let action = ToastAction { label: label.into(), run: Rc::new(run) };
        self.toast = Some(Toast { text: message.into(), action: Some(action), duration: std::time::Duration::from_millis(TOAST_ACTION_MS) });
        self.hold_toast(false, cx);
    }

    /// Hovering a toast holds it. Leaving restarts its timer.
    pub fn hold_toast(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.toast_generation += 1;
        let Some(duration) = self.toast.as_ref().map(|t| t.duration) else { return };
        if !hold {
            let generation = self.toast_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(duration).await;
                let _ = this.update(cx, |this, cx| {
                    if this.toast_generation == generation {
                        this.toast = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Moves the keyboard highlight in the open menu, wrapping and skipping disabled rows.
    pub fn menu_move(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        let enabled: Vec<usize> = menu.rows.iter().enumerate().filter(|(_, r)| matches!(r, MenuRow::Item { action: Some(_), .. })).map(|(i, _)| i).collect();
        if enabled.is_empty() {
            return;
        }
        let at = menu.focused.and_then(|f| enabled.iter().position(|e| *e == f));
        let next = match at {
            Some(i) => (i as i32 + delta).rem_euclid(enabled.len() as i32) as usize,
            None if delta >= 0 => 0,
            None => enabled.len() - 1,
        };
        menu.focused = Some(enabled[next]);
        menu.keyboard = true;
        menu.sub = None;
        self.leave_menu_find();
        cx.notify();
    }

    /// Jumps to the first or last enabled row.
    fn menu_jump(&mut self, last: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        let enabled = menu.rows.iter().enumerate().filter(|(_, r)| matches!(r, MenuRow::Item { action: Some(_), .. })).map(|(i, _)| i);
        let target = if last { enabled.last() } else { enabled.clone().next() };
        if let Some(target) = target {
            menu.focused = Some(target);
            menu.keyboard = true;
            cx.notify();
        }
    }

    /// Keys while a menu is open. The menu is a layer, so nothing else sees them. A keyboard-opened submenu takes them
    /// first.
    pub fn menu_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        if self.panel_scroll(event.keystroke.key.as_str(), cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let in_sub = self.menu.as_ref().and_then(|m| m.sub.as_ref()).is_some_and(|s| s.focused.is_some()) || self.menu_find_focused();
        if in_sub {
            let field = self.menu_find_focused();
            let at_start = self.field_caret_at_start();
            match key {
                "down" => self.submenu_move(1, None, cx),
                "up" => self.submenu_move(-1, None, cx),
                "home" | "end" if !field => self.submenu_move(0, Some(key == "end"), cx),
                "left" if !field || at_start => {
                    self.submenu_back(cx);
                }
                "tab" => self.close_menu(cx),
                "space" if !field => {
                    self.submenu_activate(cx);
                }
                _ if field => self.field_key(&crate::ui_controls::TextTarget::MenuFind, event, cx),
                _ => self.type_into_menu_find(event, cx),
            }
            return;
        }
        match key {
            "down" => self.menu_move(1, cx),
            "up" => self.menu_move(-1, cx),
            "home" => self.menu_jump(false, cx),
            "end" => self.menu_jump(true, cx),
            "space" => self.menu_activate(cx),
            "right" if !mods.modified() => {
                if let Some(i) = self.menu.as_ref().and_then(|m| m.focused) {
                    self.open_submenu(i, true, cx);
                }
            }
            "tab" => self.close_menu(cx),
            _ => self.type_into_menu_find(event, cx),
        }
    }

    /// A letter typed while a submenu with a filter is open goes to the filter.
    fn type_into_menu_find(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let mods = event.keystroke.modifiers;
        let Some(text) = event.keystroke.key_char.clone().filter(|c| !c.chars().any(char::is_control)) else { return };
        if mods.control || mods.platform || mods.alt {
            return;
        }
        let Some((spec, query)) = self.open_sub_spec() else { return };
        if spec.find.is_none() {
            return;
        }
        if let Some(sub) = self.menu.as_mut().and_then(|m| m.sub.as_mut()) {
            sub.focused = None;
        }
        // Set the text first, since focusing a field puts the caret at the end.
        self.set_menu_find_text(query + &text, cx);
        self.focus_field(crate::ui_controls::TextTarget::MenuFind, cx);
    }

    pub fn menu_activate(&mut self, cx: &mut Context<Self>) {
        if self.submenu_activate(cx) {
            return;
        }
        let Some(menu) = self.menu.as_ref() else { return };
        let Some(index) = menu.focused else { return };
        let (action, toggle, sub) = match menu.rows.get(index) {
            Some(MenuRow::Item { action: Some(action), toggle, submenu, .. }) => (action.clone(), *toggle, submenu.is_some()),
            _ => return,
        };
        if sub {
            self.open_submenu(index, true, cx);
            return;
        }
        self.run_menu_row(index, action, toggle, cx);
    }

    /// Runs a row's action. A switch row stays open and shows its new state, and others close the menu.
    fn run_menu_row(&mut self, index: usize, action: MenuAction, toggle: bool, cx: &mut Context<Self>) {
        if toggle {
            if let Some(MenuRow::Item { checked: Some(on), .. }) = self.menu.as_mut().and_then(|m| m.rows.get_mut(index)) {
                *on = !*on;
            }
        } else {
            self.close_menu(cx);
        }
        action(self, cx);
        cx.notify();
    }

    /// One menu or submenu row without its handlers. `lit` keeps the hover fill for the keyboard's row or a row with
    /// its submenu open.
    pub(crate) fn menu_row(&self, id: (&'static str, usize), row: &MenuRow, lit: bool, icon_slots: bool) -> Option<gpui::Stateful<gpui::Div>> {
        let t = self.theme;
        let MenuRow::Item { label, icon: icon_name, checked, reason, busy, action, toggle, shortcut, count, submenu } = row else { return None };
        let enabled = action.is_some();
        let color: gpui::Hsla = if enabled { t.t1().into() } else { t.t3().into() };
        let mut el = div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(t.s(11.0))
            .min_h(t.s(ROW_H))
            .px(t.s(11.0))
            .py(t.s(if reason.is_some() { 6.0 } else { 0.0 }))
            .rounded(t.s(10.0))
            .text_color(color)
            .when(lit, |d| d.bg(t.hover()));
        if let Some(name) = icon_name {
            el = el.child(icon(&t, name, 17.0, color));
        } else if icon_slots {
            el = el.child(div().flex_none().size(t.s(17.0)));
        }
        // An enabled row's second line is a detail, and a disabled row's says why.
        let note = if enabled { t.t2() } else { t.t3() };
        let text = div().flex_1().min_w(px(0.0)).child(div().truncate().child(label.clone())).when_some(reason.clone(), |d, reason| d.child(div().text_size(t.text(TEXT_XS)).line_height(relative(1.25)).text_color(note).child(reason)));
        el = el.child(text).when_some(shortcut.clone(), |d, keys| d.child(div().ml(t.s(18.0)).flex_none().text_size(t.text(TEXT_SM)).text_color(t.t3()).child(keys)));
        el = el.when_some(count.clone(), |d, n| d.child(div().flex_none().text_size(t.text(TEXT_XS)).text_color(t.t3()).child(n)));
        el = if *busy {
            el.child(div().ml(t.s(18.0)).flex_none().child(crate::ui::spinner(&t, (id.0, id.1 + 10_000), 15.0, t.t2())))
        } else if submenu.is_some() {
            el.child(div().ml(t.s(18.0)).flex_none().child(icon(&t, "caret-right-bold", 13.0, t.t2())))
        } else if *toggle {
            // The settings switch one step smaller, to fit inside the row.
            el.child(crate::ui_controls::switch_look(&t, *checked == Some(true), SWITCH_COMPACT).ml(t.s(18.0)))
        } else {
            el.when(*checked == Some(true), |d| d.child(div().size(t.s(7.0)).rounded_full().bg(t.accent_hsla())))
        };
        if enabled {
            el = el.hover(|s| s.bg(t.hover()));
        }
        Some(el)
    }

    pub fn render_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.theme;
        let menu = self.menu.as_ref()?;
        let position = menu.position;
        let focused = menu.focused.filter(|_| menu.keyboard);
        let open_sub = menu.sub.as_ref().map(|s| s.row);
        let rows = menu.rows.clone();
        let (bounds, row_bounds) = (menu.bounds.clone(), menu.row_bounds.clone());
        let panel = menu.panel.as_ref().map(|p| self.render_menu_panel(p, !rows.is_empty()));
        let mut list = crate::ui::float_panel(&t).min_w(t.s(210.0)).max_w(t.s(380.0)).p(t.s(MENU_PAD)).flex().flex_col().child(track_bounds(bounds));
        list = match panel {
            // The panel scrolls itself, so scrolling mustn't close the menu.
            Some(panel) => list.w(t.s(POP_W)).child(panel),
            None => list.on_scroll_wheel(cx.listener(|this, _: &gpui::ScrollWheelEvent, _, cx| this.close_menu(cx))),
        };
        // When some rows have icons, the rest keep an empty slot so labels line up.
        let icon_slots = rows.iter().any(|r| matches!(r, MenuRow::Item { icon: Some(_), .. }));
        for (i, row) in rows.iter().enumerate() {
            match row {
                MenuRow::Divider => {
                    list = list.child(div().h(px(1.0)).mx(t.s(8.0)).my(t.s(5.0)).bg(t.line()));
                }
                MenuRow::Heading { label, icon: glyph, busy } => {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(t.s(7.0))
                            .h(t.s(MENU_HEADING_H))
                            .px(t.s(11.0))
                            .text_size(t.text(TEXT_SM))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.t_section())
                            .when_some(*glyph, |d, g| d.child(icon(&t, g, 15.0, t.t_section())))
                            .child(div().flex_1().child(label.clone()))
                            .when(*busy, |d| d.child(crate::ui::spinner(&t, ("menu-heading-spin", i), 13.0, t.t_section()))),
                    );
                }
                MenuRow::Header { initial, name, host } => {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(t.s(11.0))
                            .min_w(t.s(240.0))
                            .pt(t.s(8.0))
                            .pb(t.s(9.0))
                            .px(t.s(11.0))
                            .child(div().flex().items_center().justify_center().flex_none().size(t.s(34.0)).rounded_full().bg(t.glass_on()).shadow(glass_edge()).font_weight(FontWeight::SEMIBOLD).child(initial.clone()))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w(px(0.0))
                                    .line_height(relative(1.3))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(name.clone()))
                                    .child(div().text_size(t.text(TEXT_SM)).text_color(t.t3()).truncate().child(host.clone())),
                            ),
                    );
                }
                MenuRow::Item { action, toggle, submenu, .. } => {
                    let Some(mut el) = self.menu_row(("menu-row", i), row, focused == Some(i) || open_sub == Some(i), icon_slots) else { continue };
                    if let Some(store) = row_bounds.get(i) {
                        el = el.relative().child(track_bounds(store.clone()));
                    }
                    let has_sub = submenu.is_some();
                    if let Some(action) = action.clone() {
                        let toggle = *toggle;
                        el = el
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if !*hovered {
                                    return;
                                }
                                if has_sub {
                                    this.open_submenu(i, false, cx);
                                } else {
                                    this.submenu_close_soon(cx);
                                }
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if has_sub {
                                    this.open_submenu(i, true, cx);
                                } else {
                                    this.run_menu_row(i, action.clone(), toggle, cx);
                                }
                            }));
                    } else {
                        el = el.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.submenu_close_soon(cx);
                            }
                        }));
                    }
                    list = list.child(el);
                }
            }
        }
        let anchor = if self.menu.as_ref().is_some_and(|m| m.align_end) { gpui::Anchor::TopRight } else { gpui::Anchor::TopLeft };
        let sub = self.render_submenu(window, cx);
        let scrim = div()
            .id("menu-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| this.close_menu(cx)))
            .on_mouse_down(gpui::MouseButton::Right, cx.listener(|this, _, _, cx| this.close_menu(cx)))
            .child(anchored().position(position).anchor(anchor).snap_to_window_with_margin(self.float_margin()).child(list))
            .children(sub);
        Some(deferred(scrim).with_priority(10).into_any_element())
    }

    /// The open submenu beside its row, on the right or on the left near the window edge. It's level with its row,
    /// moved up only as needed, and its list scrolls past `SUBMENU_MAX_H`.
    fn render_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.theme;
        let (spec, query) = self.open_sub_spec()?;
        let menu = self.menu.as_ref()?;
        let sub = menu.sub.as_ref()?;
        let parent = *menu.bounds.borrow();
        let row = *menu.row_bounds.get(sub.row)?.borrow();
        // Not drawn yet. Placed on the next frame.
        if parent.size.width <= px(0.0) {
            cx.notify();
            return None;
        }
        let focused = sub.focused.filter(|_| menu.keyboard);
        let scroll = sub.scroll.clone();
        let shown = Self::sub_rows(&spec, &query);
        let (lead, matched) = Self::sub_counts(&spec, &query);
        let f = |v: f32| f32::from(t.s(v));
        let viewport = window.viewport_size();
        let (top_limit, bottom_limit) = (f32::from(self.edge.top) + EDGE, f32::from(viewport.height - self.edge.bottom) - EDGE);
        let (left_limit, right_limit) = (f32::from(self.edge.left) + EDGE, f32::from(viewport.width - self.edge.right) - EDGE);
        let width = f(SUBMENU_W);
        let find_h = if spec.find.is_some() { f(FILTER_H + FILTER_GAP) } else { 0.0 };
        let fixed = f(MENU_PAD) * 2.0 + find_h + f(ROW_H) * lead as f32;
        let max_h = f(SUBMENU_MAX_H).min(bottom_limit - top_limit);
        let list_natural = f(ROW_H) * matched.max(1) as f32;
        let list_h = list_natural.min((max_h - fixed).max(f(ROW_H)));
        let height = fixed + list_h;
        let right = f32::from(parent.right()) + f(SUBMENU_GAP);
        let x = if right + width <= right_limit { right } else { (f32::from(parent.left()) - f(SUBMENU_GAP) - width).max(left_limit) };
        let y = (f32::from(row.top()) - f(MENU_PAD)).min(bottom_limit - height).max(top_limit);
        // Playlist names start at the row's edge, with no empty icon slots.
        let icon_slots = false;
        let row_el = |this: &mut Shell, k: usize, row: &MenuRow, cx: &mut Context<Shell>| {
            this.menu_row(("submenu-row", k), row, focused == Some(k), icon_slots).map(|el| el.on_click(cx.listener(move |this, _, _, cx| this.run_sub_row(k, cx))))
        };
        let mut panel = crate::ui::float_panel(&t)
            .id("submenu")
            .w(px(width))
            .h(px(height))
            .p(t.s(MENU_PAD))
            .flex()
            .flex_col()
            .text_color(t.t1())
            .on_hover(cx.listener(|this, inside: &bool, _, cx| {
                if let Some(menu) = this.menu.as_mut() {
                    menu.sub_close += 1;
                    if let Some(sub) = menu.sub.as_mut() {
                        sub.pointer_inside = *inside;
                    }
                }
                cx.notify();
            }))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation());
        if let Some(placeholder) = spec.find {
            let line = self.text_line(&crate::ui_controls::TextTarget::MenuFind, &query, placeholder, "submenu-find-text", window);
            panel = panel.child(crate::ui::float_filter(&t, "submenu-find", line).on_click(cx.listener(|this, _, _, cx| {
                if let Some(sub) = this.menu.as_mut().and_then(|m| m.sub.as_mut()) {
                    sub.focused = None;
                }
                this.focus_field(crate::ui_controls::TextTarget::MenuFind, cx);
            })));
        }
        for (k, row) in shown.iter().enumerate().take(lead) {
            panel = panel.children(row_el(self, k, row, cx));
        }
        let mut list = div().id("submenu-list").flex().flex_col().h(px(list_h)).overflow_y_scroll().track_scroll(&scroll);
        if matched == 0 {
            list = list.children(spec.empty.map(|e| div().px(t.s(11.0)).py(t.s(10.0)).text_size(t.text(TEXT_SM)).text_color(t.t2()).child(e)));
        }
        for (k, row) in shown.iter().enumerate().skip(lead) {
            list = list.children(row_el(self, k, row, cx));
        }
        panel = panel.child(list);
        Some(anchored().position(point(px(x), px(y))).child(panel).into_any_element())
    }

    fn render_menu_panel(&self, panel: &MenuPanel, rows_follow: bool) -> AnyElement {
        let t = self.theme;
        let body = rule_list(&t, panel.sentence.clone(), panel.heading.clone(), &panel.lines, panel.footnote.clone()).px(t.s(10.0)).pt(t.s(8.0)).pb(t.s(6.0));
        div()
            .id("menu-panel")
            .max_h(t.s(POP_MAX_H))
            .overflow_y_scroll()
            .track_scroll(&panel.scroll)
            .child(body)
            .when(rows_follow, |d| d.border_b_1().border_color(t.line()).mb(t.s(6.0)))
            .into_any_element()
    }

    /// Toasts are centered in the visible content column, between the sidebar and the dock, `TOAST_GAP` above the bar
    /// or the window inset.
    pub fn render_toast(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.theme;
        let toast = self.toast.clone()?;
        let inset = t.s(WINDOW_INSET);
        let left = if layout.sidebar_visible && !layout.sidebar_overlay { layout.sidebar_width + inset } else { inset };
        let docked = layout.player && layout.mode == PlayerMode::Dock;
        let right = if docked { layout.dock_width + inset } else { inset };
        let barred = layout.player && layout.mode == PlayerMode::Bar;
        let bottom = if barred { t.s(BAR_H) + inset * 2.0 } else { inset };
        let pad = t.s(TOAST_PAD);
        let mut surface = crate::ui::dialog_surface(&t, t.s(TOAST_H / 2.0))
            .id("toast")
            .h(t.s(TOAST_H))
            .min_w(px(0.0))
            .flex()
            .items_center()
            .gap(pad)
            .px(pad)
            .text_size(t.text(TEXT_SM))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.hold_toast(*hovered, cx)))
            .child(div().min_w(px(0.0)).truncate().child(toast.text));
        if let Some(action) = toast.action {
            // The button sits at the right, as far from the edge as from the top and bottom.
            let run = action.run.clone();
            surface = surface.pr(t.s((TOAST_H - BUTTON_SMALL_H) / 2.0)).child(
                crate::ui_controls::small_glass_action(&t, "toast-action").child(action.label).on_click(cx.listener(move |this, _, _, cx| {
                    this.clear_toast();
                    run(this, cx);
                    cx.notify();
                })),
            );
        }
        Some(div().absolute().left(left).right(right).bottom(bottom + t.s(TOAST_GAP)).px(inset).flex().justify_center().child(surface).into_any_element())
    }

    /// Opens a toolbar's sort or filter menu at the button that was clicked.
    pub fn open_toolbar_menu(&mut self, rows: Vec<MenuRow>, event: &gpui::ClickEvent, cx: &mut Context<Self>) {
        let position = event.position() + point(px(0.0), self.theme.s(14.0));
        self.open_menu(rows, position, true, cx);
    }

    pub fn invalidate_nothing(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Builds a text block for pages, usable as a `Block::View`.
    pub fn plain_block(render: impl Fn(&mut Shell, &crate::shell::Layout, &mut Window, &mut Context<Shell>) -> AnyElement + 'static) -> Block {
        Block::View(Rc::new(render))
    }
}

/// A toast message, with an optional action.
#[derive(Clone)]
pub struct Toast {
    pub text: SharedString,
    pub action: Option<ToastAction>,
    /// How long it stays while the pointer isn't on it.
    pub duration: std::time::Duration,
}

/// A toast's action, like Undo. It runs and the toast closes.
#[derive(Clone)]
pub struct ToastAction {
    pub label: SharedString,
    pub run: Rc<dyn Fn(&mut Shell, &mut Context<Shell>)>,
}

/// A smart playlist's rules: how they match, one rule per line, then the order and limit. Used by the rules panel and
/// Get Info.
pub fn rule_list(t: &crate::theme::Theme, sentence: Option<SharedString>, heading: Option<SharedString>, lines: &[SharedString], footnote: Option<SharedString>) -> gpui::Div {
    let line = |text: SharedString, first: bool| {
        div().py(t.s(8.0)).text_size(t.text(TEXT_MD)).line_height(relative(1.3)).text_color(t.t1()).when(!first, |d| d.border_t_1().border_color(t.line())).child(text)
    };
    div()
        .flex()
        .flex_col()
        .when_some(sentence, |d, s| d.child(div().pb(t.s(10.0)).mb(t.s(10.0)).border_b_1().border_color(t.line()).text_size(t.text(TEXT_MD)).line_height(relative(1.35)).text_color(t.t1()).child(s)))
        .when_some(heading, |d, h| d.child(div().mb(t.s(4.0)).text_size(t.text(TEXT_XS)).text_color(t.t2()).child(h)))
        .children(lines.iter().enumerate().map(|(i, text)| line(text.clone(), i == 0)))
        .when_some(footnote, |d, f| d.child(div().mt(t.s(6.0)).pt(t.s(10.0)).border_t_1().border_color(t.line()).text_size(t.text(TEXT_XS)).line_height(relative(1.4)).text_color(t.t2()).child(f)))
}
