/// Which NATO/US symbology standard is active.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SymbologyStandard {
    /// NATO APP-6A (and later revisions APP-6B/C/E).
    #[default]
    App6A,
    /// US MIL-STD-2525D (Joint Military Symbology).
    MilStd2525D,
}

impl SymbologyStandard {
    pub fn id(&self) -> &'static str {
        match self {
            Self::App6A       => "app6a",
            Self::MilStd2525D => "milstd2525d",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::App6A       => "APP-6A",
            Self::MilStd2525D => "MIL-STD-2525D",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "app6a" | "app-6a" | "app6" => Some(Self::App6A),
            "milstd2525d" | "milstd2525" | "2525d" | "2525" => Some(Self::MilStd2525D),
            _ => None,
        }
    }
}

impl std::fmt::Display for SymbologyStandard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
