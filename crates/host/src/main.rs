//! remotium-host: VSCodium on this machine, over HTTPS, for browsers and desktop VSCodium.

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use remotium_host::{manager, platform, router, serve, state::Store, tls, types, AppState, Listen};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "Serve VSCodium on this machine over HTTPS, to browsers and desktop VSCodium")]
struct Cli {
    #[command(flatten)]
    run: RunArgs,
    /// State directory: certificate, paired devices, downloaded VSCodium servers.
    #[arg(long, env = "REMOTIUM_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the host (the default when no command is given).
    Run(Box<RunArgs>),
    /// Create a one-time pairing link for a browser or VSCodium.
    Pair {
        /// Label for the device that pairs with this link.
        #[arg(long)]
        name: Option<String>,
    },
    /// List paired devices.
    Devices,
    /// Remove a paired device.
    Revoke { id: String },
}

#[derive(Args, Default)]
struct RunArgs {
    #[arg(long, env = "REMOTIUM_LISTEN", default_value = "0.0.0.0:8443")]
    listen: Option<SocketAddr>,
    /// Folder opened by default.
    #[arg(long, env = "REMOTIUM_FOLDER")]
    folder: Option<PathBuf>,
    /// Extra names/addresses for the self-signed certificate; the first one is also used in links.
    #[arg(long = "name", env = "REMOTIUM_NAMES", value_delimiter = ',')]
    names: Vec<String>,
    /// URL users open, if it differs from https://<first name or host name>:<port>.
    #[arg(long, env = "REMOTIUM_PUBLIC_URL")]
    public_url: Option<String>,
    /// Your own certificate (PEM) instead of the self-signed one.
    #[arg(long, env = "REMOTIUM_TLS_CERT", requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    #[arg(long, env = "REMOTIUM_TLS_KEY", requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// Plain HTTP, for running behind your own HTTPS proxy or tunnel. Loopback only unless
    /// --plain-http-any-address is given too.
    #[arg(long, env = "REMOTIUM_PLAIN_HTTP")]
    plain_http: bool,
    #[arg(long, requires = "plain_http")]
    plain_http_any_address: bool,
    /// VSCodium server download URL template.
    #[arg(long, env = "REMOTIUM_REH_URL_TEMPLATE", default_value = manager::DEFAULT_URL_TEMPLATE)]
    reh_url_template: String,
    /// Read-only directories with pre-installed servers laid out as <dir>/<commit> (colon separated).
    #[arg(long, env = "REMOTIUM_REH_SEARCH_PATH", value_delimiter = ':')]
    reh_search_path: Vec<PathBuf>,
    /// Accept server downloads without a published .sha256 file.
    #[arg(long, env = "REMOTIUM_ALLOW_UNVERIFIED_DOWNLOADS")]
    allow_unverified_downloads: bool,
    /// Stop a server after this many seconds without connections.
    #[arg(long, env = "REMOTIUM_IDLE_TIMEOUT_SECS", default_value_t = 1800)]
    idle_timeout_secs: u64,
    /// Delete downloaded servers unused for this many days.
    #[arg(long, env = "REMOTIUM_REAP_AFTER_DAYS", default_value_t = 14)]
    reap_after_days: u64,
    /// WebSocket ping interval.
    #[arg(long, env = "REMOTIUM_KEEPALIVE_SECS", default_value_t = 25)]
    keepalive_secs: u64,
}

fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join(".remotium-host")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    remotium_host::install_crypto_provider();
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    match cli.cmd.unwrap_or(Cmd::Run(Box::new(cli.run))) {
        Cmd::Run(args) => run(data_dir, *args).await,
        Cmd::Pair { name } => {
            let store = Store::open(&data_dir)?;
            let Some(url) = store.public_url() else {
                bail!("start the host once first (`remotium-host run`) so it knows its address");
            };
            print_pairing(&store, &url, name.as_deref())
        }
        Cmd::Devices => {
            let store = Store::open(&data_dir)?;
            let devices = store.devices();
            if devices.is_empty() {
                println!("No paired devices. Run `remotium-host pair`.");
            }
            for d in devices {
                println!("{}  {:<8} {}", d.id, format!("{:?}", d.kind).to_lowercase(), d.name);
            }
            Ok(())
        }
        Cmd::Revoke { id } => {
            let store = Store::open(&data_dir)?;
            if store.revoke(&id)? {
                println!("Revoked {id}.");
                Ok(())
            } else {
                bail!("no device {id}")
            }
        }
    }
}

fn print_pairing(store: &Store, url: &str, name: Option<&str>) -> Result<()> {
    let code = store.create_pairing(name)?;
    let pin = store.pin();
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    println!();
    println!("Pairing link (works once, for {} minutes):", remotium_host::state::PAIRING_TTL / 60);
    println!("  Browser:  {url}/pair?code={code}");
    print!("  VSCodium: vscodium://remotium.client/pair?host={}&code={code}", enc(url));
    if let Some(fp) = &pin {
        print!("&fp={fp}");
    }
    println!();
    if let Some(fp) = &pin {
        println!();
        println!("The certificate is self-signed. Browsers will warn; continue only if it shows this SHA-256 fingerprint:");
        println!("  {}", tls::pretty(fp));
    }
    println!();
    Ok(())
}

async fn run(data_dir: PathBuf, args: RunArgs) -> Result<()> {
    let listen = args.listen.unwrap_or_else(|| "0.0.0.0:8443".parse().unwrap());
    if args.plain_http && !listen.ip().is_loopback() && !args.plain_http_any_address {
        bail!("--plain-http on {listen}: only allowed on loopback, or add --plain-http-any-address (put HTTPS in front!)");
    }
    let folder = match args.folder {
        Some(f) => f,
        None => std::env::current_dir()?,
    };
    let folder = std::fs::canonicalize(&folder).with_context(|| format!("folder {}", folder.display()))?;
    let store = Store::open(&data_dir)?;

    let (listen_mode, pin, scheme) = if args.plain_http {
        (Listen::Plain, None, "http")
    } else {
        let names = tls::default_names(&args.names);
        let own = args.tls_cert.zip(args.tls_key);
        let files = tls::prepare(&data_dir.join("tls"), own, &names)?;
        let pin = files.self_signed.then(|| files.fingerprint.clone());
        (Listen::Tls { cert: files.cert, key: files.key }, pin, "https")
    };
    let host = args.names.first().cloned().or_else(tls::hostname).unwrap_or_else(|| "localhost".into());
    let host = if listen.ip().is_unspecified() {
        host
    } else {
        match listen.ip() {
            IpAddr::V6(ip) => format!("[{ip}]"),
            ip => ip.to_string(),
        }
    };
    let public_url = args.public_url.unwrap_or_else(|| format!("{scheme}://{host}:{}", listen.port())).trim_end_matches('/').to_owned();
    store.set_endpoint(&public_url, pin.as_deref())?;

    let manager = manager::ServerManager::new(manager::ManagerConfig {
        cache_dir: data_dir.join("servers"),
        state_dir: data_dir.join("state"),
        search_dirs: args.reh_search_path,
        url_template: args.reh_url_template,
        require_checksum: !args.allow_unverified_downloads,
        idle_timeout: Duration::from_secs(args.idle_timeout_secs),
        reap_after: Duration::from_secs(args.reap_after_days * 86_400),
        os: platform::os().into(),
        arch: platform::arch().into(),
    })?;
    let has_devices = store.has_devices();
    let state = Arc::new(AppState {
        store,
        manager: manager.clone(),
        name: tls::hostname().unwrap_or_else(|| "remotium-host".into()),
        folder: folder.display().to_string(),
        web: types::pinned(),
        tls: !args.plain_http,
        keepalive: Duration::from_secs(args.keepalive_secs.max(1)),
    });

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            manager.stop_idle().await;
            if n.is_multiple_of(6 * 60) {
                manager.reap_installs();
            }
            n += 1;
        }
    });

    tracing::info!(%listen, url = public_url, folder = state.folder, "remotium-host starting");
    if !has_devices {
        println!("\nNo device is paired yet.");
        print_pairing(&state.store, &public_url, None)?;
    } else {
        println!("\nServing {} at {public_url}  (new device: `remotium-host pair`)\n", state.folder);
    }
    tokio::select! {
        r = serve(listen, router(state.clone()), listen_mode) => r?,
        _ = shutdown() => tracing::info!("shutting down"),
    }
    Ok(())
}

async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
}
