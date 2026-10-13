// See-through bodies: the front surface over the one behind it, per
// sample, then the mean of the samples: how `craft3d_render::render` ends a
// picture with `RenderParams::opacity` below 1.
//
// The blend itself is not computed here. `opacity * front + (1 - opacity)
// * behind`, rounded, is a function of two bytes, and the renderer
// tabulates it with the CPU renderer's own expression in `f64` (256 × 256
// bytes: the column is the front's level, the row the level behind). So a
// sample whose two layers match the CPU's has the CPU's colour exactly,
// also where the exact blend is half-way between two levels and `f32`
// would round the other way.
//
// The source is a template: `{{COLOR}}` and `{{INDEX}}` are filled in for
// multisampled layers (a viewport) or single-sampled ones.

struct Blend {
    // x: supersampling factor per axis (`fs_blend_frame`), or the number of
    // samples per pixel (`fs_blend_samples`).
    count: vec4<u32>,
}

@group(0) @binding(0) var<uniform> blend: Blend;
// rgb: the shaded front. a: 1 where it is seen through, 0 where it is not
// (a highlighted face, the background).
@group(0) @binding(1) var front_color: {{COLOR}};
@group(0) @binding(2) var behind_color: {{COLOR}};
@group(0) @binding(3) var table: texture_2d<f32>;
@group(0) @binding(4) var samples_data: texture_2d<u32>;

// One triangle that covers the whole target.
@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn load(layer: {{COLOR}}, p: vec2<u32>, k: u32) -> vec4<f32> {
    return textureLoad(layer, p, {{INDEX}});
}

fn tabulated(front: u32, behind: u32) -> u32 {
    return u32(round(textureLoad(table, vec2<u32>(front, behind), 0).r * 255.0));
}

// The colour of one sample, as levels of 255.
fn sample_at(p: vec2<u32>, k: u32) -> vec3<u32> {
    let front = load(front_color, p, k);
    // Each layer holds 8-bit values: recover the integers.
    let f = vec3<u32>(round(front.rgb * 255.0));
    if front.a < 0.5 {
        return f;
    }
    let b = vec3<u32>(round(load(behind_color, p, k).rgb * 255.0));
    return vec3<u32>(tabulated(f.r, b.r), tabulated(f.g, b.g), tabulated(f.b, b.b));
}

struct FrameOut {
    @location(0) color: vec4<f32>,
    @location(1) data: vec2<u32>,
}

// Layers `ssaa` times larger per axis than the picture, one sample each:
// the rounded integer mean of the blended samples, and the first sample's
// face id and distance (the front's), as `resolve.wgsl` does.
@fragment
fn fs_blend_frame(@builtin(position) frag: vec4<f32>) -> FrameOut {
    let s = blend.count.x;
    let first = vec2<u32>(u32(frag.x), u32(frag.y)) * s;
    var sum = vec3<u32>(0u);
    for (var dy = 0u; dy < s; dy = dy + 1u) {
        for (var dx = 0u; dx < s; dx = dx + 1u) {
            sum = sum + sample_at(first + vec2<u32>(dx, dy), 0u);
        }
    }
    let count = s * s;
    let mean = (sum + vec3<u32>(count / 2u)) / count;
    var out: FrameOut;
    out.color = vec4<f32>(vec3<f32>(mean) / 255.0, 1.0);
    out.data = textureLoad(samples_data, first, 0).rg;
    return out;
}

// Layers of the picture's size with several samples per pixel: the rounded
// integer mean of the blended samples.
@fragment
fn fs_blend_samples(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let count = blend.count.x;
    let p = vec2<u32>(u32(frag.x), u32(frag.y));
    var sum = vec3<u32>(0u);
    for (var k = 0u; k < count; k = k + 1u) {
        sum = sum + sample_at(p, k);
    }
    let mean = (sum + vec3<u32>(count / 2u)) / count;
    return vec4<f32>(vec3<f32>(mean) / 255.0, 1.0);
}
