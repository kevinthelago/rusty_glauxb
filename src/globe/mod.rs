pub mod anim;
pub mod camera;

/// A single vertex on the cube-sphere surface.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlobeVertex {
    pub position: [f32; 3],
    pub normal:   [f32; 3],
    pub uv:       [f32; 2],
}

impl GlobeVertex {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBS: [wgpu::VertexAttribute; 3] = [
            wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 24, shader_location: 2, format: wgpu::VertexFormat::Float32x2 },
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBS,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CubeFace { PosX, NegX, PosY, NegY, PosZ, NegZ }

/// Cube-sphere mesh. Each of the 6 cube faces is subdivided into NxN quads,
/// then each vertex is normalised onto the unit sphere — no polar singularities.
pub struct CubeSphere {
    pub vertices:     Vec<GlobeVertex>,
    pub indices:      Vec<u32>,
    #[allow(dead_code)]
    pub subdivisions: u32,
}

impl CubeSphere {
    pub fn new(subdivisions: u32) -> Self {
        let mut vertices = Vec::new();
        let mut indices  = Vec::new();
        for face in [
            CubeFace::PosX, CubeFace::NegX,
            CubeFace::PosY, CubeFace::NegY,
            CubeFace::PosZ, CubeFace::NegZ,
        ] {
            Self::generate_face(face, subdivisions, &mut vertices, &mut indices);
        }
        Self { vertices, indices, subdivisions }
    }

    fn generate_face(
        face: CubeFace,
        subdivisions: u32,
        vertices: &mut Vec<GlobeVertex>,
        indices: &mut Vec<u32>,
    ) {
        let base = vertices.len() as u32;
        let n = subdivisions + 1;

        for row in 0..n {
            for col in 0..n {
                let s = col as f32 / subdivisions as f32;
                let t = row as f32 / subdivisions as f32;
                let (ax, ay, az) = Self::face_coords(face, s * 2.0 - 1.0, t * 2.0 - 1.0);
                let len = (ax * ax + ay * ay + az * az).sqrt();
                let position = [ax / len, ay / len, az / len];
                vertices.push(GlobeVertex { position, normal: position, uv: [s, t] });
            }
        }

        for row in 0..subdivisions {
            for col in 0..subdivisions {
                let i = base + row * n + col;
                indices.extend_from_slice(&[i, i + 1, i + n, i + 1, i + n + 1, i + n]);
            }
        }
    }

    fn face_coords(face: CubeFace, u: f32, v: f32) -> (f32, f32, f32) {
        match face {
            CubeFace::PosX => ( 1.0,   v,  -u),
            CubeFace::NegX => (-1.0,   v,   u),
            CubeFace::PosY => (  u,  1.0,  -v),
            CubeFace::NegY => (  u, -1.0,   v),
            CubeFace::PosZ => (  u,   v,  1.0),
            CubeFace::NegZ => ( -u,   v, -1.0),
        }
    }
}
