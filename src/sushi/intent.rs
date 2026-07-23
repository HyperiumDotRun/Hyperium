//! Natural language -> structured intent.
//!
//! The model's only job is to turn a sentence into a symbol + amount. It never
//! sees or emits an address, and it never computes anything: resolution and
//! arithmetic happen here, in Rust, against the curated registry.

use serde_json::Value;

use super::market;
use super::tokens::{self, Chain, Token};

/// Robinhood Chain's own board, upper-bounded the same way the board's own
/// refresh button is (`trending::fetch` clamps again regardless, but a
/// model-stated limit shouldn't even reach it looking unbounded).
pub const SCREEN_LIMIT_MAX: usize = 30;

#[derive(Debug)]
pub enum Intent {
    Market {
        sort: market::Sort,
        limit: usize,
    },
    /// What's actually trading on Robinhood Chain right now — a chat-shaped
    /// door into the same data the always-visible board already shows.
    /// Distinct from `Market`: that one is the whole crypto market via
    /// CoinGecko, this one is this chain's own pools via Dexscreener.
    ChainScreen {
        limit: usize,
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
}

pub fn system_prompt() -> String {
    format!(
        "You translate a user's crypto question into one JSON object. Reply with \
the JSON only — no prose, no code fence.\n\n\
The message may start with a recap of recent conversation turns, ending in a line \
starting \"New question:\". If so, use the recap only to resolve what a vague \
follow-up refers to (\"the second one\", \"and that one's volume\") — answer the new \
question, never the old ones, and never repeat a prior answer instead of producing \
a fresh JSON object.\n\n\
Shapes:\n\
  {{\"action\":\"market\",\"sort\":\"volume|market_cap|gainers|losers\",\"limit\":10}}\n\
  {{\"action\":\"screen\",\"limit\":20}}\n\
  {{\"action\":\"lookup\",\"ticker\":\"<symbol>\"}}\n\
  {{\"action\":\"price\",\"chain\":\"<chain>\",\"token\":\"<symbol>\"}}\n\
  {{\"action\":\"quote\",\"chain\":\"<chain>\",\"tokenIn\":\"<symbol>\",\
\"tokenOut\":\"<symbol>\",\"amount\":\"<decimal>\"}}\n\
  {{\"action\":\"unknown\",\"reason\":\"<short reason>\"}}\n\n\
PREFER \"market\". It covers every question about the market at large — what is \
moving, biggest volume, top coins, trending, best/worst performers, \"show me \
the market\", or any vague request. It needs no coin and no chain. When the \
question is even loosely about the market, answer with \"market\" rather than \
refusing. Default to sort \"volume\" and limit 10 when unstated; map \
\"biggest/top/most traded\" to volume, \"largest/biggest coins\" to market_cap, \
\"pumping/up the most/best performers\" to gainers, \"dumping/down the most/worst\" \
to losers. limit is at most 50.\n\n\
Use \"screen\" instead of \"market\" only when the question is specifically about \
Robinhood Chain itself — \"on Robinhood\", \"this chain\", \"what just launched\", \
\"new tokens\", \"good volume here/on-chain\". \"market\" is the whole crypto \
market; \"screen\" is only what is actually trading on this one chain right now. \
Default limit 20 when unstated; at most 30.\n\n\
Use \"price\" only for one specific token's value, and \"quote\" only to convert a \
stated amount of one token into another. Both are restricted to these chains \
and symbols:\n{}\n\
Rules for price/quote:\n\
- Never output a contract address. Only symbols from the list above.\n\
- If the chain is not stated, use Ethereum.\n\
- If the user names a specific token that is NOT in that list, do not refuse and do \
not fall back to \"market\" — answer with \"lookup\" instead, ticker as typed. This \
is the common case: most tokens trading on Robinhood Chain (anything freshly \
launched, any ticker you don't recognise) are not and will never be on that list.\n\
- \"amount\" is a plain decimal string of the input token, e.g. \"0.5\". Never \
convert it to base units, never do arithmetic.\n\n\
Use \"lookup\" for a specific ticker not in the whitelist above — a coin someone \
just mentioned by name, asking \"how's X doing\", \"what's X\", \"pull up X\". \
Strip a leading $ if present. This only resolves tokens actually trading on \
Robinhood Chain; if it doesn't exist there, that surfaces as an error, not a \
reason to avoid trying.\n\n\
Use \"unknown\" ONLY when the question has nothing to do with crypto, tokens, \
prices or trading. Never use it merely because a coin or chain is missing.",
        tokens::catalog()
    )
}

/// Models sometimes wrap JSON in a fence despite being told not to.
fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("```") else { return s };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim_start_matches('\n').trim_end().trim_end_matches('`').trim()
}

fn chain_of(v: &Value) -> Result<&'static Chain, String> {
    let name = v["chain"].as_str().unwrap_or("Ethereum");
    tokens::chain_by_name(name).ok_or_else(|| format!("unsupported chain: {name}"))
}

fn token_of(chain: &'static Chain, v: &Value, key: &str) -> Result<&'static Token, String> {
    let sym = v[key]
        .as_str()
        .ok_or_else(|| format!("missing `{key}` in the model's answer"))?;
    chain.token(sym).ok_or_else(|| format!("{sym} is not whitelisted on {}", chain.name))
}

pub fn parse(raw: &str) -> Result<Intent, String> {
    let cleaned = strip_fence(raw);
    let v: Value = serde_json::from_str(cleaned)
        .map_err(|_| format!("could not read the model's answer: {cleaned}"))?;

    match v["action"].as_str().unwrap_or_default() {
        "market" => {
            let sort = v["sort"]
                .as_str()
                .and_then(market::Sort::parse)
                .unwrap_or(market::Sort::Volume);
            // Clamp here as well as in `fetch`: the model is not trusted to
            // respect the stated maximum.
            let limit = v["limit"].as_u64().unwrap_or(10).clamp(1, market::LIMIT_MAX as u64);
            Ok(Intent::Market { sort, limit: limit as usize })
        }
        "screen" => {
            let limit =
                v["limit"].as_u64().unwrap_or(20).clamp(1, SCREEN_LIMIT_MAX as u64);
            Ok(Intent::ChainScreen { limit: limit as usize })
        }
        "lookup" => {
            let ticker = v["ticker"]
                .as_str()
                .map(|s| s.trim().trim_start_matches('$').to_string())
                .filter(|s| !s.is_empty())
                .ok_or("missing `ticker` in the model's answer")?;
            Ok(Intent::TokenLookup { ticker })
        }
        "price" => {
            let chain = chain_of(&v)?;
            let token = token_of(chain, &v, "token")?;
            Ok(Intent::Price { chain, token })
        }
        "quote" => {
            let chain = chain_of(&v)?;
            let token_in = token_of(chain, &v, "tokenIn")?;
            let token_out = token_of(chain, &v, "tokenOut")?;
            if token_in.address.eq_ignore_ascii_case(token_out.address) {
                return Err("input and output token are the same".into());
            }
            let amount_human = v["amount"]
                .as_str()
                .map(str::to_string)
                // A model that ignores the "string" instruction still gives a
                // usable number; don't fail the whole request over quoting.
                .or_else(|| v["amount"].as_f64().map(|n| n.to_string()))
                .ok_or("no amount in the model's answer")?;
            let amount_raw = tokens::parse_units(&amount_human, token_in.decimals)?;
            if amount_raw == 0 {
                return Err("amount is zero".into());
            }
            Ok(Intent::Quote { chain, token_in, token_out, amount_raw })
        }
        "unknown" => {
            Err(v["reason"].as_str().unwrap_or("could not understand the request").to_string())
        }
        other => Err(format!("unexpected action: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_quote() {
        let i = parse(r#"{"action":"quote","chain":"Base","tokenIn":"ETH","tokenOut":"USDC","amount":"0.5"}"#)
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
    fn parses_a_lookup_and_strips_a_leading_dollar() {
        let i = parse(r#"{"action":"lookup","ticker":"$PONS"}"#).unwrap();
        let Intent::TokenLookup { ticker } = i else { panic!("expected a lookup") };
        assert_eq!(ticker, "PONS");
    }

    #[test]
    fn refuses_a_lookup_with_no_ticker() {
        assert!(parse(r#"{"action":"lookup"}"#).is_err());
        assert!(parse(r#"{"action":"lookup","ticker":""}"#).is_err());
    }

    #[test]
    fn parses_a_chain_screen_and_clamps_its_limit() {
        let i = parse(r#"{"action":"screen","limit":9999}"#).unwrap();
        let Intent::ChainScreen { limit } = i else { panic!("expected a screen") };
        assert_eq!(limit, SCREEN_LIMIT_MAX);

        let i = parse(r#"{"action":"screen"}"#).unwrap();
        let Intent::ChainScreen { limit } = i else { panic!("expected a screen") };
        assert_eq!(limit, 20);
    }

    #[test]
    fn survives_a_code_fence() {
        let raw = "```json\n{\"action\":\"price\",\"chain\":\"Ethereum\",\"token\":\"SUSHI\"}\n```";
        assert!(matches!(parse(raw), Ok(Intent::Price { .. })));
    }

    #[test]
    fn defaults_to_ethereum() {
        let i = parse(r#"{"action":"price","token":"WBTC"}"#).unwrap();
        let Intent::Price { chain, .. } = i else { panic!("expected a price") };
        assert_eq!(chain.id, 1);
    }

    #[test]
    fn refuses_tokens_outside_the_whitelist() {
        let err = parse(r#"{"action":"quote","chain":"Ethereum","tokenIn":"ETH","tokenOut":"SCAMCOIN","amount":"1"}"#)
            .unwrap_err();
        assert!(err.contains("SCAMCOIN"), "{err}");
    }

    #[test]
    fn refuses_an_address_smuggled_in_as_a_symbol() {
        let err = parse(r#"{"action":"price","chain":"Ethereum","token":"0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}"#)
            .unwrap_err();
        assert!(err.contains("not whitelisted"), "{err}");
    }

    #[test]
    fn reports_the_models_own_refusal() {
        let err = parse(r#"{"action":"unknown","reason":"Solana is not supported"}"#).unwrap_err();
        assert_eq!(err, "Solana is not supported");
    }
}
