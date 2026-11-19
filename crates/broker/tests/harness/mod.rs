//! In-process broker + agent + fake REH (a Python echo server) for integration tests.
#![allow(dead_code)]

use futures::{SinkExt, StreamExt};
use opentunnel_agent::{link, manager};
use opentunnel_broker::{app, config::Config, AppState};
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

pub const COMMIT: &str = "c072cf642cbbadabefa22f39c58cc902a8b7427f";
pub const AGENT_TOKEN: &str = "test-agent-token";

const FAKE_SERVER: &str = r#"#!/bin/sh
exec python3 -u -c '
import socket, threading
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0)); s.listen(128)
print("Extension host agent listening on %d" % s.getsockname()[1], flush=True)
def echo(c):
    try:
        while True:
            d = c.recv(262144)
            if not d: break
            c.sendall(d)
    except OSError: pass
    c.close()
while True:
    c, _ = s.accept()
    threading.Thread(target=echo, args=(c,), daemon=True).start()
' "$@"
"#;

pub fn python_available() -> bool {
    std::process::Command::new("python3").arg("-c").arg("1").output().is_ok_and(|o| o.status.success())
}

pub fn install_fake_reh(search_dir: &Path) {
    let d = search_dir.join(COMMIT);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    std::fs::write(
        d.join("product.json"),
        serde_json::json!({"commit": COMMIT, "version": "1.143.17232", "serverApplicationName": "codium-server"}).to_string(),
    )
    .unwrap();
    let bin = d.join("bin/codium-server");
    std::fs::write(&bin, FAKE_SERVER).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub struct Stack {
    pub dir: tempfile::TempDir,
    pub addr: SocketAddr,
    pub state: Arc<AppState>,
    pub base: String,
    pub http: reqwest::Client,
    pub manager: Arc<manager::ServerManager>,
    agent: Option<JoinHandle<()>>,
    broker: JoinHandle<()>,
}

pub fn config(addr: SocketAddr) -> Config {
    let toml = format!(
        r#"
[server]
listen = "{addr}"
public_url = "http://{addr}"
keepalive_secs = 2

[auth]
dev_identity = "dev@local"

[[env]]
id = "demo"
agent_token = "{AGENT_TOKEN}"
projects = ["demo"]

[[env]]
id = "secret"
agent_token = "other-token"
projects = ["x"]

[[grant]]
users = ["dev@local"]
envs = ["demo"]
"#
    );
    let c: Config = toml::from_str(&toml).unwrap();
    c.validate().unwrap();
    c
}

impl Stack {
    pub async fn start() -> Stack {
        opentunnel_broker::install_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        install_fake_reh(&dir.path().join("prebaked"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = AppState::new(config(addr)).unwrap();
        let svc = app(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
        let broker = tokio::spawn(async move { axum::serve(listener, svc).await.unwrap() });
        let manager = manager::ServerManager::new(manager::ManagerConfig {
            cache_dir: dir.path().join("servers"),
            state_dir: dir.path().join("state"),
            search_dirs: vec![dir.path().join("prebaked")],
            url_template: "http://127.0.0.1:9/none".into(),
            require_checksum: true,
            idle_timeout: Duration::from_secs(3600),
            reap_after: Duration::from_secs(3600),
            os: "linux".into(),
            arch: "x64".into(),
        })
        .unwrap();
        let mut s = Stack {
            dir,
            addr,
            state,
            base: format!("http://{addr}"),
            http: reqwest::Client::new(),
            manager,
            agent: None,
            broker,
        };
        s.start_agent();
        s.wait_online(true).await;
        s
    }

    pub fn start_agent(&mut self) {
        let cfg = link::LinkConfig {
            broker_url: self.base.clone(),
            agent_token: AGENT_TOKEN.into(),
            agent_id: "test".into(),
            env_id: "demo".into(),
            projects: vec!["demo".into()],
            keepalive: Duration::from_secs(2),
        };
        self.agent = Some(tokio::spawn(link::run(cfg, self.manager.clone())));
    }

    pub fn stop_agent(&mut self) {
        if let Some(a) = self.agent.take() {
            a.abort();
        }
    }

    pub async fn wait_online(&self, online: bool) {
        for _ in 0..200 {
            if self.state.registry.get("demo").is_some() == online {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("env never became {}", if online { "online" } else { "offline" });
    }

    pub async fn post(&self, path: &str, body: serde_json::Value, bearer: Option<&str>) -> (u16, serde_json::Value) {
        let mut r = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(b) = bearer {
            r = r.bearer_auth(b);
        }
        let res = r.send().await.unwrap();
        let status = res.status().as_u16();
        let text = res.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
    }

    pub async fn ticket(&self) -> String {
        let (s, v) = self.post("/api/v1/launch-tickets", serde_json::json!({"envId": "demo", "project": "demo"}), None).await;
        assert_eq!(s, 200, "{v}");
        let uri = v["uri"].as_str().unwrap();
        let q = uri.split_once('?').unwrap().1;
        q.split('&').find_map(|kv| kv.strip_prefix("ticket=")).unwrap().to_owned()
    }

    /// Ticket → tokens → session. Returns (access token, session json).
    pub async fn session(&self) -> (String, serde_json::Value) {
        let t = self.ticket().await;
        let (s, pair) = self.post("/api/v1/tokens", serde_json::json!({"ticket": t}), None).await;
        assert_eq!(s, 200, "{pair}");
        let at = pair["accessToken"].as_str().unwrap().to_owned();
        let (s, sess) = self
            .post("/api/v1/sessions", serde_json::json!({"envId": "demo", "commit": COMMIT, "version": "1.143.17232"}), Some(&at))
            .await;
        assert_eq!(s, 200, "{sess}");
        (at, sess)
    }

    pub async fn connect(&self, stream_token: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/data/v1/stream", self.addr).into_client_request().unwrap();
        req.headers_mut().insert("authorization", format!("Bearer {stream_token}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await?;
        Ok(ws)
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.stop_agent();
        self.broker.abort();
        // Stop fake servers started for this stack.
        let state = self.dir.path().join("state/servers").join(COMMIT).join("server.json");
        if let Ok(b) = std::fs::read(&state) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(pid) = v["pid"].as_i64() {
                    unsafe {
                        libc_kill(-(pid as i32), 9);
                    }
                }
            }
        }
    }
}

extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

pub type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Send `data` and read the same number of bytes back.
pub async fn echo(ws: &mut Ws, data: &[u8]) -> Vec<u8> {
    let (mut tx, mut rx) = ws.split();
    let send = async {
        for chunk in data.chunks(64 * 1024) {
            tx.send(Message::Binary(chunk.to_vec().into())).await.unwrap();
        }
    };
    let recv = async {
        let mut got = Vec::with_capacity(data.len());
        while got.len() < data.len() {
            match rx.next().await {
                Some(Ok(Message::Binary(b))) => got.extend_from_slice(&b),
                Some(Ok(Message::Close(f))) => panic!("closed: {f:?}"),
                Some(Ok(_)) => {}
                other => panic!("stream ended: {other:?}"),
            }
        }
        got
    };
    let ((), got) = tokio::join!(send, recv);
    got
}

/// The close code the broker sends right after the upgrade, if any.
pub async fn close_code(ws: &mut Ws) -> Option<u16> {
    match tokio::time::timeout(Duration::from_secs(10), ws.next()).await.ok()?? {
        Ok(Message::Close(Some(f))) => Some(f.code.into()),
        _ => None,
    }
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
