//! Whether the current connection is metered, from the system's own signal. A phone hotspot looks like plain Wi-Fi, so
//! connection type can't tell. macOS handles this in Swift with `NWPathMonitor`, so this covers Linux and Windows.
//! Unknown counts as not metered.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// How often the system signal is checked.
const POLL: Duration = Duration::from_secs(5);

/// Test hook. The file holds `1` for metered or `0` for not, in place of the system signal.
const FAKE_ENV: &str = "NAVIA_FAKE_METERED_FILE";

/// Runs for the rest of the process, calling `on_change` once at start and after every change.
pub fn spawn_watcher(on_change: impl Fn(bool) + Send + 'static) {
    let fake = std::env::var_os(FAKE_ENV).map(PathBuf::from);
    let spawned = thread::Builder::new().name("navia-metered".into()).spawn(move || {
        let mut last = None;
        loop {
            let now = match &fake {
                Some(path) => read_fake(path),
                None => query(),
            }
            .unwrap_or(false);
            if last != Some(now) {
                last = Some(now);
                on_change(now);
            }
            thread::sleep(POLL);
        }
    });
    if let Err(e) = spawned {
        log::warn!("couldn't start the metered connection watcher: {e}");
    }
}

fn read_fake(path: &std::path::Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim() == "1")
}

/// NetworkManager's `Metered` value: unknown 0, yes 1, no 2, guess-yes 3, guess-no 4.
fn nm_metered_value(value: u32) -> Option<bool> {
    match value {
        1 | 3 => Some(true),
        2 | 4 => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn query() -> Option<bool> {
    portal_metered().or_else(network_manager_metered)
}

#[cfg(target_os = "linux")]
fn portal_metered() -> Option<bool> {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;

    let connection = zbus::blocking::Connection::session().ok()?;
    let reply = connection.call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.NetworkMonitor"), "GetStatus", &()).ok()?;
    let status: HashMap<String, OwnedValue> = reply.body().deserialize().ok()?;
    bool::try_from(status.get("metered")?).ok()
}

#[cfg(target_os = "linux")]
fn network_manager_metered() -> Option<bool> {
    use zbus::zvariant::OwnedValue;

    let connection = zbus::blocking::Connection::system().ok()?;
    let reply = connection.call_method(Some("org.freedesktop.NetworkManager"), "/org/freedesktop/NetworkManager", Some("org.freedesktop.DBus.Properties"), "Get", &("org.freedesktop.NetworkManager", "Metered")).ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    nm_metered_value(u32::try_from(&value).ok()?)
}

// Written from the documented API but not yet tested on Windows.
#[cfg(windows)]
fn query() -> Option<bool> {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};

    let profile = NetworkInformation::GetInternetConnectionProfile().ok()?;
    let cost = profile.GetConnectionCost().ok()?.NetworkCostType().ok()?;
    Some(cost != NetworkCostType::Unrestricted && cost != NetworkCostType::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_manager_values_mean_what_they_say() {
        assert_eq!(nm_metered_value(0), None);
        assert_eq!(nm_metered_value(1), Some(true));
        assert_eq!(nm_metered_value(2), Some(false));
        assert_eq!(nm_metered_value(3), Some(true));
        assert_eq!(nm_metered_value(4), Some(false));
        assert_eq!(nm_metered_value(9), None);
    }

    #[test]
    fn the_fake_file_answers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metered");
        assert_eq!(read_fake(&path), None);
        std::fs::write(&path, "1\n").unwrap();
        assert_eq!(read_fake(&path), Some(true));
        std::fs::write(&path, "0").unwrap();
        assert_eq!(read_fake(&path), Some(false));
    }
}
