//! Just enough ABI encoding to check and raise an ERC-20 allowance.
//!
//! Two selectors, both fixed by the standard: `allowance(address,address)` is
//! `0xdd62ed3e`, `approve(address,uint256)` is `0x095ea7b3`. Pulling in an ABI
//! crate for two four-byte constants would be a dependency for a fact that
//! never changes.

/// Left-pads a 20-byte address into a 32-byte ABI word.
fn pad_address(addr: &str) -> Result<String, String> {
    let a = addr.trim().trim_start_matches("0x");
    if a.len() != 40 || !a.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("not a valid address: {addr}"));
    }
    Ok(format!("{:0>64}", a.to_ascii_lowercase()))
}

fn pad_u128(v: u128) -> String {
    format!("{v:064x}")
}

/// Calldata for `allowance(owner, spender)` — an `eth_call`, not a transaction.
pub fn encode_allowance(owner: &str, spender: &str) -> Result<String, String> {
    Ok(format!("0xdd62ed3e{}{}", pad_address(owner)?, pad_address(spender)?))
}

/// Calldata for `approve(spender, amount)`.
pub fn encode_approve(spender: &str, amount: u128) -> Result<String, String> {
    Ok(format!("0x095ea7b3{}{}", pad_address(spender)?, pad_u128(amount)))
}

/// An `eth_call` return value is a 32-byte word, possibly with a `0x` prefix
/// and possibly short (some nodes trim leading zeros). Saturating rather than
/// erroring on overflow: an allowance bigger than `u128::MAX` still just
/// means "enough", which is all this value is ever compared against.
pub fn decode_uint256(hex: &str) -> u128 {
    let h = hex.trim().trim_start_matches("0x");
    if h.is_empty() {
        return 0;
    }
    // Only the low 32 hex chars (16 bytes) can matter to a u128 — anything
    // before that would already have overflowed, so treat it as saturated
    // rather than reading a truncated, wrong low value.
    let tail = if h.len() > 32 {
        if h[..h.len() - 32].bytes().any(|b| b != b'0') {
            return u128::MAX;
        }
        &h[h.len() - 32..]
    } else {
        h
    };
    u128::from_str_radix(tail, 16).unwrap_or(u128::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "0x0bd7d308f8e1639fab988df18a8011f41eacad73";
    const SPENDER: &str = "0xd0601ce157db5bdc3162bbac2a2c8af5320d9eec";

    #[test]
    fn allowance_selector_and_padding() {
        let c = encode_allowance(OWNER, SPENDER).unwrap();
        assert!(c.starts_with("0xdd62ed3e"));
        assert_eq!(c.len(), 2 + 8 + 64 + 64);
        assert!(c.ends_with("d0601ce157db5bdc3162bbac2a2c8af5320d9eec"));
    }

    #[test]
    fn approve_selector_and_amount() {
        let c = encode_approve(SPENDER, 1).unwrap();
        assert!(c.starts_with("0x095ea7b3"));
        assert!(c.ends_with(&format!("{}1", "0".repeat(63))));
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(encode_allowance("not-an-address", SPENDER).is_err());
        assert!(encode_allowance(OWNER, "0x1234").is_err());
    }

    #[test]
    fn decodes_plain_and_prefixed_and_short_words() {
        assert_eq!(decode_uint256("0x0000000000000000000000000000000000000000000000000000000000000001"), 1);
        assert_eq!(decode_uint256(&format!("0x{:064x}", 500_000u128)), 500_000);
        assert_eq!(decode_uint256("0x"), 0);
        assert_eq!(decode_uint256(""), 0);
    }

    #[test]
    fn saturates_rather_than_panics_on_a_huge_allowance() {
        // "infinite approval" — every high bit set.
        let max_approval = "f".repeat(64);
        assert_eq!(decode_uint256(&max_approval), u128::MAX);
    }
}
