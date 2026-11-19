//! OpenTunnel agent: runs inside a dev container, dials the broker and serves VSCodium REHs.

use opentunnel_agent::{direct, link, manager, platform, reh};

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Broker base URL, e.g. https://tunnels.example.com or http://broker:8080.
    #[arg(long, env = "BROKER_URL")]
    broker_url: Option<String>,

    /// Agent token issued for this environment in the broker config.
    #[arg(long, env = "AGENT_TOKEN", hide_env_values = true)]
    agent_token: Option<String>,

    /// File containing the agent token (alternative to --agent-token).
    #[arg(long, env = "AGENT_TOKEN_FILE")]
    agent_token_file: Option<PathBuf>,

    /// Environment id, as configured in the broker.
    #[arg(long, env = "AGENT_ENV_ID")]
    env_id: Option<String>,

    /// Agent instance id; defaults to the host name.
    #[arg(long, env = "AGENT_ID")]
    agent_id: Option<String>,

    /// Projects to advertise (comma separated). Defaults to the directories in --workspace-root.
    #[arg(long, env = "AGENT_PROJECTS", value_delimiter = ',')]
    projects: Vec<String>,

    #[arg(long, env = "AGENT_WORKSPACE_ROOT", default_value = "/workspace")]
    workspace_root: PathBuf,

    /// State directory: server cache, tokens, server data (extensions, settings).
    #[arg(long, env = "AGENT_DATA_DIR", default_value = "/data")]
    data_dir: PathBuf,

    /// Server download URL template.
    #[arg(long, env = "AGENT_SERVER_URL_TEMPLATE", default_value = manager::DEFAULT_URL_TEMPLATE)]
    server_url_template: String,

    /// Read-only directories with pre-installed servers laid out as <dir>/<commit> (colon separated).
    #[arg(long, env = "AGENT_SERVER_SEARCH_PATH", value_delimiter = ':')]
    server_search_path: Vec<PathBuf>,

    /// Accept server downloads without a published .sha256 file.
    #[arg(long, env = "AGENT_ALLOW_UNVERIFIED_DOWNLOADS")]
    allow_unverified_downloads: bool,

    /// Stop a server after this many seconds without connections.
    #[arg(long, env = "AGENT_IDLE_TIMEOUT_SECS", default_value_t = 1800)]
    idle_timeout_secs: u64,

    /// Delete downloaded servers unused for this many days.
    #[arg(long, env = "AGENT_REAP_AFTER_DAYS", default_value_t = 14)]
    reap_after_days: u64,

    /// WebSocket ping interval towards the broker.
    #[arg(long, env = "AGENT_KEEPALIVE_SECS", default_value_t = 25)]
    keepalive_secs: u64,

    /// M0 spike: serve a plain WebSocket on this address that pipes to the REH in --server-dir.
    #[arg(long, env = "AGENT_DIRECT_LISTEN")]
    direct_listen: Option<SocketAddr>,

    /// Directory of an extracted VSCodium REH, for direct mode.
    #[arg(long, env = "AGENT_SERVER_DIR")]
    server_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    opentunnel_agent::install_crypto_provider();
    let args = Args::parse();

    if let Some(listen) = args.direct_listen {
        let Some(dir) = args.server_dir else { bail!("--direct-listen needs --server-dir") };
        let state = args.data_dir.join("direct");
        let extra = ["--server-data-dir".to_owned(), state.join("server-data").display().to_string()];
        let server = reh::start(&dir, &state, &extra).await?;
        println!("connection token: {}", server.connection_token);
        let port = server.port;
        tokio::select! {
            r = direct::serve(listen, port) => r?,
            _ = shutdown_signal() => {}
        }
        drop(server);
        return Ok(());
    }

    let broker_url = args.broker_url.context("--broker-url (BROKER_URL) is required")?;
    let env_id = args.env_id.context("--env-id (AGENT_ENV_ID) is required")?;
    let agent_token = match (args.agent_token, args.agent_token_file) {
        (Some(t), _) => t,
        (None, Some(f)) => std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?.trim().to_owned(),
        (None, None) => bail!("--agent-token or --agent-token-file is required"),
    };
    let projects = if args.projects.is_empty() { list_projects(&args.workspace_root) } else { args.projects };
    let agent_id = args.agent_id.unwrap_or_else(hostname);

    let mgr = manager::ServerManager::new(manager::ManagerConfig {
        cache_dir: args.data_dir.join("servers"),
        state_dir: args.data_dir.join("state"),
        search_dirs: args.server_search_path,
        url_template: args.server_url_template,
        require_checksum: !args.allow_unverified_downloads,
        idle_timeout: Duration::from_secs(args.idle_timeout_secs),
        reap_after: Duration::from_secs(args.reap_after_days * 86_400),
        os: platform::os().into(),
        arch: platform::arch().into(),
    })?;

    let housekeeping = {
        let mgr = mgr.clone();
        async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                mgr.stop_idle().await;
                if n.is_multiple_of(6 * 60) {
                    mgr.reap_installs();
                }
                n += 1;
            }
        }
    };

    tracing::info!(env = env_id, agent = agent_id, ?projects, "starting agent");
    let link_cfg = link::LinkConfig {
        broker_url,
        agent_token,
        agent_id,
        env_id,
        projects,
        keepalive: Duration::from_secs(args.keepalive_secs.max(1)),
    };
    tokio::select! {
        _ = link::run(link_cfg, mgr) => {}
        _ = housekeeping => {}
        _ = shutdown_signal() => tracing::info!("shutting down"),
    }
    Ok(())
}

fn list_projects(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "agent".into())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
