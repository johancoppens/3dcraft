//! Inside or outside on any analytic surface, as a winding number.
//!
//! A face is a region of its surface bounded by closed loops. On a plane
//! the winding number of the loops about a point says whether the point is
//! in the region, with no special cases. The other surfaces are not planes,
//! but each is mapped onto one by a *chart*, computed from the points in
//! space rather than from the surface parameters, so that neither a seam
//! nor a pole is a special place:
//!
//! | Surface | Chart | What is missing from the plane |
//! |---|---|---|
//! | plane | its own coordinates | nothing |
//! | cylinder | angle and `exp(height / radius)` as polar coordinates (conformal) | the two ends, which no bounded face reaches |
//! | cone | dropped along its axis onto the plane across it | nothing: the apex is an ordinary point |
//! | sphere | stereographic projection from a point chosen clear of the loops | that point, which a face may contain |
//! | torus | cut open along a circle that the loops do not cross, then one angle and the other as radius | the cut circle, which a face may contain |
//! | Bézier, NURBS | its own parameters, through the nearest point; where a border of the patch is one point (a pole) the parameter rectangle drawn as a wedge or a lens | nothing: a pole is an ordinary point |
//! | Bézier, NURBS that close on themselves | the way around as an angle: an annulus, a disc, a ball through an inversion, a torus cut open (see [`Layout`]) | a point of a ball clear of the loops, the cut of a torus |
//!
//! A loop that goes around a cylinder's axis, a cone's apex or a torus'
//! tube simply goes around the origin of its chart.
//!
//! The charts are never evaluated as coordinates of a far point. What the
//! winding number needs is the direction from the image of `q` to the image
//! of `p`, up to a positive factor, and that has a bounded form on a
//! cylinder (no exponential) and on a sphere (no division), even where the
//! images themselves run off to infinity.
//!
//! A face on a sphere or a torus may contain the missing point or circle.
//! Its loops then wind about the points *outside* it (see
//! `Outline::infinite`).

use super::Reason;
use super::freeform::{IRREGULAR, PatchTree};
use super::hits::speed_bound;
use super::outline::{Seg, Sheet};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Vec3};
use std::f64::consts::{PI, TAU};
use std::sync::Arc;

/// The work bound of the operation this is part of was reached
/// ([`craft3d_geom::allowance`]).
pub(super) const SPENT: Reason = "the work bound of the operation it is part of was reached";
pub(super) const TOO_CLOSE: Reason =
    "a point too close to a face boundary to tell inside from outside (at the edge of tolerance)";
const NO_POLE: Reason =
    "a face on a sphere whose boundary comes close to every point tried as a centre of projection";
const NO_CUT: Reason = "a face on a torus whose boundary crosses every circle around its axis and every circle around its tube";
const CUT_CROSSED: Reason = "a face on a torus whose boundary runs through the cut its chart was opened along; this is a bug";
/// How much further than the chart's own bound a stretch's image may move
/// before it is read as a jump across the cut: the bound is exact on the
/// torus, and a boundary curve fitted within the tolerance of it moves
/// faster by at most the tolerance over the tube's radius.
const CUT_SLACK: f64 = 1e-2;

/// The most evaluations the winding sum spends on one curve piece.
const MAX_TURN_STEPS: usize = 1 << 16;
/// Samples per boundary piece when looking for a circle of a torus that the
/// boundary does not cross.
const CUT_SAMPLES: u32 = 16;

#[derive(Clone, Debug)]
pub(super) enum Chart {
    Plane(Frame),
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    Cone(Frame),
    Sphere {
        center: Vec3,
        radius: f64,
        /// The centre of projection, as a unit vector from the centre, and
        /// two axes across it.
        pole: Vec3,
        x: Vec3,
        y: Vec3,
    },
    /// A torus cut along the circle around its tube at angle `cut` about
    /// the axis: the angle around the tube is the chart's angle.
    Tube {
        frame: Frame,
        major: f64,
        minor: f64,
        cut: f64,
    },
    /// A torus cut along the circle around its axis at angle `cut` around
    /// the tube: the angle about the axis is the chart's angle.
    Ring {
        frame: Frame,
        major: f64,
        minor: f64,
        cut: f64,
    },
    /// A freeform surface: its own parameters are the chart, found by the
    /// nearest point through its index (drawn as a wedge or a lens where
    /// a border of the surface is one point, so that a pole is an
    /// ordinary point of the chart: `PatchTree::chart`). `speed` bounds
    /// how fast the chart moves per length travelled on the surface.
    Patch {
        tree: Arc<PatchTree>,
        speed: f64,
        /// For a surface that closes on itself: whether it closes in u
        /// (the share `s` around is u's), and how it is drawn.
        around_u: bool,
        layout: Layout,
    },
}

/// How the chart of a freeform surface is drawn, in the shares `s` of the
/// way around and `t` of the way across of a surface that closes on
/// itself (`freeform::Closure`). The parameter rectangle is no chart of
/// such a surface: a loop that goes over the seam is not closed in it, and
/// a point on the seam is in it twice. So the way around is drawn as an
/// angle, `θ = 2π s`, which closes where the surface does:
///
/// | Surface | Chart | What is missing |
/// |---|---|---|
/// | closed around (a tube: a vase, a NURBS cylinder) | `(1 + t) e^{iθ}`, an annulus | nothing: the ends are borders |
/// | closed around, a pole at one end (a cup) | `t e^{iθ}` or `(1 − t) e^{iθ}`, a disc with the pole at its centre | nothing |
/// | closed around, a pole at each end (an egg) | `z = t e^{iθ} / (1 − t)` (the plane, one pole at the origin and the other at infinity), then `1 / (z − z₀)`: a point `z₀` clear of the loops goes to infinity, both poles are ordinary points | that point, which a face may contain |
/// | closed both ways (a torus) | cut open along a line of constant `t` that the loops do not cross (or of constant `s`, the two swapped), `(1 + (t − cut mod 1)) e^{iθ}` | the cut, which a face may contain |
///
/// Each map is one to one and continuous on what it charts, so the
/// winding number of the loops is what it is on the surface; how fast it
/// moves is bounded through the tree's bound on the Jacobian (see
/// [`Chart::new`]).
#[derive(Clone, Copy, Debug)]
pub(super) enum Layout {
    /// The surface does not close: the tree's own chart
    /// ([`PatchTree::chart`]).
    Open,
    Annulus,
    /// The pole at the end of `t` when `high`, else at its start; drawn
    /// with the radius `ω = t^order` (or `(1 − t)^order`), the width of
    /// the wedge at a pole of that order (`freeform::pole_order`).
    Disc {
        high: bool,
        order: i32,
    },
    /// The point sent to infinity, as `(a₀, b₀)` with `a = t e^{iθ}` and
    /// `b = 1 − t`: `(re a₀, im a₀, b₀)`.
    Sphere {
        drop: Vec3,
    },
    /// `s` and `t` are swapped when `swapped` (cut along a line of
    /// constant `s`).
    Torus {
        cut: f64,
        swapped: bool,
    },
}

const NO_SEAM_CUT: Reason = "a face on a freeform surface that closes on itself both ways, whose boundary crosses every line of constant parameter both ways";
const NO_DROP: Reason = "a face on a freeform surface that closes on itself with a pole at each end, whose boundary comes close to every point tried as the point its chart leaves out";

/// The share `x` taken around once: in `[0, 1)`.
fn wrap_share(x: f64) -> f64 {
    let w = x.rem_euclid(1.0);
    if w >= 1.0 { 0.0 } else { w }
}

/// `a conj(b)` of complex numbers as pairs.
fn times_conj(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 + a.1 * b.1, a.1 * b.0 - a.0 * b.1)
}

/// `a = t e^{iθ}` and `b = 1 − t` of the shares `(s, t)`: the point of
/// the sphere chart in homogeneous form, `z = a / b`.
fn homogeneous((s, t): (f64, f64)) -> ((f64, f64), f64) {
    let (sin, cos) = (TAU * s).sin_cos();
    ((t * cos, t * sin), 1.0 - t)
}

/// `a b₀ − a₀ b` for the dropped point `(a₀, b₀)`: zero only at it.
fn from_drop(a: (f64, f64), b: f64, drop: Vec3) -> (f64, f64) {
    (a.0 * drop.z - drop.x * b, a.1 * drop.z - drop.y * b)
}

/// What the loops are wound about.
#[derive(Clone, Copy, Debug)]
pub(super) enum About {
    /// A point of the surface.
    Point(Vec3),
    /// The inner end of a cut torus: the origin of its chart.
    InnerEnd,
}

/// A point to wind about, prepared.
struct Target {
    /// Chart coordinates of the point, where the chart has them.
    at: (f64, f64),
    /// The point itself (cylinder: in the frame; sphere: as a unit vector
    /// less the pole).
    q: Vec3,
    /// How far the direction moves per unit of distance of the point it
    /// looks at.
    lipschitz: f64,
}

fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The angle from direction `a` to direction `b`, in `(-π, π]`.
fn angle(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 * b.1 - a.1 * b.0).atan2(a.0 * b.0 + a.1 * b.1)
}

/// The angles about the axis and around the tube of a point of a torus.
fn torus_angles(frame: &Frame, major: f64, p: Vec3) -> (f64, f64) {
    let (x, y, z) = frame.local(p);
    (y.atan2(x), z.atan2(x.hypot(y) - major))
}

/// The middle of the widest stretch of the circle that none of the
/// stretches `(start, length)` covers, and its width.
fn widest_gap(covered: Vec<(f64, f64)>) -> Option<(f64, f64)> {
    if covered
        .iter()
        .any(|&(_, length)| length.is_nan() || length >= TAU)
    {
        return None;
    }
    if covered.is_empty() {
        return Some((PI, TAU));
    }
    // Every stretch as pieces of the circle's one turn `[0, 2π)`: a
    // stretch that passes `2π` goes on from `0`, and covers the start of
    // the walk below as much as its end (a stretch that started past the
    // first one and ran on past `2π` was not counted at the start, and
    // the cut was put where it ran: the fuzzer's finding of 2026-10-20).
    let mut pieces = Vec::with_capacity(covered.len() + 1);
    for (start, length) in covered {
        let start = wrap(start);
        if start + length > TAU {
            pieces.push((start, TAU - start));
            pieces.push((0.0, start + length - TAU));
        } else {
            pieces.push((start, length));
        }
    }
    pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Walk once around, starting where the first piece starts; the gap
    // from where the last piece ends to the first one again is the one
    // over `2π`.
    let first = pieces.first().map_or(0.0, |c| c.0);
    let mut reach = first;
    let mut best: Option<(f64, f64)> = None;
    let mut consider = |from: f64, to: f64| {
        let width = to - from;
        if width > 0.0 && best.is_none_or(|(_, widest)| width > widest) {
            best = Some((wrap(from + 0.5 * width), width));
        }
    };
    for &(start, length) in &pieces {
        consider(reach, start);
        reach = reach.max(start + length);
    }
    consider(reach, first + TAU);
    best
}

/// Per boundary piece of a face on a freeform surface that closes on
/// itself both ways, the stretch of the way around (`s`) and of the way
/// across (`t`) it covers, as angles `(start, length)` for
/// [`widest_gap`]: followed from sample to sample through the nearest
/// point, and widened by as much as the piece can stray between two
/// samples (`k` bounds how fast the shares move per length travelled).
#[allow(clippy::type_complexity)]
fn patch_cover(
    tree: &PatchTree,
    around_u: bool,
    loops: &[Vec<Seg>],
    k: f64,
) -> Result<(Vec<(f64, f64)>, Vec<(f64, f64)>), Reason> {
    let shares = |p: Vec3| -> Result<(f64, f64), Reason> {
        let near = tree.nearest(p).ok_or(NO_SEAM_CUT)?;
        Ok(tree.shares(around_u, near.u, near.v))
    };
    // The step from one share to the next, the short way round.
    let step_of = |from: f64, to: f64| (to - from + 0.5).rem_euclid(1.0) - 0.5;
    let (mut around, mut across) = (Vec::new(), Vec::new());
    for seg in loops.iter().flatten() {
        let speed = speed_bound(&seg.curve);
        let width = seg.range.1 - seg.range.0;
        // How far the shares can move along the piece, and samples enough
        // for a hundredth of that between two.
        let reach = k * speed * width;
        let samples = if reach.is_finite() && reach > 0.0 {
            (reach / 0.01).ceil().clamp(16.0, 4096.0)
        } else {
            16.0
        };
        let step = width / samples;
        let stray = 0.5 * k * speed * step;
        // A stray of a quarter turn could hide a step the wrong way round.
        if !(stray.is_finite() && stray < 0.25) {
            return Err(NO_SEAM_CUT);
        }
        let start = shares(seg.curve.eval(seg.range.0))?;
        let (mut s, mut t) = (0.0_f64, 0.0_f64);
        let (mut lo, mut hi) = ((0.0_f64, 0.0_f64), (0.0_f64, 0.0_f64));
        let mut previous = start;
        for i in 1..=(samples as u32) {
            let here = shares(seg.curve.eval(seg.range.0 + step * f64::from(i)))?;
            s += step_of(previous.0, here.0);
            t += step_of(previous.1, here.1);
            lo = (lo.0.min(s), lo.1.min(t));
            hi = (hi.0.max(s), hi.1.max(t));
            previous = here;
        }
        if ![s, t, start.0, start.1].iter().all(|x| x.is_finite()) {
            return Err(NO_SEAM_CUT);
        }
        around.push((
            wrap(TAU * (start.0 + lo.0 - stray)),
            TAU * (hi.0 - lo.0 + 2.0 * stray),
        ));
        across.push((
            wrap(TAU * (start.1 + lo.1 - stray)),
            TAU * (hi.1 - lo.1 + 2.0 * stray),
        ));
    }
    Ok((around, across))
}

impl Chart {
    /// A chart of `surface` for telling the inside of `loops`.
    pub(super) fn new(sheet: Sheet, loops: &[Vec<Seg>]) -> Result<Chart, Reason> {
        let surface = sheet.surface;
        Ok(match surface {
            Surface::Plane(s) => Chart::Plane(*s.frame()),
            Surface::Cylinder(s) => Chart::Cylinder {
                frame: *s.frame(),
                radius: s.radius(),
            },
            Surface::Cone(s) => Chart::Cone(*s.frame()),
            Surface::Sphere(s) => {
                let frame = s.frame();
                let (center, radius) = (frame.origin(), s.radius());
                // The candidate furthest from the boundary: the axes, then
                // the diagonals.
                let mut best: Option<(f64, Vec3)> = None;
                let signs = [-1.0, 1.0];
                let mut candidates = Vec::with_capacity(14);
                for s in signs {
                    candidates.push(frame.z() * s);
                    candidates.push(frame.x() * s);
                    candidates.push(frame.y() * s);
                }
                for a in signs {
                    for b in signs {
                        for c in signs {
                            candidates.push(frame.vector(a, b, c) * (1.0 / 3.0_f64.sqrt()));
                        }
                    }
                }
                for pole in candidates {
                    let clear = super::outline::clearance(loops, center + pole * radius);
                    if best.is_none_or(|(most, _)| clear > most) {
                        best = Some((clear, pole));
                    }
                }
                let Some((clear, pole)) = best else {
                    return Err(NO_POLE);
                };
                if clear.is_nan() || clear < 1e-3 * radius {
                    return Err(NO_POLE);
                }
                let across = Frame::from_z(center, pole).ok_or(NO_POLE)?;
                Chart::Sphere {
                    center,
                    radius,
                    pole,
                    x: across.x(),
                    y: across.y(),
                }
            }
            Surface::Torus(s) => {
                let frame = *s.frame();
                let (major, minor) = s.radii();
                // Per boundary piece, the stretch of each angle it covers:
                // followed from sample to sample, and widened by as much
                // as the curve can stray between two samples.
                let mut about_axis = Vec::new();
                let mut around_tube = Vec::new();
                for seg in loops.iter().flatten() {
                    // Samples enough that a chord between two is a small
                    // fraction of the tube's radius: the stretch of angle a
                    // piece may cover between samples is then small too.
                    // (With a fixed count, a circle around the axis strayed
                    // by radians around the tube and covered every cut.)
                    let length = speed_bound(&seg.curve) * (seg.range.1 - seg.range.0);
                    let samples = if length.is_finite() && length > 0.0 {
                        (length / (0.05 * minor))
                            .ceil()
                            .clamp(f64::from(CUT_SAMPLES), 4096.0)
                    } else {
                        f64::from(CUT_SAMPLES)
                    };
                    let step = (seg.range.1 - seg.range.0) / samples;
                    // Half a step of travel, as an angle: at most a quarter
                    // turn's worth of arc per chord.
                    let stray = 0.25 * PI * speed_bound(&seg.curve) * step;
                    let margins = (stray / (major - minor), stray / minor);
                    let (start_u, start_v) =
                        torus_angles(&frame, major, seg.curve.eval(seg.range.0));
                    let (mut u, mut v) = (0.0_f64, 0.0_f64);
                    let (mut lo, mut hi) = ((0.0_f64, 0.0_f64), (0.0_f64, 0.0_f64));
                    let mut previous = (start_u, start_v);
                    for k in 1..=(samples as u32) {
                        let p = seg.curve.eval(seg.range.0 + step * f64::from(k));
                        let here = torus_angles(&frame, major, p);
                        u += (here.0 - previous.0 + PI).rem_euclid(TAU) - PI;
                        v += (here.1 - previous.1 + PI).rem_euclid(TAU) - PI;
                        lo = (lo.0.min(u), lo.1.min(v));
                        hi = (hi.0.max(u), hi.1.max(v));
                        previous = here;
                    }
                    if ![u, v, margins.0, margins.1].iter().all(|x| x.is_finite()) {
                        return Err(NO_CUT);
                    }
                    about_axis.push((
                        wrap(start_u + lo.0 - margins.0),
                        hi.0 - lo.0 + 2.0 * margins.0,
                    ));
                    around_tube.push((
                        wrap(start_v + lo.1 - margins.1),
                        hi.1 - lo.1 + 2.0 * margins.1,
                    ));
                }
                const NARROWEST: f64 = 1e-6;
                match (widest_gap(about_axis), widest_gap(around_tube)) {
                    (Some((cut, width)), _) if width > NARROWEST => Chart::Tube {
                        frame,
                        major,
                        minor,
                        cut,
                    },
                    (_, Some((cut, width))) if width > NARROWEST => Chart::Ring {
                        frame,
                        major,
                        minor,
                        cut,
                    },
                    _ => return Err(NO_CUT),
                }
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => {
                // Drawn as evenly as the weights allow: windings are of
                // points in space, so any parametrisation of the same
                // point set is a chart of it, and one whose speed is the
                // same all over needs the fewest steps.
                let evened = PatchTree::evened(sheet.tree.ok_or(IRREGULAR)?);
                let tree = &evened;
                match tree.closure() {
                    None => {
                        let speed = tree.chart_speed()?;
                        Chart::Patch {
                            tree: Arc::clone(tree),
                            speed,
                            around_u: true,
                            layout: Layout::Open,
                        }
                    }
                    Some(closure) => Self::closed_patch(tree, &closure?, loops)?,
                }
            }
        })
    }

    /// The chart of a freeform surface that closes on itself (see
    /// [`Layout`]).
    ///
    /// Its speed, from the tree's bound `k` on how fast the shares move
    /// (`Closure::speed`). Annulus and torus: the image moves by
    /// `|(dr, r dθ)|` with `r ≤ 2` and `dθ = 2π ds`, at most `4π |(ds, dt)|`.
    /// Disc: with `ω = tᵐ` (or `(1 − t)ᵐ`) the wedge's width at a pole of
    /// order `m`, by `|(m tᵐ⁻¹ dt, 2π ω ds)| ≤ √(m² + 4π²) |(ω ds, dt)|`. Sphere: see
    /// [`Chart::direction`]; `√(4 + 4π²)` likewise.
    fn closed_patch(
        tree: &Arc<PatchTree>,
        closure: &super::freeform::Closure,
        loops: &[Vec<Seg>],
    ) -> Result<Chart, Reason> {
        let around_u = closure.around_u;
        let k = closure.speed;
        let (layout, factor) = if closure.both {
            let (around, across) = patch_cover(tree, around_u, loops, k)?;
            const NARROWEST: f64 = 1e-6;
            match (widest_gap(across), widest_gap(around)) {
                (Some((cut, width)), _) if width > NARROWEST => (
                    Layout::Torus {
                        cut: cut / TAU,
                        swapped: false,
                    },
                    4.0 * PI,
                ),
                (_, Some((cut, width))) if width > NARROWEST => (
                    Layout::Torus {
                        cut: cut / TAU,
                        swapped: true,
                    },
                    4.0 * PI,
                ),
                _ => return Err(NO_SEAM_CUT),
            }
        } else {
            match (closure.low, closure.high) {
                (false, false) => (Layout::Annulus, 4.0 * PI),
                (low, high) if low != high => {
                    let order = closure.order.max(1);
                    let m = f64::from(order);
                    (Layout::Disc { high, order }, (m * m + 4.0 * PI * PI).sqrt())
                }
                _ => {
                    // The point furthest from the loops: a pole, or one of
                    // twelve places between them.
                    let mut best: Option<(f64, Vec3)> = None;
                    let mut candidates = vec![(0.0, 0.0), (0.0, 1.0)];
                    for t in [0.5, 0.25, 0.75] {
                        for s in [0.0, 0.25, 0.5, 0.75] {
                            candidates.push((s, t));
                        }
                    }
                    for (s, t) in candidates {
                        let clear =
                            super::outline::clearance(loops, tree.at_shares(around_u, s, t));
                        if best.is_none_or(|(most, _)| clear > most) {
                            let (a, b) = homogeneous((s, t));
                            best = Some((clear, Vec3::new(a.0, a.1, b)));
                        }
                    }
                    let Some((clear, drop)) = best else {
                        return Err(NO_DROP);
                    };
                    if clear.is_nan() || clear < 1e-3 * tree.size() {
                        return Err(NO_DROP);
                    }
                    (Layout::Sphere { drop }, (4.0 + 4.0 * PI * PI).sqrt())
                }
            }
        };
        let speed = factor * k;
        if !(speed.is_finite() && speed > 0.0) {
            return Err(IRREGULAR);
        }
        Ok(Chart::Patch {
            tree: Arc::clone(tree),
            speed,
            around_u,
            layout,
        })
    }

    /// True for the charts that leave out a place a face may contain.
    pub(super) fn is_closed(&self) -> bool {
        matches!(
            self,
            Chart::Sphere { .. }
                | Chart::Tube { .. }
                | Chart::Ring { .. }
                | Chart::Patch {
                    layout: Layout::Sphere { .. } | Layout::Torus { .. },
                    ..
                }
        )
    }

    /// For the charts of a torus cut open along a circle of it in closed
    /// form, whose image jumps between the annulus's two ends at the cut:
    /// how fast a curve of speed `speed` can move the image's distance
    /// from the origin, `1 + (angle from the cut) / 2π`, where the chart
    /// is continuous. The angle about the axis moves at most `speed` over
    /// the least distance from the axis, the angle around the tube at
    /// most `speed` over the tube's radius; the winding sum holds every
    /// stretch to it (`CUT_SLACK`).
    fn radial_pace(&self, speed: f64) -> Option<f64> {
        match self {
            Chart::Tube { major, minor, .. } => Some(speed / (TAU * (major - minor))),
            Chart::Ring { minor, .. } => Some(speed / (TAU * minor)),
            _ => None,
        }
    }

    /// True for the charts whose origin is not a point of the surface but
    /// one end of a cut torus.
    pub(super) fn has_inner_end(&self) -> bool {
        matches!(
            self,
            Chart::Tube { .. }
                | Chart::Ring { .. }
                | Chart::Patch {
                    layout: Layout::Torus { .. },
                    ..
                }
        )
    }

    /// The shares `(s, t)` of the point of a closed freeform surface
    /// nearest to `p`.
    fn shares(&self, p: Vec3) -> Option<(f64, f64)> {
        let Chart::Patch { tree, around_u, .. } = self else {
            return None;
        };
        tree.nearest(p).map(|n| tree.shares(*around_u, n.u, n.v))
    }

    /// The image of `p` in the chart of a closed freeform surface, for the
    /// layouts that draw it in the plane itself (not the sphere's).
    fn closed_image(&self, p: Vec3) -> (f64, f64) {
        let Chart::Patch { layout, .. } = self else {
            return (f64::NAN, f64::NAN);
        };
        let Some((s, t)) = self.shares(p) else {
            return (f64::NAN, f64::NAN);
        };
        let (s, reach) = match *layout {
            Layout::Annulus => (s, 1.0 + t),
            Layout::Disc { high, order } => (s, (if high { 1.0 - t } else { t }).powi(order)),
            Layout::Torus { cut, swapped } => {
                let (s, t) = if swapped { (t, s) } else { (s, t) };
                (s, 1.0 + wrap_share(t - cut))
            }
            Layout::Open | Layout::Sphere { .. } => return (f64::NAN, f64::NAN),
        };
        let (sin, cos) = (TAU * s).sin_cos();
        (reach * cos, reach * sin)
    }

    /// Chart coordinates, for the charts that have them.
    fn coordinates(&self, p: Vec3) -> (f64, f64) {
        match self {
            Chart::Plane(frame) | Chart::Cone(frame) => {
                let (x, y, _) = frame.local(p);
                (x, y)
            }
            Chart::Tube {
                frame, major, cut, ..
            } => {
                let (u, v) = torus_angles(frame, *major, p);
                let reach = 1.0 + wrap(u - cut) / TAU;
                (reach * v.cos(), reach * v.sin())
            }
            Chart::Ring {
                frame, major, cut, ..
            } => {
                let (u, v) = torus_angles(frame, *major, p);
                let reach = 1.0 + wrap(v - cut) / TAU;
                (reach * u.cos(), reach * u.sin())
            }
            Chart::Patch {
                layout: Layout::Open,
                tree,
                ..
            } => tree
                .nearest(p)
                .map_or((f64::NAN, f64::NAN), |n| tree.chart(n.u, n.v)),
            Chart::Patch { .. } => self.closed_image(p),
            Chart::Cylinder { .. } | Chart::Sphere { .. } => (0.0, 0.0),
        }
    }

    fn target(&self, about: About) -> Option<Target> {
        let q = match about {
            About::Point(q) => q,
            About::InnerEnd => {
                let lipschitz = match self {
                    Chart::Tube { major, minor, .. } => 1.0 / (TAU * (major - minor)) + 2.0 / minor,
                    Chart::Ring { major, minor, .. } => 1.0 / (TAU * minor) + 2.0 / (major - minor),
                    Chart::Patch {
                        layout: Layout::Torus { .. },
                        speed,
                        ..
                    } => *speed,
                    _ => return None,
                };
                return Some(Target {
                    at: (0.0, 0.0),
                    q: Vec3::ZERO,
                    lipschitz,
                });
            }
        };
        Some(match self {
            Chart::Plane(_) | Chart::Cone(_) => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0,
            },
            Chart::Patch {
                layout: Layout::Sphere { .. },
                speed,
                ..
            } => {
                let (a, b) = homogeneous(self.shares(q)?);
                Target {
                    at: (f64::NAN, f64::NAN),
                    q: Vec3::new(a.0, a.1, b),
                    lipschitz: *speed,
                }
            }
            Chart::Patch { speed, .. } => Target {
                at: self.coordinates(q),
                q,
                lipschitz: *speed,
            },
            Chart::Cylinder { frame, radius } => {
                let (x, y, z) = frame.local(q);
                let reach = x.hypot(y);
                if !(reach.is_finite() && reach > 0.0) {
                    return None;
                }
                Target {
                    at: (0.0, 0.0),
                    q: Vec3::new(x / reach, y / reach, z),
                    lipschitz: 2.0 / radius,
                }
            }
            Chart::Sphere {
                center,
                radius,
                pole,
                ..
            } => {
                let unit = (q - *center).normalized()?;
                let from_pole = unit - *pole;
                let far = from_pole.length();
                Target {
                    at: (0.0, 0.0),
                    q: from_pole,
                    lipschitz: far * (far + 4.0) / radius,
                }
            }
            Chart::Tube { major, minor, .. } => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0 / (TAU * (major - minor)) + 2.0 / minor,
            },
            Chart::Ring { major, minor, .. } => Target {
                at: self.coordinates(q),
                q,
                lipschitz: 1.0 / (TAU * minor) + 2.0 / (major - minor),
            },
        })
    }

    /// The direction from the image of the target to the image of `p`, up
    /// to a positive factor; zero only when they are the same point.
    fn direction(&self, target: &Target, p: Vec3) -> (f64, f64) {
        match self {
            Chart::Patch {
                layout: Layout::Sphere { drop },
                ..
            } => {
                // With `z = a / b` the plane of the poles and `w = 1 / (z −
                // z₀)` the chart, `w_p − w_q = (z_q − z_p) / ((z_p − z₀)
                // (z_q − z₀))`, which is, up to a positive factor,
                // `(a_q b_p − a_p b_q) conj(A_p) conj(A_q)` with `A = a b₀
                // − a₀ b`: bounded, and continuous through the pole at
                // infinity of `z`. With `|a| + b = 1` for every point,
                // `|∂/∂t| ≤ 2` and `|∂/∂s| ≤ 2π t (1 − t)` of the first
                // two factors, and `|A_q| ≤ 1`: what the speed assumes.
                let Some(shares) = self.shares(p) else {
                    return (0.0, 0.0);
                };
                let (a, b) = homogeneous(shares);
                let (aq, bq) = ((target.q.x, target.q.y), target.q.z);
                let apart = (aq.0 * b - a.0 * bq, aq.1 * b - a.1 * bq);
                let towards = times_conj(apart, from_drop(a, b, *drop));
                times_conj(towards, from_drop(aq, bq, *drop))
            }
            Chart::Plane(_)
            | Chart::Cone(_)
            | Chart::Tube { .. }
            | Chart::Ring { .. }
            | Chart::Patch { .. } => {
                let (x, y) = self.coordinates(p);
                (x - target.at.0, y - target.at.1)
            }
            Chart::Cylinder { frame, radius } => {
                // With w = exp((height + i radius angle) / radius) the
                // difference of two images is, up to a positive factor and
                // a fixed turn, (E c - 1, E s) / (E + 1): E the ratio of
                // their distances from the origin, (c, s) the cosine and
                // sine of the angle between them. In terms of
                // T = (E - 1) / (E + 1) = tanh(Δheight / 2 radius) nothing
                // overflows.
                let (x, y, z) = frame.local(p);
                let reach = x.hypot(y);
                if !(reach.is_finite() && reach > 0.0) {
                    return (0.0, 0.0);
                }
                let (ux, uy) = (x / reach, y / reach);
                let c = ux * target.q.x + uy * target.q.y;
                let s = target.q.x * uy - target.q.y * ux;
                let t = (0.5 * (z - target.q.z) / radius).tanh();
                (0.5 * (t * (1.0 + c) - (1.0 - c)), 0.5 * (t + 1.0) * s)
            }
            Chart::Sphere {
                center, pole, x, y, ..
            } => {
                // Stereographic projection from the pole N is an inversion
                // about it; the difference of the images of p and q is,
                // times |p - N|² |q - N|², the vector below, which lies
                // across N.
                let Some(unit) = (p - *center).normalized() else {
                    return (0.0, 0.0);
                };
                let from_pole = unit - *pole;
                let v =
                    from_pole * target.q.length_squared() - target.q * from_pole.length_squared();
                (v.dot(*x), v.dot(*y))
            }
        }
    }

    /// The angle the direction from the target to a point moving along the
    /// piece (in the curve's own direction) turns through.
    fn turn(&self, seg: &Seg, target: &Target) -> Result<f64, Reason> {
        let (from, to) = (self.direction(target, seg.a), self.direction(target, seg.b));
        // A straight piece of a plane, a ruling of a cylinder and a ruling
        // of a cone are straight in their charts: they turn by less than
        // half a turn about any point not on them.
        let straight = matches!(
            (&seg.curve, self),
            (
                Curve3::Line(_),
                Chart::Plane(_) | Chart::Cylinder { .. } | Chart::Cone(_)
            )
        );
        if straight {
            return Ok(angle(from, to));
        }
        // A stretch that stays within a disc that does not contain the
        // target turns by less than half a turn, which is what `angle`
        // assumes. Halve the stretches until they do.
        let curve_speed = speed_bound(&seg.curve);
        let pace = curve_speed * target.lipschitz;
        if !(pace.is_finite() && pace > 0.0) {
            return Err(TOO_CLOSE);
        }
        // On a freeform surface drawn in its own parameters the chart's
        // speed is bounded near the stretch, not over the whole surface:
        // where the parametrisation is far from uniform (rational weights
        // far apart) the one bound for the whole surface is many times
        // what a stretch elsewhere needs.
        let local = match self {
            Chart::Patch {
                tree,
                layout: Layout::Open,
                ..
            } if tree.parameter_speed().is_some() => Some(tree),
            _ => None,
        };
        let pace_over = |t0: f64, t1: f64| -> f64 {
            let Some(tree) = local else {
                return pace * (t1 - t0);
            };
            // The feet of the stretch's points are within twice its
            // length and the distance of its start from the surface of
            // that start: `|f(q) − p| ≤ |f(q) − q| + |q − p|` and
            // `|f(q) − q| ≤ |q − p| + d(p)`.
            let length = curve_speed * (t1 - t0);
            let start = seg.curve.eval(t0);
            let Some(off) = tree.nearest(start).map(|n| n.distance) else {
                return pace * (t1 - t0);
            };
            tree.sigma_near(start, 2.0 * length + off)
                .map_or(pace, |sigma| {
                    curve_speed * (1.0 / sigma).min(target.lipschitz)
                })
                * (t1 - t0)
        };
        let mut total = 0.0;
        let mut stack = vec![(seg.range.0, from, seg.range.1, to)];
        let mut steps = 0_usize;
        while let Some((t0, p0, t1, p1)) = stack.pop() {
            steps += 1;
            if steps > MAX_TURN_STEPS {
                return Err(TOO_CLOSE);
            }
            // Each step may search the surface's pieces near it: the work
            // bound of an operation, when one is in force, is asked per
            // step.
            if craft3d_geom::allowance::pass() {
                return Err(SPENT);
            }
            let far = p0.0.hypot(p0.1);
            if far > pace * (t1 - t0) || (local.is_some() && far > pace_over(t0, t1)) {
                // On a torus cut open the chart is continuous only off the
                // cut, and the sum assumes it: a stretch whose image moves
                // further from the chart's origin than the chart can carry
                // it (a jump between the inner and the outer end of the
                // annulus) runs through the cut.
                if let Some(radial) = self.radial_pace(curve_speed) {
                    let reach = |p: (f64, f64)| (p.0 + target.at.0).hypot(p.1 + target.at.1);
                    let moved = (reach(p1) - reach(p0)).abs();
                    if moved.is_nan() || moved > radial * (t1 - t0) * (1.0 + CUT_SLACK) + 1e-12 {
                        return Err(CUT_CROSSED);
                    }
                }
                total += angle(p0, p1);
            } else {
                let mid = 0.5 * (t0 + t1);
                if !(mid > t0 && mid < t1) {
                    return Err(TOO_CLOSE);
                }
                let pm = self.direction(target, seg.curve.eval(mid));
                stack.push((mid, pm, t1, p1));
                stack.push((t0, p0, mid, pm));
            }
        }
        Ok(total)
    }

    /// How many times the loops go around `about`, counter-clockwise in
    /// the chart.
    pub(super) fn winding<'a>(
        &self,
        loops: impl IntoIterator<Item = &'a [Seg]>,
        about: About,
    ) -> Result<i32, Reason> {
        let target = self.target(about).ok_or(TOO_CLOSE)?;
        if let (Chart::Sphere { .. }, About::Point(_)) = (self, about) {
            // At the centre of projection: at infinity in the chart, where
            // nothing winds.
            if target.q.length() < 1e-9 {
                return Ok(0);
            }
        }
        if let Chart::Patch {
            layout: Layout::Sphere { drop },
            ..
        } = self
        {
            // Likewise at the point the chart leaves out.
            let (x, y) = from_drop((target.q.x, target.q.y), target.q.z, *drop);
            if x.hypot(y) < 1e-9 {
                return Ok(0);
            }
        }
        let mut total = 0.0;
        for seg in loops.into_iter().flatten() {
            total += seg.sign() * self.turn(seg, &target)?;
        }
        let turns = total / TAU;
        let whole = turns.round();
        if !(whole.is_finite() && (turns - whole).abs() < 0.05 && whole.abs() < 1e6) {
            return Err(TOO_CLOSE);
        }
        Ok(whole as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::freeform::CLOSED_POLE;
    use craft3d_geom::NurbsSurface;
    use std::f64::consts::FRAC_PI_4;

    /// The surface swept a full turn about the z axis by a rational
    /// profile `(reach, height, weight)` of the given degree and knots, as
    /// four rational quadratic quarters (so that the net ends on the row
    /// it starts with); the turn is u, or v when `around_v`.
    fn turned(
        profile: &[(f64, f64, f64)],
        degree: usize,
        knots: Vec<f64>,
        around_v: bool,
    ) -> Surface {
        let weight = FRAC_PI_4.cos();
        let ring: Vec<(f64, f64, f64)> = (0..9)
            .map(|k| {
                let (sin, cos) = (0.3 + FRAC_PI_4 * f64::from(k)).sin_cos();
                if k % 2 == 0 {
                    (cos, sin, 1.0)
                } else {
                    (cos / weight, sin / weight, weight)
                }
            })
            .collect();
        let ring_knots = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        let point = |r: &(f64, f64, f64), q: &(f64, f64, f64)| {
            (Vec3::new(q.0 * r.0, q.0 * r.1, q.1), r.2 * q.2)
        };
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        if around_v {
            for q in profile {
                for r in &ring {
                    let (p, w) = point(r, q);
                    ctrl.push(p);
                    weights.push(w);
                }
            }
            let n = profile.len();
            Surface::Nurbs(
                NurbsSurface::new(degree, 2, knots, ring_knots, n, 9, ctrl, weights).unwrap(),
            )
        } else {
            for r in &ring {
                for q in profile {
                    let (p, w) = point(r, q);
                    ctrl.push(p);
                    weights.push(w);
                }
            }
            let n = profile.len();
            Surface::Nurbs(
                NurbsSurface::new(2, degree, ring_knots, knots, 9, n, ctrl, weights).unwrap(),
            )
        }
    }

    /// The closed surfaces the charts are for: a wall (closed around), a
    /// cup (and a pole), a ball (two poles) and a torus (closed both ways).
    fn closed(around_v: bool) -> Vec<(&'static str, Surface)> {
        let w = FRAC_PI_4.cos();
        let quarter = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let half = vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0];
        let circle = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        let tube: Vec<(f64, f64, f64)> = (0..9)
            .map(|k| {
                let (sin, cos) = (FRAC_PI_4 * f64::from(k)).sin_cos();
                if k % 2 == 0 {
                    (2.0 + 0.6 * cos, 0.6 * sin, 1.0)
                } else {
                    (2.0 + 0.6 * cos / w, 0.6 * sin / w, w)
                }
            })
            .collect();
        vec![
            (
                "wall",
                turned(
                    &[(2.0, 0.0, 1.0), (2.0, 3.0, 1.0)],
                    1,
                    vec![0.0, 0.0, 1.0, 1.0],
                    around_v,
                ),
            ),
            (
                "cup",
                turned(
                    &[(2.0, 0.0, 1.0), (2.0, 2.0, w), (0.0, 2.0, 1.0)],
                    2,
                    quarter,
                    around_v,
                ),
            ),
            (
                "ball",
                turned(
                    &[
                        (0.0, -2.0, 1.0),
                        (2.0, -2.0, w),
                        (2.0, 0.0, 1.0),
                        (2.0, 2.0, w),
                        (0.0, 2.0, 1.0),
                    ],
                    2,
                    half,
                    around_v,
                ),
            ),
            ("torus", turned(&tube, 2, circle, around_v)),
            // A needle: the profile leaves the axis along it (a cusp, a
            // disc whose radius is the square of the share across).
            (
                "needle",
                turned(
                    &[(0.0, 0.0, 1.0), (0.0, 1.0, 1.0), (1.5, 2.0, 1.0)],
                    2,
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    around_v,
                ),
            ),
        ]
    }

    /// The chart of a closed surface moves no faster than its speed says,
    /// across the seam too, and its image of a point does not depend on
    /// the side of the seam the point is found from; for the ball, the
    /// direction from every target point likewise.
    #[test]
    fn the_chart_of_a_closed_surface_moves_no_faster_than_its_speed() {
        for around_v in [false, true] {
            for (name, surface) in closed(around_v) {
                let tree = Arc::new(PatchTree::new(&surface).unwrap());
                let closure = tree.closure().unwrap().unwrap();
                let chart = Chart::closed_patch(&tree, &closure, &[]).unwrap();
                let Chart::Patch { speed, layout, .. } = &chart else {
                    panic!("{name}");
                };
                let speed = *speed;
                let ((u0, u1), (v0, v1)) = surface.domain();
                let at = |a: f64, b: f64| {
                    let (a, b) = (a.rem_euclid(1.0), b.clamp(0.0, 1.0));
                    let (a, b) = if around_v { (b, a) } else { (a, b) };
                    surface.eval(u0 + (u1 - u0) * a, v0 + (v1 - v0) * b)
                };
                let targets = [at(0.1, 0.5), at(0.6, 0.3), at(0.95, 0.8)];
                let mut fastest = 0.0_f64;
                let places: [f64; 8] = [0.0, 1e-6, 0.02, 0.27, 0.5, 0.73, 0.98, 1.0 - 1e-6];
                for &a in &places {
                    for &b in &places[..places.len() - 1] {
                        let b = b.clamp(1e-3, 0.999);
                        for turn in 0..8 {
                            let angle = f64::from(turn) * PI / 4.0;
                            let (p, q) =
                                (at(a, b), at(a + 1e-4 * angle.cos(), b + 1e-4 * angle.sin()));
                            let travelled = p.distance(q);
                            if travelled <= 1e-13 {
                                continue;
                            }
                            // A torus is cut open: the chart jumps there.
                            if let Layout::Torus { cut, swapped } = layout {
                                let near_cut = |x: Vec3| {
                                    let (s, t) = chart.shares(x).unwrap();
                                    let t = if *swapped { s } else { t };
                                    let off = wrap_share(t - cut);
                                    off.min(1.0 - off) < 1e-3
                                };
                                if near_cut(p) || near_cut(q) {
                                    continue;
                                }
                            }
                            for &target in &targets {
                                let target = chart.target(About::Point(target)).unwrap();
                                let (x0, y0) = chart.direction(&target, p);
                                let (x1, y1) = chart.direction(&target, q);
                                let pace = (x1 - x0).hypot(y1 - y0) / travelled;
                                assert!(
                                    pace <= target.lipschitz * (1.0 + 1e-6),
                                    "{name}, around v {around_v}: at ({a}, {b}) {pace} per length, above {}",
                                    target.lipschitz
                                );
                                fastest = fastest.max(pace);
                            }
                        }
                    }
                }
                println!(
                    "{name} ({layout:?}), around v {around_v}: bound {speed}, fastest seen {fastest}"
                );
                assert!(
                    speed <= 100.0 * fastest,
                    "{name}: {speed} against {fastest}"
                );
                // Either side of the seam is one point of the chart.
                if !matches!(layout, Layout::Sphere { .. }) {
                    for b in [0.2, 0.7] {
                        let (x, y) = chart.coordinates(at(0.0, b));
                        let (x1, y1) = chart.coordinates(at(1.0 - 1e-12, b));
                        assert!((x - x1).hypot(y - y1) < 1e-9, "{name}");
                    }
                }
            }
        }
    }

    /// Around a loop of a closed surface: a ring around the wall goes
    /// around the chart's origin, a small loop across the seam around the
    /// points inside it only, on every layout.
    #[test]
    fn loops_across_the_seam_wind_about_what_they_hold() {
        use craft3d_geom::Circle;
        for around_v in [false, true] {
            for (name, surface) in closed(around_v) {
                let tree = Arc::new(PatchTree::new(&surface).unwrap());
                let closure = tree.closure().unwrap().unwrap();
                // A small circle on the surface across the seam (the seam
                // is at the angle 0.3 about the z axis), as a curve near
                // enough to it for a winding number.
                let (sin, cos) = 0.3_f64.sin_cos();
                let side = Vec3::new(cos, sin, 0.0);
                let mut centre = Vec3::new(0.0, 0.0, 0.5) + side * 2.0;
                if name == "torus" {
                    centre = side * 2.6;
                }
                let near = tree.nearest(centre).unwrap().point;
                let normal = (near - Vec3::new(0.0, 0.0, near.z)).normalized().unwrap();
                let frame = Frame::new(near, normal, Vec3::Z).unwrap();
                let small = Curve3::Circle(Circle::new(frame, 0.05).unwrap());
                let seg = Seg {
                    a: small.eval(0.0),
                    b: small.eval(TAU),
                    curve: small,
                    range: (0.0, TAU),
                    reversed: false,
                };
                let loops = vec![vec![seg]];
                let chart = Chart::new(
                    Sheet {
                        surface: &surface,
                        tree: Some(&tree),
                    },
                    &loops,
                )
                .unwrap();
                let _ = closure;
                let inside = chart
                    .winding([loops[0].as_slice()], About::Point(near))
                    .unwrap();
                let far = tree
                    .nearest(-side * 2.0 + Vec3::new(0.0, 0.0, 0.4))
                    .unwrap()
                    .point;
                let outside = chart
                    .winding([loops[0].as_slice()], About::Point(far))
                    .unwrap();
                assert_eq!(
                    (inside.abs(), outside),
                    (1, 0),
                    "{name}, around v {around_v}"
                );
            }
        }
    }

    /// A surface that closes on itself and into a point along the same
    /// parameter has no chart that is built: refused by name.
    #[test]
    fn a_surface_closed_into_its_own_pole_is_refused() {
        // Row 0 and row 2 are one point, row 1 a segment: closed in u,
        // and both u borders poles.
        let p = Vec3::new(0.0, 0.0, 0.0);
        let ctrl = vec![
            p,
            p,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            p,
            p,
        ];
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                3,
                2,
                ctrl,
                vec![1.0; 6],
            )
            .unwrap(),
        );
        let tree = PatchTree::new(&surface).unwrap();
        assert!(matches!(tree.closure(), Some(Err(reason)) if reason == CLOSED_POLE));
    }

    #[test]
    fn the_widest_gap_between_stretches_of_a_circle() {
        assert_eq!(widest_gap(Vec::new()), Some((PI, TAU)));
        assert_eq!(widest_gap(vec![(1.0, TAU)]), None);
        let (middle, width) = widest_gap(vec![(1.0, 1.0), (4.0, 0.5)]).unwrap();
        // From 4.5 across the seam to 1: wider than from 2 to 4.
        assert!((width - (TAU - 3.5)).abs() < 1e-12);
        assert!((middle - wrap(4.5 + 0.5 * (TAU - 3.5))).abs() < 1e-12);
        // A stretch across the seam; overlapping stretches.
        let (middle, width) = widest_gap(vec![(6.0, 1.0), (0.5, 1.0), (1.0, 2.0)]).unwrap();
        assert!((width - 3.0).abs() < 1e-12, "{width}");
        assert!((middle - 4.5).abs() < 1e-12, "{middle}");
        // Covered between them.
        assert_eq!(widest_gap(vec![(0.0, 4.0), (3.0, 4.0)]), None);
        // A stretch that starts past the first one and runs on past 2π
        // covers the start of the walk as much as its end (the fuzzer's
        // finding of 2026-10-20: the gap was put at 4.31, inside the
        // stretch 4.81 → 10.89 = 4.61, which a loop of the face ran
        // through). The one gap is between 4.61 and 4.81.
        let (middle, width) =
            widest_gap(vec![(6.228, 2.3125), (2.148, 1.652), (4.8138, 6.0788)]).unwrap();
        assert!(
            (width - (4.8138 - (4.8138 + 6.0788 - TAU))).abs() < 1e-12,
            "{width}"
        );
        assert!((middle - (4.8138 - 0.5 * width)).abs() < 1e-12, "{middle}");
        // The same stretch alone, and with a piece at the start: the gap
        // is its own end to its start.
        let (middle, width) = widest_gap(vec![(4.8138, 6.0788)]).unwrap();
        assert!((width - (TAU - 6.0788)).abs() < 1e-12, "{width}");
        assert!((middle - (4.8138 - 0.5 * width)).abs() < 1e-12, "{middle}");
        assert_eq!(widest_gap(vec![(4.0, 2.0), (6.0, 4.5)]), None);
        assert_eq!(widest_gap(vec![(1.0, f64::NAN)]), None);
    }

    /// A loop that runs through the cut its chart was opened along is
    /// refused by the winding sum, not summed across the jump.
    #[test]
    fn a_loop_through_the_cut_of_a_torus_chart_is_refused() {
        use craft3d_geom::{Circle, Torus};
        let torus = Torus::new(Frame::WORLD, 10.0, 3.0).unwrap();
        let torus_frame = *torus.frame();
        // A circle around the axis at angle 0.5 around the tube (radius
        // 10 + 3 cos 0.5, height 3 sin 0.5), as one piece: it crosses
        // every circle around the tube, so a chart cut along one of
        // them (as if the coverage had missed it) is no chart of it.
        let (sin, cos) = 0.5_f64.sin_cos();
        let frame = Frame::WORLD.at(Vec3::new(0.0, 0.0, 3.0 * sin)).unwrap();
        let circle = Curve3::Circle(Circle::new(frame, 10.0 + 3.0 * cos).unwrap());
        let seg = Seg {
            a: circle.eval(0.0),
            b: circle.eval(0.0),
            curve: circle,
            range: (0.0, TAU),
            reversed: false,
        };
        let chart = Chart::Tube {
            frame: torus_frame,
            major: 10.0,
            minor: 3.0,
            cut: 1.0,
        };
        // A point of the torus at angle 2 about the axis and 2.5 around
        // the tube. Cut along a circle around the axis instead (angle 4
        // around the tube, which the loop does not cross) the loop winds
        // once about the inner end and not about that point, which lies
        // between the loop and the cut.
        let about = About::Point(Surface::Torus(torus).eval(2.0, 2.5));
        assert_eq!(
            chart.winding([&[seg.clone()][..]], about),
            Err(CUT_CROSSED),
            "the loop runs through the cut"
        );
        let chart = Chart::Ring {
            frame: torus_frame,
            major: 10.0,
            minor: 3.0,
            cut: 4.0,
        };
        assert_eq!(
            chart
                .winding([&[seg.clone()][..]], About::InnerEnd)
                .map(i32::abs),
            Ok(1)
        );
        assert_eq!(chart.winding([&[seg][..]], about), Ok(0));
    }
}
