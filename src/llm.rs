use std::time::Duration;

pub trait LlmProvider: Send + Sync {
    fn complete(&self, system: &str, user: &str) -> Result<String, String>;
}

pub struct Anthropic {
    api_key: String,
    model: String,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: "claude-haiku-4-5".to_string() }
    }
}

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_ATTEMPTS: u32 = 3;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .build()
}

fn is_retryable(code: u16) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504 | 529)
}

fn parse_response(resp: ureq::Response) -> Result<String, String> {
    let v: serde_json::Value =
        resp.into_json().map_err(|e| format!("bad response: {e}"))?;
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<String>()
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err("empty response from the model".into());
    }
    Ok(text)
}

impl LlmProvider for Anthropic {
    fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        if self.api_key.trim().is_empty() {
            return Err("no Anthropic API key - set it in Settings → AI".into());
        }
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });

        let ag = agent();
        let mut last_err = String::new();
        let mut backoff = Duration::from_millis(500);
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(8));
            }
            match ag
                .post(ENDPOINT)
                .set("x-api-key", &self.api_key)
                .set("anthropic-version", API_VERSION)
                .set("content-type", "application/json")
                .send_json(body.clone())
            {
                Ok(resp) => return parse_response(resp),
                Err(ureq::Error::Status(code, r)) => {
                    let retry_after =
                        r.header("retry-after").and_then(|s| s.trim().parse::<u64>().ok());
                    let detail = r
                        .into_json::<serde_json::Value>()
                        .ok()
                        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    last_err = format!("Anthropic error {code}: {detail}");
                    if !is_retryable(code) {
                        return Err(last_err);
                    }
                    if let Some(secs) = retry_after {
                        backoff = Duration::from_secs(secs.min(30));
                    }
                }
                Err(e) => last_err = format!("network error: {e}"),
            }
        }
        Err(last_err)
    }
}
