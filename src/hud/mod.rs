// ─── Follow overlay types ─────────────────────────────────────────────────────

/// Data fields that can appear in the follow-mode HUD overlay.
#[derive(Clone, Debug, PartialEq)]
pub enum FollowOverlayField {
    Callsign,
    Icao24,
    AircraftType,
    Category,
    Altitude,
    Speed,
    Heading,
    Position,
    OnGround,
}

impl FollowOverlayField {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "callsign"            => Some(Self::Callsign),
            "icao24"              => Some(Self::Icao24),
            "type" | "aircraft_type" => Some(Self::AircraftType),
            "category"            => Some(Self::Category),
            "altitude" | "alt"    => Some(Self::Altitude),
            "speed" | "spd"       => Some(Self::Speed),
            "heading" | "hdg"     => Some(Self::Heading),
            "position" | "pos"    => Some(Self::Position),
            "on_ground" | "ground" => Some(Self::OnGround),
            _                     => None,
        }
    }
}

/// Parse a comma- or space-separated string of field names into overlay fields.
/// Unknown tokens are silently ignored.
pub fn parse_overlay_fields(s: &str) -> Vec<FollowOverlayField> {
    s.split(|c: char| c == ',' || c == ' ')
        .filter_map(FollowOverlayField::from_str)
        .collect()
}

/// Live data passed to the follow overlay each frame.
pub enum FollowData {
    Aircraft {
        icao24:        String,
        callsign:      Option<String>,
        aircraft_type: Option<String>,
        category:      String,
        altitude_ft:   Option<f32>,
        speed_kts:     Option<f32>,
        heading:       Option<f32>,
        on_ground:     bool,
        lat:           f32,
        lon:           f32,
    },
    Location {
        lat:         f64,
        lon:         f64,
        alt_m:       f64,
        speed_mps:   f64,
        heading_deg: f64,
    },
}

// ─── Existing HUD types ───────────────────────────────────────────────────────

/// Projected screen-space data for the location dot drawn each frame.
pub struct HudLocation {
    pub screen_pos: egui::Pos2,
    /// True heading in degrees (0 = north).  NaN → no arrow drawn.
    pub hdg:        f32,
}

/// Which metadata field to display as a persistent label next to each aircraft dot.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum AircraftLabel {
    #[default]
    None,
    Callsign,
    Type,
    Altitude,
    Speed,
    Icao24,
}

impl AircraftLabel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None     => "none",
            Self::Callsign => "callsign",
            Self::Type     => "type",
            Self::Altitude => "altitude",
            Self::Speed    => "speed",
            Self::Icao24   => "icao24",
        }
    }
}

/// Projected screen-space data for one topo-layer feature, populated each frame.
///
/// Only features with non-null GeoJSON properties (or the H-arm of a point
/// marker) are included — the silent V-arm is filtered out in the renderer.
#[derive(Clone)]
pub struct HudGeoFeature {
    pub screen_pos:  egui::Pos2,
    /// Name of the `LineLayer` this feature belongs to (e.g. "mountain_peaks").
    pub layer_name:  String,
    /// Raw GeoJSON properties; may be `Value::Null` for bare-geometry features.
    pub properties:  serde_json::Value,
    pub lat:         f32,
    pub lon:         f32,
}

impl HudGeoFeature {
    /// Best human-readable name from properties, or `None`.
    pub fn display_name(&self) -> Option<&str> {
        for key in &["name", "Name", "NAME", "name_long", "featurecla"] {
            if let Some(s) = self.properties[key].as_str().filter(|s| !s.is_empty()) {
                return Some(s);
            }
        }
        None
    }

    /// Numeric elevation value from any common property key, if present.
    pub fn elevation(&self) -> Option<f64> {
        for key in &["elevation", "elev_m", "elev_ft", "ELEV", "Elev"] {
            if let Some(n) = self.properties[key].as_f64() {
                return Some(n);
            }
        }
        None
    }

    /// Stable identity string used to detect "same feature clicked twice".
    fn identity(&self) -> String {
        format!("{}|{:.4}|{:.4}", self.layer_name, self.lat, self.lon)
    }
}

/// Projected screen-space data for one aircraft dot, populated each frame by the renderer.
pub struct HudAircraft {
    pub screen_pos:    egui::Pos2,
    pub icao24:        String,
    pub callsign:      Option<String>,
    pub aircraft_type: Option<String>,
    pub altitude_ft:   Option<f32>,
    pub speed_kts:     Option<f32>,
    pub heading:       Option<f32>,
    pub on_ground:     bool,
    pub category:      String,
    /// Populated when milsym mode is enabled; `None` → draw as a colored dot.
    pub milsym:        Option<crate::milsym::MilSymbol>,
}

// ─── Touch / mobile UI sizing (ENG-7) ──────────────────────────────────────────
//
// These helpers make the egui HUD/console usable with a finger on mobile.  They
// depend ONLY on the screen rect egui already derives from app.rs's existing
// resize forwarding — not on any engine-core touch-input plumbing (ENG-4) — so
// they merge cleanly whenever that lands.  Tap-routing / soft-keyboard
// acceptance is deliberately out of scope here and waits on ENG-4.

/// Logical screen width (points) at/below which a finger-friendly layout is
/// used.  Phones report a small logical width in portrait regardless of DPI.
const TOUCH_WIDTH_THRESHOLD: f32 = 600.0;

/// Decide whether to use a touch layout from the logical screen width.
/// Android always counts as touch; otherwise a narrow/compact screen does.
/// Pure function so the threshold logic is unit-testable without an egui context.
pub fn is_touch_layout(screen_width_pts: f32) -> bool {
    cfg!(target_os = "android") || (screen_width_pts > 0.0 && screen_width_pts < TOUCH_WIDTH_THRESHOLD)
}

/// Finger-friendly UI metrics derived from the current egui context.
pub struct TouchMetrics {
    /// True when a finger-friendly layout should be used.
    pub touch: bool,
    /// Minimum interactive widget height (logical points) for comfortable taps.
    pub min_target: f32,
    /// Base body-text size (logical points).
    pub text_size: f32,
    /// Current logical screen width (points).
    pub screen_width: f32,
}

/// Compute [`TouchMetrics`] for the current frame.
pub fn touch_metrics(ctx: &egui::Context) -> TouchMetrics {
    let screen_width = ctx.screen_rect().width();
    let touch = is_touch_layout(screen_width);
    TouchMetrics {
        touch,
        min_target: if touch { 44.0 } else { 0.0 },
        text_size: if touch { 16.0 } else { 12.0 },
        screen_width,
    }
}

/// Apply finger-friendly spacing and hit-target sizes to `ui` in place.
/// No-op on non-touch layouts, so desktop rendering is unchanged.
pub fn apply_touch_spacing(ui: &mut egui::Ui, m: &TouchMetrics) {
    if !m.touch {
        return;
    }
    let s = ui.spacing_mut();
    s.interact_size.y = s.interact_size.y.max(m.min_target);
    s.button_padding = egui::vec2(12.0, 8.0);
    s.item_spacing = egui::vec2(8.0, 8.0);
    s.scroll.bar_width = 16.0;
}

pub struct Hud {
    voice_status:        Option<String>,
    diag_line:           Option<String>,
    /// Set by `app.rs` each frame from `WgpuState::project_geo`.
    pub location:        Option<HudLocation>,
    /// Set by `renderer/mod.rs` each frame from projected flight positions.
    pub aircraft:        Vec<HudAircraft>,
    /// Which metadata field to render as a persistent label next to each dot.
    pub label:           AircraftLabel,
    /// When true, clicking an aircraft dot follows it with the camera.
    pub follow_on_click: bool,
    /// Live data for the follow overlay; `None` when not in follow mode.
    pub follow_data:            Option<FollowData>,
    /// Whether to show the follow overlay at all.
    pub follow_overlay_visible: bool,
    /// Which fields to render in the follow overlay.
    pub follow_overlay_fields:  Vec<FollowOverlayField>,
    /// When true, render MIL-STD-2525D symbols instead of colored dots.
    pub milsym_enabled:      bool,
    /// Base half-size of milsym frames in pixels (scale multiplier × 10).
    pub milsym_base_size:    f32,
    /// Show echelon indicator above the frame.
    pub milsym_show_echelon: bool,
    /// SVG icon overrides per symbology standard.
    pub symbol_map:          crate::milsym::symbol_map::SymbolMap,
    /// Projected topo-layer features for hover/click, rebuilt each frame.
    pub geo_features: Vec<HudGeoFeature>,
    /// Per-pipeline fetch health for the health panel (PIPE-6).  Set each frame
    /// by `app.rs` from `PipelineManager::health()`; empty hides the panel.
    pub pipeline_health: Vec<crate::pipeline::PipelineHealth>,
    /// Whether to draw the pipeline-health panel when pipelines exist.
    pub show_pipeline_health: bool,
    /// Feature currently pinned open in the info panel (click to select, click again / close to dismiss).
    selected_geo: Option<HudGeoFeature>,
    // ── AI summary ────────────────────────────────────────────────────────────
    rt:              tokio::runtime::Handle,
    summary:         Option<String>,
    summary_loading: bool,
    /// Identity string of the feature the pending fetch belongs to.
    summary_feat_id: Option<String>,
    pending_summary: Option<tokio::sync::oneshot::Receiver<anyhow::Result<String>>>,
}

impl Hud {
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        Self {
            voice_status:    None,
            diag_line:       None,
            location:        None,
            aircraft:        Vec::new(),
            label:           AircraftLabel::None,
            follow_on_click: true,
            follow_data:            None,
            follow_overlay_visible: true,
            follow_overlay_fields:  parse_overlay_fields("callsign,type,altitude,speed,heading,position"),
            milsym_enabled:      false,
            milsym_base_size:    10.0,
            milsym_show_echelon: true,
            symbol_map:          crate::milsym::symbol_map::SymbolMap::default(),
            geo_features:        Vec::new(),
            pipeline_health:        Vec::new(),
            show_pipeline_health:   true,
            selected_geo:        None,
            rt,
            summary:         None,
            summary_loading: false,
            summary_feat_id: None,
            pending_summary: None,
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
    /// Returns the ICAO24 of an aircraft the user clicked, if any.
    pub fn draw(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut clicked_icao24: Option<String> = None;

        // ── Poll pending AI summary ─────────────────────────────────────────
        if let Some(rx) = &mut self.pending_summary {
            let done = match rx.try_recv() {
                Ok(Ok(text)) => { self.summary = Some(text); true }
                Ok(Err(e))   => { tracing::debug!("geo summary: {e:#}"); true }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => true,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)  => false,
            };
            if done {
                self.pending_summary = None;
                self.summary_loading = false;
                self.summary_feat_id = None;
            }
        }
        // Keep repainting while loading so the spinner animates.
        if self.summary_loading { ctx.request_repaint(); }

        // ── Pipeline health panel (PIPE-6) ─────────────────────────────────
        // Display-only: renders the snapshot app.rs sets from
        // PipelineManager::health(). Independent of ENG-4 (no input handling);
        // uses the ENG-7 touch-sizing helpers.
        if self.show_pipeline_health && !self.pipeline_health.is_empty() {
            let m = touch_metrics(ctx);
            let panel_w = if m.touch {
                (m.screen_width * 0.6).clamp(220.0, 360.0)
            } else {
                240.0
            };
            egui::Window::new("Pipelines")
                .id(egui::Id::new("pipeline_health_panel"))
                .anchor(egui::Align2::LEFT_TOP, [8.0, 40.0])
                .resizable(false)
                .collapsible(true)
                .default_width(panel_w)
                .show(ctx, |ui| {
                    apply_touch_spacing(ui, &m);
                    for h in &self.pipeline_health {
                        let (color, state) = match h.status {
                            crate::pipeline::PipelineStatus::Ok => {
                                (egui::Color32::from_rgb(64, 210, 64), format!("{} features", h.feature_count))
                            }
                            crate::pipeline::PipelineStatus::Pending => {
                                (egui::Color32::from_rgb(255, 200, 60), "fetching…".to_string())
                            }
                            crate::pipeline::PipelineStatus::Error => {
                                (egui::Color32::from_rgb(255, 80, 60), "error".to_string())
                            }
                        };
                        ui.horizontal(|ui| {
                            // Color-coded status square (widely-supported glyph).
                            ui.colored_label(color, "■");
                            ui.label(egui::RichText::new(&h.layer).size(m.text_size).strong());
                            ui.label(egui::RichText::new(state).size(m.text_size - 1.0).weak());
                        });
                        if h.status == crate::pipeline::PipelineStatus::Error {
                            if let Some(err) = &h.last_error {
                                ui.label(
                                    egui::RichText::new(err)
                                        .size(m.text_size - 2.0)
                                        .color(egui::Color32::from_rgb(255, 120, 100)),
                                );
                            }
                        }
                        if let Some(age) = h.last_update.map(|t| t.elapsed().as_secs()) {
                            ui.label(
                                egui::RichText::new(format!("updated {age}s ago"))
                                    .size(m.text_size - 2.0)
                                    .weak(),
                            );
                        }
                    }
                });
        }

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

        // ── Geo-feature hover / click ────────────────────────────────────────
        // Resolved first so aircraft hover only fires when no geo feature is hit.
        let hover_pos     = ctx.input(|i| i.pointer.hover_pos());
        let egui_wants    = ctx.wants_pointer_input();
        let primary_click = !egui_wants
            && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
        let screen_rect   = ctx.screen_rect();

        let mut geo_hovered: Option<&HudGeoFeature> = None;
        let mut geo_consumed_click = false;

        if !self.geo_features.is_empty() {
            const GEO_HOVER_R: f32 = 18.0;
            for feat in &self.geo_features {
                if !screen_rect.contains(feat.screen_pos) { continue; }
                if !egui_wants
                    && hover_pos.map_or(false, |p| (p - feat.screen_pos).length() < GEO_HOVER_R)
                {
                    geo_hovered = Some(feat);
                    if primary_click {
                        geo_consumed_click = true;
                        let id = feat.identity();
                        let same = self.selected_geo.as_ref()
                            .map_or(false, |s| s.identity() == id);
                        if same {
                            self.selected_geo    = None;
                            self.summary         = None;
                            self.summary_loading = false;
                            self.pending_summary = None;
                            self.summary_feat_id = None;
                        } else {
                            let new_feat = feat.clone();
                            self.summary         = None;
                            self.summary_loading = false;
                            self.pending_summary = None;
                            self.summary_feat_id = None;
                            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                let feat_for_task = new_feat.clone();
                                self.rt.spawn(async move {
                                    let _ = tx.send(crate::geo_summary::fetch(&feat_for_task).await);
                                });
                                self.summary_feat_id = Some(id);
                                self.summary_loading = true;
                                self.pending_summary = Some(rx);
                            }
                            self.selected_geo = Some(new_feat);
                        }
                    }
                    break;
                }
            }
        }

        // Hover ring + tooltip for geo features.
        if let Some(feat) = geo_hovered {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("geo_hover_layer"),
            ));
            painter.circle_stroke(
                feat.screen_pos,
                10.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 220, 100, 200)),
            );

            // Brief tooltip near the cursor.
            let tip_pos = egui::pos2(
                (feat.screen_pos.x + 14.0).min(screen_rect.max.x - 200.0),
                (feat.screen_pos.y - 6.0).clamp(screen_rect.min.y + 4.0, screen_rect.max.y - 90.0),
            );
            egui::Area::new(egui::Id::new("geo_tooltip"))
                .fixed_pos(tip_pos)
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgba_unmultiplied(14, 20, 34, 230))
                        .rounding(5.0)
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(180, 150, 60, 160),
                        ))
                        .inner_margin(egui::Margin::same(7.0))
                        .show(ui, |ui| {
                            // Layer badge
                            ui.label(
                                egui::RichText::new(&feat.layer_name)
                                    .monospace()
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(180, 150, 60)),
                            );
                            // Feature name (if any)
                            if let Some(name) = feat.display_name() {
                                ui.label(
                                    egui::RichText::new(name)
                                        .monospace()
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                );
                            }
                            // Elevation (if any)
                            if let Some(elev) = feat.elevation() {
                                ui.label(
                                    egui::RichText::new(format!("{elev:.0} m"))
                                        .monospace()
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(160, 200, 255)),
                                );
                            }
                            // Click hint
                            ui.label(
                                egui::RichText::new("click for details")
                                    .monospace()
                                    .size(10.0)
                                    .color(egui::Color32::from_rgba_unmultiplied(120, 140, 120, 180)),
                            );
                        });
                });
        }

        // Persistent info panel for selected geo feature.
        if let Some(feat) = self.selected_geo.clone() {
            let title = feat.display_name()
                .map(|n| format!("{} — {n}", feat.layer_name))
                .unwrap_or_else(|| feat.layer_name.clone());

            let mut open = true;
            // Touch sizing (ENG-7): wider panel + finger-sized rows on mobile.
            let m = touch_metrics(ctx);
            let panel_w = if m.touch { (m.screen_width * 0.7).min(380.0).max(270.0) } else { 270.0 };
            egui::Window::new(title)
                .id(egui::Id::new("geo_info_panel"))
                .open(&mut open)
                .resizable(true)
                .default_width(panel_w)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-10.0, -10.0])
                .show(ctx, |ui| {
                    apply_touch_spacing(ui, &m);
                    egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                        // Properties from GeoJSON
                        if let Some(obj) = feat.properties.as_object() {
                            // Strings first, then numbers, skipping nulls/empty.
                            let mut str_rows: Vec<(&str, String)> = Vec::new();
                            let mut num_rows: Vec<(&str, String)> = Vec::new();
                            for (key, val) in obj {
                                match val {
                                    serde_json::Value::String(s) if !s.is_empty() => {
                                        str_rows.push((key.as_str(), s.clone()));
                                    }
                                    serde_json::Value::Number(n) => {
                                        num_rows.push((key.as_str(), n.to_string()));
                                    }
                                    serde_json::Value::Bool(b) => {
                                        str_rows.push((key.as_str(), b.to_string()));
                                    }
                                    _ => {}
                                }
                            }
                            for (k, v) in &str_rows { overlay_row(ui, k, v); }
                            if !str_rows.is_empty() && !num_rows.is_empty() {
                                ui.add(egui::Separator::default().spacing(4.0));
                            }
                            for (k, v) in &num_rows { overlay_row(ui, k, v); }
                        }
                        // Always show geographic coordinates.
                        ui.add(egui::Separator::default().spacing(4.0));
                        overlay_row(ui, "lat", &fmt_lat(feat.lat));
                        overlay_row(ui, "lon", &fmt_lon(feat.lon));
                        overlay_row(ui, "layer", &feat.layer_name);

                        // AI summary section.
                        ui.add(egui::Separator::default().spacing(6.0));
                        if self.summary_loading {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new());
                                ui.label(
                                    egui::RichText::new("Generating summary…")
                                        .monospace()
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(140, 170, 220)),
                                );
                            });
                        } else if let Some(ref text) = self.summary {
                            ui.label(
                                egui::RichText::new(text)
                                    .size(11.5)
                                    .color(egui::Color32::from_rgb(215, 232, 255)),
                            );
                        }
                    });
                });
            if !open {
                self.selected_geo    = None;
                self.summary         = None;
                self.summary_loading = false;
                self.pending_summary = None;
                self.summary_feat_id = None;
            }
        }

        // ── Aircraft dots ───────────────────────────────────────────────────
        if !self.aircraft.is_empty() {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("aircraft_layer"),
            ));

            // Suppress aircraft click when a geo feature consumed it this frame.
            let primary_click = primary_click && !geo_consumed_click;

            const DOT_R: f32 = 4.0;
            let hover_r = if self.milsym_enabled {
                self.milsym_base_size * 1.9
            } else {
                9.0
            };

            let mut hovered_idx: Option<usize> = None;

            for (i, ac) in self.aircraft.iter().enumerate() {
                let pos = ac.screen_pos;
                if !screen_rect.contains(pos) {
                    continue;
                }

                let is_hovered = !egui_wants
                    && hover_pos.map_or(false, |p| (p - pos).length() < hover_r);
                if is_hovered {
                    hovered_idx = Some(i);
                    if primary_click && self.follow_on_click {
                        clicked_icao24 = Some(ac.icao24.clone());
                    }
                }

                if let Some(ref sym) = ac.milsym {
                    // MIL-STD-2525D symbol
                    crate::milsym::render::draw_symbol(
                        &painter,
                        pos,
                        self.milsym_base_size,
                        sym,
                        is_hovered,
                        self.milsym_show_echelon,
                        &self.symbol_map,
                    );
                } else {
                    // Standard colored dot
                    if is_hovered {
                        painter.circle_stroke(
                            pos,
                            DOT_R + 3.5,
                            egui::Stroke::new(
                                1.5,
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                            ),
                        );
                    }
                    painter.circle_filled(pos, DOT_R + 1.0, egui::Color32::WHITE);
                    painter.circle_filled(pos, DOT_R, aircraft_color(ac));
                }

                // Persistent label (if any)
                if let Some(txt) = self.label_text(ac) {
                    let label_x = if ac.milsym.is_some() {
                        pos.x + self.milsym_base_size * 1.6
                    } else {
                        pos.x + DOT_R + 4.0
                    };
                    painter.text(
                        egui::pos2(label_x, pos.y),
                        egui::Align2::LEFT_CENTER,
                        txt,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_rgba_unmultiplied(220, 220, 220, 200),
                    );
                }
            }

            // Hover tooltip
            if let Some(idx) = hovered_idx {
                let ac = &self.aircraft[idx];
                let tip_pos = egui::pos2(
                    (ac.screen_pos.x + 14.0).min(screen_rect.max.x - 185.0),
                    (ac.screen_pos.y - 8.0)
                        .clamp(screen_rect.min.y + 4.0, screen_rect.max.y - 130.0),
                );
                egui::Area::new(egui::Id::new("aircraft_tooltip"))
                    .fixed_pos(tip_pos)
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgba_unmultiplied(12, 18, 30, 228))
                            .rounding(5.0)
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_unmultiplied(90, 130, 200, 160),
                            ))
                            .inner_margin(egui::Margin::same(8.0))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(&ac.icao24)
                                        .monospace()
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                );
                                if let Some(cs) = &ac.callsign {
                                    ui.label(
                                        egui::RichText::new(format!("callsign  {cs}"))
                                            .monospace()
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(160, 200, 255)),
                                    );
                                }
                                if let Some(t) = &ac.aircraft_type {
                                    ui.label(
                                        egui::RichText::new(format!("type      {t}"))
                                            .monospace()
                                            .size(11.0),
                                    );
                                }
                                if ac.on_ground {
                                    ui.label(
                                        egui::RichText::new("on ground")
                                            .monospace()
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(160, 160, 160)),
                                    );
                                } else {
                                    if let Some(alt) = ac.altitude_ft {
                                        ui.label(
                                            egui::RichText::new(format!("altitude  {alt:.0} ft"))
                                                .monospace()
                                                .size(11.0),
                                        );
                                    }
                                    if let Some(spd) = ac.speed_kts {
                                        ui.label(
                                            egui::RichText::new(format!("speed     {spd:.0} kts"))
                                                .monospace()
                                                .size(11.0),
                                        );
                                    }
                                    if let Some(hdg) = ac.heading {
                                        ui.label(
                                            egui::RichText::new(format!("heading   {hdg:.0}°"))
                                                .monospace()
                                                .size(11.0),
                                        );
                                    }
                                }
                                if self.follow_on_click {
                                    ui.add_space(3.0);
                                    ui.label(
                                        egui::RichText::new("click to follow")
                                            .monospace()
                                            .size(10.0)
                                            .color(egui::Color32::from_rgba_unmultiplied(
                                                130, 170, 130, 180,
                                            )),
                                    );
                                }
                            });
                    });
            }
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

        // ── Bottom-left: follow overlay ─────────────────────────────────────
        if self.follow_overlay_visible {
            if let Some(data) = &self.follow_data {
                egui::Area::new(egui::Id::new("follow_overlay"))
                    .anchor(egui::Align2::LEFT_BOTTOM, [10.0, -10.0])
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgba_unmultiplied(8, 14, 26, 225))
                            .rounding(7.0)
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_unmultiplied(60, 100, 180, 160),
                            ))
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.set_min_width(190.0);
                                draw_follow_overlay(ui, data, &self.follow_overlay_fields);
                            });
                    });
            }
        }

        clicked_icao24
    }

    fn label_text(&self, ac: &HudAircraft) -> Option<String> {
        match &self.label {
            AircraftLabel::None     => None,
            AircraftLabel::Callsign => ac.callsign.clone(),
            AircraftLabel::Type     => ac.aircraft_type.clone(),
            AircraftLabel::Altitude => ac.altitude_ft.map(|a| format!("{a:.0}ft")),
            AircraftLabel::Speed    => ac.speed_kts.map(|s| format!("{s:.0}kt")),
            AircraftLabel::Icao24   => Some(ac.icao24.clone()),
        }
    }
}

// ─── Follow overlay drawing ───────────────────────────────────────────────────

fn draw_follow_overlay(ui: &mut egui::Ui, data: &FollowData, fields: &[FollowOverlayField]) {
    match data {
        FollowData::Aircraft {
            icao24, callsign, aircraft_type, category,
            altitude_ft, speed_kts, heading, on_ground, lat, lon,
        } => {
            // Header: callsign (bold) or ICAO24 if no callsign
            let header = callsign.as_deref().unwrap_or(icao24.as_str());
            ui.label(
                egui::RichText::new(header)
                    .monospace()
                    .strong()
                    .size(13.0)
                    .color(egui::Color32::WHITE),
            );
            ui.add(egui::Separator::default().spacing(4.0));

            for field in fields {
                match field {
                    FollowOverlayField::Callsign => {
                        if let Some(cs) = callsign {
                            overlay_row(ui, "callsign", cs);
                        }
                    }
                    FollowOverlayField::Icao24 => overlay_row(ui, "icao24", icao24),
                    FollowOverlayField::AircraftType => {
                        if let Some(t) = aircraft_type {
                            overlay_row(ui, "type", t);
                        }
                    }
                    FollowOverlayField::Category => overlay_row(ui, "category", category),
                    FollowOverlayField::Altitude => {
                        if *on_ground {
                            overlay_row(ui, "altitude", "on ground");
                        } else if let Some(alt) = altitude_ft {
                            overlay_row(ui, "altitude", &format!("{alt:.0} ft"));
                        }
                    }
                    FollowOverlayField::Speed => {
                        if let Some(spd) = speed_kts {
                            overlay_row(ui, "speed", &format!("{spd:.0} kts"));
                        }
                    }
                    FollowOverlayField::Heading => {
                        if let Some(hdg) = heading {
                            overlay_row(ui, "heading", &format!("{hdg:.0}°"));
                        }
                    }
                    FollowOverlayField::Position => {
                        let lat_s = fmt_lat(*lat);
                        let lon_s = fmt_lon(*lon);
                        overlay_row(ui, "lat", &lat_s);
                        overlay_row(ui, "lon", &lon_s);
                    }
                    FollowOverlayField::OnGround => {
                        overlay_row(ui, "ground", if *on_ground { "yes" } else { "no" });
                    }
                }
            }
        }

        FollowData::Location { lat, lon, alt_m, speed_mps, heading_deg } => {
            ui.label(
                egui::RichText::new("GPS")
                    .monospace()
                    .strong()
                    .size(13.0)
                    .color(egui::Color32::from_rgb(80, 160, 255)),
            );
            ui.add(egui::Separator::default().spacing(4.0));

            for field in fields {
                match field {
                    FollowOverlayField::Position => {
                        overlay_row(ui, "lat", &fmt_lat(*lat as f32));
                        overlay_row(ui, "lon", &fmt_lon(*lon as f32));
                    }
                    FollowOverlayField::Altitude => {
                        overlay_row(ui, "altitude", &format!("{alt_m:.0} m"));
                    }
                    FollowOverlayField::Speed => {
                        overlay_row(ui, "speed", &format!("{speed_mps:.1} m/s"));
                    }
                    FollowOverlayField::Heading => {
                        if heading_deg.is_finite() {
                            overlay_row(ui, "heading", &format!("{heading_deg:.0}°"));
                        }
                    }
                    _ => {} // aircraft-only fields silently skipped for location
                }
            }
        }
    }
}

fn overlay_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{label:<10}"))
                .monospace()
                .size(11.0)
                .color(egui::Color32::from_rgb(130, 155, 195)),
        );
        ui.label(
            egui::RichText::new(value)
                .monospace()
                .size(11.0)
                .color(egui::Color32::from_rgb(215, 228, 255)),
        );
    });
}

fn fmt_lat(lat: f32) -> String {
    if lat >= 0.0 { format!("{lat:.4}° N") } else { format!("{:.4}° S", lat.abs()) }
}

fn fmt_lon(lon: f32) -> String {
    if lon >= 0.0 { format!("{lon:.4}° E") } else { format!("{:.4}° W", lon.abs()) }
}

fn aircraft_color(ac: &HudAircraft) -> egui::Color32 {
    if ac.on_ground {
        return egui::Color32::from_rgb(140, 140, 140);
    }
    match ac.category.as_str() {
        "single_engine" => egui::Color32::from_rgb(64, 210, 64),
        "multi_engine"  => egui::Color32::from_rgb(77, 166, 255),
        "military"      => egui::Color32::from_rgb(255, 80, 60),
        _               => egui::Color32::from_rgb(255, 191, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // On non-Android hosts (where these tests run) is_touch_layout reduces to a
    // screen-width check; on Android it is unconditionally true.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn narrow_screen_is_touch_wide_is_not() {
        assert!(is_touch_layout(400.0), "phone-width portrait → touch");
        assert!(is_touch_layout(599.0), "just under threshold → touch");
        assert!(!is_touch_layout(600.0), "at threshold → not touch");
        assert!(!is_touch_layout(1920.0), "desktop → not touch");
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn unknown_width_is_not_touch() {
        // A zero/empty screen rect (first frame before layout) must not flip the
        // whole UI into mobile mode.
        assert!(!is_touch_layout(0.0));
    }

    #[test]
    fn android_is_always_touch() {
        #[cfg(target_os = "android")]
        assert!(is_touch_layout(2000.0));
        // On non-Android this just documents intent; nothing to assert.
    }
}
