// Copies the depth buffer of the scene pass into a target that can be read
// back (`Onscreen::read_depth`), as the lines' depth test reads it: the
// mean of the pixel's samples (`surface_at` of depth_test.wgsl), 0 where
// any sample shows nothing, as the bits of the `f32` in an integer target
// (renderable on every backend, where `R32Float` is not). The source is a
// template like lines.wgsl: `{{DEPTH}}` and `{{LOAD}}` are filled in for a
// multisampled or a single-sampled depth texture; the bind group is the
// lines'.

struct Lines {
    size: vec4<f32>,
    look: vec4<f32>,
    eye: vec4<f32>,
}

@group(0) @binding(1) var<uniform> lines: Lines;
@group(0) @binding(2) var depth_texture: {{DEPTH}};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) u32 {
    let p = vec2<i32>(frag.xy);
    let samples = i32(lines.size.w);
    var sum = 0.0;
    for (var s = 0; s < samples; s = s + 1) {
        let z = {{LOAD}};
        if !(z > 0.0) {
            return 0u;
        }
        sum = sum + z;
    }
    return bitcast<u32>(sum / f32(samples));
}
