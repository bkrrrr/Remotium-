//! User identity for the dashboard and the browser-facing API, set by an upstream auth proxy.
//!
//! Sources, in order:
//! 1. a signed JWT assertion header verified against a JWKS (e.g. Cloudflare Access),
//! 2. a plain identity header (e.g. `X-Forwarded-Email`) from a trusted proxy network,
//! 3. `dev_identity` for requests from `dev_identity_from` (local development).

use crate::{config::JwtConfig, AppState};
use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{jwk::JwkSet, Algorithm, DecodingKey, Validation};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

/// The authenticated user's email (lowercased).
#[derive(Debug, Clone)]
pub struct ProxyUser(pub String);

impl FromRequestParts<Arc<AppState>> for ProxyUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Response> {
        let auth = &state.config.auth;
        let peer: Option<IpAddr> = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_canonical());
        let in_nets = |nets: &[ipnet::IpNet]| peer.is_some_and(|ip| nets.iter().any(|n| n.contains(&ip)));
        let from_proxy = in_nets(&auth.trusted_proxies);
        let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty());

        if let (Some(jwt), Some(verifier)) = (&auth.jwt, &state.jwks) {
            if let Some(token) = header(&jwt.header) {
                if jwt.require_trusted_proxy && !from_proxy {
                    tracing::warn!(?peer, "identity assertion from an untrusted address");
                    return Err(unauthorized("identity assertion from an untrusted address"));
                }
                return match verifier.verify(token).await {
                    Ok(email) => Ok(ProxyUser(email.to_ascii_lowercase())),
                    Err(e) => {
                        tracing::warn!(?peer, "identity assertion rejected: {e:#}");
                        Err(unauthorized("invalid identity assertion"))
                    }
                };
            }
        }
        if let Some(name) = &auth.trusted_header {
            if from_proxy {
                if let Some(v) = header(name) {
                    return Ok(ProxyUser(v.to_ascii_lowercase()));
                }
            }
        }
        if let Some(dev) = &auth.dev_identity {
            if in_nets(&auth.dev_identity_from) {
                return Ok(ProxyUser(dev.to_ascii_lowercase()));
            }
        }
        Err(unauthorized("sign in through the auth proxy first"))
    }
}

fn unauthorized(msg: &'static str) -> Response {
    (StatusCode::UNAUTHORIZED, msg).into_response()
}

/// Verifies identity assertions against a cached JWKS.
pub struct JwtVerifier {
    cfg: JwtConfig,
    http: reqwest::Client,
    keys: RwLock<Option<(JwkSet, Instant)>>,
    refresh: tokio::sync::Mutex<()>,
}

const JWKS_MAX_AGE: Duration = Duration::from_secs(3600);
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30);

impl JwtVerifier {
    pub fn new(cfg: JwtConfig) -> Result<Self> {
        crate::install_crypto_provider();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Self { cfg, http, keys: RwLock::new(None), refresh: Default::default() })
    }

    /// For tests: use a fixed key set.
    pub fn with_keys(cfg: JwtConfig, keys: JwkSet) -> Result<Self> {
        let v = Self::new(cfg)?;
        *v.keys.write().unwrap() = Some((keys, Instant::now() + Duration::from_secs(1_000_000)));
        Ok(v)
    }

    async fn fetch(&self, force: bool) -> Result<()> {
        let _g = self.refresh.lock().await;
        let age = self.keys.read().unwrap().as_ref().map(|(_, at)| at.elapsed());
        match age {
            Some(a) if !force && a < JWKS_MAX_AGE => return Ok(()),
            Some(a) if force && a < JWKS_MIN_REFRESH => return Ok(()),
            _ => {}
        }
        let set: JwkSet = self
            .http
            .get(&self.cfg.jwks_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .context("fetching JWKS")?
            .json()
            .await
            .context("parsing JWKS")?;
        tracing::info!(keys = set.keys.len(), url = self.cfg.jwks_url, "loaded JWKS");
        *self.keys.write().unwrap() = Some((set, Instant::now()));
        Ok(())
    }

    fn key(&self, kid: &str) -> Option<jsonwebtoken::jwk::Jwk> {
        self.keys.read().unwrap().as_ref().and_then(|(s, _)| s.find(kid).cloned())
    }

    /// Returns the configured email claim of a valid assertion.
    pub async fn verify(&self, token: &str) -> Result<String> {
        let header = jsonwebtoken::decode_header(token)?;
        let kid = header.kid.ok_or_else(|| anyhow!("assertion has no kid"))?;
        // Only asymmetric algorithms: an HMAC "key" from a public JWKS would be forgeable.
        use Algorithm::*;
        if !matches!(header.alg, RS256 | RS384 | RS512 | PS256 | PS384 | PS512 | ES256 | ES384 | EdDSA) {
            bail!("algorithm {:?} not accepted", header.alg);
        }
        self.fetch(false).await?;
        let jwk = match self.key(&kid) {
            Some(k) => k,
            None => {
                // Keys rotate; refetch once, rate-limited.
                self.fetch(true).await?;
                self.key(&kid).ok_or_else(|| anyhow!("unknown key id {kid}"))?
            }
        };
        let key = DecodingKey::from_jwk(&jwk)?;
        let mut v = Validation::new(header.alg);
        v.set_audience(&[&self.cfg.audience]);
        if let Some(iss) = &self.cfg.issuer {
            v.set_issuer(&[iss]);
        }
        v.leeway = 30;
        let data = jsonwebtoken::decode::<serde_json::Value>(token, &key, &v)?;
        data.claims
            .get(&self.cfg.email_claim)
            .and_then(|e| e.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("assertion has no {} claim", self.cfg.email_claim))
    }
}
