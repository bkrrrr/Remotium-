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
    timeout: Duration,
    last_rx: Instant,
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
        self.keepalive = Some(Keepalive { interval: iv, timeout, last_rx: Instant::now(), ping_due: false });
        self
    }

    /// The close code the peer sent, if it sent one.
    pub fn close_code(&self) -> Option<u16> {
        self.close_code
    }

    fn poll_keepalive(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        let Some(ka) = self.keepalive.as_mut() else { return Ok(()) };
        while ka.interval.poll_tick(cx).is_ready() {
            ka.ping_due = true;
        }
        if ka.last_rx.elapsed() > ka.timeout {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "websocket keepalive timeout"));
        }
        if ka.ping_due && !self.close_sent {
            if let Poll::Ready(Ok(())) = Pin::new(&mut self.inner).poll_ready(cx) {
                Pin::new(&mut self.inner).start_send(M::ping()).map_err(io::Error::other)?;
                ka.ping_due = false;
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
            this.poll_keepalive(cx)?;
            match ready!(Pin::new(&mut this.inner).poll_next(cx)) {
                Some(Ok(msg)) => {
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
                Some(Err(e)) => return Poll::Ready(Err(io::Error::other(e))),
                None => this.read_closed = true,
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
