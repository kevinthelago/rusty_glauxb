use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

use super::source::{LocationFrame, LocationSource};

/// Deterministic circular flight path for desktop testing.
///
/// Orbits the globe at 35 000 ft over San Francisco, completing one lap
/// every ~60 seconds at 1 Hz.  All accuracy and heading values are synthetic.
pub struct MockLocationSource;

impl LocationSource for MockLocationSource {
    fn name(&self) -> &str {
        "mock"
    }

    fn start(
        self,
        rt: &tokio::runtime::Handle,
        tx: watch::Sender<Option<LocationFrame>>,
    ) -> tokio::task::JoinHandle<()> {
        rt.spawn(async move {
            let mut t: f64 = 0.0;
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;

                // Stop when the receiver is gone (LocationManager dropped).
                if tx.is_closed() {
                    break;
                }

                // Circular path centred on San Francisco, radius ~1 degree.
                let lat = 37.7749 + t.sin() * 1.0;
                let lon = -122.4194 + t.cos() * 1.0;
                let hdg = (t.to_degrees() + 90.0).rem_euclid(360.0);

                let frame = LocationFrame {
                    lat,
                    lon,
                    alt:  10_668.0, // 35 000 ft in metres
                    hdg,
                    spd:  250.0,
                    acc:  5.0,
                    ts:   SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                };

                if tx.send(Some(frame)).is_err() {
                    break;
                }

                t += 0.1047; // 2π / 60 — one orbit per 60 ticks
            }
        })
    }
}
