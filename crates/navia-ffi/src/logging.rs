//! The core's log, written to the unified log (os_log) under the app's subsystem, plus a panic hook so panics on any
//! thread land there too. Read it with `log show --predicate 'subsystem == "<bundle id>"' --info`.

use std::sync::Once;

static STARTED: Once = Once::new();

/// Starts the core's log under `subsystem`, the app's bundle id. Only the first call counts.
#[uniffi::export]
pub fn start_logging(subsystem: String) {
    STARTED.call_once(|| {
        install_subscriber(&subsystem);
        install_panic_hook();
        tracing::info!("navia core {} logging", env!("CARGO_PKG_VERSION"));
    });
}

#[cfg(target_vendor = "apple")]
fn install_subscriber(subsystem: &str) {
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::prelude::*;
    // Navia's own crates at info. Everything else, like HTTP, TLS and mDNS, only at warn.
    let filter = Targets::new()
        .with_target("navia_core", LevelFilter::INFO)
        .with_target("navia_ffi", LevelFilter::INFO)
        .with_target("navia_airplay", LevelFilter::INFO)
        .with_target("navia_integrations", LevelFilter::INFO)
        .with_default(LevelFilter::WARN);
    let layer = tracing_oslog::OsLogger::new(subsystem, "core").with_filter(filter);
    let _ = tracing_subscriber::registry().with(layer).try_init();
}

#[cfg(not(target_vendor = "apple"))]
fn install_subscriber(_subsystem: &str) {}

/// Panics go to the log, with the thread, before the default hook prints them to stderr, which nobody sees in an app
/// launched from Finder or the Dock.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        tracing::error!(target: "navia_ffi", "panic on thread {}: {info}", thread.name().unwrap_or("unnamed"));
        previous(info);
    }));
}
