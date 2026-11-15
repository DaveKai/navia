//! The account's Deezer recommendations choice, set in Settings and at sign-in.
//! The core stores it, and this switch alone gates the feature.

use gpui::Context;

use crate::shell::Shell;

#[derive(Default, Clone, Copy)]
pub struct DeezerChoice {
    /// The account's choice. `None` until it chooses, which counts as off.
    pub choice: Option<bool>,
}

impl Shell {
    /// Loads the choice for the account just attached.
    pub fn load_deezer_choice(&mut self, lib: &crate::core_library::CoreLibrary) {
        self.deezer = DeezerChoice { choice: lib.deezer_recommendations() };
    }

    /// Saves the account's choice.
    pub fn set_deezer_choice(&mut self, on: bool, cx: &mut Context<Self>) {
        self.deezer.choice = Some(on);
        if let Some(core) = self.core.as_ref() {
            let lib = core.lib.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = lib.set_deezer_recommendations(Some(on)) {
                        eprintln!("navia: couldn't save the Deezer choice: {e}");
                    }
                })
                .detach();
        }
        cx.notify();
    }
}
