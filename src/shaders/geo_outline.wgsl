struct Uniforms {
    view_proj: mat4x4<f32>,
    color:     vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    // 0.08 % above globe surface — above fills (0.03 %), below OSM (0.15 %)
    let p = normalize(position) * 1.0008;
    return u.view_proj * vec4<f32>(p, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
