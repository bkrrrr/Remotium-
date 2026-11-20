//! remotium-host end to end, in process, with the fake VSCodium server.
mod common;

use common::*;
use futures::{SinkExt, StreamExt};
use remotium_host::{manager, router, state::Store, types::ServerVersion, AppState};
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[test]
#[ignore = "fake VSCodium server, started by the tests"]
fn fake_reh_main() {
    common::fake_reh_main()
}

struct Host {
    // Dropped first, while the data dir (with server.json) still exists.
    _cleanup: KillOnDrop,
    dir: tempfile::TempDir,
    addr: SocketAddr,
    state: Arc<AppState>,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

fn version() -> ServerVersion {
    ServerVersion { commit: COMMIT.into(), quality: Some("stable".into()), version: "1.143.17232".into(), release: None }
}

fn app_state(dir: &Path) -> Arc<AppState> {
    let mgr = manager::ServerManager::new(manager::ManagerConfig {
        cache_dir: dir.join("servers"),
        state_dir: dir.join("state"),
        search_dirs: vec![dir.join("prebaked")],
        url_template: "http://127.0.0.1:9/none".into(),
        require_checksum: true,
        idle_timeout: Duration::from_secs(3600),
        reap_after: Duration::from_secs(3600),
        os: "linux".into(),
        arch: "x64".into(),
    })
    .unwrap();
    Arc::new(AppState {
        store: Store::open(dir).unwrap(),
        manager: mgr,
        name: "testhost".into(),
        folder: "/workspace/demo".into(),
        web: version(),
        tls: false,
        keepalive: Duration::from_secs(2),
        client: Some(remotium_host::client::ClientPackage { bytes: bytes::Bytes::from_static(b"PK fake vsix"), sha256: "x".into() }),
    })
}

impl Host {
    async fn start() -> Host {
        let dir = tempfile::tempdir().unwrap();
        install_fake_reh(&dir.path().join("prebaked"));
        Self::start_in(dir, None).await
    }

    async fn start_in(dir: tempfile::TempDir, addr: Option<SocketAddr>) -> Host {
        let state = app_state(dir.path());
        let want = addr.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
        // Rebinding the address of a host that was just stopped: allow the OS a moment.
        let mut tries = 0;
        let l = loop {
            match tokio::net::TcpListener::bind(want).await {
                Ok(l) => break l,
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && tries < 50 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("bind {want}: {e}"),
            }
        };
        let addr = l.local_addr().unwrap();
        let svc = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
        let task = tokio::spawn(async move { axum::serve(l, svc).await.unwrap() });
        let cleanup = KillOnDrop(dir.path().join("state"));
        Host {
            http: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
            dir,
            addr,
            state,
            task,
            _cleanup: cleanup,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Pair a desktop client; returns its device token.
    async fn pair_desktop(&self) -> String {
        let code = self.state.store.create_pairing(None).unwrap();
        let res = self.http.post(self.url("/remotium/v1/pair")).json(&serde_json::json!({ "code": code })).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        res.json::<serde_json::Value>().await.unwrap()["deviceToken"].as_str().unwrap().to_owned()
    }

    async fn session(&self, token: &str) -> reqwest::Response {
        self.http.post(self.url("/remotium/v1/session")).bearer_auth(token).json(&version()).send().await.unwrap()
    }

    async fn stream(&self, token: &str, commit: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/remotium/v1/stream?commit={commit}", self.addr).into_client_request().unwrap();
        req.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
        tokio_tungstenite::connect_async(req).await.map(|(ws, _)| ws)
    }

    /// Pair a browser through the form; returns the device cookie.
    async fn pair_browser(&self) -> String {
        let code = self.state.store.create_pairing(None).unwrap();
        let page = self.http.get(self.url(&format!("/pair?code={code}"))).send().await.unwrap();
        assert_eq!(page.status().as_u16(), 200);
        assert!(page.text().await.unwrap().contains("remotium/client.vsix"), "pairing page offers the client");
        let res = self.http.post(self.url("/pair")).form(&[("code", code.as_str()), ("name", "test")]).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 303);
        let cookie = res.headers()["set-cookie"].to_str().unwrap().to_owned();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        cookie.split(';').next().unwrap().to_owned()
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn echo(ws: &mut Ws, data: &[u8]) -> Vec<u8> {
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

fn refused(r: Result<Ws, tokio_tungstenite::tungstenite::Error>) -> u16 {
    match r {
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => resp.status().as_u16(),
        Ok(_) => panic!("upgrade should have been refused"),
        Err(e) => panic!("unexpected error {e}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desktop_pairing_session_and_stream() {
    let h = Host::start().await;
    // Unpaired: nothing works, and refusals come before the WebSocket upgrade.
    assert_eq!(h.session("rmd_nope").await.status().as_u16(), 401);
    assert_eq!(refused(h.stream("rmd_nope", COMMIT).await), 401);

    let token = h.pair_desktop().await;
    let info = h.http.get(h.url("/remotium/v1/info")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(info.json::<serde_json::Value>().await.unwrap()["folder"], "/workspace/demo");
    let res = h.session(&token).await;
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.json::<serde_json::Value>().await.unwrap()["connectionToken"].as_str().unwrap().len(), 64);
    assert_eq!(refused(h.stream(&token, "not-a-commit").await), 400);

    let mut ws = h.stream(&token, COMMIT).await.unwrap();
    assert_eq!(echo(&mut ws, b"hello over the host").await, b"hello over the host");

    // Revoked: the device is locked out.
    let id = h.state.store.devices()[0].id.clone();
    assert!(h.state.store.revoke(&id).unwrap());
    assert_eq!(h.session(&token).await.status().as_u16(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offers_the_client_without_pairing() {
    let h = Host::start().await;
    let res = h.http.get(h.url("/remotium/client.vsix")).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert!(res.headers()["content-disposition"].to_str().unwrap().contains("remotium-client.vsix"));
    assert_eq!(res.bytes().await.unwrap().as_ref(), b"PK fake vsix");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_codes_are_single_use() {
    let h = Host::start().await;
    let code = h.state.store.create_pairing(None).unwrap();
    let pair = || h.http.post(h.url("/remotium/v1/pair")).json(&serde_json::json!({ "code": code })).send();
    assert_eq!(pair().await.unwrap().status().as_u16(), 200);
    assert_eq!(pair().await.unwrap().status().as_u16(), 401);
    let browser = h.http.post(h.url("/pair")).form(&[("code", code.as_str())]).send().await.unwrap();
    assert_eq!(browser.status().as_u16(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_proxy_requires_pairing_and_injects_the_server_token() {
    let h = Host::start().await;
    let res = h.http.get(h.url("/some/page?x=1")).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 401, "unpaired browsers get nothing");

    let cookie = h.pair_browser().await;
    let res = h.http.get(h.url("/some/page?x=1")).header("cookie", format!("other=1; {cookie}")).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let set: Vec<String> = res.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_owned()).collect();
    let body = res.text().await.unwrap();
    assert!(body.contains("path=/some/page?x=1"), "{body}");
    assert!(body.contains("vscode-tkn="), "the server gets its token: {body}");
    assert!(body.contains("other=1"), "other cookies pass through: {body}");
    assert!(!body.contains("remotium_device"), "the device cookie stays at the host: {body}");
    assert!(set.iter().any(|c| c.starts_with("vscode-tkn=") && c.contains("SameSite=Strict")), "workbench gets the token cookie: {set:?}");

    // The workbench's WebSocket goes through the proxy too.
    let mut req = format!("ws://{}/stable-x?reconnectionToken=1", h.addr).into_client_request().unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(Message::Binary(b"browser".to_vec().into())).await.unwrap();
    let got = ws.next().await.unwrap().unwrap();
    assert_eq!(got.into_data().as_ref(), b"browser");

    // A revoked browser is locked out.
    let id = h.state.store.devices()[0].id.clone();
    h.state.store.revoke(&id).unwrap();
    let res = h.http.get(h.url("/")).header("cookie", cookie).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_streams_and_large_transfer() {
    let h = Host::start().await;
    let token = h.pair_desktop().await;
    assert_eq!(h.session(&token).await.status().as_u16(), 200);
    let mut tasks = Vec::new();
    for i in 0..16u8 {
        let mut ws = h.stream(&token, COMMIT).await.unwrap();
        tasks.push(tokio::spawn(async move {
            let data = random_bytes(256 * 1024, i);
            assert_eq!(echo(&mut ws, &data).await, data);
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let mut ws = h.stream(&token, COMMIT).await.unwrap();
    let data = random_bytes(64 * 1024 * 1024, 7);
    assert!(echo(&mut ws, &data).await == data, "64 MiB round trip");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backpressure_bounds_buffering() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let h = Host::start().await;
    let token = h.pair_desktop().await;
    h.session(&token).await;
    let ws = h.stream(&token, COMMIT).await.unwrap();
    let (mut tx, mut rx) = ws.split();
    let total = 600 * 1024 * 1024;
    let sent = Arc::new(AtomicUsize::new(0));
    let s2 = sent.clone();
    let writer = tokio::spawn(async move {
        let chunk = random_bytes(1024 * 1024, 3);
        while s2.load(Ordering::SeqCst) < total {
            tx.send(Message::Binary(chunk.clone().into())).await.unwrap();
            s2.fetch_add(chunk.len(), Ordering::SeqCst);
        }
        tx
    });
    tokio::time::sleep(Duration::from_secs(4)).await;
    let stalled = sent.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(sent.load(Ordering::SeqCst), stalled, "writer must be blocked while nobody reads");
    assert!(stalled < 200 * 1024 * 1024, "accepted {stalled} bytes without a reader");
    let mut got = 0;
    while got < total {
        match rx.next().await {
            Some(Ok(Message::Binary(b))) => got += b.len(),
            Some(Ok(_)) => {}
            other => panic!("ended after {got}: {other:?}"),
        }
    }
    let _ = writer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_restart_keeps_devices_and_the_running_server() {
    let h = Host::start().await;
    let token = h.pair_desktop().await;
    let first = h.session(&token).await.json::<serde_json::Value>().await.unwrap()["connectionToken"].clone();
    let mut ws = h.stream(&token, COMMIT).await.unwrap();
    assert_eq!(echo(&mut ws, b"before").await, b"before");
    let pid = std::fs::read_to_string(h.dir.path().join("state/servers").join(COMMIT).join("server.json")).unwrap();

    // Stop the host (the server keeps running) and start a new one on the same data.
    let Host { dir, addr, task, state, _cleanup, .. } = h;
    task.abort();
    // abort() only requests cancellation; wait until the task (and its listener) is gone.
    let _ = task.await;
    drop(state);
    std::mem::forget(_cleanup); // the new host's guard cleans up
    let h = Host::start_in(dir, Some(addr)).await;
    // The old connection survives (the server wasn't restarted)...
    assert_eq!(echo(&mut ws, b"still here").await, b"still here");
    // ...and the paired device works with the restarted host.
    let again = h.session(&token).await.json::<serde_json::Value>().await.unwrap()["connectionToken"].clone();
    assert_eq!(first, again, "same server, same token");
    let mut ws2 = h.stream(&token, COMMIT).await.unwrap();
    assert_eq!(echo(&mut ws2, b"after").await, b"after");
    assert_eq!(pid, std::fs::read_to_string(h.dir.path().join("state/servers").join(COMMIT).join("server.json")).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_with_the_self_signed_certificate() {
    remotium_host::install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    install_fake_reh(&dir.path().join("prebaked"));
    let files = remotium_host::tls::prepare(&dir.path().join("tls"), None, &["localhost".into(), "127.0.0.1".into()]).unwrap();
    let state = app_state(dir.path());
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (cert, key) = (files.cert.clone(), files.key.clone());
    let srv = tokio::spawn(async move {
        remotium_host::serve(addr, router(state), remotium_host::Listen::Tls { cert, key }).await.unwrap();
    });
    let _cleanup = KillOnDrop(dir.path().join("state"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The certificate the host presents is the one whose fingerprint pairing links carry.
    let der = tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("openssl")
            .args(["s_client", "-connect", &format!("127.0.0.1:{port}")])
            .stdin(std::process::Stdio::null())
            .output();
        out.ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    })
    .await
    .unwrap();
    if let Some(text) = der.filter(|t| t.contains("BEGIN CERTIFICATE")) {
        assert_eq!(remotium_host::tls::fingerprint_of(&text).unwrap(), files.fingerprint);
    }
    let client = reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();
    let res = client.get(format!("https://127.0.0.1:{port}/remotium/healthz")).send().await.unwrap();
    assert_eq!(res.text().await.unwrap(), "ok");
    srv.abort();
}
