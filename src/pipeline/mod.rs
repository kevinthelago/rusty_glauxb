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

pub mod config;
pub mod flights;
pub mod osm;
pub mod rate_limit;
pub mod remote;

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
    /// Multiplier (>= 1.0) applied to the cooldown and staleness windows.
    /// 1.0 = normal; car mode raises it to fetch less often while driving (CAR-7).
    throttle: f32,
    /// When true, the scheduler never fires — used by the pause-offscreen hook
    /// (CAR-7) so the thermal governor can suspend individual pipelines.
    paused: bool,
}

impl QueryScheduler {
    fn new(max_staleness_secs: f32, min_change_fraction: f32) -> Self {
        // The DataSource trait lets purely-static sources return f32::INFINITY;
        // Duration::from_secs_f32 panics on a non-finite value, so map "never
        // re-fetch on staleness" to an effectively-infinite finite Duration
        // (~136 years) that also stays safe when the CAR-7 throttle scales it.
        let max_staleness = if max_staleness_secs.is_finite() {
            Duration::from_secs_f32(max_staleness_secs)
        } else {
            Duration::from_secs(u32::MAX as u64)
        };
        Self {
            last_bounds:         None,
            last_fetch:          Instant::now(),
            max_staleness,
            min_change_fraction,
            min_cooldown:        Duration::from_secs(60),
            backoff_until:       None,
            force_next:          true,
            throttle:            1.0,
            paused:              false,
        }
    }

    fn with_cooldown(mut self, secs: u64) -> Self {
        self.min_cooldown = Duration::from_secs(secs);
        self
    }

    /// Set the throttle multiplier (clamped to >= 1.0 so it never *speeds up*
    /// fetching).  Used by car mode to back off all time-based fetching.
    fn set_throttle(&mut self, factor: f32) {
        self.throttle = factor.max(1.0);
    }

    /// Cooldown floor after applying the throttle.
    fn effective_cooldown(&self) -> Duration {
        self.min_cooldown.mul_f32(self.throttle)
    }

    /// Staleness window after applying the throttle.
    fn effective_staleness(&self) -> Duration {
        self.max_staleness.mul_f32(self.throttle)
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
        // A paused pipeline never fetches (CAR-7 pause-offscreen hook).
        if self.paused {
            return false;
        }

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

        // Never fetch faster than the (throttled) cooldown floor.  In car mode
        // this gate also slows viewport-change fetches below, since it runs
        // first.
        if elapsed < self.effective_cooldown() {
            return false;
        }

        // Always fetch on first call or after the (throttled) staleness timeout
        if self.last_bounds.is_none() || elapsed >= self.effective_staleness() {
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

// ─── Pipeline health (PIPE-6) ───────────────────────────────────────────────

/// Outcome of a pipeline's most recent fetch, for the pipeline-health HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineStatus {
    /// A fetch is in flight and has not reported back yet.
    Pending,
    /// The most recent fetch succeeded.
    Ok,
    /// The most recent fetch failed (network/parse error or rate-limit).
    Error,
}

/// Point-in-time view of one pipeline's fetch health.
#[derive(Clone, Debug)]
pub struct PipelineHealth {
    pub layer: String,
    pub status: PipelineStatus,
    /// Feature count from the most recent successful fetch.
    pub feature_count: usize,
    /// Message from the most recent failed fetch, if any.
    pub last_error: Option<String>,
    /// When the status last changed; `None` until the first fetch is dispatched.
    pub last_update: Option<Instant>,
}

impl PipelineHealth {
    fn new(layer: String) -> Self {
        Self {
            layer,
            status: PipelineStatus::Pending,
            feature_count: 0,
            last_error: None,
            last_update: None,
        }
    }
}

/// Fetch-outcome report sent from a spawned fetch task back to the manager so
/// it can update pipeline health on the next [`PipelineManager::tick`].
enum HealthReport {
    Ok { layer: String, count: usize },
    Err { layer: String, error: String },
}

// ─── Throttling (CAR-7) ──────────────────────────────────────────────────────

/// Graduated fetch-throttle level applied to every pipeline (CAR-7).
///
/// Higher levels multiply the cooldown/staleness windows so fetches fire less
/// often — saving battery, mobile data, and GPU churn.  This is the
/// **signal-only-from-CAR-4** half of car-mode throttling: the
/// [`crate::car`] thermal governor (which cannot edit this module) escalates the
/// level as the device heats up by calling
/// [`PipelineManager::set_throttle_level`].  It is **inert by default**
/// ([`ThrottleLevel::Normal`] = 1×), so landing it during Phase 1 changes
/// nothing until something opts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThrottleLevel {
    /// No throttling (default): normal fetch cadence.
    #[default]
    Normal,
    /// Mild back-off (~2× longer windows).
    Light,
    /// Moderate back-off (~4× longer windows).
    Moderate,
    /// Aggressive back-off (~8× longer windows) for sustained thermal pressure.
    Heavy,
}

impl ThrottleLevel {
    /// Multiplier applied to each scheduler's cooldown and staleness windows.
    pub fn multiplier(self) -> f32 {
        match self {
            ThrottleLevel::Normal => 1.0,
            ThrottleLevel::Light => 2.0,
            ThrottleLevel::Moderate => 4.0,
            ThrottleLevel::Heavy => 8.0,
        }
    }
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
    /// Per-layer fetch health (PIPE-6), keyed by `layer_name()`.
    health: HashMap<String, PipelineHealth>,
    /// Fetch-outcome reports sent from spawned fetch tasks.
    health_tx: mpsc::UnboundedSender<HealthReport>,
    health_rx: mpsc::UnboundedReceiver<HealthReport>,
    /// Current global fetch-throttle level (CAR-7).  `Normal` by default → inert.
    throttle: ThrottleLevel,
}

impl PipelineManager {
    pub fn new(rt: Handle) -> Self {
        let (backoff_tx, backoff_rx) = mpsc::unbounded_channel();
        let (health_tx, health_rx) = mpsc::unbounded_channel();
        Self {
            rt,
            sources:    Vec::new(),
            senders:    HashMap::new(),
            receivers:  HashMap::new(),
            backoff_tx,
            backoff_rx,
            health:     HashMap::new(),
            health_tx,
            health_rx,
            throttle:   ThrottleLevel::Normal,
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
        let mut scheduler = make_scheduler(&*source);
        // New sources inherit the current throttle level (CAR-7).
        scheduler.set_throttle(self.throttle.multiplier());
        self.sources.push((source, scheduler));
    }

    // ── Throttling API (CAR-7) ──────────────────────────────────────────────
    //
    // The interface the CAR-4 thermal governor (in `crate::car`, which cannot
    // edit this module) calls to back off pipeline work under thermal/battery
    // pressure.  Inert by default: a fresh manager is at `ThrottleLevel::Normal`
    // with no layers paused, so behaviour is unchanged until the governor opts
    // in.  Graduated (a level, not a bool) and paired with an explicit
    // pause-offscreen hook so the governor can escalate smoothly.

    /// Set the global fetch-throttle level, applied to every registered source
    /// and inherited by future registrations.  `ThrottleLevel::Normal` restores
    /// the normal cadence.
    pub fn set_throttle_level(&mut self, level: ThrottleLevel) {
        self.throttle = level;
        let mult = level.multiplier();
        for (_, scheduler) in &mut self.sources {
            scheduler.set_throttle(mult);
        }
    }

    /// The current global throttle level.
    pub fn throttle_level(&self) -> ThrottleLevel {
        self.throttle
    }

    /// Pause or resume fetching for a single layer without unregistering it.
    /// A paused layer keeps its config, channel, and last data; it simply stops
    /// scheduling new fetches.  Returns `true` if the layer exists.
    pub fn set_layer_paused(&mut self, layer: &str, paused: bool) -> bool {
        let mut found = false;
        for (source, scheduler) in &mut self.sources {
            if source.layer_name() == layer {
                scheduler.paused = paused;
                found = true;
            }
        }
        found
    }

    /// Pause every registered pipeline whose layer is **not** in `onscreen`, and
    /// resume those that are.  The thermal governor calls this with the visible
    /// layer set to suspend offscreen pipelines (CAR-7 "pause offscreen
    /// pipelines" thermal step) — the manager has no visibility data of its own,
    /// so the caller supplies it.
    pub fn pause_offscreen(&mut self, onscreen: &[&str]) {
        for (source, scheduler) in &mut self.sources {
            scheduler.paused = !onscreen.contains(&source.layer_name());
        }
    }

    /// Resume every paused pipeline (clears all pause-offscreen suspensions).
    pub fn resume_all(&mut self) {
        for (_, scheduler) in &mut self.sources {
            scheduler.paused = false;
        }
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
        // Apply fetch-outcome reports from in-flight tasks to pipeline health.
        while let Ok(report) = self.health_rx.try_recv() {
            self.apply_health_report(report);
        }

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
            let health_tx  = self.health_tx.clone();
            let layer      = source.layer_name().to_string();
            let Some(tx) = self.senders.get(source.layer_name()).cloned() else {
                continue;
            };

            // Mark the layer as fetching so the health HUD shows in-flight state.
            self.health
                .entry(layer.clone())
                .or_insert_with(|| PipelineHealth::new(layer.clone()))
                .status = PipelineStatus::Pending;
            if let Some(h) = self.health.get_mut(&layer) {
                h.last_update = Some(Instant::now());
            }

            self.rt.spawn(async move {
                match source.fetch(&bounds).await {
                    Ok(features) => {
                        let count = features.len();
                        // Ignore send errors — receiver may have been dropped
                        let _ = tx.send(features);
                        let _ = health_tx.send(HealthReport::Ok { layer, count });
                    }
                    Err(e) => {
                        if let Some(rl) = e.downcast_ref::<RateLimitError>() {
                            tracing::warn!(
                                source = source.name(),
                                retry_after_s = rl.retry_after.as_secs(),
                                "rate limited (429)"
                            );
                            let _ = backoff_tx.send((layer.clone(), rl.retry_after));
                            let _ = health_tx.send(HealthReport::Err {
                                layer,
                                error: format!(
                                    "rate limited — retry in {}s",
                                    rl.retry_after.as_secs()
                                ),
                            });
                        } else {
                            tracing::error!(source = source.name(), "fetch error: {e:#}");
                            let _ = health_tx.send(HealthReport::Err {
                                layer,
                                error: format!("{e:#}"),
                            });
                        }
                    }
                }
            });
        }
    }

    /// Apply a single fetch-outcome report to the per-layer health map.
    fn apply_health_report(&mut self, report: HealthReport) {
        let now = Instant::now();
        match report {
            HealthReport::Ok { layer, count } => {
                let h = self
                    .health
                    .entry(layer.clone())
                    .or_insert_with(|| PipelineHealth::new(layer));
                h.status = PipelineStatus::Ok;
                h.feature_count = count;
                h.last_error = None;
                h.last_update = Some(now);
            }
            HealthReport::Err { layer, error } => {
                let h = self
                    .health
                    .entry(layer.clone())
                    .or_insert_with(|| PipelineHealth::new(layer));
                h.status = PipelineStatus::Error;
                h.last_error = Some(error);
                h.last_update = Some(now);
            }
        }
    }

    /// Snapshot of every tracked pipeline's health, sorted by layer name for a
    /// stable display order.  Consumed by the pipeline-health HUD (PIPE-6).
    pub fn health(&self) -> Vec<PipelineHealth> {
        let mut out: Vec<PipelineHealth> = self.health.values().cloned().collect();
        out.sort_by(|a, b| a.layer.cmp(&b.layer));
        out
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
        let mut scheduler = make_scheduler(&*source);
        // Inherit the current throttle level (CAR-7).
        scheduler.set_throttle(self.throttle.multiplier());
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
                self.health.remove(name);
            }
            None => {
                self.sources.clear();
                for tx in self.senders.values() {
                    let _ = tx.send(Vec::new());
                }
                self.health.clear();
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

#[cfg(test)]
mod health_tests {
    use super::*;

    fn manager() -> PipelineManager {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        PipelineManager::new(rt.handle().clone())
    }

    #[test]
    fn ok_report_sets_status_and_count() {
        let mut m = manager();
        m.apply_health_report(HealthReport::Ok {
            layer: "roads".into(),
            count: 42,
        });
        let h = m.health();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].layer, "roads");
        assert_eq!(h[0].status, PipelineStatus::Ok);
        assert_eq!(h[0].feature_count, 42);
        assert!(h[0].last_error.is_none());
        assert!(h[0].last_update.is_some());
    }

    #[test]
    fn err_report_sets_error_and_preserves_prior_count() {
        let mut m = manager();
        m.apply_health_report(HealthReport::Ok {
            layer: "quakes".into(),
            count: 10,
        });
        m.apply_health_report(HealthReport::Err {
            layer: "quakes".into(),
            error: "boom".into(),
        });
        let h = m.health();
        assert_eq!(h[0].status, PipelineStatus::Error);
        assert_eq!(h[0].last_error.as_deref(), Some("boom"));
        // A failed refetch keeps the last good count for display.
        assert_eq!(h[0].feature_count, 10);
    }

    #[test]
    fn recovery_clears_error() {
        let mut m = manager();
        m.apply_health_report(HealthReport::Err {
            layer: "x".into(),
            error: "down".into(),
        });
        m.apply_health_report(HealthReport::Ok {
            layer: "x".into(),
            count: 3,
        });
        let h = m.health();
        assert_eq!(h[0].status, PipelineStatus::Ok);
        assert!(h[0].last_error.is_none());
        assert_eq!(h[0].feature_count, 3);
    }

    #[test]
    fn health_is_sorted_by_layer() {
        let mut m = manager();
        for layer in ["zulu", "alpha", "mike"] {
            m.apply_health_report(HealthReport::Ok {
                layer: layer.into(),
                count: 1,
            });
        }
        let layers: Vec<_> = m.health().into_iter().map(|h| h.layer).collect();
        assert_eq!(layers, vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn stop_clears_health() {
        let mut m = manager();
        m.apply_health_report(HealthReport::Ok {
            layer: "a".into(),
            count: 1,
        });
        m.apply_health_report(HealthReport::Ok {
            layer: "b".into(),
            count: 1,
        });
        m.stop(Some("a"));
        assert_eq!(m.health().len(), 1);
        assert_eq!(m.health()[0].layer, "b");
        m.stop(None);
        assert!(m.health().is_empty());
    }
}

#[cfg(test)]
mod car_tests {
    use super::*;

    fn manager() -> PipelineManager {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        PipelineManager::new(rt.handle().clone())
    }

    fn source(layer: &str) -> Arc<dyn DataSource> {
        Arc::new(StaticGeoJsonSource::new(layer, Vec::new()))
    }

    #[test]
    fn throttle_levels_are_graduated_and_ordered() {
        assert_eq!(ThrottleLevel::default(), ThrottleLevel::Normal);
        assert_eq!(ThrottleLevel::Normal.multiplier(), 1.0);
        assert!(ThrottleLevel::Light.multiplier() < ThrottleLevel::Moderate.multiplier());
        assert!(ThrottleLevel::Moderate.multiplier() < ThrottleLevel::Heavy.multiplier());
        // Ord lets the thermal governor escalate by comparing levels.
        assert!(ThrottleLevel::Normal < ThrottleLevel::Heavy);
    }

    #[test]
    fn scheduler_throttle_scales_windows_and_clamps() {
        let mut s = QueryScheduler::new(30.0, 0.25); // 60s cooldown, 30s staleness
        assert_eq!(s.effective_cooldown().as_secs(), 60);
        assert_eq!(s.effective_staleness().as_secs(), 30);
        s.set_throttle(4.0);
        assert_eq!(s.effective_cooldown().as_secs(), 240);
        assert_eq!(s.effective_staleness().as_secs(), 120);
        // Never speeds fetching up.
        s.set_throttle(0.25);
        assert_eq!(s.throttle, 1.0);
    }

    #[test]
    fn paused_scheduler_never_fetches() {
        let mut s = QueryScheduler::new(30.0, 0.25);
        s.paused = true;
        assert!(!s.should_fetch(&GeoBounds::global()));
    }

    #[test]
    fn manager_is_inert_by_default() {
        let mut m = manager();
        m.register(source("a"));
        m.register(source("b"));
        assert_eq!(m.throttle_level(), ThrottleLevel::Normal);
        for (_, s) in &m.sources {
            assert_eq!(s.throttle, 1.0);
            assert!(!s.paused);
        }
    }

    #[test]
    fn set_throttle_level_applies_and_is_inherited() {
        let mut m = manager();
        m.register(source("a"));
        m.set_throttle_level(ThrottleLevel::Heavy);
        assert_eq!(m.throttle_level(), ThrottleLevel::Heavy);
        for (_, s) in &m.sources {
            assert_eq!(s.throttle, 8.0);
        }
        // A source registered after the level was raised inherits it.
        m.register(source("b"));
        let b = m.sources.iter().find(|(s, _)| s.layer_name() == "b").unwrap();
        assert_eq!(b.1.throttle, 8.0);
        // Restoring Normal clears the throttle.
        m.set_throttle_level(ThrottleLevel::Normal);
        for (_, s) in &m.sources {
            assert_eq!(s.throttle, 1.0);
        }
    }

    #[test]
    fn pause_layer_and_offscreen_and_resume() {
        let mut m = manager();
        m.register(source("a"));
        m.register(source("b"));
        m.register(source("c"));

        assert!(m.set_layer_paused("a", true));
        assert!(!m.set_layer_paused("missing", true));
        let a = m.sources.iter().find(|(s, _)| s.layer_name() == "a").unwrap();
        assert!(a.1.paused);

        // Only "b" on screen → a and c paused, b active.
        m.pause_offscreen(&["b"]);
        for (s, sched) in &m.sources {
            if s.layer_name() == "b" {
                assert!(!sched.paused);
            } else {
                assert!(sched.paused);
            }
        }

        m.resume_all();
        for (_, sched) in &m.sources {
            assert!(!sched.paused);
        }
    }
}
