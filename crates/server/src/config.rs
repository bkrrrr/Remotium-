//! Server configuration (`server.toml`).

use anyhow::{bail, Context, Result};
use ipnet::IpNet;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default, rename = "env")]
    pub envs: Vec<EnvConfig>,
    #[serde(default, rename = "grant")]
    pub grants: Vec<Grant>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Listener for everything (dashboard, control API, data plane).
    pub listen: SocketAddr,
    /// Optional separate listener for the data plane (`/data/*`, `/agent/*`). When set,
    /// `listen` serves only the dashboard and control API.
    pub data_listen: Option<SocketAddr>,
    /// The URL clients use to reach this server, e.g. `https://tunnels.example.com`.
    pub public_url: String,
    /// Public URL of the data plane if it differs from `public_url`.
    pub public_data_url: Option<String>,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    /// Where issued client tokens are kept (hashed) so they survive a restart.
    pub state_file: Option<PathBuf>,
    #[serde(default = "default_keepalive")]
    pub keepalive_secs: u64,
    /// URI scheme of the editor the dashboard links to.
    #[serde(default = "default_scheme")]
    pub editor_scheme: String,
    /// The client package offered at `/downloads/remotium-client.vsix`. Defaults to the
    /// file baked into the server image, if present.
    pub client_vsix: Option<PathBuf>,
    /// Set once the client is published on Open VSX: the dashboard then links to the store.
    #[serde(default)]
    pub client_on_open_vsx: bool,
}

fn default_keepalive() -> u64 {
    25
}
fn default_scheme() -> String {
    "vscodium".into()
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Header that carries the user's identity, set by the auth proxy.
    pub trusted_header: Option<String>,
    /// The header is only believed from these source networks.
    #[serde(default)]
    pub trusted_proxies: Vec<IpNet>,
    /// Signed identity assertions (e.g. Cloudflare Access), verified against a JWKS.
    pub jwt: Option<JwtConfig>,
    /// Local development without a proxy: every request from `dev_identity_from` is this user.
    pub dev_identity: Option<String>,
    /// Networks the dev identity applies to. Defaults to loopback only.
    #[serde(default = "loopback")]
    pub dev_identity_from: Vec<IpNet>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { trusted_header: None, trusted_proxies: vec![], jwt: None, dev_identity: None, dev_identity_from: loopback() }
    }
}

fn loopback() -> Vec<IpNet> {
    vec!["127.0.0.0/8".parse().unwrap(), "::1/128".parse().unwrap()]
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct JwtConfig {
    pub header: String,
    pub jwks_url: String,
    pub audience: String,
    pub issuer: Option<String>,
    #[serde(default = "default_email_claim")]
    pub email_claim: String,
    /// Only accept the assertion from `trusted_proxies` (recommended).
    #[serde(default = "yes")]
    pub require_trusted_proxy: bool,
}

fn default_email_claim() -> String {
    "email".into()
}
fn yes() -> bool {
    true
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// SHA-256 (hex) of the agent token. Generate with `remotium-server hash-token`.
    pub agent_token_sha256: Option<String>,
    /// Plain agent token, for local development only.
    pub agent_token: Option<String>,
    /// Projects users may open. Empty: whatever the agent advertises.
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default = "default_workspace_root")]
    pub workspace_root: String,
}

fn default_workspace_root() -> String {
    "/workspace".into()
}

impl EnvConfig {
    pub fn token_hash(&self) -> Option<[u8; 32]> {
        if let Some(h) = &self.agent_token_sha256 {
            let mut out = [0u8; 32];
            hex::decode_to_slice(h.trim(), &mut out).ok()?;
            return Some(out);
        }
        self.agent_token.as_ref().map(|t| Sha256::digest(t.as_bytes()).into())
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Who may open what. `users` entries are exact emails, `*@domain` or `*`.
/// `envs` and `projects` entries are ids or `*`.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub users: Vec<String>,
    pub envs: Vec<String>,
    #[serde(default = "star")]
    pub projects: Vec<String>,
}

fn star() -> Vec<String> {
    vec!["*".into()]
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if !(self.server.public_url.starts_with("https://") || self.server.public_url.starts_with("http://")) {
            bail!("server.public_url must be an http(s) URL");
        }
        if self.server.tls_cert.is_some() != self.server.tls_key.is_some() {
            bail!("server.tls_cert and server.tls_key must be set together");
        }
        let mut seen = std::collections::HashSet::new();
        for e in &self.envs {
            if !valid_id(&e.id) {
                bail!("env id {:?} must be 1-64 chars of [A-Za-z0-9_-]", e.id);
            }
            if !seen.insert(&e.id) {
                bail!("duplicate env id {:?}", e.id);
            }
            if e.token_hash().is_none() {
                bail!("env {:?} needs agent_token_sha256 (64 hex chars) or agent_token", e.id);
            }
        }
        if self.auth.trusted_header.is_some() && self.auth.trusted_proxies.is_empty() {
            bail!("auth.trusted_header needs auth.trusted_proxies");
        }
        Ok(())
    }

    pub fn env(&self, id: &str) -> Option<&EnvConfig> {
        self.envs.iter().find(|e| e.id == id)
    }

    /// Is `user` allowed to open `project` in `env`? `project = None` asks about the env at all.
    pub fn allowed(&self, user: &str, env: &str, project: Option<&str>) -> bool {
        self.grants.iter().any(|g| {
            g.users.iter().any(|u| user_matches(u, user))
                && g.envs.iter().any(|e| e == "*" || e == env)
                && project.is_none_or(|p| g.projects.iter().any(|gp| gp == "*" || gp == p))
        })
    }

    pub fn data_url(&self) -> &str {
        self.server.public_data_url.as_deref().unwrap_or(&self.server.public_url)
    }
}

pub fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn user_matches(pattern: &str, user: &str) -> bool {
    let user = user.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    if let Some(domain) = pattern.strip_prefix("*@") {
        return user.rsplit_once('@').is_some_and(|(_, d)| d == domain);
    }
    pattern == user
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[server]
listen = "127.0.0.1:8080"
public_url = "http://localhost:8080"

[auth]
dev_identity = "dev@local"

[[env]]
id = "demo"
agent_token = "secret"

[[env]]
id = "other"
agent_token_sha256 = "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"

[[grant]]
users = ["dev@local", "*@example.com"]
envs = ["demo"]

[[grant]]
users = ["alice@example.com"]
envs = ["other"]
projects = ["api"]
"#;

    #[test]
    fn parses_and_authorizes() {
        let c: Config = toml::from_str(SAMPLE).unwrap();
        c.validate().unwrap();
        assert!(c.allowed("dev@local", "demo", Some("anything")));
        assert!(c.allowed("Bob@Example.com", "demo", None));
        assert!(!c.allowed("bob@evil.com", "demo", None));
        assert!(c.allowed("alice@example.com", "other", Some("api")));
        assert!(!c.allowed("alice@example.com", "other", Some("web")));
        assert!(!c.allowed("dev@local", "other", None));
        assert_eq!(c.env("other").unwrap().token_hash().unwrap(), <[u8; 32]>::from(Sha256::digest(b"secret")));
    }

    #[test]
    fn rejects_bad_env_ids() {
        let bad = SAMPLE.replace("id = \"demo\"", "id = \"de mo\"");
        let c: Config = toml::from_str(&bad).unwrap();
        assert!(c.validate().is_err());
    }
}
