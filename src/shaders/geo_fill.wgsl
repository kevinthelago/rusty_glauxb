struct Uniforms {
    view_proj: mat4x4<f32>,
    color:     vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    // 0.03 % above globe surface — below border lines (0.08 %) and OSM (0.15 %)
    let p = normalize(position) * 1.0003;
    return u.view_proj * vec4<f32>(p, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
