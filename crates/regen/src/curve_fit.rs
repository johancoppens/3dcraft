//! The curves of a sketch that regeneration fits: a curve given by formulas
//! (`craft3d_doc::Geometry::Formula`) and a spline through points fitted
//! again at every regeneration (`craft3d_doc::Geometry::Interpolated`); and
//! the B-spline arithmetic both need (the basis functions, interpolation,
//! evaluating and drawing a spline on any clamped knots).
//!
//! **Fitting a formula** ([`fit_formula`]). The curve `(x(t), y(t))` for `t`
//! from `from` to `to` is sampled at 33 places: the ends, and 31 inner ones
//! each moved off its even step by up to a quarter of a step (by the
//! golden ratio, deterministically). A cubic B-spline is interpolated
//! through the samples, each at its chord-length parameter (the length of
//! the polygon through the samples up to it, as a share of the whole), on
//! knots averaged from the parameters (*The NURBS Book*, eqs. 9.5 and
//! 9.8). That satisfies Schoenberg and Whitney's condition, so the banded
//! system is solved without pivoting (the matrix is totally positive); and
//! the spline runs at about even speed, also where the formula's parameter
//! stands still (an involute leaves its base circle at speed zero, which a
//! spline in `t` would turn into a cusp). The formula is then checked at
//! three points inside every span between samples (at 0.309, 0.5 and 0.691
//! of the way in `t`): the distance from each to the spline (its nearest
//! point, found by Newton steps from the point at the same share of the
//! span). A span off by more than half the tolerance is halved (its middle
//! check becomes a sample, each half is checked afresh at the same shares),
//! and the spline is interpolated again, until every check holds. The ends
//! are the formula's end points exactly, and the spline leaves them in the
//! formula's directions there (*The NURBS Book* § 9.2.2, the end
//! derivatives as two more equations, `interpolate_with_ends`), estimated
//! by one-sided differences extrapolated to third order (`end_direction`);
//! an end the formula leaves at speed zero (an involute's base circle) is
//! fitted free, as before, and so is any end when the system with them
//! cannot be solved.
//!
//! **What the fit guarantees.** Every sample lies on the spline (up to
//! rounding), and every check within half the document's tolerance of it,
//! as far as the Newton steps find the spline's nearest point (where the
//! spline comes back close to itself they may settle on a nearer stretch
//! of it than the one that matches, and under-report). Between checks
//! nothing is proven: a formula that wiggles faster than the checks are
//! spaced (32 spans of 3 checks to start with, 129 points before any
//! refinement) can pass unseen. The samples and checks are at shares that
//! no even grid of the parameter holds, so a periodic formula in step with
//! one (a ring of 64 waves) is seen; one in step with these shares by
//! design is not. It ends with an error, never a curve off by more at a
//! check: when the spline needs more than [`MAX_FIT_POINTS`] samples, more
//! than [`MAX_FIT_ROUNDS`] rounds, or a span shorter than the rounding of
//! its parameter (a kink or a jump in the formula), or when the formula
//! fails or is not finite at a sample.

pub use craft3d_kernel::profile::Spline2;

/// The degree of a fitted formula curve.
pub const FIT_DEGREE: usize = 3;
/// Most samples (and so control points) a fitted formula curve may have.
pub const MAX_FIT_POINTS: usize = 4000;
/// Most rounds of refinement of a formula fit.
pub const MAX_FIT_ROUNDS: usize = 48;
/// Spans a formula is first sampled in.
const FIRST_SPANS: usize = 32;
/// The share of a span's length one gap between its points (its ends and
/// checks) may take before the span is taken for a jump. Smooth curves
/// whose speed grows as a power `n` of the parameter from an end take up
/// to `1 − (3/4)^n` (0.94 for `n = 10`).
const JUMP_SHARE: f64 = 0.95;
/// Where in a span the fit is checked against the formula.
///
/// The outer two are at shares no halving reaches, (√5 − 1) / 4 and its
/// mirror, and the first samples are moved off the even grid (below), so
/// a periodic formula in step with an even grid of the parameter (a ring
/// of 64 waves, sampled at multiples of 1/128 of its turn) is not taken
/// for a smooth curve that passes through every sample and check.
const CHECKS: [f64; 3] = [0.309_016_994_374_947_4, 0.5, 0.690_983_005_625_052_6];

/// Where inner sample `i` of the first [`FIRST_SPANS`] spans is, as a share
/// of the range: off its even place by up to a quarter of a span, by the
/// fractional parts of multiples of the golden ratio (deterministic).
fn first_sample(i: usize) -> f64 {
    let n = FIRST_SPANS as f64;
    if i == 0 {
        return 0.0;
    }
    if i >= FIRST_SPANS {
        return 1.0;
    }
    let golden = 0.618_033_988_749_894_9_f64;
    let shift = ((i as f64 * golden).fract() - 0.5) * 0.5;
    (i as f64 + shift) / n
}
/// Most points [`interpolate`] takes: the system that gives the control
/// points is solved densely, which costs the cube of this.
pub const MAX_THROUGH_POINTS: usize = 400;
/// Most points a spline is drawn with.
pub const MAX_DRAWN_POINTS: usize = 4096;

type P2 = [f64; 2];

fn distance(a: P2, b: P2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The span of `knots` that holds `u` (the index `k` with
/// `knots[k] <= u < knots[k + 1]`, the last span for the end of the range)
/// and the `degree + 1` basis functions that are not zero there: those of
/// the control points `k − degree ..= k`. (*The NURBS Book*, algorithms
/// A2.1 and A2.2.) `None` when the knots do not fit `count` control
/// points of this degree. `u` is clamped to the range.
pub fn basis(degree: usize, knots: &[f64], count: usize, u: f64) -> Option<(usize, Vec<f64>)> {
    let mut values = [0.0; BASIS_SLOTS];
    let span = basis_into(degree, knots, count, u, &mut values)?;
    Some((span, values.get(..=degree)?.to_vec()))
}

/// Room for the basis functions of the highest degree a spline here has.
const BASIS_SLOTS: usize = 8;

/// [`basis`] into `values` (the first `degree + 1`), without allocating:
/// the span.
fn basis_into(
    degree: usize,
    knots: &[f64],
    count: usize,
    u: f64,
    values: &mut [f64; BASIS_SLOTS],
) -> Option<usize> {
    if degree == 0
        || degree >= BASIS_SLOTS
        || count <= degree
        || knots.len() != count.checked_add(degree)?.checked_add(1)?
    {
        return None;
    }
    let (start, end) = (*knots.get(degree)?, *knots.get(count)?);
    if !(u.is_finite() && start < end) {
        return None;
    }
    let u = u.clamp(start, end);
    // The last span whose knot is not above u, among degree..count; a
    // binary search, as the knots never go down.
    let inner = knots.get(degree..count)?;
    let span = degree + inner.partition_point(|k| *k <= u).saturating_sub(1);
    let (mut left, mut right) = ([0.0; BASIS_SLOTS], [0.0; BASIS_SLOTS]);
    *values = [0.0; BASIS_SLOTS];
    *values.first_mut()? = 1.0;
    for j in 1..=degree {
        *left.get_mut(j)? = u - knots.get((span + 1).checked_sub(j)?)?;
        *right.get_mut(j)? = knots.get(span + j)? - u;
        let mut saved = 0.0;
        for r in 0..j {
            let width = right.get(r + 1)? + left.get(j - r)?;
            let share = if width > 0.0 {
                values.get(r)? / width
            } else {
                0.0
            };
            *values.get_mut(r)? = saved + right.get(r + 1)? * share;
            saved = left.get(j - r)? * share;
        }
        *values.get_mut(j)? = saved;
    }
    Some(span)
}

/// The point of a clamped spline at `u`, in its knots' range.
pub fn point(spline: &Spline2, u: f64) -> Option<P2> {
    let p = spline.degree;
    let mut values = [0.0; BASIS_SLOTS];
    let span = basis_into(p, &spline.knots, spline.control.len(), u, &mut values)?;
    let first = span.checked_sub(p)?;
    let mut at = [0.0, 0.0];
    for (value, control) in values.iter().zip(spline.control.get(first..=span)?) {
        at[0] += value * control[0];
        at[1] += value * control[1];
    }
    Some(at)
}

/// Points along a clamped spline from start to end, so close together that
/// the polyline through them stays within `tolerance` of the curve (as far
/// as halving a chord shows), and never more than [`MAX_DRAWN_POINTS`].
/// For drawing; empty for a spline that cannot be evaluated.
pub fn polyline(spline: &Spline2, tolerance: f64) -> Vec<P2> {
    let p = spline.degree;
    let (Some(&start), Some(&end)) = (spline.knots.get(p), spline.knots.get(spline.control.len()))
    else {
        return Vec::new();
    };
    let off = |q: P2, a: P2, b: P2| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        if length > 0.0 {
            ((q[0] - a[0]) * dy - (q[1] - a[1]) * dx).abs() / length
        } else {
            distance(q, a)
        }
    };
    // Four pieces per knot span to start with, each halved until flat.
    let mut breaks: Vec<f64> = spline
        .knots
        .get(p..=spline.control.len())
        .unwrap_or(&[])
        .to_vec();
    breaks.dedup();
    let mut todo: Vec<(f64, f64, u32)> = Vec::new();
    for pair in breaks.windows(2).rev() {
        if let [a, b] = *pair {
            for k in (0..4).rev() {
                let (from, to) = (
                    a + (b - a) * f64::from(k) / 4.0,
                    a + (b - a) * f64::from(k + 1) / 4.0,
                );
                todo.push((from, to, 0));
            }
        }
    }
    let mut points = Vec::new();
    if let Some(first) = point(spline, start) {
        points.push(first);
    }
    let coarse = !(tolerance.is_finite() && tolerance > 0.0);
    while let Some((from, to, depth)) = todo.pop() {
        let (Some(a), Some(b)) = (point(spline, from), point(spline, to)) else {
            break;
        };
        let flat = CHECKS.into_iter().all(|s| {
            point(spline, from + (to - from) * s).is_some_and(|q| off(q, a, b) <= tolerance)
        });
        let room = points.len() + todo.len() + 2 < MAX_DRAWN_POINTS;
        if coarse || flat || depth >= 16 || !room {
            points.push(if to >= end {
                spline.control.last().copied().unwrap_or(b)
            } else {
                b
            });
        } else {
            let middle = from + (to - from) / 2.0;
            todo.push((middle, to, depth + 1));
            todo.push((from, middle, depth + 1));
        }
    }
    points
}

/// The control points of the spline of `degree` on clamped uniform knots
/// (`craft3d_doc::clamped_uniform_knots`) that passes through `through`,
/// in order: as many as there are points.
///
/// The spline takes point `i` at the Greville abscissa of control point
/// `i` (the mean of its `degree` inner knots), which makes the system
/// that gives the control points well conditioned whatever the points are
/// (Schoenberg and Whitney's condition holds by construction). The first
/// and last control points are the first and last points themselves.
/// `sketch.spline {through}` fits a spline so once; a live spline
/// (`craft3d_doc::Geometry::Interpolated`) at every regeneration.
pub fn interpolate(degree: u32, through: &[(f64, f64)]) -> Result<Vec<(f64, f64)>, String> {
    let count = through.len();
    let knots = craft3d_doc::clamped_uniform_knots(degree, count);
    let p = usize::try_from(degree).unwrap_or(usize::MAX);
    if knots.is_empty() {
        return Err(format!(
            "a spline of degree {degree} through points needs from {} to {} of them, got {count}",
            p.saturating_add(1),
            craft3d_doc::MAX_SPLINE_POINTS
        ));
    }
    if count > MAX_THROUGH_POINTS {
        return Err(format!(
            "a spline is fitted through at most {MAX_THROUGH_POINTS} points, got {count}; give control points instead"
        ));
    }
    // One row per point: the basis functions at its parameter, and the
    // point as the right-hand side.
    let mut rows: Vec<(Vec<f64>, f64, f64)> = Vec::with_capacity(count);
    for (i, point) in through.iter().enumerate() {
        let inner = knots.get(i + 1..=i + p).ok_or("the knots are too short")?;
        let u = if i == 0 {
            0.0
        } else if i + 1 == count {
            1.0
        } else {
            inner.iter().sum::<f64>() / p as f64
        };
        let (span, values) = basis(p, &knots, count, u).ok_or("the knots do not fit")?;
        let mut row = vec![0.0; count];
        let first = span.checked_sub(p).ok_or("the knots do not fit")?;
        for (slot, value) in row.iter_mut().skip(first).zip(&values) {
            *slot = *value;
        }
        rows.push((row, point.0, point.1));
    }
    // Gaussian elimination with partial pivoting.
    for column in 0..count {
        let pivot = (column..count)
            .max_by(|&a, &b| {
                let size = |r: usize| {
                    rows.get(r)
                        .and_then(|row| row.0.get(column))
                        .map_or(0.0, |v| v.abs())
                };
                size(a).total_cmp(&size(b))
            })
            .ok_or("there are no points")?;
        rows.swap(column, pivot);
        let (top, rest) = rows.split_at_mut(column + 1);
        let lead = top.last().ok_or("a row is missing")?;
        let diagonal = *lead.0.get(column).ok_or("a column is missing")?;
        if diagonal.is_nan() || diagonal.abs() <= 1e-12 {
            return Err("the points do not determine a spline of this degree".into());
        }
        for row in rest {
            let factor = row.0.get(column).copied().unwrap_or(0.0) / diagonal;
            if factor == 0.0 {
                continue;
            }
            for (slot, value) in row.0.iter_mut().zip(&lead.0).skip(column) {
                *slot -= factor * value;
            }
            row.1 -= factor * lead.1;
            row.2 -= factor * lead.2;
        }
    }
    let mut control = vec![(0.0, 0.0); count];
    for i in (0..count).rev() {
        let (row, x, y) = rows.get(i).ok_or("a row is missing")?;
        let (mut x, mut y) = (*x, *y);
        for (value, solved) in row.iter().zip(&control).skip(i + 1) {
            x -= value * solved.0;
            y -= value * solved.1;
        }
        let diagonal = *row.get(i).ok_or("a column is missing")?;
        let solved = (x / diagonal, y / diagonal);
        if !(solved.0.is_finite() && solved.1.is_finite()) {
            return Err("the points do not determine a spline of this degree".into());
        }
        *control.get_mut(i).ok_or("a point is missing")? = solved;
    }
    // The ends are the end points exactly, not within rounding.
    if let (Some(first), Some(point)) = (control.first_mut(), through.first()) {
        *first = *point;
    }
    if let (Some(last), Some(point)) = (control.last_mut(), through.last()) {
        *last = *point;
    }
    Ok(control)
}

/// The live spline of `degree` through `through` as the kernel takes it:
/// [`interpolate`] on clamped uniform knots.
pub fn interpolated(degree: u32, through: &[P2]) -> Result<Spline2, String> {
    let points: Vec<(f64, f64)> = through.iter().map(|p| (p[0], p[1])).collect();
    let control = interpolate(degree, &points)?;
    Ok(Spline2 {
        degree: usize::try_from(degree).map_err(|_| "the degree is too high".to_string())?,
        knots: craft3d_doc::clamped_uniform_knots(degree, control.len()),
        control: control.into_iter().map(|(x, y)| [x, y]).collect(),
    })
}

/// The cubic spline through `points` taking point `i` at `params[i]`
/// (rising from 0 to 1), on knots averaged from the parameters. The
/// system is banded and totally positive, so it is solved without
/// pivoting, in time linear in the count.
fn interpolate_at(params: &[f64], points: &[P2]) -> Result<Spline2, String> {
    let p = FIT_DEGREE;
    let n = points.len();
    if n != params.len() || n <= p {
        return Err(format!("a fit needs at least {} samples", p + 1));
    }
    // p + 1 zeros, n − p − 1 averaged knots, p + 1 ones.
    let mut knots = vec![0.0; p + 1];
    for j in 1..n - p {
        let window = params.get(j..j + p).ok_or("the samples are too few")?;
        knots.push(window.iter().sum::<f64>() / p as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    // Row i holds the columns i − p ..= i + p.
    let width = 2 * p + 1;
    let mut band = vec![0.0; n * width];
    let rhs: Vec<P2> = points.to_vec();
    for (i, u) in params.iter().enumerate() {
        let (span, values) = basis(p, &knots, n, *u).ok_or("the knots do not fit the samples")?;
        let first = span
            .checked_sub(p)
            .ok_or("the knots do not fit the samples")?;
        for (k, value) in values.iter().enumerate() {
            let column = first + k;
            if *value == 0.0 {
                continue;
            }
            let offset = (column + p)
                .checked_sub(i)
                .filter(|o| *o < width)
                .ok_or("the fit's equations are not banded")?;
            *band
                .get_mut(i * width + offset)
                .ok_or("the fit's equations are not banded")? = *value;
        }
    }
    let mut control = band_solve(p, n, band, rhs)?;
    // The ends are the end samples exactly.
    if let (Some(first), Some(sample)) = (control.first_mut(), points.first()) {
        *first = *sample;
    }
    if let (Some(last), Some(sample)) = (control.last_mut(), points.last()) {
        *last = *sample;
    }
    Ok(Spline2 {
        degree: p,
        knots,
        control,
    })
}

/// The cubic through `points` at `params`, as [`interpolate_at`], that
/// leaves its first point along `ends[0]` and arrives at its last along
/// `ends[1]` (unit directions; an end without one is free): *The NURBS
/// Book* § 9.2.2, the end derivatives as two more equations. Each given
/// direction is an end parameter counted twice for the knots (eq. 9.22),
/// and sets the control point next to that end: `P1 = P0 + u_{p+1} / p D0`,
/// with the derivative `D0` the direction times the polygon's length (the
/// speed of a chord-length parametrisation). The other control points
/// solve the collocation of the inner points, a square submatrix of a
/// B-spline collocation matrix, totally positive, so the band is solved
/// without pivoting.
fn interpolate_with_ends(
    params: &[f64],
    points: &[P2],
    ends: [Option<P2>; 2],
) -> Result<Spline2, String> {
    let p = FIT_DEGREE;
    let n = points.len();
    if n != params.len() || n <= p {
        return Err(format!("a fit needs at least {} samples", p + 1));
    }
    let (start, end) = (
        usize::from(ends[0].is_some()),
        usize::from(ends[1].is_some()),
    );
    let m = n + start + end;
    // The parameters with each given end counted twice.
    let mut doubled = Vec::with_capacity(m);
    for (i, u) in params.iter().enumerate() {
        doubled.push(*u);
        if (i == 0 && start == 1) || (i + 1 == n && end == 1) {
            doubled.push(*u);
        }
    }
    let mut knots = vec![0.0; p + 1];
    for j in 1..m - p {
        let window = doubled.get(j..j + p).ok_or("the samples are too few")?;
        knots.push(window.iter().sum::<f64>() / p as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Err("there are no samples".into());
    };
    let length: f64 = points
        .windows(2)
        .map(|w| match *w {
            [a, b] => distance(a, b),
            _ => 0.0,
        })
        .sum();
    let mut control = vec![[0.0; 2]; m];
    *control.first_mut().ok_or("no control points")? = first;
    *control.last_mut().ok_or("no control points")? = last;
    if let Some(d) = ends[0] {
        let step = knots.get(p + 1).ok_or("the knots are too short")? / p as f64 * length;
        *control.get_mut(1).ok_or("no control points")? =
            [first[0] + step * d[0], first[1] + step * d[1]];
    }
    if let Some(d) = ends[1] {
        let step = (1.0 - knots.get(m - 1).ok_or("the knots are too short")?) / p as f64 * length;
        *control.get_mut(m - 2).ok_or("no control points")? =
            [last[0] - step * d[0], last[1] - step * d[1]];
    }
    // Unknown: the control points `lo ..= hi`, one per inner point.
    let lo = 1 + start;
    let hi = m - 2 - end;
    let count = n - 2;
    let width = 2 * p + 1;
    let mut band = vec![0.0; count * width];
    let mut rhs: Vec<P2> = Vec::with_capacity(count);
    for i in 0..count {
        let u = *params.get(i + 1).ok_or("a sample is missing")?;
        let mut value = *points.get(i + 1).ok_or("a sample is missing")?;
        let (span, values) = basis(p, &knots, m, u).ok_or("the knots do not fit the samples")?;
        let first_column = span
            .checked_sub(p)
            .ok_or("the knots do not fit the samples")?;
        for (k, weight) in values.iter().enumerate() {
            let column = first_column + k;
            if *weight == 0.0 {
                continue;
            }
            if column < lo || column > hi {
                let known = control.get(column).ok_or("a control point is missing")?;
                value[0] -= weight * known[0];
                value[1] -= weight * known[1];
                continue;
            }
            let offset = (column - lo + p)
                .checked_sub(i)
                .filter(|o| *o < width)
                .ok_or("the fit's equations are not banded")?;
            *band
                .get_mut(i * width + offset)
                .ok_or("the fit's equations are not banded")? = *weight;
        }
        rhs.push(value);
    }
    let inner = band_solve(p, count, band, rhs)?;
    for (slot, solved) in control.iter_mut().skip(lo).zip(inner) {
        *slot = solved;
    }
    if control.iter().flatten().any(|v| !v.is_finite()) {
        return Err("the samples do not determine a spline".into());
    }
    Ok(Spline2 {
        degree: p,
        knots,
        control,
    })
}

/// The unit direction in which the curve `at(u)` (`u` the share of the
/// range) leaves the end `u0` going inwards along `sign` (`1` at the
/// start, `-1` at the end), as the derivative there: a one-sided
/// difference of second order at the steps `h` and `h / 2`, `h` a
/// thousandth of the range, extrapolated (Richardson) to third order.
/// `None` when the curve does not leave the end at a speed of its own (the
/// estimate under half the secant's over `2 h`: an involute leaves its
/// base circle at speed zero, its direction there in a higher derivative),
/// or when the numbers are not finite: that end is then fitted free.
fn end_direction(
    at: &mut impl FnMut(f64) -> Result<P2, String>,
    u0: f64,
    sign: f64,
) -> Result<Option<P2>, String> {
    let h = 1e-3;
    let f0 = at(u0)?;
    let mut estimate = |step: f64| -> Result<(P2, P2), String> {
        let a = at(u0 + sign * step)?;
        let b = at(u0 + sign * 2.0 * step)?;
        let d = [
            sign * (-3.0 * f0[0] + 4.0 * a[0] - b[0]) / (2.0 * step),
            sign * (-3.0 * f0[1] + 4.0 * a[1] - b[1]) / (2.0 * step),
        ];
        Ok((d, b))
    };
    let (coarse, far) = estimate(h)?;
    let (fine, _) = estimate(h / 2.0)?;
    let d = [
        (4.0 * fine[0] - coarse[0]) / 3.0,
        (4.0 * fine[1] - coarse[1]) / 3.0,
    ];
    let speed = d[0].hypot(d[1]);
    let secant = distance(far, f0) / (2.0 * h);
    if !(speed.is_finite() && secant.is_finite() && speed > 0.0 && speed >= 0.5 * secant) {
        return Ok(None);
    }
    Ok(Some([d[0] / speed, d[1] / speed]))
}

/// Solves the banded system of [`interpolate_at`]: `n` rows, row `i`
/// holding the columns `i − p ..= i + p` in `band` (`2 p + 1` per row),
/// without pivoting (the matrix is totally positive).
fn band_solve(p: usize, n: usize, mut band: Vec<f64>, mut rhs: Vec<P2>) -> Result<Vec<P2>, String> {
    let width = 2 * p + 1;
    let at = |band: &[f64], row: usize, column: usize| -> f64 {
        (column + p)
            .checked_sub(row)
            .filter(|o| *o < width)
            .and_then(|o| band.get(row * width + o))
            .copied()
            .unwrap_or(0.0)
    };
    for column in 0..n {
        let pivot = at(&band, column, column);
        if !(pivot.is_finite() && pivot.abs() > 1e-14) {
            return Err("the samples do not determine a spline".into());
        }
        let lead = *rhs.get(column).ok_or("a sample is missing")?;
        for row in column + 1..n.min(column + p + 1) {
            let factor = at(&band, row, column) / pivot;
            if factor == 0.0 {
                continue;
            }
            for c in column..n.min(column + p + 1) {
                let value = at(&band, column, c);
                if let Some(slot) = (c + p)
                    .checked_sub(row)
                    .filter(|o| *o < width)
                    .and_then(|o| band.get_mut(row * width + o))
                {
                    *slot -= factor * value;
                }
            }
            if let Some(r) = rhs.get_mut(row) {
                r[0] -= factor * lead[0];
                r[1] -= factor * lead[1];
            }
        }
    }
    let mut control = vec![[0.0; 2]; n];
    for i in (0..n).rev() {
        let mut value = *rhs.get(i).ok_or("a sample is missing")?;
        for c in i + 1..n.min(i + p + 1) {
            let a = at(&band, i, c);
            let solved = *control.get(c).ok_or("a sample is missing")?;
            value[0] -= a * solved[0];
            value[1] -= a * solved[1];
        }
        let diagonal = at(&band, i, i);
        let solved = [value[0] / diagonal, value[1] / diagonal];
        if !(solved[0].is_finite() && solved[1].is_finite()) {
            return Err("the samples do not determine a spline".into());
        }
        *control.get_mut(i).ok_or("a sample is missing")? = solved;
    }
    Ok(control)
}

/// The parameters of `points` by the length of the polygon through them,
/// from 0 to 1: the spline then runs at about even speed, also where the
/// formula's parameter does not (an involute leaves its base circle at
/// speed zero). A leg is counted at least a billionth of the whole, so the
/// parameters rise.
fn chord_lengths(points: &[P2]) -> Vec<f64> {
    let legs: Vec<f64> = points
        .windows(2)
        .map(|w| match *w {
            [a, b] => distance(a, b),
            _ => 0.0,
        })
        .collect();
    let total: f64 = legs.iter().sum();
    let least = total * 1e-9;
    let legs: Vec<f64> = legs.into_iter().map(|l| l.max(least)).collect();
    let total: f64 = legs.iter().sum();
    let mut params = Vec::with_capacity(points.len());
    let mut run = 0.0;
    params.push(0.0);
    for leg in &legs {
        run += leg;
        params.push(if total > 0.0 { run / total } else { 0.0 });
    }
    if let Some(last) = params.last_mut() {
        *last = 1.0;
    }
    params
}

/// The derivative of a clamped spline: a spline of one degree less on the
/// inner knots (*The NURBS Book*, eq. 3.4). `None` below degree 2.
fn derivative(spline: &Spline2) -> Option<Spline2> {
    let p = spline.degree;
    if p < 2 {
        return None;
    }
    let knots = &spline.knots;
    let mut control = Vec::with_capacity(spline.control.len().saturating_sub(1));
    for (i, pair) in spline.control.windows(2).enumerate() {
        let [a, b] = *pair else { continue };
        let width = knots.get(i + p + 1)? - knots.get(i + 1)?;
        control.push(if width > 0.0 {
            let f = p as f64 / width;
            [f * (b[0] - a[0]), f * (b[1] - a[1])]
        } else {
            [0.0, 0.0]
        });
    }
    Some(Spline2 {
        degree: p - 1,
        knots: knots.get(1..knots.len().checked_sub(1)?)?.to_vec(),
        control,
    })
}

/// The distance from `target` to the spline, as far as a few Newton steps
/// from `guess` find the nearest point: never less than the true distance
/// to the curve (it is the distance to a point of it).
fn nearest(spline: &Spline2, d1: &Spline2, d2: &Spline2, target: P2, guess: f64) -> Option<f64> {
    let mut u = guess.clamp(0.0, 1.0);
    let mut best = distance(point(spline, u)?, target);
    for _ in 0..12 {
        let (s, s1, s2) = (point(spline, u)?, point(d1, u)?, point(d2, u)?);
        let r = [s[0] - target[0], s[1] - target[1]];
        let slope = s1[0] * r[0] + s1[1] * r[1];
        let bend = s1[0] * s1[0] + s1[1] * s1[1] + s2[0] * r[0] + s2[1] * r[1];
        if !(bend > 0.0 && bend.is_finite()) {
            break;
        }
        let next = (u - slope / bend).clamp(0.0, 1.0);
        if !next.is_finite() || next == u {
            break;
        }
        let step = (next - u).abs();
        u = next;
        best = best.min(distance(point(spline, u)?, target));
        // Converged as far as the parameter can tell.
        if step <= 1e-13 {
            break;
        }
    }
    Some(best)
}

/// A formula curve fitted by [`fit_formula`].
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// On knots from 0 to 1; its first and last control points are the
    /// formula's end points.
    pub spline: Spline2,
    /// The largest distance found from a check of the formula to the
    /// spline.
    pub deviation: f64,
    /// Rounds of refinement it took.
    pub rounds: usize,
}

/// A cubic B-spline within `tolerance / 2` of the curve `formula(t)` for
/// `t` from `from` to `to`, at every check (see the module's text for the
/// method and what it guarantees). `formula` gives the point at a value
/// of the parameter, or why it has none. `from` and `to` are the exact end
/// values: the curve starts at `formula(from)` and ends at `formula(to)`.
pub fn fit_formula(
    mut formula: impl FnMut(f64) -> Result<P2, String>,
    from: f64,
    to: f64,
    tolerance: f64,
) -> Result<Fit, String> {
    if !(from.is_finite() && to.is_finite()) {
        return Err("the range of t is not finite".into());
    }
    if from == to {
        return Err(format!("the range of t is empty: from {from} to {to}"));
    }
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err("the document's tolerance is not positive and finite".into());
    }
    let target = tolerance / 2.0;
    let t_of = |u: f64| {
        if u >= 1.0 {
            to
        } else if u <= 0.0 {
            from
        } else {
            from + (to - from) * u
        }
    };
    let mut at = |u: f64| -> Result<P2, String> {
        let t = t_of(u);
        let point = formula(t)?;
        if point.iter().all(|v| v.is_finite()) {
            Ok(point)
        } else {
            Err(format!("at t = {t} the curve is not finite"))
        }
    };
    let mut samples: Vec<(f64, P2)> = Vec::with_capacity(FIRST_SPANS + 1);
    for i in 0..=FIRST_SPANS {
        let u = first_sample(i);
        samples.push((u, at(u)?));
    }
    let mut checks: Vec<[P2; 3]> = Vec::with_capacity(FIRST_SPANS);
    for pair in samples.windows(2) {
        if let [(a, _), (b, _)] = *pair {
            let mut three = [[0.0; 2]; 3];
            for (slot, s) in three.iter_mut().zip(CHECKS) {
                *slot = at(a + (b - a) * s)?;
            }
            checks.push(three);
        }
    }
    // A curve that does not leave its start is no curve.
    let Some(&(_, first)) = samples.first() else {
        return Err("the curve has no samples".into());
    };
    let reach = samples
        .iter()
        .map(|(_, p)| distance(*p, first))
        .chain(checks.iter().flatten().map(|p| distance(*p, first)))
        .fold(0.0, f64::max);
    if reach <= tolerance {
        return Err(format!(
            "its formulas give one point ({}, {}) over the whole range of t",
            first[0], first[1]
        ));
    }
    // The directions the formula leaves its ends in: the fit leaves them
    // in those, so a curve tangent to the fit at an end is tangent to the
    // formula there (to the estimate's accuracy).
    let ends = [
        end_direction(&mut at, 0.0, 1.0)?,
        end_direction(&mut at, 1.0, -1.0)?,
    ];
    // Where the last round found the fit worst: where samples crowd.
    let mut last_worst = from;
    for round in 0..=MAX_FIT_ROUNDS {
        let points: Vec<P2> = samples.iter().map(|(_, p)| *p).collect();
        let params = chord_lengths(&points);
        let spline = match ends {
            [None, None] => interpolate_at(&params, &points),
            // Free ends where the band cannot be solved with them.
            _ => interpolate_with_ends(&params, &points, ends)
                .or_else(|_| interpolate_at(&params, &points)),
        }
        .map_err(|why| {
            format!(
                "it could not be fitted: {why}, as its samples crowd together near t = {last_worst}. A formula with a kink or a jump is not fitted: split the curve there into two"
            )
        })?;
        let (Some(d1), Some(d2)) = (
            derivative(&spline),
            derivative(&spline).as_ref().and_then(derivative),
        ) else {
            return Err("the fitted spline has no derivatives".into());
        };
        let mut worst = (0.0_f64, 0.0_f64);
        let mut bad = Vec::new();
        for (i, ((pair, three), ends)) in samples
            .windows(2)
            .zip(&checks)
            .zip(params.windows(2))
            .enumerate()
        {
            let (&[(a, pa), (b, pb)], &[ua, ub]) = (pair, ends) else {
                continue;
            };
            // A jump: one gap between the span's points takes nearly all
            // of its length, however short the span. The spline would
            // bridge it, and the checks, all near the bridge, pass: so
            // the span is refined until it cannot be, and the fit fails.
            let walk = [pa, three[0], three[1], three[2], pb];
            let gaps: Vec<f64> = walk
                .windows(2)
                .map(|w| match *w {
                    [p, q] => distance(p, q),
                    _ => 0.0,
                })
                .collect();
            let length: f64 = gaps.iter().sum();
            let widest = gaps.iter().copied().fold(0.0, f64::max);
            let jumps = widest > target && widest > JUMP_SHARE * length;
            let mut off = 0.0_f64;
            for (check, s) in three.iter().zip(CHECKS) {
                let d = nearest(&spline, &d1, &d2, *check, ua + (ub - ua) * s)
                    .ok_or("the fitted spline cannot be evaluated")?;
                if !d.is_finite() {
                    return Err("the fitted spline is not finite".into());
                }
                if d > worst.0 {
                    worst = (d, t_of(a + (b - a) * s));
                    last_worst = worst.1;
                }
                off = off.max(d);
            }
            if off > target || jumps {
                // Into as many pieces as the error says (a cubic's falls
                // with the fourth power of the span), two to eight.
                let ratio = off / target;
                let pieces = if ratio > 1.0 && ratio.is_finite() {
                    (ratio.powf(0.25) * 1.2).ceil().clamp(2.0, 8.0) as usize
                } else {
                    2
                };
                bad.push((i, pieces));
            }
        }
        if bad.is_empty() {
            return Ok(Fit {
                spline,
                deviation: worst.0,
                rounds: round,
            });
        }
        let not_fitted = |why: String| {
            Err(format!(
                "it could not be fitted within half the tolerance ({target:e}): {why}; it is off by {:e} near t = {}. A formula with a kink or a jump is not fitted: split the curve there into two",
                worst.0, worst.1
            ))
        };
        if round == MAX_FIT_ROUNDS {
            return not_fitted(format!("{MAX_FIT_ROUNDS} rounds were not enough"));
        }
        let added: usize = bad.iter().map(|(_, k)| k.saturating_sub(1)).sum();
        if samples.len().saturating_add(added) > MAX_FIT_POINTS {
            return not_fitted(format!("it would take more than {MAX_FIT_POINTS} points"));
        }
        // Cut every span that is off into its pieces, each checked afresh
        // at the same shares of its own.
        let mut next_samples = Vec::with_capacity(samples.len() + added);
        let mut next_checks = Vec::with_capacity(checks.len() + added);
        let mut bad = bad.into_iter().peekable();
        for (i, (pair, three)) in samples.windows(2).zip(&checks).enumerate() {
            let [(a, pa), (b, _)] = *pair else { continue };
            next_samples.push((a, pa));
            let pieces = match bad.peek() {
                Some(&(at_span, pieces)) if at_span == i => pieces,
                _ => {
                    next_checks.push(*three);
                    continue;
                }
            };
            bad.next();
            let cuts: Vec<f64> = (0..=pieces)
                .map(|j| {
                    if j == pieces {
                        b
                    } else {
                        a + (b - a) * j as f64 / pieces as f64
                    }
                })
                .collect();
            if cuts.windows(2).any(|w| matches!(*w, [p, q] if q <= p)) {
                return not_fitted(format!(
                    "a span shrank to the rounding of its parameter at t = {}",
                    t_of(a)
                ));
            }
            for (j, piece) in cuts.windows(2).enumerate() {
                let &[from, to] = piece else { continue };
                if j > 0 {
                    next_samples.push((from, at(from)?));
                }
                let mut fresh = [[0.0; 2]; 3];
                for (slot, s) in fresh.iter_mut().zip(CHECKS) {
                    *slot = at(from + (to - from) * s)?;
                }
                next_checks.push(fresh);
            }
        }
        if let Some(end) = samples.last() {
            next_samples.push(*end);
        }
        samples = next_samples;
        checks = next_checks;
    }
    Err("the fit did not converge".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_circle_is_fitted_within_the_tolerance() {
        for (radius, tolerance) in [
            (1e-3, 1e-9),
            (1.0, 1e-6),
            (20.0, 1e-6),
            (1e6, 1e-3),
            (1e6, 1e-6),
        ] {
            let fit = fit_formula(
                |t: f64| Ok([radius * t.cos(), radius * t.sin()]),
                0.0,
                3.0,
                tolerance,
            )
            .unwrap();
            assert!(fit.deviation <= tolerance / 2.0);
            assert_eq!(fit.spline.control[0], [radius, 0.0]);
            assert_eq!(
                *fit.spline.control.last().unwrap(),
                [radius * 3f64.cos(), radius * 3f64.sin()]
            );
            // Densely, well between the checks, the spline stays on the
            // circle: its distance from it is how far it is off the radius.
            for k in 0..=20000 {
                let p = point(&fit.spline, f64::from(k) / 20000.0).unwrap();
                let d = (p[0].hypot(p[1]) - radius).abs();
                assert!(d <= tolerance, "r {radius}: {d:e} at {k}");
            }
            println!(
                "circle r {radius} tol {tolerance}: {} control points, {} rounds",
                fit.spline.control.len(),
                fit.rounds
            );
        }
    }

    /// An involute leaves its base circle at speed zero in its parameter:
    /// the fit has no cusp there (its first leg points along the curve).
    #[test]
    fn an_involute_is_fitted_without_a_cusp() {
        let rb = 20.0;
        let fit = fit_formula(
            |t: f64| Ok([rb * (t.cos() + t * t.sin()), rb * (t.sin() - t * t.cos())]),
            0.0,
            0.6,
            1e-6,
        )
        .unwrap();
        let c = &fit.spline.control;
        let leg = [c[1][0] - c[0][0], c[1][1] - c[0][1]];
        // Radial at the base circle: along +x from (rb, 0), within a
        // fraction of a degree.
        assert!(leg[0] > 0.0 && leg[1].abs() < 1e-2 * leg[0], "{leg:?}");
        assert!(fit.deviation <= 0.5e-6);
    }

    #[test]
    fn what_cannot_be_fitted_is_refused_with_a_reason() {
        let err = fit_formula(|_| Ok([1.0, 2.0]), 0.0, 1.0, 1e-6).unwrap_err();
        assert!(err.contains("one point"), "{err}");
        let err = fit_formula(|t| Ok([t, 0.0]), 1.0, 1.0, 1e-6).unwrap_err();
        assert!(err.contains("empty"), "{err}");
        let err = fit_formula(|t| Ok([t, 1.0 / (t - 0.5)]), 0.0, 1.0, 1e-6).unwrap_err();
        assert!(
            err.contains("not finite") || err.contains("not be fitted"),
            "{err}"
        );
        let err = fit_formula(
            |t| {
                if t > 0.7 {
                    Err("no".into())
                } else {
                    Ok([t, t])
                }
            },
            0.0,
            1.0,
            1e-6,
        )
        .unwrap_err();
        assert_eq!(err, "no");
        // A jump: refused, never a curve across it.
        let err =
            fit_formula(|t| Ok([t, if t < 0.3 { 0.0 } else { 1.0 }]), 0.0, 1.0, 1e-6).unwrap_err();
        assert!(
            err.contains("not be fitted") && err.contains("split"),
            "{err}"
        );
        for bad in [f64::NAN, f64::INFINITY] {
            assert!(fit_formula(|t| Ok([t, t]), 0.0, bad, 1e-6).is_err());
            assert!(fit_formula(|t| Ok([t, t]), 0.0, 1.0, bad).is_err());
        }
        assert!(fit_formula(|t| Ok([t, t]), 0.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn a_fit_runs_backwards_and_is_deterministic() {
        let f = |t: f64| Ok([t, (3.0 * t).sin()]);
        let a = fit_formula(f, 2.0, -1.0, 1e-6).unwrap();
        let b = fit_formula(f, 2.0, -1.0, 1e-6).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.spline.control[0], [2.0, 6f64.sin()]);
        assert_eq!(*a.spline.control.last().unwrap(), [-1.0, (-3f64).sin()]);
    }

    #[test]
    fn the_derivative_of_a_spline_is_its_slope() {
        let fit = fit_formula(|t: f64| Ok([t, t * t * t]), 0.0, 2.0, 1e-6).unwrap();
        let d1 = derivative(&fit.spline).unwrap();
        for k in 1..100 {
            let u = f64::from(k) / 100.0;
            let h = 1e-6;
            let (a, b) = (
                point(&fit.spline, u - h).unwrap(),
                point(&fit.spline, u + h).unwrap(),
            );
            let slope = point(&d1, u).unwrap();
            assert!((slope[0] - (b[0] - a[0]) / (2.0 * h)).abs() < 1e-4 * slope[0].abs().max(1.0));
            assert!((slope[1] - (b[1] - a[1]) / (2.0 * h)).abs() < 1e-4 * slope[1].abs().max(1.0));
        }
    }

    /// A kink is fitted, rounded within the tolerance, or refused: never a
    /// curve that is off.
    #[test]
    fn a_kink_is_rounded_within_the_tolerance_or_refused() {
        match fit_formula(|t: f64| Ok([t, (t - 0.5).abs()]), 0.0, 1.0, 1e-6) {
            Ok(fit) => {
                for k in 0..=20000 {
                    let p = point(&fit.spline, f64::from(k) / 20000.0).unwrap();
                    // The distance to the two legs of the V.
                    let d = ((p[1] - (p[0] - 0.5).abs()) / 2f64.sqrt()).abs();
                    assert!(d <= 1e-6, "{d:e} at {p:?}");
                }
                println!("kink: {} control points", fit.spline.control.len());
            }
            Err(why) => assert!(why.contains("split"), "{why}"),
        }
    }

    /// The largest distance from `count` points of the formula, evenly in
    /// `t` (a grid that is none of the fit's), to the fitted spline.
    fn densely_off(fit: &Fit, f: impl Fn(f64) -> P2, from: f64, to: f64, count: u32) -> f64 {
        let s = &fit.spline;
        let (d1, d2) = (
            derivative(s).unwrap(),
            derivative(&derivative(s).unwrap()).unwrap(),
        );
        let along: Vec<(f64, P2)> = (0..=8 * count)
            .map(|k| {
                let u = f64::from(k) / f64::from(8 * count);
                (u, point(s, u).unwrap())
            })
            .collect();
        let mut worst = 0.0_f64;
        // The curves run forwards together: the nearest point of the
        // spline is searched near the last one.
        let mut last: usize = 0;
        for k in 0..=count {
            let p = f(from + (to - from) * f64::from(k) / f64::from(count));
            let window = last.saturating_sub(64)..(last + 256).min(along.len());
            let (index, guess) = along[window.clone()]
                .iter()
                .enumerate()
                .min_by(|a, b| distance(a.1.1, p).total_cmp(&distance(b.1.1, p)))
                .map(|(i, a)| (window.start + i, a.0))
                .unwrap();
            last = index;
            worst = worst.max(nearest(s, &d1, &d2, p, guess).unwrap());
        }
        worst
    }

    /// A ring of 64 waves is in step with every even grid of a 128th of
    /// its turn: sampled there it is a plain circle. The fit sees the waves.
    #[test]
    fn a_periodic_formula_in_step_with_an_even_grid_is_seen() {
        let ring = |t: f64| {
            let r = 20.0 + (64.0 * t).sin();
            [r * t.cos(), r * t.sin()]
        };
        let fit = fit_formula(|t| Ok(ring(t)), 0.0, std::f64::consts::TAU, 1e-4).unwrap();
        let off = densely_off(&fit, ring, 0.0, std::f64::consts::TAU, 6007);
        assert!(
            off <= 1e-4,
            "ring: {off:e}, {} points",
            fit.spline.control.len()
        );
        assert!(fit.spline.control.len() > 64 * 4);
        let wave = |t: f64| [t, t.sin()];
        let end = 256.0 * std::f64::consts::PI;
        // 128 periods of a sine: fitted within a coarse tolerance, and
        // refused (never a flat line) where it would take too many points.
        let refused = fit_formula(|t| Ok(wave(t)), 0.0, end, 1e-6).unwrap_err();
        assert!(refused.contains("more than"), "{refused}");
        let fit = fit_formula(|t| Ok(wave(t)), 0.0, end, 1e-3).unwrap();
        let off = densely_off(&fit, wave, 0.0, end, 12007);
        assert!(
            off <= 1e-3,
            "sine:{off:e}, {} points",
            fit.spline.control.len()
        );
        println!(
            "ring of 64 waves and 128 sine periods: {} control points",
            fit.spline.control.len()
        );
    }

    #[test]
    fn a_line_needs_no_refinement() {
        let fit = fit_formula(|t| Ok([t, 2.0 * t]), 0.0, 1.0, 1e-6).unwrap();
        assert_eq!(fit.rounds, 0);
        // One sample per first span and its ends, and one more control
        // point beside each end for its direction (2026-10-28).
        assert_eq!(fit.spline.control.len(), FIRST_SPANS + 3);
    }

    /// The fit leaves its ends in the directions the formula does: the
    /// parabola y = t^2 on [0, 1] along (1, 0) and arrives along (1, 2),
    /// a circle along its tangents, at several scales; an involute, which
    /// leaves its base circle at speed zero, leaves it free.
    #[test]
    fn the_fit_leaves_its_ends_as_the_formula_does() {
        let direction = |fit: &Fit, start: bool| {
            let c = &fit.spline.control;
            let (a, b) = if start {
                (c[0], c[1])
            } else {
                (c[c.len() - 2], c[c.len() - 1])
            };
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let n = dx.hypot(dy);
            [dx / n, dy / n]
        };
        let off = |d: [f64; 2], want: [f64; 2]| {
            let n = want[0].hypot(want[1]);
            (d[0] * want[1] / n - d[1] * want[0] / n).abs()
        };
        for k in [1e-3, 1.0, 1e3, 1e6] {
            let fit = fit_formula(|t| Ok([k * t, k * t * t]), 0.0, 1.0, 1e-6 * k).unwrap();
            assert!(off(direction(&fit, true), [1.0, 0.0]) < 1e-12, "x{k}");
            assert!(off(direction(&fit, false), [1.0, 2.0]) < 1e-12, "x{k}");
            let circle = |t: f64| Ok([10.0 * k * t.cos(), 10.0 * k * t.sin()]);
            let fit = fit_formula(circle, 0.3, 2.0, 1e-6 * k).unwrap();
            assert!(
                off(direction(&fit, true), [-(0.3f64).sin(), 0.3f64.cos()]) < 1e-8,
                "x{k}"
            );
            assert!(
                off(direction(&fit, false), [-(2.0f64).sin(), 2.0f64.cos()]) < 1e-8,
                "x{k}"
            );
        }
        let involute = |t: f64| Ok([t.cos() + t * t.sin(), t.sin() - t * t.cos()]);
        let fit = fit_formula(involute, 0.0, 1.0, 1e-6).unwrap();
        // Free at the base circle: the fit's own interpolation there.
        assert!(fit.deviation <= 0.5e-6);
        // At t = 1 it runs along (t cos t, t sin t).
        assert!(off(direction(&fit, false), [1.0f64.cos(), 1.0f64.sin()]) < 1e-8);
        let mut at = |u: f64| involute(u);
        assert_eq!(end_direction(&mut at, 0.0, 1.0).unwrap(), None);
    }
}
