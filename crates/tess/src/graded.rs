//! A grid graded to the shape of a spline face, for a face whose even
//! grid is refused as too fine.
//!
//! The even grid of a spline face (`face.rs`, `seed_cells`) has one step
//! per parameter, sized by a bound on the second derivatives *in the
//! parameters* (Filip, Magedson & Markot 1986). That bound holds the
//! surface point at the middle of a cell's parameters near the middle of
//! the cell's triangle, which is more than a mesh needs: a mesh is held to
//! the surface as a set of points, and a surface drawn at an uneven pace
//! (a rational one whose weights differ: a light weight beside heavy ones
//! lets a sliver of the parameter cross most of the surface) has second
//! derivatives along itself that say nothing of its shape. A plane drawn
//! with weights a thousand apart is a plane; its even grid wants millions
//! of cells a side.
//!
//! Here, for a face on a surface drawn unevenly (the weights within one
//! of its spans more than four apart, [`EVEN_ENOUGH`]) whose even grid is
//! refused, the box of the face is cut into pieces, a tensor grid of them,
//! and each piece is taken as what it is in space:
//!
//! - **Drawn evenly.** A piece is a rational Bézier patch, and the same
//!   patch drawn through a Möbius map of each parameter is the same set of
//!   points (*The NURBS Book* §6.4; Lee & Lucian 1991): a change of
//!   weights only. Each strip of pieces is drawn by the map that brings
//!   the weights at its two ends together (`RationalBezierPatch::
//!   evening_u`, the mean over the strip, so that the map is one of the
//!   parameter alone and the grid stays a tensor grid). Where the weights
//!   have a product form (every surface of revolution, and any surface
//!   redrawn by such maps) that takes the unevenness out, and the bound on
//!   the second derivatives of the piece as drawn is the bound of the
//!   shape, as for the evenly weighted surface.
//! - **Flat within the tolerance.** A patch lies in the convex hull of its
//!   control points whatever its weights. A piece whose net lies in a slab
//!   thinner than the tolerance needs no point inside: every triangle on
//!   points of it lies in the slab with it (the flatness test of
//!   subdivision renderers: Lane, Carpenter, Whitted & Blinn 1980). This
//!   is what no reparametrisation gives where the weights have no product
//!   form: a plane is flat however it is drawn.
//! - **Cut where a cut gains.** A piece whose cells are fewer when it is
//!   cut in two at the middle of its even pace (where its weights
//!   balance) is cut, the piece with the most cells first, each half
//!   drawn evenly again; a piece that gains nothing is left.
//!
//! Each strip then gets the cells its pieces ask for, laid evenly in its
//! own pace; the lines of the grid are those of all the strips. What is
//! proven of a cell is what the even grid proves of its own: a cell of a
//! piece that is not flat is within the tolerance by the bound on the
//! second derivatives of the piece as drawn; a flat piece is one cell,
//! in one slab with its triangles. Every triangle is measured afterwards
//! all the same.
//!
//! Sources: D. Filip, R. Magedson, R. Markot, *Surface algorithms using
//! bounds on derivatives*, CAGD 3 (1986); L. Piegl, W. Tiller, *The NURBS
//! Book* (2nd ed.), §6.4 (reparametrisation by a rational linear
//! function); E. T. Y. Lee, M. L. Lucian, *Möbius reparametrizations of
//! rational B-splines*, CAGD 8 (1991); J. Lane, L. Carpenter, T. Whitted,
//! J. Blinn, *Scan line methods for displaying parametrically defined
//! surfaces*, CACM 23 (1980); J. Zheng, T. Sederberg, *Estimating
//! tessellation parameter intervals for rational curves and surfaces*,
//! ACM TOG 19 (2000), for why the bound of a rational surface's own
//! derivatives, not of its homogeneous net, is the one to take.

use crate::face::{patch_second_bounds, spline_counts};
use craft3d_geom::closure::clamped;
use craft3d_geom::{NurbsSurface, RationalBezierPatch};
use craft3d_math::Portable;

/// The part of the face's box, along a parameter, below which a stretch
/// of a knot span that the box reaches into is the rounding of where the
/// boundary's parameters were found (a face that ends on a knot line): it
/// gets no cells of its own, and the cell beside it reaches to the box.
const SLIVER: f64 = 1e-9;

/// The most pieces a face's box is cut into, both ways together.
const MOST_PIECES: usize = 1 << 14;

/// How many cuts are tried.
const MOST_CUTS: usize = 1 << 10;

/// How many pieces are bounded at most, those of the cuts tried among
/// them: each costs the bounds of some sixty nets (`patch_second_bounds`).
const MOST_BOUNDED: usize = 1 << 13;

/// How far apart the weights within one knot span of a surface may be
/// for the surface to be drawn evenly enough: its even grid is then its
/// grid, and where that is refused the face is, as before. The same
/// measure of even enough as the search for the nearest point of a
/// surface takes for a piece it walks in (`geom`, `nearest_by_pieces`).
/// The arcs a turned or a rounded surface is made of are within it (the
/// weights 1, cos, 1 of an arc of a quarter turn are 1.41 apart, of a
/// third of a turn 2, and of two such arcs across each other 4).
const EVEN_ENOUGH: f64 = 4.0;

/// The most lines of the grid along one parameter (a square grid at the
/// usual limit of 1024 a side has as many cells as one of this many by
/// one).
const MOST_LINES: f64 = 1_048_576.0;

/// A stretch of one parameter and the pace it is drawn at.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    from: f64,
    to: f64,
    /// The parameter at the share `s` of the stretch's even pace is
    /// `from + (to − from) m(s)`, `m(s) = pace s / ((1 − s) + pace s)`.
    pace: f64,
    /// No cut of it gains.
    settled: bool,
}

/// One piece: its patch, drawn at the paces of its two stretches, and the
/// cells it asks for along u and along v.
#[derive(Clone, Debug)]
struct Piece {
    patch: RationalBezierPatch,
    need: (f64, f64),
}

/// The Möbius map of [`Stretch::pace`].
fn moebius(pace: f64, s: f64) -> f64 {
    pace * s / ((1.0 - s) + pace * s)
}

/// Whether the net of `patch`, and so the patch, lies in a slab thinner
/// than `tolerance`: between two planes square to the cross product of
/// the diagonals of its corners.
fn flat(patch: &RationalBezierPatch, tolerance: f64) -> bool {
    let (m, n) = patch.degrees();
    let corner = |i: usize, j: usize| patch.control_point(i * (n + 1) + j);
    let (Some(p00), Some(p0n), Some(pm0), Some(pmn)) =
        (corner(0, 0), corner(0, n), corner(m, 0), corner(m, n))
    else {
        return false;
    };
    let Some(normal) = (pmn - p00).cross(p0n - pm0).normalized() else {
        return false;
    };
    let (mut low, mut high) = (0.0_f64, 0.0_f64);
    for point in patch.points() {
        let height = (point - p00).dot(normal);
        // (Negated: a height that is not a number is not flat.)
        if !height.is_finite() {
            return false;
        }
        low = low.min(height);
        high = high.max(height);
    }
    high - low <= tolerance
}

/// The cells `patch` asks for along u and along v: one each where it is
/// flat, else those of the bound on its second derivatives as it is
/// drawn (infinite without a bound).
fn need(patch: &RationalBezierPatch, tolerance: f64) -> (f64, f64) {
    if flat(patch, tolerance) {
        return (1.0, 1.0);
    }
    let Some((uu, uv, vv)) = patch_second_bounds(patch.clone(), (1.0, 1.0)) else {
        return (f64::INFINITY, f64::INFINITY);
    };
    let (nu, nv) = spline_counts((uu + uv, vv + uv), (1.0, 1.0), tolerance);
    let whole = |n: f64| if n >= 1.0 { n } else { 1.0 };
    // (A count that is not a number stays so, and ends as too fine.)
    (
        if nu.is_nan() {
            f64::INFINITY
        } else {
            whole(nu)
        },
        if nv.is_nan() {
            f64::INFINITY
        } else {
            whole(nv)
        },
    )
}

/// The pieces of one strip drawn evenly along it: each redrawn by the one
/// Möbius factor that brings the strip's end weights together, which is
/// returned (one where there is nothing to even or no factor is found).
fn evened(strip: &mut [RationalBezierPatch], along_u: bool) -> f64 {
    let mut log = 0.0_f64;
    for patch in strip.iter() {
        let factor = if along_u {
            patch.evening_u()
        } else {
            patch.transposed().evening_u()
        };
        match factor {
            Some(factor) => log += factor.pln(),
            None => return 1.0,
        }
    }
    let factor = (log / strip.len().max(1) as f64).pexp();
    if !(factor.is_finite() && factor > 0.0) || factor == 1.0 {
        return 1.0;
    }
    let mut redrawn = Vec::with_capacity(strip.len());
    for patch in strip.iter() {
        let drawn = if along_u {
            patch.redrawn(factor, 1.0)
        } else {
            patch.redrawn(1.0, factor)
        };
        match drawn {
            Some(drawn) => redrawn.push(drawn),
            None => return 1.0,
        }
    }
    for (slot, drawn) in strip.iter_mut().zip(redrawn) {
        *slot = drawn;
    }
    factor
}

/// A stretch of a knot span that a range covers: the span's number, the
/// stretch, and the part `s0..s1` of the span it is.
type Part = (usize, (f64, f64), (f64, f64));

/// The stretches of the knot spans `spans` (in order, each `(from, to)`)
/// that the range `low..high` covers, each with the part `s0..s1` of its
/// span; stretches narrower than [`SLIVER`] of the range are left out.
fn covered(spans: &[(f64, f64)], low: f64, high: f64) -> Vec<Part> {
    let mut out = Vec::new();
    let range = high - low;
    for (index, &(a, b)) in spans.iter().enumerate() {
        let (from, to) = (a.max(low), b.min(high));
        let width = b - a;
        if !(to > from && width > 0.0 && to - from > SLIVER * range) {
            continue;
        }
        out.push((index, (from, to), ((from - a) / width, (to - a) / width)));
    }
    out
}

/// The lines of a grid over the box `low..high` of `surface`'s own
/// parameters, along u and along v, graded to the surface's shape (see
/// the module documentation): each from `low` to `high`, increasing, with
/// at most `limit²` cells between them. `None` where no such grid is
/// found: a surface that is drawn evenly enough (no span with weights
/// more than [`EVEN_ENOUGH`] apart: its even grid is the one it has, and
/// where that is refused the face is, as before), a surface that closes
/// on itself (`closed`: its box may reach across the seam, which is not
/// followed here) or whose knots are not clamped (it may close without
/// showing it), spans that cannot be had, a piece without a bound, more
/// cells than the limit, or more pieces than are looked at.
pub(crate) fn lines(
    surface: &NurbsSurface,
    closed: bool,
    low: (f64, f64),
    high: (f64, f64),
    tolerance: f64,
    limit: f64,
) -> Option<[Vec<f64>; 2]> {
    let usable = |x: f64| x.is_finite();
    if closed
        || !(usable(low.0) && usable(low.1) && usable(high.0) && usable(high.1))
        || !(high.0 > low.0 && high.1 > low.1 && tolerance > 0.0)
    {
        return None;
    }
    let (degree_u, degree_v) = surface.degrees();
    if !(clamped(surface.knots_u(), degree_u) && clamped(surface.knots_v(), degree_v)) {
        return None;
    }
    let spans = surface.bezier_spans_shared().ok()?;
    let uneven = |span: &craft3d_geom::BezierSpan| {
        span.patch
            .weight_range()
            .is_some_and(|(lightest, heaviest)| heaviest > EVEN_ENOUGH * lightest)
    };
    if !spans.iter().any(uneven) {
        return None;
    }
    // The spans are a tensor grid of knot intervals.
    let intervals = |of: fn(&craft3d_geom::BezierSpan) -> (f64, f64)| {
        let mut all: Vec<(f64, f64)> = spans.iter().map(of).collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        all.dedup();
        all
    };
    let (spans_u, spans_v) = (intervals(|s| s.u), intervals(|s| s.v));
    let (parts_u, parts_v) = (
        covered(&spans_u, low.0, high.0),
        covered(&spans_v, low.1, high.1),
    );
    if parts_u.is_empty()
        || parts_v.is_empty()
        || parts_u.len().saturating_mul(parts_v.len()) > MOST_BOUNDED
    {
        return None;
    }
    let mut bounded = parts_u.len().saturating_mul(parts_v.len());
    // The patch of each piece: its span's, cut down to the part covered.
    let mut rows: Vec<Vec<RationalBezierPatch>> = Vec::with_capacity(parts_u.len());
    for &(iu, _, (s0, s1)) in &parts_u {
        let mut row = Vec::with_capacity(parts_v.len());
        for &(iv, _, (t0, t1)) in &parts_v {
            let (su, sv) = (spans_u.get(iu)?, spans_v.get(iv)?);
            let span = spans.iter().find(|s| s.u == *su && s.v == *sv)?;
            let mut patch = span.patch.clone();
            if s0 > 0.0 {
                patch = patch.split_u(s0).1;
            }
            if s1 < 1.0 {
                patch = patch.split_u((s1 - s0) / (1.0 - s0)).0;
            }
            if t0 > 0.0 {
                patch = patch.split_v(t0).1;
            }
            if t1 < 1.0 {
                patch = patch.split_v((t1 - t0) / (1.0 - t0)).0;
            }
            row.push(patch);
        }
        rows.push(row);
    }
    let stretch = |&(_, (from, to), _): &Part| Stretch {
        from,
        to,
        pace: 1.0,
        settled: false,
    };
    let mut along_u: Vec<Stretch> = parts_u.iter().map(stretch).collect();
    let mut along_v: Vec<Stretch> = parts_v.iter().map(stretch).collect();
    // Every strip drawn evenly, along u and then along v.
    for (i, row) in rows.iter_mut().enumerate() {
        along_u.get_mut(i)?.pace = evened(row, true);
    }
    for (j, stretch) in along_v.iter_mut().enumerate() {
        let mut column: Vec<RationalBezierPatch> = Vec::with_capacity(rows.len());
        for row in &rows {
            column.push(row.get(j)?.clone());
        }
        stretch.pace = evened(&mut column, false);
        for (row, patch) in rows.iter_mut().zip(column) {
            *row.get_mut(j)? = patch;
        }
    }
    let mut pieces: Vec<Vec<Piece>> = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|patch| Piece {
                    need: need(&patch, tolerance),
                    patch,
                })
                .collect()
        })
        .collect();

    // The cells of each strip: the most any of its pieces asks for.
    let counts = |pieces: &[Vec<Piece>], columns: usize| -> (Vec<f64>, Vec<f64>) {
        let across_u: Vec<f64> = pieces
            .iter()
            .map(|row| row.iter().fold(1.0_f64, |m, p| m.max(p.need.0)))
            .collect();
        let across_v: Vec<f64> = (0..columns)
            .map(|j| {
                pieces
                    .iter()
                    .filter_map(|row| row.get(j))
                    .fold(1.0_f64, |m, p| m.max(p.need.1))
            })
            .collect();
        (across_u, across_v)
    };
    // The two halves of a strip cut at the middle of its even pace, each
    // drawn evenly again: their patches with their needs, and their paces.
    type Halves = (Vec<Piece>, Vec<Piece>, Stretch, Stretch);
    let halves =
        |strip: Vec<&RationalBezierPatch>, of: &Stretch, along_u: bool| -> Option<Halves> {
            let (mut first, mut second) = (Vec::new(), Vec::new());
            for patch in strip {
                let (a, b) = if along_u {
                    patch.split_u(0.5)
                } else {
                    patch.split_v(0.5)
                };
                first.push(a);
                second.push(b);
            }
            let middle = of.from + (of.to - of.from) * moebius(of.pace, 0.5);
            if !(middle > of.from && middle < of.to) {
                return None;
            }
            // (The paces of the halves of a Möbius map are Möbius maps.)
            let pace_first = 0.5 * (1.0 + of.pace) * evened(&mut first, along_u);
            let pace_second = 2.0 * of.pace / (1.0 + of.pace) * evened(&mut second, along_u);
            let usable = |x: f64| x.is_finite() && x > 0.0;
            if !(usable(pace_first) && usable(pace_second)) {
                return None;
            }
            let with_needs = |patches: Vec<RationalBezierPatch>| -> Vec<Piece> {
                patches
                    .into_iter()
                    .map(|patch| Piece {
                        need: need(&patch, tolerance),
                        patch,
                    })
                    .collect()
            };
            Some((
                with_needs(first),
                with_needs(second),
                Stretch {
                    from: of.from,
                    to: middle,
                    pace: pace_first,
                    settled: false,
                },
                Stretch {
                    from: middle,
                    to: of.to,
                    pace: pace_second,
                    settled: false,
                },
            ))
        };
    for _ in 0..MOST_CUTS {
        let (across_u, across_v) = counts(&pieces, along_v.len());
        // The strip with the most cells that a cut may still help: three
        // or more (two halves have two cells between them at least).
        let mut best: Option<(f64, bool, usize)> = None;
        for (along, stretches, across) in
            [(true, &along_u, &across_u), (false, &along_v, &across_v)]
        {
            for (index, (stretch, &cells)) in stretches.iter().zip(across).enumerate() {
                if !stretch.settled && cells >= 3.0 && best.is_none_or(|(most, _, _)| cells > most)
                {
                    best = Some((cells, along, index));
                }
            }
        }
        let Some((cells, cut_u, index)) = best else {
            break;
        };
        let across = if cut_u { along_v.len() } else { along_u.len() };
        bounded = bounded.saturating_add(across.saturating_mul(2));
        if along_u.len().saturating_mul(along_v.len()) >= MOST_PIECES || bounded > MOST_BOUNDED {
            break;
        }
        let (stretches, strip): (&mut Vec<Stretch>, Vec<&RationalBezierPatch>) = if cut_u {
            (
                &mut along_u,
                pieces.get(index)?.iter().map(|p| &p.patch).collect(),
            )
        } else {
            (
                &mut along_v,
                pieces
                    .iter()
                    .filter_map(|row| row.get(index))
                    .map(|p| &p.patch)
                    .collect(),
            )
        };
        let of = *stretches.get(index)?;
        let most = |strip: &[Piece]| {
            strip.iter().fold(1.0_f64, |m, p| {
                m.max(if cut_u { p.need.0 } else { p.need.1 })
            })
        };
        // Taken where the halves have an eighth fewer cells between them
        // than the strip: a strip drawn evenly gains nothing by a cut.
        let gained = halves(strip, &of, cut_u)
            .filter(|(first, second, _, _)| (most(first) + most(second)) * 8.0 <= cells * 7.0);
        let Some((first, second, stretch_first, stretch_second)) = gained else {
            stretches.get_mut(index)?.settled = true;
            continue;
        };
        *stretches.get_mut(index)? = stretch_first;
        stretches.insert(index + 1, stretch_second);
        if cut_u {
            *pieces.get_mut(index)? = first;
            pieces.insert(index + 1, second);
        } else {
            for (row, (a, b)) in pieces.iter_mut().zip(first.into_iter().zip(second)) {
                *row.get_mut(index)? = a;
                row.insert(index + 1, b);
            }
        }
    }

    let (across_u, across_v) = counts(&pieces, along_v.len());
    let (total_u, total_v): (f64, f64) = (across_u.iter().sum(), across_v.iter().sum());
    // (Negated: a count that is not a number is over the limit. And no
    // more lines a parameter than are kept: the limit is the caller's.)
    if !(total_u * total_v <= limit * limit && total_u <= MOST_LINES && total_v <= MOST_LINES) {
        return None;
    }
    let laid = |stretches: &[Stretch], across: &[f64], low: f64, high: f64| -> Option<Vec<f64>> {
        let mut lines = Vec::new();
        for (stretch, &cells) in stretches.iter().zip(across) {
            // In range by the check of the totals above.
            let cells = cells as u32;
            for k in 0..cells {
                let share = f64::from(k) / f64::from(cells);
                lines.push(
                    stretch.from + (stretch.to - stretch.from) * moebius(stretch.pace, share),
                );
            }
        }
        lines.push(high);
        *lines.first_mut()? = low;
        lines
            .windows(2)
            .all(|w| matches!(w, [a, b] if a < b))
            .then_some(lines)
    };
    Some([
        laid(&along_u, &across_u, low.0, high.0)?,
        laid(&along_v, &across_v, low.1, high.1)?,
    ])
}

#[cfg(test)]
mod tests {
    // Test code: a failure in a helper is a failed test.
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use craft3d_math::Vec3;

    /// A plane over `[0, a] × [0, b]`, one span of `degree` each way,
    /// with the weights `cu^i cv^j corner^(i j)` (to the powers over the
    /// degree): drawn by a Möbius map of each parameter, and with
    /// `corner` by what no such map removes.
    fn sheet(degree: usize, (cu, cv): (f64, f64), corner: f64, bump: f64) -> NurbsSurface {
        let n = degree + 1;
        let mut knots = vec![0.0; n];
        knots.extend(vec![1.0; n]);
        let (mut ctrl, mut weights) = (Vec::new(), Vec::new());
        for i in 0..n {
            for j in 0..n {
                let inside = i > 0 && i < degree && j > 0 && j < degree;
                ctrl.push(Vec3::new(
                    4.0 * i as f64 / degree as f64,
                    3.0 * j as f64 / degree as f64,
                    if inside { bump } else { 0.0 },
                ));
                let d = degree as f64;
                weights.push(
                    cu.ppowf(i as f64 / d)
                        * cv.ppowf(j as f64 / d)
                        * corner.ppowf((i * j) as f64 / (d * d)),
                );
            }
        }
        NurbsSurface::new(degree, degree, knots.clone(), knots, n, n, ctrl, weights).unwrap()
    }

    #[test]
    fn a_plane_is_one_cell_however_it_is_drawn() {
        for degree in 1..=3 {
            for ratio in [30.0, 1e3, 1e6] {
                for (cu, cv, corner) in
                    [(ratio, 1.0, 1.0), (1.0, ratio, 1.0), (ratio, ratio, ratio)]
                {
                    let surface = sheet(degree, (cu, cv), corner, 0.0);
                    let lines =
                        lines(&surface, false, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).unwrap();
                    assert_eq!(lines, [vec![0.0, 1.0], vec![0.0, 1.0]], "{degree} {ratio}");
                }
            }
        }
    }

    /// The cells of a bumped sheet drawn by Möbius maps are those of the
    /// sheet drawn evenly, and its lines are that sheet's lines seen
    /// through the maps: the same points in space.
    #[test]
    fn a_sheet_redrawn_by_moebius_maps_has_the_cells_of_the_even_one() {
        let even = sheet(3, (1.0, 1.0), 1.0, 0.8);
        // A sheet drawn evenly enough has no graded grid: its even grid
        // is its grid. The one it would have is that of the sheet redrawn
        // by a map that just counts.
        assert!(lines(&even, false, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).is_none());
        let mild = sheet(3, (EVEN_ENOUGH, 1.0), 1.0, 0.8);
        assert!(lines(&mild, false, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).is_none());
        let barely = sheet(3, (EVEN_ENOUGH * 1.001, 1.0), 1.0, 0.8);
        let plain = lines(&barely, false, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).unwrap();
        assert!(plain[0].len() > 3 && plain[1].len() > 3, "{plain:?}");
        for ratio in [30.0, 1e3, 1e6] {
            for (cu, cv) in [(ratio, 1.0), (1.0, ratio), (ratio, 1.0 / ratio)] {
                let drawn = sheet(3, (cu, cv), 1.0, 0.8);
                let graded = lines(&drawn, false, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).unwrap();
                assert_eq!(
                    (graded[0].len(), graded[1].len()),
                    (plain[0].len(), plain[1].len()),
                    "{cu} {cv}"
                );
                for (&u, &s) in graded[0].iter().zip(&plain[0]) {
                    for (&v, &t) in graded[1].iter().zip(&plain[1]) {
                        let (p, q) = (drawn.eval(u, v), barely.eval(s, t));
                        assert!(p.distance(q) <= 1e-8, "{cu} {cv}: {p:?} {q:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_boxes_and_surfaces_that_close_give_no_grid() {
        // Knots that are not clamped: no grid.
        let open = NurbsSurface::new(
            1,
            1,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![0.0, 0.0, 1.0, 1.0],
            3,
            2,
            (0..6)
                .map(|k| Vec3::new((k / 2) as f64, (k % 2) as f64, 0.0))
                .collect(),
            vec![1.0, 30.0, 1.0, 30.0, 1.0, 30.0],
        )
        .unwrap();
        assert!(lines(&open, false, (1.0, 0.0), (3.0, 1.0), 1e-3, 1024.0).is_none());
        let surface = sheet(2, (30.0, 1.0), 1.0, 0.5);
        let grid = |low: (f64, f64), high: (f64, f64), tolerance: f64| {
            lines(&surface, false, low, high, tolerance, 1024.0)
        };
        assert!(grid((0.0, 0.0), (1.0, 1.0), 1e-3).is_some());
        assert!(grid((0.0, 0.0), (0.0, 1.0), 1e-3).is_none());
        assert!(grid((f64::NAN, 0.0), (1.0, 1.0), 1e-3).is_none());
        assert!(grid((0.0, 0.0), (f64::INFINITY, 1.0), 1e-3).is_none());
        assert!(grid((0.0, 0.0), (1.0, 1.0), 0.0).is_none());
        assert!(grid((0.0, 0.0), (1.0, 1.0), f64::NAN).is_none());
        assert!(grid((2.0, 2.0), (3.0, 3.0), 1e-3).is_none());
        assert!(lines(&surface, true, (0.0, 0.0), (1.0, 1.0), 1e-3, 1024.0).is_none());
        // More cells than the limit: none, not a coarser grid.
        assert!(lines(&surface, false, (0.0, 0.0), (1.0, 1.0), 1e-12, 4.0).is_none());
    }
}
