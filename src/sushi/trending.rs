//! What is actually trading on Robinhood Chain.
//!
//! Tokens launched on-chain, ranked by the volume they are doing right now.
//! Source is Dexscreener, which indexes the pools directly, so a token appears
//! here minutes after its pool exists — no listing, no curation, no waiting.
//!
//! This is the opposite trade-off from `tokens.rs`. That registry is curated
//! precisely so the model can never invent an address; here the addresses come
//! from the indexer itself, which is a source of truth rather than a guess, so
//! a token nobody has ever heard of is still safe to display.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Dexscreener names chains by slug, not by id — 4663 means nothing to it.
const CHAIN: &str = "robinhood";

/// The same chain as `CHAIN`, spelled the way `tokens::CHAINS` spells it.
pub const CHAIN_NAME: &str = "Robinhood";

/// Pairs are enumerated through their quote side: everything on this chain is
/// priced in one of these two, so the union is effectively the whole board.
/// Symbols rather than literals, resolved against the registry, so the
/// addresses stay defined in exactly one place.
const QUOTES: &[&str] = &["WETH", "USDG"];

/// Dexscreener answers 403 to a request that arrives without a plausible
/// User-Agent. Not documented anywhere; found by watching curl succeed where a
/// bare client failed.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Hyperium";

pub struct Row {
    pub symbol: String,
    pub address: String,
    pub price_usd: f64,
    /// Percent, already signed. Absent when the pair is too young to have one.
    pub change_h24: Option<f64>,
    pub volume_h24: f64,
    pub liquidity_usd: f64,
    /// Pool creation, epoch ms. Age is what tells a fresh launch from a
    /// survivor, and it is the column a curated list can never have.
    pub created_at_ms: Option<u64>,
}

/// Everything the indexer knows about one token, for the lookup card.
///
/// Wider than `Row` on purpose: the board answers "what is busy", this answers
/// "what is going on with this one", and that question needs the shorter time
/// windows and the buy/sell split to be answerable at all.
pub struct Info {
    pub symbol: String,
    pub name: String,
    pub address: String,
    pub price_usd: f64,
    pub change_m5: Option<f64>,
    pub change_h1: Option<f64>,
    pub change_h6: Option<f64>,
    pub change_h24: Option<f64>,
    pub volume_h24: f64,
    pub volume_h1: f64,
    pub liquidity_usd: f64,
    pub market_cap: Option<f64>,
    pub buys_h24: u64,
    pub sells_h24: u64,
    pub created_at_ms: Option<u64>,
    pub url: String,
}

impl Info {
    pub fn age_hours(&self) -> Option<f64> {
        let created = self.created_at_ms?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
        Some(now.saturating_sub(created) as f64 / 3_600_000.0)
    }

    /// The numbers, flattened for the model. Built here rather than in the UI
    /// so the model reads exactly what the card shows and the two can never
    /// drift into disagreeing about the same token.
    pub fn brief(&self) -> String {
        let pct = |v: Option<f64>| match v {
            Some(x) => format!("{x:+.2}%"),
            None => "n/a".into(),
        };
        let age = match self.age_hours() {
            Some(h) if h < 48.0 => format!("{h:.0} hours"),
            Some(h) => format!("{:.0} days", h / 24.0),
            None => "unknown".into(),
        };
        format!(
            "token: {} ({})\nprice: ${}\nchange 5m/1h/6h/24h: {} / {} / {} / {}\n\
             volume 1h: ${:.0}\nvolume 24h: ${:.0}\nliquidity: ${:.0}\nmarket cap: {}\n\
             buys/sells 24h: {} / {}\npool age: {}",
            self.symbol,
            self.name,
            self.price_usd,
            pct(self.change_m5),
            pct(self.change_h1),
            pct(self.change_h6),
            pct(self.change_h24),
            self.volume_h1,
            self.volume_h24,
            self.liquidity_usd,
            self.market_cap.map(|m| format!("${m:.0}")).unwrap_or_else(|| "n/a".into()),
            self.buys_h24,
            self.sells_h24,
            age,
        )
    }
}

impl Row {
    pub fn age_hours(&self) -> Option<f64> {
        let created = self.created_at_ms?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
        Some(now.saturating_sub(created) as f64 / 3_600_000.0)
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
}

fn num(v: &Value) -> Option<f64> {
    // Dexscreener sends prices as strings and volumes as numbers.
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// One quote token's worth of pairs.
fn pairs_for(address: &str) -> Result<Vec<Value>, String> {
    let url = format!("https://api.dexscreener.com/token-pairs/v1/{CHAIN}/{address}");
    let resp = agent()
        .get(&url)
        .set("User-Agent", UA)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("dexscreener HTTP {code}"),
            other => format!("network error: {other}"),
        })?;
    match resp.into_json::<Value>() {
        Ok(Value::Array(v)) => Ok(v),
        Ok(_) => Err("unexpected response shape".into()),
        Err(e) => Err(format!("bad response: {e}")),
    }
}

pub fn fetch(limit: usize) -> Result<Vec<Row>, String> {
    let chain = super::tokens::chain_by_name(CHAIN_NAME)
        .ok_or("no Robinhood chain in the registry")?;

    let mut raw: Vec<Value> = Vec::new();
    let mut last_err = None;
    for q in QUOTES {
        let Some(token) = chain.token(q) else { continue };
        match pairs_for(token.address) {
            Ok(mut v) => raw.append(&mut v),
            // One quote side failing still leaves a usable board.
            Err(e) => last_err = Some(e),
        }
    }
    if raw.is_empty() {
        return Err(last_err.unwrap_or_else(|| "no pairs returned".into()));
    }

    let rows = rank(&raw, limit);
    if rows.is_empty() {
        return Err("no tradable pairs on this chain right now".into());
    }
    Ok(rows)
}

/// Everything the indexer has on one ticker.
///
/// Ticker rather than address on purpose — a ticker is what you have when you
/// see a name scroll past. It is also ambiguous, so the busiest pair wins:
/// on a permissionless chain a dozen tokens can share a symbol, and the one
/// actually trading is the one being asked about.
pub fn lookup(ticker: &str) -> Result<Info, String> {
    let ticker = ticker.trim().trim_start_matches('$');
    if ticker.is_empty() {
        return Err("type a ticker first".into());
    }

    let url = format!(
        "https://api.dexscreener.com/latest/dex/search?q={}",
        urlencode(ticker)
    );
    let resp = agent()
        .get(&url)
        .set("User-Agent", UA)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("dexscreener HTTP {code}"),
            other => format!("network error: {other}"),
        })?;
    let v: Value = resp.into_json().map_err(|e| format!("bad response: {e}"))?;

    let pairs = v["pairs"].as_array().ok_or("unexpected response shape")?;
    let best = pairs
        .iter()
        .filter(|p| p["chainId"].as_str() == Some(CHAIN))
        .filter(|p| {
            p["baseToken"]["symbol"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(ticker))
        })
        .max_by(|a, b| {
            let va = num(&a["volume"]["h24"]).unwrap_or(0.0);
            let vb = num(&b["volume"]["h24"]).unwrap_or(0.0);
            va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| format!("no {ticker} pool on Robinhood Chain"))?;

    let txns = &best["txns"]["h24"];
    Ok(Info {
        symbol: best["baseToken"]["symbol"].as_str().unwrap_or(ticker).to_string(),
        name: best["baseToken"]["name"].as_str().unwrap_or_default().to_string(),
        address: best["baseToken"]["address"].as_str().unwrap_or_default().to_string(),
        price_usd: num(&best["priceUsd"]).unwrap_or(0.0),
        change_m5: num(&best["priceChange"]["m5"]),
        change_h1: num(&best["priceChange"]["h1"]),
        change_h6: num(&best["priceChange"]["h6"]),
        change_h24: num(&best["priceChange"]["h24"]),
        volume_h24: num(&best["volume"]["h24"]).unwrap_or(0.0),
        volume_h1: num(&best["volume"]["h1"]).unwrap_or(0.0),
        liquidity_usd: num(&best["liquidity"]["usd"]).unwrap_or(0.0),
        market_cap: num(&best["marketCap"]),
        buys_h24: txns["buys"].as_u64().unwrap_or(0),
        sells_h24: txns["sells"].as_u64().unwrap_or(0),
        created_at_ms: best["pairCreatedAt"].as_u64(),
        url: best["url"].as_str().unwrap_or_default().to_string(),
    })
}

/// Percent-encode the query. Tickers are usually bare ASCII, but nothing stops
/// someone launching a token whose symbol contains a space or an ampersand.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Fold the raw pairs into one row per token, keeping the deepest venue.
///
/// A token usually has several pools — one against WETH, one against USDG,
/// sometimes several fee tiers. Showing each would fill the board with
/// duplicates, so the pair doing the most volume wins and stands for the token.
fn rank(raw: &[Value], limit: usize) -> Vec<Row> {
    let mut best: Vec<Row> = Vec::new();

    for p in raw {
        let base = &p["baseToken"];
        let (Some(symbol), Some(address)) = (base["symbol"].as_str(), base["address"].as_str())
        else {
            continue;
        };
        // A quote token showing up as someone's base is the other side of a
        // pair we already cover, not a listing of its own.
        if QUOTES.contains(&symbol) {
            continue;
        }
        let volume_h24 = num(&p["volume"]["h24"]).unwrap_or(0.0);
        let row = Row {
            symbol: symbol.to_string(),
            address: address.to_string(),
            price_usd: num(&p["priceUsd"]).unwrap_or(0.0),
            change_h24: num(&p["priceChange"]["h24"]),
            volume_h24,
            liquidity_usd: num(&p["liquidity"]["usd"]).unwrap_or(0.0),
            created_at_ms: p["pairCreatedAt"].as_u64(),
        };

        match best.iter_mut().find(|r| r.address.eq_ignore_ascii_case(&row.address)) {
            Some(existing) if existing.volume_h24 < row.volume_h24 => *existing = row,
            Some(_) => {}
            None => best.push(row),
        }
    }

    best.sort_by(|a, b| {
        b.volume_h24.partial_cmp(&a.volume_h24).unwrap_or(std::cmp::Ordering::Equal)
    });
    best.truncate(limit);
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pair(sym: &str, addr: &str, vol: f64) -> Value {
        json!({
            "baseToken": { "symbol": sym, "address": addr },
            "priceUsd": "0.0001",
            "priceChange": { "h24": 12.5 },
            "volume": { "h24": vol },
            "liquidity": { "usd": 1000.0 },
            "pairCreatedAt": 1_700_000_000_000u64,
            "url": "https://dexscreener.com/robinhood/x"
        })
    }

    #[test]
    fn ranks_by_volume() {
        let raw = vec![pair("A", "0xa", 10.0), pair("B", "0xb", 900.0), pair("C", "0xc", 50.0)];
        let ranked = rank(&raw, 10);
        let got: Vec<&str> = ranked.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(got, ["B", "C", "A"]);
    }

    #[test]
    fn one_row_per_token_keeping_the_busiest_pool() {
        // Same token, two pools; the quieter one must not appear at all.
        let raw = vec![pair("DUP", "0xdup", 5.0), pair("DUP", "0xDUP", 700.0)];
        let rows = rank(&raw, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].volume_h24, 700.0);
    }

    #[test]
    fn drops_quote_tokens_appearing_as_a_base() {
        let raw = vec![pair("WETH", "0xw", 999.0), pair("REAL", "0xr", 1.0)];
        let rows = rank(&raw, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "REAL");
    }

    #[test]
    fn honours_the_limit() {
        let raw: Vec<Value> =
            (0..20).map(|i| pair(&format!("T{i}"), &format!("0x{i}"), i as f64)).collect();
        assert_eq!(rank(&raw, 5).len(), 5);
    }

    #[test]
    fn reads_prices_sent_as_strings_and_numbers() {
        assert_eq!(num(&json!("0.25")), Some(0.25));
        assert_eq!(num(&json!(0.25)), Some(0.25));
        assert_eq!(num(&json!(null)), None);
        assert_eq!(num(&json!("nope")), None);
    }

    #[test]
    fn survives_a_pair_with_fields_missing() {
        let raw = vec![json!({ "baseToken": { "symbol": "X", "address": "0xx" } })];
        let rows = rank(&raw, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].volume_h24, 0.0);
        assert_eq!(rows[0].change_h24, None);
        assert_eq!(rows[0].created_at_ms, None);
    }
}
