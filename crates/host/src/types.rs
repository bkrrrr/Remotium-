//! Types shared by the HTTP API and the server manager.

use serde::{Deserialize, Serialize};

/// The editor build a client wants a server for, from its `product.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ServerVersion {
    pub commit: String,
    #[serde(default)]
    pub quality: Option<String>,
    pub version: String,
    #[serde(default)]
    pub release: Option<String>,
}

impl ServerVersion {
    /// A commit is a hex SHA; versions are short tokens. Checked before they reach a path or URL.
    pub fn is_valid(&self) -> bool {
        is_commit(&self.commit)
            && safe(&self.version)
            && self.release.as_deref().is_none_or(safe)
            && self.quality.as_deref().is_none_or(safe)
    }
}

pub fn is_commit(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Unauthorized,
    VersionUnavailable,
    Internal,
}

/// The VSCodium release whose web UI the host serves to browsers (from `compat.json`).
#[derive(Deserialize, Clone, Debug)]
pub struct Pinned {
    pub version: String,
    pub release: Option<String>,
    pub commit: String,
    pub quality: String,
}

pub fn pinned() -> ServerVersion {
    #[derive(Deserialize)]
    struct Compat {
        pinned: Pinned,
    }
    let c: Compat = serde_json::from_str(include_str!("../../../compat.json")).expect("compat.json");
    ServerVersion { commit: c.pinned.commit, quality: Some(c.pinned.quality), version: c.pinned.version, release: c.pinned.release }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        let mut v = pinned();
        assert!(v.is_valid());
        v.version = "../../etc".into();
        assert!(!v.is_valid());
    }
}
