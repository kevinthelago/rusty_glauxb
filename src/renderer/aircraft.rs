use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::geo::GeoFeature;

// ─── Constants ────────────────────────────────────────────────────────────────

const MAX_INSTANCES: usize = 10_000;

/// Icon size in globe radii.  At the default zoom (camera radius 2.5) this
/// produces icons a few pixels wide — enough to see category at a glance.
const SCALES: [f32; 4] = [0.013, 0.016, 0.015, 0.014];

/// RGBA colours per category: single-engine, multi-engine, military, misc.
const COLORS: [[f32; 4]; 4] = [
    [0.25, 1.00, 0.25, 1.0], // SingleEngine — bright green
    [0.30, 0.65, 1.00, 1.0], // MultiEngine  — sky blue
    [1.00, 0.28, 0.20, 1.0], // Military     — red
    [1.00, 0.75, 0.00, 1.0], // Misc/balloon — amber
];

// ─── GPU types ────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct AircraftVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct AircraftInstance {
    sphere_pos: [f32; 3],
    heading:    f32,
    scale:      f32,
    _pad:       [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct AircraftUniforms {
    view_proj: [[f32; 4]; 4],
    color:     [f32; 4],
}

// ─── Mesh ─────────────────────────────────────────────────────────────────────

struct AircraftMesh {
    vertex_buf:  wgpu::Buffer,
    index_buf:   wgpu::Buffer,
    index_count: u32,
}

/// Build a flat polygon mesh from an XZ outline using fan triangulation from
/// the centroid.  The outline is given as (x, z) pairs; forward = +Z.
fn build_mesh(device: &wgpu::Device, label: &str, outline: &[(f32, f32)]) -> AircraftMesh {
    let n  = outline.len();
    let cx = outline.iter().map(|(x, _)| *x).sum::<f32>() / n as f32;
    let cz = outline.iter().map(|(_, z)| *z).sum::<f32>() / n as f32;

    let mut verts: Vec<AircraftVertex> = Vec::with_capacity(n + 1);
    verts.push(AircraftVertex { position: [cx, 0.0, cz] }); // centre (index 0)
    for (x, z) in outline {
        verts.push(AircraftVertex { position: [*x, 0.0, *z] });
    }

    // Fan: (0, i+1, (i+1)%n + 1) for i in 0..n
    let mut indices: Vec<u32> = Vec::with_capacity(n * 3);
    for i in 0..n as u32 {
        indices.push(0);
        indices.push(i + 1);
        indices.push(i % n as u32 + 1 + 1);
    }
    // Fix last triangle: wrap-around index should be 1, not n+1
    // The formula (i % n) + 1 + 1 gives n+1 when i=n-1; correct to 1.
    if let Some(last) = indices.last_mut() {
        if *last == n as u32 + 1 { *last = 1; }
    }

    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some(label),
        contents: bytemuck::cast_slice(&verts),
        usage:    wgpu::BufferUsages::VERTEX,
    });
    let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some(&format!("{label}:idx")),
        contents: bytemuck::cast_slice(&indices),
        usage:    wgpu::BufferUsages::INDEX,
    });

    AircraftMesh { vertex_buf, index_buf, index_count: indices.len() as u32 }
}

/// Single-engine civilian: classic high-wing shape with swept V-tail.
fn mesh_single_engine(device: &wgpu::Device) -> AircraftMesh {
    build_mesh(device, "ac:single", &[
        ( 0.00,  0.52),  // nose
        (-0.07,  0.20),  // fwd-left fuselage shoulder
        (-0.46,  0.08),  // left wing tip front
        (-0.46, -0.02),  // left wing tip rear
        (-0.07, -0.28),  // left aft fuselage
        (-0.18, -0.46),  // left tail fin tip
        ( 0.00, -0.52),  // tail centre
        ( 0.18, -0.46),  // right tail fin tip
        ( 0.07, -0.28),  // right aft fuselage
        ( 0.46, -0.02),  // right wing tip rear
        ( 0.46,  0.08),  // right wing tip front
        ( 0.07,  0.20),  // fwd-right fuselage shoulder
    ])
}

/// Multi-engine civilian: wide swept-wing airliner silhouette.
fn mesh_multi_engine(device: &wgpu::Device) -> AircraftMesh {
    build_mesh(device, "ac:multi", &[
        ( 0.00,  0.54),  // nose
        (-0.10,  0.26),
        (-0.56,  0.07),  // left wing tip front
        (-0.56, -0.07),  // left wing tip rear
        (-0.12, -0.18),
        (-0.09, -0.34),
        (-0.16, -0.47),  // left tail fin
        ( 0.00, -0.54),  // tail
        ( 0.16, -0.47),  // right tail fin
        ( 0.09, -0.34),
        ( 0.12, -0.18),
        ( 0.56, -0.07),  // right wing tip rear
        ( 0.56,  0.07),  // right wing tip front
        ( 0.10,  0.26),
    ])
}

/// Military fighter: narrow delta-wing shape with twin tail fins.
fn mesh_military(device: &wgpu::Device) -> AircraftMesh {
    build_mesh(device, "ac:military", &[
        ( 0.00,  0.68),  // sharp nose
        (-0.06,  0.30),
        (-0.52, -0.08),  // long swept wing tip
        (-0.20, -0.34),
        (-0.10, -0.52),  // left tail fin base
        (-0.05, -0.62),  // left tail tip
        ( 0.00, -0.58),  // tail centre notch
        ( 0.05, -0.62),  // right tail tip
        ( 0.10, -0.52),  // right tail fin base
        ( 0.20, -0.34),
        ( 0.52, -0.08),  // right swept wing tip
        ( 0.06,  0.30),
    ])
}

/// Misc / hot air balloon: circle top-view (12-gon) with small gondola below.
fn mesh_balloon(device: &wgpu::Device) -> AircraftMesh {
    let r = 0.38_f32;
    let n_circle = 12_usize;
    let mut pts: Vec<(f32, f32)> = (0..n_circle)
        .map(|i| {
            let a = (i as f32 / n_circle as f32) * std::f32::consts::TAU;
            (a.sin() * r, a.cos() * r)
        })
        .collect();
    // Gondola: small square below the balloon
    pts.extend_from_slice(&[
        (-0.08, -0.46),
        ( 0.08, -0.46),
        ( 0.08, -0.56),
        (-0.08, -0.56),
    ]);
    build_mesh(device, "ac:balloon", &pts)
}

// ─── AircraftRenderer ─────────────────────────────────────────────────────────

struct UniformSet {
    buf:        wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct AircraftRenderer {
    pipeline:      wgpu::RenderPipeline,
    meshes:        [AircraftMesh; 4],
    uniform_sets:  [UniformSet; 4],
    instance_bufs: [wgpu::Buffer; 4],
}

impl AircraftRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("aircraft:shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/aircraft.wgsl").into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("aircraft:bgl"),
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

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("aircraft:layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        // Vertex buffer 0: mesh vertices (position only)
        let vb_mesh = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<AircraftVertex>() as u64, // 12 bytes
            step_mode:    wgpu::VertexStepMode::Vertex,
            attributes:   &[wgpu::VertexAttribute {
                offset:          0,
                shader_location: 0,
                format:          wgpu::VertexFormat::Float32x3,
            }],
        };

        // Vertex buffer 1: per-instance data
        let vb_inst = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<AircraftInstance>() as u64, // 32 bytes
            step_mode:    wgpu::VertexStepMode::Instance,
            attributes:   &[
                wgpu::VertexAttribute { offset: 0,  shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: 12, shader_location: 2, format: wgpu::VertexFormat::Float32 },
                wgpu::VertexAttribute { offset: 16, shader_location: 3, format: wgpu::VertexFormat::Float32 },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("aircraft"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         "vs_main",
                buffers:             &[vb_mesh, vb_inst],
                compilation_options: Default::default(),
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
                cull_mode:    None, // flat icons, both sides visible
                polygon_mode: wgpu::PolygonMode::Fill,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              super::DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             wgpu::StencilState::default(),
                bias:                wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        // One uniform buffer + bind group per category
        let uniform_sets = std::array::from_fn(|_| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("aircraft:uniforms"),
                size:               std::mem::size_of::<AircraftUniforms>() as u64,
                usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:   Some("aircraft:bg"),
                layout:  &bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding:  0,
                    resource: buf.as_entire_binding(),
                }],
            });
            UniformSet { buf, bind_group }
        });

        let meshes = [
            mesh_single_engine(device),
            mesh_multi_engine(device),
            mesh_military(device),
            mesh_balloon(device),
        ];

        let instance_bufs = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("aircraft:instances"),
                size:               (MAX_INSTANCES * std::mem::size_of::<AircraftInstance>()) as u64,
                usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        Self { pipeline, meshes, uniform_sets, instance_bufs }
    }

    /// Upload instance data and issue one draw call per non-empty category.
    pub fn render<'pass>(
        &'pass self,
        queue:     &wgpu::Queue,
        pass:      &mut wgpu::RenderPass<'pass>,
        view_proj: Mat4,
        flights:   &[GeoFeature],
    ) {
        // Bucket flights by aircraft_category
        let mut buckets: [Vec<AircraftInstance>; 4] = Default::default();
        for feat in flights {
            let cat_idx = feat.meta
                .get("aircraft_category")
                .and_then(|v| v.as_str())
                .map(cat_str_to_idx)
                .unwrap_or(3); // default = Misc

            let heading_rad = feat.meta
                .get("heading")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32
                * std::f32::consts::PI / 180.0;

            let pos = feat.world_pos();
            buckets[cat_idx].push(AircraftInstance {
                sphere_pos: pos.to_array(),
                heading:    heading_rad,
                scale:      SCALES[cat_idx],
                _pad:       [0.0; 3],
            });
        }

        pass.set_pipeline(&self.pipeline);

        for cat in 0..4_usize {
            let instances = &buckets[cat];
            if instances.is_empty() { continue; }

            let count = instances.len().min(MAX_INSTANCES);

            // Update uniform buffer (view_proj + baked colour)
            let uniforms = AircraftUniforms {
                view_proj: view_proj.to_cols_array_2d(),
                color:     COLORS[cat],
            };
            queue.write_buffer(
                &self.uniform_sets[cat].buf,
                0,
                bytemuck::bytes_of(&uniforms),
            );

            // Upload instances
            queue.write_buffer(
                &self.instance_bufs[cat],
                0,
                bytemuck::cast_slice(&instances[..count]),
            );

            pass.set_bind_group(0, &self.uniform_sets[cat].bind_group, &[]);
            pass.set_vertex_buffer(0, self.meshes[cat].vertex_buf.slice(..));
            pass.set_vertex_buffer(1, self.instance_bufs[cat].slice(..));
            pass.set_index_buffer(
                self.meshes[cat].index_buf.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.meshes[cat].index_count, 0, 0..count as u32);
        }
    }
}

fn cat_str_to_idx(s: &str) -> usize {
    match s {
        "single_engine" => 0,
        "multi_engine"  => 1,
        "military"      => 2,
        _               => 3,
    }
}
