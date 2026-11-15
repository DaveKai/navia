//! The app's side of the smart playlist editor, whose logic lives in `navia_core::smart_editor`.
//! It supplies translations and playlist names for the text the core composes.

pub use navia_core::smart_editor::*;

/// The app's catalog and playlist names, for [`Say`].
pub struct AppWords {
    names: Vec<(String, String)>,
}

impl AppWords {
    /// Creates the words with the names of the playlists rules can point at.
    pub fn new(names: Vec<(String, String)>) -> Self {
        Self { names }
    }
}

impl Words for AppWords {
    fn text(&self, key: &str) -> String {
        crate::i18n::tr_dyn(key)
    }
    fn plural(&self, one: &str, other: &str, n: u64) -> String {
        crate::i18n::plural_dyn(one, other, n)
    }
    fn int(&self, n: i64) -> String {
        crate::i18n::fmt_int(n)
    }
    fn playlist_name(&self, id: &str) -> Option<String> {
        self.names.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone())
    }
}

impl crate::shell::Shell {
    /// The words for the smart playlist text, with this library's playlist names.
    pub fn smart_words(&self) -> AppWords {
        AppWords::new(self.playlists_full.iter().map(|p| (p.id.to_string(), p.name.to_string())).collect())
    }
}
