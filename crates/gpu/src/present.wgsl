// Copies the resolved colour image into a target someone else owns (the
// shell's viewport texture), pixel for pixel from the top-left corner.

@group(0) @binding(0) var resolved: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn load(frag: vec4<f32>) -> vec4<f32> {
    let size = textureDimensions(resolved);
    let at = min(vec2<u32>(u32(frag.x), u32(frag.y)), size - vec2<u32>(1u));
    return textureLoad(resolved, at, 0);
}

// For a target that stores what it is given (Rgba8Unorm, Bgra8Unorm).
@fragment
fn fs_present(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    return load(frag);
}

// For an sRGB target, which encodes on write: decode first, so the bytes
// that end up in the target are the bytes of the resolved image.
@fragment
fn fs_present_srgb(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let c = load(frag);
    let low = c.rgb / 12.92;
    let high = pow((c.rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(high, low, c.rgb <= vec3<f32>(0.04045)), c.a);
}
