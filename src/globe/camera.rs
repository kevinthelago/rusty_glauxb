use glam::{Mat4, Vec3};

/// Arcball camera orbiting the globe at the origin.
pub struct ArcballCamera {
    /// Distance from origin (zoom).
    pub radius: f32,
    /// Horizontal orbit angle in radians.
    pub yaw: f32,
    /// Vertical orbit angle in radians (clamped away from poles).
    pub pitch: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for ArcballCamera {
    fn default() -> Self {
        Self {
            radius: 2.5,
            yaw: 0.0,
            pitch: 0.3,
            fov_y: 45_f32.to_radians(),
            aspect: 16.0 / 9.0,
            near: 0.01,
            far: 100.0,
        }
    }
}

impl ArcballCamera {
    pub fn eye(&self) -> Vec3 {
        Vec3::new(
            self.radius * self.pitch.cos() * self.yaw.sin(),
            self.radius * self.pitch.sin(),
            self.radius * self.pitch.cos() * self.yaw.cos(),
        )
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), Vec3::ZERO, Vec3::Y)
    }

    pub fn projection(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.projection() * self.view()
    }

    /// Pan orbit by mouse delta (in normalised screen space).
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw   -= dx * 2.0;
        self.pitch  = (self.pitch + dy * 2.0).clamp(-1.5, 1.5);
    }

    /// Scroll to zoom.
    pub fn zoom(&mut self, delta: f32) {
        self.radius = (self.radius - delta * 0.3).clamp(1.05, 50.0);
    }

    /// Fly camera to a geographic coordinate (lat/lon in degrees).
    pub fn focus(&mut self, lat_deg: f32, lon_deg: f32) {
        self.pitch = lat_deg.to_radians();
        self.yaw = lon_deg.to_radians();
    }
}
