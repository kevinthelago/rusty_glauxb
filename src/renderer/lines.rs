use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::{
    geo::{geo_to_cartesian, FeatureKind, GeoFeature},
    tile::{LodLevel, TileGrid, TileSet, TILE_COUNT},
};

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
    /// Minimum LOD level at which this layer is rendered.
    /// Layers with coarser data (coastlines, borders) set this lower so they
    /// remain visible when zoomed out; dense layers (local roads) set it higher.
    pub min_lod:  LodLevel,
    color:        [f32; 4],
    uniform_buf:  wgpu::Buffer,
    bind_group:   wgpu::BindGroup,
    vertex_buf:   Option<wgpu::Buffer>,
    vertex_count: u32,
    /// Source features retained for tile indexing and hover projection.
    pub raw_features: Vec<GeoFeature>,
    /// Precomputed 3-D positions for every feature node (parallel to coordinates).
    node_cache:   Vec<Vec<Vec3>>,
    /// `tile_index[tile_id]` = sorted list of feature indices that have at least
    /// one node in that tile.  Built once on `update()`; queried every cull.
    tile_index:   Vec<Vec<usize>>,
    /// The tile set used for the last vertex-buffer build.  Skip rebuild when
    /// the set hasn't changed.
    last_tiles:   TileSet,
}

impl LineLayer {
    pub fn new(
        device:  &wgpu::Device,
        bgl:     &wgpu::BindGroupLayout,
        name:    impl Into<String>,
        color:   [f32; 4],
    ) -> Self {
        Self::new_with_lod(device, bgl, name, color, LodLevel::Medium)
    }

    pub fn new_with_lod(
        device:  &wgpu::Device,
        bgl:     &wgpu::BindGroupLayout,
        name:    impl Into<String>,
        color:   [f32; 4],
        min_lod: LodLevel,
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
            visible:      true,
            min_lod,
            color,
            uniform_buf,
            bind_group,
            vertex_buf:   None,
            vertex_count: 0,
            raw_features: Vec::new(),
            node_cache:   Vec::new(),
            tile_index:   vec![Vec::new(); TILE_COUNT],
            last_tiles:   TileSet::full(), // force rebuild on first cull
        }
    }

    /// Store new source features and rebuild the tile index.
    ///
    /// This is the only place that touches `tile_index`; it runs once per
    /// data refresh (network fetch or startup load), not per frame.
    pub fn update(&mut self, _device: &wgpu::Device, features: &[GeoFeature], grid: &TileGrid) {
        self.raw_features = features.to_vec();
        self.vertex_buf   = None;
        self.vertex_count = 0;

        // Pre-convert all coordinates to 3-D world positions.
        self.node_cache = features
            .iter()
            .map(|f| {
                f.coordinates
                    .iter()
                    .map(|&[lat, lon]| geo_to_cartesian(lat, lon))
                    .collect()
            })
            .collect();

        // Build the tile → feature-index map.
        for bucket in &mut self.tile_index {
            bucket.clear();
        }
        for (fi, (feat, nodes)) in
            self.raw_features.iter().zip(self.node_cache.iter()).enumerate()
        {
            if !matches!(feat.kind, FeatureKind::Line) || nodes.len() < 2 {
                continue;
            }
            // A feature belongs to every tile that contains at least one of its
            // nodes.  We find the tile for each node by checking all tiles'
            // geographic bounding boxes — cheap enough since TILE_COUNT = 384.
            let mut assigned = vec![false; TILE_COUNT];
            for node in nodes {
                let (lat, lon) = cartesian_to_geo_local(*node);
                for (ti, tile) in grid.tiles().iter().enumerate() {
                    if !assigned[ti]
                        && lat >= tile.geo.min_lat
                        && lat <= tile.geo.max_lat
                        && lon >= tile.geo.min_lon
                        && lon <= tile.geo.max_lon
                    {
                        self.tile_index[ti].push(fi);
                        assigned[ti] = true;
                    }
                }
            }
        }

        // Force a vertex-buffer rebuild on the next cull call.
        self.last_tiles = TileSet::full();

        tracing::debug!(
            layer    = %self.name,
            features = self.raw_features.len(),
            "tile index built"
        );
    }

    /// Rebuild the vertex buffer from the set of visible tiles and the current
    /// LOD level.  No-ops when neither has changed since the last build.
    pub fn cull_to_tiles(
        &mut self,
        device:  &wgpu::Device,
        visible: &TileSet,
        lod:     LodLevel,
    ) {
        // LOD gate: suppress this layer entirely when zoomed out past its threshold.
        if lod < self.min_lod {
            if self.vertex_count != 0 {
                self.vertex_buf   = None;
                self.vertex_count = 0;
            }
            return;
        }

        // Skip rebuild when the visible set hasn't changed.
        if visible.same_as(&self.last_tiles) {
            return;
        }
        self.last_tiles = *visible;

        // Collect the unique feature indices that fall in any visible tile.
        // Use a scratch bitset to avoid duplicates without allocating a HashSet.
        let mut included = vec![false; self.raw_features.len()];
        for ti in visible.iter_set() {
            for &fi in &self.tile_index[ti] {
                included[fi] = true;
            }
        }

        // Assemble the vertex buffer from included features.
        let mut verts: Vec<LineVertex> = Vec::new();
        for (fi, nodes) in self.node_cache.iter().enumerate() {
            if !included[fi] || nodes.len() < 2 {
                continue;
            }
            for pair in nodes.windows(2) {
                verts.push(LineVertex { position: pair[0].to_array() });
                verts.push(LineVertex { position: pair[1].to_array() });
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

        tracing::debug!(
            layer    = %self.name,
            segments = self.vertex_count / 2,
            total    = self.raw_features.len(),
            "line layer tile-culled"
        );
    }
}

/// Inline lat/lon extraction from a unit-sphere point to avoid a cross-module
/// dependency on `geo::cartesian_to_geo` inside the hot path.
#[inline(always)]
fn cartesian_to_geo_local(p: Vec3) -> (f32, f32) {
    let lat = p.y.clamp(-1.0, 1.0).asin().to_degrees();
    let lon = p.x.atan2(p.z).to_degrees();
    (lat, lon)
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
