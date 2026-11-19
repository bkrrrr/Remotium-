mod common;

use common::*;
use opentunnel_agent::manager::{ManagerConfig, ServerManager};
use opentunnel_protocol::messages::{ErrorCode, ServerVersion};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn version(commit: &str) -> ServerVersion {
    ServerVersion { commit: commit.into(), quality: Some("stable".into()), version: "1.143.17232".into(), release: None }
}

fn config(root: &std::path::Path, base: std::net::SocketAddr) -> ManagerConfig {
    ManagerConfig {
        cache_dir: root.join("servers"),
        state_dir: root.join("state"),
        search_dirs: vec![],
        url_template: format!("http://{base}/vscodium-reh-${{os}}-${{arch}}-${{version}}${{releaseSuffix}}.tar.gz"),
        require_checksum: true,
        idle_timeout: Duration::from_secs(0),
        reap_after: Duration::from_secs(0),
        os: "linux".into(),
        arch: "x64".into(),
    }
}

#[tokio::test]
async fn downloads_verifies_starts_and_reuses() {
    if !python_available() {
        eprintln!("python3 not available; skipping");
        return;
    }
    let files = tempfile::tempdir().unwrap();
    let tgz = fake_reh_tarball(COMMIT);
    let name = "vscodium-reh-linux-x64-1.143.17232.tar.gz";
    std::fs::write(files.path().join(name), &tgz).unwrap();
    std::fs::write(files.path().join(format!("{name}.sha256")), format!("{}  {name}\n", sha256_hex(&tgz))).unwrap();
    let addr = serve_dir(files.path()).await;

    let root = tempfile::tempdir().unwrap();
    let mgr = ServerManager::new(config(root.path(), addr)).unwrap();

    // Concurrent ensures for one commit: one download, one process, one token.
    let v = version(COMMIT);
    let (a, b) = tokio::join!(mgr.ensure(&v), mgr.ensure(&v));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a, b);
    assert!(root.path().join("servers").join(COMMIT).join("product.json").exists());

    let (mut tcp, guard) = mgr.connect(COMMIT).await.unwrap();
    tcp.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tcp.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");

    // In use: not stopped even with a zero idle timeout.
    assert!(mgr.stop_idle().await.is_empty());
    drop(tcp);
    drop(guard);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(mgr.stop_idle().await, vec![COMMIT.to_string()]);

    // A new connection restarts the stopped server with the same token.
    let (_tcp, _g) = mgr.connect(COMMIT).await.unwrap();
    assert_eq!(mgr.ensure(&v).await.unwrap(), a);
}

#[tokio::test]
async fn rejects_checksum_mismatch_and_missing_versions() {
    let files = tempfile::tempdir().unwrap();
    let tgz = fake_reh_tarball(COMMIT);
    let name = "vscodium-reh-linux-x64-1.143.17232.tar.gz";
    std::fs::write(files.path().join(name), &tgz).unwrap();
    std::fs::write(files.path().join(format!("{name}.sha256")), format!("{}  {name}\n", "0".repeat(64))).unwrap();
    let addr = serve_dir(files.path()).await;
    let root = tempfile::tempdir().unwrap();
    let mgr = ServerManager::new(config(root.path(), addr)).unwrap();

    let err = mgr.ensure(&version(COMMIT)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::VersionUnavailable);
    assert!(err.message.contains("sha256 mismatch"), "{}", err.message);
    assert!(!root.path().join("servers").join(COMMIT).exists());

    let mut other = version("0123456789abcdef0123456789abcdef01234567");
    other.version = "9.9.9".into();
    let err = mgr.ensure(&other).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::VersionUnavailable);

    let err = mgr.connect("deadbeefdeadbeef").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::VersionUnavailable);
}

#[tokio::test]
async fn rejects_wrong_commit_in_archive() {
    let files = tempfile::tempdir().unwrap();
    let tgz = fake_reh_tarball("1111111111111111111111111111111111111111");
    let name = "vscodium-reh-linux-x64-1.143.17232.tar.gz";
    std::fs::write(files.path().join(name), &tgz).unwrap();
    std::fs::write(files.path().join(format!("{name}.sha256")), sha256_hex(&tgz)).unwrap();
    let addr = serve_dir(files.path()).await;
    let root = tempfile::tempdir().unwrap();
    let mgr = ServerManager::new(config(root.path(), addr)).unwrap();
    let err = mgr.ensure(&version(COMMIT)).await.unwrap_err();
    assert!(err.message.contains("client needs"), "{}", err.message);
}

#[tokio::test]
async fn reaps_unused_installs() {
    let root = tempfile::tempdir().unwrap();
    let mgr = ServerManager::new(config(root.path(), "127.0.0.1:9".parse().unwrap())).unwrap();
    let old = root.path().join("servers").join(COMMIT);
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("product.json"), "{}").unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(mgr.reap_installs(), vec![COMMIT.to_string()]);
    assert!(!old.exists());
}
