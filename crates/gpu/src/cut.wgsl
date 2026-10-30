// The section plane (`RenderParams::section`): what every pass of a
// cut-open picture shares. This source follows `scene.wgsl` (and the
// see-through passes) in one module; the vertex stage and the shading are
// that file's.
//
// The definition is the CPU renderer's (`craft3d_render::render`,
// *Section*). A fragment is left out where its distance from the plane,
// interpolated from the corners like the position, is positive, and a
// triangle that lies in the plane goes with its body (`Cut::clips`). The
// cap is where the number of fragments left out at a sample is odd; that
// count is a pass of its own over the triangles (`fs_parity`), into a
// target whose blend turns every fragment into "one minus what was
// there", and the cap is then a quad in the plane, drawn after the
// triangles and kept on the samples where the count is odd
// (`cut_flat.wgsl`, `cut_samples.wgsl`).
//
// Where in the pixel the distance from the plane is taken decides the
// count, and the source is a template for it (the markers SIDE_SAMPLE and
// SIDE_CENTROID, filled by `craft3d_gpu::renderer::scene_source_for`).
// With several samples per pixel the count is per sample: the count pass
// has a vertex stage of its own (`vs_parity`) whose distance is
// interpolated at the sample, so a sample is counted by exactly the
// surfaces that are cut away at its own point, as the CPU counts; and the
// passes that draw take the distance at the centroid of the covered
// samples, never beyond the triangle. Taken at the pixel's centre, as it
// was, the distance of a sliver triangle (the silhouette of a curved face
// seen edge-on) is extrapolated past its edge and changes sign there, and
// a sample it covers is counted or kept wrongly: a lone sample of the cap
// missing well away from the cut's outline (docs/design/rendering.md,
// 2026-10-19). With one sample per pixel the centre is the sample and the
// markers are empty.

// `craft3d_render::SECTION_PARALLEL`.
const PARALLEL: f32 = 0.9999847412109375;
// `craft3d_render::SECTION_FACE`.
const SECTION_FACE: u32 = 4294967294u;

struct CutOut {
    // Invariant, as `scene.wgsl`'s: the two steps (`renderer::Step`).
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) flat_normal: vec3<f32>,
    @location(3) @interpolate(flat) face: u32,
    @location(4) dist: f32,
    // The distance from the plane, positive in the half that is cut away
    // (`Cut::side`); within the triangle's covered samples where there
    // are several.
    @location(5) {{SIDE_CENTROID}} side: f32,
    @location(6) @interpolate(flat) color: u32,
    @location(7) @interpolate(flat) finish: u32,
    @location(8) value: vec2<f32>,
}

// `Cut::side` at a corner.
fn side_of(in: VertexIn) -> f32 {
    return dot(in.position, scene.section.xyz) - scene.section.w;
}

@vertex
fn vs_cut(in: VertexIn) -> CutOut {
    let whole = vertex(in);
    var out: CutOut;
    out.clip = whole.clip;
    out.rel = whole.rel;
    out.normal = whole.normal;
    out.flat_normal = whole.flat_normal;
    out.face = whole.face;
    out.dist = whole.dist;
    out.color = whole.color;
    out.finish = whole.finish;
    out.value = whole.value;
    out.side = side_of(in);
    return out;
}

// What the count pass needs of a vertex: the triangle's own normal and
// the distance from the plane, at the sample where there are several.
struct ParityOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) flat_normal: vec3<f32>,
    @location(1) {{SIDE_SAMPLE}} side: f32,
}

@vertex
fn vs_parity(in: VertexIn) -> ParityOut {
    var out: ParityOut;
    out.clip = vertex(in).clip;
    out.flat_normal = in.flat_normal;
    out.side = side_of(in);
    return out;
}

// The fragment as the passes of a whole picture take it.
fn whole_of(in: CutOut) -> VertexOut {
    var out: VertexOut;
    out.clip = in.clip;
    out.rel = in.rel;
    out.normal = in.normal;
    out.flat_normal = in.flat_normal;
    out.face = in.face;
    out.dist = in.dist;
    out.color = in.color;
    out.finish = in.finish;
    out.value = in.value;
    return out;
}

// `Cut::clips`: true where a point of a triangle with the normal
// `flat_normal` at the distance `side` from the plane is cut away. A side
// that is not a number fails both comparisons and is kept, as on the CPU.
fn clips(side: f32, flat_normal: vec3<f32>) -> bool {
    let facing = dot(flat_normal, scene.section.xyz);
    if abs(side) <= scene.section_cap.w && abs(facing) >= PARALLEL {
        return facing < 0.0;
    }
    return side > 0.0;
}

// True where the fragment is cut away.
fn clipped(in: CutOut) -> bool {
    return clips(in.side, in.flat_normal);
}

// `Cut::holds_cap`: the cap stays within the box around the mesh.
fn in_cap_box(in: CutOut) -> bool {
    let p = scene.eye.xyz + in.rel;
    return all(p >= scene.section_lo.xyz) && all(p <= scene.section_hi.xyz);
}

// The cap's colour: drawn as it is, and seen through like a body's face.
fn cap_color() -> vec4<f32> {
    return vec4<f32>(scene.section_cap.rgb, 1.0);
}

// The cap's id and the bits of its view distance: the distance of the
// point of the plane itself, as the CPU renderer gives it.
fn cap_data(in: CutOut) -> vec2<u32> {
    return vec2<u32>(SECTION_FACE, bitcast<u32>(dot(in.rel, scene.forward.xyz)));
}

// Counts the fragments that are left out, modulo two: the target's blend
// is "one minus what was there". Per sample where there are several
// (`ParityOut`).
@fragment
fn fs_parity(in: ParityOut) -> @location(0) vec4<f32> {
    if !clips(in.side, in.flat_normal) {
        discard;
    }
    return vec4<f32>(1.0);
}

// `fs_main` of a cut-open picture.
@fragment
fn fs_main_cut(in: CutOut) -> FragmentOut {
    if clipped(in) {
        discard;
    }
    let whole = whole_of(in);
    var out: FragmentOut;
    out.data = data_of(whole);
    out.color = shade_of(whole);
    {{SET_DEPTH}}
    return out;
}

// `fs_color` of a cut-open picture.
@fragment
fn fs_color_cut(in: CutOut) -> ColorOut {
    if clipped(in) {
        discard;
    }
    var out: ColorOut;
    out.color = shade_of(whole_of(in));
    {{SET_DEPTH}}
    return out;
}

// `fs_data` of a cut-open picture.
@fragment
fn fs_data_cut(in: CutOut) -> DataOut {
    if clipped(in) {
        discard;
    }
    var out: DataOut;
    out.data = data_of(whole_of(in));
    {{SET_DEPTH}}
    return out;
}
