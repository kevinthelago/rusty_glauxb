//! ADSB.fi — free, no authentication required.
//!
//! Docs: https://api.adsb.fi/
//! Endpoint: GET https://api.adsb.fi/v1/aircraft?bounds=<lat_lo>,<lat_hi>,<lon_lo>,<lon_hi>
//!
//! Response: { "aircraft": [ { "hex": "...", "lat": ..., "lon": ..., ... } ] }

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

use crate::{
    geo::{GeoBounds, GeoFeature},
    pipeline::{
        rate_limit::{extract_retry_after, RateLimitError},
        DataSource,
    },
};

use super::{AircraftCategory, FlightState};

pub struct AdsbFiSource {
    client:       Client,
    refresh_secs: f32,
}

impl AdsbFiSource {
    pub fn new(refresh_secs: f32) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .user_agent("rusty_glauxb/0.1")
                .build()
                .expect("HTTP client"),
            refresh_secs,
        }
    }
}

#[async_trait]
impl DataSource for AdsbFiSource {
    fn name(&self)       -> &str { "ADSB.fi" }
    fn layer_name(&self) -> &str { "flights" }
    fn max_staleness_secs(&self) -> f32 { self.refresh_secs }

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        // Skip when zoomed too far out — the API rejects excessively large bboxes.
        let lat_span = bounds.max_lat - bounds.min_lat;
        let lon_span = (bounds.max_lon - bounds.min_lon).abs();
        if lat_span > 90.0 || lon_span > 180.0 {
            tracing::info!(
                lat_span, lon_span,
                "ADSB.fi: view too wide, skipping fetch — zoom in to see aircraft"
            );
            return Ok(Vec::new());
        }

        let (primary, _) = bounds.split();

        // ADSB.fi bounds format: lat_lo,lat_hi,lon_lo,lon_hi
        let url = format!(
            "https://api.adsb.fi/v1/aircraft?bounds={},{},{},{}",
            primary.min_lat, primary.max_lat,
            primary.min_lon, primary.max_lon,
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("ADSB.fi request failed")?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let delay = extract_retry_after(response.headers(), Duration::from_secs(60));
            return Err(RateLimitError { retry_after: delay }.into());
        }

        let resp: Value = response
            .error_for_status()
            .context("ADSB.fi HTTP error")?
            .json()
            .await
            .context("ADSB.fi JSON parse")?;

        let aircraft = match resp.get("aircraft").and_then(|v| v.as_array()) {
            Some(a) => a,
            None    => return Ok(Vec::new()),
        };

        let mut features = Vec::with_capacity(aircraft.len());
        for ac in aircraft {
            let lat = match ac.get("lat").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None    => continue,
            };
            let lon = match ac.get("lon").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None    => continue,
            };

            let icao24 = str_key(ac, "hex");
            let cat_str = ac.get("category").and_then(|v| v.as_str());
            let flight = FlightState {
                category:      AircraftCategory::from_adsb(cat_str, &icao24),
                icao24,
                callsign:      opt_str_key(ac, "flight"),
                registration:  opt_str_key(ac, "r"),
                aircraft_type: opt_str_key(ac, "t"),
                lat,
                lon,
                baro_alt_ft:   f32_key(ac, "alt_baro"),
                geo_alt_ft:    f32_key(ac, "alt_geom"),
                velocity_kts:  f32_key(ac, "gs"),
                heading:       f32_key(ac, "track"),
                vert_rate_fpm: f32_key(ac, "baro_rate"),
                on_ground:     ac.get("alt_baro")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "ground"),
                squawk:        opt_str_key(ac, "squawk"),
                origin_country: None,
                last_seen:     u64_key(ac, "seen_pos").unwrap_or(0),
            };

            features.push(GeoFeature::from(flight));
        }

        tracing::info!(source = "adsbfi", count = features.len(), "fetched flights");
        Ok(features)
    }
}

fn str_key(v: &Value, key: &str) -> String {
    v.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string()
}

fn opt_str_key(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}

fn f32_key(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|n| n as f32)
}

fn u64_key(v: &Value, key: &str) -> Option<u64> {
    v.get(key)?.as_u64()
}
