// The section plane with one sample per pixel: the cap, and the two passes
// of a see-through picture that is cut open. This source follows
// `scene.wgsl`, `peel.wgsl` and `cut.wgsl` in one module.

// The count of `fs_parity`: 1 where the number of fragments left out at
// the sample is odd.
@group(1) @binding(1) var parity: texture_2d<f32>;

// True where the cap is: inside a body, within the box around the mesh.
fn cap_here(in: CutOut) -> bool {
    if !in_cap_box(in) {
        return false;
    }
    return textureLoad(parity, vec2<i32>(in.clip.xy), 0).r > 0.5;
}

// The cap with its data: the renderer that reads the frame back. Also the
// cap as the front of a see-through picture (its alpha is 1).
@fragment
fn fs_cap(in: CutOut) -> FragmentOut {
    if !cap_here(in) {
        discard;
    }
    var out: FragmentOut;
    out.color = cap_color();
    out.data = cap_data(in);
    {{SET_DEPTH}}
    return out;
}

// The cap, colour only: a viewport.
@fragment
fn fs_cap_color(in: CutOut) -> ColorOut {
    if !cap_here(in) {
        discard;
    }
    var out: ColorOut;
    out.color = cap_color();
    {{SET_DEPTH}}
    return out;
}

// The cap, data only: a viewport's capture.
@fragment
fn fs_cap_data(in: CutOut) -> DataOut {
    if !cap_here(in) {
        discard;
    }
    var out: DataOut;
    out.data = cap_data(in);
    {{SET_DEPTH}}
    return out;
}

// `fs_front` of a cut-open picture.
@fragment
fn fs_front_cut(in: CutOut) -> FragmentOut {
    if clipped(in) {
        discard;
    }
    let whole = whole_of(in);
    var out: FragmentOut;
    out.data = data_of(whole);
    out.color = front_of(whole);
    {{SET_DEPTH}}
    return out;
}

// `fs_front_color` of a cut-open picture.
@fragment
fn fs_front_color_cut(in: CutOut) -> ColorOut {
    if clipped(in) {
        discard;
    }
    var out: ColorOut;
    out.color = front_of(whole_of(in));
    {{SET_DEPTH}}
    return out;
}

// `fs_behind` of a cut-open picture.
@fragment
fn fs_behind_cut(in: CutOut) -> ColorOut {
    if clipped(in) {
        discard;
    }
    let front = {{FRONT_DEPTH}};
    if !(stored_depth(in.clip.z, in.dist) < front) {
        discard;
    }
    var out: ColorOut;
    out.color = shade_of(whole_of(in));
    {{SET_DEPTH}}
    return out;
}

// The cap as the surface behind the front one. Where the cap is the front
// itself its depth here is the depth the front pass stored (the same quad
// through the same vertex stage), so it is not behind itself.
@fragment
fn fs_cap_behind(in: CutOut) -> ColorOut {
    if !cap_here(in) {
        discard;
    }
    let front = {{FRONT_DEPTH}};
    if !(stored_depth(in.clip.z, in.dist) < front) {
        discard;
    }
    var out: ColorOut;
    out.color = cap_color();
    {{SET_DEPTH}}
    return out;
}
