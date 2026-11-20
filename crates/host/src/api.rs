//! Desktop API for Remotium Client, under `/remotium/v1/`. Bearer device tokens.

use crate::{
    manager::EnsureError,
    state::{Device, DeviceKind, PairError},
    types::{is_commit, ErrorCode, ServerVersion},
    ws::WsStream,
    AppState,
};
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub const MAX_WS_MESSAGE: usize = 64 * 1024 * 1024;

pub struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response()
    }
}

impl From<EnsureError> for ApiError {
    fn from(e: EnsureError) -> Self {
        match e.code {
            ErrorCode::Unauthorized => ApiError(StatusCode::UNAUTHORIZED, "unauthorized", e.message),
            ErrorCode::VersionUnavailable => ApiError(StatusCode::CONFLICT, "version_unavailable", e.message),
            ErrorCode::Internal => ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.message),
        }
    }
}

fn unauthorized() -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, "unauthorized", "unknown or revoked device; pair again".into())
}

pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token.trim()).filter(|t| !t.is_empty())
}

fn device(state: &AppState, headers: &HeaderMap) -> Result<Device, ApiError> {
    bearer(headers).and_then(|t| state.store.check(t)).ok_or_else(unauthorized)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    code: String,
    #[serde(default)]
    device_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    device_token: String,
    device_id: String,
    host_id: String,
    name: String,
    folder: String,
}

pub async fn pair(State(state): State<Arc<AppState>>, Json(req): Json<PairRequest>) -> Result<Json<PairResponse>, ApiError> {
    let name = req.device_name.unwrap_or_else(|| "VSCodium".into());
    match state.store.redeem(&req.code, DeviceKind::Desktop, &name) {
        Ok(Ok((d, token))) => {
            tracing::info!(device = d.id, name = d.name, "paired a desktop client");
            Ok(Json(PairResponse {
                device_token: token,
                device_id: d.id,
                host_id: state.store.host_id(),
                name: state.name.clone(),
                folder: state.folder.clone(),
            }))
        }
        Ok(Err(PairError::Expired)) => Err(ApiError(StatusCode::UNAUTHORIZED, "expired", "pairing code expired; run `remotium-host pair` again".into())),
        Ok(Err(PairError::Invalid)) => Err(ApiError(StatusCode::UNAUTHORIZED, "invalid", "pairing code unknown or already used".into())),
        Err(e) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())),
    }
}

pub async fn info(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    device(&state, &headers)?;
    Ok(Json(json!({
        "hostId": state.store.host_id(),
        "name": state.name,
        "folder": state.folder,
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

pub async fn session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(v): Json<ServerVersion>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let d = device(&state, &headers)?;
    if !v.is_valid() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "bad_request", "invalid commit/version".into()));
    }
    let token = state.manager.ensure(&v).await?;
    tracing::info!(device = d.id, commit = v.commit, "session");
    Ok(Json(json!({ "connectionToken": token })))
}

#[derive(Deserialize)]
pub struct StreamQuery {
    commit: String,
}

/// One editor connection. Everything that can fail is checked before the upgrade, so the
/// client gets an HTTP status rather than a socket that opens and closes.
pub async fn stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(e) = device(&state, &headers) {
        return e.into_response();
    }
    if !is_commit(&q.commit) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_request", "invalid commit".into()).into_response();
    }
    let (mut tcp, guard) = match state.manager.connect(&q.commit).await {
        Ok(c) => c,
        Err(e) => return ApiError::from(e).into_response(),
    };
    let ka = state.keepalive;
    ws.max_message_size(MAX_WS_MESSAGE).max_frame_size(MAX_WS_MESSAGE).on_upgrade(move |socket| async move {
        let _guard = guard;
        let mut client = WsStream::new(socket).with_keepalive(ka, ka * 4);
        match tokio::io::copy_bidirectional_with_sizes(&mut client, &mut tcp, 64 * 1024, 64 * 1024).await {
            Ok((up, down)) => tracing::debug!(up, down, "stream closed"),
            Err(e) => tracing::debug!("stream ended: {e}"),
        }
    })
}
