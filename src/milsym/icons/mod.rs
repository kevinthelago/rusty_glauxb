pub mod air;
pub mod ground;
pub mod naval;

use egui::{Color32, Painter, Rect};

use crate::milsym::FunctionId;

/// Dispatch to the appropriate dimension's icon drawing function.
pub fn draw_icon(painter: &Painter, inner_rect: Rect, function: &FunctionId, color: Color32) {
    match function {
        FunctionId::Air(f)    => air::draw(painter, inner_rect, f, color),
        FunctionId::Ground(f) => ground::draw(painter, inner_rect, f, color),
        FunctionId::Naval(f)  => naval::draw(painter, inner_rect, f, color),
    }
}
