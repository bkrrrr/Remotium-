//! Shared wire types and stream adapters for OpenTunnel.
//!
//! - [`ws`]: turns a WebSocket (tungstenite or axum) into a tokio byte stream.
//! - [`frame`]: length-prefixed JSON frames used for stream headers and the control stream.
//! - [`messages`]: control-stream and stream-header message types.
//! - [`mux`]: a small driver around a `yamux` connection.

pub mod frame;
pub mod messages;
pub mod mux;
pub mod ws;

/// Version of the agent/broker wire protocol.
pub const PROTOCOL_VERSION: u32 = 1;

/// WebSocket close codes used on client data streams.
pub mod close {
    pub const UNAUTHORIZED: u16 = 4401;
    pub const FORBIDDEN: u16 = 4403;
    pub const ENV_OFFLINE: u16 = 4404;
    pub const VERSION_UNAVAILABLE: u16 = 4409;
    pub const INTERNAL: u16 = 4500;
}
