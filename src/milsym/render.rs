use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};

use super::{Affiliation, MilSymbol};
use super::symbol_map::{SymbolMap, function_key};

const FRAME_STROKE_W: f32 = 1.5;

/// Draw a MIL-STD-2525D symbol centered at `center`.
///
/// `base_size` is the half-extent in pixels (e.g. 10.0 at scale 1.0).
/// If `symbol_map` has an SVG assigned for this symbol's function, it is rendered
/// as vector paths inside the frame; otherwise the hand-drawn icon is used.
pub fn draw_symbol(
    painter:      &Painter,
    center:       Pos2,
    base_size:    f32,
    symbol:       &MilSymbol,
    hovered:      bool,
    show_echelon: bool,
    symbol_map:   &SymbolMap,
) {
    let stroke_w    = if hovered { FRAME_STROKE_W + 0.8 } else { FRAME_STROKE_W };
    let frame_color = symbol.affiliation.frame_color();
    let fill_color  = symbol.affiliation.fill_color();
    let frame_stroke = Stroke::new(stroke_w, frame_color);

    let inner_rect = draw_frame(
        painter, center, base_size, &symbol.affiliation, frame_stroke, fill_color,
    );

    let key = function_key(&symbol.function);
    if let Some(svg_bytes) = symbol_map.get_svg(key) {
        super::svg_render::draw_svg_icon(painter, svg_bytes, inner_rect, frame_color);
    } else {
        crate::milsym::icons::draw_icon(painter, inner_rect, &symbol.function, frame_color);
    }

    if show_echelon {
        if let Some(text) = symbol.echelon.indicator() {
            painter.text(
                Pos2::new(center.x, center.y - base_size * 1.55),
                egui::Align2::CENTER_BOTTOM,
                text,
                egui::FontId::monospace((base_size * 0.75).max(7.0)),
                frame_color,
            );
        }
    }

    if let Some(des) = &symbol.designator {
        painter.text(
            Pos2::new(center.x, center.y + base_size * 1.8),
            egui::Align2::CENTER_TOP,
            des,
            egui::FontId::monospace((base_size * 0.7).max(6.0)),
            Color32::from_rgba_unmultiplied(220, 220, 220, 200),
        );
    }
}

/// Returns the inner rect available for the icon.
fn draw_frame(
    painter:    &Painter,
    center:     Pos2,
    base_size:  f32,
    affiliation: &Affiliation,
    stroke:     Stroke,
    fill:       Color32,
) -> Rect {
    match affiliation {
        Affiliation::Friend => {
            // Horizontal rectangle (wider than tall)
            let hw = base_size * 1.45;
            let hh = base_size * 1.05;
            let rect = Rect::from_center_size(center, Vec2::new(hw * 2.0, hh * 2.0));
            painter.rect(rect, 0.0, fill, stroke);
            Rect::from_center_size(center, Vec2::new(hw * 1.5, hh * 1.5))
        }
        Affiliation::Hostile => {
            // Diamond (4-pointed star rotated 45°)
            let r = base_size * 1.6;
            let pts = vec![
                Pos2::new(center.x,     center.y - r),
                Pos2::new(center.x + r, center.y),
                Pos2::new(center.x,     center.y + r),
                Pos2::new(center.x - r, center.y),
            ];
            painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
            let inner = r * 0.6;
            Rect::from_center_size(center, Vec2::splat(inner * 2.0))
        }
        Affiliation::Neutral => {
            // Axis-aligned square
            let h = base_size * 1.3;
            let rect = Rect::from_center_size(center, Vec2::splat(h * 2.0));
            painter.rect(rect, 0.0, fill, stroke);
            Rect::from_center_size(center, Vec2::splat(h * 1.5))
        }
        Affiliation::Unknown => {
            // Circle
            let r = base_size * 1.4;
            painter.circle(center, r, fill, stroke);
            Rect::from_center_size(center, Vec2::splat(r * 1.4))
        }
    }
}

/// The bounding radius used for hover hit-testing.
pub fn hit_radius(base_size: f32) -> f32 {
    base_size * 1.8
}
