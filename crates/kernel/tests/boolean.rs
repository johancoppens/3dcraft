//! Booleans against closed forms (docs/testing.md, oracles 1 and 2): every
//! result passes the checker and has the volume, area and counts that
//! follow from the sizes.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{
    BooleanOp, EdgeSource, Entity, FaceSource, Operand, VertexSource, boolean,
};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{EdgeId, FaceId, VertexId, genus};
use std::f64::consts::PI;

use BooleanOp::{Intersect, Subtract, Union};

#[test]
fn overlapping_boxes() {
    for k in SCALES {
        let a = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let b = boxed(k, [1.0, 1.0, 1.0], [6.0, 5.0, 4.0]);
        // The overlap is 3 x 2 x 1.
        let common = solve(&a, &b, Intersect, k);
        assert_mass(&common, 6.0, 22.0, k);
        assert_eq!(counts(&common), (8, 12, 6, 1));
        let both = solve(&a, &b, Union, k);
        assert_mass(&both, 24.0 + 60.0 - 6.0, 52.0 + 94.0 - 22.0, k);
        assert_eq!(genus(&both.body), Some(0));
        // A with a corner taken out: the area is unchanged.
        let less = solve(&a, &b, Subtract, k);
        assert_mass(&less, 24.0 - 6.0, 52.0, k);
        assert_eq!(counts(&less), (14, 21, 9, 1));
        let other = solve(&b, &a, Subtract, k);
        assert_mass(&other, 60.0 - 6.0, 94.0, k);
    }
}

#[test]
fn crossing_boxes() {
    for k in SCALES {
        // Two beams through each other: a plus sign.
        let a = boxed(k, [-3.0, -1.0, -1.0], [3.0, 1.0, 1.0]);
        let b = boxed(k, [-0.5, -4.0, -0.5], [0.5, 4.0, 0.5]);
        let common = solve(&a, &b, Intersect, k);
        assert_mass(&common, 2.0, 10.0, k);
        let both = solve(&a, &b, Union, k);
        assert_mass(&both, 24.0 + 8.0 - 2.0, 56.0 + 34.0 - 2.0 - 8.0, k);
        assert_eq!(genus(&both.body), Some(0));
        // A beam with a square tunnel through it.
        let less = solve(&a, &b, Subtract, k);
        assert_mass(&less, 22.0, 56.0 - 2.0 + 8.0, k);
        assert_eq!(genus(&less.body), Some(1));
        // The beam B loses its middle and falls into two parts.
        let parts = solve(&b, &a, Subtract, k);
        assert_mass(&parts, 6.0, 2.0 * (2.0 + 4.0 * 3.0), k);
        assert_eq!(parts.body.shells.len(), 2);
        assert_eq!(genus(&parts.body), Some(0));
    }
}

#[test]
fn a_box_minus_a_cylinder_through_it() {
    for k in SCALES {
        let (r, h) = (0.75, 2.0);
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, h]);
        let volume = 24.0 - PI * r * r * h;
        let area = 52.0 - 2.0 * PI * r * r + 2.0 * PI * r * h;
        // A cutter that sticks out on both sides, and one that ends exactly
        // on the plate's faces.
        for (z0, z1) in [(-1.0, 3.5), (0.0, h)] {
            let cutter = rod(k, [1.5, 1.25, z0], r, z1 - z0);
            let holed = solve(&plate, &cutter, Subtract, k);
            assert_mass(&holed, volume, area, k);
            assert_eq!(genus(&holed.body), Some(1));
            assert_eq!(counts(&holed), (10, 14, 7, 1));
            // The wall of the hole is the cutter's, turned inside out.
            let wall: Vec<_> = holed
                .faces
                .iter()
                .zip(&holed.body.faces)
                .filter(|(source, _)| source.operand == Operand::B)
                .collect();
            assert_eq!(wall.len(), 1);
            assert!(matches!(wall[0].1.surface, Surface::Cylinder(_)));
            assert!(wall[0].1.reversed);

            let plug = solve(&plate, &cutter, Intersect, k);
            assert_mass(
                &plug,
                PI * r * r * h,
                2.0 * PI * r * r + 2.0 * PI * r * h,
                k,
            );
            assert_eq!(counts(&plug), (2, 2, 3, 1));
        }
    }
}

#[test]
fn a_blind_hole() {
    for k in SCALES {
        let (r, depth) = (0.5, 1.25);
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let cutter = rod(k, [2.0, 1.5, 2.0 - depth], r, 5.0);
        let holed = solve(&plate, &cutter, Subtract, k);
        assert_mass(
            &holed,
            24.0 - PI * r * r * depth,
            52.0 + 2.0 * PI * r * depth,
            k,
        );
        assert_eq!(genus(&holed.body), Some(0));
        assert_eq!(counts(&holed), (10, 14, 8, 1));
    }
}

#[test]
fn a_post_on_a_plate() {
    for k in SCALES {
        let (r, h) = (0.5, 3.0);
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
        let plate_volume = 12.0;
        let plate_area = 2.0 * (12.0 + 4.0 + 3.0);
        // A round post and a square one, standing exactly on the plate.
        let post = rod(k, [2.0, 1.5, 1.0], r, h);
        let joined = solve(&plate, &post, Union, k);
        assert_mass(
            &joined,
            plate_volume + PI * r * r * h,
            plate_area + 2.0 * PI * r * h,
            k,
        );
        assert_eq!(counts(&joined), (10, 14, 8, 1));
        assert_eq!(genus(&joined.body), Some(0));
        // Either way round.
        let other = solve(&post, &plate, Union, k);
        assert_mass(
            &other,
            plate_volume + PI * r * r * h,
            plate_area + 2.0 * PI * r * h,
            k,
        );
        assert_eq!(counts(&other), (10, 14, 8, 1));

        let block = boxed(k, [1.0, 1.0, 1.0], [2.0, 2.0, 1.0 + h]);
        let joined = solve(&plate, &block, Union, k);
        assert_mass(&joined, plate_volume + h, plate_area + 4.0 * h, k);
        assert_eq!(counts(&joined), (16, 24, 11, 1));

        // Touching only: nothing in common, and nothing to take away.
        assert_empty(&plate, &post, Intersect, k);
        let same = solve(&plate, &post, Subtract, k);
        assert_mass(&same, plate_volume, plate_area, k);
    }
}

#[test]
fn boxes_side_by_side_and_flush() {
    for k in SCALES {
        let a = boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 1.0]);
        // Sharing a whole face.
        let b = boxed(k, [2.0, 0.0, 0.0], [5.0, 2.0, 1.0]);
        let joined = solve(&a, &b, Union, k);
        assert_mass(&joined, 10.0, 2.0 * (10.0 + 5.0 + 2.0), k);
        assert_eq!(joined.body.shells.len(), 1);
        assert_empty(&a, &b, Intersect, k);
        // Overlapping, flush on top, bottom and one side.
        let c = boxed(k, [1.0, 0.0, 0.0], [3.0, 3.0, 1.0]);
        let joined = solve(&a, &c, Union, k);
        // An L: 2x2 plus 2x3 less the 1x2 overlap, 1 high.
        assert_mass(&joined, 8.0, 2.0 * 8.0 + 12.0, k);
        let common = solve(&a, &c, Intersect, k);
        assert_mass(&common, 2.0, 2.0 * (2.0 + 1.0 + 2.0), k);
        assert_eq!(counts(&common), (8, 12, 6, 1));
        let less = solve(&a, &c, Subtract, k);
        assert_mass(&less, 2.0, 2.0 * (2.0 + 1.0 + 2.0), k);
        assert_eq!(counts(&less), (8, 12, 6, 1));
        // A cutter flush with one side: a notch.
        let d = boxed(k, [0.5, 0.0, 0.25], [1.5, 1.0, 3.0]);
        let notched = solve(&a, &d, Subtract, k);
        // Less 1 of the top and 0.75 of the side; plus the notch's floor
        // (1), two sides (0.75 each) and back (0.75).
        assert_mass(&notched, 4.0 - 0.75, 16.0 - 1.75 + 3.25, k);
        // A step: the cutter takes a whole corner, flush on three sides.
        let e = boxed(k, [1.0, 0.0, 0.5], [2.0, 2.0, 1.0]);
        let stepped = solve(&a, &e, Subtract, k);
        // An L of area 1.5 and perimeter 6, 2 long.
        assert_mass(&stepped, 3.0, 2.0 * 1.5 + 6.0 * 2.0, k);
        assert_eq!(counts(&stepped), (12, 18, 8, 1));
        // The same body twice.
        let twice = solve(&a, &a, Union, k);
        assert_mass(&twice, 4.0, 16.0, k);
        let twice = solve(&a, &a, Intersect, k);
        assert_mass(&twice, 4.0, 16.0, k);
        assert!(twice.faces.iter().all(|f| f.operand == Operand::A));
        assert_empty(&a, &a, Subtract, k);
    }
}

#[test]
fn apart_and_one_inside_the_other() {
    for k in SCALES {
        let a = boxed(k, [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let far = rod(k, [10.0, 0.0, 0.0], 1.0, 2.0);
        let both = solve(&a, &far, Union, k);
        assert_mass(&both, 64.0 + 2.0 * PI, 96.0 + 6.0 * PI, k);
        assert_eq!(both.body.shells.len(), 2);
        assert_empty(&a, &far, Intersect, k);
        let same = solve(&a, &far, Subtract, k);
        assert_mass(&same, 64.0, 96.0, k);
        assert_eq!(counts(&same), (8, 12, 6, 1));

        let inner = rod(k, [2.0, 2.0, 1.0], 1.0, 2.0);
        let both = solve(&a, &inner, Union, k);
        assert_mass(&both, 64.0, 96.0, k);
        let common = solve(&a, &inner, Intersect, k);
        assert_mass(&common, 2.0 * PI, 6.0 * PI, k);
        // A cavity: two shells, the inner one facing inward.
        let hollow = solve(&a, &inner, Subtract, k);
        assert_mass(&hollow, 64.0 - 2.0 * PI, 96.0 + 6.0 * PI, k);
        assert_eq!(hollow.body.shells.len(), 2);
        assert_empty(&inner, &a, Subtract, k);
    }
}

#[test]
fn a_cylinder_and_a_box() {
    for k in SCALES {
        let (r, h) = (1.0, 4.0);
        let post = rod(k, [0.0, 0.0, 0.0], r, h);
        // A slab across the middle of the cylinder: a disc.
        let slab = boxed(k, [-3.0, -3.0, 1.0], [3.0, 3.0, 2.5]);
        let disc = solve(&post, &slab, Intersect, k);
        assert_mass(&disc, PI * 1.5, 2.0 * PI + 2.0 * PI * 1.5, k);
        assert_eq!(counts(&disc), (2, 2, 3, 1));
        let parts = solve(&post, &slab, Subtract, k);
        assert_mass(&parts, PI * 2.5, 4.0 * PI + 2.0 * PI * 2.5, k);
        assert_eq!(parts.body.shells.len(), 2);
        // Half the cylinder, cut along its axis.
        let half = boxed(k, [0.0, -3.0, -1.0], [3.0, 3.0, 5.0]);
        let d = solve(&post, &half, Intersect, k);
        assert_mass(&d, PI * h / 2.0, PI + PI * h + 2.0 * h, k);
        // Four corners, and the cylinder's own two vertices (at angle 0 of
        // its circles), which are on the half that is kept: they split the
        // two arcs.
        assert_eq!(counts(&d), (6, 8, 4, 1));
        // A quarter, through the axis.
        let quarter = boxed(k, [0.0, 0.0, -1.0], [3.0, 3.0, 5.0]);
        let q = solve(&post, &quarter, Intersect, k);
        assert_mass(&q, PI * h / 4.0, PI / 2.0 + PI * h / 2.0 + 2.0 * h, k);
        let rest = solve(&post, &quarter, Subtract, k);
        assert_mass(
            &rest,
            3.0 * PI * h / 4.0,
            1.5 * PI + 1.5 * PI * h + 2.0 * h,
            k,
        );
        // A segment: a chord at half the radius. Area of the circular
        // segment with half-angle a = acos(1/2): a - sin a cos a.
        let side = boxed(k, [0.5, -3.0, -1.0], [3.0, 3.0, 5.0]);
        let a = (0.5_f64).acos();
        let segment = a - a.sin() * a.cos();
        let s = solve(&post, &side, Intersect, k);
        assert_mass(
            &s,
            segment * h,
            2.0 * segment + 2.0 * a * h + 2.0 * a.sin() * h,
            k,
        );
        // A box that pokes into the side of the cylinder without reaching
        // the axis: the wall gets a hole.
        let poke = boxed(k, [0.5, -0.25, 1.0], [3.0, 0.25, 2.0]);
        let dented = solve(&post, &poke, Subtract, k);
        let chord = |y: f64| (1.0 - y * y).sqrt();
        // Cross-section of the dent: ∫ (sqrt(1 - y²) - 0.5) dy over ±0.25.
        let cap = |y: f64| 0.5 * (y * chord(y) + y.asin());
        let dent = 2.0 * (cap(0.25) - 0.5 * 0.25);
        let m = mass(&dented.body);
        assert_close(m.volume, (PI * h - dent) * k * k * k, "volume");
        assert_eq!(genus(&dented.body), Some(0));
    }
}

#[test]
fn cylinders_with_parallel_axes() {
    for k in SCALES {
        // Two equal cylinders a radius apart: a lens. Its section is two
        // circular segments with half-angle acos(1/2).
        let a = rod(k, [0.0, 0.0, 0.0], 1.0, 2.0);
        let b = rod(k, [1.0, 0.0, 0.0], 1.0, 2.0);
        let half = (0.5_f64).acos();
        let lens = 2.0 * (half - half.sin() * half.cos());
        let common = solve(&a, &b, Intersect, k);
        assert_mass(&common, lens * 2.0, 2.0 * lens + 4.0 * half * 2.0, k);
        // Four corners and the two vertices of A's circles (at angle 0,
        // which is inside B); B's are outside A.
        assert_eq!(counts(&common), (6, 8, 4, 1));
        let both = solve(&a, &b, Union, k);
        assert_mass(
            &both,
            (2.0 * PI - lens) * 2.0,
            2.0 * (2.0 * PI - lens) + 2.0 * (2.0 * PI - 2.0 * half) * 2.0,
            k,
        );
        let moon = solve(&a, &b, Subtract, k);
        assert_mass(
            &moon,
            (PI - lens) * 2.0,
            2.0 * (PI - lens) + 2.0 * PI * 2.0,
            k,
        );
        // A pin in a hole of its own size fills it; a shorter coaxial
        // cylinder of the same radius shortens.
        let plate = boxed(k, [-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]);
        let holed = solve(&plate, &a, Subtract, k);
        let filled = solve(&holed.body, &a, Union, k);
        assert_mass(&filled, 32.0, 64.0, k);
        let stub = rod(k, [0.0, 0.0, 0.5], 1.0, 4.0);
        let shorter = solve(&a, &stub, Subtract, k);
        assert_mass(&shorter, PI * 0.5, 2.0 * PI + PI, k);
        assert_eq!(counts(&shorter), (2, 2, 3, 1));
        let longer = solve(&a, &stub, Union, k);
        assert_mass(&longer, PI * 4.5, 2.0 * PI + 9.0 * PI, k);
        // A thinner coaxial cylinder through it: a tube.
        let bore = rod(k, [0.0, 0.0, -1.0], 0.5, 4.0);
        let tube = solve(&a, &bore, Subtract, k);
        assert_mass(
            &tube,
            PI * 0.75 * 2.0,
            2.0 * PI * 0.75 + 2.0 * PI * 2.0 + PI * 2.0,
            k,
        );
        assert_eq!(genus(&tube.body), Some(1));
    }
}

#[test]
fn an_oblique_cut_of_a_cylinder() {
    for k in SCALES {
        // A cylinder cut by a tilted half-space (a big box turned about
        // the y axis): what is left below a plane through the axis at
        // height 2 is half of a cylinder of height 4.
        let (r, h) = (1.0, 4.0);
        let post = rod(k, [0.0, 0.0, 0.0], r, h);
        let tilt = 0.6_f64;
        let frame = Frame::new(
            Vec3::new(0.0, 0.0, 2.0 * k),
            Vec3::new(tilt.sin(), 0.0, tilt.cos()),
            Vec3::Y,
        )
        .unwrap();
        let corner = frame.at(frame.point(-10.0 * k, -10.0 * k, 0.0)).unwrap();
        let above = primitives::block(&corner, 20.0 * k, 20.0 * k, 20.0 * k).unwrap();
        let cut = solve(&post, &above, Subtract, k);
        let ellipse = PI * r * r / tilt.cos();
        assert_mass(&cut, PI * 2.0, PI + 2.0 * PI * 2.0 + ellipse, k);
        assert_eq!(counts(&cut), (2, 2, 3, 1));
        assert!(
            cut.body
                .edges
                .iter()
                .any(|e| matches!(e.curve, Curve3::Ellipse(_)))
        );
        let top = solve(&post, &above, Intersect, k);
        assert_mass(&top, PI * 2.0, PI + 2.0 * PI * 2.0 + ellipse, k);
        // The cut body is an operand like any other: a slab through its
        // slanted top meets the ellipse.
        let slab = boxed(k, [-0.25, -3.0, -1.0], [0.25, 3.0, 5.0]);
        let slice = solve(&cut.body, &slab, Intersect, k);
        let whole = solve(&post, &slab, Intersect, k);
        // By symmetry about the axis point the slanted plane halves it.
        assert_close(
            mass(&slice.body).volume,
            mass(&whole.body).volume / 2.0,
            "volume of the slice",
        );
    }
}

#[test]
fn sources_say_where_everything_came_from() {
    let k = 1.0;
    let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let cutter = rod(k, [1.5, 1.25, -1.0], 0.75, 4.0);
    let holed = solve(&plate, &cutter, Subtract, k);
    assert_eq!(holed.faces.len(), holed.body.faces.len());
    assert_eq!(holed.edges.len(), holed.body.edges.len());
    assert_eq!(holed.vertices.len(), holed.body.vertices.len());
    // Six faces of the plate in their order, then the cutter's wall.
    let expected: Vec<FaceSource> = (0..6)
        .map(|i| FaceSource {
            operand: Operand::A,
            face: FaceId(i),
        })
        .chain([FaceSource {
            operand: Operand::B,
            face: FaceId(0),
        }])
        .collect();
    assert_eq!(holed.faces, expected);
    // The plate's twelve edges and eight vertices are kept; the two rims
    // of the hole are where the wall crosses the bottom (face 0) and the
    // top (face 1), each with one new vertex.
    let kept = |e: &&EdgeSource| {
        matches!(
            e,
            EdgeSource::Kept {
                operand: Operand::A,
                ..
            }
        )
    };
    assert_eq!(holed.edges.iter().filter(kept).count(), 12);
    let mut rims: Vec<EdgeSource> = holed.edges.iter().filter(|e| !kept(e)).copied().collect();
    rims.sort();
    assert_eq!(
        rims,
        [
            EdgeSource::Intersection {
                a: FaceId(0),
                b: FaceId(0)
            },
            EdgeSource::Intersection {
                a: FaceId(1),
                b: FaceId(0)
            },
        ]
    );
    let mut new: Vec<VertexSource> = holed
        .vertices
        .iter()
        .filter(|v| matches!(v, VertexSource::New { .. }))
        .copied()
        .collect();
    new.sort();
    assert_eq!(
        new,
        [
            VertexSource::New {
                a: Entity::Face(FaceId(0)),
                b: Entity::Face(FaceId(0)),
                index: 0
            },
            VertexSource::New {
                a: Entity::Face(FaceId(1)),
                b: Entity::Face(FaceId(0)),
                index: 0
            },
        ]
    );
    assert_eq!(holed.vertices.len(), 10);

    // Flush: the rims are the cutter's own circles and vertices.
    let flush = rod(k, [1.5, 1.25, 0.0], 0.75, 2.0);
    let holed = solve(&plate, &flush, Subtract, k);
    let mut rims: Vec<EdgeSource> = holed.edges.iter().filter(|e| !kept(e)).copied().collect();
    rims.sort();
    assert_eq!(
        rims,
        [
            EdgeSource::Kept {
                operand: Operand::B,
                edge: EdgeId(0)
            },
            EdgeSource::Kept {
                operand: Operand::B,
                edge: EdgeId(1)
            },
        ]
    );
    assert!(holed.vertices.contains(&VertexSource::Kept {
        operand: Operand::B,
        vertex: VertexId(1)
    }));

    // Crossing edges: where an edge of A goes through a face of B, and
    // the same sources the next time.
    let a = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let b = boxed(k, [1.0, 1.0, 1.0], [6.0, 5.0, 4.0]);
    let common = solve(&a, &b, Intersect, k);
    let again = solve(&a, &b, Intersect, k);
    assert_eq!(common, again);
    let through = common
        .vertices
        .iter()
        .filter(|v| {
            matches!(
                v,
                VertexSource::New {
                    a: Entity::Edge(_),
                    b: Entity::Face(_),
                    ..
                } | VertexSource::New {
                    a: Entity::Face(_),
                    b: Entity::Edge(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(through, 6);
    // One corner of each box is inside the other.
    let corners = common
        .vertices
        .iter()
        .filter(|v| matches!(v, VertexSource::Kept { .. }))
        .count();
    assert_eq!(corners, 2);
    // Coincident faces: A's is the source.
    let c = boxed(k, [1.0, 0.0, 0.0], [3.0, 3.0, 2.0]);
    let common = solve(&a, &c, Intersect, k);
    let from_a = common
        .faces
        .iter()
        .filter(|f| f.operand == Operand::A)
        .count();
    // Bottom, top and the y = 0 side are shared and count as A's; so is
    // A's own y = 3 side. B brings its two x sides.
    assert_eq!(from_a, 4);
}

#[test]
fn what_is_not_supported_says_so() {
    let k = 1.0;
    let tol = Tolerance::default();
    let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let reason = |a, b, op| match boolean(a, b, op, &tol) {
        Err(KernelError::Unsupported { reason, faces, .. }) => (reason, faces),
        other => panic!("expected Unsupported, got {other:?}"),
    };
    // Other surfaces.
    let ball = primitives::sphere(&Frame::WORLD, 1.0).unwrap();
    let (why, faces) = reason(&plate, &ball, Union);
    assert!(why.contains("not planar or cylindrical"), "{why}");
    assert_eq!(faces, [(Operand::B, FaceId(0))]);
    let cone = primitives::cone(&Frame::WORLD, 1.0, 2.0).unwrap();
    assert!(
        reason(&cone, &plate, Subtract)
            .0
            .contains("not planar or cylindrical")
    );
    // Crossing cylinders.
    let upright = rod(k, [0.0, 0.0, -2.0], 1.0, 4.0);
    let lying = primitives::cylinder(
        &Frame::new(Vec3::new(-2.0, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
        0.5,
        4.0,
    )
    .unwrap();
    let (why, faces) = reason(&upright, &lying, Union);
    assert!(why.contains("not parallel"), "{why}");
    assert_eq!(faces, [(Operand::A, FaceId(0)), (Operand::B, FaceId(0))]);
    // A cylinder lying on the plate touches it along a line; one standing
    // against a wall likewise.
    let on_top = primitives::cylinder(
        &Frame::new(Vec3::new(1.0, 1.5, 2.5), Vec3::X, Vec3::Y).unwrap(),
        0.5,
        2.0,
    )
    .unwrap();
    assert!(reason(&plate, &on_top, Union).0.contains("tangent"));
    let wall = boxed(k, [1.0, -3.0, -1.0], [2.0, 3.0, 5.0]);
    assert!(reason(&upright, &wall, Union).0.contains("tangent"));
    // Kissing cylinders.
    let beside = rod(k, [2.0, 0.0, -2.0], 1.0, 4.0);
    assert!(reason(&upright, &beside, Union).0.contains("tangent"));
    // Boxes that share only an edge.
    let diagonal = boxed(k, [4.0, 3.0, 0.0], [5.0, 4.0, 2.0]);
    let (why, _) = reason(&plate, &diagonal, Union);
    assert!(why.contains("touch along an edge"), "{why}");
    // Features below the tolerance.
    let nearly = boxed(k, [4.0 - 1.2e-6, 0.0, 0.0], [6.0, 3.0, 2.0]);
    assert!(
        reason(&plate, &nearly, Union)
            .0
            .contains("edge of tolerance")
    );
    // The message names the faces.
    let message = boolean(&upright, &lying, Union, &tol)
        .unwrap_err()
        .to_string();
    assert_eq!(
        message,
        "boolean does not support this: cylinders whose axes are not parallel (the intersection \
         is a quartic curve) (face 0 of the first body, face 0 of the second body)"
    );
    assert_eq!(
        boolean(&plate, &diagonal, Intersect, &tol)
            .unwrap_err()
            .to_string(),
        "boolean leaves nothing: the result is empty"
    );
}

#[test]
fn bad_input_is_an_error_not_a_crash() {
    let plate = boxed(1.0, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
    let tol = Tolerance::default();
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let t = Tolerance {
            linear: bad,
            angular: 1e-9,
        };
        assert!(matches!(
            boolean(&plate, &plate, Union, &t),
            Err(KernelError::InvalidSize { .. })
        ));
        let t = Tolerance {
            linear: 1e-6,
            angular: bad,
        };
        assert!(matches!(
            boolean(&plate, &plate, Union, &t),
            Err(KernelError::InvalidSize { .. })
        ));
    }
    // A broken operand: an edge removed from a loop, a vertex moved, NaN.
    let mut open = plate.clone();
    open.loops[0].coedges.pop();
    let mut moved = plate.clone();
    moved.vertices[3].point = Vec3::new(9.0, 9.0, 9.0);
    let mut nan = plate.clone();
    nan.vertices[0].point = Vec3::new(f64::NAN, 0.0, 0.0);
    let mut dangling = plate.clone();
    dangling.faces[2].loops.push(craft3d_topo::LoopId(99));
    let mut sheet = plate.clone();
    sheet.kind = craft3d_topo::BodyKind::Sheet;
    let empty = craft3d_topo::Body::new(craft3d_topo::BodyKind::Solid);
    for broken in [&open, &moved, &nan, &dangling, &sheet] {
        for op in [Union, Subtract, Intersect] {
            assert!(boolean(broken, &plate, op, &tol).is_err());
            assert!(boolean(&plate, broken, op, &tol).is_err());
        }
    }
    // Nothing at all is a body with nothing in it.
    assert!(matches!(
        boolean(&empty, &empty, Union, &tol),
        Err(KernelError::EmptyResult { .. })
    ));
    let same = boolean(&plate, &empty, Union, &tol).unwrap();
    assert_eq!(same.body.faces.len(), 6);
}

/// A plate with a grid of `n` by `n` round holes, by subtracting cylinders
/// one at a time.
fn drilled(n: usize, k: f64, flush: bool) -> craft3d_topo::Body {
    let side = 2.0 * n as f64;
    let mut plate = boxed(k, [0.0, 0.0, 0.0], [side, side, 1.0]);
    for i in 0..n {
        for j in 0..n {
            let (z, h) = if flush { (0.0, 1.0) } else { (-0.5, 2.0) };
            let cutter = rod(k, [2.0 * i as f64 + 1.0, 2.0 * j as f64 + 1.0, z], 0.5, h);
            plate = solve(&plate, &cutter, Subtract, k).body;
        }
    }
    plate
}

#[test]
fn drilling_holes_gives_the_plate_extruded_with_holes() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    for k in SCALES {
        let n = 3;
        let side = 2.0 * n as f64 * k;
        let mut curves = Vec::new();
        let corners = [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]];
        for i in 0..4 {
            curves.push(SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: corners[i],
                    end: corners[(i + 1) % 4],
                },
            });
        }
        for i in 0..n {
            for j in 0..n {
                curves.push(SketchCurve {
                    id: 10 + (i * n + j) as u64,
                    curve: Curve2::Circle {
                        center: [(2.0 * i as f64 + 1.0) * k, (2.0 * j as f64 + 1.0) * k],
                        radius: 0.5 * k,
                    },
                });
            }
        }
        let tol = tolerance(k);
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let extruded = extrude(&profiles, &Frame::WORLD, 0.0, k, &tol)
            .unwrap()
            .body;
        let expected = mass(&extruded);
        for flush in [false, true] {
            let plate = drilled(n, k, flush);
            let m = mass(&plate);
            assert_close(m.volume, expected.volume, "volume");
            assert_close(m.area, expected.area, "area");
            assert!(m.centroid.unwrap().distance(expected.centroid.unwrap()) < 1e-9 * k);
            assert_eq!(genus(&plate), genus(&extruded));
            assert_eq!(genus(&plate), Some(9));
            assert_eq!(plate.faces.len(), extruded.faces.len());
            assert_eq!(plate.edges.len(), extruded.edges.len());
            assert_eq!(plate.vertices.len(), extruded.vertices.len());
            assert_eq!(plate.shells.len(), 1);
        }
        // And in closed form.
        let holes = 9.0 * PI * 0.25;
        assert_close(expected.volume, (36.0 - holes) * k * k * k, "volume");
    }
}

/// Timing: `cargo test --release -p craft3d-kernel --test boolean --
/// --ignored --nocapture a_hundred_holes`.
#[test]
#[ignore = "a benchmark; run in release"]
fn a_hundred_holes() {
    let start = std::time::Instant::now();
    let plate = drilled(10, 1.0, false);
    let through = start.elapsed();
    let start = std::time::Instant::now();
    let flush = drilled(10, 1.0, true);
    println!(
        "100 holes one by one: {through:?} (cutters through), {:?} (cutters flush); {} faces",
        start.elapsed(),
        plate.faces.len()
    );
    for body in [&plate, &flush] {
        assert_close(mass(body).volume, 400.0 - 25.0 * PI, "volume");
        assert_eq!(genus(body), Some(100));
        assert_eq!(body.faces.len(), 106);
    }
}

#[test]
fn a_post_over_a_hole() {
    for k in SCALES {
        // A plate with a hole through it, and a post wider than the hole
        // standing over it: the top face falls into the part outside the
        // post and a ring under it, and the hole's rim belongs to the
        // ring, the smaller of the two regions around it.
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0]);
        let holed = solve(&plate, &rod(k, [2.0, 1.5, 0.0], 0.5, 1.0), Subtract, k);
        let post = rod(k, [2.0, 1.5, 1.0], 1.25, 2.0);
        let capped = solve(&holed.body, &post, Union, k);
        // Bottom 12 − π/4, top 12 − 25π/16, sides 14, the hole's wall π
        // and its ceiling π/4, the post's wall 5π and lid 25π/16.
        assert_mass(&capped, 12.0 - 0.25 * PI + 3.125 * PI, 38.0 + 6.0 * PI, k);
        assert_eq!(genus(&capped.body), Some(0));
        assert_eq!(capped.body.shells.len(), 1);
        // The other way round, and taken away again.
        let other = solve(&post, &holed.body, Union, k);
        assert_mass(&other, 12.0 - 0.25 * PI + 3.125 * PI, 38.0 + 6.0 * PI, k);
        let again = solve(&capped.body, &post, Subtract, k);
        assert_mass(&again, 12.0 - 0.25 * PI, 38.0 + 0.5 * PI, k);
        assert_eq!(genus(&again.body), Some(1));
        // A thinner post through the hole leaves a ring-shaped gap.
        let pin = rod(k, [2.0, 1.5, -1.0], 0.25, 3.0);
        let pinned = solve(&holed.body, &pin, Union, k);
        assert_eq!(pinned.body.shells.len(), 2);
        assert_mass(
            &pinned,
            12.0 - 0.25 * PI + 0.0625 * PI * 3.0,
            38.0 + 0.5 * PI + 2.0 * 0.0625 * PI + 1.5 * PI,
            k,
        );
    }
}
