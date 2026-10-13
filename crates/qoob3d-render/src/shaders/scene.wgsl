// QOOB3D viewport shaders. All colours are linear; the render target is sRGB.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    body_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

// ---------------------------------------------------------------- background

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_background(@builtin(vertex_index) i: u32) -> BgOut {
    // Fullscreen triangle.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: BgOut;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.t = uv.y * 0.5;
    return out;
}

@fragment
fn fs_background(in: BgOut) -> @location(0) vec4<f32> {
    return mix(g.bg_bottom, g.bg_top, clamp(in.t, 0.0, 1.0));
}

// ---------------------------------------------------------------- solid bodies

struct MeshIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_mesh(in: MeshIn) -> MeshOut {
    var out: MeshOut;
    out.pos = g.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    return out;
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) { n = -n; }
    let v = normalize(g.eye.xyz - in.world_pos);

    // Headlight slightly above and to the left of the eye, plus a sky/ground
    // hemisphere ambient so faces pointing away from the camera stay readable.
    let l = normalize(v + 0.35 * cross(v, vec3<f32>(0.0, 0.0, 1.0)) + vec3<f32>(0.0, 0.0, 0.35));
    let diffuse = max(dot(n, l), 0.0);
    let hemi = mix(0.18, 0.32, 0.5 + 0.5 * n.z);
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.18;

    let base = g.body_color.rgb;
    let color = base * (hemi + 0.72 * diffuse) + vec3<f32>(spec);
    return vec4<f32>(color, 1.0);
}
