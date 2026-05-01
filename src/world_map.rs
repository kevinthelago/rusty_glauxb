//! World-map layer management: async Natural Earth data loading, disk caching,
//! and administrative-hierarchy state.
//!
//! Data is fetched from the Natural Earth GeoJSON mirror on GitHub and cached
//! locally so subsequent runs are instant:
//!   countries:  ne_110m_admin_0_countries.geojson  (~850 KB)
//!   states:     ne_50m_admin_1_states_provinces.geojson  (~3 MB)

use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use anyhow::Context as _;
use tokio::{runtime::Handle, sync::watch};

use crate::geo::{AdminLevel, GeoPolygon};

const COUNTRIES_URL: &str =
    "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/\
     master/geojson/ne_110m_admin_0_countries.geojson";

const STATES_URL: &str =
    "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/\
     master/geojson/ne_50m_admin_1_states_provinces.geojson";

/// Cache TTL: re-download if the file is older than this.
const CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 3600); // 30 days

// ─── WorldMapManager ─────────────────────────────────────────────────────────

pub struct WorldMapManager {
    rt:              Handle,
    pub level:       AdminLevel,
    pub countries_rx: watch::Receiver<Vec<GeoPolygon>>,
    pub states_rx:   Option<watch::Receiver<Vec<GeoPolygon>>>,
    loaded_states_for: Option<String>,
}

impl WorldMapManager {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = watch::channel(Vec::new());
        rt.spawn(async move {
            match load_countries().await {
                Ok(data)  => { let _ = tx.send(data); }
                Err(e)    => tracing::error!("world map countries: {e:#}"),
            }
        });
        Self {
            rt,
            level: AdminLevel::Country,
            countries_rx: rx,
            states_rx: None,
            loaded_states_for: None,
        }
    }

    /// Trigger a (possibly cached) load of states for `country_iso`.
    /// No-op if already loaded for the same country.
    pub fn load_states(&mut self, country_iso: &str) {
        if self.loaded_states_for.as_deref() == Some(country_iso) {
            return;
        }
        self.loaded_states_for = Some(country_iso.to_string());
        let (tx, rx) = watch::channel(Vec::new());
        self.states_rx = Some(rx);
        let iso = country_iso.to_string();
        self.rt.spawn(async move {
            match load_states_for(&iso).await {
                Ok(data)  => { let _ = tx.send(data); }
                Err(e)    => tracing::error!("world map states ({iso}): {e:#}"),
            }
        });
    }

    /// Navigate up one administrative level.
    pub fn go_back(&mut self) -> bool {
        match &self.level {
            AdminLevel::Country => false,
            AdminLevel::State(_) => {
                self.level = AdminLevel::Country;
                self.states_rx = None;
                self.loaded_states_for = None;
                true
            }
        }
    }
}

// ─── Data loading ─────────────────────────────────────────────────────────────

async fn load_countries() -> anyhow::Result<Vec<GeoPolygon>> {
    let path = cache_path("ne_110m_countries.geojson")?;
    let text = fetch_cached(COUNTRIES_URL, &path).await?;
    parse_countries(&text)
}

async fn load_states_for(country_iso: &str) -> anyhow::Result<Vec<GeoPolygon>> {
    let path = cache_path("ne_50m_admin1.geojson")?;
    let text = fetch_cached(STATES_URL, &path).await?;
    parse_states(&text, country_iso)
}

// ─── Cache helpers ────────────────────────────────────────────────────────────

fn cache_path(filename: &str) -> anyhow::Result<PathBuf> {
    // On Android, dirs::data_local_dir() returns None; fall back to the
    // process working directory which NativeActivity sets to internal storage.
    let base = dirs::data_local_dir()
        .or_else(|| std::env::current_dir().ok())
        .context("cannot locate local data directory")?;
    let dir = base.join("rusty_glauxb").join("cache");
    std::fs::create_dir_all(&dir)
        .context("failed to create cache directory")?;
    Ok(dir.join(filename))
}

async fn fetch_cached(url: &str, path: &PathBuf) -> anyhow::Result<String> {
    // Use disk cache if fresh enough.
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(modified) = meta.modified() {
            if SystemTime::now().duration_since(modified).unwrap_or(CACHE_TTL) < CACHE_TTL {
                tracing::debug!(path = %path.display(), "loading from cache");
                return std::fs::read_to_string(path)
                    .context("reading cached file");
            }
        }
    }

    tracing::warn!(url, "downloading Natural Earth data…");
    let text = reqwest::get(url)
        .await
        .context("GET request failed")?
        .error_for_status()
        .context("HTTP error")?
        .text()
        .await
        .context("reading response body")?;

    // Save to cache (best-effort; don't fail if write fails).
    if let Err(e) = std::fs::write(path, &text) {
        tracing::warn!("could not cache {}: {e}", path.display());
    }

    Ok(text)
}

// ─── GeoJSON parsers ──────────────────────────────────────────────────────────

fn parse_countries(text: &str) -> anyhow::Result<Vec<GeoPolygon>> {
    let fc: serde_json::Value = serde_json::from_str(text)
        .context("countries JSON parse")?;
    let features = fc["features"].as_array()
        .context("missing features array")?;

    let mut result = Vec::with_capacity(features.len());
    for feat in features {
        let props = &feat["properties"];
        let name = string_prop(props, &["ADMIN", "NAME", "name"])
            .unwrap_or_else(|| "Unknown".to_string());
        let id = string_prop(props, &["ISO_A2", "ADM0_A3", "iso_a2"])
            .filter(|s| s != "-99")
            .unwrap_or_else(|| name.chars().take(3).collect());

        let Some(polygon) = parse_geometry(&feat["geometry"]) else { continue };
        let centroid = centroid_of(&polygon);
        result.push(GeoPolygon { id, name, parent: String::new(), centroid, polygons: polygon });
    }
    tracing::warn!(count = result.len(), "parsed countries");
    Ok(result)
}

fn parse_states(text: &str, country_iso: &str) -> anyhow::Result<Vec<GeoPolygon>> {
    let fc: serde_json::Value = serde_json::from_str(text)
        .context("states JSON parse")?;
    let features = fc["features"].as_array()
        .context("missing features array")?;

    let iso_up = country_iso.to_uppercase();
    let mut result = Vec::new();
    for feat in features {
        let props = &feat["properties"];
        let parent = string_prop(props, &["iso_a2", "adm0_a3", "ADM0_A3"])
            .unwrap_or_default()
            .to_uppercase();
        // Also check "admin" field for country name matching as fallback
        if parent != iso_up { continue; }

        let name = string_prop(props, &["name", "NAME", "gn_name"])
            .unwrap_or_else(|| "Unknown".to_string());
        let id = string_prop(props, &["iso_3166_2", "code_hasc", "fips"])
            .unwrap_or_else(|| format!("{}-{}", iso_up, &name.chars().take(3).collect::<String>()));

        let Some(polygon) = parse_geometry(&feat["geometry"]) else { continue };
        let centroid = centroid_of(&polygon);
        result.push(GeoPolygon {
            id,
            name,
            parent: country_iso.to_string(),
            centroid,
            polygons: polygon,
        });
    }
    tracing::warn!(country = country_iso, count = result.len(), "parsed states");
    Ok(result)
}

/// Extract a string property, trying multiple key names in order.
fn string_prop(props: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = props.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() && v != "null" {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Parse a GeoJSON `Polygon` or `MultiPolygon` geometry into our ring format.
/// GeoJSON uses `[lon, lat]`; we convert to `[lat, lon]`.
fn parse_geometry(geom: &serde_json::Value) -> Option<Vec<Vec<Vec<[f32; 2]>>>> {
    let typ   = geom["type"].as_str()?;
    let coords = &geom["coordinates"];

    match typ {
        "Polygon" => {
            let rings = parse_polygon_rings(coords)?;
            Some(vec![rings])
        }
        "MultiPolygon" => {
            let polys = coords.as_array()?;
            let mut result = Vec::with_capacity(polys.len());
            for poly in polys {
                if let Some(rings) = parse_polygon_rings(poly) {
                    result.push(rings);
                }
            }
            Some(result)
        }
        _ => None,
    }
}

fn parse_polygon_rings(coords: &serde_json::Value) -> Option<Vec<Vec<[f32; 2]>>> {
    let rings = coords.as_array()?;
    let mut result = Vec::with_capacity(rings.len());
    for ring in rings {
        let pts = ring.as_array()?;
        let mut ring_pts = Vec::with_capacity(pts.len());
        for pt in pts {
            let arr = pt.as_array()?;
            let lon = arr.first()?.as_f64()? as f32;
            let lat = arr.get(1)?.as_f64()? as f32;
            ring_pts.push([lat, lon]); // convert to [lat, lon]
        }
        if ring_pts.len() >= 3 {
            result.push(ring_pts);
        }
    }
    Some(result)
}

/// Simple centroid: average of the outer ring of the largest polygon.
fn centroid_of(polygons: &[Vec<Vec<[f32; 2]>>]) -> [f32; 2] {
    // Pick the polygon with the most outer-ring vertices as a proxy for largest.
    let outer = polygons.iter()
        .filter_map(|p| p.first())
        .max_by_key(|r| r.len());

    let Some(outer) = outer else { return [0.0, 0.0] };
    let n = outer.len() as f64;
    if n == 0.0 { return [0.0, 0.0]; }
    let (lat, lon) = outer.iter().fold((0.0_f64, 0.0_f64), |acc, &[la, lo]| {
        (acc.0 + la as f64, acc.1 + lo as f64)
    });
    [(lat / n) as f32, (lon / n) as f32]
}
