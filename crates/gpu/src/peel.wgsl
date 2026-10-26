// See-through bodies: the two scene passes of a picture with
// `RenderParams::opacity` below 1. This source follows `scene.wgsl` in one
// module (the vertex stage and the shading are that file's, untouched).
//
// The CPU renderer keeps two surfaces per sample: the nearest one (the
// front) and the nearest one strictly further away (the one behind). Here
// that is two passes over the same triangles:
//
// 1. The front: the opaque picture, with its depth buffer kept. The alpha
//    of its colour says whether the sample is seen through: 1 for a body's
//    face, 0 for a highlighted face (the target is cleared to alpha 0, so
//    the background is not blended either).
// 2. The one behind: a fragment is dropped unless its depth is greater
//    than the front's at its own sample, read from the first pass's depth
//    buffer; of what is left the depth test keeps the nearest.
//
// The comparison is strict and has no bias. Both passes rasterise the same
// triangles with the same vertex stage on the same device (and where the
// fragment writes its depth, `stored_depth`, with the same arithmetic),
// so the depth of a triangle at a sample is the same number in both, and
// the front surface is never its own "behind". Every other fragment at
// the sample is at or behind the front (the front is the nearest), so
// "greater" only sorts out the front itself and what ties with it: two
// surfaces closer together than the depth buffer resolves have equal
// depths, and then there is
// nothing behind, as on the CPU when two keys are equal.
//
// This file is for a depth buffer with one sample per pixel, where the
// fragment's own depth is the number the depth test stored. With several
// samples per pixel that is not so (`peel_samples.wgsl`).
//
// How the front's depth is read is a template (`renderer::scene_source_for`):
// `{{FRONT_DEPTH}}` is the front's depth at the fragment's pixel, loaded,
// or on OpenGL sampled through the nearest sampler `{{FRONT_SAMPLER}}`
// declares, at the texel's centre, which gives the texel itself. OpenGL ES
// 3.0 (WebGL2) takes a depth texture whose filter is not the nearest texel
// for incomplete and reads 0 from it, `texelFetch` too, and wgpu's OpenGL
// backend binds no sampler to a texture that is only loaded from.

@group(1) @binding(0) var front_depth: texture_2d<f32>;
{{FRONT_SAMPLER}}

// The shaded colour with the alpha that says whether it is seen through.
fn front_of(in: VertexOut) -> vec4<f32> {
    var color = shade_of(in);
    if highlighted(in.face) {
        color.a = 0.0;
    }
    return color;
}

// The front with its data: the renderer that reads the frame back.
@fragment
fn fs_front(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.data = data_of(in);
    out.color = front_of(in);
    {{SET_DEPTH}}
    return out;
}

// The front, colour only: a viewport.
@fragment
fn fs_front_color(in: VertexOut) -> ColorOut {
    var out: ColorOut;
    out.color = front_of(in);
    {{SET_DEPTH}}
    return out;
}

// The surface behind the front one.
@fragment
fn fs_behind(in: VertexOut) -> ColorOut {
    let front = {{FRONT_DEPTH}};
    // Behind is less with reversed depth (scene.wgsl). Negated, so that a
    // depth that is not a number is dropped.
    if !(stored_depth(in.clip.z, in.dist) < front) {
        discard;
    }
    var out: ColorOut;
    out.color = shade_of(in);
    {{SET_DEPTH}}
    return out;
}
