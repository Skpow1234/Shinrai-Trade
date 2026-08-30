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

/// Validates access tokens on WebSocket connect.
pub trait Authenticator {
    /// Authenticates a bearer token.
    ///
    /// # Errors
    ///
    /// Returns missing / invalid / revoked token errors. Do not include the
    /// token in the error display.
    fn authenticate(&self, token: Option<&str>) -> Result<SessionClaims, FanoutError>;

    /// Returns true if this token is currently revoked.
    fn is_revoked(&self, token: &str) -> bool {
        let _ = token;
        false
    }
}

struct StaticInner {
    by_token: HashMap<String, SubjectId>,
    revoked: HashSet<String>,
}

/// In-memory token table for Phase 2 (refresh rotation is 2.6).
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
    fn authenticate(&self, token: Option<&str>) -> Result<SessionClaims, FanoutError> {
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

    fn is_revoked(&self, token: &str) -> bool {
        self.inner
            .lock()
            .map(|g| g.revoked.contains(token) || !g.by_token.contains_key(token))
            .unwrap_or(true)
    }
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
            auth.authenticate(None).expect_err("missing"),
            FanoutError::MissingToken
        );
        assert_eq!(
            auth.authenticate(Some("nope")).expect_err("bad"),
            FanoutError::InvalidToken
        );
    }
}
