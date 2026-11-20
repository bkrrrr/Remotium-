mod harness;

use harness::*;
use std::time::Duration;

macro_rules! need_python {
    () => {
        if !python_available() {
            eprintln!("python3 not available; skipping");
            return;
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abrupt_client_disconnect_keeps_agent_link() {
    need_python!();
    let stack = Stack::start().await;
    let (_at, sess) = stack.session().await;
    let st = sess["streamToken"].as_str().unwrap().to_owned();
    let conn_before = stack.state.registry.get("demo").unwrap().conn_id;

    let _ = tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_default()).with_test_writer().try_init();
    let phase = |p: &str| eprintln!("[{:?}] {p}", std::time::Instant::now());
    for round in 0..5 {
        phase(&format!("round {round}: connect"));
        let mut ws = tokio::time::timeout(Duration::from_secs(10), stack.connect(&st)).await.expect("connect timed out").unwrap();
        phase("send");
        // Data in flight in both directions, then the client vanishes without a close frame.
        use futures::SinkExt;
        for _ in 0..20 {
            tokio::time::timeout(Duration::from_secs(10), ws.send(tokio_tungstenite::tungstenite::Message::Binary(random_bytes(64 * 1024, round).into())))
                .await
                .expect("send timed out")
                .unwrap();
        }
        phase("drop");
        drop(ws);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let conn_after = stack.state.registry.get("demo").expect("agent still online").conn_id;
    assert_eq!(conn_before, conn_after, "agent link must not be re-established");
    phase("final connect");
    let mut ws = tokio::time::timeout(Duration::from_secs(10), stack.connect(&st)).await.expect("final connect timed out").unwrap();
    phase("final echo");
    let got = tokio::time::timeout(Duration::from_secs(10), echo(&mut ws, b"still works")).await.expect("final echo timed out");
    assert_eq!(got, b"still works");
}

async fn refused_status(stack: &Stack, token: &str) -> u16 {
    match stack.connect(token).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
        Ok(_) => panic!("upgrade should have been refused"),
        Err(e) => panic!("unexpected error {e}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_refusals_happen_before_upgrade() {
    need_python!();
    let mut stack = Stack::start().await;
    assert_eq!(refused_status(&stack, "rms_bogus").await, 401);
    let (_at, sess) = stack.session().await;
    let st = sess["streamToken"].as_str().unwrap().to_owned();
    stack.stop_agent();
    stack.wait_online(false).await;
    assert_eq!(refused_status(&stack, &st).await, 503);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_streams_are_multiplexed() {
    need_python!();
    let stack = Stack::start().await;
    let (_at, sess) = stack.session().await;
    let st = sess["streamToken"].as_str().unwrap().to_owned();
    let mut tasks = Vec::new();
    for i in 0..32u8 {
        let mut ws = stack.connect(&st).await.unwrap();
        tasks.push(tokio::spawn(async move {
            let data = random_bytes(512 * 1024, i);
            assert_eq!(echo(&mut ws, &data).await, data, "stream {i}");
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    assert_eq!(stack.state.registry.all().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_binary_transfer() {
    need_python!();
    let stack = Stack::start().await;
    let (_at, sess) = stack.session().await;
    let mut ws = stack.connect(sess["streamToken"].as_str().unwrap()).await.unwrap();
    let data = random_bytes(64 * 1024 * 1024, 7);
    let t0 = std::time::Instant::now();
    let got = echo(&mut ws, &data).await;
    assert!(got == data, "64 MiB round trip differs");
    eprintln!("64 MiB echoed in {:?}", t0.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backpressure_bounds_buffering() {
    need_python!();
    use futures::{SinkExt, StreamExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let stack = Stack::start().await;
    let (_at, sess) = stack.session().await;
    let ws = stack.connect(sess["streamToken"].as_str().unwrap()).await.unwrap();
    let (mut tx, mut rx) = ws.split();
    let total = 600 * 1024 * 1024;
    let sent = Arc::new(AtomicUsize::new(0));
    let s2 = sent.clone();
    let writer = tokio::spawn(async move {
        let chunk = random_bytes(1024 * 1024, 3);
        while s2.load(Ordering::SeqCst) < total {
            tx.send(tokio_tungstenite::tungstenite::Message::Binary(chunk.clone().into())).await.unwrap();
            s2.fetch_add(chunk.len(), Ordering::SeqCst);
        }
        tx
    });
    // Nobody reads the echo: the whole chain must stall well before 600 MiB are accepted.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let stalled_at = sent.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(sent.load(Ordering::SeqCst), stalled_at, "writer should be blocked");
    assert!(stalled_at < 200 * 1024 * 1024, "accepted {stalled_at} bytes without a reader");
    eprintln!("writer stalled after {} MiB", stalled_at / (1024 * 1024));
    // Reading resumes the flow and everything arrives.
    let mut got = 0usize;
    while got < total {
        match rx.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(b))) => got += b.len(),
            Some(Ok(_)) => {}
            other => panic!("ended early after {got}: {other:?}"),
        }
    }
    let _ = writer.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tickets_are_single_use_and_authz_is_enforced() {
    need_python!();
    let stack = Stack::start().await;
    let t = stack.ticket().await;
    let (s, _) = stack.post("/api/v1/tokens", serde_json::json!({"ticket": t}), None).await;
    assert_eq!(s, 200);
    let (s, v) = stack.post("/api/v1/tokens", serde_json::json!({"ticket": t}), None).await;
    assert_eq!(s, 401, "reused ticket: {v}");

    // No grant for env "secret".
    let (s, _) = stack.post("/api/v1/launch-tickets", serde_json::json!({"envId": "secret", "project": "x"}), None).await;
    assert_eq!(s, 403);
    // Unknown project in an allowed env.
    let (s, _) = stack.post("/api/v1/launch-tickets", serde_json::json!({"envId": "demo", "project": "nope"}), None).await;
    assert_eq!(s, 403);
    // A token for "demo" can't open a session in another env.
    let (at, _) = stack.session().await;
    let (s, _) = stack
        .post("/api/v1/sessions", serde_json::json!({"envId": "secret", "commit": COMMIT, "version": "1.143.17232"}), Some(&at))
        .await;
    assert_eq!(s, 403);
    // No session without a token; garbage versions are rejected.
    let (s, _) = stack.post("/api/v1/sessions", serde_json::json!({"envId": "demo", "commit": COMMIT, "version": "1"}), None).await;
    assert_eq!(s, 401);
    let (s, _) = stack
        .post("/api/v1/sessions", serde_json::json!({"envId": "demo", "commit": "../../x", "version": "1"}), Some(&at))
        .await;
    assert_eq!(s, 400);
    // A plain-HTTP client from loopback gets the dev identity, but agents need a valid token.
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!("ws://{}/agent/v1/connect", stack.addr)).unwrap();
    req.headers_mut().insert("authorization", "Bearer wrong".parse().unwrap());
    match tokio_tungstenite::connect_async(req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status().as_u16(), 401),
        other => panic!("agent with a bad token: {:?}", other.map(|_| ())),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_reconnects_and_sessions_continue() {
    need_python!();
    let mut stack = Stack::start().await;
    let (at, sess) = stack.session().await;
    let sid = sess["sessionId"].as_str().unwrap().to_owned();
    let mut ws = stack.connect(sess["streamToken"].as_str().unwrap()).await.unwrap();
    assert_eq!(echo(&mut ws, b"before").await, b"before");

    stack.stop_agent();
    stack.wait_online(false).await;
    let (s, v) = stack
        .post("/api/v1/sessions", serde_json::json!({"envId": "demo", "commit": COMMIT, "version": "1.143.17232"}), Some(&at))
        .await;
    assert_eq!((s, v["error"].as_str()), (503, Some("offline")));

    stack.start_agent();
    stack.wait_online(true).await;
    // Same session, fresh stream token, same server (adopted, not restarted).
    let (s, v) = stack.post(&format!("/api/v1/sessions/{sid}/stream-tokens"), serde_json::json!({}), Some(&at)).await;
    assert_eq!(s, 200, "{v}");
    let mut ws = stack.connect(v["streamToken"].as_str().unwrap()).await.unwrap();
    assert_eq!(echo(&mut ws, b"after").await, b"after");
}
