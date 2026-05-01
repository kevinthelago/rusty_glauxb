struct Uniforms {
    view_proj: mat4x4<f32>,
    model:     mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * u.model * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.9, 1.0, 1.0); // bright cyan wireframe
}
