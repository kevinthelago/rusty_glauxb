pub mod audio;
pub mod command;
pub mod llm;
pub mod stt;

pub use command::VoiceCommand;

use std::sync::Arc;
use tokio::{runtime::Handle, sync::mpsc};

use self::{
    audio::AudioCapture,
    llm::{AnthropicClient, LlmClient},
    stt::{SpeechToText, WhisperApiClient},
};

/// Orchestrates the push-to-talk → STT → LLM → `VoiceCommand` pipeline.
///
/// The heavy work (STT + LLM network calls) runs on the tokio thread pool so
/// the render loop is never blocked. Commands arrive via a non-blocking mpsc
/// channel polled each frame via `poll_command`.
pub struct VoiceController {
    rt: Handle,
    audio: AudioCapture,
    stt: Option<Arc<dyn SpeechToText>>,
    llm: Option<Arc<dyn LlmClient>>,
    tx: mpsc::Sender<VoiceCommand>,
    rx: mpsc::Receiver<VoiceCommand>,
    available_layers: Vec<String>,
}

impl VoiceController {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = mpsc::channel(16);

        let stt: Option<Arc<dyn SpeechToText>> = match WhisperApiClient::from_env() {
            Ok(c) => Some(Arc::new(c)),
            Err(e) => {
                tracing::warn!("STT disabled: {e}");
                None
            }
        };
        let llm: Option<Arc<dyn LlmClient>> = match AnthropicClient::from_env() {
            Ok(c) => Some(Arc::new(c)),
            Err(e) => {
                tracing::warn!("LLM disabled: {e}");
                None
            }
        };

        let audio = AudioCapture::new().unwrap_or_else(|e| {
            tracing::warn!("audio capture unavailable: {e}");
            AudioCapture::default()
        });

        Self {
            rt,
            audio,
            stt,
            llm,
            tx,
            rx,
            available_layers: vec!["all".into()],
        }
    }

    /// Update the layer list passed into the LLM system prompt.
    #[allow(dead_code)]
    pub fn set_available_layers(&mut self, names: Vec<String>) {
        self.available_layers = names;
    }

    /// Begin audio capture (Space key down).
    pub fn start_recording(&mut self) {
        if let Err(e) = self.audio.start() {
            tracing::error!("start_recording: {e}");
        }
    }

    /// Stop capture and dispatch the async STT → LLM task (Space key up).
    pub fn stop_and_process(&mut self) {
        let wav = match self.audio.stop() {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("audio stop: {e}");
                return;
            }
        };

        let (Some(stt), Some(llm)) = (self.stt.clone(), self.llm.clone()) else {
            tracing::warn!("voice pipeline disabled — set OPENAI_API_KEY + ANTHROPIC_API_KEY");
            return;
        };

        let tx = self.tx.clone();
        let layer_refs: Vec<&str> = self.available_layers.iter().map(|s| s.as_str()).collect();
        let system = llm::build_system_prompt(&layer_refs);

        self.rt.spawn(async move {
            let transcript = match stt.transcribe(wav).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("STT: {e}");
                    return;
                }
            };
            tracing::info!("voice transcript: \"{transcript}\"");

            let raw = match llm.complete(&system, &transcript).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("LLM: {e}");
                    return;
                }
            };

            match llm::parse_command(&raw, &transcript) {
                Ok(cmd) => {
                    let _ = tx.send(cmd).await;
                }
                Err(e) => tracing::error!("parse_command: {e}"),
            }
        });
    }

    /// Non-blocking poll — returns the most recent command if one arrived.
    /// Drains the channel so stale commands don't accumulate.
    pub fn poll_command(&mut self) -> Option<VoiceCommand> {
        let mut last = None;
        while let Ok(cmd) = self.rx.try_recv() {
            last = Some(cmd);
        }
        last
    }
}
