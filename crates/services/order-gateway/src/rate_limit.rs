//! Simple in-memory per-subject rate limiter (token bucket style window).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fixed-window rate limiter keyed by subject string.
#[derive(Debug)]
pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    hits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    /// Creates a limiter allowing `max_per_window` events per `window`.
    #[must_use]
    pub fn new(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window: max_per_window.max(1),
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Permissive default for local demos (100 submits / second).
    #[must_use]
    pub fn demo() -> Self {
        Self::new(100, Duration::from_secs(1))
    }

    /// Returns true if the key is under the limit (and records the hit).
    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self
            .hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = map.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 1);
            return true;
        }
        if entry.1 >= self.max_per_window {
            return false;
        }
        entry.1 = entry.1.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_burst() {
        let lim = RateLimiter::new(2, Duration::from_secs(60));
        assert!(lim.allow("a"));
        assert!(lim.allow("a"));
        assert!(!lim.allow("a"));
        assert!(lim.allow("b"));
    }
}
