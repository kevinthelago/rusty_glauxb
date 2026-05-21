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
    console::{AircraftCmd, Console, DiagCmd, DirectionsCmd, DownloadCmd, GeoCmd, KillTarget, LocationCmd, MilsymCmd, SymbolCmd, TopoCmd},
    directions::{self, DirectionsResult, DirectionsSource},
    download,
    diag::DiagCollector,
    geo::{geo_to_cartesian, AdminLevel, GeoFeature},
    globe::anim::{CameraAnimator, CameraMode},
    hud::{AircraftLabel, FollowData, Hud, HudLocation, parse_overlay_fields},
    layers::LayerManager,
    location::LocationManager,
    log_capture::LogBuffer,
    natural_earth::NaturalEarthManager,
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
    natural_earth:    NaturalEarthManager,
    world_map:        WorldMapManager,
    /// Completed (or failed) downloads sent from background tasks.
    download_rx: tokio::sync::mpsc::UnboundedReceiver<(String, anyhow::Result<Vec<GeoFeature>>)>,
    download_tx: tokio::sync::mpsc::UnboundedSender<(String, anyhow::Result<Vec<GeoFeature>>)>,
    /// Layers that should be reloaded on the next session.
    active_downloads: Vec<download::SavedLayer>,
    /// Completed directions fetches from background tasks.
    directions_rx: tokio::sync::mpsc::UnboundedReceiver<anyhow::Result<DirectionsResult>>,
    directions_tx: tokio::sync::mpsc::UnboundedSender<anyhow::Result<DirectionsResult>>,
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
    /// The `settings.camera_mode` string at the moment the user started dragging.
    /// `Some(...)` means we're in a drag-interrupted state and the settings→animator
    /// sync is suspended.  Cleared when the idle timer fires or the user issues an
    /// explicit camera command.
    cam_interrupt_save: Option<String>,
    /// When the user last interacted with the camera (drag / scroll / touch).
    last_interaction:   Instant,
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
        let (download_tx, download_rx) = tokio::sync::mpsc::unbounded_channel();
        let (directions_tx, directions_rx) = tokio::sync::mpsc::unbounded_channel();

        // Re-load any layers the user had active in the previous session.
        let active_downloads = download::load_state();
        for layer in &active_downloads {
            let url = layer.url.clone()
                .or_else(|| download::find(&layer.name).map(|e| e.url.to_string()))
                .unwrap_or_default();
            if !url.is_empty() {
                let tx    = download_tx.clone();
                let name  = layer.name.clone();
                let secs  = layer.cache_secs;
                rt.spawn(async move {
                    let result = download::fetch(&name, &url, secs).await;
                    let _ = tx.send((name, result));
                });
            }
        }

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
            natural_earth: NaturalEarthManager::new(rt.clone()),
            world_map:     WorldMapManager::new(rt.clone()),
            download_rx,
            download_tx,
            active_downloads,
            directions_rx,
            directions_tx,
            voice:         None,
            location:      LocationManager::new(),
            last_frame:    Instant::now(),
            lmb_down:      false,
            last_cursor:   PhysicalPosition::new(0.0, 0.0),
            lmb_press_pos:      None,
            pending_click:      None,
            touches:            HashMap::new(),
            cam_interrupt_save: None,
            last_interaction:   Instant::now(),
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
        // Console toggle must be intercepted BEFORE egui so it fires even when
        // the text input has keyboard focus (egui would otherwise consume it).
        if let WindowEvent::KeyboardInput {
            event: KeyEvent {
                physical_key: PhysicalKey::Code(KeyCode::Backquote),
                state: ElementState::Pressed,
                repeat: false,
                ..
            },
            ..
        } = &event
        {
            self.console.toggle();
            return;
        }

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

                // ── Drag-interrupt idle resume ────────────────────────────────
                if let Some(ref saved) = self.cam_interrupt_save.clone() {
                    if self.settings.camera_mode != *saved {
                        // User explicitly changed the mode via console → accept it.
                        self.cam_interrupt_save = None;
                    } else {
                        let idle_s = self.settings.idle_resume_secs;
                        if idle_s > 0.0
                            && self.last_interaction.elapsed().as_secs_f32() >= idle_s
                        {
                            // Timer fired.  Aircraft follow is deliberately not resumed
                            // (user panned away from a target intentionally).
                            // Location follow IS resumed — the user just wanted a quick
                            // look around before snapping back to their position.
                            if saved == "follow" {
                                self.settings.camera_mode = "none".to_string();
                            }
                            // For "follow_location" and "rotate", just clear the interrupt
                            // and let the per-frame sync restore the saved camera_mode.
                            self.cam_interrupt_save = None;
                        }
                    }
                }

                // ── Sync camera mode from settings ────────────────────────────
                // Skipped while a drag interruption is in effect so the animator
                // stays in None mode until the idle timer fires.
                if self.cam_interrupt_save.is_none() {
                    let desired = self.settings.to_camera_mode();
                    if self.animator.mode != desired {
                        self.animator.set_mode(desired);
                    }
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

                // ── Natural Earth layer updates ───────────────────────────────
                if let Some(r) = &mut self.renderer {
                    if self.natural_earth.rivers_rx.has_changed().unwrap_or(false) {
                        let features = self.natural_earth.rivers_rx.borrow_and_update().clone();
                        r.update_rivers(&features);
                    }
                    if self.natural_earth.lakes_rx.has_changed().unwrap_or(false) {
                        let features = self.natural_earth.lakes_rx.borrow_and_update().clone();
                        r.update_lakes(&features);
                    }
                    if self.natural_earth.railroads_rx.has_changed().unwrap_or(false) {
                        let features = self.natural_earth.railroads_rx.borrow_and_update().clone();
                        r.update_railroads(&features);
                    }
                }

                // ── Completed direction fetches ───────────────────────────────
                while let Ok(result) = self.directions_rx.try_recv() {
                    match result {
                        Ok(res) => {
                            if let Some(r) = &mut self.renderer {
                                r.load_topo_data("directions", res.features);
                            }
                            self.console.push_line(format!("route ready — {}", res.summary));
                        }
                        Err(e) => {
                            self.console.push_line(format!("directions error: {e:#}"));
                        }
                    }
                }

                // ── Completed downloads ───────────────────────────────────────
                while let Ok((name, result)) = self.download_rx.try_recv() {
                    match result {
                        Ok(features) => {
                            if let Some(r) = &mut self.renderer {
                                r.load_topo_data(&name, features);
                            }
                            self.console.push_line(format!("download: '{name}' ready"));
                        }
                        Err(e) => {
                            tracing::error!(layer = %name, "download failed: {e:#}");
                            self.console.push_line(format!("download error '{name}': {e}"));
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

                // ── Follow overlay data ───────────────────────────────────────
                self.hud.follow_overlay_visible = self.settings.follow_overlay;
                self.hud.follow_overlay_fields  = parse_overlay_fields(&self.settings.follow_overlay_fields);
                {
                    let mode = self.animator.mode.clone();
                    self.hud.follow_data = match &mode {
                        CameraMode::Follow { entity_id, .. } => {
                            current_flights.iter()
                                .find(|f| {
                                    f.meta.get("icao24")
                                        .and_then(|v| v.as_str())
                                        == Some(entity_id.as_str())
                                })
                                .map(|f| FollowData::Aircraft {
                                    icao24: entity_id.clone(),
                                    callsign: f.meta.get("callsign")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        .map(String::from),
                                    aircraft_type: f.meta.get("type")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        .map(String::from),
                                    category: f.meta.get("aircraft_category")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("misc")
                                        .to_string(),
                                    altitude_ft: f.meta.get("baro_alt_ft")
                                        .and_then(|v| v.as_f64())
                                        .map(|v| v as f32),
                                    speed_kts: f.meta.get("velocity_kts")
                                        .and_then(|v| v.as_f64())
                                        .map(|v| v as f32),
                                    heading: f.meta.get("heading")
                                        .and_then(|v| v.as_f64())
                                        .map(|v| v as f32),
                                    on_ground: f.meta.get("on_ground")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false),
                                    lat: f.lat,
                                    lon: f.lon,
                                })
                        }
                        CameraMode::FollowLocation { .. } => {
                            self.location.latest().map(|loc| FollowData::Location {
                                lat:         loc.lat,
                                lon:         loc.lon,
                                alt_m:       loc.alt,
                                speed_mps:   loc.spd,
                                heading_deg: loc.hdg,
                            })
                        }
                        _ => None,
                    };
                }

                if let Some(diag) = &mut self.diag {
                    diag.begin_frame();
                }

                if let Some(r) = &mut self.renderer {
                    r.update_visibility();

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

                    // ── Aircraft click-to-follow ──────────────────────────────
                    if let Some(icao24) = output.follow_aircraft {
                        let already_following = matches!(
                            &self.animator.mode,
                            CameraMode::Follow { entity_id, .. } if entity_id == &icao24
                        );
                        if already_following {
                            self.settings.camera_mode = "none".to_string();
                            self.settings.save();
                        } else {
                            self.settings.set("follow_entity", &icao24);
                            self.settings.set("camera_mode", "follow");
                            self.settings.save();
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
                        // Download layers: bust cache and re-fetch.
                        // Pipeline layers: forward to the scheduler as before.
                        if let Some(saved) = self.active_downloads.iter().find(|s| s.name == layer) {
                            let url = saved.url.clone()
                                .or_else(|| download::find(&layer).map(|e| e.url.to_string()))
                                .unwrap_or_default();
                            if !url.is_empty() {
                                download::clear_cache(&layer);
                                let tx    = self.download_tx.clone();
                                let name  = layer.clone();
                                let secs  = saved.cache_secs;
                                self.rt.spawn(async move {
                                    let result = download::fetch(&name, &url, secs).await;
                                    let _ = tx.send((name, result));
                                });
                                tracing::info!(layer = %layer, "download layer refresh triggered");
                            }
                        } else {
                            self.pipeline.force_refresh(&layer);
                            tracing::info!(layer = %layer, "manual refresh requested");
                        }
                    }
                    if let Some((name, visible)) = output.set_layer_visible {
                        match name.as_str() {
                            "roads"     => r.roads.visible     = visible,
                            "railways"  => r.railways.visible  = visible,
                            "rivers"    => r.rivers.visible    = visible,
                            "lakes"     => r.lakes.visible     = visible,
                            "railroads" => r.railroads.visible = visible,
                            _           => tracing::warn!(layer = %name, "unknown layer for visibility toggle"),
                        }
                    }
                    if let Some(cmd) = output.topo_cmd {
                        match cmd {
                            TopoCmd::Load { name, path }      => r.load_topo(&name, &path),
                            TopoCmd::SetVisible { name, visible } => r.set_topo_visible(&name, visible),
                            TopoCmd::Drop(name)               => r.drop_topo(&name),
                        }
                    }
                    if let Some(cmd) = output.download_cmd {
                        match cmd {
                            DownloadCmd::Fetch { name, url, cache_secs } => {
                                // Persist: store None for catalog entries, Some(url) for custom.
                                let stored_url = if download::find(&name).is_some() {
                                    None
                                } else {
                                    Some(url.clone()).filter(|u| !u.is_empty())
                                };
                                if let Some(existing) = self.active_downloads.iter_mut().find(|s| s.name == name) {
                                    existing.url        = stored_url;
                                    existing.cache_secs = cache_secs;
                                } else {
                                    self.active_downloads.push(download::SavedLayer {
                                        name:       name.clone(),
                                        url:        stored_url,
                                        cache_secs,
                                    });
                                }
                                download::save_state(&self.active_downloads);

                                let tx     = self.download_tx.clone();
                                let name_c = name.clone();
                                self.rt.spawn(async move {
                                    let result = download::fetch(&name_c, &url, cache_secs).await;
                                    let _ = tx.send((name_c, result));
                                });
                            }
                            DownloadCmd::Clear(name) => {
                                download::clear_cache(&name);
                                r.drop_topo(&name);
                                self.active_downloads.retain(|s| s.name != name);
                                download::save_state(&self.active_downloads);
                            }
                        }
                    }
                    if let Some(cmd) = output.directions_cmd {
                        match cmd {
                            DirectionsCmd::Fetch { from, to } => {
                                let source = self.settings.directions_source
                                    .parse::<DirectionsSource>()
                                    .unwrap_or_default();
                                let api_key = self.settings.google_maps_api_key.clone();
                                let tx = self.directions_tx.clone();
                                self.rt.spawn(async move {
                                    let result = directions::fetch(
                                        &from, &to, &source, api_key.as_deref(),
                                    ).await;
                                    let _ = tx.send(result);
                                });
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
                                if v {
                                    self.settings.set("camera_mode", "follow_location");
                                    if self.location.source_name == "off" {
                                        self.console.push_line(
                                            "location follow: on  (no source running — try: source location mock)"
                                                .to_string()
                                        );
                                    } else {
                                        self.console.push_line("location follow: on".to_string());
                                    }
                                } else {
                                    if self.settings.camera_mode == "follow_location" {
                                        self.settings.set("camera_mode", "none");
                                    }
                                    self.console.push_line("location follow: off".to_string());
                                }
                                self.settings.save();
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
                    if let Some(cmd) = output.aircraft_cmd {
                        match cmd {
                            AircraftCmd::SetLabel(field) => {
                                self.hud.label = match field.as_str() {
                                    "callsign" => AircraftLabel::Callsign,
                                    "type"     => AircraftLabel::Type,
                                    "altitude" => AircraftLabel::Altitude,
                                    "speed"    => AircraftLabel::Speed,
                                    "icao24"   => AircraftLabel::Icao24,
                                    _          => AircraftLabel::None,
                                };
                            }
                            AircraftCmd::SetFollowOnClick(v) => {
                                self.hud.follow_on_click = v;
                            }
                        }
                    }
                    if let Some(cmd) = output.milsym_cmd {
                        match cmd {
                            MilsymCmd::SetEnabled(v) => {
                                self.settings.milsym_enabled = v;
                                self.settings.save();
                                self.console.push_line(format!(
                                    "milsym: {}", if v { "on" } else { "off" }
                                ));
                            }
                            MilsymCmd::SetScale(v) => {
                                self.settings.milsym_scale = v;
                                self.settings.save();
                                self.console.push_line(format!("milsym scale: {v}"));
                            }
                            MilsymCmd::SetEchelon(v) => {
                                self.settings.milsym_show_echelon = v;
                                self.settings.save();
                                self.console.push_line(format!(
                                    "milsym echelon: {}", if v { "on" } else { "off" }
                                ));
                            }
                            MilsymCmd::SetAffiliation { entity, affiliation } => {
                                let msg = match entity.as_str() {
                                    "civil"    => {
                                        self.settings.milsym_civil_affiliation = affiliation.clone();
                                        format!("milsym civil → {affiliation}")
                                    }
                                    "military" => {
                                        self.settings.milsym_military_affiliation = affiliation.clone();
                                        format!("milsym military → {affiliation}")
                                    }
                                    _ => {
                                        self.settings.milsym_unknown_affiliation = affiliation.clone();
                                        format!("milsym unknown → {affiliation}")
                                    }
                                };
                                self.settings.save();
                                self.console.push_line(msg);
                            }
                            MilsymCmd::Status => {
                                let s = &self.settings;
                                self.console.push_line(format!(
                                    "enabled:  {}  scale: {}  echelon: {}",
                                    if s.milsym_enabled { "on" } else { "off" },
                                    s.milsym_scale,
                                    if s.milsym_show_echelon { "on" } else { "off" },
                                ));
                                self.console.push_line(format!(
                                    "civil→{}  military→{}  unknown→{}",
                                    s.milsym_civil_affiliation,
                                    s.milsym_military_affiliation,
                                    s.milsym_unknown_affiliation,
                                ));
                            }
                        }
                    }
                    if let Some(cmd) = output.symbol_cmd {
                        use crate::milsym::standard::SymbologyStandard;
                        match cmd {
                            SymbolCmd::Assign { key, path } => {
                                match std::fs::read(&path) {
                                    Ok(bytes) => {
                                        self.hud.symbol_map.assign(&key, bytes);
                                        self.console.push_line(format!("symbol: assigned '{}' <- {}", key, path));
                                    }
                                    Err(e) => self.console.push_line(format!("symbol: error reading {path}: {e}")),
                                }
                            }
                            SymbolCmd::Clear(key) => {
                                self.hud.symbol_map.clear(&key);
                                self.console.push_line(format!("symbol: cleared '{key}'"));
                            }
                            SymbolCmd::SetStandard(id) => {
                                match SymbologyStandard::from_id(&id) {
                                    Some(std) => {
                                        self.hud.symbol_map.set_standard(std);
                                        self.console.push_line(format!(
                                            "symbol: standard -> {}",
                                            self.hud.symbol_map.standard.display_name()
                                        ));
                                    }
                                    None => self.console.push_line(format!("symbol: unknown standard '{id}' (try: app6a, milstd2525d)")),
                                }
                            }
                            SymbolCmd::List => {
                                let entries = self.hud.symbol_map.list_entries();
                                let total   = entries.len();
                                let loaded  = entries.iter().filter(|(_, ok)| *ok).count();
                                self.console.push_line(format!(
                                    "standard: {}  ({}/{} icons loaded)",
                                    self.hud.symbol_map.standard.display_name(), loaded, total
                                ));
                                for (key, has_svg) in entries {
                                    self.console.push_line(format!("  {}  {}", if has_svg { "+" } else { "." }, key));
                                }
                            }
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
                            GeoCmd::SetMesh(v)    => r.globe_mesh_visible     = v,
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

                        // First actual pixel of drag: interrupt any active animation.
                        if self.cam_interrupt_save.is_none()
                            && !matches!(self.animator.mode, CameraMode::None)
                        {
                            self.cam_interrupt_save =
                                Some(self.settings.camera_mode.clone());
                            self.animator.set_mode(CameraMode::None);
                        }
                    }
                    self.last_interaction = Instant::now();
                }
                self.last_cursor = position;
            }

            WindowEvent::MouseWheel { delta, .. } => {
                self.last_interaction = Instant::now();
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
                                if self.cam_interrupt_save.is_none()
                                    && !matches!(self.animator.mode, CameraMode::None)
                                {
                                    self.cam_interrupt_save =
                                        Some(self.settings.camera_mode.clone());
                                    self.animator.set_mode(CameraMode::None);
                                }
                            }
                            self.last_interaction = Instant::now();
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
