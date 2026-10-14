// See-through bodies with several samples per pixel: the two scene passes
// of `peel.wgsl` for a multisampled viewport. This source follows
// `scene.wgsl` in one module.
//
// With multisampling a fragment is shaded once per pixel, at its centre,
// and the depth test runs per sample on depths the shader never sees: the
// fragment's own depth is the triangle's depth at the centre, not at a
// sample (and a shader made to run per sample does not get the depth
// test's number either: on Intel's Vulkan driver it differs from the depth
// buffer by far more than rounding, measured 2026-10-14). So the front
// cannot be told from what is behind it by comparing the fragment's depth
// with the depth buffer.
//
// What does hold per sample is the order: the front is the nearest
// fragment at its sample, so every other fragment that covers the sample
// is at or behind it. All that is left to sort out is the front itself
// (and a triangle that coincides with it). For that the first pass writes,
// next to the colour, a key of the triangle that is in front at each
// sample: the bits of its depth at the pixel's centre. The second pass
// computes the same key for its own triangle (same triangle, same pixel,
// same arithmetic: the same bits), drops the samples whose front has that
// key through the sample mask, and the depth test keeps the nearest of
// what is left, per sample and exactly. Two triangles with the same
// corners have the same key, so a face that coincides with the front is
// not "behind" it, as on the CPU.
//
// The key is stored as four bytes of an `Rgba8Unorm` target, the one
// format a multisampled viewport needs anyway; a byte goes in and comes
// out exactly.

@group(1) @binding(0) var front_key: texture_multisampled_2d<f32>;

// The key of the fragment's triangle at this pixel.
fn key_of(in: VertexOut) -> u32 {
    return bitcast<u32>(in.clip.z);
}

struct FrontOut {
    // rgb: the shaded colour. a: 1 where it is seen through, 0 where it is
    // not (a highlighted face).
    @location(0) color: vec4<f32>,
    @location(1) key: vec4<f32>,
}

// The front: the opaque picture with its key.
@fragment
fn fs_front_samples(in: VertexOut) -> FrontOut {
    var out: FrontOut;
    out.color = shade_of(in);
    if highlighted(in.face) {
        out.color.a = 0.0;
    }
    let key = key_of(in);
    out.key = vec4<f32>(
        f32(key & 255u),
        f32((key >> 8u) & 255u),
        f32((key >> 16u) & 255u),
        f32(key >> 24u),
    ) / 255.0;
    return out;
}

struct BehindOut {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) mask: u32,
}

// The surface behind the front one: on the samples where this triangle is
// not the front.
@fragment
fn fs_behind_samples(in: VertexOut) -> BehindOut {
    let mine = key_of(in);
    let p = vec2<i32>(in.clip.xy);
    let count = i32(textureNumSamples(front_key));
    var mask = 0u;
    for (var s = 0; s < count; s = s + 1) {
        let bytes = vec4<u32>(round(textureLoad(front_key, p, s) * 255.0));
        let front = bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
        if front != mine {
            mask = mask | (1u << u32(s));
        }
    }
    if mask == 0u {
        discard;
    }
    var out: BehindOut;
    out.color = shade_of(in);
    out.mask = mask;
    return out;
}
