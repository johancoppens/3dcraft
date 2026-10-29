//! The studio look against physics and closed forms: a furnace, a mirror
//! sphere, Lambertian irradiance, the ground's shadow, determinism
//! (docs/design/rendering.md, *The studio look*).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use craft3d_math::Vec3;
use craft3d_render::studio::{
    FrameLight, HdrImage, Look, Prepared, Tonemap, env::texel_direction, render_studio, tonemap,
};
use craft3d_render::{Camera, FaceColors, Finish, NO_FACE, Projection, RenderParams};
use craft3d_tess::Mesh;
use std::path::PathBuf;

/// An equirectangular picture of a function of the direction.
fn environment(w: usize, f: impl Fn(Vec3) -> [f64; 3]) -> HdrImage {
    let h = w / 2;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(f(texel_direction(x, y, w, h)).map(|v| v as f32));
        }
    }
    HdrImage {
        width: w,
        height: h,
        pixels,
    }
}

/// A UV sphere with exact normals.
fn sphere(center: Vec3, r: f64, rings: u32, segments: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for i in 0..=rings {
        let t = std::f64::consts::PI * f64::from(i) / f64::from(rings);
        for j in 0..=segments {
            let p = 2.0 * std::f64::consts::PI * f64::from(j) / f64::from(segments);
            let n = Vec3::new(t.sin() * p.cos(), t.sin() * p.sin(), t.cos());
            mesh.positions.push(center + n * r);
            mesh.normals.push(n);
        }
    }
    let row = segments + 1;
    for i in 0..rings {
        for j in 0..segments {
            let (a, b, c, d) = (
                i * row + j,
                i * row + j + 1,
                (i + 1) * row + j,
                (i + 1) * row + j + 1,
            );
            mesh.triangles.push([a, c, b]);
            mesh.triangles.push([b, c, d]);
            mesh.face_ids.push(0);
            mesh.face_ids.push(0);
        }
    }
    mesh
}

fn front_camera() -> Camera {
    Camera {
        eye: Vec3::new(0.0, -10.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 1.25 },
    }
}

fn plain_look() -> Look {
    Look {
        tonemap: Tonemap::Linear,
        ao: false,
        shadows: false,
        ground: false,
        ..Look::default()
    }
}

fn evidence(name: &str, frame: &craft3d_render::Frame) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/studio");
    let _ = std::fs::create_dir_all(&dir);
    let png = frame.to_png().unwrap();
    let _ = std::fs::write(dir.join(name), &png);
    craft3d_testkit::evidence::print(name, &png);
}

#[test]
fn a_white_sphere_in_a_white_furnace_keeps_its_energy() {
    let env = Prepared::new(&environment(64, |_| [1.0; 3])).unwrap();
    assert!(env.key.is_none(), "an even environment has no key light");
    let mesh = sphere(Vec3::ZERO, 1.0, 4, 8);
    let light = FrameLight::new(&mesh, &env, &plain_look());
    // Over normals at every angle to the viewer, dielectric and metal,
    // smooth to rough: the light that leaves equals the light that comes
    // in, within the documented band.
    let v = Vec3::Z;
    let mut worst: f64 = 0.0;
    for metallic in [0_u8, 255] {
        for roughness in [0_u8, 64, 128, 192, 255] {
            for i in 0..20 {
                let a = f64::from(i) / 20.0 * 1.5;
                let n = Vec3::new(a.sin(), 0.0, a.cos());
                let finish = Finish {
                    metallic,
                    roughness: Some(roughness),
                };
                let c = light.shade([255; 3], finish, Vec3::ZERO, n, v, 1.0);
                for c in c {
                    worst = worst.max((c - 1.0).abs());
                    assert!(
                        (0.98..=1.02).contains(&c),
                        "m {metallic} r {roughness} at {a}: {c}"
                    );
                }
            }
        }
    }
    eprintln!("furnace: worst departure {worst:.4}");
    // End to end: at exposure −1 every pixel of the sphere is 0.5 in.
    let mesh = sphere(Vec3::ZERO, 1.0, 64, 128);
    let mut colors = FaceColors::new();
    colors.set_look(
        0,
        Some((
            [255; 3],
            Finish {
                metallic: 0,
                roughness: Some(255),
            },
        )),
    );
    let params = RenderParams {
        width: 96,
        height: 96,
        ssaa: 1,
        ..RenderParams::default()
    };
    let look = Look {
        exposure: -1.0,
        ..plain_look()
    };
    let frame = render_studio(&mesh, &colors, &front_camera(), &params, &look, &env).unwrap();
    let (lo, hi) = (
        tonemap::to_byte(tonemap::srgb_encode(0.49)),
        tonemap::to_byte(tonemap::srgb_encode(0.51)),
    );
    for (i, &id) in frame.ids.iter().enumerate() {
        let c = frame.rgb[3 * i];
        if id != NO_FACE {
            assert!((lo..=hi).contains(&c), "pixel {i}: {c} not in {lo}..={hi}");
        } else {
            // The background is the environment, 1 · 2^−1 too.
            assert!((lo..=hi).contains(&c), "background {i}: {c}");
        }
    }
    evidence("furnace.png", &frame);
}

#[test]
fn a_lambertian_surface_has_the_closed_form_irradiance() {
    // L(ω) = 1 + 0.5 ω_z: E(n) = π + (2π/3)·0.5·n_z (its mean is 1, so the
    // normalisation leaves it as it is).
    let env = Prepared::new(&environment(256, |d| [1.0 + 0.5 * d.z; 3])).unwrap();
    assert!(env.key.is_none());
    for i in 0..=12 {
        let a = std::f64::consts::PI * f64::from(i) / 12.0;
        let n = Vec3::new(a.sin() * 0.6, a.sin() * 0.8, a.cos());
        let e = env.irradiance(n);
        let want = std::f64::consts::PI + 2.0 * std::f64::consts::PI / 3.0 * 0.5 * n.z;
        for c in e {
            assert!((c - want).abs() < 2e-3 * want, "n {n:?}: {c} vs {want}");
        }
    }
}

#[test]
fn a_mirror_sphere_shows_the_environment() {
    let f = |d: Vec3| [0.5 + 0.4 * d.x, 0.5 + 0.3 * d.y, 0.6 + 0.3 * d.z];
    let env = Prepared::new(&environment(1024, f)).unwrap();
    let mesh = sphere(Vec3::ZERO, 1.0, 128, 256);
    let mut colors = FaceColors::new();
    colors.set_look(
        0,
        Some((
            [255; 3],
            Finish {
                metallic: 255,
                roughness: Some(0),
            },
        )),
    );
    let params = RenderParams {
        width: 120,
        height: 120,
        ssaa: 1,
        ..RenderParams::default()
    };
    let look = Look {
        tonemap: Tonemap::Agx,
        ..plain_look()
    };
    let frame = render_studio(&mesh, &colors, &front_camera(), &params, &look, &env).unwrap();
    evidence("mirror.png", &frame);
    // The mean luminance of `f` over the sphere is its constant part.
    let scale = 1.0 / (0.2126 * 0.5 + 0.7152 * 0.5 + 0.0722 * 0.6);
    let mut checked = 0;
    for y in 0..120 {
        for x in 0..120 {
            let (cx, cy) = (
                ((f64::from(x) + 0.5) / 120.0 * 2.0 - 1.0) * 1.25,
                (1.0 - (f64::from(y) + 0.5) / 120.0 * 2.0) * 1.25,
            );
            let rr = cx * cx + cy * cy;
            if rr > 0.8 {
                continue;
            }
            let n = Vec3::new(cx, -(1.0 - rr).sqrt(), cy);
            let v = Vec3::new(0.0, -1.0, 0.0);
            let r = n * (2.0 * n.dot(v)) - v;
            let want =
                tonemap::display(f(r).map(|c| c * scale), Tonemap::Agx).map(tonemap::to_byte);
            let i = 3 * (y * 120 + x) as usize;
            for (k, &want) in want.iter().enumerate() {
                let got = frame.rgb[i + k];
                assert!(
                    got.abs_diff(want) <= 2,
                    "({x}, {y}) channel {k}: {got} vs {want}"
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 5000, "{checked}");
}

#[test]
fn the_ground_under_a_lit_ball_is_in_its_shadow() {
    // A bright spot straight up: the key light, from above.
    let env = Prepared::new(&environment(256, |d| {
        if d.z > 0.99 { [400.0; 3] } else { [0.5; 3] }
    }))
    .unwrap();
    let key = env.key.expect("a bright spot is a key light");
    assert!(key.direction[2] > 0.99, "{key:?}");
    // A ball above the ground: the ground is at the bottom of the mesh,
    // so put a small speck at z = −3 to set it there.
    let mut mesh = sphere(Vec3::ZERO, 1.0, 32, 64);
    let speck = sphere(Vec3::new(2.5, 2.5, -3.0), 0.01, 4, 8);
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(speck.positions);
    mesh.normals.extend(speck.normals);
    mesh.triangles
        .extend(speck.triangles.iter().map(|t| t.map(|k| k + base)));
    mesh.face_ids.extend(speck.face_ids);
    let look = Look {
        shadows: true,
        ground: true,
        ..plain_look()
    };
    let light = FrameLight::new(&mesh, &env, &look);
    let down = Vec3::new(0.0, 0.0, -1.0);
    // The ground's factor is (rest + key · v) / (rest + key), with v the
    // key's visibility and rest the light from everywhere else (what the
    // key extraction leaves in the irradiance, the cap below its
    // threshold among it); v is read back from it.
    let key_light = light.key_luminance * light.key.map_or(0.0, |(l, _, _)| l.z.max(0.0));
    let rest = light.rest_up;
    assert!(
        key_light > rest,
        "the key is the main light: {key_light} {rest}"
    );
    let visibility = |x: f64, y: f64| {
        let f = light.ground_factor(Vec3::new(x, y, 5.0), down);
        (f * (rest + key_light) - rest) / key_light
    };
    // The light is the cap z > 0.99, of angular radius α = acos 0.99 ≈
    // 0.1415. A ray from the ground (z = −3) at distance ρ from the axis,
    // tilted by θ towards or away from it, passes the ball's centre at
    // |3 sin θ + ρ cos θ|: blocked below 1. Within ρ = 0.5 every ray to
    // the cap is blocked (3 sin α + 0.5 cos α ≈ 0.92); at ρ = 0.8 about a
    // fifth of the cap is seen; at ρ = 1.35 all but about 4 %; at ρ = 3.5
    // all of it. The filter's width is capped (`MAX_PENUMBRA` texels), so
    // the drawn penumbra is narrower than this: the bounds below hold for
    // both.
    let under = visibility(0.0, 0.0);
    assert!(under < 0.05, "under the centre: {under}");
    assert_eq!(light.ground_factor(Vec3::new(-2.5, -2.5, 5.0), down), 1.0);
    for k in 0..16 {
        let a = f64::from(k) / 16.0 * std::f64::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        let umbra = visibility(0.3 * c, 0.3 * s);
        let inside = visibility(0.8 * c, 0.8 * s);
        let beyond = visibility(1.35 * c, 1.35 * s);
        assert!(
            umbra < 0.05 && inside < 0.5 && beyond > 0.9,
            "at {a}: {umbra} {inside} {beyond}"
        );
    }
}

#[test]
fn the_same_input_gives_the_same_picture() {
    let env = Prepared::new(&environment(128, |d| {
        if d.z > 0.9 && d.x > 0.2 {
            [60.0, 50.0, 40.0]
        } else {
            [0.4, 0.45, 0.5]
        }
    }))
    .unwrap();
    let mesh = sphere(Vec3::ZERO, 1.0, 32, 64);
    let params = RenderParams {
        width: 80,
        height: 60,
        ssaa: 2,
        ..RenderParams::default()
    };
    let cam = Camera {
        eye: Vec3::new(3.0, -4.0, 2.5),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let a = render_studio(
        &mesh,
        &FaceColors::new(),
        &cam,
        &params,
        &Look::default(),
        &env,
    )
    .unwrap();
    let b = render_studio(
        &mesh,
        &FaceColors::new(),
        &cam,
        &params,
        &Look::default(),
        &env,
    )
    .unwrap();
    assert_eq!(a, b);
    evidence("ball.png", &a);
}

#[test]
fn bad_environments_and_numbers_are_errors_not_crashes() {
    let empty = HdrImage {
        width: 0,
        height: 0,
        pixels: Vec::new(),
    };
    assert!(Prepared::new(&empty).is_err());
    let black = environment(16, |_| [0.0; 3]);
    assert!(Prepared::new(&black).is_err());
    let wrong = HdrImage {
        width: 4,
        height: 2,
        pixels: vec![[1.0; 3]; 3],
    };
    assert!(Prepared::new(&wrong).is_err());
    let nan = environment(16, |d| if d.z > 0.5 { [f64::NAN; 3] } else { [1.0; 3] });
    let env = Prepared::new(&nan).unwrap();
    let look = Look {
        exposure: f64::NAN,
        rotation: f64::INFINITY,
        light: Some([f64::NAN, 1e300]),
        ..Look::default()
    };
    let mesh = sphere(Vec3::ZERO, 1.0, 8, 16);
    let params = RenderParams {
        width: 16,
        height: 16,
        ..RenderParams::default()
    };
    render_studio(
        &mesh,
        &FaceColors::new(),
        &front_camera(),
        &params,
        &look,
        &env,
    )
    .unwrap();
    // A corner past the last vertex.
    let broken = Mesh {
        triangles: vec![[0, 1, mesh.positions.len() as u32]],
        face_ids: vec![0],
        ..mesh.clone()
    };
    assert!(
        render_studio(
            &broken,
            &FaceColors::new(),
            &front_camera(),
            &params,
            &look,
            &env
        )
        .is_err()
    );
    assert!(craft3d_render::studio::decode_hdr(b"#?RADIANCE\n\n-Y 2 +X 2\n").is_err());
}

#[test]
fn the_bundled_environments_light_a_row_of_finishes() {
    use craft3d_render::studio::bundled::{ENVIRONMENTS, environment};
    // Five balls on the ground: plain, glossy red, matte red, gold metal,
    // a mirror.
    let finishes: [([u8; 3], Finish); 5] = [
        ([198, 204, 214], Finish::default()),
        (
            [200, 40, 40],
            Finish {
                metallic: 0,
                roughness: Some(38),
            },
        ),
        (
            [200, 40, 40],
            Finish {
                metallic: 0,
                roughness: Some(230),
            },
        ),
        (
            [255, 200, 90],
            Finish {
                metallic: 255,
                roughness: Some(77),
            },
        ),
        (
            [250, 250, 250],
            Finish {
                metallic: 255,
                roughness: Some(0),
            },
        ),
    ];
    let mut mesh = Mesh::default();
    let mut colors = FaceColors::new();
    for (k, (color, finish)) in finishes.iter().enumerate() {
        let ball = sphere(Vec3::new(k as f64 * 2.4 - 4.8, 0.0, 1.0), 1.0, 48, 96);
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(ball.positions);
        mesh.normals.extend(ball.normals);
        mesh.triangles
            .extend(ball.triangles.iter().map(|t| t.map(|i| i + base)));
        mesh.face_ids.extend(ball.face_ids.iter().map(|_| k as u32));
        colors.set_look(k as u32, Some((*color, *finish)));
    }
    let cam = Camera {
        eye: Vec3::new(0.0, -14.0, 5.0),
        target: Vec3::new(0.0, 0.0, 0.6),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let params = RenderParams {
        width: 480,
        height: 220,
        ssaa: 2,
        ..RenderParams::default()
    };
    for name in ENVIRONMENTS {
        let env = environment(name).unwrap();
        eprintln!("{name}: key {:?}", env.key);
        let started = std::time::Instant::now();
        let frame = render_studio(&mesh, &colors, &cam, &params, &Look::default(), &env).unwrap();
        eprintln!(
            "{name}: {:?} for {} triangles",
            started.elapsed(),
            mesh.triangles.len()
        );
        evidence(&format!("balls-{name}.png"), &frame);
    }
}

/// The light of `level` (luminance) on a surface of unit normal `n`, by
/// brute force over its texels: ∫ L(ω) max(n·ω, 0) dω, with each texel's
/// exact solid angle (its band of latitude), apart from the code's own.
fn brute_irradiance(level: &craft3d_render::studio::Level, n: Vec3) -> f64 {
    use std::f64::consts::PI;
    let (w, h) = (level.width, level.height);
    let mut sum = 0.0;
    for y in 0..h {
        let (t0, t1) = (y as f64 / h as f64 * PI, (y + 1) as f64 / h as f64 * PI);
        let dw = 2.0 * PI / w as f64 * (t0.cos() - t1.cos());
        let theta = (y as f64 + 0.5) / h as f64 * PI;
        for x in 0..w {
            let phi = ((x as f64 + 0.5) / w as f64 - 0.5) * 2.0 * PI;
            let d = Vec3::new(
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            );
            let c = level.texels[y * w + x];
            sum += lum(c) * n.dot(d).max(0.0) * dw;
        }
    }
    sum
}

/// The power of `level` (luminance): ∫ L dω, with the same solid angles.
fn power(level: &craft3d_render::studio::Level) -> f64 {
    use std::f64::consts::PI;
    let (w, h) = (level.width, level.height);
    let mut sum = 0.0;
    for y in 0..h {
        let (t0, t1) = (y as f64 / h as f64 * PI, (y + 1) as f64 / h as f64 * PI);
        let dw = 2.0 * PI / w as f64 * (t0.cos() - t1.cos());
        for x in 0..w {
            sum += lum(level.texels[y * w + x]) * dw;
        }
    }
    sum
}

fn lum(c: [f32; 3]) -> f64 {
    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
}

/// The key light and the irradiance without it carry the environment's
/// light (docs/design/rendering.md, *The studio look*, *Tests*): the
/// environment (its mirror level, as both renderers read it) against
/// brute force, for a bright cap and every bundled environment.
///
/// - Over all normals: the mean irradiance of any light is a quarter of
///   its power `P = ∫ L dω`, and of the model it is the spherical
///   harmonics' constant term plus a quarter of the key's irradiance `K`
///   (neither the truncation to nine terms nor the key's single direction
///   changes a mean over the sphere). Equal up to rounding: 0.2 %.
/// - At fourteen normals (the axes and the diagonals) and towards and away
///   from the key: `irradiance(n) + K max(n·l, 0)` within the sum of two
///   bounds of the brute force. (a) Nine harmonics of a light of power
///   `Q ≥ 0` are off by at most `3/32 Q` anywhere: for a point light the
///   error is `|1/4 + c/2 + 5/32 (3c² − 1) − max(c, 0)|`, largest at
///   `c = 0`, and any light is a sum of point lights; here `Q = P − K`.
///   (b) The key's excess lies within `KEY_CONE` of the brightest
///   direction, so within `ρ = 2 KEY_CONE` of `l`: on the lit side
///   (`n·l ≥ sin ρ`) the cosine is linear over it and the single direction
///   is off by at most `K (1 − cos ρ)`; on the dark side (`n·l ≤ −sin ρ`)
///   both are 0; between, the clamped cosine moves by at most the chord,
///   `2 K sin(ρ / 2)`. Plus 0.2 % of `P` for rounding. A scale error in
///   the key (its solid angles, the excess's factor, its sum) breaks the
///   mean by far more than its band; the bounds catch a key in the wrong
///   place or the irradiance of the wrong light.
#[test]
fn the_key_and_the_rest_carry_the_environments_light() {
    use craft3d_render::studio::bundled::{ENVIRONMENTS, environment as bundled};
    use craft3d_render::studio::env::KEY_CONE;
    let cap = Prepared::new(&environment(256, |d| {
        if d.z > 0.99 { [400.0; 3] } else { [0.5; 3] }
    }))
    .unwrap();
    let mut all = vec![("bright cap".to_string(), std::sync::Arc::new(cap))];
    for name in ENVIRONMENTS {
        all.push((name.to_string(), bundled(name).unwrap()));
    }
    let rho = 2.0 * KEY_CONE;
    for (name, env) in &all {
        let level = &env.levels[0];
        let p = power(level);
        let (k, l) = env.key.map_or((0.0, Vec3::Z), |key| {
            let [x, y, z] = key.direction.map(f64::from);
            (lum(key.irradiance), Vec3::new(x, y, z))
        });
        assert!(k <= p, "{name}: key {k} of {p}");
        let mean = lum(env.sh[0]) * 0.282_094_791 + k / 4.0;
        assert!(
            (mean - p / 4.0).abs() <= 2e-3 * p / 4.0,
            "{name}: mean {mean} against {}",
            p / 4.0
        );
        let mut normals = Vec::new();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            normals.extend([axis, -axis]);
        }
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    normals.push(Vec3::new(sx, sy, sz).normalized().unwrap());
                }
            }
        }
        if env.key.is_some() {
            normals.extend([l, -l]);
        }
        for n in normals {
            let brute = brute_irradiance(level, n);
            let e = env.irradiance(n);
            let model = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2] + k * n.dot(l).max(0.0);
            let nl = n.dot(l);
            let key_bound = if env.key.is_none() || nl <= -rho.sin() {
                0.0
            } else if nl >= rho.sin() {
                k * (1.0 - rho.cos())
            } else {
                2.0 * k * (rho / 2.0).sin()
            };
            let bound = 3.0 / 32.0 * (p - k) + key_bound + 2e-3 * p;
            eprintln!(
                "{name} n {n:?}: brute {brute:.4} model {model:.4} (bound {bound:.4}, P {p:.3}, K {k:.3})"
            );
            assert!(
                (model - brute).abs() <= bound,
                "{name} at {n:?}: {model} against {brute} (bound {bound})"
            );
        }
    }
}

/// What preparing an environment costs, printed for the design document
/// (*Cost*): the split-sum table (once per process) and each bundled
/// environment's preparation, the moment the desktop's thread or the
/// browser's page spends before the studio look can be drawn.
#[test]
fn preparing_an_environment_costs() {
    use craft3d_render::studio::brdf::brdf_lut;
    use craft3d_render::studio::bundled::{ENVIRONMENTS, bundled};
    let started = std::time::Instant::now();
    let lut = brdf_lut();
    eprintln!("PREPARE split-sum table: {:?}", started.elapsed());
    for name in ENVIRONMENTS {
        let bytes = bundled(name).unwrap();
        let started = std::time::Instant::now();
        let image = craft3d_render::studio::decode_hdr(bytes).unwrap();
        let decoded = started.elapsed();
        Prepared::with_lut(&image, lut.clone()).unwrap();
        eprintln!(
            "PREPARE {name}: decoded in {decoded:?}, prepared in {:?} ({}x{})",
            started.elapsed() - decoded,
            image.width,
            image.height
        );
    }
}
