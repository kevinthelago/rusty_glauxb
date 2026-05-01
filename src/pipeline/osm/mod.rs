//! OpenStreetMap data via the Overpass API.
//!
//! Endpoint: POST https://overpass-api.de/api/interpreter
//! Query language: Overpass QL  (`[out:json]; ... out geom;`)
//!
//! Two built-in sources are exposed:
//!   - `roads_source()`     — major roads (motorway → tertiary)
//!   - `railways_source()`  — passenger rail (rail, light_rail, subway, tram)
//!
//! Data is refreshed when the view pans by more than 50% of its span,
//! or when the user issues `refresh roads` / `refresh railways` in the console.

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::{
    geo::{FeatureKind, GeoBounds, GeoFeature},
    pipeline::DataSource,
};

// ─── OverpassSource ───────────────────────────────────────────────────────────

pub struct OverpassSource {
    client:       Client,
    name:         String,
    layer:        String,
    /// Overpass QL template; `{S}` `{W}` `{N}` `{E}` are substituted with the
    /// current bounding box (south, west, north, east in decimal degrees).
    query:        String,
    max_staleness: f32,
}

impl OverpassSource {
    pub fn new(
        name:          impl Into<String>,
        layer:         impl Into<String>,
        query:         impl Into<String>,
        max_staleness: f32,
    ) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .user_agent("rusty_glauxb/0.1")
                .build()
                .expect("HTTP client"),
            name:         name.into(),
            layer:        layer.into(),
            query:        query.into(),
            max_staleness,
        }
    }
}

#[async_trait]
impl DataSource for OverpassSource {
    fn name(&self)                 -> &str { &self.name }
    fn layer_name(&self)           -> &str { &self.layer }
    fn max_staleness_secs(&self)   -> f32  { self.max_staleness }
    fn min_change_fraction(&self)  -> f32  { 0.50 }

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        let lat_span = bounds.max_lat - bounds.min_lat;
        let lon_span = (bounds.max_lon - bounds.min_lon).abs();

        // Skip when zoomed too far out — Overpass would return huge datasets
        // and individual roads aren't useful at globe scale anyway.
        if lat_span > 45.0 || lon_span > 90.0 {
            tracing::info!(
                source = %self.name, lat_span, lon_span,
                "view too wide, skipping fetch — zoom in to see this layer"
            );
            return Ok(Vec::new());
        }

        let (primary, _) = bounds.split();
        let query = self.query
            .replace("{S}", &primary.min_lat.to_string())
            .replace("{W}", &primary.min_lon.to_string())
            .replace("{N}", &primary.max_lat.to_string())
            .replace("{E}", &primary.max_lon.to_string());

        tracing::info!(source = %self.name, "fetching from Overpass API…");

        let resp: Value = self
            .client
            .post("https://overpass-api.de/api/interpreter")
            .form(&[("data", &query)])
            .send()
            .await
            .context("Overpass request failed")?
            .error_for_status()
            .context("Overpass HTTP error")?
            .json()
            .await
            .context("Overpass JSON parse")?;

        let elements = match resp.get("elements").and_then(|v| v.as_array()) {
            Some(a) => a,
            None    => return Ok(Vec::new()),
        };

        let mut features = Vec::new();
        for el in elements {
            if el.get("type").and_then(|v| v.as_str()) != Some("way") {
                continue;
            }

            let geometry = match el.get("geometry").and_then(|v| v.as_array()) {
                Some(g) if g.len() >= 2 => g,
                _                       => continue,
            };

            let coords: Vec<[f32; 2]> = geometry
                .iter()
                .filter_map(|node| {
                    let lat = node.get("lat")?.as_f64()? as f32;
                    let lon = node.get("lon")?.as_f64()? as f32;
                    Some([lat, lon])
                })
                .collect();

            if coords.len() < 2 {
                continue;
            }

            let [first_lat, first_lon] = coords[0];
            let tags = el.get("tags").cloned().unwrap_or(Value::Null);

            features.push(GeoFeature {
                lat:         first_lat,
                lon:         first_lon,
                alt:         0.0,
                kind:        FeatureKind::Line,
                meta:        tags,
                coordinates: coords,
            });
        }

        tracing::info!(source = %self.name, count = features.len(), "fetched OSM ways");
        Ok(features)
    }
}

// ─── Built-in sources ─────────────────────────────────────────────────────────

/// Major road network (motorway → tertiary).
pub fn roads_source() -> OverpassSource {
    OverpassSource::new(
        "OSM Roads",
        "roads",
        "[out:json][timeout:40];\
         (way[\"highway\"~\"^(motorway|trunk|primary|secondary|tertiary)$\"]({S},{W},{N},{E}););\
         out geom;",
        86_400.0, // refresh daily; use `refresh roads` to force sooner
    )
}

/// Passenger rail network (heavy rail, light rail, subway, tram).
pub fn railways_source() -> OverpassSource {
    OverpassSource::new(
        "OSM Railways",
        "railways",
        "[out:json][timeout:40];\
         (way[\"railway\"~\"^(rail|light_rail|subway|tram)$\"]({S},{W},{N},{E}););\
         out geom;",
        86_400.0,
    )
}
