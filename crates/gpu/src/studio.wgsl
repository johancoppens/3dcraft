// The studio look (`craft3d_render::studio`), appended to `scene.wgsl`.
// Every function mirrors the CPU renderer's of the same name; the comments
// name it. Positions are relative to the mesh origin, as in the scene.
//
// Passes: the shadow map (`vs_shadow`, `fs_shadow`), the geometry of the
// samples (`fs_geometry`: view distance and normal), their ambient
// occlusion (`fs_ao`), the background (`fs_background`) and the surfaces
// (`fs_studio`, `fs_studio_color`).

const PI: f32 = 3.14159265358979;
const HALF_PI: f32 = 1.57079632679490;
const LUT_SIZE: f32 = 32.0;
const SHADOW_SIZE: f32 = 1024.0;
const BLOCKER_TAPS: i32 = 4;
const FILTER_TAPS: i32 = 5;
const MAX_PENUMBRA: f32 = 24.0;
const DEPTH_BIAS: f32 = 2.0;
const NORMAL_OFFSET: f32 = 1.5;
const AO_SLICES: i32 = 6;
const AO_STEPS: i32 = 6;
const AO_BIAS: f32 = 0.05;
const DEFAULT_ROUGHNESS: f32 = 0.5;
const DIELECTRIC_F0: f32 = 0.04;
const BACKGROUND_ROUGHNESS: f32 = 0.45;

struct Studio {
    // xyz: towards the key light. w: 1 when there is one.
    key: vec4<f32>,
    // rgb: the key's irradiance. w: its angular radius.
    key_light: vec4<f32>,
    // The light's view (`shadow::LightView`): the centre, relative to the
    // mesh origin, and the radius; its axes; one texel in model units.
    lv_center: vec4<f32>,
    // xyz: u. w: one texel.
    lv_u: vec4<f32>,
    // xyz: v. w: 1 with shadows.
    lv_v: vec4<f32>,
    // xyz: w. w: 1 with ambient occlusion.
    lv_w: vec4<f32>,
    // x: the rotation in radians. y: 2^exposure. z: the tone curve
    // (0 AgX, 1 ACES, 2 linear). w: the top level of the environment.
    look: vec4<f32>,
    // x: the ground's height relative to the mesh origin. y: 1 with a
    // ground. z: the occlusion's radius in model units. w: its most
    // reach in samples.
    ground: vec4<f32>,
    // x, y: the target in samples. z, w: the luminance of the irradiance
    // without the key on an upward face, and of the key's.
    size: vec4<f32>,
    sh: array<vec4<f32>, 9>,
}

@group(1) @binding(0) var<uniform> studio: Studio;
@group(1) @binding(1) var env_map: texture_2d<f32>;
@group(1) @binding(2) var lut_map: texture_2d<f32>;
@group(1) @binding(3) var shadow_map: texture_2d<u32>;
@group(1) @binding(4) var ao_map: texture_2d<u32>;
@group(1) @binding(5) var geometry_map: texture_2d<u32>;

// tonemap::srgb_encode.
fn srgb_encode(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if x <= 0.0031308 {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn agx_curve(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2
        + 0.1191 * x - 0.00232;
}

fn agx_ev(v: f32) -> f32 {
    let min_ev = -12.473931188;
    let max_ev = 4.026068812;
    var ev = min_ev;
    if v > 0.0 {
        ev = log2(v);
    }
    return agx_curve((clamp(ev, min_ev, max_ev) - min_ev) / (max_ev - min_ev));
}

// tonemap::display, then to_byte over 255.
fn display(c_in: vec3<f32>) -> vec3<f32> {
    let c = max(c_in * studio.look.y, vec3<f32>(0.0));
    var out = vec3<f32>(0.0);
    if studio.look.z < 0.5 {
        let r = 0.842479062253094 * c.x + 0.0784335999999992 * c.y + 0.0792237451477643 * c.z;
        let g = 0.0423282422610123 * c.x + 0.878468636469772 * c.y + 0.0791661274605434 * c.z;
        let b = 0.0423756549057051 * c.x + 0.0784336 * c.y + 0.879142973793104 * c.z;
        let k = vec3<f32>(agx_ev(r), agx_ev(g), agx_ev(b));
        out = vec3<f32>(
            1.19687900512017 * k.x - 0.0980208811401368 * k.y - 0.0990297440797205 * k.z,
            -0.0528968517574562 * k.x + 1.15190312990417 * k.y - 0.0989611768448433 * k.z,
            -0.0529716355144438 * k.x - 0.0980434501171241 * k.y + 1.15107367264116 * k.z,
        );
        out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if studio.look.z < 1.5 {
        let x = c * 0.6;
        let a = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
        out = vec3<f32>(srgb_encode(a.x), srgb_encode(a.y), srgb_encode(a.z));
    } else {
        out = vec3<f32>(srgb_encode(c.x), srgb_encode(c.y), srgb_encode(c.z));
    }
    return clamp(round(out * 255.0), vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0;
}

// Level::bilinear at one level of the environment.
fn env_level(u: f32, v: f32, level: i32) -> vec3<f32> {
    let dims = textureDimensions(env_map, level);
    let w = f32(dims.x);
    let h = f32(dims.y);
    let x = u * w - 0.5;
    let y = clamp(v * h - 0.5, 0.0, h - 1.0);
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = x - x0;
    let fy = y - y0;
    let xa = min(u32(x0 - w * floor(x0 / w)), dims.x - 1u);
    let x1 = x0 + 1.0;
    let xb = min(u32(x1 - w * floor(x1 / w)), dims.x - 1u);
    let ya = u32(y0);
    let yb = min(ya + 1u, dims.y - 1u);
    let a = textureLoad(env_map, vec2<u32>(xa, ya), level).rgb;
    let b = textureLoad(env_map, vec2<u32>(xb, ya), level).rgb;
    let c = textureLoad(env_map, vec2<u32>(xa, yb), level).rgb;
    let d = textureLoad(env_map, vec2<u32>(xb, yb), level).rgb;
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    return top + (bottom - top) * fy;
}

// FrameLight::radiance (env::trilinear with direction_uv).
fn radiance(d: vec3<f32>, lod_in: f32) -> vec3<f32> {
    let u = atan2(d.y, d.x) / (2.0 * PI) + 0.5 - studio.look.x / (2.0 * PI);
    let v = acos(clamp(d.z, -1.0, 1.0)) / PI;
    let top = studio.look.w;
    let lod = clamp(lod_in, 0.0, top);
    let l0 = floor(lod);
    let f = lod - l0;
    let a = env_level(u, v, i32(l0));
    if f <= 0.0 {
        return a;
    }
    let b = env_level(u, v, i32(l0) + 1);
    return a + (b - a) * f;
}

// FrameLight::irradiance (Prepared::irradiance of rotate_z(n, -rotation)).
fn irradiance(n_in: vec3<f32>) -> vec3<f32> {
    let c = cos(studio.look.x);
    let s = sin(studio.look.x);
    let n = vec3<f32>(c * n_in.x + s * n_in.y, -s * n_in.x + c * n_in.y, n_in.z);
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = studio.sh[0].rgb * 0.282094791;
    e += studio.sh[1].rgb * (0.488602512 * y);
    e += studio.sh[2].rgb * (0.488602512 * z);
    e += studio.sh[3].rgb * (0.488602512 * x);
    e += studio.sh[4].rgb * (1.092548431 * x * y);
    e += studio.sh[5].rgb * (1.092548431 * y * z);
    e += studio.sh[6].rgb * (0.315391565 * (3.0 * z * z - 1.0));
    e += studio.sh[7].rgb * (1.092548431 * x * z);
    e += studio.sh[8].rgb * (0.546274215 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// Prepared::split_sum.
fn split_sum(nv: f32, r: f32) -> vec2<f32> {
    let x = clamp(nv * LUT_SIZE - 0.5, 0.0, LUT_SIZE - 1.0);
    let y = clamp(r * LUT_SIZE - 0.5, 0.0, LUT_SIZE - 1.0);
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = x - x0;
    let fy = y - y0;
    let xa = u32(x0);
    let ya = u32(y0);
    let xb = min(xa + 1u, 31u);
    let yb = min(ya + 1u, 31u);
    let a = textureLoad(lut_map, vec2<u32>(xa, ya), 0).xy;
    let b = textureLoad(lut_map, vec2<u32>(xb, ya), 0).xy;
    let c = textureLoad(lut_map, vec2<u32>(xa, yb), 0).xy;
    let d = textureLoad(lut_map, vec2<u32>(xb, yb), 0).xy;
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    return top + (bottom - top) * fy;
}

// ShadowMap::at.
fn shadow_at(x: f32, y: f32) -> f32 {
    if !(x >= 0.0 && y >= 0.0 && x < SHADOW_SIZE && y < SHADOW_SIZE) {
        return bitcast<f32>(0x7f800000u);
    }
    return bitcast<f32>(textureLoad(shadow_map, vec2<u32>(u32(floor(x)), u32(floor(y))), 0).r);
}

// shadow::visibility, for a point relative to the mesh origin.
fn visibility(p: vec3<f32>, n: vec3<f32>, radius: f32) -> f32 {
    let texel = studio.lv_u.w;
    let q = p + n * (NORMAL_OFFSET * texel);
    let rel = q - studio.lv_center.xyz;
    let r = studio.lv_center.w;
    let x = (dot(rel, studio.lv_u.xyz) / r * 0.5 + 0.5) * SHADOW_SIZE;
    let y = (0.5 - dot(rel, studio.lv_v.xyz) / r * 0.5) * SHADOW_SIZE;
    let d = r - dot(rel, studio.lv_w.xyz);
    let bias = DEPTH_BIAS * texel;
    let slope = max(tan(radius), 0.0);
    let search = clamp(slope * 2.0 * r / texel, 1.0, MAX_PENUMBRA);
    var sum = 0.0;
    var count = 0u;
    for (var j = 0; j < BLOCKER_TAPS; j++) {
        for (var i = 0; i < BLOCKER_TAPS; i++) {
            let ox = (f32(i) + 0.5) / f32(BLOCKER_TAPS) * 2.0 - 1.0;
            let oy = (f32(j) + 0.5) / f32(BLOCKER_TAPS) * 2.0 - 1.0;
            let s = shadow_at(x + ox * search, y + oy * search);
            if s < d - bias {
                sum += s;
                count += 1u;
            }
        }
    }
    if count == 0u {
        return 1.0;
    }
    let blocker = sum / f32(count);
    let width = clamp((d - blocker) * slope / texel, 1.0, MAX_PENUMBRA);
    var lit = 0u;
    for (var j = 0; j < FILTER_TAPS; j++) {
        for (var i = 0; i < FILTER_TAPS; i++) {
            let ox = (f32(i) + 0.5) / f32(FILTER_TAPS) * 2.0 - 1.0;
            let oy = (f32(j) + 0.5) / f32(FILTER_TAPS) * 2.0 - 1.0;
            if shadow_at(x + ox * width, y + oy * width) >= d - bias {
                lit += 1u;
            }
        }
    }
    return f32(lit) / f32(FILTER_TAPS * FILTER_TAPS);
}

// The luminance of a linear colour (env::luminance).
fn luminance(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

// FrameLight::shade: the linear light leaving the point.
fn studio_shade(base_bytes: vec3<f32>, metallic: f32, roughness: f32, p: vec3<f32>,
                n: vec3<f32>, v: vec3<f32>, ao: f32) -> vec3<f32> {
    let base = vec3<f32>(
        srgb_decode(base_bytes.x / 255.0),
        srgb_decode(base_bytes.y / 255.0),
        srgb_decode(base_bytes.z / 255.0),
    );
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    let reflected = n * (2.0 * dot(n, v)) - v;
    let ab = split_sum(nv, roughness);
    let prefiltered = radiance(reflected, roughness * studio.look.w);
    let irr = irradiance(n);
    var key_light = vec3<f32>(0.0);
    var key_dot = 0.0;
    if studio.key.w > 0.5 {
        let nl = max(dot(n, studio.key.xyz), 0.0);
        var vis = 1.0;
        if studio.lv_v.w > 0.5 && nl > 0.0 {
            vis = visibility(p, n, studio.key_light.w);
        }
        key_light = studio.key_light.rgb;
        key_dot = nl * vis;
    }
    let occlusion = pow(nv + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao;
    let specular_occlusion = clamp(occlusion, 0.0, 1.0);
    let sum_ab = ab.x + ab.y;
    let f0 = vec3<f32>(DIELECTRIC_F0 * (1.0 - metallic)) + base * metallic;
    let fs = f0 * ab.x + vec3<f32>(ab.y);
    var multiple = vec3<f32>(1.0);
    if sum_ab > 0.0 {
        multiple = vec3<f32>(1.0) + f0 * (1.0 / sum_ab - 1.0);
    }
    let specular = prefiltered * fs * multiple * specular_occlusion;
    let light = irr * ao + key_light * key_dot;
    let diffuse = base * (1.0 - metallic) * (vec3<f32>(1.0) - fs) / PI * light;
    return diffuse + specular;
}

// View::ray and View::point_at: the offsets across the view of the
// position (px, py) of the target, in samples.
fn across(px: f32, py: f32) -> vec3<f32> {
    let cx = (px / studio.size.x * 2.0 - 1.0) / scene.right.w;
    let cy = (1.0 - py / studio.size.y * 2.0) / scene.up.w;
    return scene.right.xyz * cx + scene.up.xyz * cy;
}

fn point_at(px: f32, py: f32, dist: f32) -> vec3<f32> {
    let a = across(px, py);
    if scene.forward.w > 0.5 {
        return (scene.forward.xyz + a) * dist;
    }
    return a + scene.forward.xyz * dist;
}

fn sample_size(dist: f32) -> f32 {
    if scene.forward.w > 0.5 {
        return 2.0 * dist / (scene.up.w * studio.size.y);
    }
    return 2.0 / (scene.up.w * studio.size.y);
}

// FrameLight::ground_factor, the ray from `origin` (relative to the mesh
// origin) along the unit `d`.
fn ground_factor(origin: vec3<f32>, d: vec3<f32>) -> f32 {
    if studio.ground.y < 0.5 || studio.lv_v.w < 0.5 || studio.key.w < 0.5 {
        return 1.0;
    }
    let z = studio.ground.x;
    if !(d.z < 0.0 && origin.z > z) {
        return 1.0;
    }
    let t = (z - origin.z) / d.z;
    if !(t > 0.0) {
        return 1.0;
    }
    let g = origin + d * t;
    let lz = max(studio.key.z, 0.0);
    let rest = studio.size.z;
    let key = studio.size.w * lz;
    if !(rest + key > 0.0) || lz <= 0.0 {
        return 1.0;
    }
    let vis = visibility(g, vec3<f32>(0.0, 0.0, 1.0), studio.key_light.w);
    return (rest + key * vis) / (rest + key);
}

struct FullscreenOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vs_studio_fullscreen(@builtin(vertex_index) i: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    out.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

// FrameLight::background, through the position of a fragment.
@fragment
fn fs_background(in: FullscreenOut) -> @location(0) vec4<f32> {
    let a = across(in.clip.x, in.clip.y);
    var origin = scene.eye.xyz;
    var d = scene.forward.xyz;
    if scene.forward.w > 0.5 {
        let len = length(scene.forward.xyz + a);
        if len > 0.0 {
            d = (scene.forward.xyz + a) / len;
        }
    } else {
        origin = scene.eye.xyz + a;
    }
    let c = radiance(d, BACKGROUND_ROUGHNESS * studio.look.w) * ground_factor(origin, d);
    return vec4<f32>(display(c), 1.0);
}

// The shadow map: the light's view of the mesh, the depth from the light
// as the bits of an f32 (`ShadowMap::draw`).
struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) depth: f32,
}

@vertex
fn vs_shadow(in: VertexIn) -> ShadowOut {
    var out: ShadowOut;
    let rel = in.position - studio.lv_center.xyz;
    let r = studio.lv_center.w;
    let d = r - dot(rel, studio.lv_w.xyz);
    out.clip = vec4<f32>(dot(rel, studio.lv_u.xyz) / r, dot(rel, studio.lv_v.xyz) / r,
                         clamp(d / (2.0 * r), 0.0, 1.0), 1.0);
    out.depth = d;
    return out;
}

@fragment
fn fs_shadow(in: ShadowOut) -> @location(0) u32 {
    return bitcast<u32>(in.depth);
}

// The interpolated normal, or the triangle's own (shade_of).
fn normal_of(in: VertexOut) -> vec3<f32> {
    let len_n = length(in.normal);
    if len_n > 0.0 {
        return in.normal / len_n;
    }
    let len_f = length(in.flat_normal);
    if len_f > 0.0 {
        return in.flat_normal / len_f;
    }
    return vec3<f32>(0.0);
}

struct GeometryOut {
    // The view distance and the normal, as bits.
    @location(0) data: vec4<u32>,
    {{FRAG_DEPTH}}
}

@fragment
fn fs_geometry(in: VertexOut) -> GeometryOut {
    var out: GeometryOut;
    let n = normal_of(in);
    out.data = vec4<u32>(bitcast<u32>(in.dist), bitcast<u32>(n.x), bitcast<u32>(n.y),
                         bitcast<u32>(n.z));
    {{SET_DEPTH}}
    return out;
}

// The point of the sample (x, y) relative to the eye, or w = 0 for none.
fn geometry_point(x: i32, y: i32) -> vec4<f32> {
    if x < 0 || y < 0 || f32(x) >= studio.size.x || f32(y) >= studio.size.y {
        return vec4<f32>(0.0);
    }
    let g = textureLoad(geometry_map, vec2<i32>(x, y), 0);
    let dist = bitcast<f32>(g.x);
    if !(dist < 3.0e38) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(point_at(f32(x) + 0.5, f32(y) + 0.5, dist), 1.0);
}

// The unit vector towards the viewer from a point relative to the eye
// (View::toward_viewer).
fn toward_viewer(rel: vec3<f32>) -> vec3<f32> {
    if scene.forward.w > 0.5 {
        let len = length(rel);
        if len > 0.0 {
            return -rel / len;
        }
    }
    return -scene.forward.xyz;
}

// ao::occlusion.
fn occlusion(at: vec2<i32>, p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, radius: f32,
             reach: f32) -> f32 {
    if !(reach >= 1.0 && radius > 0.0) {
        return 1.0;
    }
    var visible = 0.0;
    var unblocked = 0.0;
    for (var k = 0; k < AO_SLICES; k++) {
        let phi = PI * (f32(k) + 0.5) / f32(AO_SLICES);
        let c = cos(phi);
        let s = sin(phi);
        let dir = scene.right.xyz * c - scene.up.xyz * s;
        let t_raw = dir - v * dot(dir, v);
        let t_len = length(t_raw);
        if !(t_len > 0.0) {
            continue;
        }
        let t = t_raw / t_len;
        let projected = v * dot(n, v) + t * dot(n, t);
        let len = length(projected);
        if len <= 1e-9 {
            continue;
        }
        let gamma = acos(clamp(dot(projected, v) / len, -1.0, 1.0))
            * select(-1.0, 1.0, dot(n, t) >= 0.0);
        var horizon = array<f32, 2>(0.0, 0.0);
        for (var side = 0; side < 2; side++) {
            let sign = select(-1.0, 1.0, side == 0);
            var highest = -1.0;
            for (var j = 0; j < AO_STEPS; j++) {
                let step = (f32(j) + 1.0) / f32(AO_STEPS) * reach;
                let x = f32(at.x) + 0.5 + sign * c * step;
                let y = f32(at.y) + 0.5 + sign * s * step;
                let q = geometry_point(i32(floor(x)), i32(floor(y)));
                if q.w < 0.5 {
                    continue;
                }
                let d = q.xyz - p;
                let l = length(d);
                if l <= 0.0 || dot(d, n) <= AO_BIAS * l {
                    continue;
                }
                let cos_h = dot(d, v) / l;
                let fall = clamp(1.0 - (l / radius) * (l / radius), 0.0, 1.0);
                highest = max(highest, -1.0 + (cos_h + 1.0) * fall);
            }
            horizon[side] = sign * acos(clamp(highest, -1.0, 1.0));
        }
        let h1 = gamma + max(horizon[1] - gamma, -HALF_PI);
        let h2 = gamma + min(horizon[0] - gamma, HALF_PI);
        let cg = cos(gamma);
        let sg = sin(gamma);
        let a1 = 0.25 * (-cos(2.0 * h1 - gamma) + cg + 2.0 * h1 * sg);
        let a2 = 0.25 * (-cos(2.0 * h2 - gamma) + cg + 2.0 * h2 * sg);
        visible += len * (a1 + a2);
        unblocked += len * (cg + gamma * sg);
    }
    if !(unblocked > 0.0) {
        return 1.0;
    }
    return clamp(visible / unblocked, 0.0, 1.0);
}

// The occlusion of every sample, as the bits of an f32 (1 where there is
// no surface).
@fragment
fn fs_ao(in: FullscreenOut) -> @location(0) u32 {
    let x = i32(floor(in.clip.x));
    let y = i32(floor(in.clip.y));
    let g = textureLoad(geometry_map, vec2<i32>(x, y), 0);
    let dist = bitcast<f32>(g.x);
    if !(dist < 3.0e38) {
        return bitcast<u32>(1.0);
    }
    let rel = point_at(f32(x) + 0.5, f32(y) + 0.5, dist);
    let v = toward_viewer(rel);
    var n = vec3<f32>(bitcast<f32>(g.y), bitcast<f32>(g.z), bitcast<f32>(g.w));
    if !(length(n) > 0.0) {
        n = v;
    }
    if dot(n, v) < 0.0 {
        n = -n;
    }
    let reach = min(studio.ground.z / sample_size(dist), studio.ground.w);
    return bitcast<u32>(occlusion(vec2<i32>(x, y), rel, n, v, studio.ground.z, reach));
}

// The studio colour of a fragment, as the bytes of the picture over 255.
fn studio_of(in: VertexOut) -> vec4<f32> {
    var base = scene.base.rgb;
    if (in.color >> 24u) != 0u {
        base = vec3<f32>(
            f32(in.color & 255u),
            f32((in.color >> 8u) & 255u),
            f32((in.color >> 16u) & 255u),
        );
    }
    var metallic = f32(in.finish & 255u) / 255.0;
    var roughness = DEFAULT_ROUGHNESS;
    let rough = in.finish >> 20u;
    if rough != 0u {
        roughness = f32(rough - 1u) / 255.0;
    }
    if highlighted(in.face) {
        base = scene.highlight.rgb;
        metallic = 0.0;
        roughness = DEFAULT_ROUGHNESS;
    }
    let p = scene.eye.xyz + in.rel;
    let v = toward_viewer(in.rel);
    var n = normal_of(in);
    if !(length(n) > 0.0) {
        n = v;
    }
    if dot(n, v) < 0.0 {
        n = -n;
    }
    var ao = 1.0;
    if studio.lv_w.w > 0.5 {
        ao = bitcast<f32>(textureLoad(ao_map, vec2<u32>(u32(in.clip.x), u32(in.clip.y)), 0).r);
    }
    return vec4<f32>(display(studio_shade(base, metallic, roughness, p, n, v, ao)), 1.0);
}

@fragment
fn fs_studio(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.data = data_of(in);
    out.color = studio_of(in);
    {{SET_DEPTH}}
    return out;
}

@fragment
fn fs_studio_color(in: VertexOut) -> ColorOut {
    var out: ColorOut;
    out.color = studio_of(in);
    {{SET_DEPTH}}
    return out;
}
