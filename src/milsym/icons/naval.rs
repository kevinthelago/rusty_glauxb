use egui::{Color32, Painter, Pos2, Rect, Stroke};

use crate::milsym::NavalFunction;

pub fn draw(painter: &Painter, rect: Rect, function: &NavalFunction, color: Color32) {
    let c  = rect.center();
    let hw = rect.width()  * 0.42;
    let hh = rect.height() * 0.42;
    let s  = Stroke::new(1.4, color);

    match function {
        NavalFunction::Surface => {
            // Hull line with small superstructure box
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.8, c.y + hh * 0.3), Pos2::new(c.x + hw * 0.8, c.y + hh * 0.3)],
                s,
            );
            let sup = egui::Rect::from_center_size(
                Pos2::new(c.x, c.y),
                egui::Vec2::new(hw * 0.8, hh * 0.55),
            );
            painter.rect_stroke(sup, 0.0, s);
        }
        NavalFunction::Submarine => {
            // Elongated oval (pressure hull)
            let hull = egui::Rect::from_center_size(c, egui::Vec2::new(hw * 1.7, hh * 0.75));
            painter.rect_stroke(hull, hh * 0.35, s);
            // Sail fin
            painter.line_segment(
                [Pos2::new(c.x, c.y - hh * 0.35), Pos2::new(c.x, c.y - hh * 0.75)],
                s,
            );
        }
        NavalFunction::Carrier => {
            // Wide flat hull + angled flight deck line
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.9, c.y + hh * 0.2), Pos2::new(c.x + hw * 0.9, c.y + hh * 0.2)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.8, c.y - hh * 0.2), Pos2::new(c.x + hw * 0.8, c.y - hh * 0.2)],
                s,
            );
            // Island superstructure
            painter.line_segment(
                [Pos2::new(c.x + hw * 0.5, c.y - hh * 0.2), Pos2::new(c.x + hw * 0.5, c.y - hh * 0.65)],
                s,
            );
        }
        NavalFunction::Generic => {
            // Simple hull line
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.8, c.y), Pos2::new(c.x + hw * 0.8, c.y)],
                s,
            );
        }
    }
}
