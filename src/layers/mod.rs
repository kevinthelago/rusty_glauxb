#![allow(dead_code)]

/// A geographic point used by every data layer.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GeoPoint {
    pub lat: f32,
    pub lon: f32,
    pub alt: f32,
    /// Arbitrary metadata (label, value, etc.) stored as JSON.
    pub meta: serde_json::Value,
}

/// Every data source — static or live — implements this trait.
pub trait DataLayer: Send + Sync {
    fn name(&self) -> &str;
    /// Called each frame; live layers swap in new data from their watch channel.
    fn update(&mut self);
    /// Points visible within the current view frustum.
    fn visible_points(&self) -> &[GeoPoint];
    /// Whether the layer is shown in the current filter set.
    fn is_visible(&self) -> bool;
    fn set_visible(&mut self, v: bool);
}

/// Manages the full set of data layers and applies voice-driven filters.
#[derive(Default)]
pub struct LayerManager {
    layers: Vec<Box<dyn DataLayer>>,
}

impl LayerManager {
    pub fn add(&mut self, layer: Box<dyn DataLayer>) {
        self.layers.push(layer);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn DataLayer> {
        self.layers.iter().map(|l| l.as_ref())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn DataLayer>> {
        self.layers.iter_mut()
    }

    /// Show only the named layers (case-insensitive). "all" re-enables everything.
    pub fn apply_filter(&mut self, filter: &[String]) {
        let show_all = filter.iter().any(|s| s.eq_ignore_ascii_case("all"));
        for layer in &mut self.layers {
            let visible = show_all
                || filter
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(layer.name()));
            layer.set_visible(visible);
        }
    }

    pub fn layer_names(&self) -> Vec<&str> {
        self.layers.iter().map(|l| l.name()).collect()
    }
}
