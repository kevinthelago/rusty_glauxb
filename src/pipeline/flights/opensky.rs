//! OpenSky Network — free REST API, optional basic auth for better rate limits.
//!
//! Docs: https://openskynetwork.github.io/opensky-api/rest.html
//! Endpoint: GET https://opensky-network.org/api/states/all?lamin=&lomin=&lamax=&lomax=
//!
//! Rate limits (unauthenticated): 10 req/min, 400 req/day
//! Rate limits (authenticated):   100 req/min, 4 000 req/day

use anyhow::Context as _;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::{
    geo::{GeoBounds, GeoFeature},
    pipeline::DataSource,
};

use super::{AircraftCategory, FlightState};

pub struct OpenSkySource {
    client:       Client,
    user:         Option<String>,
    pass:         Option<String>,
    refresh_secs: f32,
}

impl OpenSkySource {
    pub fn new(user: Option<String>, pass: Option<String>, refresh_secs: f32) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("HTTP client"),
            user,
            pass,
            refresh_secs,
        }
    }
}

#[async_trait]
impl DataSource for OpenSkySource {
    fn name(&self)       -> &str { "OpenSky Network" }
    fn layer_name(&self) -> &str { "flights" }
    fn max_staleness_secs(&self) -> f32 { self.refresh_secs }

    async fn fetch(&self, bounds: &GeoBounds) -> anyhow::Result<Vec<GeoFeature>> {
        // OpenSky expects normalized [-180, 180] coordinates; use primary box.
        let (primary, _) = bounds.split();

        let url = format!(
            "https://opensky-network.org/api/states/all\
             ?lamin={}&lomin={}&lamax={}&lomax={}",
            primary.min_lat, primary.min_lon,
            primary.max_lat, primary.max_lon,
        );

        let mut req = self.client.get(&url);
        if let (Some(u), Some(p)) = (&self.user, &self.pass) {
            req = req.basic_auth(u, Some(p));
        }

        let resp: Value = req
            .send()
            .await
            .context("OpenSky request failed")?
            .error_for_status()
            .context("OpenSky HTTP error")?
            .json()
            .await
            .context("OpenSky JSON parse")?;

        let states = match resp.get("states").and_then(|v| v.as_array()) {
            Some(a) => a,
            None    => return Ok(Vec::new()),
        };

        let time = resp.get("time").and_then(|v| v.as_u64()).unwrap_or(0);

        let mut features = Vec::with_capacity(states.len());
        for state in states {
            let arr = match state.as_array() {
                Some(a) if a.len() >= 17 => a,
                _ => continue,
            };

            let lon = match arr[5].as_f64() { Some(v) => v as f32, None => continue };
            let lat = match arr[6].as_f64() { Some(v) => v as f32, None => continue };

            let icao24 = str_field(&arr[0]);
            let flight = FlightState {
                category:       AircraftCategory::from_adsb(None, &icao24),
                icao24,
                callsign:       opt_str(&arr[1]),
                registration:   None,
                aircraft_type:  None,
                lat,
                lon,
                // OpenSky returns metres; convert to feet
                baro_alt_ft:    arr[7].as_f64().map(|m| (m * 3.28084) as f32),
                geo_alt_ft:     arr[13].as_f64().map(|m| (m * 3.28084) as f32),
                // m/s → knots (1 m/s ≈ 1.94384 kts)
                velocity_kts:   arr[9].as_f64().map(|v| (v * 1.94384) as f32),
                heading:        arr[10].as_f64().map(|v| v as f32),
                // m/s → fpm (1 m/s = 196.85 fpm)
                vert_rate_fpm:  arr[11].as_f64().map(|v| (v * 196.85) as f32),
                on_ground:      arr[8].as_bool().unwrap_or(false),
                squawk:         opt_str(&arr[14]),
                origin_country: opt_str(&arr[2]),
                last_seen:      arr[4].as_u64().unwrap_or(time),
            };

            features.push(GeoFeature::from(flight));
        }

        tracing::info!(source = "opensky", count = features.len(), "fetched flights");
        Ok(features)
    }
}

fn str_field(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

fn opt_str(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}
