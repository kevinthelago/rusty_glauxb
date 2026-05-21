pub mod classifier;
pub mod icons;
pub mod render;
pub mod standard;
pub mod symbol_map;
pub mod svg_render;

// ─── Affiliation ──────────────────────────────────────────────────────────────

/// MIL-STD-2525D standard affiliation — determines frame shape and color.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Affiliation {
    Friend,   // Blue rectangle
    Hostile,  // Red diamond
    Neutral,  // Green square
    #[default]
    Unknown,  // Yellow circle
}

impl Affiliation {
    pub fn frame_color(&self) -> egui::Color32 {
        match self {
            Self::Friend  => egui::Color32::from_rgb(0, 107, 186),
            Self::Hostile => egui::Color32::from_rgb(220, 35, 35),
            Self::Neutral => egui::Color32::from_rgb(0, 160, 60),
            Self::Unknown => egui::Color32::from_rgb(225, 175, 0),
        }
    }

    pub fn fill_color(&self) -> egui::Color32 {
        match self {
            Self::Friend  => egui::Color32::from_rgba_unmultiplied(0, 107, 186, 55),
            Self::Hostile => egui::Color32::from_rgba_unmultiplied(220, 35, 35, 55),
            Self::Neutral => egui::Color32::from_rgba_unmultiplied(0, 160, 60, 55),
            Self::Unknown => egui::Color32::from_rgba_unmultiplied(225, 175, 0, 55),
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "friend" | "friendly" | "blue" => Some(Self::Friend),
            "hostile" | "enemy" | "red"    => Some(Self::Hostile),
            "neutral" | "green"            => Some(Self::Neutral),
            "unknown" | "yellow"           => Some(Self::Unknown),
            _                              => None,
        }
    }
}

impl std::fmt::Display for Affiliation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Friend  => write!(f, "friend"),
            Self::Hostile => write!(f, "hostile"),
            Self::Neutral => write!(f, "neutral"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ─── Function IDs ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AirFunction {
    FixedWing,
    Rotary,
    MilitaryFixedWing,
    MilitaryRotary,
    Unmanned,
    Tanker,
    Bomber,
    Fighter,
    Transport,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroundFunction {
    Infantry,
    MechanizedInfantry,
    Armor,
    Artillery,
    AntiAircraft,
    Engineer,
    HQ,
    Reconnaissance,
    SpecialForces,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavalFunction {
    Surface,
    Submarine,
    Carrier,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FunctionId {
    Air(AirFunction),
    Ground(GroundFunction),
    Naval(NavalFunction),
}

// ─── Echelon ──────────────────────────────────────────────────────────────────

/// Unit size / echelon indicator — drawn above the frame.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Echelon {
    #[default]
    Unknown,
    Team,
    Squad,
    Section,
    Platoon,
    Company,
    Battalion,
    Regiment,
    Brigade,
    Division,
    Corps,
    Army,
}

impl Echelon {
    pub fn indicator(&self) -> Option<&'static str> {
        match self {
            Self::Unknown   => None,
            Self::Team      => Some("\u{00B7}"),      // ·
            Self::Squad     => Some("\u{00B7}\u{00B7}"), // ··
            Self::Section   => Some("\u{00B7}\u{00B7}\u{00B7}"),
            Self::Platoon   => Some("|"),
            Self::Company   => Some("I"),
            Self::Battalion => Some("II"),
            Self::Regiment  => Some("III"),
            Self::Brigade   => Some("X"),
            Self::Division  => Some("XX"),
            Self::Corps     => Some("XXX"),
            Self::Army      => Some("XXXX"),
        }
    }
}

// ─── MilSymbol ────────────────────────────────────────────────────────────────

/// A complete MIL-STD-2525D military symbol descriptor.
#[derive(Clone, Debug)]
pub struct MilSymbol {
    pub affiliation: Affiliation,
    pub function:    FunctionId,
    pub echelon:     Echelon,
    /// Optional unit designator label drawn below the symbol.
    pub designator:  Option<String>,
}
