//! Auto-downloaded Natural Earth line datasets: rivers, lakes (outlines), railroads.
//!
//! Data is fetched from the Natural Earth GeoJSON mirror on GitHub and cached
//! locally for 30 days — the same strategy as `world_map.rs`.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context as _;
use tokio::{runtime::Handle, sync::watch};

use crate::geo::GeoFeature;

const RIVERS_URL: &str =
    "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_rivers_lake_centerlines.geojson";

const LAKES_URL: &str =
    "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_lakes.geojson";

const RAILROADS_URL: &str =
    "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_railroads.geojson";

const CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

// ─── NaturalEarthManager ──────────────────────────────────────────────────────

pub struct NaturalEarthManager {
    pub rivers_rx:    watch::Receiver<Vec<GeoFeature>>,
    pub lakes_rx:     watch::Receiver<Vec<GeoFeature>>,
    pub railroads_rx: watch::Receiver<Vec<GeoFeature>>,
}

impl NaturalEarthManager {
    pub fn new(rt: Handle) -> Self {
        let (rivers_tx,    rivers_rx)    = watch::channel(Vec::<GeoFeature>::new());
        let (lakes_tx,     lakes_rx)     = watch::channel(Vec::<GeoFeature>::new());
        let (railroads_tx, railroads_rx) = watch::channel(Vec::<GeoFeature>::new());

        rt.spawn(async move {
            match fetch_lines(RIVERS_URL, "ne_10m_rivers.geojson").await {
                Ok(data) => { let _ = rivers_tx.send(data); }
                Err(e)   => tracing::error!("NE rivers: {e:#}"),
            }
        });
        rt.spawn(async move {
            match fetch_lines(LAKES_URL, "ne_10m_lakes.geojson").await {
                Ok(data) => { let _ = lakes_tx.send(data); }
                Err(e)   => tracing::error!("NE lakes: {e:#}"),
            }
        });
        rt.spawn(async move {
            match fetch_lines(RAILROADS_URL, "ne_10m_railroads.geojson").await {
                Ok(data) => { let _ = railroads_tx.send(data); }
                Err(e)   => tracing::error!("NE railroads: {e:#}"),
            }
        });

        Self { rivers_rx, lakes_rx, railroads_rx }
    }
}

// ─── Data loading ─────────────────────────────────────────────────────────────

async fn fetch_lines(url: &str, cache_file: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let path = cache_path(cache_file)?;
    let text = fetch_cached(url, &path).await?;
    crate::topo::parse_geojson_text(&text)
}

fn cache_path(filename: &str) -> anyhow::Result<PathBuf> {
    crate::platform_paths::cache_file("", filename)
}

async fn fetch_cached(url: &str, path: &Path) -> anyhow::Result<String> {
    if let Some(text) = crate::platform_paths::read_if_fresh(path, CACHE_TTL) {
        tracing::debug!(path = %path.display(), "NE: loading from cache");
        return Ok(text);
    }

    tracing::warn!(url, "downloading Natural Earth data…");
    match reqwest::get(url).await {
        Ok(resp) => {
            let text = resp
                .error_for_status().context("HTTP error")?
                .text().await.context("reading response body")?;
            if let Err(e) = crate::platform_paths::write_atomic(path, text.as_bytes()) {
                tracing::warn!("could not cache {}: {e}", path.display());
            }
            Ok(text)
        }
        // Offline: serve the last cached copy regardless of age.
        Err(net_err) => match crate::platform_paths::read_stale(path) {
            Some(text) => {
                tracing::warn!(url, "NE: network unreachable, serving stale cache");
                Ok(text)
            }
            None => Err(anyhow::Error::new(net_err)
                .context("GET request failed (no cached Natural Earth data)")),
        },
    }
}
