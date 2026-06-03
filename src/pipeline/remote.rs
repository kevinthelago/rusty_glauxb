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

use std::time::Duration;

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
        if let Some(text) = crate::platform_paths::read_if_fresh(&path, self.cache_ttl()) {
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
                // Parse before caching so a corrupt download never poisons the
                // cache; only valid GeoJSON is persisted (atomically).
                let features = crate::topo::parse_geojson_text(&text)?;
                if let Err(e) = crate::platform_paths::write_atomic(&path, text.as_bytes()) {
                    tracing::warn!(pipeline = %self.config.id, error = %e, "remote: cache write failed");
                }
                Ok(features)
            }
            // 3. Offline / transport failure: fall back to a stale cache if any.
            Err(net_err) => match crate::platform_paths::read_stale(&path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::config::Cadence;

    fn cfg(id: &str) -> PipelineConfig {
        PipelineConfig::new(id, "Test", "https://example.com/a.geojson", Cadence::Static).unwrap()
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
}
