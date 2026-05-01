use std::sync::Arc;

use crate::{
    geo::{FeatureKind, GeoFeature},
    pipeline::DataSource,
    settings::{FlightSource, Settings},
};

pub mod adsbexchange;
pub mod adsbfi;
pub mod flightaware;
pub mod opensky;

// ─── Aircraft category ────────────────────────────────────────────────────────

/// Broad visual category used to select the mesh drawn for each aircraft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AircraftCategory {
    /// Light piston/turboprop single-engine (ICAO category A1).
    SingleEngine = 0,
    /// Multi-engine piston, turboprop, or jet airliner (A2–A5).
    MultiEngine  = 1,
    /// Military aircraft — detected from ICAO24 hex range or category A6.
    Military     = 2,
    /// Everything else: rotorcraft, gliders, balloons, UAVs, unknown.
    Misc         = 3,
}

impl AircraftCategory {
    /// Derive a category from the raw ADSB category string (e.g. "A1") and the
    /// aircraft's 24-bit ICAO address.
    pub fn from_adsb(category: Option<&str>, icao24: &str) -> Self {
        if is_military_icao(icao24) {
            return Self::Military;
        }
        match category {
            Some("A1")                                              => Self::SingleEngine,
            Some("A2") | Some("A3") | Some("A4") | Some("A5")     => Self::MultiEngine,
            Some("A6")                                              => Self::Military,
            _                                                       => Self::Misc,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingleEngine => "single_engine",
            Self::MultiEngine  => "multi_engine",
            Self::Military     => "military",
            Self::Misc         => "misc",
        }
    }
}

/// Returns `true` for ICAO24 codes that fall in well-known military hex ranges.
fn is_military_icao(icao24: &str) -> bool {
    let Ok(code) = u32::from_str_radix(icao24.trim_start_matches("0x"), 16) else {
        return false;
    };
    // US Air Force / Navy / Army
    (0xAE0000..=0xAEFFFF).contains(&code) ||
    // UK military
    (0x43C000..=0x43FFFF).contains(&code) ||
    // France military
    (0x3B0000..=0x3BFFFF).contains(&code) ||
    // Germany military
    (0x3DC000..=0x3DFFFF).contains(&code) ||
    // Australia military
    (0x7C0000..=0x7C1FFF).contains(&code)
}

// ─── FlightState ─────────────────────────────────────────────────────────────

/// Full live state of one aircraft as returned by any source.
#[derive(Clone, Debug)]
pub struct FlightState {
    pub icao24:         String,
    pub callsign:       Option<String>,
    pub registration:   Option<String>,
    pub aircraft_type:  Option<String>,
    pub lat:            f32,
    pub lon:            f32,
    /// Barometric altitude in feet.
    pub baro_alt_ft:    Option<f32>,
    /// GNSS/geometric altitude in feet.
    pub geo_alt_ft:     Option<f32>,
    /// Ground speed in knots.
    pub velocity_kts:   Option<f32>,
    /// True track / heading in degrees (0 = north).
    pub heading:        Option<f32>,
    /// Vertical rate in feet per minute (positive = climbing).
    pub vert_rate_fpm:  Option<f32>,
    pub on_ground:      bool,
    pub squawk:         Option<String>,
    pub origin_country: Option<String>,
    /// Unix timestamp of the last ADS-B message.
    pub last_seen:      u64,
    pub category:       AircraftCategory,
}

impl From<FlightState> for GeoFeature {
    fn from(f: FlightState) -> Self {
        let alt_m = f.baro_alt_ft.or(f.geo_alt_ft).unwrap_or(0.0) * 0.3048;
        GeoFeature {
            lat:  f.lat,
            lon:  f.lon,
            alt:  alt_m,
            kind: FeatureKind::Point,
            meta: serde_json::json!({
                "icao24":             f.icao24,
                "callsign":           f.callsign,
                "registration":       f.registration,
                "type":               f.aircraft_type,
                "baro_alt_ft":        f.baro_alt_ft,
                "geo_alt_ft":         f.geo_alt_ft,
                "velocity_kts":       f.velocity_kts,
                "heading":            f.heading,
                "vert_rate_fpm":      f.vert_rate_fpm,
                "on_ground":          f.on_ground,
                "squawk":             f.squawk,
                "origin_country":     f.origin_country,
                "last_seen":          f.last_seen,
                "aircraft_category":  f.category.as_str(),
            }),
            coordinates: Vec::new(),
        }
    }
}

// ─── Source factory ───────────────────────────────────────────────────────────

/// Build the active flight `DataSource` from current settings.
pub fn create_source(settings: &Settings) -> Arc<dyn DataSource> {
    match settings.flight_source {
        FlightSource::OpenSky => Arc::new(opensky::OpenSkySource::new(
            settings.opensky_user.clone(),
            settings.opensky_pass.clone(),
            settings.flight_refresh_secs,
        )),
        FlightSource::AdsbExchange => Arc::new(adsbexchange::AdsbExchangeSource::new(
            settings.adsbx_api_key.clone(),
            settings.flight_refresh_secs,
        )),
        FlightSource::AdsbFi => Arc::new(adsbfi::AdsbFiSource::new(
            settings.flight_refresh_secs,
        )),
        FlightSource::FlightAware => Arc::new(flightaware::FlightAwareSource::new(
            settings.flightaware_api_key.clone(),
            settings.flight_refresh_secs,
        )),
    }
}
