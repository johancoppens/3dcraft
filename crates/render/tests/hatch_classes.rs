//! A direction of hatching per part (docs/design/rendering.md › *A
//! direction per part*): the CPU reference renderer counts what a section
//! cuts away per hatch class of the faces (`FaceColors::hatch_classes`),
//! hatches each body's cap in its class's direction, cross-hatches where
//! bodies of two classes overlap, and draws a mesh without classes byte
//! for byte as before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Vec3;
use craft3d_render::{
    Camera, Cut, FaceColors, HATCH_CLASSES, Hatch, Hatching, Projection, RenderParams,
    SECTION_FACE, Section, pick_cut, pick_cut_classed, render, render_colored, section_bounds,
};
use craft3d_tess::Mesh;
use std::time::Instant;

const PAPER: [u8; 3] = [232, 228, 220];
const HATCH: Hatch = Hatch {
    ink: [88, 84, 78],
    spacing_points: 9.0,
    width_points: 1.0,
    height_points: None,
};

/// Axis-aligned boxes as one mesh, six faces each (normals out), and the
/// first face id of each.
fn boxes(list: &[(Vec3, Vec3)]) -> (Mesh, Vec<u32>) {
    let mut mesh = Mesh::default();
    let mut firsts = Vec::new();
    for (k, &(lo, hi)) in list.iter().enumerate() {
        let first = (k * 6) as u32;
        firsts.push(first);
        let corner = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { hi.x } else { lo.x },
                if y { hi.y } else { lo.y },
                if z { hi.z } else { lo.z },
            )
        };
        // Per face: its normal and its corners counter-clockwise about it.
        let faces: [(Vec3, [Vec3; 4]); 6] = [
            (
                Vec3::X,
                [
                    corner(true, false, false),
                    corner(true, true, false),
                    corner(true, true, true),
                    corner(true, false, true),
                ],
            ),
            (
                -Vec3::X,
                [
                    corner(false, false, false),
                    corner(false, false, true),
                    corner(false, true, true),
                    corner(false, true, false),
                ],
            ),
            (
                Vec3::Y,
                [
                    corner(false, true, false),
                    corner(false, true, true),
                    corner(true, true, true),
                    corner(true, true, false),
                ],
            ),
            (
                -Vec3::Y,
                [
                    corner(false, false, false),
                    corner(true, false, false),
                    corner(true, false, true),
                    corner(false, false, true),
                ],
            ),
            (
                Vec3::Z,
                [
                    corner(false, false, true),
                    corner(true, false, true),
                    corner(true, true, true),
                    corner(false, true, true),
                ],
            ),
            (
                -Vec3::Z,
                [
                    corner(false, false, false),
                    corner(false, true, false),
                    corner(true, true, false),
                    corner(true, false, false),
                ],
            ),
        ];
        for (f, (normal, quad)) in faces.into_iter().enumerate() {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend(quad);
            mesh.normals.extend([normal; 4]);
            mesh.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
            mesh.face_ids.extend([first + f as u32; 2]);
        }
    }
    (mesh, firsts)
}

/// Each body of `firsts` of the class given.
fn classed(firsts: &[u32], class: impl Fn(usize) -> usize) -> FaceColors {
    let mut colors = FaceColors::new();
    for (k, &first) in firsts.iter().enumerate() {
        assert!(colors.set_hatch_class(first, class(k)));
    }
    colors
}

/// Across y at 5, the front half (y below 5) cut away, hatched.
fn section() -> Section {
    Section {
        origin: Vec3::new(0.0, 5.0, 0.0),
        normal: -Vec3::Y,
        cap: true,
        color: PAPER,
        hatch: Some(HATCH),
    }
}

/// From the front (-y), 10 pixels a unit: x from 0 at pixel 0, z = 5 on
/// row 150.
fn front() -> Camera {
    Camera {
        eye: Vec3::new(20.0, -200.0, 5.0),
        target: Vec3::new(20.0, 0.0, 5.0),
        up: Vec3::Z,
        projection: Projection::Orthographic { half_height: 15.0 },
    }
}

fn params(ssaa: u32) -> RenderParams {
    RenderParams {
        width: 400,
        height: 300,
        ssaa,
        section: Some(section()),
        ..RenderParams::default()
    }
}

/// A row of four blocks of 10, each touching the next.
fn row() -> (Mesh, Vec<u32>) {
    boxes(
        &(0..4)
            .map(|k| {
                let x = f64::from(k) * 10.0;
                (Vec3::new(x, 0.0, 0.0), Vec3::new(x + 10.0, 10.0, 10.0))
            })
            .collect::<Vec<_>>(),
    )
}

/// The hatching the renderer works the cap out with.
fn hatching(mesh: &Mesh) -> Hatching {
    let cut = Cut::new(&section(), &section_bounds(mesh)).unwrap();
    Hatching::new(cut.normal, cut.color, &HATCH, &front(), 300).unwrap()
}

/// Four touching blocks of the classes 0 to 3: every pixel of the cap of
/// each, away from where two meet, is the hatching of its own class at
/// the point the pick finds there, and a good part of each of classes 1
/// to 3 differs from what class 0 would draw there: neighbours read apart.
#[test]
fn touching_parts_are_hatched_each_in_its_own_direction() {
    let (mesh, firsts) = row();
    let colors = classed(&firsts, |k| k);
    let p = params(1);
    let frame = render_colored(&mesh, &colors, &front(), &p).unwrap();
    let h = hatching(&mesh);
    let mut seen = [0_usize; HATCH_CLASSES];
    let mut apart = [0_usize; HATCH_CLASSES];
    for (i, &id) in frame.ids.iter().enumerate() {
        let (px, py) = ((i % 400) as f64 + 0.5, (i / 400) as f64 + 0.5);
        let x = px / 10.0;
        if id != SECTION_FACE || [10.0, 20.0, 30.0].iter().any(|b| (x - b).abs() < 0.15) {
            continue;
        }
        let hit = pick_cut_classed(
            &mesh,
            colors.hatch_classes(),
            &front(),
            400,
            300,
            px,
            py,
            Some(&section()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(hit.face, SECTION_FACE);
        let class = (hit.point.x / 10.0).floor() as usize;
        let want = h.color_mask(hit.point, hit.depth, 1 << class);
        assert_eq!(
            &frame.rgb[i * 3..i * 3 + 3],
            &want,
            "pixel {i}, class {class}"
        );
        seen[class] += 1;
        if want != h.color(hit.point, hit.depth) {
            apart[class] += 1;
        }
    }
    for class in 0..HATCH_CLASSES {
        assert!(seen[class] > 5000, "{seen:?}");
    }
    assert_eq!(apart[0], 0);
    for class in 1..HATCH_CLASSES {
        assert!(apart[class] * 10 > seen[class], "{apart:?} of {seen:?}");
    }
    // The cap is where it was without classes: touching parts do not
    // overlap, so a count per class is odd where the count of all was.
    let plain = render(&mesh, &front(), &p).unwrap();
    assert_eq!(frame.ids, plain.ids);
    assert_eq!(frame.depth, plain.depth);
}

/// A mesh without classes, and one whose bodies are all of class 0, is
/// drawn byte for byte as `render` draws it, with one sample a pixel and
/// with nine, opaque and see-through.
#[test]
fn without_classes_the_picture_is_byte_for_byte_as_before() {
    let (mesh, firsts) = row();
    let zero = classed(&firsts, |_| 0);
    assert!(zero.hatch_classes().is_empty());
    for (ssaa, opacity) in [(1, 1.0), (3, 1.0), (2, 0.5)] {
        let p = RenderParams {
            opacity,
            ..params(ssaa)
        };
        let plain = render(&mesh, &front(), &p).unwrap();
        assert_eq!(
            render_colored(&mesh, &FaceColors::new(), &front(), &p).unwrap(),
            plain
        );
        assert_eq!(render_colored(&mesh, &zero, &front(), &p).unwrap(), plain);
    }
}

/// Two blocks that overlap (x 0..10 and 6..16): of different classes, the
/// cap goes on across the overlap, cross-hatched there (both classes cut:
/// the lines of 45° and 135°), and the pick hits it as the picture shows
/// it; of the same class the two counts cancel and the overlap shows no
/// cap, as before there were classes.
#[test]
fn an_overlap_is_cross_hatched_between_classes_and_a_hole_within_one() {
    let (mesh, firsts) = boxes(&[
        (Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0)),
        (Vec3::new(6.0, 0.0, 0.0), Vec3::new(16.0, 10.0, 10.0)),
    ]);
    let h = hatching(&mesh);
    let two = classed(&firsts, |k| k);
    let frame = render_colored(&mesh, &two, &front(), &params(1)).unwrap();
    let one = render_colored(&mesh, &FaceColors::new(), &front(), &params(1)).unwrap();
    let mut crossed = 0;
    for row in [105, 120, 150, 180, 195] {
        for column in 0..160 {
            let i = row * 400 + column;
            let (px, py) = (column as f64 + 0.5, row as f64 + 0.5);
            let x = px / 10.0;
            if [0.0, 6.0, 10.0, 16.0].iter().any(|b| (x - b).abs() < 0.15) {
                continue;
            }
            let hit = pick_cut_classed(
                &mesh,
                two.hatch_classes(),
                &front(),
                400,
                300,
                px,
                py,
                Some(&section()),
            )
            .unwrap();
            // The pick is what the picture shows, with classes and without.
            assert_eq!(hit.map(|h| h.face), Some(frame.ids[i]), "{column} {row}");
            let plain = pick_cut(&mesh, &front(), 400, 300, px, py, Some(&section())).unwrap();
            assert_eq!(plain.map(|h| h.face), Some(one.ids[i]), "{column} {row}");
            if x > 6.0 && x < 10.0 {
                assert_eq!(frame.ids[i], SECTION_FACE, "{column} {row}");
                let hit = hit.unwrap();
                assert_eq!(
                    &frame.rgb[i * 3..i * 3 + 3],
                    &h.color_mask(hit.point, hit.depth, 0b11)
                );
                crossed += 1;
                // Of one class: a hole (what is behind the plane shows).
                assert_ne!(one.ids[i], SECTION_FACE, "{column} {row}");
            } else if x < 16.0 {
                assert_eq!(frame.ids[i], SECTION_FACE);
                assert_eq!(one.ids[i], SECTION_FACE);
            }
        }
    }
    assert!(crossed > 150, "{crossed}");
}

/// The same picture every time, and as many bodies as asked for drawn
/// with their classes at a cost that grows with the triangles, not worse:
/// 1, 2, 5, 162 and 10 000 blocks in a grid, each touching its
/// neighbours, cut across the middle, with classes and without (the
/// timings are printed: what the classes cost the reference renderer).
#[test]
fn many_bodies_cost_what_their_triangles_cost_and_draw_the_same_twice() {
    let mut report = String::new();
    for n in [1_usize, 2, 5, 162, 10_000] {
        let side = (n as f64).sqrt().ceil() as usize;
        let size = 40.0 / side as f64;
        let list: Vec<(Vec3, Vec3)> = (0..n)
            .map(|k| {
                let (i, j) = ((k % side) as f64, (k / side) as f64);
                (
                    Vec3::new(i * size, 0.0, j * size * 0.75),
                    Vec3::new((i + 1.0) * size, 10.0, (j + 1.0) * size * 0.75),
                )
            })
            .collect();
        let (mesh, firsts) = boxes(&list);
        let colors = classed(&firsts, |k| k);
        let p = params(1);
        let started = Instant::now();
        let plain = render(&mesh, &front(), &p).unwrap();
        let without = started.elapsed();
        let started = Instant::now();
        let frame = render_colored(&mesh, &colors, &front(), &p).unwrap();
        let with = started.elapsed();
        assert_eq!(frame.ids, plain.ids, "{n}");
        assert_eq!(
            render_colored(&mesh, &colors, &front(), &p).unwrap(),
            frame,
            "{n}"
        );
        let line = format!(
            "{n} bodies, {} triangles: {:.1} ms without classes, {:.1} ms with ({:.2}x)\n",
            mesh.triangles.len(),
            without.as_secs_f64() * 1e3,
            with.as_secs_f64() * 1e3,
            with.as_secs_f64() / without.as_secs_f64().max(1e-9)
        );
        print!("{line}");
        report.push_str(&line);
    }
    craft3d_testkit::evidence::print("hatch-classes-cost.txt", report.as_bytes());
}

/// Classes beyond what a mesh has, faces beyond the classes' runs, a run
/// at the last face id: nothing panics, and a face without a run is of
/// class 0.
#[test]
fn hostile_classes_never_panic() {
    let (mesh, firsts) = row();
    let mut colors = classed(&firsts, |k| k * 7 + 3);
    assert!(colors.set_hatch_class(u32::MAX, 2));
    assert!(colors.set_hatch_class(1 << 30, usize::MAX));
    let frame = render_colored(&mesh, &colors, &front(), &params(2)).unwrap();
    assert!(frame.ids.contains(&SECTION_FACE));
    assert_eq!(colors.hatch_classes().class(u32::MAX), 2);
    for (x, y) in [(-1.0, 5.0), (f64::NAN, 2.0), (1e30, 1e30), (35.5, 150.5)] {
        let _ = pick_cut_classed(
            &mesh,
            colors.hatch_classes(),
            &front(),
            400,
            300,
            x,
            y,
            Some(&section()),
        );
    }
}
