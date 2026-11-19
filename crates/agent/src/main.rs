//! OpenTunnel agent: runs inside a dev container, dials the broker and serves VSCodium REHs.

mod direct;
mod reh;

use anyhow::{bail, Result};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// M0 spike: serve a plain WebSocket on this address that pipes to the REH.
    #[arg(long, env = "AGENT_DIRECT_LISTEN")]
    direct_listen: Option<SocketAddr>,

    /// Directory of an extracted VSCodium REH to use in direct mode.
    #[arg(long, env = "AGENT_SERVER_DIR")]
    server_dir: Option<PathBuf>,

    /// State directory (server cache, tokens, server data).
    #[arg(long, env = "AGENT_DATA_DIR", default_value = "/data")]
    data_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    if let Some(listen) = args.direct_listen {
        let Some(dir) = args.server_dir else { bail!("--direct-listen needs --server-dir") };
        let state = args.data_dir.join("direct");
        let server = reh::start(&dir, &state, &["--server-data-dir".into(), state.join("server-data").display().to_string()]).await?;
        println!("connection token: {}", server.connection_token);
        let port = server.port;
        tokio::select! {
            r = direct::serve(listen, port) => r?,
            _ = tokio::signal::ctrl_c() => {}
        }
        drop(server);
        return Ok(());
    }
    bail!("nothing to do: broker mode is not implemented yet (use --direct-listen)")
}
