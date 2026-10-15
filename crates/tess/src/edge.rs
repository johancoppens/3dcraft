//! Edges to points, once per edge: every face that uses an edge uses these
//! very points, which is what makes the mesh of a solid watertight.
//!
//! The points are first chosen for the edge alone: within the chordal
//! tolerance of its curve. A face may need some chords shorter than that
//! (`face.rs`, `Boundary::chords_to_refine`); [`EdgeSamples::split`] cuts
//! those in two, for every face that uses the edge.

use crate::face::Chord;
use crate::{TessError, TessParams};
use craft3d_geom::Curve3;
use craft3d_math::Vec3;
use craft3d_topo::Body;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, TAU};

/// Segments for an arc of `sweep` radians on a circle of `radius` so that
/// no chord is further than the tolerance from the arc: a chord over a
/// half-angle `a` sags by `r (1 - cos a)`. No segment spans more than a
/// third of a turn, so that a full circle stays a polygon and a half
/// circle does not collapse onto its diameter at a coarse tolerance.
pub(crate) fn arc_segments(radius: f64, sweep: f64, params: &TessParams) -> Result<u32, TessError> {
    let sag = (params.chordal_tolerance / radius).min(1.0);
    let half_angle = (1.0 - sag).acos();
    // With an angle criterion, no chord turns the tangent further than
    // that, unless the chord would be shorter than the floor (an angle of
    // `floor / radius` is the chord's length at that radius).
    let largest = match params.max_angle {
        Some(angle) => {
            let floor = crate::ANGLE_FLOOR * params.chordal_tolerance / radius;
            (2.0 * half_angle).min(angle.max(floor))
        }
        None => 2.0 * half_angle,
    };
    let needed = (sweep.abs() / largest).ceil();
    let minimum = (sweep.abs() / (TAU / 3.0) - 1e-9).ceil().max(1.0);
    let needed = needed.max(minimum);
    let limit = f64::from(params.max_segments) * 8.0;
    if !needed.is_finite() || needed > limit {
        return Err(TessError::TooFine {
            patch: 0,
            needed_segments: needed,
            max_segments: params.max_segments,
        });
    }
    // In range by the check above.
    Ok(needed as u32)
}

/// How deep a piece of a curve is halved before giving up.
const MAX_DEPTH: u32 = 40;

/// The shortest piece of a spline edge between two knots that is a piece
/// of its own, as a fraction of the edge's parameter range. A fraction and
/// not a length: the same edge at another size has the same pieces.
const SHORTEST_PIECE: f64 = 1e-9;

/// Points along `curve` from `t0` to `t1`, between the given end points,
/// by halving pieces until each chord is within the tolerance.
///
/// The criterion: on the piece from `C(a)` to `C(b)` the chord point at
/// fraction `s` is compared with `C(a + s (b − a))`. That distance is an
/// upper bound on the distance from the chord point to the curve (the
/// curve point it is measured to is only one candidate), so a piece that
/// passes at `s` is within tolerance there. It is checked at `s` = ¼, ½
/// and ¾: the middle is where a simple arc sags most (exactly so for a
/// parabola, i.e. to second order for any smooth piece), and the quarter
/// points catch an S-shaped piece whose middle happens to lie on the
/// chord. The starting pieces are at most a quarter turn of an ellipse or
/// one knot span of a spline, so each is a simple arc of one polynomial
/// or rational piece.
fn adaptive(
    curve: &Curve3,
    breaks: &[f64],
    ends: (Vec3, Vec3),
    index: usize,
    params: &TessParams,
) -> Result<(Vec<Vec3>, Vec<f64>), TessError> {
    let tolerance = params.chordal_tolerance;
    let limit = (params.max_segments as usize).saturating_mul(8);
    let too_fine = |needed: usize| TessError::TooFine {
        patch: index,
        needed_segments: needed as f64,
        max_segments: params.max_segments,
    };
    if breaks.len() < 2 || breaks.len() - 1 > limit {
        return Err(too_fine(breaks.len()));
    }
    // The pieces still to do, last first; each with its end points.
    let last = breaks.len() - 1;
    let mut knots: Vec<(f64, Vec3)> = Vec::with_capacity(breaks.len());
    for (k, &t) in breaks.iter().enumerate() {
        let point = if k == 0 {
            ends.0
        } else if k == last {
            ends.1
        } else {
            curve.eval(t)
        };
        knots.push((t, point));
    }
    let mut stack: Vec<(f64, Vec3, f64, Vec3, u32)> = knots
        .iter()
        .zip(knots.iter().skip(1))
        .rev()
        .map(|(a, b)| (a.0, a.1, b.0, b.1, 0))
        .collect();
    let mut points = vec![ends.0];
    let mut at: Vec<f64> = breaks.first().copied().into_iter().collect();
    while let Some((ta, pa, tb, pb, depth)) = stack.pop() {
        let fits = [0.25, 0.5, 0.75].iter().all(|&s| {
            let on_curve = curve.eval(ta + (tb - ta) * s);
            on_curve.distance(pa.lerp(pb, s)) <= tolerance
        }) && params
            .angle_for(pa.distance(pb))
            .is_none_or(|angle| turns_within(curve, ta, tb, angle));
        if fits {
            points.push(pb);
            at.push(tb);
            continue;
        }
        if depth >= MAX_DEPTH || points.len() + stack.len() >= limit {
            return Err(too_fine(points.len() + stack.len() + 1));
        }
        let tm = 0.5 * (ta + tb);
        let pm = curve.eval(tm);
        if !pm.is_finite() {
            return Err(TessError::InvalidBody(format!(
                "edge {index}: its curve has no finite point at parameter {tm}"
            )));
        }
        stack.push((tm, pm, tb, pb, depth + 1));
        stack.push((ta, pa, tm, pm, depth + 1));
    }
    Ok((points, at))
}

/// Whether the tangent of `curve` turns by at most `angle` from `ta` to
/// `tb`: between the two ends and between each end and the middle (a
/// piece that turns one way and back again has ends that agree). Where
/// the curve has no tangent (a derivative of zero) nothing is known, and
/// nothing is asked for. The ends are taken a millionth of the piece
/// inside it, so that at a knot where the curve has a kink the tangent is
/// the piece's own and not the next one's.
fn turns_within(curve: &Curve3, ta: f64, tb: f64, angle: f64) -> bool {
    let tangent = |t: f64| curve.derivatives(t).1;
    let inside = 1e-6 * (tb - ta);
    let (a, m, b) = (
        tangent(ta + inside),
        tangent(0.5 * (ta + tb)),
        tangent(tb - inside),
    );
    [(a, b), (a, m), (m, b)].into_iter().all(|(x, y)| {
        crate::angle_between(x, y).is_none_or(|turn| !crate::turns_beyond(turn, angle))
    })
}

/// The points of every edge of a body, from its start to its end vertex,
/// and the parameters of its curve they were taken at.
#[derive(Debug, Default)]
pub(crate) struct EdgeSamples {
    /// Per edge. The ends are the vertices' own positions, so edges
    /// meeting at a vertex agree.
    pub points: Vec<Vec<Vec3>>,
    /// Per edge, one parameter per point, increasing.
    at: Vec<Vec<f64>>,
    /// Per edge, how many points it had before any chord was cut.
    first: Vec<usize>,
}

/// How many points an edge may grow to by cutting chords, given how many
/// it had for its own curve: sixteen times as many (every chord cut four
/// times: twice is what a loop close to a pole may need, four times what
/// a strip a thousandth of the tolerance wide needs along its whole
/// length, each cut bringing a chord four times nearer to its curve) and
/// 32 more (one chord cut again and again, which is what an edge needs
/// where another comes near). It keeps the work on a body that makes no
/// sense within a multiple of the work on one that does: there, cutting
/// cures nothing and every face would ask for every chord in every round.
fn most_points(first: usize) -> usize {
    first.saturating_mul(16).saturating_add(32)
}

impl EdgeSamples {
    /// Cuts each of the `wanted` chords in two, at the point of the curve
    /// half way between the parameters of its ends. Returns the edges
    /// that changed, in order.
    ///
    /// A chord is left as it is when it cannot be cut: when it does not
    /// exist, when the numbers cannot tell the new point from its ends,
    /// or when the edge has as many points as it may have
    /// ([`most_points`], and what `params.max_segments` allows).
    pub(crate) fn split(&mut self, body: &Body, wanted: &[Chord], params: &TessParams) -> Vec<u32> {
        let limit = (params.max_segments as usize).saturating_mul(8);
        let mut by_edge: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &(edge, chord) in wanted {
            by_edge.entry(edge).or_default().push(chord);
        }
        let mut changed = Vec::new();
        for (index, mut chords) in by_edge {
            chords.sort_unstable();
            chords.dedup();
            let (Some(edge), Some(points), Some(at)) = (
                body.edges.get(index as usize),
                self.points.get_mut(index as usize),
                self.at.get_mut(index as usize),
            ) else {
                continue;
            };
            let first = self.first.get(index as usize).copied().unwrap_or(0);
            let mut room = most_points(first).min(limit).saturating_sub(points.len());
            let mut new_points = Vec::with_capacity(points.len() + chords.len());
            let mut new_at = Vec::with_capacity(points.len() + chords.len());
            let mut cut = false;
            let mut next = chords.iter().copied().peekable();
            for (k, (&p, &t)) in points.iter().zip(at.iter()).enumerate() {
                new_points.push(p);
                new_at.push(t);
                while next.peek().is_some_and(|&chord| (chord as usize) < k) {
                    next.next();
                }
                if next.peek().is_none_or(|&chord| chord as usize != k) {
                    continue;
                }
                let (Some(&q), Some(&u)) = (points.get(k + 1), at.get(k + 1)) else {
                    continue;
                };
                let middle = 0.5 * (t + u);
                let point = edge.curve.eval(middle);
                let apart =
                    t < middle && middle < u && point.is_finite() && point != p && point != q;
                if room > 0 && apart {
                    new_points.push(point);
                    new_at.push(middle);
                    room -= 1;
                    cut = true;
                }
            }
            if cut {
                *points = new_points;
                *at = new_at;
                changed.push(index);
            }
        }
        changed
    }
}

/// The points of every edge, each within the chordal tolerance of its
/// curve.
#[cfg(test)]
pub(crate) fn edge_points(body: &Body, params: &TessParams) -> Result<EdgeSamples, TessError> {
    edge_points_of(body, params, None)
}

/// [`edge_points`] for the edges `wanted` marks (all when `None`); the
/// others get no points at all, and nothing about them is checked.
pub(crate) fn edge_points_of(
    body: &Body,
    params: &TessParams,
    wanted: Option<&[bool]>,
) -> Result<EdgeSamples, TessError> {
    let mut all = EdgeSamples {
        points: Vec::with_capacity(body.edges.len()),
        at: Vec::with_capacity(body.edges.len()),
        first: Vec::with_capacity(body.edges.len()),
    };
    for (index, edge) in body.edges.iter().enumerate() {
        if wanted.is_some_and(|wanted| !wanted.get(index).copied().unwrap_or(false)) {
            all.first.push(0);
            all.points.push(Vec::new());
            all.at.push(Vec::new());
            continue;
        }
        let (Some(start), Some(end)) = (body.vertex(edge.start), body.vertex(edge.end)) else {
            return Err(TessError::InvalidBody(format!(
                "edge {index} refers to a vertex that does not exist"
            )));
        };
        let (t0, t1) = edge.range;
        if !(t0.is_finite() && t1.is_finite() && t0 < t1) {
            return Err(TessError::InvalidBody(format!(
                "edge {index} has an empty, reversed or non-finite parameter range"
            )));
        }
        if !(start.point.is_finite() && end.point.is_finite()) {
            return Err(TessError::InvalidBody(format!(
                "edge {index} ends at a vertex that is not finite"
            )));
        }
        let ends = (start.point, end.point);
        let with_edge = |error: TessError| match error {
            TessError::TooFine {
                needed_segments,
                max_segments,
                ..
            } => TessError::TooFine {
                patch: index,
                needed_segments,
                max_segments,
            },
            other => other,
        };
        let (points, at) = match &edge.curve {
            Curve3::Line(_) => (vec![ends.0, ends.1], vec![t0, t1]),
            Curve3::Circle(c) => {
                let segments = arc_segments(c.radius(), t1 - t0, params).map_err(with_edge)?;
                let mut points = Vec::with_capacity(segments as usize + 1);
                let mut at = Vec::with_capacity(segments as usize + 1);
                points.push(ends.0);
                at.push(t0);
                for k in 1..segments {
                    let t = t0 + (t1 - t0) * f64::from(k) / f64::from(segments);
                    points.push(edge.curve.eval(t));
                    at.push(t);
                }
                points.push(ends.1);
                at.push(t1);
                (points, at)
            }
            Curve3::Ellipse(_) => {
                let pieces = ((t1 - t0) / FRAC_PI_2).ceil();
                let limit = f64::from(params.max_segments) * 8.0;
                if !(pieces >= 1.0 && pieces <= limit) {
                    return Err(TessError::TooFine {
                        patch: index,
                        needed_segments: pieces,
                        max_segments: params.max_segments,
                    });
                }
                // In range by the check above.
                let pieces = pieces as u32;
                let breaks: Vec<f64> = (0..=pieces)
                    .map(|k| t0 + (t1 - t0) * f64::from(k) / f64::from(pieces))
                    .collect();
                adaptive(&edge.curve, &breaks, ends, index, params)?
            }
            Curve3::Nurbs(c) => {
                let mut breaks = vec![t0];
                // A knot is a break when it is inside the range by more
                // than rounding. One that is not (an edge cut from a spline
                // at a knot, whose range starts a few units in the last
                // place before it) would make a piece too short to have a
                // length: two points of the edge that no face can tell
                // apart. Left out, it lies within `SHORTEST_PIECE` of an
                // end of a piece, where it cannot bend the piece away from
                // what is measured at its quarters.
                let shortest = SHORTEST_PIECE * (t1 - t0);
                for &knot in c.knots() {
                    let after_last = breaks.last().is_some_and(|last| knot - *last > shortest);
                    if after_last && t1 - knot > shortest {
                        breaks.push(knot);
                    }
                }
                breaks.push(t1);
                adaptive(&edge.curve, &breaks, ends, index, params)?
            }
        };
        if points.iter().any(|p| !p.is_finite()) {
            return Err(TessError::InvalidBody(format!(
                "edge {index}: its curve has points that are not finite"
            )));
        }
        all.first.push(points.len());
        all.points.push(points);
        all.at.push(at);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_geom::{Ellipse, NurbsCurve};
    use craft3d_math::Frame;
    use craft3d_topo::BodyKind;
    use std::f64::consts::PI;

    fn params(tolerance: f64) -> TessParams {
        TessParams {
            chordal_tolerance: tolerance,
            ..TessParams::default()
        }
    }

    #[test]
    fn arc_segments_keep_the_chord_within_tolerance() {
        for (radius, tol) in [(1.0, 0.1), (1.0, 1e-3), (250.0, 0.01), (1e-3, 1e-6)] {
            let params = TessParams {
                chordal_tolerance: tol,
                ..TessParams::default()
            };
            let n = arc_segments(radius, TAU, &params).unwrap();
            let sag = |n: u32| radius * (1.0 - (PI / f64::from(n)).cos());
            assert!(
                sag(n) <= tol * (1.0 + 1e-12),
                "r {radius} tol {tol}: n = {n}"
            );
            // One segment fewer would not do (unless at the minimum).
            assert!(
                n == 3 || sag(n - 1) > tol,
                "r {radius} tol {tol}: n = {n} is more than needed"
            );
        }
        // A tolerance larger than the radius still gives a polygon.
        let coarse = TessParams {
            chordal_tolerance: 10.0,
            ..TessParams::default()
        };
        assert_eq!(arc_segments(1.0, TAU, &coarse).unwrap(), 3);
        let too_fine = TessParams {
            chordal_tolerance: 1e-15,
            max_segments: 16,
            ..TessParams::default()
        };
        assert!(matches!(
            arc_segments(1.0, TAU, &too_fine),
            Err(TessError::TooFine { .. })
        ));
    }

    #[test]
    fn a_coarse_arc_never_spans_more_than_a_third_of_a_turn() {
        let coarse = params(10.0);
        assert_eq!(arc_segments(1.0, PI, &coarse).unwrap(), 2);
        assert_eq!(arc_segments(1.0, FRAC_PI_2, &coarse).unwrap(), 1);
        assert_eq!(arc_segments(1.0, 1.5 * PI, &coarse).unwrap(), 3);
    }

    /// The largest distance from sampled chord points to the curve, the
    /// curve being a polyline far finer than the chords.
    fn worst_chord_error(curve: &Curve3, range: (f64, f64), points: &[Vec3]) -> f64 {
        let fine: Vec<Vec3> = (0..=4_000)
            .map(|i| curve.eval(range.0 + (range.1 - range.0) * f64::from(i) / 4_000.0))
            .collect();
        let mut worst = 0.0_f64;
        for pair in points.windows(2) {
            for k in 1..8 {
                let p = pair[0].lerp(pair[1], f64::from(k) / 8.0);
                // Distance to the fine polyline, not to its corners: those
                // are further apart than the tolerances tested.
                let nearest = fine
                    .windows(2)
                    .map(|w| {
                        let d = w[1] - w[0];
                        let t = ((p - w[0]).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                        (w[0] + d * t).distance(p)
                    })
                    .fold(f64::INFINITY, f64::min);
                worst = worst.max(nearest);
            }
        }
        worst
    }

    fn one_edge(curve: Curve3, range: (f64, f64)) -> Body {
        let mut body = Body::new(BodyKind::Sheet);
        let a = body.add_vertex(curve.eval(range.0));
        let b = body.add_vertex(curve.eval(range.1));
        body.add_edge(curve, range, a, b);
        body
    }

    fn wiggle(scale: f64) -> Curve3 {
        let ctrl = [
            (0.0, 0.0, 0.0),
            (1.0, 2.0, 0.5),
            (2.0, -1.0, 1.0),
            (3.5, 0.5, -0.5),
            (4.0, 2.0, 0.0),
            (5.0, 0.0, 1.0),
            (6.0, 1.0, 0.0),
        ]
        .map(|(x, y, z)| Vec3::new(x, y, z) * scale)
        .to_vec();
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0];
        let weights = vec![1.0, 0.7, 1.3, 1.0, 2.0, 0.5, 1.0];
        Curve3::Nurbs(NurbsCurve::new(3, knots, ctrl, weights).unwrap())
    }

    /// One cubic span in the shape of an S: point-symmetric about its
    /// middle, which therefore lies on the chord between its ends.
    fn ess(scale: f64) -> Curve3 {
        let ctrl = [(0.0, 0.0), (1.0, 1.5), (2.0, -1.5), (3.0, 0.0)]
            .map(|(x, y)| Vec3::new(x, y, 0.0) * scale)
            .to_vec();
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        Curve3::Nurbs(NurbsCurve::polynomial(3, knots, ctrl).unwrap())
    }

    /// A chord whose middle is on the curve is not thereby within
    /// tolerance: the quarter points are looked at too.
    #[test]
    fn an_s_shaped_span_is_not_taken_for_a_straight_line() {
        let curve = ess(1.0);
        assert!(curve.eval(0.5).distance(Vec3::new(1.5, 0.0, 0.0)) < 1e-15);
        let body = one_edge(curve.clone(), (0.0, 1.0));
        let points = &edge_points(&body, &params(0.3)).unwrap().points[0];
        assert!(points.len() > 2, "the S became one chord");
        assert!(worst_chord_error(&curve, (0.0, 1.0), points) <= 0.3);
    }

    #[test]
    fn ellipses_and_splines_stay_within_tolerance_without_wasting_points() {
        for scale in [1e-3, 1.0, 1e6] {
            let frame = Frame::new(
                Vec3::new(1.0, -2.0, 0.5) * scale,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap();
            let ellipse = Curve3::Ellipse(Ellipse::new(frame, 3.0 * scale, 0.8 * scale).unwrap());
            let cases = [
                (ellipse.clone(), (0.0, TAU)),
                (ellipse, (0.4, 2.9)),
                (wiggle(scale), (0.0, 1.0)),
                (wiggle(scale), (0.1, 0.77)),
                // The middle of this one lies on its chord.
                (ess(scale), (0.0, 1.0)),
            ];
            for (curve, range) in cases {
                let body = one_edge(curve.clone(), range);
                let mut previous = 0;
                for rel in [1e-1, 1e-2, 1e-3, 1e-4] {
                    let tolerance = rel * scale;
                    let points = &edge_points(&body, &params(tolerance)).unwrap().points[0];
                    let what = format!("{} at scale {scale}, tolerance {rel}", curve.kind());
                    assert_eq!(points[0], body.vertices[0].point, "{what}");
                    assert_eq!(*points.last().unwrap(), body.vertices[1].point, "{what}");
                    let worst = worst_chord_error(&curve, range, points);
                    assert!(
                        worst <= tolerance * (1.0 + 1e-9),
                        "{what}: a chord is {worst:e} from the curve"
                    );
                    // Halving stops as soon as a piece fits, so the worst
                    // chord is not far below the tolerance either.
                    assert!(
                        points.len() <= 3 || worst > tolerance / 40.0,
                        "{what}: worst chord only {worst:e} with {} points",
                        points.len()
                    );
                    assert!(points.len() >= previous, "{what}");
                    previous = points.len();
                }
            }
        }
    }

    #[test]
    fn chords_are_cut_in_two_on_the_curve_and_no_further_than_allowed() {
        let frame = Frame::new(
            Vec3::new(1.0, -2.0, 0.5),
            Vec3::new(0.3, -0.4, 1.0),
            Vec3::new(1.0, 0.2, 0.0),
        )
        .unwrap();
        let ellipse = Curve3::Ellipse(Ellipse::new(frame, 3.0, 0.8).unwrap());
        for (curve, range) in [(ellipse, (0.4, 2.9)), (wiggle(1.0), (0.1, 0.77))] {
            let mut body = one_edge(curve.clone(), range);
            // A second edge, which nothing is asked of.
            let (a, b) = (body.vertices[0].point, body.vertices[1].point);
            let (start, end) = (craft3d_topo::VertexId(0), craft3d_topo::VertexId(1));
            let line = Curve3::Line(craft3d_geom::Line::through(a, b).unwrap());
            body.add_edge(line, (0.0, a.distance(b)), start, end);
            let p = params(1e-2);
            let mut samples = edge_points(&body, &p).unwrap();
            let before = samples.points.clone();
            let n = before[0].len();
            assert!(n >= 5, "{n}");

            // Chords 1 and the last, each named twice, and one that does
            // not exist.
            let last = (n - 2) as u32;
            let wanted = [(0, last), (0, 1), (0, 1), (0, 999), (7, 0), (0, last)];
            assert_eq!(samples.split(&body, &wanted, &p), vec![0]);
            let after = &samples.points[0];
            assert_eq!(after.len(), n + 2);
            assert_eq!(samples.points[1], before[1]);
            // The old points in their order, a new one after the second
            // and one before the last.
            let mut expected = before[0].clone();
            let new_last = after[n];
            let new_first = after[2];
            expected.insert(n - 1, new_last);
            expected.insert(2, new_first);
            assert_eq!(*after, expected);
            // The new points are on the curve, between their neighbours
            // along it: where the curve is half way between them in its
            // parameter.
            assert_eq!(samples.at[0].len(), after.len());
            for k in [2, n] {
                let (t0, t, t1) = (samples.at[0][k - 1], samples.at[0][k], samples.at[0][k + 1]);
                assert_eq!(t, 0.5 * (t0 + t1));
                assert_eq!(after[k], curve.eval(t));
            }
            assert!(samples.at[0].windows(2).all(|w| w[0] < w[1]));
            assert!(worst_chord_error(&curve, range, after) <= 1e-2);

            // A straight edge can be cut too (on a freeform surface it
            // may have to be): the new point is its middle.
            assert_eq!(samples.split(&body, &[(1, 0)], &p), vec![1]);
            assert_eq!(samples.points[1].len(), 3);
            assert!(samples.points[1][1].distance(a.lerp(b, 0.5)) < 1e-12);

            // Asking for every chord again and again stops at sixteen
            // times the first number of points and 32 more.
            let mut rounds = 0;
            loop {
                let all: Vec<Chord> = (0..samples.points[0].len() as u32)
                    .map(|k| (0, k))
                    .collect();
                if samples.split(&body, &all, &p).is_empty() {
                    break;
                }
                rounds += 1;
                assert!(rounds < 100);
            }
            assert_eq!(samples.points[0].len(), 16 * n + 32);
            assert_eq!(samples.points[0][0], before[0][0]);
            assert_eq!(samples.points[0].last(), before[0].last());
            // And at what `max_segments` allows, when that is less.
            let tight = TessParams {
                chordal_tolerance: 1e-2,
                max_segments: 2,
                ..TessParams::default()
            };
            let mut samples = edge_points(&one_edge(curve.clone(), range), &p).unwrap();
            let all: Vec<Chord> = (0..n as u32).map(|k| (0, k)).collect();
            samples.split(&body, &all, &tight);
            assert_eq!(samples.points[0].len(), n.max(16));
        }
    }

    #[test]
    fn impossible_tolerances_and_broken_edges_are_errors() {
        let frame = Frame::WORLD;
        let ellipse = Curve3::Ellipse(Ellipse::new(frame, 3.0, 0.8).unwrap());
        let too_fine = TessParams {
            chordal_tolerance: 1e-13,
            max_segments: 8,
            ..TessParams::default()
        };
        for curve in [ellipse.clone(), wiggle(1.0)] {
            let range = if curve.period().is_some() {
                (0.0, TAU)
            } else {
                (0.0, 1.0)
            };
            let body = one_edge(curve, range);
            assert!(matches!(
                edge_points(&body, &too_fine),
                Err(TessError::TooFine { patch: 0, .. })
            ));
        }
        for range in [
            (1.0, 1.0),
            (2.0, 1.0),
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
        ] {
            let mut body = one_edge(ellipse.clone(), (0.0, 1.0));
            body.edges[0].range = range;
            assert!(
                matches!(
                    edge_points(&body, &params(1e-2)),
                    Err(TessError::InvalidBody(_))
                ),
                "{range:?}"
            );
        }
        // A sweep of a million turns asks for more pieces than allowed.
        let mut body = one_edge(ellipse, (0.0, 1.0));
        body.edges[0].range = (0.0, 1e7);
        assert!(matches!(
            edge_points(&body, &params(1e-2)),
            Err(TessError::TooFine { .. })
        ));
    }
}
