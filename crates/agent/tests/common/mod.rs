//! Test helpers: a fake REH archive and a static file server.
#![allow(dead_code)]

use std::{net::SocketAddr, path::Path};

pub const COMMIT: &str = "c072cf642cbbadabefa22f39c58cc902a8b7427f";

/// A fake server: `bin/codium-server` starts a Python echo server on a free port and prints
/// the same line the real REH prints.
const FAKE_SERVER: &str = r#"#!/bin/sh
exec python3 -u -c '
import socket, sys, threading
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0)); s.listen(64)
print("Extension host agent listening on %d" % s.getsockname()[1], flush=True)
def echo(c):
    while True:
        d = c.recv(65536)
        if not d: break
        c.sendall(d)
    c.close()
while True:
    c, _ = s.accept()
    threading.Thread(target=echo, args=(c,), daemon=True).start()
' "$@"
"#;

pub fn python_available() -> bool {
    std::process::Command::new("python3").arg("-c").arg("1").output().is_ok_and(|o| o.status.success())
}

/// Build `vscodium-reh-<os>-<arch>-<version>.tar.gz` for `commit` into `dir`; returns its bytes.
pub fn fake_reh_tarball(commit: &str) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    let product = serde_json::json!({
        "commit": commit, "quality": "stable", "version": "1.143.17232",
        "serverApplicationName": "codium-server", "nameShort": "VSCodium"
    })
    .to_string();
    add(&mut tar, "./product.json", product.as_bytes(), 0o644);
    add(&mut tar, "./bin/codium-server", FAKE_SERVER.as_bytes(), 0o755);
    tar.into_inner().unwrap().finish().unwrap()
}

fn add<W: std::io::Write>(tar: &mut tar::Builder<W>, path: &str, data: &[u8], mode: u32) {
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(mode);
    h.set_cksum();
    tar.append_data(&mut h, path, data).unwrap();
}

/// Serve every file in `dir` at `/<name>` on a random port.
pub async fn serve_dir(dir: &Path) -> SocketAddr {
    use axum::{extract::Path as P, http::StatusCode, routing::get, Router};
    let dir = dir.to_owned();
    let app = Router::new().route(
        "/{*name}",
        get(move |P(name): P<String>| {
            let dir = dir.clone();
            async move {
                match std::fs::read(dir.join(name.replace('/', "_"))) {
                    Ok(b) => Ok(b),
                    Err(_) => Err(StatusCode::NOT_FOUND),
                }
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    addr
}

pub fn sha256_hex(b: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(b))
}
