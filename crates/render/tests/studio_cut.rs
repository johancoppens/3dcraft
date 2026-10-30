//! The studio look of see-through and cut-open pictures against what
//! they are made of: the same surfaces as the plain picture, the cap lit
//! as a face of the same plane, the front blended over the surface behind
//! it in light, the key's shadow let through by a see-through body and not
//! cast by what a section cuts away (docs/design/rendering.md,
//! *See-through and cut-open pictures in the studio look*).
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
use craft3d_render::{
    Camera, Cut, FaceColors, NO_FACE, Projection, RenderParams, SECTION_FACE, Section,
    render_colored, section_bounds,
};
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

/// A sky brighter towards the top with a bright spot straight up (a key
/// light from above).
fn sky() -> Prepared {
    Prepared::new(&environment(256, |d| {
        if d.z > 0.99 {
            [400.0; 3]
        } else {
            [0.4 + 0.3 * d.z, 0.5 + 0.2 * d.z, 0.6 + 0.1 * d.x]
        }
    }))
    .unwrap()
}

fn append(mesh: &mut Mesh, more: Mesh) {
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(more.positions);
    mesh.normals.extend(more.normals);
    mesh.triangles
        .extend(more.triangles.iter().map(|t| t.map(|k| k + base)));
    mesh.face_ids.extend(more.face_ids);
}

/// A UV sphere with exact normals, all of face `face`.
fn sphere(center: Vec3, r: f64, rings: u32, segments: u32, face: u32) -> Mesh {
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
            mesh.face_ids.push(face);
            mesh.face_ids.push(face);
        }
    }
    mesh
}

/// A box between two corners, its six faces with flat normals, wound
/// outwards, faces `first` to `first + 5`.
fn cuboid(lo: Vec3, hi: Vec3, first: u32) -> Mesh {
    let mut mesh = Mesh::default();
    let corner = |i: u32| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    // Each face as four corners counter-clockwise seen from outside.
    let faces: [([u32; 4], Vec3); 6] = [
        ([0, 2, 6, 4], Vec3::new(-1.0, 0.0, 0.0)),
        ([1, 5, 7, 3], Vec3::new(1.0, 0.0, 0.0)),
        ([0, 4, 5, 1], Vec3::new(0.0, -1.0, 0.0)),
        ([2, 3, 7, 6], Vec3::new(0.0, 1.0, 0.0)),
        ([0, 1, 3, 2], Vec3::new(0.0, 0.0, -1.0)),
        ([4, 6, 7, 5], Vec3::new(0.0, 0.0, 1.0)),
    ];
    for (k, (quad, n)) in faces.iter().enumerate() {
        let base = mesh.positions.len() as u32;
        for &c in quad {
            mesh.positions.push(corner(c));
            mesh.normals.push(*n);
        }
        mesh.triangles.push([base, base + 1, base + 2]);
        mesh.triangles.push([base, base + 2, base + 3]);
        mesh.face_ids.push(first + k as u32);
        mesh.face_ids.push(first + k as u32);
    }
    mesh
}

/// A square sheet in the plane y = `y`, facing −y, of face `face`.
fn sheet(x: (f64, f64), y: f64, z: (f64, f64), face: u32) -> Mesh {
    let n = Vec3::new(0.0, -1.0, 0.0);
    Mesh {
        positions: vec![
            Vec3::new(x.0, y, z.0),
            Vec3::new(x.1, y, z.0),
            Vec3::new(x.1, y, z.1),
            Vec3::new(x.0, y, z.1),
        ],
        normals: vec![n; 4],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        face_ids: vec![face, face],
    }
}

fn front_camera() -> Camera {
    Camera {
        eye: Vec3::new(0.0, -10.0, 0.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 1.25 },
    }
}

fn lit_look() -> Look {
    Look {
        tonemap: Tonemap::Linear,
        ao: false,
        shadows: false,
        ground: false,
        ..Look::default()
    }
}

fn params(w: u32, h: u32) -> RenderParams {
    RenderParams {
        width: w,
        height: h,
        ssaa: 1,
        ..RenderParams::default()
    }
}

fn evidence(name: &str, frame: &craft3d_render::Frame) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/studio-cut");
    let _ = std::fs::create_dir_all(&dir);
    let png = frame.to_png().unwrap();
    let _ = std::fs::write(dir.join(name), &png);
    craft3d_testkit::evidence::print(name, &png);
}

fn pixel(frame: &craft3d_render::Frame, x: u32, y: u32) -> [u8; 3] {
    let i = (y * frame.width + x) as usize * 3;
    [frame.rgb[i], frame.rgb[i + 1], frame.rgb[i + 2]]
}

/// The studio picture of a cut-open, see-through body has exactly the
/// surfaces of the plain picture: the same ids (the cap's included) and
/// the same view distances, frame for frame.
#[test]
fn the_studio_picture_has_the_plain_pictures_surfaces() {
    let env = sky();
    let mut mesh = sphere(Vec3::ZERO, 1.0, 24, 48, 3);
    append(
        &mut mesh,
        cuboid(Vec3::new(-0.4, -0.4, -1.6), Vec3::new(0.4, 0.4, -1.0), 10),
    );
    let camera = Camera {
        eye: Vec3::new(4.0, -6.0, 3.0),
        target: Vec3::ZERO,
        up: Vec3::Z,
        projection: Projection::Perspective { fov_y_deg: 35.0 },
    };
    for (opacity, section) in [(1.0, true), (0.5, false), (0.5, true)] {
        let p = RenderParams {
            opacity,
            section: section.then_some(Section {
                origin: Vec3::new(0.1, 0.0, 0.0),
                normal: Vec3::new(1.0, -0.3, 0.2),
                cap: true,
                color: [200, 90, 60],
            }),
            ..params(160, 120)
        };
        let plain = render_colored(&mesh, &FaceColors::default(), &camera, &p).unwrap();
        let look = Look {
            ao: true,
            shadows: true,
            ground: true,
            ..lit_look()
        };
        let studio =
            render_studio(&mesh, &FaceColors::default(), &camera, &p, &look, &env).unwrap();
        assert_eq!(
            plain.ids, studio.ids,
            "opacity {opacity}, section {section}"
        );
        assert_eq!(plain.depth, studio.depth);
        if section {
            assert!(studio.ids.contains(&SECTION_FACE), "the cap is drawn");
        }
        evidence(&format!("surfaces-{opacity}-{section}.png"), &studio);
    }
}

/// The cap of a box cut in two is lit as the face of the half box that
/// lies in the same plane, with the cap's colour: the same light, the
/// same occlusion and the same shadows, since the half box is what the
/// section leaves.
#[test]
fn the_cap_is_lit_as_a_face_in_its_plane() {
    let env = sky();
    let color = [180, 120, 70];
    let whole = cuboid(Vec3::new(-1.0, -0.6, -0.5), Vec3::new(1.0, 0.6, 0.5), 1);
    let mut half = cuboid(Vec3::new(-1.0, -0.6, -0.5), Vec3::new(0.0, 0.6, 0.5), 1);
    // A triangle of no area at the whole box's far corner: the same box
    // around the mesh (the light's view, the occlusion's radius, the
    // ground), and nothing drawn.
    let far = half.positions.len() as u32;
    for _ in 0..3 {
        half.positions.push(Vec3::new(1.0, 0.6, 0.5));
        half.normals.push(Vec3::Z);
    }
    half.triangles.push([far, far + 1, far + 2]);
    half.face_ids.push(99);
    // The half box's face in the plane x = 0 has the cap's colour.
    let mut colors = FaceColors::default();
    colors.set(2, Some(color));
    let camera = Camera {
        eye: Vec3::new(6.0, -4.0, 3.0),
        target: Vec3::new(-0.3, 0.0, 0.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 1.3 },
    };
    let look = Look {
        ao: true,
        shadows: true,
        ground: true,
        ..lit_look()
    };
    let cut_open = RenderParams {
        section: Some(Section {
            origin: Vec3::ZERO,
            normal: Vec3::new(1.0, 0.0, 0.0),
            cap: true,
            color,
        }),
        ..params(200, 150)
    };
    let a = render_studio(
        &whole,
        &FaceColors::default(),
        &camera,
        &cut_open,
        &look,
        &env,
    )
    .unwrap();
    let b = render_studio(&half, &colors, &camera, &params(200, 150), &look, &env).unwrap();
    evidence("cap-cut.png", &a);
    evidence("cap-half.png", &b);
    let (w, h) = (a.width as i64, a.height as i64);
    let id = |f: &craft3d_render::Frame, x: i64, y: i64| f.ids[(y * w + x) as usize];
    let mut compared = 0;
    let mut worst = 0;
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            // Inside the cap, two pixels from its outline.
            let inside = (-2..=2).all(|dy| {
                (-2..=2)
                    .all(|dx| id(&a, x + dx, y + dy) == SECTION_FACE && id(&b, x + dx, y + dy) == 2)
            });
            if !inside {
                continue;
            }
            compared += 1;
            let (pa, pb) = (pixel(&a, x as u32, y as u32), pixel(&b, x as u32, y as u32));
            for k in 0..3 {
                worst = worst.max((i32::from(pa[k]) - i32::from(pb[k])).abs());
            }
        }
    }
    assert!(compared > 2000, "the cap is in sight: {compared}");
    assert!(
        worst <= 1,
        "the cap against the half box's face: {worst} levels"
    );
}

/// See-through sheets: where the front sheet is over the one behind it,
/// the picture is `opacity · front + (1 − opacity) · behind` in light,
/// each sheet's light read from its picture alone (no shadows, no
/// occlusion, the linear curve), within the rounding of the bytes; over
/// the background, the front over the background.
#[test]
fn a_see_through_front_is_blended_over_what_is_behind_in_light() {
    let env = sky();
    let mut both = sheet((-1.0, 0.5), 0.0, (-0.8, 0.8), 1);
    append(&mut both, sheet((-0.5, 1.0), 1.0, (-0.5, 1.0), 2));
    let front = sheet((-1.0, 0.5), 0.0, (-0.8, 0.8), 1);
    let back = sheet((-0.5, 1.0), 1.0, (-0.5, 1.0), 2);
    let mut colors = FaceColors::default();
    colors.set(1, Some([220, 60, 40]));
    colors.set(2, Some([40, 90, 230]));
    let camera = front_camera();
    let look = lit_look();
    let p = params(100, 100);
    let alone_front = render_studio(&front, &colors, &camera, &p, &look, &env).unwrap();
    let alone_back = render_studio(&back, &colors, &camera, &p, &look, &env).unwrap();
    for opacity in [0.2, 0.5, 0.85] {
        let seen = render_studio(
            &both,
            &colors,
            &camera,
            &RenderParams { opacity, ..p },
            &look,
            &env,
        )
        .unwrap();
        evidence(&format!("sheets-{opacity}.png"), &seen);
        let mut checked = [0; 2];
        for y in 0..100 {
            for x in 0..100 {
                let i = (y * 100 + x) as usize;
                if seen.ids[i] != 1 {
                    continue;
                }
                // Away from the back sheet's outline.
                let behind_here = alone_back.ids[i];
                let steady = (-1_i64..=1).all(|dy| {
                    (-1_i64..=1).all(|dx| {
                        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                        !(0..100).contains(&xx)
                            || !(0..100).contains(&yy)
                            || alone_back.ids[(yy * 100 + xx) as usize] == behind_here
                    })
                });
                if !steady {
                    continue;
                }
                let (f, b) = (pixel(&alone_front, x, y), pixel(&alone_back, x, y));
                let got = pixel(&seen, x, y);
                for k in 0..3 {
                    let light = opacity * tonemap::byte_to_linear(f[k])
                        + (1.0 - opacity) * tonemap::byte_to_linear(b[k]);
                    let want = tonemap::to_byte(tonemap::srgb_encode(light));
                    let off = (i32::from(got[k]) - i32::from(want)).abs();
                    assert!(
                        off <= 1,
                        "at ({x}, {y}) opacity {opacity}: {got:?} against {want} in {k}"
                    );
                }
                checked[usize::from(behind_here != NO_FACE)] += 1;
            }
        }
        assert!(checked[0] > 300 && checked[1] > 300, "{checked:?}");
    }
}

/// A see-through body lets `1 − opacity` of the key's light through its
/// shadow: the ground's visibility under a ball is that share plus the
/// rest of what the opaque ball lets through.
#[test]
fn a_see_through_ball_lets_its_share_of_the_key_through() {
    let env = sky();
    let mut mesh = sphere(Vec3::ZERO, 1.0, 32, 64, 0);
    append(&mut mesh, sphere(Vec3::new(2.5, 2.5, -3.0), 0.01, 4, 8, 0));
    let look = Look {
        shadows: true,
        ground: true,
        ..lit_look()
    };
    let down = Vec3::new(0.0, 0.0, -1.0);
    let visibility = |light: &FrameLight<'_>, x: f64, y: f64| {
        let key = light.key_luminance * light.key.map_or(0.0, |(l, _, _)| l.z.max(0.0));
        let f = light.ground_factor(Vec3::new(x, y, 5.0), down);
        (f * (light.rest_up + key) - light.rest_up) / key
    };
    let opaque = FrameLight::new(&mesh, &env, &look);
    for opacity in [0.25, 0.6, 0.9] {
        let light = FrameLight::of_picture(&mesh, &mesh, None, opacity, &env, &look);
        assert!((light.pass - (1.0 - opacity)).abs() < 1e-15);
        for (x, y) in [(0.0, 0.0), (0.3, 0.2), (0.8, 0.0), (-2.5, -2.5)] {
            let v = visibility(&opaque, x, y);
            let want = (1.0 - opacity) + opacity * v;
            let got = visibility(&light, x, y);
            assert!(
                (got - want).abs() < 1e-9,
                "at ({x}, {y}): {got} against {want}"
            );
        }
        assert!(visibility(&light, 0.0, 0.0) > 1.0 - opacity - 1e-9);
    }
    // Opaque bodies: the same light, bit for bit.
    let same = FrameLight::of_picture(&mesh, &mesh, None, 1.0, &env, &look);
    assert_eq!(same.pass, 0.0);
    assert_eq!(
        same.ground_factor(Vec3::new(0.3, 0.1, 5.0), down),
        opaque.ground_factor(Vec3::new(0.3, 0.1, 5.0), down)
    );
}

/// What a section cuts away casts no shadow: under the cut-away half of a
/// ball lit from straight above the ground is lit, under the kept half it
/// is in the shadow as before.
#[test]
fn what_is_cut_away_casts_no_shadow() {
    let env = sky();
    let mut mesh = sphere(Vec3::ZERO, 1.0, 32, 64, 0);
    append(&mut mesh, sphere(Vec3::new(2.5, 2.5, -3.0), 0.01, 4, 8, 0));
    let look = Look {
        shadows: true,
        ground: true,
        ..lit_look()
    };
    let down = Vec3::new(0.0, 0.0, -1.0);
    let cut = Cut::new(
        &Section {
            origin: Vec3::ZERO,
            normal: Vec3::new(1.0, 0.0, 0.0),
            cap: true,
            color: [200, 80, 80],
        },
        &section_bounds(&mesh),
    )
    .unwrap();
    let whole = FrameLight::new(&mesh, &env, &look);
    let light = FrameLight::of_picture(&mesh, &mesh, Some(&cut), 1.0, &env, &look);
    let visibility = |light: &FrameLight<'_>, x: f64| {
        let key = light.key_luminance * light.key.map_or(0.0, |(l, _, _)| l.z.max(0.0));
        let f = light.ground_factor(Vec3::new(x, 0.0, 5.0), down);
        (f * (light.rest_up + key) - light.rest_up) / key
    };
    // Every ray from (x, 0, −3) to the cap z > 0.99 passes within
    // 3 sin(acos 0.99) ≈ 0.42 of x at the ball: beyond 0.5 it is all in
    // the cut-away half.
    for x in [0.6, 0.8] {
        assert!(visibility(&whole, x) < 0.5, "whole at {x}");
        assert!(
            visibility(&light, x) > 0.97,
            "cut open at {x}: {}",
            visibility(&light, x)
        );
    }
    for x in [-0.45, -0.6] {
        let (a, b) = (visibility(&whole, x), visibility(&light, x));
        assert!(
            b < 0.05 && (a - b).abs() < 1e-12,
            "kept half at {x}: {a} {b}"
        );
    }
}

/// The same input gives the same picture, cut open and seen through, with
/// every effect of the look on.
#[test]
fn the_same_input_gives_the_same_cut_open_see_through_picture() {
    let env = sky();
    let mut mesh = sphere(Vec3::ZERO, 1.0, 24, 48, 3);
    append(&mut mesh, sphere(Vec3::new(0.2, 0.0, 0.0), 0.5, 16, 32, 4));
    let p = RenderParams {
        opacity: 0.6,
        section: Some(Section {
            origin: Vec3::ZERO,
            normal: Vec3::new(0.3, -1.0, 0.1),
            cap: true,
            color: [90, 160, 200],
        }),
        ..params(120, 90)
    };
    let look = Look {
        tonemap: Tonemap::Agx,
        accumulate: 2,
        ..Look::polished()
    };
    let camera = front_camera();
    let a = render_studio(&mesh, &FaceColors::default(), &camera, &p, &look, &env).unwrap();
    let b = render_studio(&mesh, &FaceColors::default(), &camera, &p, &look, &env).unwrap();
    assert_eq!(a.rgb, b.rgb);
    evidence("polished-cut-see-through.png", &a);
}
