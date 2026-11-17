//! A fillet is the face a ball of the radius sweeps as it rolls along the
//! edge touching both faces: it rounds a convex corner off and fills a
//! concave one. Where a face beside the edge bulges into that ball between
//! the edge and the place the ball touches it (a bead on the wall of a
//! groove, next to its rim), the fillet face passes through that face, and
//! the fillet is refused (docs/design/blends.md › *A fillet through a face
//! beside it*). What 0.1.0 built there was a valid solid that added
//! material at the convex rim, a lens of the groove's empty space under the
//! bead: property cases 585 and 716 of `blend_monotone`.
//!
//! The oracles are worked out in 2D across the edge (the wall is a spline
//! profile extruded along the edge, and the block's ends are square to it,
//! so every section is the same and the fillet is its section extruded):
//! the ball of the radius resting on the top that touches the wall at a
//! given place (Newton on its centre and the parameter of its foot), how
//! deep the wall between the edge and that foot lies inside it, between the
//! rays from its centre to its feet (sampled finely), the radius at which
//! it touches the wall at two places (Newton on the centre, the radius and
//! both parameters), and the region the fillet takes or adds per unit of
//! length by Green's theorem (five-point Gauss-Legendre on the spline's
//! spans, exact for them; the arc in closed form). At the scales 1e-3, 1
//! and 1e6, in the world and in two frames away from the origin, one
//! tilted.
//!
//! The block is 40 x 30 x 20 (x along the edge, y across it, z up, in the
//! frame of the test), its top at z = 20; the wall leaves the top at
//! (y, z) = (21, 20) at 52.5° to it, down into a groove (or up a ridge),
//! with a bead 0.365 high over its second and third inner control points.

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

use boolean_common::{SCALES, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{Fade, FilletEdge, FilletLawEdge, RadiusLaw, fillet, fillet_by_law};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check};

type P = [f64; 2];

/// Where the wall leaves the top, and where the spline comes back to it.
const A: f64 = 21.0;
const B: f64 = 6.3;

/// The tolerance at scale 1.
const TOL: f64 = 1e-6;

/// The world and two frames away from the origin, one of them tilted.
fn frames(k: f64) -> Vec<Frame> {
    let mut all = common::frames(k);
    all.push(
        Frame::new(
            Vec3::new(400.0 * k, -300.0 * k, 250.0 * k),
            Vec3::new(-2.0, 1.0, 2.0),
            Vec3::new(0.0, 1.0, -0.5),
        )
        .unwrap(),
    );
    all
}

/// The wall's clamped cubic spline in the section (y, z), from the edge.
struct Wall {
    control: Vec<P>,
    knots: Vec<f64>,
}

impl Wall {
    /// Down into a groove (`groove`) or up a ridge, with a bead (`bead`,
    /// towards the side the fillet's ball lies on) or a hollow over its
    /// second and third inner control points: the profile of the property's
    /// walls, `blend_monotone`'s, with round numbers.
    fn new(groove: bool, bead: bool) -> Wall {
        let slope = 52.5_f64.to_radians();
        let (w, h) = (2.64, 14.6);
        let lift = if bead { 0.365 } else { -0.365 };
        let along = [-slope.pcos(), slope.psin()];
        let out = [slope.psin(), slope.pcos()];
        let at = |s: f64, off: f64| {
            [
                A + along[0] * s * w + out[0] * off,
                20.0 + along[1] * s * w + out[1] * off,
            ]
        };
        let top = at(1.6, 0.0);
        let control = [
            [A, 20.0],
            at(0.15, 0.0),
            at(0.35, lift),
            at(0.55, lift),
            at(0.75, 0.0),
            at(0.95, 0.0),
            top,
            [top[0] - 1.0, 20.0 + h],
            [B, 20.0 + h],
            [B, 20.0],
        ]
        .into_iter()
        .map(|[y, z]| if groove { [y, 40.0 - z] } else { [y, z] })
        .collect();
        let knots = [
            0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 7.0, 7.0, 7.0,
        ]
        .into_iter()
        .map(|k| k / 7.0)
        .collect();
        Wall { control, knots }
    }

    fn span(&self, t: f64) -> usize {
        let mut span = 3;
        while span + 1 < self.control.len() && self.knots[span + 1] <= t {
            span += 1;
        }
        span
    }

    /// Its point at `t` (de Boor).
    fn at(&self, t: f64) -> P {
        let p = 3;
        let span = self.span(t);
        let mut d: Vec<P> = (0..=p).map(|j| self.control[span - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let den = self.knots[i + p + 1 - r] - self.knots[i];
                let a = if den > 0.0 {
                    (t - self.knots[i]) / den
                } else {
                    0.0
                };
                d[j] = [
                    (1.0 - a) * d[j - 1][0] + a * d[j][0],
                    (1.0 - a) * d[j - 1][1] + a * d[j][1],
                ];
            }
        }
        d[p]
    }

    /// Its derivative at `t`, exactly: the spline of degree two of the
    /// control points' differences.
    fn derivative(&self, t: f64) -> P {
        let p = 3;
        let n = self.control.len();
        let q: Vec<P> = (0..n - 1)
            .map(|i| {
                let s = p as f64 / (self.knots[i + p + 1] - self.knots[i + 1]);
                [
                    s * (self.control[i + 1][0] - self.control[i][0]),
                    s * (self.control[i + 1][1] - self.control[i][1]),
                ]
            })
            .collect();
        let knots = &self.knots[1..self.knots.len() - 1];
        let p = 2;
        let span = self.span(t) - 1;
        let mut d: Vec<P> = (0..=p).map(|j| q[span - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let den = knots[i + p + 1 - r] - knots[i];
                let a = if den > 0.0 { (t - knots[i]) / den } else { 0.0 };
                d[j] = [
                    (1.0 - a) * d[j - 1][0] + a * d[j][0],
                    (1.0 - a) * d[j - 1][1] + a * d[j][1],
                ];
            }
        }
        d[p]
    }

    /// Half the integral of y z' - z y' from 0 to `t1`: five-point
    /// Gauss-Legendre on each knot span (in each a polynomial of degree
    /// five).
    fn green(&self, t1: f64) -> f64 {
        const X: [f64; 5] = [
            -0.906_179_845_938_664,
            -0.538_469_310_105_683_1,
            0.0,
            0.538_469_310_105_683_1,
            0.906_179_845_938_664,
        ];
        const W: [f64; 5] = [
            0.236_926_885_056_189_1,
            0.478_628_670_499_366_5,
            0.568_888_888_888_888_9,
            0.478_628_670_499_366_5,
            0.236_926_885_056_189_1,
        ];
        let mut breaks: Vec<f64> = self.knots.iter().copied().filter(|k| *k < t1).collect();
        breaks.dedup();
        breaks.push(t1);
        let mut sum = 0.0;
        for pair in breaks.windows(2) {
            let (m, h) = (0.5 * (pair[0] + pair[1]), 0.5 * (pair[1] - pair[0]));
            for (x, w) in X.iter().zip(W) {
                let u = m + h * x;
                let (p, q) = (self.at(u), self.derivative(u));
                sum += w * h * 0.5 * (p[0] * q[1] - p[1] * q[0]);
            }
        }
        sum
    }

    /// The ball of radius `r` resting on the top (its centre `sigma` r
    /// above it: -1 in a groove, 1 beside a ridge) that touches the wall
    /// near the parameter `t0`: its centre's y and its foot's parameter,
    /// by Newton on the distance (the radius) and the squareness there.
    fn ball(&self, r: f64, sigma: f64, t0: f64) -> (f64, f64) {
        let f = |y: f64, t: f64| {
            let (p, d) = (self.at(t), self.derivative(t));
            let q = [y - p[0], 20.0 + sigma * r - p[1]];
            [q[0].phypot(q[1]) - r, q[0] * d[0] + q[1] * d[1]]
        };
        // Start square to the wall at t0, the radius out from it.
        let (p, d) = (self.at(t0), self.derivative(t0));
        let n = d[0].phypot(d[1]);
        let mut x = [p[0] + r * d[1].abs() / n, t0];
        for _ in 0..100 {
            let now = f(x[0], x[1]);
            if now.iter().all(|v| v.abs() < 1e-14) {
                break;
            }
            let h = 1e-8;
            let (dy, dt) = (
                [
                    (f(x[0] + h, x[1])[0] - f(x[0] - h, x[1])[0]) / (2.0 * h),
                    (f(x[0] + h, x[1])[1] - f(x[0] - h, x[1])[1]) / (2.0 * h),
                ],
                [
                    (f(x[0], x[1] + h)[0] - f(x[0], x[1] - h)[0]) / (2.0 * h),
                    (f(x[0], x[1] + h)[1] - f(x[0], x[1] - h)[1]) / (2.0 * h),
                ],
            );
            let det = dy[0] * dt[1] - dt[0] * dy[1];
            x[0] -= (now[0] * dt[1] - dt[0] * now[1]) / det;
            x[1] -= (dy[0] * now[1] - now[0] * dy[1]) / det;
        }
        let now = f(x[0], x[1]);
        assert!(
            now.iter().all(|v| v.abs() < 1e-11),
            "no ball of {r} near {t0}: {now:?}"
        );
        (x[0], x[1])
    }

    /// How deep the wall between the edge and the foot `t1` of the ball of
    /// radius `r` centred at (`y`, 20 + `sigma` r) lies inside it, between
    /// the rays from the centre to its two feet; and how deep the wall
    /// beyond the foot (to the parameter 5/7, up the wall) does, anywhere.
    fn inside(&self, y: f64, r: f64, sigma: f64, t1: f64) -> (f64, f64) {
        let c = [y, 20.0 + sigma * r];
        let g = self.at(t1);
        let (a1, a2) = ([0.0, -sigma * r], [g[0] - c[0], g[1] - c[1]]);
        let cross = |u: P, v: P| u[0] * v[1] - u[1] * v[0];
        let turn = cross(a1, a2).signum();
        let steps = 200_000;
        let mut before: f64 = 0.0;
        for i in 1..steps {
            let p = self.at(t1 * f64::from(i) / f64::from(steps));
            let q = [p[0] - c[0], p[1] - c[1]];
            if cross(a1, q) * turn > 0.0 && cross(q, a2) * turn > 0.0 {
                before = before.max(r - q[0].phypot(q[1]));
            }
        }
        let mut beyond: f64 = 0.0;
        for i in 1..steps {
            let p = self.at(t1 + (5.0 / 7.0 - t1) * f64::from(i) / f64::from(steps));
            beyond = beyond.max(r - (p[0] - c[0]).phypot(p[1] - c[1]));
        }
        (before, beyond)
    }

    /// The radius at which the ball resting on the top touches the wall at
    /// two places, Newton from `guess` (its centre's y, the radius and the
    /// two parameters): the radius and the two parameters.
    fn twice(&self, sigma: f64, guess: [f64; 4]) -> [f64; 4] {
        let f = |x: [f64; 4]| {
            let c = [x[0], 20.0 + sigma * x[1]];
            let mut out = [0.0; 4];
            for (k, s) in [x[2], x[3]].into_iter().enumerate() {
                let (p, d) = (self.at(s), self.derivative(s));
                let q = [c[0] - p[0], c[1] - p[1]];
                out[2 * k] = q[0].phypot(q[1]) - x[1];
                out[2 * k + 1] = q[0] * d[0] + q[1] * d[1];
            }
            out
        };
        let mut x = guess;
        for _ in 0..100 {
            let now = f(x);
            if now.iter().all(|v| v.abs() < 1e-14) {
                break;
            }
            let mut jac = [[0.0; 4]; 4];
            for j in 0..4 {
                let h = 1e-7;
                let (mut up, mut down) = (x, x);
                up[j] += h;
                down[j] -= h;
                let (fu, fd) = (f(up), f(down));
                for i in 0..4 {
                    jac[i][j] = (fu[i] - fd[i]) / (2.0 * h);
                }
            }
            let step = solve4(jac, now.map(|v| -v));
            for j in 0..4 {
                x[j] += step[j];
            }
        }
        assert!(f(x).iter().all(|v| v.abs() < 1e-12), "{x:?}: {:?}", f(x));
        x
    }

    /// What the fillet of radius `r` whose ball is centred at (`y`,
    /// 20 + `sigma` r) and touches the wall at `t1` takes or adds per unit
    /// of length: the region from the edge along the top to the foot on
    /// it, round the arc to the foot on the wall and down the wall back to
    /// the edge (Green's theorem).
    fn region(&self, y: f64, r: f64, sigma: f64, t1: f64) -> f64 {
        let (e, f1, g) = ([A, 20.0], [y, 20.0], self.at(t1));
        let c = [y, 20.0 + sigma * r];
        let top = 0.5 * (e[0] * f1[1] - f1[0] * e[1]);
        let p0 = (f1[1] - c[1]).patan2(f1[0] - c[0]);
        let mut turn = (g[1] - c[1]).patan2(g[0] - c[0]) - p0;
        if turn > std::f64::consts::PI {
            turn -= std::f64::consts::TAU;
        }
        if turn < -std::f64::consts::PI {
            turn += std::f64::consts::TAU;
        }
        let p1 = p0 + turn;
        let arc = 0.5
            * (r * c[0] * (p1.psin() - p0.psin()) - r * c[1] * (p1.pcos() - p0.pcos())
                + r * r * turn);
        (top + arc - self.green(t1)).abs()
    }
}

/// The solution of four linear equations (partial pivoting).
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> [f64; 4] {
    for c in 0..4 {
        let pivot = (c..4)
            .max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs()))
            .unwrap();
        a.swap(c, pivot);
        b.swap(c, pivot);
        let row = a[c];
        for r in 0..4 {
            if r != c {
                let m = a[r][c] / row[c];
                for (x, p) in a[r].iter_mut().zip(row) {
                    *x -= m * p;
                }
                b[r] -= m * b[c];
            }
        }
    }
    [
        b[0] / a[0][0],
        b[1] / a[1][1],
        b[2] / a[2][2],
        b[3] / a[3][3],
    ]
}

/// The block with the wall's profile, extruded 40 along x in `frame` at
/// scale `k`.
fn body(wall: &Wall, frame: &Frame, k: f64) -> Body {
    let tol = tolerance(k);
    let line = |id: u64, s: P, e: P| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [s[0] * k, s[1] * k],
            end: [e[0] * k, e[1] * k],
        },
    };
    let curves = vec![
        line(1, [0.0, 0.0], [30.0, 0.0]),
        line(2, [30.0, 0.0], [30.0, 20.0]),
        line(3, [30.0, 20.0], [A, 20.0]),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: wall.knots.clone(),
                control: wall.control.iter().map(|p| [p[0] * k, p[1] * k]).collect(),
            }),
        },
        line(5, [B, 20.0], [0.0, 20.0]),
        line(6, [0.0, 20.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    let body = extrude(&profiles, &along, 0.0, 40.0 * k, &tol)
        .unwrap()
        .body;
    assert_eq!(check(&body, &tol), []);
    body
}

/// The edge where the wall leaves the top, and the wall's face.
fn rim(body: &Body, frame: &Frame, k: f64) -> (EdgeId, FaceId) {
    let point = frame.point(20.0 * k, A * k, 20.0 * k);
    let (index, _) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(point),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let edge = EdgeId(index as u32);
    let wall = (0..body.faces.len())
        .find(|i| {
            !matches!(body.faces[*i].surface, Surface::Plane(_))
                && body.faces[*i]
                    .loops
                    .iter()
                    .flat_map(|l| &body.loop_(*l).unwrap().coedges)
                    .any(|c| c.edge == edge)
        })
        .unwrap();
    (edge, FaceId(wall as u32))
}

/// The fillet of the rim of radius `r` (scale 1) built at every scale and
/// in every frame, its volume the region `region` (per unit of length,
/// scale 1) taken (a groove) or added (a ridge) over the 40 of the edge.
fn built(groove: bool, bead: bool, r: f64, region: f64) {
    let wall = Wall::new(groove, bead);
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = body(&wall, &frame, k);
            let (edge, _) = rim(&body, &frame, k);
            let v0 = mass(&body).volume;
            let made = fillet(
                &body,
                &[FilletEdge {
                    edge,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap_or_else(|e| panic!("{r} at {k}: {e}"));
            assert_eq!(check(&made.body, &tol), [], "{r} at {k}");
            let change = 40.0 * region * k * k * k;
            let expected = if groove { v0 - change } else { v0 + change };
            let got = mass(&made.body).volume;
            assert!(
                (got - expected).abs() <= 1e-7 * v0,
                "{r} at {k}: {got} for {expected}"
            );
        }
    }
}

/// The fillet of the rim of radius `r` (scale 1) refused at every scale
/// and in every frame, naming the rim and the wall.
fn refused(groove: bool, bead: bool, r: f64) {
    let wall = Wall::new(groove, bead);
    for k in SCALES {
        let tol = tolerance(k);
        for frame in frames(k) {
            let body = body(&wall, &frame, k);
            let (edge, face) = rim(&body, &frame, k);
            match fillet(
                &body,
                &[FilletEdge {
                    edge,
                    radius: r * k,
                }],
                &tol,
            ) {
                Err(KernelError::Fillet {
                    edge: Some(e),
                    face: Some(f),
                    reason,
                }) => {
                    assert_eq!((e, f), (edge, face), "{r} at {k}: {reason}");
                    assert!(reason.contains("pass through"), "{r} at {k}: {reason}");
                }
                Err(other) => panic!("{r} at {k}: refused otherwise: {other}"),
                Ok(made) => panic!(
                    "{r} at {k}: built, {} for {}",
                    mass(&made.body).volume,
                    mass(&body).volume
                ),
            }
        }
    }
}

/// The groove (its rim convex, the ball under the top) and the ridge (its
/// foot concave, the ball over the top): the ridge's wall is the groove's
/// mirrored in the top, and so is every ball.
const SIDES: [(bool, f64); 2] = [(true, -1.0), (false, 1.0)];

/// Sampled depths at or below this are taken for none (a foot itself lies
/// on the ball, to the Newton's residual).
const NONE: f64 = 1e-9;

#[test]
fn a_fillet_is_refused_where_a_bead_on_the_wall_reaches_into_its_ball() {
    // The ball of 5.125 resting on the top and touching the wall below the
    // bead (where the construction finds it, from the flat estimate) has
    // the bead 0.1 to 0.15 deep inside it between the edge and its foot:
    // what 0.1.0 built there added that lens of the groove to the block
    // (took it from the ridge). The ball that rests on the bead instead is
    // clear of the whole wall: the fillet of that radius, which the
    // construction does not find. The same at 4.8.
    for (groove, sigma) in SIDES {
        let wall = Wall::new(groove, true);
        for r in [5.125, 4.8] {
            let (y, t1) = wall.ball(r, sigma, 0.69);
            let (before, _) = wall.inside(y, r, sigma, t1);
            assert!(before > 0.1 && before < 0.25, "{r}: {before}");
            let (yb, tb) = wall.ball(r, sigma, 0.26);
            let (on_bead, beyond_bead) = wall.inside(yb, r, sigma, tb);
            assert!(
                yb > y && tb < 0.35 && on_bead <= NONE && beyond_bead <= NONE,
                "{r}: {yb} {tb} {on_bead} {beyond_bead}"
            );
            refused(groove, true, r);
        }
    }
}

#[test]
fn a_fillet_is_built_where_the_bead_stays_within_the_tolerance_of_its_ball() {
    // As the radius grows the ball's foot moves down the wall and the bead
    // comes out of the ball: at the radius worked out (where the ball
    // touches the bead and the wall below it at once), and past it, the
    // fillet is the region by Green's theorem; just short of it, where the
    // bead lies inside the ball by no more than the tolerance, too (0.1.0
    // built those, the same). Short of that by more, refused.
    for (groove, sigma) in SIDES {
        let wall = Wall::new(groove, true);
        let (y, t1) = wall.ball(5.5, sigma, 0.69);
        let (_, tb) = wall.ball(5.5, sigma, 0.26);
        let [_, star, s0, s1] = wall.twice(sigma, [y, 5.5, tb, t1]);
        assert!(
            star > 5.0 && star < 6.0 && s0 < 0.35 && s1 > 0.6,
            "{star} {s0} {s1}"
        );
        for (step, deep) in [(-2e-6, true), (-1e-7, false), (1e-7, false), (1e-3, false)] {
            let r = star * (1.0 + step);
            let (y, t1) = wall.ball(r, sigma, s1);
            let (before, _) = wall.inside(y, r, sigma, t1);
            assert_eq!(before > TOL, deep, "{groove} {step}: {before}");
            if deep {
                refused(groove, true, r);
            } else {
                built(groove, true, r, wall.region(y, r, sigma, t1));
            }
        }
    }
}

#[test]
fn a_small_fillet_beside_the_bead_is_built() {
    // The balls of 0.5 and 1 touch the wall above the bead: the wall
    // between the edge and the foot keeps clear of them, and the fillet is
    // the region by Green's theorem, as 0.1.0 built it.
    for (groove, sigma) in SIDES {
        let wall = Wall::new(groove, true);
        for r in [0.5, 1.0] {
            let (y, t1) = wall.ball(r, sigma, 0.1);
            let (before, _) = wall.inside(y, r, sigma, t1);
            assert!(before <= NONE, "{groove} {r}: {before}");
            built(groove, true, r, wall.region(y, r, sigma, t1));
        }
    }
}

#[test]
fn a_fillet_beside_a_hollow_is_built() {
    // A hollow bends the wall away from the ball: at these radii nothing
    // of the wall between the edge and the foot comes into it, and the
    // fillet is the region by Green's theorem, as 0.1.0 built it.
    for (groove, sigma) in SIDES {
        let wall = Wall::new(groove, false);
        for (r, t0) in [(0.5, 0.02), (1.3, 0.05), (4.8, 0.69), (6.0, 0.7)] {
            let (y, t1) = wall.ball(r, sigma, t0);
            let (before, _) = wall.inside(y, r, sigma, t1);
            assert!(before <= NONE, "{groove} {r}: {before}");
            built(groove, false, r, wall.region(y, r, sigma, t1));
        }
    }
}

#[test]
fn a_radius_that_varies_is_held_to_the_wall_by_the_radius_of_each_plane() {
    // A radius by a law along the rim (`fillet_by_law`): 1.03 times the
    // radius at which the ball touches the bead and the wall below it at
    // once, the bead clear of that ball, falling at the edge's start to
    // 0.93 of it over 12, where the bead lies deep inside the ball: the
    // planes near that end show it, and the fillet is refused naming the
    // wall. Falling only to 1.005 of it the bead keeps clear all along,
    // and it is built. (Held by the law's largest radius in every plane,
    // the second is refused too: tried.)
    for (groove, sigma) in SIDES {
        let wall = Wall::new(groove, true);
        let (y, t1) = wall.ball(5.5, sigma, 0.69);
        let (_, tb) = wall.ball(5.5, sigma, 0.26);
        let [_, star, _, s1] = wall.twice(sigma, [y, 5.5, tb, t1]);
        let (y, t1) = wall.ball(0.93 * star, sigma, s1);
        let (before, _) = wall.inside(y, 0.93 * star, sigma, t1);
        assert!(before > 0.05, "{groove}: {before}");
        for k in SCALES {
            let tol = tolerance(k);
            for frame in frames(k) {
                let body = body(&wall, &frame, k);
                let (edge, face) = rim(&body, &frame, k);
                let law = |to: f64| RadiusLaw {
                    radius: 1.03 * star * k,
                    fades: [Some(Fade::to(to * star * k, 12.0 * k)), None],
                };
                let try_law =
                    |to: f64| fillet_by_law(&body, &[FilletLawEdge { edge, law: law(to) }], &tol);
                match try_law(0.93) {
                    Err(KernelError::Fillet {
                        edge: Some(e),
                        face: Some(f),
                        reason,
                    }) => {
                        assert_eq!((e, f), (edge, face), "{groove} at {k}: {reason}");
                        assert!(reason.contains("pass through"), "{groove} at {k}: {reason}");
                    }
                    Err(other) => panic!("{groove} at {k}: refused otherwise: {other}"),
                    Ok(_) => panic!("{groove} at {k}: built"),
                }
                let made = try_law(1.005).unwrap_or_else(|e| panic!("{groove} at {k}: {e}"));
                assert_eq!(check(&made.body, &tol), [], "{groove} at {k}");
            }
        }
    }
}
