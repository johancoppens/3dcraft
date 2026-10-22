// The scene pass: one sample per target pixel, two colour targets (the shaded
// colour; the face id with the view distance) and a depth buffer. It follows
// `craft3d_render::render` step by step; the comments name the line of the
// CPU renderer each step mirrors.
//
// The source is a template (`craft3d_gpu::renderer::scene_source`): on a
// backend whose clip space has its depth in -1..1 (OpenGL), where the
// `0..1` depth of `vertex` is converted by `z * 2 - w` and that conversion
// loses the key's precision, every fragment entry writes the key itself
// as the fragment's depth (the marker FRAG_DEPTH declares the output,
// SET_DEPTH sets it, STORED_DEPTH is the number the depth test stored
// for a fragment); elsewhere the markers are empty and the rasteriser's
// own depth is the key.

struct Scene {
    // xyz: camera right axis. w: clip x per model unit (1 / (half * aspect)).
    right: vec4<f32>,
    // xyz: camera up axis. w: clip y per model unit (1 / half).
    up: vec4<f32>,
    // xyz: view direction. w: 1 for perspective, 0 for orthographic.
    forward: vec4<f32>,
    // xyz: the eye, relative to the mesh origin.
    eye: vec4<f32>,
    // xyz: unit vector towards the light. w: 1 when there is one.
    light: vec4<f32>,
    // x: the near distance in perspective, the far one in orthographic.
    // y: the depth scale of an orthographic view (see vs_main).
    depth: vec4<f32>,
    // rgb: the base colour in 0..255.
    base: vec4<f32>,
    // ambient, diffuse, specular * 255, shininess.
    shade: vec4<f32>,
    // rgb: the base colour of a highlighted face in 0..255
    // (`RenderParams::highlight`).
    highlight: vec4<f32>,
    // x, y: the ambient term and the diffuse weight of a highlighted face.
    highlight_shade: vec4<f32>,
    // The highlighted faces: eight runs of face ids, two per vector, each
    // as first and last. A run whose first is above its last is empty.
    // Ids as floats: the renderer refuses a highlight above 2^24.
    runs: array<vec4<f32>, 4>,
    // The section plane (`cut.wgsl`). xyz: its unit normal, into the half
    // that is cut away. w: its offset along the normal from the mesh
    // origin.
    section: vec4<f32>,
    // rgb: the colour of the cap, as bytes over 255. w: how near to the
    // plane a triangle parallel to it lies in it (`Cut::snap`).
    section_cap: vec4<f32>,
    // xyz: the corners of the box the cap is drawn in, relative to the
    // mesh origin.
    section_lo: vec4<f32>,
    section_hi: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) flat_normal: vec3<f32>,
    @location(3) face: u32,
    // rgb in the low three bytes; the high byte is 1 when the face has a
    // colour of its own (`craft3d_render::FaceColors`).
    @location(4) color: u32,
    // The metallic byte in the low byte, the highlight's exponent above it
    // (0: the plain one): `craft3d_gpu::mesh::finish_word`.
    @location(5) finish: u32,
}

struct VertexOut {
    // Invariant: the opaque scene in two steps (`renderer::Step`) compares
    // the depth of one pipeline with that of another for equality.
    @builtin(position) @invariant clip: vec4<f32>,
    // From the eye to the surface point, in model axes.
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) flat_normal: vec3<f32>,
    @location(3) @interpolate(flat) face: u32,
    // The view distance of the surface point, interpolated
    // perspective-correct like the position (the default for a varying):
    // the CPU renderer's `Frame::depth`.
    @location(4) dist: f32,
    @location(5) @interpolate(flat) color: u32,
    @location(6) @interpolate(flat) finish: u32,
}

fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // View::project: rel = p - eye, then the three dot products.
    let rel = in.position - scene.eye.xyz;
    let cx = dot(rel, scene.right.xyz);
    let cy = dot(rel, scene.up.xyz);
    let dist = dot(rel, scene.forward.xyz);
    let x = cx * scene.right.w;
    let y = cy * scene.up.w;
    // The depth key must be affine in screen space and ordered like the CPU
    // key (-1 / dist in perspective, dist in orthographic), here reversed:
    // 1 at the near plane, falling towards 0 far away, and the depth test
    // keeps the greater. A float holds near / dist to the same relative
    // precision at every distance, so a near plane right in front of the
    // eye (the eye inside a large model) costs nothing further on; with
    // depth rising to 1 far away, all the precision of the float near 0
    // would sit at the near plane and far faces a few centimetres apart
    // would fight.
    if scene.forward.w > 0.5 {
        // z / w = near / dist: 1 at near, towards 0 at infinity (no far
        // plane), affine in 1 / dist. depth.x = near.
        out.clip = vec4<f32>(x, y, scene.depth.x, dist);
    } else {
        // (far - dist) / (far - near): 1 at near, 0 at far. depth.x = far,
        // depth.y = 1 / (far - near).
        out.clip = vec4<f32>(x, y, (scene.depth.x - dist) * scene.depth.y, 1.0);
    }
    out.dist = dist;
    out.rel = rel;
    out.normal = in.normal;
    out.flat_normal = in.flat_normal;
    out.face = in.face;
    out.color = in.color;
    out.finish = in.finish;
    return out;
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    return vertex(in);
}

struct FragmentOut {
    @location(0) color: vec4<f32>,
    // x: the face id. y: the bits of the view distance as an f32. An
    // integer target is renderable on every backend and keeps both exactly.
    @location(1) data: vec2<u32>,
    {{FRAG_DEPTH}}
}

// The colour alone: the picture a viewport shows.
struct ColorOut {
    @location(0) color: vec4<f32>,
    {{FRAG_DEPTH}}
}

// The data alone: a viewport's capture.
struct DataOut {
    @location(0) data: vec2<u32>,
    {{FRAG_DEPTH}}
}

// The depth key of `vertex` for a fragment at view distance `dist`: 1 at
// the near plane, towards 0 far away, as the rasteriser interpolates it
// from the corners (the key is affine in screen space, `dist` is
// interpolated perspective-correct, and the arithmetic is the same `f32`
// division). Clamped: a fragment on the near plane rounds past 1.
fn depth_key(dist: f32) -> f32 {
    var key = 0.0;
    if scene.forward.w > 0.5 {
        key = scene.depth.x / dist;
    } else {
        key = (scene.depth.x - dist) * scene.depth.y;
    }
    return clamp(key, 0.0, 1.0);
}

// The number the depth test stored for a fragment with the rasteriser's
// depth `clip_z` at view distance `dist`: the depth it wrote.
fn stored_depth(clip_z: f32, dist: f32) -> f32 {
    return {{STORED_DEPTH}};
}

// x to a non-negative integer power by squaring: exact where `pow` is
// implementation-defined (pow(0, n) is NaN on some drivers).
fn powi(x: f32, n: u32) -> f32 {
    var result = 1.0;
    var base = x;
    var e = n;
    loop {
        if e == 0u {
            break;
        }
        if (e & 1u) == 1u {
            result = result * base;
        }
        base = base * base;
        e = e >> 1u;
    }
    return result;
}

// The face id and the bits of the view distance of a fragment.
fn data_of(in: VertexOut) -> vec2<u32> {
    return vec2<u32>(in.face, bitcast<u32>(in.dist));
}

// True when the face is one of the highlighted ones.
fn highlighted(face: u32) -> bool {
    let id = f32(face);
    var found = false;
    for (var k = 0; k < 4; k++) {
        let run = scene.runs[k];
        if (id >= run.x && id <= run.y) || (id >= run.z && id <= run.w) {
            found = true;
        }
    }
    return found;
}

// The shaded colour of a fragment, as the bytes of the picture over 255.
fn shade_of(in: VertexOut) -> vec4<f32> {
    // render(): a highlighted face is shaded from its own base colour.
    // render_colored(): a face with a colour of its own is shaded from it.
    var base = scene.base.rgb;
    if (in.color >> 24u) != 0u {
        base = vec3<f32>(
            f32(in.color & 255u),
            f32((in.color >> 8u) & 255u),
            f32((in.color >> 16u) & 255u),
        );
    }
    var weights = scene.shade.xy;
    // The finish (`Finish`): the metallic factor, the highlight's exponent
    // and its strength relative to the plain one.
    var metallic = f32(in.finish & 255u) / 255.0;
    var shininess = u32(scene.shade.w);
    var strength = 1.0;
    let exponent = in.finish >> 8u;
    if exponent != 0u {
        shininess = exponent;
        strength = sqrt((f32(exponent) + 2.0) / (scene.shade.w + 2.0));
    }
    if highlighted(in.face) {
        base = scene.highlight.rgb;
        weights = scene.highlight_shade.xy;
        metallic = 0.0;
        shininess = u32(scene.shade.w);
        strength = 1.0;
    }

    // View::toward_viewer.
    var to_viewer = -scene.forward.xyz;
    if scene.forward.w > 0.5 {
        let len = length(in.rel);
        if len > 0.0 {
            to_viewer = -in.rel / len;
        }
    }

    // The interpolated normal, or the triangle's own where there is none.
    var n = vec3<f32>(0.0);
    var has_normal = false;
    let len_n = length(in.normal);
    if len_n > 0.0 {
        n = in.normal / len_n;
        has_normal = true;
    } else {
        let len_f = length(in.flat_normal);
        if len_f > 0.0 {
            n = in.flat_normal / len_f;
            has_normal = true;
        }
    }

    var rgb = base;
    if has_normal {
        // shade(): two-sided, one light fixed to the camera.
        if dot(n, to_viewer) < 0.0 {
            n = -n;
        }
        var light = to_viewer;
        if scene.light.w > 0.5 {
            light = scene.light.xyz;
        }
        let diffuse = max(dot(n, light), 0.0);
        var specular = 0.0;
        let half_vector = light + to_viewer;
        let len_h = length(half_vector);
        if len_h > 0.0 {
            specular = powi(max(dot(n, half_vector / len_h), 0.0), shininess);
        }
        // A metal reflects less diffusely (`SHADE_METAL_DIFFUSE`), and its highlight is in its own
        // colour: white times (1 - m) plus the base colour times m.
        let intensity = weights.x + weights.y * diffuse * (1.0 - 0.3 * metallic);
        let tint = vec3<f32>(1.0 - metallic) + base / 255.0 * metallic;
        rgb = base * intensity + tint * (scene.shade.z * strength * specular);
    }
    // Rounded here, so the target stores exactly this integer whatever the
    // driver's float-to-unorm conversion does with fractions.
    return vec4<f32>(clamp(round(rgb), vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0, 1.0);
}

// Colour and data: what the renderer that reads the frame back draws.
@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.data = data_of(in);
    out.color = shade_of(in);
    {{SET_DEPTH}}
    return out;
}

// Colour only: the picture a viewport shows, into a multisampled target.
@fragment
fn fs_color(in: VertexOut) -> ColorOut {
    var out: ColorOut;
    out.color = shade_of(in);
    {{SET_DEPTH}}
    return out;
}

// Data only: face ids and distances at one sample per pixel, for picking
// and for the tests of the viewport's picture.
@fragment
fn fs_data(in: VertexOut) -> DataOut {
    var out: DataOut;
    out.data = data_of(in);
    {{SET_DEPTH}}
    return out;
}
