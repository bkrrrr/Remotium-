//! Tickets, client tokens, sessions and stream tokens.
//!
//! Every secret is random (32 bytes, base64url) and stored only as its SHA-256. Access and
//! refresh tokens are persisted to `state_file` so clients stay signed in across a server
//! restart; tickets, sessions and stream tokens are short-lived and kept in memory.
//!
//! Refresh tokens rotate. Presenting an already-rotated refresh token revokes its whole
//! family (all tokens and sessions from the same sign-in).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const TICKET_TTL: u64 = 60;
pub const ACCESS_TTL: u64 = 15 * 60;
pub const REFRESH_TTL: u64 = 12 * 3600;
pub const STREAM_TTL: u64 = 60;
/// A session with no stream activity for this long is forgotten.
pub const SESSION_IDLE_TTL: u64 = 24 * 3600;
/// A rotated refresh token still works this long, so two editor windows that share tokens
/// and refresh at the same moment don't sign each other out.
pub const REFRESH_REUSE_GRACE: u64 = 60;

type Hash = String;

fn hash(token: &str) -> Hash {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn random_secret(prefix: &str) -> String {
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[derive(Clone, Debug)]
struct Ticket {
    user: String,
    env: String,
    project: String,
    expires: u64,
}

/// What an access token stands for.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientGrant {
    pub user: String,
    pub env: String,
    pub family: String,
    pub expires: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RefreshEntry {
    grant: ClientGrant,
    /// When it was rotated; kept until expiry to detect reuse.
    #[serde(default)]
    used_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub user: String,
    pub env: String,
    pub commit: String,
    pub family: String,
    pub created: u64,
    pub last_used: u64,
}

#[derive(Clone, Debug)]
struct StreamToken {
    session: String,
    expires: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    access: HashMap<Hash, ClientGrant>,
    refresh: HashMap<Hash, RefreshEntry>,
    revoked_families: HashMap<String, u64>,
}

#[derive(Default)]
struct Inner {
    p: Persisted,
    tickets: HashMap<Hash, Ticket>,
    sessions: HashMap<String, Session>,
    streams: HashMap<Hash, StreamToken>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPair {
    pub access_token: String,
    pub access_expires_in: u64,
    pub refresh_token: String,
    pub refresh_expires_in: u64,
    pub user: String,
    pub env: String,
    /// Only set when the pair came from a ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TokenError {
    Invalid,
    Expired,
    Reused,
}

pub struct Tokens {
    inner: Mutex<Inner>,
    state_file: Option<PathBuf>,
}

impl Tokens {
    pub fn new(state_file: Option<PathBuf>) -> Self {
        let p = state_file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| match serde_json::from_slice::<Persisted>(&b) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("ignoring unreadable token state: {e}");
                    None
                }
            })
            .unwrap_or_default();
        Self { inner: Mutex::new(Inner { p, ..Default::default() }), state_file }
    }

    fn save(&self, inner: &Inner) {
        let Some(f) = &self.state_file else { return };
        let tmp = f.with_extension("tmp");
        let res = serde_json::to_vec(&inner.p)
            .map_err(std::io::Error::other)
            .and_then(|b| write_private(&tmp, &b))
            .and_then(|_| std::fs::rename(&tmp, f));
        if let Err(e) = res {
            tracing::error!(file = %f.display(), "saving token state failed: {e}");
        }
    }

    pub fn issue_ticket(&self, user: &str, env: &str, project: &str) -> String {
        let t = random_secret("rmt_");
        let mut g = self.inner.lock().unwrap();
        g.tickets.insert(
            hash(&t),
            Ticket { user: user.into(), env: env.into(), project: project.into(), expires: now() + TICKET_TTL },
        );
        t
    }

    /// Single use: the ticket is gone after this call, whatever the outcome.
    pub fn redeem_ticket(&self, ticket: &str) -> Result<TokenPair, TokenError> {
        let mut g = self.inner.lock().unwrap();
        let t = g.tickets.remove(&hash(ticket)).ok_or(TokenError::Invalid)?;
        if t.expires < now() {
            return Err(TokenError::Expired);
        }
        let family = random_secret("");
        let mut pair = Self::issue_pair(&mut g, &t.user, &t.env, &family);
        pair.project = Some(t.project);
        self.save(&g);
        Ok(pair)
    }

    fn issue_pair(g: &mut Inner, user: &str, env: &str, family: &str) -> TokenPair {
        let at = random_secret("rma_");
        let rt = random_secret("rmr_");
        let n = now();
        let grant = |expires| ClientGrant { user: user.into(), env: env.into(), family: family.into(), expires };
        g.p.access.insert(hash(&at), grant(n + ACCESS_TTL));
        g.p.refresh.insert(hash(&rt), RefreshEntry { grant: grant(n + REFRESH_TTL), used_at: None });
        TokenPair {
            access_token: at,
            access_expires_in: ACCESS_TTL,
            refresh_token: rt,
            refresh_expires_in: REFRESH_TTL,
            user: user.into(),
            env: env.into(),
            project: None,
        }
    }

    pub fn refresh(&self, refresh_token: &str) -> Result<TokenPair, TokenError> {
        let mut g = self.inner.lock().unwrap();
        let key = hash(refresh_token);
        let entry = g.p.refresh.get(&key).cloned().ok_or(TokenError::Invalid)?;
        if g.p.revoked_families.contains_key(&entry.grant.family) {
            return Err(TokenError::Invalid);
        }
        if entry.used_at.is_some_and(|t| now() > t + REFRESH_REUSE_GRACE) {
            tracing::warn!(user = entry.grant.user, "refresh token reuse; revoking its sign-in");
            Self::revoke_family_locked(&mut g, &entry.grant.family);
            self.save(&g);
            return Err(TokenError::Reused);
        }
        if entry.grant.expires < now() {
            return Err(TokenError::Expired);
        }
        let e = g.p.refresh.get_mut(&key).unwrap();
        e.used_at.get_or_insert(now());
        let pair = Self::issue_pair(&mut g, &entry.grant.user, &entry.grant.env, &entry.grant.family);
        self.save(&g);
        Ok(pair)
    }

    pub fn check_access(&self, access_token: &str) -> Result<ClientGrant, TokenError> {
        let g = self.inner.lock().unwrap();
        let grant = g.p.access.get(&hash(access_token)).ok_or(TokenError::Invalid)?;
        if g.p.revoked_families.contains_key(&grant.family) {
            return Err(TokenError::Invalid);
        }
        if grant.expires < now() {
            return Err(TokenError::Expired);
        }
        Ok(grant.clone())
    }

    /// Sign out: revoke everything issued from the sign-in the refresh token belongs to.
    pub fn revoke_by_refresh(&self, refresh_token: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let Some(family) = g.p.refresh.get(&hash(refresh_token)).map(|e| e.grant.family.clone()) else { return false };
        Self::revoke_family_locked(&mut g, &family);
        self.save(&g);
        true
    }

    fn revoke_family_locked(g: &mut Inner, family: &str) {
        g.p.revoked_families.insert(family.into(), now() + REFRESH_TTL);
        g.p.access.retain(|_, a| a.family != family);
        g.p.refresh.retain(|_, r| r.grant.family != family);
        let dead: Vec<String> = g.sessions.values().filter(|s| s.family == family).map(|s| s.id.clone()).collect();
        for id in dead {
            g.sessions.remove(&id);
        }
        g.streams.retain(|_, s| g.sessions.contains_key(&s.session));
    }

    pub fn create_session(&self, grant: &ClientGrant, commit: &str) -> (Session, String) {
        let n = now();
        let s = Session {
            id: random_secret("ses_"),
            user: grant.user.clone(),
            env: grant.env.clone(),
            commit: commit.into(),
            family: grant.family.clone(),
            created: n,
            last_used: n,
        };
        let mut g = self.inner.lock().unwrap();
        g.sessions.insert(s.id.clone(), s.clone());
        let st = Self::stream_token_locked(&mut g, &s.id);
        (s, st)
    }

    fn stream_token_locked(g: &mut Inner, session: &str) -> String {
        let t = random_secret("rms_");
        g.streams.insert(hash(&t), StreamToken { session: session.into(), expires: now() + STREAM_TTL });
        t
    }

    /// A fresh stream token for an existing session owned by `grant`'s sign-in.
    pub fn new_stream_token(&self, session_id: &str, grant: &ClientGrant) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        let s = g.sessions.get(session_id)?;
        if s.family != grant.family {
            return None;
        }
        Some(Self::stream_token_locked(&mut g, session_id))
    }

    /// Stream tokens are reusable until they expire, for the reconnects of one session.
    pub fn check_stream(&self, stream_token: &str) -> Result<Session, TokenError> {
        let mut g = self.inner.lock().unwrap();
        let st = g.streams.get(&hash(stream_token)).cloned().ok_or(TokenError::Invalid)?;
        if st.expires < now() {
            return Err(TokenError::Expired);
        }
        let s = g.sessions.get_mut(&st.session).ok_or(TokenError::Invalid)?;
        s.last_used = now();
        Ok(s.clone())
    }

    pub fn session(&self, id: &str) -> Option<Session> {
        self.inner.lock().unwrap().sessions.get(id).cloned()
    }

    pub fn touch_session(&self, id: &str) {
        if let Some(s) = self.inner.lock().unwrap().sessions.get_mut(id) {
            s.last_used = now();
        }
    }

    pub fn revoke_session(&self, id: &str, grant: &ClientGrant) -> bool {
        let mut g = self.inner.lock().unwrap();
        match g.sessions.get(id) {
            Some(s) if s.family == grant.family => {
                g.sessions.remove(id);
                g.streams.retain(|_, s| s.session != id);
                true
            }
            _ => false,
        }
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.inner.lock().unwrap().sessions.values().cloned().collect()
    }

    /// Drop expired entries. Call periodically.
    pub fn gc(&self) {
        let n = now();
        let mut g = self.inner.lock().unwrap();
        let before = (g.p.access.len(), g.p.refresh.len(), g.p.revoked_families.len());
        g.tickets.retain(|_, t| t.expires >= n);
        g.streams.retain(|_, s| s.expires >= n);
        g.sessions.retain(|_, s| s.last_used + SESSION_IDLE_TTL >= n);
        g.p.access.retain(|_, a| a.expires >= n);
        g.p.refresh.retain(|_, r| r.grant.expires >= n);
        g.p.revoked_families.retain(|_, until| *until >= n);
        if before != (g.p.access.len(), g.p.refresh.len(), g.p.revoked_families.len()) {
            self.save(&g);
        }
    }

    #[cfg(test)]
    fn age_rotations(&self, secs: u64) {
        for r in self.inner.lock().unwrap().p.refresh.values_mut() {
            if let Some(t) = r.used_at.as_mut() {
                *t -= secs;
            }
        }
    }

    #[cfg(test)]
    fn expire_all_tickets(&self) {
        for t in self.inner.lock().unwrap().tickets.values_mut() {
            t.expires = 0;
        }
    }
}

fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_is_single_use_and_expires() {
        let t = Tokens::new(None);
        let tk = t.issue_ticket("u", "e", "p");
        let pair = t.redeem_ticket(&tk).unwrap();
        assert_eq!(pair.project.as_deref(), Some("p"));
        assert_eq!(t.redeem_ticket(&tk).unwrap_err(), TokenError::Invalid);
        let tk2 = t.issue_ticket("u", "e", "p");
        t.expire_all_tickets();
        assert_eq!(t.redeem_ticket(&tk2).unwrap_err(), TokenError::Expired);
    }

    #[test]
    fn refresh_rotates_and_detects_reuse() {
        let t = Tokens::new(None);
        let p1 = t.redeem_ticket(&t.issue_ticket("u", "e", "p")).unwrap();
        let g = t.check_access(&p1.access_token).unwrap();
        let (s, st) = t.create_session(&g, "abc");
        assert_eq!(t.check_stream(&st).unwrap().id, s.id);

        let p2 = t.refresh(&p1.refresh_token).unwrap();
        assert!(t.check_access(&p2.access_token).is_ok());
        // Within the grace period a second refresh with the old token still works.
        assert!(t.refresh(&p1.refresh_token).is_ok());
        t.age_rotations(REFRESH_REUSE_GRACE + 1);
        // Reusing the rotated token revokes the family, sessions included.
        assert_eq!(t.refresh(&p1.refresh_token).unwrap_err(), TokenError::Reused);
        assert!(t.check_access(&p2.access_token).is_err());
        assert!(t.refresh(&p2.refresh_token).is_err());
        assert!(t.check_stream(&st).is_err());
    }

    #[test]
    fn sessions_are_bound_to_their_sign_in() {
        let t = Tokens::new(None);
        let a = t.check_access(&t.redeem_ticket(&t.issue_ticket("u", "e", "p")).unwrap().access_token).unwrap();
        let b = t.check_access(&t.redeem_ticket(&t.issue_ticket("u", "e", "p")).unwrap().access_token).unwrap();
        let (s, _) = t.create_session(&a, "abc");
        assert!(t.new_stream_token(&s.id, &b).is_none());
        assert!(!t.revoke_session(&s.id, &b));
        assert!(t.new_stream_token(&s.id, &a).is_some());
        assert!(t.revoke_session(&s.id, &a));
        assert!(t.session(&s.id).is_none());
    }

    #[test]
    fn tokens_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state.json");
        let t = Tokens::new(Some(f.clone()));
        let p = t.redeem_ticket(&t.issue_ticket("u", "e", "p")).unwrap();
        drop(t);
        let t = Tokens::new(Some(f));
        assert_eq!(t.check_access(&p.access_token).unwrap().user, "u");
        assert!(t.refresh(&p.refresh_token).is_ok());
        let raw = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
        assert!(!raw.contains(&p.access_token), "state file must not contain raw tokens");
    }

    #[test]
    fn sign_out_revokes() {
        let t = Tokens::new(None);
        let p = t.redeem_ticket(&t.issue_ticket("u", "e", "p")).unwrap();
        assert!(t.revoke_by_refresh(&p.refresh_token));
        assert!(t.check_access(&p.access_token).is_err());
    }
}
