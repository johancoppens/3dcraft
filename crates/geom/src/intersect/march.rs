//! The general, numeric surface/surface intersection.
//!
//! One surface is walked in its parameters `(u, v)` (the *chart*), the
//! other is a signed distance `F` (the *field*). Their intersection is the
//! zero set of `f(u, v) = F(S(u, v))`, whose gradient is
//! `(n·S_u, n·S_v)` with `n` the field's unit normal.
//!
//! **Finding every branch.** The chart's rectangle is divided into cells
//! (a quadtree). A cell of half sizes `(hu, hv)` and centre `c` is
//!
//! - *empty* when `|F(S(c))| > Lu hu + Lv hv`: a distance is 1-Lipschitz
//!   and `Lu`, `Lv` bound the chart's speed, so `f` has no zero in it;
//! - *simple* when `|f_u(c)| > Muu hu + Muv hv` (or the same for `v`), with
//!   `M` bounding the second derivatives of `f`
//!   (`Muu = K Lu² + |S_uu|`, `K` the field's curvature): `f` is then
//!   strictly monotonic along u all over the cell, so the cell holds no
//!   point where both partial derivatives vanish, and the zero set crosses
//!   each of its two edges along u at most once, which the signs of `f` at
//!   the edge's ends detect;
//! - otherwise split in four, down to cells no larger than the linear
//!   tolerance (*floor* cells).
//!
//! A branch that no sign change detects would have to cross only edges
//! along which `f` is not monotonic. Following it through simple cells,
//! that makes it a closed loop inside cells that are all monotonic in the
//! same direction, which is impossible (a loop has a point where its
//! tangent has that direction, and there that partial derivative is zero),
//! or a piece that leaves the rectangle twice through one cell's edge on
//! the rectangle's border, which is why border edges are subdivided by the
//! same rule in one dimension. What is left passes through floor cells:
//! near points where the surfaces are tangent. There the critical point of
//! `f` is found by Newton iteration and classified by the Hessian: a
//! minimum or maximum is an isolated touch (reported as a point; a loop
//! around it where the surfaces go through each other by no more than the
//! linear tolerance is indistinguishable from it), a saddle is a point
//! where two branches cross (every arm gets a seed, and the march stops
//! there). Where the Hessian has rank one the point is looked for along the
//! valley of its strong direction, and its arms (a tacnode's four, a
//! cusp's two) are made as graphs in a disc around it (`tacnode.rs`); a
//! contact that keeps to one side is a touch; anything else is refused.
//!
//! **Discs around critical points.** Around a critical point at distance
//! `r` the gradient of `f` is about `λ r` (`λ` the smaller eigenvalue of
//! the Hessian), and a cell of size `h` is simple only where that exceeds
//! `M h`: at every level about `π (M/λ)²` cells around the point are
//! neither empty nor simple, thousands when the bound `M` is loose or the
//! contact nearly degenerate, down to the floor. So the critical point is
//! looked for as soon as cells keep failing both tests (and from every
//! floor cell), and where one is found whose second-order model
//! `∇f(x) ≈ H (x − c)` holds within a quarter on two rings around it, the
//! disc inside the outer ring is settled without cells: `f` has no other
//! critical point there, so its zero set in the disc is the point itself
//! (a touch), the two branches through it (a crossing, seeded by its
//! arms), or nothing (a critical point off the other surface with `f` of
//! the sign its Hessian keeps). The rings are samples, not a proof: the
//! same standing as the sampled derivative bounds below.
//!
//! The derivative bounds are proofs for the analytic kinds and polynomial
//! Bézier patches as chart, and for the analytic kinds as field; for NURBS
//! charts and freeform fields they are estimates from samples.
//!
//! **Marching.** From each seed the branch is followed both ways by a
//! tangent step and Newton correction back onto `f = 0`, with the step
//! limited by the turn of the tangent. It ends where it closes, leaves the
//! chart's rectangle, runs off the edge of the other surface, or reaches a
//! crossing point or a cone's apex.
//!
//! **Fitting.** The points and tangents define a C¹ cubic (a Hermite
//! spline, as a NURBS with double knots, parametrised close to arc
//! length). Its distance to both surfaces is measured at three places per
//! span and points are added until it is within the tolerance.
//!
//! **A step onto the curve beside.** Nothing a step is held to (how far
//! it goes, the turn of the tangent, its end on the curve) tells a step
//! along a curve from one that lands on another stretch of `f = 0` beside
//! it that heads the same way: a later stretch of this curve where it
//! runs back and forth a little apart (a step past the turn of a hairpin
//! lands two stretches on), or the foot of another curve on the chart's
//! border (a run's last step, which leaves the rectangle, is put on the
//! border where a zero is found from where the step pointed). The fit
//! tells them apart: between two such points it finds no points of one
//! curve, and gives up on the span. The run is then taken up again from
//! the point before that step, in steps of at most a quarter of it over
//! twice its length, and what it had marched beyond is dropped (another
//! curve there is found from its own seeds).
//! Only where the fit gives up: a branch it brings within the tolerance
//! is marched once, as it was before branches were taken up again
//! (docs/design/booleans.md › *A step of the march onto the curve beside
//! the one it follows*).
//!
//! **A step through a crossing point.** Where two curves of the crossing
//! cross each other the sense of travel flips, so a step through such a
//! point that lands on the same curve beyond it can have a tangent there
//! that agrees with the one it left with; the run then goes back towards
//! the point, over the stretch the fit fills the step's span with, and
//! the fit accepts the branch (it is within the tolerance of both
//! surfaces all along). What the fit accepted is read for that, for no
//! evaluation: two spans in a row that turn back on each other
//! ([`Marcher::folds`]). The step the fold lies on is then taken up again
//! as a span the fit gave up on is; asked as first made, the march
//! refuses ([`folded`]). A branch that does not run back is the one it
//! was, bit for bit (docs/design/booleans.md › *A step through a point
//! where two curves of a crossing cross each other: traced and mended*).
//!
//! **A crossing point the surfaces are a little apart at.** Where the
//! surfaces are within the tolerance of each other at a crossing point but
//! not on each other, `f = 0` does not reach the point: it turns round it,
//! from one arm onto the next, `turn` from it. Inside the point's zone a
//! branch is drawn straight to the point instead, and a seed there lies a
//! few tolerances off what was drawn; nearer than a few `turn` a run
//! towards the point cannot see it straight ahead and goes round it. The
//! arm a place lies along is read from its direction at the point: a seed
//! inside the zone along an arm a branch already comes into the point along
//! is on that branch ([`Marcher::along_traced`]), and a run that goes round
//! the point onto such an arm, or onto the one the trace's other run came
//! in along, ends at the point ([`Marcher::passed`]). Without such a zone
//! nothing of this is computed (docs/design/booleans.md › *A stretch given
//! twice beside a crossing point the surfaces are a little apart at*).

// `!(x > 0.0)` is deliberate throughout: unlike `x <= 0.0` it is also true
// for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use craft3d_math::Portable;
mod tacnode;

use super::field::{Bounds, Chart, Cusp, Field, Sample, TOUCH};
use super::meet::{Meeting, MeetingCurve};
use crate::{Curve3, GeomError, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Bbox3, Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};

/// Cells looked at before the surfaces are declared too close to tangent
/// (or coincident) over too large an area. A flat face against a domed
/// roof at a point where it is tangent stays near the roof over much of
/// it, and the cells there are simple only when small: two such pairs of
/// the corpus (tangent/block-spline/005, tangent/cylinder-spline/008) need
/// 401,264 and 440,000, in about 1.6 s, and their common volumes agree
/// with the volume integrated directly to 1e-7. (400,000 until
/// 2026-10-14.)
const MAX_CELLS: usize = 1_000_000;
const MAX_FLOOR_CELLS: usize = 2_000;
const MAX_POINTS: usize = 16_000;
const MAX_BRANCHES: usize = 512;
const MAX_EVALS: usize = 40_000_000;
/// Cells that are neither empty nor simple before a critical point is
/// looked for the first time; doubled after every look that settles
/// nothing.
const FIRST_LOOK: usize = 16;
/// How far the gradient of `f` may stray from its second-order model on
/// the rings of a disc, as a fraction of the model.
const DISC_MODEL: f64 = 0.25;
/// Deepest subdivision: the finest cell is `2^-MAX_LEVELS` of a base cell.
const MAX_LEVELS: u32 = 40;
/// Largest turn of the tangent in one step, in radians.
const MAX_TURN: f64 = 0.2;
/// Sine of the angle between the surfaces below which a point is not
/// marched through.
const MIN_SINE: f64 = 1e-8;
/// How often a step that goes too far or too short, however short it is
/// made, is scaled anew ([`Marcher::rescaled`]): halving between a scale
/// too short and one too far closes in on the window of a step (half to
/// twice its length) across a jump in speed of `2^(RESCALES − 2)` and
/// more.
const RESCALES: usize = 48;
/// The share of the linear tolerance within which a marched point is held
/// to the curve along the chart ([`Marcher::held`]).
const ALONG: f64 = 1.0 / 16.0;
/// The share of the linear tolerance beyond which a place is not on the
/// curve at all, however near `f = 0` it is ([`Marcher::placed`]).
const ACROSS: f64 = 0.5;
/// What rounding leaves of a number, as a share of its size: 64 units in
/// the last place. The nearest a point is asked to `f = 0`: a margin of
/// 16 over what a distance is in fact told to ([`TOLD`]).
const ROUNDING: f64 = 64.0 * f64::EPSILON;
/// What a surface's distance is told to at a place, as a share of the
/// size of the numbers it is computed from: 4 units in the last place (a
/// point of a chart and a distance to a field are each a few additions
/// and one root of numbers of that size). An estimate, not a bound.
const TOLD: f64 = 4.0 * f64::EPSILON;
/// How many times the size of the box the surfaces' own numbers are
/// before a surface is *far* ([`Marcher::placed`]).
const FAR_SURFACE: f64 = 2.0;
/// Places at which a border of a freeform chart is sampled for the
/// stretches of it that lie on the field ([`Marcher::border_stretches`]).
const BORDER_SAMPLES: u32 = 64;
/// Halvings by which an end of such a stretch is found.
const BORDER_HALVINGS: usize = 48;
/// How often a branch is taken up again from the point before a span
/// that the fit could not bring within the tolerance
/// ([`Marcher::trace`]).
const TAKEN_UP: usize = 8;
/// The longest step of a march taken up again, as a share of the span
/// that could not be fitted.
const SHORTER: f64 = 0.25;
/// How far a march taken up again is held to that step, in lengths of
/// the span: a step ends no further than twice what it was asked, so what
/// the step that made the span passed over lies within that of its start.
const HELD_OVER: f64 = 2.0;
/// The cosine of the turn between two spans in a row of a fitted branch
/// below which the branch runs back over itself there (a turn of more
/// than 120 degrees: [`Marcher::folds`]).
const FOLD: f64 = -0.5;
/// How long, in tolerances, each of the two spans about a fold must be
/// at least (and 64 `on`): two points a few tolerances apart say nothing
/// of which way the curve runs.
const FOLD_SPAN: f64 = 4.0;

fn stalled() -> GeomError {
    GeomError::NoConvergence(
        "marching an intersection curve stalled (the surfaces are nearly tangent along it, or one of them has a crease)",
    )
}

/// Half the step, or the error that marching has stalled.
///
/// (Where the march cannot go on outside the box the intersection is asked
/// in, the branch ends there: nothing is asked there, the box cuts it
/// anyway, and the cells there were not looked at, so a point of tangency
/// or a crossing the march meets there is not known. The engine block's
/// fillet tool and rib, two cylinders that touch just outside the box of
/// their faces, stalled at the touch.)
/// The stall of a march at a place that cannot be placed
/// ([`Marcher::placed`]).
fn unplaced() -> GeomError {
    GeomError::NoConvergence(
        "an intersection curve cannot be placed within the tolerance where the surfaces cross this shallowly: one of them is so much larger than the part that the nearest a point can be brought to the curve is more than half the tolerance",
    )
}

fn halved(h: f64, hmin: f64) -> Result<f64, GeomError> {
    if 0.5 * h < hmin {
        Err(stalled())
    } else {
        Ok(0.5 * h)
    }
}

/// The fit's refusal: no points it could add brought a span within the
/// tolerance.
pub(super) fn not_fitted() -> GeomError {
    GeomError::NoConvergence("an intersection curve could not be fitted within the tolerance")
}

/// The refusal of a branch whose fitted points run back over themselves
/// ([`Marcher::folds`]): what the fit accepted is not one pass along a
/// curve, and no step of the march could be put right.
pub(super) fn folded() -> GeomError {
    GeomError::NoConvergence(
        "an intersection curve could not be followed: the points marched along it run back over themselves",
    )
}

pub(super) fn too_many() -> GeomError {
    GeomError::NoConvergence(
        "the surfaces are tangent or nearly coincident over too large an area to intersect",
    )
}

#[derive(Clone, Copy, Debug)]
struct Eval {
    p: Vec3,
    su: Vec3,
    sv: Vec3,
    f: f64,
    dist: f64,
    n: Vec3,
    gu: f64,
    gv: f64,
}

/// A point of a branch: chart parameters (not wrapped), position, unit
/// tangent in the direction of travel and the same tangent in parameters.
#[derive(Clone, Copy, Debug)]
struct Pt {
    u: f64,
    v: f64,
    p: Vec3,
    t: Vec3,
    du: f64,
    dv: f64,
}

impl Pt {
    fn reversed(self) -> Pt {
        Pt {
            t: -self.t,
            du: -self.du,
            dv: -self.dv,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SingularKind {
    /// The surfaces touch and separate again: no curve.
    Touch,
    /// Two branches cross.
    Cross,
    /// The apex of the field's cone.
    Apex,
    /// A point of contact of higher order with arms that leave it in the
    /// same direction (a tacnode or a cusp, `tacnode.rs`).
    Tacnode,
}

#[derive(Clone, Copy, Debug)]
struct Singular {
    u: f64,
    v: f64,
    p: Vec3,
    kind: SingularKind,
    /// How far from a crossing point its arms are seeded.
    arm: f64,
    /// How far from a crossing point the branches of `f = 0` turn away
    /// from it, where the surfaces are a little apart there.
    turn: f64,
}

/// A crossing point read as one although the surfaces are a little apart
/// there: within `radius` a branch is drawn straight to the point instead
/// of along `f = 0`, which turns away before it gets there (round the
/// point, from one arm onto the next, `turn` from it).
#[derive(Clone, Debug)]
struct Zone {
    u: f64,
    v: f64,
    p: Vec3,
    radius: f64,
    /// Nearer the point than this a seed is left to the arms (half the
    /// radius at which they are seeded).
    near: f64,
    /// The directions in space in which its arms leave the point.
    arms: Vec<Vec3>,
}

/// Where Newton on the gradient of `f` came to rest.
#[derive(Clone, Copy, Debug)]
struct Stationary {
    u: f64,
    v: f64,
    e: Eval,
    /// The lengths of the chart's derivatives there.
    lu: f64,
    lv: f64,
    /// The gradient and the Hessian of `f` along the surface, in units of
    /// length.
    g: (f64, f64),
    h: (f64, f64, f64),
    /// No step was left to take: the Hessian is degenerate there.
    flat: bool,
}

/// An ellipse in the chart's parameters around a critical point,
/// `((u − cu) lu)² + ((v − cv) lv)² ≤ radius²`, inside which the zero set
/// of `f` is known without cells.
#[derive(Clone, Copy, Debug)]
struct Disc {
    u: f64,
    v: f64,
    lu: f64,
    lv: f64,
    radius: f64,
}

impl Disc {
    fn holds(&self, u: f64, v: f64) -> bool {
        ((u - self.u) * self.lu).phypot((v - self.v) * self.lv) <= self.radius
    }

    /// True when the cell around `(u, v)` of half sizes `(hu, hv)` lies in
    /// the disc: an ellipse is convex, so its corners decide.
    fn covers(&self, u: f64, v: f64, hu: f64, hv: f64) -> bool {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
            .iter()
            .all(|&(su, sv)| self.holds(u + su * hu, v + sv * hv))
    }
}

/// A stretch of a border of the chart (side 0 to 3: u start, u end,
/// v start, v end) that lies on the field: from `from` to `to` in the
/// border's parameter, the border's iso-curve (`None` for a border that is
/// one point).
#[derive(Clone, Debug)]
struct Lying {
    side: usize,
    from: f64,
    to: f64,
    closed: bool,
    /// The whole border: along it, everything is on the stretch.
    whole: bool,
    /// The least sine of the angle between the surfaces measured across
    /// the stretch ([`Marcher::leaves_field`]), which sizes its band;
    /// `None` for a whole border along which the surfaces do not cross
    /// (tangent: no band, as before stretches).
    sine: Option<f64>,
    curve: Option<NurbsCurve>,
    deviation: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    Closed,
    Open,
}

struct Branch {
    pts: Vec<Pt>,
    closed: bool,
}

/// A march taken up again part of the way along a run
/// ([`Marcher::trace`]).
#[derive(Clone, Copy, Debug)]
struct Again {
    /// The point it goes on from, travelling as the run does.
    from: Pt,
    /// The points the run had before it, `from` among them.
    taken: usize,
    /// The longest step while it has gone less than `over` from `from`.
    cap: f64,
    over: f64,
}

impl Again {
    /// A run taken up again from `from`, its `taken`-th point, where the
    /// fit gave up on the step from there, `span` long: in steps of a
    /// quarter of that, over twice its length.
    fn past(from: Pt, taken: usize, span: f64) -> Self {
        Self {
            from,
            taken,
            cap: SHORTER * span,
            over: HELD_OVER * span,
        }
    }
}

/// What refining a branch came to (`Marcher::refine_to_edge`).
enum Refined {
    Done,
    /// The middle of the span from this point to the next (the point
    /// given, the one the refinement put there) is past the edge of the
    /// field's surface.
    Leaves(usize, Pt),
}

struct Grid {
    u0: f64,
    v0: f64,
    du: f64,
    dv: f64,
    nu: i64,
    nv: i64,
    base: i64,
}

impl Grid {
    fn node(&self, i: i64, j: i64) -> (f64, f64) {
        (self.u0 + self.du * i as f64, self.v0 + self.dv * j as f64)
    }
}

pub(super) struct Marcher {
    chart: Chart,
    field: Field,
    own: Field,
    tol: Tolerance,
    /// How close to `f = 0` a marched point is brought.
    on: f64,
    /// How large the numbers are that the two surfaces' evaluations add
    /// and subtract beyond the place itself (the larger [`Field::scale`]
    /// of the two): with the place, what `f` is rounded at
    /// ([`Marcher::floor_at`]).
    reach: f64,
    /// Those numbers are more than [`FAR_SURFACE`] times the box's: a
    /// surface far larger than the part ([`Marcher::placed`]).
    far: bool,
    /// Distance from the field's surface beyond which a point with
    /// `f = 0` is past that surface's edge.
    cut: f64,
    hmax: f64,
    hmin: f64,
    evals: usize,
    /// Stretches of the borders of the chart's rectangle that lie on the
    /// field's surface: reported as curves, not marched.
    lying: Vec<Lying>,
    /// Crossing points read as such although the surfaces are a little
    /// apart there, each with the radius within which a branch is drawn
    /// straight to the point instead of along `f = 0`, which turns away
    /// before it gets there.
    zones: Vec<Zone>,
    /// The arms along which a branch traced so far comes into the point
    /// of a zone, as (zone, arm): a seed beside such an arm inside the
    /// zone is on that branch ([`Marcher::along_traced`]), and a run that
    /// goes round the point onto such an arm ends at the point
    /// ([`Marcher::passed`]).
    traced_into: Vec<(usize, usize)>,
    /// Points of contact of higher order whose arms are made near them.
    tacnodes: Vec<tacnode::Tacnode>,
    /// The chart of the field's surface when that is freeform: its
    /// borders are edges, where the field goes on past the surface.
    field_chart: Option<Chart>,
    /// The box the intersection is asked in.
    within: Bbox3,
    /// Cells are also left out where a bound on the field's second
    /// derivative over them proves it clear of zero ([`Marcher::bounded`]).
    bounded: bool,
    /// The span the fit last gave up on: the two points, of the points it
    /// was given, between which no points it could add brought the curve
    /// within the tolerance ([`Marcher::refine_to_edge`]).
    unfitted: Option<(Pt, Pt)>,
    /// A branch is taken up again where the fit gives up on a step's span
    /// ([`Marcher::trace`]); not when the march is asked as it was before
    /// branches were taken up ([`Marcher::once`]).
    takes_up: bool,
}

impl Marcher {
    pub(super) fn new(
        chart: Chart,
        field: Field,
        within: &Bbox3,
        tol: &Tolerance,
    ) -> Result<Self, GeomError> {
        let own = Field::new(&chart.surface)?;
        let (lo, hi) = within
            .corners()
            .ok_or(GeomError::InvalidParameter("an empty box"))?;
        let scale = lo.distance(hi);
        let corner = lo.length().max(hi.length());
        let magnitude = corner + scale;
        let field = field.for_places_within(corner);
        // The larger of the two, not their sum: two ordinary bodies a
        // million from the origin have each numbers of a million, and
        // their sum read as twice the box's.
        let reach = own.scale().max(field.scale());
        // No nearer than rounding leaves at the size of the box. (Of the
        // box alone: a surface whose own numbers are larger does not make
        // the pair one that cannot be asked, whatever the angle: a ball in
        // the wall of a cylinder of a million crosses it squarely and is
        // answered. What its size leaves of a shallow crossing is
        // `placed`'s to judge.)
        let on = (1e-4 * tol.linear).max(ROUNDING * magnitude);
        if !(scale > 0.0) || on > 0.02 * tol.linear {
            return Err(GeomError::InvalidParameter(
                "the box has no size, or the tolerance is finer than rounding allows at its size",
            ));
        }
        Ok(Self {
            chart,
            field,
            own,
            tol: *tol,
            on,
            reach,
            far: reach > FAR_SURFACE * magnitude,
            cut: 0.25 * tol.linear,
            hmax: scale / 32.0,
            hmin: 4.0 * on,
            evals: 0,
            lying: Vec::new(),
            zones: Vec::new(),
            traced_into: Vec::new(),
            tacnodes: Vec::new(),
            field_chart: None,
            within: *within,
            bounded: false,
            unfitted: None,
            takes_up: true,
        })
    }

    /// Also leaves out a cell where the field is proven clear of zero by
    /// its value, its gradient and a bound on its second derivative at
    /// the cell's centre (Taylor's theorem), not by its value and the
    /// cell's reach alone: two surfaces that run side by side a little
    /// apart over a large area (a plate's skin beside the skin of the
    /// plate over it, a few millimetres away over metres) need cells no
    /// smaller than the gap allows by the first rule, and far fewer by
    /// this one (docs/design/booleans.md › *Surfaces side by side*). Asked
    /// for only where the first rule ran out of cells, so what was traced
    /// before is traced the same way. A proof where the walked chart is
    /// analytic or a Bézier patch and the field analytic (their bounds are
    /// bounds); an estimate where the chart is a NURBS surface (its bounds
    /// are sampled) or the field freeform (its bend is estimated from the
    /// projector's grid), as the simple-cell test is.
    pub(super) fn bounded(mut self) -> Self {
        self.bounded = true;
        self
    }

    /// True when the field has no zero in the cell of half-widths
    /// `hu`, `hv` about `(u, v)`, whose centre is `e`, by the bound of
    /// [`Marcher::bounded`]. Only where the field is the signed distance
    /// to its surface throughout (the feet of the centre and the corners
    /// are not on a border of a freeform surface, past which the field is
    /// the distance to the border, with a bend of its own).
    #[allow(clippy::too_many_arguments)]
    fn clear_by_bound(
        &mut self,
        e: &Eval,
        u: f64,
        v: f64,
        hu: f64,
        hv: f64,
        reach: f64,
        b: &Bounds,
    ) -> bool {
        if !((e.f.abs() - e.dist).abs() <= self.on) {
            return false;
        }
        let Some(k) = self.field.bend_at(e.p, reach, e.dist) else {
            return false;
        };
        let (muu, muv, mvv) = (
            k * b.lu * b.lu + b.muu,
            k * b.lu * b.lv + b.muv,
            k * b.lv * b.lv + b.mvv,
        );
        let change = e.gu.abs() * hu
            + e.gv.abs() * hv
            + 0.5 * (muu * hu * hu + 2.0 * muv * hu * hv + mvv * hv * hv);
        if !(e.f.abs() > 1.01 * change + self.tol.linear) {
            return false;
        }
        if let Field::Free(projector) = &self.field {
            if projector.on_border(e.p) {
                return false;
            }
            for (du, dv) in [(-hu, -hv), (hu, -hv), (-hu, hv), (hu, hv)] {
                self.evals += 1;
                let (p, ..) = self.chart.at(u + du, v + dv);
                if !p.is_finite() || projector.on_border(p) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether a cell or an edge of the grid whose centre is `dist` from
    /// the field's surface and whose image lies within `reach` of the
    /// centre's is clear of zero: only when `dist` exceeds `reach` by more
    /// than rounding (`on`). A zero exactly on a node, where the reach is
    /// tight (a surface linear across the cell, a plane square to it),
    /// lies at the distance of the reach: a centre rounded a hair further
    /// off left the cell (or a border's edge) out as clear, the cell
    /// beyond counted the node's zero with its own sign and showed no
    /// change, and the curve was lost (docs/design/booleans.md › *A plane
    /// through a node of the marcher's grid*).
    fn clear_of_zero(&self, dist: f64, reach: f64) -> bool {
        dist > reach + self.on
    }

    /// The march as it was before branches were taken up again: where
    /// the fit gives up on a branch, the intersection is refused. What a
    /// pair is asked first, in every box it is asked in, so that whatever
    /// was answered so is answered so still (`meet.rs` › `meet_surfaces`).
    pub(super) fn once(mut self) -> Self {
        self.takes_up = false;
        self
    }

    /// Tells the marcher the chart of the field's surface, when that is a
    /// freeform surface with borders (see [`Marcher::field_border_seeds`]).
    pub(super) fn with_field_chart(mut self, chart: Chart) -> Self {
        self.field_chart = Some(chart);
        self
    }

    /// The evaluations of the chart made so far.
    pub(super) fn evaluations(&self) -> u64 {
        u64::try_from(self.evals).unwrap_or(u64::MAX)
    }

    fn spent(&self) -> Result<(), GeomError> {
        if self.evals > MAX_EVALS {
            Err(GeomError::NoConvergence(
                "the intersection needs more evaluations than the cap allows",
            ))
        } else {
            Ok(())
        }
    }

    fn eval(&mut self, u: f64, v: f64) -> Option<Eval> {
        self.evals += 1;
        let (p, su, sv) = self.chart.at(u, v);
        let Sample {
            signed,
            dist,
            normal,
        } = self.field.sample(p)?;
        (su.is_finite() && sv.is_finite()).then_some(Eval {
            p,
            su,
            sv,
            f: signed,
            dist,
            n: normal,
            gu: normal.dot(su),
            gv: normal.dot(sv),
        })
    }

    fn inside(&self, u: f64, v: f64) -> bool {
        (self.chart.periodic_u || (u >= self.chart.u.0 && u <= self.chart.u.1))
            && (self.chart.periodic_v || (v >= self.chart.v.0 && v <= self.chart.v.1))
    }

    /// True when a point is on the border of the chart's rectangle.
    fn on_border(&self, p: &Pt) -> bool {
        let c = &self.chart;
        let at = |x: f64, (lo, hi): (f64, f64)| {
            let slack = 1e-12 * (hi - lo);
            x <= lo + slack || x >= hi - slack
        };
        (!c.periodic_u && at(p.u, c.u)) || (!c.periodic_v && at(p.v, c.v))
    }

    /// How near `f = 0` a point is brought at a place: `on`, and where the
    /// surfaces cross at so shallow an angle there that a point within
    /// `on` of the field's surface could be further than [`ALONG`] of the
    /// tolerance from the curve along the chart (`on / sin θ`, `θ` the
    /// angle between the surfaces), `on sin θ`, which holds it within `on`
    /// of the curve, or as near as rounding leaves there
    /// ([`Marcher::floor_at`]).
    ///
    /// A point within `on` of the field is on both surfaces to that, but
    /// where the surfaces are nearly tangent it is anywhere across a band
    /// about the curve, `on / sin θ` wide. A step of the march is held to
    /// its own length and to the turn of the tangent, and a span of the
    /// fit to a point of the intersection between its ends
    /// ([`Marcher::deviation`]): both take their points to be on the
    /// curve. Where the band is wider than the curve's own bend (a plane
    /// that all but holds a ruling of a lofted skin crosses it in a
    /// hairpin a few tolerances across, at a sine of a few 1e-5), no step
    /// however short landed where it was held to and the march stalled,
    /// or it went round by the luck of where its steps fell and gave a
    /// curve a tolerance or two beside the crossing, on both surfaces
    /// (docs/design/booleans.md › *A spar's face beside a rib's skin*).
    fn held(&self, e: &Eval) -> f64 {
        let Some(normal) = e.su.cross(e.sv).normalized() else {
            return self.on;
        };
        let sine = normal.cross(e.n).length();
        if self.on <= ALONG * self.tol.linear * sine {
            self.on
        } else {
            (self.on * sine).max(self.floor_at(e.p)).min(self.on)
        }
    }

    /// The nearest a point is asked to `f = 0` at a place, whatever the
    /// angle: 64 units in the last place of the place's own size and of
    /// the numbers the two surfaces are computed from (`reach`). Of the
    /// place, not of the box: a boolean asks a part of millimetres against
    /// a ball of 1e5 in a box grown by thousands (a share of the ball's
    /// own box), where rounding at the box's size is `on` itself and a
    /// point could be held no nearer the curve than before points were
    /// held to it; and of the surfaces, since a point of a chart whose
    /// axis is 1e5 away is rounded at 1e5 wherever it lies. (Never above
    /// `on`: [`Marcher::held`].)
    fn floor_at(&self, p: Vec3) -> f64 {
        ROUNDING * (p.length() + self.reach)
    }

    /// Whether a place as near `f = 0` as it is held there
    /// ([`Marcher::held`]) is within half the tolerance of the curve
    /// along the chart, where one of the surfaces is far larger than the
    /// box (`far`: its own numbers are more than twice the box's).
    ///
    /// Such a surface tells its distance at its own size, not at the
    /// part's. Where that leaves a point more than half a tolerance from
    /// the curve at the sine the surfaces cross at, no place there can be
    /// told to be on the curve: a step that ends there is not a step, and
    /// the march refuses in those words ([`unplaced`]). How near a point
    /// is: as near as it is held, where the floor is within `on`; and
    /// where the floor is above `on` (the point is then asked to `on`,
    /// nearer than the floor, as it always was), `on` and what the
    /// surface's distance is in fact told to there ([`TOLD`]).
    ///
    /// Only against a far surface. A part far from the origin has a box
    /// as large as its surfaces' numbers, and the box has a rule of its
    /// own (`on` no more than a fiftieth of the tolerance,
    /// [`Marcher::new`]): within it a point brought to `on` is on the
    /// curve, as it always was, and two ordinary bodies a million from the
    /// origin are answered as at the origin. (Without a normal of the
    /// chart there is no angle to read, and the place stands.)
    fn placed(&self, e: &Eval) -> bool {
        if !self.far {
            return true;
        }
        let Some(normal) = e.su.cross(e.sv).normalized() else {
            return true;
        };
        let sine = normal.cross(e.n).length();
        let floor = self.floor_at(e.p);
        let near = if floor > self.on {
            self.held(e) + TOLD * (e.p.length() + self.reach)
        } else {
            self.held(e)
        };
        near <= ACROSS * self.tol.linear * sine
    }

    /// Newton back onto `f = 0`, moving the shortest way in space: to the
    /// first place as near `f = 0` as it is held there ([`Marcher::held`]),
    /// and where none is reached (`f` has no slope to follow, the tries run
    /// out, or a later place cannot be evaluated: the surfaces tangent
    /// there, or a hair apart), the first place within `on`, which is what
    /// was returned before points were held to the curve.
    fn correct(&mut self, mut u: f64, mut v: f64) -> Option<(Eval, f64, f64)> {
        let mut within: Option<(Eval, f64, f64)> = None;
        for _ in 0..16 {
            let Some(e) = self.eval(u, v) else {
                break;
            };
            if e.f.abs() <= self.on {
                if e.f.abs() <= self.held(&e) {
                    return Some((e, u, v));
                }
                within = within.or(Some((e, u, v)));
            }
            // w = G⁻¹ g with G the first fundamental form.
            let (a, b, c) = (e.su.dot(e.su), e.su.dot(e.sv), e.sv.dot(e.sv));
            let det = a * c - b * b;
            let (wu, wv) = if det > 1e-14 * a * c {
                ((c * e.gu - b * e.gv) / det, (a * e.gv - b * e.gu) / det)
            } else {
                (e.gu, e.gv)
            };
            let slope = e.gu * wu + e.gv * wv;
            if !(slope > 0.0) {
                break;
            }
            u -= e.f * wu / slope;
            v -= e.f * wv / slope;
            if !(u.is_finite() && v.is_finite()) {
                break;
            }
        }
        within
    }

    /// The point of a branch at an evaluated place, travelling the way
    /// `sign` says.
    fn point(e: &Eval, u: f64, v: f64, sign: f64) -> Option<Pt> {
        let (tu, tv) = (-e.gv, e.gu);
        let t = e.su * tu + e.sv * tv;
        let len = t.length();
        let sine = e.su.cross(e.sv).normalized()?.cross(e.n).length();
        (len > 0.0 && len.is_finite() && sine >= MIN_SINE).then(|| Pt {
            u,
            v,
            p: e.p,
            t: t * (sign / len),
            du: tu * sign / len,
            dv: tv * sign / len,
        })
    }

    // ----- cells and seeds -------------------------------------------------

    fn grid(&self) -> Grid {
        let b = self.chart.bounds;
        let (span_u, span_v) = (
            self.chart.u.1 - self.chart.u.0,
            self.chart.v.1 - self.chart.v.0,
        );
        let (wu, wv) = (b.lu * span_u, b.lv * span_v);
        let big = wu.max(wv);
        let base = |w: f64, periodic: bool| {
            let n = if big > 0.0 {
                (8.0 * w / big).ceil()
            } else {
                1.0
            };
            (n as i64).clamp(if periodic { 4 } else { 1 }, 8)
        };
        let (bu, bv) = (
            base(wu, self.chart.periodic_u),
            base(wv, self.chart.periodic_v),
        );
        let radius = 0.5 * (wu / bu as f64 + wv / bv as f64);
        let levels = if radius > self.tol.linear {
            ((radius / self.tol.linear).plog2().ceil() as u32).min(MAX_LEVELS)
        } else {
            0
        };
        let base_size = 1_i64 << levels;
        let (nu, nv) = (bu * base_size, bv * base_size);
        Grid {
            u0: self.chart.u.0,
            v0: self.chart.v.0,
            du: span_u / nu as f64,
            dv: span_v / nv as f64,
            nu,
            nv,
            base: base_size,
        }
    }

    /// `f` at a grid node, remembered.
    fn corner(
        &mut self,
        grid: &Grid,
        cache: &mut BTreeMap<(i64, i64), f64>,
        i: i64,
        j: i64,
    ) -> f64 {
        let key = (
            if self.chart.periodic_u {
                i.rem_euclid(grid.nu)
            } else {
                i
            },
            if self.chart.periodic_v {
                j.rem_euclid(grid.nv)
            } else {
                j
            },
        );
        if let Some(f) = cache.get(&key) {
            return *f;
        }
        let (u, v) = grid.node(key.0, key.1);
        let f = self.eval(u, v).map_or(f64::NAN, |e| e.f);
        cache.insert(key, f);
        f
    }

    /// The zero of `f` on a grid edge whose ends have different signs.
    /// `along_u` edges run from `(a, fixed)` to `(b, fixed)`.
    fn edge_root(
        &mut self,
        grid: &Grid,
        along_u: bool,
        fixed: i64,
        (a, b): (i64, i64),
        (fa, fb): (f64, f64),
    ) -> Option<(f64, f64)> {
        let place = |s: f64| {
            if along_u {
                (grid.u0 + grid.du * s, grid.v0 + grid.dv * fixed as f64)
            } else {
                (grid.u0 + grid.du * fixed as f64, grid.v0 + grid.dv * s)
            }
        };
        let (mut lo, mut hi, mut flo, mut fhi) = (a as f64, b as f64, fa, fb);
        // Regula falsi with the Illinois correction; bisection as a net.
        let mut side = 0;
        for k in 0..80 {
            let s = if k % 4 == 3 {
                0.5 * (lo + hi)
            } else {
                (lo * fhi - hi * flo) / (fhi - flo)
            };
            let s = if s > lo && s < hi { s } else { 0.5 * (lo + hi) };
            if !(s > lo && s < hi) {
                break; // The bracket is down to rounding.
            }
            let (u, v) = place(s);
            let f = self.eval(u, v)?.f;
            if f.abs() <= self.on {
                return Some((u, v));
            }
            if (f >= 0.0) == (flo >= 0.0) {
                (lo, flo) = (s, f);
                if side == -1 {
                    fhi *= 0.5;
                }
                side = -1;
            } else {
                (hi, fhi) = (s, f);
                if side == 1 {
                    flo *= 0.5;
                }
                side = 1;
            }
        }
        Some(place(0.5 * (lo + hi)))
    }

    /// The stretches of the borders of a freeform chart that lie on the
    /// field's surface ([`Lying`]). The intersection then runs along the
    /// edge of the rectangle, where no cell edge shows a change of sign and
    /// every cell along it would be split to the floor for nothing (a
    /// plane through the edge of a patch, which is every patch standing on
    /// a plate). A border is the surface's own iso-curve there, exact on
    /// the chart and on the field to the sampled deviation.
    ///
    /// **Stretches, not only whole borders** (2026-10-27, the sea kayak's
    /// touching plates). Two surfaces that share an edge each continue
    /// past its ends in their own way (a plate's rim traced past the
    /// chine's ends by the corners' reach), so only the stretch of the
    /// border along the shared edge lies on the other surface. The border
    /// is sampled at [`BORDER_SAMPLES`] places (a place too far from the
    /// field to be on it is skipped: the field is a distance, and the
    /// border moves no faster than the chart's bound on its speed, a proof
    /// for analytic and Bézier charts and an estimate for NURBS charts,
    /// whose bounds are sampled); every run of at least three places
    /// within half the tolerance is a stretch, its ends found by bisection
    /// between the last place on and the first off, held to half the
    /// tolerance at as many places again between them, and kept only where
    /// the surfaces cross along it ([`Marcher::leaves_field`]). A whole
    /// border on the field is one curve over the curve's whole domain, as
    /// before; it has a band and is divided by the branches that end on it
    /// only where the surfaces cross along it (tangent along it, as a
    /// fillet on the face it rounds, it is what it was before).
    fn border_stretches(&mut self) -> Result<(), GeomError> {
        let spline = match &self.chart.surface {
            Surface::Bezier(b) => NurbsSurface::from_bezier(b)?,
            Surface::Nurbs(s) => s.clone(),
            _ => return Ok(()),
        };
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let (degree_u, degree_v) = spline.degrees();
        let (count_u, count_v) = spline.counts();
        let (knots_u, knots_v) = (spline.knots_u(), spline.knots_v());
        let (ctrl, weights) = (spline.control_points(), spline.weights());
        // A border is the iso-curve of its row of the net only where the
        // knots are clamped at that end.
        let clamped = |knots: &[f64], degree: usize, count: usize, at_end: bool| {
            if at_end {
                knots.get(count) == knots.get(count + degree)
            } else {
                knots.first() == knots.get(degree)
            }
        };
        let tol = self.tol.linear;
        for side in 0..4 {
            let along_u = side >= 2;
            let at_end = side % 2 == 1;
            // A surface that closes on itself has no border where it goes
            // around; a border across that way (the rim of a closed wall)
            // runs around and closes on itself.
            if (along_u && self.chart.periodic_v) || (!along_u && self.chart.periodic_u) {
                continue;
            }
            let around = if along_u {
                self.chart.periodic_u
            } else {
                self.chart.periodic_v
            };
            let fixed = match side {
                0 => u0,
                1 => u1,
                2 => v0,
                _ => v1,
            };
            let (start, end) = if along_u { (u0, u1) } else { (v0, v1) };
            let speed = if along_u {
                self.chart.bounds.lu
            } else {
                self.chart.bounds.lv
            };
            let place = move |t: f64| if along_u { (t, fixed) } else { (fixed, t) };
            // The distance to the field at a place of the border; `None`
            // where it cannot be had.
            let dist_at = |me: &mut Self, t: f64| -> Option<f64> {
                let (u, v) = place(t);
                me.eval(u, v).map(|e| e.dist)
            };
            let step = (end - start) / f64::from(BORDER_SAMPLES);
            // (The places as they were before stretches, bit for bit.)
            let at = |k: u32| start + (end - start) * (f64::from(k) / f64::from(BORDER_SAMPLES));
            // The distance to the field at each place (`None`: skipped as
            // too far by the speed bound, or not to be had).
            let mut on: Vec<Option<f64>> = Vec::new();
            let mut skip_to = 0_u32;
            for k in 0..=BORDER_SAMPLES {
                self.spent()?;
                if k < skip_to {
                    on.push(None);
                    continue;
                }
                match dist_at(self, at(k)) {
                    Some(d) if d <= tol => on.push(Some(d)),
                    Some(d) => {
                        on.push(Some(d));
                        // The border comes no nearer than `d − speed Δt`.
                        if speed > 0.0 && step > 0.0 {
                            let clear = (d - tol) / (speed * step);
                            if clear.is_finite() && clear >= 2.0 {
                                let ahead = clear.floor().min(f64::from(BORDER_SAMPLES));
                                skip_to = k.saturating_add(ahead as u32);
                            }
                        }
                    }
                    None => on.push(None),
                }
            }
            let whole = on.iter().all(|d| d.is_some_and(|d| d <= tol));
            // A stretch (not the whole border) is held to half the
            // tolerance: a marched branch that leaves the border where it
            // ends joins it there ([`Marcher::onto_stretch`]), and its last
            // span is then off the field by no more than that.
            let near = if whole { tol } else { 0.5 * tol };
            let close = |d: Option<f64>| d.filter(|&d| d <= near);
            // The runs of places on the field.
            let mut runs: Vec<(u32, u32)> = Vec::new();
            let mut first: Option<u32> = None;
            for k in 0..=BORDER_SAMPLES {
                let here = close(on.get(k as usize).copied().flatten()).is_some();
                match (here, first) {
                    (true, None) => first = Some(k),
                    (false, Some(a)) => {
                        runs.push((a, k - 1));
                        first = None;
                    }
                    _ => {}
                }
            }
            if let Some(a) = first {
                runs.push((a, BORDER_SAMPLES));
            }
            let mut stretches: Vec<(f64, f64, f64, Option<f64>)> = Vec::new();
            for (a, b) in runs {
                if b < a.saturating_add(2) {
                    continue; // A crossing, not a stretch.
                }
                let mut worst = (a..=b)
                    .filter_map(|k| close(on.get(k as usize).copied().flatten()))
                    .fold(0.0_f64, f64::max);
                if whole {
                    // A band beside it only where the surfaces cross along
                    // it (two traces of one chine on the plates' skins,
                    // continued past its ends, agree only to the
                    // tolerance); none where they are tangent along it (a
                    // fillet on the face it rounds), as before stretches.
                    let mut sine: Option<f64> = None;
                    for share in [0.25, 0.5, 0.75] {
                        self.spent()?;
                        match self.leaves_field(side, start + (end - start) * share, tol) {
                            Some(s) => sine = Some(sine.map_or(s, |least: f64| least.min(s))),
                            None => {
                                sine = None;
                                break;
                            }
                        }
                    }
                    stretches.push((start, end, worst, sine));
                    break;
                }
                // The ends: between the last place on and the first off.
                let mut ends = [at(a), at(b)];
                for (which, (inside, outside)) in [(a, a.checked_sub(1)), (b, b.checked_add(1))]
                    .into_iter()
                    .enumerate()
                {
                    let Some(outside) = outside.filter(|&k| k <= BORDER_SAMPLES) else {
                        continue;
                    };
                    let (mut t_on, mut t_off) = (at(inside), at(outside));
                    for _ in 0..BORDER_HALVINGS {
                        self.spent()?;
                        let m = 0.5 * (t_on + t_off);
                        if m == t_on || m == t_off {
                            break;
                        }
                        match dist_at(self, m) {
                            Some(d) if d <= near => {
                                t_on = m;
                                worst = worst.max(d);
                            }
                            _ => t_off = m,
                        }
                    }
                    if let Some(slot) = ends.get_mut(which) {
                        *slot = t_on;
                    }
                }
                let [from, to] = ends;
                // Held to the tolerance between the places.
                let mut held = true;
                for k in 1..BORDER_SAMPLES {
                    self.spent()?;
                    let t = from + (to - from) * f64::from(k) / f64::from(BORDER_SAMPLES);
                    match dist_at(self, t) {
                        Some(d) if d <= near => worst = worst.max(d),
                        _ => {
                            held = false;
                            break;
                        }
                    }
                }
                // The surfaces cross along it: a little way across into
                // the chart the field is clear of zero. (Not a place where
                // the chart is itself within the tolerance of the field
                // all around: near a cusp's point on the axis every
                // meridian is that close to a plane through the axis, and
                // the border there is no meeting of its own.)
                let mut sine: Option<f64> = None;
                if held && to > from {
                    for share in [0.25, 0.5, 0.75] {
                        self.spent()?;
                        let t = from + (to - from) * share;
                        match self.leaves_field(side, t, tol) {
                            Some(s) => sine = Some(sine.map_or(s, |least: f64| least.min(s))),
                            None => {
                                sine = None;
                                break;
                            }
                        }
                    }
                }
                if let Some(sine) = sine {
                    stretches.push((from, to, worst, Some(sine)));
                }
            }
            if stretches.is_empty() {
                continue;
            }
            let (points, curve_weights, knots, degree): (Vec<Vec3>, Vec<f64>, Vec<f64>, usize) =
                if along_u {
                    if !clamped(knots_v, degree_v, count_v, at_end) {
                        continue;
                    }
                    let j = if at_end { count_v - 1 } else { 0 };
                    let row = |i: usize| i * count_v + j;
                    (
                        (0..count_u)
                            .filter_map(|i| ctrl.get(row(i)).copied())
                            .collect(),
                        (0..count_u)
                            .filter_map(|i| weights.get(row(i)).copied())
                            .collect(),
                        knots_u.to_vec(),
                        degree_u,
                    )
                } else {
                    if !clamped(knots_u, degree_u, count_u, at_end) {
                        continue;
                    }
                    let i = if at_end { count_u - 1 } else { 0 };
                    (
                        (0..count_v)
                            .filter_map(|j| ctrl.get(i * count_v + j).copied())
                            .collect(),
                        (0..count_v)
                            .filter_map(|j| weights.get(i * count_v + j).copied())
                            .collect(),
                        knots_v.to_vec(),
                        degree_v,
                    )
                };
            // A border that is one point (the pole of a ball, or of a
            // dome or an octant on an open patch) lies on the field when
            // the pole does: no seed is to start there, and there is no
            // curve. (On an open patch it was reported as a curve from the
            // pole to itself, a few tolerances long in the chart's
            // parameters and nowhere in space; an octant against a torus
            // through its pole, tilted, at a million, then lost the arc
            // that divides the torus's face: "imprint lost".)
            let collapsed = {
                let size = Bbox3::from_points(ctrl.iter().copied()).diagonal();
                points
                    .first()
                    .is_some_and(|&first| points.iter().all(|p| p.distance(first) <= 1e-9 * size))
            };
            let curve = if collapsed {
                None
            } else {
                match NurbsCurve::new(degree, knots, points, curve_weights) {
                    Ok(curve) => Some(curve),
                    Err(_) => continue,
                }
            };
            for (from, to, worst, sine) in stretches {
                // A whole border is the whole curve, as before.
                let (from, to) = match (&curve, whole) {
                    (Some(c), true) => c.domain(),
                    _ => (from, to),
                };
                self.lying.push(Lying {
                    side,
                    from,
                    to,
                    closed: around && whole,
                    whole,
                    sine,
                    curve: curve.clone(),
                    deviation: worst.max(4.0 * self.on),
                });
            }
        }
        Ok(())
    }

    /// The sine of the angle at which the surfaces meet at the place `t`
    /// of the border on `side`, measured as the distance to the field a
    /// little way across into the chart (64 tolerances in space, or half
    /// the chart's width) over that way; `None` unless that distance is
    /// more than twice the tolerance (a sine of 1/32 or more on the 64
    /// tolerances).
    fn leaves_field(&mut self, side: usize, t: f64, tol: f64) -> Option<f64> {
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let (u, v) = match side {
            0 => (u0, t),
            1 => (u1, t),
            2 => (t, v0),
            _ => (t, v1),
        };
        let e = self.eval(u, v)?;
        let speed = if side >= 2 {
            e.sv.length()
        } else {
            e.su.length()
        };
        let width = if side >= 2 { v1 - v0 } else { u1 - u0 };
        if !(speed > 0.0 && width > 0.0) {
            return None;
        }
        let step = (64.0 * tol / speed).min(0.5 * width);
        let (u, v) = match side {
            0 => (u0 + step, t),
            1 => (u1 - step, t),
            2 => (t, v0 + step),
            _ => (t, v1 - step),
        };
        let away = self.eval(u, v)?;
        let went = away.p.distance(e.p);
        (away.dist > 2.0 * tol && went > 0.0).then(|| (away.dist / went).min(1.0))
    }

    /// The border's parameter and the distance across to it, in the
    /// chart's parameters, of `(u, v)` for a stretch on `side`.
    fn border_place(&self, side: usize, u: f64, v: f64) -> (f64, f64) {
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        match side {
            0 => (v, (u - u0).abs()),
            1 => (v, (u - u1).abs()),
            2 => (u, (v - v0).abs()),
            _ => (u, (v - v1).abs()),
        }
    }

    /// The chart's spans along a side's border and across it.
    fn border_spans(&self, side: usize) -> (f64, f64) {
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        if side >= 2 {
            (u1 - u0, v1 - v0)
        } else {
            (v1 - v0, u1 - u0)
        }
    }

    /// True when a seed sits on a stretch of a border that lies on the
    /// field: the branch it would start is the stretch, already reported.
    fn on_lying_border(&self, u: f64, v: f64) -> bool {
        self.lying.iter().any(|l| {
            let (along, across) = self.border_spans(l.side);
            let (t, off) = self.border_place(l.side, u, v);
            let (hair_along, hair_across) = (1e-9 * along, 1e-9 * across);
            off <= hair_across && (l.whole || (t >= l.from - hair_along && t <= l.to + hair_along))
        })
    }

    /// True when a point found on `f = 0` (with the chart's speeds `lu`,
    /// `lv` there) is within `reach` (a length) of a stretch of a border
    /// that lies on the field.
    fn near_lying(&self, u: f64, v: f64, lu: f64, lv: f64, reach: f64) -> bool {
        self.lying.iter().any(|l| {
            let (t, off) = self.border_place(l.side, u, v);
            let (speed_along, speed_across) = if l.side >= 2 { (lu, lv) } else { (lv, lu) };
            off * speed_across <= reach
                && (l.whole
                    || ((t - l.from) * speed_along >= -reach && (l.to - t) * speed_along >= -reach))
        })
    }

    /// True when the stationary point is within `reach` (a length) of a
    /// stretch of a border of the chart that lies on the field.
    fn near_lying_border(&self, s: &Stationary, reach: f64) -> bool {
        self.near_lying(s.u, s.v, s.lu, s.lv, reach)
    }

    /// How far beside a stretch lying on the field `f = 0` may run and be
    /// that stretch: `2 tol / sin θ`, with `sin θ` the least measured
    /// across the stretch itself ([`Lying::sine`], 1/32 at least), not at
    /// the point asked about. `None` for a stretch with no band.
    fn band(&self, l: &Lying) -> Option<f64> {
        l.sine
            .map(|sine| 2.0 * self.tol.linear / sine.clamp(1.0 / 32.0, 1.0))
    }

    /// The stretch beside which the point at `(u, v)` on `f = 0` is that
    /// stretch: within the stretch's band of it ([`Marcher::band`]), and
    /// with the field within the tolerance halfway across from it to the
    /// border (so `f` stays at zero between them: a branch of its own a
    /// little way off the stretch, a loop or a dip that comes close to it,
    /// has the field clear of zero between, and is kept). Not beside a
    /// border that is one point, and not where the chart's speed away
    /// from the border has fallen below a thousandth of its bound (near a
    /// pole every meridian comes as close to the border as to each other:
    /// a band there ended every march to the pole, a cusp's meridians in
    /// a plane through its axis, `boolean_poles.rs`).
    fn in_band(&mut self, u: f64, v: f64) -> Option<usize> {
        if self.lying.iter().all(|l| l.sine.is_none()) {
            return None;
        }
        let (_, su, sv) = self.chart.at(u, v);
        let (lu, lv) = (su.length(), sv.length());
        let b = self.chart.bounds;
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let mut found = None;
        for (k, l) in self.lying.iter().enumerate() {
            let Some(band) = self.band(l) else {
                continue;
            };
            if l.curve.is_none() {
                continue;
            }
            let (t, off) = self.border_place(l.side, u, v);
            let (speed_along, speed_across) = if l.side >= 2 { (lu, lv) } else { (lv, lu) };
            let bound_across = if l.side >= 2 { b.lv } else { b.lu };
            if speed_across > 1e-3 * bound_across
                && off * speed_across <= band
                && (t - l.from) * speed_along >= -band
                && (l.to - t) * speed_along >= -band
            {
                let t = t.clamp(l.from, l.to);
                let (bu, bv) = match l.side {
                    0 => (u0, t),
                    1 => (u1, t),
                    2 => (t, v0),
                    _ => (t, v1),
                };
                found = Some((k, (0.5 * (u + bu), 0.5 * (v + bv))));
                break;
            }
        }
        let (k, (mu, mv)) = found?;
        let tol = self.tol.linear;
        self.eval(mu, mv).filter(|e| e.dist <= tol).map(|_| k)
    }

    /// Where a step from `cur` to `next` (both on `f = 0`) comes beside a
    /// stretch lying on the field ([`Marcher::in_band`]): the branch ends
    /// there, on the border, at the stretch's parameter there (no further
    /// than its ends), since beside the stretch it is the stretch. The
    /// place is found by halving the step (each place corrected onto
    /// `f = 0`). `None` when `next` is not beside a stretch.
    fn onto_stretch(&mut self, cur: &Pt, next: &Pt) -> Option<Pt> {
        let mut which = self.in_band(next.u, next.v)?;
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let mut entry = (next.u, next.v);
        for _ in 0..BORDER_HALVINGS {
            let m = 0.5 * (lo + hi);
            let (u, v) = (cur.u + (next.u - cur.u) * m, cur.v + (next.v - cur.v) * m);
            let beside = match self.correct(u, v) {
                Some((_, cu, cv)) => self.in_band(cu, cv).map(|k| (k, cu, cv)),
                None => None,
            };
            if let Some((k, cu, cv)) = beside {
                (which, entry) = (k, (cu, cv));
                hi = m;
            } else {
                lo = m;
            }
            if (hi - lo) * cur.p.distance(next.p) <= self.on {
                break;
            }
        }
        let lying = self.lying.get(which)?;
        let (t, _) = self.border_place(lying.side, entry.0, entry.1);
        let t = t.clamp(lying.from, lying.to);
        let ((u0, u1), (v0, v1)) = (self.chart.u, self.chart.v);
        let (bu, bv) = match lying.side {
            0 => (u0, t),
            1 => (u1, t),
            2 => (t, v0),
            _ => (t, v1),
        };
        let (p, _, _) = self.chart.at(bu, bv);
        Some(Pt {
            u: bu,
            v: bv,
            p,
            ..*next
        })
    }

    /// The stretches as meeting curves, each divided where a marched branch
    /// ends on it (where the meeting leaves the border: the surfaces go on
    /// crossing past the end of what they share, or another branch meets
    /// the border there), so that curves meet only at their ends.
    fn lying_curves(&self, branches: &[Branch]) -> Vec<MeetingCurve> {
        let mut curves = Vec::new();
        for l in &self.lying {
            let Some(curve) = &l.curve else {
                continue;
            };
            let (along, across) = self.border_spans(l.side);
            let mut cuts: Vec<f64> = Vec::new();
            if !l.closed && (!l.whole || l.sine.is_some()) {
                for branch in branches.iter().filter(|b| !b.closed) {
                    for end in [branch.pts.first(), branch.pts.last()]
                        .into_iter()
                        .flatten()
                    {
                        let (t, off) = self.border_place(l.side, end.u, end.v);
                        let hair = 1e-9 * along;
                        if off <= 1e-9 * across && t > l.from + hair && t < l.to - hair {
                            cuts.push(t);
                        }
                    }
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() <= 1e-9 * along);
            let mut from = l.from;
            for to in cuts.into_iter().chain([l.to]) {
                curves.push(MeetingCurve {
                    range: (from, to),
                    curve: Curve3::Nurbs(curve.clone()),
                    closed: l.closed,
                    tangent: false,
                    exact: false,
                    deviation: l.deviation,
                });
                from = to;
            }
        }
        curves
    }

    /// The parts of the span `(lo, hi)` of grid nodes along the border on
    /// `side` that are not inside a stretch lying on the field (each from
    /// the last node in a stretch, so that a branch that leaves the border
    /// where a stretch ends is seen).
    fn off_stretches(
        &self,
        grid: &Grid,
        side: usize,
        along_u: bool,
        (lo, hi): (i64, i64),
    ) -> Vec<(i64, i64)> {
        let (origin, step) = if along_u {
            (grid.u0, grid.du)
        } else {
            (grid.v0, grid.dv)
        };
        let mut parts = vec![(lo, hi)];
        for l in self.lying.iter().filter(|l| l.side == side) {
            if !(step > 0.0) {
                break;
            }
            if l.whole {
                return Vec::new();
            }
            let first = ((l.from - origin) / step).ceil();
            let last = ((l.to - origin) / step).floor();
            if !(first.is_finite() && last.is_finite() && first <= last) {
                continue;
            }
            let (first, last) = (first as i64, last as i64);
            let mut kept = Vec::new();
            for (a, b) in parts {
                if a < first {
                    kept.push((a, b.min(first)));
                }
                if b > last {
                    kept.push((a.max(last), b));
                }
            }
            parts = kept;
        }
        parts.retain(|&(a, b)| b > a);
        parts
    }

    /// Seeds on an edge that lies on the border of the chart's rectangle,
    /// where a branch may come in and go out between two nodes.
    fn border_seeds(
        &mut self,
        grid: &Grid,
        cache: &mut BTreeMap<(i64, i64), f64>,
        along_u: bool,
        fixed: i64,
        span: (i64, i64),
        seeds: &mut Vec<(f64, f64)>,
    ) -> Result<(), GeomError> {
        let mut stack = vec![span];
        while let Some((lo, hi)) = stack.pop() {
            self.spent()?;
            let step = if along_u { grid.du } else { grid.dv };
            let half = 0.5 * (hi - lo) as f64 * step;
            let mid = 0.5 * (lo + hi) as f64;
            let (u, v) = if along_u {
                (grid.u0 + grid.du * mid, grid.v0 + grid.dv * fixed as f64)
            } else {
                (grid.u0 + grid.du * fixed as f64, grid.v0 + grid.dv * mid)
            };
            let b = if along_u {
                self.chart.bounds_in((u - half, u + half), (v, v))
            } else {
                self.chart.bounds_in((u, u), (v - half, v + half))
            };
            let (speed, bend) = if along_u {
                (b.lu, b.muu)
            } else {
                (b.lv, b.mvv)
            };
            let reach = speed * half;
            let monotonic = match self.eval(u, v) {
                // (Clear of zero by more than rounding: `clear_of_zero`.)
                Some(e) if self.clear_of_zero(e.dist, reach) => continue,
                Some(e) => self.field.bend_at(e.p, reach, e.dist).is_some_and(|k| {
                    let slope = if along_u { e.gu } else { e.gv };
                    slope.abs() > 1.01 * (k * speed * speed + bend) * half
                }),
                None => false,
            };
            if !monotonic && hi - lo > 1 {
                let middle = lo + (hi - lo) / 2;
                stack.push((middle, hi));
                stack.push((lo, middle));
                continue;
            }
            let node = |k: i64| if along_u { (k, fixed) } else { (fixed, k) };
            let (fa, fb) = (
                self.corner(grid, cache, node(lo).0, node(lo).1),
                self.corner(grid, cache, node(hi).0, node(hi).1),
            );
            if fa.is_finite() && fb.is_finite() && (fa >= 0.0) != (fb >= 0.0) {
                seeds.extend(self.edge_root(grid, along_u, fixed, (lo, hi), (fa, fb)));
            }
        }
        Ok(())
    }

    /// Seeds where a border of the field's surface crosses the chart's
    /// surface. The field goes on past a border of a freeform surface (the
    /// distance to its nearest point, on the border), and the cells see
    /// only that: a piece of the intersection between two borders, or
    /// between a border of the field and one of the chart, may lie in
    /// cells that are simple because the zero set they hold goes on past
    /// the border at both ends, where every seed on their edges is past
    /// the surface and is dropped (a lofted side cut a corner off the
    /// half of a helmet next to its vent, between the half's meridian and
    /// the vent's rim, and the piece was missing). Such a piece ends on a
    /// border of the field where that border meets the chart's surface:
    /// those points are found along each border that is an edge (not a
    /// seam or a pole), by bisection on the parameter where the distance
    /// to the chart's surface could be zero (it changes no faster than
    /// the border moves), and each gives a seed.
    fn field_border_seeds(&mut self) -> Result<Vec<(f64, f64)>, GeomError> {
        let Some(chart) = self.field_chart.as_ref() else {
            return Ok(Vec::new());
        };
        let Field::Free(own) = &self.own else {
            return Ok(Vec::new());
        };
        let (own, surface) = (std::sync::Arc::clone(own), chart.surface.clone());
        let own_field = Field::Free(std::sync::Arc::clone(&own));
        let (cu, cv, b) = (chart.u, chart.v, chart.bounds);
        let poles = crate::pole::poles(&surface);
        // (Border, parameter along it, its range, speed along it.)
        let mut borders: Vec<(bool, f64, (f64, f64), f64)> = Vec::new();
        if !chart.periodic_u {
            borders.push((true, cu.0, cv, b.lv));
            borders.push((true, cu.1, cv, b.lv));
        }
        if !chart.periodic_v {
            borders.push((false, cv.0, cu, b.lu));
            borders.push((false, cv.1, cu, b.lu));
        }
        let place = |fixed_u: bool, at: f64, t: f64| if fixed_u { (at, t) } else { (t, at) };
        // The walked surface lies in the box of its control points, and a
        // border of the field in the box of its row of them: where those
        // boxes are apart, the border crosses nothing.
        let walked = hull(&self.chart.surface);
        let mut seeds = Vec::new();
        for (fixed_u, at, range, speed) in borders {
            let at_end = if fixed_u { at >= cu.1 } else { at >= cv.1 };
            if let (Some(walked), Some(border)) = (walked, border_hull(&surface, fixed_u, at_end))
                && apart(walked, border, self.cut)
            {
                continue;
            }
            // A border that is one point is no edge.
            let (pu, pv) = place(fixed_u, at, 0.5 * (range.0 + range.1));
            if poles.iter().any(|pole| pole.holds(&surface, pu, pv)) {
                continue;
            }
            let length = speed * (range.1 - range.0);
            if !(length > 0.0 && length.is_finite()) {
                continue;
            }
            // The finest piece looked at: a few tolerances, and no more
            // than 4096 of them along the border.
            let finest = (16.0 * self.tol.linear).max(length / 4096.0);
            let signed = |t: f64| -> Option<(f64, f64, Vec3)> {
                let (u, v) = place(fixed_u, at, t);
                let p = surface.eval(u, v);
                let sample = own_field.sample(p)?;
                Some((sample.signed, sample.dist, p))
            };
            let mut stack = vec![range];
            while let Some((lo, hi)) = stack.pop() {
                self.spent()?;
                self.evals += 1;
                let half = 0.5 * (hi - lo);
                let Some((_, dist, _)) = signed(lo + half) else {
                    continue;
                };
                // Nothing of the border here comes near the chart.
                if dist > speed * half * 1.001 + self.cut {
                    continue;
                }
                if speed * half > finest {
                    stack.push((lo + half, hi));
                    stack.push((lo, lo + half));
                    continue;
                }
                let (Some((mut fa, _, _)), Some((fb, _, _))) = (signed(lo), signed(hi)) else {
                    continue;
                };
                if (fa >= 0.0) == (fb >= 0.0) {
                    continue;
                }
                let (mut a, mut c) = (lo, hi);
                for _ in 0..60 {
                    let m = 0.5 * (a + c);
                    if !(m > a && m < c) {
                        break;
                    }
                    let Some((fm, _, _)) = signed(m) else { break };
                    if (fm >= 0.0) == (fa >= 0.0) {
                        (a, fa) = (m, fm);
                    } else {
                        c = m;
                    }
                }
                let Some((_, dist, p)) = signed(0.5 * (a + c)) else {
                    continue;
                };
                // Past the chart's own edge the crossing is no point of it.
                if dist > self.cut {
                    continue;
                }
                let (u, v, _) = own.foot(p);
                seeds.push((u, v));
            }
        }
        Ok(seeds)
    }

    /// The four sides of a cell, as `(along u, fixed index, from, to)`,
    /// with the fixed index taken around a closed chart.
    fn sides(
        chart: &Chart,
        grid: &Grid,
        (i, j, size): (i64, i64, i64),
    ) -> [(bool, i64, i64, i64); 4] {
        let wrap_u = |k: i64| {
            if chart.periodic_u {
                k.rem_euclid(grid.nu)
            } else {
                k
            }
        };
        let wrap_v = |k: i64| {
            if chart.periodic_v {
                k.rem_euclid(grid.nv)
            } else {
                k
            }
        };
        [
            (true, wrap_v(j), i, i + size),
            (true, wrap_v(j + size), i, i + size),
            (false, wrap_u(i), j, j + size),
            (false, wrap_u(i + size), j, j + size),
        ]
    }

    /// Divides the chart into cells; returns the seeds on the edges of the
    /// cells that may hold a piece of the intersection, and adds the
    /// points where the surfaces are tangent and touch to `singular`.
    fn seeds(&mut self, singular: &mut Vec<Singular>) -> Result<Vec<(f64, f64)>, GeomError> {
        let grid = self.grid();
        let b = self.chart.bounds;
        let mut stack: Vec<(i64, i64, i64)> = Vec::new();
        for i in (0..grid.nu / grid.base).rev() {
            for j in (0..grid.nv / grid.base).rev() {
                stack.push((i * grid.base, j * grid.base, grid.base));
            }
        }
        let mut leaves: Vec<(i64, i64, i64)> = Vec::new();
        // Cells are looked at only where their image can be inside the
        // box: the chart of a freeform surface, a torus or a sphere is the
        // whole surface wherever the box is, and a chart cut to the box
        // (a plane's, a cylinder's, a cone's) reaches out of it at its
        // corners.
        let mut floor = 0_usize;
        let mut cells = 0_usize;
        let merge = 8.0 * self.tol.linear;
        // The size of a floor cell in space.
        let finest = 0.5 * (b.lu * grid.du + b.lv * grid.dv);
        let mut discs: Vec<Disc> = Vec::new();
        // Stationary points already looked at, in space.
        let mut looked: Vec<Vec3> = Vec::new();
        let (mut waiting, mut patience) = (0_usize, FIRST_LOOK);
        while let Some((i, j, size)) = stack.pop() {
            cells += 1;
            if cells > MAX_CELLS {
                return Err(too_many());
            }
            let (hu, hv) = (0.5 * size as f64 * grid.du, 0.5 * size as f64 * grid.dv);
            let (u, v) = (
                grid.u0 + grid.du * i as f64 + hu,
                grid.v0 + grid.dv * j as f64 + hv,
            );
            if discs.iter().any(|d| self.disc_covers(d, u, v, hu, hv)) {
                continue;
            }
            let b = self.chart.bounds_in((u - hu, u + hu), (v - hv, v + hv));
            let reach = b.lu * hu + b.lv * hv;
            let here = self.eval(u, v);
            // The image of the cell lies within `reach` of its centre's:
            // further than that outside the box, none of it is inside.
            let away = |e: &Eval| !self.within.contains(e.p, reach + self.tol.linear);
            // (Unproven: a branch that came into a kept cell from one left
            // out and went back out across the same side between two nodes
            // would show no change of sign there and be missed. No such
            // case has been found (a search of those sides as borders
            // changed nothing on 612 cases tried and was taken out), but it
            // is not ruled out: geometry.md › *Only the box is walked*.)
            if here.as_ref().is_some_and(away) {
                continue;
            }
            if self.bounded
                && let Some(e) = &here
                && self.clear_by_bound(e, u, v, hu, hv, reach, &b)
            {
                continue;
            }
            let simple = match here {
                // Clear of zero only by more than rounding
                // (`clear_of_zero`).
                Some(e) if self.clear_of_zero(e.dist, reach) => continue,
                Some(e) => self.field.bend_at(e.p, reach, e.dist).is_some_and(|k| {
                    let (muu, muv, mvv) = (
                        k * b.lu * b.lu + b.muu,
                        k * b.lu * b.lv + b.muv,
                        k * b.lv * b.lv + b.mvv,
                    );
                    e.gu.abs() > 1.01 * (muu * hu + muv * hv)
                        || e.gv.abs() > 1.01 * (muv * hu + mvv * hv)
                }),
                None => false,
            };
            if simple {
                leaves.push((i, j, size));
                continue;
            }
            // Neither empty nor simple: near a point where the surfaces are
            // tangent, or the bounds are loose here. Every floor cell
            // looks for the point; larger cells do when enough of them
            // have piled up.
            waiting += 1;
            if size == 1 || waiting >= patience {
                waiting = 0;
                let at_apex = singular.iter().any(|s| {
                    s.kind == SingularKind::Apex
                        && self.chart.surface.eval(u, v).distance(s.p) <= 2.0 * merge
                });
                let mut settled = false;
                // A touch the Hessian does not classify (along a curve, or
                // of higher order) that keeps to one side all around.
                let found = if at_apex { None } else { self.stationary(u, v) };
                // A tacnode found again (anywhere along its valley: where
                // it lies along its arms is ill-conditioned) has its disc
                // already.
                let found = found.filter(|s| !self.tac_claims(s.u, s.v));
                // A contact of higher order along one direction with arms
                // (a tacnode or a cusp); failing that, one that keeps to
                // one side (a touch).
                // (The search for it takes no step where the Hessian is
                // flat: it goes on along the valley of the strong
                // direction, and the point it finds there is used for
                // the arms only.)
                let arms = match found {
                    Some(s) if s.flat => {
                        let settled = self.settle_flat(&s).unwrap_or(s);
                        if self.touches(&settled, finest) && !self.tac_claims(settled.u, settled.v)
                        {
                            self.tacnode(&settled, finest)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                // (Judged on the valley across: the centre of a cell off a
                // contact along a curve has a gradient of the curvature
                // times its distance from it, which can exceed what reads
                // as a touch while the place across, on the curve, is one.)
                let grazing = match found {
                    Some(s) if arms.is_none() && s.flat => {
                        let s = if self.touches(&s, finest) {
                            Some(s)
                        } else {
                            self.onto_valley(&s)
                                .filter(|on| on.flat && self.touches(on, finest))
                                .filter(|on| !self.near_tacnode(on, finest))
                        };
                        s.and_then(|s| Some((s, self.one_sided(&s, finest)?)))
                    }
                    _ => None,
                };
                if let (Some(found), Some(tac)) = (found, arms) {
                    singular.push(Singular {
                        u: tac.u,
                        v: tac.v,
                        p: tac.p,
                        kind: SingularKind::Tacnode,
                        arm: 1.25 * tac.radius,
                        turn: 0.0,
                    });
                    let disc = Disc {
                        u: tac.u,
                        v: tac.v,
                        lu: found.lu,
                        lv: found.lv,
                        radius: tac.radius,
                    };
                    settled = self.disc_covers(&disc, u, v, hu, hv);
                    discs.push(disc);
                    patience = FIRST_LOOK;
                    self.tacnodes.push(tac);
                } else if let Some((found, disc)) = grazing {
                    // (On a border of the chart that lies on the other
                    // surface the border is the meeting, already reported
                    // as a curve: that the surfaces are tangent along it,
                    // a wall and its mirror image which continues it, is
                    // no point of its own.)
                    if !self.near_lying_border(&found, 8.0 * finest)
                        && !singular.iter().any(|s| s.p.distance(found.e.p) <= merge)
                    {
                        singular.push(Singular {
                            u: found.u,
                            v: found.v,
                            p: found.e.p,
                            kind: SingularKind::Touch,
                            arm: 0.0,
                            turn: 0.0,
                        });
                    }
                    settled = self.disc_covers(&disc, u, v, hu, hv);
                    discs.push(disc);
                    patience = FIRST_LOOK;
                } else if let Some(found) = found {
                    if let Some(point) = self.singular_at(&found, finest)?
                        && !singular.iter().any(|s| s.p.distance(point.p) <= merge)
                    {
                        singular.push(point);
                    }
                    if !looked.iter().any(|p| p.distance(found.e.p) <= merge) {
                        looked.push(found.e.p);
                        if let Some(disc) = self.disc(&found, finest) {
                            settled = self.disc_covers(&disc, u, v, hu, hv);
                            discs.push(disc);
                            patience = FIRST_LOOK;
                        }
                    }
                }
                if settled {
                    continue;
                }
                if size > 1 {
                    patience = patience.saturating_mul(2);
                }
            }
            if size == 1 {
                leaves.push((i, j, size));
                floor += 1;
                if floor > MAX_FLOOR_CELLS {
                    return Err(too_many());
                }
            } else {
                let h = size / 2;
                for (di, dj) in [(h, h), (0, h), (h, 0), (0, 0)] {
                    stack.push((i + di, j + dj, h));
                }
            }
        }
        // A touch found on the valley of a tacnode before the tacnode
        // itself was is a place along its arms, not a point of its own.
        singular.retain(|s| s.kind != SingularKind::Touch || !self.tac_claims(s.u, s.v));
        // Every edge of every leaf, once.
        let mut edges: BTreeSet<(bool, i64, i64, i64)> = BTreeSet::new();
        for &leaf in &leaves {
            edges.extend(Self::sides(&self.chart, &grid, leaf));
        }
        let mut cache = BTreeMap::new();
        let mut seeds = Vec::new();
        for (along_u, fixed, lo, hi) in edges {
            self.spent()?;
            let border = if along_u {
                !self.chart.periodic_v && (fixed == 0 || fixed == grid.nv)
            } else {
                !self.chart.periodic_u && (fixed == 0 || fixed == grid.nu)
            };
            if border {
                // A border that lies on the field is a curve already.
                let side = match (along_u, fixed == 0) {
                    (true, true) => 2,
                    (true, false) => 3,
                    (false, true) => 0,
                    (false, false) => 1,
                };
                for span in self.off_stretches(&grid, side, along_u, (lo, hi)) {
                    self.border_seeds(&grid, &mut cache, along_u, fixed, span, &mut seeds)?;
                }
                continue;
            }
            let node = |k: i64| if along_u { (k, fixed) } else { (fixed, k) };
            let (fa, fb) = (
                self.corner(&grid, &mut cache, node(lo).0, node(lo).1),
                self.corner(&grid, &mut cache, node(hi).0, node(hi).1),
            );
            if fa.is_finite() && fb.is_finite() && (fa >= 0.0) != (fb >= 0.0) {
                seeds.extend(self.edge_root(&grid, along_u, fixed, (lo, hi), (fa, fb)));
            }
        }
        Ok(seeds)
    }

    // ----- singular points ---------------------------------------------------

    /// The Hessian of `f` in parameters, by central differences of its
    /// gradient over about `1e-5` of the box.
    fn hessian(&mut self, u: f64, v: f64) -> Option<(f64, f64, f64)> {
        let b = self.chart.bounds;
        let reach = 3.2e-4 * self.hmax;
        let eu = if b.lu > 0.0 {
            reach / b.lu
        } else {
            1e-6 * (self.chart.u.1 - self.chart.u.0)
        };
        let ev = if b.lv > 0.0 {
            reach / b.lv
        } else {
            1e-6 * (self.chart.v.1 - self.chart.v.0)
        };
        let (a, b_, c, d) = (
            self.eval(u + eu, v)?,
            self.eval(u - eu, v)?,
            self.eval(u, v + ev)?,
            self.eval(u, v - ev)?,
        );
        let fuu = (a.gu - b_.gu) / (2.0 * eu);
        let fvv = (c.gv - d.gv) / (2.0 * ev);
        let fuv = 0.5 * ((a.gv - b_.gv) / (2.0 * eu) + (c.gu - d.gu) / (2.0 * ev));
        (fuu.is_finite() && fuv.is_finite() && fvv.is_finite()).then_some((fuu, fuv, fvv))
    }

    /// Newton on the gradient of `f`: the point near `(u0, v0)` where the
    /// surfaces are tangent (their normals parallel), wherever the other
    /// surface is.
    fn stationary(&mut self, u0: f64, v0: f64) -> Option<Stationary> {
        let (mut u, mut v) = (u0, v0);
        let start = self.eval(u0, v0)?;
        for _ in 0..30 {
            let e = self.eval(u, v)?;
            let (a, b, c) = self.hessian(u, v)?;
            let (lu, lv) = (e.su.length(), e.sv.length());
            if !(lu > 0.0 && lv > 0.0) {
                return None;
            }
            // In units of length: the Hessian of f along the surface.
            let (ha, hb, hc) = (a / (lu * lu), b / (lu * lv), c / (lv * lv));
            let det = ha * hc - hb * hb;
            let size = ha.abs().max(hb.abs()).max(hc.abs());
            let flat = !(det.abs() > 1e-3 * size * size);
            let (gu, gv) = (e.gu / lu, e.gv / lv);
            let (step_u, step_v) = if flat {
                (0.0, 0.0)
            } else {
                ((hb * gv - hc * gu) / det, (hb * gu - ha * gv) / det)
            };
            let moved = step_u.phypot(step_v);
            if flat || moved <= self.on {
                // Converged (or no better step exists).
                let found = Stationary {
                    u,
                    v,
                    e,
                    lu,
                    lv,
                    g: (gu, gv),
                    h: (ha, hb, hc),
                    flat,
                };
                return Some(found);
            }
            u += step_u / lu;
            v += step_v / lv;
            if !(u.is_finite() && v.is_finite()) || e.p.distance(start.p) > 64.0 * self.hmax {
                return None;
            }
        }
        None
    }

    /// True when a disc covers the cell around `(u, v)`, the cell taken
    /// round a periodic chart to the disc's side.
    fn disc_covers(&self, d: &Disc, u: f64, v: f64, hu: f64, hv: f64) -> bool {
        let c = &self.chart;
        let u = Self::unwrap(u, d.u, c.periodic_u, c.u.1 - c.u.0);
        let v = Self::unwrap(v, d.v, c.periodic_v, c.v.1 - c.v.0);
        d.covers(u, v, hu, hv)
    }

    /// The gradient and Hessian of `f` at `(u, v)`, as a stationary point.
    fn stationary_at(&mut self, u: f64, v: f64) -> Option<Stationary> {
        let e = self.eval(u, v)?;
        let (a, b, c) = self.hessian(u, v)?;
        let (lu, lv) = (e.su.length(), e.sv.length());
        if !(lu > 0.0 && lv > 0.0) {
            return None;
        }
        let (ha, hb, hc) = (a / (lu * lu), b / (lu * lv), c / (lv * lv));
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let flat = !((ha * hc - hb * hb).abs() > 1e-3 * size * size);
        Some(Stationary {
            u,
            v,
            e,
            lu,
            lv,
            g: (e.gu / lu, e.gv / lv),
            h: (ha, hb, hc),
            flat,
        })
    }

    /// True when the surfaces touch at the stationary point: nearer to
    /// each other than the tolerance and than a floor cell reaches
    /// (`finest`), which is where the cells alone would have come down to
    /// the floor. Further apart the zero set of `f` keeps clear of the
    /// point (a saddle one tolerance off the other surface has its two
    /// branches thousands of tolerances apart), and cells find it.
    fn touches(&self, s: &Stationary, finest: f64) -> bool {
        let (ha, hb, hc) = s.h;
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let tangent = s.g.0.phypot(s.g.1) <= 1e-6_f64.max(TOUCH * size * self.tol.linear);
        tangent && s.e.dist <= self.tol.linear.min(finest) && self.inside(s.u, s.v)
    }

    /// The stationary point as a point of the intersection, if the
    /// surfaces touch there.
    fn singular_at(&self, s: &Stationary, finest: f64) -> Result<Option<Singular>, GeomError> {
        let (ha, hb, hc) = s.h;
        if !self.touches(s, finest) {
            return Ok(None);
        }
        if s.flat {
            return Err(GeomError::Unsupported(
                "surfaces that touch along a curve, or with more than second-order contact, without a closed form",
            ));
        }
        // Where the surfaces are a little apart at a crossing (within the
        // tolerance, so it is read as one), `f = 0` is two branches that
        // turn away at `sqrt(2 |f| / λ)` from the point, `λ` the weaker
        // eigenvalue of the Hessian: the arms start four times as far out,
        // where the branches run along the directions of the crossing
        // within a thirtieth and a march towards the point ends on it.
        let mean = 0.5 * (ha + hc);
        let weaker = (mean.abs() - (0.25 * (ha - hc) * (ha - hc) + hb * hb).sqrt()).abs();
        let turn = if weaker > 0.0 {
            (2.0 * s.e.f.abs() / weaker).sqrt()
        } else {
            0.0
        };
        Ok(Some(Singular {
            u: s.u,
            v: s.v,
            p: s.e.p,
            kind: if ha * hc - hb * hb > 0.0 {
                SingularKind::Touch
            } else {
                SingularKind::Cross
            },
            arm: self.arm_radius().max(4.0 * turn),
            turn,
        }))
    }

    /// A disc around a point where the surfaces touch and the Hessian of
    /// `f` is degenerate (they touch along a curve through it, or to more
    /// than second order), in which `f` keeps to one side: on three rings
    /// of samples it is nowhere on the other side by more than a quarter
    /// of the tolerance, and on the outer ring mostly clear of zero. The
    /// surfaces then touch there without crossing, which divides nothing:
    /// the point is a touch, and the disc needs no cells. (Going through
    /// each other by less than half the tolerance is a touch to a boolean
    /// anyway.) `None` where `f` changes sign on every ring down to the
    /// smallest: a crossing, which stays refused.
    ///
    /// The rings are a sample, and the radius shrinks until they agree: a
    /// crossing in a wedge narrower than the gaps between the samples (a
    /// cusp, `x² − y³`) passes at a small radius. Nothing is lost by it,
    /// because a disc only keeps cells from being split: the arms of such
    /// a crossing leave the disc, are found outside it and are marched to
    /// the point (`a_degenerate_contact_that_crosses_in_a_cusp_is_not_a_touch`).
    /// What a disc could hide is a closed curve wholly inside it that
    /// every one of its 48 samples misses.
    fn one_sided(&mut self, s: &Stationary, finest: f64) -> Option<Disc> {
        let c = &self.chart;
        let widest = 0.25 * ((c.u.1 - c.u.0) * s.lu).min((c.v.1 - c.v.0) * s.lv);
        let least = 8.0 * finest;
        let slack = self.cut;
        let mut radius = widest;
        'radius: while radius.is_finite() && radius >= least {
            let (mut above, mut below, mut beyond, mut seen) = (0_u32, 0_u32, 0_u32, 0_u32);
            for (ring, count) in [(1.0, 24_u32), (2.0 / 3.0, 16), (1.0 / 3.0, 8)] {
                for k in 0..count {
                    self.spent().ok()?;
                    let angle =
                        std::f64::consts::TAU * (f64::from(k) + 0.5 * ring) / f64::from(count);
                    let (x, y) = (ring * radius * angle.pcos(), ring * radius * angle.psin());
                    let (u, v) = (s.u + x / s.lu, s.v + y / s.lv);
                    if !self.inside(u, v) {
                        continue; // No cell lies there.
                    }
                    let Some(e) = self.eval(u, v) else {
                        radius *= 0.5;
                        continue 'radius;
                    };
                    seen += u32::from(ring == 1.0);
                    // Past the edge of the other surface the distance says
                    // nothing about sides: nothing of that surface is
                    // there, on either side. (All around a point of a
                    // border the two surfaces share, where one continues
                    // the other.)
                    if e.dist > e.f.abs() + self.on {
                        beyond += u32::from(e.dist > slack);
                        continue;
                    }
                    above += u32::from(e.f > slack);
                    below += u32::from(e.f < -slack);
                }
            }
            // One side only, and on the rim mostly away from the other
            // surface (not two surfaces that coincide there).
            let (most, least_side) = (above.max(below) + beyond, above.min(below));
            if least_side == 0 && seen > 0 && 2 * most >= seen {
                return Some(Disc {
                    u: s.u,
                    v: s.v,
                    lu: s.lu,
                    lv: s.lv,
                    radius: 0.9 * radius,
                });
            }
            radius *= 0.5;
        }
        None
    }

    /// The largest disc around a stationary point on whose rim, and
    /// halfway to it, the gradient of `f` is what the Hessian at the point
    /// says, within [`DISC_MODEL`]: `f` then has no other critical point
    /// in it, and its zero set there is the point, the two branches
    /// through it, or nothing. `None` where the zero set is not one of
    /// those (a critical point off the other surface around which `f`
    /// changes sign), or where no disc of at least a few floor cells
    /// (`finest` is the size of one) passes.
    fn disc(&mut self, s: &Stationary, finest: f64) -> Option<Disc> {
        let (ha, hb, hc) = s.h;
        let size = ha.abs().max(hb.abs()).max(hc.abs());
        let tangent = s.g.0.phypot(s.g.1) <= 1e-6_f64.max(TOUCH * size * self.tol.linear);
        if s.flat || !tangent || !self.inside(s.u, s.v) {
            return None;
        }
        // Off the other surface: only a minimum above it or a maximum
        // below it leaves the disc without a zero. (Past the edge of a
        // bounded surface the distance is to that edge, not a smooth
        // function of the place.)
        let touching = self.touches(s, finest);
        let on = self.on;
        let smooth = move |e: &Eval| e.dist <= e.f.abs() + on;
        if !touching && !(ha * hc - hb * hb > 0.0 && s.e.f * ha > 0.0 && smooth(&s.e)) {
            return None;
        }
        let c = &self.chart;
        let widest = 0.25 * ((c.u.1 - c.u.0) * s.lu).min((c.v.1 - c.v.0) * s.lv);
        let least = 8.0 * finest;
        let mut radius = widest;
        'radius: while radius.is_finite() && radius >= least {
            for (ring, count) in [(1.0, 16_u32), (0.5, 8)] {
                for k in 0..count {
                    self.spent().ok()?;
                    // The inner ring sits between the outer one's places.
                    let angle =
                        std::f64::consts::TAU * (f64::from(k) + 0.5 * ring) / f64::from(count);
                    let (x, y) = (ring * radius * angle.pcos(), ring * radius * angle.psin());
                    let (u, v) = (s.u + x / s.lu, s.v + y / s.lv);
                    if !self.inside(u, v) {
                        continue; // No cell lies there.
                    }
                    let model = (ha * x + hb * y, hb * x + hc * y);
                    let holds = self.eval(u, v).is_some_and(|e| {
                        let off = (e.gu / s.lu - model.0).phypot(e.gv / s.lv - model.1);
                        off <= DISC_MODEL * model.0.phypot(model.1)
                            && (touching || (e.f * s.e.f > 0.0 && smooth(&e)))
                    });
                    if !holds {
                        radius *= 0.5;
                        continue 'radius;
                    }
                }
            }
            return Some(Disc {
                u: s.u,
                v: s.v,
                lu: s.lu,
                lv: s.lv,
                radius: 0.9 * radius,
            });
        }
        None
    }

    /// How far from a crossing point its arms are seeded, at the least.
    fn arm_radius(&self) -> f64 {
        (64.0 * self.tol.linear).max(1e-2 * self.hmax)
    }

    /// Seeds on the four arms that leave a crossing point: along the
    /// directions in which the Hessian vanishes. Returns those directions
    /// in space (none where the point cannot be evaluated).
    fn arms(&mut self, s: &Singular, seeds: &mut Vec<(f64, f64)>) -> Vec<Vec3> {
        let mut found = Vec::new();
        let (Some(e), Some((a, b, c))) = (self.eval(s.u, s.v), self.hessian(s.u, s.v)) else {
            return found;
        };
        let root = (b * b - a * c).max(0.0).sqrt();
        let directions = if a.abs() >= c.abs() && a != 0.0 {
            [((-b + root) / a, 1.0), ((-b - root) / a, 1.0)]
        } else if c != 0.0 {
            [(1.0, (-b + root) / c), (1.0, (-b - root) / c)]
        } else {
            [(1.0, 0.0), (0.0, 1.0)]
        };
        let radius = s.arm;
        for (du, dv) in directions {
            let along = e.su * du + e.sv * dv;
            let len = along.length();
            if !(len > 0.0) {
                continue;
            }
            for side in [1.0, -1.0] {
                found.push(along * (side / len));
                let k = side * radius / len;
                if let Some((_, u, v)) = self.correct(s.u + du * k, s.v + dv * k) {
                    seeds.push((u, v));
                }
            }
        }
        found
    }

    /// The arm of a zone that a place lies along, by its direction from
    /// the zone's point: the arm nearest by angle, where that angle is
    /// less than a quarter of the angle between the arm and the next one
    /// (nearer the bisector, or the point itself, no arm is told).
    fn arm_of(zone: &Zone, q: Vec3) -> Option<usize> {
        let d = (q - zone.p).normalized()?;
        let mut best: Option<(usize, f64)> = None;
        for (i, a) in zone.arms.iter().enumerate() {
            let c = a.dot(d);
            if best.is_none_or(|(_, b)| c > b) {
                best = Some((i, c));
            }
        }
        let (i, c) = best?;
        let a = zone.arms.get(i)?;
        let next = zone
            .arms
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, b)| a.dot(*b))
            .fold(-1.0_f64, f64::max);
        let angle = c.clamp(-1.0, 1.0).pacos();
        let gap = next.clamp(-1.0, 1.0).pacos();
        (angle < 0.25 * gap).then_some(i)
    }

    /// The arms along which the points of a branch (or of a run, from the
    /// point it went on from) come into the point of a zone: (zone, arm)
    /// for each end on such a point, told by the point next to it.
    fn arms_into_zones(&self, pts: &[Pt]) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        let before_last = pts.len().checked_sub(2).and_then(|i| pts.get(i));
        for (z, zone) in self.zones.iter().enumerate() {
            for (end, next) in [(pts.first(), pts.get(1)), (pts.last(), before_last)] {
                if let (Some(end), Some(next)) = (end, next)
                    && end.p.distance(zone.p) <= self.on
                    && let Some(j) = Self::arm_of(zone, next.p)
                {
                    found.push((z, j));
                }
            }
        }
        found
    }

    /// Whether a place inside the zone of a crossing point lies along an
    /// arm that a branch traced already comes into the point along. Inside
    /// the zone a branch is drawn straight to the point, and `f = 0`, which
    /// turns away from it, lies beside what was drawn by about `turn² / d`
    /// at `d` from it: a few tolerances, more than a seed may be off a
    /// branch ([`Marcher::on_branch`]). Such a seed traced again gave the
    /// stretch of that branch twice (docs/design/booleans.md › *A stretch
    /// given twice beside a crossing point the surfaces are a little apart
    /// at*).
    fn along_traced(&self, q: Vec3) -> bool {
        self.zones.iter().enumerate().any(|(z, zone)| {
            let d = q.distance(zone.p);
            d <= zone.radius
                && d > self.on
                && Self::arm_of(zone, q).is_some_and(|j| self.traced_into.contains(&(z, j)))
        })
    }

    /// `march` with the arms along which `other` (the points of the run
    /// the other way from `start`) comes into the point of a zone counted
    /// as traced ([`Marcher::passed`]).
    fn beside<T>(&mut self, start: &Pt, other: &[Pt], march: impl FnOnce(&mut Self) -> T) -> T {
        let before = self.traced_into.len();
        if !self.zones.is_empty() {
            let mut run = Vec::with_capacity(other.len() + 1);
            run.push(*start);
            run.extend_from_slice(other);
            let more = self.arms_into_zones(&run);
            self.traced_into.extend(more);
        }
        let out = march(self);
        self.traced_into.truncate(before);
        out
    }

    /// Notes a run at `cur` (after `kept` points) that is inside the zone
    /// of a crossing point and heading for it: the zone, and the points it
    /// would keep were it to end at the point ([`Marcher::passed`]): up to
    /// the last one at least `near` from it, where its direction tells its
    /// arm.
    fn heading_in(&self, passing: &mut Option<(usize, usize)>, cur: &Pt, kept: usize) {
        for (z, zone) in self.zones.iter().enumerate() {
            let d = zone.p - cur.p;
            let dist = d.length();
            if !(dist <= zone.radius && dist > self.on && d.dot(cur.t) > 0.0) {
                continue;
            }
            match *passing {
                None => *passing = Some((z, kept)),
                Some((w, _)) if w == z && dist >= zone.near => *passing = Some((z, kept)),
                Some(_) => {}
            }
            return;
        }
    }

    /// A run that headed for the point of a zone and did not end there
    /// (where the surfaces are a little apart at a crossing point, a run
    /// that comes within a few `turn` of it before it looks for the point
    /// ahead cannot see it straight ahead, and follows `f = 0` round the
    /// point onto the next arm): once it heads away from the point, at
    /// least `near` from it, the arm it goes out along. Where a branch
    /// traced already, or the trace's run the other way, comes into the
    /// point along that arm, the run would trace that stretch again: it is
    /// to end at the point, keeping what [`Marcher::heading_in`] noted
    /// (`Some((zone, kept))`). Out along an arm not traced, it goes on as
    /// it did.
    fn passed(&self, passing: &mut Option<(usize, usize)>, cur: &Pt) -> Option<(usize, usize)> {
        let (z, keep) = (*passing)?;
        let zone = self.zones.get(z)?;
        let d = zone.p - cur.p;
        if d.dot(cur.t) > 0.0 || d.length() < zone.near {
            return None;
        }
        *passing = None;
        let j = Self::arm_of(zone, cur.p)?;
        self.traced_into.contains(&(z, j)).then_some((z, keep))
    }

    /// The points of a run that ends at the point of zone `z`: its first
    /// `keep`, then the point, drawn straight from the last of them (or
    /// from `first`, the point the run went on from), as a run that finds
    /// a crossing point straight ahead ends there.
    fn ended_at(&self, mut pts: Vec<Pt>, keep: usize, z: usize, first: &Pt) -> Vec<Pt> {
        pts.truncate(keep);
        let Some(zone) = self.zones.get(z) else {
            return pts;
        };
        let last = pts.last().copied().unwrap_or(*first);
        let d = zone.p - last.p;
        let dist = d.length();
        if dist > 0.0 {
            let (u, v) = self.near((zone.u, zone.v), &last);
            pts.push(Pt {
                u,
                v,
                p: zone.p,
                t: d * (1.0 / dist),
                du: 0.0,
                dv: 0.0,
            });
        }
        pts
    }

    // ----- marching ------------------------------------------------------------

    /// `x` moved by whole periods to be nearest to `near`.
    fn unwrap(x: f64, near: f64, periodic: bool, period: f64) -> f64 {
        if periodic {
            x + ((near - x) / period).round() * period
        } else {
            x
        }
    }

    fn near(&self, (u, v): (f64, f64), to: &Pt) -> (f64, f64) {
        let c = &self.chart;
        (
            Self::unwrap(u, to.u, c.periodic_u, c.u.1 - c.u.0),
            Self::unwrap(v, to.v, c.periodic_v, c.v.1 - c.v.0),
        )
    }

    /// Where the step from `cur` to `(u, v)` leaves the chart's rectangle.
    fn exit(&mut self, cur: &Pt, u: f64, v: f64, sign: f64) -> Option<Pt> {
        let c = &self.chart;
        let mut bounds: Vec<(f64, bool, f64)> = Vec::new(); // (fraction, on u, bound)
        if !c.periodic_u {
            for bound in [c.u.0, c.u.1] {
                if (u - bound) * (cur.u - bound) < 0.0 {
                    bounds.push(((bound - cur.u) / (u - cur.u), true, bound));
                }
            }
        }
        if !c.periodic_v {
            for bound in [c.v.0, c.v.1] {
                if (v - bound) * (cur.v - bound) < 0.0 {
                    bounds.push(((bound - cur.v) / (v - cur.v), false, bound));
                }
            }
        }
        bounds.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (range_u, range_v) = (c.u, c.v);
        let (periodic_u, periodic_v) = (c.periodic_u, c.periodic_v);
        for (fraction, on_u, bound) in bounds {
            // Newton along the border from the straight guess.
            let mut y = if on_u {
                cur.v + fraction * (v - cur.v)
            } else {
                cur.u + fraction * (u - cur.u)
            };
            for _ in 0..24 {
                let (pu, pv) = if on_u { (bound, y) } else { (y, bound) };
                let Some(e) = self.eval(pu, pv) else { break };
                if e.f.abs() <= self.on {
                    let slack_u = 1e-9 * (range_u.1 - range_u.0);
                    let slack_v = 1e-9 * (range_v.1 - range_v.0);
                    // The other parameter stays in its range, unless it
                    // goes round.
                    let ok = if on_u {
                        periodic_v || (y >= range_v.0 - slack_v && y <= range_v.1 + slack_v)
                    } else {
                        periodic_u || (y >= range_u.0 - slack_u && y <= range_u.1 + slack_u)
                    };
                    if !ok || e.dist > self.cut {
                        break;
                    }
                    let chord = (e.p - cur.p).normalized();
                    return Self::point(&e, pu, pv, sign).or_else(|| {
                        chord.map(|t| Pt {
                            u: pu,
                            v: pv,
                            p: e.p,
                            t,
                            du: 0.0,
                            dv: 0.0,
                        })
                    });
                }
                let slope = if on_u { e.gv } else { e.gu };
                if slope == 0.0 {
                    break;
                }
                y -= e.f / slope;
                if !y.is_finite() {
                    break;
                }
            }
        }
        None
    }

    /// Follows a branch from `start` in the direction of its tangent.
    fn run(
        &mut self,
        start: &Pt,
        sign: f64,
        singular: &[Singular],
        may_close: bool,
    ) -> Result<(Vec<Pt>, End), GeomError> {
        self.run_on(start, sign, singular, may_close, None)
    }

    /// [`Marcher::run`], or the rest of such a run taken up again from a
    /// point of it (`again`): on from there, with steps no longer than
    /// its cap until it has gone as far as the cap is held over, and as
    /// any run from there on. The points come without the one it goes on
    /// from, as a run's come without its start; it closes on `start`, the
    /// run's own.
    fn run_on(
        &mut self,
        start: &Pt,
        sign: f64,
        singular: &[Singular],
        may_close: bool,
        again: Option<Again>,
    ) -> Result<(Vec<Pt>, End), GeomError> {
        let mut pts: Vec<Pt> = Vec::new();
        let mut cur = again.map_or(*start, |a| a.from);
        let first = cur;
        let taken = again.map_or(0, |a| a.taken);
        let mut h = again.map_or(0.25 * self.hmax, |a| a.cap.min(0.25 * self.hmax));
        // How far this run has gone since it was taken up.
        let mut gone = 0.0_f64;
        let cos_turn = MAX_TURN.pcos();
        let cos_half = (0.5 * MAX_TURN).pcos();
        let hmin = self.hmin;
        // The zone of a crossing point the run is heading for, and what it
        // keeps if it is to end there (`heading_in`, `passed`).
        let mut passing: Option<(usize, usize)> = None;
        loop {
            self.spent()?;
            if pts.len() + taken > MAX_POINTS {
                return Err(GeomError::NoConvergence(
                    "an intersection curve needs more points than the cap allows",
                ));
            }
            self.heading_in(&mut passing, &cur, pts.len());
            // A crossing point or an apex straight ahead ends the branch.
            let mut snapped = None;
            for s in singular {
                if s.kind == SingularKind::Touch {
                    continue;
                }
                let d = s.p - cur.p;
                let dist = d.length();
                let along = d.dot(cur.t);
                // A crossing point is looked for two steps ahead, and from
                // the radius at which its arms are seeded: where the
                // surfaces are a little apart at it the branch turns away
                // just before the point, and a step that lands there would
                // pass it by and go on along the next arm; where they are
                // not, a step that lands on the point itself would.
                let reach = match s.kind {
                    SingularKind::Cross => (2.0 * h).max(s.arm),
                    SingularKind::Tacnode => s.arm,
                    _ => h,
                };
                if !(dist > self.on && dist <= reach && along > 0.0) {
                    continue;
                }
                let (su, sv) = self.near((s.u, s.v), &cur);
                // Inside a tacnode's radius its arms are made, not marched
                // (`tacnode.rs`): every branch heading in ends on it.
                if s.kind == SingularKind::Tacnode {
                    // Only a branch on its arms (`tac_admits`); one that
                    // passes near it goes on.
                    if !self.tac_admits(s.p, cur.u, cur.v) {
                        continue;
                    }
                    // With the direction along: the arms' tangent there.
                    snapped = Some(self.tac_arrival(s.p, (su, sv), d).ok_or(GeomError::NoConvergence(
                        "an arm that ends on a point of higher-order contact has no direction there",
                    ))?);
                    break;
                }
                if (d - cur.t * along).length() > 0.1 * dist {
                    continue;
                }
                // The curve really goes there: its middle is near the chord.
                let Some((m, _, _)) = self.correct(0.5 * (cur.u + su), 0.5 * (cur.v + sv)) else {
                    continue;
                };
                let off = m.p - cur.p;
                let t = off.dot(d) / (dist * dist);
                if t > 0.0 && t < 1.0 && (off - d * t).length() <= 0.2 * dist {
                    snapped = Some(Pt {
                        u: su,
                        v: sv,
                        p: s.p,
                        t: d * (1.0 / dist),
                        du: 0.0,
                        dv: 0.0,
                    });
                    break;
                }
            }
            if let Some(end) = snapped {
                pts.push(end);
                return Ok((pts, End::Open));
            }
            // Near a tacnode the other arm is close: short steps.
            h = self.tac_step(cur.p, h).max(hmin);
            let (pu, pv) = (cur.u + h * cur.du, cur.v + h * cur.dv);
            let Some((e, u, v)) = self.correct(pu, pv) else {
                // Past the border of the chart there may be nothing to
                // correct onto: end on the border if the curve reaches it.
                if !self.inside(pu, pv) {
                    if let Some(end) = self.exit(&cur, pu, pv, sign) {
                        pts.push(end);
                        return Ok((pts, End::Open));
                    }
                    if self.on_border(&cur) {
                        return Ok((pts, End::Open));
                    }
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(e) => return Err(e),
                };
                continue;
            };
            let step = e.p - cur.p;
            let len = step.length();
            let new = Self::point(&e, u, v, sign);
            let turn = new.map_or(-1.0, |n| n.t.dot(cur.t));
            // A step goes about as far as it was asked (`h` along the
            // tangent, to first order): one that ends less than half or
            // more than twice as far went somewhere the prediction did
            // not, such as once around a closed curve smaller than the
            // step, back to just past where it began (the tangent there
            // is the same, so the turn does not show it).
            // And it ends on the curve: a point within `on` of the field
            // that could not be brought as near as it is held where the
            // surfaces cross at a shallow angle ([`Marcher::held`]) is
            // anywhere across the band about the curve (past the turn of
            // a hairpin there is no curve to come onto): a shorter step
            // lands where there is.
            // (And at a place that can be placed at all: `placed`.)
            let held = e.f.abs() <= self.held(&e);
            let on_curve = held && self.placed(&e);
            let Some(new) = new.filter(|_| {
                step.dot(cur.t) > 0.0
                    && len >= 0.5 * h
                    && len <= 2.0 * h
                    && turn >= cos_turn
                    && on_curve
            }) else {
                // Off the edge of the other surface the field has a crease
                // and no tangent: that is an end, not a stall.
                if e.dist > self.cut {
                    pts.extend(self.edge(&cur, h, sign));
                    return Ok((pts, End::Open));
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(err) => {
                        // Where the speed of the parametrisation jumps (a
                        // knot of a spline whose spans are drawn at other
                        // paces: rational weights far apart) the step
                        // predicted from the derivatives on one side goes
                        // as far off however short it is. The parameter
                        // step is then scaled to the length it went.
                        // (Only a step that ended on the curve and went too
                        // far or too short: one that ended off it is no
                        // better for another length, and taken here the
                        // march went on from places off the curve, a
                        // least step at a time, thousands of them, to the
                        // same stall.)
                        let ahead = step.dot(cur.t) > 0.0 && turn >= cos_turn && on_curve;
                        let Some(new) = ahead
                            .then(|| self.rescaled(&cur, h, len, sign, cos_turn))
                            .flatten()
                        else {
                            // Brought as near as it is held, and still
                            // not placed: the reason is the angle.
                            return Err(if held && !on_curve { unplaced() } else { err });
                        };
                        // A loop that closes within the step closes here
                        // too, as after any other step.
                        let went = new.p.distance(cur.p);
                        if may_close && pts.len() + taken >= 2 && self.closes_on(start, &cur, went)
                        {
                            return Ok((pts, End::Closed));
                        }
                        if let Some(end) = self.onto_stretch(&cur, &new) {
                            pts.push(end);
                            return Ok((pts, End::Open));
                        }
                        pts.push(new);
                        cur = new;
                        gone += went;
                        if let Some((z, keep)) = self.passed(&mut passing, &cur) {
                            return Ok((self.ended_at(pts, keep, z, &first), End::Open));
                        }
                        continue;
                    }
                };
                continue;
            };
            if may_close && pts.len() + taken >= 2 && self.closes_on(start, &cur, len) {
                return Ok((pts, End::Closed));
            }
            if e.dist > self.cut {
                pts.extend(self.edge(&cur, h, sign));
                return Ok((pts, End::Open));
            }
            // (Before the border: a step along a stretch lying on the
            // field lands on the border by rounding as often as not.)
            if let Some(end) = self.onto_stretch(&cur, &new) {
                pts.push(end);
                return Ok((pts, End::Open));
            }
            if !self.inside(u, v) {
                // The border is closer than this step: land on it, or come
                // closer and try again.
                if let Some(end) = self.exit(&cur, u, v, sign) {
                    pts.push(end);
                    return Ok((pts, End::Open));
                }
                if self.on_border(&cur) {
                    return Ok((pts, End::Open)); // It started on the border.
                }
                h = match halved(h, hmin) {
                    Ok(h) => h,
                    Err(_) if !self.within.contains(cur.p, 0.0) => return Ok((pts, End::Open)),
                    Err(e) => return Err(e),
                };
                continue;
            }
            pts.push(new);
            cur = new;
            gone += len;
            // Round a crossing point onto an arm traced already: the run
            // ends at the point instead (`passed`).
            if let Some((z, keep)) = self.passed(&mut passing, &cur) {
                return Ok((self.ended_at(pts, keep, z, &first), End::Open));
            }
            if turn >= cos_half {
                // (A run taken up again: no longer than its cap while it
                // has not gone as far as the cap is held over.)
                let most = match again {
                    Some(a) if gone < a.over => a.cap.min(self.hmax),
                    _ => self.hmax,
                };
                h = (1.5 * h).min(most);
            }
        }
    }

    /// Whether a step of length `len` from `cur` comes back to `start`: the
    /// start is ahead within the step, near its line, heading the same way.
    ///
    /// Near its line by a fifth of the step and by what the two points
    /// may be off the curve themselves, each [`ACROSS`] of the tolerance
    /// ([`Marcher::placed`]). Without that room a march passed its own
    /// start where its steps were short and its points loose (the tip of
    /// a thin loop, where the surfaces cross at their shallowest and the
    /// curve turns in a few tens of tolerances), went round a second time
    /// and closed there: one closed curve twice round the loop, which a
    /// boolean reads as nothing in common (docs/design/booleans.md › *A
    /// surface far larger than the box*).
    fn closes_on(&self, start: &Pt, cur: &Pt, len: f64) -> bool {
        let back = start.p - cur.p;
        let s = back.dot(cur.t);
        s > 0.0
            && s <= len * (1.0 + 1e-9)
            && (back - cur.t * s).length() <= 0.2 * len + self.on + 2.0 * ACROSS * self.tol.linear
            && start.t.dot(cur.t) > 0.5
    }

    /// A step of about `h` from `cur` where the step predicted along the
    /// parameters' tangent went `len` instead, however short it was made:
    /// the parameter step scaled by the length it went (a secant on its
    /// length), and once one went too short and one too far, halved
    /// between them (the length grows with the scale, at another rate on
    /// either side of a jump). `None` unless one lands as a step must
    /// (about `h` ahead, turning little, on the field and in the chart).
    fn rescaled(&mut self, cur: &Pt, h: f64, len: f64, sign: f64, cos_turn: f64) -> Option<Pt> {
        let (mut scale, mut went) = (h, len);
        let (mut short, mut far): (Option<f64>, Option<f64>) = (None, None);
        for _ in 0..RESCALES {
            if !(went > 0.0 && went.is_finite()) {
                return None;
            }
            if went < 0.5 * h {
                short = Some(scale);
            } else {
                far = Some(scale);
            }
            scale = match (short, far) {
                (Some(a), Some(b)) => 0.5 * (a + b),
                _ => scale * h / went,
            };
            if !(scale > 0.0 && scale.is_finite()) {
                return None;
            }
            let (e, u, v) = self.correct(cur.u + scale * cur.du, cur.v + scale * cur.dv)?;
            let step = e.p - cur.p;
            went = step.length();
            let new = Self::point(&e, u, v, sign)?;
            if !(step.dot(cur.t) > 0.0 && new.t.dot(cur.t) >= cos_turn) {
                return None;
            }
            if went >= 0.5 * h && went <= 2.0 * h {
                return (e.dist <= self.cut && self.inside(u, v)).then_some(new);
            }
        }
        None
    }

    /// The last point before the edge of the field's surface, on the step
    /// of length `h` from `cur`: by bisection on the step.
    fn edge(&mut self, cur: &Pt, h: f64, sign: f64) -> Option<Pt> {
        let (mut lo, mut hi) = (0.0, 1.0);
        let mut best = None;
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            let found = self
                .correct(cur.u + mid * h * cur.du, cur.v + mid * h * cur.dv)
                .filter(|(e, u, v)| e.dist <= self.cut && self.inside(*u, *v))
                .and_then(|(e, u, v)| {
                    let chord = (e.p - cur.p).normalized()?;
                    Some(Self::point(&e, u, v, sign).unwrap_or(Pt {
                        u,
                        v,
                        p: e.p,
                        t: chord,
                        du: 0.0,
                        dv: 0.0,
                    }))
                })
                .filter(|pt| (pt.p - cur.p).dot(cur.t) > 0.0);
            match found {
                Some(pt) => {
                    best = Some(pt);
                    lo = mid;
                }
                None => hi = mid,
            }
            if (hi - lo) * h <= self.on {
                break;
            }
        }
        best.filter(|pt| pt.p.distance(cur.p) > self.on)
    }

    fn trace(&mut self, seed: (f64, f64), singular: &[Singular]) -> Result<Vec<Branch>, GeomError> {
        let Some((e, u, v)) = self.correct(seed.0, seed.1) else {
            return Ok(Vec::new());
        };
        if e.dist > self.cut || !self.inside(u, v) {
            return Ok(Vec::new());
        }
        let Some(start) = Self::point(&e, u, v, 1.0) else {
            return Ok(Vec::new());
        };
        let (mut forward, end) = self.run(&start, 1.0, singular, true)?;
        let mut closed = end == End::Closed;
        // (In the order of the march: away from the start, reversed. A
        // run that goes round a crossing point onto the arm the other run
        // came into it along ends there: `beside`, `passed`.)
        let mut backward = if closed {
            Vec::new()
        } else {
            self.beside(&start, &forward, |m| {
                m.run(&start.reversed(), -1.0, singular, false)
            })?
            .0
        };
        let mut taken_up = 0;
        loop {
            let Some((pts, at)) = self.assembled(&start, &backward, &forward, closed)? else {
                return Ok(Vec::new());
            };
            self.unfitted = None;
            let refused = match self.refine_pieces(pts.clone(), closed) {
                // What the fit accepted, unless its points run back over
                // themselves: then the step the fold lies on stands for a
                // span the fit gave up on (`folded_step`).
                Ok(pieces) => match self.folded_step(&pts, at, &pieces) {
                    None => {
                        return Ok(pieces
                            .into_iter()
                            .map(|(pts, closed)| Branch { pts, closed })
                            .collect());
                    }
                    Some(step) => {
                        self.unfitted =
                            step.and_then(|k| pts.get(k).copied().zip(pts.get(k + 1).copied()));
                        folded()
                    }
                },
                Err(e) => e,
            };
            if !self.takes_up {
                return Err(refused);
            }
            // The span the fit gave up on, where it is a step of the
            // march: that step is taken to have left the curve it
            // followed (onto a curve beside it that heads the same way:
            // nothing a step is held to tells the two apart, and the fit
            // then looks for points between two points that are not
            // neighbours on one curve; or through a point where two
            // curves cross, onto this curve beyond it, the step a fold
            // lies on). The run is taken up again from the point before
            // it, in shorter steps, and what it had marched beyond is
            // dropped: where that was another curve, its own seeds find
            // it. What the fit brings within the tolerance and does not
            // run back over itself is never marched again, so every such
            // curve fitted before is the curve it was.
            let Some((a, b)) = self.unfitted.take() else {
                return Err(refused);
            };
            if taken_up >= TAKEN_UP {
                return Err(refused);
            }
            taken_up += 1;
            let Some(k) = Self::span_of(&pts, &a, &b) else {
                return Err(refused);
            };
            let (k, a, b) = self.nearest_the_start(&pts, at, k, a, b)?;
            // (A span too short to be marched in shorter steps stands.)
            let span = a.p.distance(b.p);
            if !(SHORTER * span >= self.hmin) {
                return Err(refused);
            }
            if k >= at && k - at < forward.len() + usize::from(closed) {
                // A step of the forward run (or the step it closed by).
                forward.truncate(k - at);
                let from = forward.last().copied().unwrap_or(start);
                let again = Again::past(from, forward.len(), span);
                let (more, end) = self.beside(&start, &backward, |m| {
                    m.run_on(&start, 1.0, singular, true, Some(again))
                })?;
                forward.extend(more);
                let was_closed = closed;
                closed = end == End::Closed;
                if closed {
                    backward.clear();
                } else if was_closed {
                    backward = self
                        .beside(&start, &forward, |m| {
                            m.run(&start.reversed(), -1.0, singular, false)
                        })?
                        .0;
                }
            } else if k < at && at - k <= backward.len() {
                // A step of the backward run, from the later of the two
                // points to the earlier.
                backward.truncate(at - k - 1);
                let back = start.reversed();
                let from = backward.last().copied().unwrap_or(back);
                let again = Again::past(from, backward.len(), span);
                let (more, _) = self.beside(&start, &forward, |m| {
                    m.run_on(&back, -1.0, singular, false, Some(again))
                })?;
                backward.extend(more);
            } else {
                // Not a step of the march (a span drawn on into a cusp).
                return Err(refused);
            }
        }
    }

    /// The wrong step a run is put right from: the span the fit gave up
    /// on (`k`, from `a` to `b`), or one nearer the start (`at`) that it
    /// would give up on too.
    ///
    /// A run is put right from its first wrong step on: what it marched
    /// after that step is on another curve, or further along this one
    /// than it had come. The fit meets the spans of the forward run in
    /// the order of the march, so the one it gave up on is the first;
    /// those of the backward run it meets from the far end, so there the
    /// span nearest the start is looked for, by fitting what lies between
    /// the one found and the start. (Taken up from a later wrong step, a
    /// run follows the curve that step was made on, and is as wrong as it
    /// was up to there.)
    fn nearest_the_start(
        &mut self,
        pts: &[Pt],
        at: usize,
        mut k: usize,
        mut a: Pt,
        mut b: Pt,
    ) -> Result<(usize, Pt, Pt), GeomError> {
        while k + 1 < at {
            let mut rest: Vec<Pt> = pts.get(k + 1..=at).map(<[Pt]>::to_vec).unwrap_or_default();
            self.unfitted = None;
            let Err(e) = self.refine_to_edge(&mut rest) else {
                break;
            };
            let Some(span) = self.unfitted.take() else {
                return Err(e);
            };
            match Self::span_of(pts, &span.0, &span.1) {
                Some(nearer) if nearer > k && nearer < at => {
                    (k, a, b) = (nearer, span.0, span.1);
                }
                _ => break,
            }
        }
        Ok((k, a, b))
    }

    /// Where in `pts` the span from `a` to `b` is: the index of `a`, with
    /// `b` the point after it (told by their parameters, which no two
    /// points of a march share).
    fn span_of(pts: &[Pt], a: &Pt, b: &Pt) -> Option<usize> {
        let same = |p: &Pt, q: &Pt| p.u == q.u && p.v == q.v;
        pts.iter()
            .zip(pts.iter().skip(1))
            .position(|(p, q)| same(p, a) && same(q, b))
    }

    /// Where in `pts` the point `q` is, told by its parameters.
    fn place_of(pts: &[Pt], q: &Pt) -> Option<usize> {
        pts.iter().position(|p| p.u == q.u && p.v == q.v)
    }

    /// The places where the points of a fitted branch run back over
    /// themselves: a point with a tangent of its own (a crossing point or
    /// an apex has none, and a branch ends there) between two spans each
    /// longer than [`FOLD_SPAN`] tolerances and 64 `on` whose chords turn
    /// by more than 120 degrees (a cosine below [`FOLD`]). Read from the
    /// places of the points alone, for no evaluation.
    ///
    /// The fit does not see it: a curve that runs one way along the
    /// crossing and then back along it is within the tolerance of both
    /// surfaces all its length, and a cubic span whose ends' tangents
    /// point against each other along a stretch is a hairpin whose control
    /// points lie on the stretch's line. A true corner of a branch (a
    /// run's last step put on a border at right angles to the step before
    /// it) turns by a right angle, not back.
    fn folds(&self, pts: &[Pt]) -> Vec<usize> {
        let least = (64.0 * self.on).max(FOLD_SPAN * self.tol.linear);
        let mut found = Vec::new();
        for (i, w) in pts.windows(3).enumerate() {
            let [p0, p1, p2] = w else {
                continue;
            };
            if p1.du == 0.0 && p1.dv == 0.0 {
                continue;
            }
            let (d1, d2) = (p1.p - p0.p, p2.p - p1.p);
            let (l1, l2) = (d1.length(), d2.length());
            if !(l1 > least && l2 > least) {
                continue;
            }
            if d1.dot(d2) / (l1 * l2) < FOLD {
                found.push(i + 1);
            }
        }
        found
    }

    /// The step of the march to put right where what the fit made of a
    /// branch (`pieces`, from `pts`, the points as marched, `at` the start
    /// among them) runs back over itself ([`Marcher::folds`]): `None`
    /// where it does not; `Some(None)` where it does and no step of the
    /// march can be named for a fold; else the step nearest the start
    /// among those named (the index in `pts` of its first point, the step
    /// going on to the next).
    ///
    /// A step through a point where two curves of a crossing cross each
    /// other can land on the same curve beyond it: the sense of travel
    /// flips at such a point, so the tangent where the step lands can
    /// agree with the one it left with, and the step is taken. The run
    /// then goes on from there back towards the crossing point, over the
    /// stretch the fit fills the step's span with. So a fold lies at a
    /// point the march came to, or between the two points of a step: one
    /// at a marched point is put on the step that came to it (for the
    /// forward run the step from the point before it, for the backward
    /// run the step from the point after it, in the order of the points),
    /// one between two marched points on the step between them. The
    /// points are told by their parameters, which no two points of a
    /// march share.
    fn folded_step(
        &self,
        pts: &[Pt],
        at: usize,
        pieces: &[(Vec<Pt>, bool)],
    ) -> Option<Option<usize>> {
        let mut folded = false;
        // (How many steps from the start, the step.)
        let mut nearest: Option<(usize, usize)> = None;
        for (piece, _) in pieces {
            for i in self.folds(piece) {
                folded = true;
                let step = match piece.get(i).and_then(|q| Self::place_of(pts, q)) {
                    Some(m) if m > at => Some(m - 1),
                    Some(m) if m < at => Some(m),
                    // The start: no step came to it.
                    Some(_) => None,
                    // A point the fit put between two marched points.
                    None => {
                        let before = piece
                            .get(..i)
                            .and_then(|s| s.iter().rev().find_map(|q| Self::place_of(pts, q)));
                        let after = piece
                            .get(i + 1..)
                            .and_then(|s| s.iter().find_map(|q| Self::place_of(pts, q)));
                        match (before, after) {
                            (Some(a), Some(b)) if b == a + 1 => Some(a),
                            _ => None,
                        }
                    }
                };
                if let Some(k) = step {
                    let from_start = if k >= at { k - at } else { at - k - 1 };
                    if nearest.is_none_or(|(d, _)| from_start < d) {
                        nearest = Some((from_start, k));
                    }
                }
            }
        }
        folded.then_some(nearest.map(|(_, k)| k))
    }

    /// The points of a branch in the order of travel, from what was
    /// marched either way from `start` (`backward` in the order of its
    /// march, away from the start), with where `start` is among them;
    /// `None` when they are no curve.
    fn assembled(
        &mut self,
        start: &Pt,
        backward: &[Pt],
        forward: &[Pt],
        closed: bool,
    ) -> Result<Option<(Vec<Pt>, usize)>, GeomError> {
        let mut pts: Vec<Pt>;
        if closed {
            pts = vec![*start];
            pts.extend(forward.iter().copied());
            // Close on the start, with parameters that continue the last.
            if let Some(last) = pts.last().copied() {
                let (cu, cv) = self.near((start.u, start.v), &last);
                pts.push(Pt {
                    u: cu,
                    v: cv,
                    ..*start
                });
            }
        } else {
            pts = backward.iter().rev().copied().map(Pt::reversed).collect();
            pts.push(*start);
            pts.extend(forward.iter().copied());
        }
        if pts.len() < 2 {
            return Ok(None);
        }
        let mut at = if closed { 0 } else { backward.len() };
        // An end on a border moved in short of a cusp is drawn on to it.
        if !closed {
            if let Some(last) = pts.last().copied()
                && let Some(cusp) = self.cusp_of(&last)
            {
                let more = self.on_into_cusp(&last, &cusp)?;
                pts.extend(more);
            }
            if let Some(first) = pts.first().copied()
                && let Some(cusp) = self.cusp_of(&first)
            {
                let more = self.on_into_cusp(&first.reversed(), &cusp)?;
                let mut before: Vec<Pt> = more.into_iter().rev().map(Pt::reversed).collect();
                at += before.len();
                before.append(&mut pts);
                pts = before;
            }
        }
        Ok(Some((pts, at)))
    }

    /// The border moved in short of a cusp that `p` is on, if any.
    fn cusp_of(&self, p: &Pt) -> Option<Cusp> {
        let c = &self.chart;
        c.cusps.iter().copied().find(|cusp| {
            let (here, (lo, hi)) = if cusp.pole.across_v() {
                (p.v, c.v)
            } else {
                (p.u, c.u)
            };
            (here - cusp.at).abs() <= 1e-9 * (hi - lo)
        })
    }

    /// The rest of a branch that ends at `end` on a border moved in short
    /// of a cusp (`field.rs`, `short_of_poles`), on to the point: at
    /// places across ever nearer to the pole (halving the distance), the
    /// zero of `f` along the border's parameter next to the last one, by
    /// bracketing and regula falsi on its sign alone (there `f` and its
    /// derivative along vanish to the order of the cusp, so no step size
    /// or test in units of length applies, but the sign does), until the
    /// place is within `on` of the pole by the speed across or every
    /// iso-curve along is shorter than `on` (nearer than that the sign of
    /// `f` is rounding, and what is left is within `on` of one meridian);
    /// then the pole, with the direction the meridians leave it along. `end`
    /// travels into the pole; the points come in that order.
    ///
    /// Refused where the zero is lost on the way (the intersection turns
    /// back before the point), or the other surface ends there.
    fn on_into_cusp(&mut self, end: &Pt, cusp: &Cusp) -> Result<Vec<Pt>, GeomError> {
        let lost = GeomError::Unsupported(
            "an intersection curve that comes close to a cusp of a surface and turns back before it",
        );
        let across_v = cusp.pole.across_v();
        let ((u0, u1), periodic) = if across_v {
            (self.chart.u, self.chart.periodic_u)
        } else {
            (self.chart.v, self.chart.periodic_v)
        };
        let place = |along: f64, across: f64| {
            if across_v {
                (along, across)
            } else {
                (across, along)
            }
        };
        let speed = if across_v {
            self.chart.bounds.lv
        } else {
            self.chart.bounds.lu
        };
        let length = u1 - u0;
        // Not wrapped round a periodic chart: the points continue the
        // branch's parameters (the spans between them are halved in
        // them), and every evaluation takes them round into the domain
        // (`Chart::at`).
        let mut along = if across_v { end.u } else { end.v };
        let mut previous = cusp.at;
        let mut pts: Vec<Pt> = Vec::new();
        let mut last = *end;
        let gap = cusp.at - cusp.from;
        let n = cusp.narrowing;
        for k in 1..=80 {
            self.spent()?;
            let across = cusp.from + gap * 0.5_f64.ppowi(k);
            // Nearer than this every iso-curve along is shorter than `on`
            // (the rest is one line to that, and `f` is rounding there),
            // or the pole is within `on`.
            let share = (across - cusp.from).abs() / n.span;
            if n.width * share.ppowi(n.order as i32) <= self.on
                || speed * (across - cusp.from).abs() <= self.on
            {
                break;
            }
            let f = |m: &mut Self, y: f64| -> Option<Eval> {
                let (u, v) = place(y, across);
                m.eval(u, v)
            };
            // The zero continues the last one: along the border it moves
            // no further than the derivatives along it turn, at most
            // `twist / speed` per unit across (`pole::Narrowing`).
            let limit = (n.twist * (across - previous).abs() / n.speed.max(f64::MIN_POSITIVE))
                .min(0.25 * length);
            previous = across;
            let here = f(self, along).ok_or(lost.clone())?;
            // The nearest change of sign along, either way.
            // On the other surface already, as `correct` takes a point
            // (near the point `f` changes along by less than rounding).
            let mut found = None;
            if here.f.abs() <= self.on {
                found = Some((along, along));
            }
            let mut reach = (1e-9 * length).min(limit);
            while found.is_none() && reach <= limit {
                for side in [-1.0, 1.0] {
                    let y = along + side * reach;
                    if !periodic && !(y >= u0 && y <= u1) {
                        continue;
                    }
                    let other = f(self, y).ok_or(lost.clone())?;
                    if (other.f >= 0.0) != (here.f >= 0.0) {
                        found = Some(if side < 0.0 { (y, along) } else { (along, y) });
                        break;
                    }
                }
                if reach >= limit {
                    break;
                }
                reach = (reach * 4.0).min(limit);
            }
            let (mut lo, mut hi) = found.ok_or(lost.clone())?;
            let (mut flo, mut fhi) = (
                f(self, lo).ok_or(lost.clone())?.f,
                f(self, hi).ok_or(lost.clone())?.f,
            );
            let mut side = 0;
            for step in 0..100 {
                if lo == hi {
                    break;
                }
                if flo == 0.0 {
                    hi = lo;
                    break;
                }
                if fhi == 0.0 {
                    lo = hi;
                    break;
                }
                let y = if step % 4 == 3 {
                    0.5 * (lo + hi)
                } else {
                    (lo * fhi - hi * flo) / (fhi - flo)
                };
                let y = if y > lo && y < hi { y } else { 0.5 * (lo + hi) };
                if !(y > lo && y < hi) {
                    break;
                }
                let fy = f(self, y).ok_or(lost.clone())?.f;
                if (fy >= 0.0) == (flo >= 0.0) {
                    (lo, flo) = (y, fy);
                    if side == -1 {
                        fhi *= 0.5;
                    }
                    side = -1;
                } else {
                    (hi, fhi) = (y, fy);
                    if side == 1 {
                        flo *= 0.5;
                    }
                    side = 1;
                }
            }
            along = 0.5 * (lo + hi);
            let (u, v) = place(along, across);
            let e = self.eval(u, v).ok_or(lost.clone())?;
            if e.dist > self.cut {
                return Err(lost);
            }
            let toward = cusp.pole.point - e.p;
            let point = Self::point(&e, u, v, 1.0)
                .map(|pt| {
                    if pt.t.dot(toward) < 0.0 {
                        pt.reversed()
                    } else {
                        pt
                    }
                })
                .or_else(|| {
                    (e.p - last.p).normalized().map(|t| Pt {
                        u,
                        v,
                        p: e.p,
                        t,
                        du: 0.0,
                        dv: 0.0,
                    })
                })
                .ok_or(lost.clone())?;
            pts.push(point);
            last = point;
        }
        // The point itself: every meridian leaves it along one line, the
        // derivative across there (the same on all of them at a cusp).
        let (u, v) = place(along, cusp.from);
        let (_, su, sv) = self.chart.at(u, v);
        let out = if across_v { sv } else { su };
        let chord = (cusp.pole.point - last.p).normalized();
        let t = match (out.normalized(), chord) {
            (Some(d), Some(c)) => {
                if d.dot(c) < 0.0 {
                    -d
                } else {
                    d
                }
            }
            (Some(d), None) => {
                if d.dot(last.t) < 0.0 {
                    -d
                } else {
                    d
                }
            }
            (None, Some(c)) => c,
            (None, None) => last.t,
        };
        let rate = out.length();
        let step = if rate > 0.0 {
            (cusp.from - cusp.at).signum() / rate
        } else {
            0.0
        };
        let (du, dv) = if across_v { (0.0, step) } else { (step, 0.0) };
        pts.push(Pt {
            u,
            v,
            p: cusp.pole.point,
            t,
            du,
            dv,
        });
        Ok(pts)
    }

    // ----- fitting ---------------------------------------------------------------

    /// The handle length of the cubic span from `a` to `b` as a fraction of
    /// the chord: the value that makes a circular arc of the same turn
    /// exact to sixth order, `(2/3) tan(θ/4) / sin(θ/2)`.
    fn handle(a: &Pt, b: &Pt) -> f64 {
        let theta = a.t.dot(b.t).clamp(-1.0, 1.0).pacos().min(2.0);
        if theta < 1e-4 {
            1.0 / 3.0
        } else {
            (2.0 / 3.0) * (0.25 * theta).ptan() / (0.5 * theta).psin()
        }
    }

    /// The four Bézier points of the span from `a` to `b`.
    fn span(a: &Pt, b: &Pt) -> [Vec3; 4] {
        let l = a.p.distance(b.p) * Self::handle(a, b);
        [a.p, a.p + a.t * l, b.p - b.t * l, b.p]
    }

    fn bezier(c: &[Vec3; 4], s: f64) -> Vec3 {
        let r = 1.0 - s;
        let [c0, c1, c2, c3] = *c;
        c0 * (r * r * r) + c1 * (3.0 * r * r * s) + c2 * (3.0 * r * s * s) + c3 * (s * s * s)
    }

    /// How far the span from `a` to `b` is from either surface, at three
    /// places.
    fn deviation(&mut self, a: &Pt, b: &Pt) -> f64 {
        let c = Self::span(a, b);
        let mut worst = 0.0_f64;
        for s in [0.25, 0.5, 0.75] {
            let q = Self::bezier(&c, s);
            let other = self.field.sample(q).map_or(f64::INFINITY, |x| x.dist);
            let own = match &self.own {
                Field::Free(projector) => {
                    let (u, v) = self
                        .chart
                        .wrapped(a.u + (b.u - a.u) * s, a.v + (b.v - a.v) * s);
                    let (_, _, foot) = projector.foot_from(q, u, v);
                    foot.distance(q)
                }
                field => field.sample(q).map_or(f64::INFINITY, |x| x.dist),
            };
            self.evals += 2;
            worst = worst.max(other).max(own);
        }
        // Where the surfaces are nearly tangent a curve can be close to
        // both and still far from where they meet: hold the middle of the
        // span against a point of the intersection itself. (Not on the
        // way into a crossing point the surfaces are a little apart at:
        // there the curve is drawn to the point, and `f = 0` is not.)
        if !self.modelled(a, b)
            && let Some(m) = self.between(a, b)
        {
            let off = Self::bezier(&c, 0.5) - m.p;
            worst = worst.max((off - m.t * off.dot(m.t)).length());
        }
        worst
    }

    /// True when the span from `a` to `b` lies in the zone of a crossing
    /// point the surfaces are a little apart at.
    fn modelled(&self, a: &Pt, b: &Pt) -> bool {
        self.zones
            .iter()
            .any(|z| a.p.distance(z.p) <= z.radius && b.p.distance(z.p) <= z.radius)
    }

    /// The point of the chart halfway between `a` and `b` in its
    /// parameters, travelling from `a` to `b`: on the way into a crossing
    /// point along the direction in which the Hessian vanishes, `f` stays
    /// what it is at the point to third order.
    fn straight(&mut self, a: &Pt, b: &Pt) -> Option<Pt> {
        let (u, v) = (0.5 * (a.u + b.u), 0.5 * (a.v + b.v));
        let e = self.eval(u, v)?;
        let t = (b.p - a.p).normalized()?;
        let (g11, g12, g22) = (e.su.dot(e.su), e.su.dot(e.sv), e.sv.dot(e.sv));
        let det = g11 * g22 - g12 * g12;
        if !(det > 0.0) {
            return None;
        }
        let (tu, tv) = (t.dot(e.su), t.dot(e.sv));
        let (du, dv) = ((g22 * tu - g12 * tv) / det, (g11 * tv - g12 * tu) / det);
        (du != 0.0 || dv != 0.0).then_some(Pt {
            u,
            v,
            p: e.p,
            t,
            du,
            dv,
        })
    }

    /// A point of the branch between `a` and `b`.
    fn between(&mut self, a: &Pt, b: &Pt) -> Option<Pt> {
        if let Some(made) = self.tac_between(a, b) {
            return made;
        }
        let (mu, mv) = (0.5 * (a.u + b.u), 0.5 * (a.v + b.v));
        self.between_from(a, b, mu, mv).or_else(|| {
            // Where the speed of the parametrisation jumps between `a`
            // and `b` (a knot of a spline whose spans are drawn at other
            // paces: rational weights far apart) the middle of their
            // parameters may be next to one of them, or Newton from it
            // may not settle: start from the foot of the middle of the
            // chord instead.
            let Field::Free(projector) = &self.own else {
                return None;
            };
            let middle = a.p.lerp(b.p, 0.5);
            let (u, v, _) = projector.foot(middle);
            let (pu, pv) = (
                self.chart.u.1 - self.chart.u.0,
                self.chart.v.1 - self.chart.v.0,
            );
            let u = Self::unwrap(u, mu, self.chart.periodic_u, pu);
            let v = Self::unwrap(v, mv, self.chart.periodic_v, pv);
            // Near the chord: a point of another branch that Newton may
            // find from there is not between them.
            let chord = a.p.distance(b.p);
            self.between_from(a, b, u, v)
                .filter(|pt| pt.p.distance(middle) <= 0.5 * chord)
        })
    }

    /// The point of the branch between `a` and `b` that Newton finds from
    /// `(u, v)`, if it is between them.
    fn between_from(&mut self, a: &Pt, b: &Pt, u: f64, v: f64) -> Option<Pt> {
        let (e, u, v) = self.correct(u, v)?;
        let chord = b.p - a.p;
        let along = (e.p - a.p).dot(chord);
        if !(along > 0.0 && along < chord.length_squared()) {
            return None;
        }
        let sign = if (-e.gv * e.su.dot(chord) + e.gu * e.sv.dot(chord)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        Self::point(&e, u, v, sign)
    }

    /// Adds points until every span is within half the tolerance of both
    /// surfaces at its sampled places; refused where the branch leaves
    /// the field's surface between two of its points.
    fn refine(&mut self, pts: &mut Vec<Pt>) -> Result<(), GeomError> {
        match self.refine_to_edge(pts)? {
            Refined::Done => Ok(()),
            Refined::Leaves(..) => Err(not_fitted()),
        }
    }

    /// Refines the branch (`refine`) and, where it leaves the field's
    /// surface between two of its points, divides it there: the pieces on
    /// the surface, each refined, and whether the one piece left is still
    /// closed. A march takes steps and looks at their ends, so a step can
    /// pass over a stretch where `f = 0` runs on past the edge of the
    /// other surface and back (the field goes on past its edge); the
    /// points the refinement puts between show it, and the branch ends at
    /// the edge there as a march that met it would (`edge_between`).
    fn refine_pieces(
        &mut self,
        pts: Vec<Pt>,
        closed: bool,
    ) -> Result<Vec<(Vec<Pt>, bool)>, GeomError> {
        // Each piece with whether it begins at the branch's first point
        // and whether it ends at its last.
        let mut done: Vec<(Vec<Pt>, bool, bool)> = Vec::new();
        let mut rest = pts;
        let mut at_first = true;
        let mut split = false;
        loop {
            self.spent()?;
            let Refined::Leaves(i, m) = self.refine_to_edge(&mut rest)? else {
                done.push((rest, at_first, true));
                break;
            };
            if done.len() >= MAX_BRANCHES {
                return Err(GeomError::NoConvergence(
                    "the intersection has more branches than the cap allows",
                ));
            }
            split = true;
            let give_up = not_fitted();
            let (Some(a), Some(b)) = (rest.get(i).copied(), rest.get(i + 1).copied()) else {
                return Err(give_up);
            };
            let mut head: Vec<Pt> = rest.get(..=i).map(<[Pt]>::to_vec).unwrap_or_default();
            head.extend(self.edge_between(&a, &m, &a));
            let mut tail: Vec<Pt> = self.edge_between(&b, &m, &b).into_iter().collect();
            tail.extend(rest.get(i + 1..).unwrap_or_default().iter().copied());
            // The head was refined up to `a`; its new last span is
            // refined with the rest of it (nothing before it changes).
            if head.len() >= 2 {
                match self.refine_to_edge(&mut head)? {
                    Refined::Done => done.push((head, at_first, false)),
                    // (The edge found from `a` is on the surface, so the
                    // span to it leaves it only if the march passed over
                    // more than one stretch: refused as before.)
                    Refined::Leaves(..) => return Err(give_up),
                }
            }
            at_first = false;
            rest = tail;
        }
        let mut pieces: Vec<(Vec<Pt>, bool, bool)> =
            done.into_iter().filter(|p| p.0.len() >= 2).collect();
        if !split {
            return Ok(pieces.into_iter().map(|p| (p.0, closed)).collect());
        }
        // A closed branch divided is open, its first piece going on from
        // its last when the first begins at the loop's start and the last
        // ends on the copy of it (whose parameters continue the loop's:
        // the first piece's are moved by as much). Where either was left
        // out (the loop's start on the edge of the field's surface, so
        // that nothing of the piece before it is on it), the loop leaves
        // the surface there and its pieces stay apart.
        let joins = closed
            && pieces.len() >= 2
            && pieces.first().is_some_and(|p| p.1)
            && pieces.last().is_some_and(|p| p.2);
        if joins {
            let (first, _, _) = pieces.remove(0);
            if let (Some((last, _, _)), Some(start)) = (pieces.last_mut(), first.first()) {
                let shift = last
                    .last()
                    .map_or((0.0, 0.0), |copy| (copy.u - start.u, copy.v - start.v));
                last.extend(first.iter().skip(1).map(|p| Pt {
                    u: p.u + shift.0,
                    v: p.v + shift.1,
                    ..*p
                }));
            }
        }
        Ok(pieces.into_iter().map(|p| (p.0, false)).collect())
    }

    /// The last point from `from` towards `to` (in the chart's parameters,
    /// each brought onto `f = 0`) that is still on the field's surface,
    /// found by halving, travelling the way `like` does; `None` when
    /// there is none further than `on` from `from`.
    fn edge_between(&mut self, from: &Pt, to: &Pt, like: &Pt) -> Option<Pt> {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let mut best = None;
        let reach = from.p.distance(to.p);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            let found = self
                .correct(
                    from.u + mid * (to.u - from.u),
                    from.v + mid * (to.v - from.v),
                )
                .filter(|(e, u, v)| e.dist <= self.cut && self.inside(*u, *v))
                .map(|(e, u, v)| {
                    let point = Self::point(&e, u, v, 1.0).map(|pt| {
                        if pt.t.dot(like.t) < 0.0 {
                            Pt {
                                t: -pt.t,
                                du: -pt.du,
                                dv: -pt.dv,
                                ..pt
                            }
                        } else {
                            pt
                        }
                    });
                    point.unwrap_or(Pt {
                        u,
                        v,
                        p: e.p,
                        t: like.t,
                        du: 0.0,
                        dv: 0.0,
                    })
                });
            match found {
                Some(pt) => {
                    best = Some(pt);
                    lo = mid;
                }
                None => hi = mid,
            }
            if (hi - lo) * reach <= self.on {
                break;
            }
        }
        best.filter(|pt| pt.p.distance(from.p) > self.on)
    }

    /// `refine`, stopping at the first span whose middle on the branch is
    /// past the edge of the field's surface by more than the span may be
    /// off it (`Refined::Leaves`, with the span's index and that middle): no point added
    /// there could bring that span within the tolerance.
    fn refine_to_edge(&mut self, pts: &mut Vec<Pt>) -> Result<Refined, GeomError> {
        let target = 0.5 * self.tol.linear;
        let mut i = 0;
        // The span of the points as they were given that the span at `i`
        // is part of: where its two points are now.
        let (mut first, mut last) = (0_usize, 1_usize);
        while let (Some(mut a), Some(mut b)) = (pts.get(i).copied(), pts.get(i + 1).copied()) {
            self.spent()?;
            // An end on a crossing point or an apex has no tangent of its
            // own: it gets the one a circle through its neighbour would
            // have there (the neighbour's, mirrored in the chord).
            let mirrored = |from: &Pt, at: &Pt| {
                (at.p - from.p)
                    .normalized()
                    .map(|chord| chord * (2.0 * chord.dot(from.t)) - from.t)
            };
            if b.du == 0.0 && b.dv == 0.0 && !(a.du == 0.0 && a.dv == 0.0) {
                b.t = mirrored(&a, &b).unwrap_or(b.t);
                if let Some(slot) = pts.get_mut(i + 1) {
                    *slot = b;
                }
            } else if a.du == 0.0 && a.dv == 0.0 && !(b.du == 0.0 && b.dv == 0.0) {
                a.t = mirrored(&b, &a).unwrap_or(a.t);
                if let Some(slot) = pts.get_mut(i) {
                    *slot = a;
                }
            }
            if self.deviation(&a, &b) <= target {
                i += 1;
                if i == last {
                    (first, last) = (i, i + 1);
                }
                continue;
            }
            let give_up = not_fitted();
            if pts.len() >= 2 * MAX_POINTS || a.p.distance(b.p) <= 8.0 * self.on {
                self.unfitted = pts.get(first).copied().zip(pts.get(last).copied());
                return Err(give_up);
            }
            let middle = if self.modelled(&a, &b) {
                self.straight(&a, &b)
            } else {
                self.between(&a, &b)
            };
            let Some(middle) = middle else {
                self.unfitted = pts.get(first).copied().zip(pts.get(last).copied());
                return Err(give_up);
            };
            self.evals += 1;
            if self
                .field
                .sample(middle.p)
                .is_some_and(|x| x.dist - x.signed.abs() > target)
            {
                return Ok(Refined::Leaves(i, middle));
            }
            pts.insert(i + 1, middle);
            last += 1;
        }
        Ok(Refined::Done)
    }

    /// True when `p` lies on the branch.
    fn on_branch(&self, p: Vec3, pts: &[Pt], slack: f64) -> bool {
        for (k, end) in [pts.first(), pts.last()].into_iter().flatten().enumerate() {
            // A seed just past an end is the same branch.
            let ahead = (p - end.p).dot(end.t) * if k == 0 { -1.0 } else { 1.0 };
            if p.distance(end.p) <= 2.0 * self.cut && ahead >= 0.0 {
                return true;
            }
        }
        Self::on_spans(p, pts, slack)
    }

    /// True when `p` lies on one of the spans between the points.
    fn on_spans(p: Vec3, pts: &[Pt], slack: f64) -> bool {
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = b.p - a.p;
            let len2 = chord.length_squared();
            if !(len2 > 0.0) {
                continue;
            }
            let len = len2.sqrt();
            let t = ((p - a.p).dot(chord) / len2).clamp(0.0, 1.0);
            if (a.p + chord * t).distance(p) > 0.1 * len + slack {
                continue;
            }
            let c = Self::span(a, b);
            let mut prev = a.p;
            for k in 1..=8 {
                let q = Self::bezier(&c, f64::from(k) / 8.0);
                let seg = q - prev;
                let l2 = seg.length_squared();
                let s = if l2 > 0.0 {
                    ((p - prev).dot(seg) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if (prev + seg * s).distance(p) <= 2e-3 * len + slack {
                    return true;
                }
                prev = q;
            }
        }
        false
    }

    /// The point a fraction `share` of the way along the spans of `pts`,
    /// by the lengths of their chords.
    fn along(pts: &[Pt], share: f64) -> Option<Vec3> {
        let total: f64 = pts
            .windows(2)
            .map(|w| match w {
                [a, b] => a.p.distance(b.p),
                _ => 0.0,
            })
            .sum();
        let mut left = share * total;
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = a.p.distance(b.p);
            if left <= chord && chord > 0.0 {
                return Some(Self::bezier(&Self::span(a, b), left / chord));
            }
            left -= chord;
        }
        pts.last().map(|p| p.p)
    }

    /// The span of `pts` that `p` lies on within `slack`, and where along
    /// it (`(k, s)`: on the cubic from `pts[k]` to `pts[k + 1]` at `s`). The
    /// cubic is searched, not its chord: two branches near each other are
    /// apart by less than the chord strays from them.
    fn place_on(p: Vec3, pts: &[Pt], slack: f64) -> Option<(usize, f64)> {
        let mut best: Option<(f64, usize, f64)> = None;
        for (k, (a, b)) in pts.iter().zip(pts.iter().skip(1)).enumerate() {
            let len = a.p.distance(b.p);
            if !(len > 0.0) {
                continue;
            }
            // The span lies within a third of its chord of the chord.
            let chord = b.p - a.p;
            let t = ((p - a.p).dot(chord) / (len * len)).clamp(0.0, 1.0);
            if (a.p + chord * t).distance(p) > 0.5 * len + slack {
                continue;
            }
            let c = Self::span(a, b);
            let at = |s: f64| Self::bezier(&c, s).distance(p);
            // The nearest of 32 samples, then golden sections about it.
            let mut s0 = 0.0;
            for i in 0..=32 {
                let s = f64::from(i) / 32.0;
                if at(s) < at(s0) {
                    s0 = s;
                }
            }
            let (mut lo, mut hi) = ((s0 - 1.0 / 32.0).max(0.0), (s0 + 1.0 / 32.0).min(1.0));
            for _ in 0..60 {
                let (m1, m2) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
                if at(m1) <= at(m2) {
                    hi = m2;
                } else {
                    lo = m1;
                }
            }
            let s = 0.5 * (lo + hi);
            let d = at(s);
            if d <= slack && best.is_none_or(|(least, _, _)| d < least) {
                best = Some((d, k, s));
            }
        }
        best.map(|(_, k, s)| (k, s))
    }

    /// One branch of what two traces found, when they are the same branch:
    /// the second traced from a seed that the first passed by (its end was
    /// decided where the curve runs within the tolerance of the chart's
    /// border, of a pole or of the field's edge, so where along that
    /// stretch it ends is rounding, and a seed further along it is not
    /// "just past" the end). Two traces of one branch run along each other:
    /// they are the same when the quarter points of either lie on the
    /// other within the tolerance. Then the result is the first with what
    /// the second reaches beyond either of its ends (an end of the first
    /// that lies on the second is one the second runs past), refined at
    /// the joints as any span. `None` when they are not one.
    fn united(&mut self, old: &Branch, new: &Branch) -> Result<Option<Branch>, GeomError> {
        if old.closed || new.closed {
            return Ok(None);
        }
        let slack = self.tol.linear;
        // A quarter point of one on the other, with where it is on both.
        let shared = |a: &[Pt], b: &[Pt]| -> Option<Vec3> {
            let mut found = None;
            for share in [0.25, 0.5, 0.75] {
                let q = Self::along(a, share)?;
                Self::place_on(q, b, slack)?;
                found = Some(q);
            }
            found
        };
        let Some(q) = shared(&new.pts, &old.pts).or_else(|| shared(&old.pts, &new.pts)) else {
            return Ok(None);
        };
        // The second the way the first runs: by their spans through `q`.
        let (Some((ko, _)), Some((kn, _))) = (
            Self::place_on(q, &old.pts, slack),
            Self::place_on(q, &new.pts, slack),
        ) else {
            return Ok(None);
        };
        let direction = |pts: &[Pt], k: usize| match (pts.get(k), pts.get(k + 1)) {
            (Some(a), Some(b)) => b.p - a.p,
            _ => Vec3::ZERO,
        };
        let mut pts = new.pts.clone();
        if direction(&old.pts, ko).dot(direction(&new.pts, kn)) < 0.0 {
            pts = pts.into_iter().rev().map(Pt::reversed).collect();
        }
        let (Some(first), Some(last)) = (old.pts.first().copied(), old.pts.last().copied()) else {
            return Ok(None);
        };
        // What the second reaches beyond the first's ends: its points
        // before the span the first's start lies on, and after the one its
        // end lies on.
        let head: Vec<Pt> = match Self::place_on(first.p, &pts, slack) {
            Some((k, _)) => pts.get(..=k).unwrap_or(&[]).to_vec(),
            None => Vec::new(),
        };
        let tail: Vec<Pt> = match Self::place_on(last.p, &pts, slack) {
            Some((k, _)) => pts.get(k + 1..).unwrap_or(&[]).to_vec(),
            None => Vec::new(),
        };
        // (Points of the second at the first's ends are the first's.)
        let beyond = |list: Vec<Pt>| -> Vec<Pt> {
            list.into_iter()
                .filter(|p| Self::place_on(p.p, &old.pts, slack).is_none())
                .collect()
        };
        let (head, tail) = (beyond(head), beyond(tail));
        if head.is_empty() && tail.is_empty() {
            // Nothing of the second beyond the first: no second trace of
            // it. Its seed was not on the first (`on_branch`, a looser
            // test than this one, would have said so), so it is another
            // branch that runs within the tolerance of the first all its
            // short length: an arm that leaves a point of contact along
            // it, cut short by the box. Kept.
            return Ok(None);
        }
        // Parameters continued from the first's ends (a chart closed on
        // itself is unwrapped).
        let shifted = |list: Vec<Pt>, to: &Pt, at: Option<&Pt>, this: &Self| -> Vec<Pt> {
            let Some(at) = at else { return list };
            let (u, v) = this.near((at.u, at.v), to);
            let (du, dv) = (u - at.u, v - at.v);
            list.into_iter()
                .map(|p| Pt {
                    u: p.u + du,
                    v: p.v + dv,
                    ..p
                })
                .collect()
        };
        // An end of the first that the second runs past is no end: where
        // the march stopped there (on the chart's border, at the field's
        // edge) the curve was within the tolerance of it and went on, so
        // that point is the border's, not the curve's, and is left out.
        let mut middle = old.pts.as_slice();
        if !head.is_empty() && middle.len() > 2 {
            middle = middle.get(1..).unwrap_or(middle);
        }
        if !tail.is_empty() && middle.len() > 2 {
            middle = middle.get(..middle.len() - 1).unwrap_or(middle);
        }
        let head = shifted(head.clone(), &first, head.last(), self);
        let tail = shifted(tail.clone(), &last, tail.first(), self);
        let mut joined = head;
        joined.extend(middle.iter().copied());
        joined.extend(tail);
        self.refine(&mut joined)?;
        Ok(Some(Branch {
            pts: joined,
            closed: false,
        }))
    }

    /// The C¹ cubic through the points, and how far it is from the
    /// surfaces where that was measured.
    fn fit(&mut self, pts: &[Pt]) -> Result<(NurbsCurve, f64, f64), GeomError> {
        let mut knots = vec![0.0; 4];
        let mut ctrl = Vec::with_capacity(2 * pts.len());
        let mut total = 0.0;
        let mut worst = 0.0_f64;
        let mut spans = 0;
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            let chord = a.p.distance(b.p);
            if !(chord > 0.0) {
                continue;
            }
            let c = Self::span(a, b);
            if spans == 0 {
                ctrl.push(a.p);
            } else {
                knots.push(total);
                knots.push(total);
            }
            // A knot span of three handles makes the derivative the unit
            // tangent on both sides of every joint.
            total += 3.0 * chord * Self::handle(a, b);
            let [_, c1, c2, _] = c;
            ctrl.push(c1);
            ctrl.push(c2);
            spans += 1;
            worst = worst.max(self.deviation(a, b));
        }
        let last = pts
            .last()
            .ok_or(GeomError::InvalidParameter("an empty intersection curve"))?;
        ctrl.push(last.p);
        knots.extend([total; 4]);
        if spans == 0 {
            return Err(GeomError::InvalidParameter("an empty intersection curve"));
        }
        Ok((NurbsCurve::polynomial(3, knots, ctrl)?, total, worst))
    }

    // ----- clipping to the box --------------------------------------------------

    /// The last point inside the box on the way from `a` (inside) to `b`
    /// (outside).
    fn box_crossing(&mut self, a: &Pt, b: &Pt, within: &Bbox3, flip: bool) -> Option<Pt> {
        let (mut lo, mut hi) = (0.0, 1.0);
        let mut best = None;
        let chord = b.p - a.p;
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            let found = self
                .correct(a.u + (b.u - a.u) * mid, a.v + (b.v - a.v) * mid)
                .filter(|(e, _, _)| within.contains(e.p, 0.0));
            match found {
                Some((e, u, v)) => {
                    let forward = -e.gv * e.su.dot(chord) + e.gu * e.sv.dot(chord) >= 0.0;
                    let sign = if forward != flip { 1.0 } else { -1.0 };
                    best = Self::point(&e, u, v, sign).or(best);
                    lo = mid;
                }
                None => hi = mid,
            }
            if (hi - lo) * chord.length() <= self.on {
                break;
            }
        }
        best
    }

    /// The pieces of a branch inside the box.
    fn clip(&mut self, branch: &Branch, within: &Bbox3) -> Vec<(Vec<Pt>, bool)> {
        let pts = &branch.pts;
        let inside: Vec<bool> = pts.iter().map(|p| within.contains(p.p, 0.0)).collect();
        if inside.iter().all(|x| *x) {
            return vec![(pts.clone(), branch.closed)];
        }
        // Start a closed branch at a point outside, so no piece spans the seam.
        let order: Vec<usize> = match (branch.closed, inside.iter().position(|x| !*x)) {
            (true, Some(first)) => {
                let n = pts.len().saturating_sub(1).max(1);
                (0..=n).map(|k| (first + k) % n).collect()
            }
            _ => (0..pts.len()).collect(),
        };
        let mut pieces = Vec::new();
        let mut piece: Vec<Pt> = Vec::new();
        let mut prev: Option<(Pt, bool)> = None;
        let mut shift = (0.0, 0.0);
        for k in order {
            let (Some(raw), Some(&is_in)) = (pts.get(k), inside.get(k)) else {
                continue;
            };
            // Keep parameters continuous across the seam of a closed branch.
            let mut pt = *raw;
            if let Some((before, _)) = prev {
                let (u, v) = self.near((pt.u + shift.0, pt.v + shift.1), &before);
                shift = (u - raw.u, v - raw.v);
                (pt.u, pt.v) = (u, v);
            }
            if let Some((before, was_in)) = prev {
                if was_in && !is_in {
                    piece.extend(self.box_crossing(&before, &pt, within, false));
                    pieces.push(std::mem::take(&mut piece));
                } else if !was_in && is_in {
                    piece.extend(self.box_crossing(&pt, &before, within, true));
                }
            }
            if is_in {
                piece.push(pt);
            }
            prev = Some((pt, is_in));
        }
        pieces.push(piece);
        pieces
            .into_iter()
            .filter(|p| p.len() >= 2)
            .map(|p| (p, false))
            .collect()
    }

    // ----- the whole intersection -----------------------------------------------

    pub(super) fn solve(&mut self, within: &Bbox3) -> Result<Meeting, GeomError> {
        self.border_stretches()?;
        let mut singular: Vec<Singular> = Vec::new();
        // The apex of a cone the chart passes through.
        if let Some(apex) = self.field.apex()
            && let Ok((u, v)) = self.chart.surface.project(apex)
            && self.chart.surface.eval(u, v).distance(apex) <= self.tol.linear
        {
            singular.push(Singular {
                u,
                v,
                p: apex,
                kind: SingularKind::Apex,
                arm: 0.0,
                turn: 0.0,
            });
        }
        let merge = 8.0 * self.tol.linear;
        let mut seeds = self.seeds(&mut singular)?;
        seeds.extend(self.field_border_seeds()?);
        seeds.retain(|&(u, v)| !self.on_lying_border(u, v));
        for s in singular.clone() {
            if s.kind == SingularKind::Cross {
                let arms = self.arms(&s, &mut seeds);
                // (A quarter over: an arm's seed is brought onto `f = 0`
                // from its place on the circle.)
                if s.turn > 0.5 * self.tol.linear {
                    self.zones.push(Zone {
                        u: s.u,
                        v: s.v,
                        p: s.p,
                        radius: 1.25 * s.arm,
                        near: 0.5 * s.arm,
                        arms,
                    });
                }
            }
        }
        for tac in &self.tacnodes {
            seeds.extend(tac.seeds.iter().copied());
        }
        let mut branches: Vec<Branch> = Vec::new();
        for seed in seeds {
            self.spent()?;
            // On the curve first: a seed is only as good as its bracket.
            let Some((e, u, v)) = self.correct(seed.0, seed.1) else {
                continue;
            };
            // Where the surfaces are nearly tangent a point with `f = 0`
            // within `on` is only that well placed across the curve.
            let sine =
                e.su.cross(e.sv)
                    .normalized()
                    .map_or(0.0, |n| n.cross(e.n).length());
            let slack = self.tol.linear + 16.0 * self.on / sine.max(1e-12);
            // Beside a stretch of a border that lies on the field, `f = 0`
            // is the border to within what the stretch is off the field
            // over the sine of the angle between the surfaces: the same
            // meeting, already reported (where the borders agree only to
            // the tolerance, `f = 0` wanders across the border, in and out
            // of the chart, all along).
            if self.in_band(u, v).is_some() {
                continue;
            }
            // Close to a crossing point every arm has its own seed.
            let covered = |s: &Singular| {
                let radius = match s.kind {
                    SingularKind::Cross | SingularKind::Tacnode => 0.5 * s.arm,
                    _ => 2.0 * merge,
                };
                e.p.distance(s.p) <= radius.max(2.0 * merge)
            };
            // (Inside the zone of a crossing point the surfaces are a
            // little apart at, beside an arm a branch comes into the point
            // along: on that branch, `along_traced`.)
            if branches.iter().any(|b| self.on_branch(e.p, &b.pts, slack))
                || singular.iter().any(covered)
                || self.tac_inside(u, v)
                || self.along_traced(e.p)
            {
                continue;
            }
            let pieces = self.trace((u, v), &singular)?;
            let divided = pieces.len() > 1;
            for branch in pieces {
                // A piece of a branch divided where it left the field's
                // surface may be one traced already from a seed of its own
                // (its ends and middle on that branch; one that reaches
                // further is joined to it, or kept, as any other).
                let traced = |b: &Branch| {
                    [
                        branch.pts.first(),
                        branch.pts.get(branch.pts.len() / 2),
                        branch.pts.last(),
                    ]
                    .into_iter()
                    .all(|p| p.is_some_and(|p| Self::on_spans(p.p, &b.pts, slack)))
                };
                if divided && branches.iter().any(traced) {
                    continue;
                }
                let mut joined = false;
                for old in &mut branches {
                    if let Some(one) = self.united(old, &branch)? {
                        *old = one;
                        joined = true;
                        break;
                    }
                }
                if joined {
                    continue;
                }
                if branches.len() >= MAX_BRANCHES {
                    return Err(GeomError::NoConvergence(
                        "the intersection has more branches than the cap allows",
                    ));
                }
                branches.push(branch);
            }
            // The arms the branches come into the points of zones along.
            if !self.zones.is_empty() {
                let into: Vec<(usize, usize)> = branches
                    .iter()
                    .flat_map(|b| self.arms_into_zones(&b.pts))
                    .collect();
                self.traced_into = into;
            }
        }
        let mut curves = self.lying_curves(&branches);
        for branch in &branches {
            let whole = branch.pts.iter().all(|p| within.contains(p.p, 0.0));
            let mut pieces = Vec::new();
            for (pts, closed) in self.clip(branch, within) {
                // Where the box cut the branch, the span to the new end is
                // a new cubic (the end and its tangent are new), held to
                // nothing yet: it is refined as the branch was.
                if whole {
                    pieces.push((pts, closed));
                } else {
                    pieces.extend(self.refine_pieces(pts, closed)?);
                }
            }
            for (pts, closed) in pieces {
                let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
                    continue;
                };
                let length: f64 = pts
                    .iter()
                    .zip(pts.iter().skip(1))
                    .map(|(a, b)| a.p.distance(b.p))
                    .sum();
                if !closed
                    && first.p.distance(last.p) <= self.tol.linear
                    && length <= self.tol.linear
                {
                    continue; // Too short to be a curve.
                }
                let (curve, total, worst) = self.fit(&pts)?;
                let deviation = 1.5 * worst + 4.0 * self.on;
                if !(deviation <= self.tol.linear) {
                    return Err(not_fitted());
                }
                curves.push(MeetingCurve {
                    curve: Curve3::Nurbs(curve),
                    range: (0.0, total),
                    closed,
                    tangent: false,
                    exact: false,
                    deviation,
                });
            }
        }
        let points = singular
            .iter()
            .filter(|s| s.kind == SingularKind::Touch && within.contains(s.p, self.tol.linear))
            .map(|s| s.p)
            .collect();
        Ok(Meeting { curves, points })
    }

    /// True when every one of a grid of points of `a` lies on `b`; the
    /// second value says whether their normals agree there.
    pub(super) fn lies_on(
        a: &Surface,
        field: &Field,
        within: &Bbox3,
        tol: &Tolerance,
    ) -> Option<bool> {
        let chart = Chart::new(a, within, tol, field).ok()??;
        let mut same = None;
        // Against a freeform surface, which ends: a point whose nearest
        // point of it is on its border is past it (a patch lying on
        // another that ends sooner), and says nothing. Enough of the
        // others must be on it.
        let mut counted = 0_usize;
        for i in 0..9 {
            for j in 0..9 {
                let u = chart.u.0 + (chart.u.1 - chart.u.0) * (f64::from(i) + 0.5) / 9.0;
                let v = chart.v.0 + (chart.v.1 - chart.v.0) * (f64::from(j) + 0.5) / 9.0;
                let p = chart.surface.eval(u, v);
                if let Field::Free(projector) = field
                    && projector.on_border(p)
                {
                    continue;
                }
                let sample = field.sample(p)?;
                if sample.dist > tol.linear {
                    return None;
                }
                counted += 1;
                if same.is_none()
                    && let Some(n) = chart.surface.normal(u, v)
                {
                    same = Some(n.dot(sample.normal) > 0.0);
                }
            }
        }
        // (Every sample of an analytic field counts.)
        (counted >= LIES_ON_COUNTED).then_some(same).flatten()
    }
}

/// Of the 81 samples by which a surface is found to lie on another, how
/// many must be over it (not past a border of a freeform one).
const LIES_ON_COUNTED: usize = 16;

/// The corners of the box of the control points of a freeform surface,
/// which holds the surface (its weights are positive); `None` for the
/// other kinds.
fn hull(surface: &Surface) -> Option<(Vec3, Vec3)> {
    let points: Vec<Vec3> = match surface {
        Surface::Bezier(b) => b.control_points().to_vec(),
        Surface::Nurbs(s) => s.control_points().to_vec(),
        _ => return None,
    };
    Bbox3::from_points(points).corners()
}

/// The corners of the box of the row of control points along a border of
/// a freeform surface (`fixed_u`: the border where u is at its start or
/// `at_end`), which holds the border where the knots are clamped there;
/// `None` otherwise.
fn border_hull(surface: &Surface, fixed_u: bool, at_end: bool) -> Option<(Vec3, Vec3)> {
    let spline = match surface {
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok()?,
        Surface::Nurbs(s) => s.clone(),
        _ => return None,
    };
    let (degree_u, degree_v) = spline.degrees();
    let (count_u, count_v) = spline.counts();
    let clamped = |knots: &[f64], degree: usize, count: usize| {
        if at_end {
            knots.get(count) == knots.get(count + degree)
        } else {
            knots.first() == knots.get(degree)
        }
    };
    let ctrl = spline.control_points();
    let row: Vec<Vec3> = if fixed_u {
        if !clamped(spline.knots_u(), degree_u, count_u) {
            return None;
        }
        let i = if at_end { count_u.checked_sub(1)? } else { 0 };
        (0..count_v)
            .filter_map(|j| ctrl.get(i * count_v + j).copied())
            .collect()
    } else {
        if !clamped(spline.knots_v(), degree_v, count_v) {
            return None;
        }
        let j = if at_end { count_v.checked_sub(1)? } else { 0 };
        (0..count_u)
            .filter_map(|i| ctrl.get(i * count_v + j).copied())
            .collect()
    };
    Bbox3::from_points(row).corners()
}

/// True when two boxes are more than `margin` apart along an axis.
fn apart((alo, ahi): (Vec3, Vec3), (blo, bhi): (Vec3, Vec3), margin: f64) -> bool {
    alo.x > bhi.x + margin
        || blo.x > ahi.x + margin
        || alo.y > bhi.y + margin
        || blo.y > ahi.y + margin
        || alo.z > bhi.z + margin
        || blo.z > ahi.z + margin
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::{BezierPatch, Cylinder, Plane, Torus};
    use craft3d_math::Frame;

    /// A marcher on the plane z = 0 (its chart is x and y) at scale `k`.
    fn marcher(k: f64) -> Marcher {
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let axis = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        let rod = Surface::Cylinder(Cylinder::new(axis, k).unwrap());
        let within = Bbox3::from_points([
            Vec3::new(-3.0, -3.0, -3.0) * k,
            Vec3::new(3.0, 3.0, 3.0) * k,
        ]);
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let field = Field::new(&rod).unwrap();
        let chart = Chart::new(&plane, &within, &tol, &field).unwrap().unwrap();
        Marcher::new(chart, field, &within, &tol).unwrap()
    }

    /// A cell whose centre is exactly its reach from the field's surface
    /// (a zero on one of its nodes), or a rounding further, is not clear
    /// of zero; one further by more than the field's resolution is. Pins
    /// the rule itself, whatever rounding a given geometry meets
    /// (docs/design/booleans.md › *A plane through a node of the
    /// marcher's grid*).
    #[test]
    fn a_cell_whose_zero_is_on_a_node_is_not_clear_of_zero() {
        for k in [1e-3, 1.0, 1e6] {
            let m = marcher(k);
            for reach in [1e-3 * k, 0.37 * k, k, 2.5 * k] {
                assert!(!m.clear_of_zero(reach, reach), "k {k}, reach {reach}");
                // A centre rounded a few bits further off.
                let rounded = reach * (1.0 + 8.0 * f64::EPSILON);
                assert!(!m.clear_of_zero(rounded, reach), "k {k}, reach {reach}");
                assert!(!m.clear_of_zero(reach + 0.5 * m.on, reach), "k {k}");
                assert!(m.clear_of_zero(reach + 2.0 * m.on, reach), "k {k}");
                assert!(m.clear_of_zero(reach + m.tol.linear, reach), "k {k}");
            }
        }
    }

    /// The most evaluations the cylinder's side and the far cylinder may
    /// take before the march says it cannot place the curve
    /// (`a_march_that_comes_to_where_it_cannot_place_the_curve_says_so_at_once`).
    const EVALUATIONS_OF_THE_STALL: usize = 10_000;

    /// A marcher on a trough along y against the plane z = 0: a patch of
    /// degree two by one over x within 1 of `shift` and y in 0..1,
    /// `z = lift + a (y − ½) − (x − shift)² / 4`. Along its middle
    /// (`u = ½`) the surfaces cross at the slope `a`, and `f` is
    /// `lift + a (v − ½)`.
    fn trough(a: f64, lift: f64, shift: f64, tol: f64) -> Marcher {
        let mut ctrl = Vec::new();
        for (i, x) in [-1.0, 0.0, 1.0].into_iter().enumerate() {
            for y in [0.0, 1.0] {
                let middle = lift + a * (y - 0.5);
                let z = if i == 1 { middle + 0.25 } else { middle - 0.25 };
                ctrl.push(Vec3::new(shift + x, y, z));
            }
        }
        let patch = Surface::Bezier(BezierPatch::new(2, 1, ctrl).unwrap());
        let plane = Surface::Plane(Plane::new(Frame::WORLD));
        let within = Bbox3::from_points([
            Vec3::new(shift - 1.1, -0.1, -1.0),
            Vec3::new(shift + 1.1, 1.1, 1.0),
        ]);
        let tol = Tolerance::new(tol, 1e-9).unwrap();
        let field = Field::new(&plane).unwrap();
        let chart = Chart::new(&patch, &within, &tol, &field).unwrap().unwrap();
        Marcher::new(chart, field, &within, &tol).unwrap()
    }

    /// Where the surfaces cross steeply enough for a point within `on` of
    /// the field to be within a sixteenth of the tolerance of the curve
    /// (a sine of `16 on / tol`, 1.6e-3 here), a point within `on` is
    /// left where it is, as it was before points were held to the curve:
    /// what was marched before is marched the same.
    #[test]
    fn a_point_within_on_at_a_steep_crossing_is_left_where_it_is() {
        for a in [0.5, 2e-3] {
            let mut m = trough(a, 0.0, 0.0, 1e-6);
            assert!((m.on - 1e-10).abs() < 1e-20, "{}", m.on);
            let v = 0.5 + 0.5 * m.on / a;
            let (e, u1, v1) = m.correct(0.5, v).unwrap();
            assert!(e.f.abs() > 0.4 * m.on && e.f.abs() <= m.on, "{a}: {}", e.f);
            assert_eq!((u1, v1), (0.5, v), "slope {a}");
        }
    }

    /// Where they cross at a shallower angle than that, a point within
    /// `on` of the field (here 400 times `on` from the curve along the
    /// chart) is brought on to within `on` of the curve.
    #[test]
    fn a_point_within_on_at_a_shallow_crossing_is_brought_onto_the_curve() {
        let a = 1.2e-3;
        let mut m = trough(a, 0.0, 0.0, 1e-6);
        let v = 0.5 + 0.5 * m.on / a;
        assert!(v - 0.5 > 400.0 * m.on);
        let (e, u1, v1) = m.correct(0.5, v).unwrap();
        assert_eq!(u1, 0.5);
        assert!((v1 - 0.5).abs() <= m.on, "{} from the curve", v1 - 0.5);
        assert!(e.f.abs() <= m.on * a, "{}", e.f);
    }

    /// A point within `on` that cannot be brought nearer (the surfaces
    /// are tangent there: `f` has no slope to follow) is still the point
    /// it was, not none.
    #[test]
    fn a_point_within_on_that_cannot_be_brought_nearer_is_kept() {
        let mut m = trough(0.0, 0.5e-10, 0.0, 1e-6);
        let (e, u1, v1) = m.correct(0.5, 0.3).unwrap();
        assert!(e.f.abs() > m.floor_at(e.p) && e.f.abs() <= m.on, "{}", e.f);
        assert_eq!((u1, v1), (0.5, 0.3));
    }

    /// Where the surfaces come within `on` of each other and do not meet
    /// (the trough's ridge half of `on` under the plane), no place is as
    /// near as it is held, however long Newton goes on: the place given,
    /// the first within `on`, is the one returned, not a later one (the
    /// second try lands within `on` as well, on the other side of the
    /// ridge), as before points were held to the curve.
    #[test]
    fn of_places_within_on_none_of_them_held_the_first_is_returned() {
        let mut m = trough(0.0, -0.5e-10, 0.0, 1e-6);
        // 0.9 of `on` under the plane: 0.5 at the ridge and 0.4 down its
        // side, `x² / 4` with `x = 2 u − 1`.
        let u = 0.5 + 0.5 * (1.6e-10_f64).sqrt();
        let (e, u1, v1) = m.correct(u, 0.3).unwrap();
        assert!(e.f < -0.8 * m.on && e.f > -m.on, "{}", e.f);
        assert_eq!((u1, v1), (u, 0.3));
        // The second try is within `on` too, and is not the one returned.
        let x = 2.0 * u - 1.0;
        let second = 0.5 + 0.5 * (0.5 * x - 1e-10 / x);
        let there = m.eval(second, 0.3).unwrap();
        assert!(there.f.abs() <= m.on && second != u, "{}", there.f);
    }

    /// A cylinder's side (radius 4) in its own box, against a cylinder of
    /// radius 3e4 across it, sunk 100 tolerances: they cross in a loop 4.9
    /// long and 0.06 wide, at a sine of 7e-3 at its waist and of 8e-5 at
    /// its tips. The far cylinder's numbers are 6e4, thousands of times
    /// the box's.
    fn against_a_far_cylinder() -> (Marcher, Bbox3) {
        let side = Surface::Cylinder(Cylinder::new(Frame::WORLD, 4.0).unwrap());
        let across = Frame::new(Vec3::new(30_003.999_9, 0.0, 0.7), Vec3::Y, Vec3::X).unwrap();
        let far = Surface::Cylinder(Cylinder::new(across, 3e4).unwrap());
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        let within = Bbox3::from_points([Vec3::new(-4.2, -4.2, -5.2), Vec3::new(4.2, 4.2, 5.2)]);
        let field = Field::new(&far).unwrap();
        let chart = Chart::new(&side, &within, &tol, &field).unwrap().unwrap();
        (Marcher::new(chart, field, &within, &tol).unwrap(), within)
    }

    /// A place on the side at x = 4 where the surfaces cross at `sine`.
    fn crossing_at(x: f64, sine: f64) -> Eval {
        Eval {
            p: Vec3::new(x, 0.0, 0.7),
            su: Vec3::Y,
            sv: Vec3::Z,
            f: 0.0,
            dist: 0.0,
            n: Vec3::new((1.0 - sine * sine).sqrt(), sine, 0.0),
            gu: sine,
            gv: 0.0,
        }
    }

    /// A march that comes to where the curve cannot be placed says so,
    /// and at once. The far cylinder's distance is the difference of
    /// numbers of 3e4, told at that size (only a ball's is taken without
    /// the loss): to 5e-11, which with `on` is within a fiftieth of a
    /// tolerance of the curve at the loop's waist and 1.9 tolerances
    /// from it at a tip. The march goes from the waist towards a tip and
    /// ends where its steps can no longer be placed, in the words that
    /// say so; and it ends there for what a stall costs: the step's last
    /// resort takes no step that ended off the curve (while it did, a
    /// march crept on from such places a least step at a time: 210 938
    /// evaluations for a ball of 1e5 against this side, which is answered
    /// now).
    #[test]
    fn a_march_that_comes_to_where_it_cannot_place_the_curve_says_so_at_once() {
        let (mut m, within) = against_a_far_cylinder();
        // `on` is the box's, whatever the far cylinder's size.
        assert!((m.on - 1e-10).abs() < 1e-20, "{:e}", m.on);
        assert!(m.far && m.reach > 6e4, "{:e}", m.reach);
        let met = m.solve(&within);
        eprintln!(
            "{} evaluations, on {:e}, reach {:e}: {:?}",
            m.evals,
            m.on,
            m.reach,
            met.as_ref()
                .map(|found| (found.curves.len(), found.points.len()))
        );
        assert_eq!(met.err(), Some(unplaced()));
        assert!(
            m.evals <= EVALUATIONS_OF_THE_STALL,
            "{} evaluations",
            m.evals
        );
    }

    /// A place is judged only against a surface far larger than the box,
    /// and by what that surface's distance is told to. Against the
    /// cylinder of 3e4 (told to 5e-11 at the part; a point is asked to
    /// `on`, 1e-10): on the curve at sines of 7e-3 and of 1e-3, 0.15
    /// tolerances along the curve at most, and not at 1e-4, one and a
    /// half. (Judged by the 64 units of rounding a point is never asked
    /// nearer than, 8.5e-10 there, the place at 1e-3 would be refused as
    /// 0.85 tolerances along the curve, where the cylinder tells it to
    /// 0.15.) A part a million from the origin, its surfaces'
    /// numbers the size of its box, is not judged at all: the box's own
    /// rule holds there, as it always did, and a place at a sine of 1e-3
    /// stands (until 2026-11-21 it was refused, with every ordinary body
    /// that far out that crosses another shallowly). Without a normal of
    /// the chart there is no angle to read, and the place stands.
    #[test]
    fn a_place_is_judged_only_against_a_surface_far_larger_than_the_box() {
        let (far, _) = against_a_far_cylinder();
        assert!(far.far);
        assert!(far.placed(&crossing_at(4.0, 7e-3)));
        assert!(far.placed(&crossing_at(4.0, 1e-3)));
        assert!(!far.placed(&crossing_at(4.0, 1e-4)));
        let flat = Eval {
            su: Vec3::ZERO,
            ..crossing_at(4.0, 1e-4)
        };
        assert!(far.placed(&flat));
        // A million out, and at the origin: surfaces the size of the box.
        for shift in [1e6, 0.0] {
            let m = trough(0.0, 0.0, shift, 1e-6);
            assert!(!m.far, "{shift}: {:e}", m.reach);
            for sine in [0.1, 1e-3, 1e-5, 1e-7] {
                let e = Eval {
                    p: Vec3::new(shift, 0.5, 0.0),
                    su: Vec3::X,
                    sv: Vec3::new(0.0, 1.0, sine),
                    f: 0.0,
                    dist: 0.0,
                    n: Vec3::Z,
                    gu: 0.0,
                    gv: sine,
                };
                assert!(m.placed(&e), "{shift}, {sine}");
            }
        }
        // The box's own rule, as it was: a million out `on` is what
        // rounding leaves there, a seventieth of the tolerance.
        let out = trough(0.0, 0.0, 1e6, 1e-6);
        assert!(out.on > 1e-8 && out.on < 0.02e-6, "{:e}", out.on);
    }

    /// Two ordinary surfaces a million from the origin are not a far
    /// surface. A cylinder of 40 and one of 6 across its wall, in a box of
    /// 2 at that place: each has numbers of a million, as the box has, and
    /// a place where they cross at a sine of 1e-3 stands, under the box's
    /// own rule. (Their numbers summed are two million, twice the box's:
    /// read so, the pair was a far surface, and the place was refused.)
    #[test]
    fn two_ordinary_surfaces_a_million_from_the_origin_are_not_a_far_surface() {
        let at = Vec3::new(6e5, 6.4e5, 4.8e5);
        let towards = Vec3::new(-0.6, -0.64, 0.0).normalized().unwrap();
        let on_the_wall = at + towards * 40.0;
        let side = Surface::Cylinder(Cylinder::new(Frame::WORLD.at(at).unwrap(), 40.0).unwrap());
        let across = Frame::new(on_the_wall, Vec3::X, Vec3::Y).unwrap();
        let other = Surface::Cylinder(Cylinder::new(across, 6.0).unwrap());
        let tol = Tolerance::new(1e-6, 1e-9).unwrap();
        let pad = Vec3::new(1.0, 1.0, 1.0);
        let within = Bbox3::from_points([on_the_wall - pad, on_the_wall + pad]);
        let field = Field::new(&other).unwrap();
        let chart = Chart::new(&side, &within, &tol, &field).unwrap().unwrap();
        let m = Marcher::new(chart, field, &within, &tol).unwrap();
        assert!(m.reach > 1e6 && m.reach < 1.001e6, "{:e}", m.reach);
        assert!(!m.far);
        let e = Eval {
            p: on_the_wall,
            ..crossing_at(4.0, 1e-3)
        };
        assert!(m.placed(&e));
    }

    /// A march closes on its start within the room its points have: each
    /// may be half a tolerance from the curve. A start 0.9 of a tolerance
    /// beside a step of two tolerances is the start come back to (it was
    /// passed by, and the loop marched a second time round); one 2.5
    /// tolerances beside it, or behind, or heading the other way, is not.
    #[test]
    fn a_march_closes_on_its_start_within_the_room_its_points_have() {
        let m = trough(0.0, 0.0, 0.0, 1e-6);
        let tol = m.tol.linear;
        let at = |x: f64, y: f64, heading: f64| Pt {
            u: 0.0,
            v: 0.0,
            p: Vec3::new(x, y, 0.0),
            t: Vec3::new(heading, 0.0, 0.0),
            du: heading,
            dv: 0.0,
        };
        let cur = at(0.0, 0.0, 1.0);
        let len = 2.0 * tol;
        assert!(m.closes_on(&at(1.2 * tol, 0.9 * tol, 1.0), &cur, len));
        assert!(m.closes_on(&at(1.2 * tol, 0.0, 1.0), &cur, len));
        assert!(!m.closes_on(&at(1.2 * tol, 2.5 * tol, 1.0), &cur, len));
        assert!(!m.closes_on(&at(-1.2 * tol, 0.0, 1.0), &cur, len));
        assert!(!m.closes_on(&at(2.5 * tol, 0.0, 1.0), &cur, len));
        assert!(!m.closes_on(&at(1.2 * tol, 0.0, -1.0), &cur, len));
    }

    /// No point is asked nearer `f = 0` than rounding leaves at the size
    /// of the box: a million from the origin at a tolerance of 1e-5, `on`
    /// is that floor itself, and a point within it at a shallow crossing
    /// is left where it is.
    #[test]
    fn a_point_is_not_asked_nearer_than_rounding_leaves() {
        let a = 1e-3;
        let mut m = trough(a, 0.0, 1e6, 1e-5);
        assert!(m.on > 1e-9, "{}", m.on);
        let v = 0.5 + 0.5 * m.on / a;
        let (e, u1, v1) = m.correct(0.5, v).unwrap();
        assert!(m.floor_at(e.p) >= m.on && m.held(&e) == m.on, "{}", m.on);
        assert!(e.f.abs() > 0.4 * m.on && e.f.abs() <= m.on, "{}", e.f);
        assert_eq!((u1, v1), (0.5, v));
    }

    /// At a place where the chart has no normal (a pole: one of its
    /// derivatives vanishes) there is no angle to read: a point is held
    /// to `on`, as everywhere before. With one, at a slope of 1e-5, to
    /// what rounding leaves (less than `on` times that sine would be).
    #[test]
    fn a_point_where_the_chart_has_no_normal_is_held_to_on() {
        let m = trough(1e-5, 0.0, 0.0, 1e-6);
        let e = Eval {
            p: Vec3::ZERO,
            su: Vec3::ZERO,
            sv: Vec3::Y,
            f: 0.0,
            dist: 0.0,
            n: Vec3::Z,
            gu: 0.0,
            gv: 0.0,
        };
        assert_eq!(m.held(&e), m.on);
        let e = Eval {
            su: Vec3::X,
            sv: Vec3::new(0.0, 1.0, 1e-5),
            ..e
        };
        assert_eq!(m.held(&e), m.floor_at(e.p));
        assert!(
            m.floor_at(e.p) < 1e-3 * m.on,
            "{} {}",
            m.floor_at(e.p),
            m.on
        );
    }

    /// The arc of the circle about `centre` (in the plane) of `radius` over
    /// the angles `from..to`, as a branch.
    fn arc(centre: (f64, f64), radius: f64, from: f64, to: f64) -> Branch {
        let pts = (0..=40)
            .map(|i| {
                let a = from + (to - from) * f64::from(i) / 40.0;
                let (s, c) = a.psin_cos();
                let (x, y) = (centre.0 + radius * c, centre.1 + radius * s);
                Pt {
                    u: x,
                    v: y,
                    p: Vec3::new(x, y, 0.0),
                    t: Vec3::new(-s, c, 0.0),
                    du: -s,
                    dv: c,
                }
            })
            .collect();
        Branch { pts, closed: false }
    }

    /// Two branches that touch, or run within a few tolerances of each
    /// other, are not one: circles tangent at a point (as the arms of a
    /// tacnode leave theirs), arcs of one radius about centres three
    /// tolerances apart, and a short arc tangent to a long one that lies
    /// within the tolerance of it all its length (an arm cut short by the
    /// box) without reaching past either end of it. At three scales.
    #[test]
    fn branches_that_only_touch_or_run_close_are_not_one() {
        for k in [1e-3, 1.0, 1e3] {
            let mut m = marcher(k);
            let tol = m.tol.linear;
            // Tangent at (k, 0), a hundredth of the radius apart across.
            let inner = arc((0.0, 0.0), k, -0.5, 0.5);
            let outer = arc((-0.01 * k, 0.0), 1.01 * k, -0.5, 0.5);
            assert!(
                m.united(&inner, &outer).unwrap().is_none(),
                "touching at {k}"
            );
            assert!(
                m.united(&outer, &inner).unwrap().is_none(),
                "touching at {k}"
            );
            // Three tolerances apart all along.
            let beside = arc((3.0 * tol, 0.0), k, -0.5, 0.5);
            assert!(
                m.united(&inner, &beside).unwrap().is_none(),
                "beside at {k}"
            );
            // The tangent circle's arc, short: within 2e-8 of the inner
            // circle all along (a hundredth of the radius over 2 times its
            // angle squared), well within the tolerance.
            let short = arc((-0.01 * k, 0.0), 1.01 * k, -0.002, 0.002);
            assert!(
                m.united(&inner, &short).unwrap().is_none(),
                "short arm at {k}"
            );
        }
    }

    /// A wall round the unit circle (a rational quadratic circle extruded
    /// up to `z = 1`) whose lower border is below `z = 0` but for two
    /// bumps, about 0 and 180 degrees, where it rises above it (the seam
    /// where the circle closes inside the first): against the plane
    /// `z = 0`, the meeting is the circle less the bumps.
    fn bumped_wall(k: f64) -> Surface {
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let ring = [
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (1.0, 0.0),
        ];
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for (i, (x, y)) in ring.into_iter().enumerate() {
            let low = if i == 0 || i == 4 || i == 8 {
                0.6
            } else {
                -0.3
            };
            ctrl.push(Vec3::new(x, y, low) * k);
            ctrl.push(Vec3::new(x, y, 1.0) * k);
            let weight = if i % 2 == 1 { w } else { 1.0 };
            weights.extend([weight, weight]);
        }
        Surface::Nurbs(
            NurbsSurface::new(
                2,
                1,
                vec![
                    0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
                ],
                vec![0.0, 0.0, 1.0, 1.0],
                9,
                2,
                ctrl,
                weights,
            )
            .unwrap(),
        )
    }

    /// A closed branch that leaves the field's surface twice, starting
    /// where it leaves it the first time (on the edge, so that nothing of
    /// the piece before is on the surface): two pieces, open, each on the
    /// surface, neither bridging a bump; the first is not joined on to
    /// the last across the bump the loop starts at (review of
    /// 2026-10-28). At three scales.
    #[test]
    fn a_closed_branch_that_starts_on_the_edge_it_leaves_by_is_not_joined_across_it() {
        for k in [1e-3, 1.0, 1e6] {
            let plane = Surface::Plane(Plane::new(Frame::WORLD));
            let wall = bumped_wall(k);
            let within = Bbox3::from_points([
                Vec3::new(-3.0, -3.0, -3.0) * k,
                Vec3::new(3.0, 3.0, 3.0) * k,
            ]);
            let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
            let field = Field::new(&wall).unwrap();
            let chart = Chart::new(&plane, &within, &tol, &field).unwrap().unwrap();
            let mut m = Marcher::new(chart, field, &within, &tol).unwrap();
            // Where the border crosses `z = 0` (angles, by halving on u):
            // the first bump, about 0, from `rise` to `fall`.
            let z = |u: f64| wall.eval(u, 0.0).z;
            let cross = |mut lo: f64, mut hi: f64| {
                for _ in 0..200 {
                    let mid = 0.5 * (lo + hi);
                    if (z(mid) > 0.0) == (z(lo) > 0.0) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let p = wall.eval(0.5 * (lo + hi), 0.0);
                p.y.patan2(p.x)
            };
            let (rise, fall) = (cross(0.75, 1.0), cross(0.0, 0.25));
            assert!(
                rise < -0.3 && fall > 0.3 && (rise + fall).abs() < 1e-9,
                "{rise} {fall}"
            );
            let point = |a: f64| {
                let (s, c) = a.psin_cos();
                Pt {
                    u: k * c,
                    v: k * s,
                    p: Vec3::new(c, s, 0.0) * k,
                    t: Vec3::new(-s, c, 0.0),
                    du: -s,
                    dv: c,
                }
            };
            // The start just past the rise, short of the edge's cut by
            // half of `on`: no point beyond it, further than `on`, is on
            // the surface (by halving on its distance).
            let target = m.cut - 0.5 * m.on;
            let (mut lo, mut hi) = (rise, rise + 0.1);
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if m.field.sample(point(mid).p).unwrap().dist < target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let start = lo;
            // From there round to it again, passing over both bumps in
            // one span each.
            let tau = std::f64::consts::TAU;
            let angles = [
                start,
                fall + 0.2,
                fall + 0.9,
                rise + std::f64::consts::PI - 0.2,
                fall + std::f64::consts::PI + 0.2,
                fall + std::f64::consts::PI + 0.9,
                start + tau,
            ];
            let pts: Vec<Pt> = angles.iter().map(|&a| point(a)).collect();
            let pieces = m.refine_pieces(pts, true).unwrap();
            let described: Vec<String> = pieces
                .iter()
                .map(|(p, _)| {
                    let angle = |q: &Pt| q.p.y.patan2(q.p.x);
                    let (f, l) = (p.first().unwrap(), p.last().unwrap());
                    format!(
                        "{} pts {:.6}..{:.6} (u {:.6}..{:.6})",
                        p.len(),
                        angle(f),
                        angle(l),
                        f.u,
                        l.u
                    )
                })
                .collect();
            assert_eq!(
                pieces.len(),
                2,
                "{k}: start {start} rise {rise} fall {fall}: {described:#?}"
            );
            let in_bump = |a: f64| {
                let pi = std::f64::consts::PI;
                let near = |c: f64| (a - c + pi).rem_euclid(tau) - pi;
                near(0.0).abs() < fall - 1e-6 || near(pi).abs() < fall - 1e-6
            };
            for (piece, closed) in &pieces {
                assert!(!closed, "{k}");
                for p in piece {
                    let off = m.field.sample(p.p).unwrap();
                    assert!(off.dist <= tol.linear, "{k}: {:?} off by {}", p.p, off.dist);
                }
                // No span passes over a bump: the angles run on by less
                // than a bump's width each, and none is inside one.
                for (a, b) in piece.iter().zip(piece.iter().skip(1)) {
                    let (ta, tb) = (a.p.y.patan2(a.p.x), b.p.y.patan2(b.p.x));
                    let step = (tb - ta).rem_euclid(tau);
                    assert!(step < fall - rise, "{k}: a span of {step} rad");
                    let middle = (a.p + b.p) * 0.5;
                    assert!(!in_bump(middle.y.patan2(middle.x)), "{k}: {middle:?}");
                }
            }
        }
    }

    /// The heights of a corrugated sheet's profile at its control points:
    /// five crests of uneven height, the troughs between them, and its
    /// ends well below.
    const PROFILE: [f64; 11] = [-0.6, 0.30, 0.0, 0.24, 0.0, 0.32, 0.0, 0.26, 0.0, 0.31, -0.6];

    /// A corrugated sheet walked against a plane level with its crests
    /// that rises a little along them, at the scale `k` (a pair of
    /// `crates/geom/tests/meet_beside.rs` › `corrugated`). The sheet: the
    /// cubic spline of [`PROFILE`] over `x` in 0..10, drawn out along `y`
    /// over 0..100 to the same profile at 0.9 of its height (a loft
    /// between two sections). The plane: `z = 0.11 + 0.0012 y`, above
    /// every crest at the sheet's far end. They cross in curves side by
    /// side that begin and end on the sheet's near edge: a hairpin over
    /// a crest, up one side of it, round where the plane is level with
    /// the crest line, and back down the other; and, where the trough
    /// between two crests is above the plane at the edge, one curve over
    /// both, which turns once more where the plane is level with that
    /// trough, a little way in from the edge.
    /// `turned`: the plane's normal the other way, so that the distance
    /// has the other sign, and what a march from a place calls forward
    /// is backward.
    fn sheet(k: f64, turned: bool) -> Marcher {
        let mut knots = vec![0.0; 4];
        knots.extend((1..8).map(|i| f64::from(i) / 8.0));
        knots.extend([1.0; 4]);
        let mut ctrl = Vec::new();
        for (i, height) in PROFILE.iter().enumerate() {
            let across = (knots[i + 1] + knots[i + 2] + knots[i + 3]) / 3.0;
            ctrl.push(Vec3::new(10.0 * across, 0.0, *height) * k);
            ctrl.push(Vec3::new(10.0 * across, 100.0, 0.9 * height) * k);
        }
        let skin = Surface::Nurbs(
            NurbsSurface::new(
                3,
                1,
                knots,
                vec![0.0, 0.0, 1.0, 1.0],
                11,
                2,
                ctrl,
                vec![1.0; 22],
            )
            .unwrap(),
        );
        let along = Vec3::new(0.0, 1.0, 0.0012).normalized().unwrap();
        let normal = if turned {
            along.cross(Vec3::X)
        } else {
            Vec3::X.cross(along)
        };
        let flat = Frame::new(Vec3::new(0.0, 0.0, 0.11) * k, normal, Vec3::X).unwrap();
        let flat = Surface::Plane(Plane::new(flat));
        let (centre, half) = (
            Vec3::new(5.0, 50.0, 0.0) * k,
            Vec3::new(52.0, 52.0, 52.0) * k,
        );
        let within = Bbox3::from_points([centre - half, centre + half]);
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let field = Field::new(&flat).unwrap();
        let chart = Chart::new(&skin, &within, &tol, &field).unwrap().unwrap();
        Marcher::new(chart, field, &within, &tol).unwrap()
    }

    /// The stretches across the sheet over which its near edge is above
    /// the plane, by the sheet's own profile there: one curve of the
    /// crossing over each.
    fn strands(m: &Marcher, k: f64) -> Vec<(f64, f64)> {
        let above = |i: i32| m.chart.surface.eval(f64::from(i) / 8000.0, 0.0).z / k > 0.11;
        let across = |i: i32| 10.0 * f64::from(i) / 8000.0;
        let (mut found, mut from) = (Vec::new(), None);
        for i in 0..=8000 {
            match (from, above(i)) {
                (None, true) => from = Some(i),
                (Some(first), false) => {
                    found.push((across(first), across(i)));
                    from = None;
                }
                _ => {}
            }
        }
        assert!(from.is_none() && found.len() >= 3, "{found:?}");
        found
    }

    /// Which curve of the crossing a place is on: the stretch its `x` is
    /// in, or nearest.
    fn strand(p: Vec3, strands: &[(f64, f64)], k: f64) -> usize {
        let apart = |(a, b): (f64, f64)| (a - p.x / k).max(p.x / k - b).max(0.0);
        let mut nearest = 0;
        for (i, stretch) in strands.iter().enumerate() {
            if apart(*stretch) < apart(strands[nearest]) {
                nearest = i;
            }
        }
        nearest
    }

    /// How many steps of a run from `start` end on another curve of the
    /// crossing than they began on.
    fn steps_beside(start: &Pt, run: &[Pt], strands: &[(f64, f64)], k: f64) -> usize {
        let mut pts = vec![*start];
        pts.extend(run.iter().copied());
        pts.windows(2)
            .filter(|w| strand(w[0].p, strands, k) != strand(w[1].p, strands, k))
            .count()
    }

    /// A branch as `trace` first marches it from `seed`: its start, the
    /// forward run with how it ended, and the backward run (none where
    /// the forward run closed).
    fn first_march(m: &mut Marcher, seed: (f64, f64)) -> (Pt, Vec<Pt>, End, Vec<Pt>) {
        let (e, u, v) = m.correct(seed.0, seed.1).unwrap();
        let start = Marcher::point(&e, u, v, 1.0).unwrap();
        let (forward, end) = m.run(&start, 1.0, &[], true).unwrap();
        let backward = if end == End::Closed {
            Vec::new()
        } else {
            m.run(&start.reversed(), -1.0, &[], false).unwrap().0
        };
        (start, forward, end, backward)
    }

    /// The first of the seeds the sheet's cells give whose march, as it
    /// is first made (by a marcher that has done nothing else, as the
    /// tests' are), steps onto another curve of the crossing.
    fn a_seed_whose_march_steps_beside(k: f64) -> (f64, f64) {
        let mut cells = sheet(k, false);
        let strands = strands(&cells, k);
        cells.border_stretches().unwrap();
        let mut singular = Vec::new();
        let seeds = cells.seeds(&mut singular).unwrap();
        for seed in seeds {
            let mut m = sheet(k, false);
            let Some((e, u, v)) = m.correct(seed.0, seed.1) else {
                continue;
            };
            let Some(start) = Marcher::point(&e, u, v, 1.0).filter(|_| m.inside(u, v)) else {
                continue;
            };
            let Ok((forward, End::Open)) = m.run(&start, 1.0, &[], true) else {
                continue;
            };
            let Ok((backward, _)) = m.run(&start.reversed(), -1.0, &[], false) else {
                continue;
            };
            let stepped = steps_beside(&start, &forward, &strands, k)
                + steps_beside(&start.reversed(), &backward, &strands, k);
            if stepped > 0 {
                return (u, v);
            }
        }
        panic!("{k}: no march from the sheet's seeds steps onto another curve");
    }

    /// How far along the sheet a curve of the crossing reaches: where the
    /// plane is level with the highest crest line over its stretch, by
    /// the sheet's own profile along its near edge.
    fn reach_of(m: &Marcher, stretch: (f64, f64), k: f64) -> f64 {
        let mut top = f64::NEG_INFINITY;
        for i in 0..=20_000 {
            let x = stretch.0 + (stretch.1 - stretch.0) * f64::from(i) / 20_000.0;
            top = top.max(m.chart.surface.eval(x / 10.0, 0.0).z / k);
        }
        100.0 * (top - 0.11) / (0.12 + 0.1 * top)
    }

    /// A step of the march onto the curve beside the one it follows: the
    /// branch is marched again from the point before it, in shorter
    /// steps, and comes back as the curve its seed is on. On the sheet a
    /// march down a side of a curve, in steps of up to 5.6, comes within
    /// a few of the sheet's near edge, and its last step, which leaves
    /// the sheet, is put on the edge where the crossing is found from
    /// where the step pointed: at the foot of another curve (as traced
    /// on these pairs; the assertion below holds only that a step of the
    /// first march ends on another curve than it began on: the march
    /// steps so, or this tests nothing). The fit finds no points between
    /// two points that are on two curves, and the pair was refused, "an
    /// intersection curve could not be fitted within the tolerance".
    /// Traced, the branch is the seed's own curve: open, from the sheet's
    /// near edge up to where the plane is level with its highest crest
    /// and back to the edge. With the plane as it is and turned over, so
    /// that the wrong step is a step of the forward run from the one and
    /// of the backward run from the other; at three scales.
    #[test]
    fn a_step_onto_the_curve_beside_is_marched_again_in_shorter_steps() {
        for k in [1e-3, 1.0, 1e6] {
            let seed = a_seed_whose_march_steps_beside(k);
            let mut where_stepped = Vec::new();
            for turned in [false, true] {
                let mut m = sheet(k, turned);
                let strands = strands(&m, k);
                let (start, forward, end, backward) = first_march(&mut m, seed);
                assert_eq!(end, End::Open, "{k}, {turned}");
                where_stepped.push((
                    steps_beside(&start, &forward, &strands, k),
                    steps_beside(&start.reversed(), &backward, &strands, k),
                ));
                let mut m = sheet(k, turned);
                let branches = m.trace(seed, &[]).unwrap();
                assert_eq!(branches.len(), 1, "{k}, {turned}");
                let branch = &branches[0];
                assert!(!branch.closed, "{k}, {turned}");
                let own = strand(start.p, &strands, k);
                assert!(
                    branch.pts.iter().all(|p| strand(p.p, &strands, k) == own),
                    "{k}, {turned}: not all of it on curve {own} of {strands:?}"
                );
                let (first, last) = (branch.pts[0].p, branch.pts[branch.pts.len() - 1].p);
                assert!(
                    first.y.abs() <= 1e-3 * k && last.y.abs() <= 1e-3 * k,
                    "{k}, {turned}: from {first:?} to {last:?}"
                );
                assert!(
                    (first.x - last.x).abs() > 0.1 * k,
                    "{k}, {turned}: both ends at one foot"
                );
                let reach = reach_of(&m, strands[own], k);
                let top = branch
                    .pts
                    .iter()
                    .map(|p| p.p.y / k)
                    .fold(f64::NEG_INFINITY, f64::max);
                assert!(
                    top > 0.999 * reach && top < reach + 1e-3,
                    "{k}, {turned}: up to {top}, the curve to {reach}"
                );
            }
            // The wrong steps are the forward run's with the plane one
            // way up and the backward run's with it the other.
            where_stepped.sort_unstable();
            eprintln!("{k}: steps onto another curve, forward and backward: {where_stepped:?}");
            assert!(
                matches!(where_stepped[..], [(0, back), (forth, 0)] if back >= 1 && forth >= 1),
                "{k}: {where_stepped:?}"
            );
        }
    }

    /// Asked as it was before branches were taken up again
    /// ([`Marcher::once`]), the march refuses where the fit gives up: the
    /// sheet's branch that steps onto another curve, in the fit's words.
    #[test]
    fn a_march_asked_once_refuses_where_the_fit_gives_up() {
        for k in [1e-3, 1.0, 1e6] {
            let seed = a_seed_whose_march_steps_beside(k);
            let mut m = sheet(k, false).once();
            assert_eq!(m.trace(seed, &[]).err(), Some(not_fitted()), "{k}");
        }
    }

    /// The fit tells which span of the points it was given it gave up on,
    /// whatever it had put between them and before them by then: the two
    /// points of the step onto another curve, as the march made them.
    #[test]
    fn the_fit_names_the_span_of_the_points_it_was_given_that_it_gave_up_on() {
        for k in [1e-3, 1.0, 1e6] {
            let seed = a_seed_whose_march_steps_beside(k);
            let mut m = sheet(k, false);
            let strands = strands(&m, k);
            let (start, forward, _, backward) = first_march(&mut m, seed);
            // The run that stepped onto another curve, from the start on.
            let run = if steps_beside(&start, &forward, &strands, k) > 0 {
                let mut pts = vec![start];
                pts.extend(forward);
                pts
            } else {
                let mut pts = vec![start.reversed()];
                pts.extend(backward);
                pts
            };
            let wrong = run
                .windows(2)
                .position(|w| strand(w[0].p, &strands, k) != strand(w[1].p, &strands, k))
                .unwrap();
            let mut pts = run.clone();
            let refused = m.refine_to_edge(&mut pts).err().unwrap();
            assert_eq!(refused, not_fitted(), "{k}");
            let (a, b) = m.unfitted.unwrap();
            assert_eq!((a.u, a.v), (run[wrong].u, run[wrong].v), "{k}");
            assert_eq!((b.u, b.v), (run[wrong + 1].u, run[wrong + 1].v), "{k}");
            assert_eq!(Marcher::span_of(&run, &a, &b), Some(wrong), "{k}");
        }
    }

    /// Across the sheet, from a trough to the crest line after it, for
    /// the crests at 2.5, 5 and 7.5: on that side of each crest the
    /// curve over it runs up the sheet the same way.
    const SIDES: [(f64, f64); 3] = [(1.474, 2.5), (3.75, 5.0), (6.25, 7.5)];

    /// The place on that side of a crest's curve, `y` along the sheet,
    /// travelling up it.
    fn up_the_side(m: &mut Marcher, crest: usize, y: f64, k: f64) -> Pt {
        let (mut lo, mut hi) = (SIDES[crest].0 / 10.0, SIDES[crest].1 / 10.0);
        let v = y / 100.0;
        let below = m.eval(lo, v).unwrap().f < 0.0;
        assert!((m.eval(hi, v).unwrap().f < 0.0) != below);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if (m.eval(mid, v).unwrap().f < 0.0) == below {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (e, u, v) = m.correct(0.5 * (lo + hi), v).unwrap();
        let pt = Marcher::point(&e, u, v, 1.0).unwrap();
        assert!((pt.p.y - y * k).abs() < 0.1 * k);
        if pt.t.y < 0.0 { pt.reversed() } else { pt }
    }

    /// A run is put right from the wrong step nearest its start. The fit
    /// meets a backward run's spans from the far end, so the span it
    /// gives up on first there is the run's last wrong step; taken up
    /// from that one the run would follow the curve it was wrongly on,
    /// and be as wrong as it was up to there. Six points up three curves
    /// of the sheet, two on each (over the crests at 2.5, 5 and 7.5, on
    /// the side of each that heads the same way), as a run would hold
    /// them had it stepped from the first curve onto the second and from
    /// the second onto the third: the fit gives up on the first of the
    /// two steps; with the start at the list's end (a backward run) the
    /// step to put right is the second, and with the start at the second
    /// curve's first point (the first step then the backward run's only
    /// one) it is the first.
    #[test]
    fn a_backward_run_is_put_right_from_the_wrong_step_nearest_its_start() {
        for k in [1e-3, 1.0, 1e6] {
            let mut m = sheet(k, false);
            let pts = [
                up_the_side(&mut m, 0, 5.0, k),
                up_the_side(&mut m, 0, 10.0, k),
                up_the_side(&mut m, 1, 15.0, k),
                up_the_side(&mut m, 1, 20.0, k),
                up_the_side(&mut m, 2, 25.0, k),
                up_the_side(&mut m, 2, 30.0, k),
            ];
            let mut fitted = pts.to_vec();
            assert!(m.refine_to_edge(&mut fitted).is_err(), "{k}");
            let (a, b) = m.unfitted.unwrap();
            assert_eq!(Marcher::span_of(&pts, &a, &b), Some(1), "{k}");
            let (nearest, from, to) = m.nearest_the_start(&pts, 5, 1, a, b).unwrap();
            assert_eq!(nearest, 3, "{k}");
            assert_eq!((from.u, to.u), (pts[3].u, pts[4].u), "{k}");
            let (nearest, ..) = m.nearest_the_start(&pts, 2, 1, a, b).unwrap();
            assert_eq!(nearest, 1, "{k}");
        }
    }

    /// A plane along the axis of a thin ring (a torus of radii 10 and
    /// 0.03, at the scale `k`), `inside` nearer the axis than the plane
    /// that touches the ring's inner equator (in the ring's lengths at
    /// scale 1), walked against the ring in a box that reaches from
    /// `across.0` to `across.1` across the axis. The crossing is two long
    /// ovals, one either side of the axis, 0.06 high: at an `inside` of
    /// 0.01 from 0.45 to 1.18 from the axis. (A plain closed curve with
    /// long sides, for what a march does along one.)
    fn pinch(k: f64, inside: f64, across: (f64, f64)) -> Marcher {
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let d = (9.97 - inside) * k;
        let flat = Frame::new(Vec3::new(d, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
        let flat = Surface::Plane(Plane::new(flat));
        let ring = Surface::Torus(Torus::new(Frame::WORLD, 10.0 * k, 0.03 * k).unwrap());
        let within = Bbox3::from_points([
            Vec3::new(d - 0.1 * k, across.0 * k, -0.1 * k),
            Vec3::new(d + 0.1 * k, across.1 * k, 0.1 * k),
        ]);
        let field = Field::new(&ring).unwrap();
        let chart = Chart::new(&flat, &within, &tol, &field).unwrap().unwrap();
        Marcher::new(chart, field, &within, &tol).unwrap()
    }

    /// The place of the oval at `y` across the axis, on its upper side,
    /// as a seed.
    fn on_oval(m: &Marcher, k: f64, inside: f64, y: f64) -> (f64, f64) {
        let d = 9.97 - inside;
        let from_circle = (d * d + y * y).sqrt() - 10.0;
        let z = (0.03 * 0.03 - from_circle * from_circle).sqrt();
        m.chart.surface.project(Vec3::new(d, y, z) * k).unwrap()
    }

    /// Where the fit brings every span within the tolerance, a branch is
    /// marched once and comes back as it did before a branch could be
    /// taken up again: the same points bit for bit, for the same
    /// evaluations, as the two runs put together and fitted here by hand.
    /// An oval of the thin ring, whole (a closed branch) and cut by the
    /// box (an open one, with a backward run), at three scales.
    #[test]
    fn a_branch_the_fit_accepts_is_marched_once_and_is_the_same_bit_for_bit() {
        for k in [1e-3, 1.0, 1e6] {
            for (across, closed) in [((-3.0, 3.0), true), ((-3.0, 0.9), false)] {
                let mut by_hand = pinch(k, 0.01, across);
                let seed = on_oval(&by_hand, k, 0.01, 0.8);
                let (start, forward, end, backward) = first_march(&mut by_hand, seed);
                assert_eq!(end == End::Closed, closed, "{k}");
                let mut pts: Vec<Pt> = backward.iter().rev().copied().map(Pt::reversed).collect();
                pts.push(start);
                pts.extend(forward.iter().copied());
                if closed {
                    let last = pts[pts.len() - 1];
                    let (u, v) = by_hand.near((start.u, start.v), &last);
                    pts.push(Pt { u, v, ..start });
                }
                let fitted = by_hand.refine_pieces(pts, closed).unwrap();
                assert_eq!(fitted.len(), 1, "{k}");
                let mut traced = pinch(k, 0.01, across);
                let branches = traced.trace(seed, &[]).unwrap();
                assert_eq!(branches.len(), 1, "{k}");
                assert_eq!(branches[0].closed, fitted[0].1, "{k}");
                let bits = |pts: &[Pt]| -> Vec<[u64; 10]> {
                    pts.iter()
                        .map(|p| {
                            [
                                p.u, p.v, p.p.x, p.p.y, p.p.z, p.t.x, p.t.y, p.t.z, p.du, p.dv,
                            ]
                            .map(f64::to_bits)
                        })
                        .collect()
                };
                assert_eq!(bits(&branches[0].pts), bits(&fitted[0].0), "{k}");
                assert_eq!(traced.evals, by_hand.evals, "{k}");
                assert!(traced.unfitted.is_none(), "{k}");
            }
        }
    }

    /// A march taken up again past a span the fit gave up on keeps to
    /// steps of a quarter of that span until it has gone twice the span's
    /// length, and to no more than that: beyond it the steps grow again
    /// as any march's do. Along an oval of the thin ring, from the middle
    /// of its upper side, past a span of an eighth of the longest step:
    /// no step in that stretch is longer than twice the quarter (a step
    /// ends no further than twice what it was asked), there are four
    /// steps or more in it, and later steps are longer; a march not
    /// taken up again starts with a longer step than that at once.
    #[test]
    fn a_march_taken_up_again_keeps_to_shorter_steps_over_a_stretch_and_no_further() {
        for k in [1e-3, 1.0, 1e6] {
            let mut m = pinch(k, 0.01, (-3.0, 3.0));
            let seed = on_oval(&m, k, 0.01, 0.8);
            let (e, u, v) = m.correct(seed.0, seed.1).unwrap();
            let start = Marcher::point(&e, u, v, 1.0).unwrap();
            let span = m.hmax / 8.0;
            let again = Again::past(start, 0, span);
            let (cap, over) = (again.cap, again.over);
            assert_eq!((cap, over), (0.25 * span, 2.0 * span), "{k}");
            let (pts, _) = m.run_on(&start, 1.0, &[], true, Some(again)).unwrap();
            let (mut gone, mut held, mut longest_after) = (0.0, 0, 0.0_f64);
            let mut before = start;
            for p in &pts {
                let step = p.p.distance(before.p);
                if gone < over {
                    assert!(step <= 2.0 * cap * (1.0 + 1e-9), "{k}: {step} at {gone}");
                    held += 1;
                } else {
                    longest_after = longest_after.max(step);
                }
                gone += step;
                before = *p;
            }
            assert!(held >= 4, "{k}: {held} steps");
            assert!(longest_after > 3.0 * cap, "{k}: {longest_after}");
            let (plain, _) = m.run(&start, 1.0, &[], true).unwrap();
            assert!(plain[0].p.distance(start.p) > 2.0 * cap, "{k}");
        }
    }

    /// A point of a branch at `p` heading along `t`, with a tangent of its
    /// own; `u` tells it from the others.
    fn placed_at(u: f64, p: Vec3, t: Vec3) -> Pt {
        Pt {
            u,
            v: 0.0,
            p,
            t,
            du: 1.0,
            dv: 0.0,
        }
    }

    /// Points along a path of places given in tolerances in the plane
    /// z = 0, each heading to the next, `u` counting from `first`.
    fn path(places: &[(f64, f64)], tol: f64, first: f64) -> Vec<Pt> {
        let p: Vec<Vec3> = places
            .iter()
            .map(|&(x, y)| Vec3::new(x, y, 0.0) * tol)
            .collect();
        (0..p.len())
            .map(|i| {
                let next = if i + 1 < p.len() {
                    p[i + 1] - p[i]
                } else {
                    p[i] - p[i - 1]
                };
                placed_at(first + i as f64, p[i], next.normalized().unwrap())
            })
            .collect()
    }

    /// What runs back over itself and what does not: two spans in a row
    /// whose chords turn by more than 120 degrees, each longer than four
    /// tolerances, at a point with a tangent of its own. Not a right
    /// angle, nor the corners of random design 102900 that the probe of
    /// 2026-11-22 flagged as turning (a run's last step put on the border
    /// of a fillet's surface, at a cosine of −0.0023 and −0.0009 to the
    /// step before it, with a span of a few hundredths of a tolerance),
    /// nor a turn back over a span of three tolerances.
    #[test]
    fn a_branch_runs_back_over_itself_where_two_spans_turn_back_not_at_a_corner() {
        for k in [1e-3, 1.0, 1e6] {
            let m = marcher(k);
            let t = m.tol.linear;
            // On along x, then back: at the third point.
            let back = path(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (8.0, 0.0)], t, 0.0);
            assert_eq!(m.folds(&back), vec![2], "{k}");
            // A turn of a cosine of −0.6 is one; a right angle is not, nor
            // one a hair past it (−0.002, as 102900's corners).
            let (c, s) = (-0.6, 0.8);
            let turn = path(
                &[(0.0, 0.0), (10.0, 0.0), (10.0 + 9.0 * c, 9.0 * s)],
                t,
                0.0,
            );
            assert_eq!(m.folds(&turn), vec![1], "{k}");
            let right = path(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], t, 0.0);
            assert!(m.folds(&right).is_empty(), "{k}");
            let (c, s) = (-0.002_f64, (1.0 - 0.002_f64 * 0.002).sqrt());
            let corner = path(
                &[(0.0, 0.0), (9.36, 0.0), (9.36 + 9.2 * c, 9.2 * s)],
                t,
                0.0,
            );
            assert!(m.folds(&corner).is_empty(), "{k}");
            // Back over three tolerances: too short to say.
            let short = path(&[(0.0, 0.0), (10.0, 0.0), (7.0, 0.0)], t, 0.0);
            assert!(m.folds(&short).is_empty(), "{k}");
            // At a point without a tangent of its own (a crossing point).
            let mut through = back.clone();
            through[2].du = 0.0;
            assert!(m.folds(&through).is_empty(), "{k}");
        }
        // 102900's first corner as the probe printed it (tolerance 1e-6):
        // the five points from the border on.
        let m = marcher(1.0);
        let corner: Vec<Pt> = [
            (
                0.041_823_836_433_878_48,
                [
                    -2.837_369_248_835_125,
                    -4.321_401_397_486_659,
                    -2.477_816_771_524_89,
                ],
            ),
            (
                0.041_823_788_061_226_96,
                [
                    -2.837_369_645_527_925,
                    -4.321_410_714_629_835,
                    -2.477_815_953_588_798,
                ],
            ),
            (
                0.041_823_792_256_133_07,
                [
                    -2.837_369_608_418_528_7,
                    -4.321_410_716_237_321,
                    -2.477_815_954_893_619,
                ],
            ),
            (
                0.041_823_800_645_040_744,
                [
                    -2.837_369_534_207_343_5,
                    -4.321_410_719_569_461,
                    -2.477_815_957_492_874_4,
                ],
            ),
            (
                0.041_823_817_422_585_43,
                [
                    -2.837_369_385_787_716_7,
                    -4.321_410_726_129_645_5,
                    -2.477_815_962_700_242_7,
                ],
            ),
        ]
        .into_iter()
        .map(|(u, [x, y, z])| placed_at(u, Vec3::new(x, y, z), Vec3::X))
        .collect();
        assert!(m.folds(&corner).is_empty());
    }

    /// The step a fold is put on: the step that came to the marched point
    /// it is at (the forward run's from the point before, the backward
    /// run's from the point after), or the step it lies inside; none at
    /// the start, where no step came; none where nothing runs back. And of
    /// two, the one nearest the start.
    #[test]
    fn a_fold_is_put_on_the_step_that_came_to_it_or_that_holds_it() {
        let m = marcher(1.0);
        let t = m.tol.linear;
        let one = |pts: &[Pt]| vec![(pts.to_vec(), false)];
        // The forward run (from the third point) steps from 30 to 60 and
        // goes back: the fold at 60 is put on the step from 30.
        let forward = path(
            &[
                (0.0, 0.0),
                (10.0, 0.0),
                (20.0, 0.0),
                (30.0, 0.0),
                (60.0, 0.0),
                (50.0, 0.0),
            ],
            t,
            0.0,
        );
        assert_eq!(m.folded_step(&forward, 2, &one(&forward)), Some(Some(3)));
        // The backward run (from 10, the fourth point) steps to −40 and
        // goes back: the fold at −40 is put on the step from 10, the
        // points' third span.
        let backward = path(
            &[
                (-20.0, 0.0),
                (-30.0, 0.0),
                (-40.0, 0.0),
                (10.0, 0.0),
                (20.0, 0.0),
            ],
            t,
            0.0,
        );
        assert_eq!(m.folded_step(&backward, 3, &one(&backward)), Some(Some(2)));
        // A fold at a point the fit put inside the step from 20 to 40.
        let marched = path(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (40.0, 0.0)], t, 0.0);
        let mut fitted = marched.clone();
        fitted.insert(3, placed_at(2.5, Vec3::new(50.0, 0.0, 0.0) * t, Vec3::X));
        assert_eq!(m.folded_step(&marched, 0, &one(&fitted)), Some(Some(2)));
        // At the start: no step came to it.
        let start = path(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (8.0, 0.0)], t, 0.0);
        assert_eq!(m.folded_step(&start, 2, &one(&start)), Some(None));
        // Nothing runs back.
        let straight = path(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)], t, 0.0);
        assert_eq!(m.folded_step(&straight, 1, &one(&straight)), None);
        // Two folds of the forward run: the step nearer the start.
        let twice = path(
            &[
                (0.0, 0.0),
                (10.0, 0.0),
                (40.0, 0.0),
                (30.0, 0.0),
                (20.0, 0.0),
                (60.0, 0.0),
            ],
            t,
            0.0,
        );
        assert_eq!(m.folds(&twice), vec![2, 4]);
        assert_eq!(m.folded_step(&twice, 0, &one(&twice)), Some(Some(1)));
    }

    /// A run taken up again closes on its start as the run it is part of
    /// would: the points it had before count. Taken up from a point of
    /// the oval half a step short of the start, a run that had five
    /// points closes there at once. (A run closes only once it has two
    /// points, so that it does not close on the start it has just left;
    /// counting none but its own, a run taken up again passes the start
    /// and goes round the oval a second time.)
    #[test]
    fn a_march_taken_up_again_short_of_its_start_closes_there() {
        for k in [1e-3, 1.0, 1e6] {
            let mut m = pinch(k, 0.01, (-3.0, 3.0));
            let seed = on_oval(&m, k, 0.01, 0.8);
            let (e, u, v) = m.correct(seed.0, seed.1).unwrap();
            let start = Marcher::point(&e, u, v, 1.0).unwrap();
            let back = 0.1 * m.hmax;
            let (e, u, v) = m
                .correct(start.u - back * start.du, start.v - back * start.dv)
                .unwrap();
            let from = Marcher::point(&e, u, v, 1.0).unwrap();
            assert!(from.t.dot(start.t) > 0.9, "{k}");
            let again = Again {
                from,
                taken: 5,
                cap: 0.2 * m.hmax,
                over: m.hmax,
            };
            let (more, end) = m.run_on(&start, 1.0, &[], true, Some(again)).unwrap();
            assert_eq!(end, End::Closed, "{k}");
            assert!(more.is_empty(), "{k}: {} points past the start", more.len());
        }
    }
}
