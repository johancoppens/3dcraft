// `cut_samples.wgsl` run per sample, for the drivers that cannot take a
// stage that writes the sample mask (`variant.rs`). No sample mask, no loop
// over the samples: every stage that wrote a mask runs once per sample
// (`@builtin(sample_index)`) and reads the count and the front's key of its
// own sample only. This source follows `scene.wgsl`,
// `peel_samples_persample.wgsl` and `cut.wgsl` in one module.

// The count of `fs_parity`, per sample: 1 where the number of fragments
// left out there is odd.
@group(1) @binding(1) var parity: texture_multisampled_2d<f32>;

// Whether the cap is at this sample.
fn cap_here(in: CutOut, sample: u32) -> bool {
    return in_cap_box(in) && textureLoad(parity, vec2<i32>(in.clip.xy), i32(sample)).r > 0.5;
}

struct CapOut {
    @location(0) color: vec4<f32>,
    {{FRAG_DEPTH}}
}

// The cap of an opaque picture.
@fragment
fn fs_cap_samples(in: CutOut, @builtin(sample_index) sample: u32) -> CapOut {
    if !cap_here(in, sample) {
        discard;
    }
    var out: CapOut;
    out.color = cap_color();
    {{SET_DEPTH}}
    return out;
}

struct CapFrontOut {
    @location(0) color: vec4<f32>,
    @location(1) key: vec4<f32>,
    {{FRAG_DEPTH}}
}

fn key_bytes(key: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(key & 255u),
        f32((key >> 8u) & 255u),
        f32((key >> 16u) & 255u),
        f32(key >> 24u),
    ) / 255.0;
}

// The cap in the front of a see-through picture, with its key.
@fragment
fn fs_cap_front_samples(in: CutOut, @builtin(sample_index) sample: u32) -> CapFrontOut {
    if !cap_here(in, sample) {
        discard;
    }
    var out: CapFrontOut;
    out.color = cap_color();
    out.key = key_bytes(bitcast<u32>(in.clip.z));
    {{SET_DEPTH}}
    return out;
}

// `fs_front_samples` of a cut-open picture, per sample.
@fragment
fn fs_front_samples_cut(in: CutOut, @builtin(sample_index) sample: u32) -> FrontOut {
    // Taken, so that the stage runs per sample.
    _ = sample;
    if clipped(in) {
        discard;
    }
    let whole = whole_of(in);
    var out: FrontOut;
    out.color = shade_of(whole);
    if highlighted(whole.face) {
        out.color.a = 0.0;
    }
    out.key = key_bytes(key_of(whole));
    {{SET_DEPTH}}
    return out;
}

// `fs_behind_samples` of a cut-open picture, per sample.
@fragment
fn fs_behind_samples_cut(in: CutOut, @builtin(sample_index) sample: u32) -> BehindOut {
    if clipped(in) {
        discard;
    }
    let whole = whole_of(in);
    if front_at(in.clip, sample) == key_of(whole) {
        discard;
    }
    var out: BehindOut;
    out.color = shade_of(whole);
    {{SET_DEPTH}}
    return out;
}

// The cap as the surface behind the front one: at this sample, where it
// is, and is not the front.
@fragment
fn fs_cap_behind_samples(in: CutOut, @builtin(sample_index) sample: u32) -> BehindOut {
    if !cap_here(in, sample) || front_at(in.clip, sample) == bitcast<u32>(in.clip.z) {
        discard;
    }
    var out: BehindOut;
    out.color = cap_color();
    {{SET_DEPTH}}
    return out;
}
