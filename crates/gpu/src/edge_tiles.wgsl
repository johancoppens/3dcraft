// The density of the edges over a picture, per tile: the mean of the ink
// the edges of the bodies in sight left in each pixel of the tile (the
// pass `vs_ink`/`fs_ink` of lines.wgsl summed it), over the pixels of the
// tile that lie in the image. `craft3d_ui_egui::viewport::EdgeDensity` on
// the CPU. One fragment per tile, over a triangle that covers the target.

struct Lines {
    size: vec4<f32>,
    // z: the side of a tile in pixels.
    look: vec4<f32>,
    eye: vec4<f32>,
    outline: vec4<f32>,
    fade: vec4<f32>,
    more: vec4<f32>,
}

@group(0) @binding(0) var<uniform> lines: Lines;
@group(0) @binding(1) var ink: texture_2d<f32>;

@vertex
fn vs_cover(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_tiles(@builtin(position) at: vec4<f32>) -> @location(0) u32 {
    let side = i32(lines.look.z);
    let size = vec2<i32>(textureDimensions(ink));
    let low = vec2<i32>(floor(at.xy)) * side;
    let high = min(low + vec2<i32>(side), size);
    var sum = 0.0;
    for (var y = low.y; y < high.y; y = y + 1) {
        for (var x = low.x; x < high.x; x = x + 1) {
            sum = sum + textureLoad(ink, vec2<i32>(x, y), 0).r;
        }
    }
    let count = max((high.x - low.x) * (high.y - low.y), 1);
    // As the bits of the `f32`: an integer target renders on every backend.
    return bitcast<u32>(sum / f32(count));
}
