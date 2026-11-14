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
//! give that plane, with either spacing. With [`Spacing::Greville`] the
//! space of each direction holds every cubic polynomial, so a bicubic
//! polynomial surface `F(u, v)` sampled at the Greville abscissae `(ξ_c,
//! ξ_r)` is `F` itself. A bicubic height field `z = f(x, y)` over a regular
//! grid is not reproduced by either spacing: its parameters do not follow
//! `x` and `y` linearly (Greville is uneven near the ends, chord follows
//! the heights too), so the height is `f` composed with a cubic map, and
//! the error falls with the grid's step (about 4 times per halving with
//! Greville, about 16 times with chord; *Findings* in
//! docs/design/surfacing.md).
//!
//! **What is refused, by name** (rows and points counted from 1): fewer
//! than 2 × 2 points, more than [`MAX_GRID_POINTS`] or more than
//! [`MAX_GRID_SIDE`] a side, a coordinate that is not a finite number, two
//! neighbouring points within the tolerance (no derivative there), a row or
//! column that doubles back on itself (two legs in a row within the
//! angular tolerance of opposite), chord spacing whose parameters cannot be
//! told apart, and a sheet whose normal is not proven non-zero everywhere
//! (*Regular*, below): the sheet would fold there.
//!
//! **Regular.** Every Bézier span of the result is proven to have a
//! normal that does not vanish: the normal field `∂u × ∂v` of a
//! polynomial patch is a Bernstein form of degrees `(2p − 1, 2q − 1)`
//! (`RationalBezierPatch::normal_net`), so when every coefficient lies on
//! one side of a plane through the origin, by more than its rounding, the
//! normal is not zero anywhere on the span. A height field over a regular
//! grid passes at once however steep (the coefficients' `z` are products
//! of positive differences); a span not proven is halved each way, each
//! quarter proven in turn, at most [`REGULAR_DEPTH`] times, with all the
//! halving of one sheet bounded by [`MAX_REGULAR_PIECES`]. A span
//! not proven is refused with the grid point nearest it. A surface regular
//! everywhere and continuous in its normal (a cubic with simple knots is
//! twice differentiable) does not turn over anywhere.
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

/// Most pieces the proof of one sheet's regularity may look at beyond its
/// spans (the halving of the spans not proven at once).
pub const MAX_REGULAR_PIECES: usize = 1 << 16;

/// The parameters at which the sheet takes its points, one set for each
/// direction (all its rows, or all its columns).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spacing {
    /// Uniform clamped knots, each point at the Greville abscissa of its
    /// control point (the mean of `degree` knots), as a live spline
    /// through points takes them by default.
    #[default]
    Greville,
    /// The chord lengths along each row (each column), as a share of the
    /// row's length, averaged over all rows (columns) (*The NURBS Book*,
    /// eqs. 9.4, 9.5 and §9.2.5), on knots averaged from them (eq. 9.8).
    Chord,
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
    /// module's *At the points*).
    pub bound: f64,
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
    let count = rows.checked_mul(columns).unwrap_or(usize::MAX);
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
            if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
                return refused(format!(
                    "row {}, point {} is ({}, {}, {}): a point of the grid is finite numbers",
                    r + 1,
                    c + 1,
                    p.x,
                    p.y,
                    p.z
                ));
            }
        }
    }
    check_legs(&at, rows, columns, tolerance, angular)?;
    let (pu, pv) = ((columns - 1).min(3), (rows - 1).min(3));
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
    let bound = (gamma(3 * (pu + 1)) + gamma(3 * (pv + 1)) + gamma(4 * (pu.max(pv) + 1)))
        * size
        * 3.0_f64.sqrt();
    if !(deviation <= tolerance) {
        return refused(format!(
            "the sheet through the points misses one by {deviation:e}, more than the tolerance {tolerance:e}: the points are too unevenly spaced for a sheet through them"
        ));
    }
    regular(&surface, &u, &v)?;
    Ok(Grid {
        surface,
        u,
        v,
        deviation,
        bound,
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
                            "row {}: points {} and {} are {len:e} apart, within the tolerance {tolerance:e}: each point of a grid needs a place of its own",
                            line + 1,
                            k,
                            k + 1
                        ),
                        Direction::AcrossRows => format!(
                            "point {} of rows {} and {} are {len:e} apart, within the tolerance {tolerance:e}: each point of a grid needs a place of its own",
                            line + 1,
                            k,
                            k + 1
                        ),
                    });
                }
                if let Some(previous) = before {
                    // The angle between the legs, from their sine and
                    // cosine together (right at any angle).
                    let turn = previous.cross(leg).length().patan2(previous.dot(leg));
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
            let mut knots = vec![0.0; degree + 1];
            for window in params.get(1..n - 1).unwrap_or(&[]).windows(degree) {
                knots.push(window.iter().sum::<f64>() / degree as f64);
            }
            knots.extend(std::iter::repeat_n(1.0, degree + 1));
            Ok((params, knots))
        }
    }
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

/// Proves the surface's normal non-zero everywhere (the module's
/// *Regular*), or names the grid point nearest the first span it could
/// not prove.
fn regular(surface: &NurbsSurface, u: &[f64], v: &[f64]) -> Result<(), GridError> {
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
            if proven(&patch, 0, &mut spent)? {
                continue;
            }
            let (r, c) = (
                nearest(v, 0.5 * (range_v.0 + range_v.1)),
                nearest(u, 0.5 * (range_u.0 + range_u.1)),
            );
            return refused(format!(
                "the sheet through the points folds near row {}, point {}: its normal vanishes or turns over there (a row or a column runs back there, or its points are spaced too unevenly for these parameters: chord spacing follows how far apart they are)",
                r + 1,
                c + 1
            ));
        }
    }
    Ok(())
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

/// Whether the normal net of a polynomial patch (`∂u × ∂v` in Bernstein
/// form, `RationalBezierPatch::normal_net`) lies in an open half-space:
/// every coefficient has a dot product above the rounding margin with one
/// direction (their sum, or the sum of their directions), so the normal,
/// a convex combination of them, does not vanish on the patch. The margin
/// covers the rounding of the coefficients (`1e-12` of the product of the
/// derivative nets' sizes, some 4500 units in the last place) and of the
/// net the patch was cut from (`64 ε` of its reach, carried into the
/// derivatives).
fn normal_clear(patch: &RationalBezierPatch) -> bool {
    let (Some(net), Some((du, dv))) = (patch.normal_net(), patch.derivative_nets()) else {
        return false;
    };
    let size = |net: &[Vec3]| net.iter().map(|d| d.length()).fold(0.0, f64::max);
    let (fu, fv) = (size(&du), size(&dv));
    let reach = patch.points().map(|p| p.length()).fold(0.0, f64::max);
    let (m, n) = patch.degrees();
    let margin = 1e-12 * fu * fv + 64.0 * f64::EPSILON * reach * (m + n) as f64 * (fu + fv);
    if !margin.is_finite() {
        return false;
    }
    let sum = net.iter().fold(Vec3::ZERO, |s, d| s + *d);
    let units = net
        .iter()
        .filter_map(|d| d.normalized())
        .fold(Vec3::ZERO, |s, d| s + d);
    [sum, units].into_iter().any(|d| {
        d.normalized()
            .is_some_and(|d| net.iter().all(|c| c.dot(d) > margin))
    })
}

/// Whether the patch's normal is proven non-zero, halving it up to
/// [`REGULAR_DEPTH`] times; an error past [`MAX_REGULAR_PIECES`].
fn proven(patch: &RationalBezierPatch, depth: usize, spent: &mut usize) -> Result<bool, GridError> {
    if normal_clear(patch) {
        return Ok(true);
    }
    if depth >= REGULAR_DEPTH {
        return Ok(false);
    }
    *spent = spent.saturating_add(4);
    if *spent > MAX_REGULAR_PIECES {
        return refused(format!(
            "the sheet through the points could not be proven free of folds within {MAX_REGULAR_PIECES} pieces: the points are too uneven for a smooth sheet through them"
        ));
    }
    let (low, high) = patch.split_u(0.5);
    for half in [low, high] {
        let (a, b) = half.split_v(0.5);
        for quarter in [a, b] {
            if !proven(&quarter, depth + 1, spent)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
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
}
