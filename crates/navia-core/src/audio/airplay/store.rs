//! Pairing state: Navia's secret Ed25519 identity and each receiver's public key.
//! The secret goes to the app's [`SecretStore`], or to a file only this user can read.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The keyring entry name for Navia's AirPlay identity.
pub const IDENTITY_KEY: &str = "airplay-identity";
const IDENTITY_FILE: &str = "airplay-identity";
const DEVICES_FILE: &str = "airplay-devices.json";

/// A place for secrets, implemented by the app with the system keyring.
/// `Err` means no keyring is usable, and the store falls back to a private file.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> std::result::Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> std::result::Result<(), String>;
    fn delete(&self, key: &str) -> std::result::Result<(), String>;
}

/// Navia's controller identity, as the receivers know it.
#[derive(Clone)]
pub struct Identity {
    /// The pairing identifier (a UUID string) sent in pair-setup and pair-verify.
    pub id: String,
    /// The Ed25519 seed.
    pub seed: [u8; 32],
}

/// What pairing left with us about one receiver.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub name: String,
    /// The receiver's long-term Ed25519 public key, hex.
    pub ltpk: String,
    /// The receiver's pairing identifier.
    pub identifier: String,
}

pub struct PairingStore {
    dir: PathBuf,
    secrets: Option<Arc<dyn SecretStore>>,
    lock: Mutex<()>,
}

impl PairingStore {
    /// `dir` is the config directory, and `secrets` the app's keyring if it has one.
    pub fn new(dir: impl Into<PathBuf>, secrets: Option<Arc<dyn SecretStore>>) -> Self {
        Self { dir: dir.into(), secrets, lock: Mutex::new(()) }
    }

    /// The identity, created and saved on first use.
    pub fn identity(&self) -> Result<Identity> {
        let _g = self.lock.lock().expect("pairing store");
        if let Some(found) = self.read_identity() {
            return Ok(found);
        }
        let seed: [u8; 32] = rand::random();
        let id = new_uuid();
        let text = format!("{id}:{}", hex(&seed));
        let in_keyring = self.secrets.as_ref().is_some_and(|s| s.set(IDENTITY_KEY, &text).is_ok());
        if !in_keyring {
            write_private(&self.dir.join(IDENTITY_FILE), text.as_bytes())?;
        }
        Ok(Identity { id, seed })
    }

    fn read_identity(&self) -> Option<Identity> {
        let from_keyring = self.secrets.as_ref().and_then(|s| s.get(IDENTITY_KEY).ok().flatten());
        let text = from_keyring.or_else(|| fs::read_to_string(self.dir.join(IDENTITY_FILE)).ok())?;
        let (id, seed) = text.trim().split_once(':')?;
        let seed: [u8; 32] = unhex(seed)?.try_into().ok()?;
        Some(Identity { id: id.to_string(), seed })
    }

    pub fn device(&self, id: &str) -> Option<PairedDevice> {
        let _g = self.lock.lock().expect("pairing store");
        self.devices().remove(id)
    }

    pub fn paired_ids(&self) -> Vec<String> {
        let _g = self.lock.lock().expect("pairing store");
        self.devices().into_keys().collect()
    }

    pub fn save_device(&self, id: &str, device: PairedDevice) -> Result<()> {
        let _g = self.lock.lock().expect("pairing store");
        let mut all = self.devices();
        all.insert(id.to_string(), device);
        self.write_devices(&all)
    }

    /// Forget a receiver. Pairing with it again needs a new PIN.
    pub fn forget(&self, id: &str) -> Result<()> {
        let _g = self.lock.lock().expect("pairing store");
        let mut all = self.devices();
        if all.remove(id).is_some() {
            self.write_devices(&all)?;
        }
        Ok(())
    }

    fn devices(&self) -> BTreeMap<String, PairedDevice> {
        fs::read(self.dir.join(DEVICES_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn write_devices(&self, all: &BTreeMap<String, PairedDevice>) -> Result<()> {
        let json = serde_json::to_vec_pretty(all).map_err(|e| Error::AirPlay(format!("saving pairings: {e}")))?;
        write_private(&self.dir.join(DEVICES_FILE), &json)
    }
}

/// Write a file readable only by this user (0600 on Unix), replacing it atomically.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn new_uuid() -> String {
    let b: [u8; 16] = rand::random();
    let h = hex(&b).to_uppercase();
    format!("{}-{}-4{}-{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[16..20], &h[20..32])
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemSecrets(Mutex<BTreeMap<String, String>>);
    impl SecretStore for MemSecrets {
        fn get(&self, key: &str) -> std::result::Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> std::result::Result<(), String> {
            self.0.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> std::result::Result<(), String> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }
    struct NoKeyring;
    impl SecretStore for NoKeyring {
        fn get(&self, _: &str) -> std::result::Result<Option<String>, String> {
            Err("no keyring".into())
        }
        fn set(&self, _: &str, _: &str) -> std::result::Result<(), String> {
            Err("no keyring".into())
        }
        fn delete(&self, _: &str) -> std::result::Result<(), String> {
            Err("no keyring".into())
        }
    }

    #[test]
    fn identity_goes_to_the_keyring_and_stays_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemSecrets(Mutex::new(BTreeMap::new())));
        let store = PairingStore::new(dir.path(), Some(secrets.clone()));
        let a = store.identity().unwrap();
        let b = PairingStore::new(dir.path(), Some(secrets.clone())).identity().unwrap();
        assert_eq!((a.id.clone(), a.seed), (b.id, b.seed));
        assert!(!dir.path().join(IDENTITY_FILE).exists(), "the secret must not land in a file when a keyring works");
        assert_eq!(a.id.len(), 36);
    }

    #[test]
    fn without_a_keyring_the_identity_is_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairingStore::new(dir.path(), Some(Arc::new(NoKeyring)));
        let a = store.identity().unwrap();
        let path = dir.path().join(IDENTITY_FILE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(PairingStore::new(dir.path(), None).identity().unwrap().seed, a.seed);
    }

    #[test]
    fn devices_are_saved_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairingStore::new(dir.path(), None);
        let d = PairedDevice { name: "TV".into(), ltpk: "00".repeat(32), identifier: "BC:26".into() };
        store.save_device("02:00:00:00:00:01", d.clone()).unwrap();
        assert_eq!(store.device("02:00:00:00:00:01"), Some(d));
        store.forget("02:00:00:00:00:01").unwrap();
        assert_eq!(store.device("02:00:00:00:00:01"), None);
    }
}
