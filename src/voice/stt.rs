/// Speech-to-text backend. Swap implementations without changing callers.
#[async_trait::async_trait]
pub trait SpeechToText: Send + Sync {
    /// Transcribe raw WAV bytes to a UTF-8 string.
    async fn transcribe(&self, wav_bytes: Vec<u8>) -> anyhow::Result<String>;
}

/// Calls the OpenAI Whisper API (`/v1/audio/transcriptions`).
///
/// Set `OPENAI_API_KEY` in the environment, or swap this implementation
/// for `whisper-rs` to run Whisper fully offline.
pub struct WhisperApiClient {
    http: reqwest::Client,
    api_key: String,
}

impl WhisperApiClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set"))?;
        Ok(Self::new(key))
    }
}

#[async_trait::async_trait]
impl SpeechToText for WhisperApiClient {
    async fn transcribe(&self, wav_bytes: Vec<u8>) -> anyhow::Result<String> {
        use reqwest::multipart;
        let part = multipart::Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")?;
        let form = multipart::Form::new()
            .part("file", part)
            .text("model", "whisper-1");

        let resp = self
            .http
            .post("https://api.openai.com/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;

        let text = resp["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(text)
    }
}
