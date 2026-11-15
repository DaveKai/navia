//! AirPlay receivers and what Navia can do with each, decided from the TXT record's feature and
//! status bits, never from brand or model.

use std::collections::HashMap;
use std::net::IpAddr;

use navia_airplay::base::Features;
use navia_airplay::discovery::TxtRecordParser;

/// Status flag (TXT `flags` or `sf`): a PIN is required.
const FLAG_PIN_REQUIRED: u32 = 0x8;
/// Status flag: a password is required.
const FLAG_PASSWORD: u32 = 0x80;
/// Status flag: one-time (HomeKit) pairing with a PIN is required before streaming.
const FLAG_PAIRING_REQUIRED: u32 = 0x200;

/// How Navia authenticates to a receiver, in the order receivers expect it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pairing {
    /// HomeKit pair-setup once with the PIN the receiver shows, then pair-verify every time.
    Pin,
    /// HomeKit transient pairing, with no PIN and nothing stored.
    Transient,
    /// A password set on the receiver (`pw=true` or status bit 0x80).
    Password,
}

/// The audio path chosen for a receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub pairing: Pairing,
    /// Raw PCM instead of ALAC, for receivers that refuse ALAC.
    pub pcm: bool,
}

/// Whether Navia can stream to a receiver, and if not, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Ready(Route),
    /// Needs PTP timing (UDP 319/320), which Navia doesn't do yet.
    NeedsPtp,
    /// AirPlay 1 only (`_raop._tcp`), not yet supported.
    AirPlay1Only,
    /// Its access control allows only devices of the same home or user.
    Restricted,
    /// A password-protected receiver, not yet supported.
    NeedsPassword,
}

/// A receiver found on the network.
#[derive(Clone, Debug)]
pub struct AirPlayDevice {
    /// The receiver's `deviceid`, stable across restarts and address changes.
    pub id: String,
    /// Its mDNS instance name, which is what people named it.
    pub name: String,
    pub model: String,
    pub manufacturer: Option<String>,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub features: u64,
    pub flags: u32,
    pub support: Support,
    /// The parsed record, as the protocol code wants it.
    pub(crate) raw: navia_airplay::Device,
}

impl AirPlayDevice {
    /// Build from an `_airplay._tcp` record. None if it's not an audio receiver.
    pub fn from_txt(name: &str, txt: &HashMap<String, String>, addresses: Vec<IpAddr>, port: u16) -> Option<Self> {
        let raw = TxtRecordParser::parse_airplay_txt(name, txt, addresses.clone(), port).ok()?;
        let features = raw.features;
        if !features.supports_audio() {
            return None;
        }
        let flags = txt.get("flags").or_else(|| txt.get("sf")).and_then(|v| parse_hex(v)).unwrap_or(0) as u32;
        let support = support_for(&features, flags, txt);
        Some(Self {
            id: raw.id.to_mac_string(),
            name: name.to_string(),
            model: txt.get("model").cloned().unwrap_or_default(),
            manufacturer: txt.get("manufacturer").cloned(),
            addresses,
            port,
            features: features.raw(),
            flags,
            support,
            raw,
        })
    }

    /// Whether the first connection needs a PIN from the receiver's screen.
    pub fn needs_pin(&self) -> bool {
        matches!(self.support, Support::Ready(Route { pairing: Pairing::Pin, .. }))
    }
}

fn parse_hex(v: &str) -> Option<u64> {
    let v = v.trim();
    let v = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v);
    u64::from_str_radix(v, 16).ok()
}

/// Picks the path the way Apple senders and pyatv do from the same bits.
fn support_for(features: &Features, flags: u32, txt: &HashMap<String, String>) -> Support {
    // AirPlay 2 receivers advertise unified media control or CoreUtils pairing. The rest are AirPlay 1 only.
    let airplay2 = features.supports_unified_media_control() || features.supports_transient_pairing();
    if !airplay2 {
        return Support::AirPlay1Only;
    }
    // acl=1 means same home only, act=2 means current user only. Neither can be joined.
    if txt.get("acl").map(String::as_str) == Some("1") || txt.get("act").map(String::as_str) == Some("2") {
        return Support::Restricted;
    }
    if txt.get("pw").is_some_and(|v| v.eq_ignore_ascii_case("true")) || flags & FLAG_PASSWORD != 0 {
        return Support::NeedsPassword;
    }
    // NTP works on receivers that also offer PTP, so a PTP bit alone doesn't rule one out.
    // PTP-only receivers refuse NTP at SETUP, and that is reported then.
    let pairing = if flags & (FLAG_PAIRING_REQUIRED | FLAG_PIN_REQUIRED) != 0 { Pairing::Pin } else { Pairing::Transient };
    Support::Ready(Route { pairing, pcm: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// An LG webOS TV's record: PIN pairing, AirPlay 2.
    #[test]
    fn lg_tv_needs_a_pin() {
        let t = txt(&[("deviceid", "02:00:00:00:00:01"), ("features", "0x7F8AD0,0x38BCF46"), ("flags", "0x244"), ("model", "50UQ70006LB"), ("manufacturer", "LG"), ("acl", "0"), ("srcvers", "377.40.00"), ("pk", "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0")]);
        let d = AirPlayDevice::from_txt("[LG] webOS TV", &t, vec!["192.168.1.50".parse().unwrap()], 7000).unwrap();
        assert_eq!(d.id, "02:00:00:00:00:01");
        assert_eq!(d.support, Support::Ready(Route { pairing: Pairing::Pin, pcm: false }));
        assert!(d.needs_pin());
    }

    #[test]
    fn open_receiver_pairs_transiently() {
        let t = txt(&[("deviceid", "AA:BB:CC:DD:EE:FF"), ("features", "0x445F8A00,0x1C340"), ("flags", "0x4"), ("srcvers", "366.0")]);
        let d = AirPlayDevice::from_txt("Speaker", &t, vec![], 7000).unwrap();
        assert_eq!(d.support, Support::Ready(Route { pairing: Pairing::Transient, pcm: false }));
    }

    #[test]
    fn password_and_access_control_and_airplay1() {
        let base = [("deviceid", "AA:BB:CC:DD:EE:01"), ("features", "0x445F8A00,0x1C340"), ("srcvers", "366.0")];
        let mut t = txt(&base);
        t.insert("pw".into(), "true".into());
        assert_eq!(AirPlayDevice::from_txt("A", &t, vec![], 7000).unwrap().support, Support::NeedsPassword);
        let mut t = txt(&base);
        t.insert("acl".into(), "1".into());
        assert_eq!(AirPlayDevice::from_txt("B", &t, vec![], 7000).unwrap().support, Support::Restricted);
        // Audio, but neither unified media control (38) nor CoreUtils pairing (48): AirPlay 1.
        let t = txt(&[("deviceid", "AA:BB:CC:DD:EE:02"), ("features", "0x5A7FFFF7,0x0"), ("srcvers", "220.68")]);
        assert_eq!(AirPlayDevice::from_txt("C", &t, vec![], 7000).unwrap().support, Support::AirPlay1Only);
    }

    #[test]
    fn video_only_is_not_listed() {
        let t = txt(&[("deviceid", "AA:BB:CC:DD:EE:03"), ("features", "0x1,0x0"), ("srcvers", "1.0")]);
        assert!(AirPlayDevice::from_txt("TV", &t, vec![], 7000).is_none());
    }
}
