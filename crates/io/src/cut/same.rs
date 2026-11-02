//! Whether two parts are the same part: one shape carried onto the other
//! by a rigid motion of the sheet, turned over included (a plate cut from
//! sheet stock is the same part either side up; turned over, a bend up is
//! a bend down).
//!
//! The test is deterministic and exact to a tolerance. Each shape is a set
//! of pieces (every segment of the outline and the holes, and every mark),
//! each with its ends, centre and size, independent of the direction a
//! loop runs and of where it starts. The motion is found from the area's
//! centroid and the key point (a segment's end or a centre) farthest from
//! it: for each key point of the other shape at that distance from its
//! own centroid, mirrored or not, the one motion that takes the centroid
//! to the centroid and the point to the point. A motion is accepted when
//! it takes every piece onto a piece of the other shape, one to one, each
//! end, centre and size within the tolerance. Cheap measures that a
//! motion keeps (the counts of each kind of piece, the area, the cut
//! length) are compared first.

use super::{Mark, MarkKind, Motion, Shape, arc_sweep, cross, dot, norm, samples, sub};
use craft3d_drawing::{Path, Point, Segment};
use std::collections::BTreeMap;

/// A piece of a shape, as the test compares it.
#[derive(Clone, Debug)]
enum Piece {
    Line {
        a: Point,
        b: Point,
    },
    Arc {
        a: Point,
        b: Point,
        center: Point,
        /// The arc's length: the radius times the angle it sweeps.
        length: f64,
    },
    EllipticArc {
        a: Point,
        b: Point,
        center: Point,
        major: Point,
        ratio: f64,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    Ellipse {
        center: Point,
        major: Point,
        ratio: f64,
    },
    Spline {
        degree: usize,
        control: Vec<Point>,
    },
}

/// A piece with what it belongs to: 0 for the loops, else the mark's kind.
#[derive(Clone, Debug)]
struct Item {
    class: u8,
    piece: Piece,
}

fn class_of(kind: Option<MarkKind>) -> u8 {
    match kind {
        None => 0,
        Some(MarkKind::Cut) => 1,
        Some(MarkKind::BendUp) => 2,
        Some(MarkKind::BendDown) => 3,
        Some(MarkKind::Form) => 4,
    }
}

impl Piece {
    fn kind(&self) -> u8 {
        match self {
            Piece::Line { .. } => 0,
            Piece::Arc { .. } => 1,
            Piece::EllipticArc { .. } => 2,
            Piece::Circle { .. } => 3,
            Piece::Ellipse { .. } => 4,
            Piece::Spline { .. } => 5,
        }
    }

    /// A point that moves with the piece and does not depend on its
    /// direction: what the pieces are found by.
    fn anchor(&self) -> Point {
        let mid = |a: Point, b: Point| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        match self {
            Piece::Line { a, b } | Piece::Arc { a, b, .. } | Piece::EllipticArc { a, b, .. } => {
                mid(*a, *b)
            }
            Piece::Circle { center, .. } | Piece::Ellipse { center, .. } => *center,
            Piece::Spline { control, .. } => {
                let n = control.len().max(1) as f64;
                let s = control
                    .iter()
                    .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
                [s[0] / n, s[1] / n]
            }
        }
    }

    /// The points that pin the piece: ends and centres.
    fn keys(&self, out: &mut Vec<Point>) {
        match self {
            Piece::Line { a, b } => out.extend([*a, *b]),
            Piece::Arc { a, b, center, .. } | Piece::EllipticArc { a, b, center, .. } => {
                out.extend([*a, *b, *center]);
            }
            Piece::Circle { center, .. } | Piece::Ellipse { center, .. } => out.push(*center),
            Piece::Spline { control, .. } => {
                out.extend(control.first().copied());
                out.extend(control.last().copied());
            }
        }
    }

    fn moved(&self, m: &Motion) -> Piece {
        match self {
            Piece::Line { a, b } => Piece::Line {
                a: m.point(*a),
                b: m.point(*b),
            },
            Piece::Arc {
                a,
                b,
                center,
                length,
            } => Piece::Arc {
                a: m.point(*a),
                b: m.point(*b),
                center: m.point(*center),
                length: *length,
            },
            Piece::EllipticArc {
                a,
                b,
                center,
                major,
                ratio,
            } => Piece::EllipticArc {
                a: m.point(*a),
                b: m.point(*b),
                center: m.point(*center),
                major: m.vector(*major),
                ratio: *ratio,
            },
            Piece::Circle { center, radius } => Piece::Circle {
                center: m.point(*center),
                radius: *radius,
            },
            Piece::Ellipse {
                center,
                major,
                ratio,
            } => Piece::Ellipse {
                center: m.point(*center),
                major: m.vector(*major),
                ratio: *ratio,
            },
            Piece::Spline { degree, control } => Piece::Spline {
                degree: *degree,
                control: control.iter().map(|p| m.point(*p)).collect(),
            },
        }
    }

    /// Whether `self` and `other` are the same piece within `tol`.
    fn matches(&self, other: &Piece, tol: f64) -> bool {
        let near = |p: Point, q: Point| norm(sub(p, q)) <= tol;
        let ends = |a: Point, b: Point, c: Point, d: Point| {
            (near(a, c) && near(b, d)) || (near(a, d) && near(b, c))
        };
        // An axis of an ellipse has no sign.
        let axis = |u: Point, v: Point| near(u, v) || near(u, [-v[0], -v[1]]);
        match (self, other) {
            (Piece::Line { a, b }, Piece::Line { a: c, b: d }) => ends(*a, *b, *c, *d),
            (
                Piece::Arc {
                    a,
                    b,
                    center,
                    length,
                },
                Piece::Arc {
                    a: c,
                    b: d,
                    center: e,
                    length: l,
                },
            ) => ends(*a, *b, *c, *d) && near(*center, *e) && (length - l).abs() <= tol,
            (
                Piece::EllipticArc {
                    a,
                    b,
                    center,
                    major,
                    ratio,
                },
                Piece::EllipticArc {
                    a: c,
                    b: d,
                    center: e,
                    major: f,
                    ratio: r,
                },
            ) => {
                ends(*a, *b, *c, *d)
                    && near(*center, *e)
                    && axis(*major, *f)
                    && (ratio - r).abs() * norm(*major) <= tol
            }
            (
                Piece::Circle { center, radius },
                Piece::Circle {
                    center: e,
                    radius: r,
                },
            ) => near(*center, *e) && (radius - r).abs() <= tol,
            (
                Piece::Ellipse {
                    center,
                    major,
                    ratio,
                },
                Piece::Ellipse {
                    center: e,
                    major: f,
                    ratio: r,
                },
            ) => near(*center, *e) && axis(*major, *f) && (ratio - r).abs() * norm(*major) <= tol,
            (
                Piece::Spline { degree, control },
                Piece::Spline {
                    degree: g,
                    control: c,
                },
            ) => {
                degree == g
                    && control.len() == c.len()
                    && (control.iter().zip(c).all(|(p, q)| near(*p, *q))
                        || control.iter().rev().zip(c).all(|(p, q)| near(*p, *q)))
            }
            _ => false,
        }
    }
}

fn pieces_of(path: &Path, class: u8, out: &mut Vec<Item>) {
    let mut push = |piece| out.push(Item { class, piece });
    match path {
        Path::Circle { center, radius } => push(Piece::Circle {
            center: *center,
            radius: *radius,
        }),
        Path::Ellipse {
            center,
            major,
            ratio,
        } => push(Piece::Ellipse {
            center: *center,
            major: *major,
            ratio: *ratio,
        }),
        Path::Chain {
            start, segments, ..
        } => {
            let mut at = *start;
            for segment in segments {
                let end = segment.end();
                push(match segment {
                    Segment::Line { to } => Piece::Line { a: at, b: *to },
                    Segment::Arc {
                        to,
                        center,
                        radius,
                        ccw,
                    } => Piece::Arc {
                        a: at,
                        b: *to,
                        center: *center,
                        length: radius * arc_sweep(at, *to, *center, *ccw).1.abs(),
                    },
                    Segment::EllipticArc {
                        to,
                        center,
                        major,
                        ratio,
                        ..
                    } => Piece::EllipticArc {
                        a: at,
                        b: *to,
                        center: *center,
                        major: *major,
                        ratio: *ratio,
                    },
                    Segment::Spline(spline) => Piece::Spline {
                        degree: spline.degree(),
                        control: spline.control_points().to_vec(),
                    },
                });
                at = end;
            }
        }
    }
}

fn items(shape: &Shape) -> Vec<Item> {
    let mut out = Vec::new();
    pieces_of(&shape.outline, 0, &mut out);
    for hole in &shape.holes {
        pieces_of(hole, 0, &mut out);
    }
    for Mark { kind, path } in &shape.marks {
        pieces_of(path, class_of(Some(*kind)), &mut out);
    }
    out
}

/// The centroid of the material, from the loops' points (the same points
/// for congruent shapes, so the same centroid moved).
fn centroid(shape: &Shape) -> Option<Point> {
    let mut total = 0.0;
    let mut sum = [0.0, 0.0];
    for (sign, path) in
        std::iter::once((1.0, &shape.outline)).chain(shape.holes.iter().map(|h| (-1.0, h)))
    {
        let mut points = Vec::new();
        samples(path, 64, &mut points);
        let Some(&origin) = points.first() else {
            continue;
        };
        let (mut a, mut m) = (0.0, [0.0, 0.0]);
        for (i, p) in points.iter().enumerate() {
            let q = points
                .get((i + 1) % points.len())
                .copied()
                .unwrap_or(origin);
            let (p, q) = (sub(*p, origin), sub(q, origin));
            let c = cross(p, q);
            a += c;
            m = [m[0] + (p[0] + q[0]) * c, m[1] + (p[1] + q[1]) * c];
        }
        // The loop's area is a / 2 and its centroid origin + m / (3 a),
        // whichever way it runs.
        if a == 0.0 {
            continue;
        }
        let at = [origin[0] + m[0] / (3.0 * a), origin[1] + m[1] / (3.0 * a)];
        let weight = sign * a.abs();
        total += weight;
        sum = [sum[0] + weight * at[0], sum[1] + weight * at[1]];
    }
    (total.is_finite() && total > 0.0).then(|| [sum[0] / total, sum[1] / total])
}

/// The pieces of a shape filed by their anchors in cells of `cell`.
struct Index {
    cell: f64,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
}

impl Index {
    fn key(&self, p: Point) -> (i64, i64) {
        // `as` saturates: a point too far off for the grid lands in its
        // last cell and is compared like any other.
        (
            (p[0] / self.cell).floor() as i64,
            (p[1] / self.cell).floor() as i64,
        )
    }

    fn new(items: &[Item], cell: f64) -> Index {
        let mut index = Index {
            cell,
            cells: BTreeMap::new(),
        };
        for (i, item) in items.iter().enumerate() {
            let key = index.key(item.piece.anchor());
            index.cells.entry(key).or_default().push(i);
        }
        index
    }

    /// The pieces whose anchors lie in the cells around `p`.
    fn near(&self, p: Point) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = self.key(p);
        (-1..=1_i64)
            .flat_map(move |dx| {
                (-1..=1_i64).map(move |dy| (x.saturating_add(dx), y.saturating_add(dy)))
            })
            .filter_map(|k| self.cells.get(&k))
            .flatten()
            .copied()
    }
}

/// Whether `motion` takes every piece of `a` onto a piece of `b`, one to
/// one.
fn carries(a: &[Item], b: &[Item], index: &Index, motion: &Motion, tol: f64) -> bool {
    let mut used = vec![false; b.len()];
    for item in a {
        let moved = item.piece.moved(motion);
        let class = if motion.mirror {
            match item.class {
                2 => 3,
                3 => 2,
                other => other,
            }
        } else {
            item.class
        };
        let found = index.near(moved.anchor()).find(|&j| {
            !used.get(j).copied().unwrap_or(true)
                && b.get(j)
                    .is_some_and(|other| other.class == class && moved.matches(&other.piece, tol))
        });
        match found.and_then(|j| used.get_mut(j)) {
            Some(slot) => *slot = true,
            None => return false,
        }
    }
    true
}

/// Counts of the pieces by class and kind: a motion keeps them (bends up
/// and down counted together, since turning over swaps them).
fn census(items: &[Item]) -> BTreeMap<(u8, u8), usize> {
    let mut counts = BTreeMap::new();
    for item in items {
        let class = if item.class == 3 { 2 } else { item.class };
        *counts.entry((class, item.piece.kind())).or_insert(0) += 1;
    }
    counts
}

/// Whether `a` and `b` are the same part: carried onto each other by a
/// rigid motion of the plane, turned over included unless a shape has a
/// forming mark (a louvre's hood stands on one side), to within `tol`.
/// `work` is how many piece comparisons may still be made; `None` when it
/// ran out before the answer was known (the parts are then counted apart,
/// and the caller says so).
pub fn same_part(a: &Shape, b: &Shape, tol: f64, work: &mut usize) -> Option<bool> {
    let (pa, pb) = (items(a), items(b));
    if pa.len() != pb.len() || census(&pa) != census(&pb) || a.holes.len() != b.holes.len() {
        return Some(false);
    }
    let (area_a, area_b) = (a.area(), b.area());
    let (cut_a, cut_b) = (a.cut_length(), b.cut_length());
    if (area_a - area_b).abs() > tol * cut_a.max(cut_b) + 1e-12 * area_a.abs().max(area_b.abs())
        || (cut_a - cut_b).abs() > tol * (pa.len() as f64) + 1e-9 * cut_a.max(cut_b)
    {
        return Some(false);
    }
    let (Some(ca), Some(cb)) = (centroid(a), centroid(b)) else {
        return Some(false);
    };
    let mut keys_a = Vec::new();
    pa.iter().for_each(|i| i.piece.keys(&mut keys_a));
    let mut keys_b = Vec::new();
    pb.iter().for_each(|i| i.piece.keys(&mut keys_b));
    let reference = keys_a.iter().copied().map(|p| (norm(sub(p, ca)), p)).fold(
        None,
        |best: Option<(f64, Point)>, (d, p)| match best {
            Some((b, _)) if b >= d => best,
            _ => Some((d, p)),
        },
    );
    let index = Index::new(&pb, 4.0 * tol);
    let turn_over = ![a, b]
        .iter()
        .any(|s| s.marks.iter().any(|m| m.kind == MarkKind::Form));
    let mirrors: &[bool] = if turn_over { &[false, true] } else { &[false] };
    let mut try_motion = |motion: Motion| -> Option<bool> {
        *work = work.checked_sub(pa.len().max(1))?;
        Some(carries(&pa, &pb, &index, &motion, tol))
    };
    match reference.filter(|(r, _)| *r > tol) {
        // Every key point at the centroid (a disc, rings about one
        // centre): any turn will do.
        None => {
            for &mirror in mirrors {
                let m = Motion {
                    mirror,
                    ..Motion::IDENTITY
                };
                let shift = sub(cb, m.vector(ca));
                if try_motion(m.then_shift(shift))? {
                    return Some(true);
                }
            }
            Some(false)
        }
        Some((r, p)) => {
            for &mirror in mirrors {
                let flip = Motion {
                    mirror,
                    ..Motion::IDENTITY
                };
                let from = flip.vector(sub(p, ca));
                for q in &keys_b {
                    let to = sub(*q, cb);
                    let rq = norm(to);
                    if (rq - r).abs() > tol || rq == 0.0 {
                        continue;
                    }
                    let (c, s) = (dot(from, to) / (r * rq), cross(from, to) / (r * rq));
                    let turn = Motion {
                        cos: c,
                        sin: s,
                        mirror,
                        shift: [0.0, 0.0],
                    };
                    let shift = sub(cb, turn.vector(ca));
                    if try_motion(turn.then_shift(shift))? {
                        return Some(true);
                    }
                }
            }
            Some(false)
        }
    }
}
