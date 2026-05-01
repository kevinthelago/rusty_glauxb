use std::time::Duration;
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;

use super::source::{LocationFrame, LocationSource};

/// Receives location frames from a remote WebSocket server (e.g. the iOS
/// companion app) and forwards them through the watch channel.
///
/// Reconnects automatically with exponential backoff (1 s → 32 s cap).
pub struct NetworkLocationSource {
    pub url: String,
}

impl LocationSource for NetworkLocationSource {
    fn name(&self) -> &str {
        "network"
    }

    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()> {
        rt.spawn(async move {
            let mut backoff = Duration::from_secs(1);

            loop {
                if tx.is_closed() {
                    break;
                }

                tracing::info!(url = %self.url, "location: connecting");

                match connect_async(&self.url).await {
                    Ok((mut stream, _)) => {
                        backoff = Duration::from_secs(1); // reset on success

                        while let Some(msg) = stream.next().await {
                            if tx.is_closed() {
                                return;
                            }
                            match msg {
                                Ok(m) if m.is_text() || m.is_binary() => {
                                    match serde_json::from_slice::<LocationFrame>(&m.into_data()) {
                                        Ok(frame) => {
                                            if tx.send(Some(frame)).is_err() {
                                                return;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("location: bad frame: {e}");
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!("location: ws error: {e}");
                                    break;
                                }
                            }
                        }

                        tracing::info!("location: disconnected — reconnecting");
                    }
                    Err(e) => {
                        tracing::warn!(url = %self.url, "location: connect failed: {e}");
                    }
                }

                // Clear the fix while disconnected so the HUD dot disappears.
                let _ = tx.send(None);

                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(32));
            }
        })
    }
}
