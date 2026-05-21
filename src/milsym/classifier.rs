use super::{Affiliation, AirFunction, Echelon, FunctionId, MilSymbol};

/// Classification rules: maps ADS-B category strings to affiliations.
pub struct ClassifierConfig {
    pub civil_affiliation:    Affiliation,
    pub military_affiliation: Affiliation,
    pub unknown_affiliation:  Affiliation,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            civil_affiliation:    Affiliation::Friend,
            military_affiliation: Affiliation::Hostile,
            unknown_affiliation:  Affiliation::Unknown,
        }
    }
}

impl ClassifierConfig {
    pub fn from_settings(civil: &str, military: &str, unknown: &str) -> Self {
        Self {
            civil_affiliation:    Affiliation::from_str(civil).unwrap_or(Affiliation::Friend),
            military_affiliation: Affiliation::from_str(military).unwrap_or(Affiliation::Hostile),
            unknown_affiliation:  Affiliation::from_str(unknown).unwrap_or(Affiliation::Unknown),
        }
    }
}

/// Map an ADS-B `aircraft_category` string to a `MilSymbol`.
///
/// Category strings come from the `aircraft_category` metadata field in the
/// flight feed (e.g. "single_engine", "multi_engine", "military", "misc").
pub fn classify_aircraft(category: &str, config: &ClassifierConfig) -> MilSymbol {
    let (affiliation, air_fn) = match category {
        "military"      => (config.military_affiliation.clone(), AirFunction::MilitaryFixedWing),
        "single_engine" => (config.civil_affiliation.clone(),    AirFunction::FixedWing),
        "multi_engine"  => (config.civil_affiliation.clone(),    AirFunction::Transport),
        _               => (config.unknown_affiliation.clone(),  AirFunction::Generic),
    };

    MilSymbol {
        affiliation,
        function:   FunctionId::Air(air_fn),
        echelon:    Echelon::Unknown,
        designator: None,
    }
}
