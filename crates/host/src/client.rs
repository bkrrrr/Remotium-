//! Remotium Client, offered for download by the host (`/remotium/client.vsix`), so desktop
//! users can get it from the pairing page without an extension store.

use crate::AppState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

include!(concat!(env!("OUT_DIR"), "/client_vsix.rs"));

pub const PATH: &str = "/remotium/client.vsix";

pub struct ClientPackage {
    pub bytes: Bytes,
    pub sha256: String,
}

impl ClientPackage {
    /// A file given with `--client-vsix`, else the package embedded in release builds.
    pub fn load(file: Option<&Path>) -> anyhow::Result<Option<Self>> {
        let bytes: Bytes = match file {
            Some(p) => std::fs::read(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?.into(),
            None => match EMBEDDED_CLIENT {
                Some(b) => Bytes::from_static(b),
                None => return Ok(None),
            },
        };
        Ok(Some(Self { sha256: hex::encode(Sha256::digest(&bytes)), bytes }))
    }
}

pub async fn download(State(state): State<Arc<AppState>>) -> Response {
    let Some(c) = &state.client else {
        return (StatusCode::NOT_FOUND, "this host has no Remotium Client package; get it from Open VSX or the GitHub release").into_response();
    };
    (
        [
            (header::CONTENT_TYPE, "application/vsix".to_owned()),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"remotium-client.vsix\"".to_owned()),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
            (header::HeaderName::from_static("x-sha256"), c.sha256.clone()),
        ],
        c.bytes.clone(),
    )
        .into_response()
}
