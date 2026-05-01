use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::runtime::Handle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalPosition,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use crate::{
    console::{Console, DiagCmd, GeoCmd, KillTarget, LocationCmd, SpectrumCmd, TopoCmd},
    diag::DiagCollector,
    geo::{geo_to_cartesian, AdminLevel, GeoFeature},
    globe::anim::{CameraAnimator, CameraMode},
    hud::{Hud, HudLocation},
    layers::LayerManager,
    location::LocationManager,
    log_capture::LogBuffer,
    pipeline::{flights, osm, PipelineManager},
    renderer::WgpuState,
    settings::Settings,
    voice::VoiceController,
    world_map::WorldMapManager,
};

pub struct App {
    rt:               Handle,
    window:           Option<Arc<Window>>,
    renderer:         Option<WgpuState>,
    layers:           LayerManager,
    pipeline:         PipelineManager,
    hud:              Hud,
    console:          Console,
    settings:         Settings,
    animator:         CameraAnimator,
    diag:             Option<DiagCollector>,
    /// Watch receiver for the latest flight features (used by Follow mode).
    flights_rx:       Option<tokio::sync::watch::Receiver<Vec<GeoFeature>>>,
    roads_rx:         Option<tokio::sync::watch::Receiver<Vec<GeoFeature>>>,
    rails_rx:         Option<tokio::sync::watch::Receiver<Vec<GeoFeature>>>,
    world_map:        WorldMapManager,
    voice:            Option<VoiceController>,
    location:         LocationManager,
    last_frame:       Instant,
    lmb_down:         bool,
    last_cursor:      PhysicalPosition<f64>,
    /// Cursor position when LMB went down; used to distinguish click from drag.
    lmb_press_pos:    Option<PhysicalPosition<f64>>,
    /// Pending single-click position to process on the next redraw.
    pending_click:    Option<PhysicalPosition<f64>>,
    /// Active touch points by ID; used for 1-finger orbit and 2-finger pinch-zoom.
    touches:          HashMap<u64, PhysicalPosition<f64>>,
}

impl App {
    pub fn new(rt: Handle, log_buffer: LogBuffer) -> Self {
        let settings = Settings::load();
        let mut pipeline = PipelineManager::new(rt.clone());
        pipeline.register(flights::create_source(&settings));
        pipeline.register(Arc::new(osm::roads_source()));
        pipeline.register(Arc::new(osm::railways_source()));
        let flights_rx = pipeline.subscribe("flights");
        let roads_rx   = pipeline.subscribe("roads");
        let rails_rx   = pipeline.subscribe("railways");

        let animator = CameraAnimator::new(settings.to_camera_mode());

        Self {
            rt:            rt.clone(),
            window:        None,
            renderer:      None,
            layers:        LayerManager::default(),
            pipeline,
            hud:           Hud::new(rt.clone()),
            console:       Console::new(log_buffer),
            settings,
            animator,
            diag:          None,
            flights_rx,
            roads_rx,
            rails_rx,
            world_map:     WorldMapManager::new(rt.clone()),
            voice:         None,
            location:      LocationManager::new(),
            last_frame:    Instant::now(),
            lmb_down:      false,
            last_cursor:   PhysicalPosition::new(0.0, 0.0),
            lmb_press_pos: None,
            pending_click: None,
            touches:       HashMap::new(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("rusty_glauxb"))
                .expect("window creation failed"),
        );
        let renderer = pollster::block_on(WgpuState::new(Arc::clone(&window)));
        let voice    = VoiceController::new(self.rt.clone());
        self.diag = Some(DiagCollector::new(renderer.adapter_caps.clone()));
        self.window   = Some(Arc::clone(&window));
        self.renderer = Some(renderer);
        self.voice    = Some(voice);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let consumed = self
            .renderer
            .as_mut()
            .zip(self.window.as_ref())
            .map(|(r, w)| r.egui_handle_event(w, &event))
            .unwrap_or(false);
        if consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size);
                }
            }

            WindowEvent::RedrawRequested => {
                // ── Time delta ───────────────────────────────────────────────
                let now = Instant::now();
                let dt  = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                // ── Voice commands ───────────────────────────────────────────
                if let Some(cmd) = self.voice.as_mut().and_then(|v| v.poll_command()) {
                    self.layers.apply_filter(&cmd.layers);
                    self.hud.set_voice_status(cmd.transcript.clone());
                    if let Some(loc) = cmd.location {
                        tracing::info!(lat = loc.lat, lon = loc.lon, "voice: focus");
                        if let Some(r) = &mut self.renderer {
                            r.camera.focus(loc.lat, loc.lon);
                        }
                    }
                }

                // ── Sync camera mode from settings ───────────────────────────
                let desired = self.settings.to_camera_mode();
                if self.animator.mode != desired {
                    self.animator.set_mode(desired);
                }

                // ── Advance camera animation ─────────────────────────────────
                let follow_pos = match &self.animator.mode {
                    CameraMode::Follow { entity_id, .. } =>
                        find_flight_pos(&self.flights_rx, entity_id),
                    CameraMode::FollowLocation { .. } =>
                        self.location.latest()
                            .map(|f| geo_to_cartesian(f.lat as f32, f.lon as f32)),
                    _ => None,
                };

                if let Some(r) = &mut self.renderer {
                    self.animator.tick(&mut r.camera, dt, follow_pos);
                }

                // ── Data pipeline ────────────────────────────────────────────
                if let Some(r) = &self.renderer {
                    self.pipeline.tick(&r.geo_bounds());
                }

                // ── OSM layer updates ────────────────────────────────────────
                if let Some(r) = &mut self.renderer {
                    if let Some(rx) = &mut self.roads_rx {
                        if rx.has_changed().unwrap_or(false) {
                            let features = rx.borrow_and_update().clone();
                            r.update_roads(&features);
                        }
                    }
                    if let Some(rx) = &mut self.rails_rx {
                        if rx.has_changed().unwrap_or(false) {
                            let features = rx.borrow_and_update().clone();
                            r.update_railways(&features);
                        }
                    }
                }

                // ── World-map layer updates ───────────────────────────────────
                {
                    let country_features =
                        self.world_map.countries_rx.has_changed().unwrap_or(false)
                        .then(|| self.world_map.countries_rx.borrow_and_update().clone());

                    let state_features = self.world_map.states_rx.as_mut().and_then(|rx| {
                        rx.has_changed().unwrap_or(false)
                            .then(|| rx.borrow_and_update().clone())
                    });

                    if let Some(r) = &mut self.renderer {
                        if let Some(features) = state_features {
                            r.update_world_map(features);
                        } else if let Some(features) = country_features {
                            if matches!(self.world_map.level, AdminLevel::Country) {
                                r.update_world_map(features);
                            }
                        }
                    }
                }

                // ── Globe click → drill-down ──────────────────────────────────
                if let Some(pos) = self.pending_click.take() {
                    if let Some(r) = &mut self.renderer {
                        if let Some((lat, lon)) = r.pick_world_pos(pos.x as f32, pos.y as f32) {
                            if let Some(feature) = r.world_map.pick(lat, lon).cloned() {
                                match &self.world_map.level {
                                    AdminLevel::Country => {
                                        self.world_map.load_states(&feature.id);
                                        self.world_map.level = AdminLevel::State(feature.id.clone());
                                        r.world_map.selected = Some(feature.id.clone());
                                        r.rebuild_selected_border();
                                        tracing::info!(country = %feature.name, "drill-down to states");
                                    }
                                    AdminLevel::State(_) => {
                                        r.world_map.selected = Some(feature.id.clone());
                                        r.rebuild_selected_border();
                                        tracing::info!(state = %feature.name, "selected state");
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Render ───────────────────────────────────────────────────
                let current_flights: Vec<GeoFeature> = self.flights_rx
                    .as_ref()
                    .map(|rx| rx.borrow().clone())
                    .unwrap_or_default();

                // ── Location dot projection ──────────────────────────────────
                if let Some(r) = &self.renderer {
                    self.hud.location = self.location.latest().and_then(|f| {
                        r.project_geo(f.lat as f32, f.lon as f32)
                            .map(|p| HudLocation { screen_pos: p, hdg: f.hdg as f32 })
                    });
                }

                if let Some(diag) = &mut self.diag {
                    diag.begin_frame();
                }

                if let Some(r) = &mut self.renderer {
                    let output = r.render(
                        &mut self.hud,
                        &self.layers,
                        &mut self.console,
                        &mut self.settings,
                        &current_flights,
                    );

                    // ── Feed DiagCollector ────────────────────────────────────
                    if let Some(diag) = &mut self.diag {
                        diag.record(output.gpu_ms, output.draw_calls, output.tri_count);
                        if diag.overlay {
                            if let Some(line) = diag.hud_line() {
                                self.hud.set_diag_line(line);
                            }
                        }
                    }

                    let output = output.console;
                    if let Some(stream) = output.set_source {
                        match stream.as_str() {
                            "flights" => {
                                let new_src = flights::create_source(&self.settings);
                                self.pipeline.replace_source(new_src);
                                tracing::info!(
                                    source = %self.settings.flight_source,
                                    "flight source hot-swapped"
                                );
                            }
                            other => tracing::warn!(stream = other, "set_source: unknown datastream"),
                        }
                    }
                    if let Some(layer) = output.refresh_layer {
                        self.pipeline.force_refresh(&layer);
                        tracing::info!(layer = %layer, "manual refresh requested");
                    }
                    if let Some((name, visible)) = output.set_layer_visible {
                        match name.as_str() {
                            "roads"    => r.roads.visible    = visible,
                            "railways" => r.railways.visible = visible,
                            _          => tracing::warn!(layer = %name, "unknown layer for visibility toggle"),
                        }
                    }
                    if let Some(cmd) = output.topo_cmd {
                        match cmd {
                            TopoCmd::Load { name, path }      => r.load_topo(&name, &path),
                            TopoCmd::SetVisible { name, visible } => r.set_topo_visible(&name, visible),
                            TopoCmd::Drop(name)               => r.drop_topo(&name),
                        }
                    }
                    if let Some(cmd) = output.spectrum_cmd {
                        match cmd {
                            SpectrumCmd::SetVisible(v) => self.hud.spectrum.visible = v,
                            SpectrumCmd::SetSource(s) => {
                                if !self.hud.spectrum.set_source(&s) {
                                    tracing::warn!(source = %s, "unknown spectrum source");
                                }
                            }
                        }
                    }
                    if let Some(cmd) = output.diag_cmd {
                        if let Some(diag) = &mut self.diag {
                            match cmd {
                                DiagCmd::PrintTable => {
                                    for line in diag.format_table() {
                                        self.console.push_line(line);
                                    }
                                }
                                DiagCmd::Reset => {
                                    diag.acc.reset();
                                    self.console.push_line("diag: accumulator reset");
                                }
                                DiagCmd::SetOverlay(v) => {
                                    diag.overlay = v;
                                    self.console.push_line(format!(
                                        "diag overlay: {}",
                                        if v { "on" } else { "off" }
                                    ));
                                }
                            }
                        }
                    }
                    if let Some(cmd) = output.location_cmd {
                        match cmd {
                            LocationCmd::SetSource(name) => {
                                if let Err(e) = self.location.set_source(&name, &self.rt) {
                                    self.console.push_line(e);
                                }
                            }
                            LocationCmd::Connect(url) => {
                                self.location.network_url = url;
                                if let Err(e) = self.location.set_source("network", &self.rt) {
                                    self.console.push_line(e);
                                }
                            }
                            LocationCmd::SetFollow(v) => {
                                self.location.follow = v;
                                self.console.push_line(format!(
                                    "location follow: {}", if v { "on" } else { "off" }
                                ));
                            }
                            LocationCmd::Status => {
                                match self.location.latest() {
                                    Some(f) => {
                                        self.console.push_line(format!(
                                            "source  {}", self.location.source_name
                                        ));
                                        self.console.push_line(format!(
                                            "lat     {:.6}°", f.lat
                                        ));
                                        self.console.push_line(format!(
                                            "lon     {:.6}°", f.lon
                                        ));
                                        self.console.push_line(format!(
                                            "alt     {:.0} m", f.alt
                                        ));
                                        self.console.push_line(format!(
                                            "hdg     {:.1}°  spd {:.1} m/s", f.hdg, f.spd
                                        ));
                                        self.console.push_line(format!(
                                            "acc     ±{:.0} m", f.acc
                                        ));
                                    }
                                    None => self.console.push_line(format!(
                                        "location: source={} — no fix",
                                        self.location.source_name
                                    )),
                                }
                            }
                            LocationCmd::Stop => self.location.stop(),
                        }
                    }
                    if let Some(target) = output.kill_target {
                        match target {
                            KillTarget::All          => self.pipeline.stop(None),
                            KillTarget::Stream(name) => self.pipeline.stop(Some(&name)),
                        }
                    }
                    if output.quit {
                        event_loop.exit();
                    }
                    if let Some(geo_cmd) = output.geo_cmd {
                        match geo_cmd {
                            GeoCmd::SetNames(v)   => r.world_map.show_names   = v,
                            GeoCmd::SetFill(v)    => r.world_map.show_fill    = v,
                            GeoCmd::SetBorders(v) => r.world_map.show_borders = v,
                            GeoCmd::Back => {
                                if self.world_map.go_back() {
                                    r.world_map.selected = None;
                                    let features = self.world_map.countries_rx.borrow().clone();
                                    r.update_world_map(features);
                                }
                            }
                            GeoCmd::Status => {
                                self.console.push_line(r.world_map.status_string());
                            }
                        }
                    }
                }
            }

            // ── Mouse orbit ──────────────────────────────────────────────────
            WindowEvent::MouseInput { button: MouseButton::Left, state, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.lmb_down = true;
                        self.lmb_press_pos = Some(self.last_cursor);
                        self.animator.paused = true;
                    }
                    ElementState::Released => {
                        self.lmb_down = false;
                        self.animator.paused = false;
                        // Register as a click only if the cursor barely moved.
                        if let Some(press) = self.lmb_press_pos.take() {
                            let dx = (self.last_cursor.x - press.x).abs();
                            let dy = (self.last_cursor.y - press.y).abs();
                            if dx < 5.0 && dy < 5.0 {
                                self.pending_click = Some(self.last_cursor);
                            }
                        }
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if self.lmb_down {
                    if let (Some(r), Some(w)) = (&mut self.renderer, &self.window) {
                        let size = w.inner_size();
                        let dx = (position.x - self.last_cursor.x) as f32 / size.width as f32;
                        let dy = (position.y - self.last_cursor.y) as f32 / size.height as f32;
                        r.orbit(dx, dy);
                    }
                }
                self.last_cursor = position;
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y)  => y,
                    MouseScrollDelta::PixelDelta(p)    => p.y as f32 / 50.0,
                };
                if let Some(r) = &mut self.renderer {
                    r.zoom(amount);
                    // In follow modes, sync distance in both the active mode variant and
                    // settings, so the per-frame settings→mode sync (lines below) sees a
                    // match and doesn't reset the distance back every frame.
                    match &mut self.animator.mode {
                        CameraMode::Follow { distance, .. }
                        | CameraMode::FollowLocation { distance, .. } => {
                            *distance = r.camera.radius;
                            self.settings.follow_distance = r.camera.radius;
                        }
                        _ => {}
                    }
                }
            }

            // ── Touch (Android) ──────────────────────────────────────────────────
            WindowEvent::Touch(touch) => {
                match touch.phase {
                    TouchPhase::Started => {
                        self.touches.insert(touch.id, touch.location);
                    }
                    TouchPhase::Moved => {
                        let prev = match self.touches.get(&touch.id).copied() {
                            Some(p) => p,
                            None => {
                                self.touches.insert(touch.id, touch.location);
                                return;
                            }
                        };
                        if self.touches.len() == 1 {
                            if let (Some(r), Some(w)) = (&mut self.renderer, &self.window) {
                                let size = w.inner_size();
                                let dx = (touch.location.x - prev.x) as f32 / size.width as f32;
                                let dy = (touch.location.y - prev.y) as f32 / size.height as f32;
                                r.orbit(dx, dy);
                            }
                        } else {
                            // Snapshot other finger position before mutably borrowing renderer.
                            let other = self.touches.iter()
                                .find(|(&id, _)| id != touch.id)
                                .map(|(_, &p)| p);
                            if let Some(other_pos) = other {
                                let old_dist = touch_dist(prev, other_pos) as f32;
                                let new_dist = touch_dist(touch.location, other_pos) as f32;
                                if old_dist > 1.0 {
                                    let delta = (new_dist / old_dist - 1.0) * 5.0;
                                    if let Some(r) = &mut self.renderer {
                                        r.zoom(delta);
                                    }
                                }
                            }
                        }
                        self.touches.insert(touch.id, touch.location);
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.touches.remove(&touch.id);
                    }
                }
            }

            // ── Keyboard ─────────────────────────────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key, state, repeat: false, .. },
                ..
            } => match physical_key {
                PhysicalKey::Code(KeyCode::Backquote) if state == ElementState::Pressed => {
                    self.console.toggle();
                }
                PhysicalKey::Code(KeyCode::Space) => {
                    if let Some(v) = &mut self.voice {
                        match state {
                            ElementState::Pressed  => v.start_recording(),
                            ElementState::Released => v.stop_and_process(),
                        }
                    }
                }
                _ => {}
            },

            _ => {}
        }
    }
}

fn touch_dist(a: PhysicalPosition<f64>, b: PhysicalPosition<f64>) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Find the unit-sphere position of the first flight whose ICAO24 matches `id`.
fn find_flight_pos(
    rx: &Option<tokio::sync::watch::Receiver<Vec<GeoFeature>>>,
    id: &str,
) -> Option<glam::Vec3> {
    let rx = rx.as_ref()?;
    let features = rx.borrow();
    let id_lower = id.to_lowercase();
    features.iter().find(|f| {
        f.meta.get("icao24")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.to_lowercase() == id_lower)
    })
    .map(|f| f.world_pos())
}
