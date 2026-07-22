//! Thin client over the hosted Sushi API (api.sushi.com).
//!
//! Read-only surface only: price and quote. `/swap` is deliberately absent —
//! it returns signable calldata, and nothing in this build should be able to
//! produce that.

use std::time::Duration;

use serde_json::Value;

const BASE: &str = "https://api.sushi.com";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
}

/// Errors come back as RFC 7807 problem details. The docs use `detail` on some
/// error types and `details` on others, so read both.
fn problem(code: u16, resp: ureq::Response) -> String {
    let v: Value = resp.into_json().unwrap_or(Value::Null);
    let title = v["title"].as_str().unwrap_or("");
    let detail =
        v["detail"].as_str().or_else(|| v["details"].as_str()).unwrap_or("");

    // Validation errors carry a per-parameter list; surface it instead of a
    // bare "invalid request".
    if let Some(errors) = v["errors"].as_array() {
        let parts: Vec<String> = errors
            .iter()
            .filter_map(|e| {
                let p = e["parameter"].as_str()?;
                let d = e["detail"].as_str().unwrap_or("invalid");
                Some(format!("{p}: {d}"))
            })
            .collect();
        if !parts.is_empty() {
            return format!("{code} {title} — {}", parts.join(", "));
        }
    }

    match (title.is_empty(), detail.is_empty()) {
        (false, false) => format!("{code} {title} — {detail}"),
        (false, true) => format!("{code} {title}"),
        (true, false) => format!("{code} — {detail}"),
        (true, true) => format!("HTTP {code}"),
    }
}

fn get(url: &str, params: &[(&str, String)], api_key: &str) -> Result<Value, String> {
    let ag = agent();
    let mut req = ag.get(url);
    for (k, v) in params {
        req = req.query(k, v);
    }
    if !api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", api_key.trim()));
    }
    match req.call() {
        Ok(resp) => resp.into_json().map_err(|e| format!("bad response: {e}")),
        Err(ureq::Error::Status(code, resp)) => Err(problem(code, resp)),
        Err(e) => Err(format!("network error: {e}")),
    }
}

/// Every priced token on a chain, keyed by lowercase address.
///
/// Returned raw: the caller knows which addresses it cares about, and the map
/// runs to a couple of thousand entries on a busy chain.
pub fn prices(chain_id: u64, api_key: &str) -> Result<Value, String> {
    get(&format!("{BASE}/price/v1/{chain_id}"), &[], api_key)
}

/// USD price of a single token.
pub fn price(chain_id: u64, address: &str, api_key: &str) -> Result<f64, String> {
    let url = format!("{BASE}/price/v1/{chain_id}/{address}");
    let v = get(&url, &[], api_key)?;
    v.as_f64().ok_or_else(|| "no price available for this token".to_string())
}

#[derive(Clone)]
pub struct QuoteToken {
    pub address: String,
    pub symbol: String,
    pub decimals: u32,
}

#[derive(Clone)]
pub struct Quote {
    /// `Success`, `Partial` or `NoWay`.
    pub status: String,
    pub token_in: QuoteToken,
    pub token_out: QuoteToken,
    pub amount_in: u128,
    pub amount_out: u128,
    /// Fraction, not percent: 0.005 means 0.5%.
    pub price_impact: Option<f64>,
}

impl Quote {
    /// Unit price of one whole `token_in` expressed in `token_out`.
    ///
    /// Derived from the amounts rather than read from the response's
    /// `swapPrice`: that field is a raw base-unit ratio, so it only matches a
    /// human price when both tokens happen to share the same decimals — which
    /// hides the bug on pairs like ETH/WETH and exposes it on ETH/USDC.
    /// Display-only, hence f64.
    pub fn unit_price(&self) -> Option<f64> {
        if self.amount_in == 0 {
            return None;
        }
        let scaled_in = self.amount_in as f64 / 10f64.powi(self.token_in.decimals as i32);
        let scaled_out = self.amount_out as f64 / 10f64.powi(self.token_out.decimals as i32);
        Some(scaled_out / scaled_in)
    }
}

pub struct QuoteRequest {
    pub chain_id: u64,
    pub token_in: String,
    pub token_out: String,
    /// Raw base units, already resolved against the token's decimals.
    pub amount: u128,
    pub max_slippage: f64,
}

fn token_at(v: &Value, key: &str) -> Option<QuoteToken> {
    // `tokenFrom`/`tokenTo` are indexes into `tokens`, but the API also
    // expands them into full objects. Accept whichever we get.
    let node = match &v[key] {
        Value::Number(n) => {
            let idx = n.as_u64()? as usize;
            v["tokens"].as_array()?.get(idx)?.clone()
        }
        obj @ Value::Object(_) => obj.clone(),
        _ => return None,
    };
    Some(QuoteToken {
        address: node["address"].as_str().unwrap_or_default().to_string(),
        symbol: node["symbol"].as_str().unwrap_or("?").to_string(),
        decimals: node["decimals"].as_u64().unwrap_or(18) as u32,
    })
}

fn amount_at(v: &Value, key: &str) -> u128 {
    // Amounts are stringified integers; they overflow f64 well before they
    // overflow u128, so never route them through as_f64.
    v[key].as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn quote(req: &QuoteRequest, api_key: &str) -> Result<Quote, String> {
    let url = format!("{BASE}/quote/v7/{}", req.chain_id);
    let params = [
        ("tokenIn", req.token_in.clone()),
        ("tokenOut", req.token_out.clone()),
        ("amount", req.amount.to_string()),
        ("maxSlippage", req.max_slippage.to_string()),
    ];
    let v = get(&url, &params, api_key)?;

    let status = v["status"].as_str().unwrap_or("Unknown").to_string();
    if status == "NoWay" {
        return Err("no route found for this pair and amount".into());
    }

    let (Some(token_in), Some(token_out)) = (token_at(&v, "tokenFrom"), token_at(&v, "tokenTo"))
    else {
        return Err("unexpected response shape (no token data)".into());
    };

    Ok(Quote {
        status,
        token_in,
        token_out,
        amount_in: amount_at(&v, "amountIn"),
        amount_out: amount_at(&v, "assumedAmountOut"),
        price_impact: v["priceImpact"].as_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_expanded_and_indexed_tokens() {
        let v: Value = serde_json::from_str(
            r#"{
              "tokens": [
                {"address":"0xaa","decimals":18,"symbol":"ETH","name":"Ether"},
                {"address":"0xbb","decimals":6,"symbol":"USDC","name":"USD Coin"}
              ],
              "tokenFrom": 0,
              "tokenTo": {"address":"0xbb","decimals":6,"symbol":"USDC","name":"USD Coin"}
            }"#,
        )
        .unwrap();

        let from = token_at(&v, "tokenFrom").unwrap();
        let to = token_at(&v, "tokenTo").unwrap();
        assert_eq!(from.symbol, "ETH");
        assert_eq!(from.decimals, 18);
        assert_eq!(to.symbol, "USDC");
        assert_eq!(to.decimals, 6);
    }

    fn quote_of(dec_in: u32, amount_in: u128, dec_out: u32, amount_out: u128) -> Quote {
        Quote {
            status: "Success".into(),
            token_in: QuoteToken { address: "0xaa".into(), symbol: "IN".into(), decimals: dec_in },
            token_out: QuoteToken { address: "0xbb".into(), symbol: "OUT".into(), decimals: dec_out },
            amount_in,
            amount_out,
            price_impact: None,
        }
    }

    #[test]
    fn unit_price_survives_mismatched_decimals() {
        // 1 ETH (18 dec) -> 1929.048213 USDC (6 dec). The response's own
        // swapPrice would read 1.929e-9 here; the derived price must not.
        let q = quote_of(18, 1_000_000_000_000_000_000, 6, 1_929_048_213);
        let p = q.unit_price().unwrap();
        assert!((p - 1929.048213).abs() < 1e-6, "got {p}");
    }

    #[test]
    fn unit_price_handles_equal_decimals_and_zero_input() {
        let q = quote_of(18, 1_000_000_000_000_000_000, 18, 2_500_000_000_000_000_000);
        assert!((q.unit_price().unwrap() - 2.5).abs() < 1e-9);
        assert!(quote_of(18, 0, 6, 1).unit_price().is_none());
    }

    #[test]
    fn reads_amounts_beyond_f64_precision() {
        let v: Value =
            serde_json::from_str(r#"{"amountIn":"1234567890123456789"}"#).unwrap();
        assert_eq!(amount_at(&v, "amountIn"), 1_234_567_890_123_456_789);
    }
}
