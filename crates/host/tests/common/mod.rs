//! Test support: a fake VSCodium server (the test binary itself, no Python), an in-process
//! remotium-host, and helpers.
#![allow(dead_code)]

use std::{
    io::{BufRead, Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};

pub const COMMIT: &str = "c072cf642cbbadabefa22f39c58cc902a8b7427f";

/// Body of the hidden `fake_reh_main` test that every test file declares. When started by the
/// fake `bin/codium-server` script it acts like a VSCodium server:
/// - prints the REH's "listening on" line;
/// - raw TCP connections (desktop streams) are echoed;
/// - HTTP requests get `path=…` and `cookie=…` back; WebSocket upgrades get a message echo.
pub fn fake_reh_main() {
    if std::env::var("REMOTIUM_FAKE_REH").is_err() {
        return;
    }
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    println!("Extension host agent listening on {}", l.local_addr().unwrap().port());
    std::io::stdout().flush().unwrap();
    for c in l.incoming().flatten() {
        std::thread::spawn(move || handle(c));
    }
}

/// Replays bytes already read, then reads from the socket.
struct Replay {
    head: Cursor<Vec<u8>>,
    s: TcpStream,
}
impl Read for Replay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.head.read(buf)?;
        if n > 0 { Ok(n) } else { self.s.read(buf) }
    }
}
impl Write for Replay {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.s.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}

fn handle(mut s: TcpStream) {
    // HTTP or raw bytes? Peek (don't consume); short raw messages may never reach 4 bytes.
    let mut first = [0u8; 4];
    let start = std::time::Instant::now();
    let is_http = loop {
        match s.peek(&mut first) {
            Ok(0) => return,
            Ok(n) if n >= 4 => break &first == b"GET " || &first == b"POST",
            Ok(_) if start.elapsed() > std::time::Duration::from_millis(300) => break false,
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(_) => return,
        }
    };
    if !is_http {
        let mut r = s.try_clone().unwrap();
        let _ = std::io::copy(&mut r, &mut s);
        return;
    }
    let mut first = [0u8; 4];
    if s.read_exact(&mut first).is_err() {
        return;
    }
    // Read the request head.
    let mut head = first.to_vec();
    let mut reader = std::io::BufReader::new(s.try_clone().unwrap());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        head.extend_from_slice(line.as_bytes());
        if line == "\r\n" {
            break;
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    if text.to_ascii_lowercase().contains("upgrade: websocket") {
        let mut ws = match tungstenite::accept(Replay { head: Cursor::new(head), s }) {
            Ok(ws) => ws,
            Err(_) => return,
        };
        while let Ok(m) = ws.read() {
            if m.is_close() {
                break;
            }
            if (m.is_binary() || m.is_text()) && ws.send(m).is_err() {
                break;
            }
        }
        return;
    }
    let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
    let cookie = text.lines().find_map(|l| l.strip_prefix("cookie: ").or_else(|| l.strip_prefix("Cookie: "))).unwrap_or("").to_owned();
    let body = format!("path={path}\ncookie={cookie}\n");
    let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
}

/// A fake VSCodium server install for `COMMIT` in `<dir>/<commit>`, running this test binary.
pub fn install_fake_reh(dir: &Path) {
    install_fake_reh_for(dir, COMMIT);
}

pub fn install_fake_reh_for(dir: &Path, commit: &str) {
    let d = dir.join(commit);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    std::fs::write(
        d.join("product.json"),
        serde_json::json!({ "commit": commit, "version": "1.143.17232", "serverApplicationName": "codium-server" }).to_string(),
    )
    .unwrap();
    let exe = std::env::current_exe().unwrap();
    let script = format!(
        "#!/bin/sh\nREMOTIUM_FAKE_REH=1 exec '{}' fake_reh_main --exact --include-ignored --nocapture --test-threads=1\n",
        exe.display()
    );
    let bin = d.join("bin/codium-server");
    std::fs::write(&bin, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `vscodium-reh-*.tar.gz` containing the fake server for `commit`.
pub fn fake_reh_tarball(commit: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    install_fake_reh_for(tmp.path(), commit);
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    tar.append_dir_all(".", tmp.path().join(commit)).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

/// Serve every file in `dir` at `/<name>` on a random port.
pub async fn serve_dir(dir: &Path) -> std::net::SocketAddr {
    use axum::{extract::Path as P, http::StatusCode, routing::get, Router};
    let dir = dir.to_owned();
    let app = Router::new().route(
        "/{*name}",
        get(move |P(name): P<String>| {
            let dir = dir.clone();
            async move { std::fs::read(dir.join(name.replace('/', "_"))).map_err(|_| StatusCode::NOT_FOUND) }
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

pub fn random_bytes(n: usize, seed: u8) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9 ^ seed as u32;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// Stop the servers a manager started under `state_dir` (they outlive it on purpose).
pub fn kill_servers(state_dir: &Path) {
    let Ok(rd) = std::fs::read_dir(state_dir.join("servers")) else { return };
    for e in rd.flatten() {
        let Ok(b) = std::fs::read(e.path().join("server.json")) else { continue };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else { continue };
        if let Some(pid) = v["pid"].as_i64() {
            let _ = std::process::Command::new("kill").arg("-9").arg(format!("-{pid}")).status();
        }
    }
}

/// Kills the servers under a state dir when dropped, also when a test panics.
pub struct KillOnDrop(pub std::path::PathBuf);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        kill_servers(&self.0);
    }
}
