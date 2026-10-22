// The distances the lines' depth test reads, once per frame: per pixel the
// view distance of the surface (the mean of its samples' depths turned back
// into a distance, `surface_at` of depth_test.wgsl as it was computed from
// the depth buffer itself), or infinity where a sample shows nothing, as
// the bits of the `f32` in an integer target (renderable on every
// backend). The lines and the fills then read one texel where they read
// every sample of the pixel: the same numbers, a quarter of the reads with
// four samples, and the outline test of an edge reads as many again. The
// source is a template like lines.wgsl: `{{DEPTH}}` and `{{LOAD}}` are
// filled in for a multisampled or a single-sampled depth texture; the
// single-sampled one is read through a nearest sampler (`{{SAMPLER}}`) at
// the texel's centre, which gives the texel itself: OpenGL ES 3.0 (WebGL2)
// takes a depth texture without a sampler that filters by the nearest
// texel for incomplete, and reads 0 from it.

struct Scene {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    // x, y: the depth numbers of scene.wgsl.
    depth: vec4<f32>,
    base: vec4<f32>,
    shade: vec4<f32>,
}

struct Lines {
    // w: the number of samples of the depth texture.
    size: vec4<f32>,
    look: vec4<f32>,
    eye: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<uniform> lines: Lines;
@group(0) @binding(2) var depth_texture: {{DEPTH}};
{{SAMPLER}}

// The bits of positive infinity: nothing drawn.
const NOTHING: u32 = 0x7f800000u;

@vertex
fn vs_cover(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_distance(@builtin(position) frag: vec4<f32>) -> @location(0) u32 {
    let p = vec2<i32>(floor(frag.xy));
    let samples = i32(lines.size.w);
    var sum = 0.0;
    for (var s = 0; s < samples; s = s + 1) {
        let z = {{LOAD}};
        // Negated, so that a depth that is not a number counts as nothing.
        if !(z > 0.0) {
            return NOTHING;
        }
        sum = sum + z;
    }
    let z = sum / f32(samples);
    if scene.forward.w > 0.5 {
        // z = near / dist, as the scene pass writes it.
        return bitcast<u32>(scene.depth.x / z);
    }
    // z = (far - dist) * scale.
    return bitcast<u32>(scene.depth.x - z / scene.depth.y);
}
