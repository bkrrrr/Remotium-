//! Paired devices and pending pairing codes, in `<data-dir>/state.json`.
//!
//! Secrets are 32 random bytes and stored only as SHA-256 hashes. The CLI (`pair`, `revoke`)
//! and the running host share the file: writes take an exclusive lock and replace the file
//! atomically, reads reload it when it changed on disk.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// How long a pairing code works.
pub const PAIRING_TTL: u64 = 10 * 60;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Browser,
    Desktop,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub created: u64,
    token_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct PairingCode {
    hash: String,
    name: Option<String>,
    expires: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct State {
    host_id: String,
    #[serde(default)]
    devices: Vec<Device>,
    #[serde(default)]
    pairing: Vec<PairingCode>,
    /// Where users reach this host; written by `run`, used by `pair` for the links.
    #[serde(default)]
    public_url: Option<String>,
    /// Fingerprint clients pin (self-signed certificates only).
    #[serde(default)]
    pin: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    Invalid,
    Expired,
}

pub struct Store {
    path: PathBuf,
    cache: Mutex<(State, Option<SystemTime>)>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn secret(prefix: &str) -> String {
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
}

/// Constant-time comparison of two hex hashes.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
        let s = Self { path: data_dir.join("state.json"), cache: Mutex::new((State::default(), None)) };
        s.modify(|st| {
            if st.host_id.is_empty() {
                st.host_id = hex::encode(rand::random::<[u8; 6]>());
            }
        })?;
        Ok(s)
    }

    fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    fn read_file(&self) -> State {
        std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// The current state, reloaded if another process changed the file.
    fn read(&self) -> State {
        let mtime = self.mtime();
        let mut c = self.cache.lock().unwrap();
        if c.1 != mtime || mtime.is_none() {
            *c = (self.read_file(), mtime);
        }
        c.0.clone()
    }

    fn modify<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R> {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        use std::os::fd::AsRawFd;
        // SAFETY: flock on a file descriptor we own; released when `lock` is dropped.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut st = self.read_file();
        let n = now();
        st.pairing.retain(|p| p.expires >= n);
        let r = f(&mut st);
        let tmp = self.path.with_extension("tmp");
        {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let mut o = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            o.write_all(&serde_json::to_vec_pretty(&st)?)?;
        }
        std::fs::rename(&tmp, &self.path)?;
        *self.cache.lock().unwrap() = (st, self.mtime());
        Ok(r)
    }

    pub fn host_id(&self) -> String {
        self.read().host_id
    }

    pub fn public_url(&self) -> Option<String> {
        self.read().public_url
    }

    pub fn pin(&self) -> Option<String> {
        self.read().pin
    }

    /// Recorded by `run` so that `pair` can print correct links.
    pub fn set_endpoint(&self, url: &str, pin: Option<&str>) -> Result<()> {
        self.modify(|s| {
            s.public_url = Some(url.to_owned());
            s.pin = pin.map(str::to_owned);
        })
    }

    /// A new single-use pairing code.
    pub fn create_pairing(&self, name: Option<&str>) -> Result<String> {
        let code = secret("rmp_");
        let h = hash(&code);
        self.modify(|s| s.pairing.push(PairingCode { hash: h, name: name.map(str::to_owned), expires: now() + PAIRING_TTL }))?;
        Ok(code)
    }

    /// Trade a pairing code for a device token. The code is gone afterwards.
    pub fn redeem(&self, code: &str, kind: DeviceKind, device_name: &str) -> Result<Result<(Device, String), PairError>> {
        let h = hash(code);
        let token = secret("rmd_");
        let th = hash(&token);
        self.modify(|s| {
            let Some(i) = s.pairing.iter().position(|p| same(&p.hash, &h)) else { return Err(PairError::Invalid) };
            let p = s.pairing.remove(i);
            if p.expires < now() {
                return Err(PairError::Expired);
            }
            let name: String = p.name.unwrap_or_else(|| device_name.to_owned()).chars().take(64).collect();
            let d = Device { id: hex::encode(rand::random::<[u8; 4]>()), name, kind, created: now(), token_hash: th.clone() };
            s.devices.push(d.clone());
            Ok((d, token.clone()))
        })
    }

    pub fn check(&self, token: &str) -> Option<Device> {
        let h = hash(token);
        self.read().devices.into_iter().find(|d| same(&d.token_hash, &h))
    }

    pub fn devices(&self) -> Vec<Device> {
        self.read().devices
    }

    pub fn has_devices(&self) -> bool {
        !self.read().devices.is_empty()
    }

    pub fn revoke(&self, id: &str) -> Result<bool> {
        self.modify(|s| {
            let before = s.devices.len();
            s.devices.retain(|d| d.id != id);
            before != s.devices.len()
        })
    }

    #[cfg(test)]
    fn expire_codes(&self) {
        let _ = self.modify(|s| s.pairing.iter_mut().for_each(|p| p.expires = now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_is_single_use_and_tokens_are_hashed() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let code = s.create_pairing(Some("laptop")).unwrap();
        let (d, token) = s.redeem(&code, DeviceKind::Desktop, "x").unwrap().unwrap();
        assert_eq!(d.name, "laptop");
        assert_eq!(s.redeem(&code, DeviceKind::Desktop, "x").unwrap().unwrap_err(), PairError::Invalid);
        assert_eq!(s.check(&token).unwrap().id, d.id);
        let raw = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
        assert!(!raw.contains(&token) && !raw.contains(&code), "only hashes on disk");
    }

    #[test]
    fn codes_expire_and_devices_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let code = s.create_pairing(None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        s.expire_codes();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(s.redeem(&code, DeviceKind::Browser, "b").unwrap().is_err());

        let (d, token) = s.redeem(&s.create_pairing(None).unwrap(), DeviceKind::Browser, "b").unwrap().unwrap();
        assert!(s.revoke(&d.id).unwrap());
        assert!(s.check(&token).is_none());
    }

    #[test]
    fn other_processes_see_changes() {
        let dir = tempfile::tempdir().unwrap();
        let host = Store::open(dir.path()).unwrap();
        let cli = Store::open(dir.path()).unwrap();
        assert_eq!(host.host_id(), cli.host_id());
        let code = cli.create_pairing(None).unwrap();
        assert!(host.redeem(&code, DeviceKind::Desktop, "d").unwrap().is_ok());
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(cli.devices().len(), 1);
    }
}
