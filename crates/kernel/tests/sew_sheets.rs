//! Two sheets sewn along the boundary edges they share, without
//! intersecting them (`boolean::sew_sheets`, 2026-10-31): the operation
//! the class-A fuel tank needs to join a side held in curvature to its
//! crown, whose surfaces touch along the whole edge (a union refuses to
//! intersect them there). Held to closed-form areas and volumes at 1e-3,
//! 1 and 1e6: a plate and the B-spline trough `z = y²` tangent to it
//! along their common edge (area `3 + 3 L(1)`, `L` the length of the
//! parabola), the six faces of the unit cube sewn one at a time into a
//! solid (volume 1, area 6); the refusals by their reasons; and the same
//! result twice, bit for bit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanResult, EdgeSource, Operand, sew_sheets};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles, open_chain};
use craft3d_math::{Frame, Portable, Vec3};
use craft3d_topo::{Body, BodyKind, ShellId, boundary_loops, check};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The rectangle from `lo` to `hi` (times `k`) in the plane `frame`,
/// facing along its normal.
fn rectangle(lo: [f64; 2], hi: [f64; 2], frame: &Frame, k: f64) -> Body {
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(|[x, y]| [x * k, y * k]);
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
        .collect();
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let sheet = fill(&profiles, frame, &tol).unwrap().body;
    assert_eq!(check(&sheet, &tol), []);
    sheet
}

/// The plate `z = 0`, `x` in `[0, 3]`, `y` in `[−1, 0]` (times `k`),
/// facing up or down.
fn plate(k: f64, up: bool) -> Body {
    if up {
        rectangle([0.0, -1.0], [3.0, 0.0], &Frame::WORLD, k)
    } else {
        // Facing −z, its sketch x along +x: sketch y is (−z) × x = −y.
        let frame = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
        rectangle([0.0, 0.0], [3.0, 1.0], &frame, k)
    }
}

/// The ruled B-spline sheet `z = y²` for `y` in `[0, 1]` and `x` in
/// `[0, 3]` (times `k`): a loft of one quadratic Bézier section whose
/// control points are evenly spaced in y, so its height is `y²` exactly.
/// Along `y = 0` it is tangent to the plate (G1, not G2: its curvature
/// across is 2, the plate's 0).
fn half_trough(k: f64) -> Body {
    let tol = tolerance(k);
    let section = |x: f64| {
        let curve = SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 2,
                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                control: vec![[0.0, 0.0], [0.5 * k, 0.0], [k, k]],
            }),
        };
        Section {
            plane: Frame::new(Vec3::new(x * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
            outline: open_chain(&[curve], &tol).unwrap(),
            start: None,
        }
    };
    let options = Options {
        ruled: true,
        closed: false,
    };
    let sheet = loft(&[section(0.0), section(3.0)], &options, &tol)
        .unwrap()
        .body;
    assert_eq!(sheet.kind, BodyKind::Sheet);
    assert!(
        sheet
            .faces
            .iter()
            .all(|f| matches!(f.surface, Surface::Nurbs(_) | Surface::Bezier(_)))
    );
    assert_eq!(check(&sheet, &tol), []);
    sheet
}

/// The length of `z = y²` from `0` to `y`.
fn parabola(y: f64) -> f64 {
    0.5 * y * (1.0 + 4.0 * y * y).sqrt() + 0.25 * (2.0 * y).pasinh()
}

/// The unit square with a corner at `origin` (times `k`), facing `normal`.
fn square(origin: Vec3, normal: Vec3, u: Vec3, k: f64) -> Body {
    rectangle(
        [0.0, 0.0],
        [1.0, 1.0],
        &Frame::new(origin * k, normal, u).unwrap(),
        k,
    )
}

/// The six faces of the unit cube (times `k`), facing out, or in, in an
/// order where each shares an edge with one before it (bottom, the four
/// sides round, top).
fn cube_faces(k: f64, out: bool) -> Vec<Body> {
    let faces = [
        (Vec3::new(0.0, 1.0, 0.0), -Vec3::Z, Vec3::X),
        (Vec3::new(1.0, 0.0, 0.0), -Vec3::Y, Vec3::Z),
        (Vec3::new(1.0, 0.0, 0.0), Vec3::X, Vec3::Y),
        (Vec3::new(0.0, 1.0, 0.0), Vec3::Y, Vec3::Z),
        (Vec3::new(0.0, 0.0, 1.0), -Vec3::X, Vec3::Y),
        (Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::X),
    ];
    faces
        .iter()
        .map(|&(corner, normal, u)| {
            if out {
                square(corner, normal, u, k)
            } else {
                square(corner, -normal, normal.cross(u), k)
            }
        })
        .collect()
}

fn refused(result: Result<BooleanResult, KernelError>, reason: &str, what: &str) {
    match result {
        Err(KernelError::Unsupported { reason: r, .. }) => {
            assert!(
                r.contains(reason),
                "{what}: refused as `{r}`, not `{reason}`"
            );
        }
        Err(other) => panic!("{what}: {other}, not refused as `{reason}`"),
        Ok(r) => panic!("{what}: sewn ({} faces), not refused", r.body.faces.len()),
    }
}

/// A plate and a trough tangent to it along their common edge: sewn into
/// one sheet of two faces and seven edges, one shell with one boundary
/// loop, of area `3 + 3 L(1)` (times `k²`), its faces those of the plate
/// and then of the trough, the shared edge the plate's. Facing opposite
/// ways across the edge they are not sewn.
#[test]
fn a_plate_and_a_trough_tangent_along_their_edge_are_sewn() {
    for k in SCALES {
        let tol = tolerance(k);
        let trough = half_trough(k);
        let (mut sewn, mut apart) = (None, None);
        for up in [true, false] {
            match sew_sheets(&plate(k, up), &trough, &tol) {
                Ok(result) => sewn = Some(result),
                Err(e) => apart = Some(e),
            }
        }
        let what = format!("at {k}");
        let sewn = sewn.unwrap_or_else(|| panic!("{what}: neither way round was sewn"));
        let refusal = apart.unwrap_or_else(|| panic!("{what}: both ways round were sewn"));
        refused(Err(refusal), "share no boundary edge", &what);
        let body = &sewn.body;
        assert_eq!(body.kind, BodyKind::Sheet, "{what}");
        assert_eq!(check(body, &tol), [], "{what}");
        assert_eq!(
            (body.faces.len(), body.edges.len(), body.shells.len()),
            (2, 7, 1),
            "{what}"
        );
        assert_eq!(boundary_loops(body, ShellId(0)), Some(1), "{what}");
        let area = mass(body).area;
        assert_close(area, (3.0 + 3.0 * parabola(1.0)) * k * k, &what);
        assert_eq!(sewn.faces[0].operand, Operand::A, "{what}");
        assert_eq!(sewn.faces[1].operand, Operand::B, "{what}");
        assert_eq!(sewn.edges.len(), 7, "{what}");
        // The same twice, bit for bit.
        for up_side in [true, false] {
            if let Ok(again) = sew_sheets(&plate(k, up_side), &trough, &tol) {
                assert_eq!(format!("{:?}", again), format!("{sewn:?}"), "{what}");
            }
        }
    }
}

/// The six faces of the unit cube sewn one at a time close into a solid
/// (volume 1, area 6, one shell, six faces); facing in, the last step is
/// refused as facing inward.
#[test]
fn six_squares_sewn_one_at_a_time_close_into_a_solid() {
    for k in SCALES {
        let what = format!("cube at {k}");
        let tol = tolerance(k);
        let faces = cube_faces(k, true);
        let mut body = faces[0].clone();
        for (i, face) in faces.iter().enumerate().skip(1) {
            body = sew_sheets(&body, face, &tol)
                .unwrap_or_else(|e| panic!("{what}: step {i}: {e}"))
                .body;
            let kind = if i == 5 {
                BodyKind::Solid
            } else {
                BodyKind::Sheet
            };
            assert_eq!(body.kind, kind, "{what}: step {i}");
        }
        assert_eq!(check(&body, &tol), [], "{what}");
        let m = mass(&body);
        assert_close(m.volume, k * k * k, &what);
        assert_close(m.area, 6.0 * k * k, &what);
        assert_eq!((body.shells.len(), body.faces.len()), (1, 6), "{what}");
        let inward = cube_faces(k, false);
        let mut open = inward[0].clone();
        for face in &inward[1..5] {
            open = sew_sheets(&open, face, &tol).unwrap().body;
        }
        refused(sew_sheets(&open, &inward[5], &tol), "faces inward", &what);
    }
}

/// What is not sewn, by its reason: a solid, sheets apart, sheets whose
/// edges are three tolerances apart (beyond the tolerance, so no pair).
#[test]
fn what_cannot_be_sewn_is_refused() {
    for k in SCALES {
        let what = format!("at {k}");
        let tol = tolerance(k);
        let block = boxed(k, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let square = rectangle([0.0, 0.0], [1.0, 1.0], &Frame::WORLD, k);
        refused(sew_sheets(&block, &square, &tol), "joins two sheets", &what);
        refused(sew_sheets(&square, &block, &tol), "joins two sheets", &what);
        let away = rectangle([5.0, 5.0], [6.0, 6.0], &Frame::WORLD, k);
        refused(
            sew_sheets(&square, &away, &tol),
            "share no boundary edge",
            &what,
        );
        let gap = 3.0 * tol.linear / k;
        let near = rectangle([1.0 + gap, 0.0], [2.0, 1.0], &Frame::WORLD, k);
        refused(
            sew_sheets(&square, &near, &tol),
            "share no boundary edge",
            &what,
        );
        // Two halves of a square, flush: one sheet of two faces.
        let next = rectangle([1.0, 0.0], [2.0, 1.0], &Frame::WORLD, k);
        let both = sew_sheets(&square, &next, &tol).unwrap();
        assert_eq!(both.body.faces.len(), 2, "{what}");
        assert_close(mass(&both.body).area, 2.0 * k * k, &what);
    }
}

/// One long edge against two short ones along it (a T-junction): the
/// plate 2 × 1 sewn to a sheet of two unit squares standing on its long
/// edge. `topo::sew` splits the long edge at the short ones' common end,
/// and every piece keeps the long edge's source (it had none, and was
/// refused as a bug, before 2026-10-31): one sheet of three faces, area
/// 4, one shell, one boundary loop, valid; and the other way round.
#[test]
fn a_long_edge_against_two_short_ones_is_split_and_sewn() {
    for k in SCALES {
        let what = format!("at {k}");
        let tol = tolerance(k);
        let plate = rectangle([0.0, 0.0], [2.0, 1.0], &Frame::WORLD, k);
        let left = rectangle([0.0, 1.0], [1.0, 2.0], &Frame::WORLD, k);
        let right = rectangle([1.0, 1.0], [2.0, 2.0], &Frame::WORLD, k);
        let pair = sew_sheets(&left, &right, &tol).unwrap().body;
        // The plate's long edge, along y = 1, and the pair's two short
        // edges along it.
        let along = |body: &Body| -> Vec<u32> {
            (0..body.edges.len())
                .filter(|&i| {
                    let e = &body.edges[i];
                    let (p, q) = (
                        body.vertices[e.start.0 as usize].point,
                        body.vertices[e.end.0 as usize].point,
                    );
                    (p.y - k).abs() <= tol.linear && (q.y - k).abs() <= tol.linear
                })
                .map(|i| i as u32)
                .collect()
        };
        let long = along(&plate);
        assert_eq!(long.len(), 1, "{what}");
        let short = along(&pair);
        assert_eq!(short.len(), 2, "{what}");
        for (first, (a, b)) in [(true, (&plate, &pair)), (false, (&pair, &plate))] {
            let sewn = sew_sheets(a, b, &tol).unwrap_or_else(|e| panic!("{what}: {e}"));
            // The same twice, bit for bit.
            let again = sew_sheets(a, b, &tol).unwrap();
            assert_eq!(format!("{again:?}"), format!("{sewn:?}"), "{what}");
            let kept = |operand: Operand, edge: u32| {
                sewn.edges
                    .iter()
                    .filter(|s| {
                        **s == EdgeSource::Kept {
                            operand,
                            edge: craft3d_topo::EdgeId(edge),
                        }
                    })
                    .count()
            };
            if first {
                // The long edge is the first operand's: both pieces of it
                // keep it as their source.
                assert_eq!(kept(Operand::A, long[0]), 2, "{what}: {:?}", sewn.edges);
            } else {
                // The short edges are the first operand's: each piece is
                // one of them, and none is the long edge's.
                for s in &short {
                    assert_eq!(kept(Operand::A, *s), 1, "{what}: {:?}", sewn.edges);
                }
                assert_eq!(kept(Operand::B, long[0]), 0, "{what}: {:?}", sewn.edges);
            }
            let body = &sewn.body;
            assert_eq!(check(body, &tol), [], "{what}");
            assert_eq!((body.faces.len(), body.shells.len()), (3, 1), "{what}");
            assert_eq!(boundary_loops(body, ShellId(0)), Some(1), "{what}");
            assert_close(mass(body).area, 4.0 * k * k, &what);
            assert_eq!(sewn.edges.len(), body.edges.len(), "{what}");
        }
    }
}
