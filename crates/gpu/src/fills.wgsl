// The translucent squares of datum planes, drawn in the lines pass before
// the lines. Each fill is one triangle over the whole picture; per pixel
// the fragment shader casts the ray through the pixel's centre, meets the
// plane, keeps the pixel when the point lies inside the square, in front
// of the eye and not behind a body (the lines' depth test), and returns
// the fill's colour with its opacity. That is
// `craft3d_ui_egui::viewport::fill_datum_planes` on the CPU, which decides
// at the same pixel centres; a fragment covers every sample of its pixel,
// so a multisampled pixel resolves to the one value the CPU computes.
//
// The source is a template: `{{DEPTH}}` and `{{LOAD}}` are filled in for
// a multisampled or a single-sampled depth texture.

struct Scene {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    // x: near distance. y: depth scale, as in scene.wgsl.
    depth: vec4<f32>,
    base: vec4<f32>,
    shade: vec4<f32>,
}

struct Lines {
    // xy: the image's width and height in pixels. z: the slack of the
    // depth test in model units. w: the number of samples of the depth
    // texture.
    size: vec4<f32>,
    // x: the opacity of what is hidden, 0 to 1.
    look: vec4<f32>,
    // xyz: the eye, relative to the origin the positions are stored
    // against.
    eye: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<uniform> lines: Lines;
@group(0) @binding(2) var depth_texture: {{DEPTH}};

struct FillIn {
    @builtin(vertex_index) index: u32,
    // The centre of the square, relative to the origin.
    @location(0) center: vec3<f32>,
    // Half sides: the square is center ± u ± v.
    @location(1) u: vec3<f32>,
    @location(2) v: vec3<f32>,
    // rgba8, red in the low byte; alpha is the fill's opacity.
    @location(3) color: u32,
}

struct FillOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) center: vec3<f32>,
    @location(1) @interpolate(flat) u: vec3<f32>,
    @location(2) @interpolate(flat) v: vec3<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
}

fn unpack(color: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(color & 255u),
        f32((color >> 8u) & 255u),
        f32((color >> 16u) & 255u),
        f32((color >> 24u) & 255u),
    ) / 255.0;
}

// A direction in camera coordinates: right, up, along the view.
fn turn(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(d, scene.right.xyz), dot(d, scene.up.xyz), dot(d, scene.forward.xyz));
}

@vertex
fn vs_fill(in: FillIn) -> FillOut {
    var out: FillOut;
    // One triangle over the whole picture.
    let corner = in.index % 3u;
    var xy = vec2<f32>(-1.0, -1.0);
    if corner == 1u {
        xy = vec2<f32>(3.0, -1.0);
    } else if corner == 2u {
        xy = vec2<f32>(-1.0, 3.0);
    }
    out.clip = vec4<f32>(xy, 0.0, 1.0);
    // Camera coordinates, relative to the eye.
    out.center = turn(in.center - lines.eye.xyz);
    out.u = turn(in.u);
    out.v = turn(in.v);
    out.color = unpack(in.color);
    return out;
}

// The view distance of the surface under pixel `p`, as in lines.wgsl.
fn surface_at(p: vec2<i32>) -> f32 {
    let samples = i32(lines.size.w);
    var z = 0.0;
    for (var s = 0; s < samples; s = s + 1) {
        z = max(z, {{LOAD}});
    }
    if z >= 1.0 {
        return 1e38;
    }
    if scene.forward.w > 0.5 {
        let d = 1.0 - z / scene.depth.y;
        if d <= 0.0 {
            return 1e38;
        }
        return scene.depth.x / d;
    }
    return z / scene.depth.y + scene.depth.x;
}

@fragment
fn fs_fill(in: FillOut) -> @location(0) vec4<f32> {
    let p = in.clip.xy;
    // The ray through the pixel's centre, in camera coordinates: along the
    // view it advances by 1 per unit of `t`, so `t` is the view distance.
    let nx = p.x / lines.size.x * 2.0 - 1.0;
    let ny = 1.0 - p.y / lines.size.y * 2.0;
    let across = vec3<f32>(nx / scene.right.w, ny / scene.up.w, 0.0);
    var start = vec3<f32>(0.0, 0.0, 0.0);
    var direction = vec3<f32>(0.0, 0.0, 1.0);
    let perspective = scene.forward.w > 0.5;
    if perspective {
        direction = direction + across;
    } else {
        start = across;
    }
    let normal = cross(in.u, in.v);
    let along = dot(direction, normal);
    if along == 0.0 {
        discard;
    }
    let t = dot(in.center - start, normal) / along;
    if perspective && !(t > 1e-9) {
        discard;
    }
    let r = start + direction * t - in.center;
    if abs(dot(r, in.u) / dot(in.u, in.u)) > 1.0 || abs(dot(r, in.v) / dot(in.v, in.v)) > 1.0 {
        discard;
    }
    // Not behind a body: the lines' test (see lines.wgsl).
    let size = vec2<i32>(i32(lines.size.x), i32(lines.size.y));
    let c = vec2<i32>(floor(p));
    var furthest = 0.0;
    var nearest = 1e38;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let q = clamp(c + vec2<i32>(dx, dy), vec2<i32>(0), size - vec2<i32>(1));
            let surface = surface_at(q);
            furthest = max(furthest, surface);
            nearest = min(nearest, surface);
        }
    }
    if furthest + lines.size.z + (furthest - nearest) < t {
        discard;
    }
    return in.color;
}
