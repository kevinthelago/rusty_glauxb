use egui::{Color32, Painter, Pos2, Rect, Stroke};

use crate::milsym::AirFunction;

pub fn draw(painter: &Painter, rect: Rect, function: &AirFunction, color: Color32) {
    let c  = rect.center();
    let hw = rect.width()  * 0.42;
    let hh = rect.height() * 0.42;
    let s  = Stroke::new(1.4, color);

    match function {
        AirFunction::FixedWing | AirFunction::Transport | AirFunction::Tanker => {
            fixed_wing(painter, c, hw, hh, s);
        }
        AirFunction::Rotary | AirFunction::MilitaryRotary => {
            rotary_wing(painter, c, hw, hh, s);
        }
        AirFunction::MilitaryFixedWing | AirFunction::Fighter | AirFunction::Bomber => {
            fixed_wing(painter, c, hw, hh, s);
            // Additional vertical fin stub to distinguish military
            painter.line_segment(
                [Pos2::new(c.x + hw * 0.7, c.y), Pos2::new(c.x + hw * 0.9, c.y - hh * 0.5)],
                s,
            );
        }
        AirFunction::Unmanned => {
            // Circle (drone body) + crosshair spars
            painter.circle_stroke(c, hh * 0.35, s);
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.8, c.y), Pos2::new(c.x + hw * 0.8, c.y)],
                s,
            );
        }
        AirFunction::Generic => {
            // Single horizontal bar
            painter.line_segment(
                [Pos2::new(c.x - hw * 0.7, c.y), Pos2::new(c.x + hw * 0.7, c.y)],
                s,
            );
        }
    }
}

fn fixed_wing(painter: &Painter, c: Pos2, hw: f32, hh: f32, s: Stroke) {
    // Fuselage
    painter.line_segment(
        [Pos2::new(c.x - hw * 0.85, c.y), Pos2::new(c.x + hw * 0.85, c.y)],
        s,
    );
    // Swept wings angled back from center
    painter.line_segment([c, Pos2::new(c.x - hw * 0.65, c.y + hh * 0.55)], s);
    painter.line_segment([c, Pos2::new(c.x + hw * 0.65, c.y + hh * 0.55)], s);
}

fn rotary_wing(painter: &Painter, c: Pos2, hw: f32, hh: f32, s: Stroke) {
    // Rotor disc outline
    painter.circle_stroke(c, hh * 0.3, s);
    // Main rotor blades (horizontal)
    painter.line_segment(
        [Pos2::new(c.x - hw * 0.75, c.y), Pos2::new(c.x + hw * 0.75, c.y)],
        s,
    );
    // Tail boom stub
    painter.line_segment(
        [Pos2::new(c.x + hh * 0.3, c.y), Pos2::new(c.x + hw * 0.75, c.y - hh * 0.25)],
        s,
    );
}
