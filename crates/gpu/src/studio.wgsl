// The studio look (`craft3d_render::studio`), appended to `scene.wgsl`.
// Every function mirrors the CPU renderer's of the same name; the comments
// name it. Positions are relative to the mesh origin, as in the scene.
//
// Passes: the shadow map (`vs_shadow`, `fs_shadow`), the geometry of the
// samples (`fs_geometry`: view distance and normal), their ambient
// occlusion (`fs_ao`), the background (`fs_background`) and the surfaces
// (`fs_studio`, `fs_studio_color`). Layer 2 (`craft3d_render::studio::polish`):
// every mesh pass of the look moves its samples by the frame's jitter
// (`vs_studio`), the surfaces take the contact shadows and the specular
// shadow, and with the screen-space buffers (`polish.wgsl`) the
// background and the surfaces write their linear light and specular
// weight instead of the picture (`fs_background_post`, `fs_studio_post`).

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
// The roughness that marks the plain background (studio::PLAIN_BACKDROP).
const PLAIN_BACKDROP: f32 = -2.0;
const CONTACT_STEPS: i32 = 12;
const CONTACT_BIAS: f32 = 0.5;
// Layer 3 (`craft3d_render::studio::layers`).
const TANGENT_EPSILON: f32 = 0.0001;
const MIN_ALPHA: f32 = 0.002;
// `craft3d_gpu::mesh::optics_word`: the slot, and the bit of a face that
// lets light through.
const OPTICS_SLOT: u32 = 0xffffu;
const TRANSMISSIVE: u32 = 0x10000u;

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
    // Layer 2. x, y: where the frame's samples lie off their cells'
    // middles, in samples (`polish::jitter`). z: 1 with contact shadows.
    // w: 1 with the specular shadow.
    jitter: vec4<f32>,
    // x: 1 with screen-space reflections. y: the ground's reflection
    // (0 to 1). z: bloom's share (0 to 1). w: the levels of bloom's
    // pyramid, the picture's included.
    polish: vec4<f32>,
    // In model units: how far a reflected ray goes and how thick a
    // surface is taken to be; the same of a contact-shadow ray.
    reach: vec4<f32>,
    // The lens (`polish::Lens`): x: a h / (4 half zf), the blur radius of
    // the background in samples before the cap. y: the focus distance.
    // z: the cap in samples. w: 1 with depth of field.
    lens: vec4<f32>,
    // rgb: the picture's plain background colour, linear. w: 1 when the
    // background is that colour instead of the environment.
    backdrop: vec4<f32>,
    sh: array<vec4<f32>, 9>,
    // The lens's taps of the frame (`polish::dof_taps`), two per vector.
    taps: array<vec4<f32>, 40>,
}

@group(1) @binding(0) var<uniform> studio: Studio;
@group(1) @binding(1) var env_map: texture_2d<f32>;
@group(1) @binding(2) var lut_map: texture_2d<f32>;
@group(1) @binding(3) var shadow_map: texture_2d<u32>;
@group(1) @binding(4) var ao_map: texture_2d<u32>;
@group(1) @binding(5) var geometry_map: texture_2d<u32>;
// Layer 3: the material table (`mesh::optics_table`); the light of the
// picture without the faces that let light through (`f32` bits, as the
// screen-space buffers); the view distance of the nearest back face of
// one of those faces per sample (`f32` bits, infinity for none).
@group(1) @binding(9) var optics_map: texture_2d<u32>;
@group(1) @binding(10) var behind_map: texture_2d<u32>;
@group(1) @binding(11) var backs_map: texture_2d<u32>;

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

// polish::ggx_dv: GGX D and the height-correlated Smith visibility.
fn ggx_dv(alpha: f32, nh: f32, nv: f32, nl: f32) -> f32 {
    let a2 = alpha * alpha;
    let k = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * k * k);
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let out = d * (0.5 / (gv + gl));
    // Not finite (NaN included): none.
    if !(abs(out) <= 3.4e38) {
        return 0.0;
    }
    return max(out, 0.0);
}

// polish_key_specular, per channel of F0.
fn key_specular(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, r: f32, radius: f32,
                f0: vec3<f32>) -> vec3<f32> {
    let len = length(v + l);
    if !(len > 0.0) {
        return vec3<f32>(0.0);
    }
    let hv = (v + l) / len;
    let alpha = clamp(r * r + 0.5 * radius, 0.002, 1.0);
    let nh = clamp(dot(n, hv), 0.0, 1.0);
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    let nl = clamp(dot(n, l), 0.0, 1.0);
    let vh = clamp(dot(v, hv), 0.0, 1.0);
    let f = f0 + (vec3<f32>(1.0) - f0) * powi(1.0 - vh, 5u);
    return f * ggx_dv(alpha, nh, nv, nl);
}

// studio::Shaded: the linear light, and the weight of the environment's
// specular light in it per channel with the roughness; and what the light
// behind a transmissive surface is multiplied by (layer 3).
struct Shaded {
    light: vec3<f32>,
    weight: vec3<f32>,
    roughness: f32,
    transmit: vec3<f32>,
}

// Layer 3 (`craft3d_render::studio::layers`): a face's layers beyond its
// colour and finish, from its row of the material table
// (`craft3d_gpu::mesh::optics_table`).
struct Optics {
    anisotropy: f32,
    clearcoat: f32,
    clearcoat_roughness: f32,
    transmission: f32,
    ior: f32,
    thin: bool,
    subsurface: f32,
    around: bool,
    absorption: vec3<f32>,
    has_tint: bool,
    // The subsurface colour's bytes.
    tint: vec3<f32>,
    axis: vec3<f32>,
    // Relative to the mesh origin.
    origin: vec3<f32>,
}

fn optics_texel(row: i32, k: i32) -> vec4<f32> {
    return bitcast<vec4<f32>>(textureLoad(optics_map, vec2<i32>(k, row), 0));
}

// The layers of the slot `slot` (row plus one; 0: none, Optics::default).
fn optics_of(slot: u32) -> Optics {
    var o: Optics;
    o.ior = 1.5;
    if slot == 0u {
        return o;
    }
    let row = i32(slot - 1u);
    let a = optics_texel(row, 0);
    let b = optics_texel(row, 1);
    let c = optics_texel(row, 2);
    o.anisotropy = a.x;
    o.clearcoat = a.y;
    o.clearcoat_roughness = a.z;
    o.transmission = a.w;
    o.ior = b.x;
    o.thin = b.y > 0.5;
    o.subsurface = b.z;
    o.around = b.w > 0.5;
    o.absorption = c.xyz;
    o.has_tint = c.w > 0.5;
    o.tint = optics_texel(row, 3).xyz;
    o.axis = optics_texel(row, 4).xyz;
    o.origin = optics_texel(row, 5).xyz;
    return o;
}

// TangentField::at: the unit tangent in xyz and w = 1, or w = 0 where the
// field gives none (TANGENT_EPSILON). `p` relative to the mesh origin.
fn tangent_at(o: Optics, p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    var raw = o.axis;
    if o.around {
        raw = cross(o.axis, p - o.origin);
    }
    let along = raw - n * dot(n, raw);
    let len = length(along);
    let full = length(raw);
    if !(len <= 3.4e38 && full <= 3.4e38 && len > TANGENT_EPSILON * full) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(along / len, 1.0);
}

// layers::wrap_bands.
fn wrap_bands(w_in: f32) -> vec3<f32> {
    let w = clamp(w_in, 0.0, 1.0);
    let s = (1.0 + w) * (1.0 + w);
    return vec3<f32>(1.0, (1.0 + 1.5 * w - 0.5 * w * w * w) / s, (1.0 - w) * (1.0 - w));
}

// layers::wrapped_cosine.
fn wrapped_cosine(nl: f32, w_in: f32) -> f32 {
    let w = clamp(w_in, 0.0, 1.0);
    return max(nl + w, 0.0) / ((1.0 + w) * (1.0 + w));
}

// FrameLight::irradiance_wrapped: the irradiance through the wrapped
// kernel of `w`, band by band.
fn irradiance_wrapped(n_in: vec3<f32>, w: f32) -> vec3<f32> {
    let bands = wrap_bands(w);
    let c = cos(studio.look.x);
    let s = sin(studio.look.x);
    let n = vec3<f32>(c * n_in.x + s * n_in.y, -s * n_in.x + c * n_in.y, n_in.z);
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = studio.sh[0].rgb * (0.282094791 * bands.x);
    e += studio.sh[1].rgb * (0.488602512 * y * bands.y);
    e += studio.sh[2].rgb * (0.488602512 * z * bands.y);
    e += studio.sh[3].rgb * (0.488602512 * x * bands.y);
    e += studio.sh[4].rgb * (1.092548431 * x * y * bands.z);
    e += studio.sh[5].rgb * (1.092548431 * y * z * bands.z);
    e += studio.sh[6].rgb * (0.315391565 * (3.0 * z * z - 1.0) * bands.z);
    e += studio.sh[7].rgb * (1.092548431 * x * z * bands.z);
    e += studio.sh[8].rgb * (0.546274215 * (x * x - y * y) * bands.z);
    return max(e, vec3<f32>(0.0));
}

// layers::aniso_ggx_dv.
fn aniso_ggx_dv(at: f32, ab: f32, t: vec3<f32>, b: vec3<f32>, n: vec3<f32>, h: vec3<f32>,
                v: vec3<f32>, l: vec3<f32>) -> f32 {
    let th = dot(t, h);
    let bh = dot(b, h);
    let nh = dot(n, h);
    let k = (th / at) * (th / at) + (bh / ab) * (bh / ab) + nh * nh;
    let d = 1.0 / (PI * at * ab * k * k);
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    let nl = clamp(dot(n, l), 0.0, 1.0);
    let lv = nl * length(vec3<f32>(at * dot(t, v), ab * dot(b, v), nv));
    let ll = nv * length(vec3<f32>(at * dot(t, l), ab * dot(b, l), nl));
    let out = d * (0.5 / (lv + ll));
    if !(abs(out) <= 3.4e38) {
        return 0.0;
    }
    return max(out, 0.0);
}

// aniso_key_specular, per channel of F0.
fn aniso_key_specular(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, t: vec3<f32>, b: vec3<f32>,
                      r: f32, a: f32, radius: f32, f0: vec3<f32>) -> vec3<f32> {
    let len = length(v + l);
    if !(len > 0.0 && len <= 3.4e38) {
        return vec3<f32>(0.0);
    }
    let hv = (v + l) / len;
    let alpha = clamp(r * r + 0.5 * radius, 0.002, 1.0);
    let at = max(alpha * (1.0 + a), MIN_ALPHA);
    let ab = max(alpha * (1.0 - a), MIN_ALPHA);
    let vh = clamp(dot(v, hv), 0.0, 1.0);
    let f = f0 + (vec3<f32>(1.0) - f0) * powi(1.0 - vh, 5u);
    return f * aniso_ggx_dv(at, ab, t, b, n, hv, v, l);
}

// layers::bent_reflection.
fn bent_reflection(n: vec3<f32>, v: vec3<f32>, t: vec3<f32>, b: vec3<f32>, a: f32,
                   r: f32) -> vec3<f32> {
    var direction = t;
    if a >= 0.0 {
        direction = b;
    }
    let sideways = cross(direction, v);
    let lobe = cross(sideways, direction);
    let bend = abs(a) * clamp(5.0 * r, 0.0, 1.0);
    let mixed = n * (1.0 - bend) + lobe * bend;
    let len = length(mixed);
    var bent = n;
    if len > 0.0 && len <= 3.4e38 {
        bent = mixed / len;
    }
    return bent * (2.0 * dot(bent, v)) - v;
}

// FrameLight::shade_layered: the light leaving the point, with the key's
// contact shadow `contact` (1: none) and the layers `o`. With none
// (`optics_of(0u)`) every layer enters as a factor of 1 or a term of 0:
// layer 2's shading.
fn studio_shade(base_bytes: vec3<f32>, metallic: f32, roughness: f32, p: vec3<f32>,
                n: vec3<f32>, v: vec3<f32>, ao: f32, contact: f32, o: Optics) -> Shaded {
    let base = vec3<f32>(
        srgb_decode(base_bytes.x / 255.0),
        srgb_decode(base_bytes.y / 255.0),
        srgb_decode(base_bytes.z / 255.0),
    );
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    // Anisotropy: its frame, where the field gives a tangent.
    var aniso = false;
    var t = vec3<f32>(0.0);
    var bt = vec3<f32>(0.0);
    if o.anisotropy != 0.0 {
        let tw = tangent_at(o, p, n);
        if tw.w > 0.5 {
            aniso = true;
            t = tw.xyz;
            bt = cross(n, t);
        }
    }
    var reflected = n * (2.0 * dot(n, v)) - v;
    if aniso {
        reflected = bent_reflection(n, v, t, bt, o.anisotropy, roughness);
    }
    let ab = split_sum(nv, roughness);
    let prefiltered = radiance(reflected, roughness * studio.look.w);
    let irr = irradiance(n);
    let wrap = o.subsurface;
    var wrapped = irr;
    if wrap > 0.0 {
        wrapped = irradiance_wrapped(n, wrap);
    }
    var key_light = vec3<f32>(0.0);
    var key_dot = 0.0;
    var key_vis = 1.0;
    var key_nl = 0.0;
    var key_wrap = 0.0;
    if studio.key.w > 0.5 {
        let cos_l = dot(n, studio.key.xyz);
        let nl = max(cos_l, 0.0);
        let lit = nl > 0.0 || (wrap > 0.0 && cos_l + wrap > 0.0);
        var vis = 1.0;
        if studio.lv_v.w > 0.5 && lit {
            vis = visibility(p, n, studio.key_light.w);
        }
        vis = vis * contact;
        key_light = studio.key_light.rgb;
        key_dot = nl * vis;
        key_vis = vis;
        key_nl = nl;
        key_wrap = wrapped_cosine(cos_l, wrap) * vis;
    }
    // Specular occlusion (Lagarde and de Rousiers) with alpha = r * r.
    let occlusion = pow(nv + ao, exp2(-16.0 * roughness * roughness - 1.0)) - 1.0 + ao;
    let specular_occlusion = clamp(occlusion, 0.0, 1.0);
    let sum_ab = ab.x + ab.y;
    let f0 = vec3<f32>(DIELECTRIC_F0 * (1.0 - metallic)) + base * metallic;
    let fs = f0 * ab.x + vec3<f32>(ab.y);
    var multiple = vec3<f32>(1.0);
    if sum_ab > 0.0 {
        multiple = vec3<f32>(1.0) + f0 * (1.0 / sum_ab - 1.0);
    }
    let weight = fs * multiple * specular_occlusion;
    let specular = prefiltered * fs * multiple * specular_occlusion;
    let light = irr * ao + key_light * key_dot;
    var diffuse = base * (1.0 - metallic) * (vec3<f32>(1.0) - fs) / PI * light;
    if wrap > 0.0 {
        var tint = base;
        if o.has_tint {
            tint = vec3<f32>(
                srgb_decode(o.tint.x / 255.0),
                srgb_decode(o.tint.y / 255.0),
                srgb_decode(o.tint.z / 255.0),
            );
        }
        let light_w = wrapped * ao + key_light * key_wrap;
        diffuse = diffuse * (1.0 - wrap)
            + tint * (1.0 - metallic) * (vec3<f32>(1.0) - fs) / PI * light_w * wrap;
    }
    var transmit = vec3<f32>(0.0);
    if o.transmission > 0.0 {
        diffuse = diffuse * (1.0 - o.transmission);
        transmit = o.transmission * (1.0 - metallic) * (vec3<f32>(1.0) - fs) * base;
    }
    // The key is part of the prefiltered light: where it is hidden its
    // share of that light is taken out (the specular shadow); on an
    // anisotropic face it is taken out everywhere, its share estimated at
    // the bent normal, and its anisotropic lobe added instead.
    var keep = 1.0;
    var key_aniso = vec3<f32>(0.0);
    if studio.key.w > 0.5 && key_nl > 0.0
        && ((studio.jitter.w > 0.5 && key_vis < 1.0) || aniso) {
        var share_n = n;
        var share_nl = key_nl;
        if aniso {
            let bent = reflected + v;
            let len = length(bent);
            if len > 0.0 && len <= 3.4e38 {
                share_n = bent / len;
            }
            share_nl = max(dot(share_n, studio.key.xyz), 0.0);
        }
        let ks = key_light * share_nl
            * key_specular(share_n, v, studio.key.xyz, roughness, studio.key_light.w, f0)
            * multiple;
        let total = luminance(specular);
        var share = 0.0;
        if total > 0.0 {
            share = clamp(luminance(ks) / total, 0.0, 1.0);
        }
        if aniso {
            keep = 1.0 - share;
            key_aniso = key_light * key_nl * key_vis
                * aniso_key_specular(n, v, studio.key.xyz, t, bt, roughness, o.anisotropy,
                                     studio.key_light.w, f0)
                * multiple;
        } else {
            keep = 1.0 - (1.0 - key_vis) * share;
        }
    }
    var out: Shaded;
    out.light = diffuse + specular * keep;
    out.weight = weight * keep;
    if aniso {
        // The bent lobe is no mirror: no reflection replaces it.
        out.weight = vec3<f32>(0.0);
        out.light = out.light + key_aniso;
    }
    out.roughness = roughness;
    out.transmit = transmit;
    if o.clearcoat > 0.0 {
        let rc = o.clearcoat_roughness;
        let abc = split_sum(nv, rc);
        let fc = clamp(o.clearcoat * (DIELECTRIC_F0 * abc.x + abc.y), 0.0, 1.0);
        let coat_occlusion = pow(nv + ao, exp2(-16.0 * rc * rc - 1.0)) - 1.0 + ao;
        let so = clamp(coat_occlusion, 0.0, 1.0);
        let mirror = n * (2.0 * dot(n, v)) - v;
        let coat_light = radiance(mirror, rc * studio.look.w);
        out.light = out.light * (1.0 - fc) + coat_light * fc * so;
        out.transmit = out.transmit * (1.0 - fc);
        // The reflections replace the coat's light, the sharpest layer.
        out.weight = vec3<f32>(fc * so);
        out.roughness = rc;
    }
    return out;
}

// View::ray and View::point_at: the offsets across the view of the
// position (px, py) of the target, in samples.
fn across(px_in: f32, py_in: f32) -> vec3<f32> {
    let px = px_in + studio.jitter.x;
    let py = py_in + studio.jitter.y;
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

// The vertex stage of every mesh pass of the studio look: the scene's,
// with the samples moved by the frame's jitter (`View::project` subtracts
// it from the sample position). The jitter of frame 0 is zero, which
// leaves the position as it was.
@vertex
fn vs_studio(in: VertexIn) -> VertexOut {
    var out = vertex(in);
    out.clip.x = out.clip.x - 2.0 * studio.jitter.x / studio.size.x * out.clip.w;
    out.clip.y = out.clip.y + 2.0 * studio.jitter.y / studio.size.y * out.clip.w;
    return out;
}

// View::ray through the position (px, py): its origin relative to the
// mesh origin and its unit direction.
fn view_ray(px: f32, py: f32) -> array<vec3<f32>, 2> {
    let a = across(px, py);
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
    return array<vec3<f32>, 2>(origin, d);
}

// FrameLight::background, through the position (px, py).
fn background_light(px: f32, py: f32) -> vec3<f32> {
    let ray = view_ray(px, py);
    return radiance(ray[1], BACKGROUND_ROUGHNESS * studio.look.w) * ground_factor(ray[0], ray[1]);
}

@fragment
fn fs_background(in: FullscreenOut) -> @location(0) vec4<f32> {
    if studio.backdrop.w > 0.5 {
        // The plain colour, darkened by the ground's shadow in linear
        // light; no exposure, no tone curve (`FrameLight::background_bytes`).
        let ray = view_ray(in.clip.x, in.clip.y);
        let b = studio.backdrop.rgb * ground_factor(ray[0], ray[1]);
        return vec4<f32>(srgb_encode(b.x), srgb_encode(b.y), srgb_encode(b.z), 1.0);
    }
    return vec4<f32>(display(background_light(in.clip.x, in.clip.y)), 1.0);
}

// The light and the weight of a sample, as bits: the screen-space buffers
// (`polish::Buffers`).
struct LightOut {
    @location(0) light: vec4<u32>,
    @location(1) weight: vec4<u32>,
}

fn bits3(c: vec3<f32>, w: f32) -> vec4<u32> {
    return vec4<u32>(bitcast<u32>(c.x), bitcast<u32>(c.y), bitcast<u32>(c.z), bitcast<u32>(w));
}

// The background's light, with no weight and the roughness -1; the plain
// background's (`PLAIN_BACKDROP`) the colour over the exposure, darkened
// by the ground's shadow, with the roughness -2, which the display shows
// without the tone curve.
@fragment
fn fs_background_post(in: FullscreenOut) -> LightOut {
    var out: LightOut;
    if studio.backdrop.w > 0.5 {
        let ray = view_ray(in.clip.x, in.clip.y);
        let b = studio.backdrop.rgb * ground_factor(ray[0], ray[1]) / studio.look.y;
        out.light = bits3(b, 0.0);
        out.weight = bits3(vec3<f32>(0.0), PLAIN_BACKDROP);
        return out;
    }
    out.light = bits3(background_light(in.clip.x, in.clip.y), 0.0);
    out.weight = bits3(vec3<f32>(0.0), -1.0);
    return out;
}

// The shadow map: the light's view of the mesh, the depth from the light
// as the bits of an f32 (`ShadowMap::draw`).
struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) depth: f32,
    @location(1) @interpolate(flat) optics: u32,
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
    out.optics = in.optics;
    return out;
}

// A face that lets light through casts no shadow (layer 3,
// `FrameLight::casting`).
@fragment
fn fs_shadow(in: ShadowOut) -> @location(0) u32 {
    if (in.optics & TRANSMISSIVE) != 0u {
        discard;
    }
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

// True for a fragment of a face that lets light through (layer 3).
fn transmissive(in: VertexOut) -> bool {
    return (in.optics & TRANSMISSIVE) != 0u;
}

// The geometry of the picture without the faces that let light through
// (layer 3: what they see).
@fragment
fn fs_geometry_opaque(in: VertexOut) -> GeometryOut {
    if transmissive(in) {
        discard;
    }
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

// polish::depth: the view distance of the sample (x, y) in x, and y = 1
// when it has a surface.
fn sample_depth(x: f32, y: f32) -> vec2<f32> {
    if !(x >= 0.0 && y >= 0.0 && x < studio.size.x && y < studio.size.y) {
        return vec2<f32>(0.0);
    }
    let dist = bitcast<f32>(textureLoad(geometry_map, vec2<i32>(i32(x), i32(y)), 0).x);
    if !(dist < 3.0e38) {
        return vec2<f32>(0.0);
    }
    return vec2<f32>(dist, 1.0);
}

// View::project of a point relative to the eye: the sample position, the
// view distance, and w = 1 when it projects (in front of the eye plane of
// a perspective view).
fn project_rel(rel: vec3<f32>) -> vec4<f32> {
    let cx = dot(rel, scene.right.xyz);
    let cy = dot(rel, scene.up.xyz);
    let dist = dot(rel, scene.forward.xyz);
    var scale = 1.0;
    if scene.forward.w > 0.5 {
        if !(dist > 1e-12) {
            return vec4<f32>(0.0);
        }
        scale = 1.0 / dist;
    }
    let sx = (cx * scene.right.w * scale * 0.5 + 0.5) * studio.size.x - studio.jitter.x;
    let sy = (0.5 - cy * scene.up.w * scale * 0.5) * studio.size.y - studio.jitter.y;
    if !(abs(sx) <= 3.4e38 && abs(sy) <= 3.4e38) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sx, sy, dist, 1.0);
}

// polish::contact at the point `rel` (relative to the eye) of the sample
// `start` towards the unit `l`: 0 when a short ray finds a surface on the
// screen in front of it, else 1.
fn contact(rel: vec3<f32>, l: vec3<f32>, start: vec2<f32>) -> f32 {
    let length_c = studio.reach.z;
    let thickness = studio.reach.w;
    for (var j = 1; j <= CONTACT_STEPS; j++) {
        let t = length_c * f32(j) / f32(CONTACT_STEPS);
        let q = project_rel(rel + l * t);
        if q.w < 0.5 {
            return 1.0;
        }
        let x = floor(q.x);
        let y = floor(q.y);
        if !(x >= 0.0 && y >= 0.0 && x < studio.size.x && y < studio.size.y) {
            return 1.0;
        }
        if x == start.x && y == start.y {
            continue;
        }
        let s = sample_depth(x, y);
        if s.y < 0.5 {
            continue;
        }
        let behind = q.z - s.x;
        if behind > CONTACT_BIAS * sample_size(s.x) && behind < thickness {
            return 0.0;
        }
    }
    return 1.0;
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

// The fragment's normal turned towards the viewer `v`.
fn facing_normal(in: VertexOut, v: vec3<f32>) -> vec3<f32> {
    var n = normal_of(in);
    if !(length(n) > 0.0) {
        n = v;
    }
    if dot(n, v) < 0.0 {
        n = -n;
    }
    return n;
}

// The light of the picture behind at the sample `at`.
fn light_behind(at: vec2<i32>) -> vec3<f32> {
    return bitcast<vec3<f32>>(textureLoad(behind_map, at, 0).xyz);
}

// layers::refract: the direction of the unit ray `d` refracted into a
// medium of index `ior` at the unit normal `n`, w = 1; w = 0 for none.
fn refract_ray(d: vec3<f32>, n: vec3<f32>, ior: f32) -> vec4<f32> {
    let eta = 1.0 / ior;
    let c = -dot(n, d);
    let k = 1.0 - eta * eta * (1.0 - c * c);
    if !(k >= 0.0 && k <= 3.4e38) {
        return vec4<f32>(0.0);
    }
    let r = d * eta + n * (eta * c - sqrt(k));
    let len = length(r);
    if !(len > 0.0 && len <= 3.4e38) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(r / len, 1.0);
}

// layers::slab_path.
fn slab_path(across_len: f32, nv: f32, n: vec3<f32>, refracted: vec3<f32>) -> f32 {
    let cos_t = -dot(n, refracted);
    if !(cos_t > 1e-6 && cos_t <= 3.4e38) {
        return across_len;
    }
    return across_len * nv / cos_t;
}

// refract::pass_through for the fragment `in` shaded as `shaded`: its
// light with the light from behind that passes through added.
fn through(in: VertexOut, shaded: Shaded) -> vec3<f32> {
    if all(shaded.transmit == vec3<f32>(0.0)) {
        return shaded.light;
    }
    let o = optics_of(in.optics & OPTICS_SLOT);
    let px = in.clip.x;
    let py = in.clip.y;
    let pixel = vec2<i32>(floor(in.clip.xy));
    let v = toward_viewer(in.rel);
    let n = facing_normal(in, v);
    let back = bitcast<f32>(textureLoad(backs_map, pixel, 0).r);
    var seen = light_behind(pixel);
    var absorbed = vec3<f32>(1.0);
    if !o.thin && back < 3.0e38 && back > in.dist {
        let r = refract_ray(-v, n, o.ior);
        if r.w > 0.5 {
            let across_len = length(point_at(px, py, back) - point_at(px, py, in.dist));
            let nv = clamp(dot(n, v), 1e-4, 1.0);
            let path = slab_path(across_len, nv, n, r.xyz);
            let out_at = in.rel + r.xyz * path;
            absorbed = exp(-o.absorption * path);
            let q = project_rel(out_at);
            if q.w > 0.5 && q.x >= 0.0 && q.y >= 0.0 && q.x < studio.size.x
                && q.y < studio.size.y {
                seen = light_behind(vec2<i32>(i32(floor(q.x)), i32(floor(q.y))));
            } else {
                // Off the picture: the background along the view ray.
                let ray = view_ray(px, py);
                seen = radiance(ray[1], BACKGROUND_ROUGHNESS * studio.look.w)
                    * ground_factor(scene.eye.xyz + out_at, ray[1]);
            }
        }
    }
    return shaded.light + shaded.transmit * absorbed * seen;
}

// The studio light of a fragment (`render_studio_frame`'s shading).
fn studio_of(in: VertexOut) -> Shaded {
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
    var slot = in.optics & OPTICS_SLOT;
    if highlighted(in.face) {
        base = scene.highlight.rgb;
        metallic = 0.0;
        roughness = DEFAULT_ROUGHNESS;
        slot = 0u;
    }
    let p = scene.eye.xyz + in.rel;
    let v = toward_viewer(in.rel);
    let n = facing_normal(in, v);
    var ao = 1.0;
    if studio.lv_w.w > 0.5 {
        ao = bitcast<f32>(textureLoad(ao_map, vec2<u32>(u32(in.clip.x), u32(in.clip.y)), 0).r);
    }
    var shadowed = 1.0;
    if studio.jitter.z > 0.5 && studio.key.w > 0.5 && dot(n, studio.key.xyz) > 0.0 {
        shadowed = contact(in.rel, studio.key.xyz, floor(in.clip.xy));
    }
    return studio_shade(base, metallic, roughness, p, n, v, ao, shadowed, optics_of(slot));
}

@fragment
fn fs_studio(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.data = data_of(in);
    out.color = vec4<f32>(display(studio_of(in).light), 1.0);
    {{SET_DEPTH}}
    return out;
}

@fragment
fn fs_studio_color(in: VertexOut) -> ColorOut {
    var out: ColorOut;
    out.color = vec4<f32>(display(studio_of(in).light), 1.0);
    {{SET_DEPTH}}
    return out;
}

// The screen-space buffers of a surface: 32 bytes a sample, the most a
// pass may write (`max_color_attachment_bytes_per_sample`), so the face
// ids and distances of the readback renderer come from a pass of their
// own (`fs_data` with `vs_studio`).
struct PostLightOut {
    @location(0) light: vec4<u32>,
    @location(1) weight: vec4<u32>,
    {{FRAG_DEPTH}}
}

// With the light from behind the faces that let light through (layer 3,
// `refract::pass_through`).
@fragment
fn fs_studio_post(in: VertexOut) -> PostLightOut {
    var out: PostLightOut;
    let shaded = studio_of(in);
    out.light = bits3(through(in, shaded), 0.0);
    out.weight = bits3(shaded.weight, shaded.roughness);
    {{SET_DEPTH}}
    return out;
}

// The picture without the faces that let light through (layer 3: what
// they see), shaded completely.
@fragment
fn fs_studio_behind(in: VertexOut) -> PostLightOut {
    if transmissive(in) {
        discard;
    }
    var out: PostLightOut;
    let shaded = studio_of(in);
    out.light = bits3(shaded.light, 0.0);
    out.weight = bits3(shaded.weight, shaded.roughness);
    {{SET_DEPTH}}
    return out;
}

struct BacksOut {
    @location(0) dist: u32,
    {{FRAG_DEPTH}}
}

// The nearest back faces of the faces that let light through: their view
// distance. A triangle faces away when its own normal points away from
// the eye (`rasterise_where`: flat · (corner − eye) > 0 in perspective,
// flat · forward in orthographic; on the triangle's plane flat · rel is
// the same everywhere).
@fragment
fn fs_backs(in: VertexOut) -> BacksOut {
    var towards = scene.forward.xyz;
    if scene.forward.w > 0.5 {
        towards = in.rel;
    }
    if !transmissive(in) || !(dot(in.flat_normal, towards) > 0.0) {
        discard;
    }
    var out: BacksOut;
    out.dist = bitcast<u32>(in.dist);
    {{SET_DEPTH}}
    return out;
}
