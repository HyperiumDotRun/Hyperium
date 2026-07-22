//! A wallet Hyperium can talk to without ever holding a key: Frame
//! (frame.sh), a desktop signer that listens on `127.0.0.1:1248` and speaks
//! plain JSON-RPC 2.0 over HTTP — verified against its own `eth-provider`
//! connector source, not guessed. Every request this module sends is either a
//! read (`eth_call`, `eth_chainId`) or a *request to sign* (`eth_sendTransaction`);
//! the transaction itself is built, shown and confirmed inside Frame's own
//! window, which is the boundary that keeps a bug here from being able to
//! move money on its own.
//!
//! This is not WalletConnect. A dApp-side WalletConnect v2 implementation in
//! Rust does not exist in a form this build could respectably rely on — see
//! the plan this module was built from. Frame was chosen because its contract
//! is small, stable, and could be read straight from source rather than taken
//! on faith.

use std::time::Duration;

use serde_json::{Value, json};

const ENDPOINT: &str = "http://127.0.0.1:1248";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(120)) // eth_sendTransaction blocks on the user
        .build()
}

fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp = agent()
        .post(ENDPOINT)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| match e {
            ureq::Error::Status(_, r) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                v["error"]["message"].as_str().unwrap_or("Frame request failed").to_string()
            }
            ureq::Error::Transport(_) => {
                "Frame isn't running — install it from frame.sh and open it".to_string()
            }
        })?;
    let v: Value = resp.into_json().map_err(|e| format!("bad response from Frame: {e}"))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        return Err(err["message"].as_str().unwrap_or("Frame returned an error").to_string());
    }
    Ok(v["result"].clone())
}

/// The connected account. Frame prompts the user on first request per
/// session; later calls return instantly.
pub fn accounts() -> Result<String, String> {
    let v = rpc("eth_requestAccounts", json!([]))?;
    v.as_array()
        .and_then(|a| a.first())
        .and_then(|a| a.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Frame returned no account".to_string())
}

/// A read against whatever chain Frame currently has selected — used for the
/// allowance check before a swap. `chain_id` rides on the payload per Frame's
/// own convention; Frame still needs that chain added on its side, same as
/// any wallet.
pub fn call(chain_id: u64, to: &str, data: &str) -> Result<String, String> {
    let v = rpc(
        "eth_call",
        json!([{ "to": to, "data": data }, "latest", { "chainId": format!("0x{chain_id:x}") }]),
    )?;
    v.as_str().map(str::to_string).ok_or_else(|| "unexpected eth_call response".to_string())
}

pub struct TxRequest {
    pub chain_id: u64,
    pub from: String,
    pub to: String,
    pub data: String,
    /// Base units, hex-encoded by this function — callers pass a plain u128.
    pub value: u128,
}

/// Sends a signing *request*. The transaction is composed here but signed and
/// broadcast by Frame after the user approves it in Frame's own window — this
/// call blocks until they do, or until they don't and it errors instead.
/// Returns the transaction hash on success.
pub fn send_transaction(req: &TxRequest) -> Result<String, String> {
    let v = rpc(
        "eth_sendTransaction",
        json!([{
            "from": req.from,
            "to": req.to,
            "data": req.data,
            "value": format!("0x{:x}", req.value),
            "chainId": format!("0x{:x}", req.chain_id),
        }]),
    )?;
    v.as_str().map(str::to_string).ok_or_else(|| "unexpected eth_sendTransaction response".to_string())
}

/// A broadcast tx hash is not yet a fact on chain — an approval has to be
/// *mined* before the allowance it set is real, and the swap that follows it
/// would otherwise revert against a still-pending approval. Polls
/// `eth_getTransactionReceipt` until one appears with `status: 0x1`, or until
/// `timeout` runs out.
pub fn wait_for_receipt(chain_id: u64, tx_hash: &str, timeout: Duration) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let v = rpc(
            "eth_getTransactionReceipt",
            json!([tx_hash, { "chainId": format!("0x{chain_id:x}") }]),
        )?;
        if !v.is_null() {
            return match v["status"].as_str() {
                Some("0x1") => Ok(()),
                Some("0x0") => Err("the approval transaction reverted".into()),
                _ => Ok(()), // some nodes omit status; presence of a receipt is enough
            };
        }
        if std::time::Instant::now() >= deadline {
            return Err("approval is taking longer than expected — check Frame or the \
                         explorer, then try the swap again"
                .into());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
