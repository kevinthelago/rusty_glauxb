use tokio::sync::watch;

/// A single GPS fix, mirroring the JSON wire format shared with the iOS
/// companion app and any future network source.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LocationFrame {
    /// Latitude in decimal degrees (WGS-84).
    pub lat: f64,
    /// Longitude in decimal degrees (WGS-84).
    pub lon: f64,
    /// Altitude in metres above the WGS-84 ellipsoid.
    pub alt: f64,
    /// True heading in degrees (0 = north, 90 = east).  NaN if unavailable.
    pub hdg: f64,
    /// Ground speed in m/s.
    pub spd: f64,
    /// Horizontal accuracy radius in metres (1-sigma).
    pub acc: f64,
    /// Unix timestamp in milliseconds.
    pub ts: u64,
}

/// Common interface for every location backend.
///
/// Implementations spawn their own tokio task and push frames through the
/// provided `watch::Sender`.  The task must respect the cancellation token
/// (a dropped sender signals shutdown).
pub trait LocationSource: Send + 'static {
    fn name(&self) -> &str;

    /// Start streaming into `tx`.  The implementation owns the task lifetime;
    /// abort via the returned `JoinHandle`.
    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()>;
}
