//! KYC status gate and compliance helpers for the order gateway.

use std::collections::HashMap;

/// KYC / trading eligibility for a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KycStatus {
    /// May trade.
    Approved,
    /// Onboarding incomplete.
    Pending,
    /// Explicitly blocked.
    Rejected,
}

impl KycStatus {
    /// Parses `approved` / `pending` / `rejected` (case-insensitive).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "approved" | "ok" | "pass" => Some(Self::Approved),
            "pending" | "review" => Some(Self::Pending),
            "rejected" | "denied" | "fail" => Some(Self::Rejected),
            _ => None,
        }
    }

    /// Stable API code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
        }
    }

    /// Whether trading is allowed.
    #[must_use]
    pub const fn allows_trading(self) -> bool {
        matches!(self, Self::Approved)
    }
}

/// In-memory KYC map (subject → status). Empty map = all approved (dev default).
#[derive(Debug, Clone, Default)]
pub struct KycRegistry {
    by_subject: HashMap<String, KycStatus>,
}

impl KycRegistry {
    /// Empty registry (fail-open: unknown subjects are approved).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `subject:status,...` from `SHINRAI_OG_KYC_STATUS`.
    #[must_use]
    pub fn from_env_str(raw: Option<&str>) -> Self {
        let mut reg = Self::new();
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            return reg;
        };
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((subj, status)) = part.split_once(':') else {
                continue;
            };
            if let Some(st) = KycStatus::parse(status) {
                reg.by_subject.insert(subj.trim().to_owned(), st);
            }
        }
        reg
    }

    /// Looks up status; unknown subjects are approved when the map is empty,
    /// otherwise pending (fail closed once any KYC rows are configured).
    #[must_use]
    pub fn status(&self, subject: &str) -> KycStatus {
        if let Some(st) = self.by_subject.get(subject) {
            return *st;
        }
        if self.by_subject.is_empty() {
            KycStatus::Approved
        } else {
            KycStatus::Pending
        }
    }

    /// Sets status (tests / ops).
    #[allow(dead_code)]
    pub fn set(&mut self, subject: impl Into<String>, status: KycStatus) {
        self.by_subject.insert(subject.into(), status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_approves() {
        let reg = KycRegistry::new();
        assert_eq!(reg.status("alice"), KycStatus::Approved);
    }

    #[test]
    fn configured_unknown_is_pending() {
        let reg = KycRegistry::from_env_str(Some("alice:approved"));
        assert_eq!(reg.status("alice"), KycStatus::Approved);
        assert_eq!(reg.status("bob"), KycStatus::Pending);
    }
}
