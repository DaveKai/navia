//! Finds AirPlay receivers over mDNS in pure Rust, so Windows needs nothing installed.
//! Sleeping receivers drop off and return often, so a removal only counts after a grace period.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use super::device::AirPlayDevice;
use crate::error::{Error, Result};

const SERVICE: &str = "_airplay._tcp.local.";
/// How long a receiver stays listed after mDNS says it left.
const REMOVE_AFTER: Duration = Duration::from_secs(30);

struct Entry {
    device: AirPlayDevice,
    gone_since: Option<Instant>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    entries: Arc<Mutex<BTreeMap<String, Entry>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Discovery {
    /// Start browsing. `on_change` runs on the discovery thread whenever the list changes.
    pub fn start(on_change: impl Fn() + Send + 'static) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::AirPlay(format!("mDNS: {e}")))?;
        let events = daemon.browse(SERVICE).map_err(|e| Error::AirPlay(format!("mDNS browse: {e}")))?;
        let entries: Arc<Mutex<BTreeMap<String, Entry>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (map, flag) = (entries.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("navia-airplay-mdns".into())
            .spawn(move || {
                let mut own = local_addresses();
                let mut own_checked = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    let changed = match events.recv_timeout(Duration::from_millis(1000)) {
                        Ok(ServiceEvent::ServiceResolved(info)) => {
                            if own_checked.elapsed() > Duration::from_secs(60) {
                                own = local_addresses();
                                own_checked = Instant::now();
                            }
                            let addresses: Vec<IpAddr> = info.get_addresses().iter().map(|a| a.to_ip_addr()).collect();
                            if addresses.iter().any(|a| own.contains(a)) {
                                false
                            } else {
                                let txt: HashMap<String, String> = info.txt_properties.iter().map(|p| (p.key().to_string(), p.val_str().to_string())).collect();
                                let name = instance_name(info.get_fullname());
                                match AirPlayDevice::from_txt(&name, &txt, sort_addresses(addresses), info.get_port()) {
                                    Some(device) => {
                                        let mut m = map.lock().expect("airplay devices");
                                        m.insert(info.get_fullname().to_string(), Entry { device, gone_since: None });
                                        true
                                    }
                                    None => false,
                                }
                            }
                        }
                        Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                            if let Some(e) = map.lock().expect("airplay devices").get_mut(&fullname) {
                                e.gone_since.get_or_insert_with(Instant::now);
                            }
                            false
                        }
                        Ok(_) => false,
                        Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
                        Err(_) => false,
                    };
                    let pruned = {
                        let mut m = map.lock().expect("airplay devices");
                        let before = m.len();
                        m.retain(|_, e| e.gone_since.is_none_or(|t| t.elapsed() < REMOVE_AFTER));
                        m.len() != before
                    };
                    if changed || pruned {
                        on_change();
                    }
                }
            })
            .map_err(Error::Io)?;
        Ok(Self { daemon, entries, stop, thread: Some(thread) })
    }

    /// Receivers currently on the network, by name.
    pub fn devices(&self) -> Vec<AirPlayDevice> {
        let m = self.entries.lock().expect("airplay devices");
        let mut out: Vec<AirPlayDevice> = m.values().map(|e| e.device.clone()).collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.dedup_by(|a, b| a.id == b.id);
        out
    }

    /// The mDNS daemon, for announcing our remote-control service.
    pub fn daemon(&self) -> ServiceDaemon {
        self.daemon.clone()
    }

    pub fn find(&self, id: &str) -> Option<AirPlayDevice> {
        self.devices().into_iter().find(|d| d.id.eq_ignore_ascii_case(id))
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.daemon.shutdown();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// "Living Room._airplay._tcp.local." → "Living Room", with mDNS escapes undone.
fn instance_name(fullname: &str) -> String {
    let name = fullname.strip_suffix(SERVICE).unwrap_or(fullname).trim_end_matches('.');
    name.replace("\\.", ".").replace("\\\\", "\\")
}

/// IPv4 first, then global IPv6, then link-local.
fn sort_addresses(mut a: Vec<IpAddr>) -> Vec<IpAddr> {
    a.sort_by_key(|ip| match ip {
        IpAddr::V4(_) => 0,
        IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80 => 2,
        IpAddr::V6(_) => 1,
    });
    a
}

fn local_addresses() -> HashSet<IpAddr> {
    if_addrs::get_if_addrs().map(|v| v.into_iter().map(|i| i.ip()).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_address_order() {
        assert_eq!(instance_name("[LG] webOS TV._airplay._tcp.local."), "[LG] webOS TV");
        assert_eq!(instance_name("Alex\\.s Room._airplay._tcp.local."), "Alex.s Room");
        let sorted = sort_addresses(vec!["fe80::1".parse().unwrap(), "2001:db8::1".parse().unwrap(), "192.168.1.2".parse().unwrap()]);
        assert_eq!(sorted[0], "192.168.1.2".parse::<IpAddr>().unwrap());
        assert_eq!(sorted[2], "fe80::1".parse::<IpAddr>().unwrap());
    }
}
