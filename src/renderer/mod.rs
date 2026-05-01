pub mod aircraft;
pub mod headless;
pub mod lines;
pub mod polygons;

use std::sync::Arc;
use glam::{Mat4, Vec4Swizzles};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, event::WindowEvent, window::Window};

use crate::{
    console::{Console, ConsoleOutput},
    culling::Frustum,
    diag::{AdapterCaps, GpuTimer},
    geo::{cartesian_to_geo, GeoBounds, GeoFeature},
    globe::{camera::ArcballCamera, CubeSphere, GlobeVertex},
    hud::Hud,
    layers::LayerManager,
    settings::Settings,
};

// ─── Render output ────────────────────────────────────────────────────────────

/// Return value of `WgpuState::render()`.  Bundles the console side-effects
/// with per-frame diagnostic counters consumed by `DiagCollector`.
#[derive(Default)]
pub struct RenderOutput {
    pub console:    ConsoleOutput,
    pub gpu_ms:     Option<f64>,
    pub draw_calls: u32,
    pub tri_count:  u32,
}

use aircraft::AircraftRenderer;
use lines::{draw_layer, LineLayer, LinePipeline};
use polygons::{draw_world_map, WorldMapLayer, WorldMapPipeline};

const GLOBE_SUBDIVISIONS: u32 = 64;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

// ─── Uniforms ────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobeUniforms {
    view_proj: [[f32; 4]; 4], // 64 bytes
    model:     [[f32; 4]; 4], // 64 bytes
}

// ─── Globe render pipeline ────────────────────────────────────────────────────

struct GlobePipeline {
    pipeline:    wgpu::RenderPipeline,
    vertex_buf:  wgpu::Buffer,
    index_buf:   wgpu::Buffer,
    index_count: u32,
    uniform_buf: wgpu::Buffer,
    bind_group:  wgpu::BindGroup,
}

impl GlobePipeline {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let mesh = CubeSphere::new(GLOBE_SUBDIVISIONS);

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("globe:verts"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage:    wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("globe:idx"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage:    wgpu::BufferUsages::INDEX,
        });
        let index_count = mesh.indices.len() as u32;

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("globe:uniforms"),
            size:               std::mem::size_of::<GlobeUniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("globe:bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("globe:bg"),
            layout:  &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("globe:shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/globe.wgsl").into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("globe:layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("globe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:               &shader,
                entry_point:          "vs_main",
                buffers:              &[GlobeVertex::layout()],
                compilation_options:  Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         "fs_main",
                targets:             &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:     wgpu::PrimitiveTopology::TriangleList,
                front_face:   wgpu::FrontFace::Ccw,
                cull_mode:    Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Line,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:               DEPTH_FORMAT,
                depth_write_enabled:  true,
                depth_compare:        wgpu::CompareFunction::Less,
                stencil:              wgpu::StencilState::default(),
                bias:                 wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        Self { pipeline, vertex_buf, index_buf, index_count, uniform_buf, bind_group }
    }

    fn update_uniforms(&self, queue: &wgpu::Queue, view_proj: Mat4, model: Mat4) {
        let data = GlobeUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            model:     model.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&data));
    }
}

// ─── WgpuState ───────────────────────────────────────────────────────────────

pub struct WgpuState {
    window:       Arc<Window>,
    surface:      wgpu::Surface<'static>,
    device:       wgpu::Device,
    queue:        wgpu::Queue,
    config:       wgpu::SurfaceConfiguration,
    depth_view:   wgpu::TextureView,
    globe:        GlobePipeline,
    pub camera:   ArcballCamera,
    aircraft:     AircraftRenderer,
    /// Shared compiled line pipeline (one shader for all line layers).
    line_pipeline:      LinePipeline,
    /// Each `LineLayer` is independently toggleable via `.visible`.
    pub roads:          LineLayer,
    pub railways:       LineLayer,
    /// Dynamically loaded GeoJSON topology layers (one per `topo load` command).
    pub topo_layers:    Vec<LineLayer>,
    /// World-map polygon pipeline + per-level layers.
    world_map_pipeline: WorldMapPipeline,
    pub world_map:      WorldMapLayer,
    egui_ctx:     egui::Context,
    egui_winit:   egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    /// GPU timestamp queries; None when TIMESTAMP_QUERY is absent on adapter.
    gpu_timer:    Option<GpuTimer>,
    /// Adapter capabilities exposed to DiagCollector.
    pub adapter_caps: AdapterCaps,
}

impl WgpuState {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("surface creation failed");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter");

        let mut adapter_caps = AdapterCaps::from_adapter(&adapter);

        // Request only features the adapter actually supports.
        let mut wanted = wgpu::Features::empty();
        if adapter_caps.has_polygon_mode_line {
            wanted |= wgpu::Features::POLYGON_MODE_LINE;
        }
        if adapter_caps.has_timestamp_query {
            wanted |= wgpu::Features::TIMESTAMP_QUERY;
        }

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label:             Some("main"),
                    required_features: wanted,
                    required_limits:   wgpu::Limits::default(),
                    memory_hints:      wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .expect("device creation failed");

        adapter_caps.apply_queue(&queue);
        let gpu_timer = adapter_caps.has_timestamp_query
            .then(|| GpuTimer::new(&device, adapter_caps.timestamp_period_ns));

        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                        surface_format,
            width:                         size.width.max(1),
            height:                        size.height.max(1),
            present_mode:                  wgpu::PresentMode::AutoVsync,
            alpha_mode:                    caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view        = make_depth_view(&device, config.width, config.height);
        let globe             = GlobePipeline::new(&device, surface_format);
        let aircraft          = AircraftRenderer::new(&device, surface_format);
        let line_pipeline     = LinePipeline::new(&device, surface_format);
        let roads    = LineLayer::new(&device, &line_pipeline.bgl, "roads",    [1.00, 0.85, 0.30, 0.85]);
        let railways = LineLayer::new(&device, &line_pipeline.bgl, "railways", [0.45, 0.80, 1.00, 0.85]);
        let world_map_pipeline = WorldMapPipeline::new(&device, surface_format);
        let world_map = WorldMapLayer::new(
            &device,
            &world_map_pipeline.bgl,
            [0.10, 0.40, 0.15, 0.80], // fill: medium green, mostly opaque
            [1.00, 0.95, 0.40, 1.00], // border: bright yellow — high contrast vs cyan wireframe
            [1.00, 0.40, 0.10, 1.00], // selected: bright orange-red
        );

        let egui_ctx = egui::Context::default();
        let egui_winit = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1, false);

        let mut camera = ArcballCamera::default();
        camera.aspect = size.width as f32 / size.height as f32;

        Self {
            window,
            surface,
            device,
            queue,
            config,
            depth_view,
            globe,
            camera,
            aircraft,
            line_pipeline,
            roads,
            railways,
            topo_layers: Vec::new(),
            world_map_pipeline,
            world_map,
            egui_ctx,
            egui_winit,
            egui_renderer,
            gpu_timer,
            adapter_caps,
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width  = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = make_depth_view(&self.device, size.width, size.height);
        self.camera.aspect = size.width as f32 / size.height as f32;
    }

    pub fn egui_handle_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.egui_winit.on_window_event(window, event).consumed
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) { self.camera.orbit(dx, dy); }
    pub fn zoom(&mut self, delta: f32)         { self.camera.zoom(delta);  }

    /// Geographic bounding box of the currently visible globe region.
    /// Feed this into [`crate::pipeline::PipelineManager::tick`] each frame.
    pub fn geo_bounds(&self) -> GeoBounds {
        GeoBounds::from_eye(self.camera.eye())
    }

    /// View-frustum for the current frame.  Pass to culling helpers when
    /// building instance buffers for data-layer points.
    #[allow(dead_code)]
    pub fn frustum(&self) -> Frustum {
        Frustum::from_view_proj(self.camera.view_proj())
    }

    /// Rebuild road vertex buffer when new data arrives from the watch channel.
    pub fn update_roads(&mut self, features: &[GeoFeature]) {
        self.roads.update(&self.device, features);
    }

    /// Rebuild railway vertex buffer when new data arrives from the watch channel.
    pub fn update_railways(&mut self, features: &[GeoFeature]) {
        self.railways.update(&self.device, features);
    }

    /// Color palette cycled through as new topo layers are added.
    fn topo_color(index: usize) -> [f32; 4] {
        const PALETTE: &[[f32; 4]] = &[
            [0.80, 0.55, 0.90, 0.90], // purple
            [0.30, 0.85, 0.55, 0.90], // green
            [0.90, 0.60, 0.20, 0.90], // orange
            [0.20, 0.70, 0.90, 0.90], // cyan
            [0.90, 0.30, 0.40, 0.90], // rose
            [0.90, 0.85, 0.20, 0.90], // yellow
        ];
        PALETTE[index % PALETTE.len()]
    }

    /// Load (or reload) a named GeoJSON topology layer.
    pub fn load_topo(&mut self, name: &str, path: &str) {
        match crate::topo::load_geojson(std::path::Path::new(path)) {
            Ok(features) => {
                if let Some(layer) = self.topo_layers.iter_mut().find(|l| l.name == name) {
                    layer.update(&self.device, &features);
                } else {
                    let color = Self::topo_color(self.topo_layers.len());
                    let mut layer = LineLayer::new(&self.device, &self.line_pipeline.bgl, name, color);
                    layer.update(&self.device, &features);
                    self.topo_layers.push(layer);
                }
            }
            Err(e) => tracing::error!("topo load '{name}': {e:#}"),
        }
    }

    /// Toggle visibility of a named topo layer.
    pub fn set_topo_visible(&mut self, name: &str, visible: bool) {
        if let Some(layer) = self.topo_layers.iter_mut().find(|l| l.name == name) {
            layer.visible = visible;
        }
    }

    /// Remove a named topo layer entirely.
    pub fn drop_topo(&mut self, name: &str) {
        self.topo_layers.retain(|l| l.name != name);
    }

    /// Rebuild world-map polygon GPU buffers from a new feature list.
    pub fn update_world_map(&mut self, features: Vec<crate::geo::GeoPolygon>) {
        self.world_map.update(&self.device, features);
    }

    /// Rebuild only the selected-feature highlight border (after `world_map.selected` changes).
    pub fn rebuild_selected_border(&mut self) {
        self.world_map.rebuild_selected_border(&self.device);
    }

    /// Project a geographic coordinate to a screen-space pixel position.
    /// Returns `None` when the point is behind the camera or outside the NDC cube.
    pub fn project_geo(&self, lat: f32, lon: f32) -> Option<egui::Pos2> {
        use crate::geo::geo_to_cartesian;
        use glam::Vec4;
        let world = geo_to_cartesian(lat, lon);
        let clip  = self.camera.view_proj() * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w <= 0.0 { return None; }
        let nx = clip.x / clip.w;
        let ny = clip.y / clip.w;
        if nx < -1.0 || nx > 1.0 || ny < -1.0 || ny > 1.0 { return None; }
        let sx = (nx + 1.0) * 0.5 * self.config.width  as f32;
        let sy = (1.0 - ny) * 0.5 * self.config.height as f32;
        Some(egui::Pos2::new(sx, sy))
    }

    /// Unproject a screen pixel to `(lat_deg, lon_deg)` on the globe surface.
    /// Returns `None` if the ray misses the unit sphere.
    pub fn pick_world_pos(&self, screen_x: f32, screen_y: f32) -> Option<(f32, f32)> {
        let w = self.config.width  as f32;
        let h = self.config.height as f32;
        let ndc_x =  screen_x / w * 2.0 - 1.0;
        let ndc_y = -(screen_y / h * 2.0 - 1.0);

        let inv_vp = self.camera.view_proj().inverse();
        let near4  = inv_vp * glam::Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
        let near   = near4.xyz() / near4.w;
        let far4   = inv_vp * glam::Vec4::new(ndc_x, ndc_y,  1.0, 1.0);
        let far    = far4.xyz()  / far4.w;
        let ray_dir    = (far - near).normalize();
        let ray_origin = self.camera.eye();

        // Ray-sphere intersection (unit sphere at origin)
        let oc   = ray_origin;
        let a    = ray_dir.dot(ray_dir);
        let b    = 2.0 * oc.dot(ray_dir);
        let c    = oc.dot(oc) - 1.0;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 { return None; }
        let t = (-b - disc.sqrt()) / (2.0 * a);
        if t < 0.0 { return None; }

        let hit = ray_origin + t * ray_dir;
        let (lat, lon) = cartesian_to_geo(hit);
        Some((lat, lon))
    }

    pub fn render(
        &mut self,
        hud:      &mut Hud,
        layers:   &LayerManager,
        console:  &mut Console,
        settings: &mut Settings,
        flights:  &[GeoFeature],
    ) -> RenderOutput {
        // ── Read back previous frame's GPU timestamps ─────────────────────────
        let gpu_ms = self.gpu_timer
            .as_mut()
            .and_then(|t| t.poll_result(&self.device));

        // ── Update globe uniforms ─────────────────────────────────────────────
        self.globe.update_uniforms(&self.queue, self.camera.view_proj(), Mat4::IDENTITY);

        // ── Acquire frame ─────────────────────────────────────────────────────
        let output = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return RenderOutput::default();
            }
            Err(e) => { tracing::error!("surface error: {e}"); return RenderOutput::default(); }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("frame") },
        );

        // ── Globe pass ────────────────────────────────────────────────────────
        let mut draw_calls: u32 = 0;
        let mut tri_count:  u32 = 0;
        {
            let globe_ts = self.gpu_timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites {
                query_set:                    t.query_set(),
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index:       Some(1),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("globe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color { r: 0.01, g: 0.01, b: 0.04, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view:        &self.depth_view,
                    depth_ops:   Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes:    globe_ts,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.globe.pipeline);
            pass.set_bind_group(0, &self.globe.bind_group, &[]);
            pass.set_vertex_buffer(0, self.globe.vertex_buf.slice(..));
            pass.set_index_buffer(self.globe.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.globe.index_count, 0, 0..1);
            draw_calls += 1;
            tri_count  += self.globe.index_count / 3;

            let _ = layers;
            let vp = self.camera.view_proj();
            // World map renders below OSM lines and aircraft.
            draw_world_map(&self.world_map, &self.world_map_pipeline, &self.queue, &mut pass, vp);
            draw_calls += 3; // fill + border + selected
            draw_layer(&self.roads,    &self.line_pipeline, &self.queue, &mut pass, vp);
            draw_layer(&self.railways, &self.line_pipeline, &self.queue, &mut pass, vp);
            draw_calls += 2;
            for topo in &self.topo_layers {
                draw_layer(topo, &self.line_pipeline, &self.queue, &mut pass, vp);
                draw_calls += 1;
            }
            self.aircraft.render(&self.queue, &mut pass, vp, flights);
            draw_calls += 1;
            tri_count  += flights.len() as u32 * 2; // two triangles per aircraft quad
        }

        // ── egui HUD pass ─────────────────────────────────────────────────────
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels:   [self.config.width, self.config.height],
            pixels_per_point: self.window.scale_factor() as f32,
        };

        // Compute world-map labels before the egui closure (avoids borrow conflict).
        let geo_labels = self.world_map.visible_labels(
            self.camera.view_proj(),
            self.camera.eye(),
            self.config.width  as f32,
            self.config.height as f32,
        );

        let egui_input  = self.egui_winit.take_egui_input(&self.window);
        let mut console_out = ConsoleOutput::default();
        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            // Draw geo labels below HUD/console elements.
            if !geo_labels.is_empty() {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Background,
                    egui::Id::new("geo_labels"),
                ));
                for (pos, text) in &geo_labels {
                    painter.text(
                        *pos,
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(10.0),
                        egui::Color32::from_rgba_unmultiplied(220, 220, 200, 210),
                    );
                }
            }
            hud.draw(ctx);
            console_out = console.draw(ctx, settings);
        });
        self.egui_winit.handle_platform_output(&self.window, full_output.platform_output);

        let tris = self.egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.device, &self.queue, *id, delta);
        }
        self.egui_renderer.update_buffers(
            &self.device, &self.queue, &mut encoder, &tris, &screen_descriptor,
        );
        {
            // egui_wgpu::Renderer::render requires RenderPass<'static>; safe here
            // because rpass is dropped before encoder.finish() is called.
            let egui_ts = self.gpu_timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites {
                query_set:                    t.query_set(),
                beginning_of_pass_write_index: Some(2),
                end_of_pass_write_index:       Some(3),
            });
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view:           &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load:  wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes:         egui_ts,
                    occlusion_query_set:      None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut rpass, &tris, &screen_descriptor);
        }
        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        // Resolve timestamps before submit so they're in the same command batch.
        if let Some(timer) = &mut self.gpu_timer {
            timer.resolve(&mut encoder);
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Schedule map_async after submit — GPU will process it asynchronously.
        if let Some(timer) = &mut self.gpu_timer {
            timer.schedule_readback();
        }

        output.present();

        RenderOutput { console: console_out, gpu_ms, draw_calls, tri_count }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn make_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label:               Some("depth"),
            size:                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count:     1,
            sample_count:        1,
            dimension:           wgpu::TextureDimension::D2,
            format:              DEPTH_FORMAT,
            usage:               wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:        &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}
