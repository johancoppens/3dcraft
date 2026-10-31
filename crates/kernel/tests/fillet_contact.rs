//! A fillet built in two ways: by the blend, and as a boolean of the body
//! with the fillet's tool, whose face is tangent to the faces it rounds
//! along curves (checklist item 2 of docs/roadmap.md, "the contact a
//! fillet's boolean refused"). The two must be the same body within the
//! tolerance: volume, area, faces, the checker and the boolean's own check.
//!
//! The tools are closed forms (a cylinder along a straight edge, a torus
//! round a circular edge), the same tools with their curved face on the
//! rational patch that is that piece of the cylinder or torus (a freeform
//! face tangent to a plane along a line or a circle, with no closed form
//! for the contact; a patch that ends on the tangent curve and one a fifth
//! larger, so that the contact runs through the middle of the patch), the
//! exact tool of a rolling-ball fillet between two spline walls, and the
//! tool a rolling-ball fillet takes away (the body less the fillet) along
//! a straight edge between a plane and a cone and along the rim of a
//! lofted blade's tip.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;
mod common;
mod rational_common;

use boolean_common::{SCALES, assert_empty, boxed, mass, rod, solve, tolerance};
use common::{arc, line, scale_curves};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, EdgeId, check};
use rational_common::rationalized;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// What a square corner loses to a round of radius 1.
const SPANDREL: f64 = 1.0 - PI / 4.0;

/// How far in from both faces the centroid of that spandrel lies, for a
/// radius of 1.
const INSET: f64 = (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);

fn edge_near(body: &Body, p: Vec3) -> EdgeId {
    let (i, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let d = (0..=32)
                .map(|j| {
                    let t = e.range.0 + (e.range.1 - e.range.0) * f64::from(j) / 32.0;
                    e.curve.eval(t).distance(p)
                })
                .fold(f64::INFINITY, f64::min);
            (i, d)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    EdgeId(i as u32)
}

/// The length of all edges of a body.
fn edge_length(body: &Body) -> f64 {
    body.edges
        .iter()
        .map(|e| {
            let n = 64;
            (0..n)
                .map(|j| {
                    let at = |j: usize| {
                        e.curve
                            .eval(e.range.0 + (e.range.1 - e.range.0) * j as f64 / n as f64)
                    };
                    at(j).distance(at(j + 1))
                })
                .sum::<f64>()
        })
        .sum()
}

/// Two bodies that are the same solid within the tolerance: every point of
/// one within the tolerance of the other moves the volume by no more than
/// the tolerance times the area, and the area by no more than a band of
/// the tolerance along every edge; the same faces, shells, edges and
/// vertices.
#[track_caller]
fn same_body(made: &Body, other: &Body, tol: &Tolerance, what: &str) {
    same_body_but(made, other, tol, 0.0, what);
}

/// [`same_body`], except that `made` may have vertices of its own within
/// `reach` of a vertex of `other`, each with an edge of its own: where an
/// edge of one operand touches a face of the other tangentially, the place
/// is known only to within the length over which the edge stays within
/// the tolerance of the face, and a vertex of the other operand there
/// is not that place (structural debt: snapping to tolerant entities).
#[track_caller]
fn same_body_but(made: &Body, other: &Body, tol: &Tolerance, reach: f64, what: &str) {
    assert_eq!(check(made, tol), [], "{what}");
    assert_eq!(check(other, tol), [], "{what}");
    let (m, o) = (mass(made), mass(other));
    let dv = (m.volume - o.volume).abs();
    let da = (m.area - o.area).abs();
    println!(
        "{what}: volume {:.12e} against {:.12e} ({:.1e} of tolerance × area), area {:.1e} of tolerance × edges",
        m.volume,
        o.volume,
        dv / (tol.linear * m.area),
        da / (tol.linear * edge_length(made))
    );
    assert!(
        dv <= tol.linear * m.area,
        "{what}: volume {} and {}",
        m.volume,
        o.volume
    );
    assert!(
        da <= tol.linear * edge_length(made),
        "{what}: area {} and {}",
        m.area,
        o.area
    );
    assert_eq!(made.faces.len(), other.faces.len(), "{what}: faces");
    assert_eq!(made.shells.len(), other.shells.len(), "{what}: shells");
    let own: Vec<f64> = made
        .vertices
        .iter()
        .map(|v| {
            other
                .vertices
                .iter()
                .map(|w| w.point.distance(v.point))
                .fold(f64::INFINITY, f64::min)
        })
        .filter(|d| *d > 2.0 * tol.linear)
        .collect();
    // (Where the counts agree, a vertex may still be elsewhere: the one
    // vertex of a closed edge is wherever its maker put it.)
    let surplus = made.vertices.len().saturating_sub(other.vertices.len());
    if made.vertices.len() != other.vertices.len() {
        assert!(
            own.len() == surplus && own.iter().all(|d| *d <= reach),
            "{what}: vertices of its own {own:?}"
        );
    }
    assert_eq!(
        made.vertices.len(),
        other.vertices.len() + surplus,
        "{what}: vertices"
    );
    assert_eq!(
        made.edges.len(),
        other.edges.len() + surplus,
        "{what}: edges"
    );
}

#[track_caller]
fn close_to(actual: f64, expected: f64, slack: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= slack,
        "{what}: {actual} is not {expected} within {slack}"
    );
}

/// The section of what rounding the corner (1, 1) of the quarter plane
/// x, y ≤ 1 takes away, at radius `r`, times `k`: the two lines along the
/// faces from the corner to where the ball touches them, and the arc.
fn corner_section(r: f64, k: f64, tol: &Tolerance) -> Vec<craft3d_kernel::profile::Profile> {
    let curves = [
        line(1, [1.0, 1.0], [1.0, 1.0 - r]),
        arc(2, [1.0 - r, 1.0 - r], [1.0, 1.0 - r], [1.0 - r, 1.0]),
        line(3, [1.0 - r, 1.0], [1.0, 1.0]),
    ];
    detect_profiles(&scale_curves(&curves, k), tol).unwrap()
}

fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// A straight edge of a block rounded by the blend (a cylinder tool) and
/// by the boolean with a tool made here: the corner section extruded flush
/// with the block and past both ends, with its face on the cylinder and on
/// the rational patch that is that quarter of it, the patch ending on the
/// lines where it touches the block's faces and a fifth larger. The
/// patch is tangent to two planes along lines in the middle of it: a
/// contact with no closed form.
#[test]
fn a_straight_edge_rounded_by_its_tool() {
    let r = 0.2;
    for k in SCALES {
        let tol = tolerance(k);
        for tilted in [false, true] {
            let place = |b: Body| {
                if tilted {
                    b.transformed(&tilt(k)).unwrap()
                } else {
                    b
                }
            };
            let block = place(boxed(k, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
            let at = |p: Vec3| if tilted { tilt(k).point(p) } else { p };
            let edge = edge_near(&block, at(Vec3::new(1.0, 1.0, 0.5) * k));
            let made = fillet(
                &block,
                &[FilletEdge {
                    edge,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap();
            let m = mass(&made.body);
            close_to(
                m.volume,
                (1.0 - SPANDREL * r * r) * k * k * k,
                1e-9 * k * k * k,
                "the blend's volume",
            );
            let section = corner_section(r, k, &tol);
            for (ends, lo, hi) in [("flush", 0.0, 1.0), ("past the ends", -0.5, 1.5)] {
                let tool = place(
                    extrude(&section, &Frame::WORLD, lo * k, hi * k, &tol)
                        .unwrap()
                        .body,
                );
                for pad in [None, Some(0.0), Some(0.2)] {
                    let tool = match pad {
                        None => tool.clone(),
                        Some(pad) => {
                            let (twin, replaced) = rationalized(&tool, pad, false);
                            assert_eq!(replaced, 1);
                            twin
                        }
                    };
                    let what = format!("scale {k}, tilted {tilted}, {ends}, patch {pad:?}");
                    let cut = solve(&block, &tool, Subtract, k);
                    same_body(&cut.body, &made.body, &tol, &what);
                    // The identities: what the tool shares with the block
                    // and the block less it make up the block.
                    let common = solve(&block, &tool, Intersect, k);
                    close_to(
                        mass(&common.body).volume + mass(&cut.body).volume,
                        mass(&block).volume,
                        tol.linear * mass(&block).area,
                        &format!("{what}: common and cut"),
                    );
                    // The fillet and its tool only touch.
                    assert_empty(&made.body, &tool, Intersect, k);
                    if ends == "flush" {
                        let whole = solve(&made.body, &tool, Union, k);
                        same_body(&whole.body, &block, &tol, &format!("{what}: put back"));
                    }
                }
            }
        }
    }
}

/// The rim of a rod rounded by the blend (a torus tool) and by the boolean
/// with the corner section revolved about the axis; and a quarter of that
/// tool with its torus face on a rational patch (ending on the circles
/// where it touches the rod, and a fifth larger), against the analytic
/// quarter. Pappus gives the volumes.
#[test]
fn a_circular_edge_rounded_by_its_tool() {
    let r = 0.2;
    let removed = |turn: f64| turn * (1.0 - r * INSET) * SPANDREL * r * r;
    for k in SCALES {
        let tol = tolerance(k);
        let cylinder = rod(k, [0.0, 0.0, 0.0], 1.0, 1.0);
        let edge = edge_near(&cylinder, Vec3::new(1.0, 0.0, 1.0) * k);
        let made = fillet(
            &cylinder,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap();
        let k3 = k * k * k;
        close_to(
            mass(&made.body).volume,
            (PI - removed(TAU)) * k3,
            1e-9 * k3,
            "the blend's volume",
        );
        let xz = Frame::new(Vec3::ZERO, -Vec3::Y, Vec3::X).unwrap();
        let section = corner_section(r, k, &tol);
        let whole = revolve(&section, &xz, [0.0, 0.0], [0.0, 1.0], 0.0, TAU, &tol)
            .unwrap()
            .body;
        let cut = solve(&cylinder, &whole, Subtract, k);
        same_body(
            &cut.body,
            &made.body,
            &tol,
            &format!("scale {k}, whole ring"),
        );
        let quarter = revolve(&section, &xz, [0.0, 0.0], [0.0, 1.0], 0.0, FRAC_PI_2, &tol)
            .unwrap()
            .body;
        let analytic = solve(&cylinder, &quarter, Subtract, k);
        close_to(
            mass(&analytic.body).volume,
            (PI - removed(FRAC_PI_2)) * k3,
            tol.linear * mass(&analytic.body).area,
            "a quarter",
        );
        for pad in [0.0, 0.2] {
            let (twin, replaced) = rationalized(&quarter, pad, false);
            assert_eq!(replaced, 2, "the torus face and the side strip");
            let what = format!("scale {k}, a quarter on patches padded {pad}");
            let cut = solve(&cylinder, &twin, Subtract, k);
            same_body(&cut.body, &analytic.body, &tol, &what);
            let common = solve(&cylinder, &twin, Intersect, k);
            close_to(
                mass(&common.body).volume,
                removed(FRAC_PI_2) * k3,
                tol.linear * mass(&common.body).area,
                &format!("{what}: common"),
            );
        }
    }
}

// ----- a rolling-ball fillet between two spline walls ---------------------

type P2 = [f64; 2];

fn bez_at(p: &[P2; 4], t: f64) -> P2 {
    let s = 1.0 - t;
    let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
    [
        b[0] * p[0][0] + b[1] * p[1][0] + b[2] * p[2][0] + b[3] * p[3][0],
        b[0] * p[0][1] + b[1] * p[1][1] + b[2] * p[2][1] + b[3] * p[3][1],
    ]
}

fn bez_d(p: &[P2; 4], t: f64) -> P2 {
    let s = 1.0 - t;
    let c = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
    let q = |i: usize, k: usize| p[i + 1][k] - p[i][k];
    [
        c[0] * q(0, 0) + c[1] * q(1, 0) + c[2] * q(2, 0),
        c[0] * q(0, 1) + c[1] * q(1, 1) + c[2] * q(2, 1),
    ]
}

fn lerp(a: P2, b: P2, t: f64) -> P2 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// The two halves of a cubic Bézier at `t` (de Casteljau).
fn split(p: &[P2; 4], t: f64) -> ([P2; 4], [P2; 4]) {
    let (a, b, c) = (
        lerp(p[0], p[1], t),
        lerp(p[1], p[2], t),
        lerp(p[2], p[3], t),
    );
    let (d, e) = (lerp(a, b, t), lerp(b, c, t));
    let f = lerp(d, e, t);
    ([p[0], a, d, f], [f, e, c, p[3]])
}

/// The parameter of the point of a cubic Bézier nearest `q`: a fine
/// search, then Newton.
fn nearest(p: &[P2; 4], q: P2) -> f64 {
    let mut best = (0.0, f64::INFINITY);
    for i in 0..=4000 {
        let t = f64::from(i) / 4000.0;
        let a = bez_at(p, t);
        let d = (a[0] - q[0]).phypot(a[1] - q[1]);
        if d < best.1 {
            best = (t, d);
        }
    }
    let g = |t: f64| {
        let (a, d) = (bez_at(p, t), bez_d(p, t));
        (a[0] - q[0]) * d[0] + (a[1] - q[1]) * d[1]
    };
    let mut t = best.0;
    for _ in 0..60 {
        let h = 1e-7;
        let (f, f2) = (g(t), g(t + h));
        let step = f * h / (f2 - f);
        t = (t - step).clamp(0.0, 1.0);
        if step.abs() < 1e-16 {
            break;
        }
    }
    t
}

fn spline(id: u64, control: &[P2; 4]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Spline(Spline2 {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control: control.to_vec(),
        }),
    }
}

/// The root section of the turbofan's fan blade (blend_freeform.rs): two
/// cubic Béziers that meet at both ends.
const SUCTION: [P2; 4] = [
    [-7.0421, 42.7558],
    [-8.9846, 43.7886],
    [-7.8518, 55.0783],
    [7.0421, 69.2442],
];
const PRESSURE: [P2; 4] = [
    [7.0421, 69.2442],
    [-1.852, 55.2859],
    [-5.1711, 42.4405],
    [-7.0421, 42.7558],
];

/// The section of what a rolling ball of radius `r` takes from the corner
/// where the suction side starts: the suction side from the corner to the
/// ball's foot on it, the arc of the ball, the pressure side from its foot
/// there back to the corner. The ball's centre is `r` in from the suction
/// side where it is `r` from the pressure side too.
fn blade_corner(r: f64) -> Vec<SketchCurve> {
    let inward = |t: f64| {
        let d = bez_d(&SUCTION, t);
        let len = d[0].phypot(d[1]);
        [d[1] / len, -d[0] / len]
    };
    let centre_at = |t: f64| {
        let (p, n) = (bez_at(&SUCTION, t), inward(t));
        [p[0] + r * n[0], p[1] + r * n[1]]
    };
    let gap = |t: f64| {
        let m = centre_at(t);
        let q = bez_at(&PRESSURE, nearest(&PRESSURE, m));
        (q[0] - m[0]).phypot(q[1] - m[1]) - r
    };
    let (mut lo, mut hi) = (1e-4, 0.5);
    assert!(gap(lo) < 0.0 && gap(hi) > 0.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if gap(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t1 = 0.5 * (lo + hi);
    let m = centre_at(t1);
    let t2 = nearest(&PRESSURE, m);
    let (along_suction, _) = split(&SUCTION, t1);
    let (_, along_pressure) = split(&PRESSURE, t2);
    let (q1, q2) = (along_suction[3], along_pressure[0]);
    // Counter-clockwise from start to end, the short way.
    let (a1, a2) = (
        (q1[1] - m[1]).patan2(q1[0] - m[0]),
        (q2[1] - m[1]).patan2(q2[0] - m[0]),
    );
    let (start, end) = if (a2 - a1).rem_euclid(TAU) < PI {
        (q1, q2)
    } else {
        (q2, q1)
    };
    vec![
        spline(1, &along_suction),
        arc(2, m, start, end),
        spline(3, &along_pressure),
    ]
}

/// The blade root's edge rounded in place by the rolling ball (a spline
/// face within a quarter of the tolerance of the exact one) and by the
/// boolean with the exact tool: the corner section extruded, flush and
/// past both ends, with its face on the cylinder and on a rational patch a
/// fifth larger than it. The cylinder and the patch are tangent to the
/// spline walls along lines (no closed form for either contact), and the
/// tool's walls lie on the blade's (two spline faces on one surface).
#[test]
fn a_rolling_fillet_between_spline_walls_and_its_exact_tool() {
    let (r, height) = (0.3, 5.0);
    for k in SCALES {
        let tol = tolerance(k);
        let walls = detect_profiles(
            &scale_curves(&[spline(1, &SUCTION), spline(2, &PRESSURE)], k),
            &tol,
        )
        .unwrap();
        let body = extrude(&walls, &Frame::WORLD, 0.0, height * k, &tol)
            .unwrap()
            .body;
        let corner = SUCTION[0];
        let edge = edge_near(&body, Vec3::new(corner[0], corner[1], 0.5 * height) * k);
        let made = fillet(
            &body,
            &[FilletEdge {
                edge,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap();
        let section = detect_profiles(&scale_curves(&blade_corner(r), k), &tol).unwrap();
        for (ends, lo, hi) in [
            ("flush", 0.0, height),
            ("past the ends", -1.0, height + 1.0),
        ] {
            let exact = extrude(&section, &Frame::WORLD, lo * k, hi * k, &tol)
                .unwrap()
                .body;
            let (twin, replaced) = rationalized(&exact, 0.2, false);
            assert_eq!(replaced, 1);
            for (face, tool) in [("cylinder", exact), ("patch", twin)] {
                let what = format!("scale {k}, {ends}, a {face}");
                let cut = solve(&body, &tool, Subtract, k);
                same_body(&cut.body, &made.body, &tol, &what);
                if ends == "flush" {
                    // All of the tool is in the body.
                    close_to(
                        mass(&cut.body).volume,
                        mass(&body).volume - mass(&tool).volume,
                        tol.linear * mass(&body).area,
                        &format!("{what}: the body less the tool"),
                    );
                }
                let common = solve(&body, &tool, Intersect, k);
                close_to(
                    mass(&common.body).volume + mass(&cut.body).volume,
                    mass(&body).volume,
                    tol.linear * mass(&body).area,
                    &format!("{what}: common and cut"),
                );
            }
        }
    }
}

// ----- the tool a rolling-ball fillet takes away --------------------------

/// Half a frustum: the cone of radius 1 and height 3 about z, cut at
/// z = 1 and kept where y ≥ 0 (blend_freeform.rs).
fn half_frustum(k: f64) -> Body {
    let cone = primitives::cone(&Frame::WORLD, k, 3.0 * k).unwrap();
    let half = boxed(k, [-2.0, 0.0, -1.0], [2.0, 2.0, 1.0]);
    solve(&cone, &half, Intersect, k).body
}

/// A blade lofted through three sections of two cubic Béziers, smaller and
/// turned towards its tip at z = 3 (blend_chains.rs).
fn lofted_blade(k: f64) -> Body {
    use craft3d_kernel::loft::{Options, Section, loft};
    const BACK: [P2; 4] = [[-1.0, 0.0], [-0.5, 0.3], [0.4, 0.3], [1.0, 0.0]];
    const FRONT: [P2; 4] = [[1.0, 0.0], [0.4, 0.12], [-0.5, 0.12], [-1.0, 0.0]];
    let tol = tolerance(k);
    let turned = |control: &[P2; 4], s: f64, angle: f64| {
        let (sin, cos) = f64::psin_cos(angle);
        let mut out = *control;
        for p in &mut out {
            let (x, y) = (p[0] * s, p[1] * s);
            *p = [(x * cos - y * sin) * k, (x * sin + y * cos) * k];
        }
        out
    };
    let sections: Vec<Section> = [(0.0, 1.0, 0.0), (1.5, 0.9, 0.15), (3.0, 0.8, 0.3)]
        .iter()
        .map(|&(z, s, angle)| {
            let curves = [
                spline(1, &turned(&BACK, s, angle)),
                spline(2, &turned(&FRONT, s, angle)),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            Section {
                plane: Frame::WORLD.at(Vec3::new(0.0, 0.0, z * k)).unwrap(),
                outline: profiles[0].outer.clone(),
                start: None,
            }
        })
        .collect();
    loft(&sections, &Options::default(), &tol).unwrap().body
}

/// Where the tip's rim on the back of the lofted blade is, at scale 1.
fn tip_rim() -> Vec3 {
    let control = [[-1.0, 0.0], [-0.5, 0.3], [0.4, 0.3], [1.0, 0.0]];
    let p = bez_at(&control, 0.5);
    let (sin, cos) = 0.3_f64.psin_cos();
    let (x, y) = (p[0] * 0.8, p[1] * 0.8);
    Vec3::new(x * cos - y * sin, x * sin + y * cos, 3.0)
}

/// What a rolling-ball fillet takes away (the body less the fillet: a
/// solid whose spline face is tangent to two faces of the body along its
/// borders), taken away again by the boolean, gives the fillet; put back,
/// the body; and it shares nothing with the fillet. Along a straight edge
/// between a plane and a cone, and along the rim of a lofted blade's tip.
#[test]
fn the_tool_a_rolling_fillet_takes_away() {
    for k in SCALES {
        let tol = tolerance(k);
        let cases: Vec<(&str, Body, Vec3, f64)> = vec![
            (
                "plane and cone",
                half_frustum(k),
                Vec3::new(5.0 / 6.0, 0.0, 0.5),
                0.15,
            ),
            ("lofted tip", lofted_blade(k), tip_rim(), 0.02),
        ];
        for (name, body, near, r) in cases {
            let what = format!("{name} at scale {k}");
            let edge = edge_near(&body, near * k);
            let made = fillet(
                &body,
                &[FilletEdge {
                    edge,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap();
            let tool = solve(&body, &made.body, Subtract, k).body;
            close_to(
                mass(&tool).volume,
                mass(&body).volume - mass(&made.body).volume,
                tol.linear * mass(&body).area,
                &format!("{what}: the tool"),
            );
            let cut = solve(&body, &tool, Subtract, k);
            // (Where the body's edge across the end of the fillet touches
            // the tool's face, the touch is placed within its reach.)
            let reach = (2.0 * tol.linear * r * k).sqrt();
            same_body_but(&cut.body, &made.body, &tol, reach, &format!("{what}: cut"));
            // Put back, the body: vertices of the fillet or of the tool
            // where their ends meet the body's edges stay (the merge keeps
            // a vertex its operand had), each with the edge it splits.
            let back = solve(&made.body, &tool, Union, k);
            let theirs = |p: Vec3| {
                made.body
                    .vertices
                    .iter()
                    .chain(&tool.vertices)
                    .any(|v| v.point.distance(p) <= 2.0 * tol.linear)
            };
            let kept = back
                .body
                .vertices
                .iter()
                .filter(|v| {
                    !body
                        .vertices
                        .iter()
                        .any(|w| w.point.distance(v.point) <= 2.0 * tol.linear)
                })
                .inspect(|v| {
                    assert!(
                        theirs(v.point),
                        "{what}: put back: a vertex at {:?}",
                        v.point
                    )
                })
                .count();
            assert_eq!(
                back.body.vertices.len(),
                body.vertices.len() + kept,
                "{what}: put back"
            );
            assert_eq!(
                back.body.edges.len(),
                body.edges.len() + kept,
                "{what}: put back"
            );
            same_body_but(
                &back.body,
                &body,
                &tol,
                f64::INFINITY,
                &format!("{what}: put back"),
            );
            assert_empty(&made.body, &tool, Intersect, k);
        }
    }
}

/// A chain whose arcs go on in each other (a torus that goes on in another
/// torus, a contact the boolean once refused; until 2026-10-18 the tools
/// of a chain were joined into one by `blend::join` to avoid it, now the
/// boolean takes it tool by tool): the top rim of an outline of a line, a
/// convex quarter arc of radius 1, then a convex quarter of radius 2 (two
/// tori turning one way) or a concave one of radius 2 (an S: tori from
/// either side), and a line. Pappus along each piece gives what the round
/// takes away, since every end of the chain is cut square by a wall.
#[test]
fn a_chain_of_arcs_that_go_on_in_each_other() {
    use craft3d_kernel::fillet::tangent_chain;
    let r = 0.3;
    let removed_per_length = SPANDREL * r * r;
    let quarter = FRAC_PI_2;
    let outlines = [
        (
            "convex into convex",
            vec![
                line(1, [0.0, 0.0], [4.0, 0.0]),
                arc(2, [4.0, 1.0], [4.0, 0.0], [5.0, 1.0]),
                arc(3, [3.0, 1.0], [5.0, 1.0], [3.0, 3.0]),
                line(4, [3.0, 3.0], [0.0, 3.0]),
                line(5, [0.0, 3.0], [0.0, 0.0]),
            ],
            4.0 + 3.0 + quarter * (1.0 - r * INSET) + quarter * (2.0 - r * INSET),
        ),
        (
            "convex into concave",
            vec![
                line(1, [0.0, 0.0], [4.0, 0.0]),
                arc(2, [4.0, 1.0], [4.0, 0.0], [5.0, 1.0]),
                arc(3, [7.0, 1.0], [7.0, 3.0], [5.0, 1.0]),
                line(4, [7.0, 3.0], [9.0, 3.0]),
                line(5, [9.0, 3.0], [9.0, 5.0]),
                line(6, [9.0, 5.0], [0.0, 5.0]),
                line(7, [0.0, 5.0], [0.0, 0.0]),
            ],
            4.0 + 2.0 + quarter * (1.0 - r * INSET) + quarter * (2.0 + r * INSET),
        ),
    ];
    for k in SCALES {
        let tol = tolerance(k);
        for (name, curves, path) in &outlines {
            let profiles = detect_profiles(&scale_curves(curves, k), &tol).unwrap();
            let part = extrude(&profiles, &Frame::WORLD, 0.0, 2.0 * k, &tol)
                .unwrap()
                .body;
            let seed = edge_near(&part, Vec3::new(2.0, 0.0, 2.0) * k);
            let top = (0..part.faces.len())
                .map(|i| craft3d_topo::FaceId(i as u32))
                .find(|f| {
                    let face = part.face(*f).unwrap();
                    matches!(face.surface, craft3d_geom::Surface::Plane(_))
                        && face.surface.eval(0.0, 0.0).z > 1.5 * k
                })
                .unwrap();
            let chain = tangent_chain(&part, seed, top).unwrap().unwrap();
            assert_eq!(chain.len(), 4, "{name}");
            let edges: Vec<FilletEdge> = chain
                .iter()
                .map(|(edge, _)| FilletEdge {
                    edge: *edge,
                    radius: r * k,
                })
                .collect();
            let made = fillet(&part, &edges, &tol).unwrap();
            assert_eq!(check(&made.body, &tol), [], "{name}");
            let removed = mass(&part).volume - mass(&made.body).volume;
            let k3 = k * k * k;
            close_to(
                removed,
                removed_per_length * path * k3,
                1e-9 * mass(&part).volume,
                &format!("{name} at {k}"),
            );
        }
    }
}

/// A lid whose underside z = 1 + g(x) y² (g(0) = 0, g even) touches the
/// plane z = 1 along the lines x = 0 and y = 0, lofted through three
/// sections, sat on a rod of radius 1.
fn lid(k: f64, a: f64) -> Body {
    use craft3d_kernel::loft::{Options, Section, loft};
    let tol = tolerance(k);
    let sections: Vec<Section> = [-2.0, 0.0, 2.0]
        .iter()
        .map(|&x: &f64| {
            let g = if x == 0.0 { 0.0 } else { a };
            let curves = vec![
                SketchCurve {
                    id: 1,
                    curve: Curve2::Spline(Spline2 {
                        degree: 2,
                        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                        control: vec![
                            [-2.0 * k, (1.0 + 4.0 * g) * k],
                            [0.0, (1.0 - 4.0 * g) * k],
                            [2.0 * k, (1.0 + 4.0 * g) * k],
                        ],
                    }),
                },
                line(2, [2.0 * k, (1.0 + 4.0 * g) * k], [2.0 * k, 3.0 * k]),
                line(3, [2.0 * k, 3.0 * k], [-2.0 * k, 3.0 * k]),
                line(4, [-2.0 * k, 3.0 * k], [-2.0 * k, (1.0 + 4.0 * g) * k]),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            craft3d_kernel::loft::Section {
                plane: Frame::new(Vec3::new(x * k, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap(),
                outline: profiles[0].outer.clone(),
                start: None,
            }
        })
        .collect::<Vec<Section>>();
    loft(&sections, &Options::default(), &tol).unwrap().body
}

/// The rod's top circle comes within the tolerance of the lid's underside
/// at its quarters only (at 45° it is a twentieth of the size off): it
/// does not lie on it, and is not imprinted there (the review's probe of
/// 2026-10-18: five samples along it took it for lying on the surface).
/// The lid less the rod, unmerged, is the lid's six faces; the union of
/// bodies that only touch is refused as zero thickness.
#[test]
fn an_edge_that_touches_a_surface_at_places_does_not_lie_on_it() {
    use craft3d_kernel::KernelError;
    use craft3d_kernel::boolean::{BooleanOptions, boolean, boolean_with};
    for k in SCALES {
        let tol = tolerance(k);
        let lid = lid(k, 0.2);
        let cylinder = rod(k, [0.0, 0.0, 0.0], 1.0, 1.0);
        let unmerged = BooleanOptions {
            merge: false,
            ..BooleanOptions::default()
        };
        let cut = boolean_with(&lid, &cylinder, Subtract, &tol, &unmerged).unwrap();
        assert_eq!(check(&cut.body, &tol), [], "scale {k}");
        assert_eq!(cut.body.faces.len(), lid.faces.len(), "scale {k}");
        assert_eq!(cut.body.edges.len(), lid.edges.len(), "scale {k}");
        assert!(
            matches!(
                boolean(&cylinder, &lid, Union, &tol),
                Err(KernelError::ZeroThickness { .. })
            ),
            "scale {k}"
        );
    }
}
