//! ADS-B Exchange via RapidAPI.
//!
//! Docs: https://rapidapi.com/adsbx/api/adsbexchange-com1
//! Endpoint: GET https://adsbexchange-com1.p.rapidapi.com/v2/lat/{lat}/lon/{lon}/dist/{nm}/
//!
//! Requires: set adsbx_api_key <your-rapidapi-key>
//! Free tier: limited; paid plans available.

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::{
    geo::{GeoBounds, GeoFeature},
    pipeline::DataSource,
};

use super::{AircraftCategory, FlightState};

pub struct AdsbExchangeSource {
    client:       Client,
    api_key:      Option<String>,
    refresh_secs: f32,
}

impl AdsbExchangeSource {
    pub fn new(api_key: Option<String>, refresh_secs: f32) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("HTTP client"),
            api_key,
            refresh_secs,
        }
    }
}

#[async_trait]
impl DataSource for AdsbExchangeSource {
    fn name(&self)       -> &str { "ADS-B Exchange" }
    fn layer_name(&self) -> &str { "flights" }
    fn max_staleness_secs(&self) -> f32 { self.refresh_secs }

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        let Some(key) = &self.api_key else {
            tracing::warn!("ADS-B Exchange requires adsbx_api_key — use: set adsbx_api_key <key>");
            return Ok(Vec::new());
        };

        // Query from the centre of the visible bounds; radius in nautical miles.
        let clat = (bounds.min_lat + bounds.max_lat) * 0.5;
        let clon = (bounds.min_lon + bounds.max_lon) * 0.5;
        let lat_span_nm = (bounds.max_lat - bounds.min_lat) * 60.0;
        let lon_span_nm = (bounds.max_lon - bounds.min_lon) * 60.0 * clat.to_radians().cos();
        let radius_nm   = (lat_span_nm.max(lon_span_nm) * 0.5).clamp(50.0, 250.0) as u32;

        let url = format!(
            "https://adsbexchange-com1.p.rapidapi.com/v2/lat/{clat:.4}/lon/{clon:.4}/dist/{radius_nm}/"
        );

        let resp: Value = self
            .client
            .get(&url)
            .header("X-RapidAPI-Key", key.as_str())
            .header("X-RapidAPI-Host", "adsbexchange-com1.p.rapidapi.com")
            .send()
            .await
            .context("ADS-B Exchange request failed")?
            .error_for_status()
            .context("ADS-B Exchange HTTP error")?
            .json()
            .await
            .context("ADS-B Exchange JSON parse")?;

        let aircraft = match resp.get("ac").and_then(|v| v.as_array()) {
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

            let icao24   = str_key(ac, "hex");
            let cat_str  = ac.get("category").and_then(|v| v.as_str());
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

        tracing::info!(source = "adsbexchange", count = features.len(), "fetched flights");
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
