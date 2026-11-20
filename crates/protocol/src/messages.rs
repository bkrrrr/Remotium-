//! Messages exchanged between agent and server.
//!
//! The agent opens yamux stream 1 as the **control stream** and sends [`AgentMsg::Hello`]
//! first. Every other stream is opened by the server and starts with a [`StreamHeader`],
//! answered by a [`StreamReply`]; after that, raw bytes flow.

use serde::{Deserialize, Serialize};

/// The editor build a client wants a server for. Taken from the client's `product.json`.
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
    /// A commit is a lowercase hex SHA. We refuse anything else before it reaches a path or URL.
    pub fn is_valid(&self) -> bool {
        let hex = |s: &str| (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
        let safe = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        };
        hex(&self.commit)
            && safe(&self.version)
            && self.release.as_deref().is_none_or(safe)
            && self.quality.as_deref().is_none_or(safe)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Unauthorized,
    Forbidden,
    Offline,
    VersionUnavailable,
    Internal,
}

impl ErrorCode {
    pub fn close_code(self) -> u16 {
        use crate::close::*;
        match self {
            ErrorCode::Unauthorized => UNAUTHORIZED,
            ErrorCode::Forbidden => FORBIDDEN,
            ErrorCode::Offline => ENV_OFFLINE,
            ErrorCode::VersionUnavailable => VERSION_UNAVAILABLE,
            ErrorCode::Internal => INTERNAL,
        }
    }
}

/// Agent → server, on the control stream.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AgentMsg {
    Hello {
        agent_id: String,
        env_id: String,
        projects: Vec<String>,
        arch: String,
        version: String,
    },
    EnsureServerResult {
        id: u64,
        result: EnsureResult,
    },
    Heartbeat,
}

/// Server → agent, on the control stream.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerMsg {
    Welcome { heartbeat_secs: u64 },
    EnsureServer { id: u64, server: ServerVersion },
    Heartbeat,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EnsureResult {
    Ok { connection_token: String },
    Error { code: ErrorCode, message: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    /// Bytes to and from the REH for `commit`.
    Reh,
    /// Reserved for raw TCP forwarding.
    Tcp,
}

/// First frame on every server-opened data stream.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamHeader {
    pub v: u32,
    pub kind: StreamKind,
    pub commit: String,
    pub session_id: String,
}

/// The agent's answer to a [`StreamHeader`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl StreamReply {
    pub fn ok() -> Self {
        Self { ok: true, code: None, error: None }
    }
    pub fn err(code: ErrorCode, msg: impl Into<String>) -> Self {
        Self { ok: false, code: Some(code), error: Some(msg.into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shapes() {
        let m = AgentMsg::Hello {
            agent_id: "a".into(),
            env_id: "e".into(),
            projects: vec!["p".into()],
            arch: "x64".into(),
            version: "0.1.0".into(),
        };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            serde_json::json!({"type":"hello","agentId":"a","envId":"e","projects":["p"],"arch":"x64","version":"0.1.0"})
        );
        let h = StreamHeader { v: 1, kind: StreamKind::Reh, commit: "c".into(), session_id: "s".into() };
        assert_eq!(
            serde_json::to_value(&h).unwrap(),
            serde_json::json!({"v":1,"kind":"reh","commit":"c","sessionId":"s"})
        );
        let r = EnsureResult::Ok { connection_token: "t".into() };
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!({"status":"ok","connectionToken":"t"}));
    }

    #[test]
    fn version_validation() {
        let mut v = ServerVersion {
            commit: "c072cf642cbbadabefa22f39c58cc902a8b7427f".into(),
            quality: Some("stable".into()),
            version: "1.143.17232".into(),
            release: None,
        };
        assert!(v.is_valid());
        v.version = "../../etc".into();
        assert!(!v.is_valid());
        v.version = "1.0".into();
        v.commit = "nothex!".into();
        assert!(!v.is_valid());
    }
}
