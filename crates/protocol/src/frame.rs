//! Length-prefixed JSON frames: a big-endian `u32` length followed by that many bytes of JSON.

use serde::{de::DeserializeOwned, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Upper bound for a single frame. Control messages are tiny; this only guards against garbage.
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;

pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|l| *l <= MAX_FRAME_LEN)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&body);
    w.write_all(&buf).await?;
    w.flush().await
}

pub async fn read_frame<R, T>(r: &mut R) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{StreamHeader, StreamKind};

    #[tokio::test]
    async fn roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(64);
        let h = StreamHeader {
            v: 1,
            kind: StreamKind::Reh,
            commit: "abc".into(),
            session_id: "s1".into(),
        };
        let h2 = h.clone();
        let w = tokio::spawn(async move { write_frame(&mut a, &h2).await });
        let got: StreamHeader = read_frame(&mut b).await.unwrap();
        w.await.unwrap().unwrap();
        assert_eq!(got, h);
    }

    #[tokio::test]
    async fn rejects_oversized() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&(MAX_FRAME_LEN + 1).to_be_bytes()).await.unwrap();
        let err = read_frame::<_, serde_json::Value>(&mut b).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
