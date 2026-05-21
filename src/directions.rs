use anyhow::Context as _;

use crate::geo::{FeatureKind, GeoFeature};

// ─── Source enum ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Default)]
pub enum DirectionsSource {
    #[default]
    Osrm,
    Google,
}

impl std::fmt::Display for DirectionsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Osrm   => write!(f, "osrm"),
            Self::Google => write!(f, "google"),
        }
    }
}

impl std::str::FromStr for DirectionsSource {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "osrm"   => Ok(Self::Osrm),
            "google" => Ok(Self::Google),
            other    => Err(anyhow::anyhow!("unknown directions source '{other}' — valid: osrm, google")),
        }
    }
}

// ─── Result type ──────────────────────────────────────────────────────────────

pub struct DirectionsResult {
    pub features: Vec<GeoFeature>,
    /// Human-readable summary, e.g. "559 km · 5 h 32 min".
    pub summary:  String,
}

// ─── Public fetch entry-point ─────────────────────────────────────────────────

/// Route from `from` to `to`, where each can be `"lat,lon"` or a free-form address.
pub async fn fetch(
    from:    &str,
    to:      &str,
    source:  &DirectionsSource,
    api_key: Option<&str>,
) -> anyhow::Result<DirectionsResult> {
    match source {
        DirectionsSource::Google => fetch_google(from, to,
            api_key.context("google_maps_api_key is not set — run: set google_maps_api_key <key>")?).await,
        DirectionsSource::Osrm => fetch_osrm(from, to).await,
    }
}

// ─── Google Maps Directions API ───────────────────────────────────────────────

async fn fetch_google(from: &str, to: &str, api_key: &str) -> anyhow::Result<DirectionsResult> {
    let resp: serde_json::Value = reqwest::Client::new()
        .get("https://maps.googleapis.com/maps/api/directions/json")
        .query(&[("origin", from), ("destination", to), ("key", api_key)])
        .send().await.context("Google Directions request failed")?
        .error_for_status().context("Google Directions HTTP error")?
        .json().await.context("parsing Google Directions response")?;

    match resp["status"].as_str().unwrap_or("UNKNOWN") {
        "OK"             => {}
        "REQUEST_DENIED" => anyhow::bail!("Google API request denied — check google_maps_api_key"),
        "ZERO_RESULTS"   => anyhow::bail!("no route found between those locations"),
        "NOT_FOUND"      => anyhow::bail!("could not geocode one or both locations"),
        other            => anyhow::bail!("Google Directions API error: {other}"),
    }

    let route = &resp["routes"][0];
    let encoded = route["overview_polyline"]["points"]
        .as_str()
        .context("no polyline in Google Directions response")?;

    let dist = route["legs"][0]["distance"]["text"].as_str().unwrap_or("?");
    let dur  = route["legs"][0]["duration"]["text"].as_str().unwrap_or("?");

    let coords = decode_polyline(encoded);
    anyhow::ensure!(!coords.is_empty(), "Google Directions returned empty route");

    Ok(DirectionsResult {
        features: vec![route_feature(coords)],
        summary:  format!("{dist} · {dur}"),
    })
}

// ─── OSRM (Open Source Routing Machine) ──────────────────────────────────────

async fn fetch_osrm(from: &str, to: &str) -> anyhow::Result<DirectionsResult> {
    let (flat, flon) = resolve_location(from).await
        .with_context(|| format!("could not resolve '{from}'"))?;
    let (tlat, tlon) = resolve_location(to).await
        .with_context(|| format!("could not resolve '{to}'"))?;

    // OSRM takes coordinates as lon,lat (GeoJSON order).
    let url = format!(
        "https://router.project-osrm.org/route/v1/driving/{},{};{},{}?overview=full&geometries=geojson",
        flon, flat, tlon, tlat
    );

    let resp: serde_json::Value = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "rusty_glauxb/0.1")
        .send().await.context("OSRM request failed")?
        .error_for_status().context("OSRM HTTP error")?
        .json().await.context("parsing OSRM response")?;

    anyhow::ensure!(
        resp["code"].as_str() == Some("Ok"),
        "OSRM routing failed: {}",
        resp["message"].as_str().unwrap_or("unknown error")
    );

    let coords_raw = resp["routes"][0]["geometry"]["coordinates"]
        .as_array()
        .context("no route geometry in OSRM response")?;

    let coords: Vec<[f32; 2]> = coords_raw.iter()
        .filter_map(|v| {
            let lon = v.get(0)?.as_f64()? as f32;
            let lat = v.get(1)?.as_f64()? as f32;
            Some([lat, lon])
        })
        .collect();

    anyhow::ensure!(!coords.is_empty(), "OSRM returned empty route");

    let dist_m = resp["routes"][0]["distance"].as_f64().unwrap_or(0.0);
    let dur_s  = resp["routes"][0]["duration"].as_f64().unwrap_or(0.0);

    Ok(DirectionsResult {
        features: vec![route_feature(coords)],
        summary:  format_summary(dist_m, dur_s),
    })
}

fn format_summary(dist_m: f64, dur_s: f64) -> String {
    let dist_str = if dist_m >= 1000.0 {
        format!("{:.0} km", dist_m / 1000.0)
    } else {
        format!("{dist_m:.0} m")
    };
    let h = (dur_s / 3600.0) as u32;
    let m = ((dur_s % 3600.0) / 60.0) as u32;
    let dur_str = if h > 0 { format!("{h} h {m} min") } else { format!("{m} min") };
    format!("{dist_str} · {dur_str}")
}

// ─── Geocoding (Nominatim) ────────────────────────────────────────────────────

/// Parse `"lat,lon"` or geocode a free-form address string via Nominatim.
async fn resolve_location(q: &str) -> anyhow::Result<(f32, f32)> {
    let q = q.trim();
    if let Some((a, b)) = q.split_once(',') {
        if let (Ok(lat), Ok(lon)) = (a.trim().parse::<f32>(), b.trim().parse::<f32>()) {
            return Ok((lat, lon));
        }
    }
    geocode_nominatim(q).await
}

async fn geocode_nominatim(q: &str) -> anyhow::Result<(f32, f32)> {
    let hits: serde_json::Value = reqwest::Client::new()
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("q", q), ("format", "json"), ("limit", "1"), ("accept-language", "en")])
        .header("User-Agent", "rusty_glauxb/0.1")
        .send().await.context("Nominatim geocoding request failed")?
        .error_for_status().context("Nominatim HTTP error")?
        .json().await.context("parsing Nominatim response")?;

    let hit = hits.as_array()
        .and_then(|a| a.first())
        .with_context(|| format!("no geocoding result for '{q}'"))?;

    let lat = hit["lat"].as_str()
        .and_then(|s| s.parse::<f32>().ok())
        .context("missing lat in geocoding result")?;
    let lon = hit["lon"].as_str()
        .and_then(|s| s.parse::<f32>().ok())
        .context("missing lon in geocoding result")?;

    tracing::debug!(q, lat, lon, "geocoded via Nominatim");
    Ok((lat, lon))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn route_feature(coordinates: Vec<[f32; 2]>) -> GeoFeature {
    let (lat, lon) = coordinates.first().map(|c| (c[0], c[1])).unwrap_or_default();
    GeoFeature {
        lat,
        lon,
        alt:  0.0,
        kind: FeatureKind::Line,
        meta: serde_json::Value::Null,
        coordinates,
    }
}

/// Decode a Google Encoded Polyline string into `[lat, lon]` pairs.
fn decode_polyline(encoded: &str) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut lat: i64 = 0;
    let mut lng: i64 = 0;
    let bytes = encoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        for coord in [&mut lat, &mut lng] {
            let mut shift = 0i64;
            let mut val: i64 = 0;
            loop {
                if i >= bytes.len() { break; }
                let b = bytes[i] as i64 - 63;
                i += 1;
                val |= (b & 0x1f) << shift;
                shift += 5;
                if b < 0x20 { break; }
            }
            *coord += if val & 1 != 0 { !(val >> 1) } else { val >> 1 };
        }
        out.push([lat as f32 / 1e5, lng as f32 / 1e5]);
    }
    out
}
