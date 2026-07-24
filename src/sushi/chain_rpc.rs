//! Direct JSON-RPC against the chain itself — everything the local signer
//! path needs that Frame would otherwise have handled internally: the
//! sender's nonce, current gas fees, a gas estimate, and broadcasting the
//! already-signed transaction. Nothing in this file ever sees a private
//! key; `signer.rs` hands it a finished, signed raw transaction and nothing
//! else.

use std::time::Duration;

use serde_json::{Value, json};

fn rpc_url(chain_id: u64) -> Result<&'static str, String> {
    match chain_id {
        4663 => Ok("https://rpc.mainnet.chain.robinhood.com"),
        other => Err(format!("no RPC endpoint configured for chain {other}")),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
}

fn call(chain_id: u64, method: &str, params: Value) -> Result<Value, String> {
    let url = rpc_url(chain_id)?;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp =
        agent().post(url).set("Content-Type", "application/json").send_json(body).map_err(
            |e| match e {
                ureq::Error::Status(_, r) => {
                    let v: Value = r.into_json().unwrap_or(Value::Null);
                    v["error"]["message"].as_str().unwrap_or("chain RPC request failed").to_string()
                }
                ureq::Error::Transport(_) => "couldn't reach the chain RPC endpoint".to_string(),
            },
        )?;
    let v: Value = resp.into_json().map_err(|e| format!("bad RPC response: {e}"))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        return Err(err["message"].as_str().unwrap_or("chain RPC returned an error").to_string());
    }
    Ok(v["result"].clone())
}

fn hex_to_u64(v: &Value, what: &str) -> Result<u64, String> {
    let s = v.as_str().ok_or_else(|| format!("unexpected {what} response"))?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|_| format!("unparseable {what} response: {s}"))
}

fn hex_to_u128(v: &Value, what: &str) -> Result<u128, String> {
    let s = v.as_str().ok_or_else(|| format!("unexpected {what} response"))?;
    u128::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|_| format!("unparseable {what} response: {s}"))
}

/// A read call — same shape as `wallet::call`, but against the chain
/// directly rather than through Frame. Used for the allowance check.
pub fn eth_call(chain_id: u64, to: &str, data: &str) -> Result<String, String> {
    let v = call(chain_id, "eth_call", json!([{ "to": to, "data": data }, "latest"]))?;
    v.as_str().map(str::to_string).ok_or_else(|| "unexpected eth_call response".to_string())
}

pub fn nonce(chain_id: u64, address: &str) -> Result<u64, String> {
    let v = call(chain_id, "eth_getTransactionCount", json!([address, "pending"]))?;
    hex_to_u64(&v, "nonce")
}

/// The chain's native currency balance (ETH on Robinhood Chain) — a
/// separate RPC method from `eth_call`, since it isn't a contract read.
pub fn native_balance(chain_id: u64, address: &str) -> Result<u128, String> {
    let v = call(chain_id, "eth_getBalance", json!([address, "latest"]))?;
    hex_to_u128(&v, "balance")
}

pub struct Fees {
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
}

/// EIP-1559 fee estimate: base fee from the latest block, doubled for
/// headroom against a rising base fee before the tx is mined, plus a
/// priority fee from the node's own suggestion (or a 1.5 gwei fallback on a
/// node that doesn't implement `eth_maxPriorityFeePerGas`).
pub fn fees(chain_id: u64) -> Result<Fees, String> {
    let block = call(chain_id, "eth_getBlockByNumber", json!(["latest", false]))?;
    let base_fee = hex_to_u128(&block["baseFeePerGas"], "baseFeePerGas")
        .map_err(|_| "chain doesn't report an EIP-1559 base fee".to_string())?;
    let priority = call(chain_id, "eth_maxPriorityFeePerGas", json!([]))
        .and_then(|v| hex_to_u128(&v, "maxPriorityFeePerGas"))
        .unwrap_or(1_500_000_000);
    Ok(Fees { max_fee_per_gas: base_fee * 2 + priority, max_priority_fee_per_gas: priority })
}

/// The node's own simulation, plus 20% headroom — estimates can undershoot
/// on state-dependent paths (e.g. a storage slot the simulation saw cold
/// that won't be by the time the real transaction lands).
pub fn estimate_gas(chain_id: u64, from: &str, to: &str, data: &str, value: u128) -> Result<u64, String> {
    let v = call(
        chain_id,
        "eth_estimateGas",
        json!([{ "from": from, "to": to, "data": data, "value": format!("0x{value:x}") }]),
    )?;
    let gas = hex_to_u64(&v, "eth_estimateGas")?;
    Ok(gas + gas / 5)
}

pub fn send_raw_transaction(chain_id: u64, raw_hex: &str) -> Result<String, String> {
    let v = call(chain_id, "eth_sendRawTransaction", json!([raw_hex]))?;
    v.as_str().map(str::to_string).ok_or_else(|| "unexpected eth_sendRawTransaction response".to_string())
}

pub fn wait_for_receipt(chain_id: u64, tx_hash: &str, timeout: Duration) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let v = call(chain_id, "eth_getTransactionReceipt", json!([tx_hash]))?;
        if !v.is_null() {
            return match v["status"].as_str() {
                Some("0x1") => Ok(()),
                Some("0x0") => Err("the transaction reverted".into()),
                _ => Ok(()), // some nodes omit status; presence of a receipt is enough
            };
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "transaction is taking longer than expected — check the explorer".into()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
