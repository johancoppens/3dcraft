// The section plane with several samples per pixel: the cap, and the two
// passes of a see-through picture that is cut open. This source follows
// `scene.wgsl`, `peel_samples.wgsl` and `cut.wgsl` in one module.
//
// A fragment is shaded once per pixel, so whether it is cut away is
// decided at the pixel's centre for all of its samples, in the pass that
// counts and in the pass that draws alike: per sample the count is of the
// fragments that were left out there, and the cap takes the samples where
// it is odd, through its sample mask. Along the cut the picture is
// therefore decided per pixel (the outline of the cap is drawn over it as
// lines); the silhouettes keep their samples.

// The count of `fs_parity`, per sample: 1 where the number of fragments
// left out there is odd.
@group(1) @binding(1) var parity: texture_multisampled_2d<f32>;

// The samples of this pixel where the cap is.
fn cap_mask(in: CutOut) -> u32 {
    if !in_cap_box(in) {
        return 0u;
    }
    let p = vec2<i32>(in.clip.xy);
    let count = i32(textureNumSamples(parity));
    var mask = 0u;
    for (var s = 0; s < count; s = s + 1) {
        if textureLoad(parity, p, s).r > 0.5 {
            mask = mask | (1u << u32(s));
        }
    }
    return mask;
}

struct CapOut {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) mask: u32,
    {{FRAG_DEPTH}}
}

// The cap of an opaque picture.
@fragment
fn fs_cap_samples(in: CutOut) -> CapOut {
    let mask = cap_mask(in);
    if mask == 0u {
        discard;
    }
    var out: CapOut;
    out.color = cap_color();
    out.mask = mask;
    {{SET_DEPTH}}
    return out;
}

struct CapFrontOut {
    @location(0) color: vec4<f32>,
    @location(1) key: vec4<f32>,
    @builtin(sample_mask) mask: u32,
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
fn fs_cap_front_samples(in: CutOut) -> CapFrontOut {
    let mask = cap_mask(in);
    if mask == 0u {
        discard;
    }
    var out: CapFrontOut;
    out.color = cap_color();
    out.key = key_bytes(bitcast<u32>(in.clip.z));
    out.mask = mask;
    {{SET_DEPTH}}
    return out;
}

// `fs_front_samples` of a cut-open picture.
@fragment
fn fs_front_samples_cut(in: CutOut) -> FrontOut {
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

// The samples of this pixel whose front is not the triangle with the key
// `mine`.
fn not_front(mine: u32, at: vec4<f32>) -> u32 {
    let p = vec2<i32>(at.xy);
    let count = i32(textureNumSamples(front_key));
    var mask = 0u;
    for (var s = 0; s < count; s = s + 1) {
        let bytes = vec4<u32>(round(textureLoad(front_key, p, s) * 255.0));
        let front = bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
        if front != mine {
            mask = mask | (1u << u32(s));
        }
    }
    return mask;
}

// `fs_behind_samples` of a cut-open picture.
@fragment
fn fs_behind_samples_cut(in: CutOut) -> BehindOut {
    if clipped(in) {
        discard;
    }
    let whole = whole_of(in);
    let mask = not_front(key_of(whole), in.clip);
    if mask == 0u {
        discard;
    }
    var out: BehindOut;
    out.color = shade_of(whole);
    out.mask = mask;
    {{SET_DEPTH}}
    return out;
}

// The cap as the surface behind the front one: on the samples where it
// is, and is not the front.
@fragment
fn fs_cap_behind_samples(in: CutOut) -> BehindOut {
    let mask = cap_mask(in) & not_front(bitcast<u32>(in.clip.z), in.clip);
    if mask == 0u {
        discard;
    }
    var out: BehindOut;
    out.color = cap_color();
    out.mask = mask;
    {{SET_DEPTH}}
    return out;
}
