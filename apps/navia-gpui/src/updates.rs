//! The version and updates. Windows updates through Velopack from the feed set at build time with
//! `NAVIA_UPDATE_FEED`; without one, a check reports that updates aren't set up. Linux packages
//! update Navia, so a check there only looks for a newer GitHub release and links to it.

use crate::i18n::{tr, trf};

/// The version shown in Settings. A test build shows "0.1.0+543 (f6e8534)", the installer
/// version and commit. A release shows "0.1.0". See build.rs and tools/version.sh.
pub fn version_label() -> String {
    let version = env!("CARGO_PKG_VERSION");
    match (env!("NAVIA_COMMIT"), env!("NAVIA_BUILD")) {
        ("", _) => version.to_string(),
        (commit, "") => format!("{version} ({commit})"),
        (commit, build) => format!("{version}+{build} ({commit})"),
    }
}

/// The update feed this build was made with, if any.
pub fn feed() -> Option<&'static str> {
    option_env!("NAVIA_UPDATE_FEED").filter(|feed| !feed.is_empty())
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    /// A newer GitHub release, opened in the browser.
    Available { version: String, url: String },
    /// Downloaded, and applied by restarting.
    Ready,
    NotConfigured,
    /// Not installed by the installer, such as a folder copy or a development build.
    NotInstalled,
    Failed(String),
}

impl UpdateStatus {
    /// The line under "Version", if the status has one.
    pub fn note(&self) -> Option<String> {
        match self {
            UpdateStatus::Idle => None,
            UpdateStatus::Checking => Some(tr("Checking…").to_string()),
            UpdateStatus::UpToDate => Some(tr("Navia is up to date").to_string()),
            UpdateStatus::Available { version, .. } => Some(trf("Navia {version} is available", &[("version", version)])),
            UpdateStatus::Ready => Some(tr("A new version is ready.").to_string()),
            UpdateStatus::NotConfigured => Some(tr("Updates aren't set up for this build").to_string()),
            UpdateStatus::NotInstalled => Some(tr("Only an installed Navia can update itself.").to_string()),
            UpdateStatus::Failed(error) => Some(error.clone()),
        }
    }
}

/// Checks for a newer version, and on Windows downloads it. Blocks, so run it off the UI thread.
pub fn check_and_download() -> UpdateStatus {
    if cfg!(target_os = "windows") {
        let Some(feed) = feed() else { return UpdateStatus::NotConfigured };
        return imp::check_and_download(feed);
    }
    release_check()
}

fn release_check() -> UpdateStatus {
    use navia_core::updates::{ReleaseCheck, check_latest_release};
    match check_latest_release(env!("CARGO_PKG_VERSION")) {
        Ok(ReleaseCheck::UpToDate) => UpdateStatus::UpToDate,
        Ok(ReleaseCheck::Available { version, url }) => UpdateStatus::Available { version, url },
        Err(error) => {
            eprintln!("navia: update check failed: {error}");
            UpdateStatus::Failed(tr("Couldn't check for updates").to_string())
        }
    }
}

/// Restarts into the downloaded version. Returns only if it couldn't.
pub fn restart_to_update() -> Option<String> {
    let feed = feed()?;
    imp::restart_to_update(feed)
}

#[cfg(target_os = "windows")]
mod imp {
    use super::UpdateStatus;
    use velopack::{UpdateCheck, UpdateManager, sources::HttpSource};

    fn manager(feed: &str) -> Result<UpdateManager, velopack::Error> {
        UpdateManager::new(HttpSource::new(feed), None, None)
    }

    pub fn check_and_download(feed: &str) -> UpdateStatus {
        let manager = match manager(feed) {
            Ok(manager) => manager,
            Err(velopack::Error::NotInstalled(_)) => return UpdateStatus::NotInstalled,
            Err(error) => return UpdateStatus::Failed(error.to_string()),
        };
        match manager.check_for_updates() {
            Ok(UpdateCheck::UpdateAvailable(update)) => match manager.download_updates(&update, None) {
                Ok(()) => UpdateStatus::Ready,
                Err(error) => UpdateStatus::Failed(error.to_string()),
            },
            Ok(_) => UpdateStatus::UpToDate,
            Err(error) => UpdateStatus::Failed(error.to_string()),
        }
    }

    pub fn restart_to_update(feed: &str) -> Option<String> {
        let manager = match manager(feed) {
            Ok(manager) => manager,
            Err(error) => return Some(error.to_string()),
        };
        let pending = match manager.get_update_pending_restart() {
            Some(pending) => pending,
            None => return Some(crate::i18n::tr("No downloaded update was found.").to_string()),
        };
        manager.apply_updates_and_restart(&pending).err().map(|error| error.to_string())
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::UpdateStatus;

    pub fn check_and_download(_feed: &str) -> UpdateStatus {
        UpdateStatus::NotConfigured
    }

    pub fn restart_to_update(_feed: &str) -> Option<String> {
        None
    }
}

/// Velopack's install and uninstall hooks, run first in `main`. The installer and uninstaller
/// start Navia with their own arguments and it exits when done. A normal start passes through.
#[cfg(target_os = "windows")]
pub fn run_installer_hooks() {
    velopack::VelopackApp::build()
        // Updates are applied only when the user chooses Restart to update.
        .set_auto_apply_on_startup(false)
        // Uninstalling removes the Run key so it doesn't point at a deleted program.
        .on_before_uninstall_fast_callback(|_| {
            let _ = crate::autostart::set(crate::autostart::OpenAtLogin::Off);
        })
        .run();
}

impl crate::shell::Shell {
    /// Checks at launch and then every `UPDATE_CHECK_INTERVAL` while the setting is on.
    pub fn start_update_checks(&mut self, cx: &mut gpui::Context<Self>) {
        use navia_core::updates::{UPDATE_CHECK_INTERVAL, automatic_check_allowed};
        if cfg!(target_os = "windows") {
            if feed().is_none() {
                return;
            }
            if self.prefs.check_updates {
                self.check_for_updates(cx);
            }
        } else if automatic_check_allowed(self.sample_mode || crate::core_library::read_only_run()) {
            self.automatic_update_check(cx);
        } else {
            return;
        }
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(UPDATE_CHECK_INTERVAL).await;
            if this.update(cx, |this, cx| this.automatic_update_check(cx)).is_err() {
                return;
            }
        })
        .detach();
    }

    /// Skipped while a downloaded update waits for a restart.
    fn automatic_update_check(&mut self, cx: &mut gpui::Context<Self>) {
        if self.prefs.check_updates && !matches!(self.update_status, UpdateStatus::Checking | UpdateStatus::Ready) {
            self.run_update_check(true, cx);
        }
    }

    /// The Settings button. Checks off the UI thread, opens a newer release's page, or restarts
    /// into a downloaded version.
    pub fn check_for_updates(&mut self, cx: &mut gpui::Context<Self>) {
        match &self.update_status {
            UpdateStatus::Checking => return,
            UpdateStatus::Available { url, .. } => {
                cx.open_url(url);
                return;
            }
            UpdateStatus::Ready => {
                if let Some(error) = restart_to_update() {
                    self.update_status = UpdateStatus::Failed(error);
                    cx.notify();
                }
                return;
            }
            _ => {}
        }
        if cfg!(target_os = "windows") && feed().is_none() {
            self.update_status = UpdateStatus::NotConfigured;
            cx.notify();
            return;
        }
        self.run_update_check(false, cx);
    }

    /// An automatic check stays quiet unless it finds a newer version than the one shown.
    fn run_update_check(&mut self, automatic: bool, cx: &mut gpui::Context<Self>) {
        if !automatic {
            self.update_status = UpdateStatus::Checking;
            cx.notify();
        }
        let task = cx.background_executor().spawn(async { check_and_download() });
        cx.spawn(async move |this, cx| {
            let status = task.await;
            let _ = this.update(cx, |this, cx| {
                let found = matches!(status, UpdateStatus::Available { .. } | UpdateStatus::Ready);
                if !automatic || (found && this.update_status != status && this.update_status != UpdateStatus::Checking) {
                    this.update_status = status;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_check_updates(&mut self, on: bool, cx: &mut gpui::Context<Self>) {
        self.prefs.check_updates = on;
        self.prefs.save();
        cx.notify();
    }
}
