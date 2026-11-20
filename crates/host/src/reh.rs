//! Starting a VSCodium remote extension host (REH) and learning its port.
//!
//! Servers run in their own process group and log to a file rather than a pipe, so they
//! outlive the agent: after an agent restart or upgrade, [`adopt`] picks a running server
//! back up from `server.json` and the editor sessions on it keep going.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

/// A running `codium-server` bound to 127.0.0.1.
pub struct RunningServer {
    pub port: u16,
    pub pid: u32,
    pub connection_token: String,
    /// Set when this agent started the process (so it can reap it); `None` when adopted.
    child: Option<Child>,
}

#[derive(Serialize, Deserialize)]
struct ServerRecord {
    pid: u32,
    port: u16,
}

impl RunningServer {
    /// Has the process exited?
    pub fn has_exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => !matches!(c.try_wait(), Ok(None)),
            None => !pid_alive(self.pid),
        }
    }

    /// Stop the server and everything it started (its process group).
    pub async fn kill(&mut self) {
        signal_group(self.pid, libc::SIGTERM);
        for _ in 0..50 {
            if self.has_exited() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        signal_group(self.pid, libc::SIGKILL);
        if let Some(c) = self.child.as_mut() {
            let _ = c.wait().await;
        }
    }
}

fn signal_group(pid: u32, sig: libc::c_int) {
    if pid > 1 {
        // SAFETY: plain syscall; a negative pid addresses the process group.
        unsafe {
            libc::kill(-(pid as libc::pid_t), sig);
        }
    }
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks for existence and permission.
    pid > 1 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
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

fn read_token(state_dir: &Path) -> Option<String> {
    std::fs::read_to_string(state_dir.join("connection-token"))
        .ok()
        .filter(|t| t.len() >= 32 && t.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Pick up a server a previous agent process started from `state_dir`, if it still runs.
pub async fn adopt(state_dir: &Path) -> Option<RunningServer> {
    let rec: ServerRecord = serde_json::from_slice(&std::fs::read(state_dir.join("server.json")).ok()?).ok()?;
    let token = read_token(state_dir)?;
    if !pid_alive(rec.pid) {
        return None;
    }
    let probe = tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(("127.0.0.1", rec.port))).await;
    if !matches!(probe, Ok(Ok(_))) {
        return None;
    }
    tracing::info!(pid = rec.pid, port = rec.port, "adopted running server");
    Some(RunningServer { port: rec.port, pid: rec.pid, connection_token: token, child: None })
}

/// Start the server in `install_dir`.
///
/// `state_dir` holds the connection token (kept across restarts so clients holding it can
/// reconnect), `server.json` and `server.log`.
pub async fn start(install_dir: &Path, state_dir: &Path, extra_args: &[String]) -> Result<RunningServer> {
    let bin = server_binary(install_dir)?;
    std::fs::create_dir_all(state_dir)?;
    let token_file = state_dir.join("connection-token");
    let token = match read_token(state_dir) {
        Some(t) => t,
        None => {
            let t = random_token();
            write_private(&token_file, t.as_bytes())?;
            t
        }
    };
    let log_path = state_dir.join("server.log");
    let log = std::fs::File::create(&log_path)?;
    let _ = std::fs::remove_file(state_dir.join("server.json"));

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
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
    tracing::info!(bin = %bin.display(), log = %log_path.display(), "starting server");
    let mut child = cmd.spawn().with_context(|| format!("starting {}", bin.display()))?;
    let pid = child.id().context("server exited immediately")?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let port = loop {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if let Some(p) = text.lines().find_map(parse_listening_port) {
            break p;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let tail: String = text.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            bail!("server exited ({status}) before reporting its port:\n{tail}");
        }
        if tokio::time::Instant::now() > deadline {
            signal_group(pid, libc::SIGKILL);
            bail!("server did not report its port within 60 s (see {})", log_path.display());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    std::fs::write(state_dir.join("server.json"), serde_json::to_vec(&ServerRecord { pid, port })?)?;
    tracing::info!(port, pid, "server listening");
    Ok(RunningServer { port, pid, connection_token: token, child: Some(child) })
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
