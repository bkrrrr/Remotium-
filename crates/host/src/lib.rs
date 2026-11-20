//! remotium-host: serves VSCodium on this machine over HTTPS, to browsers and to desktop
//! VSCodium with the Remotium Client extension.

pub mod api;
pub mod client;
pub mod manager;
pub mod platform;
pub mod reh;
pub mod state;
pub mod tls;
pub mod types;
pub mod web;
pub mod ws;

use axum::{
    http::HeaderMap,
    routing::{get, post},
    Router,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};

pub struct AppState {
    pub store: state::Store,
    pub manager: Arc<manager::ServerManager>,
    /// Shown to users.
    pub name: String,
    /// Opened by default, in the browser and in VSCodium.
    pub folder: String,
    /// The VSCodium release whose web UI browsers get.
    pub web: types::ServerVersion,
    /// HTTPS terminated here (cookies get `Secure`); with a TLS proxy in front we rely on
    /// `X-Forwarded-Proto`.
    pub tls: bool,
    pub keepalive: Duration,
    /// Remotium Client for download, if this build/config has one.
    pub client: Option<client::ClientPackage>,
}

impl AppState {
    pub fn secure_cookies(&self, headers: &HeaderMap) -> bool {
        self.tls || headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https")
    }
}

/// Install `ring` as the process-wide rustls provider. Safe to call more than once.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/remotium/healthz", get(|| async { "ok" }))
        .route(client::PATH, get(client::download))
        .route("/remotium/v1/pair", post(api::pair))
        .route("/remotium/v1/info", get(api::info))
        .route("/remotium/v1/session", post(api::session))
        .route("/remotium/v1/stream", get(api::stream))
        .route("/pair", get(web::pair_page).post(web::pair_submit))
        .fallback(web::proxy)
        .with_state(state)
}

pub enum Listen {
    Tls { cert: std::path::PathBuf, key: std::path::PathBuf },
    Plain,
}

/// Serve on `addr`. With TLS, certificate files are re-read every 6 hours (renewals).
pub async fn serve(addr: SocketAddr, router: Router, listen: Listen) -> anyhow::Result<()> {
    let svc = router.into_make_service_with_connect_info::<SocketAddr>();
    match listen {
        Listen::Tls { cert, key } => {
            install_crypto_provider();
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key).await?;
            let reload = tls.clone();
            tokio::spawn(async move {
                let mut t = tokio::time::interval(Duration::from_secs(6 * 3600));
                t.tick().await;
                loop {
                    t.tick().await;
                    if let Err(e) = reload.reload_from_pem_file(&cert, &key).await {
                        tracing::warn!("reloading the TLS certificate failed: {e}");
                    }
                }
            });
            axum_server::bind_rustls(addr, tls).serve(svc).await?;
        }
        Listen::Plain => {
            let l = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(l, svc).await?;
        }
    }
    Ok(())
}
