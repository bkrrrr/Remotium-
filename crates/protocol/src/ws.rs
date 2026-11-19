//! Adapter that exposes a WebSocket as a tokio `AsyncRead + AsyncWrite` byte stream.
//!
//! Binary (and text) frames carry raw bytes. Ping/pong frames are swallowed. A close frame
//! or the end of the stream reads as EOF. With keepalive enabled the adapter sends a ping on
//! a fixed interval and fails reads when nothing at all arrived for `timeout`.

use bytes::Bytes;
use futures::{Sink, Stream};
use std::{
    io,
    marker::PhantomData,
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{Instant, Interval, MissedTickBehavior},
};

/// What a received WebSocket message means for the byte stream.
pub enum Kind {
    Data(Bytes),
    Control,
    Close(Option<u16>),
}

/// The parts of a WebSocket message type the adapter needs.
pub trait WsMessage: Sized {
    fn binary(data: Bytes) -> Self;
    fn ping() -> Self;
    fn close(code: u16, reason: &str) -> Self;
    fn kind(self) -> Kind;
}

impl WsMessage for tungstenite::Message {
    fn binary(data: Bytes) -> Self {
        tungstenite::Message::Binary(data)
    }
    fn ping() -> Self {
        tungstenite::Message::Ping(Bytes::new())
    }
    fn close(code: u16, reason: &str) -> Self {
        tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
            code: code.into(),
            reason: reason.to_owned().into(),
        }))
    }
    fn kind(self) -> Kind {
        use tungstenite::Message::*;
        match self {
            Binary(b) => Kind::Data(b),
            Text(t) => Kind::Data(Bytes::copy_from_slice(t.as_bytes())),
            Close(f) => Kind::Close(f.map(|f| f.code.into())),
            Ping(_) | Pong(_) | Frame(_) => Kind::Control,
        }
    }
}

#[cfg(feature = "axum")]
impl WsMessage for axum::extract::ws::Message {
    fn binary(data: Bytes) -> Self {
        axum::extract::ws::Message::Binary(data)
    }
    fn ping() -> Self {
        axum::extract::ws::Message::Ping(Bytes::new())
    }
    fn close(code: u16, reason: &str) -> Self {
        axum::extract::ws::Message::Close(Some(axum::extract::ws::CloseFrame {
            code,
            reason: reason.to_owned().into(),
        }))
    }
    fn kind(self) -> Kind {
        use axum::extract::ws::Message::*;
        match self {
            Binary(b) => Kind::Data(b),
            Text(t) => Kind::Data(Bytes::copy_from_slice(t.as_str().as_bytes())),
            Close(f) => Kind::Close(f.map(|f| f.code)),
            Ping(_) | Pong(_) => Kind::Control,
        }
    }
}

struct Keepalive {
    interval: Interval,
    period: Duration,
    timeout: Duration,
    last_rx: Instant,
    last_tick: Instant,
    /// The first ping sent after the last received frame.
    probe_at: Option<Instant>,
    ping_due: bool,
}

/// See the module docs.
pub struct WsStream<S, M> {
    inner: S,
    read_buf: Bytes,
    read_closed: bool,
    close_sent: bool,
    close_code: Option<u16>,
    keepalive: Option<Keepalive>,
    _msg: PhantomData<fn() -> M>,
}

impl<S, M, E> WsStream<S, M>
where
    S: Stream<Item = Result<M, E>> + Sink<M, Error = E> + Unpin,
    M: WsMessage,
    E: std::error::Error + Send + Sync + 'static,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            read_buf: Bytes::new(),
            read_closed: false,
            close_sent: false,
            close_code: None,
            keepalive: None,
            _msg: PhantomData,
        }
    }

    /// Send a ping every `interval` and fail when nothing arrives for `timeout`.
    pub fn with_keepalive(mut self, interval: Duration, timeout: Duration) -> Self {
        let mut iv = tokio::time::interval_at(Instant::now() + interval, interval);
        iv.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let now = Instant::now();
        self.keepalive = Some(Keepalive {
            interval: iv,
            period: interval,
            timeout,
            last_rx: now,
            last_tick: now,
            probe_at: None,
            ping_due: false,
        });
        self
    }

    /// The close code the peer sent, if it sent one.
    pub fn close_code(&self) -> Option<u16> {
        self.close_code
    }

    /// The peer is dead when nothing arrived for `timeout` *and* a ping we sent at least one
    /// period ago went unanswered. When our own process was stalled (a tick arrives far too
    /// late), the silence says nothing about the peer: we probe again before giving up.
    fn poll_keepalive(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        let Some(ka) = self.keepalive.as_mut() else { return Ok(()) };
        while ka.interval.poll_tick(cx).is_ready() {
            let now = Instant::now();
            if now.duration_since(ka.last_tick) > ka.period * 2 {
                tracing::debug!(stalled_secs = now.duration_since(ka.last_tick).as_secs(), "keepalive: process was stalled; probing again");
                ka.probe_at = None;
            }
            ka.last_tick = now;
            ka.ping_due = true;
        }
        let now = Instant::now();
        let unanswered = ka.probe_at.is_some_and(|p| p > ka.last_rx && now.duration_since(p) >= ka.period);
        if now.duration_since(ka.last_rx) > ka.timeout && unanswered {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "websocket keepalive timeout"));
        }
        if ka.ping_due && !self.close_sent {
            if let Poll::Ready(Ok(())) = Pin::new(&mut self.inner).poll_ready(cx) {
                Pin::new(&mut self.inner).start_send(M::ping()).map_err(io::Error::other)?;
                ka.ping_due = false;
                if ka.probe_at.is_none_or(|p| p <= ka.last_rx) {
                    ka.probe_at = Some(now);
                }
                let _ = Pin::new(&mut self.inner).poll_flush(cx);
            }
        }
        Ok(())
    }
}

impl<S, M, E> AsyncRead for WsStream<S, M>
where
    S: Stream<Item = Result<M, E>> + Sink<M, Error = E> + Unpin,
    M: WsMessage,
    E: std::error::Error + Send + Sync + 'static,
{
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if !this.read_buf.is_empty() {
                let n = this.read_buf.len().min(buf.remaining());
                buf.put_slice(&this.read_buf.split_to(n));
                return Poll::Ready(Ok(()));
            }
            if this.read_closed {
                return Poll::Ready(Ok(()));
            }
            // Read what has already arrived before judging the keepalive: after the process
            // was stalled (suspended VM, starved CPU) the deadline has passed but the peer's
            // frames are waiting in the socket.
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(msg))) => {
                    if let Some(ka) = this.keepalive.as_mut() {
                        ka.last_rx = Instant::now();
                    }
                    match msg.kind() {
                        Kind::Data(b) => this.read_buf = b,
                        Kind::Control => {}
                        Kind::Close(code) => {
                            this.close_code = code;
                            this.read_closed = true;
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(io::Error::other(e))),
                Poll::Ready(None) => this.read_closed = true,
                Poll::Pending => {
                    this.poll_keepalive(cx)?;
                    return Poll::Pending;
                }
            }
        }
    }
}

impl<S, M, E> AsyncWrite for WsStream<S, M>
where
    S: Stream<Item = Result<M, E>> + Sink<M, Error = E> + Unpin,
    M: WsMessage,
    E: std::error::Error + Send + Sync + 'static,
{
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        if self.close_sent {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let inner = &mut self.inner;
        ready!(Pin::new(&mut *inner).poll_ready(cx)).map_err(io::Error::other)?;
        Pin::new(&mut *inner)
            .start_send(M::binary(Bytes::copy_from_slice(buf)))
            .map_err(io::Error::other)?;
        // Push the frame towards the socket without waiting; poll_flush completes it.
        let _ = Pin::new(&mut *inner).poll_flush(cx);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx).map_err(io::Error::other)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if !this.close_sent {
            ready!(Pin::new(&mut this.inner).poll_ready(cx)).map_err(io::Error::other)?;
            Pin::new(&mut this.inner).start_send(M::close(1000, "")).map_err(io::Error::other)?;
            this.close_sent = true;
        }
        // Errors after our close frame (peer already gone) don't matter.
        match ready!(Pin::new(&mut this.inner).poll_close(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(_) => Poll::Ready(Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tungstenite::Message;

    /// In-memory WebSocket: frames come from a channel, sent frames are dropped.
    struct Fake {
        rx: mpsc::UnboundedReceiver<Result<Message, tungstenite::Error>>,
    }
    impl Stream for Fake {
        type Item = Result<Message, tungstenite::Error>;
        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Pin::new(&mut self.rx).poll_next(cx)
        }
    }
    impl Sink<Message> for Fake {
        type Error = tungstenite::Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stall_with_pending_data_is_not_a_timeout() {
        let (tx, rx) = mpsc::unbounded();
        let mut ws = WsStream::new(Fake { rx }).with_keepalive(Duration::from_secs(25), Duration::from_secs(75));
        // The process is suspended for longer than the timeout while the peer kept sending.
        tx.unbounded_send(Ok(Message::Binary(Bytes::from_static(b"hello")))).unwrap();
        tokio::time::advance(Duration::from_secs(200)).await;
        let mut buf = [0u8; 5];
        ws.read_exact(&mut buf).await.expect("queued data must be read, not reported as a timeout");
        assert_eq!(&buf, b"hello");
    }

    #[tokio::test(start_paused = true)]
    async fn whole_system_stall_probes_before_giving_up() {
        let (tx, rx) = mpsc::unbounded();
        let mut ws = WsStream::new(Fake { rx }).with_keepalive(Duration::from_secs(25), Duration::from_secs(75));
        // Both ends frozen for 200 s (e.g. a suspended VM): nothing is queued on resume.
        tokio::time::advance(Duration::from_secs(200)).await;
        let mut buf = [0u8; 4];
        let read = tokio::spawn(async move {
            ws.read_exact(&mut buf).await.map(|_| buf)
        });
        // The peer answers our fresh probe within the period.
        tokio::time::sleep(Duration::from_secs(10)).await;
        tx.unbounded_send(Ok(Message::Binary(Bytes::from_static(b"pong")))).unwrap();
        assert_eq!(&read.await.unwrap().expect("must survive a stall"), b"pong");
    }

    #[tokio::test(start_paused = true)]
    async fn silent_peer_times_out() {
        let (_tx, rx) = mpsc::unbounded();
        let mut ws = WsStream::new(Fake { rx }).with_keepalive(Duration::from_secs(25), Duration::from_secs(75));
        let mut buf = [0u8; 1];
        let t0 = Instant::now();
        let err = ws.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(t0.elapsed() <= Duration::from_secs(100), "gave up after {:?}", t0.elapsed());
    }
}
