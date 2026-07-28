//! ABI encoding/decoding for the bonding-curve launch feature — a new token
//! whose bonding curve is denominated in a Robinhood-Chain stock token
//! instead of ETH/USDC (the `longdotxyz` idea), targeting Robinhood Chain
//! **testnet** only (see `tokens.rs`'s "Robinhood Testnet" chain entry).
//!
//! All curve pricing math lives in the Solidity contracts under
//! `contracts/` — this module never recomputes a price from raw reserves,
//! it only ever encodes a call and decodes the single `uint256`/`bool`/
//! `address` result the contract already computed. That's what keeps
//! everything here safe in `u128`: a realistic 18-decimal token amount never
//! approaches `u128::MAX`, and there's no multiplication of two large
//! reserve values happening on the Rust side to overflow it.
//!
//! Selectors below are `keccak256(signature)[..4]`, computed once against the
//! exact signatures in `contracts/src/BondingCurve.sol` and
//! `contracts/src/BondingCurveFactory.sol` — fixed facts, not recomputed at
//! runtime, same reasoning as `erc20.rs`'s selectors.

use std::path::Path;

use super::erc20::{self, pad_address, pad_u128};

/// Settings for the bonding-curve launch feature. Not secrets — a factory
/// address and an on/off toggle are as sensitive as an RPC URL — so this
/// follows `ftp.rs`'s plain-flat-file pattern rather than `secret.rs`'s
/// DPAPI-sealed one. Testnet-only for now: there's no mainnet factory to
/// point at yet, and `use_testnet` gates whether the launch/buy tools are
/// offered to the model at all.
#[derive(Clone, Default)]
pub struct BondingConfig {
    pub factory_address: String,
    pub use_testnet: bool,
}

pub fn load_config(cfg: &Path) -> BondingConfig {
    let read = |n: &str| std::fs::read_to_string(cfg.join(n)).unwrap_or_default().trim().to_string();
    BondingConfig {
        factory_address: read("bonding_factory.txt"),
        use_testnet: read("bonding_use_testnet.txt") == "1",
    }
}

pub fn save_config(cfg: &Path, c: &BondingConfig) {
    let w = |n: &str, v: &str| {
        let _ = std::fs::write(cfg.join(n), v.trim());
    };
    w("bonding_factory.txt", &c.factory_address);
    w("bonding_use_testnet.txt", if c.use_testnet { "1" } else { "0" });
}

/// ABI-encodes a dynamic `string` as it appears in the calldata "tail": a
/// 32-byte length prefix followed by the UTF-8 bytes, right-padded with zero
/// bytes to a multiple of 32.
fn encode_dynamic_string(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = format!("{:064x}", bytes.len());
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    let padded_len = bytes.len().div_ceil(32) * 32;
    for _ in bytes.len()..padded_len {
        out.push_str("00");
    }
    out
}

/// Calldata for `launch(string name, string symbol, address pairedStockToken, uint256 graduationThreshold)`.
///
/// Head is 4 fixed 32-byte slots (one per parameter, in declaration order):
/// the two dynamic `string`s get a byte-offset into the tail section, the
/// `address` and `uint256` are inlined directly, matching standard Solidity
/// ABI head/tail encoding.
pub fn encode_launch(
    name: &str,
    symbol: &str,
    paired_stock_token: &str,
    graduation_threshold: u128,
) -> Result<String, String> {
    let addr_word = pad_address(paired_stock_token)?;
    let threshold_word = pad_u128(graduation_threshold);
    let name_tail = encode_dynamic_string(name);
    let symbol_tail = encode_dynamic_string(symbol);

    const HEAD_LEN_BYTES: usize = 4 * 32;
    let name_offset = HEAD_LEN_BYTES;
    let symbol_offset = HEAD_LEN_BYTES + name_tail.len() / 2;

    let mut out = String::from("0x940390c2");
    out.push_str(&format!("{name_offset:064x}"));
    out.push_str(&format!("{symbol_offset:064x}"));
    out.push_str(&addr_word);
    out.push_str(&threshold_word);
    out.push_str(&name_tail);
    out.push_str(&symbol_tail);
    Ok(out)
}

/// Calldata for `buy(uint256 stockIn, uint256 minTokensOut)`.
pub fn encode_buy(stock_in: u128, min_tokens_out: u128) -> String {
    format!("0xd6febde8{}{}", pad_u128(stock_in), pad_u128(min_tokens_out))
}

/// Calldata for `sell(uint256 tokensIn, uint256 minStockOut)`.
pub fn encode_sell(tokens_in: u128, min_stock_out: u128) -> String {
    format!("0xd79875eb{}{}", pad_u128(tokens_in), pad_u128(min_stock_out))
}

/// Calldata for `previewBuy(uint256 stockIn)` — an `eth_call`, not a transaction.
pub fn encode_preview_buy(stock_in: u128) -> String {
    format!("0x48153279{}", pad_u128(stock_in))
}

/// Calldata for `previewSell(uint256 tokensIn)` — an `eth_call`, not a transaction.
pub fn encode_preview_sell(tokens_in: u128) -> String {
    format!("0xfb3dd95f{}", pad_u128(tokens_in))
}

/// Calldata for `raised()` — no arguments, just the selector.
pub const RAISED_CALL: &str = "0xf0ea4bfc";
/// Calldata for `graduationThreshold()`.
pub const GRADUATION_THRESHOLD_CALL: &str = "0x8b0bc501";
/// Calldata for `graduated()`.
pub const GRADUATED_CALL: &str = "0xe7c2b772";
/// Calldata for `pool()`.
pub const POOL_CALL: &str = "0x16f0115b";

/// Calldata for the factory's `curveOf(address token)`.
pub fn encode_curve_of(token: &str) -> Result<String, String> {
    Ok(format!("0x05adc47e{}", pad_address(token)?))
}

/// Calldata for the curve's `token()` — no arguments, just the selector.
pub const TOKEN_CALL: &str = "0xfc0c546a";
/// Calldata for the curve's `stockToken()`.
pub const STOCK_TOKEN_CALL: &str = "0xbaa8e786";
/// Calldata for the factory's `allCurves()`.
pub const ALL_CURVES_CALL: &str = "0xc69cfa15";
/// Calldata for the factory's `launchFee()` — read before every `launch` so the
/// transaction's native-currency `value` matches exactly what the contract requires.
pub const LAUNCH_FEE_CALL: &str = "0xcf3cf573";
/// Calldata for the ERC-20 standard `symbol()`.
pub const SYMBOL_CALL: &str = "0x95d89b41";
/// Calldata for the ERC-20 standard `name()`.
pub const NAME_CALL: &str = "0x06fdde03";

/// Decodes a dynamic `address[]` return value: a 32-byte offset (always `0x20`
/// for a single top-level return value), a 32-byte length, then one
/// right-aligned address word per entry.
pub fn decode_address_array(hex: &str) -> Vec<String> {
    let h = hex.trim().trim_start_matches("0x");
    let words: Vec<&str> = h.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    if words.len() < 2 {
        return Vec::new();
    }
    let len = usize::from_str_radix(words[1], 16).unwrap_or(0);
    words.iter().skip(2).take(len).map(|w| decode_address(&format!("0x{w}"))).collect()
}

/// Decodes a dynamic `string` return value: a 32-byte offset, a 32-byte byte
/// length, then the UTF-8 bytes padded to a multiple of 32. Lossy on non-UTF-8
/// bytes rather than erroring — a display-only token name/symbol is never
/// worth failing an entire tool call over.
pub fn decode_string(hex: &str) -> String {
    let h = hex.trim().trim_start_matches("0x");
    if h.len() < 128 {
        return String::new();
    }
    let len_bytes = usize::from_str_radix(&h[64..128], 16).unwrap_or(0);
    let data_hex_len = len_bytes * 2;
    let data = h.get(128..128 + data_hex_len).unwrap_or("");
    let bytes = hex::decode(data).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Decodes a `uint256` return value. Delegates to `erc20::decode_uint256`
/// rather than re-implementing it: same saturating-at-`u128::MAX` behavior,
/// which is fine here since curve prices/amounts never approach that ceiling.
pub fn decode_uint256(hex: &str) -> u128 {
    erc20::decode_uint256(hex)
}

/// Decodes a `bool` return value — solc encodes it as a full 32-byte word,
/// nonzero meaning `true`.
pub fn decode_bool(hex: &str) -> bool {
    erc20::decode_uint256(hex) != 0
}

/// Decodes an `address` return value — the low 20 bytes of the returned
/// 32-byte word.
pub fn decode_address(hex: &str) -> String {
    let h = hex.trim().trim_start_matches("0x");
    let tail = if h.len() >= 40 { &h[h.len() - 40..] } else { h };
    format!("0x{tail:0>40}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "0x89af42c369534412a250e6631309237d73d91807";

    #[test]
    fn launch_selector_and_static_words() {
        let c = encode_launch("Rocket Coin", "ROCKET", STOCK, 1_000_000).unwrap();
        assert!(c.starts_with("0x940390c2"));
        let body = &c[10..]; // strip "0x" + 8 hex chars of selector
        // word 0: name offset, always 0x80 (128) — 4 head slots * 32 bytes
        assert_eq!(&body[0..64], &format!("{:064x}", 128));
        // word 2 (after the two offsets): the address, right-aligned
        let addr_word = &body[128..192];
        assert!(addr_word.ends_with(&STOCK[2..]));
        // word 3: the graduation threshold
        let threshold_word = &body[192..256];
        assert_eq!(threshold_word, &format!("{:064x}", 1_000_000u128));
    }

    #[test]
    fn launch_string_tails_round_trip() {
        let c = encode_launch("AB", "C", STOCK, 0).unwrap();
        let body = &c[10..];
        let name_offset = usize::from_str_radix(&body[0..64], 16).unwrap();
        let symbol_offset = usize::from_str_radix(&body[64..128], 16).unwrap();
        assert_eq!(name_offset, 128);
        // "AB" is 2 bytes, padded to one 32-byte word -> tail is 32 (len) + 32 (data) = 64 bytes
        assert_eq!(symbol_offset, 128 + 64);

        let name_tail = &body[name_offset * 2..symbol_offset * 2];
        let name_len = usize::from_str_radix(&name_tail[0..64], 16).unwrap();
        assert_eq!(name_len, 2);
        let name_bytes = hex::decode(&name_tail[64..64 + name_len * 2]).unwrap();
        assert_eq!(name_bytes, b"AB");

        let symbol_tail = &body[symbol_offset * 2..];
        let symbol_len = usize::from_str_radix(&symbol_tail[0..64], 16).unwrap();
        assert_eq!(symbol_len, 1);
        let symbol_bytes = hex::decode(&symbol_tail[64..64 + symbol_len * 2]).unwrap();
        assert_eq!(symbol_bytes, b"C");
    }

    #[test]
    fn buy_sell_preview_selectors_and_padding() {
        let b = encode_buy(1, 2);
        assert!(b.starts_with("0xd6febde8"));
        assert_eq!(b.len(), 2 + 8 + 64 + 64);

        let s = encode_sell(3, 4);
        assert!(s.starts_with("0xd79875eb"));

        let pb = encode_preview_buy(5);
        assert!(pb.starts_with("0x48153279"));
        assert_eq!(pb.len(), 2 + 8 + 64);

        let ps = encode_preview_sell(6);
        assert!(ps.starts_with("0xfb3dd95f"));
    }

    #[test]
    fn curve_of_selector_and_padding() {
        let c = encode_curve_of(STOCK).unwrap();
        assert!(c.starts_with("0x05adc47e"));
        assert!(c.ends_with(&STOCK[2..]));
    }

    #[test]
    fn decodes_bool_and_address() {
        let true_word = format!("0x{:064x}", 1);
        let false_word = format!("0x{:064x}", 0);
        assert!(decode_bool(&true_word));
        assert!(!decode_bool(&false_word));

        let addr_word = format!("0x{:0>64}", &STOCK[2..]);
        assert_eq!(decode_address(&addr_word), STOCK);
    }

    #[test]
    fn decodes_an_address_array() {
        // offset (0x20) + length (2) + two right-aligned address words
        let hex = format!("0x{:064x}{:064x}{:0>64}{:0>64}", 32, 2, "1", &STOCK[2..]);
        let addrs = decode_address_array(&hex);
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], "0x0000000000000000000000000000000000000001");
        assert_eq!(addrs[1], STOCK);
    }

    #[test]
    fn decodes_an_empty_address_array() {
        let hex = format!("0x{:064x}{:064x}", 32, 0);
        assert!(decode_address_array(&hex).is_empty());
    }

    #[test]
    fn decodes_a_string() {
        // offset (0x20) + length (6) + "ROCKET" padded to 32 bytes
        let mut data_hex = hex::encode(b"ROCKET");
        while data_hex.len() < 64 {
            data_hex.push('0');
        }
        let hex = format!("0x{:064x}{:064x}{data_hex}", 32, 6);
        assert_eq!(decode_string(&hex), "ROCKET");
    }

    #[test]
    fn decodes_an_empty_string() {
        let hex = format!("0x{:064x}{:064x}", 32, 0);
        assert_eq!(decode_string(&hex), "");
    }
}
