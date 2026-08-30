//! Fanout errors. Messages must not include tokens or secrets.

use core::fmt;

/// Client fanout errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutError {
    /// Connect lacked a token.
    MissingToken,
    /// Token is unknown or malformed.
    InvalidToken,
    /// Token was revoked.
    RevokedToken,
    /// Session id is not connected.
    UnknownSession,
    /// Subscription cap reached.
    TooManySubscriptions,
    /// Symbol did not resolve in the instrument master.
    UnknownInstrument,
    /// Client frame was not a valid command.
    InvalidCommand,
}

impl fmt::Display for FanoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken => f.write_str("missing access token"),
            Self::InvalidToken => f.write_str("invalid access token"),
            Self::RevokedToken => f.write_str("access token revoked"),
            Self::UnknownSession => f.write_str("unknown session"),
            Self::TooManySubscriptions => f.write_str("subscription limit reached"),
            Self::UnknownInstrument => f.write_str("unknown instrument"),
            Self::InvalidCommand => f.write_str("invalid client command"),
        }
    }
}

impl std::error::Error for FanoutError {}

impl FanoutError {
    /// Stable client `code` field (never a secret).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingToken => "missing_token",
            Self::InvalidToken => "invalid_token",
            Self::RevokedToken => "revoked",
            Self::UnknownSession => "unknown_session",
            Self::TooManySubscriptions => "too_many_subscriptions",
            Self::UnknownInstrument => "unknown_instrument",
            Self::InvalidCommand => "invalid_command",
        }
    }
}
