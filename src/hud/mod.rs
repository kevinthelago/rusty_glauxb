use crate::spectrum::SpectrumOverlay;

/// Projected screen-space data for the location dot drawn each frame.
pub struct HudLocation {
    pub screen_pos: egui::Pos2,
    /// True heading in degrees (0 = north).  NaN → no arrow drawn.
    pub hdg:        f32,
}

pub struct Hud {
    voice_status: Option<String>,
    diag_line:    Option<String>,
    pub spectrum: SpectrumOverlay,
    /// Set by `app.rs` each frame from `WgpuState::project_geo`.
    pub location: Option<HudLocation>,
}

impl Hud {
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        Self {
            voice_status: None,
            diag_line:    None,
            spectrum:     SpectrumOverlay::new(rt),
            location:     None,
        }
    }

    pub fn set_voice_status(&mut self, text: String) {
        self.voice_status = Some(text);
    }

    /// Set the compact diagnostics overlay line (fps / gpu / cpu).
    /// Pass an empty string to clear it.
    pub fn set_diag_line(&mut self, line: String) {
        self.diag_line = if line.is_empty() { None } else { Some(line) };
    }

    /// Called inside the egui render closure each frame.
    pub fn draw(&mut self, ctx: &egui::Context) {
        // ── Top-right: voice status toast ──────────────────────────────────
        if let Some(status) = &self.voice_status {
            egui::Window::new("Voice")
                .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
                .title_bar(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(status).color(egui::Color32::LIGHT_GREEN));
                });
        }

        // ── Top-left: compact diag overlay ─────────────────────────────────
        if let Some(line) = &self.diag_line {
            egui::Area::new(egui::Id::new("diag_overlay"))
                .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(line)
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::from_rgba_unmultiplied(180, 220, 180, 200)),
                    );
                });
        }

        // ── Location dot (screen-space projection) ──────────────────────────
        if let Some(loc) = &self.location {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("location_dot"),
            ));
            let pos  = loc.screen_pos;
            let r    = 8.0_f32;
            let blue = egui::Color32::from_rgb(30, 120, 255);
            let rim  = egui::Color32::WHITE;

            // Accuracy ring (fixed visual size — TODO: scale by acc + zoom)
            painter.circle_stroke(
                pos,
                r * 2.8,
                egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(30, 120, 255, 80)),
            );
            // Filled dot with white rim
            painter.circle_filled(pos, r + 2.0, rim);
            painter.circle_filled(pos, r, blue);

            // Heading arrow (only when hdg is a valid number)
            if loc.hdg.is_finite() {
                let angle = loc.hdg.to_radians() - std::f32::consts::FRAC_PI_2;
                let tip = egui::Pos2::new(
                    pos.x + angle.cos() * (r + 7.0),
                    pos.y + angle.sin() * (r + 7.0),
                );
                painter.arrow(
                    pos,
                    tip - pos,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                );
            }
        }

        // ── Bottom-center: spectrum overlay ─────────────────────────────────
        self.spectrum.draw(ctx);
    }
}
