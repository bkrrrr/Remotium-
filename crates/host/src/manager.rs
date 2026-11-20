//! Installs and runs one VSCodium REH per editor commit.
//!
//! - Installs live in `<cache>/<commit>` (downloaded) or in read-only search dirs
//!   (pre-baked into the image). Downloads go to a temp dir and are renamed into place, so
//!   concurrent agents sharing a volume never see a half-extracted server.
//! - Each commit has a slot guarded by an async mutex, so concurrent sessions for the same
//!   commit trigger one download and one server process.
//! - The connection token per commit is kept on disk and reused across restarts.
//! - A server with no connections for `idle_timeout` is stopped; installs unused for
//!   `reap_after` are deleted.

use crate::reh::{self, RunningServer};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use crate::types::{ErrorCode, ServerVersion};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncWriteExt, net::TcpStream};

/// The web variant of the VSCodium remote server: it serves desktop clients and the browser UI.
pub const DEFAULT_URL_TEMPLATE: &str = "https://github.com/VSCodium/vscodium/releases/download/${version}${releaseSuffix}/vscodium-reh-web-${os}-${arch}-${version}${releaseSuffix}.tar.gz";

#[derive(Clone, Debug)]
pub struct ManagerConfig {
    /// Writable cache for downloaded servers.
    pub cache_dir: PathBuf,
    /// Per-commit state (tokens) and the shared `--server-data-dir`.
    pub state_dir: PathBuf,
    /// Read-only dirs with pre-installed servers, each laid out as `<dir>/<commit>/`.
    pub search_dirs: Vec<PathBuf>,
    pub url_template: String,
    pub require_checksum: bool,
    pub idle_timeout: Duration,
    pub reap_after: Duration,
    pub os: String,
    pub arch: String,
}

/// An error with the code the server forwards to the client.
#[derive(Debug)]
pub struct EnsureError {
    pub code: ErrorCode,
    pub message: String,
}

impl EnsureError {
    fn unavailable(e: anyhow::Error) -> Self {
        Self { code: ErrorCode::VersionUnavailable, message: format!("{e:#}") }
    }
    fn internal(e: anyhow::Error) -> Self {
        Self { code: ErrorCode::Internal, message: format!("{e:#}") }
    }
}

#[derive(Default)]
struct Slot {
    version: Option<ServerVersion>,
    install: Option<PathBuf>,
    running: Option<RunningServer>,
}

struct SlotState {
    slot: tokio::sync::Mutex<Slot>,
    active: AtomicUsize,
    /// Unix seconds of the last time the active count changed.
    last_change: AtomicU64,
}

pub struct ServerManager {
    cfg: ManagerConfig,
    http: reqwest::Client,
    slots: Mutex<HashMap<String, Arc<SlotState>>>,
}

/// Keeps a server marked as in use while a connection is open.
#[derive(Debug)]
pub struct ActiveGuard(Arc<SlotState>);

impl std::fmt::Debug for SlotState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlotState").field("active", &self.active).finish()
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.last_change.store(now_secs(), Ordering::SeqCst);
    }
}

impl ServerManager {
    pub fn new(cfg: ManagerConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.cache_dir).with_context(|| format!("creating {}", cfg.cache_dir.display()))?;
        std::fs::create_dir_all(&cfg.state_dir)?;
        crate::install_crypto_provider();
        let http = reqwest::Client::builder()
            .user_agent(concat!("remotium-host/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Arc::new(Self { cfg, http, slots: Mutex::new(HashMap::new()) }))
    }

    fn slot(&self, commit: &str) -> Arc<SlotState> {
        self.slots
            .lock()
            .unwrap()
            .entry(commit.to_owned())
            .or_insert_with(|| {
                Arc::new(SlotState {
                    slot: Default::default(),
                    active: AtomicUsize::new(0),
                    last_change: AtomicU64::new(now_secs()),
                })
            })
            .clone()
    }

    /// Make sure a server for `v` is installed and running. Returns its connection token.
    pub async fn ensure(&self, v: &ServerVersion) -> Result<String, EnsureError> {
        if !v.is_valid() {
            return Err(EnsureError { code: ErrorCode::VersionUnavailable, message: "invalid version fields".into() });
        }
        let state = self.slot(&v.commit);
        let mut slot = state.slot.lock().await;
        slot.version = Some(v.clone());
        self.ensure_running(&mut slot, &v.commit).await?;
        Ok(slot.running.as_ref().unwrap().connection_token.clone())
    }

    async fn ensure_running(&self, slot: &mut Slot, commit: &str) -> Result<(), EnsureError> {
        if let Some(r) = slot.running.as_mut() {
            if !r.has_exited() {
                return Ok(());
            }
            tracing::warn!(commit, "server exited, restarting");
            slot.running = None;
        }
        let state = self.cfg.state_dir.join("servers").join(commit);
        if let Some(r) = reh::adopt(&state).await {
            slot.running = Some(r);
            return Ok(());
        }
        let install = match &slot.install {
            Some(p) if p.join("product.json").exists() => p.clone(),
            _ => {
                let v = slot
                    .version
                    .clone()
                    .ok_or_else(|| EnsureError::unavailable(anyhow!("no server known for commit {commit}")))?;
                let p = self.install(&v).await?;
                slot.install = Some(p.clone());
                p
            }
        };
        touch_last_used(&install);
        let data_dir = self.cfg.state_dir.join("server-data");
        let args = vec!["--server-data-dir".to_owned(), data_dir.display().to_string()];
        let running = reh::start(&install, &state, &args).await.map_err(EnsureError::internal)?;
        slot.running = Some(running);
        Ok(())
    }

    /// Open a TCP connection to the running server for `commit`, restarting it if it was
    /// stopped while idle.
    pub async fn connect(&self, commit: &str) -> Result<(TcpStream, ActiveGuard), EnsureError> {
        let (port, _token, guard) = self.endpoint(commit).await?;
        let tcp = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| EnsureError::internal(anyhow!("connecting to server: {e}")))?;
        tcp.set_nodelay(true).ok();
        Ok((tcp, guard))
    }

    /// Port and connection token of the running server for `commit` (started or restarted if
    /// needed). The guard keeps it counted as in use.
    pub async fn endpoint(&self, commit: &str) -> Result<(u16, String, ActiveGuard), EnsureError> {
        let state = self.slot(commit);
        let (port, token) = {
            let mut slot = state.slot.lock().await;
            if slot.running.is_none() && slot.version.is_none() && slot.install.is_none() {
                slot.install = self.find_install(commit);
                let adoptable = self.cfg.state_dir.join("servers").join(commit).join("server.json").exists();
                if slot.install.is_none() && !adoptable {
                    return Err(EnsureError::unavailable(anyhow!("no server for commit {commit}; create a session first")));
                }
            }
            self.ensure_running(&mut slot, commit).await?;
            state.active.fetch_add(1, Ordering::SeqCst);
            let r = slot.running.as_ref().unwrap();
            (r.port, r.connection_token.clone())
        };
        Ok((port, token, ActiveGuard(state)))
    }

    /// Stop servers idle for longer than the configured timeout. Returns the stopped commits.
    pub async fn stop_idle(&self) -> Vec<String> {
        let slots: Vec<_> = self.slots.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut stopped = Vec::new();
        for (commit, s) in slots {
            if s.active.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let idle = now_secs().saturating_sub(s.last_change.load(Ordering::SeqCst));
            if idle < self.cfg.idle_timeout.as_secs() {
                continue;
            }
            let Ok(mut slot) = s.slot.try_lock() else { continue };
            if s.active.load(Ordering::SeqCst) == 0 {
                if let Some(mut r) = slot.running.take() {
                    tracing::info!(commit, idle_secs = idle, "stopping idle server");
                    r.kill().await;
                    stopped.push(commit);
                }
            }
        }
        stopped
    }

    /// Delete downloaded installs not used for `reap_after`, except running ones.
    pub fn reap_installs(&self) -> Vec<String> {
        let busy: Vec<String> = self
            .slots
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s.active.load(Ordering::SeqCst) > 0 || s.slot.try_lock().map(|g| g.running.is_some()).unwrap_or(true))
            .map(|(k, _)| k.clone())
            .collect();
        let mut reaped = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.cfg.cache_dir) else { return reaped };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if name.starts_with(".tmp-") {
                // Leftover from an interrupted download.
                if age(&path).is_some_and(|a| a > Duration::from_secs(3600)) {
                    let _ = std::fs::remove_dir_all(&path);
                }
                continue;
            }
            if busy.contains(&name) {
                continue;
            }
            let last = age(&path.join(".last-used")).or_else(|| age(&path));
            if last.is_some_and(|a| a > self.cfg.reap_after) {
                tracing::info!(commit = name, "removing unused server install");
                if std::fs::remove_dir_all(&path).is_ok() {
                    if let Some(s) = self.slots.lock().unwrap().get(&name) {
                        if let Ok(mut g) = s.slot.try_lock() {
                            g.install = None;
                        }
                    }
                    reaped.push(name);
                }
            }
        }
        reaped
    }

    fn find_install(&self, commit: &str) -> Option<PathBuf> {
        std::iter::once(&self.cfg.cache_dir)
            .chain(self.cfg.search_dirs.iter())
            .map(|d| d.join(commit))
            .find(|p| installed_commit(p).as_deref() == Some(commit))
    }

    async fn install(&self, v: &ServerVersion) -> Result<PathBuf, EnsureError> {
        if let Some(p) = self.find_install(&v.commit) {
            return Ok(p);
        }
        let url = expand_template(&self.cfg.url_template, v, &self.cfg.os, &self.cfg.arch);
        tracing::info!(commit = v.commit, %url, "downloading server");
        let tmp = self.cfg.cache_dir.join(format!(".tmp-{}-{}", v.commit, reh::random_token()));
        let res = self.download_and_extract(&url, &tmp, &v.commit).await;
        let res = match res {
            Ok(()) => {
                let dest = self.cfg.cache_dir.join(&v.commit);
                match std::fs::rename(&tmp, &dest) {
                    Ok(()) => Ok(dest),
                    // Another agent sharing the volume finished first.
                    Err(_) if installed_commit(&dest).as_deref() == Some(v.commit.as_str()) => Ok(dest),
                    Err(e) => Err(EnsureError::internal(anyhow!("moving server into place: {e}"))),
                }
            }
            Err(e) => Err(e),
        };
        if tmp.exists() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        if let Ok(p) = &res {
            tracing::info!(commit = v.commit, path = %p.display(), "server installed");
        }
        res
    }

    async fn download_and_extract(&self, url: &str, dir: &Path, commit: &str) -> Result<(), EnsureError> {
        std::fs::create_dir_all(dir).map_err(|e| EnsureError::internal(e.into()))?;
        let expected = self.fetch_checksum(url).await.map_err(EnsureError::unavailable)?;
        let archive = dir.join("server.tar.gz");
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| EnsureError::unavailable(anyhow!("downloading {url}: {e}")))?;
        let mut file = tokio::fs::File::create(&archive).await.map_err(|e| EnsureError::internal(e.into()))?;
        let mut hasher = Sha256::new();
        let mut body = resp.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| EnsureError::unavailable(anyhow!("downloading {url}: {e}")))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| EnsureError::internal(e.into()))?;
        }
        file.flush().await.map_err(|e| EnsureError::internal(e.into()))?;
        drop(file);
        let actual = hex::encode(hasher.finalize());
        match expected {
            Some(exp) if !exp.eq_ignore_ascii_case(&actual) => {
                return Err(EnsureError::unavailable(anyhow!("sha256 mismatch for {url}: expected {exp}, got {actual}")));
            }
            Some(_) => tracing::debug!(%url, "sha256 verified"),
            None => tracing::warn!(%url, "no checksum published; not verified"),
        }

        let extract_dir = dir.to_owned();
        let archive2 = archive.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let f = std::fs::File::open(&archive2)?;
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
            ar.set_preserve_permissions(true);
            ar.unpack(&extract_dir).context("extracting server archive")?;
            Ok(())
        })
        .await
        .map_err(|e| EnsureError::internal(e.into()))?
        .map_err(EnsureError::internal)?;
        let _ = std::fs::remove_file(&archive);

        match installed_commit(dir) {
            Some(c) if c == commit => Ok(()),
            Some(c) => Err(EnsureError::unavailable(anyhow!("downloaded server is commit {c}, client needs {commit}"))),
            None => Err(EnsureError::unavailable(anyhow!("downloaded archive has no product.json"))),
        }
    }

    async fn fetch_checksum(&self, url: &str) -> Result<Option<String>> {
        let sum_url = format!("{url}.sha256");
        let resp = self.http.get(&sum_url).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let text = r.text().await?;
                let hex = text.split_whitespace().next().unwrap_or_default().to_owned();
                if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("malformed checksum file {sum_url}");
                }
                Ok(Some(hex))
            }
            Ok(r) if self.cfg.require_checksum => bail!("checksum {sum_url} not available (HTTP {})", r.status()),
            Err(e) if self.cfg.require_checksum => bail!("checksum {sum_url} not available: {e}"),
            _ => Ok(None),
        }
    }
}

/// Expand `${version}`, `${release}`, `${releaseSuffix}` (`.<release>` or empty), `${os}`,
/// `${arch}`, `${commit}` and `${quality}`. open-remote-ssh's `${version}.${release}` form is
/// accepted too and collapses to `${version}` when the editor has no release.
pub fn expand_template(t: &str, v: &ServerVersion, os: &str, arch: &str) -> String {
    let release = v.release.as_deref().unwrap_or("");
    let suffix = if release.is_empty() { String::new() } else { format!(".{release}") };
    t.replace(".${release}", &suffix)
        .replace("${releaseSuffix}", &suffix)
        .replace("${release}", release)
        .replace("${version}", &v.version)
        .replace("${commit}", &v.commit)
        .replace("${quality}", v.quality.as_deref().unwrap_or("stable"))
        .replace("${os}", os)
        .replace("${arch}", arch)
}

fn installed_commit(dir: &Path) -> Option<String> {
    let product: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("product.json")).ok()?).ok()?;
    product["commit"].as_str().map(str::to_owned)
}

fn touch_last_used(install: &Path) {
    let _ = std::fs::write(install.join(".last-used"), now_secs().to_string());
}

fn age(p: &Path) -> Option<Duration> {
    std::fs::metadata(p).ok()?.modified().ok()?.elapsed().ok()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(release: Option<&str>) -> ServerVersion {
        ServerVersion {
            commit: "c072cf642cbbadabefa22f39c58cc902a8b7427f".into(),
            quality: Some("stable".into()),
            version: "1.143.17232".into(),
            release: release.map(Into::into),
        }
    }

    #[test]
    fn template_without_release() {
        assert_eq!(
            expand_template(DEFAULT_URL_TEMPLATE, &v(None), "linux", "x64"),
            "https://github.com/VSCodium/vscodium/releases/download/1.143.17232/vscodium-reh-web-linux-x64-1.143.17232.tar.gz"
        );
    }

    #[test]
    fn template_with_release_open_remote_ssh_style() {
        let t = "https://x/${version}.${release}/vscodium-reh-${os}-${arch}-${version}.${release}.tar.gz";
        assert_eq!(expand_template(t, &v(Some("25010")), "alpine", "arm64"), "https://x/1.143.17232.25010/vscodium-reh-alpine-arm64-1.143.17232.25010.tar.gz");
        assert_eq!(expand_template(t, &v(None), "linux", "x64"), "https://x/1.143.17232/vscodium-reh-linux-x64-1.143.17232.tar.gz");
    }
}
