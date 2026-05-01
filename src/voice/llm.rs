use crate::voice::command::VoiceCommand;

/// LLM backend for parsing voice transcripts into structured layer commands.
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a system + user message and return the raw text completion.
    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String>;
}

/// Anthropic Claude client (claude-haiku for low-latency voice responses).
///
/// Set `ANTHROPIC_API_KEY` in the environment.
/// Swap for an Ollama HTTP client (`http://localhost:11434`) to run fully offline.
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
            model: "claude-haiku-4-5-20251001".to_string(),
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(key))
    }
}

#[async_trait::async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "system": system,
            "messages": [{"role": "user", "content": user}]
        });

        let resp = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

        let text = resp["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(text)
    }
}

/// System prompt template. Layer names are injected at call time.
pub fn build_system_prompt(available_layers: &[&str]) -> String {
    format!(
        r#"You are a map layer filter assistant for a globe visualization tool.
Given a natural language request, return ONLY a JSON object with these fields:
  "layers": array of layer names to show (use ["all"] to show everything)
  "location": {{ "lat": number, "lon": number, "name": string }} or null for current location
  "zoom": one of "globe" | "continent" | "country" | "city" | "street" or null

Available layers: {layers}

Return ONLY valid JSON. No explanation, no markdown, no code fences."#,
        layers = available_layers.join(", ")
    )
}

/// Parse the raw LLM JSON response into a `VoiceCommand`, injecting the
/// original transcript for HUD display.
pub fn parse_command(json_text: &str, transcript: &str) -> anyhow::Result<VoiceCommand> {
    let mut cmd: VoiceCommand = serde_json::from_str(json_text)
        .map_err(|e| anyhow::anyhow!("LLM returned invalid JSON: {e}\nraw: {json_text}"))?;
    cmd.transcript = transcript.to_string();
    Ok(cmd)
}
