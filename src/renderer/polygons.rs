use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4Swizzles};
use wgpu::util::DeviceExt;

use crate::geo::{geo_to_cartesian, ring_spans_antimeridian, GeoPolygon};

// ─── GPU vertex / uniform types ───────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GeoVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GeoUniforms {
    view_proj: [[f32; 4]; 4],
    color:     [f32; 4],
}

// ─── Shared pipeline ──────────────────────────────────────────────────────────

/// Shared compiled fill + outline pipelines for the world-map layer.
/// Create once and pass to every `WorldMapLayer`.
pub struct WorldMapPipeline {
    pub fill_pl:    wgpu::RenderPipeline,
    pub outline_pl: wgpu::RenderPipeline,
    pub bgl:        wgpu::BindGroupLayout,
}

impl WorldMapPipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let fill_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("geo:fill:shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/geo_fill.wgsl").into(),
            ),
        });
        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("geo:outline:shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/geo_outline.wgsl").into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("geo:bgl"),
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
            label:                Some("geo:layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let vb_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GeoVertex>() as u64,
            step_mode:    wgpu::VertexStepMode::Vertex,
            attributes:   &[wgpu::VertexAttribute {
                offset:          0,
                shader_location: 0,
                format:          wgpu::VertexFormat::Float32x3,
            }],
        };

        // LessEqual so world-map geometry at the same depth as the globe surface
        // is not accidentally z-fought away.
        let depth_stencil = Some(wgpu::DepthStencilState {
            format:              super::DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare:       wgpu::CompareFunction::LessEqual,
            stencil:             wgpu::StencilState::default(),
            bias:                wgpu::DepthBiasState::default(),
        });

        let fill_pl = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("geo:fill:pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:              &fill_shader,
                entry_point:         "vs_main",
                buffers:             &[vb_layout.clone()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &fill_shader,
                entry_point:         "fs_main",
                targets:             &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:   wgpu::PrimitiveTopology::TriangleList,
                cull_mode:  None, // render both faces so winding ambiguity never hides geometry
                ..Default::default()
            },
            depth_stencil: depth_stencil.clone(),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        let outline_pl = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("geo:outline:pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:              &outline_shader,
                entry_point:         "vs_main",
                buffers:             &[vb_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &outline_shader,
                entry_point:         "fs_main",
                targets:             &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
            cache:       None,
        });

        Self { fill_pl, outline_pl, bgl }
    }
}

// ─── WorldMapLayer ────────────────────────────────────────────────────────────

/// Per-layer GPU + CPU state for a world-map polygon layer
/// (countries, states, etc.).  Create one per administrative level.
pub struct WorldMapLayer {
    pub visible:      bool,
    pub show_fill:    bool,
    pub show_borders: bool,
    pub show_names:   bool,
    /// Currently selected feature ID (highlighted with a different border).
    pub selected:     Option<String>,

    fill_color:       [f32; 4],
    border_color:     [f32; 4],
    selected_color:   [f32; 4],

    // Three separate uniform buffers so concurrent queue.write_buffer calls
    // don't clobber each other (all writes land before the pass runs).
    fill_uni:     wgpu::Buffer,
    fill_bg:      wgpu::BindGroup,
    border_uni:   wgpu::Buffer,
    border_bg:    wgpu::BindGroup,
    sel_uni:      wgpu::Buffer,
    sel_bg:       wgpu::BindGroup,

    // Fill geometry (triangulated via earcut)
    fill_vbuf:        Option<wgpu::Buffer>,
    fill_ibuf:        Option<wgpu::Buffer>,
    fill_count:       u32,

    // All-border geometry (every polygon ring)
    border_vbuf:      Option<wgpu::Buffer>,
    border_count:     u32,

    // Selected-feature border (one country/state highlighted)
    sel_border_vbuf:  Option<wgpu::Buffer>,
    sel_border_count: u32,

    /// CPU-side polygon data kept for hit-testing and label positions.
    pub features:     Vec<GeoPolygon>,
}

fn make_uni_and_bg(
    device: &wgpu::Device,
    bgl:    &wgpu::BindGroupLayout,
    label:  &str,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some(label),
        size:               std::mem::size_of::<GeoUniforms>() as u64,
        usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some(label),
        layout:  bgl,
        entries: &[wgpu::BindGroupEntry {
            binding:  0,
            resource: buf.as_entire_binding(),
        }],
    });
    (buf, bg)
}

impl WorldMapLayer {
    pub fn new(
        device:         &wgpu::Device,
        bgl:            &wgpu::BindGroupLayout,
        fill_color:     [f32; 4],
        border_color:   [f32; 4],
        selected_color: [f32; 4],
    ) -> Self {
        let (fill_uni, fill_bg)     = make_uni_and_bg(device, bgl, "geo:uni:fill");
        let (border_uni, border_bg) = make_uni_and_bg(device, bgl, "geo:uni:border");
        let (sel_uni, sel_bg)       = make_uni_and_bg(device, bgl, "geo:uni:sel");
        Self {
            visible: true,
            show_fill: true,
            show_borders: true,
            show_names: true,
            selected: None,
            fill_color,
            border_color,
            selected_color,
            fill_uni, fill_bg,
            border_uni, border_bg,
            sel_uni, sel_bg,
            fill_vbuf: None,
            fill_ibuf: None,
            fill_count: 0,
            border_vbuf: None,
            border_count: 0,
            sel_border_vbuf: None,
            sel_border_count: 0,
            features: Vec::new(),
        }
    }

    /// Rebuild all GPU buffers from a new feature list.
    pub fn update(&mut self, device: &wgpu::Device, features: Vec<GeoPolygon>) {
        self.features = features;
        self.build_fill(device);
        self.build_borders(device, &self.features.clone(), false);
        if self.selected.is_some() {
            self.rebuild_selected_border(device);
        }
        tracing::warn!(
            features    = self.features.len(),
            fill_tris   = self.fill_count / 3,
            border_segs = self.border_count / 2,
            "world map layer rebuilt"
        );
    }

    /// One-line status string for the `geo status` console command.
    pub fn status_string(&self) -> String {
        format!(
            "features={} fill_tris={} border_segs={} visible={}",
            self.features.len(),
            self.fill_count / 3,
            self.border_count / 2,
            self.visible,
        )
    }

    /// Call after changing `self.selected` to update the highlight border.
    pub fn rebuild_selected_border(&mut self, device: &wgpu::Device) {
        let selected_features: Vec<GeoPolygon> = match &self.selected {
            None => Vec::new(),
            Some(id) => self.features.iter().filter(|f| &f.id == id).cloned().collect(),
        };
        let (vbuf, count) = build_border_buffers(device, &selected_features, "sel");
        self.sel_border_vbuf  = vbuf;
        self.sel_border_count = count;
    }

    fn build_fill(&mut self, device: &wgpu::Device) {
        let (vbuf, ibuf, count) = triangulate_fill(device, &self.features);
        self.fill_vbuf  = vbuf;
        self.fill_ibuf  = ibuf;
        self.fill_count = count;
    }

    fn build_borders(&mut self, device: &wgpu::Device, features: &[GeoPolygon], is_selected: bool) {
        let label = if is_selected { "sel" } else { "all" };
        let (vbuf, count) = build_border_buffers(device, features, label);
        if is_selected {
            self.sel_border_vbuf  = vbuf;
            self.sel_border_count = count;
        } else {
            self.border_vbuf  = vbuf;
            self.border_count = count;
        }
    }

    /// Project feature centroids to screen space for egui label rendering.
    /// Filters to features on the visible hemisphere.
    pub fn visible_labels(
        &self,
        view_proj:  Mat4,
        camera_eye: Vec3,
        width:      f32,
        height:     f32,
    ) -> Vec<(egui::Pos2, String)> {
        if !self.visible || !self.show_names { return Vec::new(); }
        let mut out = Vec::new();
        for f in &self.features {
            let [lat, lon] = f.centroid;
            let world = geo_to_cartesian(lat, lon);
            if world.dot(camera_eye) <= 0.0 { continue; }

            let clip = view_proj * world.extend(1.0);
            if clip.w <= 0.0 { continue; }
            let ndc = clip.xyz() / clip.w;
            if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 { continue; }

            let x = (ndc.x + 1.0) * 0.5 * width;
            let y = (1.0 - ndc.y) * 0.5 * height;
            out.push((egui::pos2(x, y), f.name.clone()));
        }
        out
    }

    /// CPU-side hit test: returns the first feature that contains `(lat, lon)`.
    pub fn pick(&self, lat: f32, lon: f32) -> Option<&GeoPolygon> {
        self.features.iter().find(|f| f.contains_point(lat, lon))
    }
}

// ─── Draw helper ─────────────────────────────────────────────────────────────

pub fn draw_world_map<'pass>(
    layer:     &'pass WorldMapLayer,
    pipeline:  &'pass WorldMapPipeline,
    queue:     &wgpu::Queue,
    pass:      &mut wgpu::RenderPass<'pass>,
    view_proj: Mat4,
) {
    if !layer.visible { return; }

    // Each draw role writes to its OWN uniform buffer so concurrent
    // queue.write_buffer calls don't overwrite each other before the pass runs.

    // ── Fill ─────────────────────────────────────────────────────────────────
    if layer.show_fill {
        if let (Some(vbuf), Some(ibuf)) = (&layer.fill_vbuf, &layer.fill_ibuf) {
            if layer.fill_count > 0 {
                write_uniforms(queue, &layer.fill_uni, view_proj, layer.fill_color);
                pass.set_pipeline(&pipeline.fill_pl);
                pass.set_bind_group(0, &layer.fill_bg, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..layer.fill_count, 0, 0..1);
            }
        }
    }

    // ── All borders ──────────────────────────────────────────────────────────
    if layer.show_borders {
        if let Some(vbuf) = &layer.border_vbuf {
            if layer.border_count > 0 {
                write_uniforms(queue, &layer.border_uni, view_proj, layer.border_color);
                pass.set_pipeline(&pipeline.outline_pl);
                pass.set_bind_group(0, &layer.border_bg, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..layer.border_count, 0..1);
            }
        }
    }

    // ── Selected border highlight ─────────────────────────────────────────────
    if layer.selected.is_some() {
        if let Some(vbuf) = &layer.sel_border_vbuf {
            if layer.sel_border_count > 0 {
                write_uniforms(queue, &layer.sel_uni, view_proj, layer.selected_color);
                pass.set_pipeline(&pipeline.outline_pl);
                pass.set_bind_group(0, &layer.sel_bg, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..layer.sel_border_count, 0..1);
            }
        }
    }
}

// ─── Buffer builders ─────────────────────────────────────────────────────────

fn write_uniforms(
    queue:      &wgpu::Queue,
    buf:        &wgpu::Buffer,
    view_proj:  Mat4,
    color:      [f32; 4],
) {
    let u = GeoUniforms { view_proj: view_proj.to_cols_array_2d(), color };
    queue.write_buffer(buf, 0, bytemuck::bytes_of(&u));
}

fn triangulate_fill(
    device:   &wgpu::Device,
    features: &[GeoPolygon],
) -> (Option<wgpu::Buffer>, Option<wgpu::Buffer>, u32) {
    let mut all_verts:   Vec<GeoVertex> = Vec::new();
    let mut all_indices: Vec<u32>       = Vec::new();

    for feature in features {
        for poly in &feature.polygons {
            let Some(outer) = poly.first() else { continue };
            if outer.len() < 3 { continue; }
            if ring_spans_antimeridian(outer) { continue; }

            // Flatten rings into earcut's format: [x0, y0, x1, y1, ...]
            // We use lon as X, lat as Y.
            let mut flat:  Vec<f64>   = Vec::new();
            let mut holes: Vec<usize> = Vec::new();

            for &[lat, lon] in outer {
                flat.push(lon as f64);
                flat.push(lat as f64);
            }
            for hole in poly.iter().skip(1) {
                holes.push(flat.len() / 2);
                for &[lat, lon] in hole {
                    flat.push(lon as f64);
                    flat.push(lat as f64);
                }
            }

            let tris = earcutr::earcut(&flat, &holes, 2).unwrap_or_default();
            if tris.is_empty() { continue; }

            let base  = all_verts.len() as u32;
            let n_pts = flat.len() / 2;
            for i in 0..n_pts {
                let lon = flat[i * 2]     as f32;
                let lat = flat[i * 2 + 1] as f32;
                all_verts.push(GeoVertex { position: geo_to_cartesian(lat, lon).to_array() });
            }
            for idx in tris {
                all_indices.push(base + idx as u32);
            }
        }
    }

    let count = all_indices.len() as u32;
    if count == 0 { return (None, None, 0); }

    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some("geo:fill:vbuf"),
        contents: bytemuck::cast_slice(&all_verts),
        usage:    wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some("geo:fill:ibuf"),
        contents: bytemuck::cast_slice(&all_indices),
        usage:    wgpu::BufferUsages::INDEX,
    });
    (Some(vbuf), Some(ibuf), count)
}

fn build_border_buffers(
    device:   &wgpu::Device,
    features: &[GeoPolygon],
    label:    &str,
) -> (Option<wgpu::Buffer>, u32) {
    let mut verts: Vec<GeoVertex> = Vec::new();
    for feature in features {
        for poly in &feature.polygons {
            for ring in poly {
                if ring.len() < 2 { continue; }
                let pts: Vec<[f32; 3]> = ring.iter()
                    .map(|&[lat, lon]| geo_to_cartesian(lat, lon).to_array())
                    .collect();
                // GeoJSON rings are explicitly closed (first == last), so
                // windows(2) naturally includes the closing segment.
                for w in pts.windows(2) {
                    verts.push(GeoVertex { position: w[0] });
                    verts.push(GeoVertex { position: w[1] });
                }
            }
        }
    }
    let count = verts.len() as u32;
    if count == 0 { return (None, 0); }

    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some(&format!("geo:border:{label}:vbuf")),
        contents: bytemuck::cast_slice(&verts),
        usage:    wgpu::BufferUsages::VERTEX,
    });
    (Some(vbuf), count)
}
