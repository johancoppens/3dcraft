//! Booleans with cones, spheres and tori against closed forms
//! (docs/testing.md, oracles 1 and 2): volume, area, genus and counts, at
//! three scales. The formulas are those of the spherical cap and segment,
//! the frustum, and Pappus for the torus.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Curve3;
use craft3d_kernel::boolean::{BooleanOp, FaceSource, Operand, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{FaceId, genus};
use std::f64::consts::PI;

use BooleanOp::{Intersect, Subtract, Union};

/// Volume and curved area of the cap of height `h` of a sphere of radius
/// `r`.
fn cap(r: f64, h: f64) -> (f64, f64) {
    (PI * h * h * (3.0 * r - h) / 3.0, 2.0 * PI * r * h)
}

fn sphere(r: f64) -> (f64, f64) {
    (4.0 / 3.0 * PI * r * r * r, 4.0 * PI * r * r)
}

#[test]
fn a_ball_cut_by_a_block() {
    for k in SCALES {
        let (r, h) = (2.0, 0.75);
        let ball = ball(k, [1.0, -0.5, 3.0], r);
        // Everything above the plane h over the centre.
        let above = boxed(k, [-4.0, -4.0, 3.0 + h], [6.0, 5.0, 9.0]);
        let (cap_volume, cap_area) = cap(r, r - h);
        let disc = PI * (r * r - h * h);
        let top = solve(&ball, &above, Intersect, k);
        assert_mass(&top, cap_volume, cap_area + disc, k);
        assert_eq!(counts(&top), (1, 1, 2, 1));
        assert_eq!(genus(&top.body), Some(0));
        let rest = solve(&ball, &above, Subtract, k);
        assert_mass(
            &rest,
            sphere(r).0 - cap_volume,
            sphere(r).1 - cap_area + disc,
            k,
        );
        assert_eq!(counts(&rest), (1, 1, 2, 1));
        // The disc is the block's bottom, turned over.
        assert_eq!(
            rest.faces,
            [
                FaceSource {
                    operand: Operand::A,
                    face: FaceId(0)
                },
                FaceSource {
                    operand: Operand::B,
                    face: FaceId(0)
                }
            ]
        );
        assert!(rest.body.faces[1].reversed);
        let both = solve(&ball, &above, Union, k);
        let block = (10.0 * 9.0 * (6.0 - h), 2.0 * (90.0 + 19.0 * (6.0 - h)));
        assert_mass(
            &both,
            block.0 + sphere(r).0 - cap_volume,
            block.1 - disc + sphere(r).1 - cap_area,
            k,
        );
        assert_eq!(genus(&both.body), Some(0));
        // Either way round.
        let other = solve(&above, &ball, Intersect, k);
        assert_mass(&other, cap_volume, cap_area + disc, k);

        // A slab through the middle: a segment, and two caps left over.
        let slab = boxed(k, [-4.0, -4.0, 3.0 - 0.5], [6.0, 5.0, 3.0 + h]);
        let (low_volume, low_area) = cap(r, r - 0.5);
        let low_disc = PI * (r * r - 0.25);
        let segment = solve(&ball, &slab, Intersect, k);
        assert_mass(
            &segment,
            sphere(r).0 - cap_volume - low_volume,
            sphere(r).1 - cap_area - low_area + disc + low_disc,
            k,
        );
        assert_eq!(counts(&segment), (2, 2, 3, 1));
        let caps = solve(&ball, &slab, Subtract, k);
        assert_mass(
            &caps,
            cap_volume + low_volume,
            cap_area + low_area + disc + low_disc,
            k,
        );
        assert_eq!(caps.body.shells.len(), 2);

        // An eighth: the corner of a block at the centre. Its three flat
        // sides meet the sphere in quarter circles, one of which runs
        // through both poles.
        let corner = boxed(k, [1.0, -0.5, 3.0], [5.0, 4.0, 8.0]);
        let eighth = solve(&ball, &corner, Intersect, k);
        assert_mass(
            &eighth,
            sphere(r).0 / 8.0,
            sphere(r).1 / 8.0 + 3.0 * PI * r * r / 4.0,
            k,
        );
        assert_eq!(counts(&eighth), (4, 6, 4, 1));
        let bitten = solve(&ball, &corner, Subtract, k);
        assert_mass(
            &bitten,
            sphere(r).0 * 7.0 / 8.0,
            sphere(r).1 * 7.0 / 8.0 + 3.0 * PI * r * r / 4.0,
            k,
        );
        assert_eq!(genus(&bitten.body), Some(0));
    }
}

#[test]
fn two_balls() {
    for k in SCALES {
        // Radii 2 and 1.5, centres 2.5 apart, on a line that is along no
        // axis.
        let (r1, r2, d) = (2.0, 1.5, 2.5);
        let a = ball(k, [0.0, 0.0, 0.0], r1);
        let b = ball(k, [1.5, 2.0, 0.0], r2);
        // The lens is a cap of each: of heights h1 and h2.
        let h1 = (r2 - r1 + d) * (r2 + r1 - d) / (2.0 * d);
        let h2 = (r1 - r2 + d) * (r1 + r2 - d) / (2.0 * d);
        let (v1, a1) = cap(r1, h1);
        let (v2, a2) = cap(r2, h2);
        let lens = solve(&a, &b, Intersect, k);
        assert_mass(&lens, v1 + v2, a1 + a2, k);
        assert_eq!(counts(&lens), (1, 1, 2, 1));
        let both = solve(&a, &b, Union, k);
        assert_mass(
            &both,
            sphere(r1).0 + sphere(r2).0 - v1 - v2,
            sphere(r1).1 + sphere(r2).1 - a1 - a2,
            k,
        );
        assert_eq!(counts(&both), (1, 1, 2, 1));
        let bitten = solve(&a, &b, Subtract, k);
        assert_mass(&bitten, sphere(r1).0 - v1 - v2, sphere(r1).1 - a1 + a2, k);
        // A ball inside a ball: a hollow ball, and the small one again.
        let inner = ball(k, [0.25, 0.0, 0.5], 1.0);
        let hollow = solve(&a, &inner, Subtract, k);
        assert_mass(
            &hollow,
            sphere(r1).0 - sphere(1.0).0,
            sphere(r1).1 + sphere(1.0).1,
            k,
        );
        assert_eq!(hollow.body.shells.len(), 2);
        assert_eq!(counts(&hollow), (0, 0, 2, 2));
        let same = solve(&a, &inner, Intersect, k);
        assert_mass(&same, sphere(1.0).0, sphere(1.0).1, k);
        assert_empty(&inner, &a, Subtract, k);
        // The same ball twice.
        let twice = solve(&a, &a, Union, k);
        assert_mass(&twice, sphere(r1).0, sphere(r1).1, k);
        assert_empty(&a, &a, Subtract, k);
    }
}

#[test]
fn a_ball_through_a_plate() {
    for k in SCALES {
        // The ball's centre is in the middle of the plate and it sticks
        // out on both sides: what it takes away is a segment between two
        // planes at t / 2 either side of the centre.
        let (r, t) = (1.0, 1.0);
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, t]);
        let ball = ball(k, [2.0, 1.5, 0.5], r);
        let a = t / 2.0;
        let segment = 2.0 * PI * (r * r * a - a * a * a / 3.0);
        let zone = 2.0 * PI * r * t;
        let rim = PI * (r * r - a * a);
        let holed = solve(&plate, &ball, Subtract, k);
        assert_mass(&holed, 12.0 - segment, 38.0 - 2.0 * rim + zone, k);
        assert_eq!(genus(&holed.body), Some(1));
        assert_eq!(counts(&holed), (10, 14, 7, 1));
        assert_eq!(kinds(&holed.body).get("sphere"), Some(&1));
        let plug = solve(&plate, &ball, Intersect, k);
        assert_mass(&plug, segment, zone + 2.0 * rim, k);
        assert_eq!(counts(&plug), (2, 2, 3, 1));
        let both = solve(&plate, &ball, Union, k);
        let (cap_volume, cap_area) = cap(r, r - a);
        assert_mass(
            &both,
            12.0 + 2.0 * cap_volume,
            38.0 - 2.0 * rim + 2.0 * cap_area,
            k,
        );
        assert_eq!(counts(&both), (10, 14, 8, 1));
    }
}

#[test]
fn a_cone_cut_across_and_through_its_axis() {
    for k in SCALES {
        // Base radius 1.5, height 3; cut at height 1.
        let (radius, height, cut) = (1.5, 3.0, 1.0);
        let cone = spike(k, [1.0, 2.0, 0.0], radius, height);
        let small = radius * (1.0 - cut / height);
        let slant = |r0: f64, r1: f64, h: f64| PI * (r0 + r1) * (r0 - r1).hypot(h);
        let above = boxed(k, [-4.0, -4.0, cut], [6.0, 8.0, 9.0]);
        let frustum = solve(&cone, &above, Subtract, k);
        assert_mass(
            &frustum,
            PI * cut * (radius * radius + radius * small + small * small) / 3.0,
            PI * radius * radius + PI * small * small + slant(radius, small, cut),
            k,
        );
        assert_eq!(counts(&frustum), (2, 2, 3, 1));
        // The tip keeps the apex, inside its one conical face.
        let tip = solve(&cone, &above, Intersect, k);
        assert_mass(
            &tip,
            PI * small * small * (height - cut) / 3.0,
            PI * small * small + slant(small, 0.0, height - cut),
            k,
        );
        assert_eq!(counts(&tip), (1, 1, 2, 1));
        // Split through its axis: the cut runs along two rulings that
        // meet at the apex.
        let beside = boxed(k, [1.0, -4.0, -1.0], [6.0, 8.0, 9.0]);
        let half = solve(&cone, &beside, Intersect, k);
        assert_mass(
            &half,
            PI * radius * radius * height / 6.0,
            0.5 * PI * radius * radius + 0.5 * slant(radius, 0.0, height) + radius * height,
            k,
        );
        // The apex, the two ends of the cut across the base, and the
        // cone's own vertex on its rim, which is on this half.
        assert_eq!(counts(&half), (4, 5, 3, 1));
        assert_eq!(genus(&half.body), Some(0));
        let other = solve(&cone, &beside, Subtract, k);
        assert_mass(
            &other,
            PI * radius * radius * height / 6.0,
            0.5 * PI * radius * radius + 0.5 * slant(radius, 0.0, height) + radius * height,
            k,
        );
        assert_eq!(counts(&other), (3, 4, 3, 1));
        // A quarter: two cuts through the axis.
        let quarter = boxed(k, [1.0, 2.0, -1.0], [6.0, 8.0, 9.0]);
        let wedge = solve(&cone, &quarter, Intersect, k);
        assert_mass(
            &wedge,
            PI * radius * radius * height / 12.0,
            0.25 * PI * radius * radius + 0.25 * slant(radius, 0.0, height) + radius * height,
            k,
        );
        // The half cut again through its axis: three rulings meet at the
        // apex, two of the boundary and one across the face, and which
        // side of the new one is kept depends on their order there.
        let behind = boxed(k, [-4.0, 2.0, -1.0], [6.0, 8.0, 9.0]);
        let again = solve(&half.body, &behind, Intersect, k);
        assert_mass(
            &again,
            PI * radius * radius * height / 12.0,
            0.25 * PI * radius * radius + 0.25 * slant(radius, 0.0, height) + radius * height,
            k,
        );
        let rest = solve(&half.body, &behind, Subtract, k);
        assert_mass(
            &rest,
            PI * radius * radius * height / 12.0,
            0.25 * PI * radius * radius + 0.25 * slant(radius, 0.0, height) + radius * height,
            k,
        );
        // The frustum is an operand like any other: its top comes off.
        let lower = boxed(k, [-4.0, -4.0, 0.5], [6.0, 8.0, 9.0]);
        let shorter = solve(&frustum.body, &lower, Subtract, k);
        let middle = radius * (1.0 - 0.5 / height);
        assert_mass(
            &shorter,
            PI * 0.5 * (radius * radius + radius * middle + middle * middle) / 3.0,
            PI * radius * radius + PI * middle * middle + slant(radius, middle, 0.5),
            k,
        );
    }
}

#[test]
fn a_ball_and_a_cylinder_on_one_axis() {
    for k in SCALES {
        // A hole of radius 1 through the centre of a ball of radius 2
        // leaves a ring of height h = 2 sqrt(r² - c²), whose volume is
        // π h³ / 6 whatever the radii (the napkin ring).
        let (r, c) = (2.0_f64, 1.0_f64);
        let ball = ball(k, [0.0, 0.0, 3.0], r);
        let drill = rod(k, [0.0, 0.0, 0.0], c, 6.0);
        let h = 2.0 * (r * r - c * c).sqrt();
        let ring_volume = PI * h * h * h / 6.0;
        let ring = solve(&ball, &drill, Subtract, k);
        assert_mass(&ring, ring_volume, 2.0 * PI * r * h + 2.0 * PI * c * h, k);
        assert_eq!(genus(&ring.body), Some(1));
        assert_eq!(counts(&ring), (2, 2, 2, 1));
        // The core: a cylinder with a cap of the ball at each end.
        let (_, cap_area) = cap(r, r - h / 2.0);
        let core = solve(&ball, &drill, Intersect, k);
        assert_mass(
            &core,
            sphere(r).0 - ring_volume,
            2.0 * PI * c * h + 2.0 * cap_area,
            k,
        );
        assert_eq!(counts(&core), (2, 2, 3, 1));
        let both = solve(&ball, &drill, Union, k);
        let stub = (6.0 - h) / 2.0;
        assert_mass(
            &both,
            ring_volume + PI * c * c * 6.0,
            2.0 * PI * r * h + 2.0 * PI * c * 2.0 * stub + 2.0 * PI * c * c,
            k,
        );
        assert_eq!(genus(&both.body), Some(0));
    }
}

#[test]
fn a_ring_cut_through_and_across_its_axis() {
    for k in SCALES {
        let (major, minor) = (3.0, 1.0);
        let ring = ring(k, [0.0, 0.0, 0.0], major, minor);
        let volume = 2.0 * PI * PI * major * minor * minor;
        let area = 4.0 * PI * PI * major * minor;
        // Through the axis: half a ring with two round ends.
        let beside = boxed(k, [0.0, -6.0, -3.0], [6.0, 6.0, 3.0]);
        let half = solve(&ring, &beside, Intersect, k);
        assert_mass(
            &half,
            volume / 2.0,
            area / 2.0 + 2.0 * PI * minor * minor,
            k,
        );
        assert_eq!(counts(&half), (2, 2, 3, 1));
        assert_eq!(genus(&half.body), Some(0));
        let other = solve(&ring, &beside, Subtract, k);
        assert_mass(
            &other,
            volume / 2.0,
            area / 2.0 + 2.0 * PI * minor * minor,
            k,
        );
        // A quarter, between two planes through the axis.
        let corner = boxed(k, [0.0, 0.0, -3.0], [6.0, 6.0, 3.0]);
        let quarter = solve(&ring, &corner, Intersect, k);
        assert_mass(
            &quarter,
            volume / 4.0,
            area / 4.0 + 2.0 * PI * minor * minor,
            k,
        );
        // Across the axis, through the middle of the tube: the upper half,
        // with a flat ring under it. By Pappus the half tube's centroid is
        // 4 minor / 3π above the cut, which does not change its distance
        // from the axis: half the volume. The outer half of the tube's
        // surface is larger than the inner half, but upper and lower are
        // alike.
        let above = boxed(k, [-6.0, -6.0, 0.0], [6.0, 6.0, 3.0]);
        let upper = solve(&ring, &above, Intersect, k);
        assert_mass(
            &upper,
            volume / 2.0,
            area / 2.0 + 4.0 * PI * major * minor,
            k,
        );
        assert_eq!(counts(&upper), (2, 2, 2, 1));
        assert_eq!(genus(&upper.body), Some(1));
        // Higher up: the cut is 2 sqrt(minor² - c²) wide and the part
        // above it is a segment of the tube's circle swept about the
        // axis. A segment's own centroid is on the line through the
        // tube's centre, so Pappus gives its area times 2π major.
        let c = 0.5_f64;
        let higher = boxed(k, [-6.0, -6.0, c], [6.0, 6.0, 3.0]);
        let slice = solve(&ring, &higher, Intersect, k);
        let half_angle = (c / minor).acos();
        let segment = minor * minor * (half_angle - half_angle.sin() * half_angle.cos());
        let chord = 2.0 * (minor * minor - c * c).sqrt();
        let m = mass(&slice.body);
        assert_close(m.volume, 2.0 * PI * major * segment * k * k * k, "volume");
        // The arc of the segment, swept: its centroid is on that line too.
        assert_close(
            m.area,
            (2.0 * PI * major * (2.0 * half_angle * minor) + 2.0 * PI * major * chord) * k * k,
            "area",
        );
        assert_eq!(genus(&slice.body), Some(1));
    }
}

#[test]
fn a_ball_ended_pin_and_the_hole_it_makes() {
    for k in SCALES {
        // A cylinder with a ball of its own radius centred on its end:
        // they touch along the cylinder's last circle, which is an edge
        // of the cylinder and so an edge of the result.
        let (r, h) = (0.5, 2.0);
        let shaft = rod(k, [2.0, 1.5, 1.0], r, h);
        let tip = ball(k, [2.0, 1.5, 1.0], r);
        let pin = solve(&shaft, &tip, Union, k);
        assert_mass(
            &pin,
            PI * r * r * h + sphere(r).0 / 2.0,
            2.0 * PI * r * h + PI * r * r + sphere(r).1 / 2.0,
            k,
        );
        assert_eq!(counts(&pin), (2, 2, 3, 1));
        assert_eq!(kinds(&pin.body).get("sphere"), Some(&1));
        // The other way round the same body.
        let other = solve(&tip, &shaft, Union, k);
        assert_mass(
            &other,
            PI * r * r * h + sphere(r).0 / 2.0,
            2.0 * PI * r * h + PI * r * r + sphere(r).1 / 2.0,
            k,
        );
        // Pressed into a plate from above: a blind hole with a round
        // bottom.
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 2.0]);
        let holed = solve(&plate, &pin.body, Subtract, k);
        assert_mass(
            &holed,
            24.0 - PI * r * r * 1.0 - sphere(r).0 / 2.0,
            52.0 - PI * r * r + 2.0 * PI * r * 1.0 + sphere(r).1 / 2.0,
            k,
        );
        assert_eq!(genus(&holed.body), Some(0));
        assert_eq!(counts(&holed), (10, 14, 8, 1));
    }
}

#[test]
fn a_countersunk_hole() {
    for k in SCALES {
        // A plate 1 thick; a cone point down from its top, 1 wide at the
        // top and 1.5 deep (so its apex is below the plate); then a hole
        // of radius 0.25 on the same axis.
        let (t, wide, deep, bore) = (1.0, 1.0, 1.5, 0.25);
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, t]);
        let sink = spike(k, [2.0, 1.5, t], wide, -deep);
        let sunk = solve(&plate, &sink, Subtract, k);
        // What the cone takes: a frustum from the top to the bottom face.
        let low = wide * (1.0 - t / deep);
        let frustum = |r0: f64, r1: f64, h: f64| PI * h * (r0 * r0 + r0 * r1 + r1 * r1) / 3.0;
        let slant = |r0: f64, r1: f64, h: f64| PI * (r0 + r1) * (r0 - r1).hypot(h);
        assert_mass(
            &sunk,
            12.0 - frustum(wide, low, t),
            38.0 - PI * wide * wide - PI * low * low + slant(wide, low, t),
            k,
        );
        assert_eq!(genus(&sunk.body), Some(1));
        // A shallower one stops inside the plate, apex and all.
        let dimple = spike(k, [2.0, 1.5, t], wide, -0.75);
        let dimpled = solve(&plate, &dimple, Subtract, k);
        assert_mass(
            &dimpled,
            12.0 - PI * wide * wide * 0.75 / 3.0,
            38.0 - PI * wide * wide + slant(wide, 0.0, 0.75),
            k,
        );
        assert_eq!(counts(&dimpled), (9, 13, 7, 1));
        // Drilled through on the same axis: the hole meets the cone in a
        // circle where the cone is as wide as the hole.
        let drill = rod(k, [2.0, 1.5, -1.0], bore, 3.0);
        let drilled = solve(&dimpled.body, &drill, Subtract, k);
        let meet = 0.75 * (1.0 - bore / wide);
        assert_mass(
            &drilled,
            12.0 - frustum(wide, bore, meet) - PI * bore * bore * (t - meet),
            38.0 - PI * wide * wide - PI * bore * bore
                + slant(wide, bore, meet)
                + 2.0 * PI * bore * (t - meet),
            k,
        );
        assert_eq!(genus(&drilled.body), Some(1));
        assert_eq!(counts(&drilled), (11, 15, 8, 1));
    }
}

#[test]
fn a_dome_on_a_plate() {
    for k in SCALES {
        let r = 1.0;
        let plate = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.5]);
        let ball = ball(k, [2.0, 1.5, 1.5], r);
        // Half the ball is in the plate already.
        let domed = solve(&plate, &ball, Union, k);
        assert_mass(
            &domed,
            18.0 + sphere(r).0 / 2.0,
            45.0 - PI * r * r + sphere(r).1 / 2.0,
            k,
        );
        assert_eq!(counts(&domed), (9, 13, 7, 1));
        // A hemisphere made first, standing on the plate with its flat
        // side on the plate's top: the same body.
        let above = boxed(k, [0.0, 0.0, 1.5], [4.0, 3.0, 4.0]);
        let dome = solve(&ball, &above, Intersect, k);
        assert_mass(&dome, sphere(r).0 / 2.0, sphere(r).1 / 2.0 + PI * r * r, k);
        let stood = solve(&plate, &dome.body, Union, k);
        assert_mass(
            &stood,
            18.0 + sphere(r).0 / 2.0,
            45.0 - PI * r * r + sphere(r).1 / 2.0,
            k,
        );
        assert_eq!(counts(&stood), (9, 13, 7, 1));
        // Nothing in common but the disc, and nothing to take away.
        assert_empty(&plate, &dome.body, Intersect, k);
        let same = solve(&plate, &dome.body, Subtract, k);
        assert_mass(&same, 18.0, 45.0, k);
        assert_eq!(counts(&same), (8, 12, 6, 1));
        // A bowl: the lower half taken out of the plate.
        let bowl = solve(&plate, &ball, Subtract, k);
        assert_mass(
            &bowl,
            18.0 - sphere(r).0 / 2.0,
            45.0 - PI * r * r + sphere(r).1 / 2.0,
            k,
        );
    }
}

#[test]
fn bodies_in_any_position() {
    // The same cuts as above with everything turned and moved: nothing
    // may depend on the world axes, on where a surface has its seam or
    // its poles.
    for k in SCALES {
        let frame = Frame::new(
            Vec3::new(3.0, -1.0, 2.0) * k,
            Vec3::new(0.2, -0.5, 1.0),
            Vec3::new(1.0, 0.3, 0.0),
        )
        .unwrap();
        let tol = tolerance(k);
        let at = |x: f64, y: f64, z: f64| frame.at(frame.point(x * k, y * k, z * k)).unwrap();
        let block = |lo: [f64; 3], hi: [f64; 3]| {
            primitives::block(
                &at(lo[0], lo[1], lo[2]),
                (hi[0] - lo[0]) * k,
                (hi[1] - lo[1]) * k,
                (hi[2] - lo[2]) * k,
            )
            .unwrap()
        };
        let run = |a: &craft3d_topo::Body, b: &craft3d_topo::Body, op| {
            let result = boolean(a, b, op, &tol).unwrap();
            assert_eq!(craft3d_topo::check(&result.body, &tol), []);
            mass(&result.body)
        };
        let (r, h) = (2.0, 0.75);
        let ball = primitives::sphere(&at(1.0, -0.5, 3.0), r * k).unwrap();
        let above = block([-4.0, -4.0, 3.0 + h], [6.0, 5.0, 9.0]);
        let (cap_volume, cap_area) = cap(r, r - h);
        let m = run(&ball, &above, Intersect);
        assert_close(m.volume, cap_volume * k * k * k, "cap volume");
        assert_close(
            m.area,
            (cap_area + PI * (r * r - h * h)) * k * k,
            "cap area",
        );
        let corner = block([1.0, -0.5, 3.0], [5.0, 4.0, 8.0]);
        let m = run(&ball, &corner, Intersect);
        assert_close(m.volume, sphere(r).0 / 8.0 * k * k * k, "eighth");

        let cone = primitives::cone(&at(1.0, 2.0, 0.0), 1.5 * k, 3.0 * k).unwrap();
        let beside = block([1.0, -4.0, -1.0], [6.0, 8.0, 9.0]);
        let m = run(&cone, &beside, Intersect);
        assert_close(m.volume, PI * 2.25 * 3.0 / 6.0 * k * k * k, "half cone");

        let ring = primitives::torus(&at(0.0, 0.0, 0.0), 3.0 * k, k).unwrap();
        let half = block([0.0, -6.0, -3.0], [6.0, 6.0, 3.0]);
        let m = run(&ring, &half, Subtract);
        assert_close(m.volume, PI * PI * 3.0 * k * k * k, "half ring");
        let upper = block([-6.0, -6.0, 0.25], [6.0, 6.0, 3.0]);
        let whole = run(&ring, &upper, Intersect).volume + run(&ring, &upper, Subtract).volume;
        assert_close(whole, 2.0 * PI * PI * 3.0 * k * k * k, "ring in two");
    }
}

#[test]
fn curves_that_are_only_approximations() {
    // Pairs of surfaces that meet in curves with no closed form: the edges
    // made from them are splines within the tolerance of both surfaces.
    for k in SCALES {
        let tol = tolerance(k);
        let near = |actual: f64, expected: f64, what: &str| {
            // An edge off by the tolerance over a size of a few units.
            assert!(
                (actual - expected).abs() <= 1e-5 * expected.abs(),
                "{what} at scale {k}: {actual} is not {expected}"
            );
        };
        let has_spline = |body: &craft3d_topo::Body| {
            body.edges
                .iter()
                .any(|e| matches!(e.curve, Curve3::Nurbs(_)))
        };
        // Two cylinders at right angles, the thinner through the thicker:
        // what they share and what is left add up, and the hole goes
        // through.
        let upright = rod(k, [0.0, 0.0, -2.0], 1.0, 4.0);
        let lying = log(k, [-3.0, 0.0, 0.0], 0.5, 6.0);
        let common = solve(&upright, &lying, Intersect, k);
        let less = solve(&upright, &lying, Subtract, k);
        let both = solve(&upright, &lying, Union, k);
        assert!(has_spline(&common.body));
        let (va, vb) = (PI * 4.0, PI * 0.25 * 6.0);
        let (vc, vl, vu) = (
            mass(&common.body).volume / (k * k * k),
            mass(&less.body).volume / (k * k * k),
            mass(&both.body).volume / (k * k * k),
        );
        near(vc + vl, va, "common + difference");
        near(vu + vc, va + vb, "union + common");
        assert_eq!(genus(&less.body), Some(1));
        assert_eq!(common.body.shells.len(), 1);
        // Equal cylinders at right angles: the solid they share has the
        // volume 16 r³ / 3 and the area 16 r² (Archimedes).
        let other = log(k, [-3.0, 0.0, 0.0], 1.0, 6.0);
        let steinmetz = solve(&upright, &other, Intersect, k);
        let m = mass(&steinmetz.body);
        near(
            m.volume,
            16.0 / 3.0 * k * k * k,
            "the volume two equal cylinders share",
        );
        near(m.area, 16.0 * k * k, "its area");
        // A plane at a slant through the centre of a ring halves it.
        let ring_ = ring(k, [0.0, 0.0, 0.0], 3.0, 1.0);
        let slanted = Frame::new(Vec3::ZERO, Vec3::new(0.3, 0.2, 1.0), Vec3::X).unwrap();
        let above = primitives::block(
            &slanted.at(slanted.point(-8.0 * k, -8.0 * k, 0.0)).unwrap(),
            16.0 * k,
            16.0 * k,
            8.0 * k,
        )
        .unwrap();
        let upper = boolean(&ring_, &above, Intersect, &tol).unwrap();
        let lower = boolean(&ring_, &above, Subtract, &tol).unwrap();
        assert_eq!(craft3d_topo::check(&upper.body, &tol), []);
        assert!(has_spline(&upper.body));
        near(
            mass(&upper.body).volume,
            PI * PI * 3.0 * k * k * k,
            "half a ring",
        );
        near(
            mass(&lower.body).volume,
            PI * PI * 3.0 * k * k * k,
            "the other half",
        );
        // A ball through the side of a cylinder, off its axis.
        let ball_ = ball(k, [0.8, 0.0, 0.0], 0.75);
        let bite = solve(&upright, &ball_, Subtract, k);
        let lump = solve(&upright, &ball_, Intersect, k);
        assert!(has_spline(&bite.body));
        near(
            mass(&bite.body).volume + mass(&lump.body).volume,
            va * k * k * k,
            "a cylinder in two parts",
        );
        // The bitten cylinder is an operand like any other.
        let slab = boxed(k, [-3.0, -3.0, -0.25], [3.0, 3.0, 0.25]);
        let slice = solve(&bite.body, &slab, Intersect, k);
        let whole = solve(&upright, &slab, Intersect, k);
        let lump_slice = solve(&lump.body, &slab, Intersect, k);
        near(
            mass(&slice.body).volume + mass(&lump_slice.body).volume,
            mass(&whole.body).volume,
            "a slice of both parts",
        );
    }
}
