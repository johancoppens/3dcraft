//! From a [`Sketch`] to scalar equations over a vector of unknowns.
//!
//! [`prepare`] validates the sketch, numbers the unknowns and turns each
//! constraint into an [`Equation`] with one or two rows. Decisions that pick
//! one of several mirror solutions (which side of a line, inside or outside
//! a circle) are taken here, once, from the starting guess, so the equations
//! are smooth during the iteration and the solve stays on its branch.

use std::f64::consts::{PI, TAU};

use crate::dual::{Dual, SLOTS, angle_of, length};
use crate::error::{Site, SolveError};
use crate::model::{Constraint, Entity, EntityId, Sketch};
use crate::{MAX_ENTITIES, MAX_EQUATIONS, MAX_UNKNOWNS};

const SHORT_LINE: &str = "a line shorter than the linear tolerance has no direction";
const NEGATIVE_RADIUS: &str = "a circle with a negative radius";
const SMALL_ARC: &str = "an arc smaller than the linear tolerance has no tangent";

/// Why an equation has no value at some point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fail {
    Degenerate(&'static str),
    NonFinite,
}

/// A point: the index of its x unknown; y follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pt(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ln {
    a: Pt,
    b: Pt,
}

/// A circle (radius is an unknown) or an arc (radius is `|c - s|`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Round {
    Circle { c: Pt, r: usize },
    Arc { c: Pt, s: Pt },
}

impl Round {
    fn center(&self) -> Pt {
        match self {
            Round::Circle { c, .. } | Round::Arc { c, .. } => *c,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Axis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Equation {
    /// Two rows: `a - b`.
    Same(Pt, Pt),
    /// `a.x - b.x` or `a.y - b.y`.
    SameCoordinate(Axis, Pt, Pt),
    /// Signed distance of the point from the line.
    PointOnLine(Pt, Ln),
    /// `|p - c| - r`.
    PointOnRound(Pt, Round),
    /// Sine of the angle between the directions.
    Parallel(Ln, Ln),
    /// Cosine of the angle between the directions.
    Perpendicular(Ln, Ln),
    /// `side * signed_distance(center, line) - r`.
    TangentLine { line: Ln, round: Round, side: f64 },
    /// A line tangent to a round at a point `p` that other constraints keep
    /// on both: the radius to `p` is perpendicular to the line. The row is
    /// `(c - p) . direction`.
    TangentLineAt { line: Ln, c: Pt, p: Pt },
    /// Two rounds tangent at a point `p` that is kept on both: the centres
    /// and `p` are in line. The row is the distance of `c2` from the line
    /// through `p` and `c1`.
    TangentRoundAt { c1: Pt, c2: Pt, p: Pt },
    /// `|c1 - c2| - (r1 + r2)`.
    TangentExternal(Round, Round),
    /// `|c1 - c2| - bigger * (r1 - r2)`.
    TangentInternal { a: Round, b: Round, bigger: f64 },
    /// `|a| - |b|`.
    EqualLength(Ln, Ln),
    /// `r1 - r2`.
    EqualRadius(Round, Round),
    /// Two rows: the midpoint of `a`,`b` on the axis, and `b - a` along it.
    Symmetric { a: Pt, b: Pt, axis: Ln },
    /// Two rows: `p - (a + b) / 2`.
    Midpoint(Pt, Ln),
    /// Two rows: `p - at`.
    Fixed(Pt, [f64; 2]),
    /// `|a - b| - value`, `value > 0`.
    Distance(Pt, Pt, f64),
    /// `side * signed_distance(p, line) - value`.
    PointLineDistance {
        point: Pt,
        line: Ln,
        side: f64,
        value: f64,
    },
    /// `side * (b - a) - value` along one axis.
    Offset {
        axis: Axis,
        a: Pt,
        b: Pt,
        side: f64,
        value: f64,
    },
    /// `angle(b) - angle(a) - value`, wrapped to `[-pi, pi)`.
    Angle(Ln, Ln, f64),
    /// `factor * r - value` (factor 2 for a diameter).
    Radius {
        round: Round,
        factor: f64,
        value: f64,
    },
    /// The built-in arc equation `|s - c| - |e - c|`.
    ArcClosure { c: Pt, s: Pt, e: Pt },
}

/// The rows of one equation at one point: residuals with their derivatives
/// with respect to the unknowns listed in `slots[..used]`.
pub(crate) struct Evaluated {
    pub count: usize,
    pub rows: [Dual; 2],
    pub slots: [usize; SLOTS],
    pub used: usize,
}

struct Ctx<'a> {
    x: &'a [f64],
    min_len: f64,
    slots: [usize; SLOTS],
    used: usize,
}

type Vec2 = [Dual; 2];

impl<'a> Ctx<'a> {
    fn new(x: &'a [f64], min_len: f64) -> Self {
        Ctx {
            x,
            min_len,
            slots: [0; SLOTS],
            used: 0,
        }
    }

    /// The unknown `ix` as a local variable (one slot per distinct unknown).
    fn var(&mut self, ix: usize) -> Result<Dual, Fail> {
        let value = *self.x.get(ix).ok_or(Fail::NonFinite)?;
        let known = self.slots.iter().take(self.used).position(|s| *s == ix);
        let slot = match known {
            Some(slot) => slot,
            None => {
                let slot = self.used;
                *self.slots.get_mut(slot).ok_or(Fail::NonFinite)? = ix;
                self.used += 1;
                slot
            }
        };
        Dual::variable(value, slot).ok_or(Fail::NonFinite)
    }

    fn pt(&mut self, p: Pt) -> Result<Vec2, Fail> {
        Ok([self.var(p.0)?, self.var(p.0 + 1)?])
    }

    /// Start point, direction and length of a line that has a direction.
    fn line(&mut self, l: Ln) -> Result<(Vec2, Vec2, Dual), Fail> {
        let a = self.pt(l.a)?;
        let b = self.pt(l.b)?;
        let [ax, ay] = a;
        let [bx, by] = b;
        let d = [bx - ax, by - ay];
        let [dx, dy] = d;
        let len = length(dx, dy);
        if !len.v.is_finite() {
            return Err(Fail::NonFinite);
        }
        if len.v <= self.min_len {
            return Err(Fail::Degenerate(SHORT_LINE));
        }
        Ok((a, d, len))
    }

    /// Distance of `p` from the line, positive on its left.
    fn signed_distance(&mut self, p: Vec2, l: Ln) -> Result<Dual, Fail> {
        let ([ax, ay], [dx, dy], len) = self.line(l)?;
        let [px, py] = p;
        Ok((dx * (py - ay) - dy * (px - ax)) / len)
    }

    fn radius(&mut self, r: Round) -> Result<Dual, Fail> {
        match r {
            Round::Circle { r, .. } => {
                let radius = self.var(r)?;
                if radius.v < 0.0 {
                    return Err(Fail::Degenerate(NEGATIVE_RADIUS));
                }
                Ok(radius)
            }
            Round::Arc { c, s } => self.distance(c, s),
        }
    }

    fn distance(&mut self, a: Pt, b: Pt) -> Result<Dual, Fail> {
        let [ax, ay] = self.pt(a)?;
        let [bx, by] = self.pt(b)?;
        Ok(length(bx - ax, by - ay))
    }

    fn coordinate(&mut self, axis: Axis, p: Pt) -> Result<Dual, Fail> {
        let [x, y] = self.pt(p)?;
        Ok(match axis {
            Axis::X => x,
            Axis::Y => y,
        })
    }

    fn eval(&mut self, eq: &Equation) -> Result<(usize, [Dual; 2]), Fail> {
        let zero = Dual::constant(0.0);
        let one = |r: Dual| (1, [r, zero]);
        Ok(match *eq {
            Equation::Same(a, b) => {
                let [ax, ay] = self.pt(a)?;
                let [bx, by] = self.pt(b)?;
                (2, [ax - bx, ay - by])
            }
            Equation::SameCoordinate(axis, a, b) => {
                one(self.coordinate(axis, a)? - self.coordinate(axis, b)?)
            }
            Equation::PointOnLine(p, l) => {
                let p = self.pt(p)?;
                one(self.signed_distance(p, l)?)
            }
            Equation::PointOnRound(p, round) => {
                one(self.distance(round.center(), p)? - self.radius(round)?)
            }
            Equation::Parallel(a, b) => {
                let (_, [ax, ay], la) = self.line(a)?;
                let (_, [bx, by], lb) = self.line(b)?;
                one((ax * by - ay * bx) / (la * lb))
            }
            Equation::Perpendicular(a, b) => {
                let (_, [ax, ay], la) = self.line(a)?;
                let (_, [bx, by], lb) = self.line(b)?;
                one((ax * bx + ay * by) / (la * lb))
            }
            Equation::TangentLine { line, round, side } => {
                let c = self.pt(round.center())?;
                one(self.signed_distance(c, line)? * side - self.radius(round)?)
            }
            Equation::TangentLineAt { line, c, p } => {
                let (_, [dx, dy], len) = self.line(line)?;
                let [cx, cy] = self.pt(c)?;
                let [px, py] = self.pt(p)?;
                one(((cx - px) * dx + (cy - py) * dy) / len)
            }
            Equation::TangentRoundAt { c1, c2, p } => {
                let [px, py] = self.pt(p)?;
                let [ax, ay] = self.pt(c1)?;
                let [bx, by] = self.pt(c2)?;
                let (ux, uy) = (ax - px, ay - py);
                let radius = length(ux, uy);
                if radius.v <= self.min_len {
                    return Err(Fail::Degenerate(SMALL_ARC));
                }
                one((ux * (by - py) - uy * (bx - px)) / radius)
            }
            Equation::TangentExternal(a, b) => {
                let gap = self.distance(a.center(), b.center())?;
                one(gap - (self.radius(a)? + self.radius(b)?))
            }
            Equation::TangentInternal { a, b, bigger } => {
                let gap = self.distance(a.center(), b.center())?;
                one(gap - (self.radius(a)? - self.radius(b)?) * bigger)
            }
            Equation::EqualLength(a, b) => one(self.distance(a.a, a.b)? - self.distance(b.a, b.b)?),
            Equation::EqualRadius(a, b) => one(self.radius(a)? - self.radius(b)?),
            Equation::Symmetric { a, b, axis } => {
                let [ax, ay] = self.pt(a)?;
                let [bx, by] = self.pt(b)?;
                let mid = [(ax + bx) * 0.5, (ay + by) * 0.5];
                let off_axis = self.signed_distance(mid, axis)?;
                let (_, [dx, dy], len) = self.line(axis)?;
                let along = ((bx - ax) * dx + (by - ay) * dy) / len;
                (2, [off_axis, along])
            }
            Equation::Midpoint(p, l) => {
                let [px, py] = self.pt(p)?;
                let [ax, ay] = self.pt(l.a)?;
                let [bx, by] = self.pt(l.b)?;
                (2, [px - (ax + bx) * 0.5, py - (ay + by) * 0.5])
            }
            Equation::Fixed(p, [tx, ty]) => {
                let [px, py] = self.pt(p)?;
                (2, [px - tx, py - ty])
            }
            Equation::Distance(a, b, value) => one(self.distance(a, b)? - value),
            Equation::PointLineDistance {
                point,
                line,
                side,
                value,
            } => {
                let p = self.pt(point)?;
                one(self.signed_distance(p, line)? * side - value)
            }
            Equation::Offset {
                axis,
                a,
                b,
                side,
                value,
            } => one((self.coordinate(axis, b)? - self.coordinate(axis, a)?) * side - value),
            Equation::Angle(a, b, value) => {
                let (_, [ax, ay], _) = self.line(a)?;
                let (_, [bx, by], _) = self.line(b)?;
                let from = angle_of(ax, ay).ok_or(Fail::Degenerate(SHORT_LINE))?;
                let to = angle_of(bx, by).ok_or(Fail::Degenerate(SHORT_LINE))?;
                let turn = to - from - value;
                one(turn.with_value(wrap_angle(turn.v)))
            }
            Equation::Radius {
                round,
                factor,
                value,
            } => one(self.radius(round)? * factor - value),
            Equation::ArcClosure { c, s, e } => one(self.distance(c, s)? - self.distance(c, e)?),
        })
    }
}

/// The angle in `[-pi, pi)` that differs from `a` by whole turns.
pub(crate) fn wrap_angle(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

impl Equation {
    /// `Parallel`, `Perpendicular` and `Angle` are measured in radians;
    /// everything else is a length.
    pub(crate) fn angular(&self) -> bool {
        matches!(
            self,
            Equation::Parallel(..) | Equation::Perpendicular(..) | Equation::Angle(..)
        )
    }

    pub(crate) fn rows(&self) -> usize {
        match self {
            Equation::Same(..)
            | Equation::Symmetric { .. }
            | Equation::Midpoint(..)
            | Equation::Fixed(..) => 2,
            _ => 1,
        }
    }

    /// Residuals and gradients at `x`. Lines whose direction is needed must
    /// be longer than `min_len`.
    pub(crate) fn eval(&self, x: &[f64], min_len: f64) -> Result<Evaluated, Fail> {
        let mut ctx = Ctx::new(x, min_len);
        let (count, rows) = ctx.eval(self)?;
        if rows.iter().take(count).any(|r| !r.is_finite()) {
            return Err(Fail::NonFinite);
        }
        Ok(Evaluated {
            count,
            rows,
            slots: ctx.slots,
            used: ctx.used,
        })
    }
}

/// A validated sketch as a system of equations.
pub(crate) struct Prepared {
    /// The starting guess.
    pub x0: Vec<f64>,
    /// Per entity: the index of its first unknown (points: x; circles:
    /// radius), if it has one.
    pub entity_var: Vec<Option<usize>>,
    /// Arc equations first, then the constraints in input order.
    pub equations: Vec<Equation>,
    /// Per equation: the constraint, or the arc entity, it came from.
    pub source: Vec<Site>,
    /// Largest magnitude among coordinates, radii and length values (1 for
    /// an all-zero sketch): the unit in which lengths are compared.
    pub magnitude: f64,
    /// Extent of the sketch: the scale of the nudge off a singular point.
    pub size: f64,
}

struct Builder<'a> {
    sketch: &'a Sketch,
    entity_var: Vec<Option<usize>>,
    x0: Vec<f64>,
    min_len: f64,
    /// Per entity: the smallest entity index among the points that
    /// coincidence constraints make the same place as this one.
    class: Vec<usize>,
    /// Per entity: the points that constraints put on this line or round,
    /// in constraint order (its own end points are not listed).
    on: Vec<Vec<EntityId>>,
}

/// Root of `i` in a forest where every parent has a smaller index.
fn find(parent: &mut [usize], mut i: usize) -> usize {
    // Path halving; the index strictly decreases, so the loop ends.
    loop {
        let Some(&p) = parent.get(i) else { return i };
        if p >= i {
            return i;
        }
        let grand = parent.get(p).copied().unwrap_or(p).min(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = grand;
        }
        i = grand;
    }
}

/// Which points the constraints make coincident, and which they put on
/// which line or round. Constraints that are not valid are skipped here;
/// they are reported when their turn comes.
fn incidences(sketch: &Sketch) -> (Vec<usize>, Vec<Vec<EntityId>>) {
    let count = sketch.entities.len();
    let is_point =
        |id: EntityId| matches!(sketch.entities.get(id.index()), Some(Entity::Point { .. }));
    let mut parent: Vec<usize> = (0..count).collect();
    let mut on: Vec<Vec<EntityId>> = vec![Vec::new(); count];
    for constraint in &sketch.constraints {
        match *constraint {
            Constraint::Coincident { a, b } | Constraint::Distance { a, b, value: 0.0 }
                if is_point(a) && is_point(b) =>
            {
                let ra = find(&mut parent, a.index());
                let rb = find(&mut parent, b.index());
                if let Some(slot) = parent.get_mut(ra.max(rb)) {
                    *slot = ra.min(rb);
                }
            }
            Constraint::PointOnLine { point, line: host }
            | Constraint::Midpoint { point, line: host }
            | Constraint::PointOnCircle {
                point,
                circle: host,
            } if is_point(point) => {
                if let Some(list) = on.get_mut(host.index()) {
                    list.push(point);
                }
            }
            _ => {}
        }
    }
    let class = (0..count).map(|i| find(&mut parent, i)).collect();
    (class, on)
}

impl Builder<'_> {
    fn entity(&self, at: Site, id: EntityId) -> Result<&Entity, SolveError> {
        self.sketch
            .entities
            .get(id.index())
            .ok_or(SolveError::UnknownEntity { at, id })
    }

    fn var(&self, at: Site, id: EntityId) -> Result<usize, SolveError> {
        self.entity_var
            .get(id.index())
            .copied()
            .flatten()
            .ok_or(SolveError::UnknownEntity { at, id })
    }

    fn wrong(&self, at: Site, id: EntityId, expected: &'static str) -> SolveError {
        match self.entity(at, id) {
            Ok(e) => SolveError::WrongKind {
                at,
                id,
                expected,
                found: e.kind(),
            },
            Err(e) => e,
        }
    }

    fn pt(&self, at: Site, id: EntityId) -> Result<Pt, SolveError> {
        match self.entity(at, id)? {
            Entity::Point { .. } => Ok(Pt(self.var(at, id)?)),
            _ => Err(self.wrong(at, id, "point")),
        }
    }

    fn ln(&self, at: Site, id: EntityId) -> Result<Ln, SolveError> {
        match *self.entity(at, id)? {
            Entity::Line { start, end } => Ok(Ln {
                a: self.pt(at, start)?,
                b: self.pt(at, end)?,
            }),
            _ => Err(self.wrong(at, id, "line")),
        }
    }

    fn round(&self, at: Site, id: EntityId) -> Result<Round, SolveError> {
        match *self.entity(at, id)? {
            Entity::Circle { center, .. } => Ok(Round::Circle {
                c: self.pt(at, center)?,
                r: self.var(at, id)?,
            }),
            Entity::Arc { center, start, .. } => Ok(Round::Arc {
                c: self.pt(at, center)?,
                s: self.pt(at, start)?,
            }),
            _ => Err(self.wrong(at, id, "circle or arc")),
        }
    }

    /// A point that the sketch keeps on both `first` and `second` (each a
    /// line or a round): a shared end point, or points tied to both by
    /// coincidence and point-on constraints. Tangency there is a statement
    /// about directions; the distance form has a double root (a zero
    /// gradient) at such a point and would leave the Jacobian singular.
    fn shared_point(
        &self,
        at: Site,
        first: EntityId,
        second: EntityId,
    ) -> Result<Option<Pt>, SolveError> {
        let class = |id: &EntityId| self.class.get(id.index()).copied();
        let mut classes: Vec<usize> = self.points_on(first).iter().filter_map(class).collect();
        classes.sort_unstable();
        for candidate in self.points_on(second) {
            let found = class(&candidate).is_some_and(|c| classes.binary_search(&c).is_ok());
            if found {
                return self.pt(at, candidate).map(Some);
            }
        }
        Ok(None)
    }

    /// End points of a line or an arc, then the points constrained onto it.
    fn points_on(&self, id: EntityId) -> Vec<EntityId> {
        let mut points = match self.sketch.entities.get(id.index()) {
            Some(Entity::Line { start, end }) | Some(Entity::Arc { start, end, .. }) => {
                vec![*start, *end]
            }
            _ => Vec::new(),
        };
        if let Some(listed) = self.on.get(id.index()) {
            points.extend_from_slice(listed);
        }
        points
    }

    /// The value of a one-row equation at the starting guess.
    fn probe(&self, index: usize, eq: &Equation) -> Result<f64, SolveError> {
        match eq.eval(&self.x0, self.min_len) {
            Ok(e) => Ok(e.rows.first().map_or(0.0, |r| r.v)),
            Err(fail) => Err(start_failure(Site::Constraint(index), fail)),
        }
    }

    fn constraint(&self, index: usize, c: &Constraint) -> Result<Equation, SolveError> {
        let at = Site::Constraint(index);
        let finite = |v: f64| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err(SolveError::NonFinite { at })
            }
        };
        let length_value = |v: f64| {
            if finite(v)? < 0.0 {
                Err(SolveError::InvalidValue {
                    constraint: index,
                    reason: "a distance or radius cannot be negative",
                })
            } else {
                Ok(v)
            }
        };
        // +1 when the probed quantity is on the positive side (or on the
        // fence), -1 otherwise.
        let side_of = |eq: Equation| -> Result<f64, SolveError> {
            match self.probe(index, &eq) {
                Ok(v) => Ok(if v < 0.0 { -1.0 } else { 1.0 }),
                // No side to read off a line without direction; the
                // constraint is undefined here and sits this pass out.
                Err(SolveError::Degenerate { .. }) => Ok(1.0),
                Err(e) => Err(e),
            }
        };
        Ok(match *c {
            Constraint::Coincident { a, b } => Equation::Same(self.pt(at, a)?, self.pt(at, b)?),
            Constraint::PointOnLine { point, line } => {
                Equation::PointOnLine(self.pt(at, point)?, self.ln(at, line)?)
            }
            Constraint::PointOnCircle { point, circle } => {
                Equation::PointOnRound(self.pt(at, point)?, self.round(at, circle)?)
            }
            Constraint::Horizontal { line } => {
                let l = self.ln(at, line)?;
                Equation::SameCoordinate(Axis::Y, l.a, l.b)
            }
            Constraint::Vertical { line } => {
                let l = self.ln(at, line)?;
                Equation::SameCoordinate(Axis::X, l.a, l.b)
            }
            Constraint::HorizontalPoints { a, b } => {
                Equation::SameCoordinate(Axis::Y, self.pt(at, a)?, self.pt(at, b)?)
            }
            Constraint::VerticalPoints { a, b } => {
                Equation::SameCoordinate(Axis::X, self.pt(at, a)?, self.pt(at, b)?)
            }
            Constraint::Parallel { a, b } => Equation::Parallel(self.ln(at, a)?, self.ln(at, b)?),
            Constraint::Perpendicular { a, b } => {
                Equation::Perpendicular(self.ln(at, a)?, self.ln(at, b)?)
            }
            Constraint::Tangent { a, b } => {
                let a_is_line = matches!(self.entity(at, a)?, Entity::Line { .. });
                let b_is_line = matches!(self.entity(at, b)?, Entity::Line { .. });
                match (a_is_line, b_is_line) {
                    (true, true) => return Err(self.wrong(at, b, "circle or arc")),
                    (true, false) | (false, true) => {
                        let (line_id, round_id) = if a_is_line { (a, b) } else { (b, a) };
                        let line = self.ln(at, line_id)?;
                        let round = self.round(at, round_id)?;
                        if let Some(p) = self.shared_point(at, line_id, round_id)? {
                            Equation::TangentLineAt {
                                line,
                                c: round.center(),
                                p,
                            }
                        } else {
                            let side = side_of(Equation::PointOnLine(round.center(), line))?;
                            Equation::TangentLine { line, round, side }
                        }
                    }
                    (false, false) => {
                        let ra = self.round(at, a)?;
                        let rb = self.round(at, b)?;
                        if let Some(p) = self.shared_point(at, b, a)? {
                            return Ok(Equation::TangentRoundAt {
                                c1: ra.center(),
                                c2: rb.center(),
                                p,
                            });
                        }
                        // gap - (r1 + r2) and gap - |r1 - r2| are equally
                        // far from zero when gap = max(r1, r2).
                        let radius_a = self.probe(
                            index,
                            &Equation::Radius {
                                round: ra,
                                factor: 1.0,
                                value: 0.0,
                            },
                        )?;
                        let radius_b = self.probe(
                            index,
                            &Equation::Radius {
                                round: rb,
                                factor: 1.0,
                                value: 0.0,
                            },
                        )?;
                        let gap =
                            self.probe(index, &Equation::Distance(ra.center(), rb.center(), 0.0))?;
                        if gap < radius_a.max(radius_b) {
                            Equation::TangentInternal {
                                a: ra,
                                b: rb,
                                bigger: if radius_a < radius_b { -1.0 } else { 1.0 },
                            }
                        } else {
                            Equation::TangentExternal(ra, rb)
                        }
                    }
                }
            }
            Constraint::Equal { a, b } => {
                if matches!(self.entity(at, a)?, Entity::Line { .. }) {
                    Equation::EqualLength(self.ln(at, a)?, self.ln(at, b)?)
                } else {
                    Equation::EqualRadius(self.round(at, a)?, self.round(at, b)?)
                }
            }
            Constraint::Concentric { a, b } => {
                Equation::Same(self.round(at, a)?.center(), self.round(at, b)?.center())
            }
            Constraint::Symmetric { a, b, axis } => Equation::Symmetric {
                a: self.pt(at, a)?,
                b: self.pt(at, b)?,
                axis: self.ln(at, axis)?,
            },
            Constraint::Midpoint { point, line } => {
                Equation::Midpoint(self.pt(at, point)?, self.ln(at, line)?)
            }
            Constraint::Fixed { point, at: target } => {
                let p = self.pt(at, point)?;
                let target = match target {
                    Some([x, y]) => [finite(x)?, finite(y)?],
                    None => [
                        self.x0.get(p.0).copied().unwrap_or(0.0),
                        self.x0.get(p.0 + 1).copied().unwrap_or(0.0),
                    ],
                };
                Equation::Fixed(p, target)
            }
            Constraint::Distance { a, b, value } => {
                let value = length_value(value)?;
                let a_is_line = matches!(self.entity(at, a)?, Entity::Line { .. });
                let b_is_line = matches!(self.entity(at, b)?, Entity::Line { .. });
                match (a_is_line, b_is_line) {
                    (true, true) => return Err(self.wrong(at, b, "point")),
                    (false, false) => {
                        let (a, b) = (self.pt(at, a)?, self.pt(at, b)?);
                        if value == 0.0 {
                            // |a - b| has no gradient where it is zero.
                            Equation::Same(a, b)
                        } else {
                            Equation::Distance(a, b, value)
                        }
                    }
                    _ => {
                        let (point, line) = if a_is_line { (b, a) } else { (a, b) };
                        let point = self.pt(at, point)?;
                        let line = self.ln(at, line)?;
                        let side = side_of(Equation::PointOnLine(point, line))?;
                        Equation::PointLineDistance {
                            point,
                            line,
                            // On the line itself there is no side to keep.
                            side: if value == 0.0 { 1.0 } else { side },
                            value,
                        }
                    }
                }
            }
            Constraint::HorizontalDistance { a, b, value }
            | Constraint::VerticalDistance { a, b, value } => {
                let axis = if matches!(c, Constraint::HorizontalDistance { .. }) {
                    Axis::X
                } else {
                    Axis::Y
                };
                let value = length_value(value)?;
                let (a, b) = (self.pt(at, a)?, self.pt(at, b)?);
                let side = side_of(Equation::SameCoordinate(axis, b, a))?;
                Equation::Offset {
                    axis,
                    a,
                    b,
                    side,
                    value,
                }
            }
            Constraint::Angle { a, b, value } => {
                Equation::Angle(self.ln(at, a)?, self.ln(at, b)?, finite(value)?)
            }
            Constraint::Radius { circle, value } => Equation::Radius {
                round: self.round(at, circle)?,
                factor: 1.0,
                value: length_value(value)?,
            },
            Constraint::Diameter { circle, value } => Equation::Radius {
                round: self.round(at, circle)?,
                factor: 2.0,
                value: length_value(value)?,
            },
        })
    }
}

/// An equation that cannot be evaluated at the starting guess, as an error.
pub(crate) fn start_failure(at: Site, fail: Fail) -> SolveError {
    match (at, fail) {
        (Site::Constraint(constraint), Fail::Degenerate(reason)) => {
            SolveError::Degenerate { constraint, reason }
        }
        _ => SolveError::NonFinite { at },
    }
}

fn too_large(what: &'static str, count: usize, limit: usize) -> Result<(), SolveError> {
    if count > limit {
        Err(SolveError::TooLarge { what, count, limit })
    } else {
        Ok(())
    }
}

pub(crate) fn prepare(sketch: &Sketch, min_len: f64) -> Result<Prepared, SolveError> {
    too_large("entities", sketch.entities.len(), MAX_ENTITIES)?;
    too_large("constraints", sketch.constraints.len(), MAX_EQUATIONS)?;

    // Unknowns: two per point, one per circle, in entity order.
    let mut x0 = Vec::new();
    let mut entity_var = Vec::with_capacity(sketch.entities.len());
    for (index, entity) in sketch.entities.iter().enumerate() {
        let at = Site::Entity(index);
        match *entity {
            Entity::Point { x, y } => {
                if !(x.is_finite() && y.is_finite()) {
                    return Err(SolveError::NonFinite { at });
                }
                entity_var.push(Some(x0.len()));
                x0.push(x);
                x0.push(y);
            }
            Entity::Circle { radius, .. } => {
                if !radius.is_finite() {
                    return Err(SolveError::NonFinite { at });
                }
                if radius < 0.0 {
                    return Err(SolveError::InvalidEntity {
                        entity: index,
                        reason: NEGATIVE_RADIUS,
                    });
                }
                entity_var.push(Some(x0.len()));
                x0.push(radius);
            }
            Entity::Line { .. } | Entity::Arc { .. } => entity_var.push(None),
        }
    }
    too_large("unknowns", x0.len(), MAX_UNKNOWNS)?;

    let (class, on) = incidences(sketch);
    let builder = Builder {
        sketch,
        entity_var,
        x0,
        min_len,
        class,
        on,
    };

    // Every entity must refer to points, used by a constraint or not.
    let mut equations = Vec::new();
    let mut source = Vec::new();
    for (index, entity) in sketch.entities.iter().enumerate() {
        let at = Site::Entity(index);
        match *entity {
            Entity::Point { .. } => {}
            Entity::Line { start, end } => {
                builder.pt(at, start)?;
                builder.pt(at, end)?;
            }
            Entity::Circle { center, .. } => {
                builder.pt(at, center)?;
            }
            Entity::Arc { center, start, end } => {
                equations.push(Equation::ArcClosure {
                    c: builder.pt(at, center)?,
                    s: builder.pt(at, start)?,
                    e: builder.pt(at, end)?,
                });
                source.push(at);
            }
        }
    }

    for (index, constraint) in sketch.constraints.iter().enumerate() {
        equations.push(builder.constraint(index, constraint)?);
        source.push(Site::Constraint(index));
    }
    let rows: usize = equations.iter().map(Equation::rows).sum();
    too_large("equations", rows, MAX_EQUATIONS)?;

    // Scale of the sketch, from everything that is a length or a place.
    let mut magnitude: f64 = 0.0;
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut extent: f64 = 0.0;
    let mut place = |p: [f64; 2]| {
        for ((lo, hi), v) in lo.iter_mut().zip(hi.iter_mut()).zip(p) {
            *lo = lo.min(v);
            *hi = hi.max(v);
        }
    };
    for entity in &sketch.entities {
        match *entity {
            Entity::Point { x, y } => {
                magnitude = magnitude.max(x.abs()).max(y.abs());
                place([x, y]);
            }
            Entity::Circle { radius, .. } => {
                magnitude = magnitude.max(radius);
                extent = extent.max(radius);
            }
            Entity::Line { .. } | Entity::Arc { .. } => {}
        }
    }
    for eq in &equations {
        match *eq {
            Equation::Fixed(_, [x, y]) => {
                magnitude = magnitude.max(x.abs()).max(y.abs());
                place([x, y]);
            }
            Equation::Distance(_, _, value)
            | Equation::PointLineDistance { value, .. }
            | Equation::Offset { value, .. }
            | Equation::Radius { value, .. } => {
                magnitude = magnitude.max(value);
                extent = extent.max(value);
            }
            _ => {}
        }
    }
    for (lo, hi) in lo.iter().zip(hi) {
        if hi >= *lo {
            extent = extent.max(hi - lo);
        }
    }
    if !(magnitude > 0.0 && magnitude.is_finite()) {
        magnitude = 1.0;
    }
    if !(extent > 0.0 && extent.is_finite()) {
        extent = magnitude;
    }

    Ok(Prepared {
        x0: builder.x0,
        entity_var: builder.entity_var,
        equations,
        source,
        magnitude,
        size: extent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every equation kind over four points and two radii, at a generic
    /// position: the automatic gradient equals central differences.
    #[test]
    fn gradients_match_finite_differences() {
        // Points P0..P3 at x[0..8], radii at x[8], x[9].
        let x = [0.3, -0.2, 2.1, 0.9, 1.7, 2.6, -0.8, 1.9, 0.75, 1.4];
        let p = |i: usize| Pt(2 * i);
        let l01 = Ln { a: p(0), b: p(1) };
        let l23 = Ln { a: p(2), b: p(3) };
        let c0 = Round::Circle { c: p(0), r: 8 };
        let c2 = Round::Circle { c: p(2), r: 9 };
        let arc = Round::Arc { c: p(1), s: p(3) };
        let equations = [
            Equation::Same(p(0), p(2)),
            Equation::SameCoordinate(Axis::X, p(0), p(2)),
            Equation::SameCoordinate(Axis::Y, p(0), p(2)),
            Equation::PointOnLine(p(2), l01),
            Equation::PointOnRound(p(2), c0),
            Equation::PointOnRound(p(2), arc),
            Equation::Parallel(l01, l23),
            Equation::Perpendicular(l01, l23),
            Equation::TangentLine {
                line: l01,
                round: c2,
                side: -1.0,
            },
            Equation::TangentLine {
                line: l01,
                round: arc,
                side: 1.0,
            },
            Equation::TangentLineAt {
                line: l01,
                c: p(2),
                p: p(3),
            },
            Equation::TangentRoundAt {
                c1: p(0),
                c2: p(2),
                p: p(3),
            },
            Equation::TangentExternal(c0, c2),
            Equation::TangentExternal(c0, arc),
            Equation::TangentInternal {
                a: c0,
                b: c2,
                bigger: -1.0,
            },
            Equation::EqualLength(l01, l23),
            Equation::EqualRadius(c0, arc),
            Equation::Symmetric {
                a: p(2),
                b: p(3),
                axis: l01,
            },
            Equation::Midpoint(p(2), l01),
            Equation::Fixed(p(1), [0.5, 0.25]),
            Equation::Distance(p(0), p(2), 1.5),
            Equation::PointLineDistance {
                point: p(3),
                line: l01,
                side: -1.0,
                value: 0.4,
            },
            Equation::Offset {
                axis: Axis::X,
                a: p(0),
                b: p(2),
                side: -1.0,
                value: 0.5,
            },
            Equation::Offset {
                axis: Axis::Y,
                a: p(0),
                b: p(2),
                side: 1.0,
                value: 0.5,
            },
            Equation::Angle(l01, l23, 0.3),
            Equation::Radius {
                round: c0,
                factor: 2.0,
                value: 1.0,
            },
            Equation::Radius {
                round: arc,
                factor: 1.0,
                value: 1.0,
            },
            Equation::ArcClosure {
                c: p(0),
                s: p(1),
                e: p(2),
            },
        ];
        let h = 1e-6;
        for eq in &equations {
            let e = eq.eval(&x, 1e-9).unwrap();
            assert_eq!(e.count, eq.rows());
            for row in 0..e.count {
                let mut gradient = [0.0; 10];
                for slot in 0..e.used {
                    gradient[e.slots[slot]] += e.rows[row].d[slot];
                }
                for (j, got) in gradient.iter().enumerate() {
                    let mut hi = x;
                    hi[j] += h;
                    let mut lo = x;
                    lo[j] -= h;
                    let want = (eq.eval(&hi, 1e-9).unwrap().rows[row].v
                        - eq.eval(&lo, 1e-9).unwrap().rows[row].v)
                        / (2.0 * h);
                    // Central differences with h = 1e-6 on O(1) data: 1e-7.
                    assert!(
                        (got - want).abs() <= 1e-7,
                        "{eq:?} row {row} d/dx{j}: {got} vs {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn angles_wrap_to_half_turns() {
        assert_eq!(wrap_angle(0.0), 0.0);
        assert!((wrap_angle(TAU + 0.25) - 0.25).abs() < 1e-15);
        assert!((wrap_angle(-TAU - 0.25) + 0.25).abs() < 1e-15);
        assert!((wrap_angle(PI + 0.5) - (0.5 - PI)).abs() < 1e-15);
        assert_eq!(wrap_angle(PI), -PI);
    }

    #[test]
    fn short_lines_have_no_direction() {
        let x = [0.0, 0.0, 1e-7, 0.0, 1.0, 1.0];
        let short = Ln { a: Pt(0), b: Pt(2) };
        let eq = Equation::PointOnLine(Pt(4), short);
        assert_eq!(eq.eval(&x, 1e-6).err(), Some(Fail::Degenerate(SHORT_LINE)));
        assert!(eq.eval(&x, 1e-8).is_ok());
        let far = [0.0, 0.0, 1e200, 1e200, 1.0, 1.0];
        assert!(eq.eval(&far, 1e-6).is_ok());
        let overflow = [0.0, 0.0, 1e308, 1e308, -1e308, 1.0];
        assert_eq!(eq.eval(&overflow, 1e-6).err(), Some(Fail::NonFinite));
    }
}
