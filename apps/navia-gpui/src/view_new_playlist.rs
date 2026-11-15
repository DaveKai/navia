//! The Playlists page's New menu, the New Playlist dialog, and the cover rows in a playlist's Edit menu. Covers only
//! come from the server, either an upload or Navidrome's album grid.

use crate::i18n::{tr, trf};
use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString};

use crate::focus::FocusStop;
use crate::pages::Route;
use crate::shell::Shell;
use crate::theme::size::*;
use crate::ui_controls::{DialogButton, TextTarget, dialog_button};
use crate::view_menu::MenuRow;

pub struct NewPlaylist {
    pub name: String,
    /// Set from Add to Playlist. The new playlist holds these songs and the app stays where it is.
    pub songs: Option<Vec<u32>>,
    /// Set from Save As in the smart playlist editor. These rules, in Navidrome's JSON, become a new smart playlist
    /// owned by the user.
    pub smart: Option<String>,
    pub creating: bool,
    pub error: Option<SharedString>,
}

impl Shell {
    /// The artwork key for a playlist, in the shared thumbnail path.
    pub fn playlist_art_key(&self, id: &str) -> SharedString {
        SharedString::from(format!("pl:{id}"))
    }

    pub fn playlist_cover_rows(&self, id: &SharedString) -> Vec<MenuRow> {
        self.playlist_image_rows(id)
    }

    pub fn new_playlist_rows(&self) -> Vec<MenuRow> {
        vec![
            MenuRow::item(tr("New Playlist"), "playlist-regular", |this, cx| this.open_new_playlist(cx)),
            MenuRow::item(tr("New Smart Playlist"), "lightning-fill", |this, cx| this.navigate(Route::SmartEditor(None), cx)),
        ]
    }

    pub fn open_new_playlist(&mut self, cx: &mut Context<Self>) {
        self.open_new_playlist_with(None, String::new(), cx);
    }

    /// New Playlist from Add to Playlist, holding `songs` and named `name` to start.
    pub fn open_new_playlist_with(&mut self, songs: Option<Vec<u32>>, name: String, cx: &mut Context<Self>) {
        self.new_playlist = Some(NewPlaylist { name, songs, smart: None, creating: false, error: None });
        self.focus_field(TextTarget::NewPlaylistName, cx);
        cx.notify();
    }

    /// Save As. Names a copy of the editor's rules, saves it as a new smart playlist, and opens it.
    pub fn open_smart_save_as(&mut self, name: String, criteria: String, cx: &mut Context<Self>) {
        self.new_playlist = Some(NewPlaylist { name, songs: None, smart: Some(criteria), creating: false, error: None });
        self.focus_field(TextTarget::NewPlaylistName, cx);
        cx.notify();
    }

    pub fn new_playlist_submit(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.new_playlist.as_mut() else { return };
        let name = dialog.name.trim().to_string();
        if name.is_empty() || dialog.creating {
            return;
        }
        match self.core.as_ref() {
            Some(core) => {
                dialog.creating = true;
                dialog.error = None;
                match (dialog.smart.clone(), dialog.songs.clone()) {
                    (Some(criteria), _) => core.lib.save_smart_playlist(None, name, criteria),
                    (None, Some(songs)) => core.lib.create_playlist_with(name, songs),
                    (None, None) => core.lib.create_playlist(name),
                }
            }
            None => dialog.error = Some(tr("Creating a playlist needs the server.").into()),
        }
        cx.notify();
    }

    pub fn on_playlist_created(&mut self, id: String, name: String, with_songs: bool, cx: &mut Context<Self>) {
        self.new_playlist = None;
        self.controls.text = None;
        if let Some(library) = self.library.clone() {
            self.playlists = library.playlists();
            self.refresh_sidebar_order();
            self.load_catalog(&library);
        }
        if std::mem::take(&mut self.saving_mix) {
            self.show_toast(trf("Saved as {name}", &[("name", &name)]), cx);
            cx.notify();
            return;
        }
        if with_songs {
            self.show_toast(trf("Added to {name}", &[("name", &name)]), cx);
            cx.notify();
            return;
        }
        self.show_toast(trf("Created {name}", &[("name", &name)]), cx);
        if !id.is_empty() {
            self.navigate(Route::Playlist(id.into()), cx);
        }
        cx.notify();
    }

    pub fn on_playlist_create_failed(&mut self, message: String, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.saving_mix) {
            self.show_toast(message.clone(), cx);
        }
        if let Some(dialog) = self.new_playlist.as_mut() {
            dialog.creating = false;
            dialog.error = Some(message.into());
        }
        cx.notify();
    }

    pub fn render_new_playlist(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.theme;
        let dialog = self.new_playlist.as_ref()?;
        let (name, creating, error, smart) = (dialog.name.clone(), dialog.creating, dialog.error.clone(), dialog.smart.is_some());
        let can_create = !name.trim().is_empty() && !creating;
        let line = self.field_line(&TextTarget::NewPlaylistName, &name, tr("Playlist Name"), "new-playlist-name", t.text(TEXT_MD));
        let focused = self.focused_target() == Some(TextTarget::NewPlaylistName);
        let field = crate::ui::dialog_field(&t, "new-playlist-field", focused, None, line)
            .on_click(cx.listener(|this, _, _, cx| this.focus_field(TextTarget::NewPlaylistName, cx)));
        let close = |this: &mut Shell, cx: &mut Context<Shell>| {
            this.new_playlist = None;
            this.controls.text = None;
            cx.notify();
        };
        let cancel = dialog_button(&t, "new-playlist-cancel", DialogButton::Ghost, true).child(tr("Cancel")).stop_top(&t).on_click(cx.listener(move |this, _, _, cx| close(this, cx)));
        let create = dialog_button(&t, "new-playlist-create", DialogButton::Primary, can_create).child(match (smart, creating) {
            (true, true) => tr("Saving\u{2026}"),
            (true, false) => tr("Save"),
            (false, true) => tr("Creating\u{2026}"),
            (false, false) => tr("Create"),
        }).stop_top(&t).on_click(cx.listener(|this, _, _, cx| this.new_playlist_submit(cx)));
        Some(
            crate::ui::dialog_scrim(&t, "new-playlist-scrim")
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| close(this, cx)))
                .child(
                    crate::ui::dialog_panel(&t, "new-playlist")
                        .child(crate::ui::dialog_title(&t, if smart { tr("Save As") } else { tr("New Playlist") }))
                        .when(smart, |d| d.child(crate::ui::dialog_text(&t, tr("Saves these rules as a new smart playlist of your own. The original stays as it is."))))
                        .child(crate::ui::dialog_block(&t, tr("Name"), field))
                        .children(error.map(|e| crate::ui::error_text(&t, e)))
                        .child(crate::ui::dialog_actions(&t).child(cancel).child(create)),
                )
                .into_any_element(),
        )
    }
}
