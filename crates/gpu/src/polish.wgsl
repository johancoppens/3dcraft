// Layer 2 of the studio look after the shading (`craft3d_render::studio::
// polish`), appended to `scene.wgsl` and `studio.wgsl`. Full-screen passes
// over the screen-space buffers of one frame at its samples, in the CPU's
// order: the reflections (`fs_reflect`), the lens (`fs_lens`), bloom's
// pyramid down (`fs_bloom_down`) and up (`fs_bloom_up`), then bloom's mix
// with the exposure, the tone curve and the bytes (`fs_display`). Every
// buffer holds the bits of `f32` in an integer target, read with
// `textureLoad`, so WebGL2 renders them; a pass reads the size of what it
// reads from the texture.

const SSR_STEPS: i32 = 32;
const SSR_REFINE: i32 = 5;
const SSR_MAX_ROUGHNESS: f32 = 0.4;
const SSR_EDGE: f32 = 0.05;
// The lens's taps: 8 i on ring i of four (polish::DOF_RINGS).
const DOF_TAPS: i32 = 80;

// The light of the samples, the specular weight with the roughness, and
// what a pass reads besides (a level of bloom's pyramid).
@group(1) @binding(6) var light_map: texture_2d<u32>;
@group(1) @binding(7) var weight_map: texture_2d<u32>;
@group(1) @binding(8) var aux_map: texture_2d<u32>;

fn light_at(x: i32, y: i32) -> vec3<f32> {
    let t = textureLoad(light_map, vec2<i32>(x, y), 0);
    return vec3<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y), bitcast<f32>(t.z));
}

fn out_bits(c: vec3<f32>) -> vec4<u32> {
    return vec4<u32>(bitcast<u32>(c.x), bitcast<u32>(c.y), bitcast<u32>(c.z), 0u);
}

// The point on the ray from a to b (projected) at u, floored to its sample.
fn trace_at(a: vec4<f32>, b: vec4<f32>, u: f32) -> vec2<f32> {
    return floor(a.xy + (b.xy - a.xy) * u);
}

// The view distance on the ray at u: 1/z interpolated in perspective.
fn trace_depth(a: vec4<f32>, b: vec4<f32>, u: f32) -> f32 {
    if scene.forward.w > 0.5 {
        return 1.0 / ((1.0 - u) / a.z + u / b.z);
    }
    return (1.0 - u) * a.z + u * b.z;
}

// polish::trace from the point `p` (relative to the eye) along the unit
// `d`: the sample it finds in xy with z = 1, or z = 0 for none.
fn trace(p: vec3<f32>, d: vec3<f32>, start: vec2<f32>) -> vec3<f32> {
    let none = vec3<f32>(0.0);
    let z0 = dot(p, scene.forward.xyz);
    let dz = dot(d, scene.forward.xyz);
    let thickness = studio.reach.y;
    var len = studio.reach.x;
    if scene.forward.w > 0.5 && dz < 0.0 {
        // Stop the ray short of the eye plane: at a tenth of its depth.
        len = min(len, 0.9 * z0 / -dz);
    }
    if !(len > 0.0 && len <= 3.4e38) {
        return none;
    }
    let a = project_rel(p);
    let b = project_rel(p + d * len);
    if a.w < 0.5 || b.w < 0.5 {
        return none;
    }
    var prev_u = 0.0;
    var prev_z = a.z;
    for (var i = 1; i <= SSR_STEPS; i++) {
        let u = f32(i) / f32(SSR_STEPS);
        let at = trace_at(a, b, u);
        if !(at.x >= 0.0 && at.y >= 0.0 && at.x < studio.size.x && at.y < studio.size.y) {
            return none;
        }
        let z = trace_depth(a, b, u);
        if !(at.x == start.x && at.y == start.y) {
            let s = sample_depth(at.x, at.y);
            if s.y > 0.5 && z >= s.x && z - s.x <= thickness + abs(z - prev_z) {
                var lo = prev_u;
                var hi = u;
                for (var k = 0; k < SSR_REFINE; k++) {
                    let mid = 0.5 * (lo + hi);
                    let m = trace_at(a, b, mid);
                    let sm = sample_depth(m.x, m.y);
                    if sm.y > 0.5 && trace_depth(a, b, mid) >= sm.x {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let h = trace_at(a, b, hi);
                if sample_depth(h.x, h.y).y < 0.5 || (h.x == start.x && h.y == start.y) {
                    return vec3<f32>(at, 1.0);
                }
                return vec3<f32>(h, 1.0);
            }
        }
        prev_u = u;
        prev_z = z;
    }
    return none;
}

// The unit normal of the sample (x, y) facing the viewer at `rel`.
fn facing_normal(g: vec4<u32>, v: vec3<f32>) -> vec3<f32> {
    var n = vec3<f32>(bitcast<f32>(g.y), bitcast<f32>(g.z), bitcast<f32>(g.w));
    let len = length(n);
    if len > 0.0 {
        n = n / len;
    } else {
        n = v;
    }
    if dot(n, v) < 0.0 {
        n = -n;
    }
    return n;
}

// Reflect::apply's `facing`: whether the surface found at `hit` faces the
// ray along `d`.
fn facing(hit: vec2<f32>, d: vec3<f32>) -> bool {
    let g = textureLoad(geometry_map, vec2<i32>(hit), 0);
    let dist = bitcast<f32>(g.x);
    let rel = point_at(hit.x + 0.5, hit.y + 0.5, dist);
    let n = facing_normal(g, toward_viewer(rel));
    return dot(n, d) <= 0.0;
}

// polish::edge_fade.
fn edge_fade(hit: vec2<f32>) -> f32 {
    let x = hit.x + 0.5;
    let y = hit.y + 0.5;
    let near = min(min(x, studio.size.x - x), min(y, studio.size.y - y));
    return clamp(near / (SSR_EDGE * studio.size.y), 0.0, 1.0);
}

// polish::dielectric_fresnel.
fn dielectric_fresnel(c: f32) -> f32 {
    return DIELECTRIC_F0 + (1.0 - DIELECTRIC_F0) * powi(1.0 - clamp(c, 0.0, 1.0), 5u);
}

// polish::Reflect::apply at one sample.
@fragment
fn fs_reflect(in: FullscreenOut) -> @location(0) vec4<u32> {
    let x = i32(floor(in.clip.x));
    let y = i32(floor(in.clip.y));
    let here = vec2<f32>(f32(x), f32(y));
    let l = light_at(x, y);
    let px = f32(x) + 0.5;
    let py = f32(y) + 0.5;
    let top = studio.look.w;
    let g = textureLoad(geometry_map, vec2<i32>(x, y), 0);
    let dist = bitcast<f32>(g.x);
    var out = l;
    if dist < 3.0e38 {
        let wt = textureLoad(weight_map, vec2<i32>(x, y), 0);
        let w = vec3<f32>(bitcast<f32>(wt.x), bitcast<f32>(wt.y), bitcast<f32>(wt.z));
        let r = bitcast<f32>(wt.w);
        if !(studio.polish.x > 0.5 && r >= 0.0 && r < SSR_MAX_ROUGHNESS) {
            return out_bits(l);
        }
        let rel = point_at(px, py, dist);
        let v = toward_viewer(rel);
        let n = facing_normal(g, v);
        let reflected = n * (2.0 * dot(n, v)) - v;
        let len = length(reflected);
        if !(len > 0.0) {
            return out_bits(l);
        }
        let d = reflected / len;
        let hit = trace(rel, d, here);
        if hit.z < 0.5 || !facing(hit.xy, d) {
            return out_bits(l);
        }
        let conf = (1.0 - r / SSR_MAX_ROUGHNESS) * edge_fade(hit.xy);
        let env = radiance(d, r * top);
        let found = light_at(i32(hit.x), i32(hit.y));
        out = l + conf * w * (found - env);
    } else {
        // The plain background reflects nothing.
        let plain = bitcast<f32>(textureLoad(weight_map, vec2<i32>(x, y), 0).w) == PLAIN_BACKDROP;
        let strength = studio.polish.y;
        if plain || !(strength > 0.0 && studio.ground.y > 0.5) {
            return out_bits(l);
        }
        let ray = view_ray(px, py);
        let origin = ray[0];
        let d = ray[1];
        let z = studio.ground.x;
        if !(d.z < 0.0 && origin.z > z) {
            return out_bits(l);
        }
        let t = (z - origin.z) / d.z;
        if !(t > 0.0 && t <= 3.4e38) {
            return out_bits(l);
        }
        // The ground's point relative to the eye.
        let g_rel = (origin - scene.eye.xyz) + d * t;
        let m = vec3<f32>(d.x, d.y, -d.z);
        let f = strength * dielectric_fresnel(-d.z);
        let env = radiance(m, BACKGROUND_ROUGHNESS * top);
        var seen = env;
        var conf = 0.0;
        let hit = trace(g_rel, m, here);
        if hit.z > 0.5 && facing(hit.xy, m) {
            seen = light_at(i32(hit.x), i32(hit.y));
            conf = edge_fade(hit.xy);
        }
        let refl = env + conf * (seen - env);
        out = l + f * (refl - l);
    }
    return out_bits(out);
}

// Lens::coc at the view distance `z` (`finite` false for the background).
fn coc(z: f32, finite: bool) -> f32 {
    var c = studio.lens.x;
    if finite {
        if z > 0.0 {
            c = studio.lens.x * abs(z - studio.lens.y) / z;
        } else {
            return studio.lens.z;
        }
    }
    if !(c >= 0.0) {
        c = 0.0;
    }
    return min(c, studio.lens.z);
}

// The view distance of a sample for the lens, with w = 1 when finite.
fn lens_depth(x: i32, y: i32) -> vec2<f32> {
    let dist = bitcast<f32>(textureLoad(geometry_map, vec2<i32>(x, y), 0).x);
    if dist < 3.0e38 {
        return vec2<f32>(dist, 1.0);
    }
    return vec2<f32>(0.0, 0.0);
}

// polish::dof_gather at one sample, with the frame's taps.
@fragment
fn fs_lens(in: FullscreenOut) -> @location(0) vec4<u32> {
    let x = i32(floor(in.clip.x));
    let y = i32(floor(in.clip.y));
    let z0 = lens_depth(x, y);
    let c0 = coc(z0.x, z0.y > 0.5);
    let l0 = light_at(x, y);
    let m0 = max(c0, 0.5);
    let w0 = 1.0 / (m0 * m0);
    var sum = l0 * w0;
    var total = w0;
    for (var j = 0; j < DOF_TAPS; j++) {
        let pair = studio.taps[j / 2];
        var o = pair.xy;
        if (j & 1) == 1 {
            o = pair.zw;
        }
        let tx = floor(f32(x) + 0.5 + o.x);
        let ty = floor(f32(y) + 0.5 + o.y);
        if !(tx >= 0.0 && ty >= 0.0 && tx < studio.size.x && ty < studio.size.y) {
            continue;
        }
        let zt = lens_depth(i32(tx), i32(ty));
        let ct = coc(zt.x, zt.y > 0.5);
        // Behind the sample: a far surface does not spread over a nearer
        // one (the background is behind everything).
        let behind = (zt.y < 0.5 && z0.y > 0.5) || (zt.y > 0.5 && z0.y > 0.5 && zt.x > z0.x);
        var c = ct;
        if behind {
            c = min(ct, c0);
        }
        let d = sqrt(o.x * o.x + o.y * o.y);
        let cover = clamp(c - d + 0.5, 0.0, 1.0);
        if cover <= 0.0 {
            continue;
        }
        let mc = max(c, 0.5);
        let wt = cover / (mc * mc);
        sum += light_at(i32(tx), i32(ty)) * wt;
        total += wt;
    }
    return out_bits(sum / total);
}

// A texel of the level in `aux_map`, clamped to its border.
fn aux_at(x: i32, y: i32) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(aux_map, 0));
    let t = textureLoad(aux_map, clamp(vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1)), 0);
    return vec3<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y), bitcast<f32>(t.z));
}

// polish::bloom_down's mean of the 2 x 2 texels around a corner.
fn bloom_corner(cx: i32, cy: i32) -> vec3<f32> {
    return (aux_at(cx - 1, cy - 1) + aux_at(cx, cy - 1) + aux_at(cx - 1, cy) + aux_at(cx, cy))
        * 0.25;
}

// polish::bloom_down: one level of the pyramid from the level in
// `aux_map`.
@fragment
fn fs_bloom_down(in: FullscreenOut) -> @location(0) vec4<u32> {
    let cx = 2 * i32(floor(in.clip.x)) + 1;
    let cy = 2 * i32(floor(in.clip.y)) + 1;
    var c = vec3<f32>(0.0);
    c += bloom_corner(cx - 1, cy - 1) * (0.5 / 4.0);
    c += bloom_corner(cx + 1, cy - 1) * (0.5 / 4.0);
    c += bloom_corner(cx - 1, cy + 1) * (0.5 / 4.0);
    c += bloom_corner(cx + 1, cy + 1) * (0.5 / 4.0);
    for (var by = -2; by <= 0; by += 2) {
        for (var bx = -2; bx <= 0; bx += 2) {
            for (var oy = 0; oy <= 2; oy += 2) {
                for (var ox = 0; ox <= 2; ox += 2) {
                    c += bloom_corner(cx + bx + ox, cy + by + oy) * (0.125 / 4.0);
                }
            }
        }
    }
    return out_bits(c);
}

// The level in `aux_map` read bilinearly at (u, v) in its texels.
fn aux_bilinear(u: f32, v: f32) -> vec3<f32> {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let ix = i32(x0);
    let iy = i32(y0);
    return aux_at(ix, iy) * ((1.0 - tx) * (1.0 - ty)) + aux_at(ix + 1, iy) * (tx * (1.0 - ty))
        + aux_at(ix, iy + 1) * ((1.0 - tx) * ty) + aux_at(ix + 1, iy + 1) * (tx * ty);
}

// polish::bloom_tent: the coarse level in `aux_map` at the fine texel
// (x, y).
fn bloom_tent(x: f32, y: f32) -> vec3<f32> {
    let u = (x + 0.5) * 0.5;
    let v = (y + 0.5) * 0.5;
    var c = vec3<f32>(0.0);
    for (var oy = -1; oy <= 1; oy++) {
        for (var ox = -1; ox <= 1; ox++) {
            let wx = select(1.0, 2.0, ox == 0);
            let wy = select(1.0, 2.0, oy == 0);
            c += aux_bilinear(u + f32(ox), v + f32(oy)) * (wx * wy / 16.0);
        }
    }
    return c;
}

// polish::bloom's sum_level: the level in `light_map` plus the tent of
// the coarser sum in `aux_map`.
@fragment
fn fs_bloom_up(in: FullscreenOut) -> @location(0) vec4<u32> {
    let x = floor(in.clip.x);
    let y = floor(in.clip.y);
    return out_bits(light_at(i32(x), i32(y)) + bloom_tent(x, y));
}

// The end of a frame: bloom's mix (polish::bloom, with the pyramid's sum
// in `aux_map`), then FrameLight::display: the plain background (its
// roughness in `weight_map` PLAIN_BACKDROP) as srgb(L 2^EV), without the
// tone curve; everything else through it.
@fragment
fn fs_display(in: FullscreenOut) -> @location(0) vec4<f32> {
    let x = floor(in.clip.x);
    let y = floor(in.clip.y);
    let l = light_at(i32(x), i32(y));
    var c = l;
    let s = studio.polish.z;
    if s > 0.0 {
        var b = l;
        let levels = studio.polish.w;
        if levels > 1.5 {
            b += bloom_tent(x, y);
        }
        c = (1.0 - s) * l + s * b / levels;
    }
    let r = bitcast<f32>(textureLoad(weight_map, vec2<i32>(i32(x), i32(y)), 0).w);
    if r == PLAIN_BACKDROP {
        let e = c * studio.look.y;
        let out = vec3<f32>(srgb_encode(e.x), srgb_encode(e.y), srgb_encode(e.z));
        return vec4<f32>(clamp(round(out * 255.0), vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0, 1.0);
    }
    return vec4<f32>(display(c), 1.0);
}
