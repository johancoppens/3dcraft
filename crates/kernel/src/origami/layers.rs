//! The order of the layers of a sheet folded flat.
//!
//! Folded flat, the facets lie in one plane, many on top of one another,
//! and the folded state is only physical with an order of the layers in
//! which no facet passes through another: a *superposition order*, one
//! answer "above or below" for every pair of facets that overlap (Justin
//! 1997; Demaine and O'Rourke 2007, §12.3). It is computed as in the
//! literature on flat-folded states (Bern and Hayes 1996; Akitaya,
//! Cheung, Demaine, Horiyama, Hull, Ku, Tachi and Uehara 2016, "Box
//! pleating is hard"; the overlay of cells and segments as described for
//! J. S. Ku's *Flat-Folder*), written here from those descriptions:
//!
//! - **The overlay.** The folded edges of all facets, collinear pieces
//!   merged, are divided as a crease pattern is (`arrange`): its faces are
//!   *cells*, each covered by a set of facets, and its edges *segments*,
//!   each with the facets on its two sides.
//! - **The variables.** One bit per pair of facets that share a cell (they
//!   overlap): which is above.
//! - **Adjacency.** The two facets of a crease: seen from the front a
//!   valley brings their fronts together, so the right one lies on the
//!   front of the left one (above it when the left one faces up).
//! - **Taco-tortilla.** A facet that passes over a segment where a crease
//!   is folded (a *tortilla* through the *taco*'s fold) is not between the
//!   crease's two facets. A facet that only ends at the segment (on either
//!   side) is not held: in the zero-thickness model a raw edge or another
//!   fold may touch a fold's bend, and a twist's pleats rely on it.
//! - **Taco-taco.** Two creases folded at one segment on the same side
//!   nest or are apart, never interleaved: exactly one of the second
//!   crease's facets between the first's is forbidden. With "x above y" as
//!   a bit, both of these are parity conditions (`(x>a) ⊕ (x>b) = 0`, and
//!   `(x>a) ⊕ (x>b) ⊕ (y>a) ⊕ (y>b) = 0`).
//! - **Transitivity.** The facets that cover one cell are in one order
//!   there: no three in a cycle.
//!   Facets that never share a place need not be: a cyclic order of
//!   pairs that overlap in different places is physical.
//! - **Tortilla-tortilla** constrains two flat joins (creases left
//!   unfolded) at one segment; the flat-folded state has none (every crease
//!   folds by 180 degrees; a line left flat is no crease of the fold).
//!
//! The bits are found by unit propagation and a bounded backtracking
//! search, component by component of the constraint graph, the variables
//! in a fixed order (deterministic). Whether a crease pattern folds flat
//! is NP-hard (Bern and Hayes 1996), so the work is capped: past
//! [`MAX_LAYER_WORK`] the answer is *undecided*, not a guess.

use super::{
    Arrangement, BoundarySegment, Crease, CreaseKind, EdgeKind, FoldError, Folded, P2, arrange,
    dot, inside_loops, lift, sub,
};
use craft3d_math::Vec3;
use std::collections::{BTreeMap, BTreeSet};

/// The most propagation steps and constraint visits one layer order may
/// take (a few seconds at the worst).
pub const MAX_LAYER_WORK: u64 = 200_000_000;
/// The most constraints (pairs, triples and segments) one layer order may
/// have.
pub const MAX_LAYER_CONSTRAINTS: usize = 1_000_000;
/// The most folded edge pieces the overlay is made of.
const MAX_PIECES: usize = 32_768;

/// The order of the layers of a flat-folded sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Layers {
    /// Every pair of facets that overlap (share a cell), as `[above,
    /// below]`, sorted.
    pub above: Vec<[usize; 2]>,
    /// Per facet, its layer: 0 for a facet with nothing below it, one more
    /// than the highest facet below it otherwise (the longest chain below).
    pub layer: Vec<usize>,
    /// Per facet, true when its front (the sketch's front) faces down.
    pub flipped: Vec<bool>,
    /// True when the pairs of overlapping facets go round in a cycle
    /// (each pair overlapping in another place, which is physical); the
    /// layers are then those of the order with the cycle broken at its
    /// lowest facet.
    pub cyclic: bool,
    /// How many cells the overlay has, and how many of them more than one
    /// facet covers.
    pub cells: usize,
    pub stacked: usize,
    /// The steps the order took, counted as against [`MAX_LAYER_WORK`].
    pub work: u64,
}

type Lit = (usize, bool);

/// A constraint on the bits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rule {
    /// The bits add up to `odd` (mod 2).
    Parity { vars: Vec<usize>, odd: bool },
    /// The three literals are not all equal (no cycle of three).
    NotCycle([Lit; 3]),
}

/// Pairs of facets as variables: bit `true` means the lower-numbered one
/// is above.
#[derive(Default)]
struct Pairs {
    index: BTreeMap<(usize, usize), usize>,
    pairs: Vec<(usize, usize)>,
}

impl Pairs {
    fn var(&mut self, x: usize, y: usize) -> usize {
        let key = (x.min(y), x.max(y));
        if let Some(&v) = self.index.get(&key) {
            return v;
        }
        let v = self.pairs.len();
        self.index.insert(key, v);
        self.pairs.push(key);
        v
    }

    /// "x is above y" as a literal: the pair's bit, negated when `x` is
    /// the higher-numbered one.
    fn above(&mut self, x: usize, y: usize) -> Lit {
        (self.var(x, y), x > y)
    }
}

fn parity(lits: &[Lit], odd: bool) -> Rule {
    let mut vars: Vec<usize> = Vec::with_capacity(lits.len());
    let mut odd = odd;
    for &(v, neg) in lits {
        odd ^= neg;
        vars.push(v);
    }
    vars.sort_unstable();
    // A bit that appears twice cancels.
    let mut out: Vec<usize> = Vec::with_capacity(vars.len());
    for v in vars {
        if out.last() == Some(&v) {
            out.pop();
        } else {
            out.push(v);
        }
    }
    Rule::Parity { vars: out, odd }
}

fn not_cycle(p: Lit, q: Lit, r: Lit) -> Rule {
    Rule::NotCycle([p, q, r])
}

/// A point inside the region the `rings` bound (even-odd), as far from
/// their edges as a scan along the middles between their corners' heights
/// finds; `None` for a region without area.
fn interior_point(rings: &[Vec<P2>]) -> Option<P2> {
    let mut ys: Vec<f64> = rings.iter().flatten().map(|p| p[1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut best: Option<(f64, P2)> = None;
    for w in ys.windows(2) {
        let [y0, y1] = w else { continue };
        let gap = y1 - y0;
        if gap.is_nan() || gap <= 0.0 {
            continue;
        }
        let y = 0.5 * (y0 + y1);
        let mut xs = Vec::new();
        for ring in rings {
            let n = ring.len();
            for i in 0..n {
                let (Some(a), Some(b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                    continue;
                };
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks(2) {
            let [x0, x1] = pair else { continue };
            let width = x1 - x0;
            let score = width.min(gap);
            if score > 0.0 && best.is_none_or(|(s, _)| score > s) {
                best = Some((score, [0.5 * (x0 + x1), y]));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// A piece of a folded edge: where it lies, and what it is.
#[derive(Clone, Copy, Debug)]
enum What {
    /// A fold crease: hinge `.0`.
    Fold(usize),
    /// The boundary or a cut, of one facet.
    End,
}

struct Piece {
    a: P2,
    b: P2,
    what: What,
}

/// The distance from `p` to the line through `a` along the unit `d`.
fn off_line(p: P2, a: P2, d: P2) -> f64 {
    let v = sub(p, a);
    (d[0] * v[1] - d[1] * v[0]).abs()
}

/// The order of the layers of `arr` folded flat by `folded` (every facet
/// in the plane z = 0 of the sheet: `fold_flat`). `tol` is the linear
/// tolerance. Outcomes: an order; `NoLayers` naming the facets that cannot
/// be layered; `LayersUndecided` past the work cap; `Overlay` when the
/// folded edges cannot be divided into cells.
pub fn layer_order(arr: &Arrangement, folded: &Folded, tol: f64) -> Result<Layers, FoldError> {
    if !(tol.is_finite() && tol > 0.0) {
        return Err(FoldError::Tolerance);
    }
    let nf = arr.facets.len();
    if folded.transforms.len() != nf {
        return Err(FoldError::Broken {
            reason: "the fold does not match the facets",
        });
    }
    let mut work: u64 = 0;
    let place = |f: usize, p: P2| -> Option<P2> {
        let q = folded.transforms.get(f)?.point(lift(p));
        Some([q.x, q.y])
    };
    let flipped: Vec<bool> = folded
        .transforms
        .iter()
        .map(|t| t.vector(Vec3::Z).z < 0.0)
        .collect();

    // The folded facets, and their boxes.
    let mut rings: Vec<Vec<Vec<P2>>> = Vec::with_capacity(nf);
    let mut boxes: Vec<[f64; 4]> = Vec::with_capacity(nf);
    for (f, facet) in arr.facets.iter().enumerate() {
        let mut fr = Vec::with_capacity(1 + facet.holes.len());
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for cycle in std::iter::once(&facet.outer).chain(&facet.holes) {
            let mut ring = Vec::with_capacity(cycle.len());
            for p in arr.polygon(cycle) {
                let q = place(f, p).ok_or(FoldError::Broken {
                    reason: "a facet has no place",
                })?;
                if !(q[0].is_finite() && q[1].is_finite()) {
                    return Err(FoldError::Broken {
                        reason: "a folded corner is not finite",
                    });
                }
                b = [
                    b[0].min(q[0]),
                    b[1].min(q[1]),
                    b[2].max(q[0]),
                    b[3].max(q[1]),
                ];
                ring.push(q);
            }
            fr.push(ring);
        }
        rings.push(fr);
        boxes.push(b);
    }

    // The folded edge pieces: a hinge once, a boundary or cut edge once
    // per facet beside it.
    let mut hinge_of_edge: BTreeMap<usize, usize> = BTreeMap::new();
    for (k, h) in arr.hinges.iter().enumerate() {
        hinge_of_edge.insert(h.edge, k);
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for (e, edge) in arr.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (arr.point(edge.from), arr.point(edge.to)) else {
            continue;
        };
        if let Some(&k) = hinge_of_edge.get(&e) {
            let Some(h) = arr.hinges.get(k) else { continue };
            let (Some(pa), Some(pb)) = (place(h.left, a), place(h.left, b)) else {
                continue;
            };
            pieces.push(Piece {
                a: pa,
                b: pb,
                what: What::Fold(k),
            });
            continue;
        }
        for side in 0..2usize {
            let Some(&Some(f)) = arr.face_of.get(2 * e + side) else {
                continue;
            };
            let (Some(pa), Some(pb)) = (place(f, a), place(f, b)) else {
                continue;
            };
            pieces.push(Piece {
                a: pa,
                b: pb,
                what: What::End,
            });
        }
    }
    if pieces.len() > MAX_PIECES {
        return Err(FoldError::LayersUndecided {
            what: "folded edge pieces",
            count: pieces.len(),
        });
    }

    // Collinear pieces that overlap or touch are one line: grouped
    // (union-find), each group merged into the pieces of its union.
    let np = pieces.len();
    let mut parent: Vec<usize> = (0..np).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        let mut guard = 0usize;
        while let Some(&p) = parent.get(i) {
            if p == i || guard > parent.len() {
                break;
            }
            if let Some(&gp) = parent.get(p)
                && let Some(slot) = parent.get_mut(i)
            {
                *slot = gp;
            }
            i = p;
            guard += 1;
        }
        i
    }
    let unit = |p: &Piece| -> Option<P2> {
        let d = sub(p.b, p.a);
        let len = dot(d, d).sqrt();
        (len > tol && len.is_finite()).then(|| [d[0] / len, d[1] / len])
    };
    let mut dirs: Vec<P2> = Vec::with_capacity(np);
    for p in &pieces {
        dirs.push(unit(p).ok_or(FoldError::Overlay(
            "a folded edge is no longer than the tolerance".to_string(),
        ))?);
    }
    let pbox = |p: &Piece| {
        [
            p.a[0].min(p.b[0]),
            p.a[1].min(p.b[1]),
            p.a[0].max(p.b[0]),
            p.a[1].max(p.b[1]),
        ]
    };
    let mut order: Vec<usize> = (0..np).collect();
    order.sort_by(|&i, &j| {
        let (Some(a), Some(b)) = (pieces.get(i), pieces.get(j)) else {
            return i.cmp(&j);
        };
        pbox(a)[0].total_cmp(&pbox(b)[0]).then(i.cmp(&j))
    });
    for (k, &i) in order.iter().enumerate() {
        let (Some(pi), Some(&di)) = (pieces.get(i), dirs.get(i)) else {
            continue;
        };
        let bi = pbox(pi);
        for &j in order.iter().skip(k + 1) {
            let (Some(pj), Some(&dj)) = (pieces.get(j), dirs.get(j)) else {
                continue;
            };
            let bj = pbox(pj);
            if bj[0] > bi[2] + tol {
                break;
            }
            work = work.saturating_add(1);
            if work > MAX_LAYER_WORK {
                return Err(FoldError::LayersUndecided {
                    what: "folded edge pieces",
                    count: np,
                });
            }
            if bj[1] > bi[3] + tol || bi[1] > bj[3] + tol {
                continue;
            }
            let collinear = off_line(pj.a, pi.a, di) <= tol
                && off_line(pj.b, pi.a, di) <= tol
                && off_line(pi.a, pj.a, dj) <= tol
                && off_line(pi.b, pj.a, dj) <= tol;
            if !collinear {
                continue;
            }
            let t = |p: P2| dot(sub(p, pi.a), di);
            let (lo, hi) = {
                let (s, e) = (t(pj.a), t(pj.b));
                (s.min(e), s.max(e))
            };
            let len = t(pi.b);
            if hi < -tol || lo > len + tol {
                continue;
            }
            let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
            if ri != rj
                && let Some(slot) = parent.get_mut(ri.max(rj))
            {
                *slot = ri.min(rj);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..np {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    // Per group: its line (the longest piece's), the merged intervals,
    // and per piece its interval on that line.
    struct Line {
        a: P2,
        d: P2,
        /// (from, to, piece) on the line, `from < to`.
        spans: Vec<(f64, f64, usize)>,
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut creases: Vec<Crease> = Vec::new();
    let mut line_of_crease: Vec<usize> = Vec::new();
    for members in groups.values() {
        let Some(&longest) = members.iter().max_by(|&&i, &&j| {
            let len = |k: usize| pieces.get(k).map_or(0.0, |p| super::dist(p.a, p.b));
            len(i).total_cmp(&len(j)).then(j.cmp(&i))
        }) else {
            continue;
        };
        let (Some(lp), Some(&d)) = (pieces.get(longest), dirs.get(longest)) else {
            continue;
        };
        let a = lp.a;
        let mut spans: Vec<(f64, f64, usize)> = members
            .iter()
            .filter_map(|&k| {
                let p = pieces.get(k)?;
                let (s, e) = (dot(sub(p.a, a), d), dot(sub(p.b, a), d));
                Some((s.min(e), s.max(e), k))
            })
            .collect();
        spans.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.2.cmp(&y.2)));
        let index = lines.len();
        let mut current: Option<(f64, f64)> = None;
        let at = |t: f64| [a[0] + d[0] * t, a[1] + d[1] * t];
        let mut push = |from: f64, to: f64, creases: &mut Vec<Crease>| {
            creases.push(Crease {
                id: u64::try_from(creases.len()).unwrap_or(u64::MAX),
                start: at(from),
                end: at(to),
                // As cuts: a folded edge that ends inside a cell (a slit's
                // or a slot's) divides nothing there, which the
                // arrangement allows of a cut only.
                kind: CreaseKind::Cut,
            });
            line_of_crease.push(index);
        };
        for &(s, e, _) in &spans {
            current = match current {
                Some((lo, hi)) if s <= hi + tol => Some((lo, hi.max(e))),
                Some((lo, hi)) => {
                    push(lo, hi, &mut creases);
                    Some((s, e))
                }
                None => Some((s, e)),
            };
        }
        if let Some((lo, hi)) = current {
            push(lo, hi, &mut creases);
        }
        lines.push(Line { a, d, spans });
    }

    // The overlay: the merged pieces inside a box round everything.
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for fb in &boxes {
        b = [
            b[0].min(fb[0]),
            b[1].min(fb[1]),
            b[2].max(fb[2]),
            b[3].max(fb[3]),
        ];
    }
    let size = ((b[2] - b[0]).powi(2) + (b[3] - b[1]).powi(2)).sqrt();
    if !size.is_finite() {
        return Err(FoldError::Broken {
            reason: "the folded sheet has no box",
        });
    }
    let margin = 0.1 * size + 100.0 * tol;
    let corners = [
        [b[0] - margin, b[1] - margin],
        [b[2] + margin, b[1] - margin],
        [b[2] + margin, b[3] + margin],
        [b[0] - margin, b[3] + margin],
    ];
    let frame: Vec<BoundarySegment> = corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .zip(0u64..)
        .map(|((&start, &end), i)| BoundarySegment {
            id: u64::MAX - i,
            start,
            end,
            reversed: false,
        })
        .collect();
    let overlay = arrange(&frame, &creases, tol).map_err(|e| match e {
        FoldError::TooMany { .. } => FoldError::LayersUndecided {
            what: "merged folded edges",
            count: creases.len(),
        },
        other => FoldError::Overlay(other.to_string()),
    })?;

    // What covers each cell.
    let mut cover: Vec<Vec<usize>> = Vec::with_capacity(overlay.facets.len());
    for cell in &overlay.facets {
        let cell_rings: Vec<Vec<P2>> = std::iter::once(&cell.outer)
            .chain(&cell.holes)
            .map(|c| overlay.polygon(c))
            .collect();
        let Some(p) = interior_point(&cell_rings) else {
            cover.push(Vec::new());
            continue;
        };
        let mut on = Vec::new();
        for (f, fb) in boxes.iter().enumerate() {
            work = work.saturating_add(1);
            if p[0] < fb[0] || p[0] > fb[2] || p[1] < fb[1] || p[1] > fb[3] {
                continue;
            }
            if let Some(fr) = rings.get(f)
                && inside_loops(p, fr.iter().map(Vec::as_slice))
            {
                on.push(f);
            }
        }
        cover.push(on);
        if work > MAX_LAYER_WORK {
            return Err(FoldError::LayersUndecided {
                what: "cells",
                count: overlay.facets.len(),
            });
        }
    }

    let mut vars = Pairs::default();
    let mut rules: BTreeSet<Rule> = BTreeSet::new();
    let too_many = |rules: &BTreeSet<Rule>, vars: &Pairs| {
        rules.len() > MAX_LAYER_CONSTRAINTS || vars.pairs.len() > MAX_LAYER_CONSTRAINTS
    };

    // Adjacency: the two facets of every crease, by its kind.
    for h in &arr.hinges {
        let kind = arr
            .creases
            .get(h.crease)
            .map(|c| c.kind)
            .ok_or(FoldError::Broken {
                reason: "a hinge has no crease",
            })?;
        let left_flipped = flipped.get(h.left).copied().unwrap_or(false);
        let right_above = (kind == CreaseKind::Valley) != left_flipped;
        let lit = vars.above(h.right, h.left);
        rules.insert(parity(&[lit], right_above));
    }

    // Transitivity in every cell.
    let stacks: BTreeSet<&Vec<usize>> = cover.iter().filter(|c| c.len() >= 2).collect();
    let stacked = cover.iter().filter(|c| c.len() >= 2).count();
    let mut overlapping: BTreeSet<(usize, usize)> = BTreeSet::new();
    // What a stack adds, counted before it is made (k facets make
    // k(k-1)/2 pairs and k(k-1)(k-2)/6 triples): a stack that could take
    // the rules past their cap is refused before it takes the memory.
    let held = |rules: &BTreeSet<Rule>| u64::try_from(rules.len()).unwrap_or(u64::MAX);
    let cap = u64::try_from(MAX_LAYER_CONSTRAINTS).unwrap_or(u64::MAX);
    for stack in &stacks {
        let k = u64::try_from(stack.len()).unwrap_or(u64::MAX);
        let triples = k
            .saturating_mul(k.saturating_sub(1))
            .saturating_mul(k.saturating_sub(2))
            / 6;
        let pairs = k.saturating_mul(k.saturating_sub(1)) / 2;
        work = work.saturating_add(triples).saturating_add(pairs);
        if held(&rules).saturating_add(triples).saturating_add(pairs) > cap || work > MAX_LAYER_WORK
        {
            return Err(FoldError::LayersUndecided {
                what: "facets on one another in one place",
                count: stack.len(),
            });
        }
        for (i, &x) in stack.iter().enumerate() {
            for &y in stack.iter().skip(i + 1) {
                vars.var(x, y);
                overlapping.insert((x, y));
                for &z in stack.iter().skip(i + 1) {
                    if z <= y {
                        continue;
                    }
                    let (p, q, r) = (vars.above(x, y), vars.above(y, z), vars.above(z, x));
                    rules.insert(not_cycle(p, q, r));
                }
            }
        }
        if too_many(&rules, &vars) {
            return Err(FoldError::LayersUndecided {
                what: "rules",
                count: rules.len(),
            });
        }
    }

    // The segments: tacos, tortillas and the facets beside them.
    for (e, edge) in overlay.edges.iter().enumerate() {
        let EdgeKind::Crease(c) = edge.kind else {
            continue;
        };
        let (Some(line), Some(pa), Some(pb)) = (
            line_of_crease.get(c).and_then(|&l| lines.get(l)),
            overlay.point(edge.from),
            overlay.point(edge.to),
        ) else {
            continue;
        };
        let cov = |h: usize| -> &[usize] {
            overlay
                .face_of
                .get(h)
                .copied()
                .flatten()
                .and_then(|f| cover.get(f))
                .map_or(&[], Vec::as_slice)
        };
        let (left, right) = (cov(2 * e), cov(2 * e + 1));
        let mid = [0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1])];
        let t = dot(sub(mid, line.a), line.d);
        // The folds lying along this segment, with the side their facets
        // are on (false: left).
        let mut tacos: Vec<(usize, usize, bool)> = Vec::new();
        for &(from, to, k) in &line.spans {
            work = work.saturating_add(1);
            if !(from < t && t < to) {
                continue;
            }
            let Some(Piece {
                what: What::Fold(hk),
                ..
            }) = pieces.get(k)
            else {
                continue;
            };
            let Some(h) = arr.hinges.get(*hk) else {
                continue;
            };
            let in_l = |f: usize| left.contains(&f) && !right.contains(&f);
            let in_r = |f: usize| right.contains(&f) && !left.contains(&f);
            let side = if in_l(h.left) && in_l(h.right) {
                false
            } else if in_r(h.left) && in_r(h.right) {
                true
            } else {
                return Err(FoldError::Overlay(
                    "a crease folded flat does not have both its facets on one side".to_string(),
                ));
            };
            tacos.push((h.left, h.right, side));
        }
        if tacos.is_empty() {
            continue;
        }
        let mut elements: Vec<usize> = left.iter().chain(right).copied().collect();
        elements.sort_unstable();
        elements.dedup();
        let side_of = |x: usize| -> Option<bool> {
            match (left.contains(&x), right.contains(&x)) {
                (true, false) => Some(false),
                (false, true) => Some(true),
                _ => None, // through
            }
        };
        // Counted before they are made, as the cells' rules are.
        let (k, t) = (
            u64::try_from(elements.len()).unwrap_or(u64::MAX),
            u64::try_from(tacos.len()).unwrap_or(u64::MAX),
        );
        let here = t.saturating_mul(k).saturating_add(t.saturating_mul(t));
        work = work.saturating_add(here);
        if held(&rules).saturating_add(here) > cap || work > MAX_LAYER_WORK {
            return Err(FoldError::LayersUndecided {
                what: "facets that meet along one fold",
                count: elements.len(),
            });
        }
        segment_rules(&mut vars, &mut rules, &elements, &tacos, side_of);
        if too_many(&rules, &vars) || work > MAX_LAYER_WORK {
            return Err(FoldError::LayersUndecided {
                what: "rules",
                count: rules.len(),
            });
        }
    }

    let rules: Vec<Rule> = rules.into_iter().collect();
    let values = solve(&vars.pairs, &rules, &mut work)?;

    // The order of the pairs that overlap, and the layers.
    let mut above = Vec::with_capacity(overlapping.len());
    for &(x, y) in &overlapping {
        let Some(&v) = vars.index.get(&(x, y)) else {
            continue;
        };
        let up = values.get(v).copied().unwrap_or(false);
        above.push(if up { [x, y] } else { [y, x] });
    }
    above.sort_unstable();
    let (layer, cyclic) = layer_numbers(nf, &above);
    Ok(Layers {
        above,
        layer,
        flipped,
        cyclic,
        cells: overlay.facets.len(),
        stacked,
        work,
    })
}

/// The rules a segment adds: `tacos` are the folds along it, each its two
/// facets and the side they are on; `elements` every facet that meets it;
/// `side_of` the side of a facet that ends at the segment, `None` for one
/// that passes over it. Taco-tortilla for a facet passing over, taco-taco
/// for two folds on one side; nothing for a facet that only ends there,
/// on either side (the zero-thickness model of the literature lets it
/// touch a fold's bend), nor between folds on opposite sides.
fn segment_rules(
    vars: &mut Pairs,
    rules: &mut BTreeSet<Rule>,
    elements: &[usize],
    tacos: &[(usize, usize, bool)],
    side_of: impl Fn(usize) -> Option<bool>,
) {
    for (i, &(a, b, side)) in tacos.iter().enumerate() {
        for &x in elements {
            if x == a || x == b {
                continue;
            }
            if side_of(x).is_some() {
                // A facet that ends at the segment, on either side:
                // an end may lie between (inside the fold, or against
                // its bend from outside, which the zero-thickness
                // model allows), another fold on the same side is
                // held by taco-taco below, one on the other side
                // meets this one only along the line.
                continue;
            }
            // A tortilla: it passes over the fold.
            let (p, q) = (vars.above(x, a), vars.above(x, b));
            rules.insert(parity(&[p, q], false));
        }
        for &(c, d, other_side) in tacos.iter().skip(i + 1) {
            if other_side != side {
                continue; // apart but for the line
            }
            let lits = [
                vars.above(c, a),
                vars.above(c, b),
                vars.above(d, a),
                vars.above(d, b),
            ];
            rules.insert(parity(&lits, false));
        }
    }
}

/// Per facet the longest chain of facets below it, by Kahn's order (the
/// lowest-numbered ready facet first); a cycle is broken at its
/// lowest-numbered facet.
fn layer_numbers(n: usize, above: &[[usize; 2]]) -> (Vec<usize>, bool) {
    let mut below: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut over: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &[a, b] in above {
        if let Some(list) = below.get_mut(a) {
            list.push(b);
        }
        if let Some(list) = over.get_mut(b) {
            list.push(a);
        }
    }
    let mut waiting: Vec<usize> = below.iter().map(Vec::len).collect();
    let mut done = vec![false; n];
    let mut layer = vec![0usize; n];
    let mut ready: BTreeSet<usize> = (0..n).filter(|&f| waiting.get(f) == Some(&0)).collect();
    let mut cyclic = false;
    for _ in 0..n {
        let f = match ready.pop_first() {
            Some(f) => f,
            None => {
                cyclic = true;
                match (0..n).find(|&f| done.get(f) == Some(&false)) {
                    Some(f) => f,
                    None => break,
                }
            }
        };
        if let Some(d) = done.get_mut(f) {
            *d = true;
        }
        let height = below
            .get(f)
            .into_iter()
            .flatten()
            .filter(|&&g| done.get(g) == Some(&true))
            .filter_map(|&g| layer.get(g).map(|l| l + 1))
            .max()
            .unwrap_or(0);
        if let Some(slot) = layer.get_mut(f) {
            *slot = height;
        }
        for &g in over.get(f).into_iter().flatten() {
            if let Some(w) = waiting.get_mut(g) {
                *w = w.saturating_sub(1);
                if *w == 0 && done.get(g) == Some(&false) {
                    ready.insert(g);
                }
            }
        }
    }
    (layer, cyclic)
}

/// The facets a rule is about.
fn facets_of(rule: &Rule, pairs: &[(usize, usize)]) -> Vec<usize> {
    let vars: Vec<usize> = match rule {
        Rule::Parity { vars, .. } => vars.clone(),
        Rule::NotCycle(lits) => lits.iter().map(|l| l.0).collect(),
    };
    let mut out: Vec<usize> = vars
        .iter()
        .filter_map(|&v| pairs.get(v))
        .flat_map(|&(a, b)| [a, b])
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The state of the search.
struct Search<'a> {
    rules: &'a [Rule],
    watch: Vec<Vec<usize>>,
    value: Vec<Option<bool>>,
    trail: Vec<usize>,
    work: &'a mut u64,
}

enum Outcome {
    Conflict(usize),
    OverBudget,
}

impl Search<'_> {
    fn lit(&self, (v, neg): Lit) -> Option<bool> {
        self.value.get(v).copied().flatten().map(|x| x != neg)
    }

    fn set(&mut self, v: usize, x: bool) {
        if let Some(slot) = self.value.get_mut(v) {
            *slot = Some(x);
            self.trail.push(v);
        }
    }

    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            if let Some(v) = self.trail.pop()
                && let Some(slot) = self.value.get_mut(v)
            {
                *slot = None;
            }
        }
    }

    /// Checks rule `r`, and sets what it forces. `Err` on a conflict.
    fn check(&mut self, r: usize, queue: &mut Vec<usize>) -> Result<(), Outcome> {
        *self.work = self.work.saturating_add(1);
        if *self.work > MAX_LAYER_WORK {
            return Err(Outcome::OverBudget);
        }
        let Some(rule) = self.rules.get(r) else {
            return Ok(());
        };
        match rule {
            Rule::Parity { vars, odd } => {
                let mut sum = false;
                let mut open = None;
                let mut opens = 0usize;
                for &v in vars {
                    match self.value.get(v).copied().flatten() {
                        Some(x) => sum ^= x,
                        None => {
                            opens += 1;
                            open = Some(v);
                        }
                    }
                }
                match (opens, open) {
                    (0, _) if sum != *odd => Err(Outcome::Conflict(r)),
                    (1, Some(v)) => {
                        self.set(v, sum != *odd);
                        queue.push(v);
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            Rule::NotCycle(lits) => {
                let known: Vec<(usize, Option<bool>)> = lits
                    .iter()
                    .enumerate()
                    .map(|(i, &l)| (i, self.lit(l)))
                    .collect();
                let set: Vec<bool> = known.iter().filter_map(|k| k.1).collect();
                match set.as_slice() {
                    [a, b, c] if a == b && b == c => Err(Outcome::Conflict(r)),
                    [a, b] if a == b => {
                        // The open one must differ.
                        if let Some(&(i, _)) = known.iter().find(|k| k.1.is_none())
                            && let Some(&(v, neg)) = lits.get(i)
                        {
                            self.set(v, !*a != neg);
                            queue.push(v);
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
        }
    }

    /// Sets bit `v` to `x` and propagates.
    fn assign(&mut self, v: usize, x: bool) -> Result<(), Outcome> {
        self.set(v, x);
        self.propagate(vec![v])
    }

    fn propagate(&mut self, mut queue: Vec<usize>) -> Result<(), Outcome> {
        while let Some(v) = queue.pop() {
            let mut i = 0usize;
            while let Some(&r) = self.watch.get(v).and_then(|w| w.get(i)) {
                self.check(r, &mut queue)?;
                i += 1;
            }
        }
        Ok(())
    }
}

/// Finds bits for `pairs` that hold every rule.
fn solve(pairs: &[(usize, usize)], rules: &[Rule], work: &mut u64) -> Result<Vec<bool>, FoldError> {
    let nv = pairs.len();
    let mut watch: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for (r, rule) in rules.iter().enumerate() {
        let vars: Vec<usize> = match rule {
            Rule::Parity { vars, .. } => vars.clone(),
            Rule::NotCycle(lits) => lits.iter().map(|l| l.0).collect(),
        };
        for v in vars {
            if let Some(list) = watch.get_mut(v)
                && list.last() != Some(&r)
            {
                list.push(r);
            }
        }
    }
    // Components of the bits through the rules.
    let mut comp: Vec<usize> = (0..nv).collect();
    fn find(comp: &mut [usize], mut i: usize) -> usize {
        let mut guard = 0usize;
        while let Some(&p) = comp.get(i) {
            if p == i || guard > comp.len() {
                break;
            }
            if let Some(&gp) = comp.get(p)
                && let Some(slot) = comp.get_mut(i)
            {
                *slot = gp;
            }
            i = p;
            guard += 1;
        }
        i
    }
    for rule in rules {
        let vars: Vec<usize> = match rule {
            Rule::Parity { vars, .. } => vars.clone(),
            Rule::NotCycle(lits) => lits.iter().map(|l| l.0).collect(),
        };
        if let Some((&first, rest)) = vars.split_first() {
            for &v in rest {
                let (a, b) = (find(&mut comp, first), find(&mut comp, v));
                if a != b
                    && let Some(slot) = comp.get_mut(a.max(b))
                {
                    *slot = a.min(b);
                }
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for v in 0..nv {
        let c = find(&mut comp, v);
        components.entry(c).or_default().push(v);
    }

    let mut search = Search {
        rules,
        watch,
        value: vec![None; nv],
        trail: Vec::new(),
        work,
    };
    let fail = |outcome: Outcome, facets: Vec<usize>| match outcome {
        Outcome::Conflict(_) => FoldError::NoLayers { facets },
        Outcome::OverBudget => FoldError::LayersUndecided {
            what: "pairs of facets",
            count: nv,
        },
    };
    // What every rule forces before any choice: the rules of one bit, and
    // what follows from them.
    for r in 0..rules.len() {
        let mut queue = Vec::new();
        if let Err(outcome) = search
            .check(r, &mut queue)
            .and_then(|()| search.propagate(queue))
        {
            let facets = match &outcome {
                Outcome::Conflict(r) => rules
                    .get(*r)
                    .map(|x| facets_of(x, pairs))
                    .unwrap_or_default(),
                Outcome::OverBudget => Vec::new(),
            };
            return Err(fail(outcome, facets));
        }
    }
    // Then each component by a search, its bits in order.
    for vars in components.values() {
        let facets = || {
            let mut out: Vec<usize> = vars
                .iter()
                .filter_map(|&v| pairs.get(v))
                .flat_map(|&(a, b)| [a, b])
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        };
        // (bit, value tried, trail mark, place in `vars`)
        let mut decisions: Vec<(usize, bool, usize, usize)> = Vec::new();
        // Every bit before `cursor` is set: a choice is made at the first
        // open bit, and undoing a choice opens only bits from it on.
        let mut cursor = 0usize;
        loop {
            while let Some(&v) = vars.get(cursor) {
                if search.value.get(v).copied().flatten().is_none() {
                    break;
                }
                cursor += 1;
                *search.work = search.work.saturating_add(1);
            }
            let Some(&v) = vars.get(cursor) else { break };
            let mark = search.trail.len();
            decisions.push((v, false, mark, cursor));
            let mut result = search.assign(v, false);
            // On a conflict, back to the last choice not yet tried both
            // ways.
            while let Err(outcome) = result {
                if matches!(outcome, Outcome::OverBudget) {
                    return Err(fail(outcome, facets()));
                }
                let Some((v, tried, mark, at)) = decisions.pop() else {
                    return Err(fail(outcome, facets()));
                };
                search.undo(mark);
                cursor = at;
                if tried {
                    result = Err(outcome);
                    continue;
                }
                decisions.push((v, true, mark, at));
                result = search.assign(v, true);
            }
        }
    }
    Ok(search.value.iter().map(|x| x.unwrap_or(false)).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Two folds at one line from opposite sides, interlocking hooks with
    /// heights a < c < b < d, are accepted: the zero-thickness model of
    /// the literature does not forbid it (a rule that did, beyond the
    /// published ones, refused a square twist; 2026-10-28).
    #[test]
    fn interlocking_hooks_on_opposite_sides_are_accepted() {
        let (a, b, c, d) = (0usize, 1usize, 2usize, 3usize);
        let mut vars = Pairs::default();
        let mut rules = BTreeSet::new();
        let left = [a, b];
        let side_of = |x: usize| Some(!left.contains(&x));
        segment_rules(
            &mut vars,
            &mut rules,
            &[a, b, c, d],
            &[(a, b, false), (c, d, true)],
            side_of,
        );
        assert!(rules.is_empty(), "{rules:?}");
        // a < c < b < d, as bits of "x above y".
        let mut rules: Vec<Rule> = rules.into_iter().collect();
        for (x, y) in [(c, a), (b, c), (d, b), (b, a), (d, c), (d, a)] {
            let lit = vars.above(x, y);
            rules.push(parity(&[lit], true));
        }
        let mut work = 0;
        assert!(solve(&vars.pairs, &rules, &mut work).is_ok());
        // The same two folds on one side may not interleave.
        let mut vars = Pairs::default();
        let mut same = BTreeSet::new();
        segment_rules(
            &mut vars,
            &mut same,
            &[a, b, c, d],
            &[(a, b, false), (c, d, false)],
            |_| Some(false),
        );
        let mut same: Vec<Rule> = same.into_iter().collect();
        for (x, y) in [(c, a), (b, c), (d, b), (b, a), (d, c), (d, a)] {
            let lit = vars.above(x, y);
            same.push(parity(&[lit], true));
        }
        assert!(matches!(
            solve(&vars.pairs, &same, &mut work),
            Err(FoldError::NoLayers { .. })
        ));
    }

    /// Three facets each above the next by a rule of its own (each
    /// satisfiable alone, and all three together as parities): only
    /// transitivity refuses the cycle.
    #[test]
    fn only_transitivity_refuses_a_cycle_of_three() {
        let mut vars = Pairs::default();
        let (ab, bc, ca) = (vars.above(0, 1), vars.above(1, 2), vars.above(2, 0));
        let pairs = vars.pairs.clone();
        let mut rules = vec![
            parity(&[ab], true),
            parity(&[bc], true),
            parity(&[ca], true),
        ];
        let mut work = 0;
        let free = solve(&pairs, &rules, &mut work).unwrap();
        assert_eq!(free.len(), 3);
        rules.push(not_cycle(ab, bc, ca));
        match solve(&pairs, &rules, &mut work) {
            Err(FoldError::NoLayers { facets }) => {
                assert!(!facets.is_empty() && facets.iter().all(|f| *f < 3));
            }
            other => panic!("expected no layers, got {other:?}"),
        }
        // Two of the three fixed: the third is forced the other way.
        let rules = vec![
            parity(&[ab], true),
            parity(&[bc], true),
            not_cycle(ab, bc, ca),
        ];
        let found = solve(&pairs, &rules, &mut work).unwrap();
        let lit = |(v, neg): Lit| found[v] != neg;
        assert!(lit(ab) && lit(bc) && !lit(ca));
    }
}
