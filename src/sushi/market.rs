//! Market overview — top tokens by 24h volume.
//!
//! Source is CoinGecko's public markets endpoint: no key, no signup. Note this
//! is *market-wide* volume, not Sushi's own. It answers "what is moving right
//! now"; it does not claim the token is tradeable on Sushi.

use std::time::Duration;

use serde_json::Value;

const URL: &str = "https://api.coingecko.com/api/v3/coins/markets";

pub struct Row {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub volume: f64,
    pub change_24h: Option<f64>,
    /// 7 days of hourly closes, used for the row sparkline.
    pub spark: Vec<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Volume,
    MarketCap,
    Gainers,
    Losers,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "volume" => Some(Self::Volume),
            "market_cap" | "marketcap" => Some(Self::MarketCap),
            "gainers" => Some(Self::Gainers),
            "losers" => Some(Self::Losers),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Volume => "by 24h volume",
            Self::MarketCap => "by market cap",
            Self::Gainers => "biggest 24h gainers",
            Self::Losers => "biggest 24h losers",
        }
    }
}

/// Upper bound on rows, enforced here rather than trusting whatever the model
/// put in `limit` — an unbounded page size burns the keyless rate limit.
pub const LIMIT_MAX: usize = 50;
/// Gainers/losers have no server-side ordering, so rank a broad pool locally.
const POOL: usize = 100;

pub fn fetch(sort: Sort, limit: usize) -> Result<Vec<Row>, String> {
    let limit = limit.clamp(1, LIMIT_MAX);
    let ranked_locally = matches!(sort, Sort::Gainers | Sort::Losers);
    let order = match sort {
        Sort::Volume => "volume_desc",
        Sort::MarketCap | Sort::Gainers | Sort::Losers => "market_cap_desc",
    };
    let per_page = if ranked_locally { POOL } else { limit };

    let mut rows = request(order, per_page)?;

    if ranked_locally {
        // Missing change data sorts last either way, never into the top slots.
        rows.sort_by(|a, b| {
            let (x, y) = (a.change_24h, b.change_24h);
            match sort {
                Sort::Gainers => y
                    .unwrap_or(f64::NEG_INFINITY)
                    .partial_cmp(&x.unwrap_or(f64::NEG_INFINITY))
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => x
                    .unwrap_or(f64::INFINITY)
                    .partial_cmp(&y.unwrap_or(f64::INFINITY))
                    .unwrap_or(std::cmp::Ordering::Equal),
            }
        });
        rows.truncate(limit);
    }

    Ok(rows)
}

fn request(order: &str, per_page: usize) -> Result<Vec<Row>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build();

    let resp = agent
        .get(URL)
        .query("vs_currency", "usd")
        .query("order", order)
        .query("per_page", &per_page.to_string())
        .query("page", "1")
        .query("sparkline", "true")
        .query("price_change_percentage", "24h")
        .call();

    let v: Value = match resp {
        Ok(r) => r.into_json().map_err(|e| format!("bad response: {e}"))?,
        // CoinGecko's keyless tier throttles hard; say so plainly rather than
        // surfacing a bare 429.
        Err(ureq::Error::Status(429, _)) => {
            return Err("CoinGecko rate limit reached — wait a moment and refresh".into());
        }
        Err(ureq::Error::Status(code, _)) => return Err(format!("CoinGecko HTTP {code}")),
        Err(e) => return Err(format!("network error: {e}")),
    };

    let arr = v.as_array().ok_or("unexpected response shape")?;
    Ok(arr.iter().map(row_of).collect())
}

fn row_of(v: &Value) -> Row {
    Row {
        symbol: v["symbol"].as_str().unwrap_or("?").to_uppercase(),
        name: v["name"].as_str().unwrap_or_default().to_string(),
        price: v["current_price"].as_f64().unwrap_or(0.0),
        volume: v["total_volume"].as_f64().unwrap_or(0.0),
        change_24h: v["price_change_percentage_24h"].as_f64(),
        spark: v["sparkline_in_7d"]["price"]
            .as_array()
            .map(|a| a.iter().filter_map(|p| p.as_f64()).map(|p| p as f32).collect())
            .unwrap_or_default(),
    }
}

/// Compact money, e.g. `$46.64B`. Volumes span nine orders of magnitude, so a
/// fixed format either overflows the column or loses every meaningful digit.
pub fn money_compact(v: f64) -> String {
    let a = v.abs();
    match a {
        _ if a >= 1e12 => format!("${:.2}T", v / 1e12),
        _ if a >= 1e9 => format!("${:.2}B", v / 1e9),
        _ if a >= 1e6 => format!("${:.2}M", v / 1e6),
        _ if a >= 1e3 => format!("${:.1}K", v / 1e3),
        _ => format!("${v:.2}"),
    }
}

/// Price with precision chosen from magnitude: a stablecoin needs four
/// decimals to show it is off peg, BTC needs none.
pub fn money_price(v: f64) -> String {
    let a = v.abs();
    match a {
        _ if a >= 1000.0 => {
            // thousands separators, without pulling in a formatting crate
            let whole = format!("{:.0}", v);
            let (sign, digits) = whole.strip_prefix('-').map_or(("", whole.as_str()), |d| ("-", d));
            let mut out = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 {
                    out.push(',');
                }
                out.push(c);
            }
            format!("${sign}{out}")
        }
        _ if a >= 1.0 => format!("${v:.2}"),
        _ if a >= 0.01 => format!("${v:.4}"),
        _ => format!("${v:.6}"),
    }
}

/// Always carries an explicit sign. The sign — not the colour — is what makes
/// direction readable for a red/green colour-blind reader.
pub fn pct_signed(v: f64) -> String {
    let sign = if v > 0.0 { "+" } else if v < 0.0 { "−" } else { "" };
    format!("{sign}{:.2}%", v.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_volume_by_magnitude() {
        assert_eq!(money_compact(46_640_159_763.0), "$46.64B");
        assert_eq!(money_compact(9_995_199_001.0), "$10.00B");
        assert_eq!(money_compact(1_500_000.0), "$1.50M");
        assert_eq!(money_compact(2_500.0), "$2.5K");
        assert_eq!(money_compact(12.0), "$12.00");
    }

    #[test]
    fn prices_keep_meaningful_digits() {
        assert_eq!(money_price(65_740.0), "$65,740");
        assert_eq!(money_price(1_931.12), "$1,931");
        assert_eq!(money_price(10.65), "$10.65");
        // a stablecoin must visibly show it is off peg
        assert_eq!(money_price(0.999357), "$0.9994");
        assert_eq!(money_price(0.00001234), "$0.000012");
    }

    #[test]
    fn percentages_always_carry_a_sign() {
        assert_eq!(pct_signed(0.0058), "+0.01%");
        assert_eq!(pct_signed(-1.35798), "−1.36%");
        assert_eq!(pct_signed(0.0), "0.00%");
    }

    #[test]
    fn parses_a_market_row() {
        let v: Value = serde_json::from_str(
            r#"{"symbol":"eth","name":"Ethereum","current_price":1931.12,
                "total_volume":9995199001,"price_change_percentage_24h":0.2668,
                "sparkline_in_7d":{"price":[1900.5,1910.25,1931.12]}}"#,
        )
        .unwrap();
        let r = row_of(&v);
        assert_eq!(r.symbol, "ETH");
        assert_eq!(r.spark.len(), 3);
        assert!((r.change_24h.unwrap() - 0.2668).abs() < 1e-9);
    }

    #[test]
    fn tolerates_missing_fields() {
        let v: Value = serde_json::from_str(r#"{"symbol":"x"}"#).unwrap();
        let r = row_of(&v);
        assert_eq!(r.symbol, "X");
        assert!(r.change_24h.is_none());
        assert!(r.spark.is_empty());
    }
}
