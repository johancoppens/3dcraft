//! The nonzero fill rule resolved into simple contours.
//!
//! A glyph may be drawn with contours that overlap (the stem and the bowl
//! of a variable font's "b" as two contours), or with one that crosses
//! itself. The font says which points are inside: those the contours wind
//! round a nonzero number of times (TrueType and CFF both fill so). A
//! profile in a sketch must be simple loops that neither touch nor cross,
//! so the outline is replaced by the boundary of the region it fills:
//!
//! 1. Every pair of segments is intersected (by subdivision of their
//!    control polygons down to `eps`; a pair whose boxes do not meet is
//!    skipped). Points where the curves meet are made one point, exactly,
//!    on both.
//! 2. Every segment is cut at the points on it. A piece is a boundary of
//!    the filled region when the winding number just left of its middle
//!    and just right of it differ in being zero; it is kept, turned so
//!    the filled side is on its left (counter-clockwise outer loops, y up).
//! 3. The kept pieces are joined end to end into closed contours, and
//!    pieces of one segment that follow each other are made one again.
//!
//! A contour that meets no other is kept or dropped whole, by one test.
//! The winding number of a point is exact for the curves (crossings of a
//! ray with the y-monotone parts of each segment, found by bisection), so
//! the result does not depend on a flattening.
//!
//! Two contours that run along each other (the same curve twice) are
//! refused: the pieces where they coincide have no side.

use crate::FontError;
use crate::bezier::{Contour, P2, Seg, cross, dist, sub};

/// Most segments of one glyph the rule is resolved for.
pub const MAX_FILL_SEGMENTS: usize = 20_000;
/// Most points where two segments meet in one glyph.
const MAX_HITS: usize = 50_000;
/// Deepest subdivision of a pair of segments.
const MAX_DEPTH: u32 = 60;
/// Most subdivision steps for one glyph.
const MAX_STEPS: usize = 4_000_000;

#[derive(Clone, Copy)]
struct Item {
    contour: usize,
    seg: Seg,
    bbox: [f64; 4],
}

/// A y-monotone part of a segment, for the winding number.
#[derive(Clone, Copy)]
struct Mono {
    seg: Seg,
    t0: f64,
    t1: f64,
    y0: f64,
    y1: f64,
    xmin: f64,
    xmax: f64,
}

/// The winding number of `p` about every segment of `monos`: crossings
/// of the ray to +x, upward ones counting +1, downward −1. Each part is
/// half open in y (`y0 <= y < y1` going up), so a ray through a joint
/// counts it once.
fn winding(monos: &[Mono], p: P2) -> i64 {
    let mut w = 0i64;
    for m in monos {
        let (lo, hi, up) = if m.y0 < m.y1 {
            (m.y0, m.y1, true)
        } else {
            (m.y1, m.y0, false)
        };
        if !(p[1] >= lo && p[1] < hi) || m.xmax < p[0] {
            continue;
        }
        let x = if m.xmin > p[0] {
            m.xmin
        } else {
            // Bisection on the monotone part for the t where y = p.y.
            let (mut a, mut b) = (m.t0, m.t1);
            for _ in 0..64 {
                let mid = 0.5 * (a + b);
                let y = m.seg.eval(mid)[1];
                if (y < p[1]) == up {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            m.seg.eval(0.5 * (a + b))[0]
        };
        if x > p[0] {
            w += if up { 1 } else { -1 };
        }
    }
    w
}

fn monotone_parts(items: &[Item]) -> Vec<Mono> {
    let mut out = Vec::new();
    for item in items {
        let s = item.seg;
        let mut cuts = vec![0.0];
        cuts.extend(s.y_extrema());
        cuts.push(1.0);
        for w in cuts.windows(2) {
            let [t0, t1] = *w else { continue };
            if t1 <= t0 {
                continue;
            }
            let part = s.sub_range(t0, t1);
            let (y0, y1) = (s.eval(t0)[1], s.eval(t1)[1]);
            if y0 == y1 {
                continue;
            }
            let b = part.bbox();
            out.push(Mono {
                seg: s,
                t0,
                t1,
                y0,
                y1,
                xmin: b[0],
                xmax: b[2],
            });
        }
    }
    out
}

fn overlap(a: [f64; 4], b: [f64; 4], eps: f64) -> bool {
    a[0] <= b[2] + eps && b[0] <= a[2] + eps && a[1] <= b[3] + eps && b[1] <= a[3] + eps
}

struct Search {
    steps: usize,
    eps: f64,
}

/// Where `a` (on `ta`) and `b` (on `tb`) meet: the parameters on each.
fn meet(
    a: &Seg,
    ta: (f64, f64),
    b: &Seg,
    tb: (f64, f64),
    depth: u32,
    search: &mut Search,
    out: &mut Vec<(f64, f64)>,
) -> Result<(), FontError> {
    search.steps += 1;
    if search.steps > MAX_STEPS || out.len() > MAX_HITS {
        return Err(FontError::TooComplex(
            "its contours meet in too many places".into(),
        ));
    }
    let (ba, bb) = (a.bbox(), b.bbox());
    if !overlap(ba, bb, search.eps) {
        return Ok(());
    }
    let size = |b: [f64; 4]| (b[2] - b[0]).max(b[3] - b[1]);
    let (sa, sb) = (size(ba), size(bb));
    if (sa <= search.eps && sb <= search.eps) || depth >= MAX_DEPTH {
        out.push((0.5 * (ta.0 + ta.1), 0.5 * (tb.0 + tb.1)));
        return Ok(());
    }
    let mid = |t: (f64, f64)| 0.5 * (t.0 + t.1);
    if sa >= sb {
        let (a0, a1) = a.split(0.5);
        let m = mid(ta);
        meet(&a0, (ta.0, m), b, tb, depth + 1, search, out)?;
        meet(&a1, (m, ta.1), b, tb, depth + 1, search, out)
    } else {
        let (b0, b1) = b.split(0.5);
        let m = mid(tb);
        meet(a, ta, &b0, (tb.0, m), depth + 1, search, out)?;
        meet(a, ta, &b1, (m, tb.1), depth + 1, search, out)
    }
}

/// Whether two straight segments lie on one line and share a stretch
/// longer than `eps`.
fn collinear_overlap(a: &Seg, b: &Seg, eps: f64) -> bool {
    if a.degree != 1 || b.degree != 1 {
        return false;
    }
    let d = sub(a.end(), a.start());
    let len = d[0].hypot(d[1]);
    if !(len > eps) {
        return false;
    }
    let off = |p: P2| cross(d, sub(p, a.start())) / len;
    if off(b.start()).abs() > eps || off(b.end()).abs() > eps {
        return false;
    }
    let along = |p: P2| (sub(p, a.start())[0] * d[0] + sub(p, a.start())[1] * d[1]) / len;
    let (u0, u1) = (along(b.start()), along(b.end()));
    let (lo, hi) = (u0.min(u1).max(0.0), u0.max(u1).min(len));
    hi - lo > eps
}

/// A point where segments meet, made one.
struct Node {
    at: P2,
}

/// The region `contours` fill by the nonzero rule, as simple closed
/// contours, filled side on the left. `eps` is the distance below which
/// two points are one (in the contours' units).
pub fn fill_nonzero(contours: &[Contour], eps: f64) -> Result<Vec<Contour>, FontError> {
    if !(eps.is_finite() && eps > 0.0) {
        return Err(FontError::Malformed(
            "the fill tolerance is not positive".into(),
        ));
    }
    // Clean: drop segments of no length, keep the chains joined exactly.
    let mut cleaned: Vec<Contour> = Vec::new();
    let mut total = 0usize;
    for c in contours {
        if !c.closed {
            continue;
        }
        let mut segs: Vec<Seg> = Vec::with_capacity(c.segs.len());
        for s in &c.segs {
            if s.points()
                .iter()
                .any(|p| !(p[0].is_finite() && p[1].is_finite()))
            {
                return Err(FontError::Malformed("a point is not finite".into()));
            }
            if s.is_point(eps) {
                continue;
            }
            let mut s = *s;
            if let Some(prev) = segs.last() {
                s.set_start(prev.end());
            }
            segs.push(s);
        }
        if let (Some(first), Some(last)) = (segs.first().copied(), segs.last_mut()) {
            if dist(last.end(), first.start()) > eps * 64.0 {
                // A contour the font did not close is closed by a line.
                let closing = Seg::line(last.end(), first.start());
                segs.push(closing);
            } else {
                last.set_end(first.start());
            }
        }
        let contour = Contour { segs, closed: true };
        if contour.segs.is_empty() || contour.signed_area().abs() <= eps * eps {
            continue;
        }
        total = total.saturating_add(contour.segs.len());
        if total > MAX_FILL_SEGMENTS {
            return Err(FontError::TooComplex(format!(
                "it has more than {MAX_FILL_SEGMENTS} segments"
            )));
        }
        cleaned.push(contour);
    }
    let mut items = Vec::with_capacity(total);
    for (ci, c) in cleaned.iter().enumerate() {
        for s in &c.segs {
            items.push(Item {
                contour: ci,
                seg: *s,
                bbox: s.bbox(),
            });
        }
    }
    let monos = monotone_parts(&items);
    // Where the segments meet: per segment, (t, node).
    let mut cuts: Vec<Vec<(f64, usize)>> = vec![Vec::new(); items.len()];
    let mut nodes: Vec<Node> = Vec::new();
    let mut search = Search { steps: 0, eps };
    // The index of the segment after `i` in its contour.
    let mut next_of = vec![0usize; items.len()];
    {
        let mut start = 0usize;
        for c in &cleaned {
            let n = c.segs.len();
            for k in 0..n {
                if let Some(slot) = next_of.get_mut(start + k) {
                    *slot = start + (k + 1) % n;
                }
            }
            start += n;
        }
    }
    // Pairs whose boxes meet, by a sweep in x.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (
            items.get(i).map_or(0.0, |x| x.bbox[0]),
            items.get(j).map_or(0.0, |x| x.bbox[0]),
        );
        a.total_cmp(&b).then(i.cmp(&j))
    });
    for (oi, &i) in order.iter().enumerate() {
        let Some(a) = items.get(i).copied() else {
            continue;
        };
        for &j in order.iter().skip(oi + 1) {
            let Some(b) = items.get(j).copied() else {
                continue;
            };
            if b.bbox[0] > a.bbox[2] + eps {
                break;
            }
            if !overlap(a.bbox, b.bbox, eps) {
                continue;
            }
            if collinear_overlap(&a.seg, &b.seg, eps) {
                return Err(FontError::Unsupported(
                    "two of its contours run along each other".into(),
                ));
            }
            let adjacent_ij = next_of.get(i) == Some(&j);
            let adjacent_ji = next_of.get(j) == Some(&i);
            let mut hits = Vec::new();
            if adjacent_ij || adjacent_ji {
                // Segments that share a joint meet there, and run close to
                // each other near it when the joint is smooth: the halves
                // that hold the joint are not intersected with each other.
                let halves = [(0.0, 0.5), (0.5, 1.0)];
                for (ka, ra) in halves.iter().enumerate() {
                    for (kb, rb) in halves.iter().enumerate() {
                        if (adjacent_ij && ka == 1 && kb == 0)
                            || (adjacent_ji && ka == 0 && kb == 1)
                        {
                            continue;
                        }
                        let sa = a.seg.sub_range(ra.0, ra.1);
                        let sb = b.seg.sub_range(rb.0, rb.1);
                        meet(&sa, *ra, &sb, *rb, 0, &mut search, &mut hits)?;
                    }
                }
            } else {
                meet(
                    &a.seg,
                    (0.0, 1.0),
                    &b.seg,
                    (0.0, 1.0),
                    0,
                    &mut search,
                    &mut hits,
                )?;
            }
            if hits.is_empty() {
                continue;
            }
            // Cluster the hits by where they are.
            hits.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut clusters: Vec<Vec<(f64, f64)>> = Vec::new();
            for h in hits {
                let p = a.seg.eval(h.0);
                let joined = clusters
                    .iter_mut()
                    .find(|c| c.iter().any(|q| dist(a.seg.eval(q.0), p) <= 8.0 * eps));
                match joined {
                    Some(c) => c.push(h),
                    None => clusters.push(vec![h]),
                }
            }
            for cluster in clusters {
                let n = cluster.len() as f64;
                let (sa, sb) = cluster
                    .iter()
                    .fold((0.0, 0.0), |acc, h| (acc.0 + h.0, acc.1 + h.1));
                let (ta, tb) = (sa / n, sb / n);
                // How far the cluster spreads: two curves along each other
                // meet in a run, not a point.
                let spread = cluster
                    .iter()
                    .fold(0.0f64, |m, h| m.max(dist(a.seg.eval(h.0), a.seg.eval(ta))));
                let pa = a.seg.eval(ta);
                // The joint two segments of a contour share is not a
                // crossing.
                let at_joint = |s: &Seg, t: f64, end: bool| {
                    let joint = if end { s.end() } else { s.start() };
                    dist(s.eval(t), joint) <= 16.0 * eps
                };
                if (adjacent_ij && at_joint(&a.seg, ta, true) && at_joint(&b.seg, tb, false))
                    || (adjacent_ji && at_joint(&a.seg, ta, false) && at_joint(&b.seg, tb, true))
                {
                    continue;
                }
                if spread > 64.0 * eps {
                    return Err(FontError::Unsupported(
                        "two of its contours run along each other".into(),
                    ));
                }
                // The node: a segment's end when the meeting is at one.
                let ends = [a.seg.start(), a.seg.end(), b.seg.start(), b.seg.end()];
                let at = ends
                    .iter()
                    .copied()
                    .find(|e| dist(*e, pa) <= 16.0 * eps)
                    .unwrap_or_else(|| {
                        let pb = b.seg.eval(tb);
                        [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0]
                    });
                let node = nodes.len();
                nodes.push(Node { at });
                for (k, s, t) in [(i, a.seg, ta), (j, b.seg, tb)] {
                    if dist(s.start(), at) <= 16.0 * eps || dist(s.end(), at) <= 16.0 * eps {
                        continue;
                    }
                    if let Some(list) = cuts.get_mut(k) {
                        list.push((t, node));
                    }
                }
            }
        }
    }
    // Pieces.
    struct Piece {
        item: usize,
        t0: f64,
        t1: f64,
        seg: Seg,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut touched = vec![false; cleaned.len()];
    for (k, item) in items.iter().enumerate() {
        let mut list = cuts.get(k).cloned().unwrap_or_default();
        if !list.is_empty()
            && let Some(slot) = touched.get_mut(item.contour)
        {
            *slot = true;
        }
        list.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut t_prev = 0.0;
        let mut p_prev = item.seg.start();
        for (t, node) in list
            .iter()
            .copied()
            .chain(std::iter::once((1.0, usize::MAX)))
        {
            let at = if node == usize::MAX {
                item.seg.end()
            } else {
                nodes.get(node).map_or(item.seg.eval(t), |n| n.at)
            };
            if dist(at, p_prev) <= eps || t <= t_prev {
                continue;
            }
            let mut seg = item.seg.sub_range(t_prev, t);
            seg.set_start(p_prev);
            seg.set_end(at);
            pieces.push(Piece {
                item: k,
                t0: t_prev,
                t1: t,
                seg,
            });
            t_prev = t;
            p_prev = at;
        }
    }
    // Classify: a contour no other meets is decided by its first piece.
    let probe = eps * 32.0;
    let side = |seg: &Seg| -> Option<(i64, i64)> {
        let m = seg.eval(0.5);
        let mut d = seg.derivative(0.5);
        let mut len = d[0].hypot(d[1]);
        if !(len > 0.0 && len.is_finite()) {
            d = sub(seg.end(), seg.start());
            len = d[0].hypot(d[1]);
        }
        if !(len > 0.0 && len.is_finite()) {
            return None;
        }
        let n = [-d[1] / len, d[0] / len];
        let left = winding(&monos, [m[0] + n[0] * probe, m[1] + n[1] * probe]);
        let right = winding(&monos, [m[0] - n[0] * probe, m[1] - n[1] * probe]);
        Some((left, right))
    };
    let mut verdict: Vec<Option<i8>> = vec![None; cleaned.len()];
    let mut kept: Vec<(Seg, usize, f64, f64)> = Vec::new();
    for piece in &pieces {
        let Some(item) = items.get(piece.item) else {
            continue;
        };
        let whole = !touched.get(item.contour).copied().unwrap_or(true);
        let decision = match verdict.get(item.contour).copied().flatten() {
            Some(v) if whole => v,
            _ => {
                let v = match side(&piece.seg) {
                    Some((l, r)) if (l != 0) != (r != 0) => {
                        if l != 0 {
                            1
                        } else {
                            -1
                        }
                    }
                    _ => 0,
                };
                if whole && let Some(slot) = verdict.get_mut(item.contour) {
                    *slot = Some(v);
                }
                v
            }
        };
        match decision {
            1 => kept.push((piece.seg, piece.item, piece.t0, piece.t1)),
            -1 => kept.push((piece.seg.reversed(), piece.item, -piece.t1, -piece.t0)),
            _ => {}
        }
    }
    // Join end to end.
    let mut used = vec![false; kept.len()];
    let mut out = Vec::new();
    let join = eps * 32.0;
    for start in 0..kept.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut chain: Vec<(Seg, usize, f64, f64)> = Vec::new();
        let mut at = start;
        let Some(first) = kept.get(start) else {
            continue;
        };
        let origin = first.0.start();
        for _ in 0..=kept.len() {
            let Some(piece) = kept.get(at).copied() else {
                break;
            };
            if let Some(u) = used.get_mut(at) {
                *u = true;
            }
            chain.push(piece);
            let end = piece.0.end();
            if dist(end, origin) <= join {
                break;
            }
            // The next piece: one starting here; the one of the same
            // segment or the next segment first, then the smallest index.
            let candidates: Vec<usize> = (0..kept.len())
                .filter(|&k| {
                    !used.get(k).copied().unwrap_or(true)
                        && kept.get(k).is_some_and(|q| dist(q.0.start(), end) <= join)
                })
                .collect();
            let preferred = candidates
                .iter()
                .copied()
                .find(|&k| {
                    kept.get(k)
                        .is_some_and(|q| q.1 == piece.1 || next_of.get(piece.1) == Some(&q.1))
                })
                .or_else(|| candidates.first().copied());
            match preferred {
                Some(k) => at = k,
                None => {
                    return Err(FontError::Unsupported(
                        "the boundary of the region its contours fill does not close".into(),
                    ));
                }
            }
        }
        if !chain.is_empty() && chain.last().is_some_and(|l| dist(l.0.end(), origin) > join) {
            return Err(FontError::Unsupported(
                "the boundary of the region its contours fill does not close".into(),
            ));
        }
        // Pieces of one segment that follow each other are one again.
        let mut merged: Vec<(Seg, usize, f64, f64)> = Vec::new();
        for piece in chain {
            if let Some(last) = merged.last_mut()
                && last.1 == piece.1
                && (last.3 - piece.2).abs() <= 1e-12
                && let Some(item) = items.get(piece.1)
            {
                let (t0, t1) = (last.2, piece.3);
                let whole = if t0 < 0.0 {
                    item.seg.sub_range(-t1, -t0).reversed()
                } else {
                    item.seg.sub_range(t0, t1)
                };
                let mut whole = whole;
                whole.set_start(last.0.start());
                whole.set_end(piece.0.end());
                *last = (whole, piece.1, t0, t1);
                continue;
            }
            merged.push(piece);
        }
        // The first and the last may be pieces of one segment too.
        if merged.len() > 1
            && let (Some(first), Some(last)) = (merged.first().copied(), merged.last().copied())
            && first.1 == last.1
            && (last.3 - first.2).abs() <= 1e-12
            && let Some(item) = items.get(first.1)
        {
            let (t0, t1) = (last.2, first.3);
            let mut whole = if t0 < 0.0 {
                item.seg.sub_range(-t1, -t0).reversed()
            } else {
                item.seg.sub_range(t0, t1)
            };
            whole.set_start(last.0.start());
            whole.set_end(first.0.end());
            merged.pop();
            if let Some(slot) = merged.first_mut() {
                *slot = (whole, first.1, t0, t1);
            }
        }
        let mut segs: Vec<Seg> = merged.into_iter().map(|p| p.0).collect();
        // Exactly closed and joined.
        for k in 1..segs.len() {
            if let Some(prev_end) = segs.get(k - 1).map(Seg::end)
                && let Some(s) = segs.get_mut(k)
            {
                s.set_start(prev_end);
            }
        }
        if let (Some(first), Some(last)) = (segs.first().map(Seg::start), segs.last_mut()) {
            last.set_end(first);
        }
        let contour = Contour { segs, closed: true };
        if contour.signed_area().abs() > eps * eps {
            out.push(contour);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
        Contour {
            segs: vec![
                Seg::line([x0, y0], [x1, y0]),
                Seg::line([x1, y0], [x1, y1]),
                Seg::line([x1, y1], [x0, y1]),
                Seg::line([x0, y1], [x0, y0]),
            ],
            closed: true,
        }
    }

    fn area(cs: &[Contour]) -> f64 {
        cs.iter().map(Contour::signed_area).sum()
    }

    #[test]
    fn disjoint_contours_are_kept_and_oriented() {
        let out = fill_nonzero(
            &[
                rect(0.0, 0.0, 1.0, 1.0),
                rect(2.0, 0.0, 3.0, 1.0).reversed(),
            ],
            1e-9,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.signed_area() > 0.0));
    }

    #[test]
    fn a_hole_stays_a_hole() {
        let out = fill_nonzero(
            &[
                rect(0.0, 0.0, 4.0, 4.0),
                rect(1.0, 1.0, 3.0, 3.0).reversed(),
            ],
            1e-9,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert!((area(&out) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn a_contour_inside_one_of_the_same_turn_is_dropped() {
        // Nonzero: winding 2 inside the inner one, so no boundary there.
        let out =
            fill_nonzero(&[rect(0.0, 0.0, 4.0, 4.0), rect(1.0, 1.0, 3.0, 3.0)], 1e-9).unwrap();
        assert_eq!(out.len(), 1);
        assert!((area(&out) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn overlapping_contours_are_united() {
        // A cross of two bars: area 2·(3·1) − 1.
        let out =
            fill_nonzero(&[rect(0.0, 1.0, 3.0, 2.0), rect(1.0, 0.0, 2.0, 3.0)], 1e-9).unwrap();
        assert_eq!(out.len(), 1);
        assert!((area(&out) - 5.0).abs() < 1e-9, "{}", area(&out));
        // Corners: 12 of them, each a line.
        assert_eq!(out[0].segs.len(), 12);
    }

    #[test]
    fn curved_overlaps_are_united_exactly() {
        // A disc of four cubics overlapping a square: the union's area by
        // inclusion–exclusion is checked against a fine grid count.
        let k = 0.552_284_749_830_793_4;
        let c = |cx: f64, cy: f64, r: f64| Contour {
            segs: vec![
                Seg::cubic(
                    [cx + r, cy],
                    [cx + r, cy + k * r],
                    [cx + k * r, cy + r],
                    [cx, cy + r],
                ),
                Seg::cubic(
                    [cx, cy + r],
                    [cx - k * r, cy + r],
                    [cx - r, cy + k * r],
                    [cx - r, cy],
                ),
                Seg::cubic(
                    [cx - r, cy],
                    [cx - r, cy - k * r],
                    [cx - k * r, cy - r],
                    [cx, cy - r],
                ),
                Seg::cubic(
                    [cx, cy - r],
                    [cx + k * r, cy - r],
                    [cx + r, cy - k * r],
                    [cx + r, cy],
                ),
            ],
            closed: true,
        };
        let input = [c(0.0, 0.0, 1.0), rect(0.0, -0.5, 2.0, 0.5)];
        let out = fill_nonzero(&input, 1e-9).unwrap();
        assert_eq!(out.len(), 1);
        let disc = c(0.0, 0.0, 1.0).signed_area();
        // The part of the rectangle inside the disc, integrated exactly
        // against the cubic disc by the same fill on the intersection
        // would be circular; count it on a grid instead.
        let monos_in = |p: P2, cs: &[Contour]| {
            let items: Vec<Item> = cs
                .iter()
                .enumerate()
                .flat_map(|(i, c)| {
                    c.segs.iter().map(move |s| Item {
                        contour: i,
                        seg: *s,
                        bbox: s.bbox(),
                    })
                })
                .collect();
            winding(&monotone_parts(&items), p) != 0
        };
        let n = 400;
        let mut inside_both = 0;
        for i in 0..n {
            for j in 0..n {
                let p = [
                    (i as f64 + 0.5) / n as f64 * 2.0,
                    (j as f64 + 0.5) / n as f64 - 0.5,
                ];
                if monos_in(p, &input[..1]) {
                    inside_both += 1;
                }
            }
        }
        let both = inside_both as f64 * 2.0 / (n * n) as f64;
        let expected = disc + 2.0 - both;
        assert!(
            (area(&out) - expected).abs() < 2e-3,
            "{} vs {expected}",
            area(&out)
        );
    }

    #[test]
    fn a_figure_eight_becomes_two_loops() {
        let eight = Contour {
            segs: vec![
                Seg::line([0.0, 0.0], [2.0, 2.0]),
                Seg::line([2.0, 2.0], [2.0, 0.0]),
                Seg::line([2.0, 0.0], [0.0, 2.0]),
                Seg::line([0.0, 2.0], [0.0, 0.0]),
            ],
            closed: true,
        };
        let out = fill_nonzero(&[eight], 1e-9).unwrap();
        assert_eq!(out.len(), 2);
        assert!((area(&out) - 2.0).abs() < 1e-9);
        assert!(out.iter().all(|c| c.signed_area() > 0.0));
    }

    #[test]
    fn coincident_contours_are_refused_not_guessed() {
        let r = rect(0.0, 0.0, 1.0, 1.0);
        assert!(fill_nonzero(&[r.clone(), rect(0.5, 0.0, 1.5, 1.0)], 1e-9).is_err());
    }
}
