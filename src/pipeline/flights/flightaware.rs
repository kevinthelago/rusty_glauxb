//! FlightAware AeroAPI — commercial, most complete data available.
//!
//! Docs: https://flightaware.com/aeroapi/portal/documentation
//! Endpoint: GET https://aeroapi.flightaware.com/aeroapi/flights/search
//!            ?query=-latlong "{lat_lo} {lon_lo} {lat_hi} {lon_hi}"
//!
//! Requires: set flightaware_api_key <your-aeroapi-key>
//! Pricing: usage-based (see FlightAware portal).

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::{
    geo::{GeoBounds, GeoFeature},
    pipeline::DataSource,
};

use super::{AircraftCategory, FlightState};

pub struct FlightAwareSource {
    client:       Client,
    api_key:      Option<String>,
    refresh_secs: f32,
}

impl FlightAwareSource {
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
impl DataSource for FlightAwareSource {
    fn name(&self)       -> &str { "FlightAware AeroAPI" }
    fn layer_name(&self) -> &str { "flights" }
    fn max_staleness_secs(&self) -> f32 { self.refresh_secs }

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        let Some(key) = &self.api_key else {
            tracing::warn!(
                "FlightAware requires flightaware_api_key — use: set flightaware_api_key <key>"
            );
            return Ok(Vec::new());
        };

        let (primary, _) = bounds.split();
        // AeroAPI latlong query: "lat_lo lon_lo lat_hi lon_hi"
        let query = format!(
            "-latlong \"{} {} {} {}\"",
            primary.min_lat, primary.min_lon,
            primary.max_lat, primary.max_lon,
        );

        let resp: Value = self
            .client
            .get("https://aeroapi.flightaware.com/aeroapi/flights/search")
            .header("x-apikey", key.as_str())
            .query(&[("query", &query), ("max_pages", &"1".to_string())])
            .send()
            .await
            .context("FlightAware request failed")?
            .error_for_status()
            .context("FlightAware HTTP error")?
            .json()
            .await
            .context("FlightAware JSON parse")?;

        let flights = match resp.get("flights").and_then(|v| v.as_array()) {
            Some(a) => a,
            None    => return Ok(Vec::new()),
        };

        let mut features = Vec::with_capacity(flights.len());
        for fl in flights {
            // last_position object carries the live fix
            let pos = match fl.get("last_position") {
                Some(p) => p,
                None    => continue,
            };
            let lat = match pos.get("latitude").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None    => continue,
            };
            let lon = match pos.get("longitude").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None    => continue,
            };

            let icao24 = str_key(fl, "ident_icao");
            let flight = FlightState {
                category:      AircraftCategory::from_adsb(None, &icao24),
                icao24,
                callsign:      opt_str_key(fl, "ident"),
                registration:  opt_str_key(fl, "registration"),
                aircraft_type: opt_str_key(fl, "aircraft_type"),
                lat,
                lon,
                baro_alt_ft:   f32_key(pos, "altitude").map(|v| v * 100.0), // AeroAPI: hundreds of feet
                geo_alt_ft:    None,
                velocity_kts:  f32_key(pos, "groundspeed"),
                heading:       f32_key(pos, "heading"),
                vert_rate_fpm: None,
                on_ground:     pos.get("altitude").and_then(|v| v.as_i64()).unwrap_or(1) == 0,
                squawk:        opt_str_key(pos, "squawk"),
                origin_country: None,
                last_seen:     0,
            };

            features.push(GeoFeature::from(flight));
        }

        tracing::info!(source = "flightaware", count = features.len(), "fetched flights");
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
