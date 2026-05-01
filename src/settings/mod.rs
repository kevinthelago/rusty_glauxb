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
    fn default() -> Self { Self::AdsbFi }
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
            follow_smoothing:  0.5,
            follow_distance:   3.0,
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
            "follow_smoothing"    => self.follow_smoothing.to_string(),
            "follow_distance"     => self.follow_distance.to_string(),
            _                     => format!("unknown key: {key}"),
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
        ]
    }
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.into()) }
}

fn masked(opt: &Option<String>) -> String {
    if opt.is_some() { "***".into() } else { "(none)".into() }
}
