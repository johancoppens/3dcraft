//! A sheet through a grid of points: the tensor-product B-spline surface
//! that passes through every point of an `m × n` grid (Piegl & Tiller,
//! *The NURBS Book*, 2nd ed., §9.2.5, global surface interpolation).
//!
//! **The surface.** The points `P[r][c]` stand in `rows` rows of `columns`
//! points each. The parameter `u` runs along a row (from its first point to
//! its last) and `v` across the rows (from the first row to the last), both
//! over `[0, 1]`. The degree is 3 each way, or the number of points that way
//! less one where there are fewer than four. Each direction takes one set of
//! parameters and one knot vector for all its rows or columns ([`Spacing`]),
//! so the surface is interpolated in two passes of curve interpolation,
//! each a banded, totally positive system solved without pivoting
//! (`refine::interpolate_on`; de Boor, *A Practical Guide to Splines*,
//! ch. XIII): first along every row at once (its points the right-hand
//! sides, `3 · rows` wide), then the control points of that pass across the
//! rows. The iso-curve `v = v_r` of the result is the curve interpolation
//! of row `r` at the row parameters, and `u = u_c` that of column `c`: the
//! interpolation is linear and separable.
//!
//! **What it reproduces exactly.** The interpolation is affine invariant
//! (every row of a collocation matrix sums to one), so points on a plane
//! give that plane, with any spacing. The space of each direction holds
//! every cubic polynomial, so a bicubic polynomial surface `F(u, v)`
//! sampled at the spacing's own parameters is `F` itself. With
//! [`Spacing::Even`] over a regular grid `x` and `y` are linear in `u` and
//! `v`, so a height field `z = f(x, y)` of degree three or less each way is
//! reproduced, and the normal's `z` is the constant product of the steps:
//! a height field never folds. Greville's parameters are uneven near the
//! ends and chord's follow the heights too, so with them the height is `f`
//! composed with a cubic map, and the error falls with the grid's step
//! about 4 times per halving with Greville, about 16 times with chord or
//! even (*Findings* in docs/design/surfacing.md).
//!
//! **What is refused, by name** (rows and points counted from 1): fewer
//! than 2 × 2 points, more than [`MAX_GRID_POINTS`] or more than
//! [`MAX_GRID_SIDE`] a side, a coordinate that is not a finite number, a
//! grid so far from the origin that the rounding bound (*At the points*)
//! exceeds the tolerance (checked on the points before anything is
//! computed, and on the result), two neighbouring points within the
//! tolerance (no derivative there), a row or column that doubles back on
//! itself (two legs in a row within the angular tolerance of opposite),
//! chord spacing whose parameters cannot be told apart, and a sheet whose
//! normal is not proven non-zero everywhere (*Regular*, below). No refusal
//! prints a number that is not one.
//!
//! **Regular.** Every Bézier span of the result is proven to have a
//! normal that does not vanish: the normal field `∂u × ∂v` of a
//! polynomial patch is a Bernstein form of degrees `(2p − 1, 2q − 1)`, so
//! when every coefficient lies on one side of a plane through the origin,
//! by more than its rounding (bounded coefficient by coefficient), the
//! normal is not zero anywhere on the span. The plane's normal is a
//! direction the grid gives, not a guess: the caller's (a table's heights'
//! axis), the mean of the grid's cell normals, the coordinate axes, then
//! the span's own coefficients' sum and that of their directions, each
//! improved by a perceptron's steps. A height field over a regular grid
//! passes at once however steep: the coefficients' `z` are products of the
//! parameters' positive speeds along x and y. A span not proven is
//! quartered, each quarter proven in turn, at most [`REGULAR_DEPTH`] times
//! and [`MAX_SPAN_PIECES`] pieces, a budget of its own, so a sheet's size
//! does not decide whether a span is proven; all spans together stop at
//! [`MAX_REGULAR_PIECES`]. A span not proven is refused with the grid point
//! nearest it and what was seen there: a row or column the sheet runs back
//! along (its tangent against the grid's leg beside it, at 5 × 5 places of
//! the span), heights far steeper than the spacing (a scale left out), or
//! neither. A surface regular everywhere and continuous in its normal (a
//! cubic with simple knots is twice differentiable) does not turn over
//! anywhere.
//!
//! **At the points.** The surface passes through every point in exact
//! arithmetic; in floating point the two solves and the evaluation leave
//! residuals. For a totally positive matrix Gaussian elimination without
//! pivoting has a backward error `|E| ≤ γ_k |A|` (de Boor & Pinkus,
//! *Backward error analysis for totally positive linear systems*, Numer.
//! Math. 27, 1977; Higham, *Accuracy and Stability of Numerical
//! Algorithms*, §9.3), and the rows of a collocation matrix are
//! non-negative and sum to one, so each pass leaves a residual of at most
//! `γ_k` times the largest control point it made, `k = 3 (p + 1)` for
//! degree `p`; the evaluation adds `γ_{4(p+1)}` of the largest control
//! point. So [`Grid::bound`] is `(γ_{3(p_u+1)} + γ_{3(p_v+1)} + γ_{4(p+1)})
//! √3 M`, `M` the largest coordinate size of the points and of either pass's
//! control points, `p` the larger degree, about `70 ε M` for bicubics:
//! [`Grid::deviation`], the largest distance found at the points, is held
//! to it by the tests, at scales from 1e-3 to 1e6 and in turned frames far
//! from the origin.

// The negated comparisons are meant: a NaN fails them, and is refused.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::refine::interpolate_on;
use crate::{GeomError, NurbsSurface, RationalBezierPatch};
use craft3d_math::{Portable, Vec3};
use std::fmt;

/// Most points one grid may hold: 2^18 (512 × 512). A grid of that size
/// makes a surface of as many control points, whose Bézier spans (kept
/// with the surface, about 512 bytes each for a bicubic) take some
/// 130 MB, and a grid given point by point is about 30 MB of the saved
/// file at that size, half the format's cap on a manifest.
pub const MAX_GRID_POINTS: usize = 1 << 18;

/// Most points along one side of a grid: 1024. The grid's own work is
/// linear in its points, but meshing and measuring its sheet cut the
/// surface into its Bézier spans by inserting knots one at a time
/// (`NurbsSurface::bezier_spans`), which costs the square of a side's
/// knots times the other side: a 63 × 2000 grid took 73 s there, a
/// 400 × 300 one 11 s. Raised when that is linear (owed).
pub const MAX_GRID_SIDE: usize = 1 << 10;

/// How many times a span whose normal is not proven non-zero at once is
/// halved, each way, before it is refused.
pub const REGULAR_DEPTH: usize = 6;

/// Most pieces the proof may look at for one span beyond the span itself
/// (its quarters, theirs, …): the budget is a span's, so a sheet's size
/// does not decide whether one of its spans is proven.
pub const MAX_SPAN_PIECES: usize = 1 << 10;

/// Most pieces the proof of one sheet may look at beyond its spans, all
/// spans together: what bounds its time (a refusal past it says so).
pub const MAX_REGULAR_PIECES: usize = 1 << 20;

/// The parameters at which the sheet takes its points, one set for each
/// direction (all its rows, or all its columns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spacing {
    /// Uniform clamped knots, each point at the Greville abscissa of its
    /// control point (the mean of `degree` knots), as a live spline
    /// through points takes them by default.
    Greville,
    /// The chord lengths along each row (each column), as a share of the
    /// row's length, averaged over all rows (columns) (*The NURBS Book*,
    /// eqs. 9.4, 9.5 and §9.2.5), on knots averaged from them (eq. 9.8).
    Chord,
    /// Evenly spaced parameters, point `i` of `n` at `i / (n − 1)` (eq.
    /// 9.3), on knots averaged from them (eq. 9.8): over a regular grid
    /// `x` and `y` are then linear in `u` and `v`, so a height field's sheet
    /// is the textbook bicubic spline of its heights.
    Even,
}

/// What [`interpolate_with`] is told besides the points.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub spacing: Spacing,
    /// The linear tolerance: two neighbours within it are one point, and
    /// the sheet must pass within it of every point, rounding included.
    pub tolerance: f64,
    /// The angular tolerance: two legs within it of opposite double back.
    pub angular: f64,
    /// A direction the sheet's normal is expected near, where the caller
    /// knows one (a height field's heights' axis), tried first by the
    /// proof that the sheet does not fold; either way round.
    pub up: Option<Vec3>,
}

/// Why a grid was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum GridError {
    /// The grid itself is refused; the text names the points and why.
    Refused(String),
    Geom(GeomError),
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridError::Refused(why) => f.write_str(why),
            GridError::Geom(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GridError {}

impl From<GeomError> for GridError {
    fn from(e: GeomError) -> Self {
        GridError::Geom(e)
    }
}

fn refused<T>(why: String) -> Result<T, GridError> {
    Err(GridError::Refused(why))
}

/// The sheet through a grid and what was found about it.
#[derive(Clone, Debug)]
pub struct Grid {
    /// The surface: `u` along a row, `v` across the rows, both over
    /// `[0, 1]`; control point `(c, r)` at `c * rows + r`.
    pub surface: NurbsSurface,
    /// The parameter of each column (along a row), from 0 to 1.
    pub u: Vec<f64>,
    /// The parameter of each row, from 0 to 1.
    pub v: Vec<f64>,
    /// The largest distance found between a point and the surface at its
    /// parameters.
    pub deviation: f64,
    /// What rounding may leave between a point and the surface (see the
    /// module's *At the points*). It may exceed the tolerance far from the
    /// origin (between [`resolved`]'s limit and the bound's); the caller
    /// warns then, the measured [`Grid::deviation`] being within it.
    pub bound: f64,
    /// The largest coordinate of the points and the control points: how far
    /// the grid reaches from the origin.
    pub reach: f64,
}

/// How many units in the last place of its largest coordinate a model may
/// take before it cannot hold its tolerance at all: positions there are
/// stored no finer than about `ε` of their size, and every computation
/// adds a few such units.
pub const PLACES: f64 = 4.0;

/// Whether a model reaching `size` from the origin can hold `tolerance`:
/// `PLACES · ε · size` within it. Beyond, a grid is refused: there a
/// curved face is not even measured or meshed in reasonable time.
pub fn resolved(size: f64, tolerance: f64) -> bool {
    PLACES * f64::EPSILON * size <= tolerance
}

/// `γ_k = k ε / (1 − k ε)` (Higham, §3.1).
fn gamma(k: usize) -> f64 {
    let ke = k as f64 * f64::EPSILON;
    ke / (1.0 - ke)
}

/// The sheet through `points`, `rows` rows of `columns` points each, row by
/// row, at the parameters `spacing` gives. `tolerance` is the document's
/// linear tolerance (two neighbours within it are one point), `angular`
/// its angular one (two legs within it of opposite double back).
pub fn interpolate(
    points: &[Vec3],
    rows: usize,
    columns: usize,
    spacing: Spacing,
    tolerance: f64,
    angular: f64,
) -> Result<Grid, GridError> {
    interpolate_with(
        points,
        rows,
        columns,
        &Options {
            spacing,
            tolerance,
            angular,
            up: None,
        },
    )
}

/// The rounding the sheet may leave at its points, `(γ_{3(p_u+1)} +
/// γ_{3(p_v+1)} + γ_{4(p+1)}) √3 M` (the module's *At the points*).
fn rounding_bound(pu: usize, pv: usize, size: f64) -> f64 {
    (gamma(3 * (pu + 1)) + gamma(3 * (pv + 1)) + gamma(4 * (pu.max(pv) + 1)))
        * size
        * 3.0_f64.sqrt()
}

/// The refusal of a grid too far from the origin for the tolerance to be
/// held there at all ([`resolved`]).
fn too_far(size: f64, tolerance: f64) -> GridError {
    GridError::Refused(format!(
        "the grid reaches {size:.3e} from the origin, where a number holds a position only to about {:.1e}: no model there holds the tolerance {tolerance:.1e} (a grid is refused from {:.3e} at this tolerance); work nearer the origin (and move the sheet there with feature.transform afterwards) or in a larger unit",
        size * f64::EPSILON,
        tolerance / (PLACES * f64::EPSILON)
    ))
}

/// [`interpolate`] with every option.
pub fn interpolate_with(
    points: &[Vec3],
    rows: usize,
    columns: usize,
    options: &Options,
) -> Result<Grid, GridError> {
    let Options {
        spacing,
        tolerance,
        angular,
        up,
    } = *options;
    if !(tolerance.is_finite() && tolerance > 0.0 && angular.is_finite() && angular > 0.0) {
        return refused("the tolerance is not positive and finite".into());
    }
    if rows < 2 || columns < 2 {
        return refused(format!(
            "a grid has at least 2 rows of at least 2 points; this one has {rows} rows of {columns}"
        ));
    }
    if rows > MAX_GRID_SIDE || columns > MAX_GRID_SIDE {
        return refused(format!(
            "a grid has at most {MAX_GRID_SIDE} points a side; this one has {rows} rows of {columns}"
        ));
    }
    let count = rows.saturating_mul(columns);
    if count > MAX_GRID_POINTS {
        return refused(format!(
            "a grid holds at most {MAX_GRID_POINTS} points; {rows} rows of {columns} are {count}"
        ));
    }
    if points.len() != count {
        return refused(format!(
            "{rows} rows of {columns} points are {count} points, not {}",
            points.len()
        ));
    }
    let at = |r: usize, c: usize| -> Result<Vec3, GridError> {
        points
            .get(r * columns + c)
            .copied()
            .ok_or_else(|| GridError::Geom(internal()))
    };
    for r in 0..rows {
        for c in 0..columns {
            let p = at(r, c)?;
            if let Some((axis, value)) = [("x", p.x), ("y", p.y), ("z", p.z)]
                .into_iter()
                .find(|(_, value)| !value.is_finite())
            {
                return refused(format!(
                    "row {}, point {}: its {axis} is {}; a point of the grid is finite numbers",
                    r + 1,
                    c + 1,
                    if value.is_nan() {
                        "not a number"
                    } else {
                        "infinite"
                    }
                ));
            }
        }
    }
    let (pu, pv) = ((columns - 1).min(3), (rows - 1).min(3));
    // Where the points' own size leaves no room for the tolerance, refused
    // before anything is computed there.
    let reach = points
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()).max(p.z.abs()))
        .fold(0.0, f64::max);
    if !resolved(reach, tolerance) {
        return Err(too_far(reach, tolerance));
    }
    check_legs(&at, rows, columns, tolerance, angular)?;
    let (u, ku) = parameters(&at, rows, columns, pu, spacing, Direction::AlongRows)?;
    let (v, kv) = parameters(&at, rows, columns, pv, spacing, Direction::AcrossRows)?;

    // Along every row at once: column c's values are the points of all
    // rows there.
    let mut size = 0.0_f64;
    let mut along: Vec<Vec<f64>> = Vec::with_capacity(columns);
    for c in 0..columns {
        let mut values = Vec::with_capacity(3 * rows);
        for r in 0..rows {
            let p = at(r, c)?;
            size = size.max(p.x.abs().max(p.y.abs()).max(p.z.abs()));
            values.extend([p.x, p.y, p.z]);
        }
        along.push(values);
    }
    let first = interpolate_on(&u, pu, &ku, &along)?;
    drop(along);
    // Across the rows: row r's values are the first pass's control points
    // of every column in that row.
    let mut across: Vec<Vec<f64>> = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut values = Vec::with_capacity(3 * columns);
        for column in &first {
            let xyz = column.get(3 * r..3 * r + 3).ok_or_else(internal)?;
            size = xyz.iter().fold(size, |s, x| s.max(x.abs()));
            values.extend_from_slice(xyz);
        }
        across.push(values);
    }
    drop(first);
    let second = interpolate_on(&v, pv, &kv, &across)?;
    drop(across);
    let mut ctrl = vec![Vec3::ZERO; count];
    for (r, row) in second.iter().enumerate() {
        for c in 0..columns {
            let xyz = row.get(3 * c..3 * c + 3).ok_or_else(internal)?;
            let (Some(&x), Some(&y), Some(&z)) = (xyz.first(), xyz.get(1), xyz.get(2)) else {
                return Err(internal().into());
            };
            size = size.max(x.abs().max(y.abs()).max(z.abs()));
            *ctrl.get_mut(c * rows + r).ok_or_else(internal)? = Vec3::new(x, y, z);
        }
    }
    drop(second);
    let surface = NurbsSurface::new(pu, pv, ku, kv, columns, rows, ctrl, vec![1.0; count])?;

    // How far the points are from the surface at their parameters.
    let mut deviation = 0.0_f64;
    for (r, &vr) in v.iter().enumerate() {
        for (c, &uc) in u.iter().enumerate() {
            deviation = deviation.max(surface.eval(uc, vr).distance(at(r, c)?));
        }
    }
    // Three coordinates: the length of a residual is at most √3 times its
    // largest coordinate's size.
    let bound = rounding_bound(pu, pv, size);
    if !resolved(size, tolerance) {
        return Err(too_far(size, tolerance));
    }
    if !(deviation <= tolerance) {
        return refused(format!(
            "the sheet through the points misses one by {:.1e}, more than the tolerance {tolerance:.1e}: the points are too unevenly spaced for a sheet through them",
            deviation.min(f64::MAX)
        ));
    }
    regular(&surface, &u, &v, &at, (rows, columns), up)?;
    Ok(Grid {
        surface,
        u,
        v,
        deviation,
        bound,
        reach: size,
    })
}

fn internal() -> GeomError {
    GeomError::InvalidParameter(craft3d_math::bug_text!("a grid lost track of its points"))
}

/// Which way a grid is walked: along its rows (the points of one row) or
/// across them (the points of one column).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    AlongRows,
    AcrossRows,
}

/// Neighbours within the tolerance, and legs that double back.
fn check_legs(
    at: &dyn Fn(usize, usize) -> Result<Vec3, GridError>,
    rows: usize,
    columns: usize,
    tolerance: f64,
    angular: f64,
) -> Result<(), GridError> {
    for (direction, lines, length) in [
        (Direction::AlongRows, rows, columns),
        (Direction::AcrossRows, columns, rows),
    ] {
        let point = |line: usize, k: usize| match direction {
            Direction::AlongRows => at(line, k),
            Direction::AcrossRows => at(k, line),
        };
        for line in 0..lines {
            let mut before: Option<Vec3> = None;
            for k in 1..length {
                let leg = point(line, k)? - point(line, k - 1)?;
                let len = leg.length();
                if !(len > tolerance) {
                    return refused(match direction {
                        Direction::AlongRows => format!(
                            "row {}: points {} and {} are {len:.1e} apart, within the tolerance {tolerance:.1e}: each point of a grid needs a place of its own",
                            line + 1,
                            k,
                            k + 1
                        ),
                        Direction::AcrossRows => format!(
                            "point {} of rows {} and {} are {len:.1e} apart, within the tolerance {tolerance:.1e}: each point of a grid needs a place of its own",
                            line + 1,
                            k,
                            k + 1
                        ),
                    });
                }
                if let Some(previous) = before {
                    // The angle between the legs' directions (made without
                    // overflow), from their sine and cosine together.
                    let turn = match (unit(previous), unit(leg)) {
                        (Some(a), Some(b)) => a.cross(b).length().patan2(a.dot(b)),
                        _ => 0.0,
                    };
                    if !(turn < std::f64::consts::PI - angular) {
                        return refused(match direction {
                            Direction::AlongRows => format!(
                                "row {} doubles back on itself at point {}: its legs before and after turn by {:.4}°; a row of a grid goes on from each point, it does not run back",
                                line + 1,
                                k,
                                turn.to_degrees()
                            ),
                            Direction::AcrossRows => format!(
                                "the column of point {} doubles back on itself at row {}: its legs before and after turn by {:.4}°; a column of a grid goes on from each row, it does not run back",
                                line + 1,
                                k,
                                turn.to_degrees()
                            ),
                        });
                    }
                }
                before = Some(leg);
            }
        }
    }
    Ok(())
}

/// Below this share of the whole, two parameters in a row cannot be told
/// apart (as a live spline's chord spacing has it).
const LEAST_GAP: f64 = 1e-9;

/// The parameters of one direction and its knots (clamped, `degree + 1`
/// zeros and ones).
fn parameters(
    at: &dyn Fn(usize, usize) -> Result<Vec3, GridError>,
    rows: usize,
    columns: usize,
    degree: usize,
    spacing: Spacing,
    direction: Direction,
) -> Result<(Vec<f64>, Vec<f64>), GridError> {
    let (lines, n) = match direction {
        Direction::AlongRows => (rows, columns),
        Direction::AcrossRows => (columns, rows),
    };
    match spacing {
        Spacing::Greville => {
            // Uniform clamped knots: inner knot i is i / (n − degree).
            let spans = n - degree;
            let mut knots = vec![0.0; degree + 1];
            knots.extend((1..spans).map(|i| i as f64 / spans as f64));
            knots.extend(std::iter::repeat_n(1.0, degree + 1));
            let mut params = Vec::with_capacity(n);
            for i in 0..n {
                params.push(if i == 0 {
                    0.0
                } else if i + 1 == n {
                    1.0
                } else {
                    let inner = knots.get(i + 1..=i + degree).ok_or_else(internal)?;
                    inner.iter().sum::<f64>() / degree as f64
                });
            }
            Ok((params, knots))
        }
        Spacing::Even => {
            let last = (n - 1) as f64;
            let params: Vec<f64> = (0..n)
                .map(|i| if i + 1 == n { 1.0 } else { i as f64 / last })
                .collect();
            let knots = averaged_knots(&params, degree);
            Ok((params, knots))
        }
        Spacing::Chord => {
            let mut sums = vec![0.0; n];
            for line in 0..lines {
                let point = |k: usize| match direction {
                    Direction::AlongRows => at(line, k),
                    Direction::AcrossRows => at(k, line),
                };
                let mut legs = Vec::with_capacity(n - 1);
                for k in 1..n {
                    legs.push((point(k)? - point(k - 1)?).length());
                }
                let total: f64 = legs.iter().sum();
                if !(total.is_finite() && total > 0.0) {
                    return Err(internal().into());
                }
                let mut run = 0.0;
                for (k, leg) in legs.iter().enumerate() {
                    run += leg;
                    if let Some(slot) = sums.get_mut(k + 1) {
                        *slot += run / total;
                    }
                }
            }
            let mut params: Vec<f64> = sums.iter().map(|s| s / lines as f64).collect();
            if let Some(first) = params.first_mut() {
                *first = 0.0;
            }
            if let Some(last) = params.last_mut() {
                *last = 1.0;
            }
            if let Some(k) = params
                .windows(2)
                .position(|w| matches!(w, [a, b] if !(b - a > LEAST_GAP)))
            {
                return refused(match direction {
                    Direction::AlongRows => format!(
                        "points {} and {} of the rows are too close beside the rows' length for chord spacing to tell them apart",
                        k + 1,
                        k + 2
                    ),
                    Direction::AcrossRows => format!(
                        "rows {} and {} are too close beside the columns' length for chord spacing to tell them apart",
                        k + 1,
                        k + 2
                    ),
                });
            }
            let knots = averaged_knots(&params, degree);
            Ok((params, knots))
        }
    }
}

/// Clamped knots averaged from rising parameters from 0 to 1 (*The NURBS
/// Book*, eq. 9.8): `degree + 1` zeros, inner knot `j` the mean of
/// `params[j..j + degree]`, `degree + 1` ones.
fn averaged_knots(params: &[f64], degree: usize) -> Vec<f64> {
    let n = params.len();
    let mut knots = vec![0.0; degree + 1];
    for window in params
        .get(1..n.saturating_sub(1))
        .unwrap_or(&[])
        .windows(degree.max(1))
    {
        knots.push(window.iter().sum::<f64>() / degree as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

/// The index of the parameter nearest `t` among `params` (rising).
fn nearest(params: &[f64], t: f64) -> usize {
    let after = params.partition_point(|p| *p < t);
    match (after.checked_sub(1), params.get(after)) {
        (Some(b), Some(&a)) if params.get(b).is_some_and(|&p| t - p <= a - t) => b,
        (Some(b), None) => b,
        _ => after,
    }
}

/// The knot spans of a clamped knot vector: the index `k` of each non-empty
/// span `[knots[k], knots[k + 1])` and its range.
fn spans(knots: &[f64], degree: usize) -> Vec<(usize, (f64, f64))> {
    let count = knots.len().saturating_sub(degree + 1);
    (degree..count)
        .filter_map(|k| match (knots.get(k), knots.get(k + 1)) {
            (Some(&lo), Some(&hi)) if hi > lo => Some((k, (lo, hi))),
            _ => None,
        })
        .collect()
}

/// The blossom of the B-spline segment of span `k` (degree `p`, control
/// points `window`, the `p + 1` of the span) at `args` (`p` parameters),
/// by de Boor's algorithm with the `r`-th argument at level `r`
/// (Ramshaw's blossoming; Piegl & Tiller, *The NURBS Book*, A3.1 for one
/// argument).
fn blossom(
    window: &[Vec3],
    knots: &[f64],
    k: usize,
    p: usize,
    args: &[f64],
) -> Result<Vec3, GridError> {
    let mut d = window.to_vec();
    for (r, &x) in (1..=p).zip(args) {
        for j in (r..=p).rev() {
            let (Some(&lo), Some(&hi)) = (knots.get(j + k - p), knots.get(j + 1 + k - r)) else {
                return Err(internal().into());
            };
            let alpha = if hi > lo { (x - lo) / (hi - lo) } else { 0.0 };
            let (Some(&before), Some(&here)) = (d.get(j - 1), d.get(j)) else {
                return Err(internal().into());
            };
            *d.get_mut(j).ok_or_else(internal)? = before * (1.0 - alpha) + here * alpha;
        }
    }
    d.get(p).copied().ok_or_else(|| internal().into())
}

/// The Bézier net of one span of a polynomial tensor-product spline from
/// its window of control points alone: along u each column's Bézier points
/// are the blossoms `b(a^{p−i}, b^i)` at the span's ends, then likewise
/// along v. Linear in the size of the surface over all spans, where
/// knot insertion over the whole surface costs the square of its knots.
fn span_patch(
    window: Vec<Vec3>,
    (ku, su, pu): (&[f64], usize, usize),
    (kv, sv, pv): (&[f64], usize, usize),
) -> Result<crate::BezierPatch, GridError> {
    let ends = |knots: &[f64], k: usize| -> Result<(f64, f64), GridError> {
        match (knots.get(k), knots.get(k + 1)) {
            (Some(&a), Some(&b)) => Ok((a, b)),
            _ => Err(internal().into()),
        }
    };
    let (au, bu) = ends(ku, su)?;
    let (av, bv) = ends(kv, sv)?;
    let width = pv + 1;
    // Along u: column j of the window.
    let mut along_u = vec![Vec3::ZERO; window.len()];
    for j in 0..=pv {
        let column: Vec<Vec3> = (0..=pu)
            .filter_map(|i| window.get(i * width + j).copied())
            .collect();
        for i in 0..=pu {
            let args: Vec<f64> = (0..pu).map(|r| if r < i { bu } else { au }).collect();
            *along_u.get_mut(i * width + j).ok_or_else(internal)? =
                blossom(&column, ku, su, pu, &args)?;
        }
    }
    // Along v: row i of that.
    let mut net = vec![Vec3::ZERO; window.len()];
    for i in 0..=pu {
        let row = along_u
            .get(i * width..(i + 1) * width)
            .ok_or_else(internal)?
            .to_vec();
        for j in 0..=pv {
            let args: Vec<f64> = (0..pv).map(|r| if r < j { bv } else { av }).collect();
            *net.get_mut(i * width + j).ok_or_else(internal)? = blossom(&row, kv, sv, pv, &args)?;
        }
    }
    Ok(crate::BezierPatch::new(pu, pv, net)?)
}

/// How the proof of a span ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Proven,
    /// Not proven within the span's depth or its own budget.
    NotProven,
    /// The sheet's budget ran out.
    OutOfWork,
}

/// The mean of the grid's cell normals, each `(P[r][c+1] − P[r+1][c]) ×
/// (P[r+1][c+1] − P[r][c])` (the direction of `∂u × ∂v` over the cell)
/// made a unit first, so no cell outweighs the rest; `None` where they
/// cancel. A direction the grid gives, not one guessed: a height field's
/// is near its heights' axis, a sheet's that turns little near its normal.
fn mean_normal(
    at: &dyn Fn(usize, usize) -> Result<Vec3, GridError>,
    rows: usize,
    columns: usize,
) -> Result<Option<Vec3>, GridError> {
    let mut sum = Vec3::ZERO;
    for r in 0..rows - 1 {
        for c in 0..columns - 1 {
            let a = at(r, c + 1)? - at(r + 1, c)?;
            let b = at(r + 1, c + 1)? - at(r, c)?;
            if let Some(n) = unit(a)
                .zip(unit(b))
                .and_then(|(a, b)| a.cross(b).normalized())
            {
                sum += n;
            }
        }
    }
    Ok(sum.normalized())
}

/// A vector's direction, found without overflow: scaled by its largest
/// coordinate first.
fn unit(a: Vec3) -> Option<Vec3> {
    let big = a.x.abs().max(a.y.abs()).max(a.z.abs());
    if !(big > 0.0 && big.is_finite()) {
        return None;
    }
    (a * (1.0 / big)).normalized()
}

/// Proves the surface's normal non-zero everywhere (the module's
/// *Regular*), or says where it could not and what it saw there.
fn regular(
    surface: &NurbsSurface,
    u: &[f64],
    v: &[f64],
    at: &dyn Fn(usize, usize) -> Result<Vec3, GridError>,
    (rows, columns): (usize, usize),
    up: Option<Vec3>,
) -> Result<(), GridError> {
    let mean = mean_normal(at, rows, columns)?;
    let mut directions = Vec::new();
    for d in [up, mean, Some(Vec3::X), Some(Vec3::Y), Some(Vec3::Z)]
        .into_iter()
        .flatten()
    {
        if let Some(d) = unit(d) {
            directions.push(d);
            directions.push(d * -1.0);
        }
    }
    let (pu, pv) = surface.degrees();
    let (ku, kv) = (surface.knots_u(), surface.knots_v());
    let (spans_u, spans_v) = (spans(ku, pu), spans(kv, pv));
    let count_v = surface.counts().1;
    let ctrl = surface.control_points();
    let mut spent = 0_usize;
    for &(su, range_u) in &spans_u {
        for &(sv, range_v) in &spans_v {
            // The span's window of control points, (pu + 1) × (pv + 1).
            let mut net = Vec::with_capacity((pu + 1) * (pv + 1));
            for i in su - pu..=su {
                for j in sv - pv..=sv {
                    net.push(ctrl.get(i * count_v + j).copied().ok_or_else(internal)?);
                }
            }
            let patch =
                RationalBezierPatch::from_bezier(&span_patch(net, (ku, su, pu), (kv, sv, pv))?);
            let mut own = 0_usize;
            let verdict = proven(&patch, 0, &directions, &mut own, &mut spent);
            if verdict != Verdict::Proven {
                let place = Place {
                    surface,
                    u,
                    v,
                    at,
                    rows,
                    columns,
                };
                return refused(place.why(range_u, range_v, verdict, mean)?);
            }
        }
    }
    Ok(())
}

/// The surface and the grid, to say where and why a span was not proven.
struct Place<'a> {
    surface: &'a NurbsSurface,
    u: &'a [f64],
    v: &'a [f64],
    at: &'a dyn Fn(usize, usize) -> Result<Vec3, GridError>,
    rows: usize,
    columns: usize,
}

/// Rises between neighbours above this many times their spacing are
/// named as a scale that may be wrong.
const STEEP: f64 = 1e3;

impl Place<'_> {
    /// The leg of the grid between two neighbours.
    fn leg(&self, from: (usize, usize), to: (usize, usize)) -> Result<Vec3, GridError> {
        Ok((self.at)(to.0, to.1)? - (self.at)(from.0, from.1)?)
    }

    /// The interval `[params[k], params[k + 1]]` that holds `t`.
    fn interval(params: &[f64], t: f64) -> usize {
        params
            .partition_point(|p| *p <= t)
            .saturating_sub(1)
            .min(params.len().saturating_sub(2))
    }

    /// The largest rise between neighbours along `normal` over their
    /// distance across it (a height field's slope), anywhere in the grid.
    fn steepest(&self, normal: Vec3) -> Result<f64, GridError> {
        let mut most = 0.0_f64;
        for r in 0..self.rows {
            for c in 0..self.columns {
                for to in [(r, c + 1), (r + 1, c)] {
                    if to.0 >= self.rows || to.1 >= self.columns {
                        continue;
                    }
                    let Some(leg) = unit(self.leg((r, c), to)?) else {
                        continue;
                    };
                    let rise = leg.dot(normal).abs();
                    let run = (leg - normal * leg.dot(normal)).length();
                    most = most.max(if run > 1e-12 { rise / run } else { 1e12 });
                }
            }
        }
        Ok(most)
    }

    /// Why the span over `range_u × range_v` was not proven, in words.
    fn why(
        &self,
        range_u: (f64, f64),
        range_v: (f64, f64),
        verdict: Verdict,
        mean: Option<Vec3>,
    ) -> Result<String, GridError> {
        let (r, c) = (
            nearest(self.v, 0.5 * (range_v.0 + range_v.1)),
            nearest(self.u, 0.5 * (range_u.0 + range_u.1)),
        );
        // A witness that a row or a column runs back: the sheet's tangent
        // along it, at 5 × 5 places of the span, against the leg of the
        // grid it lies beside, both in space and in the grid's mean plane
        // (so a height that overshoots between two points is no run-back
        // where the row goes on in plan).
        let back = |tangent: Vec3, leg: Vec3| -> bool {
            if !(tangent.dot(leg) < 0.0) {
                return false;
            }
            let Some(n) = mean else {
                return true;
            };
            let (t, l) = (tangent - n * tangent.dot(n), leg - n * leg.dot(n));
            // Where the leg stands along the mean normal within rounding its
            // plan says nothing: space decides.
            let room = 64.0 * f64::EPSILON;
            if !(l.length() > room * leg.length() && t.length() > room * tangent.length()) {
                return true;
            }
            t.dot(l) < 0.0
        };
        for i in 0..=4 {
            for j in 0..=4 {
                let s = range_u.0 + (range_u.1 - range_u.0) * f64::from(i) / 4.0;
                let t = range_v.0 + (range_v.1 - range_v.0) * f64::from(j) / 4.0;
                let (_, su, sv) = self.surface.derivatives(s, t);
                let (ci, ri) = (Self::interval(self.u, s), Self::interval(self.v, t));
                let (row, column) = (nearest(self.v, t), nearest(self.u, s));
                if back(su, self.leg((row, ci), (row, ci + 1))?) {
                    return Ok(format!(
                        "row {} runs back between points {} and {}: the sheet through it turns against the row there, so it folds near row {}, point {} (chord spacing follows unevenly spaced points, even spacing a regular grid's)",
                        row + 1,
                        ci + 1,
                        ci + 2,
                        row + 1,
                        ci + 1
                    ));
                }
                if back(sv, self.leg((ri, column), (ri + 1, column))?) {
                    return Ok(format!(
                        "the column of point {} runs back between rows {} and {}: the sheet through it turns against the column there, so it folds near row {}, point {} (chord spacing follows unevenly spaced points, even spacing a regular grid's)",
                        column + 1,
                        ri + 1,
                        ri + 2,
                        ri + 1,
                        column + 1
                    ));
                }
            }
        }
        let steep = match mean {
            Some(normal) => self.steepest(normal)?,
            None => 0.0,
        };
        let scale = if steep >= 1e12 {
            ": the points rise straight up between neighbours, far more than a smooth sheet through them follows: is their scale right (a table's factor)?".to_string()
        } else if steep > STEEP {
            format!(
                ": the points rise up to {steep:.0} times their spacing between neighbours, far more than a smooth sheet through them follows: is their scale right (a table's factor)?"
            )
        } else {
            String::new()
        };
        Ok(match verdict {
            Verdict::OutOfWork => format!(
                "the proof that the sheet does not fold ran out of its work ({MAX_REGULAR_PIECES} pieces) near row {}, point {}: nothing there was seen to fold, nothing proven either{scale}",
                r + 1,
                c + 1
            ),
            Verdict::NotProven | Verdict::Proven => format!(
                "the sheet through the points could not be proven free of folds near row {}, point {}: its normal may vanish there (no row or column was seen to run back; the proof quarters a span {REGULAR_DEPTH} times, at most {MAX_SPAN_PIECES} pieces){scale}",
                r + 1,
                c + 1
            ),
        })
    }
}

/// `n` choose `k` as a float (small arguments: the degrees of a patch).
fn choose(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

/// The cross product of the magnitudes: what each coordinate of `a × b`
/// is a difference of, added instead.
fn cross_size(a: Vec3, b: Vec3) -> Vec3 {
    let (a, b) = (
        Vec3::new(a.x.abs(), a.y.abs(), a.z.abs()),
        Vec3::new(b.x.abs(), b.y.abs(), b.z.abs()),
    );
    Vec3::new(
        a.y * b.z + a.z * b.y,
        a.z * b.x + a.x * b.z,
        a.x * b.y + a.y * b.x,
    )
}

/// Whether the normal field `∂u × ∂v` of a polynomial patch, a Bernstein
/// form whose coefficients `N_kl` are binomially weighted cross products of
/// its derivative nets, lies in an open half-space: some direction `d`
/// with `N_kl · d` above its rounding for every coefficient, so the normal,
/// a convex combination of them, is not zero on the patch. Each
/// coefficient's rounding is bounded by running error analysis (Higham,
/// §3.1): `γ_K` of the same sum taken over magnitudes (`M_kl`, by
/// [`cross_size`]; `K` the terms it adds) and `γ_4 |N_kl|` for the dot;
/// the patch's own points are off the exact span's by at most `γ` of
/// their reach (made by blossoming, cut by de Casteljau), carried into the
/// derivatives and their products. The directions tried: the sheet's
/// (`directions`), then the coefficients' sum and the sum of their
/// directions, each followed by a perceptron's steps (16 at most).
fn normal_clear(patch: &RationalBezierPatch, directions: &[Vec3]) -> bool {
    let (m, n) = patch.degrees();
    if m == 0 || n == 0 {
        return false;
    }
    let points = patch.control_points();
    let at = |i: usize, j: usize| points.get(i * (n + 1) + j).copied();
    let mut f = Vec::with_capacity(m * (n + 1));
    for i in 0..m {
        for j in 0..=n {
            let (Some(a), Some(b)) = (at(i, j), at(i + 1, j)) else {
                return false;
            };
            f.push((b - a) * m as f64);
        }
    }
    let mut g = Vec::with_capacity((m + 1) * n);
    for i in 0..=m {
        for j in 0..n {
            let (Some(a), Some(b)) = (at(i, j), at(i, j + 1)) else {
                return false;
            };
            g.push((b - a) * n as f64);
        }
    }
    crate::allowance::charge(
        ((m * (n + 1) * (m + 1) * n) as u64).saturating_mul(32)
            / crate::allowance::ARITHMETIC_PER_UNIT
            + 1,
    );
    let (ka, kb) = (2 * m - 1, 2 * n - 1);
    let mut normal = vec![Vec3::ZERO; (ka + 1) * (kb + 1)];
    let mut size = vec![Vec3::ZERO; (ka + 1) * (kb + 1)];
    for i in 0..m {
        for j in 0..=n {
            let Some(&fu) = f.get(i * (n + 1) + j) else {
                return false;
            };
            for i2 in 0..=m {
                for j2 in 0..n {
                    let Some(&gv) = g.get(i2 * n + j2) else {
                        return false;
                    };
                    let (k, l) = (i + i2, j + j2);
                    let w = choose(m - 1, i) * choose(m, i2) / choose(ka, k)
                        * choose(n, j)
                        * choose(n - 1, j2)
                        / choose(kb, l);
                    let slot = k * (kb + 1) + l;
                    if let (Some(a), Some(b)) = (normal.get_mut(slot), size.get_mut(slot)) {
                        *a += fu.cross(gv) * w;
                        *b += cross_size(fu, gv) * w;
                    }
                }
            }
        }
    }
    let largest = |net: &[Vec3]| net.iter().map(|d| d.length()).fold(0.0, f64::max);
    let (big_f, big_g) = (largest(&f), largest(&g));
    let reach = points.iter().map(|p| p.length()).fold(0.0, f64::max);
    let terms = (m + 1) * (m + 1) * (n + 1) * (n + 1) + 8;
    let off = gamma(6 * (m + n) + 8 + 4 * (m + n) * REGULAR_DEPTH) * reach;
    let carried =
        2.0 * off * (m as f64 * big_g + n as f64 * big_f) + 4.0 * (m * n) as f64 * off * off;
    if !(carried.is_finite() && big_f.is_finite() && big_g.is_finite()) {
        return false;
    }
    // The first coefficient a direction does not clear, if any.
    let blocked = |d: Vec3| -> Option<usize> {
        let d_size = Vec3::new(d.x.abs(), d.y.abs(), d.z.abs());
        normal.iter().zip(&size).position(|(c, s)| {
            !(c.dot(d) > gamma(terms) * s.dot(d_size) + gamma(4) * c.length() + carried)
        })
    };
    if directions.iter().any(|d| blocked(*d).is_none()) {
        return true;
    }
    let sum = normal.iter().fold(Vec3::ZERO, |s, d| s + *d);
    let units = normal
        .iter()
        .filter_map(|d| unit(*d))
        .fold(Vec3::ZERO, |s, d| s + d);
    for start in [sum, units] {
        let Some(mut d) = unit(start) else {
            continue;
        };
        for _ in 0..16 {
            let Some(k) = blocked(d) else {
                return true;
            };
            let Some(next) = normal
                .get(k)
                .copied()
                .and_then(unit)
                .and_then(|c| unit(d + c))
            else {
                break;
            };
            d = next;
        }
        if blocked(d).is_none() {
            return true;
        }
    }
    false
}

/// Whether the patch's normal is proven non-zero, quartering it up to
/// [`REGULAR_DEPTH`] times and [`MAX_SPAN_PIECES`] pieces (`own`), all of
/// one sheet's quartering within [`MAX_REGULAR_PIECES`] (`spent`).
fn proven(
    patch: &RationalBezierPatch,
    depth: usize,
    directions: &[Vec3],
    own: &mut usize,
    spent: &mut usize,
) -> Verdict {
    if normal_clear(patch, directions) {
        return Verdict::Proven;
    }
    if depth >= REGULAR_DEPTH || *own >= MAX_SPAN_PIECES {
        return Verdict::NotProven;
    }
    *own = own.saturating_add(4);
    *spent = spent.saturating_add(4);
    if *spent > MAX_REGULAR_PIECES {
        return Verdict::OutOfWork;
    }
    let (low, high) = patch.split_u(0.5);
    for half in [low, high] {
        let (a, b) = half.split_v(0.5);
        for quarter in [a, b] {
            let verdict = proven(&quarter, depth + 1, directions, own, spent);
            if verdict != Verdict::Proven {
                return verdict;
            }
        }
    }
    Verdict::Proven
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use craft3d_math::Frame;

    const TOL: f64 = 1e-9;
    const ANG: f64 = 1e-9;

    fn grid(rows: usize, columns: usize, f: impl Fn(usize, usize) -> Vec3) -> Vec<Vec3> {
        let mut out = Vec::with_capacity(rows * columns);
        for r in 0..rows {
            for c in 0..columns {
                out.push(f(r, c));
            }
        }
        out
    }

    fn refusal(result: Result<Grid, GridError>) -> String {
        match result {
            Err(GridError::Refused(why)) => why,
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn points_on_a_plane_give_the_plane_at_every_scale_and_frame() {
        for k in [1e-3, 1.0, 1e6] {
            let frame = Frame::new(
                Vec3::new(3e3 * k, -2e3 * k, 1e3 * k),
                Vec3::new(1.0, 2.0, 3.0).normalized().unwrap(),
                Vec3::new(3.0, 0.0, -1.0).normalized().unwrap(),
            )
            .unwrap();
            // An uneven grid on the frame's xy plane.
            let pts = grid(7, 9, |r, c| {
                let (x, y) = (
                    k * (c as f64 + 0.3 * (r as f64).psin()),
                    k * (r as f64).ppowi(2) * 0.5,
                );
                frame.point(x, y + 0.1 * k * c as f64, 0.0)
            });
            for spacing in [Spacing::Greville, Spacing::Chord] {
                let made = interpolate(&pts, 7, 9, spacing, TOL * k, ANG).unwrap();
                assert!(
                    made.deviation <= made.bound,
                    "{k} {spacing:?}: {} > {}",
                    made.deviation,
                    made.bound
                );
                let normal = frame.z();
                let origin = frame.origin();
                let mut far = 0.0_f64;
                for i in 0..=40 {
                    for j in 0..=40 {
                        let p = made.surface.eval(i as f64 / 40.0, j as f64 / 40.0);
                        far = far.max((p - origin).dot(normal).abs());
                    }
                }
                // Rounding of points about 3.7e3 times their spacing from
                // the origin.
                let size = origin.length() + 100.0 * k;
                assert!(far <= 64.0 * f64::EPSILON * size, "{k} {spacing:?}: {far}");
            }
        }
    }

    #[test]
    fn a_bicubic_sampled_at_greville_abscissae_is_reproduced() {
        // F(u, v): a bicubic polynomial patch with every coefficient used.
        let f = |u: f64, v: f64| {
            Vec3::new(
                2.0 * u + 0.3 * u * u * v - 0.1 * u.ppowi(3) * v.ppowi(3),
                1.5 * v - 0.2 * u * v * v + 0.05 * v.ppowi(3),
                u.ppowi(3) - 2.0 * u * u * v.ppowi(3) + 0.7 * u * v + v.ppowi(2),
            )
        };
        for (rows, columns) in [(4, 4), (6, 9), (12, 7)] {
            let greville = |n: usize| {
                let spans = n - 3;
                let knot = |i: usize| (i.saturating_sub(3)).min(spans) as f64 / spans as f64;
                (0..n)
                    .map(|i| (knot(i + 1) + knot(i + 2) + knot(i + 3)) / 3.0)
                    .collect::<Vec<f64>>()
            };
            let (gu, gv) = (greville(columns), greville(rows));
            let pts = grid(rows, columns, |r, c| f(gu[c], gv[r]));
            let made = interpolate(&pts, rows, columns, Spacing::Greville, TOL, ANG).unwrap();
            let mut off = 0.0_f64;
            for i in 0..=50 {
                for j in 0..=50 {
                    let (s, t) = (i as f64 / 50.0, j as f64 / 50.0);
                    off = off.max(made.surface.eval(s, t).distance(f(s, t)));
                }
            }
            assert!(off <= 1e-14, "{rows} × {columns}: {off}");
        }
    }

    #[test]
    fn the_iso_curves_at_the_parameters_are_the_rows_and_columns() {
        let pts = grid(5, 6, |r, c| {
            Vec3::new(
                c as f64,
                r as f64 * 1.3,
                (c as f64 * 0.7).psin() * (r as f64).pcos(),
            )
        });
        for spacing in [Spacing::Greville, Spacing::Chord] {
            let made = interpolate(&pts, 5, 6, spacing, TOL, ANG).unwrap();
            for r in 0..5 {
                for c in 0..6 {
                    let p = made.surface.eval(made.u[c], made.v[r]);
                    assert!(
                        p.distance(pts[r * 6 + c]) <= made.bound,
                        "{spacing:?} {r} {c}"
                    );
                }
            }
            // The corners are the corner points exactly.
            assert_eq!(made.surface.eval(0.0, 0.0), pts[0]);
            assert_eq!(made.surface.eval(1.0, 1.0), pts[29]);
        }
    }

    #[test]
    fn transposed_or_reversed_the_grid_gives_the_same_surface() {
        let (rows, columns) = (6, 8);
        let point = |r: usize, c: usize| {
            Vec3::new(
                c as f64 + 0.1 * (r as f64).psin(),
                r as f64 * 0.8,
                0.3 * (c as f64 * 0.9).psin() + 0.2 * (r as f64 * 1.1).pcos(),
            )
        };
        let pts = grid(rows, columns, point);
        let transposed = grid(columns, rows, |r, c| point(c, r));
        let reversed = grid(rows, columns, |r, c| point(rows - 1 - r, columns - 1 - c));
        for spacing in [Spacing::Greville, Spacing::Chord] {
            let a = interpolate(&pts, rows, columns, spacing, TOL, ANG).unwrap();
            let b = interpolate(&transposed, columns, rows, spacing, TOL, ANG).unwrap();
            let c = interpolate(&reversed, rows, columns, spacing, TOL, ANG).unwrap();
            let mut off = 0.0_f64;
            for i in 0..=30 {
                for j in 0..=30 {
                    let (s, t) = (i as f64 / 30.0, j as f64 / 30.0);
                    let p = a.surface.eval(s, t);
                    off = off.max(p.distance(b.surface.eval(t, s)));
                    off = off.max(p.distance(c.surface.eval(1.0 - s, 1.0 - t)));
                }
            }
            assert!(off <= 1e-13, "{spacing:?}: {off}");
        }
    }

    #[test]
    fn hostile_grids_are_refused_by_name() {
        let good = |r: usize, c: usize| Vec3::new(c as f64, r as f64, 0.0);
        // Too few.
        let why = refusal(interpolate(
            &grid(1, 3, good),
            1,
            3,
            Spacing::Greville,
            TOL,
            ANG,
        ));
        assert!(why.contains("at least 2 rows"), "{why}");
        // The wrong count.
        let why = refusal(interpolate(
            &grid(2, 3, good),
            3,
            3,
            Spacing::Greville,
            TOL,
            ANG,
        ));
        assert!(why.contains("not 6"), "{why}");
        // Too many.
        let why = refusal(interpolate(&[], 1024, 1024, Spacing::Greville, TOL, ANG));
        assert!(why.contains("at most"), "{why}");
        let why = refusal(interpolate(
            &[],
            2,
            MAX_GRID_SIDE + 1,
            Spacing::Greville,
            TOL,
            ANG,
        ));
        assert!(why.contains("a side"), "{why}");
        // Not finite.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let pts = grid(3, 3, |r, c| {
                if (r, c) == (1, 2) {
                    Vec3::new(bad, 0.0, 0.0)
                } else {
                    good(r, c)
                }
            });
            let why = refusal(interpolate(&pts, 3, 3, Spacing::Greville, TOL, ANG));
            assert!(why.contains("row 2, point 3"), "{why}");
        }
        // Coincident neighbours, along a row and across.
        let pts = grid(3, 4, |r, c| {
            if (r, c) == (2, 2) {
                good(2, 1)
            } else {
                good(r, c)
            }
        });
        let why = refusal(interpolate(&pts, 3, 4, Spacing::Greville, TOL, ANG));
        assert!(why.contains("row 3: points 2 and 3"), "{why}");
        let pts = grid(3, 4, |r, c| {
            if (r, c) == (1, 3) {
                good(0, 3)
            } else {
                good(r, c)
            }
        });
        let why = refusal(interpolate(&pts, 3, 4, Spacing::Chord, TOL, ANG));
        assert!(why.contains("point 4 of rows 1 and 2"), "{why}");
        // A row that runs back on itself.
        let pts = grid(3, 5, |r, c| {
            Vec3::new([0.0, 1.0, 2.0, 1.5, 1.0][c], r as f64, 0.0)
        });
        let why = refusal(interpolate(&pts, 3, 5, Spacing::Greville, TOL, ANG));
        assert!(
            why.contains("row 1 doubles back on itself at point 3"),
            "{why}"
        );
        // One row folded over with an offset while its neighbours go on:
        // no leg runs straight back, but the sheet turns over beside it.
        let pts = grid(4, 6, |r, c| {
            if r == 2 {
                let x = [0.0, 1.0, 2.0, 1.2, 0.4, -0.4][c];
                Vec3::new(x, 2.0, if c >= 3 { 0.3 } else { 0.0 })
            } else {
                good(r, c)
            }
        });
        let why = refusal(interpolate(&pts, 4, 6, Spacing::Greville, TOL, ANG));
        assert!(why.contains("folds near row"), "{why}");
        // A twisted cell: one point crossed over its neighbour row.
        let pts = grid(4, 4, |r, c| {
            if (r, c) == (1, 1) {
                Vec3::new(1.0, 2.5, 0.0)
            } else {
                good(r, c)
            }
        });
        let why = refusal(interpolate(&pts, 4, 4, Spacing::Greville, TOL, ANG));
        assert!(
            why.contains("folds near row") || why.contains("doubles back"),
            "{why}"
        );
    }

    #[test]
    fn spans_blossomed_one_by_one_are_the_surfaces_bezier_spans() {
        let pts = grid(6, 9, |r, c| {
            Vec3::new(
                c as f64 * 1.3,
                r as f64 + 0.2 * (c as f64).psin(),
                (c as f64 * r as f64 * 0.1).pcos(),
            )
        });
        for spacing in [Spacing::Greville, Spacing::Chord] {
            let made = interpolate(&pts, 6, 9, spacing, TOL, ANG).unwrap();
            let s = &made.surface;
            let (pu, pv) = s.degrees();
            let whole = s.bezier_spans().unwrap();
            let (su, sv) = (spans(s.knots_u(), pu), spans(s.knots_v(), pv));
            assert_eq!(whole.len(), su.len() * sv.len());
            let ctrl = s.control_points();
            let count_v = s.counts().1;
            let mut k = 0;
            for &(a, ru) in &su {
                for &(b, rv) in &sv {
                    let mut window = Vec::new();
                    for i in a - pu..=a {
                        for j in b - pv..=b {
                            window.push(ctrl[i * count_v + j]);
                        }
                    }
                    let patch =
                        span_patch(window, (s.knots_u(), a, pu), (s.knots_v(), b, pv)).unwrap();
                    let span = &whole[k];
                    assert_eq!((span.u, span.v), (ru, rv));
                    for (p, q) in patch
                        .control_points()
                        .iter()
                        .zip(span.patch.control_points())
                    {
                        assert!(p.distance(q) <= 1e-13, "{spacing:?} span {k}: {p:?} {q:?}");
                    }
                    k += 1;
                }
            }
        }
    }

    #[test]
    fn a_steep_spike_of_a_height_field_is_a_sheet() {
        // Legs turning by 178° at the spike's top: steep, not folded.
        let pts = grid(5, 5, |r, c| {
            let z = if (r, c) == (2, 2) { 60.0 } else { 0.0 };
            Vec3::new(c as f64, r as f64, z)
        });
        for spacing in [Spacing::Greville, Spacing::Chord] {
            let made = interpolate(&pts, 5, 5, spacing, TOL, ANG).unwrap();
            assert!(made.deviation <= made.bound, "{spacing:?}");
        }
    }

    /// A height field over a regular grid of unit steps, `rows × columns`.
    fn field(rows: usize, columns: usize, h: impl Fn(usize, usize) -> f64) -> Vec<Vec3> {
        grid(rows, columns, |r, c| Vec3::new(c as f64, r as f64, h(r, c)))
    }

    #[test]
    fn height_fields_over_a_regular_grid_are_sheets_however_steep() {
        // The shapes a review found refused from 12 to 248 steps high: the
        // normal's z, a product of the parameters' positive speeds, holds
        // every coefficient whatever the heights (the review's least: 0.94).
        type Shape = fn(usize, usize) -> f64;
        let shapes: [(&str, Shape); 5] = [
            (
                "checkerboard",
                |r, c| if (r + c) % 2 == 0 { 1.0 } else { -1.0 },
            ),
            ("ridges", |_, c| if c % 2 == 0 { 1.0 } else { -1.0 }),
            ("one spike", |r, c| if (r, c) == (3, 4) { 1.0 } else { 0.0 }),
            ("a cliff in two rows", |r, c| {
                if c >= 5 && (3..5).contains(&r) {
                    1.0
                } else {
                    0.0
                }
            }),
            (
                "a cliff in every row",
                |_, c| if c >= 5 { 1.0 } else { 0.0 },
            ),
        ];
        for (name, shape) in shapes {
            for h in [12.0, 50.0, 250.0, 1e4] {
                let pts = field(7, 9, |r, c| h * shape(r, c));
                for spacing in [Spacing::Greville, Spacing::Even] {
                    let made = interpolate(&pts, 7, 9, spacing, 1e-6, ANG)
                        .unwrap_or_else(|e| panic!("{name} {h} {spacing:?}: {e}"));
                    assert!(made.deviation <= made.bound, "{name} {h} {spacing:?}");
                }
                // Chord's parameters follow the heights, so its x may run
                // back: refused then, and the refusal says so.
                if let Err(e) = interpolate(&pts, 7, 9, Spacing::Chord, 1e-6, ANG) {
                    assert!(e.to_string().contains("runs back"), "{name} {h}: {e}");
                }
            }
        }
        // Each span has a budget of its own: a rough field is a sheet at
        // any size (refused at 200 × 200 while the budget was the sheet's).
        let mut seed = 0x5eed_0007_u64;
        let mut rough = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
        };
        for n in [60, 200] {
            let heights: Vec<f64> = (0..n * n).map(|_| 3.0 * rough()).collect();
            let pts = field(n, n, |r, c| heights[r * n + c]);
            for spacing in [Spacing::Greville, Spacing::Even] {
                interpolate(&pts, n, n, spacing, TOL, ANG)
                    .unwrap_or_else(|e| panic!("rough {n} {spacing:?}: {e}"));
            }
        }
    }

    #[test]
    fn even_spacing_gives_a_regular_grids_bicubic_height_field_exactly() {
        let f = |x: f64, y: f64| {
            0.5 + 0.3 * x - 0.2 * y + 0.4 * x * x * y * y - 0.25 * x.ppowi(3) * y
                + 0.15 * x.ppowi(3) * y.ppowi(3)
        };
        for (rows, columns) in [(4, 4), (9, 6), (17, 33)] {
            let pts = grid(rows, columns, |r, c| {
                let (x, y) = (c as f64 * 0.5, r as f64 * 0.25);
                Vec3::new(x, y, f(x, y))
            });
            let made = interpolate(&pts, rows, columns, Spacing::Even, TOL, ANG).unwrap();
            let (w, d) = ((columns - 1) as f64 * 0.5, (rows - 1) as f64 * 0.25);
            let (mut off, mut size) = (0.0_f64, 0.0_f64);
            for i in 0..=40 {
                for j in 0..=40 {
                    let (s, t) = (f64::from(i) / 40.0, f64::from(j) / 40.0);
                    let p = made.surface.eval(s, t);
                    size = size.max(p.z.abs());
                    // x and y linear in the parameters, z the field there.
                    off = off.max((p.x - s * w).abs()).max((p.y - t * d).abs());
                    off = off.max((p.z - f(p.x, p.y)).abs());
                }
            }
            // Rounding: a few units in the last place of the size.
            assert!(
                off <= 1e-14 * size.max(1.0),
                "{rows} × {columns}: {off:e} of {size:e}"
            );
        }
    }

    #[test]
    fn even_spacing_never_folds_a_height_field() {
        // A zigzag a thousand steps high: its normal's z is constant.
        let pts = field(9, 11, |r, c| if (r + c) % 2 == 0 { 1e3 } else { -1e3 });
        let made = interpolate(&pts, 9, 11, Spacing::Even, TOL, ANG).unwrap();
        for i in 0..=20 {
            for j in 0..=20 {
                let (_, su, sv) = made
                    .surface
                    .derivatives(f64::from(i) / 20.0, f64::from(j) / 20.0);
                let z = su.cross(sv).z;
                assert!((z - 80.0).abs() <= 1e-9, "{z}");
            }
        }
    }

    #[test]
    fn a_grid_too_far_from_the_origin_for_the_tolerance_is_refused() {
        // At 1e12 from the origin a double holds a position to 1e-4: no
        // model there holds 1e-6.
        let pts = grid(3, 3, |r, c| {
            Vec3::new(1e12 + c as f64, -2e12 + r as f64, 3e12)
        });
        let why = refusal(interpolate(&pts, 3, 3, Spacing::Greville, 1e-6, ANG));
        assert!(
            why.contains("from the origin") && why.contains("no model there holds the tolerance"),
            "{why}"
        );
        // The limit: 4 units in the last place of the largest coordinate
        // within the tolerance, tolerance / (4 ε), 1.126e9 at 1e-6.
        let limit = 1e-6 / (PLACES * f64::EPSILON);
        for (k, built) in [(0.99, true), (1.01, false)] {
            let pts = grid(3, 3, |r, c| {
                Vec3::new(limit * k + c as f64, r as f64, (r * c) as f64)
            });
            let made = interpolate(&pts, 3, 3, Spacing::Even, 1e-6, ANG);
            assert_eq!(made.is_ok(), built, "{k}: {made:?}");
        }
        // Between the bound's limit and that one (a 50 m roof 100 km away,
        // a 2 km site at national-grid coordinates, both in millimetres):
        // built, the proven bound above the tolerance, the deviation found
        // within it.
        for (origin, step) in [
            (Vec3::new(1e8, 0.0, 0.0), 1e4),
            (Vec3::new(5.3e8, 1.8e8, 0.0), 2.5e5),
        ] {
            let pts = grid(7, 7, |r, c| {
                origin
                    + Vec3::new(
                        c as f64 * step,
                        r as f64 * step,
                        3000.0 * ((r + c) as f64 * 0.4).psin(),
                    )
            });
            for spacing in [Spacing::Chord, Spacing::Even, Spacing::Greville] {
                let made = interpolate(&pts, 7, 7, spacing, 1e-6, ANG).unwrap();
                assert!(made.bound > 1e-6, "{origin:?}: {}", made.bound);
                assert!(made.deviation < 1e-6, "{origin:?}: {}", made.deviation);
            }
        }
        // At 1e160, and with a tolerance that takes it, no refusal or
        // report prints a number that is not one.
        for (k, tol) in [(1e160, 1e-6), (1e300, 1e-6), (1e160, 1e300)] {
            let pts = grid(4, 4, |r, c| {
                Vec3::new(c as f64 * k, r as f64 * k, ((r * c) as f64).psin() * k)
            });
            match interpolate(&pts, 4, 4, Spacing::Greville, tol, ANG) {
                Err(e) => {
                    let why = e.to_string();
                    assert!(
                        !why.contains("NaN") && !why.contains("inf"),
                        "{k} {tol}: {why}"
                    );
                }
                Ok(made) => assert!(made.deviation.is_finite() && made.bound.is_finite()),
            }
        }
    }

    #[test]
    fn heights_that_overshoot_are_no_run_back() {
        // An even table 6e5 steps high: x linear in u, so no row runs
        // back in plan, though the heights overshoot between the points
        // (the tangent opposes the leg in space). Built, or refused as not
        // proven with the steepness said: never as a run-back.
        for (spacing, h) in [(Spacing::Even, 6e5), (Spacing::Greville, 4e5)] {
            let pts = field(7, 9, |r, c| if (r + c) % 2 == 0 { h } else { -h });
            if let Err(e) = interpolate(&pts, 7, 9, spacing, 1e-6, ANG) {
                let why = e.to_string();
                assert!(
                    !why.contains("runs back") && why.contains("scale"),
                    "{spacing:?}: {why}"
                );
            }
        }
    }

    #[test]
    fn a_run_back_is_named_and_a_doubt_is_not_called_a_fold() {
        // Points on a plane at uneven steps: at Greville's parameters x(u)
        // runs back (by 2.1e-3, dx/du −0.067 at u = 0.609): named.
        let pts = grid(4, 5, |r, c| {
            Vec3::new([0.0, 1.0, 2.5, 3.0, 5.0][c], r as f64, 0.0)
        });
        let why = refusal(interpolate(&pts, 4, 5, Spacing::Greville, TOL, ANG));
        assert!(
            why.contains("runs back") && why.contains("folds near row"),
            "{why}"
        );
        interpolate(&pts, 4, 5, Spacing::Chord, TOL, ANG).unwrap();
    }
}
