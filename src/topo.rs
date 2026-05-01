//! GeoJSON → line-segment loader for the topology overlay layer.
//!
//! Handles the full GeoJSON geometry type set:
//!   Point / MultiPoint          — skipped
//!   LineString / MultiLineString — rendered directly
//!   Polygon / MultiPolygon       — outer + hole rings rendered as outlines
//!   GeometryCollection           — recurse
//!   Feature / FeatureCollection  — unwrap and recurse

use std::path::Path;

use anyhow::Context as _;
use serde_json::Value;

use crate::geo::{FeatureKind, GeoFeature};

/// Load a GeoJSON file from `path` and return all geometry as polylines.
/// Each ring/line becomes one `GeoFeature` with `kind = Line`.
pub fn load_geojson(path: &Path) -> anyhow::Result<Vec<GeoFeature>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let root: Value = serde_json::from_str(&text)
        .context("GeoJSON parse error")?;
    let mut out = Vec::new();
    collect(&root, &mut out);
    tracing::warn!(
        path = %path.display(),
        polylines = out.len(),
        "topo layer loaded"
    );
    Ok(out)
}

// ─── Recursive collectors ─────────────────────────────────────────────────────

fn collect(v: &Value, out: &mut Vec<GeoFeature>) {
    match v["type"].as_str() {
        Some("FeatureCollection") => {
            if let Some(arr) = v["features"].as_array() {
                for f in arr { collect(f, out); }
            }
        }
        Some("Feature") => collect(&v["geometry"], out),
        Some("GeometryCollection") => {
            if let Some(arr) = v["geometries"].as_array() {
                for g in arr { collect(g, out); }
            }
        }
        Some(geom_type) => push_geometry(geom_type, &v["coordinates"], out),
        None => {}
    }
}

fn push_geometry(typ: &str, coords: &Value, out: &mut Vec<GeoFeature>) {
    match typ {
        "LineString" => {
            if let Some(pts) = positions(coords) {
                if pts.len() >= 2 { out.push(line_feature(pts)); }
            }
        }
        "MultiLineString" => {
            for line in coords.as_array().into_iter().flatten() {
                if let Some(pts) = positions(line) {
                    if pts.len() >= 2 { out.push(line_feature(pts)); }
                }
            }
        }
        "Polygon" => {
            for ring in coords.as_array().into_iter().flatten() {
                if let Some(pts) = positions(ring) {
                    if pts.len() >= 2 { out.push(line_feature(pts)); }
                }
            }
        }
        "MultiPolygon" => {
            for poly in coords.as_array().into_iter().flatten() {
                for ring in poly.as_array().into_iter().flatten() {
                    if let Some(pts) = positions(ring) {
                        if pts.len() >= 2 { out.push(line_feature(pts)); }
                    }
                }
            }
        }
        _ => {} // Point, MultiPoint — not renderable as lines
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a GeoJSON position array `[[lon,lat],…]` into `[[lat,lon],…]`.
fn positions(arr: &Value) -> Option<Vec<[f32; 2]>> {
    let pts = arr.as_array()?;
    pts.iter().map(|pt| {
        let a = pt.as_array()?;
        let lon = a.first()?.as_f64()? as f32;
        let lat = a.get(1)?.as_f64()? as f32;
        Some([lat, lon])
    }).collect()
}

fn line_feature(coordinates: Vec<[f32; 2]>) -> GeoFeature {
    let (lat, lon) = coordinates.first().map(|&[la, lo]| (la, lo)).unwrap_or_default();
    GeoFeature {
        lat, lon, alt: 0.0,
        kind: FeatureKind::Line,
        meta: serde_json::Value::Null,
        coordinates,
    }
}
