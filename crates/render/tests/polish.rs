//! Layer 2 of the studio look against geometry and closed forms: a mirror
//! plane reflects a box where the mirrored rays meet it, contact shadows
//! fall only beside an occluder, the specular shadow only in the key's
//! shadow, a thin lens's blur has the textbook radius and keeps the plane
//! of focus sharp, bloom keeps the light's energy, accumulation converges
//! and is deterministic (docs/design/rendering.md, *Layer 2*).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use craft3d_math::Vec3;
use craft3d_render::studio::polish::{
    self, Dof, Focus, Lens, bloom, coc_radius, dof_gather, dof_taps,
};
use craft3d_render::studio::{
    HdrImage, Look, Prepared, Tonemap, env::texel_direction, render_studio, render_studio_frame,
};
use craft3d_render::{Camera, FaceColors, Finish, Frame, Projection, RenderParams};
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

/// An environment of one colour, with no key light.
fn even(c: [f64; 3]) -> Prepared {
    Prepared::new(&environment(64, |_| c)).unwrap()
}

/// An even sky with a bright small sun straight above (a key light).
fn sunny() -> Prepared {
    Prepared::new(&environment(256, |d| {
        if d.z > 0.995 { [400.0; 3] } else { [1.0; 3] }
    }))
    .unwrap()
}

/// Adds an axis-aligned box with face ids `face` (all six faces alike).
fn add_box(mesh: &mut Mesh, lo: Vec3, hi: Vec3, face: u32) {
    let quads: [([f64; 3], [usize; 4]); 6] = [
        ([-1.0, 0.0, 0.0], [0, 4, 6, 2]),
        ([1.0, 0.0, 0.0], [1, 3, 7, 5]),
        ([0.0, -1.0, 0.0], [0, 1, 5, 4]),
        ([0.0, 1.0, 0.0], [2, 6, 7, 3]),
        ([0.0, 0.0, -1.0], [0, 2, 3, 1]),
        ([0.0, 0.0, 1.0], [4, 5, 7, 6]),
    ];
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    for (n, idx) in quads {
        let base = mesh.positions.len() as u32;
        for &i in &idx {
            mesh.positions.push(corner(i));
            mesh.normals.push(Vec3::new(n[0], n[1], n[2]));
        }
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.push(face);
        mesh.face_ids.push(face);
    }
}

/// Adds the square floor `|x|, |y| <= half` at height `z`, facing up,
/// cut into `n` × `n` quads.
fn add_floor(mesh: &mut Mesh, half: f64, z: f64, n: u32, face: u32) {
    let base = mesh.positions.len() as u32;
    for i in 0..=n {
        for j in 0..=n {
            let x = -half + 2.0 * half * f64::from(j) / f64::from(n);
            let y = -half + 2.0 * half * f64::from(i) / f64::from(n);
            mesh.positions.push(Vec3::new(x, y, z));
            mesh.normals.push(Vec3::Z);
        }
    }
    for i in 0..n {
        for j in 0..n {
            let a = base + i * (n + 1) + j;
            let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
            mesh.triangles.push([a, b, d]);
            mesh.triangles.push([a, d, c]);
            mesh.face_ids.push(face);
            mesh.face_ids.push(face);
        }
    }
}

fn params(width: u32, height: u32) -> RenderParams {
    RenderParams {
        width,
        height,
        ssaa: 1,
        ..RenderParams::default()
    }
}

fn bare() -> Look {
    Look {
        tonemap: Tonemap::Linear,
        ao: false,
        shadows: false,
        ground: false,
        ..Look::default()
    }
}

fn evidence(name: &str, frame: &Frame) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/render-rl2");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(name), frame.to_png().unwrap());
    }
    craft3d_testkit::evidence::print(name, &frame.to_png().unwrap());
}

fn px(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * frame.width as usize + x) * 3;
    [frame.rgb[i], frame.rgb[i + 1], frame.rgb[i + 2]]
}

/// The camera's ray through the centre of pixel `(x, y)` of a `w` × `h`
/// picture, worked out apart from the renderer.
fn camera_ray(camera: &Camera, w: usize, h: usize, x: usize, y: usize) -> Vec3 {
    let f = (camera.target - camera.eye).normalized().unwrap();
    let r = f.cross(camera.up).normalized().unwrap();
    let u = r.cross(f);
    let Projection::Perspective { fov_y_deg } = camera.projection else {
        panic!("perspective only");
    };
    let t = (fov_y_deg.to_radians() * 0.5).tan();
    let aspect = w as f64 / h as f64;
    let cx = ((x as f64 + 0.5) / w as f64 * 2.0 - 1.0) * aspect * t;
    let cy = (1.0 - (y as f64 + 0.5) / h as f64 * 2.0) * t;
    (f + r * cx + u * cy).normalized().unwrap()
}

/// Where the ray `o + t d` enters the box, if it does.
fn enters(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> Option<(f64, usize)> {
    let (mut t0, mut t1, mut axis) = (0.0_f64, f64::INFINITY, 9);
    for k in 0..3 {
        let (oo, dd, l, h) = match k {
            0 => (o.x, d.x, lo.x, hi.x),
            1 => (o.y, d.y, lo.y, hi.y),
            _ => (o.z, d.z, lo.z, hi.z),
        };
        if dd.abs() < 1e-12 {
            if oo < l || oo > h {
                return None;
            }
            continue;
        }
        let (a, b) = ((l - oo) / dd, (h - oo) / dd);
        let (a, b) = (a.min(b), a.max(b));
        if a > t0 {
            t0 = a;
            axis = k;
        }
        t1 = t1.min(b);
    }
    (t0 <= t1 && axis < 3).then_some((t0, axis))
}

#[test]
fn a_mirror_plane_reflects_a_box_where_the_mirrored_rays_meet_it() {
    let (lo, hi) = (Vec3::new(-0.5, -0.5, 0.2), Vec3::new(0.5, 0.5, 1.2));
    let mut mesh = Mesh::default();
    add_floor(&mut mesh, 4.0, 0.0, 4, 0);
    add_box(&mut mesh, lo, hi, 1);
    let mut colors = FaceColors::default();
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
    colors.set(1, Some([230, 20, 20]));
    let camera = Camera {
        eye: Vec3::new(0.0, -6.0, 2.5),
        target: Vec3::new(0.0, 0.0, 0.4),
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 40.0 },
    };
    let env = even([0.2, 0.4, 1.0]);
    let (w, h) = (200, 150);
    let look = Look {
        ssr: true,
        ..bare()
    };
    let with = render_studio(&mesh, &colors, &camera, &params(w, h), &look, &env).unwrap();
    let without = render_studio(&mesh, &colors, &camera, &params(w, h), &bare(), &env).unwrap();
    evidence("ssr-mirror.png", &with);
    evidence("ssr-mirror-off.png", &without);
    let grow = Vec3::new(0.06, 0.06, 0.06);
    let (mut expected_red, mut found_red, mut false_red, mut blue) = (0, 0, 0, 0);
    for y in 0..h as usize {
        for x in 0..w as usize {
            if with.ids[y * w as usize + x] != 0 {
                continue;
            }
            let d = camera_ray(&camera, w as usize, h as usize, x, y);
            let t = -camera.eye.z / d.z;
            let g = camera.eye + d * t;
            let m = Vec3::new(d.x, d.y, -d.z);
            let red = |c: [u8; 3]| c[0] > c[2];
            let off = px(&without, x, y);
            assert!(!red(off), "no reflection without SSR at ({x}, {y})");
            let shrunk = enters(g, m, lo + grow, hi - grow);
            let grown = enters(g, m, lo - grow, hi + grow);
            let c = px(&with, x, y);
            // Entering through the front face (y = -0.5), which the camera
            // sees: the reflection must be the box's.
            if let Some((_, 1)) = shrunk {
                expected_red += 1;
                if red(c) {
                    found_red += 1;
                }
            } else if grown.is_none() {
                blue += 1;
                if red(c) {
                    false_red += 1;
                }
            }
        }
    }
    eprintln!("mirror: {found_red}/{expected_red} reflect the box; {false_red}/{blue} wrong");
    assert!(expected_red > 300, "the scene shows the box's reflection");
    assert!(
        found_red * 100 >= expected_red * 90,
        "{found_red}/{expected_red}"
    );
    // Off the box's reflection the mirror shows the environment, but for
    // rays passing just behind the box on the screen (the method's
    // assumed thickness).
    assert!(false_red * 100 <= blue * 2, "{false_red}/{blue}");
}

#[test]
fn contact_shadows_fall_beside_an_occluder_and_not_on_an_open_floor() {
    let env = sunny();
    let camera = Camera {
        eye: Vec3::new(3.0, -5.0, 4.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    // From behind the box, so its shadow falls towards the camera.
    // Darker, so the lit floor is not clipped by the linear curve.
    let lit = Look {
        light: Some([135.0, 35.0]),
        exposure: -2.0,
        ..bare()
    };
    let contact = Look {
        contact_shadows: true,
        ..lit
    };
    // An open floor alone: no sample shadows itself.
    let mut floor = Mesh::default();
    add_floor(&mut floor, 2.0, 0.0, 8, 0);
    let colors = FaceColors::default();
    let a = render_studio(&floor, &colors, &camera, &params(160, 120), &lit, &env).unwrap();
    let b = render_studio(&floor, &colors, &camera, &params(160, 120), &contact, &env).unwrap();
    let open = (0..a.ids.len()).filter(|&i| a.ids[i] == 0).count();
    let changed = (0..a.ids.len())
        .filter(|&i| a.ids[i] == 0 && a.rgb[i * 3..i * 3 + 3] != b.rgb[i * 3..i * 3 + 3])
        .count();
    assert!(open > 5000);
    assert_eq!(changed, 0, "an open floor has no contact shadow");

    // A box standing on it: the floor right behind it, away from the
    // light, darkens; the floor far from it does not change.
    let mut mesh = Mesh::default();
    add_floor(&mut mesh, 2.0, 0.0, 8, 0);
    add_box(
        &mut mesh,
        Vec3::new(-0.3, -0.3, 0.0),
        Vec3::new(0.3, 0.3, 0.6),
        1,
    );
    let a = render_studio(&mesh, &colors, &camera, &params(160, 120), &lit, &env).unwrap();
    let b = render_studio(&mesh, &colors, &camera, &params(160, 120), &contact, &env).unwrap();
    evidence("contact-off.png", &a);
    evidence("contact-on.png", &b);
    let (mut darker, mut far_changed) = (0, 0);
    for y in 0..120 {
        for x in 0..160 {
            let i = y * 160 + x;
            if a.ids[i] != 0 {
                continue;
            }
            let (ca, cb) = (px(&a, x, y), px(&b, x, y));
            // The floor point under the pixel.
            let d = camera_ray(&camera, 160, 120, x, y);
            let g = camera.eye + d * (-camera.eye.z / d.z);
            let near_box = g.x.abs().max(g.y.abs()) < 0.3 + 0.35;
            if !near_box && ca != cb {
                far_changed += 1;
            }
            if u32::from(cb[1]) * 10 < u32::from(ca[1]) * 9 {
                darker += 1;
            }
        }
    }
    eprintln!("contact: {darker} floor pixels darker, {far_changed} far ones changed");
    assert!(darker > 10);
    assert_eq!(far_changed, 0);
}

#[test]
fn the_specular_shadow_darkens_only_what_the_key_does_not_see() {
    let env = sunny();
    let camera = Camera {
        eye: Vec3::new(0.0, -5.0, 3.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 40.0 },
    };
    let mut mesh = Mesh::default();
    add_floor(&mut mesh, 2.5, 0.0, 6, 0);
    add_box(
        &mut mesh,
        Vec3::new(-0.4, -0.4, 0.6),
        Vec3::new(0.4, 0.4, 1.0),
        1,
    );
    let mut colors = FaceColors::default();
    colors.set_look(
        0,
        Some((
            [180; 3],
            Finish {
                metallic: 0,
                roughness: Some(50),
            },
        )),
    );
    let lit = Look {
        shadows: true,
        light: Some([0.0, 70.0]),
        ..bare()
    };
    let spec = Look {
        specular_shadow: true,
        ..lit
    };
    let a = render_studio(&mesh, &colors, &camera, &params(160, 120), &lit, &env).unwrap();
    let b = render_studio(&mesh, &colors, &camera, &params(160, 120), &spec, &env).unwrap();
    evidence("specular-shadow.png", &b);
    let (mut darker, mut brighter) = (0, 0);
    for i in 0..a.ids.len() {
        for k in 0..3 {
            let (x, y) = (a.rgb[i * 3 + k], b.rgb[i * 3 + k]);
            if y > x {
                brighter += 1;
            }
            if y < x {
                darker += 1;
            }
        }
    }
    assert_eq!(brighter, 0, "the specular shadow only takes light away");
    assert!(darker > 0);
}

#[test]
fn the_lens_blur_has_the_textbook_radius() {
    // A 50 mm lens at f/2 on a full-frame camera, focused at 1 m, a point
    // at 2 m: the circle of confusion on the sensor is
    // f² |z − zf| / (N z (zf − f)), against a sensor 24 mm high.
    let (f, n, zf, z) = (50.0, 2.0, 1000.0, 2000.0);
    let c_sensor = f * f * (z - zf) / (n * z * (zf - f));
    let h = 1000.0;
    let expected_radius = 0.5 * c_sensor / 24.0 * h;
    // The renderer's camera of that field: the image distance v.
    let v = f * zf / (zf - f);
    let half = 12.0 / v;
    let dof = Dof {
        fstop: n,
        focus: Focus::Distance(zf),
        mm_per_unit: 1.0,
    };
    let a = dof.aperture(half, zf).unwrap();
    assert!((a - f / n).abs() < 1e-9, "aperture {a}");
    let c = coc_radius(a, zf, half, h, z);
    assert!(
        (c - expected_radius).abs() < 1e-9 * expected_radius,
        "{c} against {expected_radius}"
    );
    // In metres the same lens blurs the same.
    let metres = Dof {
        mm_per_unit: 1000.0,
        ..dof
    };
    let am = metres.aperture(half, zf / 1000.0).unwrap();
    let cm = coc_radius(am, zf / 1000.0, half, h, z / 1000.0);
    assert!((cm - c).abs() < 1e-9 * c);

    // A single bright sample at that depth spreads over the disc of that
    // radius: over frames the farthest sample it reaches is at the radius.
    let (w, hh) = (41_usize, 41_usize);
    let r = 9.0;
    let mut light = vec![[0.0_f32; 3]; w * hh];
    light[20 * w + 20] = [1000.0; 3];
    let depth = vec![1.0; w * hh];
    let mut farthest = 0.0_f64;
    let mut sum = 0.0;
    for k in 0..64 {
        let taps = dof_taps(r, k);
        let out = dof_gather(&light, &depth, (w, hh), |_| r, &taps);
        for y in 0..hh {
            for x in 0..w {
                if out[y * w + x][0] > 0.0 {
                    let d = ((x as f64 - 20.0).powi(2) + (y as f64 - 20.0).powi(2)).sqrt();
                    farthest = farthest.max(d);
                }
                sum += f64::from(out[y * w + x][0]);
            }
        }
    }
    eprintln!(
        "lens: farthest {farthest} for radius {r}, mean energy {}",
        sum / 64.0
    );
    assert!((farthest - r).abs() <= 1.5, "farthest {farthest}");
}

#[test]
fn the_plane_of_focus_stays_sharp() {
    // A box's front face square to the view, at the focus distance, in
    // front of a varied environment: the face keeps its picture exactly;
    // the background blurs.
    let env = Prepared::new(&environment(128, |d| {
        let s = (d.x * 9.0).sin() * (d.z * 7.0).cos();
        [1.0 + s, 1.0 - 0.5 * s, 1.0]
    }))
    .unwrap();
    let mut mesh = Mesh::default();
    add_box(
        &mut mesh,
        Vec3::new(-1.0, -0.5, -1.0),
        Vec3::new(1.0, 0.5, 1.0),
        1,
    );
    let camera = Camera {
        eye: Vec3::new(0.0, -5.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 40.0 },
    };
    let sharp = bare();
    let lens = Look {
        dof: Some(Dof {
            fstop: 1.4,
            focus: Focus::Point([0.0, -0.5, 0.3]),
            mm_per_unit: 10.0,
        }),
        ..bare()
    };
    let mut colors = FaceColors::default();
    colors.set(2, Some([20, 160, 40]));
    let a = render_studio(&mesh, &colors, &camera, &params(160, 120), &sharp, &env).unwrap();
    let b = render_studio(&mesh, &colors, &camera, &params(160, 120), &lens, &env).unwrap();
    evidence("dof-focus.png", &b);
    let (mut face, mut background_changed) = (0, 0);
    for y in 1..119 {
        for x in 1..159 {
            let i = y * 160 + x;
            let all_face = [i - 1, i + 1, i - 160, i + 160, i]
                .iter()
                .all(|&j| a.ids[j] == 1);
            if all_face {
                assert_eq!(px(&a, x, y), px(&b, x, y), "in focus at ({x}, {y})");
                face += 1;
            } else if a.ids[i] != 1 && px(&a, x, y) != px(&b, x, y) {
                background_changed += 1;
            }
        }
    }
    assert!(face > 2000);
    eprintln!("focus: {face} sharp face pixels, {background_changed} others blurred");
    assert!(background_changed > 50, "{background_changed}");
    // The lens of the frame reaches the background's blur.
    let l = Lens::new(
        &lens.dof.unwrap(),
        &camera,
        &craft3d_math::Bbox3::from_points(mesh.positions.iter().copied()),
        120,
    )
    .unwrap();
    assert!(l.reach >= l.coc(f64::INFINITY) && l.reach <= l.cap);
    // An orthographic camera has no lens.
    let ortho = Camera {
        projection: Projection::Orthographic { half_height: 2.0 },
        ..camera
    };
    assert!(
        Lens::new(
            &lens.dof.unwrap(),
            &ortho,
            &craft3d_math::Bbox3::from_points(mesh.positions.iter().copied()),
            120
        )
        .is_none()
    );
}

#[test]
fn bloom_keeps_the_light_and_an_even_picture() {
    let (w, h) = (96, 80);
    let even: Vec<[f32; 3]> = vec![[0.7, 0.2, 3.0]; w * h];
    for c in bloom(&even, w, h, 1.0) {
        for (a, b) in c.iter().zip([0.7, 0.2, 3.0]) {
            assert!((f64::from(*a) - b).abs() < 1e-5);
        }
    }
    let mut spots = vec![[0.05_f32; 3]; w * h];
    for (x, y) in [(30, 30), (60, 45), (48, 40)] {
        spots[y * w + x] = [500.0, 100.0, 20.0];
    }
    let sum = |v: &[[f32; 3]]| v.iter().map(|c| f64::from(c[0])).sum::<f64>();
    // The 13-tap filter weighs a texel by its place among the pairs it
    // halves (the four phases do not get exactly a quarter each), so a
    // lone bright texel keeps its energy within a band, not exactly: the
    // full blur (s = 1) within 10 %, the mix within s times that.
    for s in [0.04, 0.3, 1.0] {
        let out = bloom(&spots, w, h, s);
        let ratio = sum(&out) / sum(&spots);
        eprintln!("bloom {s}: energy ratio {ratio}");
        assert!(
            (ratio - 1.0).abs() <= 0.1 * s + 1e-6,
            "ratio {ratio} at {s}"
        );
        // The spot spread: its own sample lost light to its neighbours.
        assert!(out[30 * w + 30][0] < 500.0 && out[30 * w + 31][0] > 0.05);
    }
}

#[test]
fn accumulation_converges_and_is_deterministic() {
    let env = sunny();
    let mut mesh = Mesh::default();
    add_floor(&mut mesh, 2.0, 0.0, 4, 0);
    add_box(
        &mut mesh,
        Vec3::new(-0.5, -0.5, 0.0),
        Vec3::new(0.5, 0.5, 0.8),
        1,
    );
    let camera = Camera {
        eye: Vec3::new(3.0, -4.0, 3.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    let colors = FaceColors::default();
    let p = params(120, 90);
    let look = |n: u32| Look {
        accumulate: n,
        dof: Some(Dof {
            fstop: 2.0,
            focus: Focus::Point([0.0, -0.5, 0.4]),
            mm_per_unit: 10.0,
        }),
        ..Look::polished()
    };
    // Frame 0 is the picture without accumulation.
    let one = render_studio(&mesh, &colors, &camera, &p, &look(1), &env).unwrap();
    let zero = render_studio_frame(&mesh, &colors, &camera, &p, &look(1), &env, 0).unwrap();
    assert_eq!(one, zero);
    let pictures: Vec<Frame> = [4, 8, 16, 32]
        .iter()
        .map(|&n| render_studio(&mesh, &colors, &camera, &p, &look(n), &env).unwrap())
        .collect();
    let again = render_studio(&mesh, &colors, &camera, &p, &look(8), &env).unwrap();
    assert_eq!(again, pictures[1], "the same frames give the same picture");
    let mean_diff = |a: &Frame, b: &Frame| {
        a.rgb
            .iter()
            .zip(&b.rgb)
            .map(|(&x, &y)| f64::from(x.abs_diff(y)))
            .sum::<f64>()
            / a.rgb.len() as f64
    };
    let steps: Vec<f64> = pictures
        .windows(2)
        .map(|w| mean_diff(&w[0], &w[1]))
        .collect();
    eprintln!(
        "accumulation: mean change 4→8 {}, 8→16 {}, 16→32 {}",
        steps[0], steps[1], steps[2]
    );
    assert!(steps[2] < steps[0], "{steps:?}");
    assert!(steps[2] < 1.0, "{steps:?}");
    evidence("accumulated-32.png", &pictures[3]);
    // Ids and depths are frame 0's.
    assert_eq!(pictures[3].ids, one.ids);
}

#[test]
fn bad_layer_2_numbers_are_ignored_not_crashes() {
    let env = even([1.0; 3]);
    let mut mesh = Mesh::default();
    add_box(
        &mut mesh,
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, 1.0),
        0,
    );
    let camera = Camera {
        eye: Vec3::new(0.0, -6.0, 1.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 40.0 },
    };
    let colors = FaceColors::default();
    let p = params(40, 30);
    let plain = render_studio(&mesh, &colors, &camera, &p, &bare(), &env).unwrap();
    for (fstop, focus, mm) in [
        (0.0, Focus::Distance(5.0), 1.0),
        (f64::NAN, Focus::Distance(5.0), 1.0),
        (2.0, Focus::Distance(-5.0), 1.0),
        (2.0, Focus::Point([0.0, -100.0, 0.0]), 1.0),
        (2.0, Focus::Distance(5.0), f64::INFINITY),
        (2.0, Focus::Point([f64::NAN, 0.0, 0.0]), 1.0),
    ] {
        let look = Look {
            dof: Some(Dof {
                fstop,
                focus,
                mm_per_unit: mm,
            }),
            accumulate: 0,
            ..bare()
        };
        let out = render_studio(&mesh, &colors, &camera, &p, &look, &env).unwrap();
        assert_eq!(out, plain, "{fstop} {focus:?} {mm}");
    }
    let wild = Look {
        bloom: f64::NAN,
        ground_reflection: f64::INFINITY,
        ssr: true,
        contact_shadows: true,
        specular_shadow: true,
        ..bare()
    };
    let out = render_studio(&mesh, &colors, &camera, &p, &wild, &env).unwrap();
    assert_eq!(out.rgb.len(), 40 * 30 * 3);
    assert!(polish::halton(u32::MAX, 0) >= 0.0);
}
