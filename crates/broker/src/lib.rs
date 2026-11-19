//! OpenTunnel broker: control plane (dashboard, tickets, tokens, sessions) and data plane
//! (agent links and client streams).

pub mod config;
pub mod control;
pub mod dashboard;
pub mod data;
pub mod proxyauth;
pub mod registry;
pub mod tokens;

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, sync::Arc, time::Instant};

pub struct AppState {
    pub config: config::Config,
    pub tokens: tokens::Tokens,
    pub registry: registry::Registry,
    pub jwks: Option<proxyauth::JwtVerifier>,
    pub started: Instant,
}

impl AppState {
    pub fn new(config: config::Config) -> Result<Arc<Self>> {
        let jwks = config.auth.jwt.clone().map(proxyauth::JwtVerifier::new).transpose()?;
        Ok(Self::with_verifier(config, jwks))
    }

    pub fn with_verifier(config: config::Config, jwks: Option<proxyauth::JwtVerifier>) -> Arc<Self> {
        let tokens = tokens::Tokens::new(config.server.state_file.clone());
        Arc::new(Self { config, tokens, registry: Default::default(), jwks, started: Instant::now() })
    }
}

/// Install `ring` as the process-wide rustls provider. Safe to call more than once.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn control_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(dashboard::index))
        .route("/launch", post(dashboard::launch))
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/status", get(control::status))
        .route("/api/v1/environments", get(control::environments))
        .route("/api/v1/launch-tickets", post(control::launch_ticket))
        .route("/api/v1/tokens", post(control::tokens))
        .route("/api/v1/tokens/revoke", post(control::revoke_tokens))
        .route("/api/v1/sessions", post(control::create_session))
        .route("/api/v1/sessions/{id}/stream-tokens", post(control::stream_token))
        .route("/api/v1/sessions/{id}/revoke", post(control::revoke_session))
        .with_state(state)
}

pub fn data_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/data/v1/stream", get(data::client_stream))
        .route("/agent/v1/connect", get(data::agent_connect))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

/// One router with everything, for a single listener.
pub fn app(state: Arc<AppState>) -> Router {
    let data = Router::new()
        .route("/data/v1/stream", get(data::client_stream))
        .route("/agent/v1/connect", get(data::agent_connect))
        .with_state(state.clone());
    control_router(state).merge(data).layer(tower_http::trace::TraceLayer::new_for_http())
}

/// Periodic cleanup of expired tokens and sessions.
pub fn spawn_gc(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            t.tick().await;
            state.tokens.gc();
        }
    });
}

/// Serve `router` on `addr`, with TLS when the config has a certificate.
pub async fn serve(state: &AppState, addr: SocketAddr, router: Router) -> Result<()> {
    let svc = router.into_make_service_with_connect_info::<SocketAddr>();
    match (&state.config.server.tls_cert, &state.config.server.tls_key) {
        (Some(cert), Some(key)) => {
            install_crypto_provider();
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            // Pick up renewed certificates (e.g. Let's Encrypt) without a restart.
            let (reload, cert, key) = (tls.clone(), cert.clone(), key.clone());
            tokio::spawn(async move {
                let mut t = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
                t.tick().await;
                loop {
                    t.tick().await;
                    match reload.reload_from_pem_file(&cert, &key).await {
                        Ok(()) => tracing::info!("reloaded TLS certificate"),
                        Err(e) => tracing::warn!("reloading TLS certificate failed: {e}"),
                    }
                }
            });
            tracing::info!(%addr, "listening (https)");
            axum_server::bind_rustls(addr, tls).serve(svc).await?;
        }
        _ => {
            tracing::info!(%addr, "listening (http)");
            let l = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(l, svc).await?;
        }
    }
    Ok(())
}
