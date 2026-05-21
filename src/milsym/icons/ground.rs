use egui::{Color32, Painter, Pos2, Rect, Stroke};

use crate::milsym::GroundFunction;

pub fn draw(painter: &Painter, rect: Rect, function: &GroundFunction, color: Color32) {
    let c  = rect.center();
    let hw = rect.width()  * 0.42;
    let hh = rect.height() * 0.42;
    let s  = Stroke::new(1.4, color);

    match function {
        GroundFunction::Infantry => {
            // Vertical line (soldier) with two short arms
            painter.line_segment(
                [Pos2::new(c.x, c.y - hh * 0.8), Pos2::new(c.x, c.y + hh * 0.8)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.5, c.y - hh * 0.1), Pos2::new(c.x + hw * 0.5, c.y - hh * 0.1)],
                s,
            );
        }
        GroundFunction::MechanizedInfantry => {
            // Infantry vertical line + rectangle base (APC/IFV)
            painter.line_segment(
                [Pos2::new(c.x, c.y - hh * 0.5), Pos2::new(c.x, c.y)],
                s,
            );
            let base = egui::Rect::from_center_size(
                Pos2::new(c.x, c.y + hh * 0.4),
                egui::Vec2::new(hw * 1.4, hh * 0.7),
            );
            painter.rect_stroke(base, 0.0, s);
        }
        GroundFunction::Armor => {
            // Oval hull + small turret circle
            let hull = egui::Rect::from_center_size(c, egui::Vec2::new(hw * 1.6, hh * 0.9));
            painter.rect_stroke(hull, hh * 0.4, s);
            painter.circle_stroke(c, hh * 0.3, s);
        }
        GroundFunction::Artillery => {
            // Wheel circle + gun barrel line
            painter.circle_stroke(c, hh * 0.4, s);
            painter.line_segment(
                [Pos2::new(c.x, c.y), Pos2::new(c.x + hw * 0.8, c.y - hh * 0.5)],
                s,
            );
        }
        GroundFunction::AntiAircraft => {
            // Upward-pointing arrows / V-shape
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.5, c.y + hh * 0.5), Pos2::new(c.x, c.y - hh * 0.5)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x + hw * 0.5, c.y + hh * 0.5), Pos2::new(c.x, c.y - hh * 0.5)],
                s,
            );
        }
        GroundFunction::Engineer => {
            // "E" shape: vertical + three horizontals
            let x0 = c.x - hw * 0.4;
            painter.line_segment([Pos2::new(x0, c.y - hh * 0.7), Pos2::new(x0, c.y + hh * 0.7)], s);
            for frac in [-0.65, 0.0, 0.65_f32] {
                let len = if frac == 0.0 { hw * 0.6 } else { hw * 0.8 };
                painter.line_segment(
                    [Pos2::new(x0, c.y + hh * frac), Pos2::new(x0 + len, c.y + hh * frac)],
                    s,
                );
            }
        }
        GroundFunction::HQ => {
            // "HQ" abbreviation as cross-line
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.6, c.y - hh * 0.5), Pos2::new(c.x - hw * 0.6, c.y + hh * 0.5)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x + hw * 0.6, c.y - hh * 0.5), Pos2::new(c.x + hw * 0.6, c.y + hh * 0.5)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.6, c.y), Pos2::new(c.x + hw * 0.6, c.y)],
                s,
            );
        }
        GroundFunction::Reconnaissance => {
            // Eye shape (oval) — observation unit
            painter.circle_stroke(c, hh * 0.45, s);
            painter.circle_stroke(c, hh * 0.15, Stroke::new(1.4, color));
        }
        GroundFunction::SpecialForces => {
            // Lightning bolt (zigzag)
            let pts = [
                Pos2::new(c.x + hw * 0.2, c.y - hh * 0.8),
                Pos2::new(c.x - hw * 0.1, c.y - hh * 0.05),
                Pos2::new(c.x + hw * 0.2, c.y + hh * 0.05),
                Pos2::new(c.x - hw * 0.2, c.y + hh * 0.8),
            ];
            for pair in pts.windows(2) {
                painter.line_segment([pair[0], pair[1]], s);
            }
        }
        GroundFunction::Generic => {
            // Simple horizontal bar
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.7, c.y), Pos2::new(c.x + hw * 0.7, c.y)],
                s,
            );
        }
    }
}
