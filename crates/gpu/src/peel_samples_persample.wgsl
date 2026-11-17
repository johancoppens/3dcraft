// `peel_samples.wgsl` run per sample, for the drivers that cannot take a
// stage that writes the sample mask (`variant.rs`: Intel's and AMD's Vulkan
// drivers on Windows crash on one; docs/design/rendering.md, *Per-sample
// stages on Vulkan drivers on Windows*). No sample mask, no loop
// over the samples: both passes are shaded once per sample
// (`@builtin(sample_index)`), the front writes the key of its triangle at
// the sample, and the pass behind it reads the key of its own sample only
// and discards the sample whose front is its own triangle. The key is the
// depth at the sample in both passes alike, so the comparison holds as in
// `peel_samples.wgsl`; the shading is per sample, so the picture is not
// bit for bit the one of that file on every driver. This source follows
// `scene.wgsl` in one module.

@group(1) @binding(0) var front_key: texture_multisampled_2d<f32>;

// The key of the fragment's triangle at this sample.
fn key_of(in: VertexOut) -> u32 {
    return bitcast<u32>(in.clip.z);
}

struct FrontOut {
    // rgb: the shaded colour. a: 1 where it is seen through, 0 where it is
    // not (a highlighted face).
    @location(0) color: vec4<f32>,
    @location(1) key: vec4<f32>,
    {{FRAG_DEPTH}}
}

// The front: the opaque picture with its key, per sample.
@fragment
fn fs_front_samples(in: VertexOut, @builtin(sample_index) sample: u32) -> FrontOut {
    // Taken, so that the stage runs per sample.
    _ = sample;
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
    {{SET_DEPTH}}
    return out;
}

struct BehindOut {
    @location(0) color: vec4<f32>,
    {{FRAG_DEPTH}}
}

// The key the front wrote at this pixel's sample `sample`.
fn front_at(at: vec4<f32>, sample: u32) -> u32 {
    let bytes = vec4<u32>(round(textureLoad(front_key, vec2<i32>(at.xy), i32(sample)) * 255.0));
    return bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
}

// The surface behind the front one: on this sample, where this triangle is
// not the front.
@fragment
fn fs_behind_samples(in: VertexOut, @builtin(sample_index) sample: u32) -> BehindOut {
    if front_at(in.clip, sample) == key_of(in) {
        discard;
    }
    var out: BehindOut;
    out.color = shade_of(in);
    {{SET_DEPTH}}
    return out;
}
