// The scene pass: one sample per target pixel, two colour targets (the shaded
// colour; the face id with the view distance) and a depth buffer. It follows
// `craft3d_render::render` step by step; the comments name the line of the
// CPU renderer each step mirrors.

struct Scene {
    // xyz: camera right axis. w: clip x per model unit (1 / (half * aspect)).
    right: vec4<f32>,
    // xyz: camera up axis. w: clip y per model unit (1 / half).
    up: vec4<f32>,
    // xyz: view direction. w: 1 for perspective, 0 for orthographic.
    forward: vec4<f32>,
    // xyz: the eye, relative to the mesh origin.
    eye: vec4<f32>,
    // xyz: unit vector towards the light. w: 1 when there is one.
    light: vec4<f32>,
    // x: near distance. y: depth scale (see vs_main).
    depth: vec4<f32>,
    // rgb: the base colour in 0..255.
    base: vec4<f32>,
    // ambient, diffuse, specular * 255, shininess.
    shade: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) flat_normal: vec3<f32>,
    @location(3) face: u32,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // From the eye to the surface point, in model axes.
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) @interpolate(flat) flat_normal: vec3<f32>,
    @location(3) @interpolate(flat) face: u32,
    // The view distance times the clip w. The CPU renderer interpolates the
    // distance with screen-space weights (`wa * a.dist + wb * b.dist +
    // wc * c.dist`), not perspective-correct ones. With screen weights b_i,
    // a perspective-correct varying v arrives as
    // sum(b_i v_i / w_i) / sum(b_i / w_i), and the fragment's position.w is
    // sum(b_i / w_i); so v = dist * w, times position.w, is sum(b_i dist_i):
    // the CPU's value. (`@interpolate(linear)` would say the same, but
    // lavapipe interpolates such a varying wrongly on a clipped triangle.)
    @location(4) dist_w: f32,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // View::project: rel = p - eye, then the three dot products.
    let rel = in.position - scene.eye.xyz;
    let cx = dot(rel, scene.right.xyz);
    let cy = dot(rel, scene.up.xyz);
    let dist = dot(rel, scene.forward.xyz);
    let x = cx * scene.right.w;
    let y = cy * scene.up.w;
    // The depth key must be affine in screen space and ordered like the CPU
    // key (-1 / dist in perspective, dist in orthographic).
    if scene.forward.w > 0.5 {
        // z / w = far * (dist - near) / ((far - near) * dist): 0 at near,
        // 1 at far, and affine in 1 / dist. depth.y = far / (far - near).
        out.clip = vec4<f32>(x, y, (dist - scene.depth.x) * scene.depth.y, dist);
        out.dist_w = dist * dist;
    } else {
        // depth.y = 1 / (far - near).
        out.clip = vec4<f32>(x, y, (dist - scene.depth.x) * scene.depth.y, 1.0);
        out.dist_w = dist;
    }
    out.rel = rel;
    out.normal = in.normal;
    out.flat_normal = in.flat_normal;
    out.face = in.face;
    return out;
}

struct FragmentOut {
    @location(0) color: vec4<f32>,
    // x: the face id. y: the bits of the view distance as an f32. An
    // integer target is renderable on every backend and keeps both exactly.
    @location(1) data: vec2<u32>,
}

// x to a non-negative integer power by squaring: exact where `pow` is
// implementation-defined (pow(0, n) is NaN on some drivers).
fn powi(x: f32, n: u32) -> f32 {
    var result = 1.0;
    var base = x;
    var e = n;
    loop {
        if e == 0u {
            break;
        }
        if (e & 1u) == 1u {
            result = result * base;
        }
        base = base * base;
        e = e >> 1u;
    }
    return result;
}

// The face id and the bits of the view distance of a fragment.
fn data_of(in: VertexOut) -> vec2<u32> {
    return vec2<u32>(in.face, bitcast<u32>(in.dist_w * in.clip.w));
}

// The shaded colour of a fragment, as the bytes of the picture over 255.
fn shade_of(in: VertexOut) -> vec4<f32> {
    // View::toward_viewer.
    var to_viewer = -scene.forward.xyz;
    if scene.forward.w > 0.5 {
        let len = length(in.rel);
        if len > 0.0 {
            to_viewer = -in.rel / len;
        }
    }

    // The interpolated normal, or the triangle's own where there is none.
    var n = vec3<f32>(0.0);
    var has_normal = false;
    let len_n = length(in.normal);
    if len_n > 0.0 {
        n = in.normal / len_n;
        has_normal = true;
    } else {
        let len_f = length(in.flat_normal);
        if len_f > 0.0 {
            n = in.flat_normal / len_f;
            has_normal = true;
        }
    }

    var rgb = scene.base.rgb;
    if has_normal {
        // shade(): two-sided, one light fixed to the camera.
        if dot(n, to_viewer) < 0.0 {
            n = -n;
        }
        var light = to_viewer;
        if scene.light.w > 0.5 {
            light = scene.light.xyz;
        }
        let diffuse = max(dot(n, light), 0.0);
        var specular = 0.0;
        let half_vector = light + to_viewer;
        let len_h = length(half_vector);
        if len_h > 0.0 {
            specular = powi(max(dot(n, half_vector / len_h), 0.0), u32(scene.shade.w));
        }
        let intensity = scene.shade.x + scene.shade.y * diffuse;
        rgb = scene.base.rgb * intensity + vec3<f32>(scene.shade.z * specular);
    }
    // Rounded here, so the target stores exactly this integer whatever the
    // driver's float-to-unorm conversion does with fractions.
    return vec4<f32>(clamp(round(rgb), vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0, 1.0);
}

// Colour and data: what the renderer that reads the frame back draws.
@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.data = data_of(in);
    out.color = shade_of(in);
    return out;
}

// Colour only: the picture a viewport shows, into a multisampled target.
@fragment
fn fs_color(in: VertexOut) -> @location(0) vec4<f32> {
    return shade_of(in);
}

// Data only: face ids and distances at one sample per pixel, for picking
// and for the tests of the viewport's picture.
@fragment
fn fs_data(in: VertexOut) -> @location(0) vec2<u32> {
    return data_of(in);
}
