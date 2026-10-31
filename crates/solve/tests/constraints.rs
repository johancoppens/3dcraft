//! Every constraint kind on its own.
//!
//! `alone_*`: a sketch with one constraint solves, and the geometric
//! quantity the constraint is about, measured here from the returned
//! coordinates with plain arithmetic, is within tolerance.
//!
//! `pinned_*`: with every point fixed first, the constraint cannot be met;
//! it is reported as conflicting and its residual is the geometric quantity
//! worked out by hand for that configuration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use std::f64::consts::PI;

use common::{options, solved};
use craft3d_solve::{Constraint as C, ConstraintStatus, EntityId, Sketch, Solution};

type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn len(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}
/// Distance of `p` from the infinite line through `a` and `b`.
fn line_dist(p: P, a: P, b: P) -> f64 {
    cross(sub(b, a), sub(p, a)).abs() / dist(a, b)
}
/// Counter-clockwise angle from `u` to `v`, in `(-pi, pi]`.
fn turn(u: P, v: P) -> f64 {
    cross(u, v).patan2(dot(u, v))
}

/// Four points in general position, two lines, two circles and an arc.
struct Scene {
    s: Sketch,
    p: [EntityId; 4],
    ab: EntityId,
    cd: EntityId,
    k1: EntityId,
    k2: EntityId,
    arc: EntityId,
    /// The arc's centre, start and end.
    arc_pts: [EntityId; 3],
    /// Centres of `k1` and `k2`.
    centers: [EntityId; 2],
}

fn scene() -> Scene {
    let mut s = Sketch::new();
    let p = [
        s.add_point(0.3, 0.2),
        s.add_point(4.1, 1.3),
        s.add_point(1.2, 3.4),
        s.add_point(5.3, 4.6),
    ];
    let ab = s.add_line(p[0], p[1]);
    let cd = s.add_line(p[2], p[3]);
    let c1 = s.add_point(-2.0, 1.0);
    let c2 = s.add_point(2.5, -3.0);
    let k1 = s.add_circle(c1, 1.5);
    let k2 = s.add_circle(c2, 0.8);
    let arc_pts = [
        s.add_point(7.0, 1.0),
        s.add_point(8.2, 1.1),
        s.add_point(7.1, 2.4),
    ];
    let arc = s.add_arc(arc_pts[0], arc_pts[1], arc_pts[2]);
    Scene {
        s,
        p,
        ab,
        cd,
        k1,
        k2,
        arc,
        arc_pts,
        centers: [c1, c2],
    }
}

const LINEAR: f64 = 1e-6;
const ANGULAR: f64 = 1e-9;

/// Solves the scene with one constraint and checks what every such solve
/// must show. Unknowns: 9 points (18) + 2 radii = 20; the arc takes 1.
fn alone(scene: &Scene, constraint: C, rows: usize, tolerance: f64) -> Solution {
    let mut sketch = scene.s.clone();
    sketch.add_constraint(constraint);
    let solution = solved(&sketch);
    assert_eq!(solution.unknowns, 20);
    assert_eq!(solution.dof, 20 - 1 - rows, "{constraint:?}");
    assert_eq!(solution.constraints.len(), 1);
    assert_eq!(
        solution.constraints[0].status,
        ConstraintStatus::Satisfied,
        "{constraint:?}"
    );
    assert!(
        solution.constraints[0].residual <= tolerance,
        "{constraint:?}: residual {}",
        solution.constraints[0].residual
    );
    // The arc is still an arc.
    let [c, s, e] = scene.arc_pts.map(|id| solution.point(id).unwrap());
    assert!((dist(c, s) - dist(c, e)).abs() <= LINEAR);
    solution
}

fn pts<const N: usize>(solution: &Solution, ids: [EntityId; N]) -> [P; N] {
    ids.map(|id| solution.point(id).unwrap())
}

#[test]
fn alone_coincident() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Coincident {
            a: sc.p[0],
            b: sc.p[2],
        },
        2,
        LINEAR,
    );
    let [a, _, c, _] = pts(&sol, sc.p);
    assert!(dist(a, c) <= LINEAR);
    // Both points move the same distance towards each other: they meet in
    // the middle of (0.3, 0.2) and (1.2, 3.4).
    assert!(dist(a, [0.75, 1.8]) <= 1e-9, "{a:?}");
}

#[test]
fn alone_point_on_line() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::PointOnLine {
            point: sc.p[2],
            line: sc.ab,
        },
        1,
        LINEAR,
    );
    let [a, b, c, _] = pts(&sol, sc.p);
    assert!(line_dist(c, a, b) <= LINEAR);
}

#[test]
fn alone_point_on_circle_and_on_arc() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::PointOnCircle {
            point: sc.p[0],
            circle: sc.k1,
        },
        1,
        LINEAR,
    );
    let center = sol.point(sc.centers[0]).unwrap();
    let on = sol.point(sc.p[0]).unwrap();
    assert!((dist(on, center) - sol.radius(sc.k1).unwrap()).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::PointOnCircle {
            point: sc.p[1],
            circle: sc.arc,
        },
        1,
        LINEAR,
    );
    let [c, s, _] = pts(&sol, sc.arc_pts);
    assert!((dist(sol.point(sc.p[1]).unwrap(), c) - dist(s, c)).abs() <= LINEAR);
}

#[test]
fn alone_horizontal_and_vertical() {
    let sc = scene();
    let sol = alone(&sc, C::Horizontal { line: sc.ab }, 1, LINEAR);
    let [a, b, ..] = pts(&sol, sc.p);
    assert!((a[1] - b[1]).abs() <= LINEAR);
    // The ends meet at the mean of their heights, (0.2 + 1.3) / 2, and
    // keep their x.
    assert!((a[1] - 0.75).abs() <= 1e-9 && (a[0] - 0.3).abs() <= 1e-9);

    let sc = scene();
    let sol = alone(&sc, C::Vertical { line: sc.cd }, 1, LINEAR);
    let [_, _, c, d] = pts(&sol, sc.p);
    assert!((c[0] - d[0]).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::HorizontalPoints {
            a: sc.p[0],
            b: sc.p[3],
        },
        1,
        LINEAR,
    );
    let [a, _, _, d] = pts(&sol, sc.p);
    assert!((a[1] - d[1]).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::VerticalPoints {
            a: sc.p[0],
            b: sc.p[3],
        },
        1,
        LINEAR,
    );
    let [a, _, _, d] = pts(&sol, sc.p);
    assert!((a[0] - d[0]).abs() <= LINEAR);
}

#[test]
fn alone_parallel_and_perpendicular() {
    let sc = scene();
    let sol = alone(&sc, C::Parallel { a: sc.ab, b: sc.cd }, 1, ANGULAR);
    let [a, b, c, d] = pts(&sol, sc.p);
    // The lines were drawn pointing the same way; they still do.
    assert!(turn(sub(b, a), sub(d, c)).abs() <= ANGULAR);

    let sc = scene();
    let sol = alone(&sc, C::Perpendicular { a: sc.ab, b: sc.cd }, 1, ANGULAR);
    let [a, b, c, d] = pts(&sol, sc.p);
    assert!((turn(sub(b, a), sub(d, c)).abs() - PI / 2.0).abs() <= ANGULAR);
}

#[test]
fn alone_tangent_line_and_round() {
    for round in [0, 1] {
        let sc = scene();
        let (id, center) = if round == 0 {
            (sc.k1, sc.centers[0])
        } else {
            (sc.arc, sc.arc_pts[0])
        };
        let sol = alone(&sc, C::Tangent { a: id, b: sc.ab }, 1, LINEAR);
        let [a, b, ..] = pts(&sol, sc.p);
        let c = sol.point(center).unwrap();
        assert!((line_dist(c, a, b) - sol.radius(id).unwrap()).abs() <= LINEAR);
    }
}

#[test]
fn alone_tangent_rounds() {
    // Circles drawn apart touch from outside.
    let sc = scene();
    let sol = alone(&sc, C::Tangent { a: sc.k1, b: sc.k2 }, 1, LINEAR);
    let [c1, c2] = pts(&sol, sc.centers);
    let (r1, r2) = (sol.radius(sc.k1).unwrap(), sol.radius(sc.k2).unwrap());
    assert!((dist(c1, c2) - (r1 + r2)).abs() <= LINEAR);

    // A circle and an arc.
    let sc = scene();
    let sol = alone(
        &sc,
        C::Tangent {
            a: sc.arc,
            b: sc.k2,
        },
        1,
        LINEAR,
    );
    let gap = dist(
        sol.point(sc.arc_pts[0]).unwrap(),
        sol.point(sc.centers[1]).unwrap(),
    );
    assert!((gap - (sol.radius(sc.arc).unwrap() + sol.radius(sc.k2).unwrap())).abs() <= LINEAR);

    // A small circle drawn inside a big one touches from inside.
    let mut s = Sketch::new();
    let c1 = s.add_point(0.0, 0.0);
    let big = s.add_circle(c1, 5.0);
    let c2 = s.add_point(1.0, 0.5);
    let small = s.add_circle(c2, 1.5);
    s.add_constraint(C::Tangent { a: small, b: big });
    let sol = solved(&s);
    let gap = dist(sol.point(c1).unwrap(), sol.point(c2).unwrap());
    let (rb, rs) = (sol.radius(big).unwrap(), sol.radius(small).unwrap());
    assert!((gap - (rb - rs)).abs() <= LINEAR && rb > rs);
    assert_eq!(sol.dof, 6 - 1);
}

#[test]
fn alone_tangent_at_a_shared_point() {
    // A line that starts at the end of an arc leaves it tangentially: the
    // radius to the shared point is perpendicular to the line.
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let start = s.add_point(3.0, 0.0);
    let end = s.add_point(0.2, 2.9);
    let arc = s.add_arc(c, start, end);
    let far = s.add_point(-4.0, 4.5);
    let line = s.add_line(end, far);
    s.add_constraint(C::Tangent { a: line, b: arc });
    let sol = solved(&s);
    let [c, end, far] = pts(&sol, [c, end, far]);
    let radius_direction = sub(end, c);
    let line_direction = sub(far, end);
    // (c - p) . d / |d| is a length; it is within the linear tolerance.
    assert!((dot(radius_direction, line_direction) / len(line_direction)).abs() <= LINEAR);
    assert!(sol.constraints[0].residual <= LINEAR);
    // 4 points = 8 unknowns, arc 1, tangent 1.
    assert_eq!(sol.dof, 6);

    // The same through a coincidence instead of a shared point: the line
    // has its own start point, tied to the arc's end.
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let start = s.add_point(3.0, 0.0);
    let end = s.add_point(0.2, 2.9);
    let arc = s.add_arc(c, start, end);
    let near = s.add_point(0.3, 3.1);
    let far = s.add_point(-4.0, 4.5);
    let line = s.add_line(near, far);
    s.add_constraint(C::Tangent { a: arc, b: line });
    s.add_constraint(C::Coincident { a: near, b: end });
    let sol = solved(&s);
    let [c, end, near, far] = pts(&sol, [c, end, near, far]);
    assert!(dist(near, end) <= LINEAR);
    assert!((dot(sub(end, c), sub(far, near)) / dist(far, near)).abs() <= LINEAR);
    // Tangency as a distance holds too: the line touches the circle.
    assert!((line_dist(c, near, far) - dist(c, end)).abs() <= LINEAR);
    // 5 points = 10 unknowns, arc 1, tangent 1, coincident 2. With the
    // distance form of tangency the Jacobian would lose a rank here.
    assert_eq!(sol.dof, 6);
    assert!(
        sol.constraints
            .iter()
            .all(|c| c.status == ConstraintStatus::Satisfied)
    );
}

#[test]
fn alone_equal() {
    let sc = scene();
    let sol = alone(&sc, C::Equal { a: sc.ab, b: sc.cd }, 1, LINEAR);
    let [a, b, c, d] = pts(&sol, sc.p);
    assert!((dist(a, b) - dist(c, d)).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(&sc, C::Equal { a: sc.k1, b: sc.k2 }, 1, LINEAR);
    assert!((sol.radius(sc.k1).unwrap() - sol.radius(sc.k2).unwrap()).abs() <= LINEAR);
    // Radii 1.5 and 0.8 meet halfway, at 1.15.
    assert!((sol.radius(sc.k1).unwrap() - 1.15).abs() <= 1e-9);

    let sc = scene();
    let sol = alone(
        &sc,
        C::Equal {
            a: sc.arc,
            b: sc.k1,
        },
        1,
        LINEAR,
    );
    let [c, s, _] = pts(&sol, sc.arc_pts);
    assert!((dist(c, s) - sol.radius(sc.k1).unwrap()).abs() <= LINEAR);
}

#[test]
fn alone_concentric() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Concentric {
            a: sc.k1,
            b: sc.arc,
        },
        2,
        LINEAR,
    );
    assert!(
        dist(
            sol.point(sc.centers[0]).unwrap(),
            sol.point(sc.arc_pts[0]).unwrap()
        ) <= LINEAR
    );
}

#[test]
fn alone_symmetric() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Symmetric {
            a: sc.p[2],
            b: sc.p[3],
            axis: sc.ab,
        },
        2,
        LINEAR,
    );
    let [a, b, c, d] = pts(&sol, sc.p);
    // Mirror c in the line ab by hand and compare with d.
    let u = [sub(b, a)[0] / dist(a, b), sub(b, a)[1] / dist(a, b)];
    let rel = sub(c, a);
    let along = dot(rel, u);
    let mirrored = [
        a[0] + 2.0 * along * u[0] - rel[0],
        a[1] + 2.0 * along * u[1] - rel[1],
    ];
    assert!(dist(mirrored, d) <= 2.0 * LINEAR, "{mirrored:?} vs {d:?}");
}

#[test]
fn alone_midpoint() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Midpoint {
            point: sc.p[2],
            line: sc.ab,
        },
        2,
        LINEAR,
    );
    let [a, b, c, _] = pts(&sol, sc.p);
    assert!(dist(c, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]) <= LINEAR);
}

#[test]
fn alone_fixed() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Fixed {
            point: sc.p[1],
            at: Some([-3.0, 7.5]),
        },
        2,
        LINEAR,
    );
    assert!(dist(sol.point(sc.p[1]).unwrap(), [-3.0, 7.5]) <= LINEAR);

    // Fixed where it is: exactly where it was.
    let sc = scene();
    let sol = alone(
        &sc,
        C::Fixed {
            point: sc.p[1],
            at: None,
        },
        2,
        LINEAR,
    );
    assert_eq!(sol.point(sc.p[1]).unwrap(), [4.1, 1.3]);
}

#[test]
fn alone_coordinates_are_signed() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::CoordinateX {
            point: sc.p[1],
            value: -2.75,
        },
        1,
        LINEAR,
    );
    let b = sol.point(sc.p[1]).unwrap();
    // Only x moves: a coordinate of one point, nothing else.
    assert!((b[0] + 2.75).abs() <= LINEAR && b[1] == 1.3, "{b:?}");

    let sc = scene();
    let sol = alone(
        &sc,
        C::CoordinateY {
            point: sc.p[2],
            value: -0.5,
        },
        1,
        LINEAR,
    );
    let c = sol.point(sc.p[2]).unwrap();
    assert!((c[1] + 0.5).abs() <= LINEAR && c[0] == 1.2, "{c:?}");
}

#[test]
fn pinned_coordinate_residual_is_the_offset() {
    let mut p = Pinned::new();
    let a = p.pt(1.0, 2.0);
    p.conflict(
        C::CoordinateX {
            point: a,
            value: -3.0,
        },
        4.0,
    );
    let mut p = Pinned::new();
    let a = p.pt(1.0, 2.0);
    p.conflict(
        C::CoordinateY {
            point: a,
            value: 2.5,
        },
        0.5,
    );
}

#[test]
fn alone_distances() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Distance {
            a: sc.p[0],
            b: sc.p[3],
            value: 2.5,
        },
        1,
        LINEAR,
    );
    let [a, _, _, d] = pts(&sol, sc.p);
    assert!((dist(a, d) - 2.5).abs() <= LINEAR);

    // Point and line, in both orders.
    for swap in [false, true] {
        let sc = scene();
        let (a, b) = if swap {
            (sc.ab, sc.p[2])
        } else {
            (sc.p[2], sc.ab)
        };
        let sol = alone(&sc, C::Distance { a, b, value: 1.25 }, 1, LINEAR);
        let [a, b, c, _] = pts(&sol, sc.p);
        assert!((line_dist(c, a, b) - 1.25).abs() <= LINEAR);
        // c was drawn on the left of ab and stays there.
        assert!(cross(sub(b, a), sub(c, a)) > 0.0);
    }

    let sc = scene();
    let sol = alone(
        &sc,
        C::HorizontalDistance {
            a: sc.p[0],
            b: sc.p[1],
            value: 6.0,
        },
        1,
        LINEAR,
    );
    let [a, b, ..] = pts(&sol, sc.p);
    assert!((b[0] - a[0] - 6.0).abs() <= LINEAR);
    // Nothing asked the points to change height.
    assert_eq!((a[1], b[1]), (0.2, 1.3));

    let sc = scene();
    let sol = alone(
        &sc,
        C::VerticalDistance {
            a: sc.p[3],
            b: sc.p[0],
            value: 6.0,
        },
        1,
        LINEAR,
    );
    let [a, _, _, d] = pts(&sol, sc.p);
    // p0 was drawn below p3 and stays below.
    assert!((d[1] - a[1] - 6.0).abs() <= LINEAR);
}

#[test]
fn alone_zero_distance_is_coincidence() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Distance {
            a: sc.p[0],
            b: sc.p[2],
            value: 0.0,
        },
        2,
        LINEAR,
    );
    let [a, _, c, _] = pts(&sol, sc.p);
    assert!(dist(a, c) <= LINEAR);
}

#[test]
fn alone_angle() {
    for value in [0.0, 0.5, PI / 2.0, 2.0, PI, -1.0, 4.0, 7.0] {
        let sc = scene();
        let sol = alone(
            &sc,
            C::Angle {
                a: sc.ab,
                b: sc.cd,
                value,
            },
            1,
            ANGULAR,
        );
        let [a, b, c, d] = pts(&sol, sc.p);
        // Compare unit vectors rather than angles, so that a value of 4 or
        // 7 radians (more than half a turn) needs no unwrapping: rotate ab's
        // direction by `value` and it must point along cd.
        let u = sub(b, a);
        let rotated = [
            u[0] * value.pcos() - u[1] * value.psin(),
            u[0] * value.psin() + u[1] * value.pcos(),
        ];
        assert!(
            turn(rotated, sub(d, c)).abs() <= ANGULAR,
            "angle {value}: off by {}",
            turn(rotated, sub(d, c))
        );
    }
}

#[test]
fn alone_radius_and_diameter() {
    let sc = scene();
    let sol = alone(
        &sc,
        C::Radius {
            circle: sc.k1,
            value: 2.75,
        },
        1,
        LINEAR,
    );
    assert!((sol.radius(sc.k1).unwrap() - 2.75).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::Diameter {
            circle: sc.k2,
            value: 2.75,
        },
        1,
        LINEAR,
    );
    assert!((2.0 * sol.radius(sc.k2).unwrap() - 2.75).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::Radius {
            circle: sc.arc,
            value: 2.75,
        },
        1,
        LINEAR,
    );
    let [c, s, e] = pts(&sol, sc.arc_pts);
    assert!((dist(c, s) - 2.75).abs() <= LINEAR && (dist(c, e) - 2.75).abs() <= LINEAR);

    let sc = scene();
    let sol = alone(
        &sc,
        C::Diameter {
            circle: sc.arc,
            value: 2.75,
        },
        1,
        LINEAR,
    );
    let [c, s, _] = pts(&sol, sc.arc_pts);
    assert!((2.0 * dist(c, s) - 2.75).abs() <= LINEAR);
}

/// A sketch whose points are all fixed where they are and whose circle
/// radii are pinned, with `constraint` added last.
struct Pinned {
    s: Sketch,
}

impl Pinned {
    fn new() -> Self {
        Pinned { s: Sketch::new() }
    }
    fn pt(&mut self, x: f64, y: f64) -> EntityId {
        let p = self.s.add_point(x, y);
        self.s.add_constraint(C::Fixed { point: p, at: None });
        p
    }
    fn line(&mut self, a: P, b: P) -> EntityId {
        let (a, b) = (self.pt(a[0], a[1]), self.pt(b[0], b[1]));
        self.s.add_line(a, b)
    }
    fn circle(&mut self, c: P, r: f64) -> EntityId {
        let c = self.pt(c[0], c[1]);
        let circle = self.s.add_circle(c, r);
        self.s.add_constraint(C::Radius { circle, value: r });
        circle
    }
    fn arc(&mut self, c: P, s: P, e: P) -> EntityId {
        let (c, s, e) = (
            self.pt(c[0], c[1]),
            self.pt(s[0], s[1]),
            self.pt(e[0], e[1]),
        );
        self.s.add_arc(c, s, e)
    }
    /// Adds the constraint and checks that it is the one conflict, with the
    /// residual `expected` (exact up to rounding: 1e-12).
    fn conflict(mut self, constraint: C, expected: f64) {
        let before = self.s.clone();
        let index = self.s.add_constraint(constraint);
        let solution = solved(&self.s);
        assert_eq!(solution.conflicting(), [index], "{constraint:?}");
        let residual = solution.constraints[index].residual;
        assert!(
            (residual - expected).abs() <= 1e-12,
            "{constraint:?}: residual {residual}, expected {expected}"
        );
        // Everything was pinned: nothing moved, nothing is free.
        assert_eq!(solution.entities, before.entities, "{constraint:?}");
        assert_eq!(solution.dof, 0);
        assert!(residual > options().tolerance.linear);
    }
}

#[test]
fn pinned_coincident_residual_is_the_distance() {
    let mut p = Pinned::new();
    let (a, b) = (p.pt(1.0, 2.0), p.pt(4.0, 6.0));
    // |(3, 4)| = 5.
    p.conflict(C::Coincident { a, b }, 5.0);
}

#[test]
fn pinned_point_on_line_residual_is_the_distance_from_the_line() {
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [10.0, 0.0]);
    let point = p.pt(3.0, 4.0);
    // (3, 4) is 4 above the x axis.
    p.conflict(C::PointOnLine { point, line }, 4.0);
}

#[test]
fn pinned_point_on_round_residual_is_the_radial_gap() {
    let mut p = Pinned::new();
    let circle = p.circle([0.0, 0.0], 2.0);
    let point = p.pt(3.0, 4.0);
    // 5 from the centre, radius 2.
    p.conflict(C::PointOnCircle { point, circle }, 3.0);

    let mut p = Pinned::new();
    let arc = p.arc([0.0, 0.0], [2.0, 0.0], [0.0, 2.0]);
    let point = p.pt(3.0, 4.0);
    p.conflict(C::PointOnCircle { point, circle: arc }, 3.0);
}

#[test]
fn pinned_horizontal_and_vertical_residuals_are_the_offsets() {
    // The segment (0, 0)-(3, 4) rises 4 and runs 3.
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [3.0, 4.0]);
    p.conflict(C::Horizontal { line }, 4.0);
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [3.0, 4.0]);
    p.conflict(C::Vertical { line }, 3.0);
    let mut p = Pinned::new();
    let (a, b) = (p.pt(0.0, 0.0), p.pt(3.0, 4.0));
    p.conflict(C::HorizontalPoints { a, b }, 4.0);
    let mut p = Pinned::new();
    let (a, b) = (p.pt(0.0, 0.0), p.pt(3.0, 4.0));
    p.conflict(C::VerticalPoints { a, b }, 3.0);
}

#[test]
fn pinned_parallel_and_perpendicular_residuals_are_sine_and_cosine() {
    // Directions (1, 0) and (3, 4) / 5: sine 4/5, cosine 3/5.
    let mut p = Pinned::new();
    let a = p.line([0.0, 0.0], [2.0, 0.0]);
    let b = p.line([1.0, 1.0], [4.0, 5.0]);
    p.conflict(C::Parallel { a, b }, 0.8);
    let mut p = Pinned::new();
    let a = p.line([0.0, 0.0], [2.0, 0.0]);
    let b = p.line([1.0, 1.0], [4.0, 5.0]);
    p.conflict(C::Perpendicular { a, b }, 0.6);
}

#[test]
fn pinned_tangent_residuals() {
    // Centre 3 above the x axis, radius 1: 2 short of touching.
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [10.0, 0.0]);
    let circle = p.circle([5.0, 3.0], 1.0);
    p.conflict(C::Tangent { a: line, b: circle }, 2.0);

    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [10.0, 0.0]);
    let arc = p.arc([5.0, 3.0], [5.0, 4.0], [4.0, 3.0]);
    p.conflict(C::Tangent { a: arc, b: line }, 2.0);

    // Centres 5 apart, radii 1 and 2: a gap of 2.
    let mut p = Pinned::new();
    let a = p.circle([0.0, 0.0], 1.0);
    let b = p.circle([5.0, 0.0], 2.0);
    p.conflict(C::Tangent { a, b }, 2.0);

    // Inside: centres 1 apart, radii 5 and 2 would touch at 3 apart.
    let mut p = Pinned::new();
    let a = p.circle([0.0, 0.0], 5.0);
    let b = p.circle([1.0, 0.0], 2.0);
    p.conflict(C::Tangent { a, b }, 2.0);
    // The same with the small circle named first.
    let mut p = Pinned::new();
    let a = p.circle([1.0, 0.0], 2.0);
    let b = p.circle([0.0, 0.0], 5.0);
    p.conflict(C::Tangent { a, b }, 2.0);
}

#[test]
fn pinned_tangent_at_a_shared_point_residuals() {
    // The arc around the origin ends at (2, 0); a line leaves from there
    // along (3, 4) / 5. The centre is (-2, 0) from the point, which is
    // -2 * 0.6 = -1.2 along the line.
    let mut p = Pinned::new();
    let c = p.pt(0.0, 0.0);
    let start = p.pt(0.0, -2.0);
    let end = p.pt(2.0, 0.0);
    let far = p.pt(5.0, 4.0);
    let arc = p.s.add_arc(c, start, end);
    let line = p.s.add_line(end, far);
    p.conflict(C::Tangent { a: line, b: arc }, 1.2);

    // Two arcs sharing the point (2, 0): the first centre is the origin, so
    // the second centre must be on the x axis; (5, 4) is 4 away from it.
    let mut p = Pinned::new();
    let c1 = p.pt(0.0, 0.0);
    let start = p.pt(0.0, -2.0);
    let joint = p.pt(2.0, 0.0);
    let c2 = p.pt(5.0, 4.0);
    let end = p.pt(10.0, 4.0);
    let first = p.s.add_arc(c1, start, joint);
    let second = p.s.add_arc(c2, joint, end);
    p.conflict(
        C::Tangent {
            a: first,
            b: second,
        },
        4.0,
    );
}

#[test]
fn pinned_equal_residuals_are_the_differences() {
    // Lengths 5 and 2.
    let mut p = Pinned::new();
    let a = p.line([0.0, 0.0], [3.0, 4.0]);
    let b = p.line([1.0, 1.0], [1.0, 3.0]);
    p.conflict(C::Equal { a, b }, 3.0);
    // Radii 2 and 3.5.
    let mut p = Pinned::new();
    let a = p.circle([0.0, 0.0], 2.0);
    let b = p.circle([9.0, 0.0], 3.5);
    p.conflict(C::Equal { a, b }, 1.5);
}

#[test]
fn pinned_concentric_residual_is_the_centre_distance() {
    let mut p = Pinned::new();
    let a = p.circle([1.0, 2.0], 1.0);
    let b = p.arc([4.0, 6.0], [5.0, 6.0], [4.0, 7.0]);
    p.conflict(C::Concentric { a, b }, 5.0);
}

#[test]
fn pinned_symmetric_residual() {
    // Axis: the x axis. a = (1, 2), b = (4, -6). Their midpoint (2.5, -2)
    // is 2 off the axis, and b - a = (3, -8) has 3 along the axis:
    // sqrt(2^2 + 3^2).
    let mut p = Pinned::new();
    let axis = p.line([0.0, 0.0], [10.0, 0.0]);
    let (a, b) = (p.pt(1.0, 2.0), p.pt(4.0, -6.0));
    p.conflict(C::Symmetric { a, b, axis }, 13f64.sqrt());
}

#[test]
fn pinned_midpoint_residual_is_the_distance_from_the_middle() {
    // Middle of (0, 0)-(6, 8) is (3, 4); (3, 9) is 5 above it.
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [6.0, 8.0]);
    let point = p.pt(3.0, 9.0);
    p.conflict(C::Midpoint { point, line }, 5.0);
}

#[test]
fn pinned_fixed_residual_is_the_distance_to_the_target() {
    let mut p = Pinned::new();
    let point = p.pt(1.0, 2.0);
    p.conflict(
        C::Fixed {
            point,
            at: Some([4.0, 6.0]),
        },
        5.0,
    );
}

#[test]
fn pinned_distance_residuals() {
    // (1, 2) and (4, 6) are 5 apart, 3 across and 4 up.
    let mut p = Pinned::new();
    let (a, b) = (p.pt(1.0, 2.0), p.pt(4.0, 6.0));
    p.conflict(C::Distance { a, b, value: 3.0 }, 2.0);
    let mut p = Pinned::new();
    let (a, b) = (p.pt(1.0, 2.0), p.pt(4.0, 6.0));
    p.conflict(C::HorizontalDistance { a, b, value: 1.0 }, 2.0);
    let mut p = Pinned::new();
    let (a, b) = (p.pt(1.0, 2.0), p.pt(4.0, 6.0));
    p.conflict(
        C::VerticalDistance {
            a: b,
            b: a,
            value: 1.0,
        },
        3.0,
    );
    // (3, 4) is 4 from the x axis; 1 was asked.
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [10.0, 0.0]);
    let point = p.pt(3.0, 4.0);
    p.conflict(
        C::Distance {
            a: point,
            b: line,
            value: 1.0,
        },
        3.0,
    );
    // Below the axis the distance is the same.
    let mut p = Pinned::new();
    let line = p.line([0.0, 0.0], [10.0, 0.0]);
    let point = p.pt(3.0, -4.0);
    p.conflict(
        C::Distance {
            a: line,
            b: point,
            value: 1.0,
        },
        3.0,
    );
}

#[test]
fn pinned_angle_residual_is_the_angle_missing() {
    // From +x to +y is a quarter turn; a twelfth was asked: pi/2 - pi/6.
    let mut p = Pinned::new();
    let a = p.line([0.0, 0.0], [2.0, 0.0]);
    let b = p.line([1.0, 1.0], [1.0, 3.0]);
    p.conflict(
        C::Angle {
            a,
            b,
            value: PI / 6.0,
        },
        PI / 3.0,
    );
    // Measured the other way round it is -pi/2; -pi/6 asked.
    let mut p = Pinned::new();
    let a = p.line([0.0, 0.0], [2.0, 0.0]);
    let b = p.line([1.0, 1.0], [1.0, 3.0]);
    p.conflict(
        C::Angle {
            a: b,
            b: a,
            value: -PI / 6.0,
        },
        PI / 3.0,
    );
}

#[test]
fn pinned_radius_and_diameter_residuals() {
    let mut p = Pinned::new();
    let circle = p.circle([0.0, 0.0], 2.0);
    p.conflict(C::Radius { circle, value: 5.0 }, 3.0);
    // Diameter 4, 5 asked.
    let mut p = Pinned::new();
    let circle = p.circle([0.0, 0.0], 2.0);
    p.conflict(C::Diameter { circle, value: 5.0 }, 1.0);
    let mut p = Pinned::new();
    let circle = p.arc([0.0, 0.0], [2.0, 0.0], [0.0, 2.0]);
    p.conflict(C::Radius { circle, value: 5.0 }, 3.0);
    let mut p = Pinned::new();
    let circle = p.arc([0.0, 0.0], [2.0, 0.0], [0.0, 2.0]);
    p.conflict(C::Diameter { circle, value: 5.0 }, 1.0);
}
