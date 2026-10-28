//! Sketches in regeneration: evaluating the dimensions, handing the sketch
//! to the solver as plain numbers, and reading back how it solved.
//!
//! The coordinates stored in the document are the solver's starting point
//! (the last solution), so a sketch that is already solved does not move,
//! and a changed dimension is followed from where the sketch was rather
//! than from scratch. [`settle_sketches`] writes a new solution back into
//! the document; the engine does that as part of every edit.

use crate::cause::FIX_IT_FIRST;
use crate::curve_fit::{self, Fit, Spline2};
use crate::sketch_pattern::{PatternResult, Placed};
use crate::{FeatureResult, Status, angle, length};
use craft3d_doc::{
    ConstraintId, ConstraintKind, DistanceDirection, Document, EntityId, FeatureId, FeatureKind,
    Geometry, ImagePlace, NameRole, PatternId, PersistentName, PlaceMap, Sketch, SketchPlane,
    WorldPlane,
};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_solve as solve;
use std::collections::BTreeMap;

/// How one constraint came out of the solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintState {
    /// Holds, and takes away at least one degree of freedom.
    Satisfied,
    /// Holds, but only repeats what earlier constraints already demand.
    Redundant,
    /// Cannot hold together with the constraints added before it.
    Conflicting,
    /// The sketch could not be solved at all.
    Unsolved,
    /// It uses an image point of an instance a pattern no longer has: it
    /// was left out of the solve (and is back when the count is).
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstraintResult {
    pub id: ConstraintId,
    pub state: ConstraintState,
    /// How far the constraint is from holding: a length in document units,
    /// or an angle in radians for parallel, perpendicular and angle.
    pub residual: Option<f64>,
}

/// A sketch after solving, placed in space.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchResult {
    /// The sketch's origin and axes in the world.
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
    /// Degrees of freedom left; `None` when the sketch could not be solved.
    pub dof: Option<usize>,
    /// One per constraint, in the order they were added.
    pub constraints: Vec<ConstraintResult>,
    /// Solved coordinates of the points, in sketch axes.
    pub points: Vec<(EntityId, f64, f64)>,
    /// Solved radii of the circles.
    pub radii: Vec<(EntityId, f64)>,
    /// The patterns of the sketch as evaluated, in the sketch's order.
    pub patterns: Vec<crate::sketch_pattern::PatternResult>,
    /// Where each image point that stands for an instance lies, and the
    /// numbers of the map that put it there, in the order of the ids: what
    /// the document stores with the coordinates
    /// ([`craft3d_doc::Sketch::image_places`]).
    pub image_places: Vec<ImagePlace>,
    /// The kernel's numbers for the curves of nested patterns
    /// ([`crate::sketch_pattern::CurveIds`]).
    pub curve_ids: crate::sketch_pattern::CurveIds,
    /// The curves regeneration fitted (formula curves and live splines),
    /// in the order of their ids.
    pub curves: Vec<FittedCurve>,
}

/// A curve of a sketch as regeneration fitted it.
#[derive(Clone, Debug, PartialEq)]
pub struct FittedCurve {
    pub id: EntityId,
    /// On clamped knots; it starts and ends on the curve's end points as
    /// solved.
    pub spline: Spline2,
    /// For a formula curve, the largest distance between the spline and
    /// the formula found at the fit's checks; `None` for a live spline,
    /// which passes through its points.
    pub deviation: Option<f64>,
}

impl SketchResult {
    /// The solved coordinates of a point (or of an image point).
    pub fn point(&self, id: EntityId) -> Option<(f64, f64)> {
        // In the order of the ids, which is the order they were drawn in:
        // a lookup is a search, not a walk, with thousands of points.
        match self.points.binary_search_by_key(&id, |(point, ..)| *point) {
            Ok(at) => self.points.get(at).map(|&(_, x, y)| (x, y)),
            Err(_) => None,
        }
    }

    /// A curve as the sketch calls it, from the number the kernel has for
    /// it: `e7`, `e7[3]`, `e3[2][1]`.
    pub fn curve_name(&self, id: u64) -> String {
        self.curve_ids.name(id)
    }

    /// A fitted curve (a formula curve or a live spline) as the kernel
    /// takes it.
    pub fn curve(&self, id: EntityId) -> Option<&FittedCurve> {
        match self.curves.binary_search_by_key(&id, |c| c.id) {
            Ok(at) => self.curves.get(at),
            Err(_) => None,
        }
    }

    pub fn radius(&self, id: EntityId) -> Option<f64> {
        match self.radii.binary_search_by_key(&id, |(circle, _)| *circle) {
            Ok(at) => self.radii.get(at).map(|&(_, r)| r),
            Err(_) => None,
        }
    }

    /// A point of the sketch in world coordinates.
    pub fn to_world(&self, x: f64, y: f64) -> Vec3 {
        self.origin + self.x_axis * x + self.y_axis * y
    }
}

/// The axes of a world plane: sketch x, sketch y and the normal.
fn axes(plane: WorldPlane) -> (Vec3, Vec3, Vec3) {
    match plane {
        WorldPlane::Xy => (Vec3::X, Vec3::Y, Vec3::Z),
        WorldPlane::Yz => (Vec3::Y, Vec3::Z, Vec3::X),
        WorldPlane::Zx => (Vec3::Z, Vec3::X, Vec3::Y),
    }
}

fn list(ids: &[ConstraintId]) -> String {
    let shown: Vec<String> = ids.iter().take(10).map(|id| id.to_string()).collect();
    let more = ids.len().saturating_sub(shown.len());
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// The sketch as the solver takes it, with the way back to document ids.
struct Problem {
    input: solve::Sketch,
    /// Document id of each solver entity, by solver index: every entity
    /// but the splines, in the sketch's order, and then for each helper
    /// line that stands for the end of a spline in a tangency the id of
    /// that spline.
    entities: Vec<EntityId>,
    /// Document id of each solver constraint, by solver index: the
    /// constraints that were handed to the solver, then one row per ring
    /// of runs (`chains`).
    constraints: Vec<ConstraintId>,
    /// The rings of runs whose closing equation follows the constraints,
    /// in that order.
    chains: Vec<PatternId>,
    /// The constraints left out, each with the image point that is not
    /// there and why.
    left_out: Vec<(ConstraintId, String)>,
    /// The map of each image point that stands for an instance.
    image_maps: Vec<(EntityId, solve::ImageMap)>,
    /// The ends of formula curves held where the formulas put them: one
    /// solver constraint each, after the chains, as (curve, point).
    pins: Vec<(EntityId, EntityId)>,
}

/// The fit of each formula curve of a sketch, in the sketch's order, or
/// why it has none.
pub(crate) type FormulaFits = Vec<(EntityId, Result<Fit, String>)>;

/// Fits every formula curve of `sketch` with the current variables.
pub(crate) fn formula_fits(doc: &Document, variables: &Evaluation, sketch: &Sketch) -> FormulaFits {
    sketch
        .entities
        .iter()
        .filter_map(|e| match &e.geometry {
            Geometry::Formula { x, y, from, to, .. } => Some((
                e.id,
                fit_formula_curve(doc, variables, &e.id.to_string(), [x, y, from, to]),
            )),
            _ => None,
        })
        .collect()
}

/// Fits one formula curve (`craft3d_doc::Geometry::Formula`) with the
/// variables `variables`: evaluates its range, then its formulas with the
/// parameter bound, in the document's length unit. `id` names the curve in
/// messages. The method and what it guarantees: [`crate::curve_fit`].
pub fn fit_formula_curve(
    doc: &Document,
    variables: &Evaluation,
    id: &str,
    [x, y, from, to]: [&craft3d_doc::Expr; 4],
) -> Result<Fit, String> {
    let range = |name: &str, expr: &craft3d_doc::Expr| {
        variables
            .evaluate(expr.source())
            .map_err(|e| format!("{id}.{name}: `{}`: {e}", expr.source()))
    };
    let (mut start, mut end) = (range("from", from)?, range("to", to)?);
    // An end that is exactly zero takes the other's dimension: `[0, 360
    // deg]` is a range of angles.
    if start.dimension != end.dimension {
        if start.value == 0.0 {
            start = Quantity::new(0.0, end.dimension);
        } else if end.value == 0.0 {
            end = Quantity::new(0.0, start.dimension);
        }
    }
    if start.dimension != end.dimension {
        return Err(format!(
            "{id}: the range of t runs from `{}`, which is {}, to `{}`, which is {}; both ends have one dimension",
            from.source(),
            start.dimension.described(),
            to.source(),
            end.dimension.described()
        ));
    }
    let dimension = start.dimension;
    let parse = |name: &str, expr: &craft3d_doc::Expr| {
        craft3d_expr::Expr::parse(expr.source())
            .map_err(|e| format!("{id}.{name}: `{}`: {e}", expr.source()))
    };
    let (fx, fy) = (parse("x", x)?, parse("y", y)?);
    let coordinate = |name: &str, source: &str, formula: &craft3d_expr::Expr, t: f64| {
        let quantity = variables
            .evaluate_bound(
                formula,
                craft3d_doc::FORMULA_PARAMETER,
                Quantity::new(t, dimension),
            )
            .map_err(|e| format!("{id}.{name} at t = {t}: `{source}`: {e}"))?;
        if quantity.dimension == Dimension::LENGTH {
            Ok(crate::to_document(quantity, doc).value)
        } else if quantity.dimension.is_dimensionless() {
            Ok(quantity.value)
        } else {
            Err(format!(
                "{id}.{name}: `{source}` is {}, but a length is needed{}",
                quantity.dimension.described(),
                quantity.dimension.unit_hint(source)
            ))
        }
    };
    curve_fit::fit_formula(
        |t| {
            Ok([
                coordinate("x", x.source(), &fx, t)?,
                coordinate("y", y.source(), &fy, t)?,
            ])
        },
        start.value,
        end.value,
        doc.tolerance.linear,
    )
    .map_err(|e| {
        if e.starts_with(&format!("{id}.")) {
            e
        } else {
            format!("{id}: {e}")
        }
    })
}

/// An instance of a pattern: the pattern's place in the sketch's list and
/// the instance's index.
type Instance = (usize, (u32, u32));

/// Where each image point of the sketch is: the instance it stands for,
/// or why there is no such instance. An image of an image (nested
/// patterns) is not there when the one it is made from is not, nor when
/// an end of its pattern's run is an image point that is not there.
fn image_instances(
    sketch: &Sketch,
    patterns: &[PatternResult],
) -> BTreeMap<EntityId, Result<Instance, String>> {
    fn own(
        sketch: &Sketch,
        patterns: &[PatternResult],
        id: EntityId,
    ) -> Option<Result<Instance, String>> {
        let Ok(Geometry::Image { of, pattern, index }) = sketch.entity(id).map(|e| &e.geometry)
        else {
            return None;
        };
        let found = patterns.iter().position(|p| p.id == *pattern);
        Some(match found.and_then(|n| patterns.get(n).map(|p| (n, p))) {
            None => Err(format!(
                "{id} = {of}[{index}]: the sketch has no pattern {pattern}"
            )),
            Some((n, p)) => match p.resolve(*index) {
                Some(instance) => Ok((n, instance)),
                None => Err(format!(
                    "{id} = {of}[{index}] is not there: {}",
                    p.describe()
                )),
            },
        })
    }
    let mut out = BTreeMap::new();
    for entity in &sketch.entities {
        let Some(mut at) = own(sketch, patterns, entity.id) else {
            continue;
        };
        // What it is made from, level by level inwards (bounded: a valid
        // sketch nests three levels deep).
        let mut next = vec![entity.id];
        for _ in 0..=4 * craft3d_doc::MAX_PATTERN_DEPTH {
            let Some(id) = next.pop() else { break };
            let Ok(Geometry::Image { of, pattern, .. }) = sketch.entity(id).map(|e| &e.geometry)
            else {
                continue;
            };
            let mut from = vec![*of];
            if let Some(found) = sketch.pattern(*pattern)
                && let Some((a, b)) = found.layout.chain()
            {
                from.extend([a, b]);
            }
            for point in from {
                match own(sketch, patterns, point) {
                    Some(Err(why)) if at.is_ok() => at = Err(why),
                    Some(_) => next.push(point),
                    None => {}
                }
            }
        }
        out.insert(entity.id, at);
    }
    out
}

fn problem(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    patterns: &[PatternResult],
    fits: &FormulaFits,
) -> Result<Problem, String> {
    // A spline is not an entity of the solver's: its shape is that of its
    // control points, which are. So it adds no unknowns and no equations,
    // and the degrees of freedom are those of the points.
    let mut index = BTreeMap::new();
    let mut entities = Vec::new();
    let position = |count: usize| {
        u32::try_from(count)
            .map(solve::EntityId)
            .map_err(|_| "the sketch has too many entities".to_string())
    };
    for entity in &sketch.entities {
        // Nor are the curves regeneration fits.
        if matches!(entity.geometry, Geometry::Spline { .. }) || entity.geometry.is_fitted() {
            continue;
        }
        index.insert(entity.id, position(entities.len())?);
        entities.push(entity.id);
    }
    let find = |id: EntityId| -> Result<solve::EntityId, String> {
        index
            .get(&id)
            .copied()
            .ok_or_else(|| format!("there is no entity {id}"))
    };
    let images = image_instances(sketch, patterns);
    // A curve built on an image point that is not there has nowhere to
    // be: the sketch cannot be solved.
    for entity in &sketch.entities {
        for point in entity.geometry.points() {
            if let Some(Err(why)) = images.get(&point) {
                return Err(format!("{} ends on {why}", entity.id));
            }
        }
    }
    let mut input = solve::Sketch::new();
    let mut image_maps = Vec::new();
    for entity in &sketch.entities {
        input.entities.push(match entity.geometry {
            Geometry::Spline { .. } | Geometry::Formula { .. } | Geometry::Interpolated { .. } => {
                continue;
            }
            Geometry::Image { of, .. } => {
                let map = match images.get(&entity.id) {
                    Some(Ok((n, instance))) => match patterns.get(*n) {
                        Some(pattern) => {
                            let map = pattern.map(*instance, find)?;
                            image_maps.push((entity.id, map));
                            map
                        }
                        None => solve::ImageMap::Shift { offset: [0.0; 2] },
                    },
                    // Nothing uses it: the constraints on it are left out.
                    _ => solve::ImageMap::Shift { offset: [0.0; 2] },
                };
                solve::Entity::Image { of: find(of)?, map }
            }
            Geometry::Point { x, y } => solve::Entity::Point { x, y },
            Geometry::Line { start, end } => solve::Entity::Line {
                start: find(start)?,
                end: find(end)?,
            },
            Geometry::Circle { center, radius } => solve::Entity::Circle {
                center: find(center)?,
                radius,
            },
            Geometry::Arc { center, start, end } => solve::Entity::Arc {
                center: find(center)?,
                start: find(start)?,
                end: find(end)?,
            },
        });
    }
    image_maps.sort_by_key(|(id, _)| *id);
    // Where the image points lay in the stored solution: the solver carries
    // the sketch along from there when they have moved. Only images that
    // stand for an instance (the others are placeholders nothing uses).
    for place in &sketch.image_places {
        if let Ok(at) = image_maps.binary_search_by_key(&place.entity, |(id, _)| *id)
            && let Some((_, now)) = image_maps.get(at)
            && let Ok(id) = find(place.entity)
            && place.at.iter().all(|v| v.is_finite())
        {
            input.image_starts.push(solve::ImageStart {
                image: id,
                at: place.at,
                map: place.map.and_then(|was| map_then(was, now)),
            });
        }
    }
    // What a curve gives a tangency that involves a spline: a line to be
    // parallel to, or a circle or arc to touch.
    enum Along {
        Line(solve::EntityId),
        Round(solve::EntityId),
    }
    // A clamped spline leaves its end point along the leg to the next
    // control point. For the solver that leg is a line between two of its
    // points, added here: it has no unknowns of its own, and stands for
    // the spline in messages.
    let along = |input: &mut solve::Sketch,
                 entities: &mut Vec<EntityId>,
                 curve: EntityId,
                 at: EntityId|
     -> Result<Along, String> {
        match &sketch.entity(curve).map_err(|e| e.to_string())?.geometry {
            Geometry::Line { .. } => Ok(Along::Line(find(curve)?)),
            Geometry::Circle { .. } | Geometry::Arc { .. } => Ok(Along::Round(find(curve)?)),
            Geometry::Point { .. } | Geometry::Image { .. } => {
                Err(format!("{curve} is a point, which has no direction"))
            }
            Geometry::Formula { .. } | Geometry::Interpolated { .. } => Err(format!(
                "{curve} is a fitted curve, which takes no tangent constraint yet"
            )),
            Geometry::Spline { control, .. } => {
                let leg = if control.first() == Some(&at) {
                    control.get(1)
                } else {
                    control.len().checked_sub(2).and_then(|i| control.get(i))
                };
                let Some(next) = leg else {
                    return Err(format!("{curve} has too few control points"));
                };
                let id = position(input.entities.len())?;
                input.entities.push(solve::Entity::Line {
                    start: find(at)?,
                    end: find(*next)?,
                });
                entities.push(curve);
                Ok(Along::Line(id))
            }
        }
    };
    let is_spline = |id: EntityId| {
        sketch
            .entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Spline { .. }))
    };
    let is_line = |id: EntityId| {
        sketch
            .entity(id)
            .is_ok_and(|e| matches!(e.geometry, Geometry::Line { .. }))
    };
    let ends = |id: EntityId| -> Result<(solve::EntityId, solve::EntityId), String> {
        match sketch.entity(id).map(|e| &e.geometry) {
            Ok(&Geometry::Line { start, end }) => Ok((find(start)?, find(end)?)),
            _ => Err(format!("{id} is not a line")),
        }
    };
    let mut splines: BTreeMap<EntityId, usize> = BTreeMap::new();
    let mut handed = Vec::with_capacity(sketch.constraints.len());
    let mut left_out = Vec::new();
    for constraint in &sketch.constraints {
        let id = constraint.id;
        let lost = constraint
            .kind
            .entities()
            .into_iter()
            .find_map(|e| match images.get(&e) {
                Some(Err(why)) => Some(why.clone()),
                _ => None,
            });
        if let Some(why) = lost {
            left_out.push((id, why));
            continue;
        }
        handed.push(id);
        let name = format!("{id}.value");
        use solve::Constraint as S;
        let mapped = match &constraint.kind {
            ConstraintKind::Coincident { a, b } => S::Coincident {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::PointOn { point, curve } => {
                if let Ok(Geometry::Spline { degree, control }) =
                    sketch.entity(*curve).map(|e| &e.geometry)
                {
                    // The solver has the spline as its control points, in
                    // a list of its own; each spline is given once.
                    let number = match splines.get(curve) {
                        Some(number) => *number,
                        None => {
                            let points = control
                                .iter()
                                .map(|id| find(*id))
                                .collect::<Result<Vec<_>, _>>()?;
                            let number = input.add_spline(
                                usize::try_from(*degree).unwrap_or(usize::MAX),
                                craft3d_doc::clamped_uniform_knots(*degree, control.len()),
                                points,
                            );
                            splines.insert(*curve, number);
                            number
                        }
                    };
                    S::PointOnSpline {
                        point: find(*point)?,
                        spline: number,
                    }
                } else if is_line(*curve) {
                    S::PointOnLine {
                        point: find(*point)?,
                        line: find(*curve)?,
                    }
                } else {
                    S::PointOnCircle {
                        point: find(*point)?,
                        circle: find(*curve)?,
                    }
                }
            }
            ConstraintKind::Horizontal { a, b: None } => S::Horizontal { line: find(*a)? },
            ConstraintKind::Horizontal { a, b: Some(b) } => S::HorizontalPoints {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Vertical { a, b: None } => S::Vertical { line: find(*a)? },
            ConstraintKind::Vertical { a, b: Some(b) } => S::VerticalPoints {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Parallel { a, b } => S::Parallel {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Perpendicular { a, b } => S::Perpendicular {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Tangent { a, b } if is_spline(*a) || is_spline(*b) => {
                let Some((at_a, at_b)) = sketch.tangent_at(*a, *b) else {
                    return Err(format!(
                        "{id}: a spline is tangent to another curve at an end point the two share, and {a} and {b} share none (any more); make their ends coincident, or delete the constraint"
                    ));
                };
                let first = along(&mut input, &mut entities, *a, at_a)?;
                let second = along(&mut input, &mut entities, *b, at_b)?;
                match (first, second) {
                    (Along::Line(a), Along::Line(b)) => S::Parallel { a, b },
                    // The solver takes the tangency at the point both are
                    // kept on, which is the shared end.
                    (Along::Line(line), Along::Round(round))
                    | (Along::Round(round), Along::Line(line)) => S::Tangent { a: line, b: round },
                    (Along::Round(_), Along::Round(_)) => {
                        return Err(format!("{id}: neither {a} nor {b} is a spline"));
                    }
                }
            }
            ConstraintKind::Tangent { a, b } => S::Tangent {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Equal { a, b } => S::Equal {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Concentric { a, b } => S::Concentric {
                a: find(*a)?,
                b: find(*b)?,
            },
            ConstraintKind::Symmetric { a, b, about } => S::Symmetric {
                a: find(*a)?,
                b: find(*b)?,
                axis: find(*about)?,
            },
            ConstraintKind::Midpoint { point, line } => S::Midpoint {
                point: find(*point)?,
                line: find(*line)?,
            },
            ConstraintKind::Fixed { point } => S::Fixed {
                point: find(*point)?,
                at: None,
            },
            ConstraintKind::X { point, value } => S::CoordinateX {
                point: find(*point)?,
                value: length(doc, variables, &name, value)?,
            },
            ConstraintKind::Y { point, value } => S::CoordinateY {
                point: find(*point)?,
                value: length(doc, variables, &name, value)?,
            },
            ConstraintKind::Distance {
                a,
                b,
                direction,
                value,
            } => {
                let value = length(doc, variables, &name, value)?;
                // The length of a line is the distance between its ends.
                let (a, b) = match b {
                    Some(b) => (find(*a)?, find(*b)?),
                    None => ends(*a)?,
                };
                match direction {
                    DistanceDirection::Direct => S::Distance { a, b, value },
                    DistanceDirection::Horizontal => S::HorizontalDistance { a, b, value },
                    DistanceDirection::Vertical => S::VerticalDistance { a, b, value },
                }
            }
            ConstraintKind::Angle { a, b, value } => S::Angle {
                a: find(*a)?,
                b: find(*b)?,
                value: angle(doc, variables, &name, value)?,
            },
            ConstraintKind::Radius { curve, value } => S::Radius {
                circle: find(*curve)?,
                value: length(doc, variables, &name, value)?,
            },
            ConstraintKind::Diameter { curve, value } => S::Diameter {
                circle: find(*curve)?,
                value: length(doc, variables, &name, value)?,
            },
        };
        input.constraints.push(mapped);
    }
    // A ring of runs: the end of the seed is its start turned by the
    // angle, as the pattern's own equation (an image point of the start
    // in instance 1, coincident with the end).
    let mut chains = Vec::new();
    for pattern in patterns {
        let Placed::Turn {
            chain: Some((from, to)),
            ..
        } = pattern.placed
        else {
            continue;
        };
        let turned = position(input.entities.len())?;
        input.entities.push(solve::Entity::Image {
            of: find(from)?,
            map: pattern.map((1, 0), find)?,
        });
        entities.push(from);
        input.constraints.push(solve::Constraint::Coincident {
            a: find(to)?,
            b: turned,
        });
        chains.push(pattern.id);
    }
    // The ends of formula curves, where the formulas put them.
    let mut pins = Vec::new();
    let mut held: BTreeMap<EntityId, (EntityId, [f64; 2])> = BTreeMap::new();
    let distance2 = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    for (curve, fit) in fits {
        let fit = fit.as_ref().map_err(Clone::clone)?;
        let Ok(Geometry::Formula { start, end, .. }) = sketch.entity(*curve).map(|e| &e.geometry)
        else {
            continue;
        };
        let (Some(first), Some(last)) = (fit.spline.control.first(), fit.spline.control.last())
        else {
            return Err(format!("{curve} has no fitted points"));
        };
        if start == end {
            let gap = (first[0] - last[0]).hypot(first[1] - last[1]);
            if gap > doc.tolerance.linear {
                return Err(format!(
                    "{curve} starts and ends on the one point {start}, but its formulas put its start at ({}, {}) and its end at ({}, {}), {gap:e} apart; end it on another point, or change the formulas so it closes",
                    first[0], first[1], last[0], last[1]
                ));
            }
        }
        let mut ends = vec![(*start, *first)];
        if start != end {
            ends.push((*end, *last));
        }
        for (point, at) in ends {
            // A point two curves end on is held once, where both put it.
            match held.get(&point) {
                Some(&(other, there)) => {
                    let gap: f64 = distance2(at, there);
                    if gap > doc.tolerance.linear {
                        return Err(format!(
                            "the formula curves {other} and {curve} do not meet at {point}: one ends at ({}, {}), the other at ({}, {}), {gap:e} apart; change the formulas, or end them on two points",
                            there[0], there[1], at[0], at[1]
                        ));
                    }
                }
                None => {
                    held.insert(point, (*curve, at));
                    input.constraints.push(solve::Constraint::Fixed {
                        point: find(point)?,
                        at: Some(at),
                    });
                    pins.push((*curve, point));
                }
            }
        }
    }
    Ok(Problem {
        input,
        entities,
        constraints: handed,
        chains,
        left_out,
        image_maps,
        pins,
    })
}

/// The map an image had, in the solver's terms: the numbers stored with
/// its place on the points of the map it has now, when both are of one
/// kind; `None` (carried in a straight line) when the pattern's kind
/// changed.
fn map_then(was: PlaceMap, now: &solve::ImageMap) -> Option<solve::ImageMap> {
    match (was, *now) {
        (PlaceMap::Shift { offset }, solve::ImageMap::Shift { .. }) => {
            Some(solve::ImageMap::Shift { offset })
        }
        (PlaceMap::Chain { k }, solve::ImageMap::Chain { from, to, .. }) => {
            Some(solve::ImageMap::Chain { from, to, k })
        }
        (PlaceMap::Turn { angle, at }, solve::ImageMap::Turn { center, .. }) => {
            Some(solve::ImageMap::Turn { center, at, angle })
        }
        _ => None,
    }
}

/// The numbers of a map, as the document stores them with a place.
fn place_map(map: &solve::ImageMap) -> PlaceMap {
    match *map {
        solve::ImageMap::Shift { offset } => PlaceMap::Shift { offset },
        solve::ImageMap::Chain { k, .. } => PlaceMap::Chain { k },
        solve::ImageMap::Turn { at, angle, .. } => PlaceMap::Turn { angle, at },
    }
}

/// A solver error in the document's own ids.
fn explain(error: &solve::SolveError, problem: &Problem) -> String {
    use solve::{Site, SolveError as E};
    let constraint = |i: usize| match problem.constraints.get(i) {
        Some(id) => id.to_string(),
        None => {
            let after = i.checked_sub(problem.constraints.len());
            if let Some(p) = after.and_then(|n| problem.chains.get(n)) {
                format!("the chain of {p}")
            } else if let Some((curve, point)) = after
                .and_then(|n| n.checked_sub(problem.chains.len()))
                .and_then(|n| problem.pins.get(n))
            {
                format!("the end {point} of the formula curve {curve}")
            } else {
                format!("constraint {i}")
            }
        }
    };
    let entity = |i: usize| {
        problem
            .entities
            .get(i)
            .map_or_else(|| format!("entity {i}"), |id| id.to_string())
    };
    let site = |at: &Site| match at {
        Site::Entity(i) => entity(*i),
        Site::Constraint(i) => constraint(*i),
    };
    match error {
        E::UnknownEntity { at, id } => {
            format!(
                "{} refers to {}, which does not exist",
                site(at),
                entity(id.index())
            )
        }
        E::WrongKind {
            at,
            id,
            expected,
            found,
        } => format!(
            "{} needs {} to be a {expected}, but it is a {found}",
            site(at),
            entity(id.index())
        ),
        E::NonFinite { at } => format!("{} has a value that is not finite", site(at)),
        E::InvalidValue {
            constraint: i,
            reason,
        } => {
            format!("{} has a value it cannot take: {reason}", constraint(*i))
        }
        E::Degenerate {
            constraint: i,
            reason,
        } if *reason == solve::CROWDED => {
            format!("{} is not supported yet: {reason}", constraint(*i))
        }
        E::Degenerate {
            constraint: i,
            reason,
        } => {
            format!("{} has no meaning here: {reason}", constraint(*i))
        }
        E::Unsupported {
            constraint: i,
            what,
        } => {
            format!("{} is not supported yet: {what}", constraint(*i))
        }
        other => other.to_string(),
    }
}

/// Coordinates of points, by entity.
type Points = Vec<(EntityId, f64, f64)>;
/// Radii of circles, by entity.
type Radii = Vec<(EntityId, f64)>;

/// The stored coordinates, for a sketch that could not be solved.
fn stored(sketch: &Sketch) -> (Points, Radii) {
    let mut points = Vec::new();
    let mut radii = Vec::new();
    for entity in &sketch.entities {
        match entity.geometry {
            Geometry::Point { x, y } => points.push((entity.id, x, y)),
            Geometry::Circle { radius, .. } => radii.push((entity.id, radius)),
            Geometry::Line { .. }
            | Geometry::Arc { .. }
            | Geometry::Spline { .. }
            | Geometry::Image { .. }
            | Geometry::Formula { .. }
            | Geometry::Interpolated { .. } => {}
        }
    }
    points.sort_by_key(|(id, ..)| *id);
    radii.sort_by_key(|(id, _)| *id);
    (points, radii)
}

/// Adds the image points of the sketch to its solved points: where the
/// instance they stand for puts their seed point (the same numbers as the
/// curves of that instance).
fn place_images(sketch: &Sketch, patterns: &[PatternResult], points: &mut Points) {
    points.sort_by_key(|(id, ..)| *id);
    let placed = {
        let lookup = |id: EntityId| match points.binary_search_by_key(&id, |(p, ..)| *p) {
            Ok(at) => points.get(at).map(|&(_, x, y)| [x, y]),
            Err(_) => None,
        };
        crate::sketch_pattern::image_places(sketch, patterns, lookup)
    };
    points.extend(placed);
    points.sort_by_key(|(id, ..)| *id);
}

/// Where a sketch lies in space: its origin, its two axes and its normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
}

impl Placement {
    /// The world xy plane: for solving a sketch without placing it.
    pub(crate) const FLAT: Placement = Placement {
        origin: Vec3::ZERO,
        x_axis: Vec3::X,
        y_axis: Vec3::Y,
        normal: Vec3::Z,
    };
}

/// The feature a name says made the entity, with its result: it has to
/// exist, come earlier, and have a body (taken by a later feature or not).
fn maker<'a>(
    doc: &'a Document,
    earlier: &'a [FeatureResult],
    feature: craft3d_doc::FeatureId,
    name: &PersistentName,
) -> Result<
    (
        &'a craft3d_doc::Feature,
        &'a FeatureResult,
        &'a craft3d_topo::Body,
    ),
    String,
> {
    let Some(source) = doc.feature(feature) else {
        return Err(format!(
            "plane: there is no feature {feature} for the face {name} (was it deleted?)"
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == feature) else {
        return Err(format!(
            "plane: `{made_by}`, which makes the face {name}, {}",
            crate::comes_after(doc, earlier, feature)
        ));
    };
    if let Some(off) = crate::pass::lost(outcome, name) {
        return Err(format!(
            "plane: `{made_by}`, which makes the face {name}, is {off}"
        ));
    }
    let Some(body) = &outcome.body else {
        if let Some(why) = crate::pass::absent(earlier, feature) {
            return Err(format!(
                "plane: `{made_by}`, which makes the face {name}, has no body: {why}"
            ));
        }
        return Err(match &outcome.status {
            Status::Ok => format!(
                "plane: `{made_by}` is a {}, whose faces have no names yet",
                source.kind.type_name()
            ),
            status @ (Status::Suppressed | Status::Inactive) => {
                format!(
                    "plane: `{made_by}`, which makes the face {name}, is {}",
                    crate::pass::word(status)
                )
            }
            Status::RolledBack => {
                format!("plane: `{made_by}`, which makes the face {name}, is rolled back")
            }
            Status::Failed(_) => {
                format!("plane: `{made_by}`, which makes the face {name}, {FIX_IT_FIRST}")
            }
        });
    };
    Ok((source, outcome, body))
}

/// "`Plate` no longer has a face X; its faces are: ...".
fn no_such_face(made_by: &str, name: &PersistentName, names: &[PersistentName]) -> String {
    let there: Vec<String> = names.iter().take(12).map(ToString::to_string).collect();
    format!(
        "plane: `{made_by}` no longer has a face {name}; its faces are: {}{}",
        there.join(", "),
        if names.len() > there.len() {
            ", ..."
        } else {
            ""
        }
    )
}

/// True when two face names are the same, or differ only in the outline
/// their cap was named after: all caps of one role of one feature lie in
/// one plane (see [`made_face`]).
pub(crate) fn same_plane(a: &PersistentName, b: &PersistentName) -> bool {
    let (mut a, mut b) = (a, b);
    for _ in 0..craft3d_doc::MAX_NAME_DEPTH {
        match (&a.of, &b.of) {
            (Some(next_a), Some(next_b)) => {
                if (a.feature, a.role, &a.sources) != (b.feature, b.role, &b.sources) {
                    return false;
                }
                (a, b) = (next_a, next_b);
            }
            (None, None) => {
                return a.feature == b.feature
                    && a.role == b.role
                    && (a.sources == b.sources
                        || matches!(a.role, NameRole::StartCap | NameRole::EndCap));
            }
            _ => return false,
        }
    }
    false
}

/// The frame of the face a name points at, among the features regenerated
/// so far. A name that points at nothing is an error that says what is
/// there instead; it never resolves to another face.
pub(crate) fn resolve_face(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
) -> Result<Frame, String> {
    if !name.is_face() {
        return Err(format!(
            "plane: {name} is not a face; a sketch lies on a startCap, an endCap or a side"
        ));
    }
    // A copy of a face lies where the face it is a copy of lies, moved by
    // the feature that copied it. So the way of the copy is walked down to
    // the face as it was first made, feature by feature, and its frame is
    // carried back up.
    let mut maps = Vec::new();
    let mut whole = name.clone();
    let mut rounds = 0usize;
    while whole.role == NameRole::Copy {
        // A name in the short form is one copy; one built by hand may be
        // a copy of a copy.
        rounds = rounds.saturating_add(1);
        if rounds > craft3d_doc::MAX_NAME_DEPTH {
            return Err(format!("plane: {name} is made of too many names"));
        }
        let (source, outcome, _) = maker(doc, earlier, whole.feature, name)?;
        let listed = outcome.names.as_ref().map_or(&[][..], |n| &n.faces[..]);
        // The face has to be one of the body each instance on the way was
        // made from (for a boolean: instance 0 is the target, 1 the tool).
        let Some((steps, end)) = crate::follow::steps(doc, earlier, &whole) else {
            return Err(no_such_face(&source.name, &whole, listed));
        };
        // What the face is called in the body of each feature on the way,
        // from the innermost out: each a copy of the next.
        let mut called = Vec::with_capacity(steps.len());
        let mut inner = end.clone();
        for &(feature, instance) in steps.iter().rev() {
            inner = PersistentName::copy(feature, instance, inner);
            called.push(inner.clone());
        }
        called.reverse();
        for (&(feature, instance), link) in steps.iter().zip(&called) {
            walk_copy(doc, earlier, name, feature, instance, link, &mut maps)?;
        }
        whole = end.clone();
    }
    let mut frame = made_face(doc, earlier, &whole, name)?;
    for (map, moved, turned) in maps.iter().rev() {
        // Seen from outside the moved body: z is the image of the normal
        // and x the image of x. After a mirror that leaves y pointing the
        // other way than the image of y, as a right-handed frame must.
        frame = Frame::new(
            map.point(frame.origin()),
            map.vector(frame.z()),
            map.vector(frame.x()),
        )
        .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        if *turned {
            // Seen from the other side: x stays, y and z turn round, as
            // for a face whose outward normal is against its surface's.
            frame = frame
                .reversed_at(frame.origin())
                .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        }
        if let Some((on, outward, hollow, turns)) = moved {
            if *turns {
                // Turned the shortest way onto the face's new normal:
                // about the line the old and the new plane share, which
                // is the face's own turn for every turn that tilts it.
                frame = turned_onto(&frame, *outward)
                    .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
            }
            // The origin dropped straight onto the plane the face lies in
            // now; the axes stay what they were. The face of a hollow
            // looks into it: its z is the face's own normal and its x
            // stays, so its y is the other way round, as when a wall is
            // looked at from behind.
            let z = if *hollow { *outward } else { frame.z() };
            let origin = frame.origin() + z * (*on - frame.origin()).dot(z);
            frame = if *hollow {
                Frame::new(origin, z, frame.x())
            } else {
                frame.at(origin)
            }
            .ok_or_else(|| format!("plane: the face {name} has a degenerate plane"))?;
        }
    }
    Ok(frame)
}

/// `frame` turned the shortest way so that its z is `normal` (a unit
/// vector): its x and y turn with it about the line square to both.
/// The frame itself when z is `normal` already.
fn turned_onto(frame: &Frame, normal: Vec3) -> Option<Frame> {
    let z = frame.z();
    let across = z.cross(normal);
    let sine = across.length();
    let cosine = z.dot(normal);
    if sine <= 1e-15 && cosine > 0.0 {
        return Some(*frame);
    }
    let turn = match across.normalized() {
        Some(axis) => craft3d_math::Transform::rotation(axis, sine.atan2(cosine))?,
        // Turned right round: about x.
        None => craft3d_math::Transform::rotation(frame.x(), std::f64::consts::PI)?,
    };
    Frame::new(frame.origin(), normal, turn.vector(frame.x()))
}

/// What one step of the way of a copy does to a face's frame: the map of
/// its instance, where a face operation moved the face (the point on its
/// plane, its outward normal, whether it is the face of a hollow, and
/// whether the face may have turned), and whether a subtraction turned it
/// inside out.
type Carried = (
    craft3d_math::Transform,
    Option<(Vec3, Vec3, bool, bool)>,
    bool,
);

/// One step of the way of a copy (see [`resolve_face`]): `link` is what
/// the face is called in the body of `feature`, where it is in instance
/// `instance`. Pushes the map of that instance, and how the feature moved
/// or turned the face, onto `maps`.
fn walk_copy(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    feature: craft3d_doc::FeatureId,
    instance: u64,
    link: &PersistentName,
    maps: &mut Vec<Carried>,
) -> Result<(), String> {
    let (source, outcome, _) = maker(doc, earlier, feature, name)?;
    let made_by = &source.name;
    let listed = outcome.names.as_ref().map_or(&[][..], |n| &n.faces[..]);
    let Some(instances) = &outcome.instances else {
        return Err(no_such_face(made_by, link, listed));
    };
    let instance_map = usize::try_from(instance)
        .ok()
        .and_then(|i| instances.get(i));
    let Some(map) = instance_map else {
        return Err(format!(
            "plane: `{made_by}` has {} instance(s), numbered from 0; there is no {link}",
            instances.len()
        ));
    };
    // By its own name or an alias: a face a boolean merged from faces
    // of both operands answers to the name of each.
    let names = outcome.names.as_ref();
    let found =
        names.is_some_and(|names| names.all_face_names().any(|face| same_plane(face, link)));
    if !found {
        return Err(no_such_face(made_by, link, listed));
    }
    // A face operation moves faces one by one, which no map of the
    // whole body says: the face is found where it lies now.
    let moved = match source.kind {
        FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Plate(_)
        | FeatureKind::Shell(_) => {
            let index = names.and_then(|names| {
                (0..names.faces.len())
                    .find(|&face| names.names_of(face).any(|own| same_plane(own, link)))
            });
            let face = index.and_then(|index| outcome.body.as_ref()?.faces.get(index));
            match face.map(|face| (&face.surface, face.reversed)) {
                Some((craft3d_geom::Surface::Plane(plane), reversed)) => {
                    let outward = if reversed {
                        -plane.frame().z()
                    } else {
                        plane.frame().z()
                    };
                    // The faces of a shell's hollow (instance 1)
                    // look the other way than the faces they lie
                    // behind.
                    // So do the faces a layer has from under the faces
                    // it thickens.
                    let hollow = matches!(
                        source.kind,
                        FeatureKind::Shell(_) | FeatureKind::Thicken(_) | FeatureKind::Plate(_)
                    ) && instance == 1;
                    // A moved or drafted face may also have turned.
                    let turns = matches!(
                        source.kind,
                        FeatureKind::MoveFace(_) | FeatureKind::Draft(_)
                    );
                    Some((plane.frame().origin(), outward, hollow, turns))
                }
                Some((surface, _)) => {
                    return Err(format!(
                        "plane: the face {name} is a {} face; a sketch needs a flat one",
                        surface.kind()
                    ));
                }
                None => return Err(no_such_face(made_by, link, listed)),
            }
        }
        _ => None,
    };
    // A subtraction keeps the faces of its tool turned inside out:
    // the floor of a pocket looks up out of the material, where the
    // bottom of the cutter it was looked down. A sketch on it looks
    // out of the material, as on every other face.
    let turned = matches!(
        source.kind,
        FeatureKind::Boolean(craft3d_doc::Boolean {
            op: craft3d_doc::BooleanOp::Subtract,
            ..
        })
    ) && instance == 1;
    maps.push((*map, moved, turned));
    Ok(())
}

/// The frame of a face of the feature that first made it: `link` is the
/// end of the chain of `whole`.
fn made_face(
    doc: &Document,
    earlier: &[FeatureResult],
    link: &PersistentName,
    whole: &PersistentName,
) -> Result<Frame, String> {
    let (source, outcome, body) = maker(doc, earlier, link.feature, whole)?;
    let made_by = &source.name;
    let Some(names) = &outcome.names else {
        return Err(format!(
            "plane: `{made_by}` is a {}, whose faces have no names yet (the primitives do not name their faces)",
            source.kind.type_name()
        ));
    };
    let exact = names.faces.iter().position(|face| face == link);
    // The caps of one role all lie in one plane and share one frame (that
    // of the extruded sketch), whatever their outlines are. So when the
    // outline a cap was named after has changed, the sketch still knows
    // its plane: that of any cap of the same role of the same feature.
    // A side face has no such fallback: its plane is its own.
    let same_plane = || {
        matches!(link.role, NameRole::StartCap | NameRole::EndCap)
            .then(|| names.faces.iter().position(|face| face.role == link.role))
            .flatten()
    };
    let Some(index) = exact.or_else(same_plane) else {
        if let Some(why) = crate::pass::absent_name(doc, earlier, whole) {
            return Err(format!(
                "plane: `{made_by}`, which makes the face {whole}, does not have it: {why}"
            ));
        }
        return Err(no_such_face(made_by, link, &names.faces));
    };
    let Some(face) = body.faces.get(index) else {
        return Err(format!("plane: the face {link} is missing from its body"));
    };
    let craft3d_geom::Surface::Plane(plane) = &face.surface else {
        return Err(format!(
            "plane: the face {link} is a {} face; a sketch needs a flat one",
            face.surface.kind()
        ));
    };
    let frame = plane.frame();
    // The kernel puts the origin of a side face where its line starts, at
    // the offset the extrusion starts. Both move with upstream dimensions
    // (a corner radius, the distance, the direction). A sketch on a side
    // is measured from the origin of the extruded sketch instead, dropped
    // straight onto the face: a point that only moves when the face itself
    // moves along its normal.
    let mut origin = frame.origin();
    if link.root().role == NameRole::Side
        && let FeatureKind::Extrude(extrusion) = &source.kind
        && let Some(base) = earlier
            .iter()
            .find(|r| r.id == extrusion.sketch)
            .and_then(|r| r.sketch.as_ref())
    {
        origin = base.origin - frame.z() * (base.origin - origin).dot(frame.z());
    }
    if face.reversed {
        frame.reversed_at(origin)
    } else {
        frame.at(origin)
    }
    .ok_or_else(|| format!("plane: the face {link} has a degenerate plane"))
}

/// The axes of a sketch on a face whose x axis is turned to the world
/// direction `axis` (`SketchPlane::Face::x_axis`): its projection onto the
/// plane of `normal`, and y = normal × x, so the frame keeps its normal and
/// stays right-handed. A direction along the normal (within a part in
/// 10^9 of its length) has no projection and is refused.
fn turned_axes(
    variables: &Evaluation,
    axis: &[craft3d_doc::Expr; 3],
    normal: Vec3,
) -> Result<(Vec3, Vec3), String> {
    let mut d = [0.0; 3];
    for ((slot, expr), c) in d.iter_mut().zip(axis).zip(["x", "y", "z"]) {
        let q = variables
            .evaluate(expr.source())
            .map_err(|e| format!("plane.xAxis.{c}: `{}`: {e}", expr.source()))?;
        if !q.dimension.is_dimensionless() || !q.value.is_finite() {
            return Err(format!(
                "plane.xAxis.{c}: `{}` is not a plain number (a direction has no unit)",
                expr.source()
            ));
        }
        *slot = q.value;
    }
    let [x, y, z] = d;
    let wanted = Vec3::new(x, y, z);
    let along = wanted - normal * wanted.dot(normal);
    let projected = along.length();
    if projected.is_nan() || projected <= 1e-9 * wanted.length() {
        return Err(format!(
            "plane.xAxis: [{x}, {y}, {z}] is along the face's normal (or zero), so it gives the sketch no x axis on the face"
        ));
    }
    let x_axis = along
        .normalized()
        .ok_or("plane.xAxis: the direction cannot be normalised")?;
    let y_axis = normal
        .cross(x_axis)
        .normalized()
        .ok_or("plane.xAxis: the face's normal and the direction give no y axis")?;
    Ok((x_axis, y_axis))
}

/// Where the sketch lies: its plane with the offset applied.
pub(crate) fn place(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    earlier: &[FeatureResult],
) -> Result<Placement, String> {
    let offset = length(doc, variables, "plane.offset", sketch.plane.offset())?;
    if !offset.is_finite() {
        return Err("plane.offset is not finite".into());
    }
    match &sketch.plane {
        SketchPlane::World { plane, .. } => {
            let (x_axis, y_axis, normal) = axes(*plane);
            Ok(Placement {
                origin: normal * offset,
                x_axis,
                y_axis,
                normal,
            })
        }
        SketchPlane::Face { face, x_axis, .. } => {
            let frame = resolve_face(doc, earlier, face)?;
            let normal = frame.z();
            let (x_axis, y_axis) = match x_axis {
                None => (frame.x(), frame.y()),
                Some(axis) => turned_axes(variables, axis, normal)?,
            };
            Ok(Placement {
                origin: frame.origin() + normal * offset,
                x_axis,
                y_axis,
                normal,
            })
        }
        SketchPlane::Datum { plane, .. } => {
            let frame = crate::datum::datum_plane(doc, earlier, *plane, "plane")?;
            Ok(Placement {
                origin: frame.origin() + frame.z() * offset,
                x_axis: frame.x(),
                y_axis: frame.y(),
                normal: frame.z(),
            })
        }
    }
}

/// Solves one sketch from its stored coordinates. The result is `None`
/// only when the plane itself could not be found.
pub(crate) fn solve_sketch(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    placement: Result<Placement, String>,
    fits: &FormulaFits,
) -> (Status, Option<SketchResult>) {
    crate::work::sketch_solve();
    let Placement {
        origin,
        x_axis,
        y_axis,
        normal,
    } = match placement {
        Ok(placement) => placement,
        Err(reason) => return (Status::Failed(reason), None),
    };
    let (points, radii) = stored(sketch);
    let evaluated = crate::sketch_pattern::evaluate(doc, variables, sketch);
    let patterns = evaluated.as_ref().map_or_else(|_| Vec::new(), Clone::clone);
    // The kernel's numbers for curves of nested patterns: from the counts
    // alone, before anything is solved.
    let curve_ids = crate::sketch_pattern::CurveIds::of(sketch, &patterns);
    // The formula curves as fitted, ends where the formulas put them: what
    // an unsolved sketch draws.
    let fitted: Vec<FittedCurve> = fits
        .iter()
        .filter_map(|(id, fit)| {
            let fit = fit.as_ref().ok()?;
            Some(FittedCurve {
                id: *id,
                spline: fit.spline.clone(),
                deviation: Some(fit.deviation),
            })
        })
        .collect();
    let unsolved = |reason: String| {
        let result = SketchResult {
            origin,
            x_axis,
            y_axis,
            normal,
            dof: None,
            constraints: sketch
                .constraints
                .iter()
                .map(|c| ConstraintResult {
                    id: c.id,
                    state: ConstraintState::Unsolved,
                    residual: None,
                })
                .collect(),
            points: points.clone(),
            radii: radii.clone(),
            patterns: patterns.clone(),
            image_places: Vec::new(),
            curve_ids: curve_ids.clone().unwrap_or_default(),
            curves: fitted.clone(),
        };
        (Status::Failed(reason), Some(result))
    };
    if let Err(reason) = &evaluated {
        return unsolved(reason.clone());
    }
    let curve_ids = match &curve_ids {
        Ok(ids) => ids.clone(),
        Err(reason) => return unsolved(reason.clone()),
    };
    let problem = match problem(doc, variables, sketch, &patterns, fits) {
        Ok(problem) => problem,
        Err(reason) => return unsolved(reason),
    };
    let Some(tolerance) = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular) else {
        return unsolved("the document's tolerance is not positive and finite".into());
    };
    let solution = match solve::solve(&problem.input, &solve::SolveOptions::new(tolerance)) {
        Ok(solution) => solution,
        Err(error) => return unsolved(explain(&error, &problem)),
    };
    // A solution the sketch was not carried to all the way may lie on
    // another root than the one it was on (folded over), and which one it
    // found would depend on the path of edits: the sketch fails and nothing
    // of the solve is kept (the developer's decision of 2026-10-17). The
    // solve's own verdict comes first (below): a conflict is not fixed by
    // a smaller step.
    // (Made here, while the stored coordinates are at hand.)
    let carry_failed = carry_failure(solution.carry.as_ref(), &problem, sketch).map(unsolved);

    let mut points = Vec::new();
    let mut radii = Vec::new();
    for (id, entity) in problem.entities.iter().zip(&solution.entities) {
        match *entity {
            solve::Entity::Point { x, y } => points.push((*id, x, y)),
            solve::Entity::Circle { radius, .. } => radii.push((*id, radius)),
            solve::Entity::Line { .. }
            | solve::Entity::Arc { .. }
            | solve::Entity::Image { .. } => {}
        }
    }
    radii.sort_by_key(|(id, _)| *id);
    points.sort_by_key(|(id, ..)| *id);
    let state = |report: &solve::ConstraintReport| match report.status {
        solve::ConstraintStatus::Satisfied => ConstraintState::Satisfied,
        solve::ConstraintStatus::Redundant => ConstraintState::Redundant,
        solve::ConstraintStatus::Conflicting => ConstraintState::Conflicting,
    };
    let handed: BTreeMap<ConstraintId, &solve::ConstraintReport> = problem
        .constraints
        .iter()
        .copied()
        .zip(&solution.constraints)
        .collect();
    // One per constraint of the sketch, in its order: those left out are
    // missing.
    let constraints: Vec<ConstraintResult> = sketch
        .constraints
        .iter()
        .map(|c| match handed.get(&c.id) {
            Some(report) => ConstraintResult {
                id: c.id,
                state: state(report),
                residual: Some(report.residual),
            },
            None => ConstraintResult {
                id: c.id,
                state: ConstraintState::Missing,
                residual: None,
            },
        })
        .collect();
    let mut patterns = patterns;
    let mut broken_chains = Vec::new();
    for (pattern, report) in problem
        .chains
        .iter()
        .zip(solution.constraints.iter().skip(problem.constraints.len()))
    {
        let chain = state(report);
        if chain == ConstraintState::Conflicting {
            broken_chains.push(*pattern);
        }
        if let Some(found) = patterns.iter_mut().find(|p| p.id == *pattern) {
            found.chain = Some(chain);
        }
    }
    // A ring closes when its instances make a whole turn and the last
    // joint, as the turns put it, lies on the first within the tolerance.
    {
        let lookup = |id: EntityId| match points.binary_search_by_key(&id, |(p, ..)| *p) {
            Ok(at) => points.get(at).map(|&(_, x, y)| [x, y]),
            Err(_) => None,
        };
        let rings: Vec<usize> = patterns
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.placed, Placed::Turn { chain: Some(_), .. }) && p.closes)
            .map(|(n, _)| n)
            .collect();
        let closing: Vec<(usize, bool)> = {
            let mut placer = crate::sketch_pattern::Placer::new(sketch, &patterns, lookup);
            rings
                .iter()
                .map(|n| (*n, placer.ring_closes(*n, doc.tolerance.linear)))
                .collect()
        };
        for (n, closes) in closing {
            if let Some(pattern) = patterns.get_mut(n) {
                pattern.closes = closes;
            }
        }
    }
    // The image points last: a ring's last joint is its first only once
    // `closes` is final, and the image points take the same numbers as
    // the instance curves.
    place_images(sketch, &patterns, &mut points);
    let conflicting: Vec<ConstraintId> = constraints
        .iter()
        .filter(|c| c.state == ConstraintState::Conflicting)
        .map(|c| c.id)
        .collect();
    // Why a blamed constraint has no value at all where the others put
    // the sketch (an angle on a line that has shrunk to a point).
    let undefined = |id: &ConstraintId| {
        problem
            .constraints
            .iter()
            .zip(&solution.constraints)
            .find(|(c, _)| *c == id)
            .and_then(|(_, report)| report.undefined)
    };
    let missing = || {
        let shown: Vec<String> = problem
            .left_out
            .iter()
            .take(5)
            .map(|(id, why)| format!("{id} uses {why}"))
            .collect();
        format!(
            "{}; it is left out until the instance is back",
            shown.join("; ")
        )
    };
    // The ends of formula curves the constraints would not leave where the
    // formulas put them.
    let moved_ends: Vec<String> = problem
        .pins
        .iter()
        .zip(
            solution
                .constraints
                .iter()
                .skip(problem.constraints.len() + problem.chains.len()),
        )
        .filter(|(_, report)| report.status == solve::ConstraintStatus::Conflicting)
        .map(|((curve, point), _)| format!("{point} (an end of {curve})"))
        .collect();
    let (curves, unfitted) = fitted_curves(sketch, &fitted, &points);
    let status = match conflicting.as_slice() {
        [] if !moved_ends.is_empty() => Status::Failed(format!(
            "the constraints move {} away from where the formulas put it; a point a formula curve ends on is placed by the curve: take those constraints off it, or tie it to the other curve with a coincident constraint",
            moved_ends.join(", ")
        )),
        [] if unfitted.is_some() => Status::Failed(unfitted.unwrap_or_default()),
        [] if !broken_chains.is_empty() => Status::Failed(format!(
            "the chain of {} does not close: the end of its seed has to be its start turned by the angle about the centre, and the constraints do not let it; change or delete them, or the angle",
            broken_chains
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" and ")
        )),
        [] if !problem.left_out.is_empty() => Status::Failed(missing()),
        [] => Status::Ok,
        [one] => Status::Failed(match undefined(one) {
            Some(reason) => format!(
                "constraint {one} has no meaning where the constraints before it put the sketch: {reason}; change or delete it (sketch.edit with a new value, or sketch.delete)"
            ),
            None => format!(
                "constraint {one} conflicts with the constraints before it; change or delete it (sketch.edit with a new value, or sketch.delete)"
            ),
        }),
        many => Status::Failed(format!(
            "constraints {} conflict with the constraints before them; change or delete them (sketch.edit with a new value, or sketch.delete)",
            list(many)
        )),
    };
    let status = match (status, carry_failed) {
        (status, None) => status,
        (Status::Ok, Some(failed)) => return failed,
        // The sketch fails for a reason of its own; that is the one to
        // fix, and nothing is stored either way (`clean`).
        (Status::Failed(reason), Some(_)) => Status::Failed(format!(
            "{reason} (nor could the sketch be carried along all the way to where its image points are now)"
        )),
        (status, Some(_)) => status,
    };
    // The places of the image points with the numbers of their maps: the
    // next solve carries the sketch along from them when an edit moves
    // them.
    let found = |id: EntityId| match points.binary_search_by_key(&id, |(point, ..)| *point) {
        Ok(at) => points.get(at).map(|&(_, x, y)| [x, y]),
        Err(_) => None,
    };
    let image_places = problem
        .image_maps
        .iter()
        .filter_map(|(id, map)| {
            let at = found(*id)?;
            let place = ImagePlace {
                entity: *id,
                at,
                map: Some(place_map(map)),
            };
            let finite = at.iter().all(|v| v.is_finite())
                && match place.map {
                    Some(PlaceMap::Shift { offset }) => offset.iter().all(|v| v.is_finite()),
                    Some(PlaceMap::Chain { k }) => k.is_finite(),
                    Some(PlaceMap::Turn { angle, at }) => {
                        angle.is_finite() && at.iter().all(|v| v.is_finite())
                    }
                    None => true,
                };
            finite.then_some(place)
        })
        .collect();
    let result = SketchResult {
        origin,
        x_axis,
        y_axis,
        normal,
        dof: Some(solution.dof),
        constraints,
        points,
        radii,
        patterns,
        image_places,
        curve_ids,
        curves,
    };
    (status, Some(result))
}

/// The fitted curves of a solved sketch, in the order of their ids: the
/// formula curves as `fitted` has them, with their ends on the solved end
/// points (within the tolerance of the fit's ends, as the solve holds
/// them), and the live splines fitted through their solved points. With
/// the first reason a live spline could not be fitted.
fn fitted_curves(
    sketch: &Sketch,
    fitted: &[FittedCurve],
    points: &Points,
) -> (Vec<FittedCurve>, Option<String>) {
    let solved_at = |id: EntityId| match points.binary_search_by_key(&id, |(p, ..)| *p) {
        Ok(at) => points.get(at).map(|&(_, x, y)| [x, y]),
        Err(_) => None,
    };
    let mut curves = Vec::new();
    let mut unfitted = None;
    for entity in &sketch.entities {
        match &entity.geometry {
            Geometry::Formula { start, end, .. } => {
                let Some(mut curve) = fitted.iter().find(|c| c.id == entity.id).cloned() else {
                    continue;
                };
                if let (Some(at), Some(first)) =
                    (solved_at(*start), curve.spline.control.first_mut())
                {
                    *first = at;
                }
                if let (Some(at), Some(last)) = (solved_at(*end), curve.spline.control.last_mut()) {
                    *last = at;
                }
                curves.push(curve);
            }
            Geometry::Interpolated { degree, through } => {
                let at: Option<Vec<[f64; 2]>> = through.iter().map(|id| solved_at(*id)).collect();
                let spline = at
                    .ok_or_else(|| "a point it passes through has no solved position".to_string())
                    .and_then(|at| curve_fit::interpolated(*degree, &at));
                match spline {
                    Ok(spline) => curves.push(FittedCurve {
                        id: entity.id,
                        spline,
                        deviation: None,
                    }),
                    Err(why) => {
                        unfitted.get_or_insert(format!("{}: {why}", entity.id));
                    }
                }
            }
            _ => {}
        }
    }
    curves.sort_by_key(|c| c.id);
    (curves, unfitted)
}

/// Why a solve the sketch was not carried to all the way cannot be used,
/// or `None` when it was carried all the way or nothing had to be.
fn carry_failure(
    carry: Option<&solve::Carried>,
    problem: &Problem,
    sketch: &Sketch,
) -> Option<String> {
    let images = |moved: &[solve::EntityId]| {
        let named: Vec<String> = moved
            .iter()
            .filter_map(|id| problem.entities.get(id.index()))
            .take(3)
            .map(|id| {
                match sketch
                    .entities
                    .iter()
                    .find(|e| e.id == *id)
                    .map(|e| &e.geometry)
                {
                    Some(Geometry::Image { of, index, .. }) => format!("{id} = {of}[{index}]"),
                    _ => id.to_string(),
                }
            })
            .collect();
        let more = moved.len().saturating_sub(named.len());
        let listed = if more > 0 {
            format!("{} and {more} more", named.join(", "))
        } else {
            named.join(", ")
        };
        if moved.len() == 1 {
            format!("image point {listed} went from where it was to where its pattern puts it now")
        } else {
            format!(
                "image points {listed} went from where they were to where their patterns put them now"
            )
        }
    };
    const FOLDED: &str = "solving from anywhere else could find another solution than the one the sketch was on (folded over it), so the sketch is not solved and keeps its coordinates";
    match carry? {
        solve::Carried::Whole => None,
        solve::Carried::Stopped {
            reached,
            moved,
            out_of_work: false,
        } => Some(format!(
            "the sketch could be carried along only {:.1}% of the way as {}: past there its constraints have no solution that moves with the images; {FOLDED}; make the change in smaller steps, or change the constraints on the geometry that hangs on the images",
            reached * 100.0,
            images(moved)
        )),
        solve::Carried::Stopped {
            reached,
            moved,
            out_of_work: true,
        } => Some(format!(
            "carrying the sketch along as {} ran out of work {:.1}% of the way; {FOLDED}; make the change in smaller steps",
            images(moved),
            reached * 100.0
        )),
        solve::Carried::Skipped { reason, moved } => Some(format!(
            "the sketch could not be carried along as {}: {reason}; {FOLDED}; make the change in smaller steps, or use other constraints on the images (each on fewer image points)",
            images(moved)
        )),
    }
}

/// The places of image points a document lacks (a file written before
/// places were stored), from the solution of the document as it is, per
/// sketch: taken before an edit, so that the solve after it can carry the
/// sketch along from there ([`record_image_places`]). A sketch that does
/// not solve cleanly gives none. Reads the document only.
pub fn image_places_to_record(
    doc: &Document,
    cache: &mut crate::RegenCache,
) -> Vec<(FeatureId, Vec<ImagePlace>)> {
    let Ok(table) = doc.variable_table() else {
        return Vec::new();
    };
    let variables = table.evaluate();
    let mut found = Vec::new();
    for feature in &doc.features {
        let FeatureKind::Sketch(sketch) = &feature.kind else {
            continue;
        };
        // The places are kept in ascending order of entity (`validate`).
        let has = |id: EntityId| {
            sketch
                .image_places
                .binary_search_by_key(&id, |p| p.entity)
                .is_ok()
        };
        let lacking = sketch
            .entities
            .iter()
            .any(|e| matches!(e.geometry, Geometry::Image { .. }) && !has(e.id));
        let on = matches!(crate::pass::switch(&variables, feature), Ok(None));
        if !on || !lacking {
            continue;
        }
        let (_, Some(result)) = cache.solve(feature.id, doc, &variables, sketch) else {
            continue;
        };
        if !clean(&result) {
            continue;
        }
        let places: Vec<ImagePlace> = result
            .image_places
            .iter()
            .filter(|p| !has(p.entity))
            .copied()
            .collect();
        if !places.is_empty() {
            found.push((feature.id, places));
        }
    }
    found
}

/// Adds the places [`image_places_to_record`] found to the sketches that
/// still have those image points without a place.
pub fn record_image_places(doc: &mut Document, places: Vec<(FeatureId, Vec<ImagePlace>)>) {
    for (id, places) in places {
        let Some(FeatureKind::Sketch(sketch)) = doc
            .features
            .iter_mut()
            .find(|f| f.id == id)
            .map(|f| &mut f.kind)
        else {
            continue;
        };
        sketch.image_places.extend(places);
        // Keeps the first of each (the one already there) and drops those
        // whose image went in the edit.
        sketch.tidy_image_places();
    }
}

/// A solution to start from: solved, with no constraint conflicting or
/// missing and every ring closed.
fn clean(result: &SketchResult) -> bool {
    result.dof.is_some()
        // A solution with a conflict in it is not a place to start from:
        // it may have lost what the sketch looked like (a line shrunk to
        // a point no longer says which way it pointed). The sketch keeps
        // its last good coordinates until the conflict is gone, and then
        // comes back on the side it was on.
        && result.constraints.iter().all(|c| {
            c.state != ConstraintState::Conflicting && c.state != ConstraintState::Missing
        })
        && result
            .patterns
            .iter()
            .all(|p| p.chain != Some(ConstraintState::Conflicting))
}

/// Solves every sketch and stores the solution in the document, so the
/// next solve starts from it. A sketch that cannot be solved, or only with
/// a constraint left out as conflicting, or that could not be carried
/// along all the way to where its image points are now, keeps its
/// coordinates and image places. Returns
/// true when anything moved.
pub fn settle_sketches(doc: &mut Document) -> bool {
    settle_sketches_cached(doc, &mut crate::RegenCache::default())
}

/// [`settle_sketches`], solving only the sketches that have changed since
/// `cache` last saw them solved. The outcome is the same.
pub fn settle_sketches_cached(doc: &mut Document, cache: &mut crate::RegenCache) -> bool {
    let Ok(table) = doc.variable_table() else {
        return false;
    };
    let variables = table.evaluate();
    let mut solved = Vec::new();
    for (index, feature) in doc.features.iter().enumerate() {
        if let FeatureKind::Sketch(sketch) = &feature.kind
            // A sketch switched off is left as it is, as when suppressed.
            && matches!(crate::pass::switch(&variables, feature), Ok(None))
            // Where the sketch lies does not change how it solves.
            && let (_, Some(result)) = cache.solve(feature.id, doc, &variables, sketch)
            && clean(&result)
        {
            solved.push((index, result));
        }
    }
    let mut moved = false;
    for (index, result) in solved {
        let Some(FeatureKind::Sketch(sketch)) = doc.features.get_mut(index).map(|f| &mut f.kind)
        else {
            continue;
        };
        // The image points with the points: the next solve carries the
        // sketch along from them when an edit moves them.
        let places = result.image_places.clone();
        if sketch.image_places != places {
            sketch.image_places = places;
            moved = true;
        }
        for entity in &mut sketch.entities {
            match &mut entity.geometry {
                Geometry::Point { x, y } => {
                    if let Some((sx, sy)) = result.point(entity.id)
                        && sx.is_finite()
                        && sy.is_finite()
                        && (*x, *y) != (sx, sy)
                    {
                        (*x, *y) = (sx, sy);
                        moved = true;
                    }
                }
                Geometry::Circle { radius, .. } => {
                    if let Some(r) = result.radius(entity.id)
                        && r.is_finite()
                        && r > 0.0
                        && *radius != r
                    {
                        *radius = r;
                        moved = true;
                    }
                }
                Geometry::Line { .. }
                | Geometry::Arc { .. }
                | Geometry::Spline { .. }
                | Geometry::Image { .. }
                | Geometry::Formula { .. }
                | Geometry::Interpolated { .. } => {}
            }
        }
    }
    moved
}
