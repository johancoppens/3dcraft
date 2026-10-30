//! Layer 3's materials against physics and closed forms: a white furnace
//! for every layer, the clear coat's attenuation of the base, the
//! anisotropic highlight's elongation, the screen-space refraction of a
//! glass slab, determinism (docs/design/rendering.md, *Layer 3:
//! materials*).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_range_loop
)]

use craft3d_math::Vec3;
use craft3d_render::studio::layers::{self, Optics, TangentField};
use craft3d_render::studio::{
    FrameLight, HdrImage, Look, Prepared, Tonemap, env::texel_direction, render_studio,
    render_studio_doubt,
};
use craft3d_render::{Camera, FaceColors, Finish, Projection, RenderParams};
use craft3d_tess::Mesh;
use std::path::PathBuf;

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
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/layers");
    let _ = std::fs::create_dir_all(&dir);
    let png = frame.to_png().unwrap();
    let _ = std::fs::write(dir.join(name), &png);
    craft3d_testkit::evidence::print(name, &png);
}

fn finish(metallic: u8, roughness: u8) -> Finish {
    Finish {
        metallic,
        roughness: Some(roughness),
    }
}

fn furnace() -> Prepared {
    let env = Prepared::new(&environment(64, |_| [1.0; 3])).unwrap();
    assert!(env.key.is_none());
    env
}

/// The light a white surface of `finish` and `optics` sends back at
/// normals from straight on to grazing, with the light behind it (for a
/// transmissive one) the furnace's 1.
fn furnace_light(env: &Prepared, finish: Finish, optics: &Optics) -> Vec<f64> {
    let mesh = Mesh::default();
    let light = FrameLight::new(&mesh, env, &plain_look());
    let v = Vec3::Z;
    let mut out = Vec::new();
    for i in 0..20 {
        let a = f64::from(i) / 20.0 * 1.5;
        let n = Vec3::new(a.sin(), 0.0, a.cos());
        let s = light.shade_layered(
            [255; 3],
            finish,
            Some(optics),
            Vec3::new(0.3, 0.2, 0.0),
            n,
            v,
            1.0,
            1.0,
        );
        for k in 0..3 {
            out.push(s.light[k] + s.transmit[k]);
        }
    }
    out
}

#[test]
fn every_layer_keeps_the_furnaces_energy() {
    let env = furnace();
    let mut cases: Vec<(String, Finish, Optics)> = Vec::new();
    for metallic in [0_u8, 255] {
        for roughness in [0_u8, 128, 255] {
            let f = finish(metallic, roughness);
            for a in [-0.8_f32, 0.5, 0.95] {
                cases.push((
                    format!("anisotropy {a} m {metallic} r {roughness}"),
                    f,
                    Optics {
                        anisotropy: a,
                        tangent: TangentField::Axis([0.0, 1.0, 0.0]),
                        ..Optics::default()
                    },
                ));
            }
            for (c, rc) in [(0.5_f32, 0.0_f32), (1.0, 0.3), (1.0, 1.0)] {
                cases.push((
                    format!("clearcoat {c}/{rc} m {metallic} r {roughness}"),
                    f,
                    Optics {
                        clearcoat: c,
                        clearcoat_roughness: rc,
                        ..Optics::default()
                    },
                ));
            }
            for w in [0.3_f32, 1.0] {
                cases.push((
                    format!("subsurface {w} m {metallic} r {roughness}"),
                    f,
                    Optics {
                        subsurface: w,
                        subsurface_color: Some([255; 3]),
                        ..Optics::default()
                    },
                ));
            }
            for t in [0.5_f32, 1.0] {
                cases.push((
                    format!("transmission {t} m {metallic} r {roughness}"),
                    f,
                    Optics {
                        transmission: t,
                        ..Optics::default()
                    },
                ));
            }
        }
    }
    let mut worst: f64 = 0.0;
    for (name, f, optics) in &cases {
        for c in furnace_light(&env, *f, optics) {
            worst = worst.max((c - 1.0).abs());
            assert!((0.98..=1.02).contains(&c), "{name}: {c}");
        }
    }
    eprintln!(
        "layers' furnace: {} cases, worst departure {worst:.4}",
        cases.len()
    );
}

#[test]
fn the_clear_coat_weakens_the_base_by_its_albedo() {
    // Under a sky that is not even, the coat's light is its albedo times
    // the environment along the mirror direction at its roughness, and
    // the base's light is weakened by exactly one minus that albedo.
    let env = Prepared::new(&environment(128, |d| [0.4 + 0.6 * d.z.max(0.0), 0.5, 0.3])).unwrap();
    let mesh = Mesh::default();
    let light = FrameLight::new(&mesh, &env, &plain_look());
    let v = Vec3::Z;
    for (c, rc) in [(1.0_f32, 0.1_f32), (0.4, 0.6)] {
        for i in 0..8 {
            let a = f64::from(i) / 8.0 * 1.4;
            let n = Vec3::new(a.sin(), 0.0, a.cos());
            let base = [200, 40, 30];
            let f = finish(0, 150);
            let bare = light.shade_layered(base, f, None, Vec3::ZERO, n, v, 1.0, 1.0);
            let coated = light.shade_layered(
                base,
                f,
                Some(&Optics {
                    clearcoat: c,
                    clearcoat_roughness: rc,
                    ..Optics::default()
                }),
                Vec3::ZERO,
                n,
                v,
                1.0,
                1.0,
            );
            let nv = n.dot(v).clamp(1e-4, 1.0);
            let (sa, sb) = env.split_sum(nv, f64::from(rc));
            let fc = f64::from(c) * (0.04 * sa + sb);
            let mirror = n * (2.0 * n.dot(v)) - v;
            let top = (env.levels.len() - 1) as f64;
            let coat = light.radiance(mirror, f64::from(rc) * top);
            for k in 0..3 {
                let want = bare.light[k] * (1.0 - fc) + coat[k] * fc;
                assert!(
                    (coated.light[k] - want).abs() < 1e-9,
                    "c {c} rc {rc} at {a}: {} against {want}",
                    coated.light[k]
                );
            }
            assert!(fc > 0.0 && fc < 1.0);
        }
    }
}

/// The half-maximum width, in slope, of the GGX `D` of roughness `alpha`
/// along one axis, with the half vector fixed and the normal tilted:
/// `alpha · √((√2 − 1) / (1 − √2 α²))`.
fn half_width(alpha: f64) -> f64 {
    let r2 = std::f64::consts::SQRT_2;
    alpha * ((r2 - 1.0) / (1.0 - r2 * alpha * alpha)).sqrt()
}

#[test]
fn an_anisotropic_highlight_stretches_along_its_tangent_by_the_closed_form() {
    // A point key straight behind the viewer: the half vector is the view
    // direction, and tilting the normal sweeps the lobe. Its half-maximum
    // contour in slope space has the semi-axes of the closed form for αt
    // along the tangent and αb across it.
    let env = Prepared::new(&environment(64, |d| {
        [if d.z > 0.99 { 50.0 } else { 0.0 }; 3]
    }))
    .unwrap();
    let mesh = Mesh::default();
    let mut light = FrameLight::new(&mesh, &env, &plain_look());
    let l = Vec3::Z;
    light.key = Some((l, [1.0; 3], 0.0));
    let v = l;
    let r = 0.3_f64;
    for (a, angle) in [(0.6_f32, 0.0_f64), (0.6, 0.5), (-0.4, 1.1), (0.85, 2.0)] {
        let axis = [angle.cos() as f32, angle.sin() as f32, 0.0];
        let optics = Optics {
            anisotropy: a,
            tangent: TangentField::Axis(axis),
            ..Optics::default()
        };
        let f = finish(255, (r * 255.0).round() as u8);
        let r = f64::from(f.roughness.unwrap()) / 255.0;
        let t = Vec3::new(f64::from(axis[0]), f64::from(axis[1]), 0.0)
            .normalized()
            .unwrap();
        let b = Vec3::Z.cross(t);
        let shade = |sx: f64, sy: f64| {
            let n = (Vec3::Z + t * sx + b * sy).normalized().unwrap();
            let s = light.shade_layered([255; 3], f, Some(&optics), Vec3::ZERO, n, v, 1.0, 1.0);
            s.light[1]
        };
        // The half-maximum along a direction ψ of the slope plane, by
        // bisection (the lobe falls monotonically from its peak).
        let peak = shade(0.0, 0.0);
        let width = |psi: f64| {
            let (c, s) = (psi.cos(), psi.sin());
            let (mut lo, mut hi) = (0.0, 2.0);
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if shade(c * mid, s * mid) > 0.5 * peak {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let alpha = r * r;
        let (at, ab) = layers::aniso_alphas(alpha, f64::from(a));
        let (wt, wb) = (width(0.0), width(std::f64::consts::FRAC_PI_2));
        let want = half_width(at) / half_width(ab);
        let ratio = wt / wb;
        assert!(
            (ratio / want - 1.0).abs() < 0.03,
            "a {a}: widths {wt:.4} along, {wb:.4} across, ratio {ratio:.4} against {want:.4}"
        );
        // The direction: of 72 directions the widest is the tangent's
        // (for a > 0) or the one across it (a < 0).
        let widest = (0..72)
            .map(|i| f64::from(i) * std::f64::consts::PI / 72.0)
            .max_by(|x, y| width(*x).total_cmp(&width(*y)))
            .unwrap();
        let expected = if a > 0.0 {
            0.0
        } else {
            std::f64::consts::FRAC_PI_2
        };
        let off = (widest - expected).rem_euclid(std::f64::consts::PI);
        let off = off.min(std::f64::consts::PI - off);
        assert!(
            off <= std::f64::consts::PI / 72.0 + 1e-9,
            "a {a} at {angle}: the widest direction is {widest:.3} rad from the tangent's"
        );
    }
}

/// A box with flat faces and outward corners in order, one face id per
/// side starting at `first`, its centre at `c`, its half sizes along the
/// unit axes `u`, `v`, `w` (right-handed).
fn slab(
    c: Vec3,
    (u, hu): (Vec3, f64),
    (v, hv): (Vec3, f64),
    (w, hw): (Vec3, f64),
    first: u32,
) -> Mesh {
    let mut mesh = Mesh::default();
    let sides = [
        (u, hu, v, hv, w, hw),
        (-u, hu, w, hw, v, hv),
        (v, hv, w, hw, u, hu),
        (-v, hv, u, hu, w, hw),
        (w, hw, u, hu, v, hv),
        (-w, hw, v, hv, u, hu),
    ];
    for (k, (n, hn, a, ha, b, hb)) in sides.into_iter().enumerate() {
        let base = mesh.positions.len() as u32;
        let centre = c + n * hn;
        for (sa, sb) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            mesh.positions.push(centre + a * (sa * ha) + b * (sb * hb));
            mesh.normals.push(n);
        }
        // a × b = n for every side, so (0, 1, 2) runs counter-clockwise
        // seen from outside.
        assert!((a.cross(b) - n).length() < 1e-12);
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.push(first + k as u32);
        mesh.face_ids.push(first + k as u32);
    }
    mesh
}

fn join(a: &Mesh, b: &Mesh) -> Mesh {
    let mut out = a.clone();
    let offset = out.positions.len() as u32;
    out.positions.extend(&b.positions);
    out.normals.extend(&b.normals);
    out.triangles
        .extend(b.triangles.iter().map(|t| t.map(|i| i + offset)));
    out.face_ids.extend(&b.face_ids);
    out
}

/// The column where a row turns from the left plate's red to the right
/// plate's blue, as a fraction (the first column whose blue passes its
/// red).
fn edge_column(frame: &craft3d_render::Frame, row: usize) -> usize {
    let w = frame.width as usize;
    (0..w)
        .find(|&x| {
            let i = 3 * (row * w + x);
            frame.rgb[i + 2] > frame.rgb[i]
        })
        .unwrap()
}

#[test]
fn a_glass_slab_shifts_what_is_behind_it_by_the_closed_form() {
    // Seen straight along +y in an orthographic picture: a plate at y = 4
    // whose left half (x < 0) is red and right half blue, and in front of
    // it a glass slab of thickness 0.6 turned by θ about z. Through the
    // slab the plate's edge moves by the slab's lateral offset
    // `t sin θ (1 − cos θ / √(n² − sin² θ))`; above it, it stays at x = 0.
    let env = Prepared::new(&environment(64, |_| [1.0; 3])).unwrap();
    let left = slab(
        Vec3::new(-1.5, 4.0, 0.0),
        (Vec3::X, 1.5),
        (Vec3::Y, 0.1),
        (Vec3::Z, 1.5),
        0,
    );
    let right = slab(
        Vec3::new(1.5, 4.0, 0.0),
        (Vec3::X, 1.5),
        (Vec3::Y, 0.1),
        (Vec3::Z, 1.5),
        6,
    );
    let plate = join(&left, &right);
    let camera = Camera {
        eye: Vec3::new(0.0, -10.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 2.0 },
    };
    let (width, height) = (400_u32, 400_u32);
    let pixel = 4.0 / f64::from(height);
    let params = RenderParams {
        width,
        height,
        ssaa: 1,
        ..RenderParams::default()
    };
    for (theta, ior, thickness) in [
        (0.5_f64, 1.5_f64, 0.6_f64),
        (0.9, 1.5, 0.6),
        (0.7, 2.0, 0.4),
    ] {
        let u = Vec3::new(theta.cos(), theta.sin(), 0.0);
        let normal = Vec3::new(-theta.sin(), theta.cos(), 0.0);
        let glass = slab(
            Vec3::new(0.0, 0.0, 0.0),
            (u, 1.6),
            (normal, 0.5 * thickness),
            (Vec3::Z, 0.8),
            12,
        );
        let mesh = join(&plate, &glass);
        for thin in [false, true] {
            let mut colors = FaceColors::new();
            for face in 0..6 {
                colors.set_look(face, Some(([255, 0, 0], finish(0, 255))));
                colors.set_look(face + 6, Some(([0, 0, 255], finish(0, 255))));
            }
            let optics = Optics {
                transmission: 1.0,
                ior: ior as f32,
                thin_walled: thin,
                ..Optics::default()
            };
            for face in 12..18 {
                colors.set_look(face, Some(([255; 3], finish(0, 0))));
                assert!(colors.set_optics(face, Some(optics)));
            }
            let frame =
                render_studio(&mesh, &colors, &camera, &params, &plain_look(), &env).unwrap();
            if !thin {
                evidence(&format!("slab-{theta}-{ior}.png"), &frame);
            }
            // Above the slab (z = 1.2) and through it (z = 0).
            let above = edge_column(&frame, (0.5 * f64::from(height) - 1.2 / pixel) as usize);
            let through = edge_column(&frame, (height / 2) as usize);
            let shift = (through as f64 - above as f64) * pixel;
            // The exit point, worked out apart from the renderer: the view
            // ray +y enters the front face (normal −normal) at θ, bends
            // by Snell, and leaves through the back face.
            let sin_t = theta.sin() / ior;
            let theta_t = sin_t.asin();
            let offset = layers::slab_offset(thickness, ior, theta);
            let path = thickness / theta_t.cos();
            let lateral = path * (theta - theta_t).sin();
            assert!(
                (lateral - offset).abs() < 1e-12,
                "{lateral} against {offset}"
            );
            // Inside, the ray turns from +y towards the slab's normal
            // (−sin θ, cos θ) by θ − θt, so it leaves at x − offset: the
            // plate's edge (x = 0) is seen where x = +offset.
            let want = if thin { 0.0 } else { offset };
            assert!(
                (shift - want).abs() <= 1.5 * pixel,
                "θ {theta} n {ior} t {thickness} thin {thin}: shift {shift:.4} against {want:.4} (pixel {pixel})"
            );
        }
    }
}

#[test]
fn layered_pictures_are_deterministic_and_their_doubt_is_small() {
    let env = Prepared::new(&environment(64, |d| [0.5 + d.z.max(0.0) * 3.0; 3])).unwrap();
    let plate = slab(
        Vec3::new(0.0, 3.0, 0.0),
        (Vec3::X, 2.0),
        (Vec3::Y, 0.1),
        (Vec3::Z, 2.0),
        0,
    );
    let glass = slab(
        Vec3::ZERO,
        (Vec3::new(0.8, 0.6, 0.0), 1.0),
        (Vec3::new(-0.6, 0.8, 0.0), 0.3),
        (Vec3::Z, 0.7),
        6,
    );
    let mesh = join(&plate, &glass);
    let mut colors = FaceColors::new();
    for face in 0..6 {
        colors.set_look(face, Some(([180, 120, 60], finish(255, 90))));
        colors.set_optics(
            face,
            Some(Optics {
                anisotropy: 0.7,
                tangent: TangentField::Around {
                    origin: [0.0, 3.0, 0.0],
                    axis: [0.0, 1.0, 0.0],
                },
                clearcoat: 1.0,
                clearcoat_roughness: 0.1,
                ..Optics::default()
            }),
        );
    }
    for face in 6..12 {
        colors.set_look(face, Some(([230, 240, 255], finish(0, 20))));
        colors.set_optics(
            face,
            Some(Optics {
                transmission: 1.0,
                ior: 1.5,
                absorption: layers::absorption_coefficients([120, 200, 160], 1.0),
                ..Optics::default()
            }),
        );
    }
    let camera = Camera {
        eye: Vec3::new(3.0, -9.0, 4.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let params = RenderParams {
        width: 160,
        height: 120,
        ssaa: 2,
        ..RenderParams::default()
    };
    let look = Look {
        ground: true,
        shadows: true,
        ao: true,
        ..Look::polished()
    };
    let a = render_studio(&mesh, &colors, &camera, &params, &look, &env).unwrap();
    let b = render_studio(&mesh, &colors, &camera, &params, &look, &env).unwrap();
    assert_eq!(a.rgb, b.rgb);
    let (c, doubt) =
        render_studio_doubt(&mesh, &colors, &camera, &params, &look, &env, 128.0).unwrap();
    assert_eq!(a.rgb, c.rgb, "marking doubt does not change the picture");
    let unsure = doubt.iter().filter(|&&d| d > 1.0 / 64.0).count();
    assert!(
        unsure * 32 <= doubt.len(),
        "{unsure} of {} in doubt",
        doubt.len()
    );
    evidence("layers-scene.png", &a);
}
