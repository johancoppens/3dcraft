//! A boolean of two sheets (decided by the developer, 2026-10-22). A
//! sheet cut by a sheet keeps, for subtract, the part of the target in
//! front of the tool's normal and, for intersect, the part behind: held
//! to closed-form areas, to `area(front) + area(behind) = area(target)`,
//! to the side the kept part lies on and to the target's orientation
//! kept, for a plane target cut by a plane and by a cylinder and for a
//! freeform (B-spline) target cut by a plane and by a cylinder. A union
//! sews two sheets along boundary edges they share: two halves are the
//! whole (area, one boundary loop). Every refusal by its reason. At
//! 1e-3, 1 and 1e6, as built and turned off-axis, and bit for bit the
//! same when run twice.

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
use craft3d_kernel::boolean::{BooleanOp, BooleanResult, Operand, boolean};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles, open_chain};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, BodyKind, Coedge, ShellId, boundary_loops, check, mass_bound};
use std::collections::BTreeMap;
use std::f64::consts::PI;

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

/// The plane `z = 0` facing up.
fn ground() -> Frame {
    Frame::WORLD
}

/// The plane `x = c` (times `k`), facing `+x` (or `−x` when `back`), its
/// sketch axes the world's y and z (z and y when `back`).
fn wall(c: f64, k: f64, back: bool) -> Frame {
    let origin = Vec3::new(c * k, 0.0, 0.0);
    if back {
        Frame::new(origin, -Vec3::X, Vec3::Z).unwrap()
    } else {
        Frame::new(origin, Vec3::X, Vec3::Y).unwrap()
    }
}

/// Faces of `body` as one sheet, sharing the edges they share there.
fn face_as_sheet(body: &Body, face: usize) -> Body {
    use craft3d_topo::VertexId;
    let mut out = Body::new(BodyKind::Sheet);
    let mut vertices: BTreeMap<u32, VertexId> = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let source = &body.faces[face];
    let mut loops = Vec::new();
    for &l in &source.loops {
        let mut coedges = Vec::new();
        for c in &body.loops[l.0 as usize].coedges {
            let edge = *edges.entry(c.edge.0).or_insert_with(|| {
                let e = &body.edges[c.edge.0 as usize];
                let mut vertex = |v: VertexId| {
                    *vertices
                        .entry(v.0)
                        .or_insert_with(|| out.add_vertex(body.vertices[v.0 as usize].point))
                };
                let (start, end) = (vertex(e.start), vertex(e.end));
                out.add_edge(e.curve.clone(), e.range, start, end)
            });
            coedges.push(Coedge {
                edge,
                reversed: c.reversed,
            });
        }
        loops.push(out.add_loop(coedges));
    }
    let made = out.add_face(source.surface.clone(), source.reversed, loops);
    out.add_shell(vec![made]);
    out
}

/// The side of a cylinder of radius `r` (times `k`) along `axis` through
/// `origin`, from `-reach` to `reach` along it: a tube, facing out.
fn tube(origin: Vec3, axis: Vec3, x_hint: Vec3, r: f64, reach: f64, k: f64) -> Body {
    let frame = Frame::new(origin * k - axis * (reach * k), axis, x_hint).unwrap();
    let rod = craft3d_kernel::primitives::cylinder(&frame, r * k, 2.0 * reach * k).unwrap();
    let side = rod
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let sheet = face_as_sheet(&rod, side);
    assert_eq!(check(&sheet, &tolerance(k)), []);
    sheet
}

/// The ruled B-spline sheet `z = y²` for `y` in `[−1, 1]` and `x` in
/// `[x0, x1]` (times `k`): a loft of one quadratic Bézier section, whose
/// height is `y²` exactly (its control points evenly spaced in y). It
/// faces `y × x = −z`.
fn trough(x0: f64, x1: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let section = |x: f64| {
        let curve = SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 2,
                knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                control: vec![[-k, k], [0.0, -k], [k, k]],
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
    let sheet = loft(&[section(x0), section(x1)], &options, &tol)
        .unwrap()
        .body;
    assert_eq!(sheet.kind, BodyKind::Sheet);
    assert!(
        sheet
            .faces
            .iter()
            .all(|f| matches!(f.surface, Surface::Nurbs(_) | Surface::Bezier(_))),
        "the trough is a B-spline face"
    );
    assert_eq!(check(&sheet, &tol), []);
    sheet
}

/// The length of `z = y²` from `0` to `y`.
fn parabola(y: f64) -> f64 {
    0.5 * y * (1.0 + 4.0 * y * y).sqrt() + 0.25 * (2.0 * y).asinh()
}

/// The welded mesh of a sheet: every directed side of a triangle at most
/// once (the sheet is one-sided and consistently oriented), and its area
/// that of the B-rep.
fn meshed(body: &Body, k: f64, what: &str) {
    let mesh = tessellate_body(
        body,
        &TessParams {
            chordal_tolerance: 1e-3 * k,
            ..TessParams::default()
        },
    )
    .unwrap()
    .weld()
    .unwrap();
    let mut sides: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for &[a, b, c] in &mesh.triangles {
        for side in [(a, b), (b, c), (c, a)] {
            *sides.entry(side).or_default() += 1;
        }
    }
    assert!(
        sides.values().all(|&n| n == 1),
        "{what}: a side of two triangles runs the same way"
    );
}

/// The boundary loops of every shell of a sheet.
fn loops_of(body: &Body) -> Vec<usize> {
    (0..body.shells.len())
        .map(|s| boundary_loops(body, ShellId(s as u32)).unwrap())
        .collect()
}

/// The target cut both ways by the tool: the part in front (subtract)
/// has area `front`, both are sheets, faces of the target only, facing
/// as the target's did, and the two add up to the target.
fn cut_both(target: &Body, tool: &Body, k: f64, front: f64, what: &str) -> [BooleanResult; 2] {
    let tol = tolerance(k);
    let whole = mass(target).area;
    let ahead = solve(target, tool, BooleanOp::Subtract, k);
    let behind = solve(target, tool, BooleanOp::Intersect, k);
    for (result, name) in [(&ahead, "in front"), (&behind, "behind")] {
        let what = format!("{what}: {name}");
        assert_eq!(result.body.kind, BodyKind::Sheet, "{what}");
        assert_eq!(check(&result.body, &tol), [], "{what}");
        assert!(
            result.faces.iter().all(|s| s.operand == Operand::A),
            "{what}: only the target's faces"
        );
        for (face, source) in result.body.faces.iter().zip(&result.faces) {
            let of = &target.faces[source.face.0 as usize];
            assert_eq!(face.surface, of.surface, "{what}: the target's surface");
            assert_eq!(face.reversed, of.reversed, "{what}: the target's side");
        }
        meshed(&result.body, k, &what);
    }
    // Held to 1e-9 of the area, the integrator's own bounds, and the
    // tolerance times the length of the cut (a curve on a freeform face
    // is fitted to the tolerance; no cut here is longer than 10).
    let measured = |body: &Body| {
        let (m, bound) = mass_bound(body).unwrap();
        (m.area, bound.area)
    };
    let ((a_front, b_front), (a_behind, b_behind)) =
        (measured(&ahead.body), measured(&behind.body));
    let fitted = 10.0 * k * tol.linear;
    let slack = 1e-9 * whole + b_front + b_behind + mass_bound(target).unwrap().1.area + fitted;
    assert!(
        (a_front - front * k * k).abs() <= slack,
        "{what}: in front {a_front} is not {}",
        front * k * k
    );
    assert!(
        (a_front + a_behind - whole).abs() <= slack,
        "{what}: {a_front} + {a_behind} is not {whole}"
    );
    [ahead, behind]
}

/// Every vertex of `body` on the side `sign` of the plane through `at`
/// with normal `normal` (or on it).
fn on_side(body: &Body, at: Vec3, normal: Vec3, sign: f64, k: f64, what: &str) {
    for v in &body.vertices {
        let d = (v.point - at).dot(normal) * sign;
        assert!(d >= -1e-6 * k, "{what}: a vertex {d} on the wrong side");
    }
}

fn turns() -> [Option<Transform>; 2] {
    [
        None,
        Some(Transform::rotation(Vec3::new(0.3, -0.7, 0.5), 0.9).unwrap()),
    ]
}

fn turned(body: Body, turn: &Option<Transform>) -> Body {
    match turn {
        Some(t) => body.transformed(t).unwrap(),
        None => body,
    }
}

fn turned_point(p: Vec3, turn: &Option<Transform>) -> Vec3 {
    match turn {
        Some(t) => t.point(p),
        None => p,
    }
}

fn turned_vector(v: Vec3, turn: &Option<Transform>) -> Vec3 {
    match turn {
        Some(t) => t.vector(v),
        None => v,
    }
}

#[test]
fn a_plane_sheet_cut_by_a_plane_and_by_a_tube() {
    for k in SCALES {
        for turn in turns() {
            let plate = turned(rectangle([-2.0, -1.5], [2.0, 1.5], &ground(), k), &turn);
            // A wall at x = 0.5 reaching across the plate, facing +x: in
            // front x > 0.5 (1.5 × 3), behind the rest (2.5 × 3).
            for back in [false, true] {
                let what = format!("wall at {k}, back {back}, turned {}", turn.is_some());
                let tool = turned(
                    rectangle([-5.0, -5.0], [5.0, 5.0], &wall(0.5, k, back), k),
                    &turn,
                );
                let front = if back { 7.5 } else { 4.5 };
                let [ahead, behind] = cut_both(&plate, &tool, k, front, &what);
                let at = turned_point(Vec3::new(0.5 * k, 0.0, 0.0), &turn);
                let normal = turned_vector(Vec3::X, &turn);
                let sign = if back { -1.0 } else { 1.0 };
                on_side(&ahead.body, at, normal, sign, k, &what);
                on_side(&behind.body, at, normal, -sign, k, &what);
                for result in [&ahead, &behind] {
                    assert_eq!(result.body.faces.len(), 1, "{what}");
                    assert_eq!(loops_of(&result.body), [1], "{what}");
                }
            }
            // A tube of radius 1 standing through it, facing out: in
            // front the plate outside the circle (12 − π), behind the
            // disc (π), the disc's rim a new edge on the tube.
            let what = format!("tube at {k}, turned {}", turn.is_some());
            let tool = turned(tube(Vec3::ZERO, Vec3::Z, Vec3::X, 1.0, 2.0, k), &turn);
            let [ahead, behind] = cut_both(&plate, &tool, k, 12.0 - PI, &what);
            assert_eq!(loops_of(&ahead.body), [2], "{what}: a plate with a hole");
            assert_eq!(loops_of(&behind.body), [1], "{what}: a disc");
        }
    }
}

/// A tube (a cylinder's side, closed around by its seam) cut by a plane
/// square to its axis: in front of a plane facing up the tube above it,
/// 2π · 1.5; and a plate cut by the tube's other way round, the tube as
/// the target of a plane through its axis: half on each side.
#[test]
fn a_tube_cut_by_a_plane() {
    for k in SCALES {
        for turn in turns() {
            let what = format!("tube at {k}, turned {}", turn.is_some());
            let pipe = turned(tube(Vec3::ZERO, Vec3::Z, Vec3::X, 1.0, 2.0, k), &turn);
            let level = Frame::WORLD.at(Vec3::new(0.0, 0.0, 0.5 * k)).unwrap();
            let lid = turned(rectangle([-3.0, -3.0], [3.0, 3.0], &level, k), &turn);
            let [ahead, behind] = cut_both(&pipe, &lid, k, 3.0 * PI, &what);
            assert_eq!(loops_of(&ahead.body), [2], "{what}");
            assert_eq!(loops_of(&behind.body), [2], "{what}");
            // Through its axis, a wall at x = 0: half the tube each way.
            let tool = turned(
                rectangle([-5.0, -5.0], [5.0, 5.0], &wall(0.0, k, false), k),
                &turn,
            );
            let [ahead, _] = cut_both(&pipe, &tool, k, 4.0 * PI, &format!("{what}: halved"));
            assert_eq!(loops_of(&ahead.body), [1], "{what}: halved");
        }
    }
}

/// A tool of two faces folded along an edge that passes through the
/// target (made by a union of two sheets): a wall at x = ½ for y < 0
/// facing +x and a wall at y = 0 for x > ½ facing −y. In front of both
/// is the corner x > ½, y < 0 of the plate (1.5 × 1.5); the two curves
/// the walls cut meet at the fold.
#[test]
fn a_sheet_cut_by_a_folded_sheet() {
    for k in SCALES {
        for turn in turns() {
            let what = format!("folded tool at {k}, turned {}", turn.is_some());
            let plate = turned(rectangle([-2.0, -1.5], [2.0, 1.5], &ground(), k), &turn);
            let first = rectangle([-5.0, -5.0], [0.0, 5.0], &wall(0.5, k, false), k);
            let across = Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap();
            let second = rectangle([0.5, -5.0], [5.0, 5.0], &across, k);
            let folded = solve(&first, &second, BooleanOp::Union, k).body;
            assert_eq!(folded.faces.len(), 2, "{what}");
            let folded = turned(folded, &turn);
            let [ahead, behind] = cut_both(&plate, &folded, k, 2.25, &what);
            assert_eq!(loops_of(&ahead.body), [1], "{what}");
            assert_eq!(loops_of(&behind.body), [1], "{what}");
        }
    }
}

#[test]
fn a_freeform_sheet_cut_by_a_plane_and_by_a_tube() {
    let arc = 2.0 * parabola(1.0);
    for k in SCALES {
        for turn in turns() {
            let sheet = turned(trough(0.0, 4.0, k), &turn);
            assert_close(mass(&sheet).area, 4.0 * arc * k * k, "the trough");
            // A wall at x = 1.5 facing +x: in front 2.5 lengths of the
            // parabola, behind 1.5.
            for back in [false, true] {
                let what = format!(
                    "trough by a wall at {k}, back {back}, turned {}",
                    turn.is_some()
                );
                let tool = turned(
                    rectangle([-5.0, -5.0], [5.0, 5.0], &wall(1.5, k, back), k),
                    &turn,
                );
                let front = if back { 1.5 * arc } else { 2.5 * arc };
                cut_both(&sheet, &tool, k, front, &what);
            }
            // A tube of radius ½ along x through the axis: it meets the
            // trough where y² + y⁴ = ¼; inside it (behind its normal) the
            // trough between ±y0, 4 long.
            let what = format!("trough by a tube at {k}, turned {}", turn.is_some());
            let y0 = ((-1.0 + 2.0_f64.sqrt()) / 2.0).sqrt();
            let inside = 4.0 * 2.0 * parabola(y0);
            let tool = turned(
                tube(Vec3::new(2.0, 0.0, 0.0), Vec3::X, Vec3::Y, 0.5, 4.0, k),
                &turn,
            );
            let [ahead, behind] = cut_both(&sheet, &tool, k, 4.0 * arc - inside, &what);
            // The tube runs past both ends: the two strips outside it are
            // apart, one part each.
            assert_eq!(loops_of(&ahead.body), [1, 1], "{what}: two strips");
            assert_eq!(loops_of(&behind.body), [1], "{what}");
        }
    }
}

#[test]
fn a_union_sews_two_sheets_along_a_shared_edge() {
    for k in SCALES {
        for turn in turns() {
            let t = format!("at {k}, turned {}", turn.is_some());
            // Two halves of a plate, facing the same way: the whole plate,
            // one face, one boundary loop of four edges.
            let left = turned(rectangle([-2.0, -1.5], [0.0, 1.5], &ground(), k), &turn);
            let right = turned(rectangle([0.0, -1.5], [2.0, 1.5], &ground(), k), &turn);
            let whole = solve(&left, &right, BooleanOp::Union, k);
            assert_eq!(whole.body.kind, BodyKind::Sheet);
            assert_close(mass(&whole.body).area, 12.0 * k * k, &format!("halves {t}"));
            assert_eq!(loops_of(&whole.body), [1], "halves {t}");
            assert_eq!(whole.body.faces.len(), 1, "halves {t}: merged");
            assert_eq!(whole.body.edges.len(), 4, "halves {t}: merged");
            meshed(&whole.body, k, &t);
            // A half that only shares part of an edge: an L of two plates.
            let short = turned(rectangle([0.0, -1.5], [2.0, 0.5], &ground(), k), &turn);
            let l = solve(&left, &short, BooleanOp::Union, k);
            assert_close(mass(&l.body).area, 10.0 * k * k, &format!("L {t}"));
            assert_eq!(loops_of(&l.body), [1], "L {t}");
            // Two plates at a right angle sharing an edge: a fold. Of
            // the two ways the wall can face, one continues the floor's
            // side and is sewn; the other is refused.
            let floor = turned(rectangle([0.0, -1.5], [2.0, 1.5], &ground(), k), &turn);
            let mut sewn = 0;
            for back in [false, true] {
                let up = turned(
                    rectangle([-1.5, 0.0], [1.5, 2.0], &wall(0.0, k, back), k),
                    &turn,
                );
                // (The wall's sketch axes are y and z, or z and y.)
                let up = if back {
                    turned(
                        rectangle([0.0, -1.5], [2.0, 1.5], &wall(0.0, k, back), k),
                        &turn,
                    )
                } else {
                    up
                };
                match boolean(&floor, &up, BooleanOp::Union, &tolerance(k)) {
                    Ok(_) => {
                        let fold = solve(&floor, &up, BooleanOp::Union, k);
                        assert_close(mass(&fold.body).area, 12.0 * k * k, &format!("fold {t}"));
                        assert_eq!(fold.body.faces.len(), 2, "fold {t}");
                        assert_eq!(loops_of(&fold.body), [1], "fold {t}");
                        meshed(&fold.body, k, &t);
                        sewn += 1;
                    }
                    Err(KernelError::Unsupported { reason, .. }) => {
                        assert!(reason.contains("opposite ways"), "fold {t}: {reason}")
                    }
                    Err(e) => panic!("fold {t}: {e}"),
                }
            }
            assert_eq!(sewn, 1, "fold {t}: one way sews");
            // Two halves of the B-spline trough, sewn along the spline
            // they share: the whole trough, one boundary loop.
            let near = turned(trough(0.0, 1.5, k), &turn);
            let far = turned(trough(1.5, 4.0, k), &turn);
            let trough_whole = solve(&near, &far, BooleanOp::Union, k);
            let arc = 2.0 * parabola(1.0);
            assert_close(
                mass(&trough_whole.body).area,
                4.0 * arc * k * k,
                &format!("trough {t}"),
            );
            assert_eq!(loops_of(&trough_whole.body), [1], "trough {t}");
            meshed(&trough_whole.body, k, &t);
            // Two sheets apart: one body of two shells.
            let away = turned(rectangle([5.0, 5.0], [6.0, 6.0], &ground(), k), &turn);
            let two = solve(&left, &away, BooleanOp::Union, k);
            assert_eq!(loops_of(&two.body), [1, 1], "apart {t}");
            assert_close(mass(&two.body).area, 7.0 * k * k, &format!("apart {t}"));
        }
    }
}

fn refused(result: Result<BooleanResult, KernelError>, wanted: &str, what: &str) {
    match result {
        Err(KernelError::Unsupported { reason, faces, .. }) => {
            assert!(reason.contains(wanted), "{what}: refused as `{reason}`");
            assert!(!faces.is_empty(), "{what}: the refusal names no face");
        }
        other => panic!(
            "{what}: expected a refusal ({wanted}), got {:?}",
            other.map(|r| r.body.faces.len())
        ),
    }
}

fn refused_unnamed(result: Result<BooleanResult, KernelError>, wanted: &str, what: &str) {
    match result {
        Err(KernelError::Unsupported { reason, .. }) => {
            assert!(reason.contains(wanted), "{what}: refused as `{reason}`");
        }
        other => panic!(
            "{what}: expected a refusal ({wanted}), got {:?}",
            other.map(|r| r.body.faces.len())
        ),
    }
}

#[test]
fn what_two_sheets_cannot_do_is_refused() {
    for k in SCALES {
        let tol = tolerance(k);
        let plate = rectangle([-2.0, -1.5], [2.0, 1.5], &ground(), k);
        let cuts = [BooleanOp::Subtract, BooleanOp::Intersect];
        // A wall that ends inside the plate (y up to 1 of 1.5).
        let short = rectangle([-5.0, -5.0], [1.0, 5.0], &wall(0.5, k, false), k);
        for op in cuts {
            refused(
                boolean(&plate, &short, op, &tol),
                "ends inside",
                "short wall",
            );
        }
        // A sheet on the plate's surface.
        let over = rectangle([-1.0, -1.0], [1.0, 1.0], &ground(), k);
        for op in cuts {
            refused(
                boolean(&plate, &over, op, &tol),
                "lies on the surface",
                "on the plate",
            );
        }
        // A wall that misses the plate, and one that only meets its edge.
        let away = rectangle([-5.0, -5.0], [5.0, 5.0], &wall(3.0, k, false), k);
        let edge = rectangle([-5.0, -5.0], [5.0, 5.0], &wall(2.0, k, false), k);
        for op in cuts {
            refused_unnamed(
                boolean(&plate, &away, op, &tol),
                "does not pass through",
                "away",
            );
            refused_unnamed(
                boolean(&plate, &edge, op, &tol),
                "does not pass through",
                "edge",
            );
        }
        // A ring cut once: a half-wall whose edge lies in the hole.
        let ring = {
            let curves = vec![
                line(1, [-2.0 * k, -2.0 * k], [2.0 * k, -2.0 * k]),
                line(2, [2.0 * k, -2.0 * k], [2.0 * k, 2.0 * k]),
                line(3, [2.0 * k, 2.0 * k], [-2.0 * k, 2.0 * k]),
                line(4, [-2.0 * k, 2.0 * k], [-2.0 * k, -2.0 * k]),
                SketchCurve {
                    id: 5,
                    curve: Curve2::Circle {
                        center: [0.0, 0.0],
                        radius: k,
                    },
                },
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            fill(&profiles, &ground(), &tol).unwrap().body
        };
        let half = rectangle([0.0, -5.0], [5.0, 5.0], &wall(0.0, k, false), k);
        for op in cuts {
            refused(
                boolean(&ring, &half, op, &tol),
                "both sides",
                "ring cut once",
            );
        }
        // A target of two parts, of which the tool crosses one.
        let pair = solve(
            &plate,
            &rectangle([5.0, 5.0], [6.0, 6.0], &ground(), k),
            BooleanOp::Union,
            k,
        )
        .body;
        let across = rectangle([-5.0, -5.0], [5.0, 5.0], &wall(0.5, k, false), k);
        for op in cuts {
            refused(
                boolean(&pair, &across, op, &tol),
                "does not cross",
                "two parts",
            );
        }
        // A union of sheets that cross, of one ending on the other, and
        // of sheets that overlap on one surface.
        refused(
            boolean(&plate, &across, BooleanOp::Union, &tol),
            "inside a face",
            "crossing union",
        );
        let standing = rectangle([-1.0, 0.0], [1.0, 2.0], &wall(0.5, k, false), k);
        refused(
            boolean(&plate, &standing, BooleanOp::Union, &tol),
            "inside a face",
            "standing on it",
        );
        refused(
            boolean(&plate, &over, BooleanOp::Union, &tol),
            "overlap",
            "overlap",
        );
        refused(
            boolean(&plate, &plate, BooleanOp::Union, &tol),
            "overlap",
            "the same sheet",
        );
        // Two halves facing opposite ways.
        let left = rectangle([-2.0, -1.5], [0.0, 1.5], &ground(), k);
        let flipped = rectangle(
            [-1.5, 0.0],
            [1.5, 2.0],
            &Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::Y).unwrap(),
            k,
        );
        refused(
            boolean(&left, &flipped, BooleanOp::Union, &tol),
            "opposite ways",
            "halves facing apart",
        );
    }
}

#[test]
fn a_cut_of_two_sheets_is_the_same_every_time() {
    let k = 1.0;
    let sheet = trough(0.0, 4.0, k);
    let tool = tube(Vec3::new(2.0, 0.0, 0.0), Vec3::X, Vec3::Y, 0.5, 4.0, k);
    let first = solve(&sheet, &tool, BooleanOp::Subtract, k);
    let second = solve(&sheet, &tool, BooleanOp::Subtract, k);
    assert_eq!(format!("{:?}", first.body), format!("{:?}", second.body));
    assert_eq!(first.faces, second.faces);
    assert_eq!(first.edges, second.edges);
    let halves = (trough(0.0, 1.5, k), trough(1.5, 4.0, k));
    let one = solve(&halves.0, &halves.1, BooleanOp::Union, k);
    let two = solve(&halves.0, &halves.1, BooleanOp::Union, k);
    assert_eq!(format!("{:?}", one.body), format!("{:?}", two.body));
}

/// The square `[0, 1]²` (times `k`) of the plane through `origin` (times
/// `k`) facing `normal`, its sketch x along `u` (and y along normal × u).
fn square(origin: Vec3, normal: Vec3, u: Vec3, k: f64) -> Body {
    rectangle(
        [0.0, 0.0],
        [1.0, 1.0],
        &Frame::new(origin * k, normal, u).unwrap(),
        k,
    )
}

/// The six faces of the unit cube (times `k`), facing out, or in.
fn cube_faces(k: f64, out: bool) -> Vec<Body> {
    // (corner, outward normal, u) with normal × u the face's other side.
    let faces = [
        (Vec3::new(0.0, 1.0, 0.0), -Vec3::Z, Vec3::X),
        (Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::X),
        (Vec3::new(0.0, 0.0, 1.0), -Vec3::X, Vec3::Y),
        (Vec3::new(1.0, 0.0, 0.0), Vec3::X, Vec3::Y),
        (Vec3::new(1.0, 0.0, 0.0), -Vec3::Y, Vec3::Z),
        (Vec3::new(0.0, 1.0, 0.0), Vec3::Y, Vec3::Z),
    ];
    faces
        .iter()
        .map(|&(corner, normal, u)| {
            if out {
                square(corner, normal, u, k)
            } else {
                // Facing in, its axes swapped: (−n) × (n × u) = u.
                square(corner, -normal, normal.cross(u), k)
            }
        })
        .collect()
}

/// A union of sheets that closes (six squares sewn one at a time into a
/// cube): facing out it is a solid, the unit cube (volume 1, area 6,
/// one closed shell, the names of its faces those of the six sheets);
/// facing in it is refused at the last step, as is a union that closes
/// one shell and leaves another open.
#[test]
fn a_union_that_closes_the_sheet_is_a_solid() {
    for k in SCALES {
        for turn in turns() {
            let what = format!("cube at {k}, turned {}", turn.is_some());
            let faces: Vec<Body> = cube_faces(k, true)
                .into_iter()
                .map(|f| turned(f, &turn))
                .collect();
            let mut sewn = faces[0].clone();
            for (i, face) in faces.iter().enumerate().skip(1) {
                let result = solve(&sewn, face, BooleanOp::Union, k);
                let kind = if i == 5 {
                    BodyKind::Solid
                } else {
                    BodyKind::Sheet
                };
                assert_eq!(result.body.kind, kind, "{what}: step {i}");
                sewn = result.body;
            }
            assert_eq!(check(&sewn, &tolerance(k)), [], "{what}");
            let m = mass(&sewn);
            assert_close(m.volume, k * k * k, &format!("{what}: volume"));
            assert_close(m.area, 6.0 * k * k, &format!("{what}: area"));
            assert_eq!(sewn.shells.len(), 1, "{what}");
            assert_eq!(sewn.faces.len(), 6, "{what}");
            let mesh = tessellate_body(
                &sewn,
                &TessParams {
                    chordal_tolerance: 1e-3 * k,
                    ..TessParams::default()
                },
            )
            .unwrap()
            .weld()
            .unwrap();
            assert!(mesh.is_closed(), "{what}: the mesh is closed");
            // The cube is a solid like any other: a block through it.
            let block = turned(boxed(k, [0.5, 0.5, 0.5], [2.0, 2.0, 2.0]), &turn);
            let less = solve(&sewn, &block, BooleanOp::Subtract, k);
            assert_close(mass(&less.body).volume, 0.875 * k * k * k, &what);
            // Facing in: the last union is refused.
            let inward: Vec<Body> = cube_faces(k, false)
                .into_iter()
                .map(|f| turned(f, &turn))
                .collect();
            let mut open = inward[0].clone();
            for face in &inward[1..5] {
                open = solve(&open, face, BooleanOp::Union, k).body;
            }
            refused_unnamed(
                boolean(&open, &inward[5], BooleanOp::Union, &tolerance(k)),
                "faces inward",
                &what,
            );
            // A closed shell and an open one: refused.
            let mut five = faces[0].clone();
            for face in &faces[1..5] {
                five = solve(&five, face, BooleanOp::Union, k).body;
            }
            let away = turned(rectangle([5.0, 5.0], [6.0, 6.0], &ground(), k), &turn);
            let apart = solve(&five, &away, BooleanOp::Union, k).body;
            refused_unnamed(
                boolean(&apart, &faces[5], BooleanOp::Union, &tolerance(k)),
                "closes some of the sheets",
                &what,
            );
        }
    }
}

/// A tool must reach through the target (decided 2026-10-22): a wall
/// standing on the plate (its bottom edge lying across it, or across a
/// part of it) is refused as standing on it; a tool that only touches the
/// target's boundary edge with its own (a sheet hanging from the plate's
/// edge, beside a wall that crosses the plate) is told nothing there and
/// the plate is cut by the wall alone; a tool folded so that one of its
/// faces ends inside the plate is refused as ending inside; and a tool
/// through the fold of a folded target with the whole target on one side
/// cuts nothing.
#[test]
fn a_tool_that_does_not_reach_through_the_target() {
    for k in SCALES {
        let tol = tolerance(k);
        let what = format!("at {k}");
        let plate = rectangle([-2.0, -1.5], [2.0, 1.5], &ground(), k);
        let cuts = [BooleanOp::Subtract, BooleanOp::Intersect];
        let standing = rectangle([-5.0, 0.0], [5.0, 5.0], &wall(0.5, k, false), k);
        let partly = rectangle([-1.0, 0.0], [1.0, 5.0], &wall(0.5, k, false), k);
        for op in cuts {
            refused(boolean(&plate, &standing, op, &tol), "stands on", &what);
            refused(boolean(&plate, &partly, op, &tol), "stands on", &what);
        }
        // Hanging from the plate's edge at x = 2, beside a wall at ½.
        let crossing = rectangle([-5.0, -5.0], [5.0, 5.0], &wall(0.5, k, false), k);
        let hanging = rectangle([-5.0, -5.0], [5.0, 0.0], &wall(2.0, k, false), k);
        let tool = solve(&crossing, &hanging, BooleanOp::Union, k).body;
        assert_eq!(tool.shells.len(), 2);
        cut_both(&plate, &tool, k, 4.5, &format!("{what}: hanging"));
        // A folded tool whose second face stops inside the plate.
        let first = rectangle([-5.0, -5.0], [0.0, 5.0], &wall(0.5, k, false), k);
        let across = Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap();
        let second = rectangle([0.5, -5.0], [1.0, 5.0], &across, k);
        let folded = solve(&first, &second, BooleanOp::Union, k).body;
        for op in cuts {
            refused(boolean(&plate, &folded, op, &tol), "ends inside", &what);
        }
        // Through the fold of an L, the whole L on one side.
        let l = folded_target(k);
        let beside = rectangle(
            [-5.0, -5.0],
            [5.0, 5.0],
            &Frame::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0), Vec3::Y).unwrap(),
            k,
        );
        for op in cuts {
            refused_unnamed(
                boolean(&l, &beside, op, &tol),
                "does not pass through",
                &what,
            );
        }
    }
}

/// An L of two plates of 2 × 3 (times `k`) sharing the edge x = z = 0: a
/// floor facing up and a wall facing +x.
fn folded_target(k: f64) -> Body {
    let floor = rectangle([0.0, -1.5], [2.0, 1.5], &ground(), k);
    let wall_ = rectangle([-1.5, 0.0], [1.5, 2.0], &wall(0.0, k, false), k);
    let l = solve(&floor, &wall_, BooleanOp::Union, k).body;
    assert_eq!(l.faces.len(), 2);
    l
}

/// A plate of 4 × 4 with a round hole of radius 1 cut across the hole by
/// a wall through its middle: halves of (16 − π) / 2. An L cut through
/// its fold by the plane x = z (the floor in front, 6; the wall behind,
/// 6) and crossed by x + z = ½ (in front 4.5 of the floor and 4.5 of
/// the wall). At three scales, as built and turned.
#[test]
fn a_holed_plate_and_a_folded_target_cut_by_a_sheet() {
    for k in SCALES {
        for turn in turns() {
            let what = format!("at {k}, turned {}", turn.is_some());
            let tol = tolerance(k);
            let curves = vec![
                line(1, [-2.0 * k, -2.0 * k], [2.0 * k, -2.0 * k]),
                line(2, [2.0 * k, -2.0 * k], [2.0 * k, 2.0 * k]),
                line(3, [2.0 * k, 2.0 * k], [-2.0 * k, 2.0 * k]),
                line(4, [-2.0 * k, 2.0 * k], [-2.0 * k, -2.0 * k]),
                SketchCurve {
                    id: 5,
                    curve: Curve2::Circle {
                        center: [0.0, 0.0],
                        radius: k,
                    },
                },
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let holed = turned(fill(&profiles, &ground(), &tol).unwrap().body, &turn);
            let middle = turned(
                rectangle([-5.0, -5.0], [5.0, 5.0], &wall(0.0, k, false), k),
                &turn,
            );
            let [ahead, behind] = cut_both(
                &holed,
                &middle,
                k,
                (16.0 - PI) / 2.0,
                &format!("holed {what}"),
            );
            assert_eq!(loops_of(&ahead.body), [1], "holed {what}");
            assert_eq!(loops_of(&behind.body), [1], "holed {what}");
            let l = turned(folded_target(k), &turn);
            let through = turned(
                rectangle(
                    [-5.0, -5.0],
                    [5.0, 5.0],
                    &Frame::new(Vec3::ZERO, Vec3::new(1.0, 0.0, -1.0), Vec3::Y).unwrap(),
                    k,
                ),
                &turn,
            );
            let [floor, wall_] = cut_both(&l, &through, k, 6.0, &format!("fold {what}"));
            assert_eq!(floor.body.faces.len(), 1, "fold {what}");
            assert_eq!(wall_.body.faces.len(), 1, "fold {what}");
            let slant = turned(
                rectangle(
                    [-5.0, -5.0],
                    [5.0, 5.0],
                    &Frame::new(
                        Vec3::new(0.25 * k, 0.0, 0.25 * k),
                        Vec3::new(1.0, 0.0, 1.0),
                        Vec3::Y,
                    )
                    .unwrap(),
                    k,
                ),
                &turn,
            );
            let [ahead, behind] = cut_both(&l, &slant, k, 9.0, &format!("slant {what}"));
            // In front: a strip of the floor and one of the wall, apart.
            assert_eq!(loops_of(&ahead.body), [1, 1], "slant {what}");
            assert_eq!(loops_of(&behind.body), [1], "slant {what}");
        }
    }
}
