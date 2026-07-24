//! Local wallet signer — the alternative to Frame (`wallet.rs`) for anyone
//! who doesn't want a second app running. The private key lives encrypted at
//! rest in the same DPAPI-backed store as every other secret
//! (`crate::secret`), and this file is the only place that ever puts it back
//! into plaintext memory, for exactly as long as it takes to sign one
//! transaction — never on the same thread that talks to the network or
//! draws the UI.
//!
//! This trades Frame's separate-app, separate-codebase confirmation window
//! for an in-app one built in `mod.rs`: the amount, recipient, and gas are
//! computed and shown *before* this module is ever asked to sign anything.
//! What this file guarantees is narrower, and deliberately so — it only
//! signs exactly the transaction it's handed, and every integer it encodes
//! is checked against a known-good reference vector in the tests below
//! rather than trusted from memory.

use std::path::{Path, PathBuf};

use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use zeroize::Zeroize;

fn key_path(cfg: &Path) -> PathBuf {
    cfg.join("wallet.key")
}
fn address_path(cfg: &Path) -> PathBuf {
    cfg.join("wallet_address.txt")
}

pub fn has_key(cfg: &Path) -> bool {
    key_path(cfg).exists()
}

/// The address is not secret — it's read straight from disk in the clear so
/// nothing needs decrypting just to display it or use it as `sender`.
pub fn address(cfg: &Path) -> Option<String> {
    let s = std::fs::read_to_string(address_path(cfg)).ok()?;
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}

pub fn clear_key(cfg: &Path) {
    let _ = std::fs::remove_file(key_path(cfg));
    let _ = std::fs::remove_file(address_path(cfg));
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn strip0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

/// secp256k1 pubkey -> Ethereum address: keccak256 of the 64-byte
/// uncompressed point (X||Y, no `0x04` prefix), last 20 bytes.
fn derive_address(signing_key: &SigningKey) -> String {
    let point = signing_key.verifying_key().to_encoded_point(false);
    let hash = keccak256(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Validates the key, derives its address, saves the key encrypted at rest
/// and the address in the clear. Returns the derived address so the caller
/// can show it back immediately (e.g. "imported 0xabc…" ) for the user to
/// check it's the account they meant to import.
pub fn set_key(cfg: &Path, hex_key: &str) -> Result<String, String> {
    let clean = strip0x(hex_key.trim());
    let mut bytes = hex::decode(clean).map_err(|_| "not valid hex".to_string())?;
    let key = SigningKey::from_slice(&bytes).map_err(|_| "not a valid private key".to_string());
    bytes.zeroize();
    let key = key?;
    let addr = derive_address(&key);
    crate::secret::save_secret(&key_path(cfg), clean);
    std::fs::write(address_path(cfg), &addr).map_err(|e| e.to_string())?;
    Ok(addr)
}

/// One transaction, ready to sign — chain id, nonce, fees and gas are all
/// gathered by the caller (network calls, not this module's job) so the
/// worker process this gets handed to only ever has to sign, never reach
/// the network itself.
#[derive(Serialize, Deserialize, Clone)]
pub struct UnsignedTx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    /// `0x`-prefixed 20-byte address.
    pub to: String,
    pub value: u128,
    /// `0x`-prefixed calldata, empty (`"0x"`) for a plain value transfer.
    pub data: String,
}

/// RLP's integer convention: minimal big-endian bytes, zero as an empty
/// string. Used for every scalar field in the transaction, including `r`
/// and `s` — a signature is just two more RLP integers, not a fixed-width
/// byte string.
fn rlp_uint(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    bytes[start..].to_vec()
}

fn rlp_u128(n: u128) -> Vec<u8> {
    rlp_uint(&n.to_be_bytes())
}

/// The EIP-1559 (type 2) payload: `0x02 || rlp([...])`, either the 9-field
/// unsigned form (what gets hashed and signed) or the 12-field signed form
/// (what actually gets broadcast), depending on whether `sig` is given.
fn encode(tx: &UnsignedTx, sig: Option<(u8, &[u8], &[u8])>) -> Vec<u8> {
    let to = hex::decode(strip0x(&tx.to)).unwrap_or_default();
    let data = hex::decode(strip0x(&tx.data)).unwrap_or_default();

    let mut s = rlp::RlpStream::new_list(if sig.is_some() { 12 } else { 9 });
    s.append(&rlp_u128(tx.chain_id as u128));
    s.append(&rlp_u128(tx.nonce as u128));
    s.append(&rlp_u128(tx.max_priority_fee_per_gas));
    s.append(&rlp_u128(tx.max_fee_per_gas));
    s.append(&rlp_u128(tx.gas_limit as u128));
    s.append(&to);
    s.append(&rlp_u128(tx.value));
    s.append(&data);
    s.begin_list(0); // access list — always empty, this signer doesn't build one
    if let Some((y_parity, r, sv)) = sig {
        s.append(&rlp_uint(&[y_parity]));
        s.append(&rlp_uint(r));
        s.append(&rlp_uint(sv));
    }

    let mut out = vec![0x02u8];
    out.extend_from_slice(&s.out());
    out
}

fn sign_hash(key: &SigningKey, hash: &[u8; 32]) -> (u8, [u8; 32], [u8; 32]) {
    let (sig, rec): (Signature, RecoveryId) =
        key.sign_prehash(hash).expect("signing a 32-byte hash cannot fail");
    let r: [u8; 32] = sig.r().to_bytes().into();
    let sv: [u8; 32] = sig.s().to_bytes().into();
    (rec.to_byte(), r, sv)
}

/// Signs `tx` with the key stored for this config dir. Returns the raw
/// signed transaction as `0x`-prefixed hex, ready for `eth_sendRawTransaction`
/// — nothing here ever touches the network.
pub fn sign(cfg: &Path, tx: &UnsignedTx) -> Result<String, String> {
    let hex_key = crate::secret::load_secret(&key_path(cfg));
    if hex_key.is_empty() {
        return Err("no local wallet key configured".into());
    }
    let mut bytes = hex::decode(&hex_key).map_err(|_| "stored key is corrupt".to_string())?;
    let key = SigningKey::from_slice(&bytes).map_err(|_| "stored key is corrupt".to_string());
    bytes.zeroize();
    let key = key?;

    let unsigned = encode(tx, None);
    let hash = keccak256(&unsigned);
    let (y_parity, r, sv) = sign_hash(&key, &hash);
    let signed = encode(tx, Some((y_parity, &r, &sv)));
    Ok(format!("0x{}", hex::encode(signed)))
}

/// Parent-process side of the same operation: relaunches this very
/// executable as `hyperium --sign-worker`, hands it `tx` over its stdin, and
/// reads the signed raw tx back from its stdout. This process's own memory
/// never holds the key — only the short-lived child's does, for exactly the
/// span of `run_worker` below.
pub fn sign_via_worker(tx: &UnsignedTx) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().map_err(|e| format!("can't find own executable: {e}"))?;
    let mut child = Command::new(exe)
        .arg("--sign-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start the signing process: {e}"))?;

    let payload = serde_json::to_vec(tx).map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin on signing process")?
        .write_all(&payload)
        .map_err(|e| format!("couldn't hand the transaction to the signing process: {e}"))?;

    let output =
        child.wait_with_output().map_err(|e| format!("signing process failed: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("signing failed: {}", err.trim()));
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err("signing process returned nothing".into());
    }
    Ok(raw)
}

/// Entry point when Hyperium is relaunched as `hyperium --sign-worker`: reads
/// one `UnsignedTx` as JSON from stdin, signs it against the key stored in
/// the given config dir, writes the signed raw tx hex to stdout, and exits.
/// A whole separate process for one signature, so the plaintext key's
/// lifetime is bounded by this process's — it cannot outlive it, and it
/// never exists in the long-running app process at all.
pub fn run_worker(cfg: &Path) -> i32 {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("sign-worker: failed to read stdin");
        return 1;
    }
    let tx: UnsignedTx = match serde_json::from_str(&input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("sign-worker: bad tx json: {e}");
            return 1;
        }
    };
    match sign(cfg, &tx) {
        Ok(raw) => {
            println!("{raw}");
            0
        }
        Err(e) => {
            eprintln!("sign-worker: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checked byte-for-byte against ethers.js v6 (`Transaction.from`
    /// + `Wallet.signTransaction`) with the same key and the same fields —
    /// this is not a self-consistency check, it's agreement with an
    /// independent, widely-used implementation of the real protocol.
    /// Key is Hardhat/Anvil's public default test account #0: well-known,
    /// never holds real funds, used only as a reference vector here.
    #[test]
    fn matches_ethers_js_reference_vector() {
        let key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let key = SigningKey::from_slice(&hex::decode(key_hex).unwrap()).unwrap();
        assert_eq!(derive_address(&key), "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");

        let tx = UnsignedTx {
            chain_id: 4663,
            nonce: 5,
            max_priority_fee_per_gas: 1_500_000_000,
            max_fee_per_gas: 30_000_000_000,
            gas_limit: 100_000,
            to: "0x0bd7d308f8e1639fab988df18a8011f41eacad73".to_string(),
            value: 1_000_000_000_000_000_000,
            data: "0xa9059cbb000000000000000000000000000000000000000000000000000000000000dead0000000000000000000000000000000000000000000000000de0b6b3a7640000".to_string(),
        };

        let unsigned = encode(&tx, None);
        let hash = keccak256(&unsigned);
        let (y_parity, r, sv) = sign_hash(&key, &hash);
        let signed = encode(&tx, Some((y_parity, &r, &sv)));
        let got = format!("0x{}", hex::encode(signed));

        let want = "0x02f8bb821237058459682f008506fc23ac00830186a0940bd7d308f8e1639fab988df18a8011f41eacad73880de0b6b3a7640000b844a9059cbb000000000000000000000000000000000000000000000000000000000000dead0000000000000000000000000000000000000000000000000de0b6b3a7640000c001a0448b4d64b00b1b26357e66f9fadce6c6b3609988edff9996dcd3829dacc8c20aa057973297792c1e85f577e4b3da661795cb31f9b2d06966bbd60f8da158461bb4";

        assert_eq!(got, want, "signed tx does not match the ethers.js reference vector");
    }

    #[test]
    fn rlp_uint_strips_leading_zeros_and_encodes_zero_as_empty() {
        assert_eq!(rlp_u128(0), Vec::<u8>::new());
        assert_eq!(rlp_u128(1), vec![1u8]);
        assert_eq!(rlp_u128(256), vec![1u8, 0u8]);
    }
}
