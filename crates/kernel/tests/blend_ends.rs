//! Where a freeform blend ends (kernel::blend's `rolling`): a convex
//! fillet that runs into a wall (the impeller's finding F2), and a
//! concave root fillet that runs out at the end of a wall standing on a
//! plate (F1), whose border on the plate goes on past the corner, so the
//! plate's edge there is lengthened and the end face's spline surface
//! grown to take it. Held against the definition of a rolling-ball
//! fillet, at three scales, in a tilted frame.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::{SCALES, tolerance};
use craft3d_kernel::boolean::{BooleanOp, boolean, contains_points};
use craft3d_kernel::fillet::{FilletEdge, FilletFace, FilletResult, fillet};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, check};

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

/// A cubic Bézier in the plane, continued past its ends as the cubic it
/// is (as the kernel continues a spline face past its edge).
#[derive(Clone, Copy)]
struct Bez([[f64; 2]; 4]);

impl Bez {
    fn eval(&self, t: f64) -> [f64; 2] {
        let p = self.0;
        let s = 1.0 - t;
        let b = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        [
            (0..4).map(|i| b[i] * p[i][0]).sum(),
            (0..4).map(|i| b[i] * p[i][1]).sum(),
        ]
    }

    fn d(&self, t: f64) -> [f64; 2] {
        let p = self.0;
        let s = 1.0 - t;
        let c = [3.0 * s * s, 6.0 * s * t, 3.0 * t * t];
        let q = |i: usize, k: usize| p[i + 1][k] - p[i][k];
        [
            c[0] * q(0, 0) + c[1] * q(1, 0) + c[2] * q(2, 0),
            c[0] * q(0, 1) + c[1] * q(1, 1) + c[2] * q(2, 1),
        ]
    }

    fn dd(&self, t: f64) -> [f64; 2] {
        let p = self.0;
        let s = 1.0 - t;
        let c = |k: usize| {
            6.0 * (s * (p[2][k] - 2.0 * p[1][k] + p[0][k])
                + t * (p[3][k] - 2.0 * p[2][k] + p[1][k]))
        };
        [c(0), c(1)]
    }

    /// The signed distance of `q` from the curve (continued over
    /// [-0.4, 1.4]), positive to the right of its direction of travel.
    fn signed(&self, q: [f64; 2]) -> f64 {
        let mut t = (0..=180)
            .map(|i| -0.4 + 1.8 * f64::from(i) / 180.0)
            .min_by(|a, b| {
                let (pa, pb) = (self.eval(*a), self.eval(*b));
                let da = (pa[0] - q[0]).hypot(pa[1] - q[1]);
                let db = (pb[0] - q[0]).hypot(pb[1] - q[1]);
                da.total_cmp(&db)
            })
            .unwrap();
        for _ in 0..40 {
            let (c, d, e) = (self.eval(t), self.d(t), self.dd(t));
            let r = [c[0] - q[0], c[1] - q[1]];
            let f = r[0] * d[0] + r[1] * d[1];
            let df = d[0] * d[0] + d[1] * d[1] + r[0] * e[0] + r[1] * e[1];
            t -= f / df;
        }
        let (c, d) = (self.eval(t), self.d(t));
        let len = d[0].hypot(d[1]);
        (q[0] - c[0]) * d[1] / len - (q[1] - c[1]) * d[0] / len
    }

    fn scaled(&self, k: f64) -> Bez {
        Bez(self.0.map(|p| [p[0] * k, p[1] * k]))
    }

    fn sketch(&self, id: u64, k: f64) -> SketchCurve {
        SketchCurve {
            id,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: self.scaled(k).0.to_vec(),
            }),
        }
    }
}

/// The top view of a wall: an arch over (-1, 0) to (1, 0) with the
/// material below it, a side leaving its right end at `phi` (radians) to
/// it, bowed by `bow`, and lines closing it.
struct Outline {
    arch: Bez,
    side: Bez,
}

impl Outline {
    fn new(phi: f64, bow: f64) -> Outline {
        let arch = Bez([[-1.0, 0.0], [-1.0 / 3.0, 0.5], [1.0 / 3.0, 0.5], [1.0, 0.0]]);
        let back = [-0.8_f64, 0.6_f64];
        let (s, c) = phi.sin_cos();
        let dir = [back[0] * c - back[1] * s, back[0] * s + back[1] * c];
        // Bowed to the left of its direction (out of the material).
        let n = [-dir[1], dir[0]];
        let at = |f: f64, b: f64| {
            [
                1.0 + 1.2 * f * dir[0] + b * n[0],
                1.2 * f * dir[1] + b * n[1],
            ]
        };
        let side = Bez([
            at(0.0, 0.0),
            at(1.0 / 3.0, bow),
            at(2.0 / 3.0, bow),
            at(1.0, 0.0),
        ]);
        Outline { arch, side }
    }

    fn corner(&self) -> [f64; 2] {
        self.side.0[3]
    }

    /// The prism of the outline from 0 to `h`, in `frame`.
    fn prism(&self, frame: &Frame, h: f64, k: f64) -> Body {
        use craft3d_kernel::extrude::extrude;
        let tol = tolerance(k);
        let q = self.corner();
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: [a[0] * k, a[1] * k],
                end: [b[0] * k, b[1] * k],
            },
        };
        let curves = vec![
            self.arch.sketch(1, k),
            self.side.sketch(2, k),
            line(3, q, [-1.0, q[1]]),
            line(4, [-1.0, q[1]], [-1.0, 0.0]),
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        extrude(&profiles, frame, 0.0, h * k, &tol).unwrap().body
    }
}

fn frame(k: f64) -> Frame {
    Frame::new(
        Vec3::new(0.3, -0.2, 0.5) * k,
        Vec3::new(0.2, 0.3, 1.0),
        Vec3::new(1.0, 0.0, -0.2),
    )
    .unwrap()
}

/// A block in `frame` from `lo` to `hi`, times `k`.
fn block(frame: &Frame, lo: [f64; 3], hi: [f64; 3], k: f64) -> Body {
    let at = frame
        .at(frame.point(lo[0] * k, lo[1] * k, lo[2] * k))
        .unwrap();
    craft3d_kernel::primitives::block(
        &at,
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

/// The checks every result gets: valid, not through itself, and the
/// points of `grid` (in the frame's units over `k`) inside exactly where
/// `want` says; every mesh point of the fillet within the tolerance of
/// where `on_fillet` is zero. Returns how many points changed sides.
#[allow(clippy::too_many_arguments)]
fn held(
    made: &FilletResult,
    before: &Body,
    k: f64,
    tol: &Tolerance,
    least: usize,
    grid: &[[f64; 3]],
    want: &dyn Fn([f64; 3]) -> Option<bool>,
    on_fillet: &dyn Fn([f64; 3]) -> Option<f64>,
    what: &str,
) {
    let tol = *tol;
    let frame = frame(k);
    assert_eq!(check(&made.body, &tol), [], "{what}");
    let deep = body_self_intersections(&made.body, &SelfCheckParams::new(tol.linear)).unwrap();
    assert_eq!(deep.certain_crossings, 0, "{what}: {deep:?}");
    let (mut points, mut expected) = (Vec::new(), Vec::new());
    for p in grid {
        if let Some(w) = want(*p) {
            points.push(frame.point(p[0] * k, p[1] * k, p[2] * k));
            expected.push(w);
        }
    }
    let found = contains_points(&made.body, &points, &tol).unwrap();
    let was = contains_points(before, &points, &tol).unwrap();
    let (mut undecided, mut changed) = (0, 0);
    for (((p, w), got), old) in points.iter().zip(&expected).zip(&found).zip(&was) {
        match got {
            Some(got) => assert_eq!(got, w, "{what}: at {p:?}"),
            None => undecided += 1,
        }
        if *old != Some(*w) {
            changed += 1;
        }
    }
    assert!(
        undecided * 100 < points.len(),
        "{what}: {undecided} undecided"
    );
    assert!(
        changed > least,
        "{what}: only {changed} points changed sides"
    );
    let params = TessParams {
        chordal_tolerance: 1e-3 * k,
        ..TessParams::default()
    };
    let mesh = tessellate_body(&made.body, &params).unwrap();
    let (mut worst, mut seen) = (0.0_f64, 0);
    for (t, face) in mesh.face_ids.iter().enumerate() {
        if !matches!(made.faces[*face as usize], FilletFace::Fillet(_)) {
            continue;
        }
        for p in mesh.corners(t).unwrap() {
            let (x, y, z) = frame.local(p);
            if let Some(g) = on_fillet([x / k, y / k, z / k]) {
                worst = worst.max(g.abs());
                seen += 1;
            }
        }
    }
    println!(
        "{what}: {} points, {changed} changed, {undecided} undecided; {seen} mesh points off the definition by {:e} of the tolerance",
        points.len(),
        worst * k / tol.linear
    );
    assert!(seen > 50, "{what}: {seen} mesh points");
    assert!(worst * k <= tol.linear, "{what}: {} off", worst * k);
}

fn grid(lo: [f64; 3], hi: [f64; 3], n: usize) -> Vec<[f64; 3]> {
    let f = |m: usize, a: f64, b: f64| a + (b - a) * (m as f64 + 0.37) / n as f64;
    let mut out = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for l in 0..n {
                out.push([f(i, lo[0], hi[0]), f(j, lo[1], hi[1]), f(l, lo[2], hi[2])]);
            }
        }
    }
    out
}

/// The top rim of an arch wall whose end face goes on from the arch
/// at a shallow angle (20° and 30° off straight): the edge runs past its end
/// at a slant to the end face, so the fillet has to be followed past the end
/// for longer (by the sine at which the edge meets the face) before it is
/// wholly out of it. A point is solid exactly when it is in the wall and
/// a ball of the radius behind the cap and the arch holds it.
#[test]
fn a_fillet_whose_edge_runs_out_at_a_slant_to_its_end_face() {
    let (r, h) = (0.15, 1.0);
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        for degrees in [160.0_f64, 150.0] {
            let outline = Outline::new(degrees.to_radians(), 0.0);
            let body = outline.prism(&f, h, k);
            assert_eq!(check(&body, &tol), []);
            let rim = edge_near(&body, f.point(0.0, 0.375 * k, h * k));
            let made = fillet(
                &body,
                &[FilletEdge {
                    edge: rim,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{degrees}°, scale {k}: {e}"));
            let margin = 2e-4;
            let to_centres = |p: [f64; 3]| {
                let s = outline.arch.signed([p[0], p[1]]);
                (p[2] - (h - r)).max(0.0).hypot((r - s).max(0.0))
            };
            let want = |p: [f64; 3]| {
                let (a, s) = (
                    outline.arch.signed([p[0], p[1]]),
                    outline.side.signed([p[0], p[1]]),
                );
                let in_wall = a > 0.0 && s > 0.0 && p[2] > 0.0 && p[2] < h;
                let d = to_centres(p);
                let near = [a, s, p[2] - h, d - r].iter().any(|x| x.abs() < margin);
                (!near).then_some(in_wall && d <= r)
            };
            let on_fillet = |p: [f64; 3]| {
                (p[0] > 0.3 && outline.side.signed([p[0], p[1]]) > 0.0).then(|| to_centres(p) - r)
            };
            held(
                &made,
                &body,
                k,
                &tol,
                40,
                &grid([0.3, -0.05, h - 0.3], [1.1, 0.45, h + 0.02], 14),
                &want,
                &on_fillet,
                &format!("{degrees}°, scale {k}"),
            );
        }
    }
}

/// The top rim of an arch wall whose right end is buried in a block
/// (the block's face at x = 0.8 is the third face, met at concave edges):
/// the fillet runs into the block's face and is cut there. A point is
/// solid exactly when it is in the block, or in the wall and a ball of the
/// radius behind the cap and the arch holds it.
#[test]
fn a_convex_fillet_that_runs_into_a_wall() {
    let (r, h, wall) = (0.15, 1.0, 0.8);
    let outline = Outline::new(90f64.to_radians(), 0.0);
    for k in SCALES {
        let tol = tolerance(k);
        let f = frame(k);
        let prism = outline.prism(&f, h, k);
        let blocked = block(&f, [wall, -2.0, -0.5], [1.6, 2.0, 2.0], k);
        let body = boolean(&prism, &blocked, BooleanOp::Union, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&body, &tol), []);
        let rim = edge_near(&body, f.point(0.0, 0.375 * k, h * k));
        let made = fillet(
            &body,
            &[FilletEdge {
                edge: rim,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        let margin = 2e-4;
        let to_centres = |p: [f64; 3]| {
            let s = outline.arch.signed([p[0], p[1]]);
            (p[2] - (h - r)).max(0.0).hypot((r - s).max(0.0))
        };
        let want = |p: [f64; 3]| {
            let s = outline.arch.signed([p[0], p[1]]);
            let in_block = p[0] > wall && p[2] > -0.5 && p[2] < 2.0;
            let in_wall = s > 0.0 && p[2] > 0.0 && p[2] < h;
            let d = to_centres(p);
            let near = [s, p[0] - wall, p[2] - h, d - r]
                .iter()
                .any(|x| x.abs() < margin);
            (!near).then_some(in_block || (in_wall && d <= r))
        };
        let on_fillet = |p: [f64; 3]| (p[0] > -0.5).then(|| to_centres(p) - r);
        held(
            &made,
            &body,
            k,
            &tol,
            40,
            &grid([0.3, -0.05, h - 0.3], [1.0, 0.45, h + 0.02], 14),
            &want,
            &on_fillet,
            &format!("scale {k}"),
        );
    }
}

/// The root of an arch wall standing on a plate, its end a bowed spline
/// side meeting the arch at an angle each side of a right one: the root
/// fillet adds material outside the wall up to the side's surface
/// continued, so its border on the plate ends past the corner of the
/// plate's edge along the side (lengthened) and the side's spline face is
/// grown past its patch. A point is solid exactly when it is in the plate
/// or the wall, or above the plate, outside the wall, on the wall's side
/// of the side's surface, and no ball of the radius in the air above the
/// plate and beside the arch holds it.
#[test]
fn a_concave_root_fillet_that_runs_out_at_the_end_of_a_wall() {
    let (r, h) = (0.15, 1.0);
    for degrees in [80.0_f64, 100.0] {
        let outline = Outline::new(degrees.to_radians(), 0.05);
        for k in SCALES {
            let tol = tolerance(k);
            let f = frame(k);
            let prism = outline.prism(&f, h, k);
            let plate = block(&f, [-2.0, -2.0, -0.5], [2.5, 2.0, 0.0], k);
            let body = boolean(&plate, &prism, BooleanOp::Union, &tol)
                .unwrap()
                .body;
            assert_eq!(check(&body, &tol), []);
            let root = edge_near(&body, f.point(0.0, 0.375 * k, 0.0));
            let made = fillet(
                &body,
                &[FilletEdge {
                    edge: root,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{degrees}°, scale {k}: {e}"));
            let margin = 2e-4;
            // The distance to the centres of balls in the air: above the
            // plate by the radius, out of the arch by it.
            let to_centres = |p: [f64; 3]| {
                let out = -outline.arch.signed([p[0], p[1]]);
                (r - p[2]).max(0.0).hypot((r - out).max(0.0))
            };
            let want = |p: [f64; 3]| {
                let (a, s) = (
                    outline.arch.signed([p[0], p[1]]),
                    outline.side.signed([p[0], p[1]]),
                );
                let d = to_centres(p);
                let near = [a, s, p[2], d - r].iter().any(|x| x.abs() < margin);
                let plate = p[2] < 0.0;
                let wall = a > 0.0 && s > 0.0 && p[2] < h;
                let filled = p[2] > 0.0 && a < 0.0 && s > 0.0 && d > r;
                (!near).then_some(plate || wall || filled)
            };
            let on_fillet = |p: [f64; 3]| (p[0] > 0.3).then(|| to_centres(p) - r);
            held(
                &made,
                &body,
                k,
                &tol,
                40,
                &grid([0.4, -0.15, -0.03], [1.3, 0.4, 0.25], 14),
                &want,
                &on_fillet,
                &format!("{degrees}°, scale {k}"),
            );
        }
    }
}

/// A wavy spline (a cubic B-spline of 24 control points over x from -1
/// to 1, its ends on y = 0, the material below it), in the plane and as
/// a sketch curve.
fn wavy() -> (craft3d_geom::NurbsCurve, Vec<f64>, Vec<[f64; 2]>) {
    let n = 24;
    let control: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let x = -1.0 + 2.0 * f64::from(i) / f64::from(n - 1);
            [x, (1.0 - x * x) * (0.3 + 0.08 * (9.0 * x).sin())]
        })
        .collect();
    let mut knots = vec![0.0; 4];
    for j in 1..(n - 3) {
        knots.push(f64::from(j) / f64::from(n - 3));
    }
    knots.extend([1.0; 4]);
    let curve = craft3d_geom::NurbsCurve::polynomial(
        3,
        knots.clone(),
        control.iter().map(|p| Vec3::new(p[0], p[1], 0.0)).collect(),
    )
    .unwrap();
    (curve, knots, control)
}

/// The signed distance of `q` from `curve` in the plane, positive to the
/// right of its direction (below the wavy top).
fn signed_from(curve: &craft3d_geom::NurbsCurve, q: [f64; 2]) -> f64 {
    let p = Vec3::new(q[0], q[1], 0.0);
    let mut t = (0..=400)
        .map(|i| f64::from(i) / 400.0)
        .min_by(|a, b| {
            curve
                .eval(*a)
                .distance(p)
                .total_cmp(&curve.eval(*b).distance(p))
        })
        .unwrap();
    for _ in 0..40 {
        let (c, d) = curve.derivatives(t);
        let h = 1e-6;
        let (_, d2) = curve.derivatives(t + h);
        let dd = (d2 - d) * (1.0 / h);
        let r = c - p;
        t -= r.dot(d) / (d.dot(d) + r.dot(dd));
        t = t.clamp(0.0, 1.0);
    }
    let (c, d) = curve.derivatives(t);
    let d = d.normalized().unwrap();
    (p.x - c.x) * d.y - (p.y - c.y) * d.x
}

/// The top rim of a wall standing on a wavy spline, rounded with a small
/// radius at a fine tolerance (the impeller's finding F6: a blade's tip
/// fillet at the default tolerance): the fillet face needs more samples
/// along the edge than a dense interpolation takes (it is laid through
/// them by the banded system), and is held to the definition.
#[test]
fn a_small_fillet_along_a_wavy_rim_at_a_fine_tolerance() {
    use craft3d_kernel::extrude::extrude;
    let (r, h) = (0.02, 1.0);
    let (curve, knots, control) = wavy();
    for k in SCALES {
        let tol = Tolerance::new(1e-7 * k, 1e-9).unwrap();
        let f = frame(k);
        let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
            id,
            curve: Curve2::Line {
                start: [a[0] * k, a[1] * k],
                end: [b[0] * k, b[1] * k],
            },
        };
        let curves = vec![
            SketchCurve {
                id: 1,
                curve: Curve2::Spline(Spline2 {
                    degree: 3,
                    knots: knots.clone(),
                    control: control.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
                }),
            },
            line(2, [1.0, 0.0], [1.0, -0.5]),
            line(3, [1.0, -0.5], [-1.0, -0.5]),
            line(4, [-1.0, -0.5], [-1.0, 0.0]),
        ];
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let body = extrude(&profiles, &f, 0.0, h * k, &tol).unwrap().body;
        assert_eq!(check(&body, &tol), []);
        let rim = edge_near(&body, f.point(0.0, 0.3 * k, h * k));
        let made = fillet(
            &body,
            &[FilletEdge {
                edge: rim,
                radius: r * k,
            }],
            &tol,
        )
        .unwrap_or_else(|e| panic!("scale {k}: {e}"));
        let samples = made
            .faces
            .iter()
            .zip(&made.body.faces)
            .filter(|(f, _)| matches!(f, FilletFace::Fillet(_)))
            .map(|(_, face)| match &face.surface {
                craft3d_geom::Surface::Nurbs(n) => n.counts().0,
                _ => 0,
            })
            .max()
            .unwrap();
        assert!(samples > 256, "{samples} samples");
        let margin = 2e-5;
        let to_centres = |p: [f64; 3]| {
            let s = signed_from(&curve, [p[0], p[1]]);
            (p[2] - (h - r)).max(0.0).hypot((r - s).max(0.0))
        };
        let want = |p: [f64; 3]| {
            let s = signed_from(&curve, [p[0], p[1]]);
            let in_wall = s > 0.0 && p[2] > 0.0 && p[2] < h;
            let d = to_centres(p);
            let near = [s, p[2] - h, d - r].iter().any(|x| x.abs() < margin);
            (!near).then_some(in_wall && d <= r)
        };
        let on_fillet = |p: [f64; 3]| (p[0].abs() < 0.25).then(|| to_centres(p) - r);
        // Across the rim over a stretch of it.
        let y = curve.eval(0.5).y;
        held(
            &made,
            &body,
            k,
            &tol,
            15,
            &grid([-0.06, y - 0.04, h - 0.04], [0.06, y + 0.01, h + 0.005], 14),
            &want,
            &on_fillet,
            &format!("{samples} samples, scale {k}"),
        );
    }
}
