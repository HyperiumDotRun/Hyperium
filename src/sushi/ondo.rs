//! Read-only client for Ondo Finance's tokenized-stocks API ("GM" API,
//! `api.gm.ondo.finance`).
//!
//! Ondo Stocks (`TSLAon`, `AAPLon`, ...) are a *separate* product from
//! anything else in `sushi/`: they live on Ethereum / BNB Chain / Solana, not
//! Robinhood Chain, and there is no DEX pool for them inside this app. This
//! module only reads reference price/market data for the chat agent to talk
//! about — there is no mint/redeem here yet. Real trading needs an approved,
//! allowlisted API key from Ondo's institutional onboarding
//! (`onboarding@ondo.finance`), which this project doesn't have; see
//! `CLAUDE.md` for the full picture.
//!
//! Two ways in: bring your own `x-api-key` (`get`, straight to Ondo), or —
//! with no key configured — go through Hyperium's own read-only proxy
//! (`get_proxy`), a Cloudflare Worker holding one shared key so a user never
//! needs their own just to look at a price. `market` picks between them by
//! whether `api_key` is blank; the AI Agent's Ondo dashboard always calls it
//! with `""` since the whole point of the dashboard is not asking every user
//! for a key.

use std::time::Duration;

use serde_json::Value;

const BASE: &str = "https://api.gm.ondo.finance/v1";
const PROXY_BASE: &str = "https://curly-bird-c943.plain-pond-ab91.workers.dev";

/// The dashboard's fixed watchlist — the same off-hours-tradable large-cap
/// set `CLAUDE.md` documents (AAPL, TSLA, NVDA, ...), the ones actually worth
/// showing a glance at. Not user-editable yet: Ondo has no bulk-quote
/// endpoint, so this list is exactly how many requests the dashboard fires.
pub const DASHBOARD_TICKERS: &[&str] = &[
    "AAPL", "TSLA", "NVDA", "MSFT", "GOOGL", "META", "AMZN", "NFLX", "SPY", "QQQ",
];

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
}

fn get(path: &str, api_key: &str) -> Result<Value, String> {
    if api_key.trim().is_empty() {
        return Err("no Ondo API key configured".into());
    }
    let url = format!("{BASE}{path}");
    let resp = agent().get(&url).set("x-api-key", api_key.trim()).call();
    read_response(resp)
}

/// Same request, routed through Hyperium's shared proxy instead — no key, no
/// `x-api-key` header, just the symbol in the path. The proxy caches each
/// symbol at the edge for 30s, so many users asking about the same ticker at
/// once collapse into one call against Ondo's own limits.
fn get_proxy(symbol: &str) -> Result<Value, String> {
    let url = format!("{PROXY_BASE}/market/{symbol}");
    let resp = agent().get(&url).call();
    read_response(resp)
}

fn read_response(resp: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
    match resp {
        Ok(r) => r.into_json().map_err(|e| format!("bad response: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let v: Value = resp.into_json().unwrap_or(Value::Null);
            let msg = v["message"].as_str().or_else(|| v["error"].as_str()).unwrap_or_default();
            if msg.is_empty() {
                Err(format!("Ondo HTTP {code}"))
            } else {
                Err(format!("{code} {msg}"))
            }
        }
        Err(e) => Err(format!("network error: {e}")),
    }
}

/// Reference market data for one Ondo Stocks symbol. The on-chain "shares
/// multiplier" means this price tends to drift above the underlying stock's
/// real price over time (dividends compound in rather than get paid out) —
/// this is reference data for the agent to talk about, not a live quote for
/// a swap that can happen inside this app.
#[derive(Debug)]
pub struct Market {
    pub symbol: String,
    pub name: Option<String>,
    pub price_usd: f64,
    pub change_24h: Option<f64>,
    /// Same-day closes from Ondo's own `priceHistory24h`, oldest first — the
    /// dashboard's sparkline, sourced live rather than accumulated locally
    /// (accumulating would mean every symbol starts as a flat line on launch
    /// and only grows a shape after sitting open for a while).
    pub spark: Vec<f32>,
}

/// Normalizes a bare ticker (`TSLA`) to Ondo's symbol convention — minimum 3
/// characters, must end in a lowercase `on` — but leaves an already-correct
/// symbol (`TSLAon`) untouched.
fn normalize_symbol(ticker: &str) -> String {
    let t = ticker.trim().trim_start_matches('$');
    if t.len() >= 3 && t.to_ascii_lowercase().ends_with("on") {
        t.to_string()
    } else {
        format!("{t}on")
    }
}

pub fn market(ticker: &str, api_key: &str) -> Result<Market, String> {
    let symbol = normalize_symbol(ticker);
    let v = if api_key.trim().is_empty() {
        get_proxy(&symbol)?
    } else {
        get(&format!("/assets/{symbol}/market"), api_key)?
    };
    parse_market(&symbol, &v)
}

/// One request per symbol run across a scoped thread pool — Ondo has no bulk
/// endpoint, and the proxy's per-symbol edge cache means fanning these out is
/// cheap rather than something to be saved up and batched. A single symbol's
/// failure (a delisted/typo'd ticker) is dropped rather than failing the
/// whole dashboard — one bad row shouldn't blank the other nine.
pub fn dashboard(tickers: &[&str]) -> Vec<Market> {
    std::thread::scope(|scope| {
        let handles: Vec<_> =
            tickers.iter().map(|t| scope.spawn(move || market(t, ""))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()?.ok()).collect()
    })
}

/// A number that may have arrived as a JSON number or — as Ondo's real
/// responses do throughout — as a numeric string.
fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Ondo's actual shape nests everything under `primaryMarket` (the on-chain
/// token's own data) and `underlyingMarket` (the real stock it tracks), with
/// numbers as strings throughout (`"311.345"`, not `311.345`). The flatter
/// shapes below are kept as a fallback rather than deleted outright — cheap
/// insurance if a different endpoint or a future API version ever hands this
/// function something shaped like the original guess.
fn parse_market(symbol: &str, v: &Value) -> Result<Market, String> {
    let pm = &v["primaryMarket"];

    let price_usd = num(&pm["price"])
        .or_else(|| num(&v["priceUsd"]))
        .or_else(|| num(&v["price"]))
        .or_else(|| num(&v["price"]["usd"]))
        .ok_or_else(|| format!("no price available for {symbol} — check the symbol is right"))?;

    let change_24h = num(&pm["priceChangePct24h"])
        .or_else(|| num(&v["change24h"]))
        .or_else(|| num(&v["priceChange24h"]))
        .or_else(|| num(&v["change"]["h24"]));

    let name = v["underlyingMarket"]["name"]
        .as_str()
        .or_else(|| v["name"].as_str())
        .or_else(|| v["assetName"].as_str())
        .map(str::to_string);

    let spark = pm["priceHistory24h"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| num(&p["price"])).map(|p| p as f32).collect())
        .unwrap_or_default();

    Ok(Market { symbol: symbol.to_uppercase(), name, price_usd, change_24h, spark })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_a_bare_ticker() {
        assert_eq!(normalize_symbol("TSLA"), "TSLAon");
        assert_eq!(normalize_symbol("$TSLA"), "TSLAon");
        assert_eq!(normalize_symbol(" tsla "), "tslaon");
    }

    #[test]
    fn leaves_an_already_correct_symbol_alone() {
        assert_eq!(normalize_symbol("TSLAon"), "TSLAon");
        assert_eq!(normalize_symbol("AAPLon"), "AAPLon");
    }

    /// `get` (the bring-your-own-key path) must still short-circuit before
    /// ever building a request — `market`/`get_proxy` are what an empty key
    /// now falls through to instead.
    #[test]
    fn get_fails_cleanly_with_no_api_key() {
        let err = get("/assets/TSLAon/market", "").unwrap_err();
        assert_eq!(err, "no Ondo API key configured");
    }

    #[test]
    fn get_fails_cleanly_with_a_blank_api_key() {
        let err = get("/assets/TSLAon/market", "   ").unwrap_err();
        assert_eq!(err, "no Ondo API key configured");
    }

    #[test]
    fn parse_market_reads_the_real_ondo_shape() {
        let v: Value = serde_json::from_str(
            r#"{
                "primaryMarket": {
                    "symbol": "TSLAon",
                    "price": "311.345",
                    "priceChangePct24h": "-3.940670236142447777",
                    "priceHistory24h": [
                        {"timestamp": 1, "price": "324.113139"},
                        {"timestamp": 2, "price": "311.340909"}
                    ]
                },
                "underlyingMarket": {"ticker": "TSLA", "name": "Tesla, Inc. Common Stock"}
            }"#,
        )
        .unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.symbol, "TSLAON");
        assert!((m.price_usd - 311.345).abs() < 1e-9);
        assert!((m.change_24h.unwrap() - (-3.940670236142447777)).abs() < 1e-6);
        assert_eq!(m.name.as_deref(), Some("Tesla, Inc. Common Stock"));
        assert_eq!(m.spark, vec![324.113139_f32, 311.340909_f32]);
    }

    #[test]
    fn parse_market_reads_flat_price_usd() {
        let v: Value =
            serde_json::from_str(r#"{"priceUsd": 271.5, "change24h": 1.2, "name": "Tesla"}"#)
                .unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.symbol, "TSLAON");
        assert_eq!(m.price_usd, 271.5);
        assert_eq!(m.change_24h, Some(1.2));
        assert_eq!(m.name.as_deref(), Some("Tesla"));
    }

    #[test]
    fn parse_market_reads_bare_price_number() {
        let v: Value = serde_json::from_str(r#"{"price": 271.5}"#).unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.price_usd, 271.5);
        assert_eq!(m.change_24h, None);
        assert_eq!(m.name, None);
    }

    #[test]
    fn parse_market_reads_nested_price_usd() {
        let v: Value = serde_json::from_str(r#"{"price": {"usd": 271.5}}"#).unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.price_usd, 271.5);
    }

    #[test]
    fn parse_market_reads_price_change_24h_alias() {
        let v: Value = serde_json::from_str(r#"{"priceUsd": 100.0, "priceChange24h": -2.5}"#)
            .unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.change_24h, Some(-2.5));
    }

    #[test]
    fn parse_market_reads_nested_change_h24() {
        let v: Value =
            serde_json::from_str(r#"{"priceUsd": 100.0, "change": {"h24": 3.3}}"#).unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.change_24h, Some(3.3));
    }

    #[test]
    fn parse_market_fails_cleanly_when_no_price_field_matches() {
        let v: Value = serde_json::from_str(r#"{"symbol": "TSLAon"}"#).unwrap();
        let err = parse_market("TSLAon", &v).unwrap_err();
        assert!(err.contains("no price available"), "{err}");
    }
}
