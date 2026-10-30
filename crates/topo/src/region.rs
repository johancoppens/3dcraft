//! Whether the loops of a face bound a region of its surface, read in the
//! surface's parameters (`docs/design/names.md` has the faces; the method
//! is in `docs/design/text.md` › *Loops that bound a region*).
//!
//! **What must hold.** A face lies to the left of its loops seen from
//! outside, so in the surface's `(u, v)`, whose orientation is the
//! outward one unless the face is reversed, a face that is a disc with
//! holes has exactly one loop that runs counter-clockwise (its outer
//! loop, enclosing the most area) and every other one clockwise (holes),
//! and no loop crosses itself or another. Holes inside the outer loop are
//! held on planes by [`crate::stranded_loops`]; here, on every other
//! surface, a point of each hole clear of the other loops must be wound
//! once by the outer loop and by no other hole.
//!
//! **What is not held.** Faces this test cannot chart without doubt are
//! passed, the other checks and the tessellator saying what is wrong
//! there: a loop that winds around a closed surface (a band of a
//! cylinder, a torus's handle), a face on a surface closed both ways (a
//! torus: any loop there bounds a region on one side or the other), a
//! loop through or near a pole or an apex (a sphere's pole, a cone's
//! apex, a freeform surface's collapsed border), a freeform surface that
//! closes on itself (its seam is not a period of the parameters), a point
//! that does not project back onto the surface, a loop whose area is not
//! larger than its polyline's error allows to be told from nothing, and a
//! face with more than [`MOST_PER_FACE`] points. On a sphere loops that
//! all run clockwise are the sphere less some caps, which is a region.
//!
//! **How.** Each edge is followed in a few steps (a line in one on a
//! plane, else in four; a circle or an ellipse in steps of at most π/16; a
//! spline in sixteen per edge), each point projected onto the surface
//! (from the one before on a freeform surface, checked against the point,
//! else from nothing) and moved by whole periods to the one before it.
//! The middle of every step is projected too: how far it is from the
//! chord is the polyline's error. Crossings are proper crossings of two
//! steps whose ends lie on either side of the other step's line by more
//! than the errors of both loops and ten tolerances read in parameters at
//! the smallest scale of the surface over the face (the shortest of
//! `∂u`, `∂v` at the points): loops that touch, or come within that,
//! cross nothing here. Steps are swept in order of their lowest `u`, so a
//! face of `n` steps costs about `n log n` and the pairs that overlap.

use crate::{Body, Coedge, Face, FaceId};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Tolerance, Vec3};
use std::f64::consts::PI;

/// Most points one face is followed at; a larger face is passed.
pub const MOST_PER_FACE: usize = 1 << 15;
/// Steps of a spline edge.
const SPLINE_STEPS: usize = 16;
/// Steps of a line on a curved surface.
const LINE_STEPS: usize = 4;
/// Largest angle of a circle's or an ellipse's step.
const CONIC_STEP: f64 = PI / 16.0;
/// Most pairs of steps looked at for a crossing; a face that needs more
/// is not held to it.
const MOST_PAIRS: usize = 1 << 22;
/// Most steps of a conic.
const MOST_CONIC_STEPS: usize = 64;

/// One loop in the surface's parameters.
struct Ring {
    points: Vec<[f64; 2]>,
    /// How far the polyline may be from the curves, in parameters.
    error: f64,
    low: [f64; 2],
    high: [f64; 2],
}

/// Why the loops of `face` do not bound a region of its surface, or `None`
/// when they do or this test cannot tell (see the module).
pub fn region_fault(body: &Body, face: &Face, tol: &Tolerance) -> Option<String> {
    if face.loops.is_empty() {
        return None;
    }
    let surface = &face.surface;
    let periods = [surface.period_u(), surface.period_v()];
    if periods.iter().all(Option::is_some) {
        return None;
    }
    let freeform = matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_));
    if freeform
        && (craft3d_geom::closure::closes(surface).iter().any(|c| *c)
            || !craft3d_geom::pole::poles(surface).is_empty())
    {
        return None;
    }
    let near = 10.0 * body.working_tolerance(tol.linear);
    let mut chart = Chart {
        surface,
        periods,
        freeform,
        near,
        last: None,
        scale: f64::INFINITY,
    };
    let mut rings = Vec::with_capacity(face.loops.len());
    let mut total = 0_usize;
    for id in &face.loops {
        let lp = body.loop_(*id)?;
        let ring = follow(body, &lp.coedges, &mut chart, &mut total)?;
        rings.push(ring);
    }
    // Areas with the face's sense: positive for a loop that runs
    // counter-clockwise about the outward normal.
    let sense = if face.reversed { -1.0 } else { 1.0 };
    let mut areas = Vec::with_capacity(rings.len());
    for ring in &rings {
        let area = signed_area(&ring.points) * sense;
        let perimeter = perimeter(&ring.points);
        if !(area.is_finite() && area.abs() > 2.0 * perimeter * ring.error) {
            return None;
        }
        areas.push(area);
    }
    let outer: Vec<usize> = (0..areas.len())
        .filter(|i| areas.get(*i).is_some_and(|a| *a > 0.0))
        .collect();
    let largest = areas
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, _)| i)?;
    match outer.as_slice() {
        [] if matches!(surface, Surface::Sphere(_)) => {}
        [] => {
            return Some(format!(
                "every loop runs clockwise about the outward normal (areas {}): the face would be everything outside them",
                list(&areas)
            ));
        }
        [o] if *o == largest => {}
        [o] => {
            return Some(format!(
                "its counter-clockwise loop {o} encloses less than its hole {largest} (areas {})",
                list(&areas)
            ));
        }
        _ => {
            return Some(format!(
                "{} of its loops run counter-clockwise about the outward normal (areas {}): a face has one outer loop",
                outer.len(),
                list(&areas)
            ));
        }
    }
    // Margin of a crossing, in parameters.
    let errors: Vec<f64> = rings.iter().map(|r| r.error).collect();
    let reach = near / chart.scale.max(f64::MIN_POSITIVE);
    let margin_of = |i: usize, j: usize| {
        let e = |k: usize| errors.get(k).copied().unwrap_or(0.0);
        e(i) + e(j) + reach
    };
    // Holes moved by whole periods to lie over the outer loop.
    if let [o] = outer.as_slice() {
        let o = *o;
        let (lo, hi) = rings.get(o).map(|r| (r.low, r.high))?;
        let outer_points = rings.get(o).map(|r| r.points.clone())?;
        for (k, ring) in rings.iter_mut().enumerate() {
            if k == o {
                continue;
            }
            for (d, period) in periods.iter().enumerate() {
                let Some(p) = period else { continue };
                let (Some(l), Some(h), Some(rl), Some(rh)) =
                    (lo.get(d), hi.get(d), ring.low.get(d), ring.high.get(d))
                else {
                    continue;
                };
                // The whole periods that bring its middle nearest the
                // outer loop's, or one either side of that when the outer
                // loop winds around its point only there (an outer loop
                // that spans nearly a period).
                let nearest = ((0.5 * (l + h) - 0.5 * (rl + rh)) / p).round();
                let point = ring.points.first().copied();
                let wound_at = |turns: f64| {
                    point.map_or(0, |q| {
                        let mut q = q;
                        if let Some(x) = q.get_mut(d) {
                            *x += turns * p;
                        }
                        winding(q, &outer_points)
                    })
                };
                let turns = [nearest, nearest - 1.0, nearest + 1.0]
                    .into_iter()
                    .find(|t| wound_at(*t) != 0)
                    .unwrap_or(nearest);
                if turns != 0.0 {
                    ring.shift(d, turns * p);
                }
            }
        }
        // Each hole wound once by the outer loop, by no other hole.
        if !matches!(surface, Surface::Plane(_)) {
            for (k, ring) in rings.iter().enumerate() {
                if k == o {
                    continue;
                }
                let clear = ring.points.iter().find(|p| {
                    rings.iter().enumerate().all(|(j, other)| {
                        j == k
                            || other.box_distance(**p) > margin_of(j, k)
                            || distance(**p, &other.points) > margin_of(j, k)
                    })
                });
                let Some(point) = clear else { continue };
                let wound: i64 = rings
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .map(|(_, other)| {
                        if other.box_distance(*point) > 0.0 {
                            0
                        } else {
                            winding(*point, &other.points)
                        }
                    })
                    .sum();
                if wound != 1 {
                    let each: Vec<String> = rings
                        .iter()
                        .map(|other| winding(*point, &other.points).to_string())
                        .collect();
                    return Some(format!(
                        "its hole, loop {k}, lies outside its outer loop {o} or inside another hole (wound {wound} times at ({:.6}, {:.6}) of the {}; by each loop {}; areas {}; boxes {})",
                        point[0],
                        point[1],
                        surface.kind(),
                        each.join(", "),
                        list(&areas),
                        rings
                            .iter()
                            .map(|r| format!(
                                "[{:.4}, {:.4}]x[{:.4}, {:.4}]",
                                r.low[0], r.high[0], r.low[1], r.high[1]
                            ))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                }
            }
        }
    }
    crossing(&rings, &margin_of).map(|(i, j)| {
        if i == j {
            format!("its loop {i} crosses itself")
        } else {
            format!("its loops {i} and {j} cross")
        }
    })
}

/// Every face of `body` whose loops do not bound a region, with why.
pub fn regionless(body: &Body, tol: &Tolerance) -> Vec<(FaceId, String)> {
    let mut found = Vec::new();
    for (i, face) in body.faces.iter().enumerate() {
        if let Some(why) = region_fault(body, face, tol) {
            found.push((FaceId(u32::try_from(i).unwrap_or(u32::MAX)), why));
        }
    }
    found
}

fn list(areas: &[f64]) -> String {
    areas
        .iter()
        .map(|a| format!("{a:.3e}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Points of the surface to parameters, each moved by whole periods to the
/// one before.
struct Chart<'a> {
    surface: &'a Surface,
    periods: [Option<f64>; 2],
    freeform: bool,
    near: f64,
    last: Option<[f64; 2]>,
    /// The shortest of `∂u`, `∂v` at the points so far.
    scale: f64,
}

impl Chart<'_> {
    fn at(&mut self, p: Vec3) -> Option<[f64; 2]> {
        let found = match (self.freeform, self.last) {
            (true, Some([u, v])) => self
                .surface
                .project_from(p, (u, v))
                .ok()
                .filter(|(u, v)| self.surface.eval(*u, *v).distance(p) <= self.near)
                .or_else(|| self.surface.project(p).ok()),
            _ => self.surface.project(p).ok(),
        };
        let (u, v) = found?;
        let off = self.surface.eval(u, v).distance(p);
        if off.is_nan() || off > self.near {
            return None;
        }
        // Poles and apexes: the parameter around them is not defined.
        match self.surface {
            Surface::Sphere(s) if s.radius() * v.cos() <= self.near => return None,
            Surface::Cone(c) if self.surface.eval(0.0, c.apex_v()).distance(p) <= self.near => {
                return None;
            }
            _ => {}
        }
        let (du, dv, _) = self.surface.derivatives(u, v);
        let shortest = du.length().min(dv.length());
        if shortest.is_finite() {
            self.scale = self.scale.min(shortest);
        }
        let mut uv = [u, v];
        if let Some(before) = self.last {
            for (d, period) in self.periods.iter().enumerate() {
                if let (Some(p), Some(x), Some(b)) = (period, uv.get_mut(d), before.get(d)) {
                    *x += ((b - *x) / p).round() * p;
                }
            }
        }
        self.last = Some(uv);
        (uv[0].is_finite() && uv[1].is_finite()).then_some(uv)
    }
}

/// A loop as a closed polyline in the chart; `None` where it cannot be
/// followed or winds around a period.
fn follow(
    body: &Body,
    coedges: &[Coedge],
    chart: &mut Chart<'_>,
    total: &mut usize,
) -> Option<Ring> {
    let planar = matches!(chart.surface, Surface::Plane(_));
    let mut points = Vec::new();
    let mut error = 0.0_f64;
    chart.last = None;
    let mut start: Option<[f64; 2]> = None;
    for coedge in coedges {
        let edge = body.edge(coedge.edge)?;
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
            return None;
        }
        let steps = match &edge.curve {
            Curve3::Line(_) if planar => 1,
            Curve3::Line(_) => LINE_STEPS,
            Curve3::Circle(_) | Curve3::Ellipse(_) => {
                let n = ((t1 - t0) / CONIC_STEP).ceil();
                if n.is_finite() {
                    (n.max(2.0) as usize).min(MOST_CONIC_STEPS)
                } else {
                    return None;
                }
            }
            Curve3::Nurbs(_) => SPLINE_STEPS,
        };
        *total = total.saturating_add(2 * steps);
        if *total > MOST_PER_FACE {
            return None;
        }
        let at = |k: f64| {
            let f = k / steps as f64;
            if coedge.reversed {
                t1 - (t1 - t0) * f
            } else {
                t0 + (t1 - t0) * f
            }
        };
        let mut before = chart.at(edge.curve.eval(at(0.0)))?;
        if start.is_none() {
            start = Some(before);
        }
        for k in 0..steps {
            points.push(before);
            let middle = chart.at(edge.curve.eval(at(k as f64 + 0.5)))?;
            let next = chart.at(edge.curve.eval(at(k as f64 + 1.0)))?;
            let chord = [0.5 * (before[0] + next[0]), 0.5 * (before[1] + next[1])];
            error = error.max(2.0 * (chord[0] - middle[0]).hypot(chord[1] - middle[1]));
            before = next;
        }
    }
    // Back at its start: a loop that went around a period is a band.
    let (Some(s), Some(end)) = (start, chart.last) else {
        return None;
    };
    for (d, period) in chart.periods.iter().enumerate() {
        if let (Some(p), Some(a), Some(b)) = (period, s.get(d), end.get(d))
            && (a - b).abs() > 0.5 * p
        {
            return None;
        }
    }
    if points.len() < 3 {
        return None;
    }
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for p in &points {
        low = [low[0].min(p[0]), low[1].min(p[1])];
        high = [high[0].max(p[0]), high[1].max(p[1])];
    }
    Some(Ring {
        points,
        error,
        low,
        high,
    })
}

impl Ring {
    /// How far `p` is from the box (0 inside it).
    fn box_distance(&self, p: [f64; 2]) -> f64 {
        let dx = (self.low[0] - p[0]).max(p[0] - self.high[0]).max(0.0);
        let dy = (self.low[1] - p[1]).max(p[1] - self.high[1]).max(0.0);
        dx.hypot(dy)
    }

    fn shift(&mut self, d: usize, by: f64) {
        for p in &mut self.points {
            if let Some(x) = p.get_mut(d) {
                *x += by;
            }
        }
        if let (Some(l), Some(h)) = (self.low.get_mut(d), self.high.get_mut(d)) {
            *l += by;
            *h += by;
        }
    }
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let Some(o) = points.first() else { return 0.0 };
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some((a[0] - o[0]) * (b[1] - o[1]) - (b[0] - o[0]) * (a[1] - o[1]))
        })
        .sum::<f64>()
        * 0.5
}

fn perimeter(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some((b[0] - a[0]).hypot(b[1] - a[1]))
        })
        .sum()
}

/// The distance from `p` to the closed polyline `points`.
fn distance(p: [f64; 2], points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .filter_map(|i| {
            let (a, b) = (points.get(i)?, points.get((i + 1) % n)?);
            Some(to_segment(p, *a, *b))
        })
        .fold(f64::INFINITY, f64::min)
}

fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a[0] + t * dx - p[0]).hypot(a[1] + t * dy - p[1])
}

/// How many times the closed polyline `points` winds around `p`
/// anticlockwise.
fn winding(p: [f64; 2], points: &[[f64; 2]]) -> i64 {
    let n = points.len();
    let mut w = 0_i64;
    for i in 0..n {
        let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % n)) else {
            continue;
        };
        let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] {
            if b[1] > p[1] && side > 0.0 {
                w += 1;
            }
        } else if b[1] <= p[1] && side < 0.0 {
            w -= 1;
        }
    }
    w
}

/// Signed distance of `p` from the line through `a` and `b` (left
/// positive), or `None` for a step of no length.
fn side(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> Option<f64> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    (length > 0.0).then(|| (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / length)
}

/// Whether the steps `a b` and `c d` cross properly, each one's ends on
/// either side of the other's line by more than `margin`.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], margin: f64) -> bool {
    let apart = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => (x > margin && y < -margin) || (x < -margin && y > margin),
        _ => false,
    };
    apart(side(a, b, c), side(a, b, d)) && apart(side(c, d, a), side(c, d, b))
}

/// The first pair of loops (or a loop and itself) whose steps cross.
fn crossing(rings: &[Ring], margin_of: &dyn Fn(usize, usize) -> f64) -> Option<(usize, usize)> {
    // Every step: (lowest u, highest u, ring, index).
    let mut steps: Vec<(f64, f64, usize, usize)> = Vec::new();
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.points.len();
        for i in 0..n {
            let (Some(a), Some(b)) = (ring.points.get(i), ring.points.get((i + 1) % n)) else {
                continue;
            };
            steps.push((a[0].min(b[0]), a[0].max(b[0]), r, i));
        }
    }
    steps.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.2.cmp(&y.2)).then(x.3.cmp(&y.3)));
    let largest_margin = (0..rings.len())
        .flat_map(|i| (0..rings.len()).map(move |j| (i, j)))
        .map(|(i, j)| margin_of(i, j))
        .fold(0.0, f64::max);
    let point = |r: usize, i: usize| -> Option<([f64; 2], [f64; 2])> {
        let ring = rings.get(r)?;
        let n = ring.points.len();
        Some((*ring.points.get(i)?, *ring.points.get((i + 1) % n)?))
    };
    let mut active: Vec<(f64, usize, usize)> = Vec::new();
    let mut work = 0_usize;
    for &(low, high, r, i) in &steps {
        active.retain(|(h, _, _)| *h + largest_margin >= low);
        let (a, b) = point(r, i)?;
        work = work.saturating_add(active.len());
        if work > MOST_PAIRS {
            return None;
        }
        for &(_, s, j) in &active {
            if s == r {
                let n = rings.get(r).map_or(0, |x| x.points.len());
                // Steps that meet at a point of the loop.
                if j == i || (j + 1) % n == i || (i + 1) % n == j {
                    continue;
                }
            }
            let (c, d) = point(s, j)?;
            if cross(a, b, c, d, margin_of(r, s)) {
                return Some((s.min(r), s.max(r)));
            }
        }
        active.push((high, r, i));
    }
    None
}
