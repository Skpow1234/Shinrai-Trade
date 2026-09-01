//! Fetches last-trade quotes from the market-data gateway.

use shinrai_instruments::{ExternalId, InstrumentId, InstrumentMaster, PriceTicks};

/// Fetches `GET /v1/quotes?symbol=` from the MD gateway.
///
/// Returns `None` when the base URL is unset, the symbol is unknown, or the quote is missing.
pub async fn fetch_quote(
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
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
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
