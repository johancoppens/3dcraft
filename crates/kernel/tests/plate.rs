//! Plates (docs/design/plates.md): faces given a thickness on a side, and
//! the joints between plates along a shared edge. Held to closed forms at
//! 1e-3, 1 and 1e6:
//!
//! - a flat plate's volume is its area times the thickness, whatever the
//!   side, with a hole as well (the area less the hole's);
//! - a plate on a cylinder at the middle is `A t` exactly (the Steiner
//!   formula `V = ∫(1 − 2Hs + Ks²) ds dA` with `K = 0` and levels
//!   `±t/2`), and to the side out or in `θ h ((R ± t)² − R²) / 2`;
//! - two plates in an L (a floor and a wall sharing an edge) with every
//!   joint: the volumes of the trapezoid, rectangle and lap cross-sections,
//!   that the two plates together fill exactly the L they make (their
//!   union's volume is the sum: no overlap; for mitre, over/under the
//!   corner square is filled, for touch it stays empty), that the mitre
//!   faces lie in the bisector plane `x = z` and are one surface;
//! - a bevel's volume (`A t + L t² tan α / 2`);
//! - refusals by reason.

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
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::fill;
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::plate::{Joint, Made, Neighbour, Plate, Side, plate};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles, open_chain};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check, mass_properties};
use std::collections::BTreeMap;
use std::f64::consts::PI;

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// The rectangle `lo..hi` (times `k`) filled in `frame`, with an optional
/// round hole.
fn sheet(lo: [f64; 2], hi: [f64; 2], hole: Option<([f64; 2], f64)>, frame: &Frame, k: f64) -> Body {
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let corners = [s(lo), s([hi[0], lo[1]]), s(hi), s([lo[0], hi[1]])];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, corners[i], corners[(i + 1) % 4]))
        .collect();
    if let Some((center, radius)) = hole {
        curves.push(SketchCurve {
            id: 9,
            curve: Curve2::Circle {
                center: s(center),
                radius: radius * k,
            },
        });
    }
    let tol = tolerance(k);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    fill(&profiles, frame, &tol).unwrap().body
}

fn all_faces(body: &Body) -> Vec<FaceId> {
    (0..body.faces.len() as u32).map(FaceId).collect()
}

fn volume(body: &Body) -> f64 {
    mass_properties(body).unwrap().volume
}

#[track_caller]
fn near(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual} is not {expected} (to {relative})"
    );
}

/// The edge of `body` whose two ends are `a` and `b` (either way), times
/// `k`.
fn edge_between(body: &Body, a: [f64; 3], b: [f64; 3], k: f64) -> EdgeId {
    let (a, b) = (
        Vec3::new(a[0] * k, a[1] * k, a[2] * k),
        Vec3::new(b[0] * k, b[1] * k, b[2] * k),
    );
    let tol = 1e-6 * k;
    let found = body.edges.iter().position(|e| {
        let (s, t) = (
            body.vertices[e.start.0 as usize].point,
            body.vertices[e.end.0 as usize].point,
        );
        (s.distance(a) <= tol && t.distance(b) <= tol)
            || (s.distance(b) <= tol && t.distance(a) <= tol)
    });
    EdgeId(found.expect("no such edge") as u32)
}

fn plated(body: &Body, t: f64, side: Side, joints: &BTreeMap<EdgeId, Joint<'_>>, k: f64) -> Plate {
    let tol = tolerance(k);
    let made = plate(body, &all_faces(body), t, side, joints, &tol)
        .unwrap_or_else(|e| panic!("plate at scale {k}: {e}"));
    assert_eq!(check(&made.body, &tol), [], "a plate is a valid solid");
    assert_eq!(made.faces.len(), made.body.faces.len());
    made
}

#[test]
fn a_flat_plate_is_its_area_times_the_thickness_on_every_side() {
    for k in SCALES {
        let frame = Frame::WORLD;
        let body = sheet([0.0, 0.0], [3.0, 2.0], None, &frame, k);
        for side in [Side::Out, Side::In, Side::Mid] {
            let made = plated(&body, 0.25 * k, side, &BTreeMap::new(), k);
            assert_eq!(
                made.body.faces.len(),
                6,
                "{side:?}: two skins and four rims"
            );
            assert!(
                made.body
                    .faces
                    .iter()
                    .all(|f| matches!(f.surface, Surface::Plane(_)))
            );
            near(
                volume(&made.body),
                6.0 * 0.25 * k * k * k,
                1e-12,
                "flat plate",
            );
            let m = mass_properties(&made.body).unwrap();
            let z = m.centroid.unwrap().z / k;
            let expected = match side {
                Side::Out => 0.125,
                Side::In => -0.125,
                Side::Mid => 0.0,
            };
            assert!(
                (z - expected).abs() <= 1e-9,
                "{side:?}: the centroid at {z}"
            );
            assert!(
                made.faces
                    .iter()
                    .filter(|m| matches!(m, Made::Rim(_)))
                    .count()
                    == 4,
                "a rim per edge"
            );
        }
    }
}

#[test]
fn a_flat_plate_with_a_hole_keeps_the_hole() {
    for k in SCALES {
        let body = sheet(
            [0.0, 0.0],
            [4.0, 3.0],
            Some(([2.0, 1.5], 0.75)),
            &Frame::WORLD,
            k,
        );
        let made = plated(&body, 0.2 * k, Side::Mid, &BTreeMap::new(), k);
        assert_eq!(
            made.body.faces.len(),
            7,
            "two skins, four rims and the hole's"
        );
        let area = 12.0 - PI * 0.75 * 0.75;
        // The hole's rim is a cubic fit of the cylinder within a quarter
        // of the tolerance.
        near(
            volume(&made.body),
            area * 0.2 * k * k * k,
            1e-6,
            "plate with a hole",
        );
    }
}

/// A sheet on the cylinder of radius `r` about z, over the angle `theta`
/// from x, from z = 0 to `h` (a ruled loft of two arcs), times `k`.
fn cylinder_sheet(r: f64, theta: f64, h: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let arc = SketchCurve {
        id: 1,
        curve: Curve2::Arc {
            center: [0.0, 0.0],
            start: [r * k, 0.0],
            end: [r * k * theta.cos(), r * k * theta.sin()],
        },
    };
    let section = |z: f64| Section {
        plane: Frame::new(Vec3::new(0.0, 0.0, z * k), Vec3::Z, Vec3::X).unwrap(),
        outline: open_chain(std::slice::from_ref(&arc), &tol).unwrap(),
        start: None,
    };
    let options = Options {
        ruled: true,
        closed: false,
    };
    loft(&[section(0.0), section(h)], &options, &tol)
        .unwrap()
        .body
}

#[test]
fn a_plate_on_a_cylinder_follows_the_steiner_formula() {
    for k in SCALES {
        let (r, theta, h, t) = (2.0, 1.2, 1.5, 0.3);
        let body = cylinder_sheet(r, theta, h, k);
        assert!(matches!(body.faces[0].surface, Surface::Cylinder(_)));
        let k3 = k * k * k;
        // In the middle: the area of the sheet times the thickness.
        let mid = plated(&body, t * k, Side::Mid, &BTreeMap::new(), k);
        near(volume(&mid.body), r * theta * h * t * k3, 1e-9, "middle");
        // Out and in: annular sectors, one outside the radius and one
        // inside (which is which follows the sheet's normal).
        let outer = theta * h * ((r + t) * (r + t) - r * r) / 2.0 * k3;
        let inner = theta * h * (r * r - (r - t) * (r - t)) / 2.0 * k3;
        let out = volume(&plated(&body, t * k, Side::Out, &BTreeMap::new(), k).body);
        let inn = volume(&plated(&body, t * k, Side::In, &BTreeMap::new(), k).body);
        let (a, b) = if (out - outer).abs() < (out - inner).abs() {
            (outer, inner)
        } else {
            (inner, outer)
        };
        near(out, a, 1e-9, "out");
        near(inn, b, 1e-9, "in");
    }
}

/// A channel: a line down, a quarter arc of radius `r` and a line across
/// (tangent to each other) in the plane x = 0, swept to x = `h` by a
/// ruled loft: a sheet of three faces (a plane, a cylinder, a plane) that
/// are tangent along the edges they share.
fn channel(r: f64, h: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let curves = vec![
        line(1, s([0.0, 1.0]), s([0.0, r])),
        SketchCurve {
            id: 2,
            curve: Curve2::Arc {
                center: s([r, r]),
                start: s([0.0, r]),
                end: s([r, 0.0]),
            },
        },
        line(3, s([r, 0.0]), s([1.2, 0.0])),
    ];
    let section = |x: f64| Section {
        plane: Frame::new(Vec3::new(x * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
        outline: open_chain(&curves, &tol).unwrap(),
        start: None,
    };
    let options = Options {
        ruled: true,
        closed: false,
    };
    loft(&[section(0.0), section(h)], &options, &tol)
        .unwrap()
        .body
}

/// The cylinder sheet of [`cylinder_sheet`] with its face's surface
/// given as the rational B-spline of the same cylinder (a quadratic arc
/// across, a line along), facing the same way: a freeform twin.
fn freeform_twin(r: f64, theta: f64, h: f64, k: f64) -> Body {
    let mut body = cylinder_sheet(r, theta, h, k);
    let (c, half) = ((theta / 2.0).cos(), theta / 2.0);
    let arc = [
        Vec3::new(r * k, 0.0, 0.0),
        Vec3::new(r * k * half.cos() / c, r * k * half.sin() / c, 0.0),
        Vec3::new(r * k * theta.cos(), r * k * theta.sin(), 0.0),
    ];
    let mut ctrl = Vec::new();
    let mut weights = Vec::new();
    for (p, w) in arc.iter().zip([1.0, c, 1.0]) {
        for z in [0.0, h * k] {
            ctrl.push(*p + Vec3::new(0.0, 0.0, z));
            weights.push(w);
        }
    }
    let nurbs = craft3d_geom::NurbsSurface::new(
        2,
        1,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        3,
        2,
        ctrl,
        weights,
    )
    .unwrap();
    let face = &mut body.faces[0];
    let probe = Vec3::new(r * k * half.cos(), r * k * half.sin(), 0.5 * h * k);
    let outward = |s: &Surface, reversed: bool| {
        let (u, v) = s.project(probe).unwrap();
        s.normal(u, v).unwrap() * if reversed { -1.0 } else { 1.0 }
    };
    let was = outward(&face.surface, face.reversed);
    let surface = Surface::Nurbs(nurbs);
    let reversed = outward(&surface, false).dot(was) < 0.0;
    face.surface = surface;
    face.reversed = reversed;
    assert_eq!(check(&body, &tolerance(k)), [], "the twin is a valid sheet");
    body
}

#[test]
fn a_freeform_plate_matches_its_analytic_twin() {
    let (r, theta, h, t) = (2.0, 1.2, 1.5, 0.3);
    for k in SCALES {
        let k3 = k * k * k;
        let analytic = cylinder_sheet(r, theta, h, k);
        let twin = freeform_twin(r, theta, h, k);
        for side in [Side::Mid, Side::Out, Side::In] {
            let exact = volume(&plated(&analytic, t * k, side, &BTreeMap::new(), k).body);
            let fitted = plated(&twin, t * k, side, &BTreeMap::new(), k);
            assert!(
                fitted
                    .body
                    .faces
                    .iter()
                    .any(|f| matches!(f.surface, Surface::Nurbs(_))),
                "the twin's skins are splines"
            );
            near(volume(&fitted.body), exact, 1e-6, &format!("{side:?} twin"));
            if side == Side::Mid {
                near(
                    volume(&fitted.body),
                    r * theta * h * t * k3,
                    1e-6,
                    "Steiner, freeform",
                );
            }
        }
    }
}

#[test]
fn a_plate_of_tangent_faces_is_one_solid() {
    let (r, h, t) = (0.3, 0.8, 0.05);
    for k in SCALES {
        let body = channel(r, h, k);
        assert_eq!(body.faces.len(), 3);
        let k3 = k * k * k;
        let flat = (1.0 - r) + (1.2 - r);
        let quarter = std::f64::consts::FRAC_PI_2;
        let mid = plated(&body, t * k, Side::Mid, &BTreeMap::new(), k);
        assert_eq!(mid.body.shells.len(), 1);
        assert_eq!(
            mid.body.faces.len(),
            6 + 8,
            "three faces on two sides, eight rims"
        );
        near(
            volume(&mid.body),
            h * (flat + r * quarter) * t * k3,
            1e-9,
            "middle",
        );
        let outer = h * (flat * t + quarter * ((r + t) * (r + t) - r * r) / 2.0) * k3;
        let inner = h * (flat * t + quarter * (r * r - (r - t) * (r - t)) / 2.0) * k3;
        let out = volume(&plated(&body, t * k, Side::Out, &BTreeMap::new(), k).body);
        let inn = volume(&plated(&body, t * k, Side::In, &BTreeMap::new(), k).body);
        let (a, b) = if (out - outer).abs() < (out - inner).abs() {
            (outer, inner)
        } else {
            (inner, outer)
        };
        near(out, a, 1e-9, "out");
        near(inn, b, 1e-9, "in");
    }
}

/// A floor (`x` in `0..a`, `y` in `0..l`, at `z = 0`, facing down) and a
/// wall (`z` in `0..b`, at `x = 0`, facing −x): an L whose outside is the
/// side the normals point to; the material of plates to the side in lies
/// inside the corner.
fn floor_and_wall(a: f64, b: f64, l: f64, k: f64) -> (Body, Body) {
    let floor_frame = Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap();
    // In that frame y is −y of the world: the rectangle runs to −l there.
    let floor = sheet([0.0, -l], [a, 0.0], None, &floor_frame, k);
    let wall_frame = Frame::new(Vec3::ZERO, -Vec3::X, Vec3::Z).unwrap();
    // x of the frame is z, y is (−x) × z ... = y of the world.
    let wall = sheet([0.0, 0.0], [b, l], None, &wall_frame, k);
    (floor, wall)
}

fn the_edge(body: &Body, l: f64, k: f64) -> EdgeId {
    edge_between(body, [0.0, 0.0, 0.0], [0.0, l, 0.0], k)
}

#[derive(Clone, Copy)]
enum K {
    Mitre,
    Touch,
    Over,
    Under,
}

fn joint(kind: K, n: Neighbour<'_>) -> Joint<'_> {
    match kind {
        K::Mitre => Joint::Mitre(n),
        K::Touch => Joint::Touch(n),
        K::Over => Joint::Over(n),
        K::Under => Joint::Under(n),
    }
}

struct Pair {
    floor: Plate,
    wall: Plate,
}

fn joined(a: f64, b: f64, l: f64, t: f64, k: f64, floor_joint: K, wall_joint: K) -> Pair {
    let (floor, wall) = floor_and_wall(a, b, l, k);
    let wall_n = Neighbour {
        body: &wall,
        face: FaceId(0),
        thickness: t * k,
        side: Side::In,
    };
    let floor_n = Neighbour {
        body: &floor,
        face: FaceId(0),
        thickness: t * k,
        side: Side::In,
    };
    let floor_joints = BTreeMap::from([(the_edge(&floor, l, k), joint(floor_joint, wall_n))]);
    let wall_joints = BTreeMap::from([(the_edge(&wall, l, k), joint(wall_joint, floor_n))]);
    Pair {
        floor: plated(&floor, t * k, Side::In, &floor_joints, k),
        wall: plated(&wall, t * k, Side::In, &wall_joints, k),
    }
}

/// The two plates fill `expected` together without overlapping: the
/// union's volume is the sum of theirs.
fn fill_together(pair: &Pair, expected: f64, k: f64, what: &str) {
    let (vf, vw) = (volume(&pair.floor.body), volume(&pair.wall.body));
    near(vf + vw, expected, 1e-9, what);
    let union = boolean(
        &pair.floor.body,
        &pair.wall.body,
        BooleanOp::Union,
        &tolerance(k),
    )
    .unwrap_or_else(|e| panic!("{what}: the union at scale {k}: {e}"));
    near(
        volume(&union.body),
        vf + vw,
        1e-9,
        &format!("{what}: no overlap"),
    );
}

#[test]
fn a_mitre_lies_on_the_bisector_and_the_plates_close_the_corner() {
    let (a, b, l, t) = (3.0, 2.0, 1.5, 0.25);
    for k in SCALES {
        let pair = joined(a, b, l, t, k, K::Mitre, K::Mitre);
        let k3 = k * k * k;
        near(
            volume(&pair.floor.body),
            l * (a * t - t * t / 2.0) * k3,
            1e-9,
            "floor",
        );
        near(
            volume(&pair.wall.body),
            l * (b * t - t * t / 2.0) * k3,
            1e-9,
            "wall",
        );
        fill_together(&pair, l * t * (a + b - t) * k3, k, "mitre");
        // Each mitre face is the plane x = z.
        for plate in [&pair.floor, &pair.wall] {
            let rims: Vec<usize> = plate
                .body
                .faces
                .iter()
                .enumerate()
                .filter(|(_, f)| match &f.surface {
                    Surface::Plane(p) => {
                        let n = p.frame().z();
                        (n.x.abs() - n.z.abs()).abs() < 1e-12 && n.y.abs() < 1e-12
                    }
                    _ => false,
                })
                .map(|(i, _)| i)
                .collect();
            assert_eq!(rims.len(), 1, "one face on the bisector");
            let Surface::Plane(p) = &plate.body.faces[rims[0]].surface else {
                panic!("not a plane")
            };
            let o = p.frame().origin();
            assert!(
                (o.x - o.z).abs() <= 1e-9 * k,
                "the bisector plane passes the edge"
            );
        }
    }
}

#[test]
fn touching_plates_meet_at_their_inner_edges() {
    let (a, b, l, t) = (3.0, 2.0, 1.5, 0.25);
    for k in SCALES {
        let pair = joined(a, b, l, t, k, K::Touch, K::Touch);
        let k3 = k * k * k;
        near(
            volume(&pair.floor.body),
            l * (a - t) * t * k3,
            1e-9,
            "floor",
        );
        near(volume(&pair.wall.body), l * (b - t) * t * k3, 1e-9, "wall");
        // The corner square t × t stays open outside, and the two touch
        // along a line only (their union would be zero thickness there):
        // the floor lies in x >= t, the wall in z >= t, both in the
        // corner's square of side t across.
        let (floor_lo, _) = pair.floor.body.bbox().unwrap().corners().unwrap();
        let (wall_lo, wall_hi) = pair.wall.body.bbox().unwrap().corners().unwrap();
        assert!(
            (floor_lo.x - t * k).abs() <= 1e-9 * k,
            "the floor starts at x = t"
        );
        assert!(
            (wall_lo.z - t * k).abs() <= 1e-9 * k,
            "the wall starts at z = t"
        );
        assert!(wall_hi.x <= t * k * (1.0 + 1e-9), "the wall is t thick");
    }
}

#[test]
fn a_plate_over_another_rests_on_its_edge() {
    let (a, b, l, t) = (3.0, 2.0, 1.5, 0.25);
    for k in SCALES {
        // The floor covers the wall's edge: the wall stands under it.
        let pair = joined(a, b, l, t, k, K::Over, K::Under);
        let k3 = k * k * k;
        near(volume(&pair.floor.body), l * a * t * k3, 1e-9, "floor over");
        near(
            volume(&pair.wall.body),
            l * (b - t) * t * k3,
            1e-9,
            "wall under",
        );
        fill_together(&pair, l * t * (a + b - t) * k3, k, "lap");
        // And the other way round.
        let pair = joined(a, b, l, t, k, K::Under, K::Over);
        near(
            volume(&pair.floor.body),
            l * (a - t) * t * k3,
            1e-9,
            "floor under",
        );
        near(volume(&pair.wall.body), l * b * t * k3, 1e-9, "wall over");
        fill_together(&pair, l * t * (a + b - t) * k3, k, "lap the other way");
    }
}

#[test]
fn a_bevel_adds_or_takes_its_wedge() {
    let (a, l, t) = (3.0, 1.5, 0.25);
    for k in SCALES {
        let (floor, _) = floor_and_wall(a, 2.0, l, k);
        let edge = the_edge(&floor, l, k);
        for degrees in [30.0_f64, -20.0] {
            let angle = degrees.to_radians();
            let joints = BTreeMap::from([(edge, Joint::Bevel(angle))]);
            for side in [Side::Out, Side::In, Side::Mid] {
                let made = plated(&floor, t * k, side, &joints, k);
                // The rim leans outward at the front by tan α per height:
                // the plate gains l ∫ s tan α ds over its levels.
                let (lo, hi) = side.levels(t);
                let wedge = l * angle.tan() * (hi * hi - lo * lo) / 2.0;
                near(
                    volume(&made.body),
                    (l * a * t + wedge) * k * k * k,
                    1e-9,
                    &format!("bevel {degrees} {side:?}"),
                );
            }
        }
    }
}

/// Half of a hard-chine hull (starboard), lofted ruled between a section
/// at x = 0 and another shape at x = 2: one open chain of three lines,
/// keel → chine → sheer → the deck's centre line, so the sheet has three
/// faces, the bottom, the side and the deck panel, sharing the chine and
/// the sheer (the sections are not alike: the panels are twisted ruled
/// surfaces, not planes).
fn hull_sheet(k: f64) -> Body {
    let tol = tolerance(k);
    let section = |x: f64, points: [[f64; 2]; 4]| {
        let points = points.map(|[y, z]| [y * k, z * k]);
        let curves: Vec<SketchCurve> = (0..3)
            .map(|i| line(i as u64 + 1, points[i], points[i + 1]))
            .collect();
        Section {
            plane: Frame::new(Vec3::new(x * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
            outline: open_chain(&curves, &tol).unwrap(),
            start: None,
        }
    };
    let options = Options {
        ruled: true,
        closed: false,
    };
    let stern = [[0.0, 0.0], [0.3, 0.12], [0.36, 0.3], [0.0, 0.36]];
    let bow = [[0.0, 0.02], [0.25, 0.08], [0.28, 0.26], [0.0, 0.34]];
    loft(&[section(0.0, stern), section(2.0, bow)], &options, &tol)
        .unwrap()
        .body
}

/// The edge two faces of `body` share.
fn between_faces(body: &Body, a: FaceId, b: FaceId) -> EdgeId {
    let uses = craft3d_topo::edge_faces(body);
    let found = uses
        .iter()
        .position(|faces| faces.contains(&a) && faces.contains(&b))
        .expect("the faces share no edge");
    EdgeId(found as u32)
}

/// The face of the hull sheet whose middle lies nearest `(y, z)` (at
/// scale 1, mid-length).
fn panel(body: &Body, y: f64, z: f64, k: f64) -> FaceId {
    let target = Vec3::new(1.0 * k, y * k, z * k);
    let middle = |f: usize| {
        let points: Vec<Vec3> = body.faces[f]
            .loops
            .iter()
            .flat_map(|l| &body.loops[l.0 as usize].coedges)
            .map(|c| body.vertices[body.edges[c.edge.0 as usize].start.0 as usize].point)
            .collect();
        points.iter().fold(Vec3::ZERO, |s, p| s + *p) * (1.0 / points.len() as f64)
    };
    let best = (0..body.faces.len())
        .min_by(|a, b| {
            middle(*a)
                .distance(target)
                .total_cmp(&middle(*b).distance(target))
        })
        .unwrap();
    FaceId(best as u32)
}

/// Points looked at along an edge where two plates must meet.
const DENSE: usize = 64;

/// No common volume, by sampling: points throughout the layers of the
/// two sheet faces the plates are made of (a grid over each face, at two
/// levels across the thickness, `levels` along the outward normal),
/// classified against both plates (`contains_points`, the booleans' ray
/// test, independent of the plates' construction): none is inside both,
/// and most of each plate's own points are inside it.
fn apart(
    sheet: &Body,
    faces: [FaceId; 2],
    plates: [&Body; 2],
    levels: (f64, f64),
    k: f64,
    what: &str,
) {
    let tol = tolerance(k);
    for (face, own) in faces.into_iter().zip([0usize, 1]) {
        let f = &sheet.faces[face.0 as usize];
        let ((u0, u1), (v0, v1)) = match &f.surface {
            Surface::Nurbs(n) => n.domain(),
            _ => panic!("the hull's panels are spline surfaces"),
        };
        let mut points = Vec::new();
        for i in 0..=6 {
            for j in 0..=6 {
                let (u, v) = (
                    u0 + (u1 - u0) * f64::from(i) / 6.0,
                    v0 + (v1 - v0) * f64::from(j) / 6.0,
                );
                let n = f.surface.normal(u, v).unwrap() * if f.reversed { -1.0 } else { 1.0 };
                for level in 1..=2 {
                    let s = levels.0 + (levels.1 - levels.0) * f64::from(level) / 3.0;
                    points.push(f.surface.eval(u, v) + n * s);
                }
            }
        }
        let inside: Vec<Vec<Option<bool>>> = plates
            .iter()
            .map(|p| craft3d_kernel::boolean::contains_points(p, &points, &tol).unwrap())
            .collect();
        let both = (0..points.len())
            .filter(|&i| inside[0][i] == Some(true) && inside[1][i] == Some(true))
            .count();
        assert_eq!(both, 0, "{what} at scale {k}: points inside both plates");
        let in_own = inside[own].iter().filter(|x| **x == Some(true)).count();
        assert!(
            in_own * 2 > points.len(),
            "{what} at scale {k}: only {in_own} of {} points of the layer are inside its plate",
            points.len()
        );
    }
}

#[test]
fn the_panels_of_a_hard_chine_hull_close_up_without_overlap() {
    for k in SCALES {
        let hull = hull_sheet(k);
        assert_eq!(hull.faces.len(), 3);
        let (bottom, side, deck) = (
            panel(&hull, 0.14, 0.05, k),
            panel(&hull, 0.3, 0.2, k),
            panel(&hull, 0.16, 0.3, k),
        );
        let t = 0.006 * k;
        let neighbour = |face| Neighbour {
            body: &hull,
            face,
            thickness: t,
            side: Side::In,
        };
        let chine = between_faces(&hull, bottom, side);
        let sheer = between_faces(&hull, side, deck);
        let tol = tolerance(k);
        let made = |face: FaceId, joints: BTreeMap<EdgeId, Joint<'_>>| {
            let p = plate(&hull, &[face], t, Side::In, &joints, &tol)
                .unwrap_or_else(|e| panic!("panel {} at scale {k}: {e:?}", face.0));
            assert_eq!(check(&p.body, &tol), [], "a valid panel");
            assert!(volume(&p.body) > 0.0);
            p
        };
        let clock = std::time::Instant::now();
        let b = made(
            bottom,
            BTreeMap::from([(chine, Joint::Touch(neighbour(side)))]),
        );
        let s = made(
            side,
            BTreeMap::from([
                (chine, Joint::Touch(neighbour(bottom))),
                (sheer, Joint::Under(neighbour(deck))),
            ]),
        );
        let d = made(
            deck,
            BTreeMap::from([(sheer, Joint::Over(neighbour(side)))]),
        );
        println!("scale {k}: three panels in {:?}", clock.elapsed());
        let clock = std::time::Instant::now();
        // The inner edges of the bottom and the side just touch, the deck
        // rests on the side panel, and no two overlap.
        let levels = Side::In.levels(t);
        apart(
            &hull,
            [bottom, side],
            [&b.body, &s.body],
            levels,
            k,
            "bottom and side",
        );
        apart(
            &hull,
            [side, deck],
            [&s.body, &d.body],
            levels,
            k,
            "side and deck",
        );
        println!("scale {k}: classified in {:?}", clock.elapsed());
        // The inner edges along the chine are one line: each of the two
        // panels' edges between its back skin and its rim there lies on
        // the other's, at every point.
        let inner = |p: &Plate, face: FaceId| {
            let faces = |made: Made| p.faces.iter().position(|m| *m == made).unwrap();
            let (back, rim) = (faces(Made::Back(face)), faces(Made::Rim(chine)));
            let uses = craft3d_topo::edge_faces(&p.body);
            let e = uses
                .iter()
                .position(|f| f.contains(&FaceId(back as u32)) && f.contains(&FaceId(rim as u32)))
                .unwrap();
            p.body.edges[e].clone()
        };
        let (eb, es) = (inner(&b, bottom), inner(&s, side));
        let t_k = t;
        for (from, onto, what) in [(&eb, &es, "the bottom's"), (&es, &eb, "the side's")] {
            for i in 0..=DENSE {
                let f = i as f64 / DENSE as f64;
                let x = from
                    .curve
                    .eval(from.range.0 + (from.range.1 - from.range.0) * f);
                // On the other's line, and within its stretch but for the
                // ends, where each panel is cut square to its own normal
                // (they end apart by about the thickness times the angle
                // between the panels).
                let t = onto.curve.project(x).unwrap();
                let off = onto.curve.eval(t).distance(x);
                let end = |u: f64| onto.curve.eval(u).distance(x);
                let within = (t >= onto.range.0 && t <= onto.range.1)
                    || end(onto.range.0).min(end(onto.range.1)) <= 2.0 * t_k;
                assert!(
                    off <= tol.linear && within,
                    "{what} inner chine edge at scale {k}: {off} off the other's, at {x:?}"
                );
            }
        }
        // The deck rests on the side panel: the side panel's rim along
        // the sheer lies on the deck's back skin, and the deck's rim on the
        // side panel's front skin (the outside), at every point of their
        // edges.
        let skin_of = |p: &Plate, made: Made| {
            let i = p.faces.iter().position(|m| *m == made).unwrap();
            p.body.faces[i].surface.clone()
        };
        let lies_on = |p: &Plate, rim: Made, surface: &Surface, what: &str| {
            let i = p.faces.iter().position(|m| *m == rim).unwrap();
            for l in &p.body.faces[i].loops {
                for c in &p.body.loops[l.0 as usize].coedges {
                    let e = &p.body.edges[c.edge.0 as usize];
                    for i in 0..=DENSE {
                        let f = i as f64 / DENSE as f64;
                        let x = e.curve.eval(e.range.0 + (e.range.1 - e.range.0) * f);
                        let (u, v) = surface.project(x).unwrap();
                        let off = surface.eval(u, v).distance(x);
                        assert!(off <= tol.linear, "{what} at scale {k}: {off} off");
                    }
                }
            }
        };
        lies_on(
            &s,
            Made::Rim(sheer),
            &skin_of(&d, Made::Back(deck)),
            "the side panel's top on the deck",
        );
        lies_on(
            &d,
            Made::Rim(sheer),
            &skin_of(&s, Made::Front(side)),
            "the deck's edge flush with the side panel",
        );
    }
}

#[test]
fn the_same_plate_twice_is_the_same_body() {
    let body = cylinder_sheet(2.0, 1.0, 1.0, 1.0);
    let a = plated(&body, 0.1, Side::Out, &BTreeMap::new(), 1.0);
    let b = plated(&body, 0.1, Side::Out, &BTreeMap::new(), 1.0);
    assert_eq!(format!("{:?}", a.body), format!("{:?}", b.body));
}

#[test]
fn refusals_say_why() {
    let tol = tolerance(1.0);
    let (floor, wall) = floor_and_wall(3.0, 2.0, 1.5, 1.0);
    let none = BTreeMap::new();
    assert!(matches!(
        plate(&floor, &[FaceId(0)], 0.0, Side::In, &none, &tol),
        Err(KernelError::InvalidSize { .. })
    ));
    assert!(matches!(
        plate(&floor, &[FaceId(0)], f64::NAN, Side::In, &none, &tol),
        Err(KernelError::InvalidSize { .. })
    ));
    assert!(matches!(
        plate(&floor, &[], 0.1, Side::In, &none, &tol),
        Err(KernelError::Plate { .. })
    ));
    assert!(matches!(
        plate(&floor, &[FaceId(7)], 0.1, Side::In, &none, &tol),
        Err(KernelError::Plate { .. })
    ));
    // A joint with a face the edge does not run along.
    let far = sheet([5.0, 5.0], [6.0, 6.0], None, &Frame::WORLD, 1.0);
    let joints = BTreeMap::from([(
        the_edge(&floor, 1.5, 1.0),
        Joint::Mitre(Neighbour {
            body: &far,
            face: FaceId(0),
            thickness: 0.1,
            side: Side::In,
        }),
    )]);
    let error = plate(&floor, &[FaceId(0)], 0.1, Side::In, &joints, &tol).unwrap_err();
    assert!(error.to_string().contains("does not run along"), "{error}");
    // Plates in line have no corner to join at.
    let next = sheet(
        [3.0, -1.5],
        [5.0, 0.0],
        None,
        &Frame::new(Vec3::ZERO, -Vec3::Z, Vec3::X).unwrap(),
        1.0,
    );
    let shared = edge_between(&floor, [3.0, 0.0, 0.0], [3.0, 1.5, 0.0], 1.0);
    let joints = BTreeMap::from([(
        shared,
        Joint::Mitre(Neighbour {
            body: &next,
            face: FaceId(0),
            thickness: 0.1,
            side: Side::In,
        }),
    )]);
    let error = plate(&floor, &[FaceId(0)], 0.1, Side::In, &joints, &tol).unwrap_err();
    assert!(error.to_string().contains("in line"), "{error}");
    // A bevel past square.
    let joints = BTreeMap::from([(shared, Joint::Bevel(2.0))]);
    assert!(plate(&floor, &[FaceId(0)], 0.1, Side::In, &joints, &tol).is_err());
    // Faces that meet at a crease are plated apart.
    let hull = hull_sheet(1.0);
    let error = plate(&hull, &all_faces(&hull), 0.01, Side::In, &none, &tol).unwrap_err();
    assert!(error.to_string().contains("crease"), "{error}");
    // A joint on an edge between two of the plate's faces.
    let crease = between_faces(&hull, FaceId(0), FaceId(1));
    let joints = BTreeMap::from([(crease, Joint::Bevel(0.1))]);
    let error = plate(
        &hull,
        &[FaceId(0), FaceId(1)],
        0.01,
        Side::In,
        &joints,
        &tol,
    )
    .unwrap_err();
    assert!(error.to_string().contains("not on the boundary"), "{error}");
    let _ = wall;
}
