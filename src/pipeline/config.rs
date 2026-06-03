//! User-defined pipeline model and on-disk persistence.
//!
//! A **pipeline** is a user-configured data flow: a remote GeoJSON URL fetched
//! on a chosen cadence and rendered into its own layer.  This is the product's
//! core value — users compose their own map by adding pipelines rather than
//! relying only on the built-in [`crate::download`] catalog.
//!
//! ## Static vs. dynamic cadence
//!
//! The cadence maps directly onto the [`crate::pipeline::DataSource`] scheduler
//! knobs so the existing [`crate::pipeline::PipelineManager`] /
//! `QueryScheduler` machinery drives user pipelines unchanged:
//!
//! | Cadence              | `max_staleness_secs()`          | refetch trigger          |
//! |----------------------|----------------------------------|--------------------------|
//! | [`Cadence::Static`]  | ≥ 1 day ([`STATIC_STALENESS_SECS`]) | manual / cache-expiry  |
//! | [`Cadence::Dynamic`] | the user-chosen interval         | automatic on the interval |
//!
//! Both use a `min_change_fraction` of `1.0`: a user pipeline fetches the whole
//! dataset and filters in memory, so panning/zooming never triggers a refetch —
//! only the cadence does.
//!
//! ## Persistence
//!
//! The set of pipelines round-trips through `pipelines.json` in the platform
//! data directory ([`crate::platform_paths::data_file`]) so a user's composed
//! map survives restarts.

use serde::{Deserialize, Serialize};

/// Filename of the persisted pipeline registry inside the data directory.
const STORE_FILE: &str = "pipelines.json";

/// Staleness threshold (seconds) that classifies a pipeline as *static*: one
/// day.  A static pipeline is only refetched after this long, or on an explicit
/// manual trigger, so it behaves like a download-once dataset.
pub const STATIC_STALENESS_SECS: f32 = 24.0 * 3600.0;

/// Smallest legal automatic refresh interval for a dynamic pipeline (seconds).
/// Guards against a user configuring a hammering 0-second loop.
pub const MIN_DYNAMIC_INTERVAL_SECS: f32 = 1.0;

// ─── Cadence ───────────────────────────────────────────────────────────────────

/// How often a pipeline refetches its source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Cadence {
    /// Download-once data (boundaries, infrastructure, …).  Refetched only when
    /// the cache passes [`STATIC_STALENESS_SECS`] or the user triggers it.
    Static,
    /// Live data refetched automatically every `interval_secs`.
    Dynamic { interval_secs: f32 },
}

impl Cadence {
    /// The `DataSource::max_staleness_secs` value this cadence implies.
    pub fn max_staleness_secs(&self) -> f32 {
        match self {
            Cadence::Static => STATIC_STALENESS_SECS,
            // A dynamic pipeline is never staler than its own interval.
            Cadence::Dynamic { interval_secs } => *interval_secs,
        }
    }

    /// True for [`Cadence::Static`].
    pub fn is_static(&self) -> bool {
        matches!(self, Cadence::Static)
    }

    /// Validate the cadence, returning a human-readable error on bad input.
    fn validate(&self) -> Result<(), String> {
        match self {
            Cadence::Static => Ok(()),
            Cadence::Dynamic { interval_secs } => {
                if interval_secs.is_finite() && *interval_secs >= MIN_DYNAMIC_INTERVAL_SECS {
                    Ok(())
                } else {
                    Err(format!(
                        "dynamic interval must be a finite value ≥ {MIN_DYNAMIC_INTERVAL_SECS}s"
                    ))
                }
            }
        }
    }
}

// ─── PipelineConfig ─────────────────────────────────────────────────────────────

/// A single user-defined pipeline.
///
/// The [`id`](Self::id) doubles as the layer name the pipeline's
/// [`crate::pipeline::DataSource`] populates, so it must be unique within a
/// [`PipelineStore`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Stable unique identifier; also the populated layer name.
    pub id: String,
    /// Human-readable label shown in the HUD / console.
    pub name: String,
    /// Remote GeoJSON URL to fetch.
    pub url: String,
    /// Refresh behaviour (static vs. dynamic).
    pub cadence: Cadence,
    /// Whether the pipeline is currently active (fetching).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl PipelineConfig {
    /// Build a new pipeline, normalising the id and validating every field.
    ///
    /// The id is slugified (lower-cased, non-alphanumerics → `_`) so it is safe
    /// to use as a layer name and filename fragment.  Returns a descriptive
    /// error if any field is invalid.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        url: impl Into<String>,
        cadence: Cadence,
    ) -> Result<Self, String> {
        let id = slugify(&id.into());
        if id.is_empty() {
            return Err("pipeline id must contain at least one alphanumeric character".into());
        }
        let url = url.into();
        validate_url(&url)?;
        cadence.validate()?;
        let name = {
            let n = name.into();
            if n.trim().is_empty() {
                id.clone()
            } else {
                n
            }
        };
        Ok(Self {
            id,
            name,
            url,
            cadence,
            enabled: true,
        })
    }

    /// The `DataSource::max_staleness_secs` value for this pipeline.
    pub fn max_staleness_secs(&self) -> f32 {
        self.cadence.max_staleness_secs()
    }
}

/// Normalise a free-form string into a layer/id slug: lower-case, with each run
/// of non-alphanumeric characters collapsed to a single underscore and the ends
/// trimmed.
pub fn slugify(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Reject obviously invalid URLs.  We only accept `http`/`https` since the
/// remote DataSource fetches over HTTP.
fn validate_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if u.starts_with("http://") || u.starts_with("https://") {
        // Require something after the scheme.
        let rest = u
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        if rest.is_empty() {
            Err("url has no host".into())
        } else {
            Ok(())
        }
    } else {
        Err("url must start with http:// or https://".into())
    }
}

// ─── PipelineStore ──────────────────────────────────────────────────────────────

/// The persisted set of user pipelines.
///
/// Acts as an ordered registry keyed by [`PipelineConfig::id`].  All mutations
/// are in-memory; call [`save`](Self::save) to persist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PipelineStore {
    #[serde(default)]
    pipelines: Vec<PipelineConfig>,
}

impl PipelineStore {
    /// Load the store from `pipelines.json`, returning an empty store if the
    /// file is missing or unreadable (a corrupt file is logged and ignored so a
    /// bad write never bricks startup).
    pub fn load() -> Self {
        let Ok(path) = crate::platform_paths::data_file(STORE_FILE) else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(error = %e, "pipelines.json is corrupt; starting empty");
                Self::default()
            }
        }
    }

    /// Persist the store to `pipelines.json`.  Best-effort: I/O errors are
    /// logged, not propagated, so a failed write never crashes the app.
    pub fn save(&self) {
        let Ok(path) = crate::platform_paths::data_file(STORE_FILE) else {
            tracing::warn!("cannot resolve data dir; pipelines not saved");
            return;
        };
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, "failed to write pipelines.json");
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to serialise pipelines"),
        }
    }

    /// All pipelines in insertion order.
    pub fn all(&self) -> &[PipelineConfig] {
        &self.pipelines
    }

    /// Number of registered pipelines.
    pub fn len(&self) -> usize {
        self.pipelines.len()
    }

    /// True when no pipelines are registered.
    pub fn is_empty(&self) -> bool {
        self.pipelines.is_empty()
    }

    /// Look up a pipeline by id.
    pub fn get(&self, id: &str) -> Option<&PipelineConfig> {
        self.pipelines.iter().find(|p| p.id == id)
    }

    /// Add a pipeline, rejecting a duplicate id.  Returns the stored id on
    /// success.
    pub fn add(&mut self, config: PipelineConfig) -> Result<String, String> {
        if self.get(&config.id).is_some() {
            return Err(format!("a pipeline with id '{}' already exists", config.id));
        }
        let id = config.id.clone();
        self.pipelines.push(config);
        Ok(id)
    }

    /// Remove a pipeline by id, returning the removed config if present.
    pub fn remove(&mut self, id: &str) -> Option<PipelineConfig> {
        let idx = self.pipelines.iter().position(|p| p.id == id)?;
        Some(self.pipelines.remove(idx))
    }

    /// Enable or disable a pipeline by id, returning `true` if it existed.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        match self.pipelines.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// All currently enabled pipelines.
    pub fn enabled(&self) -> impl Iterator<Item = &PipelineConfig> {
        self.pipelines.iter().filter(|p| p.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, cadence: Cadence) -> PipelineConfig {
        PipelineConfig::new(id, "", "https://example.com/data.geojson", cadence).unwrap()
    }

    #[test]
    fn static_cadence_is_at_least_a_day() {
        assert!(Cadence::Static.max_staleness_secs() >= 24.0 * 3600.0);
        assert!(Cadence::Static.is_static());
    }

    #[test]
    fn dynamic_cadence_uses_interval() {
        let c = Cadence::Dynamic {
            interval_secs: 30.0,
        };
        assert_eq!(c.max_staleness_secs(), 30.0);
        assert!(!c.is_static());
    }

    #[test]
    fn rejects_bad_dynamic_interval() {
        let err = PipelineConfig::new(
            "x",
            "X",
            "https://e.com/a.geojson",
            Cadence::Dynamic { interval_secs: 0.0 },
        );
        assert!(err.is_err());
    }

    #[test]
    fn rejects_non_http_url() {
        assert!(PipelineConfig::new("x", "X", "ftp://e.com/a", Cadence::Static).is_err());
        assert!(PipelineConfig::new("x", "X", "", Cadence::Static).is_err());
        assert!(PipelineConfig::new("x", "X", "https://", Cadence::Static).is_err());
    }

    #[test]
    fn slugifies_id_and_defaults_name() {
        let p = PipelineConfig::new(
            "  My Cool Layer!! ",
            "",
            "https://e.com/a.geojson",
            Cadence::Static,
        )
        .unwrap();
        assert_eq!(p.id, "my_cool_layer");
        // empty name defaults to the slug
        assert_eq!(p.name, "my_cool_layer");
    }

    #[test]
    fn rejects_empty_slug() {
        assert!(
            PipelineConfig::new("!!!", "X", "https://e.com/a.geojson", Cadence::Static).is_err()
        );
    }

    #[test]
    fn add_rejects_duplicate_id() {
        let mut store = PipelineStore::default();
        store.add(sample("dup", Cadence::Static)).unwrap();
        assert!(store.add(sample("dup", Cadence::Static)).is_err());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_and_lookup() {
        let mut store = PipelineStore::default();
        store.add(sample("a", Cadence::Static)).unwrap();
        store
            .add(sample("b", Cadence::Dynamic { interval_secs: 5.0 }))
            .unwrap();
        assert!(store.get("a").is_some());
        let removed = store.remove("a").unwrap();
        assert_eq!(removed.id, "a");
        assert!(store.get("a").is_none());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn set_enabled_filters() {
        let mut store = PipelineStore::default();
        store.add(sample("a", Cadence::Static)).unwrap();
        store.add(sample("b", Cadence::Static)).unwrap();
        assert_eq!(store.enabled().count(), 2);
        assert!(store.set_enabled("a", false));
        assert_eq!(store.enabled().count(), 1);
        assert!(!store.set_enabled("missing", false));
    }

    #[test]
    fn json_round_trips() {
        let mut store = PipelineStore::default();
        store.add(sample("roads", Cadence::Static)).unwrap();
        store
            .add(sample(
                "flights",
                Cadence::Dynamic {
                    interval_secs: 15.0,
                },
            ))
            .unwrap();
        let json = serde_json::to_string_pretty(&store).unwrap();
        let back: PipelineStore = serde_json::from_str(&json).unwrap();
        assert_eq!(store, back);
    }

    #[test]
    fn deserialises_with_missing_enabled_defaulting_true() {
        let json = r#"{
            "pipelines": [
                {"id":"x","name":"X","url":"https://e.com/a.geojson","cadence":{"mode":"static"}}
            ]
        }"#;
        let store: PipelineStore = serde_json::from_str(json).unwrap();
        assert!(store.get("x").unwrap().enabled);
    }
}
