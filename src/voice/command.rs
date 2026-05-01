/// A parsed instruction produced by the LLM from transcribed speech.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VoiceCommand {
    /// The raw transcript from STT (kept for HUD display).
    #[serde(default)]
    pub transcript: String,
    /// Layer names to show; empty = no change; ["all"] = show all.
    #[serde(default)]
    pub layers: Vec<String>,
    /// Optional camera target (defaults to current position when omitted).
    pub location: Option<GeoTarget>,
    /// Suggested zoom hint.
    #[allow(dead_code)]
    pub zoom: Option<ZoomLevel>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GeoTarget {
    pub lat: f32,
    pub lon: f32,
    #[allow(dead_code)]
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoomLevel {
    Globe,
    Continent,
    Country,
    City,
    Street,
}
