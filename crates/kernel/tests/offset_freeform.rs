//! Offset face and shell of freeform faces against closed forms.
//!
//! The faces are rational NURBS surfaces that are pieces of a sphere and
//! of a torus *exactly* (`rational_common`): the offset of such a face is
//! the piece of the sphere or the torus with its radius moved, which the
//! kernel cannot know (it sees a spline and builds a spline within a
//! share of the tolerance of the exact offset), so the volume and the
//! area of the result have closed forms. Three scales, tilted, with the
//! twins' circular edges as circles and as rational arcs; a fold refused
//! by its name.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;
mod rational_common;

use boolean_common::{SCALES, ball, boxed, mass, ring, solve, tolerance};
use craft3d_geom::{Curve3, Line, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::BooleanOp::Intersect;
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::shell::{Skin, shell};
use craft3d_math::{Transform, Vec3};
use craft3d_topo::{Body, FaceId, check};
use rational_common::rationalized;
use std::f64::consts::PI;

/// A tilt and a shift that leave nothing along an axis.
fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// A quarter of the zone of a ball of radius 2 between the heights −1 and
/// 1 (x and y at least 0).
fn ball_zone(k: f64) -> Body {
    let ball = ball(k, [0.0, 0.0, 0.0], 2.0);
    let slab = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 1.0]);
    solve(&ball, &slab, Intersect, k).body
}

/// A quarter of the slice of a ring (major 3, minor 1) between the
/// heights −0.5 and 0.5: an outer and an inner face on the torus.
fn ring_slice(k: f64) -> Body {
    let ring = ring(k, [0.0, 0.0, 0.0], 3.0, 1.0);
    let slab = boxed(k, [0.0, 0.0, -0.5], [5.0, 5.0, 0.5]);
    solve(&ring, &slab, Intersect, k).body
}

/// The twin of `body` at scale `k` with its curved faces rational (the
/// variants of `rational_common`), one variant per scale so that the
/// gate stays short: at 1e-3 and 1e6 a patch just under the face with its
/// circles, tilted; at 1 a wider patch with its edges as rational arcs
/// (which are found again as splines), upright.
fn twins(body: &Body, k: f64) -> Vec<(String, Body, Transform)> {
    let (pad, edges, tilted) = if k == 1.0 {
        (0.2, true, false)
    } else {
        (0.0, false, true)
    };
    let (mut twin, _) = rationalized(body, pad, edges);
    let frame = if tilted { tilt(k) } else { Transform::IDENTITY };
    if tilted {
        twin = twin.transformed(&frame).unwrap();
    }
    vec![(
        format!("k {k}, pad {pad}, arcs {edges}, tilted {tilted}"),
        twin,
        frame,
    )]
}

/// The distances tried at scale `k` (out and in): out at the smallest
/// scale, in at the largest, both at 1.
fn moves(k: f64, [out, inward]: [f64; 2]) -> Vec<f64> {
    if k < 1.0 {
        vec![out]
    } else if k > 1.0 {
        vec![inward]
    } else {
        vec![out, inward]
    }
}

fn freeform(body: &Body) -> Vec<FaceId> {
    (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| matches!(body.face(*f).unwrap().surface, Surface::Nurbs(_)))
        .collect()
}

/// The freeform face whose middle lies farthest from the axis of the
/// body's (tilted) frame, or nearest.
fn by_reach(body: &Body, frame: &Transform, outer: bool) -> FaceId {
    let axis_point = frame.point(Vec3::ZERO);
    let axis = frame.vector(Vec3::new(0.0, 0.0, 1.0));
    let reach = |f: FaceId| {
        let s = &body.face(f).unwrap().surface;
        let ((u0, u1), (v0, v1)) = s.domain();
        let p = s.eval(0.5 * (u0 + u1), 0.5 * (v0 + v1)) - axis_point;
        (p - axis * p.dot(axis)).length()
    };
    let mut faces = freeform(body);
    faces.sort_by(|a, b| reach(*a).total_cmp(&reach(*b)));
    if outer {
        *faces.last().unwrap()
    } else {
        faces[0]
    }
}

/// `actual` within `slack` of `expected`.
#[track_caller]
fn within(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected} (off by {:e}, slack {slack:e})",
        actual - expected
    );
}

/// Gauss–Legendre on 48 points over `[a, b]` (the integrands are analytic
/// there: far below 1e-14 relative).
fn integral(a: f64, b: f64, f: impl Fn(f64) -> f64) -> f64 {
    // The nodes by Newton's iteration on the Legendre polynomial.
    const N: usize = 48;
    let mut sum = 0.0;
    for i in 0..N {
        let mut x = (PI * (i as f64 + 0.75) / (N as f64 + 0.5)).cos();
        let mut dp = 0.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for n in 2..=N {
                let p2 = ((2 * n - 1) as f64 * x * p1 - (n - 1) as f64 * p0) / n as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = N as f64 * (x * p1 - p0) / (x * x - 1.0);
            let step = p1 / dp;
            x -= step;
            if step.abs() < 1e-16 {
                break;
            }
        }
        let w = 2.0 / ((1.0 - x * x) * dp * dp);
        sum += w * f(0.5 * (a + b) + 0.5 * (b - a) * x);
    }
    0.5 * (b - a) * sum
}

/// `∫ √(ρ² − x²) dx` from 0 to `x`.
fn under_circle(rho: f64, x: f64) -> f64 {
    0.5 * (x * (rho * rho - x * x).max(0.0).sqrt() + rho * rho * (x / rho).asin())
}

/// The quarter ball zone of radius `rho`, heights −1 to 1: volume and
/// area (at scale 1).
fn zone(rho: f64) -> (f64, f64) {
    let volume = PI / 4.0 * (2.0 * rho * rho - 2.0 / 3.0);
    let side = (rho * rho - 1.0).sqrt() + rho * rho * (1.0 / rho).asin();
    let area = PI * rho + PI * (rho * rho - 1.0) / 2.0 + 2.0 * side;
    (volume, area)
}

#[test]
fn the_sphere_face_of_a_rational_ball_zone_moves_out_and_in() {
    for k in SCALES {
        let tol = tolerance(k);
        for (what, twin, _) in twins(&ball_zone(k), k) {
            let face = freeform(&twin)[0];
            for d in moves(k, [0.4, -0.5]) {
                let what = format!("{what}, d {d}");
                let made = offset_faces(
                    &twin,
                    &[FaceOffset {
                        face,
                        distance: d * k,
                    }],
                    &tol,
                )
                .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{what}");
                assert_eq!(made.body.faces.len(), twin.faces.len(), "{what}");
                let m = mass(&made.body);
                let (volume, area) = zone(2.0 + d);
                // The face is within a quarter of the tolerance of the
                // sphere: the volume within that times the area.
                within(
                    m.volume,
                    volume * k.powi(3),
                    area * k * k * 0.25 * tol.linear,
                    &what,
                );
                within(m.area, area * k * k, 1e-7 * area * k * k, &what);
            }
        }
    }
}

/// The section of the quarter ring slice whose outer face moved by `d`
/// (the inner keeps its tube of 1): `θ ∫∫ ρ dA` over it, with θ a quarter
/// turn (Pappus).
fn ring_volume(d: f64) -> f64 {
    let outer = 1.0 + d;
    let right = integral(-0.5, 0.5, |z| {
        let reach = 3.0 + (outer * outer - z * z).sqrt();
        0.5 * (reach * reach - 9.0)
    });
    let left = integral(-0.5, 0.5, |z| {
        let reach = 3.0 - (1.0 - z * z).sqrt();
        0.5 * (9.0 - reach * reach)
    });
    PI / 2.0 * (right + left)
}

#[test]
fn the_outer_face_of_a_rational_ring_slice_moves_out_and_in() {
    for k in SCALES {
        let tol = tolerance(k);
        let slice = ring_slice(k);
        for (what, twin, frame) in twins(&slice, k) {
            let face = by_reach(&twin, &frame, true);
            for d in moves(k, [0.3, -0.4]) {
                let what = format!("{what}, d {d}");
                let made = offset_faces(
                    &twin,
                    &[FaceOffset {
                        face,
                        distance: d * k,
                    }],
                    &tol,
                )
                .unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{what}");
                let m = mass(&made.body);
                let volume = ring_volume(d) * k.powi(3);
                within(m.volume, volume, m.area * 0.25 * tol.linear, &what);
            }
        }
    }
}

/// The hollow of the quarter ball zone made hollow by `t`: the region
/// x, y ≥ t, |z| ≤ 1 − t, |p| ≤ 2 − t.
fn zone_hollow(t: f64) -> f64 {
    let rho_ball = 2.0 - t;
    integral(-(1.0 - t), 1.0 - t, |z| {
        let rho = (rho_ball * rho_ball - z * z).sqrt();
        let reach = (rho * rho - t * t).sqrt();
        under_circle(rho, reach) - under_circle(rho, t) - t * (reach - t)
    })
}

#[test]
fn a_rational_ball_zone_made_hollow_has_the_volume_of_the_exact_shell() {
    for k in SCALES {
        let tol = tolerance(k);
        let t = 0.3;
        let (outer, _) = zone(2.0);
        let expected = (outer - zone_hollow(t)) * k.powi(3);
        // The analytic body first: the oracle is right.
        let plain = shell(&ball_zone(k), t * k, &[], &tol).unwrap();
        let m = mass(&plain.body);
        within(
            m.volume,
            expected,
            1e-9 * expected,
            &format!("analytic at {k}"),
        );
        for (what, twin, _) in twins(&ball_zone(k), k) {
            let made = shell(&twin, t * k, &[], &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(check(&made.body, &tol), [], "{what}");
            assert_eq!(made.body.shells.len(), 2, "{what}");
            assert_eq!(
                made.faces.iter().filter(|(s, _)| *s == Skin::Inner).count(),
                twin.faces.len(),
                "{what}"
            );
            let m = mass(&made.body);
            within(m.volume, expected, m.area * 0.25 * tol.linear, &what);
        }
    }
}

/// `body` (with planes square to the axes, lines, and freeform faces and
/// edges only) stretched by `sx` along x: an affine map, under which a
/// rational surface or curve is the one with its control points mapped.
fn stretched(body: &Body, sx: f64) -> Body {
    let map = |p: Vec3| Vec3::new(p.x * sx, p.y, p.z);
    let mut out = body.clone();
    for vertex in &mut out.vertices {
        vertex.point = map(vertex.point);
    }
    for face in &mut out.faces {
        face.surface = match &face.surface {
            Surface::Nurbs(n) => {
                let (du, dv) = n.degrees();
                let (cu, cv) = n.counts();
                Surface::Nurbs(
                    NurbsSurface::new(
                        du,
                        dv,
                        n.knots_u().to_vec(),
                        n.knots_v().to_vec(),
                        cu,
                        cv,
                        n.control_points().iter().map(|p| map(*p)).collect(),
                        n.weights().to_vec(),
                    )
                    .unwrap(),
                )
            }
            Surface::Plane(plane) => {
                let frame = plane.frame();
                assert!(
                    [frame.z().x.abs(), frame.z().y.abs(), frame.z().z.abs()]
                        .iter()
                        .any(|c| (c - 1.0).abs() < 1e-12),
                    "a plane square to an axis"
                );
                Surface::Plane(Plane::new(frame.at(map(frame.origin())).unwrap()))
            }
            other => panic!("not stretched: {}", other.kind()),
        };
    }
    for edge in &mut out.edges {
        edge.curve = match &edge.curve {
            Curve3::Nurbs(n) => Curve3::Nurbs(
                NurbsCurve::new(
                    n.degree(),
                    n.knots().to_vec(),
                    n.control_points().iter().map(|p| map(*p)).collect(),
                    n.weights().to_vec(),
                )
                .unwrap(),
            ),
            Curve3::Line(_) => {
                let (a, b) = (
                    map(edge.curve.eval(edge.range.0)),
                    map(edge.curve.eval(edge.range.1)),
                );
                edge.range = (0.0, a.distance(b));
                Curve3::Line(Line::through(a, b).unwrap())
            }
            other => panic!("not stretched: {}", other.kind()),
        };
    }
    out
}

#[test]
fn an_offset_that_would_fold_a_freeform_face_is_refused_with_the_largest_that_fits() {
    let k = 1.0;
    let tol = tolerance(k);
    // A quarter of an elliptic rod, semi-axes 2 (along x) and 1, 3 long:
    // the quarter rod of radius 1 with rational faces and edges,
    // stretched. Its round face bends most at the end of its long axis,
    // with a radius of curvature of b² / a = 0.5 there (and a² / b = 4 at
    // the other end): offset inward by more, it folds there, while the
    // rod is still a unit thick.
    let rod = boolean_common::rod(k, [0.0, 0.0, 0.0], 1.0, 3.0);
    let corner = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 4.0]);
    let quarter = solve(&rod, &corner, Intersect, k).body;
    let (twin, _) = rationalized(&quarter, 0.0, true);
    let elliptic = stretched(&twin, 2.0);
    assert_eq!(check(&elliptic, &tol), []);
    within(
        mass(&elliptic).volume,
        PI * 2.0 * 3.0 / 4.0,
        1e-9,
        "the elliptic rod",
    );
    let face = freeform(&elliptic)[0];
    match offset_faces(
        &elliptic,
        &[FaceOffset {
            face,
            distance: -0.6,
        }],
        &tol,
    ) {
        Err(KernelError::FaceOpTooFar {
            reason,
            largest,
            faces,
            ..
        }) => {
            assert!(reason.contains("fold over"), "{reason}");
            assert_eq!(faces, vec![face]);
            // Short of the smallest radius of curvature, by what the
            // bounds cannot prove next to the fold and the tolerance the
            // halving stops at (0.4999998 is found).
            assert!(largest < 0.5 && largest > 0.4999, "{largest}");
        }
        other => panic!("{other:?}"),
    }
    // Short of it, built.
    let made = offset_faces(
        &elliptic,
        &[FaceOffset {
            face,
            distance: -0.3,
        }],
        &tol,
    )
    .unwrap();
    assert_eq!(check(&made.body, &tol), []);
    // Well short of a radius of curvature a rod is thinner: the quarter
    // rod of radius 2 (rational, with its circles) moved in by 1.5 is the
    // quarter of the rod of radius 0.5.
    let rod = boolean_common::rod(k, [0.0, 0.0, 0.0], 2.0, 3.0);
    let quarter = solve(&rod, &corner, Intersect, k).body;
    let (twin, _) = rationalized(&quarter, 0.0, false);
    let face = freeform(&twin)[0];
    let made = offset_faces(
        &twin,
        &[FaceOffset {
            face,
            distance: -1.5,
        }],
        &tol,
    )
    .unwrap();
    let m = mass(&made.body);
    within(
        m.volume,
        PI * 0.25 * 3.0 / 4.0,
        m.area * 0.25 * tol.linear,
        "thinner rod",
    );
}

/// A prism 4 wide and 3 long whose roof is two flat faces meeting at a
/// ridge `rise` above their eaves at height 1 (the ridge 2 from each
/// eave: their normals part by about `rise`), both roof faces given as
/// bilinear NURBS patches whose borders are their edges. The prism with
/// flat faces, the one with the patches, and the faces of its roof.
fn ridged(rise: f64, tol: &craft3d_math::Tolerance) -> (Body, Body, [FaceId; 2]) {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    let points = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.0],
        [2.0, 1.0 + rise],
        [0.0, 1.0],
    ];
    let curves: Vec<SketchCurve> = (0..points.len())
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: points[i],
                end: points[(i + 1) % points.len()],
            },
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).unwrap();
    let plain = extrude(&profiles, &craft3d_math::Frame::WORLD, 0.0, 3.0, tol)
        .unwrap()
        .body;
    let mut body = plain.clone();
    let mut roof = Vec::new();
    for (index, face) in plain.faces.iter().enumerate() {
        let Surface::Plane(plane) = &face.surface else {
            continue;
        };
        let normal = plane.frame().z() * if face.reversed { -1.0 } else { 1.0 };
        if normal.y < 0.5 {
            continue;
        }
        // The corners in loop order, each coedge from its start.
        let corners: Vec<Vec3> = plain
            .loop_(face.loops[0])
            .unwrap()
            .coedges
            .iter()
            .map(|c| {
                let e = plain.edge(c.edge).unwrap();
                plain
                    .vertex(if c.reversed { e.end } else { e.start })
                    .unwrap()
                    .point
            })
            .collect();
        assert_eq!(corners.len(), 4);
        let patch = NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![corners[0], corners[1], corners[3], corners[2]],
            vec![1.0; 4],
        )
        .unwrap();
        let surface = Surface::Nurbs(patch);
        let own = surface.normal(0.5, 0.5).unwrap();
        body.faces[index].surface = surface;
        body.faces[index].reversed = own.dot(normal) < 0.0;
        roof.push(FaceId(index as u32));
    }
    assert_eq!(roof.len(), 2);
    assert_eq!(check(&body, tol), []);
    (plain, body, [roof[0], roof[1]])
}

#[test]
fn a_crease_too_shallow_to_be_a_crease_is_built_within_the_tolerance_or_refused_as_too_far() {
    // A ridge whose faces' normals part by 2e-5 (between what fitting
    // leaves of a tangency and a crease a design means): offset along the
    // normal they share, each face moves by its distance times 1e-5;
    // within a quarter of the tolerance (1e-6) that is a distance of
    // 0.025 at most.
    let tol = tolerance(1.0);
    let (plain, body, roof) = ridged(2e-5, &tol);
    let moves = |d: f64| roof.map(|face| FaceOffset { face, distance: d });
    match offset_faces(&body, &moves(0.2), &tol) {
        Err(KernelError::FaceOpTooFar {
            reason, largest, ..
        }) => {
            assert!(reason.contains("tangent here only"), "{reason}");
            assert!(largest > 0.02 && largest <= 0.025, "{largest}");
        }
        other => panic!("{other:?}"),
    }
    // Short of it, built, and within the tolerance of the same prism with
    // its roof flat faces (offset in closed form, the crease kept).
    let made = offset_faces(&body, &moves(0.02), &tol).unwrap();
    assert_eq!(check(&made.body, &tol), []);
    let exact = offset_faces(&plain, &moves(0.02), &tol).unwrap();
    let (m, e) = (mass(&made.body), mass(&exact.body));
    within(m.volume, e.volume, e.area * tol.linear, "the ridged prism");
}

/// A face operation is held to a work bound (`FACE_OP_WORK`): under a
/// bound too small for it, it is refused by that name, not as too far and
/// not with a body, also when the bound ends its search for the largest
/// size that fits half way; without one the same offset is made, well
/// within the operation's own bound.
#[test]
fn a_freeform_offset_that_needs_more_work_than_its_bound_is_refused_by_name() {
    use craft3d_geom::allowance;
    use craft3d_kernel::offset_face::FACE_OP_WORK;
    let k = 1.0;
    let tol = tolerance(k);
    let spent =
        |got: Result<craft3d_kernel::offset_face::Offset, KernelError>, what: &str| match got {
            Err(error @ KernelError::FaceOp { .. }) => {
                assert!(
                    error.to_string().contains("needs more work"),
                    "{what}: {error}"
                );
            }
            Err(other) => panic!("{what}: {other}"),
            Ok(_) => panic!("{what}: built"),
        };
    let (_, twin, _) = twins(&ball_zone(k), k).remove(0);
    let face = freeform(&twin)[0];
    let offsets = [FaceOffset {
        face,
        distance: 0.4 * k,
    }];
    let before = allowance::spent();
    offset_faces(&twin, &offsets, &tol).unwrap();
    let whole = allowance::spent() - before;
    // Well within the bound (1.9 million when this was written).
    assert!(whole > 0 && whole < FACE_OP_WORK / 4, "{whole}");
    spent(
        allowance::within(0, || offset_faces(&twin, &offsets, &tol)),
        "no work at all",
    );
    // The elliptic rod of the fold above: its search halves twenty times
    // on proofs of regularity; half its work ends it half way.
    let rod = boolean_common::rod(k, [0.0, 0.0, 0.0], 1.0, 3.0);
    let corner = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 4.0]);
    let quarter = solve(&rod, &corner, Intersect, k).body;
    let (twin, _) = rationalized(&quarter, 0.0, true);
    let elliptic = stretched(&twin, 2.0);
    let offsets = [FaceOffset {
        face: freeform(&elliptic)[0],
        distance: -0.6,
    }];
    let before = allowance::spent();
    let far = offset_faces(&elliptic, &offsets, &tol);
    let whole = allowance::spent() - before;
    assert!(
        matches!(far, Err(KernelError::FaceOpTooFar { .. })),
        "{far:?}"
    );
    spent(
        allowance::within(whole / 2, || offset_faces(&elliptic, &offsets, &tol)),
        "half the work of the search",
    );
    // And a shell the same way.
    match allowance::within(0, || shell(&elliptic, 0.1 * k, &[], &tol)) {
        Err(error) => assert!(error.to_string().contains("needs more work"), "{error}"),
        Ok(_) => panic!("a shell under a bound of nothing was built"),
    }
}

/// What an operation counts toward its bound is a function of its inputs:
/// the same offset and shell on bodies built afresh each time count the
/// same, the first time (the trees, projections and intersections of
/// their surfaces not made yet), the second (all of them on the shelves)
/// and after another boolean ran in between. A cache that is hit charges
/// what its miss cost.
#[test]
fn the_same_operation_counts_the_same_whatever_ran_before() {
    use craft3d_geom::allowance;
    let k = 1.0;
    let tol = tolerance(k);
    let counts = |what: &str| {
        let (_, twin, _) = twins(&ball_zone(k), k).remove(0);
        let face = freeform(&twin)[0];
        let offsets = [FaceOffset {
            face,
            distance: 0.4 * k,
        }];
        let (made, offset) = allowance::measured(|| offset_faces(&twin, &offsets, &tol));
        made.unwrap_or_else(|e| panic!("{what}: {e}"));
        // The boolean of the shell is under no bound of its own: counted
        // under one here, so that it remembers for itself too.
        let (made, shelled) = allowance::within(u64::MAX, || {
            allowance::measured(|| shell(&twin, 0.1 * k, &[], &tol))
        });
        assert!(made.is_ok(), "{what}");
        (offset, shelled)
    };
    let first = counts("first");
    let second = counts("second");
    // Another boolean between: a ring and a block, freeform twins.
    let ring = ring_slice(k);
    let (_, other, _) = twins(&ring, k).remove(0);
    let _ = solve(
        &other,
        &boxed(k, [0.0, 0.0, -0.2], [2.0, 2.0, 0.2]),
        Intersect,
        k,
    );
    let third = counts("after another boolean");
    eprintln!("offset and shell: {first:?} {second:?} {third:?}");
    assert_eq!(first, second);
    assert_eq!(first, third);
}
