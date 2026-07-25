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

/// The proxy's `/market-all` route, one call for Ondo's whole supported-asset
/// list (upstream `GET /v1/assets/all/market`) — what `screen` uses instead
/// of fanning out one request per symbol, so coverage isn't capped at
/// whatever's in `DASHBOARD_TICKERS`.
fn get_proxy_all() -> Result<Value, String> {
    let url = format!("{PROXY_BASE}/market-all");
    let resp = agent().get(&url).call();
    read_response(resp)
}

/// The proxy's `/dividends/:symbol` route (upstream `GET
/// /v1/assets/{symbol}/dividends`) — cached for an hour on the proxy side
/// since this changes at most quarterly, nowhere near as often as a price.
fn get_proxy_dividends(symbol: &str) -> Result<Value, String> {
    let url = format!("{PROXY_BASE}/dividends/{symbol}");
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
    /// Today's volume on the real, underlying stock (shares), and Ondo's own
    /// trailing average for it — both straight off `underlyingMarket`, not
    /// derived. This is what answers "is this trading more than usual
    /// today", not `primaryMarket`'s on-chain-token numbers, which are a
    /// much thinner market than the stock itself.
    pub volume: Option<f64>,
    pub avg_volume: Option<f64>,
    /// The real stock's 52-week high/low, straight off `underlyingMarket` —
    /// what `range_position` uses to say how close today's price sits to
    /// either edge, without this module ever having to track a year of
    /// history itself.
    pub price_high_52w: Option<f64>,
    pub price_low_52w: Option<f64>,
}

impl Market {
    /// >1.0 means today is running hotter than usual, <1.0 quieter. `None`
    /// when either side is missing or the average is zero — a ratio against
    /// a zero baseline is not a real "how unusual is this", it's a division
    /// artifact.
    pub fn volume_ratio(&self) -> Option<f64> {
        match (self.volume, self.avg_volume) {
            (Some(v), Some(a)) if a > 0.0 => Some(v / a),
            _ => None,
        }
    }

    /// Where today's price sits in the 52-week range: 0.0 at the 52w low,
    /// 1.0 at the 52w high. A classic screener read (near-highs/near-lows),
    /// and a real one here — Ondo hands back both edges itself, this isn't
    /// reconstructed from a shorter window standing in for a year. `None`
    /// when either edge is missing or the range is degenerate (high <= low).
    pub fn range_position(&self) -> Option<f64> {
        match (self.price_high_52w, self.price_low_52w) {
            (Some(high), Some(low)) if high > low => {
                Some(((self.price_usd - low) / (high - low)).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// One symbol's dividend info — a separate endpoint from `market`, since
/// unlike price this changes at most quarterly. Ondo's tokens don't pay
/// this out in cash to holders: it's the underlying stock's real dividend,
/// which on-chain shows up as the token's `sharesMultiplier` ratcheting up
/// instead (see the module doc comment) — this data explains *why* the
/// price drifts, it isn't itself something a holder receives here.
#[derive(Debug)]
pub struct Dividend {
    pub ticker: String,
    /// Annualized yield as a fraction (0.0245 = 2.45%), not a percentage —
    /// multiply by 100 for display, same convention as everything else this
    /// module hands back as a plain decimal.
    pub yield_frac: Option<f64>,
    pub payout_frequency: Option<String>,
    pub last_cash_amount: Option<f64>,
    pub last_payment_date: Option<String>,
}

pub fn dividends(ticker: &str, api_key: &str) -> Result<Dividend, String> {
    let symbol = normalize_symbol(ticker);
    let v = if api_key.trim().is_empty() {
        get_proxy_dividends(&symbol)?
    } else {
        get(&format!("/assets/{symbol}/dividends"), api_key)?
    };
    parse_dividend(&v)
}

fn parse_dividend(v: &Value) -> Result<Dividend, String> {
    let ticker = v["ticker"].as_str().unwrap_or_default().to_uppercase();
    Ok(Dividend {
        ticker,
        yield_frac: num(&v["dividendYield"]),
        payout_frequency: v["payoutFrequency"].as_str().map(str::to_string),
        last_cash_amount: num(&v["lastCashAmount"]),
        last_payment_date: v["lastPaymentDate"].as_str().map(str::to_string),
    })
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

/// One request per symbol run across a scoped thread pool — used for the
/// small, fixed dashboard watchlist, where a per-symbol request each staying
/// cheap and independently edge-cached matters more than round-trip count.
/// `screen` below uses the real bulk endpoint instead; this stays for the
/// UI's own Ondo tab, which deliberately shows a short curated list rather
/// than the whole exchange. A single symbol's failure (a delisted/typo'd
/// ticker) is dropped rather than failing the whole dashboard — one bad row
/// shouldn't blank the other nine.
pub fn dashboard(tickers: &[&str]) -> Vec<Market> {
    std::thread::scope(|scope| {
        let handles: Vec<_> =
            tickers.iter().map(|t| scope.spawn(move || market(t, ""))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()?.ok()).collect()
    })
}

/// Every Ondo Stocks asset Ondo itself supports (`GET /v1/assets/all/market`,
/// one call, via the proxy's `/market-all` route) — real coverage of the
/// whole exchange rather than whatever's hardcoded in `DASHBOARD_TICKERS`.
/// Symbol comes off each element's own `primaryMarket.symbol`, not a ticker
/// this function already knew to ask for, since there's no per-symbol
/// request here to remember one.
pub fn all_markets() -> Result<Vec<Market>, String> {
    let v = get_proxy_all()?;
    let arr = v.as_array().ok_or("unexpected response shape from /market-all")?;
    Ok(arr
        .iter()
        .filter_map(|elem| {
            let symbol = elem["primaryMarket"]["symbol"]
                .as_str()
                .map(str::to_string)
                .or_else(|| elem["underlyingMarket"]["ticker"].as_str().map(|t| format!("{t}on")))?;
            parse_market(&symbol, elem).ok()
        })
        .collect())
}

/// Ranked by `volume_ratio`, highest (most unusual) first, rows with no
/// ratio pushed to the bottom rather than dropped, so "nothing looks unusual
/// today" is still a real, inspectable answer rather than an empty list that
/// reads like a failed call. Tries the real bulk endpoint first (the whole
/// exchange); if that fails — proxy not yet updated, a network hiccup — falls
/// back to fanning out over the small curated watchlist rather than handing
/// back nothing. Sorting happens here rather than being left to the caller
/// (or the model) for the same reason `market::fetch`'s Gainers/Losers sort
/// happens in Rust: a ranking is arithmetic, not something to hand an LLM a
/// pile of numbers and trust it to get right.
pub fn screen() -> Vec<Market> {
    let mut rows = all_markets().unwrap_or_default();
    if rows.is_empty() {
        rows = dashboard(DASHBOARD_TICKERS);
    }
    rows.sort_by(|a, b| {
        b.volume_ratio()
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&a.volume_ratio().unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
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

    // The stock's own volume, not the on-chain token's — `underlyingMarket`
    // only, deliberately no fallback to a flatter/older shape: unlike price,
    // getting this one wrong silently (e.g. reading a thin on-chain number
    // as if it were the real market's) would make `screen`'s ranking lie.
    let volume = num(&v["underlyingMarket"]["volume"]);
    let avg_volume = num(&v["underlyingMarket"]["averageVolume"]);
    let price_high_52w = num(&v["underlyingMarket"]["priceHigh52w"]);
    let price_low_52w = num(&v["underlyingMarket"]["priceLow52w"]);

    Ok(Market {
        symbol: symbol.to_uppercase(),
        name,
        price_usd,
        change_24h,
        spark,
        volume,
        avg_volume,
        price_high_52w,
        price_low_52w,
    })
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

    #[test]
    fn parse_dividend_reads_the_real_ondo_shape() {
        let v: Value = serde_json::from_str(
            r#"{
                "ticker": "AAPL",
                "dividendYield": "0.0245",
                "payoutFrequency": "quarterly",
                "lastCashAmount": "1.54",
                "lastPaymentDate": "2025-02-15",
                "timestamp": 1746655938000
            }"#,
        )
        .unwrap();
        let d = parse_dividend(&v).unwrap();
        assert_eq!(d.ticker, "AAPL");
        assert!((d.yield_frac.unwrap() - 0.0245).abs() < 1e-9);
        assert_eq!(d.payout_frequency.as_deref(), Some("quarterly"));
        assert!((d.last_cash_amount.unwrap() - 1.54).abs() < 1e-9);
        assert_eq!(d.last_payment_date.as_deref(), Some("2025-02-15"));
    }

    #[test]
    fn parse_dividend_tolerates_a_non_paying_stock() {
        let v: Value = serde_json::from_str(
            r#"{"ticker": "META", "payoutFrequency": "none"}"#,
        )
        .unwrap();
        let d = parse_dividend(&v).unwrap();
        assert_eq!(d.ticker, "META");
        assert!(d.yield_frac.is_none());
        assert_eq!(d.payout_frequency.as_deref(), Some("none"));
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
                "underlyingMarket": {
                    "ticker": "TSLA",
                    "name": "Tesla, Inc. Common Stock",
                    "volume": "62760007.674769",
                    "averageVolume": "69157673",
                    "priceHigh52w": "498.83",
                    "priceLow52w": "297.82"
                }
            }"#,
        )
        .unwrap();
        let m = parse_market("TSLAon", &v).unwrap();
        assert_eq!(m.symbol, "TSLAON");
        assert!((m.price_usd - 311.345).abs() < 1e-9);
        assert!((m.change_24h.unwrap() - (-3.940670236142447777)).abs() < 1e-6);
        assert_eq!(m.name.as_deref(), Some("Tesla, Inc. Common Stock"));
        assert_eq!(m.spark, vec![324.113139_f32, 311.340909_f32]);
        assert!((m.volume.unwrap() - 62_760_007.674769).abs() < 1e-3);
        assert!((m.avg_volume.unwrap() - 69_157_673.0).abs() < 1e-3);
        assert!((m.price_high_52w.unwrap() - 498.83).abs() < 1e-6);
        assert!((m.price_low_52w.unwrap() - 297.82).abs() < 1e-6);
        // (311.345 - 297.82) / (498.83 - 297.82) — real Tesla numbers, not a
        // round one, which is exactly why it's worth pinning down.
        assert!((m.range_position().unwrap() - 0.06728520969106032).abs() < 1e-6);
        assert!((m.volume_ratio().unwrap() - 0.9074916050858015).abs() < 1e-6);
    }

    #[test]
    fn volume_ratio_is_none_without_both_sides() {
        let v: Value = serde_json::from_str(r#"{"price": 100.0}"#).unwrap();
        let m = parse_market("XON", &v).unwrap();
        assert!(m.volume.is_none());
        assert!(m.volume_ratio().is_none());
    }

    fn mkt(symbol: &str, volume: Option<f64>, avg_volume: Option<f64>) -> Market {
        Market {
            symbol: symbol.into(),
            name: None,
            price_usd: 1.0,
            change_24h: None,
            spark: vec![],
            volume,
            avg_volume,
            price_high_52w: None,
            price_low_52w: None,
        }
    }

    #[test]
    fn screen_ranks_by_volume_ratio_highest_first() {
        let hot = mkt("HOT", Some(300.0), Some(100.0));
        let normal = mkt("NORM", Some(100.0), Some(100.0));
        let unknown = mkt("UNK", None, None);
        let mut rows = vec![normal, unknown, hot];
        rows.sort_by(|a, b| {
            b.volume_ratio()
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&a.volume_ratio().unwrap_or(f64::NEG_INFINITY))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let order: Vec<&str> = rows.iter().map(|m| m.symbol.as_str()).collect();
        assert_eq!(order, vec!["HOT", "NORM", "UNK"]);
    }

    #[test]
    fn range_position_reads_where_price_sits_in_the_52w_band() {
        let mut m = mkt("X", None, None);
        m.price_usd = 75.0;
        m.price_low_52w = Some(50.0);
        m.price_high_52w = Some(100.0);
        assert!((m.range_position().unwrap() - 0.5).abs() < 1e-9);

        m.price_usd = 100.0;
        assert!((m.range_position().unwrap() - 1.0).abs() < 1e-9);

        m.price_usd = 50.0;
        assert!((m.range_position().unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn range_position_clamps_a_price_that_has_moved_past_the_52w_edge() {
        // The 52w figures update slower than the live price — a fresh
        // breakout can sit above the recorded high for a moment.
        let mut m = mkt("X", None, None);
        m.price_usd = 120.0;
        m.price_low_52w = Some(50.0);
        m.price_high_52w = Some(100.0);
        assert_eq!(m.range_position(), Some(1.0));
    }

    #[test]
    fn range_position_is_none_without_both_edges() {
        let m = mkt("X", None, None);
        assert!(m.range_position().is_none());
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
