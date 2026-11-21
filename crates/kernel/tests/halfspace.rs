//! A solid cut by a plane, one side kept (`craft3d_kernel::halfspace`,
//! docs/design/plane-cut.md), at three scales with the tolerance scaled
//! along. Every body built is held to a closed form (volume, area, and
//! for the corner the centroid), to `topo::check`, to the boolean's own
//! check in full against the block it was cut with, and to the sources
//! the kernel promises (every face a piece of the body's or of the
//! plane). The contact cases (a plane through vertices, along edges, in a
//! face, touching, tangent, missing, a tolerance from a face) are held to
//! what each must give: the body whole, nothing, a cut, or the boolean's
//! refusal. Membership of seeded points, which is a dot product for a
//! half-space, is an oracle independent of the boolean.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::{Plane, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, BooleanOptions, Operand, contains_points};
use craft3d_kernel::halfspace::{CAP, Kept, Tool, keep_below, tool};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Vec3};
use craft3d_testkit::solids;
use craft3d_topo::{Body, check};
use std::f64::consts::PI;

/// The plane through `at` (times `k`) square to `normal`, which points to
/// the side cut away.
fn plane(k: f64, at: [f64; 3], normal: [f64; 3]) -> Frame {
    Frame::from_z(
        Vec3::new(at[0] * k, at[1] * k, at[2] * k),
        Vec3::new(normal[0], normal[1], normal[2]),
    )
    .unwrap()
}

/// The same plane keeping the other side.
fn turned(plane: &Frame) -> Frame {
    plane.reversed_at(plane.origin()).unwrap()
}

fn cut(body: &Body, plane: &Frame, k: f64) -> Result<Kept, KernelError> {
    keep_below(body, plane, &tolerance(k), &BooleanOptions::default())
}

/// What the plane leaves of the body, which must be something: the body
/// itself when nothing is cut, else the cut body, checked.
#[track_caller]
fn below(body: &Body, plane: &Frame, k: f64) -> Body {
    let tol = tolerance(k);
    match cut(body, plane, k) {
        Ok(Kept::Whole) => body.clone(),
        Ok(Kept::Cut(result)) => {
            let result = *result;
            assert_eq!(check(&result.body, &tol), [], "scale {k}");
            assert_eq!(result.faces.len(), result.body.faces.len());
            assert_eq!(result.merged.len(), result.body.faces.len());
            assert_eq!(result.edges.len(), result.body.edges.len());
            assert_eq!(result.vertices.len(), result.body.vertices.len());
            // Every face is a piece of the body's or of the plane, and a
            // piece of the plane lies on the plane as given, bit for bit,
            // facing the side cut away.
            let mut caps = 0usize;
            for (index, source) in result.faces.iter().enumerate() {
                if source.operand == Operand::B {
                    assert_eq!(source.face, CAP, "scale {k}: face {index}");
                    let face = &result.body.faces[index];
                    assert_eq!(face.surface, Surface::Plane(Plane::new(*plane)));
                    assert!(!face.reversed, "scale {k}: face {index}");
                    caps += 1;
                }
            }
            let merged = result
                .merged
                .iter()
                .flatten()
                .filter(|s| s.operand == Operand::B)
                .inspect(|s| assert_eq!(s.face, CAP))
                .count();
            assert!(caps + merged > 0, "scale {k}: a cut without the plane");
            let Ok(Tool::Block(block)) = tool(body, plane, &tol) else {
                panic!("scale {k}: a cut without a block");
            };
            verified(
                body,
                &block,
                BooleanOp::Intersect,
                &result.body,
                &tol,
                &format!("scale {k}"),
            );
            result.body
        }
        Err(e) => panic!("scale {k}: {e}"),
    }
}

#[track_caller]
fn assert_whole(body: &Body, plane: &Frame, k: f64, what: &str) {
    assert_eq!(cut(body, plane, k), Ok(Kept::Whole), "{what} at scale {k}");
}

#[track_caller]
fn assert_nothing(body: &Body, plane: &Frame, k: f64, what: &str) {
    match cut(body, plane, k) {
        Err(KernelError::EmptyResult { .. }) => {}
        other => panic!("{what} at scale {k}: expected nothing left, got {other:?}"),
    }
}

/// Volume and area, given for scale 1.
#[track_caller]
fn assert_measures(body: &Body, volume: f64, area: f64, k: f64, what: &str) {
    let m = mass(body);
    assert_close(m.volume, volume * k * k * k, &format!("{what}: volume"));
    assert_close(m.area, area * k * k, &format!("{what}: area"));
}

fn slab(k: f64) -> Body {
    boxed(k, [0.0, 0.0, 0.0], [40.0, 30.0, 20.0])
}

const SLAB_VOLUME: f64 = 24_000.0;
const SLAB_AREA: f64 = 5_200.0;

/// The corner at (40, 0, 20) cut off through the points 10 along each
/// of its edges: a tetrahedron of 1000/6, three right triangles of 50
/// replaced by an equilateral one of side 10 √2.
#[test]
fn a_corner_cut_off_a_block() {
    for k in SCALES {
        let body = slab(k);
        let through = plane(k, [30.0, 0.0, 20.0], [1.0, -1.0, 1.0]);
        let slant = 50.0 * 3f64.sqrt();
        let tip = 1000.0 / 6.0;
        let kept = below(&body, &through, k);
        assert_measures(
            &kept,
            SLAB_VOLUME - tip,
            SLAB_AREA - 150.0 + slant,
            k,
            "the block",
        );
        assert_eq!(kept.faces.len(), 7, "scale {k}");
        let c = mass(&kept).centroid.unwrap();
        let rest = SLAB_VOLUME - tip;
        let want = [
            (SLAB_VOLUME * 20.0 - tip * 37.5) / rest,
            (SLAB_VOLUME * 15.0 - tip * 2.5) / rest,
            (SLAB_VOLUME * 10.0 - tip * 17.5) / rest,
        ];
        for (got, want) in [c.x, c.y, c.z].into_iter().zip(want) {
            assert_close(got, want * k, "the block's centroid");
        }
        let off = below(&body, &turned(&through), k);
        assert_measures(&off, tip, 150.0 + slant, k, "the corner");
        assert_eq!(off.faces.len(), 4, "scale {k}");
        let c = mass(&off).centroid.unwrap();
        for (got, want) in [c.x, c.y, c.z].into_iter().zip([37.5, 2.5, 17.5]) {
            assert_close(got, want * k, "the corner's centroid");
        }
    }
}

/// A plane through three vertices, and one through two opposite edges:
/// nothing is matched by position, the vertices and edges are shared.
#[test]
fn a_plane_through_vertices_and_along_edges() {
    for k in SCALES {
        let body = slab(k);
        // Through (40, 0, 0), (0, 30, 0) and (0, 0, 20).
        let through = plane(k, [40.0, 0.0, 0.0], [3.0, 4.0, 6.0]);
        let slant = 0.5 * (600f64 * 600.0 + 800.0 * 800.0 + 1200.0 * 1200.0).sqrt();
        let near = below(&body, &through, k);
        assert_measures(&near, 4000.0, 1300.0 + slant, k, "the corner at the origin");
        assert_eq!(
            (near.vertices.len(), near.edges.len(), near.faces.len()),
            (4, 6, 4),
            "scale {k}"
        );
        let far = below(&body, &turned(&through), k);
        assert_measures(
            &far,
            SLAB_VOLUME - 4000.0,
            SLAB_AREA - 1300.0 + slant,
            k,
            "the block less that corner",
        );
        // Through the edges x = 0, z = 0 and x = 40, z = 20.
        let diagonal = plane(k, [0.0, 0.0, 0.0], [-1.0, 0.0, 2.0]);
        let wedge = 1200.0 + 600.0 + 800.0 + 30.0 * (40f64 * 40.0 + 20.0 * 20.0).sqrt();
        for side in [diagonal, turned(&diagonal)] {
            let half = below(&body, &side, k);
            assert_measures(&half, SLAB_VOLUME / 2.0, wedge, k, "a wedge");
            assert_eq!(
                (half.vertices.len(), half.edges.len(), half.faces.len()),
                (6, 9, 5),
                "scale {k}"
            );
        }
        // Through the vertex (40, 30, 20) and across: the corner at
        // (40, 0, 20) with the points (20, 0, 20) and (40, 0, 10).
        let across = plane(k, [20.0, 0.0, 20.0], [3.0, -2.0, 6.0]);
        let kept = below(&body, &across, k);
        assert_close(
            mass(&kept).volume,
            (SLAB_VOLUME - 1000.0) * k * k * k,
            "through a vertex and across",
        );
        let off = below(&body, &turned(&across), k);
        assert_close(mass(&off).volume, 1000.0 * k * k * k, "the piece cut off");
    }
}

/// A plane that only touches the body, lies in a face of it or misses it
/// cuts nothing: the body whole on one side, nothing on the other.
#[test]
fn a_plane_that_touches_or_misses_cuts_nothing() {
    for k in SCALES {
        let body = slab(k);
        for (what, at, normal) in [
            ("along an edge", [40.0, 0.0, 20.0], [1.0, 0.0, 1.0]),
            ("at a vertex", [40.0, 30.0, 20.0], [1.0, 1.0, 1.0]),
            ("in the top face", [0.0, 0.0, 20.0], [0.0, 0.0, 1.0]),
            (
                "0.4 tolerances under the top face",
                [0.0, 0.0, 20.0 - 0.4e-6],
                [0.0, 0.0, 1.0],
            ),
            ("above the body", [0.0, 0.0, 25.0], [0.0, 0.0, 1.0]),
            ("beside the body, tilted", [80.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
        ] {
            let touching = plane(k, at, normal);
            assert_whole(&body, &touching, k, what);
            assert_nothing(&body, &turned(&touching), k, what);
        }
        let rod = rod(k, [0.0, 0.0, 0.0], 10.0, 30.0);
        let tangent = plane(k, [10.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert_whole(&rod, &tangent, k, "tangent to a cylinder");
        assert_nothing(&rod, &turned(&tangent), k, "tangent to a cylinder");
        let ball = ball(k, [0.0, 0.0, 0.0], 10.0);
        let tangent = plane(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
        assert_whole(&ball, &tangent, k, "tangent to a ball");
        assert_nothing(&ball, &turned(&tangent), k, "tangent to a ball");
    }
}

/// A plane between half and twice the tolerance from a face it is over is
/// neither that face nor safely another: refused on both sides, as the
/// boolean refuses it. Three tolerances away it cuts.
#[test]
fn a_plane_a_tolerance_from_a_face_is_refused() {
    for k in SCALES {
        let body = slab(k);
        let near = plane(k, [0.0, 0.0, 20.0 - 1e-6], [0.0, 0.0, 1.0]);
        for side in [near, turned(&near)] {
            match cut(&body, &side, k) {
                Err(KernelError::Unsupported { reason, .. }) => {
                    assert!(reason.contains("at the edge of tolerance"), "{reason}");
                }
                other => panic!("scale {k}: expected a refusal, got {other:?}"),
            }
        }
        let clear = plane(k, [0.0, 0.0, 20.0 - 3e-6], [0.0, 0.0, 1.0]);
        let kept = below(&body, &clear, k);
        assert_close(
            mass(&kept).volume,
            1200.0 * (20.0 - 3e-6) * k * k * k,
            "three tolerances under the top",
        );
        // A skin three tolerances thick: its volume is a difference of
        // sums seven digits larger, so it is held to a millionth.
        let skin = mass(&below(&body, &turned(&clear), k)).volume;
        let want = 1200.0 * 3e-6 * k * k * k;
        assert!(
            (skin - want).abs() <= 1e-6 * want,
            "scale {k}: the skin cut off is {skin}, not {want}"
        );
    }
}

/// A cylinder across at half its radius (a segment), through its axis
/// and tilted; a ball's cap.
#[test]
fn round_bodies_cut_to_their_closed_forms() {
    for k in SCALES {
        let (r, h) = (10.0, 30.0);
        let rod = rod(k, [0.0, 0.0, 0.0], r, h);
        let disc = PI * r * r;
        // acos(1/2) is a third of π.
        let segment = r * r * PI / 3.0 - 5.0 * 75f64.sqrt();
        let across = plane(k, [5.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let kept = below(&rod, &across, k);
        assert_close(
            mass(&kept).volume,
            (disc - segment) * h * k * k * k,
            "a cylinder less a segment",
        );
        let off = below(&rod, &turned(&across), k);
        assert_close(mass(&off).volume, segment * h * k * k * k, "the segment");
        let chord = 2.0 * 75f64.sqrt();
        assert_close(
            mass(&off).area,
            (2.0 * segment + chord * h + r * (2.0 * PI / 3.0) * h) * k * k,
            "the segment's area",
        );

        let axial = plane(k, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        for side in [axial, turned(&axial)] {
            let half = below(&rod, &side, k);
            assert_measures(
                &half,
                disc * h / 2.0,
                disc + PI * r * h + 2.0 * r * h,
                k,
                "half a cylinder",
            );
        }

        // z = 15 + x / 2: the mean height below it is 15, above it 15.
        let tilted = plane(k, [0.0, 0.0, 15.0], [-1.0, 0.0, 2.0]);
        for side in [tilted, turned(&tilted)] {
            let part = below(&rod, &side, k);
            assert_close(
                mass(&part).volume,
                disc * 15.0 * k * k * k,
                "a cylinder cut on the slant",
            );
        }

        let ball = ball(k, [0.0, 0.0, 0.0], r);
        let level = plane(k, [0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
        let whole = 4.0 / 3.0 * PI * r * r * r;
        let cap = PI * 25.0 * (3.0 * r - 5.0) / 3.0;
        let kept = below(&ball, &level, k);
        assert_close(
            mass(&kept).volume,
            (whole - cap) * k * k * k,
            "a ball less a cap",
        );
        let off = below(&ball, &turned(&level), k);
        assert_measures(&off, cap, 2.0 * PI * r * 5.0 + PI * 75.0, k, "a ball's cap");
    }
}

/// A ring halved by the plane it lies in, and by a plane through its
/// axis, which leaves two lumps in one body.
#[test]
fn a_ring_halved() {
    for k in SCALES {
        let (major, minor) = (10.0, 3.0);
        let ring = ring(k, [0.0, 0.0, 0.0], major, minor);
        let half = PI * PI * major * minor * minor;
        let flat = plane(k, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        for side in [flat, turned(&flat)] {
            let part = below(&ring, &side, k);
            assert_close(mass(&part).volume, half * k * k * k, "half a ring, flat");
            assert_eq!(part.shells.len(), 1, "scale {k}");
        }
        let upright = plane(k, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        for side in [upright, turned(&upright)] {
            let part = below(&ring, &side, k);
            assert_close(mass(&part).volume, half * k * k * k, "half a ring, upright");
            assert_eq!(part.shells.len(), 1, "scale {k}");
        }
        // A second plane through the axis, square to the first, leaves a
        // quarter.
        let second = plane(k, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let c_shape = below(&ring, &upright, k);
        let quarter = below(&c_shape, &second, k);
        assert_close(
            mass(&quarter).volume,
            half / 2.0 * k * k * k,
            "a quarter of a ring",
        );
        // A plane across the tube away from the hole (the hole's radius
        // is 7): a cap on one side, the ring with a flat on the other.
        let chord = plane(k, [0.0, 9.0, 0.0], [0.0, 1.0, 0.0]);
        let cap = below(&ring, &turned(&chord), k);
        let rest = below(&ring, &chord, k);
        assert_close(
            mass(&cap).volume + mass(&rest).volume,
            2.0 * half * k * k * k,
            "the two sides of a ring",
        );
        assert_eq!((cap.shells.len(), rest.shells.len()), (1, 1), "scale {k}");
        // The half with x < 0 cut again at x = -5, through the hole: the
        // slab between the planes holds two lumps, one body; with what
        // lies beyond it they are the half.
        let inner = plane(k, [-5.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let lumps = below(&c_shape, &turned(&inner), k);
        let beyond = below(&c_shape, &inner, k);
        assert_eq!(
            (lumps.shells.len(), beyond.shells.len()),
            (2, 1),
            "scale {k}"
        );
        assert_close(
            mass(&lumps).volume + mass(&beyond).volume,
            half * k * k * k,
            "the two sides of half a ring",
        );
    }
}

/// A plane through the hole between the two lumps of a body meets
/// neither, and still takes the one above it away: no face of what is
/// left is a piece of the plane, and what is left is not the body. The
/// two lumps are mirror images, so what is left is half the body.
#[test]
fn a_plane_between_two_lumps_takes_the_one_above_it_away() {
    for k in SCALES {
        let ring = ring(k, [0.0, 0.0, 0.0], 10.0, 3.0);
        let upright = plane(k, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let inner = plane(k, [-5.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let lumps = below(&below(&ring, &upright, k), &turned(&inner), k);
        assert_eq!(lumps.shells.len(), 2, "scale {k}");
        let both = mass(&lumps).volume;
        let between = plane(k, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        for side in [between, turned(&between)] {
            let kept = match cut(&lumps, &side, k) {
                Ok(Kept::Cut(result)) => {
                    let result = *result;
                    result.body
                }
                Ok(Kept::Whole) => {
                    panic!("scale {k}: both lumps kept, the one above the plane too")
                }
                Err(e) => panic!("scale {k}: {e}"),
            };
            assert_eq!(check(&kept, &tolerance(k)), [], "scale {k}");
            assert_eq!(kept.shells.len(), 1, "scale {k}");
            let one = mass(&kept).volume;
            assert!(
                (one - both / 2.0).abs() <= 1e-7 * both,
                "scale {k}: {one} is not half of {both}"
            );
            let c = mass(&kept).centroid.unwrap();
            assert!(c.y * side.z().y < 0.0, "scale {k}: the lump kept is above");
        }
    }
}

/// A block of 40 × 30 × 20 with a closed hollow of 20 × 10 × 10 in its
/// middle: two shells. A plane through the hollow opens it: one shell,
/// the cut face a ring, either side the same by symmetry. A plane over
/// the hollow, through solid only, leaves the hollow whole under it (two
/// shells) and a plain slab over it.
#[test]
fn a_body_with_a_cavity_is_cut_through_it_and_past_it() {
    for k in SCALES {
        let hollow = boxed(k, [10.0, 10.0, 5.0], [30.0, 20.0, 15.0]);
        let body = solve(&slab(k), &hollow, BooleanOp::Subtract, k).body;
        assert_eq!(body.shells.len(), 2, "scale {k}");
        assert_measures(&body, SLAB_VOLUME - 2000.0, SLAB_AREA + 1000.0, k, "hollow");
        // Through the middle of the hollow: 40 × 30 × 10 less 20 × 10 × 5;
        // the bottom, the walls, the ring (1200 − 200), and of the hollow
        // its floor and half its walls.
        let across = plane(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
        for side in [across, turned(&across)] {
            let half = below(&body, &side, k);
            assert_eq!(half.shells.len(), 1, "scale {k}");
            assert_measures(
                &half,
                11_000.0,
                1200.0 + 1400.0 + 1000.0 + 200.0 + 300.0,
                k,
                "through the hollow",
            );
        }
        // Over the hollow, at 17.
        let over = plane(k, [0.0, 0.0, 17.0], [0.0, 0.0, 1.0]);
        let low = below(&body, &over, k);
        assert_eq!(low.shells.len(), 2, "scale {k}");
        assert_measures(
            &low,
            40.0 * 30.0 * 17.0 - 2000.0,
            2400.0 + 140.0 * 17.0 + 1000.0,
            k,
            "under a plane over the hollow",
        );
        let high = below(&body, &turned(&over), k);
        assert_eq!(high.shells.len(), 1, "scale {k}");
        assert_measures(&high, 3600.0, 2400.0 + 140.0 * 3.0, k, "the slab over it");
        // On the slant through the hollow: the two sides add up.
        let slant = plane(k, [20.0, 15.0, 10.0], [0.3, -0.2, 1.0]);
        let (under, above) = (below(&body, &slant, k), below(&body, &turned(&slant), k));
        assert_close(
            mass(&under).volume + mass(&above).volume,
            (SLAB_VOLUME - 2000.0) * k * k * k,
            "the two sides of a slant through the hollow",
        );
    }
}

/// Two lumps, 10 × 10 × 10 each and 10 apart, the right one with a closed
/// hollow of 6 × 6 × 6: three shells. A plane between them takes one lump
/// away whole, with its hollow when it is the right one, and no face of
/// what is left is a piece of the plane. A plane through the right lump's
/// hollow leaves the left lump as it was and opens the hollow.
#[test]
fn a_plane_between_two_lumps_takes_a_lump_with_its_cavity() {
    for k in SCALES {
        let bar = boxed(k, [0.0, 0.0, 0.0], [30.0, 10.0, 10.0]);
        let hollow = boxed(k, [22.0, 2.0, 2.0], [28.0, 8.0, 8.0]);
        let gap = boxed(k, [10.0, -1.0, -1.0], [20.0, 11.0, 11.0]);
        let hollowed = solve(&bar, &hollow, BooleanOp::Subtract, k).body;
        let lumps = solve(&hollowed, &gap, BooleanOp::Subtract, k).body;
        assert_eq!(lumps.shells.len(), 3, "scale {k}");
        assert_measures(&lumps, 2000.0 - 216.0, 1200.0 + 216.0, k, "two lumps");
        let between = plane(k, [15.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        for (side, shells, volume, area) in [
            (between, 1, 1000.0, 600.0),
            (turned(&between), 2, 784.0, 816.0),
        ] {
            let kept = match cut(&lumps, &side, k) {
                Ok(Kept::Cut(result)) => {
                    let result = *result;
                    assert!(
                        result.faces.iter().all(|s| s.operand == Operand::A),
                        "scale {k}: a face of the plane where it touches nothing"
                    );
                    result.body
                }
                Ok(Kept::Whole) => panic!("scale {k}: both lumps kept"),
                Err(e) => panic!("scale {k}: {e}"),
            };
            assert_eq!(check(&kept, &tolerance(k)), [], "scale {k}");
            assert_eq!(kept.shells.len(), shells, "scale {k}");
            assert_measures(&kept, volume, area, k, "one lump of two");
        }
        // Through the hollow, at x = 25: the left lump whole, and of the
        // right one 5 × 10 × 10 less 3 × 6 × 6, the hollow open to the cut.
        let through = plane(k, [25.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let kept = below(&lumps, &through, k);
        assert_eq!(kept.shells.len(), 2, "scale {k}");
        assert_measures(
            &kept,
            1000.0 + 500.0 - 108.0,
            600.0 + (100.0 + 200.0 + 64.0) + (36.0 + 72.0),
            k,
            "a lump whole and a lump cut through its hollow",
        );
    }
}

/// Several planes in turn are the intersection of what each keeps,
/// whatever the order: a gable roof on the block, eaves at 12 and the
/// ridge at 20.
#[test]
fn planes_in_turn_keep_what_every_one_keeps() {
    for k in SCALES {
        let body = slab(k);
        let south = plane(k, [0.0, 0.0, 12.0], [0.0, -8.0, 15.0]);
        let north = plane(k, [40.0, 30.0, 12.0], [0.0, 8.0, 15.0]);
        for order in [[south, north], [north, south]] {
            let [first, second] = order;
            let roofed = below(&below(&body, &first, k), &second, k);
            assert_measures(&roofed, 19_200.0, 4480.0, k, "the roofed block");
            assert_eq!(roofed.faces.len(), 7, "scale {k}");
        }
        // The same plane again cuts nothing.
        let once = below(&body, &south, k);
        assert_whole(&once, &south, k, "a plane given twice");
    }
}

/// The two sides of a plane across a body with a spline face add up to
/// the body, and each is valid: the cone a spline sweeps, cut on the
/// slant away from its apex and its base.
#[test]
fn the_two_sides_of_a_freeform_body_add_up() {
    for k in SCALES {
        let frame = Frame::WORLD;
        let cone = solids::spline_cone(&frame, 4.0 * k, 8.0 * k).unwrap();
        let through = plane(k, [0.0, 4.0, 0.0], [0.3, 1.0, 0.2]);
        let tip = below(&cone.body, &through, k);
        let base = below(&cone.body, &turned(&through), k);
        let (a, b) = (mass(&tip), mass(&base));
        let whole = mass(&cone.body);
        // The curve where the plane meets the spline face lies on both
        // within the tolerance: the volumes add up to the tolerance times
        // the area, far inside a millionth.
        let sum = a.volume + b.volume;
        assert!(
            (sum - whole.volume).abs() <= 1e-6 * whole.volume,
            "scale {k}: {} + {} is not {}",
            a.volume,
            b.volume,
            whole.volume
        );
        assert!(a.volume > 0.0 && b.volume > 0.0, "scale {k}");
        // Both have the cut face: the areas add up to the body's and
        // twice the section.
        let section = (a.area + b.area - whole.area) / 2.0;
        assert!(section > 0.0, "scale {k}: {section}");
    }
}

/// A point is in what the planes leave exactly when it is in the body
/// and below every plane: a dot product, which no boolean had a hand in.
#[test]
fn seeded_points_are_kept_exactly_when_below_every_plane() {
    for k in SCALES {
        let tol = tolerance(k);
        let cone = solids::spline_cone(&Frame::WORLD, 4.0 * k, 8.0 * k)
            .unwrap()
            .body;
        // The body, the planes in turn, and the box the points are seeded in.
        let cases = [
            (
                slab(k),
                vec![
                    plane(k, [30.0, 0.0, 20.0], [1.0, -1.0, 1.0]),
                    plane(k, [0.0, 0.0, 12.0], [0.0, -8.0, 15.0]),
                    plane(k, [40.0, 30.0, 12.0], [0.0, 8.0, 15.0]),
                ],
                [-1.0, -1.0, -1.0],
                [41.0, 31.0, 21.0],
            ),
            (
                rod(k, [0.0, 0.0, 0.0], 10.0, 30.0),
                vec![
                    plane(k, [0.0, 0.0, 15.0], [-1.0, 0.0, 2.0]),
                    plane(k, [5.0, 0.0, 0.0], [1.0, 0.2, 0.0]),
                ],
                [-11.0, -11.0, -1.0],
                [11.0, 11.0, 31.0],
            ),
            (
                cone,
                vec![plane(k, [0.0, 4.0, 0.0], [0.3, 1.0, 0.2])],
                [-2.5, -0.5, -2.5],
                [2.5, 5.5, 2.5],
            ),
        ];
        for (index, (body, planes, lo, hi)) in cases.into_iter().enumerate() {
            let mut kept = body.clone();
            for through in &planes {
                kept = below(&kept, through, k);
            }
            let mut rng = Rng(0x00C0_FFEE ^ index as u64);
            let points: Vec<Vec3> = (0..600)
                .map(|_| {
                    Vec3::new(
                        rng.between(lo[0], hi[0]) * k,
                        rng.between(lo[1], hi[1]) * k,
                        rng.between(lo[2], hi[2]) * k,
                    )
                })
                .collect();
            let before = contains_points(&body, &points, &tol).unwrap();
            let after = contains_points(&kept, &points, &tol).unwrap();
            let mut told = 0;
            let mut inside = 0;
            for ((point, was), is) in points.iter().zip(before).zip(after) {
                let heights: Vec<f64> = planes
                    .iter()
                    .map(|p| (*point - p.origin()).dot(p.z()))
                    .collect();
                // A point too near a plane to call is left out.
                if heights.iter().any(|h| h.abs() < 1e-3 * k) {
                    continue;
                }
                let (Some(was), Some(is)) = (was, is) else {
                    continue;
                };
                let want = was && heights.iter().all(|h| *h < 0.0);
                assert_eq!(is, want, "case {index} at scale {k}: {point:?}");
                told += 1;
                inside += usize::from(is);
            }
            assert!(told > 500, "case {index} at scale {k}: {told} points told");
            assert!(inside > 20, "case {index} at scale {k}: {inside} inside");
        }
    }
}

/// What the plane x + y = c leaves of a cone of radius and height 7.5 on
/// the origin, on the side away from the axis, in closed form. A level
/// section at the height where the cone's radius is `r` is the segment of
/// a disc beyond a chord `d = c / √2` from its centre: its area is
/// `A(r) = r² acos(d / r) − d √(r² − d²)` and its chord `2 √(r² − d²)`,
/// and the cone's slope is one, so `r` falls as the height rises. With
/// `s = √(r² − d²)`: `∫ A dr = r³ acos(d / r) / 3 − 2 d r s / 3 + d³ ln(r
/// + s) / 3` and `∫ 2 s dr = r s − d² ln(r + s)`. The cut ends in the
/// hyperbola's tip, where `r = d`.
struct HalfCone {
    d: f64,
}

impl HalfCone {
    fn section(&self, r: f64) -> f64 {
        let s = (r * r - self.d * self.d).max(0.0).sqrt();
        r * r * (self.d / r).min(1.0).pacos() - self.d * s
    }

    fn sections_to(&self, r: f64) -> f64 {
        let (d, s) = (self.d, (r * r - self.d * self.d).max(0.0).sqrt());
        r * r * r * (d / r).min(1.0).pacos() / 3.0 - 2.0 * d * r * s / 3.0
            + d * d * d * (r + s).pln() / 3.0
    }

    fn chords_to(&self, r: f64) -> f64 {
        let (d, s) = (self.d, (r * r - self.d * self.d).max(0.0).sqrt());
        r * s - d * d * (r + s).pln()
    }

    /// The volume and the area of the part between the levels where the
    /// cone's radius is `top` and `bottom`: its two level faces, the
    /// cone's piece (√2 times what it covers of the base) and the flat
    /// piece in the plane.
    fn between(&self, top: f64, bottom: f64) -> (f64, f64) {
        let (over, under) = (self.section(top), self.section(bottom));
        (
            self.sections_to(bottom) - self.sections_to(top),
            under + over + 2f64.sqrt() * (under - over) + self.chords_to(bottom)
                - self.chords_to(top),
        )
    }
}

fn counts_of(body: &Body) -> (usize, usize, usize) {
    (body.vertices.len(), body.edges.len(), body.faces.len())
}

/// Fuzz seed 118385 (2026-11-20), reduced. A cone halved by a plane that
/// passes its axis by a thousandth of its size and less: the section is a
/// hyperbola whose tip is that far from the apex, and the boolean makes
/// its edge a rational spline whose parameter crawls about the tip. A
/// level plane across what is left crosses that edge twice. From half the
/// cone's height up, the crossing on the way up the first arm was found
/// and then not found again on the edge (the nearest point of the edge
/// to it was answered as the tip, 3.5 away: `outline::nearest_on_spline`
/// ran out of stretches), so the circle and the line the level plane cuts
/// were never imprinted and the boolean built a body of two faces, which
/// its own check refused as invalid. Both sides of the level plane, held
/// to the closed forms above, at three scales.
#[test]
fn half_a_cone_cut_beside_its_apex_is_cut_level() {
    for k in SCALES {
        let cone = spike(k, [0.0, 0.0, 0.0], 7.5, 7.5);
        for c in [10.003936261680792e-3, 3e-3, 1e-3] {
            let shape = HalfCone { d: c / 2f64.sqrt() };
            // Its normal towards the axis: what is kept is x + y > c.
            let beside = plane(k, [c, 0.0, 3.75], [-1.0, -1.0, 0.0]);
            let half = below(&cone, &beside, k);
            let (volume, area) = shape.between(shape.d, 7.5);
            assert_measures(&half, volume, area, k, &format!("c {c}: the half"));
            assert_eq!(counts_of(&half), (3, 4, 3), "scale {k}, c {c}");
            for h in [3.0, 3.75, 4.0, 5.0, 7.0] {
                let level = plane(k, [0.0, 0.0, h], [0.0, 0.0, 1.0]);
                let what = format!("c {c}, level {h}");
                let low = below(&half, &level, k);
                let (volume, area) = shape.between(7.5 - h, 7.5);
                assert_measures(&low, volume, area, k, &format!("{what}: under"));
                assert_eq!(counts_of(&low), (5, 7, 4), "scale {k}, {what}");
                let high = below(&half, &turned(&level), k);
                let (volume, area) = shape.between(shape.d, 7.5 - h);
                assert_measures(&high, volume, area, k, &format!("{what}: over"));
                assert_eq!(counts_of(&high), (2, 3, 3), "scale {k}, {what}");
            }
        }
    }
}

/// The same half with the plane nearer the axis, and levels up to just
/// under the hyperbola's tip: each side of each level is built, to the
/// closed forms, or refused as a state that cannot be told, never handed
/// to the boolean's own check as a body it refuses (before the repair of
/// seed 118385 every level from half the height up was, and the levels
/// closest under the tip were refused as *"not the boolean of its
/// operands"*). What is refused today is owed: the winding sum about a
/// point (`chart::turn`) runs out of steps on the spline's one speed
/// bound, as the search for its nearest point did, and says *"a point too
/// close to a face boundary"* of a point that is not. Per distance of the
/// plane from the axis, the highest level that is built at every scale: a
/// level up to it must build, one above it may.
#[test]
fn a_level_cut_near_the_tip_is_built_or_refused_in_words() {
    for k in SCALES {
        let cone = spike(k, [0.0, 0.0, 0.0], 7.5, 7.5);
        for (c, built_to) in [(1e-3, 7.3), (3e-4, 5.0), (1e-4, 1.0)] {
            let shape = HalfCone { d: c / 2f64.sqrt() };
            let beside = plane(k, [c, 0.0, 3.75], [-1.0, -1.0, 0.0]);
            let half = below(&cone, &beside, k);
            for h in [1.0, 3.0, 5.0, 6.5, 7.3, 7.4, 7.48] {
                let level = plane(k, [0.0, 0.0, h], [0.0, 0.0, 1.0]);
                let sides = [
                    (level, "under", shape.between(7.5 - h, 7.5)),
                    (turned(&level), "over", shape.between(shape.d, 7.5 - h)),
                ];
                for (side, which, (volume, area)) in sides {
                    let what = format!("scale {k}, c {c}, level {h}, {which}");
                    match cut(&half, &side, k) {
                        Ok(_) => {
                            let kept = below(&half, &side, k);
                            assert_measures(&kept, volume, area, k, &what);
                        }
                        Err(KernelError::Unsupported { reason, faces, .. }) => {
                            assert!(h > built_to, "{what} was built: {reason}");
                            assert!(
                                reason.starts_with("a point too close to a face boundary"),
                                "{what}: {reason}"
                            );
                            assert!(!faces.is_empty(), "{what}: no face named");
                        }
                        Err(other) => panic!("{what}: {other}"),
                    }
                }
            }
        }
    }
}

/// Fuzz seed 115098 (2026-11-20), reduced. A block with a hole whose wall
/// is tangent to one of its sides (a wall that thins to nothing along a
/// line, which a boolean accepts: `boolean_contact.rs` › *a hole that
/// touches the side*), cut across the hole. The circle the plane cuts
/// from the hole touches the outline it cuts from the block, in a point
/// that is a vertex of neither. Where the circle's middle (half a turn
/// from where it starts) was that point, the hole's loop in the cut face
/// was given to the wrong region (`outline::left_of` stepped across the
/// outline there), the cut face was built without its hole, and the
/// boolean's own check refused the body as invalid. Both sides of the
/// plane, wherever the circle starts, at three scales: a plate of
/// 1 × 10 × 10 less a hole of radius 2.5.
#[test]
fn a_hole_tangent_to_a_side_is_cut_across() {
    for k in SCALES {
        let block = boxed(k, [0.0, 0.0, 0.0], [2.0, 10.0, 10.0]);
        // Where the circles about the hole's axis start, as an angle from
        // the direction away from the side the hole touches: at 0 the
        // middle of a circle is where it touches the side, at half a turn
        // its start is, and at the third its second station.
        for turn in [
            0.0,
            PI,
            PI - 0.2764 * 2.0 * PI,
            0.5 * PI,
            0.6457718232379019,
        ] {
            let (sin, cos) = turn.psin_cos();
            let frame = Frame::new(
                Vec3::new(-2.0, 2.5, 5.0) * k,
                Vec3::X,
                Vec3::new(0.0, cos, sin),
            )
            .unwrap();
            let drill = primitives::cylinder(&frame, 2.5 * k, 6.0 * k).unwrap();
            let holed = solve(&block, &drill, BooleanOp::Subtract, k).body;
            let across = plane(k, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]);
            for (side, which) in [(across, "x > 1"), (turned(&across), "x < 1")] {
                let what = format!("turn {turn}, {which}");
                let kept = below(&holed, &side, k);
                assert_measures(&kept, 100.0 - 6.25 * PI, 240.0 - 7.5 * PI, k, &what);
                assert_eq!(kept.shells.len(), 1, "scale {k}, {what}");
                assert_eq!(counts_of(&kept), (10, 15, 7), "scale {k}, {what}");
            }
        }
    }
}

/// A sheet has no inside: refused, by name.
#[test]
fn a_sheet_is_refused() {
    let mut sheet = slab(1.0);
    sheet.kind = craft3d_topo::BodyKind::Sheet;
    let through = plane(1.0, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
    match cut(&sheet, &through, 1.0) {
        Err(KernelError::Unsupported { reason, .. }) => {
            assert!(reason.contains("a sheet"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}
