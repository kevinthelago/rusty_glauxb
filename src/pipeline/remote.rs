//! Remote GeoJSON [`DataSource`] driving a user-defined pipeline.
//!
//! A [`RemoteGeoJsonSource`] wraps a [`PipelineConfig`] and plugs straight into
//! the existing [`crate::pipeline::PipelineManager`] /
//! [`crate::pipeline::QueryScheduler`].  The pipeline's [`Cadence`] decides how
//! often the scheduler fires the fetch (via [`DataSource::max_staleness_secs`]),
//! so static and dynamic pipelines share one implementation:
//!
//! - **static** → `max_staleness_secs` ≥ 1 day, so the scheduler only refetches
//!   after the cache expires or on a manual `force_refresh`.
//! - **dynamic** → `max_staleness_secs` = the user interval, so it refetches on
//!   that cadence.
//!
//! The whole dataset is fetched and returned; the renderer culls per frame, so
//! [`DataSource::min_change_fraction`] is `1.0` (panning never triggers a
//! refetch — only the cadence does).
//!
//! A disk cache (`<cache>/pipelines/<id>.geojson`) backs every pipeline so the
//! data survives restarts and remains available offline.  When the network is
//! unreachable the fetch falls back to the last cached copy regardless of age.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use anyhow::Context as _;
use async_trait::async_trait;

use crate::geo::{GeoBounds, GeoFeature};
use crate::pipeline::config::PipelineConfig;
use crate::pipeline::rate_limit::{extract_retry_after, RateLimitError};
use crate::pipeline::DataSource;

/// HTTP timeout for a single pipeline fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(45);

/// Fallback backoff when a 429 response carries no `Retry-After` header.
const DEFAULT_BACKOFF: Duration = Duration::from_secs(60);

/// A user pipeline backed by a remote GeoJSON URL.
pub struct RemoteGeoJsonSource {
    config: PipelineConfig,
    client: reqwest::Client,
}

impl RemoteGeoJsonSource {
    /// Build a source for `config`.  The id must already be validated by
    /// [`PipelineConfig::new`].
    pub fn new(config: PipelineConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent("rusty_glauxb/0.1")
            .build()
            .expect("HTTP client build");
        Self { config, client }
    }

    /// Path to this pipeline's on-disk GeoJSON cache.
    pub fn cache_path(&self) -> anyhow::Result<std::path::PathBuf> {
        crate::platform_paths::cache_file("pipelines", &format!("{}.geojson", self.config.id))
    }

    /// How long a cached copy is considered fresh before a network refetch.
    /// Mirrors the cadence staleness so a static pipeline doesn't re-download a
    /// daily dataset on every scheduler tick.
    fn cache_ttl(&self) -> Duration {
        Duration::from_secs_f32(self.config.max_staleness_secs().max(1.0))
    }
}

#[async_trait]
impl DataSource for RemoteGeoJsonSource {
    fn name(&self) -> &str {
        &self.config.name
    }
    fn layer_name(&self) -> &str {
        &self.config.id
    }
    fn max_staleness_secs(&self) -> f32 {
        self.config.max_staleness_secs()
    }
    fn min_change_fraction(&self) -> f32 {
        1.0
    }

    async fn fetch(&self, _bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        let path = self.cache_path()?;

        // 1. Serve a fresh cache without touching the network.
        if let Some(text) = read_fresh_cache(&path, self.cache_ttl()) {
            tracing::debug!(pipeline = %self.config.id, "remote: cache hit");
            return crate::topo::parse_geojson_text(&text);
        }

        // 2. Fetch from the network.
        match self.client.get(&self.config.url).send().await {
            Ok(resp) => {
                if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let delay = extract_retry_after(resp.headers(), DEFAULT_BACKOFF);
                    return Err(RateLimitError { retry_after: delay }.into());
                }
                let resp = resp.error_for_status().context("HTTP error")?;
                let text = resp.text().await.context("reading response body")?;
                write_cache(&path, &text);
                crate::topo::parse_geojson_text(&text)
            }
            // 3. Offline / transport failure: fall back to a stale cache if any.
            Err(net_err) => match read_any_cache(&path) {
                Some(text) => {
                    tracing::warn!(
                        pipeline = %self.config.id,
                        "remote: network unreachable, serving stale cache"
                    );
                    crate::topo::parse_geojson_text(&text)
                }
                None => Err(anyhow::Error::new(net_err).context(format!(
                    "fetching pipeline '{}' (no cache available offline)",
                    self.config.id
                ))),
            },
        }
    }
}

// ─── Cache helpers (sync, testable) ─────────────────────────────────────────────

/// Read the cache file at `path` if it exists and is younger than `ttl`.
fn read_fresh_cache(path: &Path, ttl: Duration) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or(ttl);
    if age < ttl {
        std::fs::read_to_string(path).ok()
    } else {
        None
    }
}

/// Read the cache file regardless of age; used as the offline fallback.
/// Treats an empty file as absent so a truncated write never yields `[]`.
fn read_any_cache(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Best-effort cache write; logs but does not fail on I/O error.
fn write_cache(path: &Path, text: &str) {
    if let Err(e) = std::fs::write(path, text) {
        tracing::warn!(path = %path.display(), error = %e, "remote: cache write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::config::Cadence;

    fn cfg(id: &str) -> PipelineConfig {
        PipelineConfig::new(id, "Test", "https://example.com/a.geojson", Cadence::Static).unwrap()
    }

    fn unique_tmp(name: &str) -> std::path::PathBuf {
        // Process id keeps parallel test runs from colliding without needing a
        // random source (unavailable here).
        std::env::temp_dir().join(format!("glauxb_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn source_exposes_config_scheduler_knobs() {
        let src = RemoteGeoJsonSource::new(cfg("roads"));
        assert_eq!(src.name(), "Test");
        assert_eq!(src.layer_name(), "roads");
        assert!(src.max_staleness_secs() >= 24.0 * 3600.0);
        assert_eq!(src.min_change_fraction(), 1.0);
    }

    #[test]
    fn dynamic_source_uses_interval_for_staleness() {
        let c = PipelineConfig::new(
            "live",
            "Live",
            "https://e.com/a.geojson",
            Cadence::Dynamic {
                interval_secs: 12.0,
            },
        )
        .unwrap();
        let src = RemoteGeoJsonSource::new(c);
        assert_eq!(src.max_staleness_secs(), 12.0);
        assert_eq!(src.cache_ttl(), Duration::from_secs(12));
    }

    #[test]
    fn fresh_cache_is_read_stale_is_not() {
        let path = unique_tmp("fresh.geojson");
        std::fs::write(&path, "hello").unwrap();
        // Just written → fresh under a 1h ttl.
        assert_eq!(
            read_fresh_cache(&path, Duration::from_secs(3600)).as_deref(),
            Some("hello")
        );
        // Zero ttl → always stale.
        assert!(read_fresh_cache(&path, Duration::from_secs(0)).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn any_cache_falls_back_but_skips_empty() {
        let path = unique_tmp("any.geojson");
        assert!(read_any_cache(&path).is_none()); // missing
        std::fs::write(&path, "   \n").unwrap();
        assert!(read_any_cache(&path).is_none()); // empty/whitespace
        std::fs::write(&path, "{\"type\":\"FeatureCollection\"}").unwrap();
        assert!(read_any_cache(&path).is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_cache_round_trips() {
        let path = unique_tmp("write.geojson");
        write_cache(&path, "payload");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "payload");
        let _ = std::fs::remove_file(&path);
    }
}
