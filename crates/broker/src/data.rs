//! Data plane: agent connections (`/agent/v1/connect`) and client streams (`/data/v1/stream`).

use crate::{
    registry::{AgentConn, AgentInfo},
    AppState,
};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use opentunnel_protocol::{
    close,
    frame::{read_frame, write_frame},
    messages::{AgentMsg, BrokerMsg},
    mux::{self, Mode},
    ws::WsStream,
};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::mpsc;

const MAX_WS_MESSAGE: usize = 64 * 1024 * 1024;

pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token.trim()).filter(|t| !t.is_empty())
}

/// Constant-time lookup of the env whose agent token hashes to `token`'s hash.
fn env_for_agent_token(state: &AppState, token: &str) -> Option<String> {
    let h: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let mut found = None;
    for e in &state.config.envs {
        if let Some(expected) = e.token_hash() {
            let diff = expected.iter().zip(h.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b));
            if diff == 0 {
                found = Some(e.id.clone());
            }
        }
    }
    found
}

pub async fn agent_connect(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(env) = bearer(&headers).and_then(|t| env_for_agent_token(&state, t)) else {
        tracing::warn!(%peer, "agent connection with an invalid token");
        return (StatusCode::UNAUTHORIZED, "invalid agent token").into_response();
    };
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| async move {
            if let Err(e) = run_agent(state, socket, env.clone(), peer).await {
                tracing::warn!(env, %peer, "agent connection ended: {e:#}");
            }
        })
}

async fn run_agent(state: Arc<AppState>, socket: WebSocket, env: String, peer: SocketAddr) -> anyhow::Result<()> {
    let ka = Duration::from_secs(state.config.server.keepalive_secs.max(1));
    let io = WsStream::new(socket).with_keepalive(ka, ka * 3);
    let (mux, mut inbound, mut driver) = mux::spawn(io, Mode::Server);

    let mut ctrl = tokio::select! {
        s = inbound.recv() => s.ok_or_else(|| anyhow::anyhow!("closed before control stream"))?,
        _ = tokio::time::sleep(Duration::from_secs(30)) => anyhow::bail!("no control stream"),
    };
    let hello: AgentMsg = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut ctrl)).await??;
    let AgentMsg::Hello { agent_id, env_id, projects, arch, version } = hello else {
        anyhow::bail!("expected hello");
    };
    if env_id != env {
        anyhow::bail!("agent token is for env {env:?} but agent says {env_id:?}");
    }
    write_frame(&mut ctrl, &BrokerMsg::Welcome { heartbeat_secs: 30 }).await?;

    let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel::<BrokerMsg>();
    let conn_id = state.registry.next_conn_id();
    let info = AgentInfo {
        agent_id,
        env_id: env.clone(),
        projects,
        arch,
        version,
        connected_at: crate::tokens::now(),
        remote_addr: peer.to_string(),
    };
    let conn = Arc::new(AgentConn::new(conn_id, info.clone(), mux, ctrl_tx));
    if state.registry.insert(conn.clone()).is_some() {
        tracing::info!(env, "agent replaced a previous connection");
    }
    tracing::info!(env, agent = info.agent_id, arch = info.arch, version = info.version, %peer, "agent online");

    let (mut r, mut w) = tokio::io::split(ctrl);
    let writer = async move {
        let mut hb = tokio::time::interval(Duration::from_secs(30));
        loop {
            let msg = tokio::select! {
                m = ctrl_rx.recv() => match m { Some(m) => m, None => break },
                _ = hb.tick() => BrokerMsg::Heartbeat,
            };
            write_frame(&mut w, &msg).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let reader_conn = conn.clone();
    let reader = async move {
        loop {
            match read_frame::<_, AgentMsg>(&mut r).await? {
                AgentMsg::EnsureServerResult { id, result } => reader_conn.complete(id, result),
                AgentMsg::Heartbeat => {}
                AgentMsg::Hello { .. } => tracing::debug!("duplicate hello ignored"),
            }
        }
        #[allow(unreachable_code)]
        Ok::<_, std::io::Error>(())
    };
    // The agent never opens streams after the control stream; drain them so yamux stays happy.
    let drain = async move { while inbound.recv().await.is_some() {} };

    let res = tokio::select! {
        r = &mut driver => r.map_err(anyhow::Error::from).and_then(|r| r.map_err(Into::into)),
        r = writer => r.map_err(Into::into),
        r = reader => r.map_err(Into::into),
        _ = drain => Ok(()),
    };
    driver.abort();
    state.registry.remove(&env, conn_id);
    tracing::info!(env, %peer, "agent offline");
    res
}

pub async fn client_stream(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // Errors are reported as close codes after the upgrade, so clients get one mechanism.
    let session = bearer(&headers).map(|t| state.tokens.check_stream(t));
    ws.max_message_size(MAX_WS_MESSAGE).max_frame_size(MAX_WS_MESSAGE).on_upgrade(move |mut socket| async move {
        let session = match session {
            Some(Ok(s)) => s,
            _ => return close_with(&mut socket, close::UNAUTHORIZED, "invalid or expired stream token").await,
        };
        if !state.config.allowed(&session.user, &session.env, None) {
            return close_with(&mut socket, close::FORBIDDEN, "not allowed").await;
        }
        let Some(agent) = state.registry.get(&session.env) else {
            return close_with(&mut socket, close::ENV_OFFLINE, "environment offline").await;
        };
        let (mut upstream, _guard) = match agent.open_stream(&session.commit, &session.id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::info!(env = session.env, session = session.id, "stream refused: {}", e.message);
                return close_with(&mut socket, e.code.close_code(), &e.message).await;
            }
        };
        tracing::debug!(env = session.env, session = session.id, %peer, "client stream open");
        let ka = Duration::from_secs(state.config.server.keepalive_secs.max(1));
        let mut client = WsStream::new(socket).with_keepalive(ka, ka * 4);
        let res = tokio::io::copy_bidirectional_with_sizes(&mut client, &mut upstream, 64 * 1024, 64 * 1024).await;
        state.tokens.touch_session(&session.id);
        match res {
            Ok((up, down)) => tracing::debug!(session = session.id, up, down, "client stream closed"),
            Err(e) => tracing::debug!(session = session.id, "client stream ended: {e}"),
        }
    })
}

async fn close_with(socket: &mut WebSocket, code: u16, reason: &str) {
    // Close reasons are limited to 123 bytes.
    let mut reason = reason.to_owned();
    while reason.len() > 120 {
        reason.pop();
    }
    let _ = socket.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
}
