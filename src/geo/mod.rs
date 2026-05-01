//! Geographic coordinate utilities and the canonical data types shared by
//! every data layer.
//!
//! ## Coordinate system
//!
//! All geographic coordinates are stored as **decimal degrees** (WGS-84).
//! Internally the globe is a unit sphere, so Cartesian ↔ geographic conversion
//! uses a radius of 1.0; altitude is added on top when positioning a point in
//! 3-D space.
//!
//! The Cartesian axes match the camera convention in `globe::camera`:
//! ```text
//!   x = cos(lat) * sin(lon)
//!   y = sin(lat)
//!   z = cos(lat) * cos(lon)
//! ```
//!
//! ## Visible-region query bounds
//!
//! `GeoBounds` represents the bounding box of the currently visible globe
//! region.  It is recomputed every frame from the camera position and fed into
//! the data pipeline so that data sources only return features that could
//! actually appear on screen.

use glam::Vec3;

// ─── Geographic bounds ────────────────────────────────────────────────────────

/// Axis-aligned bounding box in geographic coordinates (decimal degrees).
///
/// `min_lon` / `max_lon` may fall outside `[-180, 180]` when the visible
/// region crosses the anti-meridian; use [`GeoBounds::split`] to break such a
/// box into two normalised sub-boxes before passing it to external APIs.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoBounds {
    pub min_lat: f32,
    pub max_lat: f32,
    /// May be outside `[-180, 180]` when crossing the anti-meridian.
    pub min_lon: f32,
    pub max_lon: f32,
}

impl GeoBounds {
    /// Entire globe — used as a fallback when fully zoomed out.
    pub fn global() -> Self {
        Self { min_lat: -90.0, max_lat: 90.0, min_lon: -180.0, max_lon: 180.0 }
    }

    /// Compute the visible geographic bounding box from a camera eye position
    /// (world space, radius = distance from globe centre).
    ///
    /// The result is a conservative hemisphere bound: it covers everything the
    /// camera *could* see regardless of FOV, which is exactly what data sources
    /// need — better to fetch a little too much than to miss visible features.
    pub fn from_eye(eye: Vec3) -> Self {
        let radius = eye.length();
        if radius <= 1.0 {
            return Self::global();
        }

        let unit = eye / radius;

        // Sub-camera (nadir) lat/lon
        let nadir_lat = unit.y.clamp(-1.0, 1.0).asin().to_degrees();
        // atan2(x, z) matches our Cartesian layout: x = cos(lat)*sin(lon)
        let nadir_lon = unit.x.atan2(unit.z).to_degrees();

        // Half-angle of the visible hemisphere cap (degrees)
        let half_deg = (1.0_f32 / radius).clamp(-1.0, 1.0).acos().to_degrees() + 1.0; // +1° safety margin

        // Fall back to global if we can see most of the globe
        if half_deg >= 89.0 {
            return Self::global();
        }

        Self {
            min_lat: (nadir_lat - half_deg).max(-90.0),
            max_lat: (nadir_lat + half_deg).min(90.0),
            min_lon: nadir_lon - half_deg,
            max_lon: nadir_lon + half_deg,
        }
    }

    /// Returns `true` if the point is within the bounding box.
    /// Handles anti-meridian crossing (when `min_lon < -180` or `max_lon > 180`).
    #[allow(dead_code)]
    pub fn contains(&self, lat: f32, lon: f32) -> bool {
        if lat < self.min_lat || lat > self.max_lat {
            return false;
        }
        if self.wraps_antimeridian() {
            lon >= self.min_lon + 360.0 || lon <= self.max_lon - 360.0
                || (lon >= self.min_lon && lon <= self.max_lon)
        } else {
            lon >= self.min_lon && lon <= self.max_lon
        }
    }

    /// Returns `true` when the longitude range crosses the ±180° meridian.
    pub fn wraps_antimeridian(&self) -> bool {
        self.min_lon < -180.0 || self.max_lon > 180.0
    }

    /// Split an anti-meridian-crossing box into at most two normalised boxes.
    /// Most external REST APIs expect coordinates in `[-180, 180]`.
    pub fn split(&self) -> (Self, Option<Self>) {
        if !self.wraps_antimeridian() {
            return (self.clone(), None);
        }
        // Clamp the primary box to the right side of the anti-meridian
        let primary = Self {
            min_lat: self.min_lat,
            max_lat: self.max_lat,
            min_lon: self.min_lon.max(-180.0),
            max_lon: self.max_lon.min(180.0),
        };
        // Overflow wraps around: the bit left of -180 appears at +180 side
        let secondary = if self.min_lon < -180.0 {
            Some(Self {
                min_lat: self.min_lat,
                max_lat: self.max_lat,
                min_lon: self.min_lon + 360.0,
                max_lon: 180.0,
            })
        } else {
            Some(Self {
                min_lat: self.min_lat,
                max_lat: self.max_lat,
                min_lon: -180.0,
                max_lon: self.max_lon - 360.0,
            })
        };
        (primary, secondary)
    }

    /// Fraction [0, 1] representing how much the bounds have changed relative
    /// to their own span.  Used by [`crate::pipeline::QueryScheduler`] to
    /// decide whether to trigger a new fetch.
    pub fn change_fraction(&self, other: &Self) -> f32 {
        let dlat = (self.min_lat - other.min_lat).abs() + (self.max_lat - other.max_lat).abs();
        let dlon = (self.min_lon - other.min_lon).abs() + (self.max_lon - other.max_lon).abs();
        let lat_span = (self.max_lat - self.min_lat).max(1.0);
        let lon_span = (self.max_lon - self.min_lon).max(1.0);
        (dlat / lat_span + dlon / lon_span) * 0.5
    }

    /// Span in square degrees (approximate, ignoring curvature).
    #[allow(dead_code)]
    pub fn area_sq_deg(&self) -> f32 {
        (self.max_lat - self.min_lat) * (self.max_lon - self.min_lon)
    }
}

// ─── Coordinate conversions ───────────────────────────────────────────────────

/// Convert geographic coordinates to a unit-sphere Cartesian position.
///
/// The returned vector lies on the surface of the unit sphere (radius = 1).
/// Multiply by `(1.0 + alt_metres / EARTH_RADIUS_M)` to account for altitude.
pub fn geo_to_cartesian(lat_deg: f32, lon_deg: f32) -> Vec3 {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos())
}

/// Convert a unit-sphere Cartesian position back to geographic coordinates.
/// Returns `(lat_deg, lon_deg)`.
pub fn cartesian_to_geo(p: Vec3) -> (f32, f32) {
    let p = p.normalize();
    let lat_deg = p.y.clamp(-1.0, 1.0).asin().to_degrees();
    let lon_deg = p.x.atan2(p.z).to_degrees();
    (lat_deg, lon_deg)
}

// ─── Canonical feature types ─────────────────────────────────────────────────

/// A single geographic data point emitted by a [`crate::pipeline::DataSource`].
///
/// The renderer converts `GeoFeature`s into `GpuPoint` instances for
/// instanced draw calls.  The `meta` field carries any layer-specific
/// payload (name, value, icon, …) that the HUD tooltip or colour-mapping
/// logic may need.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GeoFeature {
    /// Latitude of the feature (for Point) or of the first node (for Line).
    pub lat: f32,
    pub lon: f32,
    /// Altitude in metres above the reference ellipsoid (0 = surface).
    pub alt: f32,
    pub kind: FeatureKind,
    pub meta: serde_json::Value,
    /// Ordered `[lat, lon]` pairs for Line / Polygon features.  Empty for Point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coordinates: Vec<[f32; 2]>,
}

impl GeoFeature {
    /// Pre-compute the world-space unit-sphere position for this feature.
    pub fn world_pos(&self) -> Vec3 {
        // Altitude offset ignored at globe scale; add when zoomed-in LOD lands.
        geo_to_cartesian(self.lat, self.lon)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    /// A single location (city, POI, flight position, …).
    Point,
    /// Future: a polyline (roads, flight paths, …).
    Line,
    /// Future: a filled polygon (census tracts, weather cells, …).
    Polygon,
}

// ─── World-map polygon types ─────────────────────────────────────────────────

/// A filled geographic polygon (country, state, district, …) used by the
/// world-map layer for GPU rendering and CPU hit-testing.
#[derive(Clone, Debug)]
pub struct GeoPolygon {
    /// Short unique identifier: ISO A2 for countries, ISO 3166-2 for states.
    pub id:       String,
    /// Human-readable display name.
    pub name:     String,
    /// Parent feature ID (empty for countries, country ISO A2 for states).
    #[allow(dead_code)]
    pub parent:   String,
    /// Label placement point `[lat, lon]` (degrees).
    pub centroid: [f32; 2],
    /// `polygons[poly][ring][pt] = [lat, lon]`.
    /// `ring 0` = outer boundary; `ring 1..` = holes.  Matches GeoJSON
    /// MultiPolygon layout after swapping `[lon, lat]` → `[lat, lon]`.
    pub polygons: Vec<Vec<Vec<[f32; 2]>>>,
}

impl GeoPolygon {
    /// Returns `true` if `(lat, lon)` falls inside this polygon (any sub-polygon).
    /// Uses the 2-D ray-casting algorithm in geographic space; minor errors near
    /// the anti-meridian are acceptable at country/state scale.
    pub fn contains_point(&self, lat: f32, lon: f32) -> bool {
        for poly in &self.polygons {
            if let Some(outer) = poly.first() {
                if ring_contains(lat, lon, outer)
                    && !poly.iter().skip(1).any(|hole| ring_contains(lat, lon, hole))
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Ray-casting point-in-polygon for a single ring.
/// Ring points are `[lat, lon]`; internally treats lon as X and lat as Y.
pub fn ring_contains(lat: f32, lon: f32, ring: &[[f32; 2]]) -> bool {
    let n = ring.len();
    if n < 3 { return false; }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][1], ring[i][0]); // lon, lat
        let (xj, yj) = (ring[j][1], ring[j][0]);
        if ((yi > lat) != (yj > lat))
            && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Returns `true` when the ring's longitude span > 350°, indicating it
/// crosses the anti-meridian and earcut triangulation would be incorrect.
pub fn ring_spans_antimeridian(ring: &[[f32; 2]]) -> bool {
    if ring.is_empty() { return false; }
    let min_lon = ring.iter().map(|p| p[1]).fold(f32::INFINITY,     f32::min);
    let max_lon = ring.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
    max_lon - min_lon > 350.0
}

/// Administrative hierarchy level used by the world-map layer.
#[derive(Clone, Debug, PartialEq)]
pub enum AdminLevel {
    Country,
    State(String), // parent country ISO A2
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_geo_cartesian() {
        let (lat, lon) = (37.77_f32, -122.42_f32); // San Francisco
        let cart = geo_to_cartesian(lat, lon);
        let (lat2, lon2) = cartesian_to_geo(cart);
        assert!((lat - lat2).abs() < 0.001, "lat roundtrip: {lat} vs {lat2}");
        assert!((lon - lon2).abs() < 0.001, "lon roundtrip: {lon} vs {lon2}");
    }

    #[test]
    fn bounds_from_eye_close_camera() {
        // Camera at height 1.5 directly above north pole.
        // Horizon half-angle ≈ 48°, so the southern edge (min_lat) should be
        // well above -90° — confirming we don't fall back to global bounds.
        let eye = Vec3::new(0.0, 1.5, 0.0);
        let b = GeoBounds::from_eye(eye);
        assert!(b.min_lat > -90.0, "should not cover full globe: {b:?}");
        assert!(b.min_lat > 40.0, "expected ~41° min_lat, got {}", b.min_lat);
    }

    #[test]
    fn bounds_from_eye_far_camera() {
        // Camera very far away → should fall back to global
        let eye = Vec3::new(0.0, 50.0, 0.0);
        let b = GeoBounds::from_eye(eye);
        assert_eq!(b, GeoBounds::global());
    }

    #[test]
    fn antimeridian_split() {
        let b = GeoBounds { min_lat: -10.0, max_lat: 10.0, min_lon: 170.0, max_lon: 200.0 };
        assert!(b.wraps_antimeridian());
        let (primary, secondary) = b.split();
        assert!(secondary.is_some());
        assert!(primary.min_lon >= -180.0 && primary.max_lon <= 180.0);
    }
}
