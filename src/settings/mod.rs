use std::{fmt, path::PathBuf, str::FromStr};
use serde::{Deserialize, Serialize};

use crate::globe::anim::CameraMode;

// ─── FlightSource ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightSource {
    OpenSky,
    AdsbExchange,
    AdsbFi,
    FlightAware,
}

impl Default for FlightSource {
    fn default() -> Self { Self::OpenSky }
}

impl fmt::Display for FlightSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenSky      => write!(f, "opensky"),
            Self::AdsbExchange => write!(f, "adsbexchange"),
            Self::AdsbFi       => write!(f, "adsbfi"),
            Self::FlightAware  => write!(f, "flightaware"),
        }
    }
}

impl FromStr for FlightSource {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "opensky"      => Ok(Self::OpenSky),
            "adsbexchange" => Ok(Self::AdsbExchange),
            "adsbfi"       => Ok(Self::AdsbFi),
            "flightaware"  => Ok(Self::FlightAware),
            other          => Err(anyhow::anyhow!("unknown flight source: {other}")),
        }
    }
}

// ─── Settings ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    // ── Flight data ───────────────────────────────────────────────────────────
    pub flight_source:       FlightSource,
    pub flight_refresh_secs: f32,
    pub opensky_user:        Option<String>,
    pub opensky_pass:        Option<String>,
    pub adsbx_api_key:       Option<String>,
    pub flightaware_api_key: Option<String>,

    // ── Camera animation ──────────────────────────────────────────────────────
    /// Active animation mode: "none", "rotate", or "follow".
    pub camera_mode:       String,
    /// Auto-rotate speed in degrees per second.
    pub rotate_speed:      f32,
    /// Spring time constant for rotate wind-up/wind-down [0, 1] (higher = springier).
    pub rotate_elasticity: f32,
    /// ICAO24 (or other string ID) of the entity to follow.
    pub follow_entity:     String,
    /// Smoothing factor for follow mode [0, 1] (higher = smoother / slower response).
    pub follow_smoothing:  f32,
    /// Camera radius (globe radii) while following.
    pub follow_distance:   f32,

    // ── Follow overlay ────────────────────────────────────────────────────────
    /// Show the HUD info panel while following an entity.
    pub follow_overlay:        bool,
    /// Comma-separated list of fields shown in the overlay.
    pub follow_overlay_fields: String,

    // ── Drag interrupt / idle resume ──────────────────────────────────────────
    /// Seconds of idle after a drag before the default animation is restored.
    /// 0.0 = never auto-resume.
    pub idle_resume_secs: f32,

    // ── Directions routing ────────────────────────────────────────────────────
    /// Active routing source: "osrm" or "google".
    pub directions_source:   String,
    pub google_maps_api_key: Option<String>,

    // ── Military symbology ────────────────────────────────────────────────────
    /// Replace entity dots with MIL-STD-2525D symbols.
    pub milsym_enabled:              bool,
    /// Scale multiplier for symbol size (1.0 = default ~20px frame).
    pub milsym_scale:                f32,
    /// Show echelon indicator above the frame.
    pub milsym_show_echelon:         bool,
    /// Affiliation assigned to civil aircraft (single_engine / multi_engine).
    pub milsym_civil_affiliation:    String,
    /// Affiliation assigned to military aircraft.
    pub milsym_military_affiliation: String,
    /// Affiliation assigned to aircraft with unknown/misc category.
    pub milsym_unknown_affiliation:  String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            flight_source:       FlightSource::default(),
            flight_refresh_secs: 15.0,
            opensky_user:        None,
            opensky_pass:        None,
            adsbx_api_key:       None,
            flightaware_api_key: None,

            camera_mode:       "rotate".into(),
            rotate_speed:      10.0,
            rotate_elasticity: 0.4,
            follow_entity:     String::new(),
            follow_smoothing:  0.15,
            follow_distance:   3.0,

            follow_overlay:        true,
            follow_overlay_fields: "callsign,type,altitude,speed,heading,position".into(),

            idle_resume_secs: 30.0,

            directions_source:   "osrm".into(),
            google_maps_api_key: None,

            milsym_enabled:              false,
            milsym_scale:                1.0,
            milsym_show_echelon:         true,
            milsym_civil_affiliation:    "friend".into(),
            milsym_military_affiliation: "hostile".into(),
            milsym_unknown_affiliation:  "unknown".into(),
        }
    }
}

impl Settings {
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("rusty_glauxb").join("settings.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        toml::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, text);
        }
    }

    /// Build the `CameraMode` value from the current settings fields.
    pub fn to_camera_mode(&self) -> CameraMode {
        match self.camera_mode.as_str() {
            "rotate" => CameraMode::Rotate {
                speed_deg:  self.rotate_speed,
                elasticity: self.rotate_elasticity,
            },
            "follow" => CameraMode::Follow {
                entity_id: self.follow_entity.clone(),
                smoothing: self.follow_smoothing,
                distance:  self.follow_distance,
            },
            "follow_location" => CameraMode::FollowLocation {
                smoothing: self.follow_smoothing,
                distance:  self.follow_distance,
            },
            _ => CameraMode::None,
        }
    }

    /// Set a named key. Returns a result message for console output.
    pub fn set(&mut self, key: &str, value: &str) -> String {
        match key {
            // ── Flight ───────────────────────────────────────────────────────
            "flight_source" => match value.parse::<FlightSource>() {
                Ok(s)  => { self.flight_source = s; format!("flight_source = {}", self.flight_source) }
                Err(e) => format!("error: {e}"),
            },
            "flight_refresh_secs" => match value.parse::<f32>() {
                Ok(v) if v > 0.0 => { self.flight_refresh_secs = v; format!("flight_refresh_secs = {v}") }
                Ok(_)            => "error: must be > 0".into(),
                Err(e)           => format!("error: {e}"),
            },
            "opensky_user" => {
                self.opensky_user = non_empty(value);
                format!("opensky_user = {value}")
            }
            "opensky_pass" => {
                self.opensky_pass = non_empty(value);
                "opensky_pass = ***".into()
            }
            "adsbx_api_key" => {
                self.adsbx_api_key = non_empty(value);
                "adsbx_api_key = ***".into()
            }
            "flightaware_api_key" => {
                self.flightaware_api_key = non_empty(value);
                "flightaware_api_key = ***".into()
            }

            // ── Camera animation ─────────────────────────────────────────────
            "camera_mode" => match value {
                "none" | "rotate" | "follow" | "follow_location" => {
                    self.camera_mode = value.into();
                    format!("camera_mode = {value}")
                }
                _ => "error: expected none, rotate, follow, or follow_location".into(),
            },
            "rotate_speed" => match value.parse::<f32>() {
                Ok(v) => { self.rotate_speed = v; format!("rotate_speed = {v}") }
                Err(e) => format!("error: {e}"),
            },
            "rotate_elasticity" => match value.parse::<f32>() {
                Ok(v) if (0.0..=1.0).contains(&v) => { self.rotate_elasticity = v; format!("rotate_elasticity = {v}") }
                Ok(_)  => "error: must be 0.0 – 1.0".into(),
                Err(e) => format!("error: {e}"),
            },
            "follow_entity" => {
                self.follow_entity = value.into();
                format!("follow_entity = {value}")
            }
            "follow_smoothing" => match value.parse::<f32>() {
                Ok(v) if (0.0..=1.0).contains(&v) => { self.follow_smoothing = v; format!("follow_smoothing = {v}") }
                Ok(_)  => "error: must be 0.0 – 1.0".into(),
                Err(e) => format!("error: {e}"),
            },
            "follow_distance" => match value.parse::<f32>() {
                Ok(v) if v > 1.0 => { self.follow_distance = v; format!("follow_distance = {v}") }
                Ok(_)  => "error: must be > 1.0".into(),
                Err(e) => format!("error: {e}"),
            },
            "follow_overlay" => match value {
                "on"  | "true"  | "1" => { self.follow_overlay = true;  "follow_overlay = on".into() }
                "off" | "false" | "0" => { self.follow_overlay = false; "follow_overlay = off".into() }
                _ => "error: expected on or off".into(),
            },
            "follow_overlay_fields" => {
                // Accept space- or comma-separated; store normalised as comma-separated.
                let normalised: String = value
                    .split(|c: char| c == ',' || c == ' ')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(",");
                self.follow_overlay_fields = normalised.clone();
                format!("follow_overlay_fields = {normalised}")
            }
            "idle_resume_secs" => match value.parse::<f32>() {
                Ok(v) if v >= 0.0 => { self.idle_resume_secs = v; format!("idle_resume_secs = {v}") }
                Ok(_)  => "error: must be >= 0".into(),
                Err(e) => format!("error: {e}"),
            },

            // ── Directions ───────────────────────────────────────────────────
            "directions_source" => match value {
                "osrm" | "google" => { self.directions_source = value.into(); format!("directions_source = {value}") }
                _ => "error: expected osrm or google".into(),
            },
            "google_maps_api_key" => {
                self.google_maps_api_key = non_empty(value);
                "google_maps_api_key = ***".into()
            }

            // ── Milsym ───────────────────────────────────────────────────────
            "milsym_enabled" => match value {
                "on"  | "true"  | "1" => { self.milsym_enabled = true;  "milsym_enabled = on".into() }
                "off" | "false" | "0" => { self.milsym_enabled = false; "milsym_enabled = off".into() }
                _ => "error: expected on or off".into(),
            },
            "milsym_scale" => match value.parse::<f32>() {
                Ok(v) if v > 0.0 => { self.milsym_scale = v; format!("milsym_scale = {v}") }
                Ok(_)  => "error: must be > 0".into(),
                Err(e) => format!("error: {e}"),
            },
            "milsym_show_echelon" => match value {
                "on"  | "true"  | "1" => { self.milsym_show_echelon = true;  "milsym_show_echelon = on".into() }
                "off" | "false" | "0" => { self.milsym_show_echelon = false; "milsym_show_echelon = off".into() }
                _ => "error: expected on or off".into(),
            },
            "milsym_civil_affiliation" => {
                self.milsym_civil_affiliation = value.into();
                format!("milsym_civil_affiliation = {value}")
            }
            "milsym_military_affiliation" => {
                self.milsym_military_affiliation = value.into();
                format!("milsym_military_affiliation = {value}")
            }
            "milsym_unknown_affiliation" => {
                self.milsym_unknown_affiliation = value.into();
                format!("milsym_unknown_affiliation = {value}")
            }

            _ => format!("unknown key: {key}"),
        }
    }

    pub fn get(&self, key: &str) -> String {
        match key {
            "flight_source"       => self.flight_source.to_string(),
            "flight_refresh_secs" => self.flight_refresh_secs.to_string(),
            "opensky_user"        => self.opensky_user.clone().unwrap_or_else(|| "(none)".into()),
            "opensky_pass"        => masked(&self.opensky_pass),
            "adsbx_api_key"       => masked(&self.adsbx_api_key),
            "flightaware_api_key" => masked(&self.flightaware_api_key),
            "camera_mode"         => self.camera_mode.clone(),
            "rotate_speed"        => self.rotate_speed.to_string(),
            "rotate_elasticity"   => self.rotate_elasticity.to_string(),
            "follow_entity"       => self.follow_entity.clone(),
            "follow_smoothing"       => self.follow_smoothing.to_string(),
            "follow_distance"        => self.follow_distance.to_string(),
            "follow_overlay"         => if self.follow_overlay { "on".into() } else { "off".into() },
            "follow_overlay_fields"  => self.follow_overlay_fields.clone(),
            "idle_resume_secs"       => self.idle_resume_secs.to_string(),
            "directions_source"      => self.directions_source.clone(),
            "google_maps_api_key"    => masked(&self.google_maps_api_key),
            "milsym_enabled"              => if self.milsym_enabled { "on".into() } else { "off".into() },
            "milsym_scale"                => self.milsym_scale.to_string(),
            "milsym_show_echelon"         => if self.milsym_show_echelon { "on".into() } else { "off".into() },
            "milsym_civil_affiliation"    => self.milsym_civil_affiliation.clone(),
            "milsym_military_affiliation" => self.milsym_military_affiliation.clone(),
            "milsym_unknown_affiliation"  => self.milsym_unknown_affiliation.clone(),
            _                        => format!("unknown key: {key}"),
        }
    }

    pub fn all_keys() -> &'static [&'static str] {
        &[
            "flight_source",
            "flight_refresh_secs",
            "opensky_user",
            "opensky_pass",
            "adsbx_api_key",
            "flightaware_api_key",
            "camera_mode",
            "rotate_speed",
            "rotate_elasticity",
            "follow_entity",
            "follow_smoothing",
            "follow_distance",
            "follow_overlay",
            "follow_overlay_fields",
            "idle_resume_secs",
            "directions_source",
            "google_maps_api_key",
            "milsym_enabled",
            "milsym_scale",
            "milsym_show_echelon",
            "milsym_civil_affiliation",
            "milsym_military_affiliation",
            "milsym_unknown_affiliation",
        ]
    }
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.into()) }
}

fn masked(opt: &Option<String>) -> String {
    if opt.is_some() { "***".into() } else { "(none)".into() }
}
