// The accumulation of a studio picture's frames (`render_studio`): each
// frame's pixel bytes summed exactly as integers (`fs_accumulate`, the
// sum so far in `sum_map` plus the frame in `frame_map`, into the other
// of two targets), and the mean, rounded once (`fs_average`:
// `(sum + n / 2) / n`).

struct Accumulate {
    // x: the number of frames in the sum.
    count: vec4<u32>,
}

@group(0) @binding(0) var<uniform> accumulate: Accumulate;
@group(0) @binding(1) var frame_map: texture_2d<f32>;
@group(0) @binding(2) var sum_map: texture_2d<u32>;

@vertex
fn vs_accumulate(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_accumulate(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<u32> {
    let at = vec2<i32>(i32(frag.x), i32(frag.y));
    let bytes = vec3<u32>(round(clamp(textureLoad(frame_map, at, 0).rgb, vec3<f32>(0.0),
                                      vec3<f32>(1.0)) * 255.0));
    return vec4<u32>(textureLoad(sum_map, at, 0).rgb + bytes, 0u);
}

@fragment
fn fs_average(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(i32(frag.x), i32(frag.y));
    let n = max(accumulate.count.x, 1u);
    let mean = min((textureLoad(sum_map, at, 0).rgb + vec3<u32>(n / 2u)) / n, vec3<u32>(255u));
    return vec4<f32>(vec3<f32>(mean) / 255.0, 1.0);
}
