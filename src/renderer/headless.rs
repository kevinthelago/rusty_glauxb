//! Headless wgpu renderer for the `bench` binary.
//!
//! Renders to a `wgpu::Texture` — no window, no display server required.
//! Reuses the same pipeline constructors as `WgpuState`.

use glam::Mat4;
use std::time::Instant;
use wgpu::util::DeviceExt;

use crate::{
    diag::{stats::FrameStats, AdapterCaps, GpuTimer},
    geo::GeoFeature,
    globe::{camera::ArcballCamera, CubeSphere, GlobeVertex},
};

use super::{
    aircraft::AircraftRenderer,
    lines::LinePipeline,
    polygons::{draw_world_map, WorldMapLayer, WorldMapPipeline},
    DEPTH_FORMAT,
};

pub const HEADLESS_WIDTH:  u32 = 1280;
pub const HEADLESS_HEIGHT: u32 = 720;
pub const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

const BENCH_SUBDIVISIONS: u32 = 32; // lower than production 64 for faster bench startup

// ─── Minimal Globe pipeline for headless ─────────────────────────────────────

struct GlobePipeline {
    pipeline:    wgpu::RenderPipeline,
    vertex_buf:  wgpu::Buffer,
    index_buf:   wgpu::Buffer,
    index_count: u32,
    uniform_buf: wgpu::Buffer,
    bind_group:  wgpu::BindGroup,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobeUniforms {
    view_proj: [[f32; 4]; 4],
    model:     [[f32; 4]; 4],
}

impl GlobePipeline {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, polygon_mode: wgpu::PolygonMode) -> Self {
        let mesh = CubeSphere::new(BENCH_SUBDIVISIONS);

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("bench:globe:verts"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage:    wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("bench:globe:idx"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage:    wgpu::BufferUsages::INDEX,
        });
        let index_count = mesh.indices.len() as u32;

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("bench:globe:uni"),
            size:               std::mem::size_of::<GlobeUniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("bench:globe:bgl"),
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
            label:   Some("bench:globe:bg"),
            layout:  &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/globe.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("bench:globe:layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("bench:globe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: "vs_main",
                buffers:     &[GlobeVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: "fs_main",
                targets:     &[Some(wgpu::ColorTargetState {
                    format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:     wgpu::PrimitiveTopology::TriangleList,
                polygon_mode,
                cull_mode:    Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             wgpu::StencilState::default(),
                bias:                wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        Self { pipeline, vertex_buf, index_buf, index_count, uniform_buf, bind_group }
    }

    fn update_uniforms(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        let uni = GlobeUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            model:     Mat4::IDENTITY.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uni));
    }
}

// ─── HeadlessRenderer ────────────────────────────────────────────────────────

pub struct HeadlessRenderer {
    pub device:       wgpu::Device,
    pub queue:        wgpu::Queue,
    color_view:       wgpu::TextureView,
    depth_view:       wgpu::TextureView,
    globe:            GlobePipeline,
    aircraft:         AircraftRenderer,
    #[allow(dead_code)]
    line_pipeline:    LinePipeline,
    world_map_pipe:   WorldMapPipeline,
    pub world_map:    WorldMapLayer,
    pub adapter_caps: AdapterCaps,
    pub gpu_timer:    Option<GpuTimer>,
}

impl HeadlessRenderer {
    pub async fn new(force_software: bool) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     None,
                force_fallback_adapter: force_software,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("no suitable GPU adapter"))?;

        let mut caps = AdapterCaps::from_adapter(&adapter);

        let mut wanted = wgpu::Features::empty();
        if caps.has_polygon_mode_line { wanted |= wgpu::Features::POLYGON_MODE_LINE; }
        if caps.has_timestamp_query   { wanted |= wgpu::Features::TIMESTAMP_QUERY; }

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label:             Some("bench"),
                    required_features: wanted,
                    required_limits:   wgpu::Limits::default(),
                    memory_hints:      wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        caps.apply_queue(&queue);
        let gpu_timer = caps.has_timestamp_query
            .then(|| GpuTimer::new(&device, caps.timestamp_period_ns));

        let polygon_mode = if caps.has_polygon_mode_line {
            wgpu::PolygonMode::Line
        } else {
            wgpu::PolygonMode::Fill
        };

        let color_view = device.create_texture(&wgpu::TextureDescriptor {
            label:               Some("bench:color"),
            size:                wgpu::Extent3d { width: HEADLESS_WIDTH, height: HEADLESS_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count:     1,
            sample_count:        1,
            dimension:           wgpu::TextureDimension::D2,
            format:              HEADLESS_FORMAT,
            usage:               wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats:        &[],
        }).create_view(&wgpu::TextureViewDescriptor::default());

        let depth_view = device.create_texture(&wgpu::TextureDescriptor {
            label:               Some("bench:depth"),
            size:                wgpu::Extent3d { width: HEADLESS_WIDTH, height: HEADLESS_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count:     1,
            sample_count:        1,
            dimension:           wgpu::TextureDimension::D2,
            format:              DEPTH_FORMAT,
            usage:               wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:        &[],
        }).create_view(&wgpu::TextureViewDescriptor::default());

        let globe          = GlobePipeline::new(&device, HEADLESS_FORMAT, polygon_mode);
        let aircraft       = AircraftRenderer::new(&device, HEADLESS_FORMAT);
        let line_pipeline  = LinePipeline::new(&device, HEADLESS_FORMAT);
        let world_map_pipe = WorldMapPipeline::new(&device, HEADLESS_FORMAT);
        let world_map      = WorldMapLayer::new(
            &device, &world_map_pipe.bgl,
            [0.10, 0.40, 0.15, 0.80],
            [1.00, 0.95, 0.40, 1.00],
            [1.00, 0.40, 0.10, 1.00],
        );

        Ok(Self {
            device, queue,
            color_view, depth_view,
            globe, aircraft, line_pipeline, world_map_pipe, world_map,
            adapter_caps: caps,
            gpu_timer,
        })
    }

    /// Render one frame with the supplied flights.
    /// Uses `Maintain::Wait` so wall time includes full GPU execution.
    pub fn render_frame(&mut self, flights: &[GeoFeature]) -> FrameStats {
        let gpu_ms = self.gpu_timer
            .as_mut()
            .and_then(|t| t.poll_result(&self.device));

        let wall_start = Instant::now();

        let vp = {
            let mut cam = ArcballCamera::default();
            cam.aspect = HEADLESS_WIDTH as f32 / HEADLESS_HEIGHT as f32;
            cam.view_proj()
        };
        self.globe.update_uniforms(&self.queue, vp);

        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("bench:frame") },
        );

        let mut draw_calls: u32 = 0;
        let mut tri_count:  u32 = 0;
        {
            let ts = self.gpu_timer.as_ref().map(|t| wgpu::RenderPassTimestampWrites {
                query_set:                    t.query_set(),
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index:       Some(1),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bench:pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &self.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color { r: 0.01, g: 0.01, b: 0.04, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes:    ts,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.globe.pipeline);
            pass.set_bind_group(0, &self.globe.bind_group, &[]);
            pass.set_vertex_buffer(0, self.globe.vertex_buf.slice(..));
            pass.set_index_buffer(self.globe.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.globe.index_count, 0, 0..1);
            draw_calls += 1;
            tri_count  += self.globe.index_count / 3;

            draw_world_map(&self.world_map, &self.world_map_pipe, &self.queue, &mut pass, vp);
            draw_calls += 3;

            self.aircraft.render(&self.queue, &mut pass, vp, flights);
            draw_calls += 1;
            tri_count  += flights.len() as u32 * 2;
        }

        if let Some(timer) = &mut self.gpu_timer {
            timer.resolve(&mut encoder);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        if let Some(timer) = &mut self.gpu_timer {
            timer.schedule_readback();
        }

        // Block until GPU finishes so wall_ms is the true frame cost.
        self.device.poll(wgpu::Maintain::Wait);

        let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
        let cpu_ms  = gpu_ms.map(|g| (wall_ms - g).max(0.0)).unwrap_or(wall_ms);

        FrameStats { wall_ms, gpu_ms, cpu_ms, rss_mb: crate::diag::current_rss_mb(), draw_calls, tri_count }
    }
}
