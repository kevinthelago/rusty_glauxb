use std::collections::HashMap;

use super::{AirFunction, FunctionId, GroundFunction, NavalFunction};
use super::standard::SymbologyStandard;

/// All recognised icon keys, one per function variant.
pub const ALL_KEYS: &[&str] = &[
    "air.fixed_wing",
    "air.rotary",
    "air.military_fixed_wing",
    "air.unmanned",
    "air.transport",
    "air.tanker",
    "air.bomber",
    "air.fighter",
    "air.generic",
    "ground.infantry",
    "ground.armor",
    "ground.artillery",
    "ground.mechanized_infantry",
    "ground.anti_aircraft",
    "ground.engineer",
    "ground.hq",
    "ground.reconnaissance",
    "ground.special_forces",
    "ground.generic",
    "naval.surface",
    "naval.submarine",
    "naval.carrier",
    "naval.generic",
];

/// Map a `FunctionId` to its canonical key string.
pub fn function_key(f: &FunctionId) -> &'static str {
    match f {
        FunctionId::Air(a) => match a {
            AirFunction::FixedWing         => "air.fixed_wing",
            AirFunction::Rotary            => "air.rotary",
            AirFunction::MilitaryFixedWing => "air.military_fixed_wing",
            AirFunction::MilitaryRotary    => "air.rotary",
            AirFunction::Unmanned          => "air.unmanned",
            AirFunction::Tanker            => "air.tanker",
            AirFunction::Bomber            => "air.bomber",
            AirFunction::Fighter           => "air.fighter",
            AirFunction::Transport         => "air.transport",
            AirFunction::Generic           => "air.generic",
        },
        FunctionId::Ground(g) => match g {
            GroundFunction::Infantry           => "ground.infantry",
            GroundFunction::MechanizedInfantry => "ground.mechanized_infantry",
            GroundFunction::Armor              => "ground.armor",
            GroundFunction::Artillery          => "ground.artillery",
            GroundFunction::AntiAircraft       => "ground.anti_aircraft",
            GroundFunction::Engineer           => "ground.engineer",
            GroundFunction::HQ                 => "ground.hq",
            GroundFunction::Reconnaissance     => "ground.reconnaissance",
            GroundFunction::SpecialForces      => "ground.special_forces",
            GroundFunction::Generic            => "ground.generic",
        },
        FunctionId::Naval(n) => match n {
            NavalFunction::Surface   => "naval.surface",
            NavalFunction::Submarine => "naval.submarine",
            NavalFunction::Carrier   => "naval.carrier",
            NavalFunction::Generic   => "naval.generic",
        },
    }
}

// ─── SymbolMap ────────────────────────────────────────────────────────────────

/// Stores SVG icon bytes per key for each supported symbology standard.
/// When a key has no assigned SVG the renderer falls back to the hand-drawn icon.
pub struct SymbolMap {
    pub standard: SymbologyStandard,
    app6a:        HashMap<String, Vec<u8>>,
    milstd2525d:  HashMap<String, Vec<u8>>,
}

impl Default for SymbolMap {
    fn default() -> Self {
        Self {
            standard:    SymbologyStandard::default(),
            app6a:       HashMap::new(),
            milstd2525d: HashMap::new(),
        }
    }
}

impl SymbolMap {
    fn active(&self) -> &HashMap<String, Vec<u8>> {
        match self.standard {
            SymbologyStandard::App6A       => &self.app6a,
            SymbologyStandard::MilStd2525D => &self.milstd2525d,
        }
    }

    fn active_mut(&mut self) -> &mut HashMap<String, Vec<u8>> {
        match self.standard {
            SymbologyStandard::App6A       => &mut self.app6a,
            SymbologyStandard::MilStd2525D => &mut self.milstd2525d,
        }
    }

    pub fn set_standard(&mut self, standard: SymbologyStandard) {
        self.standard = standard;
    }

    /// Assign raw SVG bytes to `key` in the currently active standard.
    pub fn assign(&mut self, key: &str, bytes: Vec<u8>) {
        self.active_mut().insert(key.to_string(), bytes);
    }

    /// Remove a custom assignment for `key`; renderer reverts to hand-drawn icon.
    pub fn clear(&mut self, key: &str) {
        self.active_mut().remove(key);
    }

    /// Return the SVG bytes for `key` in the active standard, if any are assigned.
    pub fn get_svg(&self, key: &str) -> Option<&[u8]> {
        self.active().get(key).map(Vec::as_slice)
    }

    /// List all keys with a flag indicating whether an SVG is loaded.
    pub fn list_entries(&self) -> Vec<(&'static str, bool)> {
        let map = self.active();
        ALL_KEYS.iter().map(|&k| (k, map.contains_key(k))).collect()
    }

    /// Number of SVGs loaded in the active standard.
    pub fn loaded_count(&self) -> usize {
        self.active().len()
    }
}
