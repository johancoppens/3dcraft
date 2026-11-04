//! From a [`Sketch`] to scalar equations over a vector of unknowns.
//!
//! [`prepare`] validates the sketch, numbers the unknowns and turns each
//! constraint into an [`Equation`] with one or two rows. Decisions that pick
//! one of several mirror solutions (which side of a line, inside or outside
//! a circle) are taken here, once, from the starting guess, so the equations
//! are smooth during the iteration and the solve stays on its branch.

use craft3d_math::Portable;
use std::f64::consts::{PI, TAU};

use crate::dual::{Dual, SLOTS, angle_of, length};
use crate::error::{Site, SolveError};
use crate::model::{Constraint, Entity, EntityId, EntityKind, Sketch};
use crate::spline::{At, Foot, SplineVars};
use crate::{MAX_ENTITIES, MAX_EQUATIONS, MAX_UNKNOWNS};

const SHORT_LINE: &str = "a line shorter than the linear tolerance has no direction";
const NEGATIVE_RADIUS: &str = "a circle with a negative radius";
const SMALL_ARC: &str = "an arc smaller than the linear tolerance has no tangent";
const STALLED_SPLINE: &str =
    "a spline that stands still where the point is nearest to it has no direction there";
const STALLED_END: &str = "a spline that stands still at its end has no direction there";
const SEAM: &str = "the knots of a closed spline do not give it a derivative where it closes";
/// A line, circle or arc touching a spline anywhere is one equation in
/// the curve's unknowns and those of the `degree + 1` control points that
/// count where they touch: more than [`SLOTS`] from degree 7 on when the
/// control points move.
pub const TOUCH_TOO_HIGH: &str = "a line, circle or arc touching a spline whose control points move (a live spline, a sketch spline) anywhere takes one of degree 6 at most: at degree 7 one equation would hold 16 unknowns of the spline and 3 or 4 of the curve, more than the 18 it holds; give the spline degree 6 or lower, or let the two share an end point";
const NOWHERE_PARALLEL: &str =
    "the line runs parallel to the curve nowhere between its ends, so it cannot touch it";
/// Most image points an image point may be made from one after the other
/// (an image of an image of ...): a tree of sketch patterns is three
/// levels deep, and an end of a run that is itself an image adds one.
pub const MAX_IMAGE_DEPTH: usize = 8;
const TOO_DEEP: &str = "it is an image of an image of ... more than eight deep";

/// Why an equation is not built: through its image points it would depend
/// on more unknowns than one equation holds (`SLOTS`, 18: the
/// coordinates of the points the images are made from, the ends of runs
/// and the centres). Not supported yet rather than a wrong gradient.
pub const CROWDED: &str = "through its image points it depends on more than 18 unknowns (the coordinates of the seed points, run ends and centres they are made from), more than one constraint holds; split it into constraints on fewer image points";

/// Why an equation has no value at some point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fail {
    Degenerate(&'static str),
    NonFinite,
}

/// The line of a spline's end leg without length is the spline standing
/// still there.
fn stalled(fail: Fail) -> Fail {
    match fail {
        Fail::Degenerate(_) => Fail::Degenerate(STALLED_SPLINE),
        other => other,
    }
}

/// A point: one with unknowns of its own (the index of its x unknown; y
/// follows it), or an image point (its index in [`Prepared::images`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Pt {
    Own(usize),
    Image(usize),
    /// Fixed coordinates: a control point of a spline without unknowns.
    At([f64; 2]),
}

/// The centre of a turn: a point with unknowns, or fixed coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Centre {
    Own(usize),
    At([f64; 2]),
}

/// An image point: how it is made from unknowns, and while the sketch is
/// carried along by continuation (`crate::carry`), how it gets from where
/// it was to where its map puts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageVars {
    pub map: MapVars,
    pub carry: Option<Carry>,
    /// The stretch of the path parameter over which a carried image moves
    /// by its own numbers ([`Stage`]).
    pub stage: Stage,
}

/// The stretch `[lo, hi)` of the path parameter over which a carried image
/// moves by its own numbers: an image made from moving images (nested
/// patterns) is carried after them, one stage per level, so that along
/// each stage every carried image moves on its family or a straight line,
/// linearly in the parameter (`crate::carry`). Before `lo` its numbers are
/// the old ones, from `hi` on the new ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stage {
    pub lo: f64,
    pub hi: f64,
}

impl Stage {
    pub(crate) const WHOLE: Stage = Stage { lo: 0.0, hi: 1.0 };

    /// How far along its own stage the image is at `t`, from 0 to 1.
    fn local(&self, t: f64) -> f64 {
        if *self == Stage::WHOLE {
            t
        } else if t < self.lo {
            0.0
        } else if t >= self.hi {
            1.0
        } else {
            (t - self.lo) / (self.hi - self.lo)
        }
    }

    /// [`Stage::local`] in duals.
    fn local_dual(&self, t: Dual) -> Dual {
        if *self == Stage::WHOLE {
            t
        } else if t.v < self.lo {
            Dual::constant(0.0)
        } else if t.v >= self.hi {
            Dual::constant(1.0)
        } else {
            (t - Dual::constant(self.lo)) * (1.0 / (self.hi - self.lo))
        }
    }
}

/// How a carried image moves with the continuation parameter, the unknown
/// `x[t]` (0 where the image was, 1 where its map puts it now). When the
/// map it had then is of the same family as its map now (the same kind,
/// about the same points), the image moves through the family: the map's
/// own numbers (an offset, the instance number of a chained run, an angle
/// and a fixed centre) go from their old values to their new ones, so the
/// image of a ring swings round its centre and that of a run slides along
/// it. Otherwise it moves in a straight line from its old place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Carry {
    /// The map plus `(1 - x[t]) * by`.
    Straight { t: usize, by: [f64; 2] },
    /// A shift whose offset goes from `from` to the map's.
    Shift { t: usize, from: [f64; 2] },
    /// A chained run whose instance number goes from `from` to the map's.
    Chain { t: usize, from: f64 },
    /// A turn whose angle goes from `from` to `to` (the map's, which keeps
    /// only its sine and cosine) and whose fixed centre, if it has one,
    /// from `at` to the map's.
    Turn {
        t: usize,
        from: f64,
        to: f64,
        at: [f64; 2],
    },
}

impl Carry {
    pub(crate) fn t(&self) -> usize {
        match *self {
            Carry::Straight { t, .. }
            | Carry::Shift { t, .. }
            | Carry::Chain { t, .. }
            | Carry::Turn { t, .. } => t,
        }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

impl ImageVars {
    pub(crate) fn new(map: MapVars) -> Self {
        ImageVars {
            map,
            carry: None,
            stage: Stage::WHOLE,
        }
    }

    /// The coordinates at `x`; `images` are the image points it may be
    /// made from (those before it).
    pub(crate) fn at(&self, x: &[f64], images: &[ImageVars]) -> Option<[f64; 2]> {
        let Some(carry) = self.carry else {
            return self.map.at(x, images);
        };
        let t = self.stage.local(*x.get(carry.t())?);
        match (carry, self.map) {
            (Carry::Straight { by: [bx, by], .. }, map) => {
                let [px, py] = map.at(x, images)?;
                let rest = 1.0 - t;
                Some([px + rest * bx, py + rest * by])
            }
            (Carry::Shift { from, .. }, MapVars::Shift { of, offset }) => MapVars::Shift {
                of,
                offset: [lerp(from[0], offset[0], t), lerp(from[1], offset[1], t)],
            }
            .at(x, images),
            (Carry::Chain { from, .. }, MapVars::Chain { of, from: a, to, k }) => MapVars::Chain {
                of,
                from: a,
                to,
                k: lerp(from, k, t),
            }
            .at(x, images),
            (Carry::Turn { from, to, at, .. }, MapVars::Turn { of, centre, .. }) => {
                let (sin, cos) = lerp(from, to, t).psin_cos();
                let centre = match centre {
                    Centre::Own(c) => Centre::Own(c),
                    Centre::At(c) => Centre::At([lerp(at[0], c[0], t), lerp(at[1], c[1], t)]),
                };
                MapVars::Turn {
                    of,
                    centre,
                    sin,
                    cos,
                }
                .at(x, images)
            }
            // A family that does not match its map is never made
            // (`crate::carry`); no place rather than a wrong one.
            _ => None,
        }
    }
}

/// How an image point is made from unknowns ([`crate::ImageMap`] with the
/// points resolved to the indices of their x unknowns, or to image points
/// before it: an image of an image, for nested patterns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MapVars {
    Shift {
        of: Pt,
        offset: [f64; 2],
    },
    Chain {
        of: Pt,
        from: Pt,
        to: Pt,
        k: f64,
    },
    Turn {
        of: Pt,
        centre: Centre,
        sin: f64,
        cos: f64,
    },
}

/// The two numbers at `i` and `i + 1`.
fn pair(x: &[f64], i: usize) -> Option<[f64; 2]> {
    Some([*x.get(i)?, *x.get(i.checked_add(1)?)?])
}

impl MapVars {
    /// The coordinates at `x`, as [`crate::ImageMap::apply`] gives them.
    fn at(&self, x: &[f64], images: &[ImageVars]) -> Option<[f64; 2]> {
        Some(match *self {
            MapVars::Shift {
                of,
                offset: [dx, dy],
            } => {
                let [px, py] = coords(x, images, of)?;
                [px + dx, py + dy]
            }
            MapVars::Chain { of, from, to, k } => {
                let [px, py] = coords(x, images, of)?;
                let [ax, ay] = coords(x, images, from)?;
                let [bx, by] = coords(x, images, to)?;
                [px + (bx - ax) * k, py + (by - ay) * k]
            }
            MapVars::Turn {
                of,
                centre,
                sin,
                cos,
            } => {
                let [px, py] = coords(x, images, of)?;
                let [cx, cy] = match centre {
                    Centre::Own(c) => pair(x, c)?,
                    Centre::At(at) => at,
                };
                let (ux, uy) = (px - cx, py - cy);
                [cx + (ux * cos - uy * sin), cy + (ux * sin + uy * cos)]
            }
        })
    }
}

/// The coordinates of a point at `x`. An image point is made from the
/// image points before it only (`prepare`), so this ends.
pub(crate) fn coords(x: &[f64], images: &[ImageVars], p: Pt) -> Option<[f64; 2]> {
    match p {
        Pt::Own(i) => pair(x, i),
        Pt::Image(i) => images.get(i)?.at(x, images.get(..i)?),
        Pt::At(at) => Some(at),
    }
}

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
    /// Signed distance of the point from spline number `spline`, at its
    /// foot on the curve (`crate::spline`).
    PointOnSpline { p: Pt, spline: usize },
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
    /// `p - value` along one axis: a signed coordinate.
    Coordinate(Axis, Pt, f64),
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
    /// A built-in equation of a spline through a point: two rows,
    /// `C(u) - p`.
    SplineThrough { p: Pt, spline: usize, u: f64 },
    /// A built-in equation of a periodic spline ([`crate::model::Spline::periodic`]):
    /// one row, the `order`th derivative at its start less that at its
    /// end, along `axis`, scaled to a length (divided by its largest
    /// weight).
    SplineSeam {
        spline: usize,
        order: usize,
        axis: Axis,
    },
    /// A line tangent to a spline: at an end (`Some(u)`, that end's
    /// parameter), the sine of the angle between the line and the
    /// spline's direction there; else the signed distance from the line
    /// of the place of the spline where it runs parallel to it.
    TangentSplineLine {
        line: Ln,
        spline: usize,
        end: Option<f64>,
    },
    /// A round tangent to a spline: at an end, the cosine of the angle
    /// between the radius to the spline's end and the spline's direction
    /// there; else `side * distance(c, spline) - r`, the distance at the
    /// centre's foot.
    TangentSplineRound {
        round: Round,
        spline: usize,
        end: Option<f64>,
        side: f64,
    },
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
    splines: &'a [SplineVars],
    images: &'a [ImageVars],
    slots: [usize; SLOTS],
    used: usize,
}

type Vec2 = [Dual; 2];

impl<'a> Ctx<'a> {
    fn new(x: &'a [f64], min_len: f64, splines: &'a [SplineVars], images: &'a [ImageVars]) -> Self {
        Ctx {
            x,
            min_len,
            splines,
            images,
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
                *self.slots.get_mut(slot).ok_or(Fail::Degenerate(CROWDED))? = ix;
                self.used += 1;
                slot
            }
        };
        Dual::variable(value, slot).ok_or(Fail::NonFinite)
    }

    fn own(&mut self, i: usize) -> Result<Vec2, Fail> {
        Ok([
            self.var(i)?,
            self.var(i.checked_add(1).ok_or(Fail::NonFinite)?)?,
        ])
    }

    fn pt(&mut self, p: Pt) -> Result<Vec2, Fail> {
        let images = self.images;
        let image = match p {
            Pt::Own(i) => return self.own(i),
            Pt::Image(i) => *images.get(i).ok_or(Fail::NonFinite)?,
            Pt::At([x, y]) => return Ok([Dual::constant(x), Dual::constant(y)]),
        };
        let Some(carry) = image.carry else {
            return self.map(image.map);
        };
        let t = image.stage.local_dual(self.var(carry.t())?);
        match (carry, image.map) {
            (Carry::Straight { by: [bx, by], .. }, map) => {
                let [px, py] = self.map(map)?;
                let rest = Dual::constant(1.0) - t;
                Ok([px + rest * bx, py + rest * by])
            }
            (Carry::Shift { from, .. }, MapVars::Shift { of, offset }) => {
                let [px, py] = self.pt(of)?;
                // The offset first, as `ImageVars::at` adds it.
                let dx = t * (offset[0] - from[0]) + Dual::constant(from[0]);
                let dy = t * (offset[1] - from[1]) + Dual::constant(from[1]);
                Ok([px + dx, py + dy])
            }
            (Carry::Chain { from, .. }, MapVars::Chain { of, from: a, to, k }) => {
                let [px, py] = self.pt(of)?;
                let [ax, ay] = self.pt(a)?;
                let [bx, by] = self.pt(to)?;
                let k = t * (k - from) + Dual::constant(from);
                Ok([px + (bx - ax) * k, py + (by - ay) * k])
            }
            (Carry::Turn { from, to, at, .. }, MapVars::Turn { of, centre, .. }) => {
                let [px, py] = self.pt(of)?;
                let [cx, cy] = match centre {
                    Centre::Own(c) => self.own(c)?,
                    Centre::At([x, y]) => [
                        t * (x - at[0]) + Dual::constant(at[0]),
                        t * (y - at[1]) + Dual::constant(at[1]),
                    ],
                };
                let (sin, cos) = (t * (to - from) + Dual::constant(from)).sin_cos();
                let (ux, uy) = (px - cx, py - cy);
                Ok([cx + (ux * cos - uy * sin), cy + (ux * sin + uy * cos)])
            }
            _ => Err(Fail::NonFinite),
        }
    }

    fn map(&mut self, map: MapVars) -> Result<Vec2, Fail> {
        // The same expressions as `MapVars::at`, in duals.
        Ok(match map {
            MapVars::Shift {
                of,
                offset: [dx, dy],
            } => {
                let [px, py] = self.pt(of)?;
                [px + Dual::constant(dx), py + Dual::constant(dy)]
            }
            MapVars::Chain { of, from, to, k } => {
                let [px, py] = self.pt(of)?;
                let [ax, ay] = self.pt(from)?;
                let [bx, by] = self.pt(to)?;
                [px + (bx - ax) * k, py + (by - ay) * k]
            }
            MapVars::Turn {
                of,
                centre,
                sin,
                cos,
            } => {
                let [px, py] = self.pt(of)?;
                let [cx, cy] = match centre {
                    Centre::Own(c) => self.own(c)?,
                    Centre::At([x, y]) => [Dual::constant(x), Dual::constant(y)],
                };
                let (ux, uy) = (px - cx, py - cy);
                [cx + (ux * cos - uy * sin), cy + (ux * sin + uy * cos)]
            }
        })
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

    /// Signed distance of `p` from the spline, positive on its left: at
    /// the foot of `p`, with the foot and the normal held where they are
    /// (their changes do not change the distance to first order), or from
    /// the line of an end leg when the foot would be beyond that end.
    fn spline_distance(&mut self, p: Pt, spline: usize) -> Result<Dual, Fail> {
        let splines = self.splines;
        let s = splines.get(spline).ok_or(Fail::NonFinite)?;
        let images = self.images;
        let place = |x: &[f64], pt: Pt| -> Result<[f64; 2], Fail> {
            coords(x, images, pt).ok_or(Fail::NonFinite)
        };
        let target = place(self.x, p)?;
        let mut points = Vec::with_capacity(s.control.len());
        for control in &s.control {
            points.push(place(self.x, *control)?);
        }
        let foot = s.foot(&points, target).ok_or(Fail::NonFinite)?;
        let leg = |from: Option<&Pt>, to: Option<&Pt>| match (from, to) {
            (Some(a), Some(b)) => Ok(Ln { a: *a, b: *b }),
            _ => Err(Fail::NonFinite),
        };
        let n = s.control.len();
        let at = match foot {
            Foot::BeforeStart => {
                let line = leg(s.control.first(), s.control.get(1))?;
                let point = self.pt(p)?;
                return self.signed_distance(point, line).map_err(stalled);
            }
            Foot::AfterEnd => {
                let line = leg(
                    n.checked_sub(2).and_then(|i| s.control.get(i)),
                    s.control.last(),
                )?;
                let point = self.pt(p)?;
                return self.signed_distance(point, line).map_err(stalled);
            }
            Foot::On(at) | Foot::Corner(at) => at,
        };
        let [tx, ty] = at.tangent;
        let speed = tx.phypot(ty);
        if !speed.is_finite() {
            return Err(Fail::NonFinite);
        }
        // The tangent is per unit of the parameter, whose whole range the
        // curve is drawn over: a curve no longer than the tolerance there.
        if speed <= self.min_len {
            return Err(Fail::Degenerate(STALLED_SPLINE));
        }
        // Across the curve; at a corner that the point is nearest to,
        // from the corner to the point (unless it is on it already).
        let away = [target[0] - at.point[0], target[1] - at.point[1]];
        let far = away[0].phypot(away[1]);
        let normal = match foot {
            Foot::Corner(_) if far > self.min_len => [away[0] / far, away[1] / far],
            _ => [-ty / speed, tx / speed],
        };
        let (nx, ny) = (Dual::constant(normal[0]), Dual::constant(normal[1]));
        let [px, py] = self.pt(p)?;
        let (mut cx, mut cy) = (Dual::constant(0.0), Dual::constant(0.0));
        for (r, weight) in at.basis.iter().take(s.degree + 1).enumerate() {
            let control = s.control.get(at.first + r).ok_or(Fail::NonFinite)?;
            let [x, y] = self.pt(*control)?;
            let weight = Dual::constant(*weight);
            cx = cx + x * weight;
            cy = cy + y * weight;
        }
        Ok(nx * (px - cx) + ny * (py - cy))
    }

    /// The places of the control points of spline `spline` at `x`.
    fn spline_places(&self, s: &SplineVars) -> Result<Vec<[f64; 2]>, Fail> {
        let mut points = Vec::with_capacity(s.control.len());
        for control in &s.control {
            points.push(coords(self.x, self.images, *control).ok_or(Fail::NonFinite)?);
        }
        Ok(points)
    }

    /// The point and the tangent of the spline at the place `at`, as
    /// duals in its control points, with the parameter held. A control
    /// point that counts for neither is left out, so it takes no slot.
    fn spline_dual(&mut self, s: &SplineVars, at: &At) -> Result<(Vec2, Vec2), Fail> {
        let zero = Dual::constant(0.0);
        let (mut point, mut tangent) = ([zero, zero], [zero, zero]);
        for r in 0..=s.degree {
            let (weight, slope) = (
                *at.basis.get(r).ok_or(Fail::NonFinite)?,
                *at.slopes.get(r).ok_or(Fail::NonFinite)?,
            );
            if weight == 0.0 && slope == 0.0 {
                continue;
            }
            let control = s.control.get(at.first + r).ok_or(Fail::NonFinite)?;
            let [x, y] = self.pt(*control)?;
            let (w, d) = (Dual::constant(weight), Dual::constant(slope));
            point = [point[0] + x * w, point[1] + y * w];
            tangent = [tangent[0] + x * d, tangent[1] + y * d];
        }
        Ok((point, tangent))
    }

    /// `C(u) - p` for spline `spline`.
    fn spline_through(&mut self, p: Pt, spline: usize, u: f64) -> Result<[Dual; 2], Fail> {
        let splines = self.splines;
        let s = splines.get(spline).ok_or(Fail::NonFinite)?;
        let points = self.spline_places(s)?;
        let at = s.at(&points, u).ok_or(Fail::NonFinite)?;
        let ([cx, cy], _) = self.spline_dual(s, &at)?;
        let [px, py] = self.pt(p)?;
        Ok([cx - px, cy - py])
    }

    /// The row of [`Equation::SplineSeam`]: one coordinate of the
    /// `order`th derivative at the start less that at the end. Only that
    /// coordinate of each control point takes a slot.
    fn spline_seam(&mut self, spline: usize, order: usize, axis: Axis) -> Result<Dual, Fail> {
        let splines = self.splines;
        let s = splines.get(spline).ok_or(Fail::NonFinite)?;
        let n = s.control.len();
        let (start, end) = crate::spline::seam_weights(&s.knots, s.degree, n, order)
            .ok_or(Fail::Degenerate(SEAM))?;
        let scale = start
            .iter()
            .chain(&end)
            .fold(0.0_f64, |m, w| m.max(w.abs()));
        if !(scale.is_finite() && scale > 0.0) {
            return Err(Fail::Degenerate(SEAM));
        }
        let from_end = n.checked_sub(order + 1).ok_or(Fail::Degenerate(SEAM))?;
        let terms = start
            .iter()
            .enumerate()
            .map(|(j, w)| (j, *w))
            .chain(end.iter().enumerate().map(|(j, w)| (from_end + j, -*w)));
        let mut row = Dual::constant(0.0);
        for (index, weight) in terms {
            if weight == 0.0 {
                continue;
            }
            let control = *s.control.get(index).ok_or(Fail::NonFinite)?;
            let value = match control {
                Pt::Own(i) => {
                    let offset = match axis {
                        Axis::X => 0,
                        Axis::Y => 1,
                    };
                    self.var(i.checked_add(offset).ok_or(Fail::NonFinite)?)?
                }
                other => {
                    let [x, y] = self.pt(other)?;
                    match axis {
                        Axis::X => x,
                        Axis::Y => y,
                    }
                }
            };
            row = row + value * Dual::constant(weight / scale);
        }
        Ok(row)
    }

    /// [`TOUCH_TOO_HIGH`] when a curve with `own` unknowns touching
    /// spline `s` anywhere would need more slots than one equation has.
    fn touch_fits(s: &SplineVars, own: usize) -> Result<(), Fail> {
        let moving = s.control.iter().any(|c| !matches!(c, Pt::At(_)));
        let needed = 2usize
            .saturating_mul(s.degree.saturating_add(1))
            .saturating_add(own);
        if moving && needed > SLOTS {
            return Err(Fail::Degenerate(TOUCH_TOO_HIGH));
        }
        Ok(())
    }

    /// The row of [`Equation::TangentSplineLine`].
    fn tangent_spline_line(
        &mut self,
        line: Ln,
        spline: usize,
        end: Option<f64>,
    ) -> Result<Dual, Fail> {
        let splines = self.splines;
        let s = splines.get(spline).ok_or(Fail::NonFinite)?;
        let points = self.spline_places(s)?;
        if end.is_none() {
            Self::touch_fits(s, 4)?;
        }
        let ([ax, ay], [dx, dy], len) = self.line(line)?;
        let at = match end {
            Some(u) => s.at(&points, u).ok_or(Fail::NonFinite)?,
            None => s
                .parallel(&points, [dx.v, dy.v], [ax.v, ay.v])
                .ok_or(Fail::NonFinite)?
                .ok_or(Fail::Degenerate(NOWHERE_PARALLEL))?,
        };
        let ([cx, cy], [tx, ty]) = self.spline_dual(s, &at)?;
        match end {
            Some(_) => {
                let speed = length(tx, ty);
                if !speed.v.is_finite() {
                    return Err(Fail::NonFinite);
                }
                if speed.v <= self.min_len {
                    return Err(Fail::Degenerate(STALLED_END));
                }
                Ok((dx * ty - dy * tx) / (len * speed))
            }
            // At the place where the curve runs along the line, moving
            // along the curve does not change the distance to first
            // order: the parameter is held.
            None => Ok((dx * (cy - ay) - dy * (cx - ax)) / len),
        }
    }

    /// The row of [`Equation::TangentSplineRound`].
    fn tangent_spline_round(
        &mut self,
        round: Round,
        spline: usize,
        end: Option<f64>,
        side: f64,
    ) -> Result<Dual, Fail> {
        let Some(u) = end else {
            let own = match round {
                Round::Circle { .. } => 3,
                Round::Arc { .. } => 4,
            };
            Self::touch_fits(self.splines.get(spline).ok_or(Fail::NonFinite)?, own)?;
            let gap = self.spline_distance(round.center(), spline)?;
            return Ok(gap * side - self.radius(round)?);
        };
        let splines = self.splines;
        let s = splines.get(spline).ok_or(Fail::NonFinite)?;
        let points = self.spline_places(s)?;
        let at = s.at(&points, u).ok_or(Fail::NonFinite)?;
        let ([px, py], [tx, ty]) = self.spline_dual(s, &at)?;
        let speed = length(tx, ty);
        if !speed.v.is_finite() {
            return Err(Fail::NonFinite);
        }
        if speed.v <= self.min_len {
            return Err(Fail::Degenerate(STALLED_END));
        }
        let [cx, cy] = self.pt(round.center())?;
        let (ux, uy) = (px - cx, py - cy);
        let radius = length(ux, uy);
        if radius.v <= self.min_len {
            return Err(Fail::Degenerate(SMALL_ARC));
        }
        Ok((ux * tx + uy * ty) / (radius * speed))
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
            Equation::PointOnSpline { p, spline } => one(self.spline_distance(p, spline)?),
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
            Equation::Coordinate(axis, p, value) => one(self.coordinate(axis, p)? - value),
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
            Equation::SplineThrough { p, spline, u } => (2, self.spline_through(p, spline, u)?),
            Equation::SplineSeam {
                spline,
                order,
                axis,
            } => one(self.spline_seam(spline, order, axis)?),
            Equation::TangentSplineLine { line, spline, end } => {
                one(self.tangent_spline_line(line, spline, end)?)
            }
            Equation::TangentSplineRound {
                round,
                spline,
                end,
                side,
            } => one(self.tangent_spline_round(round, spline, end, side)?),
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
            Equation::Parallel(..)
                | Equation::Perpendicular(..)
                | Equation::Angle(..)
                | Equation::TangentSplineLine { end: Some(_), .. }
                | Equation::TangentSplineRound { end: Some(_), .. }
        )
    }

    pub(crate) fn rows(&self) -> usize {
        match self {
            Equation::Same(..)
            | Equation::Symmetric { .. }
            | Equation::Midpoint(..)
            | Equation::Fixed(..)
            | Equation::SplineThrough { .. } => 2,
            _ => 1,
        }
    }

    /// Residuals and gradients at `x`. Lines whose direction is needed must
    /// be longer than `min_len`.
    #[cfg(test)]
    pub(crate) fn eval(&self, x: &[f64], min_len: f64) -> Result<Evaluated, Fail> {
        self.eval_with(x, min_len, &[], &[])
    }

    /// As [`Equation::eval`], for a sketch with `splines` in it.
    pub(crate) fn eval_with(
        &self,
        x: &[f64],
        min_len: f64,
        splines: &[SplineVars],
        images: &[ImageVars],
    ) -> Result<Evaluated, Fail> {
        let mut ctx = Ctx::new(x, min_len, splines, images);
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
    /// The splines of the sketch, by their index in it.
    pub splines: Vec<SplineVars>,
    /// The image points of the sketch, by [`Pt::Image`].
    pub images: Vec<ImageVars>,
    /// Arc equations first, then the splines' through equations, then the
    /// constraints in input order.
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
    splines: Vec<SplineVars>,
    images: Vec<ImageVars>,
    /// Per entity: its index in `images`, for an image point.
    image_of: Vec<Option<usize>>,
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
    let is_point = |id: EntityId| {
        matches!(
            sketch.entities.get(id.index()),
            Some(Entity::Point { .. } | Entity::Image { .. })
        )
    };
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
            Entity::Point { .. } => Ok(Pt::Own(self.var(at, id)?)),
            Entity::Image { .. } => self
                .image_of
                .get(id.index())
                .copied()
                .flatten()
                .map(Pt::Image)
                .ok_or(SolveError::UnknownEntity { at, id }),
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
        match eq.eval_with(&self.x0, self.min_len, &self.splines, &self.images) {
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
        let length_value = |v: f64, reason: &'static str| {
            if finite(v)? < 0.0 {
                Err(SolveError::InvalidValue {
                    constraint: index,
                    reason,
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
            Constraint::PointOnSpline { point, spline } => {
                if spline >= self.splines.len() {
                    return Err(SolveError::Unsupported {
                        constraint: index,
                        what: "it refers to a spline the sketch does not have",
                    });
                }
                Equation::PointOnSpline {
                    p: self.pt(at, point)?,
                    spline,
                }
            }
            Constraint::TangentToSpline {
                curve,
                spline,
                at: end,
            } => {
                let Some(s) = self.splines.get(spline) else {
                    return Err(SolveError::Unsupported {
                        constraint: index,
                        what: "it refers to a spline the sketch does not have",
                    });
                };
                let end = match end {
                    Some(which) => Some(s.end(which).ok_or(SolveError::NonFinite { at })?),
                    None => None,
                };
                match self.entity(at, curve)? {
                    Entity::Line { .. } => Equation::TangentSplineLine {
                        line: self.ln(at, curve)?,
                        spline,
                        end,
                    },
                    Entity::Circle { .. } | Entity::Arc { .. } => {
                        let round = self.round(at, curve)?;
                        let side = match end {
                            Some(_) => 1.0,
                            // The side of the spline the centre is on.
                            None => side_of(Equation::PointOnSpline {
                                p: round.center(),
                                spline,
                            })?,
                        };
                        Equation::TangentSplineRound {
                            round,
                            spline,
                            end,
                            side,
                        }
                    }
                    _ => return Err(self.wrong(at, curve, "line, circle or arc")),
                }
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
                    None => coords(&self.x0, &self.images, p).unwrap_or([0.0, 0.0]),
                };
                Equation::Fixed(p, target)
            }
            Constraint::CoordinateX { point, value } => {
                Equation::Coordinate(Axis::X, self.pt(at, point)?, finite(value)?)
            }
            Constraint::CoordinateY { point, value } => {
                Equation::Coordinate(Axis::Y, self.pt(at, point)?, finite(value)?)
            }
            Constraint::Distance { a, b, value } => {
                let value = length_value(value, NEGATIVE_DISTANCE)?;
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
                let value = length_value(value, NEGATIVE_DISTANCE)?;
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
                value: length_value(value, NEGATIVE_RADIUS_VALUE)?,
            },
            Constraint::Diameter { circle, value } => Equation::Radius {
                round: self.round(at, circle)?,
                factor: 2.0,
                value: length_value(value, NEGATIVE_RADIUS_VALUE)?,
            },
        })
    }
}

/// Why a negative distance is refused, and what to give instead: a
/// distance is a size, and the side it lies on is the side the sketch is
/// drawn on (found from the starting guess).
const NEGATIVE_DISTANCE: &str = "a distance cannot be negative: give its size, and draw the point on the side it is to lie on (the side is kept from the sketch as drawn)";
/// Why a negative radius or diameter is refused.
const NEGATIVE_RADIUS_VALUE: &str = "a radius or diameter cannot be negative";

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
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Image { .. } => {
                entity_var.push(None);
            }
        }
    }
    too_large("unknowns", x0.len(), MAX_UNKNOWNS)?;

    // Image points: the indices of the unknowns of the points they are
    // made from, which have to be points with coordinates or image points
    // before them (an image of an image), at most `MAX_IMAGE_DEPTH` deep.
    let mut images = Vec::new();
    let mut image_of: Vec<Option<usize>> = Vec::with_capacity(sketch.entities.len());
    let mut depth_of: Vec<usize> = Vec::new();
    for (index, entity) in sketch.entities.iter().enumerate() {
        let Entity::Image { of, map } = *entity else {
            image_of.push(None);
            continue;
        };
        let at = Site::Entity(index);
        let own = |id: EntityId| -> Result<usize, SolveError> {
            match sketch.entities.get(id.index()) {
                Some(Entity::Point { .. }) => entity_var
                    .get(id.index())
                    .copied()
                    .flatten()
                    .ok_or(SolveError::UnknownEntity { at, id }),
                Some(other) => Err(SolveError::WrongKind {
                    at,
                    id,
                    expected: "point with coordinates",
                    found: other.kind(),
                }),
                None => Err(SolveError::UnknownEntity { at, id }),
            }
        };
        let mut depth = 1usize;
        let mut point = |id: EntityId| -> Result<Pt, SolveError> {
            match sketch.entities.get(id.index()) {
                Some(Entity::Image { .. }) => {
                    let earlier = image_of.get(id.index()).copied().flatten();
                    let Some(earlier) = earlier.filter(|_| id.index() < index) else {
                        return Err(SolveError::WrongKind {
                            at,
                            id,
                            expected: "point with coordinates, or an image point before it",
                            found: EntityKind::Image,
                        });
                    };
                    let below = depth_of.get(earlier).copied().unwrap_or(MAX_IMAGE_DEPTH);
                    depth = depth.max(below.saturating_add(1));
                    Ok(Pt::Image(earlier))
                }
                _ => own(id).map(Pt::Own),
            }
        };
        let finite = |values: &[f64]| {
            if values.iter().all(|v| v.is_finite()) {
                Ok(())
            } else {
                Err(SolveError::NonFinite { at })
            }
        };
        let of = point(of)?;
        let vars = match map {
            crate::ImageMap::Shift { offset } => {
                finite(&offset)?;
                MapVars::Shift { of, offset }
            }
            crate::ImageMap::Chain { from, to, k } => {
                finite(&[k])?;
                MapVars::Chain {
                    of,
                    from: point(from)?,
                    to: point(to)?,
                    k,
                }
            }
            crate::ImageMap::Turn { center, at, angle } => {
                finite(&[at[0], at[1], angle])?;
                let (sin, cos) = angle.psin_cos();
                let centre = match center {
                    Some(id) => Centre::Own(own(id)?),
                    None => Centre::At(at),
                };
                MapVars::Turn {
                    of,
                    centre,
                    sin,
                    cos,
                }
            }
        };
        if depth > MAX_IMAGE_DEPTH {
            return Err(SolveError::InvalidEntity {
                entity: index,
                reason: TOO_DEEP,
            });
        }
        image_of.push(Some(images.len()));
        depth_of.push(depth);
        images.push(ImageVars::new(vars));
    }

    let (class, on) = incidences(sketch);
    let mut builder = Builder {
        sketch,
        splines: Vec::new(),
        images,
        image_of,
        entity_var,
        x0,
        min_len,
        class,
        on,
    };

    // Every spline is one, on points, used by a constraint or not. (A
    // fault of a spline is reported at its first control point.)
    too_large("splines", sketch.splines.len(), MAX_ENTITIES)?;
    let mut splines = Vec::with_capacity(sketch.splines.len());
    // The built-in equations of splines through points, after the arcs'.
    let mut throughs = Vec::new();
    for (number, spline) in sketch.splines.iter().enumerate() {
        let first = spline
            .control
            .first()
            .or(spline.through.first().map(|(id, _)| id))
            .map_or(0, |id| id.index());
        let invalid = |reason: &'static str| SolveError::InvalidEntity {
            entity: first,
            reason,
        };
        if !spline.control.is_empty() && !spline.fixed.is_empty() {
            return Err(invalid(
                "a spline has points of the sketch or fixed control points, not both",
            ));
        }
        let count = spline.control.len().max(spline.fixed.len());
        if let Some(reason) = crate::spline::fault(spline.degree, &spline.knots, count) {
            return Err(invalid(reason));
        }
        let mut control = Vec::with_capacity(count);
        for id in &spline.control {
            control.push(builder.pt(Site::Entity(first), *id)?);
        }
        for at in &spline.fixed {
            if !at.iter().all(|v| v.is_finite()) {
                return Err(SolveError::NonFinite {
                    at: Site::Entity(first),
                });
            }
            control.push(Pt::At(*at));
        }
        let vars = SplineVars {
            degree: spline.degree,
            knots: spline.knots.clone(),
            control,
        };
        too_large(
            "points a spline passes through",
            spline.through.len(),
            MAX_EQUATIONS,
        )?;
        for (id, u) in &spline.through {
            let at = Site::Entity(id.index());
            if !vars.covers(*u) {
                return Err(SolveError::InvalidEntity {
                    entity: id.index(),
                    reason: "a spline passes through a point at a parameter outside its knots",
                });
            }
            throughs.push((
                Equation::SplineThrough {
                    p: builder.pt(at, *id)?,
                    spline: number,
                    u: *u,
                },
                at,
            ));
        }
        if spline.periodic {
            // Smooth where it closes: each derivative below the degree
            // the same at both ends, one row per coordinate.
            if spline.control.first() != spline.control.last() {
                return Err(invalid(
                    "a periodic spline starts and ends on the same control point",
                ));
            }
            for order in 1..spline.degree {
                for axis in [Axis::X, Axis::Y] {
                    throughs.push((
                        Equation::SplineSeam {
                            spline: number,
                            order,
                            axis,
                        },
                        Site::Entity(first),
                    ));
                }
            }
        }
        splines.push(vars);
    }
    builder.splines = splines;

    // Every entity must refer to points, used by a constraint or not.
    let mut equations = Vec::new();
    let mut source = Vec::new();
    for (index, entity) in sketch.entities.iter().enumerate() {
        let at = Site::Entity(index);
        match *entity {
            Entity::Point { .. } | Entity::Image { .. } => {}
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

    for (eq, at) in throughs {
        equations.push(eq);
        source.push(at);
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
            Entity::Image { .. } => {}
        }
    }
    // An image may lie far from the points it is made from (the end of a
    // long row).
    // The control points of splines without unknowns are places too.
    for spline in &sketch.splines {
        for &[x, y] in &spline.fixed {
            magnitude = magnitude.max(x.abs()).max(y.abs());
            place([x, y]);
        }
    }
    for (i, image) in builder.images.iter().enumerate() {
        if let Some([x, y]) = builder
            .images
            .get(..i)
            .and_then(|before| image.at(&builder.x0, before))
        {
            magnitude = magnitude.max(x.abs()).max(y.abs());
            place([x, y]);
        }
    }
    for eq in &equations {
        match *eq {
            Equation::Fixed(_, [x, y]) => {
                magnitude = magnitude.max(x.abs()).max(y.abs());
                place([x, y]);
            }
            Equation::Coordinate(_, _, value) => {
                magnitude = magnitude.max(value.abs());
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
        splines: builder.splines,
        images: builder.images,
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

    /// Image points of every kind (a shift, a chained run, a turn about a
    /// point and about fixed coordinates) in equations: the gradient with
    /// respect to the unknowns of the points they are made from equals
    /// central differences, and their place is what `ImageMap::apply`
    /// gives.
    #[test]
    fn image_points_have_exact_gradients() {
        // Points 0..4 at x[0..8]: of, from, to, centre.
        let x = [0.7, -0.4, 0.1, 0.2, 1.3, 0.9, -0.6, 0.35];
        let images = [
            MapVars::Shift {
                of: Pt::Own(0),
                offset: [3.0, -1.5],
            },
            MapVars::Chain {
                of: Pt::Own(0),
                from: Pt::Own(2),
                to: Pt::Own(4),
                k: 3.0,
            },
            MapVars::Chain {
                of: Pt::Own(4),
                from: Pt::Own(2),
                to: Pt::Own(4),
                k: 2.0,
            },
            MapVars::Turn {
                of: Pt::Own(0),
                centre: Centre::Own(6),
                sin: 0.8_f64.psin(),
                cos: 0.8_f64.pcos(),
            },
            MapVars::Turn {
                of: Pt::Own(4),
                centre: Centre::At([0.25, -0.5]),
                sin: (-2.1_f64).psin(),
                cos: (-2.1_f64).pcos(),
            },
        ]
        .map(ImageVars::new);
        // The place: the same numbers as the map the caller applies.
        let point = |i: u32| {
            let i = i as usize * 2;
            Some([x[i], x[i + 1]])
        };
        let maps = [
            crate::ImageMap::Shift {
                offset: [3.0, -1.5],
            },
            crate::ImageMap::Chain {
                from: EntityId(1),
                to: EntityId(2),
                k: 3.0,
            },
            crate::ImageMap::Chain {
                from: EntityId(1),
                to: EntityId(2),
                k: 2.0,
            },
            crate::ImageMap::Turn {
                center: Some(EntityId(3)),
                at: [0.0, 0.0],
                angle: 0.8,
            },
            crate::ImageMap::Turn {
                center: None,
                at: [0.25, -0.5],
                angle: -2.1,
            },
        ];
        let of = [0, 0, 2, 0, 2];
        for ((image, map), of) in images.iter().zip(&maps).zip(of) {
            let at = image.at(&x, &[]).unwrap();
            let want = map.apply(point(of).unwrap(), |id| point(id.0)).unwrap();
            assert_eq!(at, want, "{map:?}");
        }
        let img = |i: usize| Pt::Image(i);
        let own = |i: usize| Pt::Own(2 * i);
        let line = Ln {
            a: own(1),
            b: img(4),
        };
        let equations = [
            Equation::Same(img(0), own(3)),
            Equation::Same(img(1), img(3)),
            Equation::Distance(img(2), own(0), 2.0),
            Equation::PointOnLine(
                img(3),
                Ln {
                    a: own(1),
                    b: img(1),
                },
            ),
            Equation::Midpoint(
                img(4),
                Ln {
                    a: own(0),
                    b: img(0),
                },
            ),
            Equation::Fixed(img(1), [1.0, 2.0]),
            Equation::Parallel(
                line,
                Ln {
                    a: img(2),
                    b: img(3),
                },
            ),
        ];
        let h = 1e-6;
        let eval = |eq: &Equation, x: &[f64]| eq.eval_with(x, 1e-9, &[], &images).unwrap();
        for eq in &equations {
            let e = eval(eq, &x);
            for row in 0..e.count {
                let mut gradient = [0.0; 8];
                for slot in 0..e.used {
                    gradient[e.slots[slot]] += e.rows[row].d[slot];
                }
                for (j, got) in gradient.iter().enumerate() {
                    let (mut hi, mut lo) = (x, x);
                    hi[j] += h;
                    lo[j] -= h;
                    let want = (eval(eq, &hi).rows[row].v - eval(eq, &lo).rows[row].v) / (2.0 * h);
                    assert!(
                        (got - want).abs() <= 1e-7,
                        "{eq:?} row {row} d/dx{j}: {got} vs {want}"
                    );
                }
            }
        }
    }

    /// Images of images (nested patterns), plain and carried in stages:
    /// their place in numbers is the map applied to the place of the image
    /// they are made from, the same in duals, and their gradient equals
    /// central differences.
    #[test]
    fn images_of_images_have_exact_gradients() {
        // Points 0..4 at x[0..8], the path parameter at x[8].
        let x = [0.7, -0.4, 0.1, 0.2, 1.3, 0.9, -0.6, 0.35, 0.62];
        let t = 8;
        let own = Pt::Own;
        let first = |stage| ImageVars {
            map: MapVars::Chain {
                of: own(4),
                from: own(2),
                to: own(4),
                k: 3.0,
            },
            carry: Some(Carry::Chain { t, from: 1.0 }),
            stage,
        };
        let half = Stage { lo: 0.0, hi: 0.5 };
        let second = Stage { lo: 0.5, hi: 1.0 };
        let images = [
            // 0: the end of an inner run, carried in the first stage.
            first(half),
            // 1: the end of the outer run, from the inner end: an image of
            // an image, its chain ending on it too, carried second.
            ImageVars {
                map: MapVars::Chain {
                    of: Pt::Image(0),
                    from: own(2),
                    to: Pt::Image(0),
                    k: 4.0,
                },
                carry: Some(Carry::Chain { t, from: 2.0 }),
                stage: second,
            },
            // 2: a turn of that, not carried.
            ImageVars::new(MapVars::Turn {
                of: Pt::Image(1),
                centre: Centre::Own(6),
                sin: 0.3_f64.psin(),
                cos: 0.3_f64.pcos(),
            }),
            // 3: a shift of the inner end, carried straight.
            ImageVars {
                map: MapVars::Shift {
                    of: Pt::Image(0),
                    offset: [0.5, -0.25],
                },
                carry: Some(Carry::Straight { t, by: [0.1, 0.2] }),
                stage: second,
            },
        ];
        // In numbers: composed level by level.
        let tl = 1.0; // x[8] = 0.62 is past the first stage.
        let k0 = 1.0 + tl * (3.0 - 1.0);
        let inner = [x[4] + (x[4] - x[2]) * k0, x[5] + (x[5] - x[3]) * k0];
        assert_eq!(images[0].at(&x, &[]).unwrap(), inner);
        let ts = (0.62 - 0.5) / 0.5;
        let k1 = ts * (4.0 - 2.0) + 2.0;
        let outer = images[1].at(&x, &images[..1]).unwrap();
        let want = [
            inner[0] + (inner[0] - x[2]) * k1,
            inner[1] + (inner[1] - x[3]) * k1,
        ];
        assert!((outer[0] - want[0]).abs() <= 1e-14 && (outer[1] - want[1]).abs() <= 1e-14);
        let equations = [
            Equation::Same(Pt::Image(1), own(0)),
            Equation::Distance(Pt::Image(2), own(0), 2.0),
            Equation::Fixed(Pt::Image(3), [1.0, 2.0]),
            Equation::PointOnLine(
                Pt::Image(2),
                Ln {
                    a: Pt::Image(1),
                    b: Pt::Image(3),
                },
            ),
        ];
        let h = 1e-6;
        let eval = |eq: &Equation, x: &[f64]| eq.eval_with(x, 1e-9, &[], &images).unwrap();
        for eq in &equations {
            let e = eval(eq, &x);
            // The same place in duals as in numbers.
            for row in 0..e.count {
                let mut gradient = [0.0; 9];
                for slot in 0..e.used {
                    gradient[e.slots[slot]] += e.rows[row].d[slot];
                }
                for (j, got) in gradient.iter().enumerate() {
                    let (mut hi, mut lo) = (x, x);
                    hi[j] += h;
                    lo[j] -= h;
                    let want = (eval(eq, &hi).rows[row].v - eval(eq, &lo).rows[row].v) / (2.0 * h);
                    assert!(
                        (got - want).abs() <= 1e-6 * (1.0 + want.abs()),
                        "{eq:?} row {row} d/dx{j}: {got} vs {want}"
                    );
                }
            }
        }
        let place = coords(&x, &images, Pt::Image(1)).unwrap();
        assert_eq!(place, outer);
    }

    /// Carried images (`Carry`): their place is the same in numbers and in
    /// duals, and their gradient, the path parameter's column included,
    /// equals central differences.
    #[test]
    fn carried_images_have_exact_gradients() {
        // Points 0..4 at x[0..8], the path parameter at x[8].
        let x = [0.7, -0.4, 0.1, 0.2, 1.3, 0.9, -0.6, 0.35, 0.37];
        let t = 8;
        let images = [
            ImageVars {
                map: MapVars::Shift {
                    of: Pt::Own(0),
                    offset: [3.0, -1.5],
                },
                stage: Stage::WHOLE,
                carry: Some(Carry::Shift {
                    t,
                    from: [-1.0, 0.5],
                }),
            },
            ImageVars {
                map: MapVars::Chain {
                    of: Pt::Own(0),
                    from: Pt::Own(2),
                    to: Pt::Own(4),
                    k: 9.0,
                },
                stage: Stage::WHOLE,
                carry: Some(Carry::Chain { t, from: 2.0 }),
            },
            ImageVars {
                map: MapVars::Turn {
                    of: Pt::Own(0),
                    centre: Centre::Own(6),
                    sin: 4.7_f64.psin(),
                    cos: 4.7_f64.pcos(),
                },
                stage: Stage::WHOLE,
                carry: Some(Carry::Turn {
                    t,
                    from: 1.0,
                    to: 4.7,
                    at: [0.0, 0.0],
                }),
            },
            ImageVars {
                map: MapVars::Turn {
                    of: Pt::Own(4),
                    centre: Centre::At([0.25, -0.5]),
                    sin: (-2.1_f64).psin(),
                    cos: (-2.1_f64).pcos(),
                },
                stage: Stage::WHOLE,
                carry: Some(Carry::Turn {
                    t,
                    from: 0.4,
                    to: -2.1,
                    at: [1.0, 1.5],
                }),
            },
            ImageVars {
                map: MapVars::Chain {
                    of: Pt::Own(4),
                    from: Pt::Own(2),
                    to: Pt::Own(4),
                    k: 2.0,
                },
                stage: Stage::WHOLE,
                carry: Some(Carry::Straight { t, by: [0.8, -2.5] }),
            },
        ];
        // At t = 1 every carried image is where its map puts it.
        let mut end = x;
        end[t] = 1.0;
        for image in &images {
            let plain = ImageVars::new(image.map);
            let (a, b) = (image.at(&end, &[]).unwrap(), plain.at(&end, &[]).unwrap());
            assert!(
                (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
                "{image:?}"
            );
        }
        let img = Pt::Image;
        let own = |i: usize| Pt::Own(2 * i);
        let mut equations = Vec::new();
        for i in 0..images.len() {
            equations.push(Equation::Fixed(img(i), [0.3, 0.2]));
            equations.push(Equation::Distance(img(i), own(1), 0.5));
        }
        let h = 1e-6;
        let eval = |eq: &Equation, x: &[f64]| eq.eval_with(x, 1e-9, &[], &images).unwrap();
        for eq in &equations {
            let e = eval(eq, &x);
            if let Equation::Fixed(Pt::Image(i), [fx, fy]) = eq {
                let at = images[*i].at(&x, &[]).unwrap();
                assert_eq!(e.rows[0].v, at[0] - fx, "{eq:?}");
                assert_eq!(e.rows[1].v, at[1] - fy, "{eq:?}");
            }
            for row in 0..e.count {
                let mut gradient = [0.0; 9];
                for slot in 0..e.used {
                    gradient[e.slots[slot]] += e.rows[row].d[slot];
                }
                for (j, got) in gradient.iter().enumerate() {
                    let (mut hi, mut lo) = (x, x);
                    hi[j] += h;
                    lo[j] -= h;
                    let want = (eval(eq, &hi).rows[row].v - eval(eq, &lo).rows[row].v) / (2.0 * h);
                    assert!(
                        (got - want).abs() <= 1e-7,
                        "{eq:?} row {row} d/dx{j}: {got} vs {want}"
                    );
                }
            }
        }
    }

    /// Every equation kind over four points and two radii, at a generic
    /// position: the automatic gradient equals central differences.
    #[test]
    fn gradients_match_finite_differences() {
        // Points P0..P3 at x[0..8], radii at x[8], x[9].
        let x = [0.3, -0.2, 2.1, 0.9, 1.7, 2.6, -0.8, 1.9, 0.75, 1.4];
        let p = |i: usize| Pt::Own(2 * i);
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

    /// The distance from a spline is taken at the foot of the point, with
    /// the foot and the normal held still for the gradient. That this is
    /// the gradient of the distance itself (which finds its foot anew at
    /// every place) is what central differences say here.
    #[test]
    fn the_gradient_of_the_distance_from_a_spline_matches_finite_differences() {
        let control = [
            [0.0, 0.0],
            [1.0, 2.0],
            [2.5, -1.0],
            [4.0, 3.0],
            [5.0, 0.5],
            [6.5, 1.0],
            [8.0, 0.0],
            [9.0, 2.0],
            [11.0, 1.0],
        ];
        for degree in 1..=crate::spline::MAX_SPLINE_DEGREE {
            let count = control.len();
            let spans = count - degree;
            let mut knots = vec![0.0; degree + 1];
            knots.extend((1..spans).map(|i| i as f64 / spans as f64));
            knots.extend(vec![1.0; degree + 1]);
            let splines = [SplineVars {
                degree,
                knots,
                control: (0..count).map(|i| Pt::Own(2 * i)).collect(),
            }];
            // On either side of the curve, near and far, and beyond both
            // ends (where the distance is from the line of the end leg).
            // (A polyline's distance has a kink where the foot is a
            // corner: for degree 1, one place whose foot is inside a leg.)
            let targets: &[[f64; 2]] = if degree == 1 {
                &[[2.45, 1.3], [-1.0, -3.0], [12.5, 0.2]]
            } else {
                &[
                    [2.0, 3.0],
                    [4.2, -2.0],
                    [6.1, 2.5],
                    [3.1, 0.7],
                    [-1.0, -3.0],
                    [12.5, 0.2],
                ]
            };
            for &target in targets {
                let mut x: Vec<f64> = control.iter().flatten().copied().collect();
                x.extend(target);
                let eq = Equation::PointOnSpline {
                    p: Pt::Own(2 * count),
                    spline: 0,
                };
                let e = eq.eval_with(&x, 1e-9, &splines, &[]).unwrap();
                assert_eq!(e.count, 1);
                assert!(e.used <= SLOTS);
                let mut gradient = vec![0.0; x.len()];
                for slot in 0..e.used {
                    gradient[e.slots[slot]] += e.rows[0].d[slot];
                }
                let h = 1e-6;
                for i in 0..x.len() {
                    let (mut lo, mut hi) = (x.clone(), x.clone());
                    lo[i] -= h;
                    hi[i] += h;
                    let want = (eq.eval_with(&hi, 1e-9, &splines, &[]).unwrap().rows[0].v
                        - eq.eval_with(&lo, 1e-9, &splines, &[]).unwrap().rows[0].v)
                        / (2.0 * h);
                    assert!(
                        (gradient[i] - want).abs() < 1e-6,
                        "degree {degree} {target:?} unknown {i}: {} against {want}",
                        gradient[i]
                    );
                }
            }
        }
    }

    /// The seam of a periodic spline: the weights of its derivatives at
    /// the ends in closed form (a cubic on uniform clamped knots of `s`
    /// spans leaves its start at `3 s (Q1 − Q0)`), and the rows of
    /// [`Equation::SplineSeam`], one coordinate each, with the gradients
    /// of their central differences and only that coordinate's slots.
    #[test]
    fn the_seam_of_a_periodic_spline_has_exact_rows() {
        let control = [
            [0.0, 0.0],
            [1.0, 2.0],
            [2.5, -1.0],
            [4.0, 3.0],
            [5.0, 0.5],
            [6.5, 1.0],
            [8.0, 0.0],
            [9.0, 2.0],
            [11.0, 1.0],
        ];
        let count = control.len();
        let knots_of = |degree: usize| {
            let spans = count - degree;
            let mut knots = vec![0.0; degree + 1];
            knots.extend((1..spans).map(|i| i as f64 / spans as f64));
            knots.extend(vec![1.0; degree + 1]);
            knots
        };
        let (start, end) = crate::spline::seam_weights(&knots_of(3), 3, count, 1).unwrap();
        let s = (count - 3) as f64;
        for (got, want) in start
            .iter()
            .chain(&end)
            .zip([-3.0 * s, 3.0 * s, -3.0 * s, 3.0 * s])
        {
            assert!((got - want).abs() <= 1e-12 * s, "{start:?} {end:?}");
        }
        assert!(crate::spline::seam_weights(&knots_of(3), 3, count, 0).is_none());
        assert!(crate::spline::seam_weights(&knots_of(3), 3, count, 4).is_none());
        for degree in 2..=crate::spline::MAX_SPLINE_DEGREE {
            let splines = [SplineVars {
                degree,
                knots: knots_of(degree),
                control: (0..count).map(|i| Pt::Own(2 * i)).collect(),
            }];
            let x: Vec<f64> = control.iter().flatten().copied().collect();
            for order in 1..degree {
                for axis in [Axis::X, Axis::Y] {
                    let eq = Equation::SplineSeam {
                        spline: 0,
                        order,
                        axis,
                    };
                    let e = eq.eval_with(&x, 1e-9, &splines, &[]).unwrap();
                    assert_eq!(e.count, 1);
                    assert!(
                        e.used <= 2 * (order + 1),
                        "degree {degree} order {order}: {}",
                        e.used
                    );
                    let mut gradient = vec![0.0; x.len()];
                    for slot in 0..e.used {
                        gradient[e.slots[slot]] += e.rows[0].d[slot];
                    }
                    let h = 1e-6;
                    for i in 0..x.len() {
                        let (mut lo, mut hi) = (x.clone(), x.clone());
                        lo[i] -= h;
                        hi[i] += h;
                        let want = (eq.eval_with(&hi, 1e-9, &splines, &[]).unwrap().rows[0].v
                            - eq.eval_with(&lo, 1e-9, &splines, &[]).unwrap().rows[0].v)
                            / (2.0 * h);
                        assert!(
                            (gradient[i] - want).abs() < 1e-6,
                            "degree {degree} order {order} unknown {i}: {} against {want}",
                            gradient[i]
                        );
                    }
                }
            }
        }
    }

    /// The rows of a line or a circle tangent to a spline, at an end and
    /// touching it anywhere, and of a spline through a point, have the
    /// gradients of their central differences: the place where they touch
    /// is found anew at every evaluation, so this holds the elimination of
    /// its parameter to account.
    #[test]
    fn the_gradients_of_tangency_to_a_spline_match_finite_differences() {
        let control = [
            [0.0, 0.0],
            [1.0, 2.0],
            [2.5, -1.0],
            [4.0, 3.0],
            [5.0, 0.5],
            [6.5, 1.0],
            [8.0, 0.0],
            [9.0, 2.0],
            [11.0, 1.0],
        ];
        let count = control.len();
        for degree in 2..=6 {
            let spans = count - degree;
            let mut knots = vec![0.0; degree + 1];
            knots.extend((1..spans).map(|i| i as f64 / spans as f64));
            knots.extend(vec![1.0; degree + 1]);
            let splines = [SplineVars {
                degree,
                knots,
                control: (0..count).map(|i| Pt::Own(2 * i)).collect(),
            }];
            let base = 2 * count;
            let mut x: Vec<f64> = control.iter().flatten().copied().collect();
            // A line, a circle (centre and radius) and a point.
            x.extend([2.0, 4.0, 6.0, 4.5, 3.1, 0.7, 0.5, 4.4, 1.2]);
            let line = Ln {
                a: Pt::Own(base),
                b: Pt::Own(base + 2),
            };
            let circle = Round::Circle {
                c: Pt::Own(base + 4),
                r: base + 6,
            };
            let arc = Round::Arc {
                c: Pt::Own(base + 4),
                s: Pt::Own(0),
            };
            let equations = [
                Equation::TangentSplineLine {
                    line,
                    spline: 0,
                    end: None,
                },
                Equation::TangentSplineLine {
                    line,
                    spline: 0,
                    end: Some(1.0),
                },
                Equation::TangentSplineRound {
                    round: circle,
                    spline: 0,
                    end: None,
                    side: -1.0,
                },
                Equation::TangentSplineRound {
                    round: arc,
                    spline: 0,
                    end: Some(0.0),
                    side: 1.0,
                },
                Equation::SplineThrough {
                    p: Pt::Own(base + 7),
                    spline: 0,
                    u: 0.37,
                },
            ];
            for eq in equations {
                let e = eq.eval_with(&x, 1e-9, &splines, &[]).unwrap();
                assert!(e.used <= SLOTS);
                for row in 0..e.count {
                    let mut gradient = vec![0.0; x.len()];
                    for slot in 0..e.used {
                        gradient[e.slots[slot]] += e.rows[row].d[slot];
                    }
                    let h = 1e-6;
                    for i in 0..x.len() {
                        let (mut lo, mut hi) = (x.clone(), x.clone());
                        lo[i] -= h;
                        hi[i] += h;
                        let want = (eq.eval_with(&hi, 1e-9, &splines, &[]).unwrap().rows[row].v
                            - eq.eval_with(&lo, 1e-9, &splines, &[]).unwrap().rows[row].v)
                            / (2.0 * h);
                        assert!(
                            (gradient[i] - want).abs() < 1e-6,
                            "degree {degree} {eq:?} row {row} unknown {i}: {} against {want}",
                            gradient[i]
                        );
                    }
                }
            }
        }
    }

    /// At degree 7 a line, circle or arc touching a spline whose control
    /// points move needs more slots than one equation has: refused with
    /// its own reason, not the image points'. With fixed control points
    /// (a formula curve's fit) it is no problem.
    #[test]
    fn touching_a_moving_spline_of_degree_seven_is_refused_with_its_reason() {
        let count = 9;
        let knots = {
            let mut k = vec![0.0; 8];
            k.push(0.5);
            k.extend(vec![1.0; 8]);
            k
        };
        let mut x: Vec<f64> = (0..count)
            .flat_map(|i| [i as f64, ((i * 7) % 5) as f64])
            .collect();
        let base = x.len();
        x.extend([0.0, 3.0, 8.0, 3.5, 4.0, 1.0, 0.5]);
        let line = Ln {
            a: Pt::Own(base),
            b: Pt::Own(base + 2),
        };
        let circle = Round::Circle {
            c: Pt::Own(base + 4),
            r: base + 6,
        };
        let moving = [SplineVars {
            degree: 7,
            knots: knots.clone(),
            control: (0..count).map(|i| Pt::Own(2 * i)).collect(),
        }];
        let fixed = [SplineVars {
            degree: 7,
            knots,
            control: (0..count)
                .map(|i| Pt::At([i as f64, ((i * 7) % 5) as f64]))
                .collect(),
        }];
        let touch_line = Equation::TangentSplineLine {
            line,
            spline: 0,
            end: None,
        };
        let touch_circle = Equation::TangentSplineRound {
            round: circle,
            spline: 0,
            end: None,
            side: 1.0,
        };
        for eq in [touch_line, touch_circle] {
            assert!(matches!(
                eq.eval_with(&x, 1e-9, &moving, &[]),
                Err(Fail::Degenerate(TOUCH_TOO_HIGH))
            ));
            let e = eq.eval_with(&x, 1e-9, &fixed, &[]);
            assert!(
                !matches!(e, Err(Fail::Degenerate(TOUCH_TOO_HIGH))),
                "{eq:?}"
            );
        }
        // At its end it is two control points, which fit.
        let at_end = Equation::TangentSplineLine {
            line,
            spline: 0,
            end: Some(0.0),
        };
        assert!(at_end.eval_with(&x, 1e-9, &moving, &[]).is_ok());
    }

    /// A spline of fixed control points has no unknowns: a point on it
    /// depends on the point alone.
    #[test]
    fn a_spline_of_fixed_points_has_no_unknowns() {
        let splines = [SplineVars {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control: vec![Pt::At([-2.0, 4.0]), Pt::At([0.0, -4.0]), Pt::At([2.0, 4.0])],
        }];
        let eq = Equation::PointOnSpline {
            p: Pt::Own(0),
            spline: 0,
        };
        let e = eq.eval_with(&[1.0, 1.0], 1e-9, &splines, &[]).unwrap();
        assert_eq!(e.used, 2);
        // y = x^2 there: (1, 1) is on it.
        assert!(e.rows[0].v.abs() < 1e-12, "{}", e.rows[0].v);
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
        let short = Ln {
            a: Pt::Own(0),
            b: Pt::Own(2),
        };
        let eq = Equation::PointOnLine(Pt::Own(4), short);
        assert_eq!(eq.eval(&x, 1e-6).err(), Some(Fail::Degenerate(SHORT_LINE)));
        assert!(eq.eval(&x, 1e-8).is_ok());
        let far = [0.0, 0.0, 1e200, 1e200, 1.0, 1.0];
        assert!(eq.eval(&far, 1e-6).is_ok());
        let overflow = [0.0, 0.0, 1e308, 1e308, -1e308, 1.0];
        assert_eq!(eq.eval(&overflow, 1e-6).err(), Some(Fail::NonFinite));
    }
}
