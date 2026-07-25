//! Structured trade intents, and the curated-registry lookups behind them.
//!
//! These shapes used to be reached by asking a model to emit JSON and
//! parsing its answer back out. That's gone: the conversational agent
//! (`mod.rs`) now calls Anthropic's own tool-use, which hands back a
//! *structured, schema-validated* `input` object directly — there is no
//! free text to parse or a code fence to strip, so every `Intent` here is
//! built straight from that `serde_json::Value`. What hasn't changed is the
//! rule this module exists to enforce: the model never sees or emits an
//! address, and it never computes anything — resolution and arithmetic
//! happen here, in Rust, against the curated registry.

use serde_json::Value;

use super::tokens::{self, Chain, Token};
use super::trending::Window;

/// Robinhood Chain's own board, upper-bounded the same way the board's own
/// refresh button is (`trending::fetch` clamps again regardless, but a
/// tool call stating a limit shouldn't even reach it looking unbounded).
pub const SCREEN_LIMIT_MAX: usize = 30;

#[derive(Debug)]
pub enum Intent {
    Market {
        sort: super::market::Sort,
        limit: usize,
    },
    /// What's actually trading on Robinhood Chain right now — a chat-shaped
    /// door into the same data the always-visible board already shows.
    /// Distinct from `Market`: that one is the whole crypto market via
    /// CoinGecko, this one is this chain's own pools via Dexscreener.
    ChainScreen {
        limit: usize,
        /// Which of Dexscreener's windows to rank by — Dexscreener tracks
        /// 5m/1h/6h/24h, and unlike CoinGecko's market-wide view, this data
        /// source actually has all four, so "what's moving right now" can
        /// mean something finer than a day.
        window: Window,
    },
    /// One specific ticker on Robinhood Chain, looked up against the indexer
    /// rather than the curated registry — this is the door for a token that
    /// isn't (and will never be) on the `price`/`quote` whitelist, like
    /// anything that just launched.
    TokenLookup {
        ticker: String,
    },
    Price {
        chain: &'static Chain,
        token: &'static Token,
    },
    Quote {
        chain: &'static Chain,
        token_in: &'static Token,
        token_out: &'static Token,
        /// Base units. The human string is not carried along: the API echoes
        /// the amount back, and that echo is what gets displayed.
        amount_raw: u128,
    },
    /// One Ondo Stocks ticker (`TSLA`/`TSLAon`) — a reference read from
    /// Ondo's own API, not Robinhood Chain data. Kept as a bare ticker
    /// string rather than resolved against a registry: unlike the curated
    /// `Chain`/`Token` whitelist, `ondo.rs` normalizes and validates the
    /// symbol itself, since the set of Ondo Stocks isn't known ahead of time
    /// here.
    StockLookup {
        ticker: String,
    },
    /// One Ondo Stocks ticker's dividend info (yield, payout frequency, last
    /// cash amount/date) — a separate Ondo endpoint from `StockLookup`'s
    /// market data, so its own intent rather than folded into that one.
    DividendLookup {
        ticker: String,
    },
}

/// Reads `input["chain"]` (defaulting to Ethereum) against the curated
/// registry — shared by the `price` and `quote` tools, which both take a
/// chain name the same way.
pub fn chain_of(v: &Value) -> Result<&'static Chain, String> {
    let name = v["chain"].as_str().unwrap_or("Ethereum");
    tokens::chain_by_name(name).ok_or_else(|| format!("unsupported chain: {name}"))
}

/// Resolves `input[key]` against `chain`'s curated symbol list. Never
/// resolves a bare address — the whitelist only knows symbols, so anything
/// smuggled in as a symbol (including an address) just fails the lookup.
pub fn token_of(chain: &'static Chain, v: &Value, key: &str) -> Result<&'static Token, String> {
    let sym = v[key].as_str().ok_or_else(|| format!("missing `{key}`"))?;
    chain.token(sym).ok_or_else(|| format!("{sym} is not whitelisted on {}", chain.name))
}

/// Builds a `Quote` from the same `tool_use` input shape the `get_quote`
/// tool declares — chain/tokenIn/tokenOut/amount — validating everything a
/// hand-typed JSON reply used to need checked by hand: both tokens
/// resolve, they aren't the same token, and the amount parses to a
/// non-zero base-unit value against the input token's own decimals.
pub fn quote_of(v: &Value) -> Result<Intent, String> {
    let chain = chain_of(v)?;
    let token_in = token_of(chain, v, "token_in")?;
    let token_out = token_of(chain, v, "token_out")?;
    if token_in.address.eq_ignore_ascii_case(token_out.address) {
        return Err("input and output token are the same".into());
    }
    let amount_human = v["amount"]
        .as_str()
        .map(str::to_string)
        // A model that sends a bare number instead of a string still gives a
        // usable value; don't fail the whole request over quoting.
        .or_else(|| v["amount"].as_f64().map(|n| n.to_string()))
        .ok_or("no amount given")?;
    let amount_raw = tokens::parse_units(&amount_human, token_in.decimals)?;
    if amount_raw == 0 {
        return Err("amount is zero".into());
    }
    Ok(Intent::Quote { chain, token_in, token_out, amount_raw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_a_quote() {
        let i = quote_of(&json!({
            "chain": "Base", "token_in": "ETH", "token_out": "USDC", "amount": "0.5"
        }))
        .unwrap();
        let Intent::Quote { chain, token_in, token_out, amount_raw, .. } = i else {
            panic!("expected a quote");
        };
        assert_eq!(chain.id, 8453);
        assert_eq!(token_in.symbol, "ETH");
        assert_eq!(token_out.symbol, "USDC");
        assert_eq!(amount_raw, 500_000_000_000_000_000);
    }

    #[test]
    fn chain_of_defaults_to_ethereum() {
        let chain = chain_of(&json!({})).unwrap();
        assert_eq!(chain.id, 1);
    }

    #[test]
    fn token_of_refuses_a_symbol_outside_the_whitelist() {
        let chain = chain_of(&json!({"chain": "Ethereum"})).unwrap();
        let err = token_of(chain, &json!({"token": "SCAMCOIN"}), "token").unwrap_err();
        assert!(err.contains("SCAMCOIN") && err.contains("not whitelisted"), "{err}");
    }

    #[test]
    fn token_of_refuses_an_address_smuggled_in_as_a_symbol() {
        let chain = chain_of(&json!({"chain": "Ethereum"})).unwrap();
        let err = token_of(
            chain,
            &json!({"token": "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}),
            "token",
        )
        .unwrap_err();
        assert!(err.contains("not whitelisted"), "{err}");
    }

    #[test]
    fn quote_of_refuses_the_same_token_on_both_sides() {
        let err = quote_of(&json!({
            "chain": "Ethereum", "token_in": "ETH", "token_out": "ETH", "amount": "1"
        }))
        .unwrap_err();
        assert!(err.contains("same"), "{err}");
    }

    #[test]
    fn quote_of_accepts_a_bare_number_amount() {
        let i = quote_of(&json!({
            "chain": "Ethereum", "token_in": "ETH", "token_out": "USDC", "amount": 0.5
        }))
        .unwrap();
        let Intent::Quote { amount_raw, .. } = i else { panic!("expected a quote") };
        assert_eq!(amount_raw, 500_000_000_000_000_000);
    }
}
