use egui::{Context, Key, ScrollArea, TextEdit};
use tracing::Level;

use crate::log_capture::{LogBuffer, LogEntry};
use crate::settings::Settings;

const MAX_OUTPUT: usize = 500;
/// Seconds the result overlay stays fully visible before fading.
const RESULT_TTL: f32 = 6.0;
/// Duration of the fade-out in seconds.
const RESULT_FADE: f32 = 0.8;
/// Pixel height of one monospace line inside the overlay.
const OVERLAY_LINE_H: f32 = 16.0;
/// Maximum lines visible at once in the overlay (scroll for more).
const OVERLAY_MAX_LINES: usize = 12;

// ─── Console output ───────────────────────────────────────────────────────────

pub struct ConsoleOutput {
    /// A datastream's source was switched; caller should hot-swap the pipeline source.
    /// Value is the datastream name, e.g. `"flights"`.
    pub set_source:        Option<String>,
    /// User issued `refresh <layer>` — caller should call `pipeline.force_refresh`.
    pub refresh_layer:     Option<String>,
    /// User issued `layer <name> on|off` — caller should update `LayerName.visible`.
    pub set_layer_visible: Option<(String, bool)>,
    /// User issued a `geo` sub-command.
    pub geo_cmd:           Option<GeoCmd>,
    /// User issued a `topo` sub-command.
    pub topo_cmd:          Option<TopoCmd>,
    /// User issued a `diag` sub-command.
    pub diag_cmd:          Option<DiagCmd>,
    /// User issued a `location` sub-command.
    pub location_cmd:      Option<LocationCmd>,
    /// User issued an `aircraft` sub-command.
    pub aircraft_cmd:      Option<AircraftCmd>,
    /// User issued a `download` sub-command.
    pub download_cmd:      Option<DownloadCmd>,
    /// User issued a `directions` sub-command.
    pub directions_cmd:    Option<DirectionsCmd>,
    /// User issued a `milsym` sub-command.
    pub milsym_cmd:        Option<MilsymCmd>,
    /// User issued a `symbol` sub-command.
    pub symbol_cmd:        Option<SymbolCmd>,
    /// User issued `kill [<stream>]` — stop one or all pipeline streams.
    pub kill_target:       Option<KillTarget>,
    /// User issued `quit` — request application exit.
    pub quit:              bool,
    /// Text to copy to the system clipboard; consumed inside `draw()`, never reaches `app.rs`.
    clip_text:  Option<String>,
}

impl Default for ConsoleOutput {
    fn default() -> Self {
        Self {
            set_source:        None,
            refresh_layer:     None,
            set_layer_visible: None,
            geo_cmd:           None,
            topo_cmd:          None,
            diag_cmd:          None,
            location_cmd:      None,
            aircraft_cmd:      None,
            download_cmd:      None,
            directions_cmd:    None,
            milsym_cmd:        None,
            symbol_cmd:        None,
            kill_target:       None,
            quit:              false,
            clip_text:         None,
        }
    }
}

/// Topology-layer console commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum TopoCmd {
    /// Load a GeoJSON file as a named layer.
    Load { name: String, path: String },
    /// Toggle a named layer's visibility.
    SetVisible { name: String, visible: bool },
    /// Remove a named layer entirely.
    Drop(String),
}

/// World-map console commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum GeoCmd {
    SetNames(bool),
    SetFill(bool),
    SetBorders(bool),
    /// Toggle the globe wireframe mesh visibility.
    SetMesh(bool),
    Back,
    Status,
}

/// Diagnostics commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum DiagCmd {
    /// Print the stats table (delegated back via push_line chain).
    PrintTable,
    /// Reset the rolling accumulator.
    Reset,
    /// Toggle compact HUD overlay.
    SetOverlay(bool),
}

/// Location service commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum LocationCmd {
    /// Switch the active source by name (mock / network / native / off).
    SetSource(String),
    /// Update the WebSocket URL and reconnect the network source.
    Connect(String),
    /// Lock/unlock camera to follow the current location.
    SetFollow(bool),
    /// Print the latest fix to the console.
    Status,
    /// Stop the active source (same as SetSource("off")).
    Stop,
}

/// Aircraft display commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum AircraftCmd {
    /// Set the always-on label field: "none" | "callsign" | "type" | "altitude" | "speed" | "icao24".
    SetLabel(String),
    /// Enable or disable click-to-follow.
    SetFollowOnClick(bool),
}

/// Download catalog / custom-URL commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum DownloadCmd {
    /// Fetch a GeoJSON URL and load it as a topo layer.
    Fetch { name: String, url: String, cache_secs: u64 },
    /// Delete the cached data and drop the layer.
    Clear(String),
}

/// Routing / directions commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum DirectionsCmd {
    /// Fetch a driving route between two locations (address or "lat,lon").
    Fetch { from: String, to: String },
}

/// Military symbology commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum MilsymCmd {
    /// Enable or disable milsym mode.
    SetEnabled(bool),
    /// Set symbol scale multiplier.
    SetScale(f32),
    /// Show/hide echelon indicator above each symbol.
    SetEchelon(bool),
    /// Override the affiliation for a category: entity = "civil" | "military" | "unknown".
    SetAffiliation { entity: String, affiliation: String },
    /// Print current milsym configuration.
    Status,
}

/// Symbol-map console commands forwarded to `app.rs`.
#[derive(Debug, Clone)]
pub enum SymbolCmd {
    /// Load an SVG file from disk and assign it to an icon key in the active standard.
    Assign { key: String, path: String },
    /// Clear the SVG assignment for a key (reverts to hand-drawn icon).
    Clear(String),
    /// Switch the active symbology standard ("app6a" | "milstd2525d").
    SetStandard(String),
    /// List all recognised keys with a loaded/missing indicator.
    List,
}

/// Target for the `kill` command.
#[derive(Debug, Clone)]
pub enum KillTarget {
    /// Stop all pipeline streams.
    All,
    /// Stop a single named stream.
    Stream(String),
}

const MAX_CMD_HISTORY: usize = 50;

struct ResultOverlay {
    lines:   Vec<String>,
    ttl:     f32,
    max_ttl: f32,
    /// Unique ID incremented per command so each result gets a fresh egui Area
    /// with no cached size from the previous result.
    gen:     u64,
}

fn next_overlay_gen() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GEN: AtomicU64 = AtomicU64::new(0);
    GEN.fetch_add(1, Ordering::Relaxed)
}

pub struct Console {
    pub open:         bool,
    input:            String,
    output:           Vec<String>,
    cmd_history:      Vec<String>,
    history_cursor:   Option<usize>,
    /// True while the view is at (or scrolled back to) the bottom.
    auto_scroll:      bool,
    /// Minimum log level to display in the console output.
    filter_level:     Level,
    /// Optional file/module substring filter (case-insensitive).
    filter_module:    Option<String>,
    /// Mirrored geo layer visibility state for status display.
    geo_names:        bool,
    geo_fill:         bool,
    geo_borders:      bool,
    geo_mesh:         bool,
    /// Mirrored topo layer list (name, visible) for status display.
    topo_layers:      Vec<(String, bool)>,
    /// Mirrored aircraft display state for status display.
    aircraft_label:   String,
    aircraft_follow:  bool,
    /// Floating TTL overlay showing the most recent command's output.
    result_overlay:   Option<ResultOverlay>,
    /// Set to true while execute() is running so push() also writes to captured_lines.
    capturing:        bool,
    captured_lines:   Vec<String>,
    log_buffer:       LogBuffer,
    /// Set by `clip <deferred-cmd>` so push_line() intercepts the next result delivery.
    clip_next_push:   bool,
    /// Populated by push_line() when clip_next_push fires; copied to clipboard in draw().
    pending_clip_text: Option<String>,
}

impl Console {
    pub fn new(log_buffer: LogBuffer) -> Self {
        let mut c = Self {
            open:          false,
            input:         String::new(),
            output:        Vec::new(),
            cmd_history:   Vec::new(),
            history_cursor: None,
            auto_scroll:   true,
            filter_level:  Level::WARN,
            filter_module: None,
            geo_names:        true,
            geo_fill:         true,
            geo_borders:      true,
            geo_mesh:         true,
            topo_layers:      Vec::new(),
            aircraft_label:   "none".to_string(),
            aircraft_follow:  true,
            result_overlay:   None,
            capturing:        false,
            captured_lines:   Vec::new(),
            log_buffer,
            clip_next_push:   false,
            pending_clip_text: None,
        };
        c.push("rusty_glauxb console  —  type 'help' for commands");
        c
    }
}

impl Console {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        let s: String = line.into();
        if self.clip_next_push {
            self.clip_next_push = false;
            self.pending_clip_text = Some(s.clone());
        }
        self.push(s);
    }

    fn push(&mut self, line: impl Into<String>) {
        let s = line.into();
        if self.capturing {
            self.captured_lines.push(s.clone());
        }
        self.output.push(s);
        if self.output.len() > MAX_OUTPUT {
            self.output.drain(0..self.output.len() - MAX_OUTPUT);
        }
    }

    /// Draw the console window.  Returns a `ConsoleOutput` describing any
    /// side-effects the caller (app.rs) needs to act on this frame.
    pub fn draw(&mut self, ctx: &Context, settings: &mut Settings) -> ConsoleOutput {
        // Drain log entries emitted since last frame, applying display filters.
        // Runs whether the console is open or not to prevent unbounded buffer growth.
        let new_entries: Vec<LogEntry> = self.log_buffer
            .lock()
            .map(|mut buf| buf.drain(..).collect())
            .unwrap_or_default();
        let lvl = self.filter_level;
        let mod_filter = self.filter_module.clone();
        for entry in new_entries {
            // tracing Level ordering: TRACE > DEBUG > INFO > WARN > ERROR (more verbose = greater).
            // "at least as severe as filter" means level <= filter_level.
            if entry.level <= lvl
                && mod_filter.as_deref().map_or(true, |p| entry.matches_module(p))
            {
                self.push(entry.format());
            }
        }

        // Deferred clip: app.rs delivered a result via push_line() since last frame.
        if let Some(text) = self.pending_clip_text.take() {
            ctx.copy_text(text);
            self.push("clip: copied to clipboard");
        }

        if !self.open {
            return ConsoleOutput::default();
        }

        // ── Tick overlay TTL ──────────────────────────────────────────────────
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if let Some(ov) = &mut self.result_overlay {
            ov.ttl -= dt;
            if ov.ttl <= 0.0 {
                self.result_overlay = None;
            }
        }

        // Pre-compute overlay display data before borrowing ctx for the window.
        let overlay_snapshot: Option<(Vec<String>, f32, u64)> =
            self.result_overlay.as_ref().map(|ov| {
                let alpha = if ov.ttl >= RESULT_FADE {
                    1.0_f32
                } else {
                    (ov.ttl / RESULT_FADE).max(0.0)
                };
                (ov.lines.clone(), alpha, ov.gen)
            });

        // egui sets this to false when the user clicks X
        let mut window_open  = true;
        let mut submitted_cmd: Option<String> = None;
        let mut reset_ttl    = false;

        egui::Window::new("Console")
            .resizable(true)
            .default_width(620.0)
            .default_height(420.0)
            .collapsible(false)
            .order(egui::Order::Foreground)
            .open(&mut window_open)
            .show(ctx, |ui| {
                let output_height = (ui.available_height() - 36.0).max(50.0);

                let scroll_out = ScrollArea::vertical()
                    .max_height(output_height)
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.auto_scroll)
                    .show(ui, |ui| {
                        for line in &self.output {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(line).monospace().size(12.0),
                                )
                                .truncate(),
                            );
                        }
                    });

                // Re-enable auto-scroll when the user scrolls back to the bottom;
                // disable it as soon as they scroll away.
                let at_bottom = scroll_out.content_size.y - scroll_out.state.offset.y
                    <= scroll_out.inner_rect.height() + 2.0;
                self.auto_scroll = at_bottom;

                // ── Floating result overlay ───────────────────────────────────
                if let Some((lines, alpha, gen)) = &overlay_snapshot {
                    let sr = scroll_out.inner_rect;
                    let n_vis = lines.len().min(OVERLAY_MAX_LINES);
                    let ov_h  = (n_vis as f32 * OVERLAY_LINE_H + 16.0)
                        .min(sr.height() * 0.65)
                        .max(32.0);
                    let ov_w  = (sr.width() - 20.0).max(120.0);
                    // Anchor to the TOP of the scroll area so the overlay never
                    // reaches the separator / input bar below.
                    let ov_pos = egui::pos2(
                        sr.min.x + 10.0,
                        sr.min.y + 8.0,
                    );

                    let bg_a   = (*alpha * 210.0) as u8;
                    let text_a = (*alpha * 235.0) as u8;
                    let rim_a  = (*alpha * 90.0)  as u8;

                    egui::Area::new(egui::Id::new(("cmd_result_overlay", *gen)))
                        .fixed_pos(ov_pos)
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            egui::Frame::none()
                                .fill(egui::Color32::from_rgba_unmultiplied(15, 28, 20, bg_a))
                                .rounding(6.0)
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_rgba_unmultiplied(90, 150, 110, rim_a),
                                ))
                                .inner_margin(egui::Margin::same(6.0))
                                .show(ui, |ui| {
                                    ui.set_min_width(ov_w - 12.0);
                                    ui.set_max_width(ov_w - 12.0);
                                    let text_color = egui::Color32::from_rgba_unmultiplied(
                                        160, 215, 175, text_a,
                                    );
                                    ScrollArea::vertical()
                                        .id_salt("cmd_result_scroll")
                                        .auto_shrink([false, false])
                                        .max_height(ov_h - 12.0)
                                        .show(ui, |ui| {
                                            for line in lines {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(line)
                                                            .monospace()
                                                            .size(12.0)
                                                            .color(text_color),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                        });
                                });
                        });

                    // Check pointer position directly — child widgets consume the
                    // Area's hover response so area_resp.response.hovered() is unreliable.
                    let overlay_rect = egui::Rect::from_min_size(
                        ov_pos, egui::vec2(ov_w, ov_h),
                    );
                    let pointer_over = ctx.input(|i| {
                        i.pointer.hover_pos().map_or(false, |p| overlay_rect.contains(p))
                    });
                    if pointer_over {
                        reset_ttl = true;
                    }
                }

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(">").monospace());
                    let resp = ui.add(
                        TextEdit::singleline(&mut self.input)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace),
                    );

                    // Check lost_focus BEFORE re-requesting focus; otherwise request_focus
                    // re-assigns focus this frame and lost_focus returns false.
                    if resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) {
                        let cmd = self.input.trim().to_string();
                        if !cmd.is_empty() {
                            submitted_cmd = Some(cmd);
                        }
                        self.input.clear();
                    }

                    // Keep the text field focused so the user can keep typing.
                    resp.request_focus();

                    // History navigation
                    if resp.has_focus() {
                        if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
                            let next = match self.history_cursor {
                                None if !self.cmd_history.is_empty() => {
                                    Some(self.cmd_history.len() - 1)
                                }
                                Some(0) => Some(0),
                                Some(n) => Some(n - 1),
                                None    => None,
                            };
                            if let Some(i) = next {
                                self.input          = self.cmd_history[i].clone();
                                self.history_cursor = Some(i);
                            }
                        }
                        if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
                            let next = match self.history_cursor {
                                Some(n) if n + 1 < self.cmd_history.len() => Some(n + 1),
                                _ => None,
                            };
                            self.history_cursor = next;
                            self.input = next
                                .map(|i| self.cmd_history[i].clone())
                                .unwrap_or_default();
                        }
                    }
                });
            });

        // Sync close state from X button
        if !window_open {
            self.open = false;
        }

        // Reset overlay TTL when the user interacts with it.
        if reset_ttl {
            if let Some(ov) = &mut self.result_overlay {
                ov.ttl = ov.max_ttl;
            }
        }

        let mut output = ConsoleOutput::default();
        if let Some(cmd) = submitted_cmd {
            self.push(format!("> {cmd}"));
            if !self.cmd_history.last().is_some_and(|l| l == &cmd) {
                if self.cmd_history.len() >= MAX_CMD_HISTORY {
                    self.cmd_history.remove(0);
                }
                self.cmd_history.push(cmd.clone());
            }
            self.history_cursor = None;
            self.auto_scroll    = true;
            self.capturing      = true;
            self.captured_lines.clear();
            output = self.execute(&cmd, settings);
            self.capturing = false;
            // Instant clip: copy synchronous output now while ctx is available.
            if let Some(text) = output.clip_text.take() {
                ctx.copy_text(text);
            }
            let result_lines = std::mem::take(&mut self.captured_lines);
            if !result_lines.is_empty() {
                self.result_overlay = Some(ResultOverlay {
                    lines:   result_lines,
                    ttl:     RESULT_TTL,
                    max_ttl: RESULT_TTL,
                    gen:     next_overlay_gen(),
                });
            }
        }

        output
    }

    fn execute(&mut self, raw: &str, settings: &mut Settings) -> ConsoleOutput {
        let parts: Vec<&str> = raw.splitn(3, ' ').collect();
        let help = parts.get(1).copied() == Some("help");

        match parts[0] {
            // ── help ─────────────────────────────────────────────────────────
            "help" => {
                self.push("Commands  —  type '<cmd> help' for details");
                self.push("");
                self.push("  set       change a setting and save to disk");
                self.push("  get       read the current value of a setting");
                self.push("  list      show all settings and their values");
                self.push("  source    switch the data source for any datastream");
                self.push("  refresh   force-refresh a data layer");
                self.push("  layer     toggle layer visibility");
                self.push("  log       view and control the in-app log stream");
                self.push("  geo       control the world-map polygon layer");
                self.push("  topo      load and control a GeoJSON topology layer");
                self.push("  camera    control camera animation mode");
                self.push("  diag      show frame-time / FPS / memory diagnostics");
                self.push("  clip      copy any command's output to the clipboard");
                self.push("  clear     clear this output");
                self.push("  download    browse and fetch GeoJSON datasets from the catalog");
                self.push("  directions  get driving directions between two locations");
                self.push("  location    control location services");
                self.push("  aircraft    configure aircraft dot labels and click-to-follow");
                self.push("  milsym      toggle MIL-STD-2525D military symbology mode");
                self.push("  symbol      assign SVG icons per symbology standard");
                self.push("  exit        close the console");
                self.push("  kill        stop streams or kill the application");
            }

            // ── set ──────────────────────────────────────────────────────────
            "set" => {
                if help {
                    self.push("set <key> <value>");
                    self.push("");
                    self.push("Changes a setting and persists it to disk immediately.");
                    self.push("");
                    self.push("  Examples:");
                    self.push("    set flight_source opensky");
                    self.push("    set refresh_secs 5");
                    self.push("");
                    self.push("Use 'list' to see all keys, 'get <key>' to read one.");
                } else if parts.len() < 3 {
                    self.push("usage: set <key> <value>  (type 'set help' for details)");
                } else {
                    let prev = settings.flight_source.clone();
                    let msg  = settings.set(parts[1], parts[2]);
                    self.push(msg);
                    settings.save();
                    if settings.flight_source != prev {
                        return ConsoleOutput { set_source: Some("flights".to_string()), ..Default::default() };
                    }
                }
            }

            // ── get ──────────────────────────────────────────────────────────
            "get" => {
                if help {
                    self.push("get <key>");
                    self.push("");
                    self.push("Reads the current value of a single setting.");
                    self.push("");
                    self.push("  Example:  get flight_source");
                    self.push("");
                    self.push("Use 'list' to see all available keys.");
                } else if parts.len() < 2 || parts[1].is_empty() {
                    self.push("usage: get <key>  (type 'get help' for details)");
                } else {
                    let v = settings.get(parts[1]);
                    self.push(format!("{} = {v}", parts[1]));
                }
            }

            // ── list ─────────────────────────────────────────────────────────
            "list" => {
                if help {
                    self.push("list");
                    self.push("");
                    self.push("Prints every setting key and its current value.");
                    self.push("Use 'set <key> <value>' to change one.");
                } else {
                    for key in Settings::all_keys() {
                        let v = settings.get(key);
                        self.push(format!("  {key} = {v}"));
                    }
                }
            }

            // ── source ───────────────────────────────────────────────────────
            "source" => {
                if help {
                    self.push("source                    — list all datastreams and current sources");
                    self.push("source <stream>           — list available sources for that stream");
                    self.push("source <stream> <src>     — switch the stream to a different source");
                    self.push("");
                    self.push("  Streams:  flights   directions   location");
                    self.push("            roads, railways  (fixed OSM source, not configurable)");
                    self.push("");
                    self.push("  Examples:");
                    self.push("    source flights opensky");
                    self.push("    source location mock");
                } else {
                    let stream = parts.get(1).copied().unwrap_or("");
                    let api    = parts.get(2).copied().unwrap_or("");
                    match stream {
                        "" => {
                            self.push("datastream    source");
                            self.push(format!("  flights     {}", settings.flight_source));
                            self.push(format!("  directions  {}", settings.directions_source));
                            self.push("  location    (use 'source location' to list)");
                            self.push("  roads       osm (fixed)");
                            self.push("  railways    osm (fixed)");
                        }
                        "flights" => {
                            if api.is_empty() {
                                self.push(format!("flights source: {}  (current)", settings.flight_source));
                                self.push("");
                                self.push("  adsbfi       — ADSB.fi, free, no auth");
                                self.push("  opensky      — OpenSky Network; set opensky_user/pass for higher limits");
                                self.push("  adsbexchange — ADS-B Exchange via RapidAPI; requires adsbx_api_key");
                                self.push("  flightaware  — FlightAware AeroAPI; requires flightaware_api_key");
                            } else {
                                const VALID: &[&str] = &["adsbfi", "opensky", "adsbexchange", "flightaware"];
                                if !VALID.contains(&api) {
                                    self.push(format!(
                                        "unknown flights source '{api}' — valid: adsbfi, opensky, adsbexchange, flightaware"
                                    ));
                                } else if settings.get("flight_source") == api {
                                    self.push(format!("flights source already: {api}"));
                                } else {
                                    settings.set("flight_source", api);
                                    settings.save();
                                    self.push(format!("flights → {api}"));
                                    return ConsoleOutput {
                                        set_source: Some("flights".to_string()),
                                        ..Default::default()
                                    };
                                }
                            }
                        }
                        "location" => {
                            if api.is_empty() {
                                self.push("location sources:");
                                self.push("  mock     — animated circular path (always available)");
                                self.push("  network  — WebSocket stream; set URL with 'location connect'");
                                self.push("  native   — Android device GPS (Android only)");
                                self.push("  off      — stop location services");
                            } else {
                                self.push(format!("location → {api}"));
                                return ConsoleOutput {
                                    location_cmd: Some(LocationCmd::SetSource(api.to_string())),
                                    ..Default::default()
                                };
                            }
                        }
                        "directions" => {
                            if api.is_empty() {
                                self.push(format!("directions source: {}  (current)", settings.directions_source));
                                self.push("");
                                self.push("  osrm    — OSRM public API, no auth required (default)");
                                self.push("  google  — Google Maps Directions API; requires google_maps_api_key");
                            } else {
                                match api {
                                    "osrm" | "google" => {
                                        settings.directions_source = api.to_string();
                                        settings.save();
                                        self.push(format!("directions → {api}"));
                                    }
                                    _ => self.push(format!(
                                        "unknown directions source '{api}' — valid: osrm, google"
                                    )),
                                }
                            }
                        }
                        "roads" | "railways" => {
                            self.push(format!("{stream}: source is fixed (OSM — not configurable)"));
                        }
                        other => {
                            self.push(format!(
                                "unknown datastream '{other}' — known: flights, directions, location, roads, railways"
                            ));
                        }
                    }
                }
            }

            // ── refresh ──────────────────────────────────────────────────────
            "refresh" => {
                if help {
                    self.push("refresh <layer>");
                    self.push("");
                    self.push("Forces an immediate re-fetch of a data layer.");
                    self.push("For pipeline layers, bypasses the normal refresh interval.");
                    self.push("For download layers, busts the disk cache and re-downloads.");
                    self.push("");
                    self.push("  Pipeline layers:  flights, roads, railways");
                    self.push("  Download layers:  airports, earthquakes, volcanoes, …");
                    self.push("  Example:  refresh airports");
                } else if parts.len() < 2 || parts[1].is_empty() {
                    self.push("usage: refresh <layer>  (type 'refresh help' for details)");
                } else {
                    let layer = parts[1].to_string();
                    self.push(format!("refreshing layer: {layer}"));
                    return ConsoleOutput { refresh_layer: Some(layer), ..Default::default() };
                }
            }

            // ── layer ────────────────────────────────────────────────────────
            "layer" => {
                if help {
                    self.push("layer <name> on|off");
                    self.push("");
                    self.push("Toggles the visibility of a named data layer.");
                    self.push("");
                    self.push("  Available layers:  roads  railways  rivers  lakes  railroads  topo");
                    self.push("  Examples:");
                    self.push("    layer rivers off");
                    self.push("    layer roads on");
                } else if parts.len() < 3 {
                    self.push("usage: layer <name> on|off  (type 'layer help' for details)");
                } else {
                    let name = parts[1].to_string();
                    match parts[2] {
                        "on" => {
                            self.push(format!("{name}: visible"));
                            return ConsoleOutput {
                                set_layer_visible: Some((name, true)),
                                ..Default::default()
                            };
                        }
                        "off" => {
                            self.push(format!("{name}: hidden"));
                            return ConsoleOutput {
                                set_layer_visible: Some((name, false)),
                                ..Default::default()
                            };
                        }
                        other => {
                            self.push(format!("unknown visibility '{other}'  (use on or off)"));
                        }
                    }
                }
            }

            // ── log ──────────────────────────────────────────────────────────
            "log" => {
                if help {
                    self.push("log  —  in-app log stream filter");
                    self.push("");
                    self.push("  log                    show current filter settings");
                    self.push("  log level <level>      set minimum display level");
                    self.push("                           trace  debug  info  warn  error");
                    self.push("  log module <pattern>   show only logs whose path contains pattern");
                    self.push("  log module off         clear module filter (show all)");
                    self.push("");
                    self.push("Default level is INFO. The buffer always captures DEBUG+,");
                    self.push("so lowering the level reveals events already buffered.");
                } else {
                    match parts.get(1).copied() {
                        None | Some("") => {
                            self.push(format!("log level  = {}", self.filter_level));
                            self.push(format!("log module = {}",
                                self.filter_module.as_deref().unwrap_or("(all)")));
                        }
                        Some("level") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "trace" => { self.filter_level = Level::TRACE; self.push("log level = TRACE"); }
                                "debug" => { self.filter_level = Level::DEBUG; self.push("log level = DEBUG"); }
                                "info"  => { self.filter_level = Level::INFO;  self.push("log level = INFO"); }
                                "warn"  => { self.filter_level = Level::WARN;  self.push("log level = WARN"); }
                                "error" => { self.filter_level = Level::ERROR; self.push("log level = ERROR"); }
                                other   => { self.push(format!("unknown level '{other}'  (type 'log help')")); }
                            }
                        }
                        Some("module") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "" => { self.push("usage: log module <pattern>  |  log module off"); }
                                "off" => {
                                    self.filter_module = None;
                                    self.push("log module filter cleared");
                                }
                                pat => {
                                    self.filter_module = Some(pat.to_string());
                                    self.push(format!("log module = {pat}"));
                                }
                            }
                        }
                        Some(other) => {
                            self.push(format!("unknown log sub-command '{other}'  (type 'log help')"));
                        }
                    }
                }
            }

            // ── geo ──────────────────────────────────────────────────────────
            "geo" => {
                if help {
                    self.push("geo  —  world-map polygon layer");
                    self.push("");
                    self.push("  geo                  show current settings");
                    self.push("  geo names on|off     toggle country/state name labels");
                    self.push("  geo fill on|off      toggle polygon fill");
                    self.push("  geo borders on|off   toggle border outlines");
                    self.push("  geo mesh on|off      toggle the globe wireframe mesh");
                    self.push("  geo back             navigate up one admin level");
                    self.push("");
                    self.push("Click a country on the globe to drill into its states.");
                } else {
                    match parts.get(1).copied() {
                        None | Some("") => {
                            self.push(format!("geo names   = {}", if self.geo_names   { "on" } else { "off" }));
                            self.push(format!("geo fill    = {}", if self.geo_fill    { "on" } else { "off" }));
                            self.push(format!("geo borders = {}", if self.geo_borders { "on" } else { "off" }));
                            self.push(format!("geo mesh    = {}", if self.geo_mesh    { "on" } else { "off" }));
                        }
                        Some("names") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => { self.geo_names = true;  self.push("geo names = on");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetNames(true)),  ..Default::default() }; }
                                "off" => { self.geo_names = false; self.push("geo names = off");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetNames(false)), ..Default::default() }; }
                                other => { self.push(format!("usage: geo names on|off  (got '{other}')")); }
                            }
                        }
                        Some("fill") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => { self.geo_fill = true;  self.push("geo fill = on");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetFill(true)),  ..Default::default() }; }
                                "off" => { self.geo_fill = false; self.push("geo fill = off");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetFill(false)), ..Default::default() }; }
                                other => { self.push(format!("usage: geo fill on|off  (got '{other}')")); }
                            }
                        }
                        Some("borders") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => { self.geo_borders = true;  self.push("geo borders = on");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetBorders(true)),  ..Default::default() }; }
                                "off" => { self.geo_borders = false; self.push("geo borders = off");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetBorders(false)), ..Default::default() }; }
                                other => { self.push(format!("usage: geo borders on|off  (got '{other}')")); }
                            }
                        }
                        Some("mesh") => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => { self.geo_mesh = true;  self.push("geo mesh = on");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetMesh(true)),  ..Default::default() }; }
                                "off" => { self.geo_mesh = false; self.push("geo mesh = off");
                                           return ConsoleOutput { geo_cmd: Some(GeoCmd::SetMesh(false)), ..Default::default() }; }
                                other => { self.push(format!("usage: geo mesh on|off  (got '{other}')")); }
                            }
                        }
                        Some("back") => {
                            self.push("geo: navigating up one level");
                            return ConsoleOutput { geo_cmd: Some(GeoCmd::Back), ..Default::default() };
                        }
                        Some("status") => {
                            return ConsoleOutput { geo_cmd: Some(GeoCmd::Status), ..Default::default() };
                        }
                        Some(other) => {
                            self.push(format!("unknown geo sub-command '{other}'  (type 'geo help')"));
                        }
                    }
                }
            }

            // ── topo ─────────────────────────────────────────────────────────
            "topo" => {
                if help {
                    self.push("topo  —  named GeoJSON topology layers");
                    self.push("");
                    self.push("  topo list                    list all loaded layers");
                    self.push("  topo load <name> <path>      load GeoJSON as a named layer");
                    self.push("  topo on   <name>             show a layer");
                    self.push("  topo off  <name>             hide a layer");
                    self.push("  topo drop <name>             remove a layer entirely");
                    self.push("");
                    self.push("Supports: LineString, MultiLineString, Polygon (outline),");
                    self.push("          MultiPolygon (outline), Feature, FeatureCollection.");
                    self.push("");
                    self.push("  Examples:");
                    self.push("    topo load rivers  C:/data/ne_rivers.geojson");
                    self.push("    topo load lakes   C:/data/ne_lakes.geojson");
                    self.push("    topo off  lakes");
                    self.push("    topo drop rivers");
                } else {
                    // splitn(3) gives parts[0]="topo" parts[1]=sub parts[2]=rest
                    // For "load" we need to split parts[2] further into name + path.
                    let sub = parts.get(1).copied().unwrap_or("");
                    match sub {
                        "" => {
                            self.push("usage: topo <list|load|on|off|drop>  (type 'topo help')");
                        }
                        "list" => {
                            if self.topo_layers.is_empty() {
                                self.push("no topo layers loaded");
                            } else {
                                let lines: Vec<String> = self.topo_layers.iter()
                                    .map(|(n, v)| format!("  [{}]  {n}", if *v { "on " } else { "off" }))
                                    .collect();
                                self.push("topo layers:");
                                for l in lines { self.push(l); }
                            }
                        }
                        "load" => {
                            // parts[2] contains "<name> <path>" (splitn stopped at 3)
                            let rest = parts.get(2).copied().unwrap_or("").trim();
                            let (name, path) = rest.split_once(' ')
                                .map(|(n, p)| (n.trim(), p.trim()))
                                .unwrap_or(("", ""));
                            if name.is_empty() || path.is_empty() {
                                self.push("usage: topo load <name> <path>");
                            } else {
                                // Update or insert mirror entry
                                if let Some(entry) = self.topo_layers.iter_mut().find(|(n, _)| n == name) {
                                    entry.1 = true;
                                } else {
                                    self.topo_layers.push((name.to_string(), true));
                                }
                                self.push(format!("topo: loading '{name}' from {path}"));
                                return ConsoleOutput {
                                    topo_cmd: Some(TopoCmd::Load {
                                        name: name.to_string(),
                                        path: path.to_string(),
                                    }),
                                    ..Default::default()
                                };
                            }
                        }
                        "on" | "off" => {
                            let name = parts.get(2).copied().unwrap_or("").trim();
                            if name.is_empty() {
                                self.push(format!("usage: topo {sub} <name>"));
                            } else {
                                let visible = sub == "on";
                                if let Some(entry) = self.topo_layers.iter_mut().find(|(n, _)| n == name) {
                                    entry.1 = visible;
                                }
                                self.push(format!("topo '{name}': {sub}"));
                                return ConsoleOutput {
                                    topo_cmd: Some(TopoCmd::SetVisible {
                                        name: name.to_string(),
                                        visible,
                                    }),
                                    ..Default::default()
                                };
                            }
                        }
                        "drop" => {
                            let name = parts.get(2).copied().unwrap_or("").trim();
                            if name.is_empty() {
                                self.push("usage: topo drop <name>");
                            } else {
                                self.topo_layers.retain(|(n, _)| n != name);
                                self.push(format!("topo '{name}': dropped"));
                                return ConsoleOutput {
                                    topo_cmd: Some(TopoCmd::Drop(name.to_string())),
                                    ..Default::default()
                                };
                            }
                        }
                        other => {
                            self.push(format!("unknown topo sub-command '{other}'  (type 'topo help')"));
                        }
                    }
                }
            }

            // ── camera ───────────────────────────────────────────────────────
            "camera" => {
                if help {
                    self.push("camera  —  camera animation control");
                    self.push("");
                    self.push("  camera status                      show current mode and parameters");
                    self.push("  camera none                        free camera (no animation)");
                    self.push("  camera rotate [spd] [el]           auto-rotate");
                    self.push("                                       spd — degrees/second (default 10)");
                    self.push("                                       el  — elasticity 0.0–1.0 (default 0.4)");
                    self.push("  camera follow aircraft <icao24>    lock onto an aircraft by ICAO24 hex");
                    self.push("  camera follow location             lock onto the device GPS position");
                    self.push("  camera follow smoothing <0–1>      follow lag (0=instant, 1=very smooth)");
                    self.push("  camera follow distance <r>         follow camera radius (globe radii, >1)");
                    self.push("");
                    self.push("  Examples:");
                    self.push("    camera rotate 10 0.3");
                    self.push("    camera follow aircraft a1b2c3");
                    self.push("    camera follow location");
                    self.push("    camera follow smoothing 0.1");
                    self.push("    camera follow distance 2.5");
                } else {
                    return self.exec_camera(parts, settings);
                }
            }

            // ── clip ─────────────────────────────────────────────────────────
            "clip" => {
                if help {
                    self.push("clip log [level]       — copy session log entries to clipboard");
                    self.push("clip <command> [args]  — run a command and copy its output");
                    self.push("");
                    self.push("  log levels: error  warn (default)  info  debug  trace");
                    self.push("");
                    self.push("  Instant commands copy immediately:");
                    self.push("    clip geo              geo layer toggle state");
                    self.push("    clip camera status    camera mode and params");
                    self.push("    clip list             all settings");
                    self.push("    clip log error        only error entries");
                    self.push("  Deferred commands copy when the result arrives:");
                    self.push("    clip geo status       feature/triangle counts (needs renderer)");
                } else {
                    let sub = parts.get(1).copied().unwrap_or("");
                    if sub.is_empty() {
                        self.push("usage: clip log [level]  |  clip <command> [args]");
                        self.push("type 'clip help' for details");
                    } else if sub == "log" {
                        let level_arg = parts.get(2).copied().unwrap_or("warn");
                        let prefixes: &[&str] = match level_arg {
                            "error" => &["[ERROR]"],
                            "warn"  => &["[ERROR]", "[WARN]"],
                            "info"  => &["[ERROR]", "[WARN]", "[INFO]"],
                            "debug" => &["[ERROR]", "[WARN]", "[INFO]", "[DEBUG]"],
                            "trace" => &["[ERROR]", "[WARN]", "[INFO]", "[DEBUG]", "[TRACE]"],
                            other   => {
                                self.push(format!(
                                    "unknown level '{other}' — use error, warn, info, debug, trace"
                                ));
                                return ConsoleOutput::default();
                            }
                        };
                        let lines: Vec<&str> = self.output.iter()
                            .filter(|l| prefixes.iter().any(|p| l.starts_with(p)))
                            .map(|s| s.as_str())
                            .collect();
                        let n = lines.len();
                        if n == 0 {
                            self.push(format!("no {level_arg}+ log entries in session"));
                        } else {
                            let text = lines.join("\n");
                            self.push(format!("copied {n} {level_arg}+ log entries to clipboard"));
                            return ConsoleOutput { clip_text: Some(text), ..Default::default() };
                        }
                    } else {
                        // Generic: run the inner command and capture whatever it pushes.
                        let inner_cmd = match parts.get(2) {
                            Some(rest) if !rest.is_empty() => format!("{sub} {rest}"),
                            _ => sub.to_string(),
                        };
                        let snapshot = self.captured_lines.len();
                        let inner_output = self.execute(&inner_cmd, settings);
                        let new_lines = self.captured_lines[snapshot..].to_vec();
                        if !new_lines.is_empty() {
                            // Instant: output was pushed synchronously.
                            let n = new_lines.len();
                            let text = new_lines.join("\n");
                            self.push(format!(
                                "copied {} line{} to clipboard",
                                n, if n == 1 { "" } else { "s" }
                            ));
                            return ConsoleOutput { clip_text: Some(text), ..inner_output };
                        } else {
                            // Deferred: result will arrive via push_line() from app.rs.
                            self.clip_next_push = true;
                            self.push("clip: pending — result will be copied when ready");
                            return inner_output;
                        }
                    }
                }
            }

            // ── diag ─────────────────────────────────────────────────────────
            "diag" => {
                if help {
                    self.push("diag  —  frame-time / FPS / memory diagnostics");
                    self.push("");
                    self.push("  diag              print rolling stats table (last ~300 frames)");
                    self.push("  diag reset         clear the accumulator and start fresh");
                    self.push("  diag overlay on|off  show compact fps/gpu line in HUD corner");
                    self.push("");
                    self.push("Columns: mean  p50  p95  p99  max");
                    self.push("Rows:    fps  frame-ms  gpu-ms  cpu-ms  rss-MB  triangles  draw-calls");
                    self.push("GPU time requires TIMESTAMP_QUERY support (discrete GPUs, Vulkan/DX12).");
                } else {
                    match parts.get(1).copied().unwrap_or("") {
                        "" => {
                            return ConsoleOutput {
                                diag_cmd: Some(DiagCmd::PrintTable),
                                ..Default::default()
                            };
                        }
                        "reset" => {
                            return ConsoleOutput {
                                diag_cmd: Some(DiagCmd::Reset),
                                ..Default::default()
                            };
                        }
                        "overlay" => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => return ConsoleOutput {
                                    diag_cmd: Some(DiagCmd::SetOverlay(true)),
                                    ..Default::default()
                                },
                                "off" => return ConsoleOutput {
                                    diag_cmd: Some(DiagCmd::SetOverlay(false)),
                                    ..Default::default()
                                },
                                other => self.push(format!(
                                    "usage: diag overlay on|off  (got '{other}')"
                                )),
                            }
                        }
                        other => {
                            self.push(format!(
                                "unknown diag sub-command '{other}'  (type 'diag help')"
                            ));
                        }
                    }
                }
            }

            // ── clear ────────────────────────────────────────────────────────
            "clear" => {
                if help {
                    self.push("clear");
                    self.push("");
                    self.push("Clears all text from the console output.");
                    self.push("Does not affect the log buffer or any settings.");
                } else {
                    self.output.clear();
                }
            }

            // ── location ─────────────────────────────────────────────────────
            "location" => {
                if help {
                    self.push("location  —  device location services");
                    self.push("");
                    self.push("  location                   show current source and last fix");
                    self.push("  location connect <url>     set WebSocket URL and switch to network");
                    self.push("  location follow on|off     lock camera to follow device position");
                    self.push("  location status            print latest lat/lon/alt/hdg/spd/acc");
                    self.push("  location stop              stop location services");
                    self.push("");
                    self.push("  Use 'source location <src>' to switch the active source.");
                } else {
                    match parts.get(1).copied().unwrap_or("") {
                        "" => {
                            return ConsoleOutput {
                                location_cmd: Some(LocationCmd::Status),
                                ..Default::default()
                            };
                        }
                        "connect" => {
                            let url = parts.get(2).copied().unwrap_or("").to_string();
                            if url.is_empty() {
                                self.push("usage: location connect <ws://ip:port>");
                            } else {
                                self.push(format!("location: connecting to {url}"));
                                return ConsoleOutput {
                                    location_cmd: Some(LocationCmd::Connect(url)),
                                    ..Default::default()
                                };
                            }
                        }
                        "follow" => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on"  => return ConsoleOutput {
                                    location_cmd: Some(LocationCmd::SetFollow(true)),
                                    ..Default::default()
                                },
                                "off" => return ConsoleOutput {
                                    location_cmd: Some(LocationCmd::SetFollow(false)),
                                    ..Default::default()
                                },
                                other => self.push(format!(
                                    "usage: location follow on|off  (got '{other}')"
                                )),
                            }
                        }
                        "status" => {
                            return ConsoleOutput {
                                location_cmd: Some(LocationCmd::Status),
                                ..Default::default()
                            };
                        }
                        "stop" => {
                            self.push("location: stopped");
                            return ConsoleOutput {
                                location_cmd: Some(LocationCmd::Stop),
                                ..Default::default()
                            };
                        }
                        other => {
                            self.push(format!(
                                "unknown location sub-command '{other}'  (type 'location help')"
                            ));
                        }
                    }
                }
            }

            // ── aircraft ─────────────────────────────────────────────────────
            "aircraft" => {
                if help {
                    self.push("aircraft  —  aircraft dot display settings");
                    self.push("");
                    self.push("  aircraft                     show current settings");
                    self.push("  aircraft label <field>       set always-on label next to each dot");
                    self.push("                                 none  callsign  type  altitude  speed  icao24");
                    self.push("  aircraft follow on|off       enable/disable click-to-follow");
                    self.push("");
                    self.push("  Mouse over a dot to see full aircraft details.");
                    self.push("  Click a dot to follow it (when follow is on).");
                } else {
                    match parts.get(1).copied().unwrap_or("") {
                        "" => {
                            self.push(format!("aircraft label  = {}", self.aircraft_label));
                            self.push(format!("aircraft follow = {}", if self.aircraft_follow { "on" } else { "off" }));
                        }
                        "label" => {
                            let field = parts.get(2).copied().unwrap_or("");
                            const VALID: &[&str] =
                                &["none", "callsign", "type", "altitude", "speed", "icao24"];
                            if field.is_empty() {
                                self.push("usage: aircraft label none|callsign|type|altitude|speed|icao24");
                            } else if !VALID.contains(&field) {
                                self.push(format!(
                                    "unknown field '{field}' — valid: none, callsign, type, altitude, speed, icao24"
                                ));
                            } else {
                                self.aircraft_label = field.to_string();
                                self.push(format!("aircraft label = {field}"));
                                return ConsoleOutput {
                                    aircraft_cmd: Some(AircraftCmd::SetLabel(field.to_string())),
                                    ..Default::default()
                                };
                            }
                        }
                        "follow" => {
                            match parts.get(2).copied().unwrap_or("") {
                                "on" => {
                                    self.aircraft_follow = true;
                                    self.push("aircraft follow: on");
                                    return ConsoleOutput {
                                        aircraft_cmd: Some(AircraftCmd::SetFollowOnClick(true)),
                                        ..Default::default()
                                    };
                                }
                                "off" => {
                                    self.aircraft_follow = false;
                                    self.push("aircraft follow: off");
                                    return ConsoleOutput {
                                        aircraft_cmd: Some(AircraftCmd::SetFollowOnClick(false)),
                                        ..Default::default()
                                    };
                                }
                                other => {
                                    self.push(format!(
                                        "usage: aircraft follow on|off  (got '{other}')"
                                    ));
                                }
                            }
                        }
                        other => {
                            self.push(format!(
                                "unknown aircraft sub-command '{other}'  (type 'aircraft help')"
                            ));
                        }
                    }
                }
            }

            // ── exit ─────────────────────────────────────────────────────────
            "exit" => {
                self.open = false;
            }

            // ── kill ─────────────────────────────────────────────────────────
            "kill" => {
                if help {
                    self.push("kill              — stop all data pipeline streams");
                    self.push("kill <stream>     — stop a single named stream");
                    self.push("kill app          — exit the application");
                    self.push("");
                    self.push("  Streams:  flights  roads  railways");
                    self.push("  In-flight fetch tasks finish normally; no new fetches");
                    self.push("  will be scheduled for the stopped stream(s).");
                } else {
                    match parts.get(1).copied().unwrap_or("") {
                        "app" => {
                            self.push("kill app: closing application…");
                            return ConsoleOutput { quit: true, ..Default::default() };
                        }
                        "" => {
                            self.push("kill: stopping all pipeline streams");
                            return ConsoleOutput {
                                kill_target: Some(KillTarget::All),
                                ..Default::default()
                            };
                        }
                        name => {
                            self.push(format!("kill: stopping stream '{name}'"));
                            return ConsoleOutput {
                                kill_target: Some(KillTarget::Stream(name.to_string())),
                                ..Default::default()
                            };
                        }
                    }
                }
            }

            // ── download ─────────────────────────────────────────────────────
            "download" => {
                return self.exec_download(&parts, help);
            }

            // ── directions ───────────────────────────────────────────────────
            "directions" => {
                return self.exec_directions(raw, settings, help);
            }

            // ── milsym ───────────────────────────────────────────────────────
            "milsym" => {
                return self.exec_milsym(&parts, settings, help);
            }

            "symbol" => {
                return self.exec_symbol(&parts, help);
            }

            other => {
                self.push(format!("unknown command '{other}'  (type 'help')"));
            }
        }

        ConsoleOutput::default()
    }

    fn exec_download(&mut self, parts: &[&str], help: bool) -> ConsoleOutput {
        if help {
            self.push("download  —  browse and fetch GeoJSON datasets");
            self.push("");
            self.push("  download                      list catalog sources and cache status");
            self.push("  download list                 same as above");
            self.push("  download <name>               download a catalog source as a layer");
            self.push("  download url <name> <url>     download any GeoJSON URL as a layer");
            self.push("  download clear <name>         delete cached file and drop the layer");
            self.push("");
            self.push("  Point features render as × markers; lines/polygons render as outlines.");
            self.push("  Layers can be toggled with 'topo on|off <name>' after loading.");
            return ConsoleOutput::default();
        }

        let sub = parts.get(1).copied().unwrap_or("");

        match sub {
            // bare `download` or `download list` — show catalog
            "" | "list" => {
                self.push("GeoJSON catalog  (type 'download <name>' to add as a layer):");
                self.push("");
                for entry in crate::download::CATALOG {
                    let status = if crate::download::is_cached(entry.name) {
                        "cached"
                    } else {
                        "available"
                    };
                    self.push(format!(
                        "  {:<12}  {}  [{}]",
                        entry.name, entry.description, status
                    ));
                }
            }

            "url" => {
                // parts[2] = "<name> <url>"
                let rest = parts.get(2).copied().unwrap_or("").trim();
                let Some((name, url)) = rest.split_once(' ') else {
                    self.push("usage: download url <name> <url>");
                    return ConsoleOutput::default();
                };
                let name = name.trim().to_string();
                let url  = url.trim().to_string();
                if name.is_empty() || url.is_empty() {
                    self.push("usage: download url <name> <url>");
                    return ConsoleOutput::default();
                }
                // Add optimistically to topo mirror
                if !self.topo_layers.iter().any(|(n, _)| n == &name) {
                    self.topo_layers.push((name.clone(), true));
                }
                self.push(format!("download: fetching '{name}' from {url}"));
                return ConsoleOutput {
                    download_cmd: Some(DownloadCmd::Fetch {
                        name,
                        url,
                        cache_secs: 24 * 3600, // default 1-day cache for custom URLs
                    }),
                    ..Default::default()
                };
            }

            "clear" => {
                let name = parts.get(2).copied().unwrap_or("").trim();
                if name.is_empty() {
                    self.push("usage: download clear <name>");
                    return ConsoleOutput::default();
                }
                self.topo_layers.retain(|(n, _)| n != name);
                self.push(format!("download: cleared '{name}'"));
                return ConsoleOutput {
                    download_cmd: Some(DownloadCmd::Clear(name.to_string())),
                    ..Default::default()
                };
            }

            // Named catalog entry
            name => {
                match crate::download::find(name) {
                    Some(entry) => {
                        let status = if crate::download::is_cached(entry.name) {
                            "loading from cache"
                        } else {
                            "downloading…"
                        };
                        self.push(format!("download: '{}' — {status}", entry.name));
                        if !self.topo_layers.iter().any(|(n, _)| n == entry.name) {
                            self.topo_layers.push((entry.name.to_string(), true));
                        }
                        return ConsoleOutput {
                            download_cmd: Some(DownloadCmd::Fetch {
                                name:       entry.name.to_string(),
                                url:        entry.url.to_string(),
                                cache_secs: entry.cache_secs,
                            }),
                            ..Default::default()
                        };
                    }
                    None => {
                        self.push(format!("download: unknown source '{name}'"));
                        self.push("  type 'download list' to see available sources");
                    }
                }
            }
        }

        ConsoleOutput::default()
    }

    /// Handle `directions [<from> to <to>]`
    fn exec_directions(&mut self, raw: &str, settings: &Settings, help: bool) -> ConsoleOutput {
        if help {
            self.push("directions <from> to <to>");
            self.push("");
            self.push("Fetch a driving route and render it on the globe.");
            self.push("Both endpoints can be GPS coordinates or a place name.");
            self.push("");
            self.push("  Separators: 'to'  or  '|'");
            self.push("");
            self.push("  Examples:");
            self.push("    directions San Francisco to Los Angeles");
            self.push("    directions 37.7749,-122.4194 to 34.0522,-118.2437");
            self.push("    directions JFK | LAX");
            self.push("");
            self.push("  Switch routing source:");
            self.push("    source directions osrm    (free, no auth)");
            self.push("    source directions google  (requires google_maps_api_key)");
            self.push("");
            self.push("  Remove route:  topo drop directions");
            return ConsoleOutput::default();
        }

        // Strip the command name, leaving "from <sep> to"
        let rest = raw.strip_prefix("directions").unwrap_or("").trim();

        if rest.is_empty() {
            self.push(format!("directions source: {}", settings.directions_source));
            self.push("usage: directions <from> to <to>   (type 'directions help' for details)");
            return ConsoleOutput::default();
        }

        // Parse separators: " to " (case-insensitive) then "|" then single space
        let (from, to) = if let Some(i) = rest.to_lowercase().find(" to ") {
            (&rest[..i], &rest[i + 4..])
        } else if let Some((a, b)) = rest.split_once('|') {
            (a, b)
        } else if let Some((a, b)) = rest.split_once(' ') {
            (a, b)
        } else {
            self.push("usage: directions <from> to <to>   (type 'directions help' for details)");
            return ConsoleOutput::default();
        };

        let from = from.trim().to_string();
        let to   = to.trim().to_string();

        if from.is_empty() || to.is_empty() {
            self.push("usage: directions <from> to <to>   (type 'directions help' for details)");
            return ConsoleOutput::default();
        }

        self.push(format!("directions: {from} → {to}  (fetching…)"));
        ConsoleOutput {
            directions_cmd: Some(DirectionsCmd::Fetch { from, to }),
            ..Default::default()
        }
    }

    fn exec_milsym(&mut self, parts: &[&str], settings: &Settings, help: bool) -> ConsoleOutput {
        if help {
            self.push("milsym  —  MIL-STD-2525D military symbology overlay");
            self.push("");
            self.push("  milsym                  show current configuration");
            self.push("  milsym on|off            enable or disable symbology mode");
            self.push("  milsym scale <f>         symbol size multiplier (default 1.0)");
            self.push("  milsym echelon on|off    show/hide echelon indicator above frame");
            self.push("  milsym affiliation civil|military|unknown <friend|hostile|neutral|unknown>");
            self.push("");
            self.push("  Affiliations:  friend (blue rect)  hostile (red diamond)");
            self.push("                 neutral (green sq)   unknown (yellow circle)");
            self.push("");
            self.push("  Default mapping: single/multi-engine → friend, military → hostile,");
            self.push("                   misc/balloon        → unknown");
            return ConsoleOutput::default();
        }

        let sub = parts.get(1).copied().unwrap_or("");

        match sub {
            "" => {
                self.push(format!(
                    "milsym  enabled={}  scale={}  echelon={}",
                    if settings.milsym_enabled { "on" } else { "off" },
                    settings.milsym_scale,
                    if settings.milsym_show_echelon { "on" } else { "off" },
                ));
                self.push(format!(
                    "  civil→{}  military→{}  unknown→{}",
                    settings.milsym_civil_affiliation,
                    settings.milsym_military_affiliation,
                    settings.milsym_unknown_affiliation,
                ));
                self.push("  icon size scales with zoom — adjust with: milsym scale <0.5…2.0>");
                ConsoleOutput::default()
            }
            "on" | "off" => ConsoleOutput {
                milsym_cmd: Some(MilsymCmd::SetEnabled(sub == "on")),
                ..Default::default()
            },
            "scale" => {
                let v = parts.get(2).copied().unwrap_or("");
                match v.parse::<f32>() {
                    Ok(f) if f > 0.0 => ConsoleOutput {
                        milsym_cmd: Some(MilsymCmd::SetScale(f)),
                        ..Default::default()
                    },
                    Ok(_) => { self.push("error: scale must be > 0"); ConsoleOutput::default() }
                    Err(_) => { self.push("usage: milsym scale <number>"); ConsoleOutput::default() }
                }
            }
            "echelon" => {
                match parts.get(2).copied().unwrap_or("") {
                    "on"  => ConsoleOutput { milsym_cmd: Some(MilsymCmd::SetEchelon(true)),  ..Default::default() },
                    "off" => ConsoleOutput { milsym_cmd: Some(MilsymCmd::SetEchelon(false)), ..Default::default() },
                    _     => { self.push("usage: milsym echelon on|off"); ConsoleOutput::default() }
                }
            }
            "affiliation" => {
                let entity      = parts.get(2).copied().unwrap_or("").to_string();
                let affiliation = parts.get(3).copied().unwrap_or("").to_string();
                if matches!(entity.as_str(), "civil" | "military" | "unknown")
                    && matches!(affiliation.as_str(), "friend" | "hostile" | "neutral" | "unknown")
                {
                    ConsoleOutput {
                        milsym_cmd: Some(MilsymCmd::SetAffiliation { entity, affiliation }),
                        ..Default::default()
                    }
                } else {
                    self.push("usage: milsym affiliation civil|military|unknown friend|hostile|neutral|unknown");
                    ConsoleOutput::default()
                }
            }
            _ => {
                self.push("usage: milsym [on|off] [scale <f>] [echelon on|off] [affiliation …]");
                self.push("type 'milsym help' for details");
                ConsoleOutput::default()
            }
        }
    }

    /// Handle `symbol <sub> [args…]`
    fn exec_symbol(&mut self, parts: &[&str], help: bool) -> ConsoleOutput {
        if help || parts.get(1).copied().unwrap_or("") == "help" {
            self.push("symbol assign <key> <path>   — load SVG file and assign to icon key");
            self.push("symbol clear <key>           — remove SVG assignment (reverts to built-in)");
            self.push("symbol standard <id>         — switch active standard: app6a | milstd2525d");
            self.push("symbol list                  — list all keys with loaded/missing status");
            self.push("");
            self.push("  Keys: air.fixed_wing  air.rotary  air.fighter  air.bomber  air.tanker");
            self.push("        air.transport  air.unmanned  air.military_fixed_wing  air.generic");
            self.push("        ground.infantry  ground.armor  ground.artillery  ground.mechanized_infantry");
            self.push("        ground.anti_aircraft  ground.engineer  ground.hq  ground.reconnaissance");
            self.push("        ground.special_forces  ground.generic");
            self.push("        naval.surface  naval.submarine  naval.carrier  naval.generic");
            return ConsoleOutput::default();
        }

        let sub = parts.get(1).copied().unwrap_or("");
        match sub {
            "assign" => {
                // parts[2] = "<key> <path>" (execute() uses splitn(3))
                let rest = parts.get(2).copied().unwrap_or("");
                let (key, path) = rest.split_once(' ')
                    .map(|(k, p)| (k.trim(), p.trim()))
                    .unwrap_or((rest, ""));
                if key.is_empty() || path.is_empty() {
                    self.push("usage: symbol assign <key> <path>");
                    ConsoleOutput::default()
                } else {
                    ConsoleOutput {
                        symbol_cmd: Some(SymbolCmd::Assign { key: key.to_string(), path: path.to_string() }),
                        ..Default::default()
                    }
                }
            }
            "clear" => {
                let key = parts.get(2).copied().unwrap_or("").to_string();
                if key.is_empty() {
                    self.push("usage: symbol clear <key>");
                    ConsoleOutput::default()
                } else {
                    ConsoleOutput { symbol_cmd: Some(SymbolCmd::Clear(key)), ..Default::default() }
                }
            }
            "standard" => {
                let id = parts.get(2).copied().unwrap_or("").to_string();
                if id.is_empty() {
                    self.push("usage: symbol standard app6a|milstd2525d");
                    ConsoleOutput::default()
                } else {
                    ConsoleOutput { symbol_cmd: Some(SymbolCmd::SetStandard(id)), ..Default::default() }
                }
            }
            "list" | "" => ConsoleOutput { symbol_cmd: Some(SymbolCmd::List), ..Default::default() },
            _ => {
                self.push("usage: symbol assign|clear|standard|list  (type 'symbol help')");
                ConsoleOutput::default()
            }
        }
    }

    /// Handle `camera <sub> [args…]`
    fn exec_camera(&mut self, parts: Vec<&str>, settings: &mut Settings) -> ConsoleOutput {
        let sub = parts.get(1).copied().unwrap_or("status");
        match sub {
            "none" => {
                settings.set("camera_mode", "none");
                settings.save();
                self.push("camera_mode = none");
            }

            "rotate" => {
                if let Some(speed) = parts.get(2) {
                    let msg = settings.set("rotate_speed", speed);
                    self.push(msg);
                }
                settings.set("camera_mode", "rotate");
                settings.save();
                self.push(format!(
                    "camera_mode = rotate  (speed={} deg/s, elasticity={})",
                    settings.rotate_speed, settings.rotate_elasticity,
                ));
            }

            "follow" => {
                // parts[2] = "<sub> [arg]", e.g. "aircraft a1b2c3" or "location"
                let rest  = parts.get(2).copied().unwrap_or("");
                let mut it = rest.splitn(2, ' ');
                let sub   = it.next().unwrap_or("");
                let arg   = it.next().unwrap_or("").trim();

                match sub {
                    "aircraft" => {
                        if arg.is_empty() {
                            self.push("usage: camera follow aircraft <icao24>");
                            return ConsoleOutput::default();
                        }
                        settings.set("follow_entity", arg);
                        settings.set("camera_mode", "follow");
                        settings.save();
                        self.push(format!(
                            "camera_mode = follow aircraft  (entity={arg}, smoothing={}, distance={})",
                            settings.follow_smoothing, settings.follow_distance,
                        ));
                    }
                    "location" => {
                        settings.set("camera_mode", "follow_location");
                        settings.save();
                        self.push(format!(
                            "camera_mode = follow location  (smoothing={}, distance={})",
                            settings.follow_smoothing, settings.follow_distance,
                        ));
                    }
                    "smoothing" => {
                        if arg.is_empty() {
                            self.push(format!("follow_smoothing = {}", settings.follow_smoothing));
                        } else {
                            let msg = settings.set("follow_smoothing", arg);
                            settings.save();
                            self.push(msg);
                        }
                    }
                    "distance" => {
                        if arg.is_empty() {
                            self.push(format!("follow_distance = {}", settings.follow_distance));
                        } else {
                            let msg = settings.set("follow_distance", arg);
                            settings.save();
                            self.push(msg);
                        }
                    }
                    _ => {
                        self.push("usage: camera follow aircraft <icao24>  |  camera follow location");
                        self.push("       camera follow smoothing <0–1>    |  camera follow distance <r>");
                        self.push("type 'camera help' for details");
                    }
                }
            }

            "overlay" => {
                let rest = parts.get(2).copied().unwrap_or("");
                let mut it = rest.splitn(2, ' ');
                let sub2 = it.next().unwrap_or("");
                let arg2 = it.next().unwrap_or("").trim();
                match sub2 {
                    "" => {
                        self.push(format!("follow_overlay        = {}", settings.get("follow_overlay")));
                        self.push(format!("follow_overlay_fields = {}", settings.get("follow_overlay_fields")));
                        self.push("  available fields: callsign  icao24  type  category");
                        self.push("                    altitude  speed  heading  position  on_ground");
                    }
                    "on" | "off" => {
                        let msg = settings.set("follow_overlay", sub2);
                        settings.save();
                        self.push(msg);
                    }
                    "fields" => {
                        if arg2.is_empty() {
                            self.push(format!("follow_overlay_fields = {}", settings.follow_overlay_fields));
                        } else {
                            let msg = settings.set("follow_overlay_fields", arg2);
                            settings.save();
                            self.push(msg);
                        }
                    }
                    _ => {
                        self.push("usage: camera overlay [on|off|fields <field,...>]");
                        self.push("  e.g.  camera overlay fields callsign,altitude,speed");
                    }
                }
            }

            "status" => {
                let mode  = settings.get("camera_mode");
                let extra = match mode.as_str() {
                    "rotate" => format!(
                        "  rotate_speed={} deg/s  rotate_elasticity={}",
                        settings.rotate_speed, settings.rotate_elasticity
                    ),
                    "follow" => format!(
                        "  target=aircraft  entity={}  smoothing={}  distance={}",
                        settings.follow_entity, settings.follow_smoothing, settings.follow_distance
                    ),
                    "follow_location" => format!(
                        "  target=location  smoothing={}  distance={}",
                        settings.follow_smoothing, settings.follow_distance
                    ),
                    _ => String::new(),
                };
                self.push(format!("camera_mode = {mode}{extra}"));
            }

            other => {
                self.push(format!(
                    "unknown camera sub-command '{other}'  (type 'camera help')"
                ));
            }
        }
        ConsoleOutput::default()
    }
}
