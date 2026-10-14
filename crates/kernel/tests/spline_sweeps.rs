//! Profiles with splines in them, turned about an axis into solids with
//! rational B-spline faces; and the profiles a spline bounds on its own or
//! in pieces: one that closes on itself, one with corners.
//!
//! The oracle is independent of the kernel. A profile is described in the
//! half-plane that turns (ρ away from the axis, z along it) as a loop of
//! lines and splines; its area and moments are boundary integrals (Green's
//! theorem) with the splines evaluated by de Boor's algorithm on their
//! knots and summed by a five-point Gauss–Legendre rule on 64 stretches of
//! every knot span (the kernel uses Bézier pieces, and `mass_properties`
//! the surfaces themselves). From them, by the theorems of Pappus:
//!
//! - volume  = angle × ∬ ρ dA            (area × the way its centroid goes)
//! - surface = angle × Σ ∫ ρ ds  (+ 2 A)  (length × the way its centroid
//!   goes, plus the two caps of a partial turn)
//! - centroid: `∬ ρ z dA / ∬ ρ dA` along the axis, and
//!   `∬ ρ² dA ∫ (cos φ, sin φ) dφ / volume` across it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{frames, polygon};
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::{OpResult, Origin, Role, extrude};
use craft3d_kernel::profile::{Curve2, Profile, Shape, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::{PI, TAU};

type P = [f64; 2];

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// De Boor's algorithm: the point at knot value `u`.
fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let n = control.len();
    let k = (degree..n).rev().find(|&k| knots[k] <= u).unwrap_or(degree);
    let mut d: Vec<P> = control[k - degree..=k].to_vec();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = k - degree + j;
            let alpha = (u - knots[i]) / (knots[i + degree + 1 - r] - knots[i]);
            d[j] = [
                d[j - 1][0] + (d[j][0] - d[j - 1][0]) * alpha,
                d[j - 1][1] + (d[j][1] - d[j - 1][1]) * alpha,
            ];
        }
    }
    d[degree]
}

/// A piece of the boundary of a profile in the half-plane (ρ, z).
#[derive(Clone)]
enum Piece {
    Line(P, P),
    Spline(usize, Vec<P>),
}

/// Gauss–Legendre, five points on `[-1, 1]`.
const GAUSS: [(f64, f64); 5] = [
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.0, 0.568_888_888_888_888_9),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

/// `∫ f(point, derivative) dt` along a piece.
fn along(piece: &Piece, f: &dyn Fn(P, P) -> f64) -> f64 {
    match piece {
        Piece::Line(a, b) => {
            let d = [b[0] - a[0], b[1] - a[1]];
            GAUSS
                .iter()
                .map(|&(x, w)| {
                    let t = (x + 1.0) / 2.0;
                    f([a[0] + d[0] * t, a[1] + d[1] * t], d) * w / 2.0
                })
                .sum()
        }
        Piece::Spline(degree, control) => {
            let degree = *degree;
            let knots = knots(degree, control.len());
            // The derivative is a spline of one degree less on the inner
            // knots.
            let derivative: Vec<P> = (0..control.len() - 1)
                .map(|i| {
                    let scale = degree as f64 / (knots[i + degree + 1] - knots[i + 1]);
                    [
                        (control[i + 1][0] - control[i][0]) * scale,
                        (control[i + 1][1] - control[i][1]) * scale,
                    ]
                })
                .collect();
            let inner = &knots[1..knots.len() - 1];
            let mut sum = 0.0;
            let stretches = 64;
            for span in 0..control.len() - degree {
                let (a, b) = (knots[degree + span], knots[degree + span + 1]);
                let h = (b - a) / stretches as f64;
                for i in 0..stretches {
                    for &(x, w) in &GAUSS {
                        let u = a + h * (i as f64 + (x + 1.0) / 2.0);
                        let p = de_boor(degree, &knots, control, u);
                        let d = if degree == 1 {
                            derivative[span]
                        } else {
                            de_boor(degree - 1, inner, &derivative, u)
                        };
                        sum += f(p, d) * w * h / 2.0;
                    }
                }
            }
            sum
        }
    }
}

/// What the oracle knows of a profile in the half-plane.
#[derive(Clone, Copy, Debug)]
struct Sums {
    /// `∬ dA`.
    area: f64,
    /// `∬ ρ dA`.
    rho: f64,
    /// `∬ ρ z dA`.
    rho_z: f64,
    /// `∬ ρ² dA`.
    rho2: f64,
    /// `Σ ∫ ρ ds` over the boundary.
    rho_length: f64,
    /// `∬ z dA`.
    z: f64,
    /// `Σ ∫ ds`: the length of the boundary.
    length: f64,
}

fn sums(loops: &[Vec<Piece>]) -> Sums {
    let mut total = [0.0; 7];
    for pieces in loops {
        // A loop either way round: the sign of its area turns the rest.
        let integral = |f: &dyn Fn(P, P) -> f64| pieces.iter().map(|p| along(p, f)).sum::<f64>();
        let area = integral(&|p, d| p[0] * d[1]);
        let sign = area.signum();
        // The first loop is the outside, the others are holes.
        let part = if total[0] == 0.0 { 1.0 } else { -1.0 } * sign;
        total[0] += part * area;
        total[1] += part * integral(&|p, d| p[0] * p[0] / 2.0 * d[1]);
        total[2] += part * integral(&|p, d| p[0] * p[0] * p[1] / 2.0 * d[1]);
        total[3] += part * integral(&|p, d| p[0] * p[0] * p[0] / 3.0 * d[1]);
        total[4] += integral(&|p, d| p[0] * d[0].hypot(d[1]));
        total[5] += part * integral(&|p, d| p[0] * p[1] * d[1]);
        total[6] += integral(&|_, d| d[0].hypot(d[1]));
    }
    Sums {
        area: total[0],
        rho: total[1],
        rho_z: total[2],
        rho2: total[3],
        rho_length: total[4],
        z: total[5],
        length: total[6],
    }
}

/// Where the half-plane lies in the sketch: a point of the axis, its
/// direction, and the side the profile is on (+1 left of the axis).
#[derive(Clone, Copy)]
struct Placing {
    point: P,
    direction: P,
    side: f64,
}

impl Placing {
    fn unit(&self) -> P {
        let l = self.direction[0].hypot(self.direction[1]);
        [self.direction[0] / l, self.direction[1] / l]
    }

    fn sketch(&self, p: P) -> P {
        let d = self.unit();
        let left = [-d[1] * self.side, d[0] * self.side];
        [
            self.point[0] + d[0] * p[1] + left[0] * p[0],
            self.point[1] + d[1] * p[1] + left[1] * p[0],
        ]
    }

    /// The sketch curves of the loops, ids from 3 upwards in steps of 2.
    /// `flip` draws every other spline from its end to its start.
    fn curves(&self, loops: &[Vec<Piece>], flip: bool) -> Vec<SketchCurve> {
        let mut curves = Vec::new();
        let mut id = 3;
        for piece in loops.iter().flatten() {
            let curve = match piece {
                Piece::Line(a, b) => Curve2::Line {
                    start: self.sketch(*a),
                    end: self.sketch(*b),
                },
                Piece::Spline(degree, control) => {
                    let mut control: Vec<P> = control.iter().map(|p| self.sketch(*p)).collect();
                    if flip && id % 4 == 3 {
                        control.reverse();
                    }
                    Curve2::Spline(Spline2 {
                        degree: *degree,
                        knots: knots(*degree, control.len()),
                        control,
                    })
                }
            };
            curves.push(SketchCurve { id, curve });
            id += 2;
        }
        curves
    }
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

struct Case {
    name: &'static str,
    loops: Vec<Vec<Piece>>,
    /// Faces on spline surfaces, per spline.
    splines: usize,
    /// Splines that end on the axis.
    on_axis: bool,
}

/// The wall of a vase, from its foot at (3, 0) up to its rim at (2, 10).
const VASE: [P; 7] = [
    [3.0, 0.0],
    [5.5, 1.0],
    [6.0, 4.0],
    [1.0, 5.5],
    [1.5, 8.0],
    [3.5, 9.0],
    [2.0, 10.0],
];
/// The inside of a ring: from (4, 1) bulging towards the axis to (4, 7).
const INNER: [P; 4] = [[4.0, 1.0], [2.0, 2.5], [2.5, 6.0], [4.0, 7.0]];
/// Its outside, drawn the same way up.
const OUTER: [P; 5] = [[4.0, 1.0], [7.0, 0.0], [9.0, 4.0], [6.5, 8.0], [4.0, 7.0]];
/// A dome from the axis at (0, 0) out and back to the axis at (0, 6).
const DOME: [P; 5] = [[0.0, 0.0], [4.0, 0.0], [6.0, 3.0], [3.0, 6.0], [0.0, 6.0]];
/// A drop: from the axis at (0, 0), where it leaves at a slant, to a foot
/// at (3, 5).
const DROP: [P; 4] = [[0.0, 0.0], [2.0, 1.0], [5.0, 2.0], [3.0, 5.0]];

fn cases(k: f64) -> Vec<Case> {
    let line = |a: P, b: P| Piece::Line([a[0] * k, a[1] * k], [b[0] * k, b[1] * k]);
    let spline = |degree: usize, points: &[P]| Piece::Spline(degree, scaled(points, k));
    let mut cases = Vec::new();
    for (degree, name) in [
        (2, "vase, degree 2"),
        (3, "vase, degree 3"),
        (5, "vase, degree 5"),
    ] {
        // Solid: along the axis, out along the bottom, up the wall, back
        // along the rim.
        cases.push(Case {
            name,
            loops: vec![vec![
                line([0.0, 10.0], [0.0, 0.0]),
                line([0.0, 0.0], [3.0, 0.0]),
                spline(degree, &VASE),
                line([2.0, 10.0], [0.0, 10.0]),
            ]],
            splines: 1,
            on_axis: false,
        });
    }
    let mut back = INNER.to_vec();
    back.reverse();
    cases.push(Case {
        name: "ring of two splines",
        loops: vec![vec![spline(4, &OUTER), spline(3, &back)]],
        splines: 2,
        on_axis: false,
    });
    // A ring with a hole in its section: a hollow ring.
    cases.push(Case {
        name: "hollow ring",
        loops: vec![
            vec![spline(4, &OUTER), spline(3, &back)],
            vec![
                line([5.0, 3.0], [6.5, 3.0]),
                spline(2, &[[6.5, 3.0], [7.5, 4.0], [6.5, 5.0]]),
                line([6.5, 5.0], [5.0, 5.0]),
                line([5.0, 5.0], [5.0, 3.0]),
            ],
        ],
        splines: 3,
        on_axis: false,
    });
    cases.push(Case {
        name: "dome between two points of the axis",
        loops: vec![vec![spline(3, &DOME), line([0.0, 6.0], [0.0, 0.0])]],
        splines: 1,
        on_axis: true,
    });
    cases.push(Case {
        name: "drop from a point of the axis",
        loops: vec![vec![
            spline(3, &DROP),
            line([3.0, 5.0], [0.0, 5.0]),
            line([0.0, 5.0], [0.0, 0.0]),
        ]],
        splines: 1,
        on_axis: true,
    });
    cases
}

fn close(a: f64, b: f64, relative: f64, what: &str) {
    assert!(
        (a - b).abs() <= relative * b.abs(),
        "{what}: {a} against {b} (off by {:e})",
        ((a - b) / b).abs()
    );
}

fn placings(k: f64) -> Vec<Placing> {
    vec![
        Placing {
            point: [0.0, 0.0],
            direction: [0.0, 1.0],
            side: -1.0,
        },
        Placing {
            point: [2.0 * k, -k],
            direction: [3.0, -1.0],
            side: 1.0,
        },
    ]
}

/// The point of space at (ρ, z) of the half-plane turned through `angle`
/// about the axis, worked out from the sketch plane alone (Rodrigues).
fn turned(plane: &Frame, placing: &Placing, p: P, angle: f64) -> Vec3 {
    let d = placing.unit();
    let origin = plane.point(placing.point[0], placing.point[1], 0.0);
    let axis = plane.vector(d[0], d[1], 0.0);
    let out = plane.vector(-d[1] * placing.side, d[0] * placing.side, 0.0);
    origin + axis * p[1] + (out * angle.cos() + axis.cross(out) * angle.sin()) * p[0]
}

fn watertight(body: &Body, chord: f64, volume: f64, area: f64, name: &str) {
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{name}: {e}"));
    let welded = mesh.weld().unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert!(welded.is_closed(), "{name}: the mesh is not watertight");
    assert!(
        (welded.volume() - volume).abs() <= area * chord,
        "{name}: mesh volume {} against {volume} at chord {chord}",
        welded.volume(),
    );
    for face in 0..body.faces.len() {
        assert!(
            mesh.face_ids.contains(&(face as u32)),
            "{name}: face {face}"
        );
    }
}

const TURNS: [(f64, f64); 6] = [
    (0.0, TAU),
    (0.0, TAU / 3.0),
    (0.0, PI / 2.0),
    (-PI / 3.0, PI / 3.0),
    (0.3, 0.3 + 1.5 * PI),
    (-PI, 0.9 * PI),
];

#[test]
fn a_revolved_spline_profile_is_a_checked_solid_of_the_size_pappus_gives() {
    for k in [1e-3, 1.0, 1e3, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for (number, case) in cases(k).into_iter().enumerate() {
            // Every case and every turn at scale 1; at the other scales
            // one vase and two turns, which is what the scale can change.
            if k != 1.0 && (number == 0 || number == 2) {
                continue;
            }
            let turns = if k == 1.0 { &TURNS[..] } else { &TURNS[..3] };
            let exact = sums(&case.loops);
            for (p, placing) in placings(k).iter().enumerate() {
                for (f, frame) in frames(k).iter().enumerate() {
                    // The profile to the right of the axis in the world
                    // frame, to the left of it in the other.
                    if f != p {
                        continue;
                    }
                    for &(from, to) in turns {
                        let name = format!(
                            "{} x{k} placing {p} frame {f} from {from:.2} to {to:.2}",
                            case.name
                        );
                        let curves = placing.curves(&case.loops, p == 1);
                        let profiles = detect_profiles(&curves, &tol)
                            .unwrap_or_else(|e| panic!("{name}: {e}"));
                        assert_eq!(profiles.len(), 1, "{name}");
                        let result = revolve(
                            &profiles,
                            frame,
                            placing.point,
                            placing.direction,
                            from,
                            to,
                            &tol,
                        )
                        .unwrap_or_else(|e| panic!("{name}: {e}"));
                        let body = &result.body;
                        assert_eq!(check(body, &tol), [], "{name}");
                        let full = to - from == TAU;
                        let nurbs_faces = body
                            .faces
                            .iter()
                            .filter(|face| matches!(face.surface, Surface::Nurbs(_)))
                            .count();
                        // A full turn is made in two halves.
                        let halves = if full { 2 } else { 1 };
                        assert_eq!(nurbs_faces, halves * case.splines, "{name}");
                        let nurbs_edges = body
                            .edges
                            .iter()
                            .filter(|edge| matches!(edge.curve, Curve3::Nurbs(_)))
                            .count();
                        assert_eq!(nurbs_edges, 2 * case.splines, "{name}");

                        let angle = to - from;
                        let volume = angle * exact.rho;
                        let caps = if full { 0.0 } else { 2.0 * exact.area };
                        let area = angle * exact.rho_length + caps;
                        let mass = match mass_properties(body) {
                            Ok(mass) => mass,
                            // What a spline that ends on the axis sweeps
                            // has a border that is one point, through
                            // which its boundary passes: the exact measure
                            // declines that (see the ignored test below).
                            // The mesh still has to be the solid.
                            Err(e) if case.on_axis && e.to_string().contains("collapsed") => {
                                if k == 1.0 || p == 1 {
                                    watertight(body, 2e-3 * k, volume, area, &name);
                                }
                                continue;
                            }
                            Err(e) => panic!("{name}: {e}"),
                        };
                        close(mass.volume, volume, 1e-9, &format!("{name} volume"));
                        close(mass.area, area, 1e-9, &format!("{name} area"));
                        let (start, end) = if full { (0.0, TAU) } else { (from, to) };
                        let across = exact.rho2 / volume;
                        let centre = turned(frame, placing, [0.0, exact.rho_z / exact.rho], 0.0)
                            + (turned(frame, placing, [1.0, 0.0], 0.0)
                                - turned(frame, placing, [0.0, 0.0], 0.0))
                                * (across * (end.sin() - start.sin()))
                            + (turned(frame, placing, [1.0, 0.0], PI / 2.0)
                                - turned(frame, placing, [0.0, 0.0], 0.0))
                                * (across * (start.cos() - end.cos()));
                        let found = mass.centroid.unwrap();
                        assert!(
                            found.distance(centre) <= 1e-8 * k,
                            "{name}: centroid {found:?} against {centre:?}"
                        );
                        if k == 1.0 || p == 1 {
                            for chord in [2e-2 * k, 2e-3 * k] {
                                watertight(body, chord, volume, area, &name);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_surface_a_spline_sweeps_is_the_spline_turned_exactly() {
    let k = 1.0;
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let case = &cases(k)[1];
    let Piece::Spline(degree, control) = &case.loops[0][2] else {
        panic!("the vase's wall is a spline");
    };
    for (p, placing) in placings(k).iter().enumerate() {
        for frame in frames(k) {
            for (from, to) in TURNS {
                let curves = placing.curves(&case.loops, false);
                let profiles = detect_profiles(&curves, &tol).unwrap();
                let result = revolve(
                    &profiles,
                    &frame,
                    placing.point,
                    placing.direction,
                    from,
                    to,
                    &tol,
                )
                .unwrap();
                let face = result
                    .body
                    .faces
                    .iter()
                    .find(|face| matches!(face.surface, Surface::Nurbs(_)))
                    .unwrap();
                let Surface::Nurbs(surface) = &face.surface else {
                    panic!()
                };
                assert_eq!(surface.degrees(), (2, *degree));
                // The first face of a full turn is its first half.
                let (start, sweep) = if to - from == TAU {
                    (0.0, PI)
                } else {
                    (from, to - from)
                };
                // Weights of a quarter turn or less are no smaller than
                // cos 45°.
                let least = surface.weights().iter().copied().fold(1.0, f64::min);
                assert!(least >= (PI / 4.0).cos() - 1e-12, "weights down to {least}");
                let knots = knots(*degree, control.len());
                let axis_point = turned(&frame, placing, [0.0, 0.0], 0.0);
                let axis = turned(&frame, placing, [0.0, 1.0], 0.0) - axis_point;
                let out = turned(&frame, placing, [1.0, 0.0], start) - axis_point;
                let onward = axis.cross(out);
                let mut angles = Vec::new();
                for i in 0..=24 {
                    let v = f64::from(i) / 24.0;
                    let on_spline = de_boor(*degree, &knots, control, v.min(1.0 - 1e-15));
                    angles.clear();
                    for j in 0..=40 {
                        let u = f64::from(j) / 40.0;
                        let point = surface.eval(u, v) - axis_point;
                        let z = point.dot(axis);
                        let radial = point - axis * z;
                        assert!(
                            (z - on_spline[1]).abs() < 1e-12
                                && (radial.length() - on_spline[0]).abs() < 1e-12,
                            "placing {p} at ({u}, {v}): ({}, {z}) against {on_spline:?}",
                            radial.length()
                        );
                        let angle = radial.dot(onward).atan2(radial.dot(out)).rem_euclid(TAU);
                        angles.push(angle);
                    }
                    // From where the turn starts, onwards, to where it ends.
                    assert!(angles[0] < 1e-12 || TAU - angles[0] < 1e-12);
                    for pair in angles[1..].windows(2) {
                        assert!(pair[1] > pair[0], "{angles:?}");
                    }
                    let last = *angles.last().unwrap();
                    assert!((last - sweep).abs() < 1e-11, "{last} against {sweep}");
                }
            }
        }
    }
}

/// A spline whose control points lie on a line is that line: its
/// revolution is a cone, a cylinder or a disc in a spline's clothes.
#[test]
fn a_straight_spline_sweeps_what_the_line_sweeps() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let (r0, r1, h) = (2.0 * k, 5.0 * k, 4.0 * k);
        let between = |a: P, b: P, count: usize| -> Vec<P> {
            (0..count)
                .map(|i| {
                    // Unevenly spaced along the line.
                    let t = (i as f64 / (count - 1) as f64).powf(1.7);
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                })
                .collect()
        };
        // A frustum: slanted side as a cubic; a tube: both sides as
        // splines parallel to the axis.
        let frustum = vec![vec![
            Piece::Line([0.0, h], [0.0, 0.0]),
            Piece::Line([0.0, 0.0], [r1, 0.0]),
            Piece::Spline(3, between([r1, 0.0], [r0, h], 6)),
            Piece::Line([r0, h], [0.0, h]),
        ]];
        let tube = vec![vec![
            Piece::Line([r0, 0.0], [r1, 0.0]),
            Piece::Spline(2, between([r1, 0.0], [r1, h], 5)),
            Piece::Line([r1, h], [r0, h]),
            Piece::Spline(4, between([r0, h], [r0, 0.0], 7)),
        ]];
        let slant = (r1 - r0).hypot(h);
        let shapes = [
            (
                "frustum",
                &frustum,
                h * (r0 * r0 + r0 * r1 + r1 * r1) / 6.0,
                (r0 + r1) * slant / 2.0 + (r0 * r0 + r1 * r1) / 2.0,
                (r0 + r1) * h / 2.0,
            ),
            (
                "tube",
                &tube,
                h * (r1 * r1 - r0 * r0) / 2.0,
                (r0 + r1) * h + (r1 * r1 - r0 * r0),
                (r1 - r0) * h,
            ),
        ];
        for (name, loops, volume_per_radian, area_per_radian, section) in shapes {
            for placing in placings(k) {
                for (from, to) in TURNS {
                    let name = format!("{name} x{k} from {from:.2} to {to:.2}");
                    let profiles = detect_profiles(&placing.curves(loops, false), &tol).unwrap();
                    let frame = &frames(k)[1];
                    let result = revolve(
                        &profiles,
                        frame,
                        placing.point,
                        placing.direction,
                        from,
                        to,
                        &tol,
                    )
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert_eq!(check(&result.body, &tol), [], "{name}");
                    let angle = to - from;
                    let caps = if angle == TAU { 0.0 } else { 2.0 * section };
                    let mass = mass_properties(&result.body).unwrap();
                    close(mass.volume, angle * volume_per_radian, 1e-10, &name);
                    close(mass.area, angle * area_per_radian + caps, 1e-10, &name);
                }
            }
        }
    }
}

#[test]
fn the_faces_of_a_revolved_spline_are_named_after_it_and_point_outwards() {
    let tol = Tolerance::default();
    let case = &cases(1.0)[1];
    for placing in placings(1.0) {
        let frame = &frames(1.0)[1];
        for flip in [false, true] {
            let curves = placing.curves(&case.loops, flip);
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let origin = |role: Role, sources: &[u64]| Origin::new(role, sources);
            for (from, to) in [(0.0, TAU), (0.0, 2.0)] {
                let result = revolve(
                    &profiles,
                    frame,
                    placing.point,
                    placing.direction,
                    from,
                    to,
                    &tol,
                )
                .unwrap();
                let full = to == TAU;
                let count =
                    |list: &[Origin], wanted: &Origin| list.iter().filter(|o| *o == wanted).count();
                let origins = &result.origins;
                // Every origin is unique.
                for list in [&origins.faces, &origins.edges, &origins.vertices] {
                    for o in list.iter() {
                        assert_eq!(count(list, o), 1, "{o:?}");
                    }
                }
                // The spline is curve 7: an edge where the turn starts and
                // one where it ends (half a turn on, for a full turn), and
                // one face, or the two halves of a full turn. The circles
                // at its ends (with the bottom, 5, and the rim, 9) are in
                // two arcs then.
                assert_eq!(count(&origins.faces, &origin(Role::Side, &[7])), 1);
                assert_eq!(count(&origins.edges, &origin(Role::StartEdge, &[7])), 1);
                assert_eq!(count(&origins.edges, &origin(Role::EndEdge, &[7])), 1);
                let second = |role: Role, sources: &[u64]| Origin::of_piece(role, sources, 1);
                assert_eq!(
                    count(&origins.faces, &second(Role::Side, &[7])),
                    usize::from(full)
                );
                // (Which way round the loop runs depends on the side of
                // the axis the profile is on.)
                for (a, b) in [(5, 7), (7, 9)] {
                    let either = count(&origins.edges, &second(Role::LateralEdge, &[a, b]))
                        + count(&origins.edges, &second(Role::LateralEdge, &[b, a]));
                    assert_eq!(either, usize::from(full), "{a} {b}");
                }
                let pieces = |list: &[Origin]| list.iter().filter(|o| o.piece > 0).count();
                assert_eq!(pieces(&origins.faces), usize::from(full));
                assert_eq!(pieces(&origins.edges), 2 * usize::from(full));
                assert_eq!(pieces(&origins.vertices), 0);
                for index in 0..result.body.faces.len() {
                    if origins.faces[index].sources != [7] {
                        continue;
                    }
                    let face = &result.body.faces[index];
                    // Away from the axis is outside, all along the wall of
                    // the vase where it is not too steep to tell.
                    let ((u0, u1), (v0, v1)) = face.surface.domain();
                    let axis_point = turned(frame, &placing, [0.0, 0.0], 0.0);
                    let axis = turned(frame, &placing, [0.0, 1.0], 0.0) - axis_point;
                    for (s, t) in [(0.5, 0.1), (0.25, 0.5), (0.9, 0.95)] {
                        let (u, v) = (u0 + (u1 - u0) * s, v0 + (v1 - v0) * t);
                        let mut normal = face.surface.normal(u, v).unwrap();
                        if face.reversed {
                            normal = -normal;
                        }
                        let point = face.surface.eval(u, v) - axis_point;
                        let radial = point - axis * point.dot(axis);
                        assert!(normal.dot(radial) > 0.0, "flip {flip} at ({s}, {t})");
                    }
                }
            }
        }
    }
}

#[test]
fn a_spline_that_reaches_the_axis_between_its_ends_is_refused_by_name() {
    let tol = Tolerance::default();
    let placing = placings(1.0)[0];
    // Touching the axis in the middle, from one side.
    let touching = vec![vec![
        Piece::Spline(2, vec![[3.0, 0.0], [-3.0, 4.0], [3.0, 8.0]]),
        Piece::Line([3.0, 8.0], [6.0, 8.0]),
        Piece::Line([6.0, 8.0], [6.0, 0.0]),
        Piece::Line([6.0, 0.0], [3.0, 0.0]),
    ]];
    let profiles = detect_profiles(&placing.curves(&touching, false), &tol).unwrap();
    let error = revolve(
        &profiles,
        &Frame::WORLD,
        placing.point,
        placing.direction,
        0.0,
        TAU,
        &tol,
    )
    .unwrap_err();
    assert!(
        matches!(error, KernelError::UnsupportedCurve { id: 3, reason } if reason.contains("pinch")),
        "{error}"
    );
    // Across the axis: named as crossing it.
    let across = vec![vec![
        Piece::Spline(2, vec![[3.0, 0.0], [-5.0, 4.0], [3.0, 8.0]]),
        Piece::Line([3.0, 8.0], [6.0, 8.0]),
        Piece::Line([6.0, 8.0], [6.0, 0.0]),
        Piece::Line([6.0, 0.0], [3.0, 0.0]),
    ]];
    let profiles = detect_profiles(&placing.curves(&across, false), &tol).unwrap();
    let error = revolve(
        &profiles,
        &Frame::WORLD,
        placing.point,
        placing.direction,
        0.0,
        1.0,
        &tol,
    )
    .unwrap_err();
    assert_eq!(error, KernelError::CrossesAxis { curves: vec![3] });
    // A control point across the axis is fine when the curve is not.
    let clear = vec![vec![
        Piece::Spline(2, vec![[3.0, 0.0], [-1.0, 4.0], [3.0, 8.0]]),
        Piece::Line([3.0, 8.0], [6.0, 8.0]),
        Piece::Line([6.0, 8.0], [6.0, 0.0]),
        Piece::Line([6.0, 0.0], [3.0, 0.0]),
    ]];
    let profiles = detect_profiles(&placing.curves(&clear, false), &tol).unwrap();
    let result = revolve(
        &profiles,
        &Frame::WORLD,
        placing.point,
        placing.direction,
        0.0,
        TAU,
        &tol,
    )
    .unwrap();
    let exact = sums(&clear);
    close(
        mass_properties(&result.body).unwrap().volume,
        TAU * exact.rho,
        1e-9,
        "a control point across the axis",
    );
}

/// Work owed by `craft3d_topo::mass_properties`: the face a spline sweeps
/// when it ends on the axis is bounded by edges that meet in the point its
/// surface's border collapses into, and the measure declines a boundary
/// through such a point. The solid is valid, meshes watertight and has the
/// volume Pappus gives (the test above); only the exact measure is
/// missing. Remove `#[ignore]` when it is there.
#[test]
#[ignore = "mass_properties declines a boundary through a collapsed border of a spline surface"]
fn the_measure_takes_a_revolved_spline_that_ends_on_the_axis() {
    let tol = Tolerance::default();
    let case = cases(1.0).into_iter().find(|c| c.on_axis).unwrap();
    let placing = placings(1.0)[0];
    let profiles = detect_profiles(&placing.curves(&case.loops, false), &tol).unwrap();
    let exact = sums(&case.loops);
    for (from, to) in [(0.0, PI / 2.0), (0.0, TAU)] {
        let result = revolve(
            &profiles,
            &Frame::WORLD,
            placing.point,
            placing.direction,
            from,
            to,
            &tol,
        )
        .unwrap();
        assert_eq!(check(&result.body, &tol), []);
        let mass = mass_properties(&result.body).unwrap();
        close(mass.volume, (to - from) * exact.rho, 1e-9, "volume");
    }
}

/// A ring of control points clear of the axis, back on its first: a
/// spline on it closes on itself, with a corner where it does.
const EGG: [P; 7] = [
    [4.0, 2.0],
    [8.0, 1.0],
    [10.0, 5.0],
    [7.0, 9.0],
    [3.0, 8.0],
    [2.0, 4.0],
    [4.0, 2.0],
];

fn origins_are_unique(result: &OpResult, name: &str) {
    let origins = &result.origins;
    assert_eq!(origins.faces.len(), result.body.faces.len(), "{name}");
    assert_eq!(origins.edges.len(), result.body.edges.len(), "{name}");
    assert_eq!(origins.vertices.len(), result.body.vertices.len(), "{name}");
    for list in [&origins.faces, &origins.edges, &origins.vertices] {
        let mut sorted = list.clone();
        sorted.sort();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1], "{name}");
        }
    }
}

#[test]
fn a_spline_that_closes_on_itself_is_a_profile_of_two_pieces() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for degree in [2, 3, 5] {
            let loops = vec![vec![Piece::Spline(degree, scaled(&EGG, k))]];
            let exact = sums(&loops);
            for (p, placing) in placings(k).iter().enumerate() {
                let name = format!("egg of degree {degree} x{k} placing {p}");
                let curves = placing.curves(&loops, p == 1);
                let profiles =
                    detect_profiles(&curves, &tol).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(profiles.len(), 1, "{name}");
                let segments = &profiles[0].outer.segments;
                assert_eq!(segments.len(), 2, "{name}");
                assert!(segments.iter().all(|s| s.source == 3), "{name}");
                let mut pieces: Vec<u32> = segments.iter().map(|s| s.piece).collect();
                pieces.sort_unstable();
                assert_eq!(pieces, [0, 1], "{name}");
                close(profiles[0].area(), exact.area, 1e-11, &name);

                // Extruded: two walls, two caps.
                let frame = &frames(k)[p];
                let (from, to) = (-k, 2.5 * k);
                let result = extrude(&profiles, frame, from, to, &tol)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                let body = &result.body;
                assert_eq!(check(body, &tol), [], "{name}");
                assert_eq!(body.faces.len(), 4, "{name}");
                origins_are_unique(&result, &name);
                for piece in 0..2 {
                    for (list, role, sources) in [
                        (&result.origins.faces, Role::Side, &[3_u64][..]),
                        (&result.origins.edges, Role::StartEdge, &[3][..]),
                        (&result.origins.edges, Role::EndEdge, &[3][..]),
                        (&result.origins.edges, Role::LateralEdge, &[3, 3][..]),
                        (&result.origins.vertices, Role::StartVertex, &[3, 3][..]),
                        (&result.origins.vertices, Role::EndVertex, &[3, 3][..]),
                    ] {
                        let wanted = Origin::of_piece(role, sources, piece);
                        assert!(list.contains(&wanted), "{name}: {wanted:?}");
                    }
                }
                assert!(
                    result
                        .origins
                        .faces
                        .contains(&Origin::new(Role::StartCap, &[3]))
                );
                let depth = to - from;
                let mass = mass_properties(body).unwrap_or_else(|e| panic!("{name}: {e}"));
                let (volume, area) = (exact.area * depth, 2.0 * exact.area + exact.length * depth);
                close(mass.volume, volume, 1e-9, &format!("{name} volume"));
                close(mass.area, area, 1e-9, &format!("{name} area"));
                let middle = placing.sketch([exact.rho / exact.area, exact.z / exact.area]);
                let centre = frame.point(middle[0], middle[1], (from + to) / 2.0);
                assert!(
                    mass.centroid.unwrap().distance(centre) <= 1e-8 * k,
                    "{name}"
                );
                for chord in [2e-2 * k, 2e-3 * k] {
                    watertight(body, chord, volume, area, &name);
                }

                // Revolved: part of a turn, and a whole one in two halves
                // (four faces: two pieces, each in two halves).
                for (from, to) in [(0.0, TAU), (-PI / 3.0, PI / 3.0), (0.2, 0.2 + PI / 2.0)] {
                    let name = format!("{name} turned from {from:.2} to {to:.2}");
                    let result = revolve(
                        &profiles,
                        frame,
                        placing.point,
                        placing.direction,
                        from,
                        to,
                        &tol,
                    )
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                    let body = &result.body;
                    assert_eq!(check(body, &tol), [], "{name}");
                    origins_are_unique(&result, &name);
                    let full = to - from == TAU;
                    assert_eq!(body.faces.len(), 4, "{name}");
                    let sides = result
                        .origins
                        .faces
                        .iter()
                        .filter(|o| o.role == Role::Side)
                        .map(|o| o.piece)
                        .collect::<Vec<_>>();
                    assert_eq!(sides.len(), if full { 4 } else { 2 }, "{name}");
                    assert_eq!(
                        *sides.iter().max().unwrap(),
                        if full { 3 } else { 1 },
                        "{name}"
                    );
                    let angle = to - from;
                    let caps = if full { 0.0 } else { 2.0 * exact.area };
                    let (volume, area) = (angle * exact.rho, angle * exact.rho_length + caps);
                    let mass = mass_properties(body).unwrap_or_else(|e| panic!("{name}: {e}"));
                    close(mass.volume, volume, 1e-9, &format!("{name} volume"));
                    close(mass.area, area, 1e-9, &format!("{name} area"));
                    if k == 1.0 {
                        watertight(body, 5e-3 * k, volume, area, &name);
                    }
                }
            }
        }
    }
}

#[test]
fn a_closed_spline_is_a_hole_and_an_island() {
    let tol = Tolerance::default();
    let placing = placings(1.0)[0];
    // A plate around the egg, and a small closed spline inside the egg.
    let seed: Vec<P> = EGG
        .iter()
        .map(|p| [5.5 + (p[0] - 6.0) * 0.2, 5.0 + (p[1] - 5.0) * 0.2])
        .collect();
    let plate = [[1.0, 0.0], [12.0, 0.0], [12.0, 10.0], [1.0, 10.0]];
    let mut curves = polygon(20, 1, &plate.map(|p| placing.sketch(p)));
    let egg = vec![vec![Piece::Spline(3, EGG.to_vec())]];
    let island = vec![vec![Piece::Spline(4, seed)]];
    for (id, loops) in [(40, &egg), (50, &island)] {
        let mut curve = placing.curves(loops, false);
        curve[0].id = id;
        curves.extend(curve);
    }
    let profiles = detect_profiles(&curves, &tol).unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].holes.len(), 1);
    assert_eq!(profiles[0].holes[0].segments.len(), 2);
    let (hole, seed) = (sums(&egg), sums(&island));
    close(profiles[0].area(), 110.0 - hole.area, 1e-11, "plate");
    close(profiles[1].area(), seed.area, 1e-11, "island");
    let result = extrude(&profiles, &frames(1.0)[1], 0.0, 2.0, &tol).unwrap();
    assert_eq!(check(&result.body, &tol), []);
    origins_are_unique(&result, "plate with an egg-shaped hole");
    let mass = mass_properties(&result.body).unwrap();
    let section = 110.0 - hole.area + seed.area;
    close(mass.volume, 2.0 * section, 1e-9, "volume");
    close(
        mass.area,
        2.0 * section + 2.0 * (42.0 + hole.length + seed.length),
        1e-9,
        "area",
    );
    watertight(&result.body, 2e-3, 2.0 * section, mass.area, "plate");
}

/// A spline of degree 1 is a polyline: its profile is the polygon, its
/// sweep the same solid the polygon's lines sweep, face for face.
#[test]
fn a_polyline_is_cut_at_its_corners_into_lines() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let placing = placings(k)[1];
        let corners: Vec<P> = scaled(
            &[
                [2.0, 1.0],
                [7.0, 0.0],
                [9.0, 4.0],
                [6.0, 5.0],
                [5.0, 8.0],
                [3.0, 6.0],
            ],
            k,
        )
        .into_iter()
        .map(|p| placing.sketch(p))
        .collect();
        let lines = polygon(30, 1, &corners);
        let polyline = |control: Vec<P>| SketchCurve {
            id: 7,
            curve: Curve2::Spline(Spline2 {
                degree: 1,
                knots: knots(1, control.len()),
                control,
            }),
        };
        // Closed: back on its first point.
        let mut ring = corners.clone();
        ring.push(corners[0]);
        let closed = vec![polyline(ring)];
        // Open, with a line that closes it.
        let open = vec![
            polyline(corners.clone()),
            SketchCurve {
                id: 9,
                curve: Curve2::Line {
                    start: corners[5],
                    end: corners[0],
                },
            },
        ];
        let reference = detect_profiles(&lines, &tol).unwrap();
        let frame = &frames(k)[1];
        for (name, curves, pieces) in [("closed", &closed, 6), ("open", &open, 5)] {
            let name = format!("{name} polyline x{k}");
            let profiles = detect_profiles(curves, &tol).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(profiles.len(), 1, "{name}");
            let segments = &profiles[0].outer.segments;
            assert_eq!(segments.len(), 6, "{name}");
            assert!(
                segments.iter().all(|s| matches!(s.shape, Shape::Line)),
                "{name}"
            );
            let mut numbers: Vec<u32> = segments
                .iter()
                .filter(|s| s.source == 7)
                .map(|s| s.piece)
                .collect();
            numbers.sort_unstable();
            assert_eq!(numbers, (0..pieces).collect::<Vec<u32>>(), "{name}");
            close(profiles[0].area(), reference[0].area(), 1e-12, &name);
            for turn in [None, Some((0.0, TAU)), Some((0.0, 2.0))] {
                let build = |profiles: &[Profile]| match turn {
                    None => extrude(profiles, frame, 0.0, 3.0 * k, &tol),
                    Some((from, to)) => revolve(
                        profiles,
                        frame,
                        placing.point,
                        placing.direction,
                        from,
                        to,
                        &tol,
                    ),
                };
                let (ours, theirs) = (
                    build(&profiles).unwrap_or_else(|e| panic!("{name}: {e}")),
                    build(&reference).unwrap(),
                );
                assert_eq!(check(&ours.body, &tol), [], "{name}");
                origins_are_unique(&ours, &name);
                assert_eq!(ours.body.faces.len(), theirs.body.faces.len(), "{name}");
                assert_eq!(ours.body.edges.len(), theirs.body.edges.len(), "{name}");
                assert!(
                    !ours
                        .body
                        .faces
                        .iter()
                        .any(|f| matches!(f.surface, Surface::Nurbs(_))),
                    "{name}"
                );
                let (a, b) = (
                    mass_properties(&ours.body).unwrap(),
                    mass_properties(&theirs.body).unwrap(),
                );
                close(a.volume, b.volume, 1e-12, &format!("{name} volume"));
                close(a.area, b.area, 1e-12, &format!("{name} area"));
            }
        }
    }
}

/// A cubic with an inner knot three times has a corner there: it is two
/// cubic Bézier curves end to end, which is what the oracle integrates.
#[test]
fn a_spline_with_a_corner_sweeps_a_face_per_smooth_piece() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let first = scaled(&[[3.0, 0.0], [6.0, 1.0], [7.0, 3.0], [5.0, 5.0]], k);
        let second = scaled(&[[5.0, 5.0], [8.0, 6.0], [6.0, 9.0], [2.0, 10.0]], k);
        let loops = vec![vec![
            Piece::Line([0.0, 10.0 * k], [0.0, 0.0]),
            Piece::Line([0.0, 0.0], [3.0 * k, 0.0]),
            Piece::Spline(3, first.clone()),
            Piece::Spline(3, second.clone()),
            Piece::Line([2.0 * k, 10.0 * k], [0.0, 10.0 * k]),
        ]];
        let exact = sums(&loops);
        for (p, placing) in placings(k).iter().enumerate() {
            let name = format!("cornered cubic x{k} placing {p}");
            let mut control: Vec<P> = first.iter().map(|q| placing.sketch(*q)).collect();
            control.extend(second.iter().skip(1).map(|q| placing.sketch(*q)));
            let line = |id: u64, a: P, b: P| SketchCurve {
                id,
                curve: Curve2::Line {
                    start: placing.sketch([a[0] * k, a[1] * k]),
                    end: placing.sketch([b[0] * k, b[1] * k]),
                },
            };
            let curves = vec![
                line(3, [0.0, 10.0], [0.0, 0.0]),
                line(5, [0.0, 0.0], [3.0, 0.0]),
                SketchCurve {
                    id: 7,
                    curve: Curve2::Spline(Spline2 {
                        degree: 3,
                        knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
                        control,
                    }),
                },
                line(9, [2.0, 10.0], [0.0, 10.0]),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(profiles[0].outer.segments.len(), 5, "{name}");
            close(profiles[0].area(), exact.area, 1e-11, &name);
            let frame = &frames(k)[p];
            let result = extrude(&profiles, frame, 0.0, 2.0 * k, &tol).unwrap();
            assert_eq!(check(&result.body, &tol), [], "{name}");
            origins_are_unique(&result, &name);
            for piece in 0..2 {
                assert!(
                    result
                        .origins
                        .faces
                        .contains(&Origin::of_piece(Role::Side, &[7], piece)),
                    "{name}"
                );
            }
            // The edge along the corner lies between the two pieces.
            assert!(
                result
                    .origins
                    .edges
                    .iter()
                    .any(|o| o.role == Role::LateralEdge && o.sources == [7, 7]),
                "{name}"
            );
            let mass = mass_properties(&result.body).unwrap();
            close(
                mass.volume,
                exact.area * 2.0 * k,
                1e-9,
                &format!("{name} volume"),
            );
            close(
                mass.area,
                2.0 * exact.area + exact.length * 2.0 * k,
                1e-9,
                &format!("{name} area"),
            );
            for (from, to) in [(0.0, TAU), (0.0, TAU / 3.0)] {
                let name = format!("{name} turned through {to:.2}");
                let result = revolve(
                    &profiles,
                    frame,
                    placing.point,
                    placing.direction,
                    from,
                    to,
                    &tol,
                )
                .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(check(&result.body, &tol), [], "{name}");
                origins_are_unique(&result, &name);
                let full = to == TAU;
                let caps = if full { 0.0 } else { 2.0 * exact.area };
                let (volume, area) = (to * exact.rho, to * exact.rho_length + caps);
                let mass = mass_properties(&result.body).unwrap();
                close(mass.volume, volume, 1e-9, &format!("{name} volume"));
                close(mass.area, area, 1e-9, &format!("{name} area"));
                if k == 1.0 {
                    watertight(&result.body, 5e-3, volume, area, &name);
                }
            }
        }
    }
}

#[test]
fn a_closed_spline_that_cannot_be_a_profile_says_why() {
    let tol = Tolerance::default();
    let closed = |degree: usize, points: &[P]| {
        vec![SketchCurve {
            id: 7,
            curve: Curve2::Spline(Spline2 {
                degree,
                knots: knots(degree, points.len()),
                control: points.to_vec(),
            }),
        }]
    };
    // A figure of eight (with one eye larger than the other) crosses
    // itself.
    let eight = [
        [0.0, 0.0],
        [4.0, 4.0],
        [9.0, -1.0],
        [9.0, 5.0],
        [4.0, 0.0],
        [0.0, 4.0],
        [0.0, 0.0],
    ];
    assert_eq!(
        detect_profiles(&closed(1, &eight), &tol),
        Err(KernelError::CrossingCurves { curves: [7, 7] })
    );
    assert_eq!(
        detect_profiles(&closed(3, &eight), &tol),
        Err(KernelError::CrossingCurves { curves: [7, 7] })
    );
    // Out and back along itself encloses nothing.
    let flat = [[0.0, 0.0], [4.0, 0.0], [0.0, 0.0]];
    assert!(detect_profiles(&closed(1, &flat), &tol).is_err());
    assert!(detect_profiles(&closed(2, &flat), &tol).is_err());
    // Two control points in a row on one spot.
    let stutter = [[0.0, 0.0], [4.0, 0.0], [4.0, 0.0], [2.0, 3.0], [0.0, 0.0]];
    assert!(matches!(
        detect_profiles(&closed(1, &stutter), &tol),
        Err(KernelError::DegenerateCurve { id: 7, .. })
    ));
    // Turned about an axis it crosses.
    let profiles = detect_profiles(&closed(3, &EGG), &tol).unwrap();
    let error = revolve(
        &profiles,
        &Frame::WORLD,
        [6.0, 0.0],
        [0.0, 1.0],
        0.0,
        1.0,
        &tol,
    )
    .unwrap_err();
    assert_eq!(error, KernelError::CrossesAxis { curves: vec![7] });
}

/// Work owed by `craft3d_kernel::boolean`, the same face as above: a block
/// through the dome a spline sweeps when it ends on the axis, cutting the
/// face whose surface closes into a point there. Remove `#[ignore]` when
/// booleans take it; `feature.revolve` can then stop refusing a spline
/// that ends on the axis (`crates/regen/src/revolve.rs`).
#[test]
#[ignore = "booleans decline a freeform face with a collapsed border"]
fn booleans_take_a_revolved_spline_that_ends_on_the_axis() {
    use craft3d_kernel::boolean::{BooleanOp, boolean};
    let tol = Tolerance::default();
    let case = cases(1.0).into_iter().find(|c| c.on_axis).unwrap();
    let placing = placings(1.0)[0];
    let profiles = detect_profiles(&placing.curves(&case.loops, false), &tol).unwrap();
    let dome = revolve(
        &profiles,
        &Frame::WORLD,
        placing.point,
        placing.direction,
        0.0,
        TAU,
        &tol,
    )
    .unwrap()
    .body;
    // The dome turns about the world's y axis between y = 0 and y = 6; the
    // block passes through its side, clear of both poles.
    let corner = Frame::WORLD.at(Vec3::new(2.0, 1.5, -1.0)).unwrap();
    let block = craft3d_kernel::primitives::block(&corner, 6.0, 2.0, 2.0).unwrap();
    for op in [BooleanOp::Subtract, BooleanOp::Intersect, BooleanOp::Union] {
        let result = boolean(&dome, &block, op, &tol).unwrap_or_else(|e| panic!("{op:?}: {e}"));
        assert_eq!(check(&result.body, &tol), [], "{op:?}");
    }
}
