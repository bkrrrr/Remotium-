//! The agent's single outbound connection to the broker.
//!
//! `wss://<broker>/agent/v1/connect` with the agent token as bearer, then yamux (agent is
//! the yamux client). The agent opens stream 1 as the control stream; every stream the
//! broker opens is a data stream that starts with a [`StreamHeader`].

use crate::manager::ServerManager;
use anyhow::{anyhow, bail, Context, Result};
use opentunnel_protocol::{
    frame::{read_frame, write_frame},
    messages::{AgentMsg, BrokerMsg, EnsureResult, ErrorCode, StreamHeader, StreamKind, StreamReply},
    mux::{self, Mode, MuxStream},
    ws::WsStream,
    PROTOCOL_VERSION,
};
use std::{sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, sync::mpsc};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue};

#[derive(Clone, Debug)]
pub struct LinkConfig {
    pub broker_url: String,
    pub agent_token: String,
    pub agent_id: String,
    pub env_id: String,
    pub projects: Vec<String>,
    pub keepalive: Duration,
}

/// `https://host/base` → `wss://host/base/agent/v1/connect` (`http` → `ws`).
pub fn agent_ws_url(broker_url: &str) -> Result<String> {
    let base = broker_url.trim_end_matches('/');
    let rest = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_owned()
    } else {
        bail!("broker URL must start with https:// or http://: {broker_url}");
    };
    Ok(format!("{rest}/agent/v1/connect"))
}

/// Connect forever, with jittered exponential backoff between attempts.
pub async fn run(cfg: LinkConfig, manager: Arc<ServerManager>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = tokio::time::Instant::now();
        match connect_once(&cfg, &manager).await {
            Ok(()) => tracing::warn!("broker connection closed"),
            Err(e) => tracing::warn!("broker connection failed: {e:#}"),
        }
        // A connection that lived a while resets the backoff.
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(1);
        }
        let jitter = Duration::from_millis(rand::random_range(0..=backoff.as_millis() as u64 / 2));
        let wait = backoff + jitter;
        tracing::info!("reconnecting in {:.1}s", wait.as_secs_f32());
        tokio::time::sleep(wait).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn connect_once(cfg: &LinkConfig, manager: &Arc<ServerManager>) -> Result<()> {
    crate::install_crypto_provider();
    let url = agent_ws_url(&cfg.broker_url)?;
    let mut req = url.as_str().into_client_request()?;
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", cfg.agent_token)).context("agent token is not a valid header value")?,
    );
    tracing::info!(%url, "connecting to broker");
    let (ws, _resp) = match tokio::time::timeout(Duration::from_secs(30), tokio_tungstenite::connect_async(req)).await {
        Err(_) => bail!("timed out connecting to {url}"),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(resp))) => {
            bail!("broker refused the agent: HTTP {}", resp.status())
        }
        Ok(r) => r?,
    };
    if let tokio_tungstenite::MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_nodelay(true).ok();
    }
    let io = WsStream::new(ws).with_keepalive(cfg.keepalive, cfg.keepalive * 3);
    let (mux, mut inbound, mut driver) = mux::spawn(io, Mode::Client);

    let mut ctrl = mux.open().await?;
    write_frame(
        &mut ctrl,
        &AgentMsg::Hello {
            agent_id: cfg.agent_id.clone(),
            env_id: cfg.env_id.clone(),
            projects: cfg.projects.clone(),
            arch: format!("{}-{}", crate::platform::os(), crate::platform::arch()),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await?;
    let welcome: BrokerMsg = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut ctrl))
        .await
        .map_err(|_| anyhow!("no welcome from broker"))??;
    let BrokerMsg::Welcome { heartbeat_secs } = welcome else { bail!("expected welcome, got {welcome:?}") };
    tracing::info!(env = cfg.env_id, "connected to broker");

    let (ctrl_r, mut ctrl_w) = tokio::io::split(ctrl);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<AgentMsg>();

    let writer = async move {
        let mut hb = tokio::time::interval(Duration::from_secs(heartbeat_secs.max(5)));
        loop {
            let msg = tokio::select! {
                m = out_rx.recv() => match m { Some(m) => m, None => break },
                _ = hb.tick() => AgentMsg::Heartbeat,
            };
            write_frame(&mut ctrl_w, &msg).await?;
        }
        Ok::<(), std::io::Error>(())
    };

    let mgr = manager.clone();
    let reader = async move {
        let mut ctrl_r = ctrl_r;
        loop {
            let msg: BrokerMsg = match read_frame(&mut ctrl_r).await {
                Ok(m) => m,
                Err(e) => return Err::<(), std::io::Error>(e),
            };
            match msg {
                BrokerMsg::EnsureServer { id, server } => {
                    let mgr = mgr.clone();
                    let out = out_tx.clone();
                    tokio::spawn(async move {
                        let result = match mgr.ensure(&server).await {
                            Ok(connection_token) => EnsureResult::Ok { connection_token },
                            Err(e) => {
                                tracing::warn!(commit = server.commit, "ensureServer failed: {}", e.message);
                                EnsureResult::Error { code: e.code, message: e.message }
                            }
                        };
                        let _ = out.send(AgentMsg::EnsureServerResult { id, result });
                    });
                }
                BrokerMsg::Heartbeat => {}
                BrokerMsg::Welcome { .. } => tracing::debug!("unexpected welcome"),
            }
        }
    };

    let mgr = manager.clone();
    let acceptor = async move {
        while let Some(stream) = inbound.recv().await {
            let mgr = mgr.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_data_stream(stream, &mgr).await {
                    tracing::debug!("data stream ended: {e:#}");
                }
            });
        }
    };

    let res: Result<()> = tokio::select! {
        r = &mut driver => r.map_err(anyhow::Error::from).and_then(|r| r.map_err(Into::into)),
        r = writer => r.map_err(|e: std::io::Error| e.into()),
        r = reader => r.map_err(|e: std::io::Error| e.into()),
        _ = acceptor => Ok(()),
    };
    driver.abort();
    res
}

async fn handle_data_stream(mut stream: MuxStream, mgr: &ServerManager) -> Result<()> {
    let header: StreamHeader = tokio::time::timeout(Duration::from_secs(15), read_frame(&mut stream))
        .await
        .map_err(|_| anyhow!("no stream header"))??;
    if header.v != PROTOCOL_VERSION || header.kind != StreamKind::Reh {
        write_frame(&mut stream, &StreamReply::err(ErrorCode::Internal, "unsupported stream kind or version")).await?;
        bail!("unsupported stream {:?} v{}", header.kind, header.v);
    }
    let (mut tcp, _guard) = match mgr.connect(&header.commit).await {
        Ok(c) => c,
        Err(e) => {
            write_frame(&mut stream, &StreamReply::err(e.code, e.message.clone())).await?;
            bail!("{}", e.message);
        }
    };
    write_frame(&mut stream, &StreamReply::ok()).await?;
    tracing::debug!(session = header.session_id, commit = header.commit, "data stream open");
    let r = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
    let _ = stream.shutdown().await;
    let (up, down) = r?;
    tracing::debug!(session = header.session_id, up, down, "data stream closed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_urls() {
        assert_eq!(agent_ws_url("https://t.example.com/").unwrap(), "wss://t.example.com/agent/v1/connect");
        assert_eq!(agent_ws_url("http://broker:8080").unwrap(), "ws://broker:8080/agent/v1/connect");
        assert!(agent_ws_url("broker:8080").is_err());
    }
}
