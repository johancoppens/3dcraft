// The lines over the picture of a viewport: the edges of the bodies, the
// sketches, the world axes and the sketch points. Each segment is a quad
// in screen space, wide enough for the line and its feather; the fragment
// shader inks it by the distance of the pixel's centre to the segment, as
// `craft3d_ui_egui::viewport::draw_line` does on the CPU, and tests it
// against the scene's depth buffer the way the CPU viewport tests its
// overlay: a point is behind a body only when the surface under its own
// pixel and under the eight around it is nearer than the point, with some
// slack. A hidden edge is left out; a hidden sketch is drawn dimmed.
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
    // xyz: the eye, relative to the origin the line positions are stored
    // against.
    eye: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<uniform> lines: Lines;
@group(0) @binding(2) var depth_texture: {{DEPTH}};

// Tested against the depth buffer.
const TESTED: u32 = 1u;
// Drawn dimmed where hidden, instead of left out.
const DIMMED: u32 = 2u;

struct VertexIn {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    // x: 0 at the `a` end, 1 at the `b` end. y: -1 or 1, the side.
    @location(2) corner: vec2<f32>,
    // rgba8, red in the low byte.
    @location(3) color: u32,
    // In pixels.
    @location(4) width: f32,
    @location(5) flags: u32,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The view distance along the segment: with the clip w set to the
    // distance of each end, the hardware interpolates it as the CPU's
    // `Projector::depth_between` does (evenly in its reciprocal).
    @location(0) dist: f32,
    @location(1) @interpolate(flat) pa: vec2<f32>,
    @location(2) @interpolate(flat) pb: vec2<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) half: f32,
    @location(5) @interpolate(flat) flags: u32,
}

// Camera coordinates: right, up, and the distance along the view.
fn view(p: vec3<f32>) -> vec3<f32> {
    let rel = p - lines.eye.xyz;
    return vec3<f32>(dot(rel, scene.right.xyz), dot(rel, scene.up.xyz), dot(rel, scene.forward.xyz));
}

// Pixel coordinates (x right, y down from the top-left corner).
fn pixel(v: vec3<f32>) -> vec2<f32> {
    var s = 1.0;
    if scene.forward.w > 0.5 {
        s = 1.0 / v.z;
    }
    let nx = v.x * scene.right.w * s;
    let ny = v.y * scene.up.w * s;
    return vec2<f32>((nx + 1.0) * 0.5 * lines.size.x, (1.0 - ny) * 0.5 * lines.size.y);
}

fn unpack(color: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(color & 255u),
        f32((color >> 8u) & 255u),
        f32((color >> 16u) & 255u),
        f32((color >> 24u) & 255u),
    ) / 255.0;
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    var va = view(in.a);
    var vb = view(in.b);
    let perspective = scene.forward.w > 0.5;
    if perspective {
        // Projector::segment: the part in front of the eye, cut a little in
        // front of the eye plane so the cut end has a place on the screen.
        let near = max(max(va.z, vb.z) * 1e-4, 2e-9);
        if va.z < near && vb.z < near {
            // Nothing of it is in front of the eye: outside the clip volume.
            out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
            return out;
        }
        if va.z < near {
            va = mix(va, vb, (near - va.z) / (vb.z - va.z));
        } else if vb.z < near {
            vb = mix(vb, va, (near - vb.z) / (va.z - vb.z));
        }
    }
    let pa = pixel(va);
    let pb = pixel(vb);
    var d = pb - pa;
    let len = length(d);
    if len > 0.0 {
        d = d / len;
    } else {
        d = vec2<f32>(1.0, 0.0);
    }
    let n = vec2<f32>(-d.y, d.x);
    let half = clamp(in.width * 0.5, 0.35, 64.0);
    // The ink reaches half + 0.5 pixel from the centre line; the quad
    // reaches one pixel further, so every inked pixel's centre is inside.
    let pad = half + 1.5;
    var end = pa;
    var along = -1.0;
    var w = va.z;
    if in.corner.x > 0.5 {
        end = pb;
        along = 1.0;
        w = vb.z;
    }
    let p = end + d * (along * pad) + n * (in.corner.y * pad);
    let ndc = vec2<f32>(p.x / lines.size.x * 2.0 - 1.0, 1.0 - p.y / lines.size.y * 2.0);
    if perspective {
        out.clip = vec4<f32>(ndc * w, 0.0, w);
    } else {
        out.clip = vec4<f32>(ndc, 0.0, 1.0);
    }
    out.dist = w;
    out.pa = pa;
    out.pb = pb;
    out.color = unpack(in.color);
    out.half = half;
    out.flags = in.flags;
    return out;
}

// The view distance of the surface under pixel `p`: the furthest of its
// samples, so a line on a surface is hidden only when every sample is
// nearer; infinite where nothing was drawn.
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
        // z = scale * (1 - near / dist), as the scene pass writes it.
        let d = 1.0 - z / scene.depth.y;
        if d <= 0.0 {
            return 1e38;
        }
        return scene.depth.x / d;
    }
    return z / scene.depth.y + scene.depth.x;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.clip.xy;
    // The distance from the pixel's centre to the segment.
    let ab = in.pb - in.pa;
    let l2 = dot(ab, ab);
    var t = 0.0;
    if l2 > 0.0 {
        t = clamp(dot(p - in.pa, ab) / l2, 0.0, 1.0);
    }
    let distance = length(p - (in.pa + ab * t));
    let alpha = clamp(in.half + 0.5 - distance, 0.0, 1.0);
    if alpha <= 0.0 {
        discard;
    }
    var opacity = 1.0;
    if (in.flags & TESTED) != 0u {
        let size = vec2<i32>(i32(lines.size.x), i32(lines.size.y));
        let c = vec2<i32>(floor(p));
        let slack = lines.size.z;
        // The surface under the pixel and under the eight around it: the
        // point is hidden only when the furthest of them is nearer than
        // the point by more than the slack and the spread of the nine.
        // The spread is what the surface changes by over the
        // neighbourhood: a line on a surface, also in the hollow where two
        // surfaces meet, is within it; a line behind a body is beyond it.
        // (The CPU viewport tests against its frame's distance, which is
        // interpolated in screen space and so lies further than the true
        // surface by about as much on a large triangle; the GPU tests
        // against the true depth and allows the spread instead.)
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
        // A surface at or behind the point does not hide it.
        let hidden = furthest + slack + (furthest - nearest) < in.dist;
        if hidden {
            if (in.flags & DIMMED) == 0u {
                discard;
            }
            opacity = lines.look.x;
        }
    }
    return vec4<f32>(in.color.rgb, alpha * opacity);
}
