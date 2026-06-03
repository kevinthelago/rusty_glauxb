use std::{path::PathBuf, time::Duration};

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
///
/// Hardened for offline use: a fresh-but-corrupt cache is ignored and refetched,
/// only valid GeoJSON is cached (atomically, so a crash mid-write can't truncate
/// it), and when the network is unreachable the last cached copy is served
/// regardless of age.
pub async fn fetch(
    name:       &str,
    url:        &str,
    cache_secs: u64,
) -> anyhow::Result<Vec<GeoFeature>> {
    let path = cache_path(name)?;

    // 1. Fresh, parseable cache → use it.  A corrupt cache is logged and
    //    refetched rather than surfaced as an error.
    if cache_secs > 0 {
        if let Some(text) =
            crate::platform_paths::read_if_fresh(&path, Duration::from_secs(cache_secs))
        {
            match crate::topo::parse_geojson_text(&text) {
                Ok(features) => {
                    tracing::info!(name, "download: reading from cache");
                    return Ok(features);
                }
                Err(e) => {
                    tracing::warn!(name, error = %e, "download: cached file corrupt, refetching");
                }
            }
        }
    }

    // 2. Network fetch.
    tracing::info!(name, url, "download: fetching…");
    match reqwest::get(url).await {
        Ok(resp) => {
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!("rate limited (429) fetching {url}");
            }
            let text = resp
                .error_for_status()
                .context("HTTP error")?
                .text()
                .await
                .context("reading response body")?;

            // Parse before caching so garbage is never persisted.
            let features = crate::topo::parse_geojson_text(&text)?;
            if cache_secs > 0 {
                if let Err(e) = crate::platform_paths::write_atomic(&path, text.as_bytes()) {
                    tracing::warn!("could not cache download '{name}': {e}");
                }
            }
            Ok(features)
        }
        // 3. Transport failure (offline) → serve a stale cache if usable.
        Err(net_err) => {
            if let Some(text) = crate::platform_paths::read_stale(&path) {
                if let Ok(features) = crate::topo::parse_geojson_text(&text) {
                    tracing::warn!(name, "download: network unreachable, serving stale cache");
                    return Ok(features);
                }
            }
            Err(anyhow::Error::new(net_err)
                .context(format!("HTTP GET failed for '{name}' (no usable cache)")))
        }
    }
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
    crate::platform_paths::cache_file("downloads", &format!("{name}.geojson"))
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
    crate::platform_paths::data_file("active_downloads.json")
}
