use glam::Vec3;

use crate::geo::cartesian_to_geo;

use super::camera::ArcballCamera;

// ─── Camera mode ──────────────────────────────────────────────────────────────

/// What the camera should do when the user is not actively dragging.
#[derive(Clone, Debug, PartialEq)]
pub enum CameraMode {
    /// Free camera — no autonomous movement.
    None,

    /// Continuously rotate (auto-spin) the globe.
    Rotate {
        /// Degrees per second (positive = eastward).
        speed_deg:  f32,
        /// Spring time constant in seconds.  Smaller = snappier wind-up/wind-down.
        elasticity: f32,
    },

    /// Lock onto a live entity identified by its ICAO24 (or any string key).
    Follow {
        entity_id: String,
        /// Exponential-smoothing time constant in seconds for yaw/pitch.
        smoothing: f32,
        /// Camera radius (globe radii) to maintain.
        distance:  f32,
    },

    /// Lock onto the device's GPS position from `LocationManager`.
    FollowLocation {
        smoothing: f32,
        distance:  f32,
    },
}

impl Default for CameraMode {
    fn default() -> Self {
        Self::Rotate { speed_deg: 10.0, elasticity: 0.4 }
    }
}

// ─── CameraAnimator ───────────────────────────────────────────────────────────

/// Drives the arcball camera each frame according to the active `CameraMode`.
pub struct CameraAnimator {
    pub mode:    CameraMode,
    /// Set `true` while the user is dragging to suspend animation without
    /// resetting velocity state.
    pub paused:  bool,
    /// Current angular velocity (rad/s) maintained by the spring model.
    yaw_vel:     f32,
}

impl Default for CameraAnimator {
    fn default() -> Self {
        Self { mode: CameraMode::default(), paused: false, yaw_vel: 0.0 }
    }
}

impl CameraAnimator {
    pub fn new(mode: CameraMode) -> Self {
        Self { mode, paused: false, yaw_vel: 0.0 }
    }

    /// Replace the current mode and reset spring state.
    pub fn set_mode(&mut self, mode: CameraMode) {
        if self.mode != mode {
            self.yaw_vel = 0.0;
        }
        self.mode = mode;
    }

    /// Advance camera state by `dt` seconds.
    ///
    /// `entity_pos` — world-space unit-sphere position of the followed entity,
    /// if one is active and visible.
    pub fn tick(&mut self, camera: &mut ArcballCamera, dt: f32, entity_pos: Option<Vec3>) {
        if self.paused {
            // Bleed velocity during drag so the spring doesn't lurch on release.
            self.yaw_vel *= (1.0 - dt * 8.0_f32).max(0.0);
            return;
        }

        match self.mode.clone() {
            CameraMode::None => {
                self.yaw_vel = 0.0;
            }

            CameraMode::Rotate { speed_deg, elasticity } => {
                let target_vel = speed_deg.to_radians();
                // First-order low-pass on velocity; tau controls the lag.
                let tau    = elasticity * 3.0 + 0.05;
                let factor = 1.0 - (-dt / tau).exp();
                self.yaw_vel += (target_vel - self.yaw_vel) * factor;
                camera.yaw   += self.yaw_vel * dt;
            }

            CameraMode::Follow { smoothing, distance, .. }
            | CameraMode::FollowLocation { smoothing, distance } => {
                if let Some(pos) = entity_pos {
                    let (lat_deg, lon_deg) = cartesian_to_geo(pos);
                    let t_yaw   = lon_deg.to_radians();
                    let t_pitch = lat_deg.to_radians().clamp(-1.3, 1.3);

                    let tau    = smoothing * 5.0 + 0.1;
                    let factor = 1.0 - (-dt / tau).exp();

                    camera.yaw    = lerp_angle(camera.yaw, t_yaw, factor);
                    camera.pitch  = lerp(camera.pitch,  t_pitch,  factor);
                    camera.radius = lerp(camera.radius, distance, factor * 0.5);
                    self.yaw_vel  = 0.0;
                }
            }
        }
    }

    /// Returns the aircraft entity ID when in `Follow` mode, otherwise `None`.
    pub fn follow_entity_id(&self) -> Option<&str> {
        match &self.mode {
            CameraMode::Follow { entity_id, .. } => Some(entity_id.as_str()),
            _ => None,
        }
    }

    /// Returns `true` when the camera should track the device location.
    pub fn is_follow_location(&self) -> bool {
        matches!(self.mode, CameraMode::FollowLocation { .. })
    }
}

// ─── Math helpers ─────────────────────────────────────────────────────────────

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Linear interpolation over the shortest arc between two angles (radians).
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut diff = b - a;
    while diff >  std::f32::consts::PI { diff -= std::f32::consts::TAU; }
    while diff < -std::f32::consts::PI { diff += std::f32::consts::TAU; }
    a + diff * t
}
