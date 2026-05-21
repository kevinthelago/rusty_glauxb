//! Data pipeline — fetches geographic features for the visible globe region
//! and distributes them to the appropriate data layers.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Main render loop (every frame)                              │
//! │                                                             │
//! │  camera.eye()                                               │
//! │       │                                                     │
//! │       ▼                                                     │
//! │  GeoBounds::from_eye()  ──►  PipelineManager::tick(bounds) │
//! │                                       │                     │
//! │                          QueryScheduler: changed enough?    │
//! │                                       │  yes               │
//! │                                       ▼                     │
//! │                          tokio::spawn per DataSource        │
//! │                            DataSource::fetch(bounds)        │
//! │                                       │                     │
//! │                            watch::Sender<Vec<GeoFeature>>   │
//! │                                       │                     │
//! │  DataLayer::update()  ◄───  watch::Receiver (per source)   │
//! │       │                                                     │
//! │       ▼                                                     │
//! │  GPU instance buffer upload + frustum/horizon cull          │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Adding a new data source
//!
//! 1. Implement [`DataSource`] for your type.
//! 2. Call [`PipelineManager::register`] at startup with your source and the
//!    layer name it should populate.
//! 3. The corresponding [`crate::layers`] `DataLayer` implementation calls
//!    [`PipelineManager::subscribe`] once to obtain its `watch::Receiver`.
//!
//! ## Fetch triggering
//!
//! [`QueryScheduler`] debounces refetch requests so the pipeline fires when:
//! - The visible bounds changed by more than `min_change_fraction` (default
//!   25 % of the current span), **or**
//! - More than `max_staleness_secs` have elapsed since the last fetch (default
//!   30 s for live feeds, 300 s for static/cached sources).

pub mod flights;
pub mod osm;
pub mod rate_limit;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{runtime::Handle, sync::{mpsc, watch}};

use rate_limit::RateLimitError;

use crate::geo::{GeoBounds, GeoFeature};

// ─── DataSource trait ─────────────────────────────────────────────────────────

/// A single geographic data provider.
///
/// Implementations range from simple in-memory GeoJSON readers to live REST
/// APIs and WebSocket feeds.  The trait is object-safe so sources can be
/// stored as `Arc<dyn DataSource>`.
///
/// Every implementation **must** be `Send + Sync` so it can be called from
/// a `tokio::spawn` task.
#[async_trait::async_trait]
pub trait DataSource: Send + Sync {
    /// Human-readable name used for logging and the HUD layer list.
    fn name(&self) -> &str;

    /// Which [`crate::layers`] layer this source populates.
    /// Must match the layer's `DataLayer::name()`.
    fn layer_name(&self) -> &str;

    /// Maximum age of a cached result before a forced refetch (seconds).
    /// Return `f32::INFINITY` for purely static sources that never need
    /// refreshing after the first load.
    fn max_staleness_secs(&self) -> f32 { 30.0 }

    /// Minimum fraction of the current viewport span that must change before
    /// a pan/zoom triggers a new fetch.  0.0 = always refetch on any movement;
    /// 1.0 = never refetch on pan/zoom alone.
    fn min_change_fraction(&self) -> f32 { 0.25 }

    /// Fetch all features within `bounds`.
    ///
    /// Called from a background `tokio::spawn` task — blocking here is safe
    /// (use `tokio::task::spawn_blocking` for CPU-bound or sync I/O work).
    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>>;
}

// ─── QueryScheduler ───────────────────────────────────────────────────────────

/// Per-source state that decides when to trigger a new fetch.
struct QueryScheduler {
    last_bounds: Option<GeoBounds>,
    last_fetch:  Instant,
    max_staleness: Duration,
    min_change_fraction: f32,
    /// Hard floor between any two fetches regardless of viewport movement.
    min_cooldown: Duration,
    /// When set, suppresses all fetches until this instant (rate-limit backoff).
    backoff_until: Option<Instant>,
    /// Bypasses the cooldown floor for the very first fetch.
    force_next: bool,
}

impl QueryScheduler {
    fn new(max_staleness_secs: f32, min_change_fraction: f32) -> Self {
        Self {
            last_bounds:         None,
            last_fetch:          Instant::now(),
            max_staleness:       Duration::from_secs_f32(max_staleness_secs),
            min_change_fraction,
            min_cooldown:        Duration::from_secs(60),
            backoff_until:       None,
            force_next:          true,
        }
    }

    fn with_cooldown(mut self, secs: u64) -> Self {
        self.min_cooldown = Duration::from_secs(secs);
        self
    }

    /// Suppress fetches for at least `delay` from now.  Takes the maximum if
    /// a backoff is already in effect.
    fn apply_backoff(&mut self, delay: Duration) {
        let until = Instant::now() + delay;
        self.backoff_until = Some(match self.backoff_until {
            Some(existing) => existing.max(until),
            None           => until,
        });
        // Also push last_fetch forward so staleness doesn't immediately override the backoff.
        self.last_fetch = Instant::now();
    }

    /// Returns `true` and records the new bounds if a fetch should fire.
    fn should_fetch(&mut self, bounds: &GeoBounds) -> bool {
        let now = Instant::now();

        // Rate-limit backoff takes top priority.
        if let Some(until) = self.backoff_until {
            if now < until {
                return false;
            }
            self.backoff_until = None;
        }

        // Force the very first fetch regardless of cooldown.
        if self.force_next {
            self.force_next  = false;
            self.last_bounds = Some(bounds.clone());
            self.last_fetch  = now;
            return true;
        }

        let elapsed = now.duration_since(self.last_fetch);

        // Never fetch faster than min_cooldown, regardless of other conditions.
        if elapsed < self.min_cooldown {
            return false;
        }

        // Always fetch on first call or after staleness timeout
        if self.last_bounds.is_none() || elapsed >= self.max_staleness {
            self.last_bounds = Some(bounds.clone());
            self.last_fetch  = now;
            return true;
        }

        // Fetch if the viewport moved significantly
        if let Some(last) = &self.last_bounds {
            if bounds.change_fraction(last) >= self.min_change_fraction {
                self.last_bounds = Some(bounds.clone());
                self.last_fetch  = now;
                return true;
            }
        }

        false
    }
}

fn make_scheduler(source: &dyn DataSource) -> QueryScheduler {
    let cooldown_secs = if source.max_staleness_secs() >= 3600.0 {
        // Static/daily sources (OSM, world-map): 60 s hard floor so the app
        // can't hammer rate-limited public APIs when panning rapidly.
        60
    } else {
        // Live feeds: 5 s floor protects against burst retries on error.
        5
    };
    QueryScheduler::new(
        source.max_staleness_secs(),
        source.min_change_fraction(),
    ).with_cooldown(cooldown_secs)
}

// ─── PipelineManager ─────────────────────────────────────────────────────────

/// Holds all registered data sources and drives the fetch-dispatch loop.
pub struct PipelineManager {
    rt:       Handle,
    sources:  Vec<(Arc<dyn DataSource>, QueryScheduler)>,
    /// One sender per source (keyed by `DataSource::layer_name()`).
    senders:  HashMap<String, watch::Sender<Vec<GeoFeature>>>,
    /// Receivers handed to `DataLayer` impls during startup via `subscribe()`.
    receivers: HashMap<String, watch::Receiver<Vec<GeoFeature>>>,
    /// Rate-limit backoff signals sent from spawned fetch tasks.
    backoff_tx: mpsc::UnboundedSender<(String, Duration)>,
    backoff_rx: mpsc::UnboundedReceiver<(String, Duration)>,
}

impl PipelineManager {
    pub fn new(rt: Handle) -> Self {
        let (backoff_tx, backoff_rx) = mpsc::unbounded_channel();
        Self {
            rt,
            sources:    Vec::new(),
            senders:    HashMap::new(),
            receivers:  HashMap::new(),
            backoff_tx,
            backoff_rx,
        }
    }

    /// Register a data source.  Creates the watch channel for its layer if
    /// one does not already exist.
    pub fn register(&mut self, source: Arc<dyn DataSource>) {
        let layer = source.layer_name().to_string();
        self.senders.entry(layer.clone()).or_insert_with(|| {
            let (tx, rx) = watch::channel(Vec::new());
            self.receivers.insert(layer.clone(), rx);
            tx
        });
        let scheduler = make_scheduler(&*source);
        self.sources.push((source, scheduler));
    }

    /// Obtain the watch receiver for a named layer.
    ///
    /// Call once during layer initialisation.  Returns `None` if no source
    /// has been registered for this layer yet.
    pub fn subscribe(&self, layer_name: &str) -> Option<watch::Receiver<Vec<GeoFeature>>> {
        self.receivers.get(layer_name).cloned()
    }

    /// Called every frame from the render loop.
    ///
    /// Iterates over all registered sources; for those whose scheduler fires,
    /// spawns a `tokio` task that calls `DataSource::fetch` and pushes the
    /// result through the watch channel.
    pub fn tick(&mut self, bounds: &GeoBounds) {
        // Apply any rate-limit backoffs signalled by in-flight tasks.
        while let Ok((layer, delay)) = self.backoff_rx.try_recv() {
            for (source, scheduler) in &mut self.sources {
                if source.layer_name() == layer {
                    tracing::warn!(
                        source = source.name(),
                        delay_s = delay.as_secs(),
                        "applying rate-limit backoff"
                    );
                    scheduler.apply_backoff(delay);
                }
            }
        }

        for (source, scheduler) in &mut self.sources {
            if !scheduler.should_fetch(bounds) {
                continue;
            }

            let source     = Arc::clone(source);
            let bounds     = bounds.clone();
            let backoff_tx = self.backoff_tx.clone();
            let Some(tx) = self.senders.get(source.layer_name()).cloned() else {
                continue;
            };

            self.rt.spawn(async move {
                match source.fetch(&bounds).await {
                    Ok(features) => {
                        // Ignore send errors — receiver may have been dropped
                        let _ = tx.send(features);
                    }
                    Err(e) => {
                        if let Some(rl) = e.downcast_ref::<RateLimitError>() {
                            tracing::warn!(
                                source = source.name(),
                                retry_after_s = rl.retry_after.as_secs(),
                                "rate limited (429)"
                            );
                            let _ = backoff_tx.send((
                                source.layer_name().to_string(),
                                rl.retry_after,
                            ));
                        } else {
                            tracing::error!(source = source.name(), "fetch error: {e:#}");
                        }
                    }
                }
            });
        }
    }

    /// Names of all registered layers (for the HUD layer list and voice prompt).
    #[allow(dead_code)]
    pub fn layer_names(&self) -> Vec<&str> {
        self.senders.keys().map(|s| s.as_str()).collect()
    }

    /// Force the next `tick()` to re-fetch all sources for `layer`, ignoring
    /// staleness and viewport-change thresholds.  Used by the `refresh` console
    /// command.
    pub fn force_refresh(&mut self, layer: &str) {
        for (source, scheduler) in &mut self.sources {
            if source.layer_name() == layer {
                scheduler.last_fetch  = Instant::now() - Duration::from_secs(3_600_000);
                scheduler.last_bounds = None;
            }
        }
    }

    /// Hot-swap all registered sources whose `layer_name()` matches the new
    /// source's `layer_name()`.  Reuses the existing watch channel so
    /// subscribers don't need to re-subscribe.
    pub fn replace_source(&mut self, source: Arc<dyn DataSource>) {
        let layer = source.layer_name().to_string();
        // Ensure the watch channel exists (no-op if already registered)
        self.senders.entry(layer.clone()).or_insert_with(|| {
            let (tx, rx) = watch::channel(Vec::new());
            self.receivers.insert(layer.clone(), rx);
            tx
        });
        // Drop all old sources for this layer, install the new one
        self.sources.retain(|(s, _)| s.layer_name() != layer);
        let scheduler = make_scheduler(&*source);
        self.sources.push((source, scheduler));
    }

    /// Remove sources and clear the watch channel for `layer`, or all layers
    /// when `layer` is `None`.  In-flight fetch tasks complete normally but no
    /// new fetches will be scheduled for the stopped layer(s).
    pub fn stop(&mut self, layer: Option<&str>) {
        match layer {
            Some(name) => {
                self.sources.retain(|(s, _)| s.layer_name() != name);
                if let Some(tx) = self.senders.get(name) {
                    let _ = tx.send(Vec::new());
                }
            }
            None => {
                self.sources.clear();
                for tx in self.senders.values() {
                    let _ = tx.send(Vec::new());
                }
            }
        }
    }

    /// Names of all currently active (non-stopped) layer streams.
    pub fn active_streams(&self) -> Vec<&str> {
        self.sources.iter().map(|(s, _)| s.layer_name()).collect()
    }
}

// ─── Built-in stub sources ───────────────────────────────────────────────────

/// Example static GeoJSON source — loads a file once, filters in-memory.
///
/// Replace or extend this for real data:
/// - `TiledOsmSource`   — Overpass API tiles for POIs / roads
/// - `SocrataSource`    — city open-data portals (crime, housing, …)
/// - `AviationSource`   — OpenSky Network REST API for live flights
/// - `WeatherSource`    — Open-Meteo or NOAA NDFD tiles
#[allow(dead_code)]
pub struct StaticGeoJsonSource {
    layer:    String,
    features: Vec<GeoFeature>,
}

impl StaticGeoJsonSource {
    #[allow(dead_code)]
    pub fn new(layer: impl Into<String>, features: Vec<GeoFeature>) -> Self {
        Self { layer: layer.into(), features }
    }
}

#[async_trait::async_trait]
impl DataSource for StaticGeoJsonSource {
    fn name(&self) -> &str { &self.layer }
    fn layer_name(&self) -> &str { &self.layer }
    fn max_staleness_secs(&self) -> f32 { f32::INFINITY }
    fn min_change_fraction(&self) -> f32 { 1.0 } // never re-fetch; filter in memory

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        Ok(self.features
            .iter()
            .filter(|f| bounds.contains(f.lat, f.lon))
            .cloned()
            .collect())
    }
}
