//! M0 spike: expose a local REH on a plain WebSocket, no broker involved.
//!
//! Each accepted WebSocket is piped to a fresh TCP connection to the REH. Use only on a
//! trusted network: the REH connection token is the only protection.

use anyhow::Result;
use opentunnel_protocol::ws::WsStream;
use std::{net::SocketAddr, time::Duration};
use tokio::net::{TcpListener, TcpStream};

pub async fn serve(listen: SocketAddr, reh_port: u16) -> Result<()> {
    let listener = TcpListener::bind(listen).await?;
    tracing::info!(%listen, "direct mode listening");
    loop {
        let (tcp, peer) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle(tcp, reh_port).await {
                tracing::warn!(%peer, "direct connection ended: {e:#}");
            }
        });
    }
}

async fn handle(tcp: TcpStream, reh_port: u16) -> Result<()> {
    tcp.set_nodelay(true)?;
    let ws = tokio_tungstenite::accept_async(tcp).await?;
    let mut ws = WsStream::new(ws).with_keepalive(Duration::from_secs(25), Duration::from_secs(90));
    let mut reh = TcpStream::connect(("127.0.0.1", reh_port)).await?;
    reh.set_nodelay(true)?;
    let (up, down) = tokio::io::copy_bidirectional(&mut ws, &mut reh).await?;
    tracing::debug!(up, down, "direct connection closed");
    Ok(())
}
