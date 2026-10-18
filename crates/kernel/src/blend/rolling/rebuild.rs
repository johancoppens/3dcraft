//! The offset of a rolling-ball fillet: the fillet rebuilt between the
//! offsets of the faces it rolls along, with its radius changed by the
//! distance.
//!
//! **Why rebuilt, not offset.** A fillet is the surface a ball of radius
//! `r` sweeps rolling along two faces: the balls' centres (the spine) are
//! the points `r` away from both. The exact offset of that surface by `d`
//! is the surface the balls of radius `r ± d` about the same centres
//! sweep (`+` where the centres lie in the material, a convex fillet;
//! `-` at a concave one), and those balls are exactly the ones `r ± d`
//! away from both faces moved by `d`: the offset of a fillet is the
//! fillet of radius `r ± d` between the offset faces, tangent to them by
//! construction. Offsetting the fitted spline instead, along its own
//! normal, carries its fitting error in the normal into the offset times
//! the distance, and leaves it tangent to its moved neighbours only as
//! well as it was fitted (the bike's saddle: to 6.6e-5).
//!
//! **What a fillet face is, read off its surface.** The surface
//! [`super::refine`] makes: `u` along the fillet, `v` from the first face
//! (`v = 0`) to the second (`v = 1`), each row across a rational quadratic
//! arc, interpolated through the exact sections so that the first and the
//! last row (and, round a ring, the first) are exact sections. The radius
//! and the side the centres lie on are measured there, from the feet and
//! the faces' normals (`ρ = (q₁ − q₂) · (n₁ − n₂) / |n₁ − n₂|²`; the radius
//! the fillet was made with, when the caller knows it, is taken when the
//! measure agrees with it); then each of 17 rows along it is held against
//! the exact ball of that radius between its two faces, found in the
//! row's own plane: every point the radius from its centre and its ends
//! on the faces, to the tolerance. A face that does not hold is not taken
//! for a fillet: it is offset as any freeform face.
//!
//! **The planes the sections are taken in.** Each ball is found in the
//! plane through the old surface's first border at `u`, square to that
//! border. The old centre there lies in that plane (it is the radius away
//! from the first face along that face's normal, square to the border),
//! and so does the new one (the same centre); the new section at `u` is
//! the ball about the same centre, so the new surface keeps the old
//! one's parameters: the same `u` is the same place along the fillet, an
//! iso-curve at a joint of a chain is the joint's section on both sides,
//! and each section is started from the old centre, exact to the fit;
//! at a joint the planes turn into the joint's own (the old end row,
//! exact), so two fillets of a chain rebuilt each on its own meet in one
//! section. Past the part of the old surface its face's edges reach, a
//! ball is kept only while its feet stay on the faces it rolls along.
//! The new surface is refined as every rolling-ball blend is: within a
//! quarter of the tolerance of the exact sections between samples,
//! tangent to both faces to `bend`, its borders proven on their faces.

use super::{FOLLOW_DEPTH, Section, Shape, Side, Solver, Spine, bend, border, refine};
use craft3d_geom::{Curve3, NurbsSurface, Surface};
use craft3d_math::{Tolerance, Vec3};

/// A face's surface and which way is out of the material.
#[derive(Clone, Copy)]
pub(crate) struct Support<'a> {
    pub surface: &'a Surface,
    pub reversed: bool,
}

impl<'a> Support<'a> {
    fn side(self) -> Side<'a> {
        Side {
            surface: self.surface,
            reversed: self.reversed,
        }
    }
}

/// A fillet face read off its surface: the radius of its ball and
/// whether the ball's centres lie in the material (a convex edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ball {
    pub radius: f64,
    pub convex: bool,
}

/// Why a fillet has no rebuilt offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoRebuild {
    /// The radius reaches zero: the fillet would vanish into an edge.
    Vanishes,
    /// The fillet could not be rebuilt: why.
    Fails(&'static str),
}

pub(crate) const UNSOLVED: &str = "the fillet could not be rebuilt between the offset faces: a ball of the new radius touching both was not found along it";
pub(crate) const NOT_FITTED: &str = "the fillet rebuilt between the offset faces could not be approximated within the tolerance in the samples allowed";

/// Steps along the face at whose ends (17 rows) it is held against the
/// exact ball.
const HELD_AT: usize = 16;

/// Over how much of the old surface's parameter range (from each end) the
/// planes the sections are taken in turn into the end section's plane.
const END_TURN: f64 = 0.05;

/// Most samples the new surface starts from.
const MOST_START: usize = 512;

/// The size of the numbers about `surface`: what "settled" is measured
/// against.
fn scale(surface: &NurbsSurface, radius: f64, tol: f64) -> f64 {
    let ((u0, u1), _) = surface.domain();
    surface
        .eval(u0, 0.0)
        .length()
        .max(surface.eval(u1, 0.0).length())
        .max(radius)
        .max(tol)
}

/// Whether the surface has the shape [`super::refine`] gives a fillet:
/// three rows across, rational quadratic in `v` over `[0, 1]`.
fn shaped(surface: &NurbsSurface) -> bool {
    let (_, degree_v) = surface.degrees();
    let (_, count_v) = surface.counts();
    let (_, (v0, v1)) = surface.domain();
    degree_v == 2 && count_v == 3 && v0 == 0.0 && v1 == 1.0
}

/// The section of the old surface at `u` as a seed: its feet, their
/// parameters on the faces, and the centre `ρ` behind the first foot.
fn seed(
    surface: &NurbsSurface,
    u: f64,
    sides: [Side<'_>; 2],
    signed: f64,
    near: Option<&Section>,
) -> Option<Section> {
    let feet = [surface.eval(u, 0.0), surface.eval(u, 1.0)];
    let mut params = [(0.0, 0.0); 2];
    for k in 0..2 {
        let side = sides.get(k)?;
        let foot = *feet.get(k)?;
        let start = match near {
            Some(n) => *n.params.get(k)?,
            None => side.surface.project(foot).ok()?,
        };
        let (uv, _) = side.foot(foot, start)?;
        *params.get_mut(k)? = uv;
    }
    let n = sides.first()?.normal(*params.first()?)?;
    let centre = *feet.first()? - n * signed;
    centre.is_finite().then_some(Section {
        feet,
        params,
        centre: Some(centre),
    })
}

/// The signed radius of the arc of `surface` at `u` between faces whose
/// outward normals at its feet are `n1` and `n2`: positive when the centre
/// lies behind the faces (in the material).
fn signed_radius(surface: &NurbsSurface, u: f64, sides: [Side<'_>; 2]) -> Option<f64> {
    let (q1, q2) = (surface.eval(u, 0.0), surface.eval(u, 1.0));
    let mut normals = [Vec3::ZERO; 2];
    for (k, q) in [(0_usize, q1), (1, q2)] {
        let side = sides.get(k)?;
        let start = side.surface.project(q).ok()?;
        let (uv, _) = side.foot(q, start)?;
        *normals.get_mut(k)? = side.normal(uv)?;
    }
    let [n1, n2] = normals;
    let spread = n1 - n2;
    let square = spread.dot(spread);
    // Faces nearly parallel at the feet: the arc is too flat to measure.
    if !(square > 1e-12) {
        return None;
    }
    let rho = (q1 - q2).dot(spread) / square;
    // Both feet the radius from one centre.
    let gap = (q1 - n1 * rho).distance(q2 - n2 * rho);
    (rho.is_finite() && gap <= 1e-6 * rho.abs()).then_some(rho)
}

/// The fillet `surface` (a face's, whose outward normal is reversed from
/// its own when `reversed`) is, between `sides` (the faces along its
/// borders `v = 0` and `v = 1`): its ball, when it is a rolling-ball
/// fillet between them to the tolerance; `None` when it is not one.
/// `radius` is the radius it was made with, when known.
pub(crate) fn fillet_of(
    surface: &NurbsSurface,
    reversed: bool,
    sides: [Support<'_>; 2],
    radius: Option<f64>,
    tolerance: &Tolerance,
) -> Option<Ball> {
    let tol = tolerance.linear;
    if !shaped(surface) {
        return None;
    }
    let sides = sides.map(Support::side);
    let ((u0, u1), _) = surface.domain();
    if !(u1 > u0) {
        return None;
    }
    // At the ends, which are exact sections.
    let (first, last) = (
        signed_radius(surface, u0, sides)?,
        signed_radius(surface, u1, sides)?,
    );
    if first.signum() != last.signum() {
        return None;
    }
    let measured = 0.5 * (first + last);
    let rho = match radius {
        Some(r) if (first.abs() - r).abs() <= tol && (last.abs() - r).abs() <= tol => {
            r * measured.signum()
        }
        _ if (first - last).abs() <= 1e-9 * measured.abs() => measured,
        _ => return None,
    };
    let ball = Ball {
        radius: rho.abs(),
        convex: rho > 0.0,
    };
    // The face's outward normal is the faces' at its borders.
    let (point, du, dv) = surface.derivatives(0.5 * (u0 + u1), 0.0);
    let own = du.cross(dv).normalized()?;
    let own = if reversed { -own } else { own };
    let first_side = sides.first()?;
    let (uv, _) = first_side.foot(point, first_side.surface.project(point).ok()?)?;
    if !(own.dot(first_side.normal(uv)?) > 0.0) {
        return None;
    }
    // Held against the exact ball between the faces: each row across is
    // an arc of a ball the radius away from both faces, in the plane of
    // the row (through the centre it gives, square to the arc), its ends
    // on the faces. Not point for point against a ball found in another
    // plane: two fits of one blend may take their sections a little apart
    // along it, which moves no point off the surface.
    let solver = Solver {
        spine: Spine {
            curve: &Curve3::Nurbs(border(surface, false)?),
            range: (u0, u1),
            closed: false,
            joints: [None, None],
            turn: 0.0,
        },
        sides,
        shape: Shape::Ball {
            radius: ball.radius,
            convex: ball.convex,
        },
        scale: scale(surface, ball.radius, tol),
        moved: None,
        settle: 1e-2 * tol,
    };
    for k in 0..=HELD_AT {
        let u = u0 + (u1 - u0) * (k as f64 / HELD_AT as f64);
        let near = seed(surface, u, sides, rho, None)?;
        let centre = near.centre?;
        let [q1, q2] = near.feet;
        let normal = (q1 - centre).cross(q2 - centre).normalized()?;
        let across = super::Across {
            point: centre,
            normal,
        };
        let exact = solver.ball(across, &near, ball.radius, ball.convex)?;
        let centre = exact.centre?;
        // The ball's own arc: on the ball, and in the plane of its centre
        // and feet (any other circle of the ball through the feet is
        // tangent to the faces there too).
        let [f1, f2] = exact.feet;
        let plane = (f1 - centre).cross(f2 - centre).normalized()?;
        for j in 0..=8 {
            let v = f64::from(j) / 8.0;
            let p = surface.eval(u, v);
            let off = (p.distance(centre) - ball.radius).abs();
            let aside = (p - centre).dot(plane).abs();
            if !(off <= tol && aside <= tol) {
                return None;
            }
        }
        for (k, v) in [(0_usize, 0.0), (1, 1.0)] {
            let off = surface.eval(u, v).distance(*exact.feet.get(k)?);
            if !(off <= tol) {
                return None;
            }
        }
    }
    Some(ball)
}

/// Rows of `surface` across `v` (`count_v` of them) taken in the curve at
/// `v`: the iso-curve of a surface of one span in `v` along `u`.
fn iso_v(surface: &NurbsSurface, v: f64) -> Option<craft3d_geom::NurbsCurve> {
    let (degree_u, degree_v) = surface.degrees();
    let (count_u, count_v) = surface.counts();
    if count_v != degree_v + 1 {
        return None;
    }
    // Bernstein weights of one Bézier span over [0, 1].
    let n = degree_v;
    let mut basis = vec![0.0; n + 1];
    let mut choose = 1.0_f64;
    for (j, slot) in basis.iter_mut().enumerate() {
        if j > 0 {
            choose = choose * (n + 1 - j) as f64 / j as f64;
        }
        *slot = choose * v.powi(j as i32) * (1.0 - v).powi((n - j) as i32);
    }
    let mut ctrl = Vec::with_capacity(count_u);
    let mut weights = Vec::with_capacity(count_u);
    for i in 0..count_u {
        let (mut point, mut weight) = (Vec3::ZERO, 0.0);
        for (j, b) in basis.iter().enumerate() {
            let k = i.checked_mul(count_v)?.checked_add(j)?;
            let w = *surface.weights().get(k)?;
            point += *surface.control_points().get(k)? * (w * b);
            weight += w * b;
        }
        if !(weight > 0.0) {
            return None;
        }
        ctrl.push(point * (1.0 / weight));
        weights.push(weight);
    }
    craft3d_geom::NurbsCurve::new(degree_u, surface.knots_u().to_vec(), ctrl, weights).ok()
}

/// Lines along the face, `v` fixed, proven against the rebuilt fillet.
const PROVEN_LINES: usize = 8;

/// Most pieces of the proof of one line (the saddle's fillets of 264 to
/// 352 spans take about 16 000 at the default tolerance).
const PROOF_PIECES: usize = 1 << 16;

/// Whether `surface`, the fillet [`fillet_of`] read off it (`ball`, between
/// `sides`), is that fillet over `core` (the part of its `u` its face's
/// edges reach) to the tolerance, proven, not sampled: the fillet is
/// rebuilt between the same faces with the same radius (within a quarter
/// of the tolerance of the exact balls, as every rolling-ball blend), and
/// the lines of `surface` at `v = k / 8` are proven within three quarters
/// of the tolerance of it over the whole core (so within the tolerance of
/// the exact fillet) ([`craft3d_geom::certify`]). A
/// surface whose rows are arcs of the balls at the rows held but not
/// between them is no fillet.
pub(crate) fn proven(
    surface: &NurbsSurface,
    ball: Ball,
    sides: [Support<'_>; 2],
    core: (f64, f64),
    joints: [bool; 2],
    tolerance: &Tolerance,
) -> bool {
    use craft3d_geom::certify::{CurveBound, SurfaceBound};
    let Ok(reference) = rebuilt(
        surface, ball, sides, sides, 0.0, core, core, joints, tolerance,
    ) else {
        return false;
    };
    let reference = Surface::Nurbs(reference);
    let Some(bound) = SurfaceBound::new(&reference) else {
        return false;
    };
    let ((u0, u1), _) = surface.domain();
    let range = if closed(surface, tolerance.linear) {
        (u0, u1)
    } else {
        (core.0.max(u0), core.1.min(u1))
    };
    (0..=PROVEN_LINES).all(|k| {
        let v = k as f64 / PROVEN_LINES as f64;
        let Some(line) = iso_v(surface, v) else {
            return false;
        };
        // The rebuilt fillet keeps the old one's parameters: the foot of
        // the line's start is at its own `(u, v)` there (round a ring a
        // projection could find it across the seam, at the other end).
        let ((r0, r1), _) = reference.domain();
        let start = (range.0.clamp(r0, r1), v);
        bound
            .curve_within(
                &CurveBound::nurbs(&line),
                range,
                start,
                &[],
                0.75 * tolerance.linear,
                PROOF_PIECES,
            )
            .is_ok()
    })
}

/// Whether the fillet goes round on itself along `u` (a ring).
fn closed(surface: &NurbsSurface, tol: f64) -> bool {
    surface.closes(true, tol)
}

/// The fillet `surface` ([`fillet_of`] gave `ball`) moved by `distance`
/// along its outward normal: the fillet of radius `r + distance` (convex)
/// or `r − distance` (concave) between `sides`, the faces it rolls along
/// moved by the same distance, in the parameters of the old surface, and
/// over `range` of its `u` (the part the face needs; it may reach past the
/// old surface's ends, where the face grows past them: the balls are
/// followed there as far as they are found; round a ring, the whole).
/// Outside `core` (where the face's edges lie) the surface is made only as
/// far as balls are found with their feet on the faces it rolls along
/// ([`on_face`]): a freeform face continued far past its border is no
/// face the fillet rolls along (past a smooth edge it rolls along the
/// next face).
#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuilt(
    surface: &NurbsSurface,
    ball: Ball,
    sides: [Support<'_>; 2],
    old_sides: [Support<'_>; 2],
    distance: f64,
    range: (f64, f64),
    core: (f64, f64),
    joints: [bool; 2],
    tolerance: &Tolerance,
) -> Result<NurbsSurface, NoRebuild> {
    let tol = tolerance.linear;
    let radius = if ball.convex {
        ball.radius + distance
    } else {
        ball.radius - distance
    };
    if !(radius > tol) {
        return Err(NoRebuild::Vanishes);
    }
    let unsolved = || NoRebuild::Fails(UNSOLVED);
    let ((u0, u1), _) = surface.domain();
    let width = u1 - u0;
    if !(width > 0.0) {
        return Err(unsolved());
    }
    let ring = closed(surface, tol);
    let curve = Curve3::Nurbs(border(surface, false).ok_or_else(unsolved)?);
    let old = old_sides.map(Support::side);
    let signed = if ball.convex {
        ball.radius
    } else {
        -ball.radius
    };
    // At a joint with the next fillet of a chain (`joints`: an edge along
    // the old surface's end there) the old row is an exact section: the
    // planes across the border turn into its plane there, so the new end
    // section is the ball about the same centre, and two fillets of a
    // chain rebuilt each on its own meet in one section.
    let end_plane = |u: f64| -> Option<super::Across> {
        let section = seed(surface, u, old, signed, None)?;
        let centre = section.centre?;
        let [q1, q2] = section.feet;
        let normal = (q1 - centre).cross(q2 - centre).normalized()?;
        let (_, along) = curve.derivatives(u);
        let normal = if normal.dot(along) < 0.0 {
            -normal
        } else {
            normal
        };
        Some(super::Across {
            point: centre,
            normal,
        })
    };
    let joints = if ring {
        [None, None]
    } else {
        let [at_start, at_end] = joints;
        [
            if at_start { end_plane(u0) } else { None },
            if at_end { end_plane(u1) } else { None },
        ]
    };
    let solver = Solver {
        spine: Spine {
            curve: &curve,
            range: (u0, u1),
            closed: ring,
            joints,
            turn: END_TURN,
        },
        sides: sides.map(Support::side),
        shape: Shape::Ball {
            radius,
            convex: ball.convex,
        },
        scale: scale(surface, radius.max(ball.radius), tol),
        moved: None,
        settle: 1e-2 * tol,
    };
    // The part to build, in the spine's `s` (0 to 1 over the old
    // surface): of the old surface, and past its ends.
    let (s_lo, s_hi) = if ring {
        (0.0, 1.0)
    } else {
        ((range.0 - u0) / width, (range.1 - u0) / width)
    };
    let (inside_lo, inside_hi) = (s_lo.max(0.0), s_hi.min(1.0));
    if !(inside_hi > inside_lo) {
        return Err(unsolved());
    }
    // Where to start: evenly, as many as the old surface has spans there
    // (where it needed samples, the new one needs about as many); the
    // refinement adds the rest. Evenly: an interpolating spline through
    // steps whose lengths jump swings between them, and the old surface's
    // knots are as uneven as its refinement made them.
    let mut spans = 0_usize;
    let mut last = f64::NAN;
    for &knot in surface.knots_u() {
        let at = (knot - u0) / width;
        if at > inside_lo && at < inside_hi && knot != last {
            spans += 1;
            last = knot;
        }
    }
    let n = (spans + 1).clamp(if ring { 12 } else { 24 }, MOST_START);
    let starts: Vec<f64> = if ring {
        (0..n).map(|k| k as f64 / n as f64).collect()
    } else {
        (0..=n)
            .map(|k| inside_lo + (inside_hi - inside_lo) * (k as f64 / n as f64))
            .collect()
    };
    let (core_lo, core_hi) = ((core.0 - u0) / width, (core.1 - u0) / width);
    // The ends of the core are samples (the face's edges reach them): the
    // nearest start moves there, so the steps stay even (an interpolating
    // spline swings between steps whose lengths jump).
    let mut starts = starts;
    if !ring {
        for bound in [core_lo, core_hi] {
            if !(bound > inside_lo && bound < inside_hi) {
                continue;
            }
            let nearest = starts
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1 - bound).abs().total_cmp(&(b.1 - bound).abs()))
                .map(|(i, _)| i);
            if let Some(slot) = nearest.and_then(|i| starts.get_mut(i)) {
                *slot = bound;
            }
        }
    }
    let needed = |s: f64| ring || (s >= core_lo && s <= core_hi);
    // Per start, its section, or none where it is not needed and no ball
    // with its feet on the faces is found.
    let mut found: Vec<Option<Section>> = Vec::with_capacity(starts.len());
    let mut near: Option<(f64, Section)> = None;
    for &s in &starts {
        let u = u0 + width * s;
        // Started from the old centre, which the new ball shares, with
        // the feet on the new faces near the old feet.
        let section = (|| {
            let mut from = seed(surface, u, old, signed, None)?;
            for k in 0..2 {
                let side = solver.sides.get(k)?;
                let foot = *from.feet.get(k)?;
                let start = match &near {
                    Some((_, n)) => *n.params.get(k)?,
                    None => side.surface.project(foot).ok()?,
                };
                let (uv, point) = side.foot(foot, start)?;
                *from.params.get_mut(k)? = uv;
                *from.feet.get_mut(k)? = point;
            }
            let made = solver.section(s, &from).or_else(|| {
                let (back, previous) = near.as_ref()?;
                solver.follow(*back, previous, s, FOLLOW_DEPTH)
            })?;
            on_face(solver.sides, &made).then_some(made)
        })();
        match section {
            Some(made) => near = Some((s, made)),
            None if needed(s) => return Err(unsolved()),
            None => {}
        }
        found.push(section);
    }
    // The run of sections round the core.
    let first = found
        .iter()
        .position(Option::is_some)
        .ok_or_else(unsolved)?;
    let last = found
        .iter()
        .rposition(Option::is_some)
        .ok_or_else(unsolved)?;
    let mut params = Vec::with_capacity(starts.len());
    let mut sections = Vec::with_capacity(starts.len());
    for (s, section) in starts.iter().zip(&found).take(last + 1).skip(first) {
        let Some(section) = section else {
            return Err(unsolved());
        };
        params.push(*s);
        sections.push(*section);
    }
    let (inside_lo, inside_hi) = (
        *params.first().ok_or_else(unsolved)?,
        *params.last().ok_or_else(unsolved)?,
    );
    // On past the old surface's ends, in steps of the end spans, as far
    // as balls are found: past a joint of a chain the faces it rolls
    // along may end, and what the face needs of the surface past its end
    // the closing of the body asks for, and says when it is not there.
    if !ring {
        for (end, to) in [(0_usize, s_lo), (1, s_hi)] {
            let reach = if end == 0 {
                inside_lo - to
            } else {
                to - inside_hi
            };
            if !(reach > 0.0 && reach.is_finite()) {
                continue;
            }
            let step = if end == 0 {
                params.get(1).zip(params.first()).map(|(b, a)| b - a)
            } else {
                let n = params.len();
                params
                    .get(n.wrapping_sub(1))
                    .zip(params.get(n.wrapping_sub(2)))
                    .map(|(b, a)| b - a)
            }
            .filter(|step| *step > 0.0)
            .ok_or_else(unsolved)?;
            // Outward a step at a time, halved where the ball is not
            // found or its feet leave the faces, until it is a thousandth
            // of a step.
            let mut step = step.min(reach);
            let smallest = step / 1024.0;
            let mut done = 0.0_f64;
            let mut turns = 0_usize;
            while done < reach && step >= smallest && turns < 4 * 256 {
                turns += 1;
                let next = (done + step).min(reach);
                let (at, from, previous) = if end == 0 {
                    (
                        inside_lo - next,
                        inside_lo - done,
                        *sections.first().ok_or_else(unsolved)?,
                    )
                } else {
                    (
                        inside_hi + next,
                        inside_hi + done,
                        *sections.last().ok_or_else(unsolved)?,
                    )
                };
                let Some(found) = solver
                    .follow(from, &previous, at, FOLLOW_DEPTH)
                    .filter(|f| on_face(solver.sides, f))
                else {
                    step *= 0.5;
                    continue;
                };
                if end == 0 {
                    params.insert(0, at);
                    sections.insert(0, found);
                } else {
                    params.push(at);
                    sections.push(found);
                }
                done = next;
            }
        }
    }
    let limit = 0.25 * tol;
    let bend = bend(tolerance, radius);
    let made = refine(&solver, &mut params, &mut sections, limit, bend, ring)
        .ok_or(NoRebuild::Fails(NOT_FITTED))?;
    // In the old surface's `u`: `u = u0 + s (u1 − u0)`.
    in_parameters(&made, u0, width).ok_or_else(unsolved)
}

/// How far a foot may lie past the border of a freeform face's parameters,
/// as a share of their range: the fit of an offset holds a little past its
/// borders, the end spans go on smoothly; further the face continued is
/// no face the fillet rolls along.
const PAST_FACE: f64 = 1e-3;

/// Whether both feet of `section` lie on their faces' surfaces within
/// [`PAST_FACE`] of their parameters' borders (an analytic surface has
/// none).
fn on_face(sides: [Side<'_>; 2], section: &Section) -> bool {
    sides.iter().zip(&section.params).all(|(side, (u, v))| {
        if !matches!(side.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
            return true;
        }
        let ((u0, u1), (v0, v1)) = side.surface.domain();
        let (mu, mv) = (PAST_FACE * (u1 - u0), PAST_FACE * (v1 - v0));
        *u >= u0 - mu && *u <= u1 + mu && *v >= v0 - mv && *v <= v1 + mv
    })
}

/// `surface` with its `u` taken from `s` to `u0 + s · width`: the same
/// points.
fn in_parameters(surface: &NurbsSurface, u0: f64, width: f64) -> Option<NurbsSurface> {
    let (degree_u, degree_v) = surface.degrees();
    let (count_u, count_v) = surface.counts();
    let knots_u: Vec<f64> = surface.knots_u().iter().map(|s| u0 + s * width).collect();
    NurbsSurface::new(
        degree_u,
        degree_v,
        knots_u,
        surface.knots_v().to_vec(),
        count_u,
        count_v,
        surface.control_points().to_vec(),
        surface.weights().to_vec(),
    )
    .ok()
}
