//! Remotium agent library: REH management and the server link. The binary is `main.rs`.

pub mod link;
pub mod manager;
pub mod platform;
pub mod reh;

/// Install `ring` as the process-wide rustls provider. Safe to call more than once.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
