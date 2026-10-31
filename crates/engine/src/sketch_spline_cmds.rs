//! `sketch.spline`: a B-spline in a sketch, by its control points or
//! through given points; and the little B-spline arithmetic the engine
//! needs for it (the basis functions, for interpolating and for drawing).
//!
//! A sketch spline is a polynomial B-spline on clamped uniform knots whose
//! control points are point entities of the sketch (`craft3d_doc::Geometry`),
//! so the solver shapes it by its points. `through` is not a second kind
//! of spline: the control points of the spline that passes through the
//! given points are worked out here, once, and from then on it is a spline
//! like any other. With `live`, it is: a spline through points that is
//! fitted again at every regeneration (`craft3d_doc::Geometry::Interpolated`),
//! so points held by formulas move it.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::sketch_cmds::{PointArg, change, ids, sketch_ref, sketch_result};
use crate::typed::Typed;
use crate::{CommandSpec, Session};
use craft3d_doc::{
    EntityId, MAX_SPLINE_DEGREE, MAX_SPLINE_POINTS, Sketch, SketchError, clamped_uniform_knots,
};
use craft3d_math::Portable;
use craft3d_regen::curve_fit::{basis, interpolate};
use serde_json::{Value, json};

/// Most points a spline is drawn with.
const MAX_DRAWN_POINTS: usize = 4096;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("sketch.spline", "Sketch Spline", r#"{"sketch": id or name, "control": [point id or [x, y], ...], "through": [point id or [x, y], ...], "degree": integer = 3, "closed": boolean = false, "live": boolean = false, "construction": boolean = false}"#,
        "Adds a B-spline. Give either control: its control points in order (it starts at the first, ends at the last and leaves each along the leg to the next; the points in between pull it without lying on it), or through: points it passes through, from which the control points are worked out once (the first and last are shared as given, the ones in between become new control points). An existing point (\"e1\") shares an end with a line, an arc or another spline, which is how a spline closes a profile; a tangent constraint between the two then makes the corner smooth. degree is from 1 to 7 and needs degree + 1 points; closed brings the spline back to its first point. A profile takes splines that do not cross or touch themselves or other curves, extruded or revolved: a closed spline is a profile on its own (its wall is made in two faces, the second called F2:piece(1)/F2:side(e7)), and a spline of degree 1 is a polyline whose every leg sweeps a flat face (numbered the same way). Revolved, a spline has to stay clear of the axis (a line may lie on it). Returns the spline and its control points; move one with sketch.edit (entity and at). With live (only with through) the points given are the spline's own points, kept as they are, and the spline is fitted through them again at every regeneration: points held by formulas ([\"r * cos(30 deg)\", \"r * sin(30 deg)\"]) or by constraints move it, and the fit is the same as through gives once. A live spline takes pointOn and tangent as a formula curve does (see sketch.constrain); returns the spline and its through points.", doc: true, sketch_spline),
];

/// A sketch spline of `degree` on these control points, for evaluating.
pub(crate) struct SplineCurve<'a> {
    degree: usize,
    knots: Vec<f64>,
    control: &'a [(f64, f64)],
}

impl<'a> SplineCurve<'a> {
    /// `None` for a degree or a number of points a sketch spline cannot
    /// have.
    pub(crate) fn new(degree: u32, control: &'a [(f64, f64)]) -> Option<Self> {
        let knots = clamped_uniform_knots(degree, control.len());
        (!knots.is_empty()).then_some(Self {
            degree: usize::try_from(degree).ok()?,
            knots,
            control,
        })
    }

    /// The point at `u`, from 0 at the start to 1 at the end.
    pub(crate) fn point(&self, u: f64) -> Option<(f64, f64)> {
        let (span, values) = basis(self.degree, &self.knots, self.control.len(), u)?;
        let first = span.checked_sub(self.degree)?;
        let mut point = (0.0, 0.0);
        for (value, control) in values.iter().zip(self.control.get(first..=span)?) {
            point.0 += value * control.0;
            point.1 += value * control.1;
        }
        Some(point)
    }

    /// Points along the curve from start to end, so close together that
    /// the polyline through them stays within `tolerance` of the curve
    /// (as far as halving a chord shows: where its middle and quarter
    /// points are within tolerance, it is taken as flat), and never more
    /// than [`MAX_DRAWN_POINTS`]. Every knot span starts on a point.
    pub(crate) fn polyline(&self, tolerance: f64) -> Vec<(f64, f64)> {
        let off = |p: (f64, f64), a: (f64, f64), b: (f64, f64)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = dx.phypot(dy);
            if length > 0.0 {
                ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / length
            } else {
                (p.0 - a.0).phypot(p.1 - a.1)
            }
        };
        let spans = self.control.len().saturating_sub(self.degree).max(1);
        // Stretches still to draw, the last one first; each starts flat
        // enough only when halving it shows so.
        let pieces = spans.saturating_mul(4);
        let mut todo: Vec<(f64, f64, u32)> = (0..pieces)
            .rev()
            .map(|i| (i as f64 / pieces as f64, (i + 1) as f64 / pieces as f64, 0))
            .collect();
        let mut points = Vec::new();
        if let Some(start) = self.point(0.0) {
            points.push(start);
        }
        while let Some((from, to, depth)) = todo.pop() {
            let (Some(a), Some(b)) = (self.point(from), self.point(to)) else {
                break;
            };
            let flat = [0.25, 0.5, 0.75].into_iter().all(|s| {
                self.point(from + (to - from) * s)
                    .is_some_and(|p| off(p, a, b) <= tolerance)
            });
            let room = points.len() + todo.len() + 2 < MAX_DRAWN_POINTS;
            if flat || depth >= 16 || !room || tolerance.is_nan() || tolerance <= 0.0 {
                points.push(b);
            } else {
                let middle = from + (to - from) / 2.0;
                todo.push((middle, to, depth + 1));
                todo.push((from, middle, depth + 1));
            }
        }
        points
    }
}

fn points_param(
    typed: &Typed<'_>,
    p: &Params,
    command: &'static str,
    key: &'static str,
) -> Result<Option<Vec<PointArg>>, crate::CommandError> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > MAX_SPLINE_POINTS {
                return fail(format!(
                    "{command}: a spline has at most {MAX_SPLINE_POINTS} points, got {}",
                    items.len()
                ));
            }
            items
                .iter()
                .map(|item| PointArg::parse(typed, command, key, item))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        Some(_) => fail(format!(
            "{command}: `{key}` must be an array of point ids or coordinates [x, y]"
        )),
    }
}

/// Where a point given to the command is: an existing point's stored
/// position, or the coordinates given.
fn position(sketch: &Sketch, point: &PointArg) -> Result<(f64, f64), SketchError> {
    match point {
        PointArg::Existing(id) => sketch.point(*id),
        PointArg::New(x, y, _) => Ok((*x, *y)),
        PointArg::Image(of, path) => invalid(format!(
            "{of}[{path}]: a spline on an image point is not supported yet"
        )),
    }
}

fn invalid<T>(what: impl Into<String>) -> Result<T, SketchError> {
    Err(SketchError::Invalid(what.into()))
}

fn sketch_spline(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.spline";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "control",
            "through",
            "degree",
            "closed",
            "live",
            "construction",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let typed = session.typed();
    let control = points_param(&typed, &p, COMMAND, "control")?;
    let through = points_param(&typed, &p, COMMAND, "through")?;
    let (mut given, fitted) = match (control, through) {
        (Some(control), None) => (control, false),
        (None, Some(through)) => (through, true),
        (Some(_), Some(_)) => {
            return fail(format!(
                "{COMMAND}: give either `control` or `through`, not both"
            ));
        }
        (None, None) => {
            return fail(format!(
                "{COMMAND}: give the points of the spline: `control` (its control points) or `through` (points it passes through)"
            ));
        }
    };
    let degree = p
        .integer("degree", 1, u64::from(MAX_SPLINE_DEGREE))?
        .map_or(Ok(3), u32::try_from)
        .map_err(|_| p.missing("degree"))?;
    let closed = p.bool("closed")?.unwrap_or(false);
    let live = p.bool("live")?.unwrap_or(false);
    let construction = p.bool("construction")?.unwrap_or(false);
    if live && !fitted {
        return fail(format!(
            "{COMMAND}: `live` fits a spline through points at every regeneration: give `through`, not `control`"
        ));
    }
    let most = craft3d_regen::curve_fit::MAX_THROUGH_POINTS;
    if live && given.len().saturating_add(usize::from(closed)) > most {
        return fail(format!(
            "{COMMAND}: a live spline is fitted through at most {most} points, got {}",
            given.len()
        ));
    }
    if closed {
        let Some(first) = given.first().cloned() else {
            return fail(format!("{COMMAND}: a closed spline needs points"));
        };
        given.push(first);
    }
    let needed = usize::try_from(degree).map_or(usize::MAX, |d| d.saturating_add(1));
    if given.len() < needed {
        return fail(format!(
            "{COMMAND}: a spline of degree {degree} needs at least {needed} points{}, got {}",
            if closed {
                " (the first counted again as the last)"
            } else {
                ""
            },
            given.len()
        ));
    }
    let tolerance = session.document()?.history.current().tolerance.linear;

    let before = crate::commands::failing(session)?;
    if live {
        let (spline, points) = change(session, COMMAND, "Add live spline", id, |s| {
            let last = given.len().saturating_sub(1);
            // Two points in a row at one place, where both are known now.
            let places: Vec<Option<(f64, f64)>> = given.iter().map(|g| g.drawn(s)).collect();
            for (i, pair) in places.windows(2).enumerate() {
                if let [Some(a), Some(b)] = pair
                    && (a.0 - b.0).phypot(a.1 - b.1) <= tolerance
                    && !(closed && i + 1 == last)
                {
                    return invalid(format!(
                        "points {} and {} of the spline are at the same place ({}, {}); a spline takes each place once",
                        i + 1,
                        i + 2,
                        a.0,
                        a.1
                    ));
                }
            }
            let mut points: Vec<EntityId> = Vec::with_capacity(given.len());
            for (i, point) in given.iter().enumerate() {
                points.push(if closed && i == last {
                    *points
                        .first()
                        .ok_or(SketchError::Invalid("a closed spline needs points".into()))?
                } else {
                    point.place(s)?
                });
            }
            let spline = s.add_interpolated(degree, points.clone(), construction)?;
            Ok((spline, points))
        })?;
        let (start, end) = (points.first().copied(), points.last().copied());
        return sketch_result(
            session,
            id,
            Some(&before),
            json!({
                "entity": spline.to_string(),
                "degree": degree,
                "live": true,
                "through": ids(&points),
                "start": start.map(|id| id.to_string()),
                "end": end.map(|id| id.to_string()),
            }),
        );
    }
    let (spline, points) = change(session, COMMAND, "Add spline", id, |s| {
        // Two points in a row at one place make a corner (or, for a point
        // to pass through, ask the impossible).
        let places: Vec<(f64, f64)> = given
            .iter()
            .map(|point| position(s, point))
            .collect::<Result<_, _>>()?;
        for (i, pair) in places.windows(2).enumerate() {
            if let [a, b] = pair
                && (a.0 - b.0).phypot(a.1 - b.1) <= tolerance
            {
                return invalid(format!(
                    "points {} and {} of the spline are at the same place ({}, {}); a spline takes each place once",
                    i + 1,
                    i + 2,
                    a.0,
                    a.1
                ));
            }
        }
        let last = given.len().saturating_sub(1);
        let mut points: Vec<EntityId> = Vec::with_capacity(given.len());
        if fitted {
            let solved = interpolate(degree, &places).map_err(SketchError::Invalid)?;
            for (i, (point, at)) in given.iter().zip(&solved).enumerate() {
                points.push(if closed && i == last {
                    *points
                        .first()
                        .ok_or(SketchError::Invalid("a closed spline needs points".into()))?
                } else if i == 0 || i == last {
                    // The ends are shared as given.
                    point.place(s)?
                } else if matches!(point, PointArg::New(_, _, held) if *held != Default::default()) {
                    // The point made is a control point, not the point
                    // given: an expression could not hold it.
                    return invalid(format!(
                        "point {} of `through` has a coordinate written as an expression; a point the spline passes through between its ends is fitted, not held: give it as numbers, or give `control` points",
                        i + 1
                    ));
                } else {
                    s.add_point(at.0, at.1)?
                });
            }
        } else {
            for (i, point) in given.iter().enumerate() {
                points.push(if closed && i == last {
                    *points
                        .first()
                        .ok_or(SketchError::Invalid("a closed spline needs points".into()))?
                } else {
                    point.place(s)?
                });
            }
        }
        let spline = s.add_spline(degree, points.clone(), construction)?;
        Ok((spline, points))
    })?;
    let (start, end) = (points.first().copied(), points.last().copied());
    sketch_result(
        session,
        id,
        Some(&before),
        json!({
            "entity": spline.to_string(),
            "degree": degree,
            "control": ids(&points),
            "start": start.map(|id| id.to_string()),
            "end": end.map(|id| id.to_string()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;
    use craft3d_regen::curve_fit::MAX_THROUGH_POINTS;

    #[test]
    fn the_basis_functions_are_a_partition_of_unity() {
        for degree in 1..=7_u32 {
            for count in (degree as usize + 1)..16 {
                let knots = clamped_uniform_knots(degree, count);
                for step in 0..=100 {
                    let u = f64::from(step) / 100.0;
                    let (span, values) = basis(degree as usize, &knots, count, u).unwrap();
                    assert!(span >= degree as usize && span < count);
                    assert_eq!(values.len(), degree as usize + 1);
                    assert!(values.iter().all(|v| (-1e-15..=1.0 + 1e-15).contains(v)));
                    assert!((values.iter().sum::<f64>() - 1.0).abs() < 1e-14);
                }
                // The curve starts and ends on its end control points.
                let (span, values) = basis(degree as usize, &knots, count, 0.0).unwrap();
                assert_eq!((span, values[0]), (degree as usize, 1.0));
                let (span, values) = basis(degree as usize, &knots, count, 1.0).unwrap();
                assert_eq!((span, values[degree as usize]), (count - 1, 1.0));
            }
        }
        // Knots that do not fit, and parameters that are not numbers.
        let knots = clamped_uniform_knots(3, 6);
        assert!(basis(3, &knots, 7, 0.5).is_none());
        assert!(basis(0, &knots, 6, 0.5).is_none());
        assert!(basis(3, &knots, 6, f64::NAN).is_none());
        assert!(basis(3, &[], 0, 0.5).is_none());
        assert_eq!(basis(3, &knots, 6, -5.0), basis(3, &knots, 6, 0.0));
        assert_eq!(basis(3, &knots, 6, 5.0), basis(3, &knots, 6, 1.0));
    }

    /// The Greville abscissa of control point `i`.
    fn greville(degree: u32, count: usize, i: usize) -> f64 {
        let knots = clamped_uniform_knots(degree, count);
        let p = degree as usize;
        knots[i + 1..=i + p].iter().sum::<f64>() / p as f64
    }

    #[test]
    fn a_spline_through_points_passes_through_them() {
        // A wavy row of points, at three sizes and every degree.
        for scale in [1e-3, 1.0, 1e3] {
            for degree in 1..=7_u32 {
                for count in [degree as usize + 1, degree as usize + 2, 12, 40, 200] {
                    let through: Vec<(f64, f64)> = (0..count)
                        .map(|i| {
                            let t = i as f64;
                            (
                                scale * (t + 0.3 * (t * 1.7).psin()),
                                scale * (2.0 * (t * 0.9).pcos() + 0.01 * t * t),
                            )
                        })
                        .collect();
                    let control = interpolate(degree, &through).unwrap();
                    assert_eq!(control.len(), count);
                    assert_eq!(control[0], through[0]);
                    assert_eq!(control[count - 1], through[count - 1]);
                    let curve = SplineCurve::new(degree, &control).unwrap();
                    let size = through
                        .iter()
                        .map(|p| p.0.abs().max(p.1.abs()))
                        .fold(0.0, f64::max);
                    let mut worst: f64 = 0.0;
                    for (i, wanted) in through.iter().enumerate() {
                        let u = if i == 0 {
                            0.0
                        } else if i + 1 == count {
                            1.0
                        } else {
                            greville(degree, count, i)
                        };
                        let got = curve.point(u).unwrap();
                        worst = worst.max((got.0 - wanted.0).phypot(got.1 - wanted.1));
                    }
                    assert!(
                        worst <= 1e-12 * size,
                        "degree {degree}, {count} points, x{scale}: off by {worst:e} of {size}"
                    );
                }
            }
        }
    }

    #[test]
    fn points_that_cannot_be_fitted_are_refused() {
        let line: Vec<(f64, f64)> = (0..5).map(|i| (f64::from(i), 0.0)).collect();
        assert!(
            interpolate(3, &line[..3])
                .unwrap_err()
                .contains("needs from 4")
        );
        assert!(interpolate(0, &line).is_err());
        assert!(interpolate(8, &line).is_err());
        assert!(interpolate(u32::MAX, &line).is_err());
        let many = vec![(0.0, 0.0); MAX_THROUGH_POINTS + 1];
        assert!(interpolate(3, &many).unwrap_err().contains("at most"));
        for bad in [f64::NAN, f64::INFINITY, 1e308] {
            let mut points = line.clone();
            points[2].1 = bad;
            // Never a panic; huge but finite numbers may give a spline.
            if let Ok(control) = interpolate(3, &points) {
                assert!(control.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
            }
        }
        // Points on a line give a spline on that line.
        let control = interpolate(3, &line).unwrap();
        assert!(control.iter().all(|p| p.1 == 0.0));
    }

    #[test]
    fn a_spline_is_drawn_within_tolerance() {
        let control = [(0.0, 0.0), (0.0, 4.0), (4.0, 4.0), (4.0, 0.0), (8.0, -3.0)];
        for degree in 1..=4_u32 {
            let curve = SplineCurve::new(degree, &control).unwrap();
            let mut before = 0;
            for tolerance in [0.1, 1e-2, 1e-3, 1e-4] {
                let points = curve.polyline(tolerance);
                assert_eq!(points[0], control[0]);
                assert_eq!(points[points.len() - 1], control[4]);
                assert!(points.len() >= before, "finer never takes fewer points");
                before = points.len();
                // Every point of the curve is near the polyline.
                for step in 0..=2000 {
                    let p = curve.point(f64::from(step) / 2000.0).unwrap();
                    let nearest = points
                        .windows(2)
                        .map(|w| {
                            let (a, b) = (w[0], w[1]);
                            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                            let l2 = dx * dx + dy * dy;
                            let t = if l2 > 0.0 {
                                (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            (p.0 - a.0 - dx * t).phypot(p.1 - a.1 - dy * t)
                        })
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        nearest <= tolerance * 1.01,
                        "degree {degree} at {tolerance}: {nearest}"
                    );
                }
            }
            // A tolerance that is no tolerance draws the coarsest polyline,
            // and an impossible one stops at the cap.
            for tolerance in [0.0, -1.0, f64::NAN] {
                assert_eq!(
                    curve.polyline(tolerance).len(),
                    (5 - degree as usize) * 4 + 1
                );
            }
            assert!(curve.polyline(1e-300).len() <= MAX_DRAWN_POINTS);
        }
        assert!(SplineCurve::new(0, &control).is_none());
        assert!(SplineCurve::new(5, &control).is_none());
    }
}
