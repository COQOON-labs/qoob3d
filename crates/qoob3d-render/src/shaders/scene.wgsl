// QOOB3D viewport shaders (ADR-0019). All colours are linear; the render
// target is sRGB. Depth is reverse-Z: 1 = near, 0 = far.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz = eye position, w = 1 perspective / 0 orthographic
    eye: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // viewing direction (eye -> target)
    cam_forward: vec4<f32>,
    // xyz = orbit target, w = camera distance
    orbit_target: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    body_color: vec4<f32>,
    // x = plate width, y = plate depth (mm, centred on origin),
    // z = show grid (0/1), w = show plate (0/1)
    grid: vec4<f32>,
    // x, y = viewport size in px, z = pixels per point
    viewport: vec4<f32>,
    grid_minor: vec4<f32>,
    grid_major: vec4<f32>,
    plate_fill: vec4<f32>,
    plate_border: vec4<f32>,
    axis_x: vec4<f32>,
    axis_y: vec4<f32>,
    restricted: vec4<f32>,
    // Plane of the grid (world XY or the active sketch plane): origin, u, v, n.
    grid_origin: vec4<f32>,
    grid_u: vec4<f32>,
    grid_v: vec4<f32>,
    grid_n: vec4<f32>,
};

// Bed polygons back to back: printable area, exclude zone, nozzle 1, nozzle 2
// (lengths in `counts`). Even-odd polygons in world XY (ADR-0021).
struct Bed {
    counts: vec4<u32>,
    points: array<vec4<f32>, 192>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<uniform> bed: Bed;

fn fullscreen_uv(i: u32) -> vec2<f32> {
    return vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
}

// ---------------------------------------------------------------- background

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_background(@builtin(vertex_index) i: u32) -> BgOut {
    let uv = fullscreen_uv(i);
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

fn view_dir_to_eye(world_pos: vec3<f32>) -> vec3<f32> {
    if (g.eye.w > 0.5) {
        return normalize(g.eye.xyz - world_pos);
    }
    return -g.cam_forward.xyz;
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) { n = -n; }
    let v = view_dir_to_eye(in.world_pos);
    let right = g.cam_right.xyz;
    let up = g.cam_up.xyz;

    // Key light from the upper left of the viewer, fill from the lower
    // right, sky/ground ambient: faces of different orientation stay
    // distinguishable from every view (CAD look, no shadows).
    let key = normalize(0.55 * v + 0.65 * up - 0.45 * right);
    let fill = normalize(0.6 * v - 0.4 * up + 0.5 * right);
    let diffuse = 0.68 * max(dot(n, key), 0.0) + 0.16 * max(dot(n, fill), 0.0);
    let ambient = mix(0.16, 0.26, 0.5 + 0.5 * n.z);
    let h = normalize(key + v);
    let spec = pow(max(dot(n, h), 0.0), 64.0) * 0.12;

    let color = g.body_color.rgb * (ambient + diffuse) + vec3<f32>(spec);
    return vec4<f32>(color, 1.0);
}

// ---------------------------------------------------------------- lines
// Each segment is one instance, expanded to a screen-aligned quad so lines
// have a real pixel width (wgpu lines are always 1 px).

struct LineIn {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color: vec4<f32>,
    // x = width in points, y = depth offset factor
    @location(3) style: vec2<f32>,
};

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Pulls a point slightly towards the viewer so edges win the depth test
// against the faces they lie on.
fn towards_viewer(p: vec3<f32>, amount: f32) -> vec3<f32> {
    return p + view_dir_to_eye(p) * amount * g.orbit_target.w;
}

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, in: LineIn) -> LineOut {
    let pa = towards_viewer(in.a, in.style.y);
    let pb = towards_viewer(in.b, in.style.y);
    let ca = g.view_proj * vec4<f32>(pa, 1.0);
    let cb = g.view_proj * vec4<f32>(pb, 1.0);

    var out: LineOut;
    out.color = in.color;
    if (ca.w <= 0.0 || cb.w <= 0.0) {
        out.pos = vec4<f32>(0.0, 0.0, -1.0, 1.0); // behind the camera: drop
        return out;
    }
    let size = g.viewport.xy;
    let sa = ca.xy / ca.w * size * 0.5;
    let sb = cb.xy / cb.w * size * 0.5;
    var dir = sb - sa;
    if (length(dir) < 1e-4) { dir = vec2<f32>(1.0, 0.0); }
    dir = normalize(dir);
    let normal = vec2<f32>(-dir.y, dir.x);
    let half_width = 0.5 * in.style.x * g.viewport.z;

    // Two triangles: corners (end, side) = (0,-), (0,+), (1,+), (0,-), (1,+), (1,-)
    var ends = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    var sides = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    let at_b = ends[vi];
    let clip = mix(ca, cb, at_b);
    // Extend slightly along the line so joints between segments close.
    let along = (at_b * 2.0 - 1.0) * half_width;
    let offset_px = normal * sides[vi] * half_width + dir * along;
    out.pos = vec4<f32>(clip.xy + offset_px / (size * 0.5) * clip.w, clip.z, clip.w);
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

// ---------------------------------------------------------------- grid
// Analytic, anti-aliased, infinite grid on the XY plane with build plate and
// X/Y axes. Spacing adapts to the zoom level in powers of ten.

struct GridOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

struct GridFrag {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@vertex
fn vs_grid(@builtin(vertex_index) i: u32) -> GridOut {
    let uv = fullscreen_uv(i);
    var out: GridOut;
    out.ndc = uv * 2.0 - 1.0;
    out.pos = vec4<f32>(out.ndc, 0.0, 1.0);
    return out;
}

fn unproject(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let p = g.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    return p.xyz / p.w;
}

// Coverage (0..1) of grid lines with the given spacing, ~1 px wide.
fn grid_lines(coord: vec2<f32>, spacing: f32) -> f32 {
    let c = coord / spacing;
    let w = max(fwidth(c), vec2<f32>(1e-6));
    let d = abs(fract(c - 0.5) - 0.5) / w;
    return 1.0 - min(min(d.x, d.y), 1.0);
}

// Coverage of a line at coordinate value 0, `px` pixels wide.
fn axis_line(v: f32, px: f32) -> f32 {
    let d = abs(v) / max(fwidth(v), 1e-6);
    return 1.0 - smoothstep(0.5 * px - 0.5, 0.5 * px + 0.5, d);
}

fn poly_inside(p: vec2<f32>, start: u32, count: u32) -> bool {
    var inside = false;
    if (count < 3u) { return false; }
    var j = start + count - 1u;
    for (var i = start; i < start + count; i = i + 1u) {
        let a = bed.points[i].xy;
        let b = bed.points[j].xy;
        if ((a.y > p.y) != (b.y > p.y)) && (p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x) {
            inside = !inside;
        }
        j = i;
    }
    return inside;
}

// Distance from p to the polygon outline (world units).
fn poly_distance(p: vec2<f32>, start: u32, count: u32) -> f32 {
    var d = 1e30;
    if (count < 2u) { return d; }
    var j = start + count - 1u;
    for (var i = start; i < start + count; i = i + 1u) {
        let a = bed.points[j].xy;
        let b = bed.points[i].xy;
        let ab = b - a;
        let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-12), 0.0, 1.0);
        d = min(d, length(p - (a + ab * t)));
        j = i;
    }
    return d;
}

fn over(dst: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    let a = src.a + dst.a * (1.0 - src.a);
    let rgb = (src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / max(a, 1e-6);
    return vec4<f32>(rgb, a);
}

@fragment
fn fs_grid(in: GridOut) -> GridFrag {
    let p0 = unproject(in.ndc, 1.0);
    let p1 = unproject(in.ndc, 0.5);
    let dir = p1 - p0;
    let plane_o = g.grid_origin.xyz;
    let plane_n = g.grid_n.xyz;
    let denom = dot(dir, plane_n);
    if (abs(denom) < 1e-12) { discard; }
    let t = dot(plane_o - p0, plane_n) / denom;
    if (t < 0.0) { discard; }
    let p = p0 + dir * t;
    // Local plane coordinates; equal to world XY for the default plane.
    let q = vec2<f32>(dot(p - plane_o, g.grid_u.xyz), dot(p - plane_o, g.grid_v.xyz));
    let target_rel = g.orbit_target.xyz - plane_o;
    let target_q = vec2<f32>(dot(target_rel, g.grid_u.xyz), dot(target_rel, g.grid_v.xyz));

    var out: GridFrag;
    let clip = g.view_proj * vec4<f32>(p, 1.0);
    out.depth = clip.z / clip.w;
    if (out.depth <= 0.0 || out.depth > 1.0) { discard; }

    // Fade towards the horizon and at grazing angles (avoids moiré).
    let dist = g.orbit_target.w;
    let from_target = length(q - target_q);
    let fade = (1.0 - smoothstep(3.0 * dist, 8.0 * dist, from_target))
        * smoothstep(0.02, 0.2, abs(dot(normalize(dir), plane_n)));

    var c = vec4<f32>(0.0);
    // World units per pixel, for pixel-wide outlines and anti-aliasing.
    let px = max(max(fwidth(q.x), fwidth(q.y)), 1e-6);
    let n_plate = bed.counts.x;
    let n_excl = bed.counts.y;
    let n_e1 = bed.counts.z;
    let n_e2 = bed.counts.w;
    let show_plate = g.grid.w > 0.5;
    let inside = show_plate && poly_inside(q, 0u, n_plate);
    if (inside) {
        c = over(c, g.plate_fill);
        // Zones only one nozzle reaches (multi-nozzle printers): tinted.
        if (n_e1 >= 3u && n_e2 >= 3u) {
            let both = poly_inside(q, n_plate + n_excl, n_e1)
                && poly_inside(q, n_plate + n_excl + n_e1, n_e2);
            if (!both) {
                c = over(c, vec4<f32>(g.restricted.rgb, 0.11));
            }
        }
        // Non-printable zone: hatched amber.
        if (poly_inside(q, n_plate, n_excl)) {
            let stripe_period = 4.0;
            let s = fract((q.x + q.y) / stripe_period);
            let edge = px / stripe_period;
            let stripe = smoothstep(0.5 - edge, 0.5 + edge, s);
            c = over(c, vec4<f32>(g.restricted.rgb, 0.18 + 0.32 * stripe));
        }
    }
    if (g.grid.z > 0.5) {
        let lod = log2(max(dist, 1e-3)) / log2(10.0) - 1.0;
        let base = pow(10.0, floor(lod));
        let blend = fract(lod);
        let minor = grid_lines(q, base) * (1.0 - blend);
        let mid = grid_lines(q, base * 10.0);
        let major = grid_lines(q, base * 100.0);
        let a_minor = g.grid_minor.a * max(minor, mid * blend);
        let a_major = g.grid_major.a * max(mid * (1.0 - blend), major);
        c = over(c, vec4<f32>(g.grid_minor.rgb, a_minor * fade));
        c = over(c, vec4<f32>(g.grid_major.rgb, a_major * fade));
    }
    if (show_plate) {
        // Outlines: plate in grey, exclude zone in amber (~1.5 px wide).
        let d_plate = poly_distance(q, 0u, n_plate) / px;
        c = over(c, vec4<f32>(g.plate_border.rgb, g.plate_border.a * (1.0 - smoothstep(0.5, 1.5, d_plate))));
        let d_excl = poly_distance(q, n_plate, n_excl) / px;
        c = over(c, vec4<f32>(g.restricted.rgb, 0.9 * (1.0 - smoothstep(0.5, 1.5, d_excl))));
    }
    if (g.grid.z > 0.5) {
        let ax = axis_line(q.y, 1.6 * g.viewport.z);
        let ay = axis_line(q.x, 1.6 * g.viewport.z);
        c = over(c, vec4<f32>(g.axis_x.rgb, g.axis_x.a * ax * fade));
        c = over(c, vec4<f32>(g.axis_y.rgb, g.axis_y.a * ay * fade));
    }
    if (c.a < 0.003) { discard; }
    out.color = c;
    return out;
}
