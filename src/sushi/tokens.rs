//! Curated token registry.
//!
//! The whole point of this table is that the model never gets to invent an
//! address: it may only answer with a `symbol` that is resolved here. A
//! hallucinated contract address is the one mistake that cannot be undone, so
//! symbol -> address is a lookup, never a generation.
//!
//! Read-only for now (price + quote), where a wrong entry only yields a wrong
//! number. Re-verify every address against the official token list before any
//! signing path is added.

/// Sentinel the Sushi API uses for a chain's native asset.
pub const NATIVE: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

#[derive(Debug)]
pub struct Token {
    pub symbol: &'static str,
    pub address: &'static str,
    pub decimals: u32,
}

#[derive(Debug)]
pub struct Chain {
    pub id: u64,
    pub name: &'static str,
    pub tokens: &'static [Token],
}

pub const CHAINS: &[Chain] = &[
    // First on purpose: the tool opens on this chain, and with the default
    // from/to indexes (0 and 2) the panel lands on WETH -> NVDA, which is the
    // one pair that shows at a glance what this chain is for.
    //
    // Robinhood Chain. Tokenised equities and ETFs, quoted like any other ERC-20.
    // Every address below was resolved through `GET /token/v1/4663/{address}`
    // and checked against the symbol and decimals the API reports — none was
    // typed from memory. The equities all carry a "· Robinhood Token" name.
    //
    // No native ETH entry, deliberately: `/price/v1/4663` 404s on the NATIVE
    // sentinel, so a price lookup on it would fail even though quotes route
    // fine. WETH is the wrapped form the pools actually hold.
    Chain {
        id: 4663,
        name: "Robinhood",
        tokens: &[
            Token {
                symbol: "WETH",
                address: "0x0bd7d308f8e1639fab988df18a8011f41eacad73",
                decimals: 18,
            },
            // The chain's reference stable. Note 6 decimals against the
            // equities' 18 — the pair every quote here has to get right.
            Token {
                symbol: "USDG",
                address: "0x5fc5360d0400a0fd4f2af552add042d716f1d168",
                decimals: 6,
            },
            // The third quote token launches actually route through: this is
            // what a bonding-curve launch on Virtuals pairs against, so a
            // freshly-launched project's own pool is usually against this, not
            // WETH or USDG. See `trending::QUOTES`.
            Token {
                symbol: "VIRTUAL",
                address: "0xc6911796042b15d7fa4f6cde69e245ddcd3d9c31",
                decimals: 18,
            },
            Token {
                symbol: "NVDA",
                address: "0xd0601ce157db5bdc3162bbac2a2c8af5320d9eec",
                decimals: 18,
            },
            Token {
                symbol: "AAPL",
                address: "0xaf3d76f1834a1d425780943c99ea8a608f8a93f9",
                decimals: 18,
            },
            Token {
                symbol: "TSLA",
                address: "0x322f0929c4625ed5bad873c95208d54e1c003b2d",
                decimals: 18,
            },
            Token {
                symbol: "MSFT",
                address: "0xe93237c50d904957cf27e7b1133b510c669c2e74",
                decimals: 18,
            },
            Token {
                symbol: "GOOGL",
                address: "0x2e0847e8910a9732eb3fb1bb4b70a580adad4fe3",
                decimals: 18,
            },
            Token {
                symbol: "META",
                address: "0xc0d6457c16cc70d6790dd43521c899c87ce02f35",
                decimals: 18,
            },
            Token {
                symbol: "AMZN",
                address: "0x12f190a9f9d7d37a250758b26824b97ce941bf54",
                decimals: 18,
            },
            Token {
                symbol: "AMD",
                address: "0x86923f96303d656e4aa86d9d42d1e57ad2023fdc",
                decimals: 18,
            },
            Token {
                symbol: "PLTR",
                address: "0x894e1ec2d74ffe5aef8dc8a9e84686accb964f2a",
                decimals: 18,
            },
            Token {
                symbol: "MSTR",
                address: "0xec262a75e413fafd0df80480274532c79d42da09",
                decimals: 18,
            },
            Token {
                symbol: "GME",
                address: "0x1b0e319c6a659f002271b69db8a7df2f911c153e",
                decimals: 18,
            },
            Token {
                symbol: "SPY",
                address: "0x117cc2133c37b721f49de2a7a74833232b3b4c0c",
                decimals: 18,
            },
            Token {
                symbol: "QQQ",
                address: "0xd5f3879160bc7c32ebb4dc785f8a4f505888de68",
                decimals: 18,
            },
            // Private companies. There is no ticker for these on a normal
            // exchange, which is the whole point of listing them here.
            Token {
                symbol: "SPCX",
                address: "0x4a0e65a3eccec6dbe60ae065f2e7bb85fae35eea",
                decimals: 18,
            },
            Token {
                symbol: "CBRS",
                address: "0x5c90450bbb4273d7b2f17cf6917aeb237a569679",
                decimals: 18,
            },
        ],
    },
    Chain {
        id: 1,
        name: "Ethereum",
        tokens: &[
            Token { symbol: "ETH", address: NATIVE, decimals: 18 },
            Token {
                symbol: "WETH",
                address: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                decimals: 18,
            },
            Token {
                symbol: "USDC",
                address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                decimals: 6,
            },
            Token {
                symbol: "USDT",
                address: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
                decimals: 6,
            },
            Token {
                symbol: "DAI",
                address: "0x6B175474E89094C44Da98b954EedeAC495271d0F",
                decimals: 18,
            },
            Token {
                symbol: "WBTC",
                address: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599",
                decimals: 8,
            },
            Token {
                symbol: "SUSHI",
                address: "0x6B3595068778DD592e39A122f4f5a5cF09C90fE2",
                decimals: 18,
            },
        ],
    },
    Chain {
        id: 8453,
        name: "Base",
        tokens: &[
            Token { symbol: "ETH", address: NATIVE, decimals: 18 },
            Token {
                symbol: "WETH",
                address: "0x4200000000000000000000000000000000000006",
                decimals: 18,
            },
            Token {
                symbol: "USDC",
                address: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                decimals: 6,
            },
        ],
    },
    Chain {
        id: 42161,
        name: "Arbitrum",
        tokens: &[
            Token { symbol: "ETH", address: NATIVE, decimals: 18 },
            Token {
                symbol: "WETH",
                address: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
                decimals: 18,
            },
            Token {
                symbol: "USDC",
                address: "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                decimals: 6,
            },
            Token {
                symbol: "USDT",
                address: "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9",
                decimals: 6,
            },
            Token {
                symbol: "WBTC",
                address: "0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f",
                decimals: 8,
            },
        ],
    },
    Chain {
        id: 10,
        name: "Optimism",
        tokens: &[
            Token { symbol: "ETH", address: NATIVE, decimals: 18 },
            Token {
                symbol: "WETH",
                address: "0x4200000000000000000000000000000000000006",
                decimals: 18,
            },
            Token {
                symbol: "USDC",
                address: "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
                decimals: 6,
            },
        ],
    },
    Chain {
        id: 137,
        name: "Polygon",
        tokens: &[
            Token { symbol: "POL", address: NATIVE, decimals: 18 },
            Token {
                symbol: "WPOL",
                address: "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270",
                decimals: 18,
            },
            Token {
                symbol: "USDC",
                address: "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
                decimals: 6,
            },
            Token {
                symbol: "WETH",
                address: "0x7ceB23fD6bC0adD59E62ac25578270cFf1b9f619",
                decimals: 18,
            },
        ],
    },
];

pub fn chain_by_name(name: &str) -> Option<&'static Chain> {
    CHAINS.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim()))
}

impl Chain {
    pub fn token(&self, symbol: &str) -> Option<&'static Token> {
        let s = symbol.trim();
        self.tokens.iter().find(|t| t.symbol.eq_ignore_ascii_case(s))
    }

    pub fn symbols(&self) -> Vec<&'static str> {
        self.tokens.iter().map(|t| t.symbol).collect()
    }
}

/// Whitelisted symbols per chain, for the model's system prompt.
pub fn catalog() -> String {
    let mut out = String::new();
    for c in CHAINS {
        out.push_str(&format!("- {}: {}\n", c.name, c.symbols().join(", ")));
    }
    out
}

/// Parse a human decimal string into raw base units.
///
/// String-based on purpose: going through f64 would silently corrupt amounts
/// well within everyday range (0.1 ETH is not representable in binary float).
pub fn parse_units(s: &str, decimals: u32) -> Result<u128, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty amount".into());
    }
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
    let int_part = if int_part.is_empty() { "0" } else { int_part };
    if int_part.is_empty()
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!("`{s}` is not a decimal number"));
    }
    let d = decimals as usize;
    if frac_part.len() > d {
        return Err(format!("too many decimals: at most {d} for this token"));
    }

    let mut digits = String::with_capacity(int_part.len() + d);
    digits.push_str(int_part);
    digits.push_str(frac_part);
    for _ in frac_part.len()..d {
        digits.push('0');
    }

    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return Ok(0);
    }
    trimmed.parse::<u128>().map_err(|_| "amount out of range".to_string())
}

/// Render raw base units back as a human decimal string.
pub fn format_units(raw: u128, decimals: u32) -> String {
    let d = decimals as usize;
    if d == 0 {
        return raw.to_string();
    }
    let s = raw.to_string();
    let s = if s.len() <= d { format!("{}{}", "0".repeat(d - s.len() + 1), s) } else { s };
    let (int_part, frac_part) = s.split_at(s.len() - d);
    let frac = frac_part.trim_end_matches('0');
    if frac.is_empty() { int_part.to_string() } else { format!("{int_part}.{frac}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimals() {
        assert_eq!(parse_units("1", 18).unwrap(), 1_000_000_000_000_000_000);
        assert_eq!(parse_units("0.5", 18).unwrap(), 500_000_000_000_000_000);
        assert_eq!(parse_units("1.5", 6).unwrap(), 1_500_000);
        assert_eq!(parse_units(".5", 6).unwrap(), 500_000);
        assert_eq!(parse_units("0", 18).unwrap(), 0);
        assert_eq!(parse_units("0.000000", 6).unwrap(), 0);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_units("", 18).is_err());
        assert!(parse_units("abc", 18).is_err());
        assert!(parse_units("1.2.3", 18).is_err());
        assert!(parse_units("-1", 18).is_err());
        // more precision than the token can hold: refuse rather than truncate
        assert!(parse_units("1.1234567", 6).is_err());
    }

    #[test]
    fn formats_round_trip() {
        assert_eq!(format_units(1_000_000_000_000_000_000, 18), "1");
        assert_eq!(format_units(500_000_000_000_000_000, 18), "0.5");
        assert_eq!(format_units(1_500_000, 6), "1.5");
        assert_eq!(format_units(0, 18), "0");
        assert_eq!(format_units(1, 18), "0.000000000000000001");
    }

    #[test]
    fn every_token_address_is_well_formed() {
        for c in CHAINS {
            for t in c.tokens {
                assert!(t.address.starts_with("0x"), "{} {}", c.name, t.symbol);
                assert_eq!(t.address.len(), 42, "{} {}", c.name, t.symbol);
                assert!(
                    t.address[2..].bytes().all(|b| b.is_ascii_hexdigit()),
                    "{} {}",
                    c.name,
                    t.symbol
                );
            }
        }
    }
}
