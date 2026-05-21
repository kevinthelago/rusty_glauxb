use crate::hud::HudGeoFeature;

/// Call the Anthropic Messages API and return a 2-3 sentence geographic
/// summary for `feat`.  Requires `ANTHROPIC_API_KEY` in the environment;
/// returns an error (which the caller silently ignores) if the variable is
/// absent or the request fails.
pub async fn fetch(feat: &HudGeoFeature) -> anyhow::Result<String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;

    // Build a compact description from available GeoJSON properties.
    let mut parts: Vec<String> = Vec::new();
    if let Some(name) = feat.display_name() {
        parts.push(format!("Name: {name}"));
    }
    parts.push(format!("Layer: {}", feat.layer_name));
    if let Some(elev) = feat.elevation() {
        parts.push(format!("Elevation: {elev:.0} m"));
    }
    parts.push(format!("Coordinates: {:.4}°, {:.4}°", feat.lat, feat.lon));
    if let Some(obj) = feat.properties.as_object() {
        for (key, val) in obj {
            if ["name", "Name", "NAME", "name_long"].contains(&key.as_str()) { continue; }
            if let Some(s) = val.as_str().filter(|s| !s.is_empty()) {
                parts.push(format!("{key}: {s}"));
            }
        }
    }

    let context = parts.join("; ");
    let prompt = format!(
        "Give a concise 2-3 sentence geographic summary of this location: {context}. \
         Focus on what makes it geographically or historically notable. Be factual and brief."
    );

    let body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 300,
        "messages": [{"role": "user", "content": prompt}]
    });

    let resp = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;

    resp["content"][0]["text"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("unexpected Anthropic API response shape"))
}
