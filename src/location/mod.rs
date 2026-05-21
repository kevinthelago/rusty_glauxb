pub mod mock;
pub mod native;
pub mod network;
pub mod source;

pub use source::{LocationFrame, LocationSource};

use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use mock::MockLocationSource;
use native::AndroidGpsSource;
use network::NetworkLocationSource;

// ─── LocationManager ─────────────────────────────────────────────────────────

/// Owns the active location source and exposes a watch channel for the rest of
/// the app.  Exactly one source runs at a time; switching sources aborts the
/// previous task immediately.
pub struct LocationManager {
    tx:              watch::Sender<Option<LocationFrame>>,
    pub rx:          watch::Receiver<Option<LocationFrame>>,
    task:            Option<JoinHandle<()>>,
    pub source_name: String,
    /// WebSocket URL used by the network source.
    pub network_url: String,
}

impl Default for LocationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LocationManager {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(None);
        Self {
            tx,
            rx,
            task:        None,
            source_name: "off".to_string(),
            network_url: "ws://192.168.1.100:7777".to_string(),
        }
    }

    /// Most recent fix, or `None` if the source is stopped / not yet locked.
    pub fn latest(&self) -> Option<LocationFrame> {
        self.rx.borrow().clone()
    }

    /// Stop the active source and clear the fix.
    pub fn stop(&mut self) {
        if let Some(h) = self.task.take() {
            h.abort();
        }
        let _ = self.tx.send(None);
        self.source_name = "off".to_string();
    }

    pub fn set_mock(&mut self, rt: &Handle) {
        self.stop();
        self.source_name = "mock".to_string();
        self.task = Some(MockLocationSource.start(rt, self.tx.clone()));
        tracing::info!("location: source = mock");
    }

    pub fn set_network(&mut self, rt: &Handle) {
        self.stop();
        self.source_name = "network".to_string();
        let src = NetworkLocationSource { url: self.network_url.clone() };
        self.task = Some(src.start(rt, self.tx.clone()));
        tracing::info!(url = %self.network_url, "location: source = network");
    }

    pub fn set_native(&mut self, rt: &Handle) {
        self.stop();
        self.source_name = "native".to_string();
        self.task = Some(AndroidGpsSource.start(rt, self.tx.clone()));
        tracing::info!("location: source = native (Android GPS)");
    }

    /// Switch to a source by name.  Returns an error string for the console if
    /// the name is unrecognised.
    pub fn set_source(&mut self, name: &str, rt: &Handle) -> Result<(), String> {
        match name {
            "mock"    => { self.set_mock(rt);    Ok(()) }
            "network" => { self.set_network(rt); Ok(()) }
            "native"  => { self.set_native(rt);  Ok(()) }
            "off"     => { self.stop();           Ok(()) }
            other     => Err(format!(
                "unknown location source '{other}'  (valid: mock, network, native, off)"
            )),
        }
    }
}

impl Drop for LocationManager {
    fn drop(&mut self) {
        if let Some(h) = self.task.take() {
            h.abort();
        }
    }
}
