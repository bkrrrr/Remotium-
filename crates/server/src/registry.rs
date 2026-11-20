//! Connected agents, one per environment.

use remotium_protocol::{
    frame::{read_frame, write_frame},
    messages::{ServerMsg, EnsureResult, ErrorCode, ServerVersion, StreamHeader, StreamReply},
    mux::{MuxHandle, MuxStream},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub struct AgentConn {
    /// Unique per connection, so a stale disconnect never removes a newer agent.
    pub conn_id: u64,
    pub info: AgentInfo,
    pub mux: MuxHandle,
    ctrl_tx: mpsc::UnboundedSender<ServerMsg>,
    pending: Mutex<HashMap<u64, oneshot::Sender<EnsureResult>>>,
    next_req: AtomicU64,
    pub open_streams: AtomicUsize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent_id: String,
    pub env_id: String,
    pub projects: Vec<String>,
    pub arch: String,
    pub version: String,
    pub connected_at: u64,
    pub remote_addr: String,
}

#[derive(Debug)]
pub struct RouteError {
    pub code: ErrorCode,
    pub message: String,
}

impl RouteError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// Counts an open client stream for as long as it lives.
pub struct StreamGuard(Arc<AgentConn>);

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.open_streams.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AgentConn {
    pub fn new(conn_id: u64, info: AgentInfo, mux: MuxHandle, ctrl_tx: mpsc::UnboundedSender<ServerMsg>) -> Self {
        Self {
            conn_id,
            info,
            mux,
            ctrl_tx,
            pending: Mutex::new(HashMap::new()),
            next_req: AtomicU64::new(1),
            open_streams: AtomicUsize::new(0),
        }
    }

    /// Ask the agent to install and start the server for `v`. Can take minutes on first use.
    pub async fn ensure_server(&self, v: &ServerVersion, timeout: Duration) -> Result<String, RouteError> {
        let id = self.next_req.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.ctrl_tx.send(ServerMsg::EnsureServer { id, server: v.clone() }).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(RouteError::new(ErrorCode::Offline, "environment disconnected"));
        }
        let res = tokio::time::timeout(timeout, rx).await;
        self.pending.lock().unwrap().remove(&id);
        match res {
            Err(_) => Err(RouteError::new(ErrorCode::Internal, "timed out waiting for the environment to start the server")),
            Ok(Err(_)) => Err(RouteError::new(ErrorCode::Offline, "environment disconnected")),
            Ok(Ok(EnsureResult::Ok { connection_token })) => Ok(connection_token),
            Ok(Ok(EnsureResult::Error { code, message })) => Err(RouteError { code, message }),
        }
    }

    pub fn complete(&self, id: u64, result: EnsureResult) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(result);
        }
    }

    pub fn send(&self, msg: ServerMsg) -> bool {
        self.ctrl_tx.send(msg).is_ok()
    }

    /// Open a data stream to the REH for `commit`.
    pub async fn open_stream(self: &Arc<Self>, commit: &str, session_id: &str) -> Result<(MuxStream, StreamGuard), RouteError> {
        let internal = |e: std::io::Error| RouteError::new(ErrorCode::Offline, format!("environment link: {e}"));
        let mut s = self.mux.open().await.map_err(internal)?;
        self.open_streams.fetch_add(1, Ordering::SeqCst);
        let guard = StreamGuard(self.clone());
        let header = StreamHeader {
            v: remotium_protocol::PROTOCOL_VERSION,
            kind: remotium_protocol::messages::StreamKind::Reh,
            commit: commit.into(),
            session_id: session_id.into(),
        };
        write_frame(&mut s, &header).await.map_err(internal)?;
        let reply: StreamReply = tokio::time::timeout(Duration::from_secs(120), read_frame(&mut s))
            .await
            .map_err(|_| RouteError::new(ErrorCode::Internal, "environment did not answer"))?
            .map_err(internal)?;
        if !reply.ok {
            return Err(RouteError::new(
                reply.code.unwrap_or(ErrorCode::Internal),
                reply.error.unwrap_or_else(|| "stream refused".into()),
            ));
        }
        Ok((s, guard))
    }
}

#[derive(Default)]
pub struct Registry {
    agents: RwLock<HashMap<String, Arc<AgentConn>>>,
    next_conn: AtomicU64,
}

impl Registry {
    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Register an agent; a previous connection for the same env is replaced.
    pub fn insert(&self, conn: Arc<AgentConn>) -> Option<Arc<AgentConn>> {
        self.agents.write().unwrap().insert(conn.info.env_id.clone(), conn)
    }

    pub fn remove(&self, env: &str, conn_id: u64) {
        let mut g = self.agents.write().unwrap();
        if g.get(env).is_some_and(|c| c.conn_id == conn_id) {
            g.remove(env);
        }
    }

    pub fn get(&self, env: &str) -> Option<Arc<AgentConn>> {
        self.agents.read().unwrap().get(env).filter(|c| !c.mux.is_closed()).cloned()
    }

    pub fn all(&self) -> Vec<Arc<AgentConn>> {
        self.agents.read().unwrap().values().cloned().collect()
    }
}
