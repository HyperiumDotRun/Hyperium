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

    /// Overrides the default model for this instance — for call sites that
    /// need more reliable instruction-following than Haiku gives (e.g. the
    /// tool-calling agent, which has to consistently call a tool rather than
    /// answer from memory) without changing the cheaper default everywhere
    /// else `Anthropic::new` is used.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_ATTEMPTS: u32 = 3;
/// A tool-use turn that never stops asking for tools is a bug in the prompt,
/// not something to loop on forever — this is a circuit breaker, not a
/// budget anyone is meant to spend.
const MAX_TOOL_TURNS: u32 = 6;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .build()
}

fn is_retryable(code: u16) -> bool {
    matches!(code, 429 | 500 | 502 | 503 | 504 | 529)
}

/// One tool the model may call. `input_schema` is the same JSON Schema shape
/// the Messages API expects, so a tool's argument validation happens on
/// Anthropic's side before this app ever sees it — not something this file
/// re-checks.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

impl Anthropic {
    fn send(&self, body: serde_json::Value) -> Result<serde_json::Value, String> {
        if self.api_key.trim().is_empty() {
            return Err("no Anthropic API key - set it in Settings → AI".into());
        }
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
                Ok(resp) => {
                    return resp
                        .into_json::<serde_json::Value>()
                        .map_err(|e| format!("bad response: {e}"));
                }
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

    fn text_of(content: &[serde_json::Value]) -> String {
        content.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect()
    }

    /// A full tool-use conversation, not a single completion: sends
    /// `messages` (appending to it as the exchange goes) plus `tools`, and
    /// for every `tool_use` block the model asks for, calls
    /// `dispatch(name, input)` for the result and hands it back as a
    /// `tool_result` — repeating until the model answers in plain text
    /// instead of asking for another tool.
    ///
    /// Every number that ends up in the final reply came out of a real
    /// `dispatch` call against a real API; the model only ever decides
    /// *when* to ask, never what the answer is.
    pub fn converse(
        &self,
        system: &str,
        messages: &mut Vec<serde_json::Value>,
        tools: &[ToolSpec],
        dispatch: impl Fn(&str, &serde_json::Value) -> String,
    ) -> Result<String, String> {
        let tools_json: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        for _ in 0..MAX_TOOL_TURNS {
            let body = serde_json::json!({
                "model": self.model,
                "max_tokens": 2048,
                "system": system,
                "messages": messages.clone(),
                "tools": tools_json,
            });
            let v = self.send(body)?;
            if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                return Err(err["message"].as_str().unwrap_or("Anthropic returned an error").to_string());
            }
            let content = v["content"].as_array().cloned().unwrap_or_default();
            messages.push(serde_json::json!({ "role": "assistant", "content": content.clone() }));

            let tool_uses: Vec<&serde_json::Value> =
                content.iter().filter(|b| b["type"] == "tool_use").collect();
            if tool_uses.is_empty() {
                let text = Self::text_of(&content);
                if text.trim().is_empty() {
                    return Err("empty response from the model".into());
                }
                return Ok(text);
            }

            let results: Vec<serde_json::Value> = tool_uses
                .iter()
                .map(|tu| {
                    let id = tu["id"].as_str().unwrap_or_default();
                    let name = tu["name"].as_str().unwrap_or_default();
                    let input = &tu["input"];
                    let result = dispatch(name, input);
                    serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": result,
                    })
                })
                .collect();
            messages.push(serde_json::json!({ "role": "user", "content": results }));
        }
        Err("the model kept calling tools without answering — try rephrasing".into())
    }
}

impl LlmProvider for Anthropic {
    fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });
        let v = self.send(body)?;
        let text = v["content"]
            .as_array()
            .map(|blocks| Self::text_of(blocks))
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err("empty response from the model".into());
        }
        Ok(text)
    }
}
