//! The client extension, offered for download by the server itself.
//!
//! The server image contains the matching `remotium-client.vsix`, so users can install the
//! client from the same dashboard they launch from, with no extension store involved.

use crate::AppState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

/// Where the server image puts the client package.
pub const DEFAULT_CLIENT_VSIX: &str = "/usr/share/remotium/remotium-client.vsix";
pub const VSIX_NAME: &str = "remotium-client.vsix";

pub struct ClientPackage {
    pub bytes: Bytes,
    pub sha256: String,
}

impl ClientPackage {
    /// Load the configured package, or the image default if it exists.
    pub fn load(configured: Option<&Path>) -> Option<Self> {
        let path = match configured {
            Some(p) => p.to_owned(),
            None => {
                let d = Path::new(DEFAULT_CLIENT_VSIX);
                if !d.exists() {
                    return None;
                }
                d.to_owned()
            }
        };
        match std::fs::read(&path) {
            Ok(b) => {
                let sha256 = hex::encode(Sha256::digest(&b));
                tracing::info!(path = %path.display(), bytes = b.len(), sha256, "serving the client package");
                Some(Self { bytes: b.into(), sha256 })
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "client package not available: {e}");
                None
            }
        }
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "this server has no client package configured").into_response()
}

pub async fn vsix(State(state): State<Arc<AppState>>) -> Response {
    let Some(c) = &state.client else { return not_found() };
    (
        [
            (header::CONTENT_TYPE, "application/vsix".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{VSIX_NAME}\"")),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
        ],
        c.bytes.clone(),
    )
        .into_response()
}

pub async fn vsix_sha256(State(state): State<Arc<AppState>>) -> Response {
    let Some(c) = &state.client else { return not_found() };
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], format!("{}  {VSIX_NAME}\n", c.sha256)).into_response()
}
