//! Shared MD gateway HTTP client (optional mTLS).

use std::fs;
use std::time::Duration;

use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks};

/// Shared reqwest client for MD quote fetches.
#[derive(Debug, Clone)]
pub struct MdHttpClient {
    inner: reqwest::Client,
}

impl MdHttpClient {
    /// Default TLS client (no client cert).
    #[must_use]
    pub fn plain() -> Self {
        Self {
            inner: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Builds a client with optional client identity + CA for mTLS to the MD gateway.
    ///
    /// Paths: PEM client cert+key file(s) and optional CA PEM.
    /// When all path args are `None`, behaves like [`Self::plain`].
    ///
    /// # Errors
    ///
    /// Returns a string description when files cannot be read or identity is invalid.
    pub fn from_mtls_env(
        client_pem: Option<&str>,
        key_pem: Option<&str>,
        ca_pem: Option<&str>,
    ) -> Result<Self, String> {
        if client_pem.is_none() && key_pem.is_none() && ca_pem.is_none() {
            return Ok(Self::plain());
        }
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(5));
        if let (Some(cert_path), Some(key_path)) = (client_pem, key_pem) {
            let mut identity_pem =
                fs::read(cert_path).map_err(|e| format!("md client cert: {e}"))?;
            let mut key = fs::read(key_path).map_err(|e| format!("md client key: {e}"))?;
            identity_pem.push(b'\n');
            identity_pem.append(&mut key);
            let identity = reqwest::Identity::from_pem(&identity_pem)
                .map_err(|e| format!("md client identity: {e}"))?;
            builder = builder.identity(identity);
        } else if client_pem.is_some() || key_pem.is_some() {
            return Err(
                "SHINRAI_OG_MD_CLIENT_CERT and SHINRAI_OG_MD_CLIENT_KEY must both be set".into(),
            );
        }
        if let Some(ca_path) = ca_pem {
            let ca = fs::read(ca_path).map_err(|e| format!("md ca: {e}"))?;
            let cert =
                reqwest::Certificate::from_pem(&ca).map_err(|e| format!("md ca pem: {e}"))?;
            builder = builder.add_root_certificate(cert);
        }
        let inner = builder
            .build()
            .map_err(|e| format!("md client build: {e}"))?;
        Ok(Self { inner })
    }

    /// Reads mTLS paths from env (`SHINRAI_OG_MD_CLIENT_CERT` / `_KEY` / `_CA`).
    #[must_use]
    pub fn from_env() -> Self {
        match Self::from_mtls_env(
            std::env::var("SHINRAI_OG_MD_CLIENT_CERT")
                .ok()
                .filter(|s| !s.is_empty())
                .as_deref(),
            std::env::var("SHINRAI_OG_MD_CLIENT_KEY")
                .ok()
                .filter(|s| !s.is_empty())
                .as_deref(),
            std::env::var("SHINRAI_OG_MD_CA")
                .ok()
                .filter(|s| !s.is_empty())
                .as_deref(),
        ) {
            Ok(c) => c,
            Err(err) => {
                eprintln!(
                    "shinrai-order-gateway: MD mTLS client config failed ({err}); using plain TLS"
                );
                Self::plain()
            }
        }
    }
}

/// Fetches `GET /v1/quotes?symbol=` from the MD gateway.
///
/// Returns `None` when the symbol is unknown or the quote is missing.
pub async fn fetch_quote(
    client: &MdHttpClient,
    base_url: &str,
    token: Option<&str>,
    master: &InstrumentMaster,
    symbol: &str,
) -> Option<PriceTicks> {
    let alias = ExternalId::ticker(symbol).ok()?;
    let _id = master.resolve_alias(&alias).ok()?;
    let url = format!(
        "{}/v1/quotes?symbol={}",
        base_url.trim_end_matches('/'),
        symbol
    );
    let mut req = client.inner.get(&url);
    if let Some(t) = token.filter(|s| !s.is_empty()) {
        req = req.query(&[("token", t)]);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let scaled = json.get("price_scaled")?.as_i64()?;
    if scaled <= 0 {
        return None;
    }
    Some(PriceTicks::from_scaled(scaled))
}

/// Resolves symbol to instrument id (for inserting fetched quotes).
#[must_use]
pub fn instrument_for_symbol(master: &InstrumentMaster, symbol: &str) -> Option<InstrumentId> {
    let alias = ExternalId::ticker(symbol).ok()?;
    master.resolve_alias(&alias).ok()
}
