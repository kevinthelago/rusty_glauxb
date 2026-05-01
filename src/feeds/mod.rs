use tokio::task::JoinHandle;

/// Manages all live WebSocket feed tasks.
///
/// Each feed runs in its own tokio task and pushes data into a
/// `tokio::sync::watch` channel. The corresponding DataLayer reads
/// the receiver each frame without blocking the render loop.
#[derive(Default)]
pub struct FeedManager {
    handles: Vec<JoinHandle<()>>,
}

impl FeedManager {
    /// Spawn a new feed task. The task should own a `watch::Sender` and
    /// reconnect with exponential backoff on disconnect.
    #[allow(dead_code)]
    pub fn spawn(&mut self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.handles.push(tokio::spawn(task));
    }

    /// Abort all running feed tasks (called on shutdown).
    pub fn shutdown(&mut self) {
        for h in self.handles.drain(..) {
            h.abort();
        }
    }
}

impl Drop for FeedManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
