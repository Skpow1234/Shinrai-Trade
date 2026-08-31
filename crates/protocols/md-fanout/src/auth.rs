//! Connect-time authentication. Tokens are never displayed.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::error::FanoutError;

/// Authenticated principal (not a secret).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubjectId(String);

impl SubjectId {
    /// Creates a subject id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Claims after a successful authenticate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionClaims {
    subject: SubjectId,
}

impl SessionClaims {
    /// Builds claims for a subject.
    #[must_use]
    pub fn new(subject: SubjectId) -> Self {
        Self { subject }
    }

    /// Subject.
    #[must_use]
    pub fn subject(&self) -> &SubjectId {
        &self.subject
    }
}

/// Validates access tokens on WebSocket / REST.
pub trait Authenticator {
    /// Authenticates a bearer access token at logical time `now`.
    ///
    /// # Errors
    ///
    /// Returns missing / invalid / revoked / expired token errors. Do not
    /// include the token in the error display.
    fn authenticate(
        &self,
        token: Option<&str>,
        now_logical: u64,
    ) -> Result<SessionClaims, FanoutError>;

    /// Returns true if this access token is revoked or expired at `now`.
    fn is_revoked(&self, token: &str, now_logical: u64) -> bool {
        let _ = (token, now_logical);
        false
    }
}

struct StaticInner {
    by_token: HashMap<String, SubjectId>,
    revoked: HashSet<String>,
}

/// In-memory static access-token table (no expiry; useful for hub unit tests).
#[derive(Clone)]
pub struct StaticTokenAuth {
    inner: Arc<Mutex<StaticInner>>,
}

impl fmt::Debug for StaticTokenAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.inner.lock().map(|g| g.by_token.len()).unwrap_or(0);
        f.debug_struct("StaticTokenAuth")
            .field("entries", &n)
            .finish_non_exhaustive()
    }
}

impl StaticTokenAuth {
    /// Empty table (fail closed: every token is rejected).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StaticInner {
                by_token: HashMap::new(),
                revoked: HashSet::new(),
            })),
        }
    }

    /// Registers a token for a subject. Existing token is replaced.
    pub fn grant(&self, token: impl Into<String>, subject: SubjectId) {
        if let Ok(mut inner) = self.inner.lock() {
            let token = token.into();
            inner.revoked.remove(&token);
            inner.by_token.insert(token, subject);
        }
    }

    /// Revokes a token. Subsequent authenticate and hub sweeps fail the session.
    pub fn revoke(&self, token: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.revoked.insert(token.to_owned());
            inner.by_token.remove(token);
        }
    }
}

impl Default for StaticTokenAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl Authenticator for StaticTokenAuth {
    fn authenticate(
        &self,
        token: Option<&str>,
        _now_logical: u64,
    ) -> Result<SessionClaims, FanoutError> {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return Err(FanoutError::MissingToken);
        };
        let inner = self.inner.lock().map_err(|_| FanoutError::InvalidToken)?;
        if inner.revoked.contains(token) {
            return Err(FanoutError::RevokedToken);
        }
        let subject = inner
            .by_token
            .get(token)
            .cloned()
            .ok_or(FanoutError::InvalidToken)?;
        Ok(SessionClaims::new(subject))
    }

    fn is_revoked(&self, token: &str, _now_logical: u64) -> bool {
        self.inner
            .lock()
            .map(|g| g.revoked.contains(token) || !g.by_token.contains_key(token))
            .unwrap_or(true)
    }
}

/// Access / refresh TTLs in logical clock units (unix seconds at the gateway).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenTtl {
    access_logical: u64,
    refresh_logical: u64,
}

impl TokenTtl {
    /// Builds TTLs (minimum 1).
    #[must_use]
    pub fn new(access_logical: u64, refresh_logical: u64) -> Self {
        Self {
            access_logical: access_logical.max(1),
            refresh_logical: refresh_logical.max(1),
        }
    }

    /// Access token lifetime.
    #[must_use]
    pub const fn access_logical(self) -> u64 {
        self.access_logical
    }

    /// Refresh token lifetime.
    #[must_use]
    pub const fn refresh_logical(self) -> u64 {
        self.refresh_logical
    }
}

impl Default for TokenTtl {
    fn default() -> Self {
        // Access ≤ 60s meets the Phase 2.6 revoke SLA when sessions re-check on clock.
        Self::new(60, 3_600)
    }
}

/// Opaque token pair returned by issue / refresh.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedTokens {
    access_token: String,
    refresh_token: String,
    access_expires_at: u64,
    refresh_expires_at: u64,
}

impl IssuedTokens {
    /// Access token (Bearer).
    #[must_use]
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Refresh token (single-use; rotate on each refresh).
    #[must_use]
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Access expiry (logical clock).
    #[must_use]
    pub const fn access_expires_at(&self) -> u64 {
        self.access_expires_at
    }

    /// Refresh expiry (logical clock).
    #[must_use]
    pub const fn refresh_expires_at(&self) -> u64 {
        self.refresh_expires_at
    }
}

impl fmt::Debug for IssuedTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedTokens")
            .field("access_expires_at", &self.access_expires_at)
            .field("refresh_expires_at", &self.refresh_expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct AccessRecord {
    subject: SubjectId,
    family: u64,
    expires_at: u64,
}

#[derive(Clone)]
struct RefreshRecord {
    subject: SubjectId,
    family: u64,
    expires_at: u64,
    /// Set after a successful rotate; reuse revokes the family.
    consumed: bool,
}

struct ClientRecord {
    secret: String,
    subject: SubjectId,
}

struct TokenInner {
    clients: HashMap<String, ClientRecord>,
    access: HashMap<String, AccessRecord>,
    refresh: HashMap<String, RefreshRecord>,
    revoked_access: HashSet<String>,
    revoked_families: HashSet<u64>,
    next_id: u64,
    next_family: u64,
    pepper: u64,
}

/// Short-lived access tokens + rotating refresh tokens.
///
/// Client credentials mint the first pair. Refresh tokens are single-use;
/// presenting a consumed refresh revokes the whole token family.
#[derive(Clone)]
pub struct TokenAuth {
    inner: Arc<Mutex<TokenInner>>,
    ttl: TokenTtl,
}

impl fmt::Debug for TokenAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (clients, access) = self
            .inner
            .lock()
            .map(|g| (g.clients.len(), g.access.len()))
            .unwrap_or((0, 0));
        f.debug_struct("TokenAuth")
            .field("ttl", &self.ttl)
            .field("clients", &clients)
            .field("access_entries", &access)
            .finish_non_exhaustive()
    }
}

impl TokenAuth {
    /// Empty issuer (fail closed until clients or static grants are added).
    #[must_use]
    pub fn new(ttl: TokenTtl) -> Self {
        Self {
            inner: Arc::new(Mutex::new(TokenInner {
                clients: HashMap::new(),
                access: HashMap::new(),
                refresh: HashMap::new(),
                revoked_access: HashSet::new(),
                revoked_families: HashSet::new(),
                next_id: 1,
                next_family: 1,
                pepper: 0xA5A5_5A5A_C3C3_3C3C,
            })),
            ttl,
        }
    }

    /// TTL configuration.
    #[must_use]
    pub const fn ttl(&self) -> TokenTtl {
        self.ttl
    }

    /// Registers an OAuth-style client (`client_id` + secret → subject).
    pub fn register_client(
        &self,
        client_id: impl Into<String>,
        secret: impl Into<String>,
        subject: SubjectId,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.clients.insert(
                client_id.into(),
                ClientRecord {
                    secret: secret.into(),
                    subject,
                },
            );
        }
    }

    /// Grants a non-expiring access token (demo / test bootstrap).
    pub fn grant_static_access(&self, token: impl Into<String>, subject: SubjectId) {
        if let Ok(mut inner) = self.inner.lock() {
            let token = token.into();
            inner.revoked_access.remove(&token);
            inner.access.insert(
                token,
                AccessRecord {
                    subject,
                    family: 0,
                    expires_at: u64::MAX,
                },
            );
        }
    }

    /// Issues access + refresh from client credentials.
    ///
    /// # Errors
    ///
    /// Invalid client id/secret.
    pub fn issue_client_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
        now_logical: u64,
    ) -> Result<IssuedTokens, FanoutError> {
        let mut inner = self.inner.lock().map_err(|_| FanoutError::InvalidToken)?;
        let client = inner
            .clients
            .get(client_id)
            .ok_or(FanoutError::InvalidCredentials)?;
        if client.secret != client_secret {
            return Err(FanoutError::InvalidCredentials);
        }
        let subject = client.subject.clone();
        let family = inner.next_family;
        inner.next_family = inner.next_family.saturating_add(1);
        Ok(mint_pair(
            &mut inner,
            subject,
            family,
            now_logical,
            self.ttl,
        ))
    }

    /// Rotates a refresh token; old refresh becomes invalid.
    ///
    /// # Errors
    ///
    /// Unknown / expired / reused refresh.
    pub fn refresh(
        &self,
        refresh_token: &str,
        now_logical: u64,
    ) -> Result<IssuedTokens, FanoutError> {
        let mut inner = self.inner.lock().map_err(|_| FanoutError::InvalidToken)?;
        let Some(record) = inner.refresh.get(refresh_token).cloned() else {
            return Err(FanoutError::InvalidToken);
        };
        if inner.revoked_families.contains(&record.family) {
            return Err(FanoutError::RevokedToken);
        }
        if record.expires_at < now_logical {
            inner.refresh.remove(refresh_token);
            return Err(FanoutError::ExpiredToken);
        }
        if record.consumed {
            revoke_family(&mut inner, record.family);
            return Err(FanoutError::RevokedToken);
        }
        if let Some(r) = inner.refresh.get_mut(refresh_token) {
            r.consumed = true;
        }
        Ok(mint_pair(
            &mut inner,
            record.subject,
            record.family,
            now_logical,
            self.ttl,
        ))
    }

    /// Revokes an access token and/or refresh token (and its family).
    pub fn revoke(&self, token: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(access) = inner.access.remove(token) {
                inner.revoked_access.insert(token.to_owned());
                if access.family != 0 {
                    revoke_family(&mut inner, access.family);
                }
                return;
            }
            if let Some(refresh) = inner.refresh.remove(token) {
                revoke_family(&mut inner, refresh.family);
            }
        }
    }
}

impl Default for TokenAuth {
    fn default() -> Self {
        Self::new(TokenTtl::default())
    }
}

impl Authenticator for TokenAuth {
    fn authenticate(
        &self,
        token: Option<&str>,
        now_logical: u64,
    ) -> Result<SessionClaims, FanoutError> {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return Err(FanoutError::MissingToken);
        };
        let inner = self.inner.lock().map_err(|_| FanoutError::InvalidToken)?;
        if inner.revoked_access.contains(token) {
            return Err(FanoutError::RevokedToken);
        }
        let Some(record) = inner.access.get(token) else {
            return Err(FanoutError::InvalidToken);
        };
        if record.family != 0 && inner.revoked_families.contains(&record.family) {
            return Err(FanoutError::RevokedToken);
        }
        if record.expires_at < now_logical {
            return Err(FanoutError::ExpiredToken);
        }
        Ok(SessionClaims::new(record.subject.clone()))
    }

    fn is_revoked(&self, token: &str, now_logical: u64) -> bool {
        self.authenticate(Some(token), now_logical).is_err()
    }
}

fn mint_pair(
    inner: &mut TokenInner,
    subject: SubjectId,
    family: u64,
    now: u64,
    ttl: TokenTtl,
) -> IssuedTokens {
    let access_id = inner.next_id;
    inner.next_id = inner.next_id.saturating_add(1);
    let refresh_id = inner.next_id;
    inner.next_id = inner.next_id.saturating_add(1);

    let access_token = opaque("at", access_id, now, inner.pepper);
    let refresh_token = opaque("rt", refresh_id, now, inner.pepper);
    let access_expires_at = now.saturating_add(ttl.access_logical);
    let refresh_expires_at = now.saturating_add(ttl.refresh_logical);

    inner.access.insert(
        access_token.clone(),
        AccessRecord {
            subject: subject.clone(),
            family,
            expires_at: access_expires_at,
        },
    );
    inner.refresh.insert(
        refresh_token.clone(),
        RefreshRecord {
            subject,
            family,
            expires_at: refresh_expires_at,
            consumed: false,
        },
    );

    IssuedTokens {
        access_token,
        refresh_token,
        access_expires_at,
        refresh_expires_at,
    }
}

fn revoke_family(inner: &mut TokenInner, family: u64) {
    if family == 0 {
        return;
    }
    inner.revoked_families.insert(family);
    let doomed: Vec<String> = inner
        .access
        .iter()
        .filter(|(_, rec)| rec.family == family)
        .map(|(tok, _)| tok.clone())
        .collect();
    for tok in doomed {
        inner.access.remove(&tok);
        inner.revoked_access.insert(tok);
    }
    inner.refresh.retain(|_, rec| rec.family != family);
}

fn opaque(kind: &str, id: u64, now: u64, pepper: u64) -> String {
    let mut h = pepper ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= now.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = h.rotate_left(17).wrapping_mul(0x1656_67B1);
    format!("{kind}_{id:x}_{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_contain_token() {
        let auth = StaticTokenAuth::new();
        auth.grant("super-secret-token", SubjectId::new("u1"));
        let rendered = format!("{auth:?}");
        assert!(!rendered.contains("super-secret-token"));
    }

    #[test]
    fn missing_and_invalid() {
        let auth = StaticTokenAuth::new();
        assert_eq!(
            auth.authenticate(None, 0).expect_err("missing"),
            FanoutError::MissingToken
        );
        assert_eq!(
            auth.authenticate(Some("nope"), 0).expect_err("bad"),
            FanoutError::InvalidToken
        );
    }

    #[test]
    fn client_credentials_issue_and_authenticate() {
        let auth = TokenAuth::new(TokenTtl::new(60, 3_600));
        auth.register_client("cli", "sec", SubjectId::new("alice"));
        let pair = auth
            .issue_client_credentials("cli", "sec", 100)
            .expect("issue");
        assert!(auth.authenticate(Some(pair.access_token()), 100).is_ok());
        assert!(auth.authenticate(Some(pair.access_token()), 160).is_ok());
        assert_eq!(
            auth.authenticate(Some(pair.access_token()), 161)
                .expect_err("expired"),
            FanoutError::ExpiredToken
        );
        assert!(!format!("{pair:?}").contains(pair.access_token()));
    }

    #[test]
    fn refresh_rotates_and_reuse_revokes_family() {
        let auth = TokenAuth::new(TokenTtl::new(60, 3_600));
        auth.register_client("cli", "sec", SubjectId::new("alice"));
        let first = auth
            .issue_client_credentials("cli", "sec", 0)
            .expect("issue");
        let old_access = first.access_token().to_owned();
        let old_refresh = first.refresh_token().to_owned();

        let second = auth.refresh(&old_refresh, 10).expect("refresh");
        assert_ne!(second.access_token(), old_access);
        assert_ne!(second.refresh_token(), old_refresh);
        assert!(auth.authenticate(Some(&old_access), 10).is_ok()); // old access still valid until TTL
        assert!(auth.authenticate(Some(second.access_token()), 10).is_ok());

        assert_eq!(
            auth.refresh(&old_refresh, 11).expect_err("reuse"),
            FanoutError::RevokedToken
        );
        assert!(auth.is_revoked(second.access_token(), 11));
        assert!(auth.is_revoked(&old_access, 11));
    }

    #[test]
    fn revoke_access_kills_session_token() {
        let auth = TokenAuth::new(TokenTtl::default());
        auth.register_client("cli", "sec", SubjectId::new("alice"));
        let pair = auth
            .issue_client_credentials("cli", "sec", 0)
            .expect("issue");
        auth.revoke(pair.access_token());
        assert_eq!(
            auth.authenticate(Some(pair.access_token()), 0)
                .expect_err("revoked"),
            FanoutError::RevokedToken
        );
    }

    #[test]
    fn bad_client_secret() {
        let auth = TokenAuth::default();
        auth.register_client("cli", "sec", SubjectId::new("alice"));
        assert_eq!(
            auth.issue_client_credentials("cli", "wrong", 0)
                .expect_err("bad"),
            FanoutError::InvalidCredentials
        );
    }
}
