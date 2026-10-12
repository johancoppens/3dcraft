//! Edges to points, once per edge: every face that uses an edge uses these
//! very points, which is what makes the mesh of a solid watertight.

use crate::{TessError, TessParams};
use craft3d_geom::Curve3;
use craft3d_math::Vec3;
use craft3d_topo::Body;
use std::f64::consts::{FRAC_PI_2, TAU};

/// Segments for an arc of `sweep` radians on a circle of `radius` so that
/// no chord is further than the tolerance from the arc: a chord over a
/// half-angle `a` sags by `r (1 - cos a)`. No segment spans more than a
/// third of a turn, so that a full circle stays a polygon and a half
/// circle does not collapse onto its diameter at a coarse tolerance.
pub(crate) fn arc_segments(radius: f64, sweep: f64, params: &TessParams) -> Result<u32, TessError> {
    let sag = (params.chordal_tolerance / radius).min(1.0);
    let half_angle = (1.0 - sag).acos();
    let needed = (sweep.abs() / (2.0 * half_angle)).ceil();
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
) -> Result<Vec<Vec3>, TessError> {
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
    while let Some((ta, pa, tb, pb, depth)) = stack.pop() {
        let fits = [0.25, 0.5, 0.75].iter().all(|&s| {
            let on_curve = curve.eval(ta + (tb - ta) * s);
            on_curve.distance(pa.lerp(pb, s)) <= tolerance
        });
        if fits {
            points.push(pb);
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
    Ok(points)
}

/// The points of every edge, from its start to its end vertex. The ends
/// are the vertices' own positions, so edges meeting at a vertex agree.
pub(crate) fn edge_points(body: &Body, params: &TessParams) -> Result<Vec<Vec<Vec3>>, TessError> {
    let mut all = Vec::with_capacity(body.edges.len());
    for (index, edge) in body.edges.iter().enumerate() {
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
        let points = match &edge.curve {
            Curve3::Line(_) => vec![ends.0, ends.1],
            Curve3::Circle(c) => {
                let segments = arc_segments(c.radius(), t1 - t0, params).map_err(with_edge)?;
                let mut points = Vec::with_capacity(segments as usize + 1);
                points.push(ends.0);
                for k in 1..segments {
                    points.push(
                        edge.curve
                            .eval(t0 + (t1 - t0) * f64::from(k) / f64::from(segments)),
                    );
                }
                points.push(ends.1);
                points
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
                for &knot in c.knots() {
                    let after_last = breaks.last().is_some_and(|last| knot > *last);
                    if after_last && knot < t1 {
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
        all.push(points);
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
        let points = &edge_points(&body, &params(0.3)).unwrap()[0];
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
                    let points = &edge_points(&body, &params(tolerance)).unwrap()[0];
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
    fn impossible_tolerances_and_broken_edges_are_errors() {
        let frame = Frame::WORLD;
        let ellipse = Curve3::Ellipse(Ellipse::new(frame, 3.0, 0.8).unwrap());
        let too_fine = TessParams {
            chordal_tolerance: 1e-13,
            max_segments: 8,
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
