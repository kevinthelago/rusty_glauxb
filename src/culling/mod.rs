#![allow(dead_code)]
//! Frustum and horizon culling for globe data points.
//!
//! Two culling passes are applied in order (cheapest first):
//!
//! 1. **Horizon cull** — drops points on the back hemisphere of the globe
//!    that are occluded by the sphere itself. This is a single dot-product.
//!
//! 2. **Frustum cull** — drops points outside the camera's view frustum.
//!    Planes are extracted from the view-projection matrix using the
//!    Gribb/Hartmann method, adapted for wgpu's clip space (z ∈ [0, 1]).

use glam::{Mat4, Vec3, Vec4};

// ─── Frustum ─────────────────────────────────────────────────────────────────

/// Six clip-space half-spaces extracted from a view-projection matrix.
/// A point is inside when it satisfies all six plane inequalities.
pub struct Frustum {
    /// Each plane is stored as (a, b, c, d) where `ax + by + cz + d >= 0`
    /// means "inside". Planes are length-normalised so the scalar distance
    /// to a point equals `plane.dot(point.extend(1.0))`.
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extract frustum planes from a combined view-projection matrix.
    ///
    /// Uses the Gribb/Hartmann row-combination method.  For wgpu clip space
    /// (z ∈ [0, 1], right-handed), the near plane is `row2` rather than
    /// `row3 + row2` as in the OpenGL convention.
    pub fn from_view_proj(vp: Mat4) -> Self {
        let r0 = vp.row(0);
        let r1 = vp.row(1);
        let r2 = vp.row(2);
        let r3 = vp.row(3);

        let mut planes = [
            r3 + r0, // left   (-w ≤ x)
            r3 - r0, // right  (x ≤  w)
            r3 + r1, // bottom (-w ≤ y)
            r3 - r1, // top    (y ≤  w)
            r2,      // near   (0 ≤ z)  — wgpu z-in-[0,1] convention
            r3 - r2, // far    (z ≤  w)
        ];

        // Normalise so plane.dot(p.extend(1)) gives a metric distance.
        for p in &mut planes {
            let inv = 1.0 / p.truncate().length();
            *p *= inv;
        }

        Self { planes }
    }

    /// Returns `true` if `p` (world space) is inside all six planes.
    pub fn contains_point(&self, p: Vec3) -> bool {
        let p4 = p.extend(1.0);
        self.planes.iter().all(|plane| plane.dot(p4) >= 0.0)
    }

    /// Returns `true` if a sphere at `center` with `radius` overlaps the frustum.
    /// Used for coarse per-chunk culling before testing individual points.
    pub fn contains_sphere(&self, center: Vec3, radius: f32) -> bool {
        let c4 = center.extend(1.0);
        self.planes.iter().all(|plane| plane.dot(c4) >= -radius)
    }

    /// Combined horizon + frustum test for a point on the unit sphere.
    ///
    /// This is the primary entry point for data-layer point culling.
    /// Returns `true` only if the point is both on the near hemisphere AND
    /// inside the view frustum.
    pub fn is_globe_point_visible(&self, world_pos: Vec3, camera_eye: Vec3) -> bool {
        horizon_visible(world_pos, camera_eye) && self.contains_point(world_pos)
    }
}

// ─── Horizon culling ─────────────────────────────────────────────────────────

/// Returns `true` if `point` (on the unit sphere) is on the near hemisphere
/// as seen from `camera_eye` — i.e., not occluded by the globe body itself.
///
/// Derivation: the horizon tangent from a camera at distance `d` touches the
/// unit sphere at angle `θ = acos(1/d)` from the sub-camera direction.
/// A point is visible when its angle from that direction is < θ, equivalent
/// to `dot(eye_dir, point) > 1/d`.
pub fn horizon_visible(point: Vec3, camera_eye: Vec3) -> bool {
    let d = camera_eye.length();
    let eye_dir = camera_eye / d;
    // Threshold is cos(horizon_angle) = 1/d.  Points with larger dot are closer
    // to the nadir and therefore definitely visible.
    eye_dir.dot(point) > 1.0 / d
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn horizon_near_side_visible() {
        // Camera directly above north pole at height 2.0
        let eye = Vec3::new(0.0, 2.0, 0.0);
        // North pole itself — directly under camera
        assert!(horizon_visible(Vec3::Y, eye));
    }

    #[test]
    fn horizon_far_side_culled() {
        let eye = Vec3::new(0.0, 2.0, 0.0);
        // South pole — directly behind the globe
        assert!(!horizon_visible(-Vec3::Y, eye));
    }

    #[test]
    fn horizon_equator_depends_on_height() {
        // Camera at height 1.5, looking down the +Y axis.
        // Horizon angle = acos(1/1.5) ≈ 48.2°.  The equator is at 90° from
        // the north pole, so the equatorial point along +X should be culled.
        let eye = Vec3::new(0.0, 1.5, 0.0);
        assert!(!horizon_visible(Vec3::X, eye));
    }
}
