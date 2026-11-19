//! Starting a VSCodium remote extension host (REH) and learning its port.

use anyhow::{bail, Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

/// A running `codium-server` process bound to 127.0.0.1.
pub struct RunningServer {
    pub port: u16,
    pub connection_token: String,
    child: Child,
}

impl RunningServer {
    /// Has the process exited?
    pub fn has_exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

/// Generates a random token as lowercase hex.
pub fn random_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

/// The server's start script, `bin/<serverApplicationName>`.
pub fn server_binary(install_dir: &Path) -> Result<PathBuf> {
    let product: serde_json::Value = serde_json::from_slice(
        &std::fs::read(install_dir.join("product.json")).context("reading server product.json")?,
    )?;
    let name = product["serverApplicationName"].as_str().unwrap_or("codium-server");
    if name.contains('/') || name.contains("..") {
        bail!("invalid serverApplicationName {name:?}");
    }
    Ok(install_dir.join("bin").join(name))
}

/// Parse the port from a line like `Extension host agent listening on 57533`.
pub fn parse_listening_port(line: &str) -> Option<u16> {
    let rest = line.split("Extension host agent listening on ").nth(1)?;
    rest.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// Start the server in `install_dir` with a fresh connection token.
///
/// `state_dir` holds the token file and the server's data dir (`--server-data-dir`).
pub async fn start(install_dir: &Path, state_dir: &Path, extra_args: &[String]) -> Result<RunningServer> {
    let bin = server_binary(install_dir)?;
    std::fs::create_dir_all(state_dir)?;
    let token = random_token();
    let token_file = state_dir.join("connection-token");
    write_private(&token_file, token.as_bytes())?;

    let mut cmd = Command::new(&bin);
    cmd.arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--connection-token-file")
        .arg(&token_file)
        .arg("--telemetry-level")
        .arg("off")
        .arg("--accept-server-license-terms")
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tracing::info!(bin = %bin.display(), "starting server");
    let mut child = cmd.spawn().with_context(|| format!("starting {}", bin.display()))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            tracing::debug!(target: "reh", "{l}");
        }
    });
    let mut lines = BufReader::new(stdout).lines();
    let port = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(l) = lines.next_line().await? {
            tracing::debug!(target: "reh", "{l}");
            if let Some(p) = parse_listening_port(&l) {
                return Ok::<_, anyhow::Error>(Some(p));
            }
        }
        Ok(None)
    })
    .await
    .context("server did not report its port within 60 s")??
    .context("server exited before reporting its port")?;
    tokio::spawn(async move {
        while let Ok(Some(l)) = lines.next_line().await {
            tracing::debug!(target: "reh", "{l}");
        }
    });
    tracing::info!(port, "server listening");
    Ok(RunningServer { port, connection_token: token, child })
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port() {
        assert_eq!(parse_listening_port("Extension host agent listening on 57533"), Some(57533));
        assert_eq!(parse_listening_port("Server bound to 127.0.0.1:57533 (IPv4)"), None);
    }
}
