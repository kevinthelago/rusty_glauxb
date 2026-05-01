use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::geo::{geo_to_cartesian, FeatureKind, GeoFeature};

// ─── GPU types ────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct LineVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct LineUniforms {
    view_proj: [[f32; 4]; 4],
    color:     [f32; 4],
}

// ─── Shared compiled pipeline ─────────────────────────────────────────────────

/// The compiled shader + pipeline shared by every `LineLayer`.
/// Create once; pass a reference to each `draw_layer` call.
pub struct LinePipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bgl:      wgpu::BindGroupLayout,
}

impl LinePipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("lines:shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/lines.wgsl").into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("lines:bgl"),
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
            label:                Some("lines:layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let vb = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode:    wgpu::VertexStepMode::Vertex,
            attributes:   &[wgpu::VertexAttribute {
                offset:          0,
                shader_location: 0,
                format:          wgpu::VertexFormat::Float32x3,
            }],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("lines:pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         "vs_main",
                buffers:             &[vb],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         "fs_main",
                targets:             &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:  wgpu::PrimitiveTopology::LineList,
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

        Self { pipeline, bgl }
    }
}

// ─── LineLayer ────────────────────────────────────────────────────────────────

/// Per-layer GPU state.  Each data layer (roads, railways, …) gets its own
/// `LineLayer` instance with independent colour, vertex data, and visibility.
pub struct LineLayer {
    /// Human-readable name used in log messages and console commands.
    pub name:     String,
    /// Toggle with `layer.visible = false` to hide without dropping GPU data.
    pub visible:  bool,
    color:        [f32; 4],
    uniform_buf:  wgpu::Buffer,
    bind_group:   wgpu::BindGroup,
    vertex_buf:   Option<wgpu::Buffer>,
    vertex_count: u32,
}

impl LineLayer {
    pub fn new(
        device:  &wgpu::Device,
        bgl:     &wgpu::BindGroupLayout,
        name:    impl Into<String>,
        color:   [f32; 4],
    ) -> Self {
        let name = name.into();
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some(&format!("lines:{name}:uni")),
            size:               std::mem::size_of::<LineUniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some(&format!("lines:{name}:bg")),
            layout:  bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        Self {
            name,
            visible: true,
            color,
            uniform_buf,
            bind_group,
            vertex_buf:   None,
            vertex_count: 0,
        }
    }

    /// Rebuild vertex buffer from line features.
    /// Called only when the watch channel delivers new data (not every frame).
    pub fn update(&mut self, device: &wgpu::Device, features: &[GeoFeature]) {
        let mut verts: Vec<LineVertex> = Vec::new();

        for feat in features {
            if !matches!(feat.kind, FeatureKind::Line) || feat.coordinates.len() < 2 {
                continue;
            }
            let nodes: Vec<[f32; 3]> = feat.coordinates.iter()
                .map(|&[lat, lon]| geo_to_cartesian(lat, lon).to_array())
                .collect();

            for pair in nodes.windows(2) {
                verts.push(LineVertex { position: pair[0] });
                verts.push(LineVertex { position: pair[1] });
            }
        }

        self.vertex_count = verts.len() as u32;

        if verts.is_empty() {
            self.vertex_buf = None;
        } else {
            self.vertex_buf = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label:    Some(&format!("lines:{}:vbuf", self.name)),
                    contents: bytemuck::cast_slice(&verts),
                    usage:    wgpu::BufferUsages::VERTEX,
                },
            ));
        }

        tracing::info!(
            layer = %self.name,
            segments = self.vertex_count / 2,
            "line layer rebuilt"
        );
    }
}

// ─── Draw helper ─────────────────────────────────────────────────────────────

/// Issue one draw call for `layer` using the shared `LinePipeline`.
/// No-op if the layer is hidden or has no data.
pub fn draw_layer<'pass>(
    layer:     &'pass LineLayer,
    lp:        &'pass LinePipeline,
    queue:     &wgpu::Queue,
    pass:      &mut wgpu::RenderPass<'pass>,
    view_proj: Mat4,
) {
    if !layer.visible { return; }
    let Some(ref vbuf) = layer.vertex_buf else { return };
    if layer.vertex_count == 0 { return; }

    let uniforms = LineUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        color:     layer.color,
    };
    queue.write_buffer(&layer.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

    pass.set_pipeline(&lp.pipeline);
    pass.set_bind_group(0, &layer.bind_group, &[]);
    pass.set_vertex_buffer(0, vbuf.slice(..));
    pass.draw(0..layer.vertex_count, 0..1);
}
