//! Long straight chords of flat faces, cut before anything is
//! triangulated.
//!
//! A plane face gets no points inside: its triangles are those of its
//! boundary alone. A straight edge is one chord, however long, so where a
//! row of holes runs along a long edge (a plate 4 m long with 400 holes),
//! the only points between the edge and the holes are the two ends of the
//! edge, and every triangle there is a sliver fanned from one of them
//! across the whole face. Such triangles are poor to draw and to export,
//! and a search that prunes by boxes (the self-intersection check) finds
//! every one of them near every other.
//!
//! So a straight chord of a plane face is cut where it is much longer
//! than the face is wide in front of it: where more than one point of the
//! face's boundary lies in front of a piece of it (projected inside the
//! piece, on the side of the face), nearer to it than a [`LONG_CHORD`]th
//! of its length, the piece is cut in two, and its halves are looked at
//! again. One point in front makes one triangle, not a fan, and is left
//! alone, and so are the points of the edges before and after the chord
//! in its loop, which meet it at a corner: a long block without holes, a
//! thin wedge, a half disc stay as they were. The cuts stop at the
//! chordal tolerance. Ratios only,
//! so the same face at any scale (with its tolerance) is cut at the same
//! places.
//!
//! The points are points of the edge, so every face that uses the edge
//! gets them; a face across a thin wall can ask in its turn for its other
//! edge to be cut, which the rounds of [`LONG_ROUNDS`] allow (each round
//! a face is further from what started it, and wider in front of the
//! chord, so the cuts die out).

use crate::face::{Chord, loop_points};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
use craft3d_topo::{Body, Face};

/// A piece of a straight chord is cut when two or more points of the
/// face lie in front of it nearer than its length divided by this.
pub(crate) const LONG_CHORD: f64 = 8.0;

/// How often the faces that use an edge cut by another face are looked at
/// again.
pub(crate) const LONG_ROUNDS: usize = 6;

/// No chord is cut into more pieces than this.
const MOST_PIECES: usize = 4096;

/// A point in front of a chord projecting this close to one of its ends
/// (as a part of its length) is taken to be at that end.
const AT_AN_END: f64 = 1e-9;

/// Cuts of one chord: the chord and the places of the new points as parts
/// of it, from its first point to its last as the edge runs, increasing.
pub(crate) type Cuts = (Chord, Vec<f64>);

/// The edges before and after one in its loop.
type Neighbours = (Option<u32>, Option<u32>);

/// The cuts face `index` asks for along its straight chords (none for a
/// face that is not on a plane).
pub(crate) fn long_chords(
    body: &Body,
    points: &[Vec<Vec3>],
    face: &Face,
    index: usize,
    tolerance: f64,
) -> Vec<Cuts> {
    let Surface::Plane(_) = face.surface else {
        return Vec::new();
    };
    let straight = |edge: u32| {
        body.edges
            .get(edge as usize)
            .is_some_and(|edge| matches!(edge.curve, Curve3::Line(_)))
    };
    let Ok(loops) = loop_points(body, points, face, index) else {
        return Vec::new();
    };
    if !loops.iter().flatten().any(|(_, (edge, _))| straight(*edge)) {
        return Vec::new();
    }
    let Some(normal) = face.surface.normal(0.0, 0.0) else {
        return Vec::new();
    };
    let normal = if face.reversed { -normal } else { normal };
    let Some(grid) = Grid::new(&loops, normal) else {
        return Vec::new();
    };
    let mut wanted = Vec::new();
    for (number, ring) in loops.iter().enumerate() {
        let count = ring.len();
        let edge_at = |i: usize| ring.get(i % count).map(|(_, (edge, _))| *edge);
        let mut neighbours: Option<(u32, Neighbours)> = None;
        for (k, &(p, (edge, chord))) in ring.iter().enumerate() {
            if !straight(edge) {
                continue;
            }
            let Some(&(q, _)) = ring.get((k + 1) % count) else {
                continue;
            };
            // The points of the edge itself, to tell which way the loop
            // runs along it.
            let Some(polyline) = points.get(edge as usize) else {
                continue;
            };
            let (Some(&a), Some(&b)) = (
                polyline.get(chord as usize),
                polyline.get(chord as usize + 1),
            ) else {
                continue;
            };
            let forward = a == p && b == q;
            if !forward && !(a == q && b == p) {
                continue;
            }
            let length = a.distance(b);
            if !(length.is_finite() && length > 2.0 * tolerance) {
                continue;
            }
            let Some(along) = (b - a).normalized() else {
                continue;
            };
            // The face is to the left of its loop, around its normal.
            let inward = normal.cross(along);
            let inward = if forward { inward } else { -inward };
            // The edges of the loop before and after this one share a
            // corner with it: their points make triangles with it at that
            // corner (a half disc's arc over its diameter, a wedge), not a
            // fan across the face; they are not points in front.
            // (The chords of an edge follow each other along the loop:
            // its neighbours are looked for once per edge.)
            let (before, after) = match neighbours {
                Some((of, found)) if of == edge => found,
                _ => {
                    let found = (
                        (1..count)
                            .map(|back| edge_at(k + count - back))
                            .find(|e| *e != Some(edge))
                            .flatten(),
                        (1..count)
                            .map(|on| edge_at(k + on))
                            .find(|e| *e != Some(edge))
                            .flatten(),
                    );
                    neighbours = Some((edge, found));
                    found
                }
            };
            let skip = |(at_loop, at): (usize, usize)| {
                at_loop == number
                    && edge_at(at)
                        .is_some_and(|e| e == edge || Some(e) == before || Some(e) == after)
            };
            let reach = length / LONG_CHORD;
            let mut front: Vec<(f64, f64)> = Vec::new();
            grid.near(a, b, inward, reach, |at, point| {
                if skip(at) {
                    return;
                }
                let offset = point - a;
                let s = offset.dot(along) / length;
                let h = offset.dot(inward);
                if s > AT_AN_END && s < 1.0 - AT_AN_END && h > 0.0 && h < reach {
                    front.push((s, h));
                }
            });
            if front.len() < 2 {
                continue;
            }
            front.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
            let cuts = cut(&front, length, tolerance);
            if !cuts.is_empty() {
                wanted.push(((edge, chord), cuts));
            }
        }
    }
    wanted
}

/// The places (parts of the chord) where a chord of `length` with the
/// points `front` in front of it (part along it, height; sorted by part)
/// is cut, increasing.
fn cut(front: &[(f64, f64)], length: f64, tolerance: f64) -> Vec<f64> {
    let mut cuts = Vec::new();
    // Pieces still to look at, the first piece of the chord on top.
    let mut pieces = vec![(0.0_f64, 1.0_f64)];
    while let Some((from, to)) = pieces.pop() {
        let long = (to - from) * length;
        if long < 2.0 * tolerance || cuts.len() + 1 >= MOST_PIECES {
            continue;
        }
        let first = front.partition_point(|&(s, _)| s <= from);
        let last = front.partition_point(|&(s, _)| s < to);
        let near = front
            .get(first..last)
            .unwrap_or_default()
            .iter()
            .filter(|&&(_, h)| h * LONG_CHORD < long)
            .take(2)
            .count();
        if near < 2 {
            continue;
        }
        let middle = 0.5 * (from + to);
        if !(from < middle && middle < to) {
            continue;
        }
        cuts.push(middle);
        // The second half under the first, so that the first is done
        // first; the order does not matter, the cuts are sorted.
        pieces.push((middle, to));
        pieces.push((from, middle));
    }
    cuts.sort_by(f64::total_cmp);
    cuts
}

/// The points of a face's loops in a grid over its plane, to find those
/// near a chord without looking at all of them.
struct Grid {
    /// Two directions in the plane.
    axes: [Vec3; 2],
    low: [f64; 2],
    cell: f64,
    size: [usize; 2],
    /// Per cell, `(loop, index)` of its points.
    cells: Vec<Vec<(usize, usize)>>,
    points: Vec<Vec<Vec3>>,
}

/// No grid has more cells than this many per point.
const CELLS_PER_POINT: usize = 4;

impl Grid {
    fn new(loops: &[Vec<(Vec3, Chord)>], normal: Vec3) -> Option<Grid> {
        let side = if normal.x.abs() < 0.6 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let first = normal.cross(side).normalized()?;
        let second = normal.cross(first).normalized()?;
        let axes = [first, second];
        let points: Vec<Vec<Vec3>> = loops
            .iter()
            .map(|ring| ring.iter().map(|(p, _)| *p).collect())
            .collect();
        let count: usize = points.iter().map(Vec::len).sum();
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for p in points.iter().flatten() {
            for (k, axis) in axes.iter().enumerate() {
                let x = p.dot(*axis);
                if let (Some(l), Some(h)) = (low.get_mut(k), high.get_mut(k)) {
                    *l = l.min(x);
                    *h = h.max(x);
                }
            }
        }
        let extent = [high[0] - low[0], high[1] - low[1]];
        if !(extent.iter().all(|e| e.is_finite()) && count > 0) {
            return None;
        }
        // About one point per cell, square cells.
        let largest = extent[0].max(extent[1]);
        let mut cell = (extent[0] * extent[1] / count as f64).sqrt();
        let most = count.saturating_mul(CELLS_PER_POINT).max(1);
        if !(cell.is_finite() && cell > 0.0) || largest / cell > most as f64 {
            cell = largest / most as f64;
        }
        if !(cell.is_finite() && cell > 0.0) {
            return None;
        }
        let along = |k: usize| -> usize {
            let n = (extent.get(k).copied().unwrap_or(0.0) / cell).floor();
            // Within `most` by the choice of the cell.
            (n as usize).saturating_add(1).min(most)
        };
        let size = [along(0), along(1)];
        let mut cells = vec![Vec::new(); size[0].saturating_mul(size[1])];
        let mut grid = Grid {
            axes,
            low,
            cell,
            size,
            cells: Vec::new(),
            points: Vec::new(),
        };
        for (number, ring) in points.iter().enumerate() {
            for (k, p) in ring.iter().enumerate() {
                let [i, j] = grid.place(*p);
                if let Some(cell) = cells.get_mut(j * size[0] + i) {
                    cell.push((number, k));
                }
            }
        }
        grid.cells = cells;
        grid.points = points;
        Some(grid)
    }

    fn coordinate(&self, p: Vec3, k: usize) -> f64 {
        let (Some(axis), Some(low)) = (self.axes.get(k), self.low.get(k)) else {
            return 0.0;
        };
        (p.dot(*axis) - low) / self.cell
    }

    /// The cell of `p`, held to the grid.
    fn place(&self, p: Vec3) -> [usize; 2] {
        [0, 1].map(|k| {
            let x = self.coordinate(p, k).floor();
            let most = self.size.get(k).copied().unwrap_or(1).saturating_sub(1);
            if x.is_finite() && x > 0.0 {
                (x as usize).min(most)
            } else {
                0
            }
        })
    }

    /// Every point in the cells that the rectangle over the chord `a b`,
    /// `reach` deep towards `inward`, touches: `found((loop, index),
    /// point)`, in a fixed order.
    fn near(
        &self,
        a: Vec3,
        b: Vec3,
        inward: Vec3,
        reach: f64,
        mut found: impl FnMut((usize, usize), Vec3),
    ) {
        let corners = [a, b, a + inward * reach, b + inward * reach];
        let mut low = [usize::MAX; 2];
        let mut high = [0_usize; 2];
        for corner in corners {
            let [i, j] = self.place(corner);
            low = [low[0].min(i), low[1].min(j)];
            high = [high[0].max(i), high[1].max(j)];
        }
        for j in low[1]..=high[1] {
            for i in low[0]..=high[0] {
                let Some(cell) = self.cells.get(j * self.size[0] + i) else {
                    continue;
                };
                for &(number, k) in cell {
                    if let Some(p) = self.points.get(number).and_then(|ring| ring.get(k)) {
                        found((number, k), *p);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_point_in_front_is_no_fan() {
        assert!(cut(&[(0.5, 0.01)], 100.0, 0.001).is_empty());
    }

    #[test]
    fn a_row_in_front_cuts_down_to_its_distance() {
        let front: Vec<(f64, f64)> = (1..100).map(|i| (f64::from(i) / 100.0, 1.0)).collect();
        let cuts = cut(&front, 100.0, 0.001);
        // Pieces no longer than eight times the distance: 100 / 8 needs
        // pieces of 100 / 16 = 6.25.
        assert_eq!(cuts.len(), 15, "{cuts:?}");
        assert!(cuts.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_tolerance_ends_the_cuts() {
        let front = [(0.5, 1e-12), (0.5 + 1e-12, 1e-12)];
        let cuts = cut(&front, 1.0, 0.01);
        assert!(cuts.len() <= 7, "{cuts:?}");
    }
}
