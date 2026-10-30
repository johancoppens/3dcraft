//! Oracles for the torsion of a section: the circle and the annulus
//! (`J = π (R⁴ − r⁴) / 2`, the warping function zero), rectangles against
//! the series solution (Timoshenko & Goodier, §109), a thin closed box
//! against Bredt's formula, a thin channel against the open thin-walled
//! forms (`J ≈ Σ b t³ / 3`, shear centre `e = 3 b² t_f / (6 b t_f + h t_w)`
//! from the web), symmetric sections with their shear centre at the
//! centroid, invariance under a rigid motion, three scales, a box cut
//! through a hole in its wall (an open section), and what must be refused
//! without a panic.

use super::*;
use crate::boolean::{BooleanOp, boolean};
use crate::extrude::extrude;
use crate::primitives::{block, cylinder};
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Transform};

fn tol(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

fn tess(p: &[P2], s: &[(usize, usize)], i: &[P2], c: &[f64]) -> Result<RegionMesh, String> {
    craft3d_tess::triangulate_region(p, s, i, c)
        .map(|m| RegionMesh {
            points: m.points,
            triangles: m.triangles,
            boundary: m.boundary,
            interior: m.interior,
        })
        .map_err(|e| e.to_string())
}

fn upright() -> CutPlane {
    CutPlane::new(Vec3::Z, Vec3::X).unwrap()
}

/// The torsion of `body` cut by the upright plane at height `z`.
fn twist(body: &Body, plane: &CutPlane, offset: f64, k: f64) -> Torsion {
    let started = std::time::Instant::now();
    let (_, t) = section_torsion(
        &[body],
        plane,
        offset,
        &tol(k),
        &TorsionOptions::default(),
        &tess,
    )
    .unwrap();
    let t = t.unwrap().unwrap();
    eprintln!(
        "J {:.9e} change {:.2e} shear ({:.6e}, {:.6e}) Cw {:.6e} converged {} elements {} rounds {} in {:.0} ms",
        t.constant,
        t.change,
        t.shear_offset.0,
        t.shear_offset.1,
        t.warping,
        t.converged,
        t.elements,
        t.rounds,
        started.elapsed().as_secs_f64() * 1e3
    );
    t
}

#[track_caller]
fn near(got: f64, want: f64, relative: f64, what: &str) {
    let off = (got - want).abs() / want.abs();
    assert!(
        off <= relative,
        "{what}: got {got}, want {want}, off by {off:.2e} (allowed {relative:.0e})"
    );
}

/// A polygon extruded from z = 0 to `h`, its corners scaled by `k`.
fn prism(corners: &[(f64, f64)], h: f64, k: f64) -> Body {
    let t = tol(k);
    let n = corners.len();
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % n]);
            SketchCurve {
                id: 1 + i as u64,
                curve: Curve2::Line {
                    start: [a.0 * k, a.1 * k],
                    end: [b.0 * k, b.1 * k],
                },
            }
        })
        .collect();
    let profiles = detect_profiles(&curves, &t).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, h * k, &t)
        .unwrap()
        .body
}

/// Dunavant's rule integrates every monomial up to degree 6 exactly over
/// the reference triangle: ∫ ξ^a η^b = a! b! / (a + b + 2)!.
#[test]
fn the_quadrature_is_exact_to_degree_six() {
    let fact = |n: u32| (1..=n).map(f64::from).product::<f64>();
    for a in 0..=6u32 {
        for b in 0..=(6 - a) {
            let sum: f64 = RULE
                .iter()
                .map(|(_, x, y, w)| 0.5 * w * x.powi(a as i32) * y.powi(b as i32))
                .sum();
            let want = fact(a) * fact(b) / fact(a + b + 2);
            assert!((sum - want).abs() < 1e-14, "{a} {b}: {sum} vs {want}");
        }
    }
}

/// The shape functions are one at their own node and zero at the others,
/// and sum to one with derivatives summing to zero.
#[test]
fn the_shape_functions_interpolate_their_nodes() {
    let nodes = [
        (0.0, 0.0),
        (1.0, 0.0),
        (0.0, 1.0),
        (0.5, 0.0),
        (0.5, 0.5),
        (0.0, 0.5),
    ];
    for (i, (xi, eta)) in nodes.iter().enumerate() {
        let (n, dx, de) = shape(*xi, *eta);
        for (j, v) in n.iter().enumerate() {
            let want = if i == j { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-15);
        }
        assert!(dx.iter().sum::<f64>().abs() < 1e-14);
        assert!(de.iter().sum::<f64>().abs() < 1e-14);
    }
}

/// A circle: J = π r⁴ / 2 (the warping function is zero) and the shear
/// centre at the centroid, at three scales.
#[test]
fn a_circle_twists_by_its_polar_moment() {
    for k in [1e-3, 1.0, 1e3] {
        let r = 2.0 * k;
        let body = cylinder(&Frame::WORLD, r, 3.0 * k).unwrap();
        let t = twist(&body, &upright(), 1.5 * k, k);
        assert!(t.converged);
        near(t.constant, PI * r.powi(4) / 2.0, 1e-8, "circle J");
        assert!(t.shear_offset.0.hypot(t.shear_offset.1) <= 1e-6 * r);
        assert!(t.warping.abs() <= 1e-6 * r.powi(6));
    }
}

/// An annulus: J = π (R⁴ − r⁴) / 2, the hole taking no constant.
#[test]
fn an_annulus_twists_by_its_polar_moment() {
    let k = 1.0;
    let (big, small, h) = (2.0, 1.4, 3.0);
    let outer = cylinder(&Frame::WORLD, big, h).unwrap();
    let low = Frame::new(Vec3::new(0.0, 0.0, -1.0), Vec3::Z, Vec3::X).unwrap();
    let inner = cylinder(&low, small, h + 2.0).unwrap();
    let tube = boolean(&outer, &inner, BooleanOp::Subtract, &tol(k))
        .unwrap()
        .body;
    let t = twist(&tube, &upright(), 1.5, k);
    assert!(t.converged);
    near(
        t.constant,
        PI * (big.powi(4) - small.powi(4)) / 2.0,
        1e-6,
        "annulus J",
    );
    assert!(t.shear_offset.0.hypot(t.shear_offset.1) <= 1e-6 * big);
}

/// The series solution for a rectangle of sides a ≥ b:
/// J = a b³ / 3 (1 − 192 b / (π⁵ a) Σ_{n odd} tanh(n π a / 2b) / n⁵).
fn rectangle(a: f64, b: f64) -> f64 {
    let mut sum = 0.0;
    let mut n = 1.0;
    while n < 200.0 {
        sum += (n * PI * a / (2.0 * b)).tanh() / n.powi(5);
        n += 2.0;
    }
    a * b.powi(3) / 3.0 * (1.0 - 192.0 * b / (PI.powi(5) * a) * sum)
}

/// Rectangles of several aspect ratios against the series, their shear
/// centre at the centroid.
#[test]
fn rectangles_match_the_series_solution() {
    for (a, b) in [(1.0, 1.0), (2.0, 1.0), (4.0, 1.0), (10.0, 1.0)] {
        let frame = Frame::new(Vec3::new(-0.3, 0.7, 0.0), Vec3::Z, Vec3::X).unwrap();
        let body = block(&frame, a, b, 1.0).unwrap();
        let t = twist(&body, &upright(), 0.5, 1.0);
        assert!(t.converged, "{a} x {b}");
        near(t.constant, rectangle(a, b), 3e-4, &format!("{a} x {b}"));
        assert!(t.shear_offset.0.hypot(t.shear_offset.1) <= 1e-5 * a);
    }
}

/// The welded beam of plan/repro-welded-beam-2: a box 100 × 150 with
/// walls of 3 (along z) and 4 (along y), cut across. Bredt's thin-walled
/// form 4 A_m² / ∮ ds / t is within its thin-wall error (t / b, a few
/// percent); the shear centre is at the centroid by symmetry.
#[test]
fn a_thin_closed_box_lands_near_bredt() {
    let k = 1.0;
    let outer = block(&Frame::WORLD, 500.0, 100.0, 150.0).unwrap();
    let inner_at = Frame::new(Vec3::new(-1.0, 3.0, 4.0), Vec3::Z, Vec3::X).unwrap();
    let inner = block(&inner_at, 502.0, 94.0, 142.0).unwrap();
    let boxed = boolean(&outer, &inner, BooleanOp::Subtract, &tol(k))
        .unwrap()
        .body;
    let across = CutPlane::new(Vec3::X, Vec3::Y).unwrap();
    let started = std::time::Instant::now();
    crate::below::below(&[&boxed], &across, 250.0, &tol(k)).unwrap();
    eprintln!(
        "the cut alone: {:.0} ms",
        started.elapsed().as_secs_f64() * 1e3
    );
    let t = twist(&boxed, &across, 250.0, k);
    assert!(t.converged);
    let (w, h): (f64, f64) = (97.0, 146.0);
    let bredt = 4.0 * (w * h).powi(2) / (2.0 * w / 4.0 + 2.0 * h / 3.0);
    near(t.constant, bredt, 0.03, "Bredt");
    assert!(t.shear_offset.0.hypot(t.shear_offset.1) <= 1e-4 * h);
}

/// A thin channel: web 100 between the flanges' middles, flanges 50 from
/// the web's middle, all 2 thick. J ≈ Σ b t³ / 3 and the shear centre
/// e = 3 b² / (6 b + h) behind the web, both within the thin-wall error.
fn channel(k: f64) -> Body {
    prism(
        &[
            (-1.0, -51.0),
            (50.0, -51.0),
            (50.0, -49.0),
            (1.0, -49.0),
            (1.0, 49.0),
            (50.0, 49.0),
            (50.0, 51.0),
            (-1.0, 51.0),
        ],
        10.0,
        k,
    )
}

#[test]
fn a_thin_channel_matches_the_open_thin_walled_forms() {
    let k = 1.0;
    let body = channel(k);
    let (found, t) = section_torsion(
        &[&body],
        &upright(),
        5.0,
        &tol(k),
        &TorsionOptions::default(),
        &tess,
    )
    .unwrap();
    let t = t.unwrap().unwrap();
    assert!(t.converged);
    let thin = (100.0 + 2.0 * 50.0) * 8.0 / 3.0;
    near(t.constant, thin, 0.08, "Σ b t³ / 3");
    // The shear centre from the web's middle (x = 0), behind it.
    let centroid = found.section.centroid.unwrap();
    let at = t.shear_centre;
    let e = 3.0 * 50.0f64.powi(2) / (6.0 * 50.0 + 100.0);
    near(-at.x, e, 0.05, "shear centre behind the web");
    assert!(at.y.abs() <= 1e-4, "on the axis of symmetry: {}", at.y);
    assert!((at.x - centroid.x - t.shear_offset.0).abs() < 1e-9);
    // The thin-walled warping constant of a channel,
    // Cw = t b³ h² (3b + 2h) / (12 (6b + h)), within the thin-wall error.
    let (b, h, th): (f64, f64, f64) = (50.0, 100.0, 2.0);
    let cw = th * b.powi(3) * h * h * (3.0 * b + 2.0 * h) / (12.0 * (6.0 * b + h));
    near(t.warping, cw, 0.03, "Cw");
}

/// Moved and turned together with its plane, a section has the same
/// torsion and the same shear centre relative to its centroid; at three
/// scales J goes with the fourth power of the scale.
#[test]
fn torsion_does_not_depend_on_where_the_section_lies_or_its_scale() {
    let reference = twist(&channel(1.0), &upright(), 5.0, 1.0);
    let turn = Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 1.1).unwrap();
    let shift = Transform::translation(Vec3::new(1000.0, -300.0, 77.0));
    let motion = turn.then(&shift);
    let moved = channel(1.0).transformed(&motion).unwrap();
    let plane = CutPlane::new(motion.vector(Vec3::Z), motion.vector(Vec3::X)).unwrap();
    let offset = motion.point(Vec3::new(0.0, 0.0, 5.0)).dot(plane.normal);
    let t = twist(&moved, &plane, offset, 1.0);
    near(t.constant, reference.constant, 2e-4, "moved J");
    assert!((t.shear_offset.0 - reference.shear_offset.0).abs() <= 1e-3);
    assert!((t.shear_offset.1 - reference.shear_offset.1).abs() <= 1e-3);
    for k in [1e-3, 1e3] {
        let t = twist(&channel(k), &upright(), 5.0 * k, k);
        near(t.constant, reference.constant * k.powi(4), 2e-4, "scaled J");
        near(
            t.shear_offset.0,
            reference.shear_offset.0 * k,
            1e-3,
            "scaled shear centre",
        );
    }
}

/// A box with a hole through its top wall, cut through the hole: the top
/// wall is cut, so the section is open and twists far less than the
/// closed one, between the open thin-walled form and Bredt's. Cut away
/// from the hole it is the closed box again.
#[test]
fn a_box_cut_through_a_hole_in_its_wall_is_open() {
    let k = 1.0;
    let outer = block(&Frame::WORLD, 200.0, 100.0, 150.0).unwrap();
    let inner_at = Frame::new(Vec3::new(-1.0, 3.0, 4.0), Vec3::Z, Vec3::X).unwrap();
    let inner = block(&inner_at, 202.0, 94.0, 142.0).unwrap();
    let boxed = boolean(&outer, &inner, BooleanOp::Subtract, &tol(k))
        .unwrap()
        .body;
    let drill_at = Frame::new(Vec3::new(100.0, 50.0, 140.0), Vec3::Z, Vec3::X).unwrap();
    let drill = cylinder(&drill_at, 10.0, 20.0).unwrap();
    let holed = boolean(&boxed, &drill, BooleanOp::Subtract, &tol(k))
        .unwrap()
        .body;
    let across = CutPlane::new(Vec3::X, Vec3::Y).unwrap();
    let open = twist(&holed, &across, 100.0, k);
    let closed = twist(&holed, &across, 30.0, k);
    let (w, h): (f64, f64) = (97.0, 146.0);
    let bredt = 4.0 * (w * h).powi(2) / (2.0 * w / 4.0 + 2.0 * h / 3.0);
    near(closed.constant, bredt, 0.03, "away from the hole");
    let thin = (2.0 * h * 27.0 + w * 64.0 + (w - 20.0) * 64.0) / 3.0;
    assert!(
        open.constant > 0.5 * thin && open.constant < 0.01 * bredt,
        "open {} between {} and {}",
        open.constant,
        thin,
        bredt
    );
    // The cut wall moves the shear centre off the centroid, away from
    // the slit (down, along −z, the section's y here).
    assert!(open.shear_offset.1 < -1.0, "{:?}", open.shear_offset);
}

/// What cannot be measured is said, never a panic: a plane that cuts
/// nothing, a tolerance or an element cap out of range, a mesher that
/// fails, a sliver of a section, and an element cap reached. A torsion
/// that cannot be solved for keeps the section's own measures.
#[test]
fn what_cannot_be_twisted_is_said() {
    let k = 1.0;
    let body = block(&Frame::WORLD, 4.0, 1.0, 1.0).unwrap();
    let run = |offset: f64, options: TorsionOptions| {
        section_torsion(&[&body], &upright(), offset, &tol(k), &options, &tess)
    };
    assert!(run(5.0, TorsionOptions::default()).unwrap().1.is_none());
    assert!(run(f64::NAN, TorsionOptions::default()).is_err());
    for (tolerance, max_elements) in [(0.0, 100), (f64::NAN, 100), (1e-4, 0), (1e-4, usize::MAX)] {
        let options = TorsionOptions {
            tolerance,
            max_elements,
        };
        assert!(run(0.5, options).is_err());
    }
    // Capped: either no mesh fits (the torsion's own error, the section
    // kept), or an answer that says it did not converge.
    for cap in [1, 50, 400] {
        let options = TorsionOptions {
            tolerance: 1e-9,
            max_elements: cap,
        };
        let (found, torsion) = run(0.5, options).unwrap();
        assert!((found.section.area - 4.0).abs() < 1e-9);
        match torsion {
            Some(Ok(t)) => assert!(!t.converged && t.elements <= cap),
            Some(Err(e)) => assert!(matches!(e, CutError::Torsion(_)), "{e}"),
            None => panic!("the section has area"),
        }
    }
    let failing =
        |_: &[P2], _: &[(usize, usize)], _: &[P2], _: &[f64]| -> Result<RegionMesh, String> {
            Err("no".into())
        };
    let empty =
        |_: &[P2], _: &[(usize, usize)], _: &[P2], _: &[f64]| -> Result<RegionMesh, String> {
            Ok(RegionMesh::default())
        };
    for mesher in [&failing as &Mesher<'_>, &empty] {
        let (found, torsion) = section_torsion(
            &[&body],
            &upright(),
            0.5,
            &tol(k),
            &TorsionOptions::default(),
            mesher,
        )
        .unwrap();
        assert!((found.section.area - 4.0).abs() < 1e-9);
        assert!(matches!(torsion, Some(Err(CutError::Torsion(_)))));
    }
    // A sliver, 1000 by 0.01: thin-strip J = b t³ / 3 (1 − 0.63 t / b).
    let sliver = block(&Frame::WORLD, 1000.0, 0.01, 1.0).unwrap();
    let options = TorsionOptions {
        tolerance: 1e-3,
        max_elements: 20_000,
    };
    let (_, torsion) =
        section_torsion(&[&sliver], &upright(), 0.5, &tol(k), &options, &tess).unwrap();
    match torsion {
        Some(Ok(t)) if t.converged => near(t.constant, 1000.0 * 1e-6 / 3.0, 0.01, "sliver"),
        Some(Ok(_)) => {}
        Some(Err(e)) => assert!(matches!(e, CutError::Torsion(_)), "{e}"),
        None => panic!("the sliver has area"),
    }
}

/// Two separate rectangles, mirror images about the plane x = 0: J is
/// the sum of the two (each by the series), and the shear centre lies on
/// the axis of symmetry, the same for meshes refined to two tolerances, within their tolerance
/// (each piece's free constant of ψ is taken off by its own mean, not by
/// where its held vertex happens to be).
#[test]
fn two_separate_pieces_twist_as_their_sum() {
    let k = 1.0;
    let left = block(
        &Frame::new(Vec3::new(-5.0, -0.5, 0.0), Vec3::Z, Vec3::X).unwrap(),
        3.0,
        1.0,
        1.0,
    )
    .unwrap();
    let right = block(
        &Frame::new(Vec3::new(2.0, -0.2, 0.0), Vec3::Z, Vec3::X).unwrap(),
        3.0,
        1.0,
        1.0,
    )
    .unwrap();
    // The mirror image of `left` about x = 0.
    let mirror = block(
        &Frame::new(Vec3::new(2.0, -0.5, 0.0), Vec3::Z, Vec3::X).unwrap(),
        3.0,
        1.0,
        1.0,
    )
    .unwrap();
    for tolerance in [1e-4, 1e-5] {
        let options = TorsionOptions {
            tolerance,
            ..TorsionOptions::default()
        };
        let (found, torsion) =
            section_torsion(&[&left, &mirror], &upright(), 0.5, &tol(k), &options, &tess).unwrap();
        let t = torsion.unwrap().unwrap();
        near(t.constant, 2.0 * rectangle(3.0, 1.0), 3e-4, "J₁ + J₂");
        let c = found.section.centroid.unwrap();
        assert!(c.x.abs() < 1e-9);
        // Within the solve's tolerance of the section's size, √A ≈ 2.4.
        assert!(t.shear_centre.x.abs() <= 1e-3, "{:?}", t.shear_centre);
        assert!(t.shear_centre.y.abs() <= 1e-3, "{:?}", t.shear_centre);
    }
    // `right` is `left` turned half a turn about (0, 0.15): the pair is
    // symmetric about that point, so the shear centre is there, the same
    // at both tolerances.
    let mut moved = Vec::new();
    for tolerance in [1e-4, 1e-5] {
        let options = TorsionOptions {
            tolerance,
            ..TorsionOptions::default()
        };
        let (_, torsion) =
            section_torsion(&[&left, &right], &upright(), 0.5, &tol(k), &options, &tess).unwrap();
        let t = torsion.unwrap().unwrap();
        near(
            t.constant,
            2.0 * rectangle(3.0, 1.0),
            3e-4,
            "J₁ + J₂ shifted",
        );
        moved.push(t.shear_centre);
    }
    let (a, b) = (moved[0], moved[1]);
    assert!((a - b).length() <= 1e-3, "{a:?} vs {b:?}");
    assert!(a.x.abs() <= 1e-3 && (a.y - 0.15).abs() <= 1e-3, "{a:?}");
}
