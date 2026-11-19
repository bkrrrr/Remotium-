//! Runs a `yamux` connection on its own task and hands out streams.

use futures::future::poll_fn;
use std::{collections::VecDeque, io, task::Poll};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

pub use yamux::Mode;

/// A yamux stream as a tokio byte stream.
pub type MuxStream = Compat<yamux::Stream>;

type OpenReply = oneshot::Sender<Result<yamux::Stream, yamux::ConnectionError>>;

/// Opens outbound streams on a running connection. Cheap to clone.
#[derive(Clone)]
pub struct MuxHandle {
    open_tx: mpsc::UnboundedSender<OpenReply>,
}

impl MuxHandle {
    pub async fn open(&self) -> io::Result<MuxStream> {
        let (tx, rx) = oneshot::channel();
        self.open_tx
            .send(tx)
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "mux connection closed"))?;
        match rx.await {
            Ok(Ok(s)) => Ok(s.compat()),
            Ok(Err(e)) => Err(io::Error::other(e)),
            Err(_) => Err(io::Error::new(io::ErrorKind::NotConnected, "mux connection closed")),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.open_tx.is_closed()
    }
}

/// Concurrent streams per link. One editor window uses a handful (management, extension
/// host, one per forwarded port connection).
pub const MAX_STREAMS: usize = 256;

/// Bounds the memory one link can buffer. yamux needs at least 256 KiB per stream.
pub const MAX_CONNECTION_WINDOW: usize = MAX_STREAMS * 256 * 1024;

pub fn config() -> yamux::Config {
    let mut cfg = yamux::Config::default();
    cfg.set_max_num_streams(MAX_STREAMS);
    cfg.set_max_connection_receive_window(Some(MAX_CONNECTION_WINDOW));
    cfg
}

/// Start driving a yamux connection over `io`.
///
/// Returns a handle to open streams, a receiver for inbound streams and the driver task,
/// which ends when the connection does.
pub fn spawn<T>(io: T, mode: Mode) -> (MuxHandle, mpsc::UnboundedReceiver<MuxStream>, JoinHandle<io::Result<()>>)
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (open_tx, mut open_rx) = mpsc::unbounded_channel::<OpenReply>();
    let (in_tx, in_rx) = mpsc::unbounded_channel();
    let mut conn = yamux::Connection::new(io.compat(), config(), mode);
    let task = tokio::spawn(async move {
        let mut pending: VecDeque<OpenReply> = VecDeque::new();
        let mut handles_gone = false;
        let res = poll_fn(|cx| {
            while !handles_gone {
                match open_rx.poll_recv(cx) {
                    Poll::Ready(Some(tx)) => pending.push_back(tx),
                    Poll::Ready(None) => handles_gone = true,
                    Poll::Pending => break,
                }
            }
            while !pending.is_empty() {
                match conn.poll_new_outbound(cx) {
                    Poll::Ready(Ok(s)) => {
                        let _ = pending.pop_front().unwrap().send(Ok(s));
                    }
                    Poll::Ready(Err(e)) => {
                        let msg = e.to_string();
                        let _ = pending.pop_front().unwrap().send(Err(e));
                        return Poll::Ready(Err(io::Error::other(msg)));
                    }
                    Poll::Pending => break,
                }
            }
            loop {
                match conn.poll_next_inbound(cx) {
                    Poll::Ready(Some(Ok(s))) => {
                        let _ = in_tx.send(s.compat());
                    }
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(io::Error::other(e))),
                    Poll::Ready(None) => return Poll::Ready(Ok(())),
                    Poll::Pending => return Poll::Pending,
                }
            }
        })
        .await;
        drop(pending);
        // The caller may see only a symptom (a closed stream); log the cause here.
        match &res {
            Ok(()) => tracing::debug!("mux connection closed"),
            Err(e) => tracing::warn!("mux connection failed: {e}"),
        }
        res
    });
    (MuxHandle { open_tx }, in_rx, task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn open_and_accept() {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let (client, _client_in, _t1) = spawn(a, Mode::Client);
        let (_server, mut server_in, _t2) = spawn(b, Mode::Server);
        let echo = tokio::spawn(async move {
            while let Some(mut s) = server_in.recv().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    loop {
                        let n = s.read(&mut buf).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        s.write_all(&buf[..n]).await.unwrap();
                    }
                    s.shutdown().await.unwrap();
                });
            }
        });
        let mut tasks = Vec::new();
        for i in 0..8u8 {
            let client = client.clone();
            tasks.push(tokio::spawn(async move {
                let mut s = client.open().await.unwrap();
                let data = vec![i; 100_000];
                let (mut r, mut w) = tokio::io::split(&mut s);
                let wd = data.clone();
                let (_, got) = tokio::join!(
                    async move {
                        w.write_all(&wd).await.unwrap();
                        w.shutdown().await.unwrap();
                    },
                    async move {
                        let mut got = Vec::new();
                        r.read_to_end(&mut got).await.unwrap();
                        got
                    }
                );
                assert_eq!(got, data);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        echo.abort();
    }
}
