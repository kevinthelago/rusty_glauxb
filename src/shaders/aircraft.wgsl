// Aircraft instanced rendering.
//
// Mesh vertices are in local tangent space (XZ = tangent plane, Z = forward).
// The instance transform places the mesh on the globe surface at the correct
// lat/lon and rotates it by the aircraft heading.

struct Uniforms {
    view_proj : mat4x4<f32>,
    color     : vec4<f32>,
}

@group(0) @binding(0)
var<uniform> u: Uniforms;

// ── Vertex inputs ─────────────────────────────────────────────────────────────

struct VertIn {
    // Mesh vertex in local space (XZ = tangent plane, Z = forward/north)
    @location(0) mesh_pos   : vec3<f32>,
    // Per-instance data
    @location(1) sphere_pos : vec3<f32>,  // unit-sphere surface point
    @location(2) heading    : f32,        // radians, 0=north, π/2=east
    @location(3) scale      : f32,        // world-space icon size (globe radii)
}

// ── Vertex shader ─────────────────────────────────────────────────────────────

@vertex
fn vs_main(v: VertIn) -> @builtin(position) vec4<f32> {
    let up    = normalize(v.sphere_pos);
    let ref_y = vec3<f32>(0.0, 1.0, 0.0);

    // Build an east vector; fall back to X-axis near the poles.
    let east = select(
        normalize(cross(ref_y, up)),
        normalize(cross(up, vec3<f32>(1.0, 0.0, 0.0))),
        abs(dot(up, ref_y)) > 0.999,
    );
    let north = normalize(cross(up, east));

    // Rotate in the tangent plane by heading.
    let ch  = cos(v.heading);
    let sh  = sin(v.heading);
    let fwd   = ch * north + sh * east;
    let right = ch * east  - sh * north;

    // Lift the icon slightly above the globe surface to avoid z-fighting.
    let lifted = v.sphere_pos * 1.003;

    // Apply the local mesh offset (XZ) and scale; ignore Y (kept in mesh for
    // potential future extrusion but treated as zero here).
    let world_pos = lifted
        + (right * v.mesh_pos.x + fwd * v.mesh_pos.z) * v.scale;

    return u.view_proj * vec4<f32>(world_pos, 1.0);
}

// ── Fragment shader ───────────────────────────────────────────────────────────

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
