// The lines over the picture of a viewport: the edges of the bodies, the
// sketches, the world axes and the sketch points. Each segment is one
// instance, drawn as a quad in screen space wide enough for the line and
// its feather; the fragment shader inks it by the distance of the pixel's
// centre to the segment, as `craft3d_ui_egui::viewport::draw_line` does on
// the CPU, and tests the point of the segment nearest to the pixel's
// centre against the scene's depth buffer with the CPU viewport's rule
// (depth_test.wgsl). A hidden edge is left out; a hidden sketch is drawn
// dimmed.
//
// An edge of a body (`EDGE`) is drawn as the CPU viewport draws it: in
// its outline's colour and width where the surface on one side of it
// steps back (`viewport::surface_steps_back`), and faded where it is short
// on screen (`viewport::fade_short`, from the box of its edge, `EXTENT`)
// or where the edges crowd (`viewport::edge_fade` of the density in
// `tiles`, which the passes `vs_ink`/`fs_ink` and edge_tiles.wgsl made
// before this one, as `viewport::EdgeDensity` does).
//
// The source is a template: depth_test.wgsl is pasted in at its marker. It
// reads the distances of distance.wgsl, made from the depth buffer once
// per frame.

struct Scene {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    // x, y: the depth numbers of scene.wgsl.
    depth: vec4<f32>,
    base: vec4<f32>,
    shade: vec4<f32>,
}

struct Lines {
    // xy: the image's width and height in pixels. z: the slack of the
    // depth test in model units. w: the number of samples of the depth
    // texture.
    size: vec4<f32>,
    // x: the opacity of what is hidden, 0 to 1. y: the grain of the depth
    // test, in pixels (`craft3d_render::SUBPIXEL_GRAIN`). z: the side of a
    // tile of the density, in pixels. w: the widths' scale (`EdgeLook::ink`),
    // applied after their least width.
    look: vec4<f32>,
    // xyz: the eye, relative to the origin the line positions are stored
    // against.
    eye: vec4<f32>,
    // How an outline is told (`viewport::OUTLINE_NEAR`, `OUTLINE_FAR`,
    // `OUTLINE_STEP`, `OUTLINE_SLOPE`).
    outline: vec4<f32>,
    // The fade of an edge: x, y: short and long extent in pixels; z: the
    // share of a tile the edges' ink may cover together.
    fade: vec4<f32>,
    // x: the opacity a fully faded edge keeps, 0 to 1. y, z, w: the lens
    // the edges follow (`EdgeLook::soft`): the blur radius in pixels far
    // away, the distance in focus and the largest radius; y 0 for none.
    more: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<uniform> lines: Lines;
// The density per tile, and the distances, as the bits of `f32`.
@group(0) @binding(3) var tiles: texture_2d<u32>;
@group(0) @binding(4) var distance_texture: texture_2d<u32>;

// Tested against the depth buffer.
const TESTED: u32 = 1u;
// Drawn dimmed where hidden, instead of left out.
const DIMMED: u32 = 2u;
// An edge of a body: its outline told, faded, counted in the density.
const EDGE: u32 = 4u;
// `low` and `high` are the corners of the box of the segment's edge.
const EXTENT: u32 = 8u;

// Far beyond any distance: a surface that recedes without end.
const FAR_AWAY: f32 = 3.0e38;

struct VertexIn {
    @builtin(vertex_index) index: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    // rgba8, red in the low byte.
    @location(2) color: u32,
    // In pixels.
    @location(3) width: f32,
    @location(4) flags: u32,
    @location(5) outline_color: u32,
    @location(6) outline_width: f32,
    @location(7) low: vec3<f32>,
    @location(8) high: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(1) @interpolate(flat) pa: vec2<f32>,
    @location(2) @interpolate(flat) pb: vec2<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) half: f32,
    @location(5) @interpolate(flat) flags: u32,
    // The view distances of the two ends (after the cut in front of the
    // eye), for the depth of the point of the segment under a pixel.
    @location(6) @interpolate(flat) ends: vec2<f32>,
    @location(7) @interpolate(flat) outline_color: vec4<f32>,
    @location(8) @interpolate(flat) outline_half: f32,
    // What the edge keeps of its ink for being short on screen.
    @location(9) @interpolate(flat) long: f32,
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

// Hermite's smooth step from 0 at `lo` to 1 at `hi`, a step at one point
// when the two are equal (`viewport::smooth_step`).
fn smooth_step(lo: f32, hi: f32, x: f32) -> f32 {
    if !(hi > lo) {
        return select(0.0, 1.0, x >= hi);
    }
    let t = clamp((x - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The extent of the box from `low` to `high` on screen, in pixels
// (`viewport::extent_on_screen`): infinite when a corner is not in front
// of the eye.
fn extent(low: vec3<f32>, high: vec3<f32>) -> f32 {
    let vl = view(low);
    let vh = view(high);
    if scene.forward.w > 0.5 && !(vl.z > 1e-9 && vh.z > 1e-9) {
        return FAR_AWAY;
    }
    return length(pixel(vh) - pixel(vl));
}

// The corners of the quad of a segment, as (end, side).
fn corner(index: u32) -> vec2<f32> {
    switch index {
        case 0u, 3u: { return vec2<f32>(0.0, -1.0); }
        case 1u: { return vec2<f32>(1.0, -1.0); }
        case 2u, 4u: { return vec2<f32>(1.0, 1.0); }
        default: { return vec2<f32>(0.0, 1.0); }
    }
}

// The quad of a segment. For the ink of the density (`ink`), only an edge
// of a body, at its own width.
fn quad(in: VertexIn, ink: bool) -> VertexOut {
    var out: VertexOut;
    let edge = (in.flags & EDGE) != 0u;
    if ink && !edge {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return out;
    }
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
    // The least width in the viewport's own pixels, then scaled to the
    // picture's (`EdgeLook::ink`: 1 unless it is drawn smaller).
    let half = clamp(in.width * 0.5, 0.35, 64.0) * lines.look.w;
    let outline_half = clamp(in.outline_width * 0.5, 0.35, 64.0) * lines.look.w;
    var widest = half;
    if edge && !ink {
        widest = max(half, outline_half);
    }
    // The ink reaches half + 0.5 pixel from the centre line; the quad
    // reaches one pixel further, so every inked pixel's centre is inside.
    let pad = widest + 1.5;
    let c = corner(in.index % 6u);
    var end = pa;
    var along = -1.0;
    var w = va.z;
    if c.x > 0.5 {
        end = pb;
        along = 1.0;
        w = vb.z;
    }
    let p = end + d * (along * pad) + n * (c.y * pad);
    let ndc = vec2<f32>(p.x / lines.size.x * 2.0 - 1.0, 1.0 - p.y / lines.size.y * 2.0);
    if perspective {
        out.clip = vec4<f32>(ndc * w, 0.0, w);
    } else {
        out.clip = vec4<f32>(ndc, 0.0, 1.0);
    }
    out.pa = pa;
    out.pb = pb;
    out.color = unpack(in.color);
    out.half = half;
    out.flags = in.flags;
    out.ends = vec2<f32>(va.z, vb.z);
    out.outline_color = unpack(in.outline_color);
    out.outline_half = outline_half;
    out.long = 1.0;
    if (in.flags & EXTENT) != 0u {
        out.long = smooth_step(lines.fade.x, lines.fade.y, extent(in.low, in.high));
    }
    return out;
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    return quad(in, false);
}

@vertex
fn vs_ink(in: VertexIn) -> VertexOut {
    return quad(in, true);
}

{{DEPTH_TEST}}

// The view distance at the fraction `t` of the segment: evenly in its
// reciprocal in perspective (the CPU's `Projector::depth_between`).
fn depth_between(ends: vec2<f32>, t: f32) -> f32 {
    if scene.forward.w > 0.5 && ends.x > 0.0 && ends.y > 0.0 {
        let inverse = 1.0 / ends.x + (1.0 / ends.y - 1.0 / ends.x) * t;
        if inverse > 0.0 {
            return 1.0 / inverse;
        }
    }
    return ends.x + (ends.y - ends.x) * t;
}

// The surface at pixel position `at`: the bilinear interpolation of the
// four samples around it, clamped at the border of the image; nothing
// where one of them shows nothing (`DepthMap::surface` of the viewport).
fn surface_between(at: vec2<f32>) -> Surface {
    let size = vec2<i32>(i32(lines.size.x), i32(lines.size.y));
    let u = at - vec2<f32>(0.5, 0.5);
    let cell = vec2<i32>(floor(u));
    let f = u - vec2<f32>(cell);
    let top = size - vec2<i32>(1);
    let a = surface_at(clamp(cell, vec2<i32>(0), top));
    let b = surface_at(clamp(cell + vec2<i32>(1, 0), vec2<i32>(0), top));
    let c = surface_at(clamp(cell + vec2<i32>(0, 1), vec2<i32>(0), top));
    let d = surface_at(clamp(cell + vec2<i32>(1, 1), vec2<i32>(0), top));
    if !(a.drawn && b.drawn && c.drawn && d.drawn) {
        return Surface(0.0, false);
    }
    return Surface(mix(mix(a.distance, b.distance, f.x), mix(c.distance, d.distance, f.x), f.y), true);
}

// Whether the surface beside an edge, at `near` and `far` across it, steps
// back from the edge at `depth` by more than `allowance`
// (`viewport::surface_steps_back`).
fn steps_back(depth: f32, near: Surface, far: Surface, allowance: f32) -> bool {
    if !near.drawn {
        return true;
    }
    var reached = near.distance;
    var slope = abs(near.distance - depth) / lines.outline.x;
    if far.drawn {
        if scene.forward.w > 0.5 && near.distance > 0.0 && far.distance > 0.0 {
            let inverse = 2.0 / near.distance - 1.0 / far.distance;
            reached = FAR_AWAY;
            if inverse > 0.0 {
                reached = 1.0 / inverse;
            }
        } else {
            reached = 2.0 * near.distance - far.distance;
        }
        slope = abs(far.distance - near.distance) / (lines.outline.y - lines.outline.x);
    }
    return reached - depth > allowance + slope * lines.outline.w;
}

// Whether the point `at` of an edge at `depth`, running along `direction`
// on the screen, is an outline: the surface on either side steps back.
fn outline_at(at: vec2<f32>, depth: f32, direction: vec2<f32>) -> bool {
    // How long a pixel is at the edge's depth (`Projector::footprint`).
    var footprint = 2.0 / (scene.up.w * lines.size.y);
    if scene.forward.w > 0.5 {
        footprint = footprint * depth;
    }
    let allowance = lines.outline.z * footprint + lines.size.z;
    let normal = vec2<f32>(-direction.y, direction.x);
    for (var k = 0; k < 2; k = k + 1) {
        let side = select(-1.0, 1.0, k == 0);
        let near = surface_between(at + normal * (side * lines.outline.x));
        let far = surface_between(at + normal * (side * lines.outline.y));
        if steps_back(depth, near, far, allowance) {
            return true;
        }
    }
    return false;
}

// The density of the edges at `p` (`EdgeDensity::at`): the tiles' means
// blended bilinearly between their centres, clamped at the border.
fn density_at(p: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(tiles));
    let u = p / lines.look.z - vec2<f32>(0.5, 0.5);
    let cell = vec2<i32>(floor(u));
    let f = u - vec2<f32>(cell);
    let top = size - vec2<i32>(1);
    let a = bitcast<f32>(textureLoad(tiles, clamp(cell, vec2<i32>(0), top), 0).r);
    let b = bitcast<f32>(textureLoad(tiles, clamp(cell + vec2<i32>(1, 0), vec2<i32>(0), top), 0).r);
    let c = bitcast<f32>(textureLoad(tiles, clamp(cell + vec2<i32>(0, 1), vec2<i32>(0), top), 0).r);
    let d = bitcast<f32>(textureLoad(tiles, clamp(cell + vec2<i32>(1, 1), vec2<i32>(0), top), 0).r);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The share of its ink an edge `width` pixels wide keeps at view distance
// `z` where the lens blurs it (`viewport::EdgeBlur::fade`): 1 without a
// lens.
fn lens_fade(z: f32, width: f32) -> f32 {
    if !(lines.more.y > 0.0) {
        return 1.0;
    }
    // The cap at or behind the eye, none for a distance that is not a
    // number (`EdgeBlur::radius`).
    var c = 0.0;
    if z > 0.0 {
        c = min(lines.more.y * abs(z - lines.more.z) / z, lines.more.w);
    } else if z <= 0.0 {
        c = lines.more.w;
    }
    if !(c > 0.0) {
        return 1.0;
    }
    let w = max(width, 0.0);
    return w / (w + 1.5707964 * c);
}

// The share of its opacity an edge keeps (`viewport::edge_fade`).
fn edge_fade(long: f32, density: f32) -> f32 {
    let least = clamp(lines.more.x, 0.0, 1.0);
    var sparse = 1.0;
    if density > lines.fade.z {
        sparse = lines.fade.z / density;
    }
    return least + (1.0 - least) * clamp(long * sparse, 0.0, 1.0);
}

struct Foot {
    // The point of the segment nearest to the pixel's centre.
    at: vec2<f32>,
    t: f32,
    distance: f32,
}

fn foot(in: VertexOut, p: vec2<f32>) -> Foot {
    let ab = in.pb - in.pa;
    let l2 = dot(ab, ab);
    var t = 0.0;
    if l2 > 0.0 {
        t = clamp(dot(p - in.pa, ab) / l2, 0.0, 1.0);
    }
    let at = in.pa + ab * t;
    return Foot(at, t, length(p - at));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.clip.xy;
    let f = foot(in, p);
    let edge = (in.flags & EDGE) != 0u;
    var widest = in.half;
    if edge {
        widest = max(in.half, in.outline_half);
    }
    if widest + 0.5 - f.distance <= 0.0 {
        discard;
    }
    var opacity = 1.0;
    var hidden = false;
    let depth = depth_between(in.ends, f.t);
    if (in.flags & TESTED) != 0u {
        // The point of the segment nearest to the pixel's centre, at its
        // own depth: on the line itself, as the CPU tests its pieces, so
        // that a line on a sloping face is not tested beside itself.
        if surface_hides(f.at, depth) {
            if (in.flags & DIMMED) == 0u {
                discard;
            }
            hidden = true;
            opacity = lines.look.x;
        }
    }
    var color = in.color;
    var half = in.half;
    if edge {
        let ab = in.pb - in.pa;
        let length_ab = length(ab);
        if !hidden && length_ab > 0.0 && outline_at(f.at, depth, ab / length_ab) {
            color = in.outline_color;
            half = in.outline_half;
        }
        opacity = opacity * edge_fade(in.long, density_at(p)) * lens_fade(depth, half * 2.0);
    }
    let alpha = clamp(half + 0.5 - f.distance, 0.0, 1.0);
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(color.rgb, alpha * opacity);
}

// The ink of an edge of a body in sight, at its own width, summed per
// pixel by the blend: what `EdgeDensity` counts on the CPU.
@fragment
fn fs_ink(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.clip.xy;
    let f = foot(in, p);
    let alpha = clamp(in.half + 0.5 - f.distance, 0.0, 1.0);
    if alpha <= 0.0 {
        discard;
    }
    if (in.flags & TESTED) != 0u && surface_hides(f.at, depth_between(in.ends, f.t)) {
        discard;
    }
    return vec4<f32>(alpha, 0.0, 0.0, 0.0);
}
