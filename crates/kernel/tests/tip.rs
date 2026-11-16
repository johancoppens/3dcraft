//! The tip spliced into a body (`kernel::tip`): the flat end of a prism
//! or a frustum with spline sides replaced by the faces of
//! `craft3d_geom::tip`, the rest of the body as it was.
//!
//! Held to closed forms (oracle 1 in docs/testing.md), at the scales 1e-3,
//! 1 and 1e6 and in two frames: the half ball on a rod as long as its
//! radius (also at the length chosen when none is given), the half
//! ellipsoid on an elliptic prism, two thirds of base times length over
//! any prism with quarter ellipses and eleven fifteenths with parabolas;
//! every body valid, measured exactly and meshed watertight at two
//! tolerances; the faces behind the end unchanged; the tip tangent to its
//! sides along the rim and with one tangent plane at its point, sampled
//! here independently of the proofs of the nets.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::tip::{ROUND_FULLNESS, TipAsk, TipOrigin, Tipped, tip};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::tips::{
    PARABOLA, Piece, ROUND, ellipse, polygon, polygon_area, spline_frustum, spline_prism,
};
use craft3d_topo::{Body, FaceId, check, mass_properties};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const DEPTH: f64 = 2.0;

fn frames(k: f64) -> [Frame; 2] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap(),
    ]
}

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-7 * k, 1e-9).unwrap()
}

/// The end of a prism made by `spline_prism`: its last face.
fn end_of(body: &Body) -> FaceId {
    FaceId(u32::try_from(body.faces.len() - 1).unwrap())
}

fn ask(face: FaceId, length: Option<f64>, fullness: f64) -> TipAsk {
    TipAsk {
        face,
        length,
        at: None,
        fullness,
    }
}

/// Valid, measured, and meshed watertight at two tolerances: the body's
/// volume.
#[track_caller]
fn sound(body: &Body, k: f64, size: f64, what: &str) -> f64 {
    assert_eq!(check(body, &tolerance(k)), [], "{what}");
    let exact = mass_properties(body).unwrap_or_else(|e| panic!("{what}: {e}"));
    for chord in [2e-2, 2e-3] {
        let chord = chord * size * k;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * chord,
            "{what}: mesh volume {} against {}",
            welded.volume(),
            exact.volume
        );
    }
    exact.volume
}

#[track_caller]
fn assert_near(got: f64, want: f64, relative: f64, what: &str) {
    println!("{what}: {:e} of itself off", ((got - want) / want).abs());
    assert!(
        (got - want).abs() <= relative * want.abs(),
        "{what}: got {got:e}, want {want:e} (off by {:e})",
        ((got - want) / want).abs()
    );
}

/// The faces of the tip, with the side each continues.
fn tip_faces(made: &Tipped) -> Vec<(FaceId, FaceId)> {
    made.faces
        .iter()
        .enumerate()
        .filter_map(|(index, origin)| match origin {
            TipOrigin::Tip { side } => Some((FaceId(u32::try_from(index).unwrap()), *side)),
            TipOrigin::Kept(_) => None,
        })
        .collect()
}

fn nurbs(body: &Body, face: FaceId) -> &craft3d_geom::NurbsSurface {
    match &body.face(face).unwrap().surface {
        Surface::Nurbs(surface) => surface,
        other => panic!("a tip face on a {}", other.kind()),
    }
}

/// The outward unit normal of `face` of `body` at `(u, v)`.
fn outward(body: &Body, face: FaceId, u: f64, v: f64) -> Vec3 {
    let surface = nurbs(body, face);
    let (_, du, dv) = surface.derivatives(u, v);
    let n = du.cross(dv).normalized().unwrap();
    if body.face(face).unwrap().reversed {
        -n
    } else {
        n
    }
}

/// Everything that was behind the end is as it was: every kept face has
/// the surface, orientation and loops of the face it is (edges and
/// vertices keep their numbers), and every edge and vertex is there.
#[track_caller]
fn behind_unchanged(before: &Body, made: &Tipped, what: &str) {
    for (index, origin) in made.faces.iter().enumerate() {
        let TipOrigin::Kept(was) = origin else {
            continue;
        };
        let (now, was) = (&made.body.faces[index], &before.faces[was.0 as usize]);
        assert_eq!(now.surface, was.surface, "{what}: face {index}");
        assert_eq!(now.reversed, was.reversed, "{what}: face {index}");
        let coedges = |body: &Body, face: &craft3d_topo::Face| -> Vec<Vec<craft3d_topo::Coedge>> {
            face.loops
                .iter()
                .map(|l| body.loop_(*l).unwrap().coedges.clone())
                .collect()
        };
        assert_eq!(
            coedges(&made.body, now),
            coedges(before, was),
            "{what}: the loops of face {index}"
        );
    }
    assert_eq!(
        made.body.edges[..before.edges.len()],
        before.edges[..],
        "{what}"
    );
    assert_eq!(
        made.body.vertices[..before.vertices.len()],
        before.vertices[..],
        "{what}"
    );
    assert_eq!(
        made.body.edges.len(),
        before.edges.len() + tip_faces(made).len()
    );
    assert_eq!(made.body.vertices.len(), before.vertices.len() + 1);
}

/// Along every rim edge the tip's face and its side have one tangent
/// plane (both outward normals, at 17 places), and near the tip every
/// face's normal turns to the end's normal in proportion to the distance
/// from it.
#[track_caller]
fn tangent(made: &Tipped, normal: Vec3, what: &str) {
    let mut rim = 0.0_f64;
    let mut near_tip = [0.0_f64; 3];
    for (face, side) in tip_faces(made) {
        let surface = nurbs(&made.body, face);
        let ((u0, u1), _) = surface.domain();
        for i in 0..=16 {
            let u = u0 + (u1 - u0) * f64::from(i) / 16.0;
            let point = surface.eval(u, 0.0);
            // The side's border at the rim is its v = 1 (the prism's
            // sides run from the base up to the end).
            let side_normal = outward(&made.body, side, u, 1.0);
            assert!(
                nurbs(&made.body, side).eval(u, 1.0).distance(point)
                    <= 1e-12 * point.length().max(1.0),
                "{what}: the rim is not the side's border"
            );
            let angle = outward(&made.body, face, u, 0.0).angle_to(side_normal);
            assert!(angle <= 1e-12, "{what}: {angle:e} rad across the rim");
            rim = rim.max(angle);
        }
        let mut last = f64::INFINITY;
        for (slot, eps) in near_tip.iter_mut().zip([1e-2, 1e-3, 1e-4]) {
            let largest = (0..=8)
                .map(|i| {
                    let u = u0 + (u1 - u0) * f64::from(i) / 8.0;
                    outward(&made.body, face, u, 1.0 - eps).angle_to(normal)
                })
                .fold(0.0_f64, f64::max);
            assert!(
                largest < last * 0.2 && largest < 50.0 * eps,
                "{what}: {largest:e} rad off the end's normal at {eps:e} from the tip"
            );
            last = largest;
            *slot = slot.max(largest);
        }
    }
    println!(
        "{what}: across the rim at most {rim:e} rad; off the end's normal at 1e-2, 1e-3, 1e-4 of the way from the tip {:e}, {:e}, {:e} rad; star {:e}",
        near_tip[0], near_tip[1], near_tip[2], made.star
    );
}

#[test]
fn a_tip_on_a_rod_as_long_as_its_radius_is_the_half_ball() {
    let r = 5.0;
    for k in SCALES {
        for frame in frames(k) {
            let rod = spline_prism(&frame, k, &ellipse(r, r), DEPTH * r).unwrap();
            let base = PI * r * r * DEPTH * r;
            // Given the length, and at the length chosen: the radius of
            // the circle with the end's area, which never closes in here.
            for length in [Some(r * k), None] {
                let what = format!("k {k:e}, {frame:?}, length {length:?}");
                let made = tip(
                    &rod,
                    &ask(end_of(&rod), length, ROUND_FULLNESS),
                    &tolerance(k),
                    None,
                )
                .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_near(made.length, r * k, 1e-14, &what);
                assert!(
                    made.tip.distance(frame.point(0.0, 0.0, r * k)) <= 1e-13 * k * r,
                    "{what}"
                );
                let volume = sound(&made.body, k, r, &what);
                assert_near(
                    volume,
                    (base + 2.0 / 3.0 * PI * r * r * r) * k * k * k,
                    1e-12,
                    &what,
                );
                // Every point of the tip on the ball.
                for (face, _) in tip_faces(&made) {
                    let surface = nurbs(&made.body, face);
                    for i in 0..=8 {
                        for j in 0..=8 {
                            let p = surface.eval(f64::from(i) / 8.0, f64::from(j) / 8.0);
                            let off = (p.distance(frame.origin()) - r * k).abs();
                            assert!(off <= 1e-13 * r * k, "{what}: {off:e} off the ball");
                        }
                    }
                }
                behind_unchanged(&rod, &made, &what);
                // What the nets prove and report: the sides go on toward
                // the tip, the shoulder is seen from the tip, and every
                // rail of the circle is smooth.
                assert!(made.onward >= craft3d_geom::tip::ONWARD_MARGIN, "{what}");
                assert!(made.star > 0.0, "{what}");
                assert!(made.crease <= 1e-12, "{what}: crease {:e}", made.crease);
                tangent(&made, frame.z(), &what);
            }
        }
    }
}

#[test]
fn a_tip_on_an_elliptic_prism_is_the_half_ellipsoid() {
    let (a, b, l) = (4.0, 1.5, 3.0);
    for k in SCALES {
        for frame in frames(k) {
            let what = format!("k {k:e}, {frame:?}");
            let prism = spline_prism(&frame, k, &ellipse(a, b), DEPTH).unwrap();
            let made = tip(
                &prism,
                &ask(end_of(&prism), Some(l * k), ROUND_FULLNESS),
                &tolerance(k),
                None,
            )
            .unwrap_or_else(|e| panic!("{what}: {e}"));
            let volume = sound(&made.body, k, a, &what);
            let want = (PI * a * b * DEPTH + 2.0 / 3.0 * PI * a * b * l) * k * k * k;
            assert_near(volume, want, 1e-12, &what);
            for (face, _) in tip_faces(&made) {
                let surface = nurbs(&made.body, face);
                for i in 0..=8 {
                    for j in 0..=8 {
                        let p = surface.eval(f64::from(i) / 8.0, f64::from(j) / 8.0);
                        let (x, y, z) = frame.local(p);
                        let (x, y, z) = (x / (a * k), y / (b * k), z / (l * k));
                        let on = x * x + y * y + z * z;
                        assert!((on - 1.0).abs() <= 1e-12, "{what}: {on} off the ellipsoid");
                    }
                }
            }
            behind_unchanged(&prism, &made, &what);
            tangent(&made, frame.z(), &what);
            assert!(made.crease <= 1e-12, "{what}: crease {:e}", made.crease);
        }
    }
}

/// Sections of the prisms: a square, a triangle, an L, a lens of two
/// parabolas (corners) and an oval of two cubic arcs (no corners).
/// A section: its name, its pieces, and where its tip is placed when
/// not over its centre of area.
type Section = (&'static str, Vec<Piece>, Option<[f64; 2]>);

fn sections() -> Vec<Section> {
    let lens = vec![
        (2, vec![[-3.0, 0.0], [0.0, -2.0], [3.0, 0.0]], vec![1.0; 3]),
        (2, vec![[3.0, 0.0], [0.0, 2.0], [-3.0, 0.0]], vec![1.0; 3]),
    ];
    let oval = vec![
        (
            3,
            vec![[-3.0, 0.0], [-3.0, -1.0], [3.0, -1.0], [3.0, 0.0]],
            vec![1.0; 4],
        ),
        (
            3,
            vec![[3.0, 0.0], [3.0, 1.0], [-3.0, 1.0], [-3.0, 0.0]],
            vec![1.0; 4],
        ),
    ];
    // The L's centre of area lies on the line of its inner edge, where
    // the tip would see that edge edge-on: its tip is placed in the
    // corner square, from which the whole L is in view.
    vec![
        (
            "square",
            polygon(&[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]),
            None,
        ),
        (
            "triangle",
            polygon(&[[0.0, 0.0], [3.0, 0.0], [0.0, 2.0]]),
            None,
        ),
        (
            "L",
            polygon(&[
                [0.0, 0.0],
                [4.0, 0.0],
                [4.0, 1.0],
                [1.0, 1.0],
                [1.0, 3.0],
                [0.0, 3.0],
            ]),
            Some([0.5, 0.5]),
        ),
        ("lens", lens, None),
        ("oval", oval, None),
    ]
}

/// The area of a section: polygons by their corners; the lens two
/// parabolic segments, each two thirds of the triangle of its control
/// points (6 · 2 / 2): 8; the oval two cubic arcs over a chord 6 long
/// whose inner control points stand 1 off it at its ends, each enclosing
/// 3/5 of 6 · 1 with the chord: 7.2.
fn area(name: &str, section: &[Piece]) -> f64 {
    match name {
        "lens" => 8.0,
        "oval" => 7.2,
        _ => polygon_area(
            &section
                .iter()
                .map(|(_, points, _)| points[0])
                .collect::<Vec<_>>(),
        ),
    }
}

#[test]
fn over_any_prism_the_tip_is_two_thirds_of_base_times_length_or_eleven_fifteenths() {
    for (name, section, at) in sections() {
        let base = area(name, &section);
        for (fullness, share) in [ROUND, PARABOLA] {
            for k in SCALES {
                for frame in frames(k) {
                    let what = format!("{name}, fullness {fullness}, k {k:e}, {frame:?}");
                    let prism = spline_prism(&frame, k, &section, DEPTH).unwrap();
                    let before = mass_properties(&prism).unwrap().volume;
                    assert_near(before, base * DEPTH * k * k * k, 1e-13, &what);
                    let l = 1.5;
                    let made = tip(
                        &prism,
                        &TipAsk {
                            at: at.map(|[x, y]| [x * k, y * k]),
                            ..ask(end_of(&prism), Some(l * k), fullness)
                        },
                        &tolerance(k),
                        None,
                    )
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let volume = sound(&made.body, k, 4.0, &what);
                    assert_near(volume - before, share * base * l * k * k * k, 1e-12, &what);
                    behind_unchanged(&prism, &made, &what);
                    tangent(&made, frame.z(), &what);
                    // A section with corners creases its rails as its
                    // sides crease; the oval's are smooth.
                    if name == "oval" {
                        assert!(made.crease <= 1e-12, "{what}: crease {:e}", made.crease);
                    } else {
                        assert!(made.crease > 0.1, "{what}: crease {:e}", made.crease);
                    }
                }
            }
        }
    }
}

#[test]
fn the_same_body_gives_the_same_bits() {
    let frame = frames(1.0)[1];
    let prism = spline_prism(&frame, 1.0, &sections()[4].1, DEPTH).unwrap();
    let once = tip(
        &prism,
        &ask(end_of(&prism), None, ROUND_FULLNESS),
        &tolerance(1.0),
        None,
    )
    .unwrap();
    let again = tip(
        &prism,
        &ask(end_of(&prism), None, ROUND_FULLNESS),
        &tolerance(1.0),
        None,
    )
    .unwrap();
    assert_eq!(format!("{:?}", once.body), format!("{:?}", again.body));
}

#[test]
fn the_base_of_a_prism_takes_a_tip_too() {
    let k = 1.0;
    let frame = frames(k)[1];
    let prism = spline_prism(&frame, k, &ellipse(2.0, 2.0), DEPTH).unwrap();
    // The base is the face before the end.
    let base = FaceId(end_of(&prism).0 - 1);
    let made = tip(
        &prism,
        &ask(base, Some(2.0), ROUND_FULLNESS),
        &tolerance(k),
        None,
    )
    .unwrap();
    let volume = sound(&made.body, k, 2.0, "base");
    assert_near(
        volume,
        PI * 4.0 * DEPTH + 2.0 / 3.0 * PI * 8.0,
        1e-12,
        "base",
    );
    assert!(made.tip.distance(frame.point(0.0, 0.0, -DEPTH - 2.0)) <= 1e-13);
}

#[test]
fn a_tip_placed_off_the_middle_still_has_the_closed_form_volume() {
    let section = polygon(&[[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]]);
    let frame = frames(1.0)[1];
    let prism = spline_prism(&frame, 1.0, &section, DEPTH).unwrap();
    let before = mass_properties(&prism).unwrap().volume;
    let made = tip(
        &prism,
        &TipAsk {
            face: end_of(&prism),
            length: Some(2.0),
            at: Some([3.0, 0.5]),
            fullness: ROUND_FULLNESS,
        },
        &tolerance(1.0),
        None,
    )
    .unwrap();
    assert!(made.tip.distance(frame.point(3.0, 0.5, 2.0)) <= 1e-13);
    let volume = sound(&made.body, 1.0, 4.0, "off the middle");
    assert_near(
        volume - before,
        2.0 / 3.0 * 12.0 * 2.0,
        1e-12,
        "off the middle",
    );
}

#[test]
fn a_frustum_takes_a_tip_up_to_where_its_sides_close_in() {
    // Sides that lean in to meet 2 · ½ / ½ = 2 beyond the end.
    let frame = frames(1.0)[1];
    let section = ellipse(2.0, 2.0);
    let frustum = spline_frustum(&frame, 1.0, &section, DEPTH, 0.5, 0.0).unwrap();
    let end = end_of(&frustum);
    let made = tip(
        &frustum,
        &ask(end, Some(1.0), ROUND_FULLNESS),
        &tolerance(1.0),
        None,
    )
    .unwrap();
    sound(&made.body, 1.0, 2.0, "frustum");
    tangent(&made, frame.z(), "frustum");
    // At the length chosen: no more than half of where the sides meet.
    let chosen = tip(
        &frustum,
        &ask(end, None, ROUND_FULLNESS),
        &tolerance(1.0),
        None,
    )
    .unwrap();
    assert!(chosen.length <= 1.0 + 1e-6, "chosen {}", chosen.length);
    sound(&chosen.body, 1.0, 2.0, "frustum, length chosen");
    // Past where they meet: refused, with the edge named.
    let error = tip(
        &frustum,
        &ask(end, Some(3.0), ROUND_FULLNESS),
        &tolerance(1.0),
        None,
    )
    .expect_err("past the apex");
    let text = error.to_string();
    assert!(
        matches!(error, KernelError::Tip { .. }) && text.contains("shorter"),
        "{text}"
    );
}

/// Sides whose steps across the end differ in length (the base tilted):
/// the tip's general form, four rows, through the splice. No closed form;
/// held to the kernel's checks, the mesh, the tangency at the rim and the
/// tip, and the faces behind the end.
#[test]
fn sides_that_reach_the_tip_s_plane_unevenly_take_the_general_form() {
    use craft3d_geom::tip::TipForm;
    for k in SCALES {
        for frame in frames(k) {
            for (name, section, _) in sections() {
                let what = format!("{name}, k {k:e}, {frame:?}");
                let leaning = spline_frustum(&frame, k, &section, DEPTH, 1.0, 0.2).unwrap();
                let at = if name == "L" {
                    Some([0.5 * k, 0.5 * k])
                } else {
                    None
                };
                let made = tip(
                    &leaning,
                    &TipAsk {
                        at,
                        ..ask(end_of(&leaning), Some(1.0 * k), ROUND_FULLNESS)
                    },
                    &tolerance(k),
                    None,
                )
                .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(made.form, TipForm::General, "{what}");
                sound(&made.body, k, 4.0, &what);
                behind_unchanged(&leaning, &made, &what);
                tangent(&made, frame.z(), &what);
            }
        }
    }
}

#[test]
fn what_is_not_a_flat_end_with_spline_sides_is_refused_with_its_reason() {
    let frame = Frame::WORLD;
    let prism = spline_prism(&frame, 1.0, &ellipse(2.0, 2.0), DEPTH).unwrap();
    let end = end_of(&prism);
    let tol = tolerance(1.0);
    let refused = |body: &Body, asked: TipAsk| -> String {
        match tip(body, &asked, &tol, None) {
            Ok(_) => panic!("{asked:?} built"),
            Err(error) => error.to_string(),
        }
    };
    // A side: not flat.
    let text = refused(&prism, ask(FaceId(0), Some(1.0), ROUND_FULLNESS));
    assert!(text.contains("not flat"), "{text}");
    // No such face.
    let text = refused(&prism, ask(FaceId(99), Some(1.0), ROUND_FULLNESS));
    assert!(text.contains("no such face"), "{text}");
    // Lengths and fullness that are no numbers of a tip.
    for length in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let text = refused(&prism, ask(end, Some(length), ROUND_FULLNESS));
        assert!(text.contains("length"), "{length}: {text}");
    }
    for fullness in [0.0, 1.0, -0.5, f64::NAN] {
        let text = refused(&prism, ask(end, Some(1.0), fullness));
        assert!(text.contains("fullness"), "{fullness}: {text}");
    }
    let text = refused(
        &prism,
        TipAsk {
            at: Some([f64::NAN, 0.0]),
            ..ask(end, Some(1.0), ROUND_FULLNESS)
        },
    );
    assert!(text.contains("at"), "{text}");
    // A tip standing outside the end.
    let text = refused(
        &prism,
        TipAsk {
            at: Some([5.0, 0.0]),
            ..ask(end, Some(1.0), ROUND_FULLNESS)
        },
    );
    assert!(text.contains("tip"), "{text}");
    // A block's top: its sides are planes, not built yet.
    let block = craft3d_kernel::primitives::block(&Frame::WORLD, 1.0, 1.0, 1.0).unwrap();
    let top = (0..block.faces.len())
        .map(|i| FaceId(u32::try_from(i).unwrap()))
        .find(|f| match &block.face(*f).unwrap().surface {
            Surface::Plane(p) => {
                p.frame().z().dot(Vec3::new(0.0, 0.0, 1.0)) > 0.5
                    && !block.face(*f).unwrap().reversed
            }
            _ => false,
        })
        .unwrap();
    let text = refused(&block, ask(top, Some(0.5), ROUND_FULLNESS));
    assert!(text.contains("plane"), "{text}");
}

#[test]
fn a_huge_length_is_built_or_refused_never_wrong() {
    let frame = Frame::WORLD;
    let prism = spline_prism(&frame, 1.0, &ellipse(2.0, 2.0), DEPTH).unwrap();
    for length in [1e3, 1e9, 1e300] {
        match tip(
            &prism,
            &ask(end_of(&prism), Some(length), ROUND_FULLNESS),
            &tolerance(1.0),
            None,
        ) {
            Ok(made) => {
                assert_eq!(check(&made.body, &tolerance(1.0)), [], "{length}");
            }
            Err(error) => {
                let _ = error.to_string();
            }
        }
    }
}

/// The largest angle between a tip face's outward normal and its side's
/// at the rim, sampled (17 places a face), measured here independently
/// of the kernel's own check.
fn rim_angle(made: &Tipped) -> f64 {
    let users = craft3d_topo::edge_faces(&made.body);
    let mut worst = 0.0_f64;
    for (face, _) in tip_faces(made) {
        let surface = nurbs(&made.body, face);
        let rim = made
            .body
            .loop_(made.body.face(face).unwrap().loops[0])
            .unwrap()
            .coedges[0]
            .edge;
        let side = users[rim.0 as usize]
            .iter()
            .copied()
            .find(|f| *f != face)
            .unwrap();
        let side_face = made.body.face(side).unwrap();
        let ((u0, u1), (v0, _)) = surface.domain();
        for i in 0..=16 {
            let u = u0 + (u1 - u0) * f64::from(i) / 16.0;
            let (p, du, dv) = surface.derivatives(u, v0);
            let n1 = du.cross(dv).normalized().unwrap();
            let (a, b) = side_face.surface.project(p).unwrap();
            let n2 = side_face.surface.normal(a, b).unwrap();
            let n2 = if side_face.reversed { -n2 } else { n2 };
            worst = worst.max(n1.angle_to(n2));
        }
    }
    worst
}

/// Very short tips, at a few tolerances, over sides whose steps differ
/// in length (the general form; at such lengths the sides read as level
/// within the tolerance): built only tangent to the sides within the
/// angular tolerance and measurable, refused otherwise, never built
/// creased.
#[test]
fn a_very_short_tip_is_tangent_and_measurable_or_refused() {
    let frame = frames(1.0)[1];
    let leaning = spline_frustum(&frame, 1.0, &ellipse(3.0, 2.0), DEPTH, 1.0, 0.2).unwrap();
    let tol = tolerance(1.0);
    let mut refused = 0;
    let mut built = 0;
    for length in [2e-7, 5e-7, 2e-6, 1e-5, 1e-4, 1e-3, 1e-2] {
        match tip(
            &leaning,
            &ask(end_of(&leaning), Some(length), ROUND_FULLNESS),
            &tol,
            None,
        ) {
            Ok(made) => {
                built += 1;
                let angle = rim_angle(&made);
                assert!(angle <= 1e-9, "{length}: built creased by {angle:e} rad");
                assert!(
                    mass_properties(&made.body).is_ok(),
                    "{length}: not measurable"
                );
            }
            Err(error) => {
                refused += 1;
                let text = error.to_string();
                println!("{length:e}: refused: {text}");
                assert!(!text.contains("internal error"), "{length}: {text}");
            }
        }
    }
    // The longest is built; shorter ones may be refused (a tip shorter
    // than about a thousandth of its end cannot be measured yet).
    assert!(built >= 1, "built {built}, refused {refused}");
    assert!(
        tip(
            &leaning,
            &ask(end_of(&leaning), Some(1e-2), ROUND_FULLNESS),
            &tol,
            None
        )
        .is_ok()
    );
}

/// Across the fullness and the length, every tip built can be measured:
/// one that cannot is refused, saying what to change.
#[test]
fn every_tip_built_can_be_measured() {
    let frame = Frame::WORLD;
    let r = 5.0;
    let rod = spline_prism(&frame, 1.0, &ellipse(r, 2.0), DEPTH * r).unwrap();
    let mut built = 0;
    for fullness in [0.01, 0.05, 0.2, ROUND_FULLNESS, 0.7, 0.9, 0.99] {
        for share in [0.01, 0.1, 1.0] {
            match tip(
                &rod,
                &ask(end_of(&rod), Some(share * r), fullness),
                &tolerance(1.0),
                None,
            ) {
                Ok(made) => {
                    built += 1;
                    let measured = mass_properties(&made.body);
                    assert!(measured.is_ok(), "{fullness} {share}: {measured:?}");
                }
                Err(error) => {
                    let text = error.to_string();
                    assert!(
                        !text.contains("internal error"),
                        "{fullness} {share}: {text}"
                    );
                    if text.contains("cannot be measured") {
                        assert!(
                            text.contains("longer") && text.contains("fullness"),
                            "{text}"
                        );
                    }
                }
            }
        }
    }
    assert!(built >= 12, "{built}");
}

/// What is said where the point's place is to blame: an end no point sees
/// whole (a U), an end whose centre of area does not see it whole (an L),
/// a place given that does not.
#[test]
fn where_no_point_or_not_this_one_sees_the_end_whole_the_refusal_says_so() {
    let frame = Frame::WORLD;
    let tol = tolerance(1.0);
    let u = spline_prism(
        &frame,
        1.0,
        &polygon(&[
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 3.0],
            [2.0, 3.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 3.0],
            [0.0, 3.0],
        ]),
        DEPTH,
    )
    .unwrap();
    for length in [None, Some(0.5), Some(2.0)] {
        let error = tip(&u, &ask(end_of(&u), length, ROUND_FULLNESS), &tol, None).unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("no point over this end sees its whole outline"),
            "{length:?}: {text}"
        );
        assert!(
            matches!(error, KernelError::Tip { blame: "face", .. }),
            "{error:?}"
        );
    }
    let l = spline_prism(
        &frame,
        1.0,
        &polygon(&[
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 1.0],
            [1.0, 1.0],
            [1.0, 3.0],
            [0.0, 3.0],
        ]),
        DEPTH,
    )
    .unwrap();
    for length in [None, Some(1.0)] {
        let text = tip(&l, &ask(end_of(&l), length, ROUND_FULLNESS), &tol, None)
            .unwrap_err()
            .to_string();
        assert!(
            text.contains("centre of area") && text.contains("give at"),
            "{length:?}: {text}"
        );
        assert!(!text.contains("close in at once"), "{text}");
    }
    // In the corner square the whole L is seen: built.
    assert!(
        tip(
            &l,
            &TipAsk {
                at: Some([0.5, 0.5]),
                ..ask(end_of(&l), Some(1.0), ROUND_FULLNESS)
            },
            &tol,
            None
        )
        .is_ok()
    );
    // A place given outside the end is the place's fault.
    let error = tip(
        &l,
        &TipAsk {
            at: Some([3.0, 2.5]),
            ..ask(end_of(&l), Some(1.0), ROUND_FULLNESS)
        },
        &tol,
        None,
    )
    .unwrap_err();
    assert!(
        matches!(error, KernelError::Tip { blame: "at", .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("place given"), "{error}");
}
