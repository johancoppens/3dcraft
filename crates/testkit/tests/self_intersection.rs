//! Oracle 9 (docs/testing.md): no body passes through itself.
//!
//! `craft3d_topo::check` does not look for self-intersection, so a body
//! can pass it while two of its faces cross or two of its shells overlap.
//! `craft3d_tess::body_self_intersections` looks, without the kernel's
//! help. Here it is run on every body the kernel can make today (the
//! reference solids, the primitives, extrusions, moved, mirrored and
//! merged bodies, the results of the boolean scenarios and of a seeded
//! random run of booleans), none of which may have a certain finding; and
//! on bodies with a seeded defect, each of which must be found.
//!
//! This lives in `testkit` because `kernel` and `tess` share a layer and
//! may not depend on each other, not even for tests.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

// The operands and the generator of the kernel's own boolean tests.
#[path = "../../kernel/tests/boolean_common/mod.rs"]
mod boolean_common;

use boolean_common::{Rng, SCALES, boxed, rod, tolerance};
use craft3d_geom::{Curve3, Line, Plane, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_tess::{
    CrossingKind, SelfCheck, SelfCheckParams, ShellDefect, body_self_intersections,
};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, check, merge};
use std::collections::BTreeMap;

use BooleanOp::{Intersect, Subtract, Union};

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

/// A frame with nothing round about it.
fn tilted(k: f64) -> Frame {
    Frame::new(
        v(0.3 * k, -1.7 * k, 0.9 * k),
        v(0.3, -0.5, 0.8),
        v(1.0, 0.2, 0.1),
    )
    .unwrap()
}

fn params(k: f64) -> SelfCheckParams {
    SelfCheckParams::new(tolerance(k).linear)
}

fn inspect(body: &Body, k: f64) -> SelfCheck {
    match body_self_intersections(body, &params(k)) {
        Ok(found) => found,
        Err(e) => panic!("the body cannot be looked at: {e}"),
    }
}

/// Nothing found, not even within the margin.
#[track_caller]
fn assert_spotless(body: &Body, k: f64, what: &str) {
    assert_eq!(check(body, &tolerance(k)), [], "{what}");
    let found = inspect(body, k);
    assert!(found.is_spotless(), "{what} at scale {k}: {found}");
    assert_eq!(found.degenerate_triangles, 0, "{what} at scale {k}");
}

/// Something certain found.
#[track_caller]
fn assert_caught(body: &Body, k: f64, what: &str) -> SelfCheck {
    let found = inspect(body, k);
    assert!(
        !found.is_clean(),
        "{what} at scale {k} was not found: {found}"
    );
    found
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn polygon(points: &[[f64; 2]]) -> Vec<SketchCurve> {
    (0..points.len())
        .map(|i| line(i as u64 + 1, points[i], points[(i + 1) % points.len()]))
        .collect()
}

/// An L: `a` by `b` with the corner beyond the middle cut away, `c` high.
fn l_shape(frame: &Frame, a: f64, b: f64, c: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let (ha, hb) = (a * 0.5, b * 0.5);
    let curves = polygon(&[[0.0, 0.0], [a, 0.0], [a, hb], [ha, hb], [ha, b], [0.0, b]]);
    let profiles = detect_profiles(&curves, &tol).unwrap();
    extrude(&profiles, frame, 0.0, c, &tol).unwrap().body
}

/// A slot (two half circles joined by lines, which are tangent to them)
/// of length `a` and width `b` with a round hole, `c` high.
fn slot(frame: &Frame, a: f64, b: f64, c: f64, k: f64) -> Body {
    let tol = tolerance(k);
    let (r, hole) = (b * 0.5, b * 0.25);
    let arc = |id: u64, center: [f64; 2], start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Arc { center, start, end },
    };
    let curves = vec![
        line(1, [0.0, -r], [a, -r]),
        arc(2, [a, 0.0], [a, -r], [a, r]),
        line(3, [a, r], [0.0, r]),
        arc(4, [0.0, 0.0], [0.0, r], [0.0, -r]),
        SketchCurve {
            id: 5,
            curve: Curve2::Circle {
                center: [a * 0.5, 0.0],
                radius: hole,
            },
        },
    ];
    let profiles = detect_profiles(&curves, &tol).unwrap();
    extrude(&profiles, frame, 0.0, c, &tol).unwrap().body
}

/// A solid with flat faces: `faces` lists the corners of each,
/// counter-clockwise seen from outside. Nothing is checked: this is how
/// the defects are seeded.
fn polyhedron(points: &[Vec3], faces: &[&[usize]]) -> Body {
    let mut body = Body::new(BodyKind::Solid);
    let vertices: Vec<_> = points.iter().map(|&p| body.add_vertex(p)).collect();
    let mut edges: BTreeMap<(usize, usize), EdgeId> = BTreeMap::new();
    let mut ids = Vec::new();
    for corners in faces {
        let mut coedges = Vec::new();
        for (i, &from) in corners.iter().enumerate() {
            let to = corners[(i + 1) % corners.len()];
            let key = (from.min(to), from.max(to));
            let edge = *edges.entry(key).or_insert_with(|| {
                let (start, end) = (points[key.0], points[key.1]);
                let curve = Curve3::Line(Line::through(start, end).unwrap());
                body.add_edge(
                    curve,
                    (0.0, start.distance(end)),
                    vertices[key.0],
                    vertices[key.1],
                )
            });
            coedges.push(Coedge {
                edge,
                reversed: from > to,
            });
        }
        let (a, b, c) = (points[corners[0]], points[corners[1]], points[corners[2]]);
        let frame = Frame::new(a, (b - a).cross(c - a), b - a).unwrap();
        let loop_ = body.add_loop(coedges);
        ids.push(body.add_face(Surface::Plane(Plane::new(frame)), false, vec![loop_]));
    }
    body.add_shell(ids);
    body
}

/// The corners of a box in `frame`, numbered `i + 2j + 4k`.
fn corners(frame: &Frame, lo: [f64; 3], hi: [f64; 3]) -> Vec<Vec3> {
    (0..8)
        .map(|n| {
            let pick = |axis: usize| {
                if n >> axis & 1 == 0 {
                    lo[axis]
                } else {
                    hi[axis]
                }
            };
            frame.point(pick(0), pick(1), pick(2))
        })
        .collect()
}

/// The sides of that box, facing outward: -z, +z, -y, +y, -x, +x.
const SIDES: [[usize; 4]; 6] = [
    [0, 2, 3, 1],
    [4, 5, 7, 6],
    [0, 1, 5, 4],
    [2, 6, 7, 3],
    [0, 4, 6, 2],
    [1, 3, 7, 5],
];

/// A box that is inside out: a cavity without anything around it.
fn inside_out(frame: &Frame, lo: [f64; 3], hi: [f64; 3]) -> Body {
    let points = corners(frame, lo, hi);
    let reversed: Vec<Vec<usize>> = SIDES
        .iter()
        .map(|s| s.iter().rev().copied().collect())
        .collect();
    let faces: Vec<&[usize]> = reversed.iter().map(Vec::as_slice).collect();
    polyhedron(&points, &faces)
}

/// A unit box (times `k`) whose lid is four triangles meeting in a point,
/// which is put at `peak` (in units, in the frame): at (x, y, 1) the lid
/// is flat; anywhere else it is a roof, a dent, or goes through the box.
fn tent(frame: &Frame, k: f64, peak: [f64; 3]) -> Body {
    let mut points = corners(frame, [0.0; 3], [k, k, k]);
    points.push(frame.point(peak[0] * k, peak[1] * k, peak[2] * k));
    let mut faces: Vec<&[usize]> = vec![&SIDES[0], &SIDES[2], &SIDES[3], &SIDES[4], &SIDES[5]];
    let lid: [&[usize]; 4] = [&[4, 5, 8], &[5, 7, 8], &[7, 6, 8], &[6, 4, 8]];
    faces.extend(lid);
    polyhedron(&points, &faces)
}

fn joined(bodies: &[&Body]) -> Body {
    merge(bodies).unwrap().0
}

// ------------------------------------------------------- valid bodies

#[test]
fn reference_solids_are_spotless() {
    for k in SCALES {
        for frame in [Frame::WORLD, tilted(k)] {
            let mut all = solids::all(&frame, k).unwrap();
            all.extend(trimmed::all(&frame, k).unwrap());
            for solid in &all {
                assert_spotless(&solid.body, k, solid.name);
            }
            // Not in `all`: its mass properties cannot be confirmed, its
            // mesh can.
            let elbow = trimmed::pipe_elbow(&frame, 3.0 * k, k, 1.9).unwrap();
            let found = inspect(&elbow.body, k);
            assert!(found.is_spotless(), "{}: {found}", elbow.name);
        }
    }
}

#[test]
fn primitives_and_extrusions_are_spotless() {
    for k in SCALES {
        for frame in [Frame::WORLD, tilted(k)] {
            let bodies = [
                (
                    "block",
                    primitives::block(&frame, 3.0 * k, 2.0 * k, k).unwrap(),
                ),
                (
                    "cylinder",
                    primitives::cylinder(&frame, k, 3.0 * k).unwrap(),
                ),
                ("cone", primitives::cone(&frame, 2.0 * k, 3.0 * k).unwrap()),
                ("sphere", primitives::sphere(&frame, 2.0 * k).unwrap()),
                ("torus", primitives::torus(&frame, 3.0 * k, k).unwrap()),
                // A thin torus and a fat one, a flat cone and a needle.
                (
                    "thin torus",
                    primitives::torus(&frame, 3.0 * k, 0.05 * k).unwrap(),
                ),
                (
                    "fat torus",
                    primitives::torus(&frame, 3.0 * k, 2.9 * k).unwrap(),
                ),
                (
                    "flat cone",
                    primitives::cone(&frame, 5.0 * k, 0.1 * k).unwrap(),
                ),
                (
                    "needle",
                    primitives::cone(&frame, 0.05 * k, 5.0 * k).unwrap(),
                ),
                (
                    "a thin plate",
                    primitives::block(&frame, 5.0 * k, 5.0 * k, 0.01 * k).unwrap(),
                ),
                ("an L", l_shape(&frame, 3.0 * k, 2.0 * k, k, k)),
                // The flat sides of a slot are tangent to its round ends.
                ("a slot with a hole", slot(&frame, 3.0 * k, 2.0 * k, k, k)),
                (
                    "a long thin slot",
                    slot(&frame, 9.0 * k, 0.2 * k, 0.1 * k, k),
                ),
            ];
            for (name, body) in &bodies {
                assert_spotless(body, k, name);
            }
        }
    }
}

#[test]
fn moved_mirrored_and_merged_bodies() {
    for k in SCALES {
        let frame = tilted(k);
        let turn = Transform::rotation_about(v(k, 0.0, 2.0 * k), v(0.2, 0.9, -0.3), 1.1).unwrap();
        let mirror = Transform::mirror(v(0.5 * k, 0.0, 0.0), v(0.6, 0.0, 0.8)).unwrap();
        let shift = Transform::translation(v(20.0 * k, 0.0, 0.0));
        let slot = slot(&frame, 3.0 * k, 2.0 * k, k, k);
        let cap = trimmed::spherical_cap(&frame, 2.5 * k, k).unwrap().body;
        for body in [&slot, &cap] {
            for (name, t) in [
                ("turned", &turn),
                ("mirrored", &mirror),
                ("shifted", &shift),
            ] {
                assert_spotless(&body.transformed(t).unwrap(), k, name);
            }
            // The body and its copy far away: two shells, nothing wrong.
            let far = body.transformed(&shift).unwrap();
            assert_spotless(&joined(&[body, &far]), k, "a body and its copy apart");
            // The body and its copy a little aside: they overlap.
            let near = body
                .transformed(&Transform::translation(v(0.3 * k, 0.2 * k, 0.1 * k)))
                .unwrap();
            let both = joined(&[body, &near]);
            assert_eq!(
                check(&both, &tolerance(k)),
                [],
                "the checker does not see it"
            );
            let found = assert_caught(&both, k, "a body and its copy overlapping");
            assert!(found.certain_crossings > 0, "{found}");
            for c in found.crossings.iter().filter(|c| c.certain) {
                assert_eq!(c.shells, [Some(0), Some(1)], "{found}");
            }
        }

        // Two blocks apart, and the two blocks of the kernel's first
        // boolean test, which overlap in a 3 x 2 x 1 corner.
        let a = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let apart = boxed(k, [4.5, 0.0, 0.0], [6.0, 5.0, 4.0]);
        assert_spotless(&joined(&[&a, &apart]), k, "two blocks apart");
        let b = boxed(k, [1.0, 1.0, 1.0], [6.0, 5.0, 4.0]);
        let both = joined(&[&a, &b]);
        assert_eq!(
            check(&both, &tolerance(k)),
            [],
            "the checker does not see it"
        );
        let found = assert_caught(&both, k, "two overlapping blocks merged");
        // Twelve edges of the common corner... six of them are crossings
        // of a face of one with a face of the other.
        let pairs: std::collections::BTreeSet<[u32; 2]> = found
            .crossings
            .iter()
            .filter(|c| c.certain)
            .map(|c| c.faces)
            .collect();
        assert_eq!(pairs.len(), 6, "{found}");
        for c in found.crossings.iter().filter(|c| c.certain) {
            assert_eq!(c.kind, CrossingKind::Crossing);
            assert_eq!(c.shells, [Some(0), Some(1)]);
            // The common corner is one unit deep at its shallowest; the
            // depth is measured up to four times the margin.
            assert!(
                c.depth > c.margin && c.margin == 2.0 * tolerance(k).linear,
                "{c:?}"
            );
            let p = c.at * (1.0 / k);
            let within = |x: f64, lo: f64, hi: f64| x > lo - 1e-9 && x < hi + 1e-9;
            assert!(
                within(p.x, 1.0, 4.0) && within(p.y, 1.0, 3.0) && within(p.z, 1.0, 2.0),
                "{c:?}"
            );
        }
        // The union of the same two is one clean shell.
        let union = boolean(&a, &b, Union, &tolerance(k)).unwrap().body;
        assert_spotless(&union, k, "the union of the two blocks");
    }
}

// ------------------------------------------------------ seeded defects

/// (a), (b) and (g) on a solid of flat faces, where the mesh is the body
/// and the margin is twice the modelling tolerance.
#[test]
fn a_lid_pushed_through_the_body_is_caught() {
    for k in SCALES {
        let tol = tolerance(k);
        for frame in [Frame::WORLD, tilted(k)] {
            // A roof, a flat lid and a dent are fine.
            for z in [1.5, 1.0, 0.5, 0.01] {
                assert_spotless(&tent(&frame, k, [0.6, 0.3, z]), k, "a tent");
            }
            // (a) The peak pushed out through the bottom: the lid crosses
            // the bottom face, which it shares nothing with.
            let through = tent(&frame, k, [0.6, 0.3, -0.5]);
            assert_eq!(check(&through, &tol), [], "the checker does not see it");
            let found = assert_caught(&through, k, "a lid through the bottom");
            for c in found.crossings.iter().filter(|c| c.certain) {
                assert_eq!(c.faces[0], 0, "the bottom is face 0: {found}");
                assert!(c.faces[1] >= 5, "the lid is faces 5 to 8: {found}");
                assert_eq!(c.kind, CrossingKind::Crossing);
            }
            // (b) The peak pulled out through a side: the lid is folded
            // through the wall it hangs on.
            let folded = tent(&frame, k, [1.5, 0.5, 0.5]);
            assert_eq!(check(&folded, &tol), [], "the checker does not see it");
            let found = assert_caught(&folded, k, "a lid through a side");
            for c in found.crossings.iter().filter(|c| c.certain) {
                assert_eq!(c.faces[0], 4, "the side at +x is face 4: {found}");
                assert!(c.faces[1] >= 5, "{found}");
            }
            // (g) Through the bottom by three times the tolerance, with a
            // margin of twice the tolerance: caught, and the depth is the
            // three. By once the tolerance: within the margin.
            let unit = tol.linear / k;
            let just = tent(&frame, k, [0.6, 0.3, -3.0 * unit]);
            let found = assert_caught(&just, k, "a lid just through the bottom");
            let deepest = found.crossings.iter().map(|c| c.depth).fold(0.0, f64::max);
            assert!(
                (deepest - 3.0 * tol.linear).abs() < 0.01 * tol.linear,
                "{deepest} at scale {k}: {found}"
            );
            assert!(found.crossings.iter().all(|c| c.margin == 2.0 * tol.linear));
            let hardly = tent(&frame, k, [0.6, 0.3, -unit]);
            let found = inspect(&hardly, k);
            assert!(found.is_clean(), "{found}");
            assert!(found.marginal_crossings > 0, "{found}");
        }
    }
}

/// (c), (d) and (e): shells that overlap, and shells that lie where they
/// should not.
#[test]
fn shells_in_the_wrong_place_are_caught() {
    for k in SCALES {
        let tol = tolerance(k);
        let frame = tilted(k);
        let big = primitives::block(&frame, 4.0 * k, 4.0 * k, 4.0 * k).unwrap();
        let at = |x: f64, y: f64, z: f64| frame.at(frame.point(x * k, y * k, z * k)).unwrap();

        // (c) Partly overlapping: a block, a cylinder and a sphere, each
        // half in the big block.
        let half_in = [
            (
                "a block",
                primitives::block(&at(3.0, 1.0, 1.0), 2.0 * k, k, k).unwrap(),
            ),
            (
                "a cylinder",
                primitives::cylinder(&at(2.0, 2.0, 3.0), k, 2.0 * k).unwrap(),
            ),
            (
                "a sphere",
                primitives::sphere(&at(4.0, 2.0, 2.0), k).unwrap(),
            ),
            (
                "a torus",
                primitives::torus(&at(2.0, 2.0, 4.0), 1.5 * k, 0.5 * k).unwrap(),
            ),
        ];
        for (name, other) in &half_in {
            let both = joined(&[&big, other]);
            assert_eq!(check(&both, &tol), [], "the checker does not see it");
            let found = assert_caught(&both, k, name);
            assert!(found.certain_crossings > 0, "{name}: {found}");
            for c in found.crossings.iter().filter(|c| c.certain) {
                assert_eq!(c.shells, [Some(0), Some(1)], "{name}: {found}");
            }
        }

        // (d) Wholly inside: nothing crosses, and the inner shell is an
        // outside within an outside.
        let inner = [
            (
                "a block",
                primitives::block(&at(1.0, 1.0, 1.0), k, 2.0 * k, k).unwrap(),
            ),
            (
                "a sphere",
                primitives::sphere(&at(2.0, 2.0, 2.0), k).unwrap(),
            ),
            (
                "a torus",
                primitives::torus(&at(2.0, 2.0, 2.0), k, 0.25 * k).unwrap(),
            ),
        ];
        for (name, other) in &inner {
            for (order, outer_shell, inner_shell) in [([&big, other], 0, 1), ([other, &big], 1, 0)]
            {
                let both = joined(&order);
                assert_eq!(check(&both, &tol), [], "the checker does not see it");
                let found = assert_caught(&both, k, name);
                assert_eq!(
                    found.certain_crossings + found.marginal_crossings,
                    0,
                    "{found}"
                );
                assert_eq!(found.shells.len(), 1, "{name}: {found}");
                let finding = &found.shells[0];
                assert_eq!(finding.shell, inner_shell, "{name}: {found}");
                assert_eq!(finding.defect, ShellDefect::OuterInsideSolid);
                assert_eq!(finding.inside, [outer_shell]);
                assert!(finding.certain);
            }
        }

        // (e) A cavity with nothing around it: alone, beside a block, and
        // in a cavity of the block.
        let lost = inside_out(&at(9.0, 0.0, 0.0), [0.0; 3], [k, k, k]);
        let found = assert_caught(&lost, k, "a body inside out");
        assert_eq!(found.shells.len(), 1, "{found}");
        assert_eq!(found.shells[0].defect, ShellDefect::CavityOutsideSolid);
        let both = joined(&[&big, &lost]);
        assert_eq!(check(&both, &tol), [], "the checker does not see it");
        let found = assert_caught(&both, k, "a cavity beside the block");
        assert_eq!(found.shells.len(), 1, "{found}");
        assert_eq!(
            (found.shells[0].shell, found.shells[0].defect),
            (1, ShellDefect::CavityOutsideSolid)
        );
        assert_eq!(found.shells[0].inside, []);

        // A cavity where it belongs: in the block. Made by hand and by
        // the kernel.
        let cavity = inside_out(&at(1.0, 1.0, 1.0), [0.0; 3], [2.0 * k, 2.0 * k, 2.0 * k]);
        let hollow = joined(&[&big, &cavity]);
        assert_spotless(&hollow, k, "a block with a cavity");
        let core = primitives::block(&at(1.0, 1.0, 1.0), 2.0 * k, 2.0 * k, 2.0 * k).unwrap();
        let cut = boolean(&big, &core, Subtract, &tol).unwrap().body;
        assert_eq!(cut.shells.len(), 2);
        assert_spotless(&cut, k, "a block less its core");
        // A lump loose in the cavity is fine; a cavity in the cavity and a
        // lump in the material are not.
        let lump = primitives::sphere(&at(2.0, 2.0, 2.0), 0.5 * k).unwrap();
        assert_spotless(&joined(&[&hollow, &lump]), k, "a lump in a cavity");
        let void = inside_out(&at(1.5, 1.5, 1.5), [0.0; 3], [k, k, k]);
        let found = assert_caught(&joined(&[&hollow, &void]), k, "a cavity in a cavity");
        assert_eq!(found.shells.len(), 1, "{found}");
        assert_eq!(
            (found.shells[0].shell, found.shells[0].defect),
            (2, ShellDefect::CavityOutsideSolid)
        );
        let stray = primitives::block(&at(0.25, 0.25, 0.25), 0.5 * k, 0.5 * k, 0.5 * k).unwrap();
        let found = assert_caught(&joined(&[&hollow, &stray]), k, "a lump in the material");
        assert_eq!(found.shells.len(), 1, "{found}");
        assert_eq!(
            (found.shells[0].shell, found.shells[0].defect),
            (2, ShellDefect::OuterInsideSolid)
        );
        assert_eq!(found.shells[0].inside, [0]);
    }
}

/// The box from `lo` to `hi` (times `k`) in `frame`.
fn boxed_in(frame: &Frame, k: f64, lo: [f64; 3], hi: [f64; 3]) -> Body {
    let corner = frame
        .at(frame.point(lo[0] * k, lo[1] * k, lo[2] * k))
        .unwrap();
    primitives::block(
        &corner,
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

/// (f) Faces in one plane that share area: along the axes, where they are
/// in one plane to the last bit, and in a tilted frame, where they are
/// not and the modelling tolerance says they are.
#[test]
fn faces_overlapping_in_one_plane_are_caught() {
    for k in SCALES {
        let tol = tolerance(k);
        for frame in [Frame::WORLD, tilted(k)] {
            let overlaps = |found: &SelfCheck| {
                found
                    .crossings
                    .iter()
                    .filter(|c| c.certain && c.kind == CrossingKind::CoplanarOverlap)
                    .map(|c| c.faces)
                    .collect::<std::collections::BTreeSet<_>>()
            };
            // Facing each other: a block standing on a block. Only the
            // two faces that lie on each other, and nothing else.
            let a = boxed_in(&frame, k, [0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
            let on_top = boxed_in(&frame, k, [0.5, 0.5, 2.0], [1.5, 1.5, 3.0]);
            let stacked = joined(&[&a, &on_top]);
            assert_eq!(check(&stacked, &tol), [], "the checker does not see it");
            let found = assert_caught(&stacked, k, "a block standing on a block");
            // +z of the first (face 1) and -z of the second (face 6).
            assert_eq!(overlaps(&found), [[1, 6]].into(), "{found}");
            assert!(
                found
                    .crossings
                    .iter()
                    .all(|c| !c.certain || c.kind == CrossingKind::CoplanarOverlap),
                "{found}"
            );
            // The upper block is one unit wide: the middle of the common
            // square is half a unit from its border.
            let deepest = found.crossings.iter().map(|c| c.depth).fold(0.0, f64::max);
            assert!(
                deepest > 0.2 * k && deepest <= 0.5 * k * (1.0 + 1e-9),
                "{found}"
            );
            // Lifted off by five tolerances it stands apart: nothing.
            let lifted = boxed_in(&frame, k, [0.5, 0.5, 2.0 + 5e-6], [1.5, 1.5, 3.0]);
            assert_spotless(&joined(&[&a, &lifted]), k, "a block above a block");
            // Facing the same way: two blocks through each other with
            // tops, bottoms and one side flush.
            let beside = boxed_in(&frame, k, [1.0, 0.0, 0.0], [3.0, 3.0, 2.0]);
            let flush = joined(&[&a, &beside]);
            let found = assert_caught(&flush, k, "two blocks flush");
            assert_eq!(overlaps(&found), [[0, 6], [1, 7], [2, 8]].into(), "{found}");
            // The same body twice: every face on its twin.
            let twice = joined(&[&a, &a]);
            let found = assert_caught(&twice, k, "a block twice");
            assert_eq!(
                overlaps(&found),
                (0..6).map(|f| [f, f + 6]).collect(),
                "{found}"
            );
            // Side by side, sharing a whole face and a part of one: that
            // face is an overlap; the flush tops and sides, which share
            // an edge and no area, are not.
            let next = boxed_in(&frame, k, [2.0, 0.0, 0.0], [3.0, 2.0, 2.0]);
            let found = assert_caught(&joined(&[&a, &next]), k, "blocks sharing a face");
            // +x of the first (face 5) and -x of the second (face 10).
            assert_eq!(overlaps(&found), [[5, 10]].into(), "{found}");
            // Their union and their difference are clean: the kernel
            // merges what lies on each other, and leaves faces side by
            // side in one plane, which is fine.
            for (other, op) in [
                (&on_top, Union),
                (&beside, Union),
                (&beside, Subtract),
                (&next, Union),
            ] {
                let body = boolean(&a, other, op, &tol).unwrap().body;
                assert_spotless(&body, k, "a boolean of flush blocks");
            }
        }
    }
}

/// (g) on curved faces: two spheres that overlap by a multiple of the
/// chordal tolerance, and things that touch without overlapping.
#[test]
fn the_margin_separates_touching_from_crossing() {
    for k in SCALES {
        let tol = tolerance(k);
        let radius = k;
        // One mesh only: the margin between two spheres is twice the
        // chordal tolerance and twice the modelling tolerance.
        let coarse = SelfCheckParams {
            refinements: 0,
            ..params(k)
        };
        let spheres = |overlap: f64| {
            let a = primitives::sphere(&Frame::WORLD, radius).unwrap();
            let centre = v(0.6, 0.0, 0.8) * (2.0 * radius - overlap);
            let b = primitives::sphere(&Frame::WORLD.at(centre).unwrap(), radius).unwrap();
            joined(&[&a, &b])
        };
        let probe = body_self_intersections(&spheres(0.0), &coarse).unwrap();
        let (chord, margin) = (probe.chordal_tolerance, probe.margin);
        assert_eq!(margin, 2.0 * chord + 2.0 * tol.linear);
        assert!(chord > 1e-4 * radius && chord < 1e-2 * radius, "{chord}");
        // Touching in a point: never certain, at any fineness.
        assert!(probe.is_clean(), "{probe}");
        assert!(inspect(&spheres(0.0), k).is_clean());
        // The mesh of a sphere is inside it, so the meshes go through
        // each other by less than the spheres do; and what is measured is
        // how far one mesh reaches beyond the flat triangles of the other,
        // which is less again by a chord or two (measured: an overlap of
        // four chords is certain in most places, of six everywhere, the
        // poles of the spheres being the worst). Eight chords, between
        // the spheres anywhere and between their poles.
        for towards in [v(0.6, 0.0, 0.8), Vec3::Z] {
            let a = primitives::sphere(&Frame::WORLD, radius).unwrap();
            let centre = towards * (2.0 * radius - 8.0 * chord);
            let b = primitives::sphere(&Frame::WORLD.at(centre).unwrap(), radius).unwrap();
            let found = body_self_intersections(&joined(&[&a, &b]), &coarse).unwrap();
            assert!(!found.is_clean(), "an overlap of eight chords: {found}");
            assert_eq!(found.passes, 1);
            for c in found.crossings.iter().filter(|c| c.certain) {
                assert_eq!(c.margin, found.margin);
                assert!(
                    c.depth > found.margin && c.depth <= 1.2 * 8.0 * chord,
                    "{c:?}"
                );
                assert_eq!(c.shells, [Some(0), Some(1)]);
            }
        }
        // A chord and a half: within the margin of two chords, because
        // the meshes are inside the spheres.
        let shallow = spheres(1.5 * chord);
        let found = body_self_intersections(&shallow, &coarse).unwrap();
        assert!(
            found.is_clean(),
            "an overlap of a chord and a half: {found}"
        );
        assert_eq!(found.passes, 1);
        // Looked at again with finer meshes it is certain: the overlap
        // has not changed, the margin has.
        let found = inspect(&shallow, k);
        assert!(!found.is_clean(), "{found}");
        assert!(
            found.passes > 1 && found.chordal_tolerance < chord,
            "{found}"
        );

        // A ball on the end of a cylinder touches it in a point; a ball in
        // a bowl of twice its radius likewise, from inside.
        let post = primitives::cylinder(&Frame::WORLD, radius, 2.0 * radius).unwrap();
        let on_top = Frame::WORLD
            .at(v(0.2 * radius, 0.1 * radius, 3.0 * radius))
            .unwrap();
        let ball = primitives::sphere(&on_top, radius).unwrap();
        let found = inspect(&joined(&[&post, &ball]), k);
        assert!(found.is_clean(), "a ball on a cylinder: {found}");
        // A ball that sinks a tenth of its radius into the end is caught.
        let sunk = Frame::WORLD
            .at(v(0.2 * radius, 0.1 * radius, 2.9 * radius))
            .unwrap();
        let ball = primitives::sphere(&sunk, radius).unwrap();
        assert_caught(&joined(&[&post, &ball]), k, "a ball sunk into a cylinder");

        // A pin in a hole of its own size, not united with the plate: the
        // same cylinder twice, as the wall of the hole and as the side of
        // the pin, facing each other. Meshed alike, their triangles lie
        // in each other's planes, which no approximation brings about:
        // caught, as an overlap of exactly those two faces.
        let plate = boxed(k, [-2.0, -2.0, 0.5], [2.0, 2.0, 1.5]);
        let holed = boolean(&plate, &post, Subtract, &tol).unwrap().body;
        let wall = holed
            .faces
            .iter()
            .position(|f| matches!(f.surface, Surface::Cylinder(_)))
            .unwrap() as u32;
        let side = holed.faces.len() as u32
            + post
                .faces
                .iter()
                .position(|f| matches!(f.surface, Surface::Cylinder(_)))
                .unwrap() as u32;
        let found = assert_caught(&joined(&[&holed, &post]), k, "a pin in its hole");
        for c in found.crossings.iter().filter(|c| c.certain) {
            assert_eq!(c.kind, CrossingKind::CoplanarOverlap, "{found}");
            assert_eq!(c.faces, [wall, side], "{found}");
        }
        // Turned a little about its axis the pin is meshed otherwise: the
        // meshes weave through each other within a chord, and the same
        // defect is only a finding within the margin. (A weakness: see
        // docs/testing.md.)
        let turned = post
            .transformed(&Transform::rotation(Vec3::Z, 0.1).unwrap())
            .unwrap();
        let found = inspect(&joined(&[&holed, &turned]), k);
        assert!(found.is_clean(), "a pin turned in its hole: {found}");
        assert!(found.marginal_crossings > 0, "{found}");
        // United with the plate it is the plate again, and spotless.
        let filled = boolean(&holed, &post, Union, &tol).unwrap().body;
        assert_spotless(&filled, k, "a pin united with its plate");

        // Walls: a tube with a wall a little thicker than the margin
        // (nothing may be seen, with one mesh) and thinner than a chord
        // (the two meshes may cross, within the margin).
        let tube = |wall: f64| {
            let outer = primitives::cylinder(&Frame::WORLD, radius, radius).unwrap();
            let inner = Frame::WORLD.at(v(0.0, 0.0, -radius)).unwrap();
            let bore = primitives::cylinder(&inner, radius - wall, 3.0 * radius).unwrap();
            boolean(&outer, &bore, Subtract, &tol).unwrap().body
        };
        let probe = body_self_intersections(&tube(0.1 * radius), &coarse).unwrap();
        assert!(probe.is_spotless(), "{probe}");
        let thick = tube(1.05 * probe.margin);
        let found = body_self_intersections(&thick, &coarse).unwrap();
        assert!(
            found.is_spotless(),
            "a wall just thicker than the margin: {found}"
        );
        let thin = tube(0.25 * probe.chordal_tolerance);
        assert!(body_self_intersections(&thin, &coarse).unwrap().is_clean());
        assert!(inspect(&thin, k).is_clean());
    }
}

// --------------------------------------------- what the booleans return

#[derive(Default)]
struct Tally {
    bodies: usize,
    spotless: usize,
    /// Bodies with findings within the margin only.
    marginal: usize,
    refused: usize,
    empty: usize,
    /// What was found for certain: the bugs.
    caught: Vec<String>,
}

impl Tally {
    /// A boolean and, when it gives a body, a look at that body.
    fn run(&mut self, a: &Body, b: &Body, op: BooleanOp, k: f64, what: &str) -> Option<Body> {
        let tol = tolerance(k);
        match boolean(a, b, op, &tol) {
            Ok(result) => {
                let what = format!("{what}: {op:?} at scale {k}");
                assert_eq!(check(&result.body, &tol), [], "{what}");
                let found = match body_self_intersections(&result.body, &params(k)) {
                    Ok(found) => found,
                    Err(e) => panic!("{what}: cannot be looked at: {e}"),
                };
                self.bodies += 1;
                if !found.is_clean() {
                    self.caught.push(format!("{what}: {found}"));
                } else if found.is_spotless() {
                    self.spotless += 1;
                } else {
                    self.marginal += 1;
                }
                Some(result.body)
            }
            Err(KernelError::EmptyResult { .. }) => {
                self.empty += 1;
                None
            }
            Err(KernelError::Unsupported { .. }) => {
                self.refused += 1;
                None
            }
            Err(other) => panic!("{what}: {op:?} at scale {k}: {other}"),
        }
    }

    /// All three booleans, both ways round.
    fn all(&mut self, a: &Body, b: &Body, k: f64, what: &str) {
        for op in [Union, Intersect, Subtract] {
            self.run(a, b, op, k, what);
            self.run(b, a, op, k, &format!("{what} (swapped)"));
        }
    }

    fn report(&self, title: &str) {
        println!(
            "{title}: {} bodies, {} spotless, {} with findings within the margin, {} with \
             certain findings; {} empty results, {} refused",
            self.bodies,
            self.spotless,
            self.marginal,
            self.caught.len(),
            self.empty,
            self.refused
        );
        for line in &self.caught {
            println!("{line}");
        }
    }
}

/// The operands of the scenarios in `crates/kernel/tests/boolean.rs` and
/// `crates/testkit/tests/boolean_tess.rs`, every boolean of every pair,
/// both ways round.
#[test]
fn the_boolean_scenarios_give_bodies_that_do_not_cross_themselves() {
    let mut tally = Tally::default();
    for (k, turned) in SCALES.into_iter().flat_map(|k| [(k, false), (k, true)]) {
        // As the scenarios have them, along the axes; and all turned and
        // moved alike, so that nothing is along an axis and what was in
        // one plane is so within rounding only.
        let place = if turned {
            Transform::rotation_about(v(k, 0.0, 2.0 * k), v(0.2, 0.9, -0.3), 1.1)
                .unwrap()
                .then(&Transform::translation(v(0.7 * k, -0.3 * k, 0.2 * k)))
        } else {
            Transform::IDENTITY
        };
        let b = |lo: [f64; 3], hi: [f64; 3]| boxed(k, lo, hi).transformed(&place).unwrap();
        let r = |base: [f64; 3], radius: f64, height: f64| {
            rod(k, base, radius, height).transformed(&place).unwrap()
        };
        let plate2 = b([0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let plate1 = b([0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
        let cube = b([0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let flat = b([0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        let post = r([0.0, 0.0, 0.0], 1.0, 4.0);
        let stub = r([0.0, 0.0, 0.0], 1.0, 2.0);
        let tilt = 0.6_f64;
        let slanted = Frame::new(
            v(0.0, 0.0, 2.0 * k),
            v(tilt.sin(), 0.0, tilt.cos()),
            Vec3::Y,
        )
        .unwrap();
        let above = primitives::block(
            &slanted
                .at(slanted.point(-10.0 * k, -10.0 * k, 0.0))
                .unwrap(),
            20.0 * k,
            20.0 * k,
            20.0 * k,
        )
        .unwrap()
        .transformed(&place)
        .unwrap();
        let pairs: Vec<(&str, Body, Body)> = vec![
            (
                "overlapping boxes",
                plate2.clone(),
                b([1.0, 1.0, 1.0], [6.0, 5.0, 4.0]),
            ),
            (
                "crossing beams",
                b([-3.0, -1.0, -1.0], [3.0, 1.0, 1.0]),
                b([-0.5, -4.0, -0.5], [0.5, 4.0, 0.5]),
            ),
            (
                "a cutter through a plate",
                plate2.clone(),
                r([1.5, 1.25, -1.0], 0.75, 4.5),
            ),
            (
                "a cutter flush with a plate",
                plate2.clone(),
                r([1.5, 1.25, 0.0], 0.75, 2.0),
            ),
            (
                "a blind hole",
                plate2.clone(),
                r([2.0, 1.5, 0.75], 0.5, 5.0),
            ),
            (
                "a blind hole from inside",
                plate1.clone(),
                r([2.0, 1.5, 0.25], 0.5, 3.0),
            ),
            (
                "a round post on a plate",
                plate1.clone(),
                r([2.0, 1.5, 1.0], 0.5, 3.0),
            ),
            (
                "a square post on a plate",
                plate1.clone(),
                b([1.0, 1.0, 1.0], [2.0, 2.0, 4.0]),
            ),
            (
                "boxes sharing a face",
                flat.clone(),
                b([2.0, 0.0, 0.0], [5.0, 2.0, 1.0]),
            ),
            (
                "boxes flush on three sides",
                flat.clone(),
                b([1.0, 0.0, 0.0], [3.0, 3.0, 1.0]),
            ),
            (
                "a notch",
                flat.clone(),
                b([0.5, 0.0, 0.25], [1.5, 1.0, 3.0]),
            ),
            ("a step", flat.clone(), b([1.0, 0.0, 0.5], [2.0, 2.0, 1.0])),
            ("the same box twice", flat.clone(), flat.clone()),
            (
                "a box and a cylinder apart",
                cube.clone(),
                r([10.0, 0.0, 0.0], 1.0, 2.0),
            ),
            (
                "a cylinder inside a box",
                cube.clone(),
                r([2.0, 2.0, 1.0], 1.0, 2.0),
            ),
            (
                "a slab across a cylinder",
                post.clone(),
                b([-3.0, -3.0, 1.0], [3.0, 3.0, 2.5]),
            ),
            (
                "half a cylinder",
                post.clone(),
                b([0.0, -3.0, -1.0], [3.0, 3.0, 5.0]),
            ),
            (
                "a quarter of a cylinder",
                post.clone(),
                b([0.0, 0.0, -1.0], [3.0, 3.0, 5.0]),
            ),
            (
                "a segment of a cylinder",
                post.clone(),
                b([0.5, -3.0, -1.0], [3.0, 3.0, 5.0]),
            ),
            (
                "a box poking into a cylinder",
                post.clone(),
                b([0.5, -0.25, 1.0], [3.0, 0.25, 2.0]),
            ),
            (
                "a slot through a cylinder",
                post.clone(),
                b([-3.0, -0.25, 1.0], [3.0, 0.25, 2.0]),
            ),
            (
                "two cylinders a radius apart",
                stub.clone(),
                r([1.0, 0.0, 0.0], 1.0, 2.0),
            ),
            (
                "a cylinder and a longer one on it",
                stub.clone(),
                r([0.0, 0.0, 0.5], 1.0, 4.0),
            ),
            (
                "a bore through a cylinder",
                stub.clone(),
                r([0.0, 0.0, -1.0], 0.5, 4.0),
            ),
            (
                "a pin for a plate",
                b([-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]),
                stub.clone(),
            ),
            ("a cylinder cut at a slant", post.clone(), above),
        ];
        for (name, a, b) in &pairs {
            tally.all(a, b, k, name);
        }

        // Scenarios that build on a result.
        let plate = b([-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]);
        let holed = tally.run(&plate, &stub, Subtract, k, "a hole").unwrap();
        tally.all(&holed, &stub, k, "a pin in its hole");
        let cut = tally
            .run(
                &post,
                &pairs.last().unwrap().2,
                Subtract,
                k,
                "a slanted cut",
            )
            .unwrap();
        tally.all(
            &cut,
            &b([-0.25, -3.0, -1.0], [0.25, 3.0, 5.0]),
            k,
            "a slab through the cut",
        );
        let holed = tally
            .run(
                &plate1,
                &r([2.0, 1.5, 0.0], 0.5, 1.0),
                Subtract,
                k,
                "a hole",
            )
            .unwrap();
        let wide = r([2.0, 1.5, 1.0], 1.25, 2.0);
        tally.all(&holed, &wide, k, "a post over a hole");
        let capped = tally
            .run(&holed, &wide, Union, k, "a post over a hole")
            .unwrap();
        tally.run(&capped, &wide, Subtract, k, "the post taken away again");
        tally.all(
            &holed,
            &r([2.0, 1.5, -1.0], 0.25, 3.0),
            k,
            "a pin through a hole",
        );
        // A plate drilled nine times, cutters through and cutters flush.
        for (z, h) in [(-0.5, 2.0), (0.0, 1.0)] {
            let mut drilled = b([0.0, 0.0, 0.0], [6.0, 6.0, 1.0]);
            for i in 0..3 {
                for j in 0..3 {
                    let cutter = r([2.0 * i as f64 + 1.0, 2.0 * j as f64 + 1.0, z], 0.5, h);
                    drilled = tally
                        .run(&drilled, &cutter, Subtract, k, "drilling")
                        .unwrap();
                }
            }
        }
    }
    tally.report("boolean scenarios");
    // The run means something only when most booleans gave a body.
    assert!(tally.bodies > 800, "{}", tally.bodies);
    assert!(tally.caught.is_empty(), "{:#?}", tally.caught);
}

/// How a random pair is placed; as in the kernel's own random run.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
    /// Each operand turned and moved at random.
    General,
    /// Both turned the same way: parallel axes and faces.
    Parallel,
    /// On a grid of halves along the world axes: flush faces, touching
    /// faces, shared edges.
    Grid,
}

fn random_frame(rng: &mut Rng, placement: Placement, shared: &Frame, k: f64) -> Frame {
    let any = |rng: &mut Rng| {
        v(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        )
    };
    match placement {
        Placement::General => {
            let origin = any(rng) * (1.5 * k);
            loop {
                if let Some(frame) = Frame::new(origin, any(rng), any(rng)) {
                    return frame;
                }
            }
        }
        Placement::Parallel => shared
            .at(shared.point(0.0, 0.0, 0.0) + any(rng) * (1.5 * k))
            .unwrap(),
        Placement::Grid => {
            let half = |rng: &mut Rng| (rng.below(5) as f64 - 2.0) * 0.5 * k;
            let origin = v(half(rng), half(rng), half(rng));
            match rng.below(6) {
                0 => Frame::new(origin, Vec3::X, Vec3::Y).unwrap(),
                1 => Frame::new(origin, Vec3::Y, Vec3::Z).unwrap(),
                _ => Frame::WORLD.at(origin).unwrap(),
            }
        }
    }
}

/// A block, a cylinder, an L or a slot with a hole.
fn random_shape(rng: &mut Rng, placement: Placement, shared: &Frame, k: f64) -> Body {
    let frame = random_frame(rng, placement, shared, k);
    let size = |rng: &mut Rng| {
        if placement == Placement::Grid {
            (rng.below(5) as f64 + 1.0) * 0.5 * k
        } else {
            rng.between(0.5, 3.0) * k
        }
    };
    let (a, b, c) = (size(rng), size(rng), size(rng));
    match rng.below(6) {
        0 | 1 => primitives::block(&frame, a, b, c).unwrap(),
        2 | 3 => primitives::cylinder(&frame, a * 0.5, c).unwrap(),
        4 => l_shape(&frame, a, b, c, k),
        _ => slot(&frame, a, b, c, k),
    }
}

fn random_run(pairs_per_cell: u64, first_seed: u64) -> Tally {
    let mut total = Tally::default();
    for k in SCALES {
        for placement in [Placement::General, Placement::Parallel, Placement::Grid] {
            let mut tally = Tally::default();
            for seed in first_seed..first_seed + pairs_per_cell {
                let mut rng = Rng(seed);
                let shared = random_frame(&mut rng, Placement::General, &Frame::WORLD, k);
                let a = random_shape(&mut rng, placement, &shared, k);
                let b = random_shape(&mut rng, placement, &shared, k);
                let what = format!("seed {seed} {placement:?}");
                tally.all(&a, &b, k, &what);
                // What two results give together: their pieces meet in
                // faces that lie on each other.
                let tol = tolerance(k);
                if let (Ok(less), Ok(common)) = (
                    boolean(&a, &b, Subtract, &tol),
                    boolean(&a, &b, Intersect, &tol),
                ) {
                    tally.run(
                        &less.body,
                        &common.body,
                        Union,
                        k,
                        &format!("{what}, put together"),
                    );
                }
            }
            tally.report(&format!("scale {k:e} {placement:?}"));
            assert!(
                tally.bodies as u64 >= pairs_per_cell,
                "{placement:?} at {k}: only {} bodies",
                tally.bodies
            );
            total.bodies += tally.bodies;
            total.spotless += tally.spotless;
            total.marginal += tally.marginal;
            total.refused += tally.refused;
            total.empty += tally.empty;
            total.caught.extend(tally.caught);
        }
    }
    total.report("random booleans");
    total
}

/// Seeded random pairs of blocks, cylinders, L shapes and slots, in three
/// placements at three scales: every body a boolean returns is free of
/// certain self-intersections. A finding prints the seed, the placement
/// and the scale; `one_pair` below reproduces it.
#[test]
fn random_booleans_give_bodies_that_do_not_cross_themselves() {
    let total = random_run(40, 1);
    assert!(total.caught.is_empty(), "{:#?}", total.caught);
}

/// The long run: `cargo test --release -p craft3d-testkit --test
/// self_intersection -- --ignored --nocapture random_booleans_long_run`.
#[test]
#[ignore = "thousands of pairs; run in release"]
fn random_booleans_long_run() {
    let total = random_run(1000, 1_000_000);
    assert!(total.caught.is_empty(), "{:#?}", total.caught);
}

/// One pair, to look at: `SELF_PAIR="seed placement scale"` with the
/// placement as 0 (general), 1 (parallel) or 2 (grid).
#[test]
#[ignore = "a tool for reproducing one pair of a run"]
fn one_pair() {
    let spec = std::env::var("SELF_PAIR").unwrap_or_default();
    let fields: Vec<f64> = spec
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    let [seed, placement, k] = fields.as_slice() else {
        panic!("SELF_PAIR needs a seed, a placement and a scale");
    };
    let placement = [Placement::General, Placement::Parallel, Placement::Grid][*placement as usize];
    let mut rng = Rng(*seed as u64);
    let shared = random_frame(&mut rng, Placement::General, &Frame::WORLD, *k);
    let a = random_shape(&mut rng, placement, &shared, *k);
    let b = random_shape(&mut rng, placement, &shared, *k);
    let mut tally = Tally::default();
    tally.all(&a, &b, *k, &format!("seed {seed} {placement:?}"));
    tally.report("one pair");
}
