// The resolve pass: from `ssaa * ssaa` samples per pixel to one pixel, the
// way `craft3d_render::render` ends. The colour is the rounded integer mean
// of the samples; the face id and the distance come from the pixel's first
// sample, because they cannot be averaged.

struct Resolve {
    // x: supersampling factor per axis.
    ssaa: vec4<u32>,
}

@group(0) @binding(0) var<uniform> resolve: Resolve;
@group(0) @binding(1) var samples_color: texture_2d<f32>;
@group(0) @binding(2) var samples_data: texture_2d<u32>;

// One triangle that covers the whole target.
@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

struct ResolveOut {
    @location(0) color: vec4<f32>,
    @location(1) data: vec2<u32>,
}

@fragment
fn fs_resolve(@builtin(position) frag: vec4<f32>) -> ResolveOut {
    let s = resolve.ssaa.x;
    let first = vec2<u32>(u32(frag.x), u32(frag.y)) * s;
    var sum = vec3<u32>(0u);
    for (var dy = 0u; dy < s; dy = dy + 1u) {
        for (var dx = 0u; dx < s; dx = dx + 1u) {
            let texel = textureLoad(samples_color, first + vec2<u32>(dx, dy), 0).rgb;
            // Each sample is an 8-bit value: recover the integer.
            sum = sum + vec3<u32>(round(texel * 255.0));
        }
    }
    let count = s * s;
    let mean = (sum + vec3<u32>(count / 2u)) / count;
    var out: ResolveOut;
    out.color = vec4<f32>(vec3<f32>(mean) / 255.0, 1.0);
    out.data = textureLoad(samples_data, first, 0).rg;
    return out;
}
