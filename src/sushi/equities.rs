//! Tokenised equities on Robinhood Chain.
//!
//! One board, one HTTP call. `/price/v1/{chain}` returns every priced token on
//! the chain in a single map, so the whole board costs exactly as much as a
//! single lookup would — we ask once and pick out the symbols we curate.
//!
//! Prices are what the Sushi pools imply, not what the underlying share trades
//! at on an exchange. The two track each other; they are not the same number,
//! and nothing here should be read as a quote on the actual security.

use serde_json::Value;

use super::api;
use super::tokens;

/// The chain this board is about. Named rather than indexed so a reordering of
/// `CHAINS` can't silently point the board at Ethereum.
pub const CHAIN: &str = "Robinhood";

/// Not equities, so they are excluded from the board even though they live in
/// the same curated list: WETH is the routing asset and USDG the unit.
const NOT_EQUITY: &[&str] = &["WETH", "USDG"];

pub struct Row {
    pub symbol: &'static str,
    pub price: f64,
}

pub fn fetch(api_key: &str) -> Result<Vec<Row>, String> {
    let chain = tokens::chain_by_name(CHAIN)
        .ok_or_else(|| format!("no chain named {CHAIN} in the registry"))?;
    select(chain, &api::prices(chain.id, api_key)?)
}

/// The whole of the board's logic, kept free of I/O so it can be tested against
/// a literal response rather than against whatever the pools happen to say today.
fn select(chain: &'static tokens::Chain, prices: &Value) -> Result<Vec<Row>, String> {
    let Value::Object(map) = prices else {
        return Err("unexpected price response shape".into());
    };

    // The endpoint keys by address in lowercase; the registry stores checksummed
    // mixed case for some chains, so normalise rather than trust either side.
    let mut rows: Vec<Row> = chain
        .tokens
        .iter()
        .filter(|t| !NOT_EQUITY.contains(&t.symbol))
        .filter_map(|t| {
            let price = map.get(&t.address.to_ascii_lowercase())?.as_f64()?;
            Some(Row { symbol: t.symbol, price })
        })
        .collect();

    if rows.is_empty() {
        return Err("no equity prices returned for this chain".into());
    }

    // Dearest first: it puts the index funds and the mega caps at the top left,
    // which is the reading order the board is laid out for.
    rows.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chain() -> &'static tokens::Chain {
        tokens::chain_by_name(CHAIN).expect("Robinhood chain in registry")
    }

    #[test]
    fn ranks_by_price_and_drops_non_equities() {
        let c = chain();
        let addr = |s: &str| c.token(s).unwrap().address.to_ascii_lowercase();
        let prices = json!({
            addr("NVDA"): 213.5,
            addr("SPY"): 747.7,
            addr("AAPL"): 324.4,
            // Present in the response, but never a tile.
            addr("WETH"): 1936.8,
            addr("USDG"): 1.0,
        });

        let rows = select(c, &prices).unwrap();
        let got: Vec<&str> = rows.iter().map(|r| r.symbol).collect();
        assert_eq!(got, ["SPY", "AAPL", "NVDA"]);
    }

    #[test]
    fn skips_tokens_the_response_omits() {
        let c = chain();
        let prices = json!({ c.token("TSLA").unwrap().address.to_ascii_lowercase(): 377.8 });
        let rows = select(c, &prices).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "TSLA");
    }

    #[test]
    fn matches_addresses_regardless_of_case() {
        let c = chain();
        let prices = json!({ c.token("GME").unwrap().address.to_ascii_uppercase(): 20.7 });
        // Uppercase keys are not what the API sends, so nothing should match —
        // the lookup normalises our side, not the response's.
        assert!(select(c, &prices).is_err());
    }

    #[test]
    fn rejects_a_non_object_response() {
        assert!(select(chain(), &json!([])).is_err());
        assert!(select(chain(), &Value::Null).is_err());
    }

    #[test]
    fn every_equity_is_priced_in_whole_units() {
        // Equities are all 18-decimal; a stray 6 would silently scale a quote
        // by 10^12 against USDG.
        for t in chain().tokens.iter().filter(|t| !NOT_EQUITY.contains(&t.symbol)) {
            assert_eq!(t.decimals, 18, "{}", t.symbol);
        }
    }
}
