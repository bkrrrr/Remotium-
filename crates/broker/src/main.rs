//! OpenTunnel broker binary.

use anyhow::Result;
use clap::{Parser, Subcommand};
use opentunnel_broker::{app, config::Config, control_router, data_router, serve, spawn_gc, AppState};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(long, short, env = "BROKER_CONFIG", default_value = "broker.toml")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the broker (default).
    Serve,
    /// Check the config file and exit.
    CheckConfig,
    /// Generate a new agent token and print it with its SHA-256 for broker.toml.
    GenToken,
    /// Print the SHA-256 of an agent token for `agent_token_sha256`.
    HashToken { token: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();
    opentunnel_broker::install_crypto_provider();
    let args = Args::parse();
    match args.cmd.unwrap_or(Cmd::Serve) {
        Cmd::GenToken => {
            let t = opentunnel_broker::tokens::random_secret("otag_");
            println!("agent token:        {t}\nagent_token_sha256: {}", hex::encode(Sha256::digest(t.as_bytes())));
            Ok(())
        }
        Cmd::HashToken { token } => {
            println!("{}", hex::encode(Sha256::digest(token.trim().as_bytes())));
            Ok(())
        }
        Cmd::CheckConfig => {
            let c = Config::load(&args.config)?;
            println!("ok: {} env(s), {} grant(s)", c.envs.len(), c.grants.len());
            Ok(())
        }
        Cmd::Serve => run(Config::load(&args.config)?).await,
    }
}

async fn run(config: Config) -> Result<()> {
    if config.auth.dev_identity.is_some() {
        tracing::warn!(
            nets = ?config.auth.dev_identity_from,
            "dev_identity is set: requests from these networks are signed in without a proxy. Don't use this in production."
        );
    }
    let state = AppState::new(config)?;
    spawn_gc(state.clone());
    let listen = state.config.server.listen;
    match state.config.server.data_listen {
        Some(data_addr) => {
            let control = control_router(state.clone()).layer(tower_http::trace::TraceLayer::new_for_http());
            let data = data_router(state.clone()).layer(tower_http::trace::TraceLayer::new_for_http());
            tokio::select! {
                r = serve(&state, listen, control) => r?,
                r = serve(&state, data_addr, data) => r?,
                _ = shutdown() => {}
            }
        }
        None => tokio::select! {
            r = serve(&state, listen, app(state.clone())) => r?,
            _ = shutdown() => {}
        },
    }
    tracing::info!("shutting down");
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
