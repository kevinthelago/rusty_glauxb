/// Tile-based visibility and LOD for the globe mesh.
///
/// The cube-sphere is subdivided into a coarse 8×8 grid per face, giving
/// 384 tiles in total.  Each tile corresponds to exactly 8×8 quads of the
/// 64-subdivision mesh, so tile visibility is the CPU equivalent of asking
/// "would the GPU render any triangle in this patch?"
///
/// Visibility is tested with the same two checks the GPU applies:
///   1. Horizon cull  — is the tile's bounding sphere on the back hemisphere?
///   2. Frustum cull  — does the tile's bounding sphere overlap the view frustum?
///
/// Results are stored in a compact `TileSet` (6 × u64 = 384 bits) so that
/// comparing two consecutive frames is a handful of XOR operations.
use glam::Vec3;

use crate::{culling::Frustum, geo::GeoBounds};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Tiles per face edge.  8 × 8 per face × 6 faces = 384 tiles.
pub const TILES_PER_EDGE: u32 = 8;
/// Total number of tiles.
pub const TILE_COUNT: usize = (TILES_PER_EDGE * TILES_PER_EDGE * 6) as usize;

// ─── LodLevel ────────────────────────────────────────────────────────────────

/// Detail level driven by camera distance (globe radii from centre).
///
/// The same level that governs mesh resolution governs data density, so both
/// always agree on what "zoomed out" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LodLevel {
    /// radius ≥ 8  — minimal wireframe, no line layers, aircraft count only.
    Overview = 0,
    /// radius 3–8  — coarse mesh, country/coastline borders only, clustered dots.
    Coarse   = 1,
    /// radius 1.5–3 — medium mesh, major roads + rivers, small dots.
    Medium   = 2,
    /// radius < 1.5 — full-res mesh, all layers, full dot/milsym detail.
    Full     = 3,
}

impl LodLevel {
    /// Derive the LOD level from the camera's distance from the globe centre.
    pub fn from_radius(radius: f32) -> Self {
        if      radius >= 8.0 { Self::Overview }
        else if radius >= 3.0 { Self::Coarse   }
        else if radius >= 1.5 { Self::Medium   }
        else                   { Self::Full     }
    }

    /// Globe mesh subdivision count appropriate for this LOD.
    pub fn mesh_subdivisions(self) -> u32 {
        match self {
            Self::Overview => 8,
            Self::Coarse   => 16,
            Self::Medium   => 32,
            Self::Full     => 64,
        }
    }
}

// ─── TileSet ─────────────────────────────────────────────────────────────────

/// Compact 384-bit bitset — one bit per tile.
///
/// Stored as 6 × u64.  Tile index `i` maps to word `i / 64`, bit `i % 64`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct TileSet([u64; 6]);

impl TileSet {
    pub fn empty() -> Self { Self([0; 6]) }
    pub fn full()  -> Self { Self([u64::MAX; 6]) }

    #[inline]
    pub fn set(&mut self, idx: usize) {
        self.0[idx >> 6] |= 1u64 << (idx & 63);
    }

    #[inline]
    pub fn get(&self, idx: usize) -> bool {
        (self.0[idx >> 6] >> (idx & 63)) & 1 != 0
    }

    /// True when no tiles are visible (e.g., globe off-screen).
    pub fn is_empty(&self) -> bool { self.0.iter().all(|&w| w == 0) }

    /// True when this set is identical to `other`.  Used for change detection.
    pub fn same_as(&self, other: &Self) -> bool { self.0 == other.0 }

    /// Iterate over the indices of all set tiles.
    pub fn iter_set(&self) -> impl Iterator<Item = usize> + '_ {
        (0..TILE_COUNT).filter(|&i| self.get(i))
    }
}

// ─── Tile ────────────────────────────────────────────────────────────────────

/// Precomputed bounding sphere and geographic extent of one tile patch.
#[derive(Clone)]
pub struct Tile {
    /// Bounding-sphere centre in world space (unit-sphere surface).
    pub center: Vec3,
    /// Bounding-sphere radius (world space).  Slightly over-estimated for safety.
    pub radius: f32,
    /// Conservative geographic bounding box of the tile.
    pub geo:    GeoBounds,
}

// ─── TileGrid ────────────────────────────────────────────────────────────────

/// Precomputed tile grid covering the cube-sphere.
///
/// Construct once at startup with [`TileGrid::new`]; then call
/// [`TileGrid::visible_tiles`] every frame to obtain the visibility set.
pub struct TileGrid {
    tiles: Vec<Tile>,   // length == TILE_COUNT
}

impl TileGrid {
    /// Build the tile grid.  Computes bounding spheres for all 384 tiles.
    /// This is done once at startup — no per-frame allocation.
    pub fn new() -> Self {
        let mut tiles = Vec::with_capacity(TILE_COUNT);

        for face in 0u32..6 {
            for row in 0..TILES_PER_EDGE {
                for col in 0..TILES_PER_EDGE {
                    tiles.push(build_tile(face, row, col));
                }
            }
        }

        debug_assert_eq!(tiles.len(), TILE_COUNT);
        Self { tiles }
    }

    /// Compute the visibility set for this frame.
    ///
    /// A tile is visible when its bounding sphere:
    ///   - passes the horizon test (not occluded by the globe body), AND
    ///   - overlaps the view frustum.
    pub fn visible_tiles(&self, frustum: &Frustum, camera_eye: Vec3) -> TileSet {
        let mut set = TileSet::empty();
        let d = camera_eye.length();
        // Horizon threshold: cos(acos(1/d)) = 1/d.  A tile's centre is on the
        // near hemisphere when dot(centre, eye_dir) > 1/d.  We test against a
        // conservative threshold reduced by the tile radius to avoid culling
        // edge tiles prematurely.
        let eye_dir = camera_eye / d;
        let horizon_cos = 1.0 / d;

        for (i, tile) in self.tiles.iter().enumerate() {
            // Horizon cull: is the bounding sphere entirely behind the globe?
            // We pass if the centre could be visible, accounting for the radius.
            let dot = eye_dir.dot(tile.center);
            if dot + tile.radius < horizon_cos {
                continue; // entirely on back hemisphere
            }
            // Frustum cull
            if !frustum.contains_sphere(tile.center, tile.radius) {
                continue;
            }
            set.set(i);
        }
        set
    }

    /// Geographic bounding box of the union of all visible tiles.
    ///
    /// This replaces the conservative hemisphere cap used by `geo_bounds()`.
    pub fn geo_bounds_for(&self, visible: &TileSet) -> GeoBounds {
        if visible.is_empty() {
            return GeoBounds::global();
        }
        let mut min_lat =  90.0_f32;
        let mut max_lat = -90.0_f32;
        let mut min_lon =  180.0_f32;
        let mut max_lon = -180.0_f32;

        for i in visible.iter_set() {
            let g = &self.tiles[i].geo;
            min_lat = min_lat.min(g.min_lat);
            max_lat = max_lat.max(g.max_lat);
            min_lon = min_lon.min(g.min_lon);
            max_lon = max_lon.max(g.max_lon);
        }
        GeoBounds { min_lat, max_lat, min_lon, max_lon }
    }

    /// Geographic bounding box for a single tile (for data-source filtering).
    pub fn tile_geo(&self, idx: usize) -> &GeoBounds {
        &self.tiles[idx].geo
    }

    /// Return a reference to all precomputed tiles (used by LineLayer indexing).
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
}

impl Default for TileGrid {
    fn default() -> Self { Self::new() }
}

// ─── Tile construction helpers ────────────────────────────────────────────────

/// Map a cube-sphere face index (0–5) and normalised (s,t) ∈ [0,1] to a
/// 3-D position on the unit cube, before sphere projection.
fn face_cube_pos(face: u32, s: f32, t: f32) -> Vec3 {
    // s,t ∈ [0,1] → u,v ∈ [-1,1]
    let u = s * 2.0 - 1.0;
    let v = t * 2.0 - 1.0;
    match face {
        0 => Vec3::new( 1.0,   v,  -u), // PosX
        1 => Vec3::new(-1.0,   v,   u), // NegX
        2 => Vec3::new(  u,  1.0,  -v), // PosY
        3 => Vec3::new(  u, -1.0,   v), // NegY
        4 => Vec3::new(  u,   v,  1.0), // PosZ
        5 => Vec3::new( -u,   v, -1.0), // NegZ
        _ => unreachable!(),
    }
}

/// Project a cube point onto the unit sphere.
fn sphere_pos(cube: Vec3) -> Vec3 {
    cube.normalize()
}

/// Build one tile by sampling its four corner vertices (and a dense interior
/// grid for a tight bounding sphere) then computing the enclosing sphere.
fn build_tile(face: u32, row: u32, col: u32) -> Tile {
    let step = 1.0 / TILES_PER_EDGE as f32;
    let s0 = col as f32 * step;
    let s1 = s0 + step;
    let t0 = row as f32 * step;
    let t1 = t0 + step;

    // Sample a 3×3 grid inside the tile for a tight enclosing sphere.
    const SAMPLES: u32 = 3;
    let mut pts: Vec<Vec3> = Vec::with_capacity((SAMPLES * SAMPLES) as usize);
    for si in 0..SAMPLES {
        for ti in 0..SAMPLES {
            let s = s0 + (si as f32 + 0.5) / SAMPLES as f32 * (s1 - s0);
            let t = t0 + (ti as f32 + 0.5) / SAMPLES as f32 * (t1 - t0);
            pts.push(sphere_pos(face_cube_pos(face, s, t)));
        }
    }
    // Also include the four exact corners.
    for &(s, t) in &[(s0,t0),(s1,t0),(s0,t1),(s1,t1)] {
        pts.push(sphere_pos(face_cube_pos(face, s, t)));
    }

    // Bounding sphere: centre = normalised average, radius = max distance.
    let centroid = (pts.iter().copied().fold(Vec3::ZERO, |a, p| a + p)
        / pts.len() as f32)
        .normalize();
    let radius = pts.iter().map(|&p| (p - centroid).length()).fold(0.0_f32, f32::max)
        + 0.01; // small safety margin

    // Geographic bounds: min/max of corner and interior lat/lon.
    let geo_pts: Vec<(f32, f32)> = pts.iter()
        .map(|&p| (
            p.y.clamp(-1.0, 1.0).asin().to_degrees(),
            p.x.atan2(p.z).to_degrees(),
        ))
        .collect();

    let min_lat = geo_pts.iter().map(|&(lat,_)| lat).fold( 90.0_f32, f32::min);
    let max_lat = geo_pts.iter().map(|&(lat,_)| lat).fold(-90.0_f32, f32::max);
    let min_lon = geo_pts.iter().map(|&(_,lon)| lon).fold( 180.0_f32, f32::min);
    let max_lon = geo_pts.iter().map(|&(_,lon)| lon).fold(-180.0_f32, f32::max);

    Tile {
        center: centroid,
        radius,
        geo: GeoBounds { min_lat, max_lat, min_lon, max_lon },
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn tile_count() {
        let grid = TileGrid::new();
        assert_eq!(grid.tiles.len(), TILE_COUNT);
    }

    #[test]
    fn bounding_spheres_on_unit_sphere() {
        let grid = TileGrid::new();
        for tile in &grid.tiles {
            // All tile centres must lie on or very near the unit sphere.
            let len = tile.center.length();
            assert!((len - 1.0).abs() < 0.001, "centre not on unit sphere: {len}");
            // Radius must be positive and < 2 (can't span more than a hemisphere).
            assert!(tile.radius > 0.0 && tile.radius < 2.0);
        }
    }

    #[test]
    fn tileset_set_get() {
        let mut ts = TileSet::empty();
        ts.set(0);
        ts.set(383);
        ts.set(192);
        assert!(ts.get(0));
        assert!(ts.get(383));
        assert!(ts.get(192));
        assert!(!ts.get(1));
    }

    #[test]
    fn camera_above_north_pole_sees_roughly_half_tiles() {
        use crate::culling::Frustum;
        use crate::globe::camera::ArcballCamera;

        let mut cam = ArcballCamera::default();
        cam.radius = 2.5;
        cam.pitch  = 0.0;
        cam.yaw    = 0.0;

        let grid    = TileGrid::new();
        let frustum = Frustum::from_view_proj(cam.view_proj());
        let visible = grid.visible_tiles(&frustum, cam.eye());

        let count = visible.iter_set().count();
        // At radius 2.5 with 45° FoV we expect well under half the 384 tiles.
        assert!(count > 0,   "at least one tile must be visible");
        assert!(count < 250, "should cull majority of tiles, got {count}");
    }
}
