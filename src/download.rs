use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use anyhow::Context as _;

use crate::geo::GeoFeature;

// ─── Catalog ──────────────────────────────────────────────────────────────────

pub struct CatalogEntry {
    pub name:        &'static str,
    pub description: &'static str,
    pub url:         &'static str,
    /// Seconds to keep the cached file before re-downloading.  0 = never cache.
    pub cache_secs:  u64,
}

pub static CATALOG: &[CatalogEntry] = &[
    // ── Static geography ──────────────────────────────────────────────────────
    CatalogEntry {
        name:        "graticules",
        description: "10° lat/lon coordinate grid (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_graticules_10.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "coastlines",
        description: "High-detail coastlines (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_coastline.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "tectonic",
        description: "Tectonic plate boundaries (Fraxen/PB2002)",
        url:         "https://raw.githubusercontent.com/fraxen/tectonicplates/master/GeoJSON/PB2002_boundaries.geojson",
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "timezones",
        description: "Timezone boundaries (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_time_zones.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "glaciers",
        description: "Glaciated areas (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_glaciated_areas.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "reefs",
        description: "Coral reefs and atolls (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_reefs.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "urban",
        description: "World urban areas (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_urban_areas.geojson"),
        cache_secs: 365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "protected_areas",
        description: "National parks and protected lands (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_parks_and_protected_lands_area.geojson"),
        cache_secs: 365 * 24 * 3600,
    },

    // ── Infrastructure / points of interest ───────────────────────────────────
    CatalogEntry {
        name:        "airports",
        description: "World airports (Natural Earth, point data)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_airports.geojson"),
        cache_secs: 30 * 24 * 3600,
    },
    CatalogEntry {
        name:        "ports",
        description: "World seaports (Natural Earth, point data)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_ports.geojson"),
        cache_secs: 30 * 24 * 3600,
    },
    CatalogEntry {
        name:        "populated_places",
        description: "World cities and towns (Natural Earth, point data)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_populated_places.geojson"),
        cache_secs: 30 * 24 * 3600,
    },
    CatalogEntry {
        name:        "submarine_cables",
        description: "Undersea internet cable routes (TeleGeography)",
        url:         "https://raw.githubusercontent.com/telegeography/www.submarinecablemap.com/master/web/public/api/v3/cable/cable-geo.json",
        cache_secs: 30 * 24 * 3600,
    },

    // ── Physical geography / topology ────────────────────────────────────────
    CatalogEntry {
        name:        "mountain_peaks",
        description: "Named mountain summits with elevation (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_geography_regions_elevation_points.geojson"),
        cache_secs:  365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "geographic_lines",
        description: "Equator, tropics, arctic/antarctic circles, date line (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_geographic_lines.geojson"),
        cache_secs:  365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "physical_regions",
        description: "Named geographic regions — plains, deserts, basins (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_geography_regions_polys.geojson"),
        cache_secs:  365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "marine_regions",
        description: "Named ocean and sea areas (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_geography_marine_polys.geojson"),
        cache_secs:  365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "bathymetry_200",
        description: "Ocean depth contour at 200 m — continental shelves (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_bathymetry_K_200.geojson"),
        cache_secs:  365 * 24 * 3600,
    },
    CatalogEntry {
        name:        "bathymetry_1000",
        description: "Ocean depth contour at 1 000 m (Natural Earth)",
        url:         concat!("https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson", "/ne_10m_bathymetry_J_1000.geojson"),
        cache_secs:  365 * 24 * 3600,
    },

    // ── Natural hazards ───────────────────────────────────────────────────────
    CatalogEntry {
        name:        "earthquakes",
        description: "USGS significant earthquakes — past 30 days (point data)",
        url:         "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/significant_month.geojson",
        cache_secs:  3600,
    },
    CatalogEntry {
        name:        "earthquakes_all",
        description: "USGS M2.5+ earthquakes — past 7 days (point data)",
        url:         "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_week.geojson",
        cache_secs:  3600,
    },
    CatalogEntry {
        name:        "volcanoes",
        description: "Active volcanoes worldwide — Smithsonian GVP (point data)",
        url:         "https://raw.githubusercontent.com/nicholasmolefe/volcanoes/master/volcanoes.geojson",
        cache_secs: 7 * 24 * 3600,
    },
];


/// Look up a catalog entry by name (case-insensitive).
pub fn find(name: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.name.eq_ignore_ascii_case(name))
}

/// Fetch a GeoJSON dataset, using a disk cache when `cache_secs > 0`.
pub async fn fetch(
    name:       &str,
    url:        &str,
    cache_secs: u64,
) -> anyhow::Result<Vec<GeoFeature>> {
    let path = cache_path(name)?;

    if cache_secs > 0 {
        let ttl = Duration::from_secs(cache_secs);
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                let age = SystemTime::now().duration_since(modified).unwrap_or(ttl);
                if age < ttl {
                    tracing::info!(name, "download: reading from cache");
                    let text = std::fs::read_to_string(&path)
                        .context("reading cached GeoJSON")?;
                    return crate::topo::parse_geojson_text(&text);
                }
            }
        }
    }

    tracing::info!(name, url, "download: fetching…");
    let resp = reqwest::get(url)
        .await
        .context("HTTP GET failed")?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        anyhow::bail!("rate limited (429) fetching {url}");
    }

    let text = resp
        .error_for_status()
        .context("HTTP error")?
        .text()
        .await
        .context("reading response body")?;

    if cache_secs > 0 {
        if let Err(e) = std::fs::write(&path, &text) {
            tracing::warn!("could not cache download '{name}': {e}");
        }
    }

    crate::topo::parse_geojson_text(&text)
}

/// Return `true` if a non-empty cache file exists for `name`.
pub fn is_cached(name: &str) -> bool {
    cache_path(name)
        .map(|p| p.exists() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
        .unwrap_or(false)
}

/// Delete the cache file for `name`.  No-op if it doesn't exist.
pub fn clear_cache(name: &str) {
    if let Ok(path) = cache_path(name) {
        let _ = std::fs::remove_file(&path);
    }
}

fn cache_path(name: &str) -> anyhow::Result<PathBuf> {
    let dir = data_dir()?.join("cache").join("downloads");
    std::fs::create_dir_all(&dir).context("creating downloads cache directory")?;
    Ok(dir.join(format!("{name}.geojson")))
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(|| std::env::current_dir().ok())
        .context("cannot locate local data directory")?;
    Ok(base.join("rusty_glauxb"))
}

// ─── Session persistence ──────────────────────────────────────────────────────

/// A download layer that should be reloaded on the next session.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SavedLayer {
    pub name: String,
    /// Custom URL for non-catalog downloads; `None` for catalog entries
    /// (URL is re-resolved from the catalog at load time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub cache_secs: u64,
}

/// Write the active-download list to disk.  Silently ignores I/O errors.
pub fn save_state(layers: &[SavedLayer]) {
    let Ok(path) = state_path() else { return };
    if let Ok(json) = serde_json::to_string_pretty(layers) {
        let _ = std::fs::write(&path, json);
    }
}

/// Read the active-download list saved by the previous session.
/// Returns an empty list on any error.
pub fn load_state() -> Vec<SavedLayer> {
    let Ok(path) = state_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    serde_json::from_str(&text).unwrap_or_default()
}

fn state_path() -> anyhow::Result<PathBuf> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).context("creating data directory")?;
    Ok(dir.join("active_downloads.json"))
}
