//! Control API (`/api/v1/*`).

use crate::{
    data::bearer,
    proxyauth::ProxyUser,
    tokens::{ClientGrant, TokenError, STREAM_TTL, TICKET_TTL},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use remotium_protocol::messages::{ErrorCode, ServerVersion};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// JSON error body `{error, message}` with a matching status.
pub struct ApiError(pub StatusCode, pub &'static str, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response()
    }
}

impl ApiError {
    pub fn unauthorized(m: impl Into<String>) -> Self {
        Self(StatusCode::UNAUTHORIZED, "unauthorized", m.into())
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, "forbidden", m.into())
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, "bad_request", m.into())
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, "not_found", m.into())
    }
    pub fn from_code(code: ErrorCode, m: String) -> Self {
        match code {
            ErrorCode::Unauthorized => Self::unauthorized(m),
            ErrorCode::Forbidden => Self::forbidden(m),
            ErrorCode::Offline => Self(StatusCode::SERVICE_UNAVAILABLE, "offline", m),
            ErrorCode::VersionUnavailable => Self(StatusCode::CONFLICT, "version_unavailable", m),
            ErrorCode::Internal => Self(StatusCode::INTERNAL_SERVER_ERROR, "internal", m),
        }
    }
    fn token(e: TokenError) -> Self {
        match e {
            TokenError::Expired => Self(StatusCode::UNAUTHORIZED, "expired", "token expired".into()),
            TokenError::Reused => Self(StatusCode::UNAUTHORIZED, "reused", "refresh token reused; signed out".into()),
            TokenError::Invalid => Self::unauthorized("invalid token"),
        }
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvView {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub agent: Option<crate::registry::AgentInfo>,
    pub open_streams: usize,
    pub projects: Vec<ProjectView>,
}

/// Envs and projects `user` may open.
pub fn visible_envs(state: &AppState, user: &str) -> Vec<EnvView> {
    state
        .config
        .envs
        .iter()
        .filter(|e| state.config.allowed(user, &e.id, None))
        .map(|e| {
            let agent = state.registry.get(&e.id);
            let names: Vec<String> = if !e.projects.is_empty() {
                e.projects.clone()
            } else {
                agent.as_ref().map(|a| a.info.projects.clone()).unwrap_or_default()
            };
            let root = e.workspace_root.trim_end_matches('/');
            EnvView {
                id: e.id.clone(),
                name: e.display_name().to_owned(),
                online: agent.is_some(),
                open_streams: agent.as_ref().map(|a| a.open_streams.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0),
                agent: agent.map(|a| a.info.clone()),
                projects: names
                    .into_iter()
                    .filter(|p| valid_project(p) && state.config.allowed(user, &e.id, Some(p)))
                    .map(|p| ProjectView { path: format!("{root}/{p}"), name: p })
                    .collect(),
            }
        })
        .collect()
}

pub fn valid_project(p: &str) -> bool {
    (1..=128).contains(&p.len())
        && !p.starts_with('.')
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

pub async fn environments(State(state): State<Arc<AppState>>, ProxyUser(user): ProxyUser) -> Json<Vec<EnvView>> {
    Json(visible_envs(&state, &user))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    pub env_id: String,
    pub project: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResponse {
    pub uri: String,
    pub expires_in: u64,
}

/// Rejects browser requests that come from another site.
pub fn check_same_site(headers: &HeaderMap, public_url: &str) -> Result<(), ApiError> {
    if headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site") {
        return Err(ApiError::forbidden("cross-site request"));
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let own = origin_of(public_url);
        let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or_default();
        let same_host = origin.split_once("://").is_some_and(|(_, h)| h == host);
        if origin != own && !same_host {
            return Err(ApiError::forbidden("cross-origin request"));
        }
    }
    Ok(())
}

fn origin_of(url: &str) -> &str {
    let after = url.find("://").map(|i| i + 3).unwrap_or(0);
    match url[after..].find('/') {
        Some(i) => &url[..after + i],
        None => url,
    }
}

pub fn create_launch(state: &AppState, user: &str, env_id: &str, project: &str) -> Result<LaunchResponse, ApiError> {
    let env = visible_envs(state, user).into_iter().find(|e| e.id == env_id).ok_or_else(|| ApiError::forbidden("no access to this environment"))?;
    let proj = env.projects.iter().find(|p| p.name == project).ok_or_else(|| ApiError::forbidden("no access to this project"))?;
    let ticket = state.tokens.issue_ticket(user, env_id, project);
    let uri = format!(
        "{}://remotium.client/connect?ticket={}&env={}&path={}&server={}",
        state.config.server.editor_scheme,
        pct(&ticket),
        pct(env_id),
        pct(&proj.path),
        pct(&state.config.server.public_url),
    );
    tracing::info!(user, env = env_id, project, "launch ticket issued");
    Ok(LaunchResponse { uri, expires_in: TICKET_TTL })
}

pub async fn launch_ticket(
    State(state): State<Arc<AppState>>,
    ProxyUser(user): ProxyUser,
    headers: HeaderMap,
    Json(req): Json<LaunchRequest>,
) -> ApiResult<LaunchResponse> {
    check_same_site(&headers, &state.config.server.public_url)?;
    Ok(Json(create_launch(&state, &user, &req.env_id, &req.project)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenRequest {
    pub ticket: Option<String>,
    pub refresh_token: Option<String>,
}

pub async fn tokens(State(state): State<Arc<AppState>>, Json(req): Json<TokenRequest>) -> ApiResult<crate::tokens::TokenPair> {
    let pair = match (req.ticket, req.refresh_token) {
        (Some(t), None) => state.tokens.redeem_ticket(&t).map_err(ApiError::token)?,
        (None, Some(r)) => state.tokens.refresh(&r).map_err(ApiError::token)?,
        _ => return Err(ApiError::bad_request("send exactly one of ticket or refreshToken")),
    };
    if !state.config.allowed(&pair.user, &pair.env, pair.project.as_deref()) {
        return Err(ApiError::forbidden("access to this environment was removed"));
    }
    Ok(Json(pair))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeRequest {
    pub refresh_token: String,
}

pub async fn revoke_tokens(State(state): State<Arc<AppState>>, Json(req): Json<RevokeRequest>) -> StatusCode {
    state.tokens.revoke_by_refresh(&req.refresh_token);
    StatusCode::NO_CONTENT
}

fn client_grant(state: &AppState, headers: &HeaderMap) -> Result<ClientGrant, ApiError> {
    let t = bearer(headers).ok_or_else(|| ApiError::unauthorized("missing bearer token"))?;
    let g = state.tokens.check_access(t).map_err(ApiError::token)?;
    if !state.config.allowed(&g.user, &g.env, None) {
        return Err(ApiError::forbidden("access to this environment was removed"));
    }
    Ok(g)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    pub env_id: String,
    #[serde(flatten)]
    pub server: ServerVersion,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub session_id: String,
    pub stream_url: String,
    pub stream_token: String,
    pub stream_token_expires_in: u64,
    pub connection_token: String,
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SessionRequest>,
) -> ApiResult<SessionResponse> {
    let grant = client_grant(&state, &headers)?;
    if grant.env != req.env_id {
        return Err(ApiError::forbidden("token is for another environment"));
    }
    if !req.server.is_valid() {
        return Err(ApiError::bad_request("invalid commit/version"));
    }
    let agent = state
        .registry
        .get(&req.env_id)
        .ok_or_else(|| ApiError::from_code(ErrorCode::Offline, "environment offline".into()))?;
    let connection_token = agent
        .ensure_server(&req.server, Duration::from_secs(600))
        .await
        .map_err(|e| ApiError::from_code(e.code, e.message))?;
    let (session, stream_token) = state.tokens.create_session(&grant, &req.server.commit);
    tracing::info!(user = grant.user, env = grant.env, session = session.id, commit = req.server.commit, "session created");
    Ok(Json(SessionResponse {
        session_id: session.id,
        stream_url: stream_url(state.config.data_url()),
        stream_token,
        stream_token_expires_in: STREAM_TTL,
        connection_token,
    }))
}

pub fn stream_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let ws = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        base.to_owned()
    };
    format!("{ws}/data/v1/stream")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamTokenResponse {
    pub stream_token: String,
    pub expires_in: u64,
}

pub async fn stream_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StreamTokenResponse> {
    let grant = client_grant(&state, &headers)?;
    let t = state.tokens.new_stream_token(&id, &grant).ok_or_else(|| ApiError::not_found("unknown session"))?;
    Ok(Json(StreamTokenResponse { stream_token: t, expires_in: STREAM_TTL }))
}

pub async fn revoke_session(State(state): State<Arc<AppState>>, Path(id): Path<String>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    let grant = client_grant(&state, &headers)?;
    if state.tokens.revoke_session(&id, &grant) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("unknown session"))
    }
}

pub async fn status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let agents = state.registry.all();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptimeSecs": state.started.elapsed().as_secs(),
        "envsConfigured": state.config.envs.len(),
        "envsOnline": agents.len(),
        "openStreams": agents.iter().map(|a| a.open_streams.load(std::sync::atomic::Ordering::SeqCst)).sum::<usize>(),
        "sessions": state.tokens.sessions().len(),
    }))
}

/// Percent-encode everything but unreserved characters.
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(stream_url("https://t.example.com/"), "wss://t.example.com/data/v1/stream");
        assert_eq!(pct("/workspace/a b"), "%2Fworkspace%2Fa%20b");
        assert_eq!(origin_of("https://t.example.com/x"), "https://t.example.com");
    }

    #[test]
    fn project_names() {
        assert!(valid_project("demo"));
        assert!(valid_project("my.app-2"));
        assert!(!valid_project(".."));
        assert!(!valid_project("a/b"));
    }
}
