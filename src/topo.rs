//! GeoJSON → line-segment loader for the topology overlay layer.
//!
//! Handles the full GeoJSON geometry type set:
//!   Point / MultiPoint          — rendered as a ×-marker; properties preserved on the H arm
//!   LineString / MultiLineString — rendered directly
//!   Polygon / MultiPolygon       — outer + hole rings rendered as outlines
//!   GeometryCollection           — recurse
//!   Feature / FeatureCollection  — unwrap; Feature properties are threaded through to
//!                                   every geometry produced by that feature

use std::path::Path;

use anyhow::Context as _;
use serde_json::Value;

use crate::geo::{FeatureKind, GeoFeature};

/// Load a GeoJSON file from `path` and return all geometry as polylines.
/// Each ring/line becomes one `GeoFeature` with `kind = Line`.
pub fn load_geojson(path: &Path) -> anyhow::Result<Vec<GeoFeature>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let features = parse_geojson_text(&text)?;
    tracing::warn!(
        path = %path.display(),
        polylines = features.len(),
        "topo layer loaded"
    );
    Ok(features)
}

/// Parse raw GeoJSON text and return all geometry as polylines.
/// Shared by the file-based topo loader and the Natural Earth HTTP loader.
pub(crate) fn parse_geojson_text(text: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let root: Value = serde_json::from_str(text).context("GeoJSON parse error")?;
    let mut out = Vec::new();
    collect(&root, &mut out);
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
        Some("Feature") => {
            // Capture the Feature-level properties and thread them into every
            // geometry that this feature produces.
            let props = v["properties"].clone();
            if !v["geometry"].is_null() {
                push_geometry(&v["geometry"], props, out);
            }
        }
        Some("GeometryCollection") => {
            if let Some(arr) = v["geometries"].as_array() {
                for g in arr { push_geometry(g, Value::Null, out); }
            }
        }
        // Bare geometry object (no Feature wrapper).
        Some(_) => push_geometry(v, Value::Null, out),
        None => {}
    }
}

fn push_geometry(geom: &Value, props: Value, out: &mut Vec<GeoFeature>) {
    let typ = match geom["type"].as_str() { Some(t) => t, None => return };
    let coords = &geom["coordinates"];

    match typ {
        "LineString" => {
            if let Some(pts) = positions(coords) {
                if pts.len() >= 2 { out.push(line_feature(pts, props)); }
            }
        }
        "MultiLineString" => {
            for line in coords.as_array().into_iter().flatten() {
                if let Some(pts) = positions(line) {
                    if pts.len() >= 2 { out.push(line_feature(pts, props.clone())); }
                }
            }
        }
        "Polygon" => {
            for ring in coords.as_array().into_iter().flatten() {
                if let Some(pts) = positions(ring) {
                    if pts.len() >= 2 { out.push(line_feature(pts, props.clone())); }
                }
            }
        }
        "MultiPolygon" => {
            for poly in coords.as_array().into_iter().flatten() {
                for ring in poly.as_array().into_iter().flatten() {
                    if let Some(pts) = positions(ring) {
                        if pts.len() >= 2 { out.push(line_feature(pts, props.clone())); }
                    }
                }
            }
        }
        "Point" => {
            if let Some([lat, lon]) = point_pos(coords) {
                push_point_marker(lat, lon, props, out);
            }
        }
        "MultiPoint" => {
            for p in coords.as_array().into_iter().flatten() {
                if let Some([lat, lon]) = point_pos(p) {
                    push_point_marker(lat, lon, props.clone(), out);
                }
            }
        }
        "GeometryCollection" => {
            if let Some(arr) = geom["geometries"].as_array() {
                for g in arr { push_geometry(g, props.clone(), out); }
            }
        }
        _ => {}
    }
}

/// Emit a ×-marker for a point feature.
///
/// The horizontal arm carries the feature properties (used for hover tooltips).
/// The vertical arm carries `Null` so hover detection finds exactly one entry
/// per point, not two.  Both arms store the true centre as `(lat, lon)` so
/// projection always finds the right screen position.
fn push_point_marker(lat: f32, lon: f32, props: Value, out: &mut Vec<GeoFeature>) {
    let d = 0.3_f32;
    out.push(GeoFeature {
        lat, lon, alt: 0.0,
        kind:        FeatureKind::Line,
        meta:        props,
        coordinates: vec![[lat - d, lon], [lat + d, lon]],
    });
    out.push(GeoFeature {
        lat, lon, alt: 0.0,
        kind:        FeatureKind::Line,
        meta:        Value::Null,
        coordinates: vec![[lat, lon - d], [lat, lon + d]],
    });
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a GeoJSON `[lon, lat]` position into `[lat, lon]`.
fn point_pos(v: &Value) -> Option<[f32; 2]> {
    let a = v.as_array()?;
    let lon = a.first()?.as_f64()? as f32;
    let lat = a.get(1)?.as_f64()? as f32;
    Some([lat, lon])
}

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

fn line_feature(coordinates: Vec<[f32; 2]>, props: Value) -> GeoFeature {
    let (lat, lon) = coordinates.first().map(|&[la, lo]| (la, lo)).unwrap_or_default();
    GeoFeature {
        lat, lon, alt: 0.0,
        kind:        FeatureKind::Line,
        meta:        props,
        coordinates,
    }
}
