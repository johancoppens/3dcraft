//! A property every blend has, whatever builds it: a chamfer or a fillet
//! of an edge that is concave all along fills the corner and never takes
//! material away; one of an edge that is convex all along cuts the corner
//! off and never adds any; and nothing changes farther from the edge than
//! the blend reaches. Held over seeded random blends of the kinds the
//! kernel accepts: a block with a cylinder lying in its top, standing
//! through it or lying at a slant, a cone standing or lying, a ball or a
//! ring (torus) standing or lying united with it or taken out of it, and
//! its top shaped as a spline hump, an ogee (bending one way and then the
//! other, its nose over the top) or a wall with a bead or a hollow next to
//! the edge, or the groove they make in it; every edge between the top and
//! the other face chamfered or filleted at random sizes from small to well
//! past what fits, convex and concave, with sections the same all along
//! and changing along the edge (docs/design/blends.md › *A chamfer through
//! a face beside it*). Cost: about 5 minutes for the five parts side by
//! side in the test profile (the last, with the lying ring, the longest).
//!
//! A body whose points cannot all be classified is held to its volume
//! only; they are counted, and more than one in ten fails the test.
//!
//! The oracles do not ask how the blend was built: the volumes before and
//! after (`mass_properties`), and points classified against both bodies
//! by rays (`classify_points`, which does not use the mass integrals): a
//! point inside before and outside after is material taken, the other way
//! round material added; either one farther from the edge than the blend
//! reaches is a change where there should be none. A wrong body is never
//! allowed.
//!
//! Every case's outcome is held against the one the released kernel
//! (0.1.0) gave, recorded in [`RELEASED`]: a blend it built that is not
//! seen wrong here has to be built still (a chamfer it built right may not
//! be refused now), one it built that is seen wrong may be refused, one it
//! refused may be refused or built right. The cases that set up no blend
//! (the boolean refused, or no edge between the top and the other face is
//! convex or concave all along) are counted and pinned as well.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod boolean_common;

use boolean_common::{Rng, tolerance};
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, check, edge_uses};

/// The kinds of second body, by the surface its faces next to the block
/// are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Lying,
    Standing,
    Cone,
    Ball,
    Ring,
    Hump,
    /// A spline profile that bends one way and then the other, its nose
    /// overhanging the top: a face that curves back between the edge and
    /// a cut line on it.
    Ogee,
    /// A cone lying along x: its sections across the edge change along
    /// the edge.
    LyingCone,
    /// A cylinder lying at a slant: an elliptic edge whose sections change
    /// along it.
    Tilted,
    /// A spline wall rising from the top with a bead near the edge: its
    /// trace bends out and back between the edge and a cut line on it, and
    /// can lie past the chamfer's line beyond its foot.
    Bead,
    /// The same with a hollow near the edge.
    Hollow,
    /// A ring lying on its side (its axis along x) through the top: its
    /// sections across the edge bend both ways on its inner side.
    LyingRing,
    /// The wall with a bead, chamfered with its foot on the bead's far
    /// slope, near the size at which the line between the feet is tangent
    /// to the wall there.
    BeadSlope,
    /// The same with a hollow.
    HollowSlope,
}

const KINDS: [Kind; 9] = [
    Kind::Lying,
    Kind::Standing,
    Kind::Cone,
    Kind::Ball,
    Kind::Ring,
    Kind::Hump,
    Kind::Ogee,
    Kind::LyingCone,
    Kind::Tilted,
];

/// The kinds of the cases from [`FIRST_NEW`] on (2026-11-14): traces that
/// bend both ways next to the edge.
const NEW_KINDS: [Kind; 3] = [Kind::Bead, Kind::Hollow, Kind::LyingRing];

/// The first case of [`NEW_KINDS`]; the cases before it keep their kinds.
const FIRST_NEW: usize = 480;

/// The kinds of the cases from [`FIRST_SLOPE`] on (2026-11-14): a foot on
/// the far slope of a bead or a hollow ([`slope`]).
const SLOPE_KINDS: [Kind; 2] = [Kind::BeadSlope, Kind::HollowSlope];

/// The first case of [`SLOPE_KINDS`].
const FIRST_SLOPE: usize = 600;

/// The block with its top shaped across y by a spline from (a, 20) to
/// (b, 20) (its `control` points, clamped `knots`): above the top as a
/// ridge (`union`), or mirrored into the block as a groove. One profile
/// extruded along x over the block's length, so that the edges where the
/// spline meets the top end at corners of three faces.
fn ridge(
    frame: &Frame,
    tol: &Tolerance,
    (a, b): (f64, f64),
    knots: Vec<f64>,
    control: Vec<[f64; 2]>,
    union: bool,
) -> Option<Body> {
    let line = |id: u64, start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    };
    let control = control
        .into_iter()
        .map(|[y, z]| if union { [y, z] } else { [y, 40.0 - z] })
        .collect();
    let curves = vec![
        line(1, [0.0, 0.0], [30.0, 0.0]),
        line(2, [30.0, 0.0], [30.0, 20.0]),
        line(3, [30.0, 20.0], [a, 20.0]),
        SketchCurve {
            id: 4,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots,
                control,
            }),
        },
        line(5, [b, 20.0], [0.0, 20.0]),
        line(6, [0.0, 20.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tol).ok()?;
    let along = Frame::new(frame.origin(), frame.x(), frame.y()).unwrap();
    let body = extrude(&profiles, &along, 0.0, 40.0, tol).ok()?.body;
    check(&body, tol).is_empty().then_some(body)
}

/// The spline of a wall (in the section (y, z), as the body has it) from
/// the edge at (a, 20), and the bead's scale.
struct Wall {
    control: Vec<[f64; 2]>,
    knots: Vec<f64>,
    a: f64,
    w: f64,
}

impl Wall {
    /// Its point at `t` (de Boor, cubic).
    fn at(&self, t: f64) -> [f64; 2] {
        let p = 3;
        let n = self.control.len();
        let mut span = p;
        while span + 1 < n && self.knots[span + 1] <= t {
            span += 1;
        }
        let mut d: Vec<[f64; 2]> = (0..=p).map(|j| self.control[span - p + j]).collect();
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

    /// The parameter of its point `d` (straight) from the edge, the first
    /// from the edge.
    fn foot(&self, d: f64) -> Option<f64> {
        let e = [self.a, 20.0];
        let from = |t: f64| {
            let p = self.at(t);
            (p[0] - e[0]).phypot(p[1] - e[1])
        };
        let steps = 4000;
        let mut lo = 0.0;
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            if from(t) >= d {
                let mut hi = t;
                for _ in 0..100 {
                    let middle = 0.5 * (lo + hi);
                    if from(middle) < d {
                        lo = middle;
                    } else {
                        hi = middle;
                    }
                }
                return Some(0.5 * (lo + hi));
            }
            lo = t;
        }
        None
    }

    /// For the chamfer `d1` along the wall and `d2` along the top: how far
    /// the wall between the edge and its foot lies past the segment between
    /// the feet (behind it, between its ends), and, where the wall comes to
    /// its foot from past the segment's line, how far the wall beyond the
    /// foot runs back into the region between the feet. In the section,
    /// sampled finely; `None` where there is no foot.
    fn held(&self, d1: f64, d2: f64) -> Option<(f64, f64)> {
        let t1 = self.foot(d1)?;
        let (e, g1, g2) = ([self.a, 20.0], self.at(t1), [self.a + d2, 20.0]);
        let length = (g2[0] - g1[0]).phypot(g2[1] - g1[1]);
        let along = [(g2[0] - g1[0]) / length, (g2[1] - g1[1]) / length];
        let off = [e[0] - g1[0], e[1] - g1[1]];
        let a = off[0] * along[0] + off[1] * along[1];
        let t = [off[0] - a * along[0], off[1] - a * along[1]];
        let n = t[0].phypot(t[1]);
        let towards = [t[0] / n, t[1] / n];
        let clearance = |p: [f64; 2]| (p[0] - g1[0]) * towards[0] + (p[1] - g1[1]) * towards[1];
        let at = |p: [f64; 2]| (p[0] - g1[0]) * along[0] + (p[1] - g1[1]) * along[1];
        let steps = 20_000;
        let mut behind: f64 = 0.0;
        for i in 1..steps {
            let p = self.at(t1 * f64::from(i) / f64::from(steps));
            let x = at(p);
            if x > 0.0 && x < length {
                behind = behind.max(-clearance(p));
            }
        }
        let mut beyond: f64 = 0.0;
        if clearance(self.at(t1 * (1.0 - 1e-9))) < 0.0 {
            for i in 1..=steps {
                let p = self.at((t1 + t1 * f64::from(i) / f64::from(steps)).min(1.0));
                let (c, x) = (clearance(p), at(p));
                if !(c > 0.0 && x > 0.0 && x < length) {
                    break;
                }
                beyond = beyond.max(c);
            }
        }
        Some((behind, beyond))
    }

    /// The size along the wall, between `lo` and `hi` straight from the
    /// edge, at which the line from its foot to the point `d2` along the
    /// top is tangent to the wall there: where the sine of the angle
    /// between them changes sign.
    fn tangent(&self, d2: f64, lo: f64, hi: f64) -> Option<f64> {
        let sine = |d1: f64| {
            let t1 = self.foot(d1)?;
            let g1 = self.at(t1);
            let h = 1e-7;
            let (p, q) = (self.at((t1 - h).max(0.0)), self.at((t1 + h).min(1.0)));
            let d = [q[0] - p[0], q[1] - p[1]];
            let chord = [self.a + d2 - g1[0], 20.0 - g1[1]];
            let (nd, nc) = (d[0].phypot(d[1]), chord[0].phypot(chord[1]));
            Some((chord[0] * d[1] - chord[1] * d[0]) / (nd * nc))
        };
        let steps = 400;
        let mut last = (lo, sine(lo)?);
        for i in 1..=steps {
            let d1 = lo + (hi - lo) * f64::from(i) / f64::from(steps);
            let now = sine(d1)?;
            if now.signum() != last.1.signum() {
                let (mut a, mut b) = (last.0, d1);
                for _ in 0..100 {
                    let middle = 0.5 * (a + b);
                    if sine(middle)?.signum() == last.1.signum() {
                        a = middle;
                    } else {
                        b = middle;
                    }
                }
                return Some(0.5 * (a + b));
            }
            last = (d1, now);
        }
        None
    }
}

/// A case of [`SLOPE_KINDS`]: the wall's edge at (a, 20) chamfered `d2` (2
/// to 5) along the top and along the wall a size near the one at which the
/// line between the feet is tangent to the wall on the bead's far slope
/// (the first such size from the bead's top on), off it by one of a few
/// steps either way. A body built is held to the property and to the
/// section in 2D ([`Wall::held`]: past the segment, or the wall beyond the
/// foot back in the region between the feet, by more than the tolerance,
/// is wrong). `None` where no such size is found: then the case is set up
/// as the others are.
#[allow(clippy::too_many_arguments)]
fn slope(
    rng: &mut Rng,
    at: u64,
    body: &Body,
    wall: &Wall,
    frame: &Frame,
    tol: &Tolerance,
    what: &str,
    tally: &mut Tally,
) -> Option<()> {
    let d2 = rng.between(2.0, 5.0);
    let tangent = wall.tangent(d2, 0.5 * wall.w, 1.1 * wall.w)?;
    let steps = [-2e-2, -1e-2, -3e-4, -1e-4, 0.0, 1e-4, 1e-3];
    let d1 = tangent + steps[rng.below(steps.len() as u64) as usize] * wall.w;
    let (behind, beyond) = wall.held(d1, d2)?;
    // The edge at (a, 20).
    let edge = candidates(body, frame).into_iter().find(|(e, _)| {
        let data = body.edge(*e).unwrap();
        let m = data.curve.eval(0.5 * (data.range.0 + data.range.1)) - frame.origin();
        (m.dot(frame.y()) - wall.a).abs() < 1e-6
    })?;
    let (edge, convex) = edge;
    let uses = edge_uses(body);
    let spline = uses[edge.0 as usize].iter().map(|u| u.face).find(|f| {
        !matches!(
            body.face(*f).map(|f| &f.surface),
            Some(craft3d_geom::Surface::Plane(_))
        )
    })?;
    let what = format!(
        "{what}, {}: chamfer {d1:.7} along the wall (tangent {tangent:.7}) and {d2:.4}",
        if convex { "convex" } else { "concave" }
    );
    let result = chamfer(
        body,
        &[ChamferEdge {
            edge,
            first: spline,
            size: ChamferSize::Distances {
                first: d1,
                second: d2,
            },
        }],
        tol,
    );
    match result {
        Ok(made) => {
            tally.built += 1;
            println!("{what}: built");
            let found = tally.wrong.len();
            let measured = craft3d_topo::mass_properties(&made.body)
                .map_or_else(|_| "????????".to_string(), |m| measure(m.volume, m.area));
            tally.measures.push_str(&measured);
            if behind > tol.linear || beyond > tol.linear {
                tally.wrong.push(format!(
                    "{what}: the section shows it wrong (past the segment {behind:.3e}, \
                     the wall beyond the foot in the region {beyond:.3e})"
                ));
            }
            let blends = made
                .faces
                .iter()
                .map(|f| matches!(f, ChamferFace::Chamfer(_)))
                .collect::<Vec<_>>();
            judge(
                &mut Rng(at ^ 0x5DEE_CE66_D1CE_4E5B),
                body,
                &made.body,
                &blends,
                edge,
                convex,
                2.0 * d1.max(d2),
                tol,
                &what,
                &mut tally.wrong,
                &mut tally.unclassified,
            );
            tally
                .outcomes
                .push(if tally.wrong.len() > found { 'W' } else { 'B' });
        }
        Err(e) => {
            tally.refused += 1;
            tally.outcomes.push('R');
            tally.measures.push_str("--------");
            println!("{what}: refused: {e}");
        }
    }
    Some(())
}

/// The block 40 x 30 x 20 in `frame`, and a second body of `kind` placed
/// at random so that it crosses the block's top, united with it or taken
/// out of it. `None` where the boolean refuses (a refusal is not a blend's
/// to judge).
fn case(
    rng: &mut Rng,
    frame: &Frame,
    kind: Kind,
    union: bool,
    tol: &Tolerance,
    keep_wall: Option<&mut Option<Wall>>,
) -> Option<Body> {
    let block = primitives::block(frame, 40.0, 30.0, 20.0).unwrap();
    let other = match kind {
        Kind::Lying => {
            let r = rng.between(3.0, 9.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let axis = Frame::new(frame.point(-1.0, y, z), frame.x(), frame.y()).unwrap();
            primitives::cylinder(&axis, r, 42.0).unwrap()
        }
        Kind::Standing => {
            let r = rng.between(2.0, 7.0);
            let (x, y) = (rng.between(12.0, 28.0), rng.between(10.0, 20.0));
            let base = frame.point(x, y, rng.between(5.0, 15.0));
            let axis = Frame::new(base, frame.z(), frame.x()).unwrap();
            primitives::cylinder(&axis, r, 20.0).unwrap()
        }
        Kind::Cone => {
            let r = rng.between(4.0, 9.0);
            let (x, y) = (rng.between(12.0, 28.0), rng.between(10.0, 20.0));
            let base = frame.point(x, y, rng.between(8.0, 16.0));
            let axis = Frame::new(base, frame.z(), frame.x()).unwrap();
            primitives::cone(&axis, r, rng.between(8.0, 20.0)).unwrap()
        }
        Kind::Ball => {
            let r = rng.between(3.0, 9.0);
            let (x, y) = (rng.between(12.0, 28.0), rng.between(10.0, 20.0));
            let centre = frame.point(x, y, 20.0 + rng.between(-0.8, 0.8) * r);
            primitives::sphere(&Frame::new(centre, frame.z(), frame.x()).unwrap(), r).unwrap()
        }
        Kind::Ring => {
            let (major, minor) = (rng.between(5.0, 9.0), rng.between(1.5, 3.5));
            let (x, y) = (rng.between(15.0, 25.0), rng.between(12.0, 18.0));
            let centre = frame.point(x, y, 20.0 + rng.between(-0.7, 0.7) * minor);
            primitives::torus(
                &Frame::new(centre, frame.z(), frame.x()).unwrap(),
                major,
                minor,
            )
            .unwrap()
        }
        Kind::Hump => {
            // A cubic Bézier from (a, 20) up and over to (b, 20) across y,
            // its inner points at random heights and leaning out over the
            // top.
            let (a, b) = (rng.between(18.0, 24.0), rng.between(6.0, 12.0));
            let h = rng.between(4.0, 12.0);
            let lean = rng.between(-2.0, 6.0);
            let control = vec![
                [a, 20.0],
                [a + lean, 20.0 + h],
                [b - lean, 20.0 + h],
                [b, 20.0],
            ];
            return ridge(
                frame,
                tol,
                (a, b),
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control,
                union,
            );
        }
        Kind::Ogee => {
            // A clamped cubic B-spline from (a, 20) out over the top, back
            // (its nose), up and over to (b, 20).
            let (a, b) = (rng.between(18.0, 24.0), rng.between(6.0, 12.0));
            let (out, nose, h) = (
                rng.between(1.5, 4.0),
                rng.between(3.0, 6.0),
                rng.between(10.0, 16.0),
            );
            let control = vec![
                [a, 20.0],
                [a + out, 21.0 + 0.3 * nose],
                [a + out, 20.0 + nose],
                [a - 2.0, 21.0 + nose],
                [a - 1.0, 20.0 + h],
                [b, 20.0 + h],
                [b, 20.0],
            ];
            return ridge(
                frame,
                tol,
                (a, b),
                vec![0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0],
                control,
                union,
            );
        }
        Kind::LyingCone => {
            let r = rng.between(5.0, 9.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let axis = Frame::new(frame.point(-1.0, y, z), frame.x(), frame.y()).unwrap();
            primitives::cone(&axis, r, rng.between(70.0, 200.0)).unwrap()
        }
        Kind::Bead | Kind::Hollow | Kind::BeadSlope | Kind::HollowSlope => {
            // A clamped cubic spline from (a, 20) up the wall at the
            // angle `slope` from the top, a bead (or a hollow) lifting
            // its second and third inner points off the wall, then over
            // to (b, 20).
            let (a, b) = (rng.between(18.0, 24.0), rng.between(6.0, 10.0));
            let slope = rng.between(35.0, 80.0).to_radians();
            let wall = [-slope.pcos(), slope.psin()];
            let out = [slope.psin(), slope.pcos()];
            let w = rng.between(1.5, 4.0);
            let bead = matches!(kind, Kind::Bead | Kind::BeadSlope);
            let lift = rng.between(0.1, 0.35) * w * if bead { 1.0 } else { -1.0 };
            let h = rng.between(10.0, 16.0);
            let at = |along: f64, off: f64| {
                [
                    a + wall[0] * along * w + out[0] * off,
                    20.0 + wall[1] * along * w + out[1] * off,
                ]
            };
            let top = at(1.6, 0.0);
            let control = vec![
                [a, 20.0],
                at(0.15, 0.0),
                at(0.35, lift),
                at(0.55, lift),
                at(0.75, 0.0),
                at(0.95, 0.0),
                top,
                [top[0] - 1.0, 20.0 + h],
                [b, 20.0 + h],
                [b, 20.0],
            ];
            let knots = vec![
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / 7.0,
                2.0 / 7.0,
                3.0 / 7.0,
                4.0 / 7.0,
                5.0 / 7.0,
                6.0 / 7.0,
                1.0,
                1.0,
                1.0,
                1.0,
            ];
            if let Some(keep) = keep_wall {
                *keep = Some(Wall {
                    control: control
                        .iter()
                        .map(|&[y, z]| if union { [y, z] } else { [y, 40.0 - z] })
                        .collect(),
                    knots: knots.clone(),
                    a,
                    w,
                });
            }
            return ridge(frame, tol, (a, b), knots, control, union);
        }
        Kind::LyingRing => {
            let (major, minor) = (rng.between(5.0, 8.0), rng.between(1.5, 3.5));
            let (x, y) = (rng.between(15.0, 25.0), rng.between(13.0, 17.0));
            let centre = frame.point(x, y, 20.0 + rng.between(-0.6, 0.6) * major);
            primitives::torus(
                &Frame::new(centre, frame.x(), frame.y()).unwrap(),
                major,
                minor,
            )
            .unwrap()
        }
        Kind::Tilted => {
            let r = rng.between(3.0, 8.0);
            let (y, z) = (rng.between(10.0, 20.0), 20.0 + rng.between(-0.8, 0.8) * r);
            let direction = frame.x()
                + frame.y() * rng.between(-0.3, 0.3)
                + frame.z() * rng.between(-0.15, 0.15);
            let axis = Frame::new(
                frame.point(20.0, y, z) - direction * 30.0,
                direction,
                frame.z(),
            )
            .unwrap();
            primitives::cylinder(&axis, r, 60.0 * direction.length()).unwrap()
        }
    };
    let op = if union {
        BooleanOp::Union
    } else {
        BooleanOp::Subtract
    };
    let made = boolean(&block, &other, op, tol).ok()?.body;
    check(&made, tol).is_empty().then_some(made)
}

/// How the faces of `edge` turn against each other at the parameter `t`
/// of its curve: the sine of the turn from the first face into the
/// second, negative at a convex edge and positive at a concave one (the
/// way the kernel tells them, with the faces' loops and normals).
fn turn(body: &Body, edge: EdgeId, t: f64) -> Option<f64> {
    let uses = edge_uses(body);
    let [a, b] = uses.get(edge.0 as usize)?.as_slice() else {
        return None;
    };
    if a.face == b.face {
        return None;
    }
    let data = body.edge(edge)?;
    let (point, d) = data.curve.derivatives(t);
    let along = d.normalized()?;
    let normal = |face: FaceId| {
        let f = body.face(face)?;
        let (u, v) = f.surface.project(point).ok()?;
        let n = f.surface.normal(u, v)?;
        Some(if f.reversed { -n } else { n })
    };
    let run = if a.reversed { -along } else { along };
    let into_a = normal(a.face)?.cross(run).normalized()?;
    Some(into_a.dot(normal(b.face)?))
}

/// Whether `edge` is convex all along (`Some(true)`), concave all along
/// (`Some(false)`), or neither, at nine points.
fn convexity(body: &Body, edge: EdgeId) -> Option<bool> {
    let data = body.edge(edge)?;
    let mut signs = Vec::new();
    for i in 1..10 {
        let t = data.range.0 + (data.range.1 - data.range.0) * f64::from(i) / 10.0;
        let sine = turn(body, edge, t)?;
        if sine.abs() <= 1e-3 {
            return None;
        }
        signs.push(sine < 0.0);
    }
    let first = *signs.first()?;
    signs.iter().all(|s| *s == first).then_some(first)
}

/// The distance from `p` to the edge (its curve within its range).
fn from_edge(body: &Body, edge: EdgeId, p: Vec3) -> f64 {
    let data = body.edge(edge).unwrap();
    let (t0, t1) = (
        data.range.0.min(data.range.1),
        data.range.0.max(data.range.1),
    );
    let mut best = f64::INFINITY;
    for i in 0..=64 {
        let t = t0 + (t1 - t0) * f64::from(i) / 64.0;
        best = best.min(data.curve.eval(t).distance(p));
    }
    if let Ok(t) = data.curve.project(p)
        && t >= t0
        && t <= t1
    {
        best = best.min(data.curve.eval(t).distance(p));
    }
    best
}

/// What one random blend did.
#[derive(Default, Debug)]
struct Tally {
    built: usize,
    refused: usize,
    /// Bodies whose points could not all be classified (the point checks
    /// of that body not made).
    unclassified: usize,
    /// Per finding, a line saying what and where.
    wrong: Vec<String>,
    /// Per case, what it did: `S` no body to blend (the boolean or the
    /// solid refused), `N` no edge convex or concave all along, `B` built
    /// and held, `W` built and seen wrong, `R` refused.
    outcomes: String,
    /// Per case, the volume and the area of the body built (their bits,
    /// hashed: [`measure`]), `--------` where none was.
    measures: String,
}

/// Eight hexadecimal digits standing for a volume and an area, bit for
/// bit (FNV-1a over their bits).
fn measure(volume: f64, area: f64) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in volume
        .to_bits()
        .to_le_bytes()
        .into_iter()
        .chain(area.to_bits().to_le_bytes())
    {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The outward normal of `face` of `body` at the point of it nearest `p`.
fn normal_of(body: &Body, face: FaceId, p: Vec3) -> Option<Vec3> {
    let f = body.face(face)?;
    let (u, v) = f.surface.project(p).ok()?;
    let n = f.surface.normal(u, v)?;
    Some(if f.reversed { -n } else { n })
}

/// Holds the body `after` a blend of `edge` (`convex` or not, reaching
/// `reach` from it; `blends` says per face of `after` whether it is a
/// blend face) against the body `before`; adds what is wrong.
#[allow(clippy::too_many_arguments)]
fn judge(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    blends: &[bool],
    edge: EdgeId,
    convex: bool,
    reach: f64,
    tol: &Tolerance,
    what: &str,
    wrong: &mut Vec<String>,
    unclassified: &mut usize,
) {
    let defects = check(after, tol);
    if !defects.is_empty() {
        wrong.push(format!("{what}: not valid: {defects:?}"));
        return;
    }
    let (Ok(v0), Ok(v1)) = (
        craft3d_topo::mass_properties(before),
        craft3d_topo::mass_properties(after),
    ) else {
        wrong.push(format!("{what}: cannot be measured"));
        return;
    };
    let slack = 1e-9 * v0.volume.abs() + 4.0 * tol.linear * v0.area;
    if convex && v1.volume > v0.volume + slack {
        wrong.push(format!(
            "{what}: a convex edge gained {}",
            v1.volume - v0.volume
        ));
    }
    if !convex && v1.volume < v0.volume - slack {
        wrong.push(format!(
            "{what}: a concave edge lost {}",
            v0.volume - v1.volume
        ));
    }
    // Points near the edge, and anywhere in the box round both.
    let data = before.edge(edge).unwrap();
    let mut points = Vec::new();
    for _ in 0..160 {
        let t = data.range.0 + (data.range.1 - data.range.0) * rng.unit();
        let c = data.curve.eval(t);
        let offset = Vec3::new(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        ) * (1.5 * reach);
        points.push(c + offset);
    }
    let (lo, hi) = bounds(before);
    for _ in 0..80 {
        points.push(Vec3::new(
            rng.between(lo.x, hi.x),
            rng.between(lo.y, hi.y),
            rng.between(lo.z, hi.z),
        ));
    }
    // Where a blend face goes wrong first: next to its borders. Points a
    // little way into the blend face from its edges with the faces beside
    // it, just off it on the side it does not change (outside at a concave
    // edge, inside at a convex one): such a point is outside (inside)
    // after, and has to have been so before.
    let off = 20.0 * tol.linear;
    let mut near = Vec::new();
    for (index, users) in edge_uses(after).iter().enumerate() {
        let [x, y] = users.as_slice() else { continue };
        for (own, other) in [(x, y), (y, x)] {
            let (Some(true), Some(false)) = (
                blends.get(own.face.0 as usize),
                blends.get(other.face.0 as usize),
            ) else {
                continue;
            };
            let border = after.edge(EdgeId(index as u32)).unwrap();
            let face = after.face(own.face).unwrap();
            // At 31 points along each border: a lens near an end of the
            // edge is short.
            for i in 1..32 {
                let t = border.range.0 + (border.range.1 - border.range.0) * f64::from(i) / 32.0;
                let (g, d) = border.curve.derivatives(t);
                let (Some(along), Some(n)) = (d.normalized(), normal_of(after, own.face, g)) else {
                    continue;
                };
                let run = if own.reversed { -along } else { along };
                let Some(into) = n.cross(run).normalized() else {
                    continue;
                };
                // A face that curves tightly takes back what a line that
                // enters it cuts off within a short way: several ways in.
                for step in [0.005, 0.02, 0.05] {
                    let Ok((u, v)) = face.surface.project(g + into * (step * reach)) else {
                        continue;
                    };
                    let q = face.surface.eval(u, v);
                    let Some(nq) = normal_of(after, own.face, q) else {
                        continue;
                    };
                    near.push(if convex { q - nq * off } else { q + nq * off });
                }
            }
        }
    }
    let near_was = classify_points(before, &near, tol);
    if near_was.is_err() {
        *unclassified += 1;
    }
    if let Ok(was) = near_was {
        for (p, place) in near.iter().zip(&was) {
            let bad = if convex {
                *place == Place::Outside
            } else {
                *place == Place::Inside
            };
            if bad {
                wrong.push(format!(
                    "{what}: next to the blend face, {:.4} from the edge, {} the body",
                    from_edge(before, edge, *p),
                    if convex { "added to" } else { "taken from" }
                ));
                break;
            }
        }
    }
    let (Ok(was), Ok(is)) = (
        classify_points(before, &points, tol),
        classify_points(after, &points, tol),
    ) else {
        *unclassified += 1;
        return;
    };
    for ((p, a), b) in points.iter().zip(&was).zip(&is) {
        let (taken, added) = (
            *a == Place::Inside && *b == Place::Outside,
            *a == Place::Outside && *b == Place::Inside,
        );
        if !(taken || added) {
            continue;
        }
        let away = from_edge(before, edge, *p);
        if (convex && added) || (!convex && taken) {
            wrong.push(format!(
                "{what}: a point {away:.4} from the edge {} the body",
                if added { "added to" } else { "taken from" }
            ));
            return;
        }
        if away > reach + 16.0 * tol.linear {
            wrong.push(format!(
                "{what}: a point {away:.4} from the edge changed, past the reach {reach:.4}"
            ));
            return;
        }
    }
}

fn bounds(body: &Body) -> (Vec3, Vec3) {
    let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = -lo;
    for v in &body.vertices {
        lo = Vec3::new(
            lo.x.min(v.point.x),
            lo.y.min(v.point.y),
            lo.z.min(v.point.z),
        );
        hi = Vec3::new(
            hi.x.max(v.point.x),
            hi.y.max(v.point.y),
            hi.z.max(v.point.z),
        );
    }
    // Curved faces bulge past their vertices: a margin.
    let margin = (hi - lo) * 0.25;
    (lo - margin, hi + margin)
}

/// The edges between a face of the block's top and a face that is not a
/// plane, convex or concave all along.
fn candidates(body: &Body, frame: &Frame) -> Vec<(EdgeId, bool)> {
    let uses = edge_uses(body);
    let mut found = Vec::new();
    for (index, users) in uses.iter().enumerate() {
        let [a, b] = users.as_slice() else { continue };
        if a.face == b.face {
            continue;
        }
        // The block's top, and the second body's face.
        let top = |f: FaceId| match body.face(f).map(|f| &f.surface) {
            Some(craft3d_geom::Surface::Plane(plane)) => {
                plane.frame().z().cross(frame.z()).length() < 1e-9
                    && (plane.frame().origin() - frame.point(0.0, 0.0, 20.0))
                        .dot(frame.z())
                        .abs()
                        < 1e-6
            }
            _ => false,
        };
        let curved = |f: FaceId| {
            !matches!(
                body.face(f).map(|f| &f.surface),
                Some(craft3d_geom::Surface::Plane(_))
            )
        };
        if !((top(a.face) && curved(b.face)) || (top(b.face) && curved(a.face))) {
            continue;
        }
        let edge = EdgeId(index as u32);
        if let Some(convex) = convexity(body, edge) {
            found.push((edge, convex));
        }
    }
    found
}

/// Seeded random blends, from small to well past what fits: every one
/// built is held to the property. Run in the world and in a frame turned
/// and moved away from the origin.
fn sweep(seed: u64, cases: std::ops::Range<usize>) -> Tally {
    let tol = tolerance(1.0);
    let frames = [
        Frame::WORLD,
        Frame::new(
            Vec3::new(130.0, -70.0, 45.0),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 0.0, 0.2),
        )
        .unwrap(),
    ];
    let mut tally = Tally::default();
    for n in cases {
        // A seed per case, and one for its points: what one case draws
        // does not depend on how another came out.
        let at = seed ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut rng = Rng(at);
        let kind = if let Some(m) = n.checked_sub(FIRST_SLOPE) {
            SLOPE_KINDS[m % SLOPE_KINDS.len()]
        } else if let Some(m) = n.checked_sub(FIRST_NEW) {
            NEW_KINDS[m % NEW_KINDS.len()]
        } else {
            KINDS[n % KINDS.len()]
        };
        let union = rng.below(2) == 0;
        let frame = frames[rng.below(2) as usize];
        let mut wall = None;
        let keep = matches!(kind, Kind::BeadSlope | Kind::HollowSlope).then_some(&mut wall);
        let Some(body) = case(&mut rng, &frame, kind, union, &tol, keep) else {
            tally.outcomes.push('S');
            tally.measures.push_str("--------");
            continue;
        };
        if let Some(wall) = &wall {
            let what = format!(
                "case {n} ({kind:?}, {})",
                if union { "united" } else { "taken out" }
            );
            if slope(&mut rng, at, &body, wall, &frame, &tol, &what, &mut tally).is_some() {
                continue;
            }
        }
        let edges = candidates(&body, &frame);
        if edges.is_empty() {
            tally.outcomes.push('N');
            tally.measures.push_str("--------");
            continue;
        }
        let (edge, convex) = edges[rng.below(edges.len() as u64) as usize];
        let uses = edge_uses(&body);
        let faces: Vec<FaceId> = uses[edge.0 as usize].iter().map(|u| u.face).collect();
        // From small to well past what fits.
        let size = 0.2 * (1.5_f64).ppowi(rng.below(10) as i32);
        let what = format!(
            "case {n} ({kind:?}, {}, {}, edge {})",
            if union { "united" } else { "taken out" },
            if convex { "convex" } else { "concave" },
            edge.0
        );
        let made = if rng.below(2) == 0 {
            // The two distances from a twentieth to twice each other: a line
            // across the edge that runs far along one face and little along
            // the other is the one that can enter a face.
            let second = size * 10.0_f64.ppowf(rng.between(-1.3, 0.3));
            let what = format!("{what}: chamfer {size:.4} and {second:.4}");
            let result = chamfer(
                &body,
                &[ChamferEdge {
                    edge,
                    first: faces[rng.below(2) as usize],
                    size: ChamferSize::Distances {
                        first: size,
                        second,
                    },
                }],
                &tol,
            )
            .map(|m| {
                let blends = m
                    .faces
                    .iter()
                    .map(|f| matches!(f, ChamferFace::Chamfer(_)))
                    .collect::<Vec<_>>();
                (m.body, blends)
            });
            (what, size.max(second), result)
        } else {
            let what = format!("{what}: fillet {size:.4}");
            let result = fillet(&body, &[FilletEdge { edge, radius: size }], &tol).map(|m| {
                let blends = m
                    .faces
                    .iter()
                    .map(|f| matches!(f, FilletFace::Fillet(_)))
                    .collect::<Vec<_>>();
                (m.body, blends)
            });
            // The ball's feet lie within r / tan(θ/2) of the edge; for the
            // sharp corners here within four radii.
            (what, 4.0 * size, result)
        };
        let (what, largest, result) = made;
        match result {
            Ok((after, blends)) => {
                tally.built += 1;
                println!("{what}: built");
                let found = tally.wrong.len();
                let measured = craft3d_topo::mass_properties(&after)
                    .map_or_else(|_| "????????".to_string(), |m| measure(m.volume, m.area));
                tally.measures.push_str(&measured);
                // The blend's region lies within its distances of the
                // edge, measured along the faces; past the ends of the
                // edge it runs on to the third faces, as far again.
                let reach = 2.0 * largest;
                judge(
                    &mut Rng(at ^ 0x5DEE_CE66_D1CE_4E5B),
                    &body,
                    &after,
                    &blends,
                    edge,
                    convex,
                    reach,
                    &tol,
                    &what,
                    &mut tally.wrong,
                    &mut tally.unclassified,
                );
                tally
                    .outcomes
                    .push(if tally.wrong.len() > found { 'W' } else { 'B' });
            }
            Err(e) => {
                tally.refused += 1;
                tally.outcomes.push('R');
                tally.measures.push_str("--------");
                println!("{what}: refused: {e}");
            }
        }
    }
    tally
}

/// The cases `cases` of the sweep, held to the property and to what the
/// released kernel did ([`RELEASED`]). The sweep is cut in four, which run
/// side by side.
fn hold(cases: std::ops::Range<usize>) {
    let start = cases.start;
    let tally = sweep(SEED, cases);
    let count = |c: char| tally.outcomes.chars().filter(|o| *o == c).count();
    println!(
        "built {}, refused {}, wrong {}, not all points classified {}, \
         no body to blend {}, no edge convex or concave all along {}",
        tally.built,
        tally.refused,
        tally.wrong.len(),
        tally.unclassified,
        count('S'),
        count('N')
    );
    println!("outcomes from {start}: {}", tally.outcomes);
    println!("measures from {start}: {}", tally.measures);
    for line in &tally.wrong {
        println!("  {line}");
    }
    let released = RELEASED.get(start..start + tally.outcomes.len()).unwrap();
    let measured = MEASURES
        .get(8 * start..8 * (start + tally.outcomes.len()))
        .unwrap();
    let mut changed = Vec::new();
    for (i, (then, now)) in released.chars().zip(tally.outcomes.chars()).enumerate() {
        // A body built right is the same body: its volume and area bit for
        // bit.
        let (was, is) = (
            measured.get(8 * i..8 * i + 8).unwrap(),
            tally.measures.get(8 * i..8 * i + 8).unwrap(),
        );
        if then == 'B' && now == 'B' && was != is {
            changed.push(format!(
                "case {}: built by 0.1.0 and now, another volume or area",
                start + i
            ));
        }
        if then == 'W' && now == 'W' && KNOWN.contains(&(start + i)) {
            println!("case {}: built wrong with 0.1.0 and now, known", start + i);
            continue;
        }
        let fine = match then {
            // Set up the same way, or not at all.
            'S' | 'N' => now == then,
            // Built and not seen wrong: built still.
            'B' => now == 'B',
            // Refused: refused still.
            'R' => now == 'R',
            // Built wrong: refused, or built right.
            _ => now != 'S' && now != 'N',
        };
        if !fine {
            changed.push(format!("case {}: {then} with 0.1.0, {now} now", start + i));
        }
        if then != now {
            println!("case {}: {then} with 0.1.0, {now} now", start + i);
        }
    }
    assert!(changed.is_empty(), "{changed:#?}");
    assert!(tally.built >= 40, "too few built to say much: {tally:?}");
    // A body whose points cannot be classified is held to its volume
    // only: few may be.
    assert!(
        tally.unclassified * 10 <= tally.built,
        "too many bodies whose points could not be classified: {tally:?}"
    );
    let unknown: Vec<&String> = tally
        .wrong
        .iter()
        .filter(|line| {
            !KNOWN
                .iter()
                .any(|n| line.starts_with(&format!("case {n} ")))
        })
        .collect();
    assert!(unknown.is_empty(), "{unknown:#?}");
}

/// The sweep's seed. With 0.1.0's kernel 36 of its 720 cases are built
/// wrong and are seen so here: 34 chamfers (beside a hump or a groove of
/// spline 5, a slanted cylinder or its groove 7, a ring 5, a lying cone 3,
/// a ball, a lying cylinder, an ogee, a lying ring 1 each, a wall with a
/// hollow 3, with a bead and the foot on its far slope 7) and two fillets
/// (beside a bead). Since 2026-11-13 33 of the chamfers are refused, and
/// every other case builds as it did, the same body, or is refused as it
/// was ([`RELEASED`], [`MEASURES`]); the other three are [`KNOWN`].
const SEED: u64 = 0x00C4_A3F3_2026_1113;

/// Cases built wrong with 0.1.0 and still, by what the chamfer's search
/// does not look at; owed. Case 377 (a slanted cylinder united with the
/// block, the concave edge chamfered 0.45 and 0.0764): not its line
/// through a face (held in 8193 planes across the edge, no point of either
/// face's trace past the line by more than a quarter of the tolerance) but
/// its face, 3e-5 inside the cylinder 0.1 from the start of the edge, next
/// to its border on the cylinder (a point 2e-5 off the chamfer face on the
/// side it leaves empty lies 1.1e-5 inside the cylinder). Cases 585 and
/// 716 (a groove with a bead beside it, its convex edge filleted 5.1258
/// and 3.4172): fillets, which the search does not hold; a point next to
/// the fillet face (1.0844 and 2.2609 from the edge), on the side it leaves
/// solid, was outside before: the bead reaches into the ball.
const KNOWN: [usize; 3] = [377, 585, 716];

/// The volume and area of each body the released kernel built, in the
/// digits of [`Tally::measures`], ten cases to a line. Recorded from
/// release/0.1's kernel without the chamfer's search and with the measure
/// that follows edges off their faces (2026-11-14): its outcomes are
/// [`RELEASED`] for every case, and 163 of its measures differ from those
/// 0.1.0's own measure gave (150 of bodies built right, 13 of bodies built
/// wrong), the bodies being the same.
const MEASURES: &str = concat!(
    "5c16c5b5f1c44a298ad2ad18ccfcf252220083a84480b4563ca0b53e--------ff84213897125370",
    "8ddcf41541ddd2f0c51df8af030d03d8c0c910daca5f400a5bd5d39fc1088ffa9f59682aa250f4af",
    "9049a4c406a9b654460cb63ba8fc00c45d45ca48bb4d98f87a1112e6bb60d518e56e633281b3c6be",
    "454202217d616e73694f5f211e556d902ea45e5652bad9e0204f24ab146f0184ec65189f49718298",
    "05c68de629b51c548f2f911bc90b5e9efcff735b049b6b4ec30c1bdcc5f352f0e6e3dc6f--------",
    "6da14a9472e13d07c69d673816b350a860efdb358146935662d34c439a194f4ea447a91d0dbd738c",
    "e6f5ef7d6e1e53067ad6ab7d496d22c064907452fa872173--------a4dbfe90f408be2446f15aab",
    "--------7e11fcf822a862c7cf592d52634a1530--------26df72842d22ce35--------7597c1d3",
    "--------1806852260987532----------------844df24ba62db4f6df853036574d4303f31c0588",
    "28795d8170e4092f1c1df7943b737e11ca5baf02a94220df--------fd2710c1a19f750771de83a7",
    "0c89ad97f4e136b4305f0951137b5f94be8bac855d245780c1a34286374e01e57d47b9e39ed528cf",
    "df58bb309778bf954a11b693--------de3595c0712de7e6--------318d40b550b9bdc1331b2b3a",
    "48f439287429f97a--------7aeccdc39cd01c89c3c1308a55378c566c81116ac2e096016b45645f",
    "c52897e7bf121da91dd315c2--------f72d8626--------4d1084c0fec062bba83edb6c358bddb8",
    "406bd260b8b8c6a2ef7343e5f115a37bab976b1915763913862f28cf5acf498a227ef8498031e14d",
    "8bf8074d--------0fef57533673c5f007b72212b0c1b23646883801af4ed9e9346806e0c194ec4d",
    "--------6344d2ed1cd1a0e731ee0805--------948d6c35----------------d3bed9571c48e4cc",
    "e4a7fa5196325d16c52d1ea3c92efe6cfe388203e7b805a74a3ba09c----------------5a167e6c",
    "c8aad7d93b338487--------5b7aa85631337eaa768e6ebb41b4fbd7d953a6f3--------c3640ef1",
    "dfe56c99--------a9ae937fbf38c18e518577c06810bcc2--------09de15e3462915c80dedfcc7",
    "da180c27--------309b16628465ff1fde178dc75b28e137c3152dce0ffd03b5--------cb272903",
    "------------------------54f5884fe55181720a59a0e1275896fed20b961ff369a5ab5f9bedb3",
    "d521e829998cb47868a795d6e33916849ba7a9143ff042eadabc71411b793fae32584785d27b7286",
    "f023d9de167ca772d56ba17f--------a41d65c8e433614195560b161b22d7520dbef98bb4d6ce35",
    "cdf57db4fbde55e4--------c3e968db49d5c23aea663b65810803b7598f42bcf0295b1327f2a28a",
    "8a43dd64--------b6e8470fdf6d87a5a0d47c72966c41849140864e7c4576601f25f0b5ab879acb",
    "dc39d5c6479d864ef54c3da218ac033fbd70db819c181f7681c79e90ebaa80f0dd11810e--------",
    "4325ddae--------c413cbd07306e3d85d34d28b--------4b3c8ff5dd360cbdb68531175185b240",
    "66395ff8929120b054b1880c--------3a2984cd--------f5019281b56ffa7300af31fbc948e53b",
    "9a8112c53ae433d95464e189--------0959513f638d893c085a7ca262d58ac27ae9ae29d2b4cda2",
    "d30d5923df884ea368a3f92a--------12d16d2b--------4dfad1b0571c4f9a6e51d96148ad0321",
    "16af6a9f--------daa9018f2a05738857b377aeb9ad32ed3976b086e3304eb7a4a68c191b392959",
    "0e9ea1c9959912ec--------6e653d64d8fb0c4eaed26ff3a39a557e194d1f61--------452a9ed6",
    "915b225128e6c9889b1084a5908e064e1ac012db82d3efd63a9766505edd3677a4debadb3fcaf4a3",
    "--------5639d7f19f56378baec9e9c4--------311af0fd8261892606601c0d40c41d6ddbc09ec3",
    "4d4a523a3d1db97600e01c53dcc444972b9994ac75a32ddcf50642ba61fc4d71d6fee89235a2171d",
    "12593e664a03ae7a399243256e80a020a546d6254eaff2efd09cc4b7--------981a9cb134b21525",
    "8bf2b5f8----------------31c91839ac740d48bd8cf0d1ef5ad09e4ea64c7bde7719b5bf7a738b",
    "f0334be22a1401c50461abad4e8150d9b7faf68f6d1779f5010c96ce4c532e1167d6c153c608a5d0",
    "b8a8faa22733446b1438f039dd3cf2e0d06ccfd1827d70c02650df76294d536756b4257884e3f8bb",
    "2c08456b792aae2c9e9c5948724567d1--------15908ae924176fb6696163ca34039b1983e80a85",
    "8c46ab3397e27be4fb5913010778f5668afcb49e1a3e6c464c9a80b4d9cd8d21c47fadff0e660251",
    "e2175b29------------------------44770e844e3c2e4e--------1cbd96f7a6785dea--------",
    "60d542d7d93ef5da90d12092f92f93a7711d3f0f--------e0b5e955--------f9b9d0ba04ec691c",
    "--------1ba380019e31a592a6c57cd1694e90c303aac648134e25473e0c8308----------------",
    "4db7ee4feaf912fb--------60ada9d8058f7340a0ddd051--------c3330aa03ad793718ce0dc73",
    "98a16b545ad077966515ce8c1f59000ada9c13b0b1f959af28125a280a0b5916b3c271c0cf66a52f",
    "7b84f1ff315afd0335a2dc7b--------43407156a7db5cad--------0919f9975d5fd52e8f406097",
    "7fdaf3bb129bfeeb--------36ee325a78170d36--------e3d6eb62--------88b1504ce50cfcbb",
    "02734d76d1276a9970e2a6b11cebde25--------cae8bacc--------fc3e9f625198e08faf6c49af",
    "aca2b7a5dfe909f2263a245d088f5a10aec7f3059ff1957890ead05dc154112fbf474075555ca865",
    "c541993a03b7f6bc--------ad418eada816236840a026bd218ff9f5fc4ad74259b6aa08409a655c",
    "47189775--------7a4c888f61ba8986a2098771b62350581a72b437c82e30ce6f401600--------",
    "--------e37ffc2c1544058d--------ef33ecac4153ec11--------08ba7ee550a4a729--------",
    "ca2922dbfae56d7e----------------80f7784cf2f047bc5cf3ed1473eafec7--------ba1002f7",
    "71234e3628f6bab87c55351145e7d4c62c96dde1243126fc62572b60213c2d665d9ce0c7ae3b568f",
    "057e467d7250313fe79dc41c623bf78d2b1cf46f23df4d584f87d710--------2e1d8950--------",
    "7b2c4009----------------2ea14c88a9f10ee42415e9d6ab137d99d2c3b63f--------e88fac05",
    "e056caf9--------511c7703a4634bb46620b58b5898241d5fa4d135--------04f6ba0e660398a0",
    "--------aa7fba5aa500f1e6--------d9953dd3cbfcf321--------def6e56e94ea1056--------",
    "226b3afb606094cf415b5b76110290abc4bbfb882507e61b3ab27b946afdda2968ba0176557ce1ec",
    "c157e55b5aba611e90bf94e502d678ce162c3225222eb80ca527f11dc19f3511862a9e6c6b101d36",
    "03741ea324d35129a873459b--------d8e427fe63b15bbb1812231d--------19084fce88b6d4dd",
    "84c208800a4aaf779eaf16bb--------faec91674165dffc7ee2757d7c4e66471a65af30c205c7b0",
    "f458d8d8f04f0e67f9a9a8769bcc1ba365baec0a--------326c46d02b6d0604081bf2b08ae5a2a0",
    "2009604d--------c7f919349dda87ab2b406d008a7d44f07ce7a074647c192c76d1b411cef47372",
    "a74f146b0faa4ab5--------9bc314567c43ac3103e6b5c8--------b2fe723e0cc24008c9212974",
    "8aaa7cb23600c66cbc11566c569aa14e55c0bc6a8878e15a7a9fcf261302bc95bbb1b12e7c1402c9",
    "8a2b142593b1f5e7--------c0149e50--------e4df38829ad73714ab533ca1bf65664f--------",
    "311bc406cbff3f5dc07950ff4eebcf4241c060295c1da02952296482a282575f24f8616c2d338ff8",
    "aac10d976f31ee999943ec2a71a1ed2b905908ac166f14a8365a44cba7839e0c4d9318389c741d71",
    "ba52561d6a16ec47abbd465cdbf8cc4c--------de7736992ea856f150e5cd2d705f2c239031c344",
);

/// What the released kernel (0.1.0's blends) did with each case of the
/// sweep, in the letters of [`Tally::outcomes`], 80 to a line.
const RELEASED: &str = concat!(
    "BBBBBBBRBBBBBBBBBBBBBBBWBBWBBBBBBBBBBBBBBBBWBBBBBRBBBBBBBBWWBWBBBBRWWBRBBBBRBBRB",
    "RBBRRBBBBBBBBBWBRBBBBBBBBBBWBBBBBRBBRBBBBBRBBBBBBBBBBRBRBBBBBBBWBBBBBBBRBBBBBBBB",
    "RBBBRBRRBBBBBBWBBRRBBBRBWBBBRWBRBWBBRWBBBRBBBBBBRBRRRBBBBBBBBBBBBBBBBBBBBRBBBBBB",
    "BBRBBBBBBBBRBBBBBBBBBBBBBBBBBRBRBBBRBBBBBBBRBRBBBBBBBRBBBBBBBBBRBRBBBBBRBBBBBBBB",
    "BWRBBBBBRBBBBBBBBBBBRBBBRBBBBBBBBBBBBBBBBBBBBWBRWBBRRBBBBWBBBBBBBBBBBBBBBBBWBBBB",
    "BBBBRBBBBBBBBBBBBBBBBRRRBBRBBRBBBBBRBRBWRBBBBBBBRRBBRBBBRBBBBBBBWBBBBBBBBRBBRBBB",
    "BBRBBRBRBBBBBBRBRBBBBBBBBBBBBBBBRBBBBBBBBRBWBBBWBRRBBRBBRBBRBBRRBBBBRBBBBBBBBBBB",
    "BBBBBBBRBRBRRBBBBBRBBRBBBWBRBBRBBRBBRBBRBBBBBBBBBBWBBBBBBBBBBBWRBBBRBBBBBRBBBBWB",
    "BBBBBRBBWBWRWBBBBBBBBBRBBBRBBBBBBWBBBBBBBBRBRBBBBRBBBBBBBBBBBBBBBBBBBBBBBBRBWBWW",
);

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_0() {
    hold(0..120);
}

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_1() {
    hold(120..240);
}

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_2() {
    hold(240..360);
}

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_4() {
    hold(FIRST_NEW..FIRST_NEW + 120);
}

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_5() {
    hold(FIRST_SLOPE..FIRST_SLOPE + 120);
}

#[test]
fn random_blends_fill_concave_corners_and_cut_convex_ones_only_near_the_edge_3() {
    hold(360..480);
}
