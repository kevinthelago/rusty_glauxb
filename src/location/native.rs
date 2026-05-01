//! Android native GPS via `android.location.LocationManager` (JNI).
//!
//! Stub — the JNI plumbing is wired up here once the Android activity
//! exposes a `getLocation()` method through the JavaVM handle provided by
//! `android-activity`.  For now the source connects but sends no frames,
//! so the HUD dot stays hidden until the native side is implemented.

use tokio::sync::watch;

use super::source::{LocationFrame, LocationSource};

pub struct AndroidGpsSource;

impl LocationSource for AndroidGpsSource {
    fn name(&self) -> &str {
        "native"
    }

    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()> {
        rt.spawn(async move {
            tracing::warn!(
                "location: native Android GPS is not yet implemented — \
                 switch to 'mock' or 'network' for now"
            );
            // Keep the task alive so LocationManager's abort() works uniformly.
            // When the JNI implementation lands, replace this with a loop that
            // reads from a jni::JavaVM and calls tx.send(Some(frame)).
            let _ = tx.send(None);
            tokio::signal::ctrl_c().await.ok();
        })
    }
}
