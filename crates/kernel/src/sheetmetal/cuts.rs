//! Holes across bends: a cut-out of the flat pattern that lies in no one
//! flat but inside the pattern, clear of its edges, crosses the tangent
//! lines of bends (docs/design/sheetmetal.md › *Holes across a bend*).
//!
//! **The pattern cut into faces.** The tangent lines (each a segment of a
//! flat's outline that a bend's strip continues) divide the pattern into
//! cells: the flats and the strips. Every cut is split where it crosses a
//! tangent line, and every crossed tangent line where the cut crosses it;
//! each piece of a cut then lies in one cell (found by its middle), and the
//! pieces of a tangent line inside a cut are dropped. Every piece runs with
//! the material on its left (the outlines counter-clockwise, the holes and
//! cuts clockwise; a strip uses its tangent lines against their flats'
//! direction), so the boundary of what is left of each cell is traced
//! piece to piece by the vertices they share, exactly, without a
//! tolerance: a loop of positive area is the outside of one face, one of
//! negative area a hole of the face it lies in. A cell may be left in
//! several faces (a slot across a whole strip leaves the bend in two).
//!
//! **Folded.** A piece in a flat is carried by the flat's motion, as the
//! flat's own edges are; a piece in a strip is wrapped onto the bend's
//! bottom skin: a point `u` along the tangent line and `s` across the strip
//! goes to the angle `s / (R + K T)` about the bend's axis, `u` along it
//! (the neutral fibre keeps its length, so the strip's width `θ (R + K T)`
//! is the angle θ). A line along the bend stays a line, one across it is an
//! arc of the skin's circle; any other curve's image is a cubic Hermite
//! spline through its points with its tangents (both exact), the steps
//! halved until it holds the image within a quarter of the tolerance at
//! their middles: the ends' tangents are the image's, so where a cut crosses
//! a tangent line smoothly its edges meet as the image does.

use super::{
    Bend, Builder, EdgeKey, Ends, Flat, P2, Segment, Shape, SheetFace, add, cross, dot, lift, line,
    refuse, scale, sub, unit, world,
};
use crate::KernelError;
use crate::profile::{Loop2, distance};
use craft3d_geom::{Circle, Curve3, Cylinder, Plane, Surface};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, VertexId};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// A cut across bends: its loop in the pattern's axes (clockwise) and how
/// messages name it ("the closed profile of curve 20 of sketch `Base`").
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub segments: Vec<Segment>,
    pub named: String,
}

/// A tangent line: segment `seg` of flat `flat`'s outline, which bend
/// `bend`'s strip continues, from `a` to `b` in the flat's direction.
#[derive(Clone, Copy, Debug)]
struct Tangent {
    flat: usize,
    seg: usize,
    bend: usize,
    a: P2,
    b: P2,
}

impl Tangent {
    fn length(&self) -> f64 {
        distance(self.a, self.b)
    }

    fn direction(&self) -> Option<P2> {
        unit(sub(self.b, self.a))
    }

    /// The signed distance of `p` from the line (positive on its left).
    fn side(&self, p: P2) -> f64 {
        let Some(e) = self.direction() else {
            return f64::NAN;
        };
        cross(e, sub(p, self.a))
    }

    fn along(&self, p: P2) -> f64 {
        let Some(e) = self.direction() else {
            return f64::NAN;
        };
        dot(e, sub(p, self.a))
    }
}

/// A vertex of the folded sheet, by what it is in the pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VRef {
    /// Where segment `k` of flat `f`'s outline starts.
    Outline(usize, usize),
    /// Where segment `j` of hole `h` of flat `f` starts.
    Hole(usize, usize, usize),
    /// A crossing of a cut with a tangent line inside a segment of the cut.
    Cross(usize),
    /// Where segment `j` of cut `c` starts.
    Cut(usize, usize),
}

/// An edge of the folded sheet, by what it is in the pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ERef {
    /// Segment `k` of flat `f`'s outline, whole.
    Outline(usize, usize),
    /// Piece `i` of segment `k` of flat `f`'s outline (a crossed tangent
    /// line), counted along the flat's direction.
    Sub(usize, usize, usize),
    Hole(usize, usize, usize),
    /// The arc at the start (0) or the end (1) of bend `b`.
    Arc(usize, usize),
    /// Piece `i` of segment `j` of cut `c`.
    Cut(usize, usize, usize),
}

/// What a face of the folded sheet is cut from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Cell {
    Flat(usize),
    Strip(usize),
}

/// A piece of a cell's boundary, in the cell's direction (the material on
/// its left), and whether that is its edge's own direction.
#[derive(Clone, Debug)]
struct Piece {
    seg: Segment,
    edge: ERef,
    forward: bool,
    from: VRef,
    to: VRef,
}

impl Piece {
    fn reversed(&self) -> Piece {
        Piece {
            seg: opposite(&self.seg),
            edge: self.edge,
            forward: !self.forward,
            from: self.to,
            to: self.from,
        }
    }
}

fn opposite(seg: &Segment) -> Segment {
    Segment {
        source: seg.source,
        piece: seg.piece,
        reversed: !seg.reversed,
        start: seg.end,
        end: seg.start,
        shape: seg.shape.clone(),
    }
}

/// The point at `t` in `[0, 1]` along a segment in the loop's direction
/// (from `start` to `end`).
fn loop_point(seg: &Segment, t: f64) -> P2 {
    match &seg.shape {
        Shape::Line => add(seg.start, scale(sub(seg.end, seg.start), t)),
        Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
            let angle = loop_angle(seg, t);
            add(*center, [radius * angle.pcos(), radius * angle.psin()])
        }
        Shape::Spline(_) => seg.point_at(if seg.reversed { 1.0 - t } else { t }),
    }
}

/// For an arc or a circle, the angle at `t` along the loop's direction.
fn loop_angle(seg: &Segment, t: f64) -> f64 {
    let (begin, sweep) = seg.angles().unwrap_or((0.0, TAU));
    if seg.reversed {
        begin + (1.0 - t) * sweep
    } else {
        begin + t * sweep
    }
}

/// The piece of `seg` between `t0 < t1` along the loop's direction (a
/// circle's piece is an arc); a spline only whole.
fn sub_segment(seg: &Segment, t0: f64, t1: f64) -> Segment {
    let (start, end) = (loop_point(seg, t0), loop_point(seg, t1));
    let shape = match &seg.shape {
        Shape::Circle { center, radius } => Shape::Arc {
            center: *center,
            radius: *radius,
        },
        other => other.clone(),
    };
    Segment {
        source: seg.source,
        piece: seg.piece,
        reversed: seg.reversed,
        start,
        end,
        shape,
    }
}

/// The parameters (along the loop's direction, inside the segment, more
/// than the tolerance from its ends) where `seg` crosses the infinite line
/// of `tangent`; an error message when it touches the line or runs along
/// it there, or is a spline that comes within the tolerance of it.
fn crossings_of(seg: &Segment, tangent: &Tangent, tol: f64) -> Result<Vec<f64>, String> {
    let f = |p: P2| tangent.side(p);
    match &seg.shape {
        Shape::Line => {
            let (fa, fb) = (f(seg.start), f(seg.end));
            if fa.abs() > tol && fb.abs() > tol && (fa < 0.0) != (fb < 0.0) {
                Ok(vec![fa / (fa - fb)])
            } else {
                Ok(Vec::new())
            }
        }
        Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
            let (Some(e), r) = (tangent.direction(), *radius) else {
                return Ok(Vec::new());
            };
            let n = [-e[1], e[0]];
            let dc = f(*center);
            if !(dc.abs() < r + tol) {
                return Ok(Vec::new());
            }
            let (begin, sweep) = seg.angles().unwrap_or((0.0, TAU));
            let at = |alpha: f64| {
                let w = (alpha - begin).rem_euclid(TAU);
                (w <= sweep).then(|| {
                    let u = w / sweep;
                    if seg.reversed { 1.0 - u } else { u }
                })
            };
            let beta = n[1].patan2(n[0]);
            let length = r * sweep;
            let inside = |t: f64| t * length > tol && (1.0 - t) * length > tol;
            if (dc.abs() - r).abs() <= tol {
                // Touching the line: where the circle's foot is.
                let foot = if dc > 0.0 {
                    beta + std::f64::consts::PI
                } else {
                    beta
                };
                return match at(foot) {
                    Some(t) if inside(t) => Err("touches".to_string()),
                    _ => Ok(Vec::new()),
                };
            }
            let half = (-dc / r).clamp(-1.0, 1.0).pacos();
            let mut ts: Vec<f64> = [beta + half, beta - half]
                .into_iter()
                .filter_map(at)
                .filter(|t| inside(*t))
                .collect();
            ts.sort_by(f64::total_cmp);
            Ok(ts)
        }
        Shape::Spline(_) => {
            let near = seg
                .distance_to(tangent.a, tol)
                .min(seg.distance_to(tangent.b, tol));
            let close = match seg.spline_to_line(tangent.a, tangent.b, tol) {
                Some((d, _)) => d,
                None => near,
            };
            if close > tol {
                Ok(Vec::new())
            } else {
                Err("spline".to_string())
            }
        }
    }
}

/// The analysis of the cuts against the tangent lines: every cell's
/// boundary pieces.
struct Analysis {
    tangents: Vec<Tangent>,
    /// The crossings inside a cut's segments: their points and the tangent
    /// line they are on.
    crossings: Vec<(P2, usize)>,
    /// Where each cut vertex is folded from: the flat or strip it lies in
    /// (a vertex on a tangent line: that line's flat).
    homes: BTreeMap<(usize, usize), Cell>,
    cells: BTreeMap<Cell, Vec<Piece>>,
}

/// The tangent lines of the pattern: per bend its parent's edge and the
/// flange's segment 0.
fn tangents(flats: &[Flat], bends: &[Bend]) -> Result<Vec<Tangent>, KernelError> {
    let mut out = Vec::with_capacity(2 * bends.len());
    for (i, b) in bends.iter().enumerate() {
        let seat = super::seat_of(flats, i, b)?;
        if seat.hole.is_some() {
            return Err(refuse(
                Some(i),
                "a hole across a bend in a part with a flange on a hole's edge is not supported (owed: the tangent lines of such a bend are a hole's, which the cells do not trace)",
            ));
        }
        let k = seat.seg;
        out.push(Tangent {
            flat: b.parent,
            seg: k,
            bend: i,
            a: b.start,
            b: b.end,
        });
        let (a0, a1) = b.far_line();
        out.push(Tangent {
            flat: b.child,
            seg: 0,
            bend: i,
            a: a1,
            b: a0,
        });
    }
    Ok(out)
}

fn flange_name(bend: usize) -> String {
    format!("flange {bend}")
}

/// Splits the cuts and the tangent lines they cross, and sorts every piece
/// into its cell.
fn analyse(
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    tolerance: &Tolerance,
) -> Result<Analysis, KernelError> {
    let tol = tolerance.linear;
    let tangents = tangents(flats, bends)?;
    let mut crossings: Vec<(P2, usize)> = Vec::new();
    // Per tangent line: the crossings on it (along, point, vertex).
    let mut on_line: Vec<Vec<(f64, P2, VRef)>> = vec![Vec::new(); tangents.len()];
    let mut homes: BTreeMap<(usize, usize), Cell> = BTreeMap::new();
    let mut cells: BTreeMap<Cell, Vec<Piece>> = BTreeMap::new();
    // Which cuts cross each tangent line.
    let mut crossed_by: Vec<Vec<usize>> = vec![Vec::new(); tangents.len()];
    for (c, cut) in cuts.iter().enumerate() {
        let named = &cut.named;
        let segs = &cut.segments;
        let m = segs.len();
        let at_line = |li: usize, what: &str, p: P2| {
            let flange = tangents.get(li).map_or(0, |t| t.bend);
            refuse(
                None,
                format!(
                    "{named} lies wholly inside no one flat and {what} a tangent line of the bend of {} at ({}, {}) instead of crossing it: a hole across a bend has to cross its tangent lines, clear of where the strip ends; move it",
                    flange_name(flange),
                    p[0],
                    p[1]
                ),
            )
        };
        // The crossings inside each segment, by parameter.
        let mut splits: Vec<Vec<(f64, VRef, usize)>> = vec![Vec::new(); m];
        for (j, seg) in segs.iter().enumerate() {
            for (li, tangent) in tangents.iter().enumerate() {
                let ts = crossings_of(seg, tangent, tol).map_err(|why| {
                    if why == "spline" {
                        refuse(
                            None,
                            format!(
                                "{named} lies wholly inside no one flat and crosses or touches a tangent line of the bend of {} with a spline: a spline across a tangent line is not supported (owed: its crossings found by subdivision); draw that stretch with lines or arcs, or keep the spline inside the strip or a flat",
                                flange_name(tangent.bend)
                            ),
                        )
                    } else {
                        at_line(li, "touches", seg.start)
                    }
                })?;
                for t in ts {
                    let p = loop_point(seg, t);
                    let along = tangent.along(p);
                    let len = tangent.length();
                    if along > tol && along < len - tol {
                        let id = crossings.len();
                        crossings.push((p, li));
                        on_line
                            .get_mut(li)
                            .ok_or_else(|| refuse(None, "a tangent line is missing"))?
                            .push((along, p, VRef::Cross(id)));
                        if let Some(s) = splits.get_mut(j) {
                            s.push((t, VRef::Cross(id), li));
                        }
                    } else if along > -tol && along < len + tol {
                        return Err(at_line(li, "meets the end of", p));
                    }
                }
            }
            if let Some(s) = splits.get_mut(j) {
                s.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
        }
        // The pieces of every segment.
        let mut pieces: Vec<(Segment, usize, usize, VRef, VRef)> = Vec::new();
        for (j, seg) in segs.iter().enumerate() {
            let split = splits.get(j).map_or(&[][..], Vec::as_slice);
            let mut t0 = 0.0;
            let mut from = VRef::Cut(c, j);
            for (i, &(t, v, _)) in split.iter().enumerate() {
                pieces.push((sub_segment(seg, t0, t), j, i, from, v));
                t0 = t;
                from = v;
            }
            let last = if split.is_empty() {
                seg.clone()
            } else {
                sub_segment(seg, t0, 1.0)
            };
            pieces.push((last, j, split.len(), from, VRef::Cut(c, (j + 1) % m)));
        }
        // No piece may run along a tangent line, and a vertex of the cut on
        // a tangent line must be crossed there.
        for (li, tangent) in tangents.iter().enumerate() {
            let len = tangent.length();
            let within = |p: P2| {
                let a = tangent.along(p);
                a > -tol && a < len + tol
            };
            for (seg, ..) in &pieces {
                let mid = loop_point(seg, 0.5);
                if tangent.side(mid).abs() <= tol && within(mid) {
                    return Err(at_line(li, "runs along", mid));
                }
            }
            for (j, seg) in segs.iter().enumerate() {
                let v = seg.start;
                if !(tangent.side(v).abs() <= tol && within(v)) {
                    continue;
                }
                let along = tangent.along(v);
                if !(along > tol && along < len - tol) {
                    return Err(at_line(li, "meets the end of", v));
                }
                let before = pieces
                    .iter()
                    .rev()
                    .find(|p| p.1 == (j + m - 1) % m)
                    .map(|p| tangent.side(loop_point(&p.0, 0.5)));
                let after = pieces
                    .iter()
                    .find(|p| p.1 == j)
                    .map(|p| tangent.side(loop_point(&p.0, 0.5)));
                match (before, after) {
                    (Some(x), Some(y)) if (x < 0.0) != (y < 0.0) => {
                        on_line
                            .get_mut(li)
                            .ok_or_else(|| refuse(None, "a tangent line is missing"))?
                            .push((along, v, VRef::Cut(c, j)));
                        homes.insert((c, j), Cell::Flat(tangent.flat));
                    }
                    _ => return Err(at_line(li, "touches", v)),
                }
            }
        }
        for (li, list) in on_line.iter().enumerate() {
            let this = list.iter().any(|(_, _, v)| match v {
                VRef::Cross(_) => splits.iter().flatten().any(|(_, w, _)| w == v),
                VRef::Cut(cc, _) => *cc == c,
                _ => false,
            });
            if this && let Some(by) = crossed_by.get_mut(li) {
                by.push(c);
            }
        }
        // Every piece into its cell.
        for (seg, j, i, from, to) in pieces {
            let mid = loop_point(&seg, 0.5);
            let cell = cell_of(flats, bends, mid).ok_or_else(|| {
                refuse(
                    None,
                    format!(
                        "{named}: a piece of it near ({}, {}) lies in no one flat or bend of the flat pattern; this is a bug",
                        mid[0], mid[1]
                    ),
                )
            })?;
            if let VRef::Cut(cc, jj) = from {
                homes.entry((cc, jj)).or_insert(cell);
            }
            cells.entry(cell).or_default().push(Piece {
                seg,
                edge: ERef::Cut(c, j, i),
                forward: true,
                from,
                to,
            });
        }
    }

    // The tangent lines' pieces outside every cut.
    let mut lines: Vec<Vec<Piece>> = Vec::with_capacity(tangents.len());
    for (li, tangent) in tangents.iter().enumerate() {
        let n = flats.get(tangent.flat).map_or(0, |f| f.outer.len());
        let start = VRef::Outline(tangent.flat, tangent.seg);
        let end = VRef::Outline(tangent.flat, (tangent.seg + 1) % n.max(1));
        let mut points = on_line.get(li).cloned().unwrap_or_default();
        if points.is_empty() {
            lines.push(vec![Piece {
                seg: line(tangent.a, tangent.b),
                edge: ERef::Outline(tangent.flat, tangent.seg),
                forward: true,
                from: start,
                to: end,
            }]);
            continue;
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut stops: Vec<(P2, VRef)> = vec![(tangent.a, start)];
        stops.extend(points.iter().map(|(_, p, v)| (*p, *v)));
        stops.push((tangent.b, end));
        let by: Vec<&Cut> = crossed_by
            .get(li)
            .map(|cs| cs.iter().filter_map(|&c| cuts.get(c)).collect())
            .unwrap_or_default();
        let mut kept = Vec::new();
        for (i, pair) in stops.windows(2).enumerate() {
            let [(p, vp), (q, vq)] = pair else { continue };
            if !(distance(*p, *q) > tol) {
                return Err(refuse(
                    Some(tangent.bend),
                    format!(
                        "a hole across the bend of {} crosses its tangent line within the tolerance of another crossing or of the strip's end, at ({}, {}); move it",
                        flange_name(tangent.bend),
                        q[0],
                        q[1]
                    ),
                ));
            }
            let mid = scale(add(*p, *q), 0.5);
            let inside = by.iter().any(|cut| {
                Loop2 {
                    segments: cut.segments.clone(),
                }
                .winding_about(mid)
                    != 0.0
            });
            if !inside {
                kept.push(Piece {
                    seg: line(*p, *q),
                    edge: ERef::Sub(tangent.flat, tangent.seg, i),
                    forward: true,
                    from: *vp,
                    to: *vq,
                });
            }
        }
        lines.push(kept);
    }

    // The cells' own boundaries.
    let line_of =
        |flat: usize, seg: usize| tangents.iter().position(|t| t.flat == flat && t.seg == seg);
    for (f, flat) in flats.iter().enumerate() {
        let n = flat.outer.len();
        let mut own = Vec::new();
        for (k, seg) in flat.outer.iter().enumerate() {
            match line_of(f, k) {
                Some(li) => own.extend(lines.get(li).cloned().unwrap_or_default()),
                None => own.push(Piece {
                    seg: seg.clone(),
                    edge: ERef::Outline(f, k),
                    forward: true,
                    from: VRef::Outline(f, k),
                    to: VRef::Outline(f, (k + 1) % n),
                }),
            }
        }
        for (h, hole) in flat.holes.iter().enumerate() {
            let len = hole.len();
            for (j, seg) in hole.iter().enumerate() {
                own.push(Piece {
                    seg: seg.clone(),
                    edge: ERef::Hole(f, h, j),
                    forward: true,
                    from: VRef::Hole(f, h, j),
                    to: VRef::Hole(f, h, (j + 1) % len),
                });
            }
        }
        let cell = cells.entry(Cell::Flat(f)).or_default();
        own.append(cell);
        *cell = own;
    }
    for (b, bend) in bends.iter().enumerate() {
        let (Some(lp), Some(lf)) = (
            tangents
                .iter()
                .position(|t| t.bend == b && t.flat == bend.parent),
            tangents
                .iter()
                .position(|t| t.bend == b && t.flat == bend.child),
        ) else {
            return Err(refuse(Some(b), "a bend lost its tangent lines"));
        };
        let (Some(tp), Some(_)) = (tangents.get(lp), tangents.get(lf)) else {
            return Err(refuse(Some(b), "a bend lost its tangent lines"));
        };
        let n = flats.get(bend.parent).map_or(1, |f| f.outer.len().max(1));
        let (a0, a1) = bend.far_line();
        let mut own = vec![Piece {
            seg: line(bend.start, a0),
            edge: ERef::Arc(b, 0),
            forward: true,
            from: VRef::Outline(tp.flat, tp.seg),
            to: VRef::Outline(bend.child, 1),
        }];
        own.extend(
            lines
                .get(lf)
                .map(|l| l.iter().rev().map(Piece::reversed).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        own.push(Piece {
            seg: line(a1, bend.end),
            edge: ERef::Arc(b, 1),
            forward: false,
            from: VRef::Outline(bend.child, 0),
            to: VRef::Outline(tp.flat, (tp.seg + 1) % n),
        });
        own.extend(
            lines
                .get(lp)
                .map(|l| l.iter().rev().map(Piece::reversed).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        let cell = cells.entry(Cell::Strip(b)).or_default();
        own.append(cell);
        *cell = own;
    }
    Ok(Analysis {
        tangents,
        crossings,
        homes,
        cells,
    })
}

/// The cell a point strictly inside the pattern lies in: a strip it is
/// inside, else a flat whose outline goes round it.
fn cell_of(flats: &[Flat], bends: &[Bend], p: P2) -> Option<Cell> {
    let mut found = Vec::new();
    for (b, bend) in bends.iter().enumerate() {
        let Some(d) = unit(sub(bend.end, bend.start)) else {
            continue;
        };
        let u = dot(sub(p, bend.start), d);
        let s = dot(sub(p, bend.start), bend.across);
        if u > 0.0 && u < distance(bend.start, bend.end) && s > 0.0 && s < bend.allowance {
            found.push(Cell::Strip(b));
        }
    }
    for (f, flat) in flats.iter().enumerate() {
        let around = Loop2 {
            segments: flat.outer.clone(),
        }
        .winding_about(p);
        if around != 0.0 {
            found.push(Cell::Flat(f));
        }
    }
    match found.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// A bend's bottom skin in the world: the frame of its first arc (origin
/// on the axis at the bend's start, `x` towards the parent's tangent line,
/// `z` along the axis so that the angle grows towards the flange), the
/// axis's direction along the tangent line, the skin's radius, the neutral
/// fibre's.
struct Skin {
    frame: Frame,
    axis: Vec3,
    radius: f64,
    neutral: f64,
    theta: f64,
    start: P2,
    along: P2,
    across: P2,
}

impl Skin {
    fn of(bend: &Bend, wp: &Transform) -> Result<Skin, KernelError> {
        let t = bend.thickness;
        let zc = if bend.angle > 0.0 {
            t + bend.radius
        } else {
            -bend.radius
        };
        let along = unit(sub(bend.end, bend.start)).ok_or_else(|| refuse(None, "no length"))?;
        let axis = wp.vector(lift(along));
        let c0 = wp.point(Vec3::new(bend.start[0], bend.start[1], zc));
        let s0 = wp.point(lift(bend.start));
        let frame = Frame::new(c0, axis * (-bend.angle.signum()), s0 - c0)
            .ok_or_else(|| refuse(None, "a bend's axis is not finite"))?;
        Ok(Skin {
            frame,
            axis,
            radius: zc.abs(),
            neutral: bend.neutral(),
            theta: bend.theta(),
            start: bend.start,
            along,
            across: bend.across,
        })
    }

    /// `u` along the tangent line and `s` across the strip of a pattern
    /// point.
    fn local(&self, p: P2) -> (f64, f64) {
        let d = sub(p, self.start);
        (dot(d, self.along), dot(d, self.across))
    }

    fn angle(&self, s: f64) -> f64 {
        s / self.neutral
    }

    /// The point of the bottom skin a pattern point of the strip folds to.
    fn point(&self, p: P2) -> Vec3 {
        let (u, s) = self.local(p);
        let phi = self.angle(s);
        self.frame.origin()
            + self.axis * u
            + (self.frame.x() * phi.pcos() + self.frame.y() * phi.psin()) * self.radius
    }

    /// The tangent of the image of a pattern curve through `p` with the
    /// tangent `dp` there.
    fn tangent(&self, p: P2, dp: P2) -> Vec3 {
        let (_, s) = self.local(p);
        let phi = self.angle(s);
        let du = dot(dp, self.along);
        let dphi = dot(dp, self.across) / self.neutral;
        self.axis * du
            + (self.frame.y() * phi.pcos() - self.frame.x() * phi.psin()) * (self.radius * dphi)
    }

    /// The circle of the skin `u` along the axis.
    fn circle(&self, u: f64) -> Result<Circle, KernelError> {
        let f = self
            .frame
            .at(self.frame.origin() + self.axis * u)
            .ok_or_else(|| refuse(None, "a bend's axis is not finite"))?;
        Ok(Circle::new(f, self.radius)?)
    }
}

/// The edge a piece of a cut in a strip folds to, from `va` to `vb`: a
/// line along the axis, an arc of the skin's circle, or a fitted spline;
/// and whether the loop runs against it.
fn wrapped_edge(
    builder: &mut Builder,
    skin: &Skin,
    seg: &Segment,
    va: VertexId,
    vb: VertexId,
    tol: f64,
) -> Result<(EdgeId, bool), KernelError> {
    let (u0, s0) = skin.local(seg.start);
    let (u1, s1) = skin.local(seg.end);
    let small = 1e-3 * tol;
    if matches!(seg.shape, Shape::Line) && (s1 - s0).abs() <= small {
        return Ok((builder.line(va, vb)?, false));
    }
    if matches!(seg.shape, Shape::Line) && (u1 - u0).abs() <= small {
        let circle = Curve3::Circle(skin.circle(0.5 * (u0 + u1))?);
        let (p0, p1) = (skin.angle(s0), skin.angle(s1));
        return Ok(if p0 < p1 {
            (builder.body.add_edge(circle, (p0, p1), va, vb), false)
        } else {
            (builder.body.add_edge(circle, (p1, p0), vb, va), true)
        });
    }
    let mut count = match &seg.shape {
        Shape::Arc { .. } | Shape::Circle { .. } => {
            let sweep = seg.angles().map_or(TAU, |(_, s)| s);
            ((sweep / 0.05).ceil() as usize).clamp(16, 512)
        }
        Shape::Spline(spline) => spline.control.len().saturating_mul(8).clamp(16, 512),
        Shape::Line => 16,
    };
    // A cubic Hermite spline through the wrapped points with the wrapped
    // tangents: its ends' tangents are the image's exactly, so a cut that
    // crosses a tangent line smoothly folds to edges that meet as the
    // image does (a fit's end tangent would be a little off, a kink the
    // plate's corners would have to make out).
    let limit = 0.25 * tol;
    while count <= MAX_WRAP_STEPS {
        let params: Vec<f64> = (0..=count).map(|i| i as f64 / count as f64).collect();
        let points: Vec<Vec3> = params
            .iter()
            .map(|&t| skin.point(loop_point(seg, t)))
            .collect();
        let tangents: Vec<Vec3> = params
            .iter()
            .map(|&t| skin.tangent(loop_point(seg, t), loop_tangent(seg, t)))
            .collect();
        let curve = hermite(&params, &points, &tangents)?;
        let held = params.windows(2).all(|w| {
            let [a, b] = w else { return false };
            let m = 0.5 * (a + b);
            curve.eval(m).distance(skin.point(loop_point(seg, m))) <= limit
        });
        if held {
            let range = curve.domain();
            return Ok((
                builder.body.add_edge(Curve3::Nurbs(curve), range, va, vb),
                false,
            ));
        }
        count *= 2;
    }
    Err(refuse(
        None,
        format!(
            "an edge of a hole across a bend could not be wrapped onto the bend within a quarter of the tolerance in {MAX_WRAP_STEPS} steps"
        ),
    ))
}

/// The most steps a wrapped edge is laid in.
const MAX_WRAP_STEPS: usize = 4096;

/// The tangent of a segment at `t` along the loop's direction (per unit
/// of `t`).
fn loop_tangent(seg: &Segment, t: f64) -> P2 {
    match &seg.shape {
        Shape::Line => sub(seg.end, seg.start),
        Shape::Arc { radius, .. } | Shape::Circle { radius, .. } => {
            let (_, sweep) = seg.angles().unwrap_or((0.0, TAU));
            let turn = if seg.reversed { -sweep } else { sweep };
            let angle = loop_angle(seg, t);
            [-radius * angle.psin() * turn, radius * angle.pcos() * turn]
        }
        Shape::Spline(_) => {
            // By the central difference (a spline lies inside a strip, so
            // its ends meet only its own loop's corners).
            let h = 1e-6;
            let (a, b) = ((t - h).max(0.0), (t + h).min(1.0));
            let (pa, pb) = (loop_point(seg, a), loop_point(seg, b));
            scale(sub(pb, pa), 1.0 / (b - a))
        }
    }
}

/// The cubic B-spline, once differentiable, through `points` at the
/// increasing `params` with the tangents `tangents` there: per step the
/// Bézier piece `P_i, P_i + h T_i / 3, P_(i+1) − h T_(i+1) / 3, P_(i+1)`.
fn hermite(
    params: &[f64],
    points: &[Vec3],
    tangents: &[Vec3],
) -> Result<craft3d_geom::NurbsCurve, KernelError> {
    let (Some(&first), Some(&last)) = (params.first(), params.last()) else {
        return Err(refuse(None, "a wrapped edge has no points"));
    };
    let mut knots = vec![first; 4];
    let mut control: Vec<Vec3> = Vec::with_capacity(3 * points.len());
    for (i, w) in params.windows(2).enumerate() {
        let [a, b] = w else { continue };
        let (Some(&p), Some(&q), Some(&tp), Some(&tq)) = (
            points.get(i),
            points.get(i + 1),
            tangents.get(i),
            tangents.get(i + 1),
        ) else {
            return Err(refuse(None, "a wrapped edge lost a point"));
        };
        let h = b - a;
        if i == 0 {
            control.push(p);
        }
        control.push(p + tp * (h / 3.0));
        control.push(q - tq * (h / 3.0));
        control.push(q);
        if i + 2 < params.len() {
            knots.extend([*b; 3]);
        }
    }
    knots.extend([last; 4]);
    Ok(craft3d_geom::NurbsCurve::polynomial(3, knots, control)?)
}

/// The folded bottom skin of a pattern with cuts across bends (see the
/// module documentation): the faces of the flats, then of the bends, then
/// the further faces a cell is left in; per face what it is; the ends of
/// the closed corners' extensions; per edge what it is.
pub(super) fn folded_sheet(
    plane: &Frame,
    flats: &[Flat],
    bends: &[Bend],
    cuts: &[Cut],
    tolerance: &Tolerance,
    checked: bool,
) -> Result<(Body, Vec<SheetFace>, Ends, Vec<EdgeKey>), KernelError> {
    let tol = tolerance.linear;
    let found = analyse(flats, bends, cuts, tolerance)?;
    let worlds: Vec<Transform> = flats.iter().map(|f| world(plane, f)).collect();
    let w_of = |i: usize| -> Result<Transform, KernelError> {
        worlds
            .get(i)
            .copied()
            .ok_or_else(|| refuse(None, "a flat is missing"))
    };
    let skins: Vec<Skin> = bends
        .iter()
        .map(|b| Skin::of(b, &w_of(b.parent)?))
        .collect::<Result<_, _>>()?;

    // Each cell's boundary traced into loops, and the loops into faces.
    let mut regions: Vec<(Cell, Vec<Vec<usize>>)> = Vec::new();
    for (cell, pieces) in &found.cells {
        let mut starts: BTreeMap<VRef, usize> = BTreeMap::new();
        for (i, p) in pieces.iter().enumerate() {
            if starts.insert(p.from, i).is_some() {
                return Err(refuse(
                    None,
                    "the faces of a flat pattern with a hole across a bend could not be traced: two pieces start at one point; this is a bug",
                ));
            }
        }
        let mut used = vec![false; pieces.len()];
        let mut loops: Vec<Vec<usize>> = Vec::new();
        for first in 0..pieces.len() {
            if used.get(first).copied().unwrap_or(true) {
                continue;
            }
            let mut ring = Vec::new();
            let mut at = first;
            loop {
                if used.get(at).copied().unwrap_or(true) {
                    return Err(refuse(
                        None,
                        "the faces of a flat pattern with a hole across a bend could not be traced: a boundary does not close; this is a bug",
                    ));
                }
                if let Some(u) = used.get_mut(at) {
                    *u = true;
                }
                ring.push(at);
                let Some(p) = pieces.get(at) else { break };
                let Some(&next) = starts.get(&p.to) else {
                    return Err(refuse(
                        None,
                        "the faces of a flat pattern with a hole across a bend could not be traced: a boundary stops; this is a bug",
                    ));
                };
                if next == first {
                    break;
                }
                at = next;
            }
            loops.push(ring);
        }
        let area = |ring: &[usize]| {
            Loop2 {
                segments: ring
                    .iter()
                    .filter_map(|&i| pieces.get(i).map(|p| p.seg.clone()))
                    .collect(),
            }
            .signed_area()
        };
        let (outers, holes): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
            loops.into_iter().partition(|r| area(r) > 0.0);
        if outers.is_empty() {
            return Err(refuse(
                None,
                "a hole across a bend leaves nothing of a flat or a bend's strip; make it smaller",
            ));
        }
        let mut faces: Vec<Vec<Vec<usize>>> = outers.into_iter().map(|o| vec![o]).collect();
        for hole in holes {
            let p = hole
                .first()
                .and_then(|&i| pieces.get(i))
                .map(|p| p.seg.start)
                .unwrap_or([f64::NAN, f64::NAN]);
            let count = faces.len();
            let home = faces.iter_mut().find(|f| {
                count == 1
                    || f.first().is_some_and(|o| {
                        Loop2 {
                            segments: o
                                .iter()
                                .filter_map(|&i| pieces.get(i).map(|p| p.seg.clone()))
                                .collect(),
                        }
                        .winding_about(p)
                            != 0.0
                    })
            });
            match home {
                Some(f) => f.push(hole),
                None => {
                    return Err(refuse(
                        None,
                        "a hole of a flat pattern with a hole across a bend lies in none of its faces; this is a bug",
                    ));
                }
            }
        }
        for face in faces {
            regions.push((*cell, face));
        }
    }
    // Faces in the order flats, bends, then the further faces of a cell.
    let mut order: Vec<(usize, SheetFace)> = Vec::with_capacity(regions.len());
    let mut seen: BTreeMap<Cell, usize> = BTreeMap::new();
    for (r, (cell, _)) in regions.iter().enumerate() {
        let count = seen.entry(*cell).or_insert(0);
        let role = match (*cell, *count) {
            (Cell::Flat(f), 0) => SheetFace::Flat(f),
            (Cell::Strip(b), 0) => SheetFace::Bend(b),
            (Cell::Flat(f), j) => SheetFace::FlatPiece(f, j),
            (Cell::Strip(b), j) => SheetFace::BendPiece(b, j),
        };
        *count += 1;
        order.push((r, role));
    }
    let rank = |role: &SheetFace| match role {
        SheetFace::Flat(f) => (0, 0, *f, 0),
        SheetFace::Bend(b) => (0, 1, *b, 0),
        SheetFace::FlatPiece(f, j) => (1, 0, *f, *j),
        SheetFace::BendPiece(b, j) => (1, 1, *b, *j),
    };
    order.sort_by_key(|(_, role)| rank(role));

    // The body: vertices and edges as the faces first need them.
    let mut b = Builder {
        body: Body::new(BodyKind::Sheet),
    };
    let mut vertices: BTreeMap<VRef, VertexId> = BTreeMap::new();
    let mut edges: BTreeMap<ERef, (EdgeId, bool)> = BTreeMap::new();
    let position = |v: VRef| -> Result<Vec3, KernelError> {
        let lost = || refuse(None, "a vertex of the flat pattern is missing");
        match v {
            VRef::Outline(f, k) => {
                let p = flats
                    .get(f)
                    .and_then(|fl| fl.outer.get(k))
                    .ok_or_else(lost)?
                    .start;
                Ok(w_of(f)?.point(lift(p)))
            }
            VRef::Hole(f, h, j) => {
                let p = flats
                    .get(f)
                    .and_then(|fl| fl.holes.get(h))
                    .and_then(|hole| hole.get(j))
                    .ok_or_else(lost)?
                    .start;
                Ok(w_of(f)?.point(lift(p)))
            }
            VRef::Cross(id) => {
                let (p, li) = found.crossings.get(id).copied().ok_or_else(lost)?;
                let flat = found.tangents.get(li).ok_or_else(lost)?.flat;
                Ok(w_of(flat)?.point(lift(p)))
            }
            VRef::Cut(c, j) => {
                let p = cuts
                    .get(c)
                    .and_then(|cut| cut.segments.get(j))
                    .ok_or_else(lost)?
                    .start;
                match found.homes.get(&(c, j)).copied().ok_or_else(lost)? {
                    Cell::Flat(f) => Ok(w_of(f)?.point(lift(p))),
                    Cell::Strip(s) => Ok(skins.get(s).ok_or_else(lost)?.point(p)),
                }
            }
        }
    };
    let mut faces: Vec<FaceId> = Vec::with_capacity(order.len());
    let mut roles: Vec<SheetFace> = Vec::with_capacity(order.len());
    for (r, role) in &order {
        let Some((cell, rings)) = regions.get(*r) else {
            continue;
        };
        let pieces = found.cells.get(cell).map_or(&[][..], Vec::as_slice);
        let mut loops = Vec::with_capacity(rings.len());
        for ring in rings {
            let mut list: Vec<Coedge> = Vec::with_capacity(ring.len());
            for &i in ring {
                let p = pieces
                    .get(i)
                    .ok_or_else(|| refuse(None, "a piece of a face is missing"))?;
                for v in [p.from, p.to] {
                    if let std::collections::btree_map::Entry::Vacant(e) = vertices.entry(v) {
                        e.insert(b.vertex(position(v)?));
                    }
                }
                let (Some(&va), Some(&vb)) = (vertices.get(&p.from), vertices.get(&p.to)) else {
                    return Err(refuse(None, "a vertex of the flat pattern is missing"));
                };
                let (edge, against) = match edges.get(&p.edge) {
                    Some(&e) => e,
                    None => {
                        // Made in the edge's own direction.
                        let (seg, from, to) = if p.forward {
                            (p.seg.clone(), va, vb)
                        } else {
                            (opposite(&p.seg), vb, va)
                        };
                        let made = match (p.edge, *cell) {
                            (ERef::Arc(bend, end), _) => {
                                let skin = skins
                                    .get(bend)
                                    .ok_or_else(|| refuse(Some(bend), "a bend is missing"))?;
                                let shift = if end == 0 {
                                    0.0
                                } else {
                                    bends.get(bend).map_or(0.0, |x| distance(x.start, x.end))
                                };
                                let circle = Curve3::Circle(skin.circle(shift)?);
                                (b.body.add_edge(circle, (0.0, skin.theta), from, to), false)
                            }
                            (ERef::Cut(..), Cell::Strip(bend)) => {
                                let skin = skins
                                    .get(bend)
                                    .ok_or_else(|| refuse(Some(bend), "a bend is missing"))?;
                                wrapped_edge(&mut b, skin, &seg, from, to, tol)?
                            }
                            // A flat's own edge is carried by that flat,
                            // whichever face meets it first (a further face
                            // of the flat comes after the bends).
                            (ERef::Outline(f, ..) | ERef::Sub(f, ..) | ERef::Hole(f, ..), _)
                            | (ERef::Cut(..), Cell::Flat(f)) => {
                                b.segment(&seg, &w_of(f)?, (from, to))?
                            }
                        };
                        edges.insert(p.edge, made);
                        made
                    }
                };
                list.push(Coedge {
                    edge,
                    reversed: against != !p.forward,
                });
            }
            loops.push(b.body.add_loop(list));
        }
        let surface = match cell {
            Cell::Flat(f) => {
                let frame = w_of(*f)?
                    .frame(&Frame::WORLD)
                    .ok_or_else(|| refuse(None, "a flat's plane is not finite"))?;
                (Surface::Plane(Plane::new(frame)), false)
            }
            Cell::Strip(bend) => {
                let skin = skins
                    .get(*bend)
                    .ok_or_else(|| refuse(Some(*bend), "a bend is missing"))?;
                let up = bends.get(*bend).is_some_and(|x| x.angle > 0.0);
                (
                    Surface::Cylinder(Cylinder::new(skin.frame, skin.radius)?),
                    up,
                )
            }
        };
        faces.push(b.body.add_face(surface.0, surface.1, loops));
        roles.push(*role);
    }
    b.body.add_shell(faces);

    // The extensions' ends, with their planes in the world.
    let to_world = Transform::from_frame(plane);
    let mut ends: Ends = Vec::new();
    for (f, flat) in flats.iter().enumerate() {
        for end in &flat.ends {
            let (edge, _) = edges
                .get(&ERef::Outline(f, end.segment))
                .copied()
                .ok_or_else(|| refuse(f.checked_sub(1), "an extension's end has no edge"))?;
            let frame = Frame::from_z(to_world.point(end.point), to_world.vector(end.normal))
                .ok_or_else(|| {
                    refuse(f.checked_sub(1), "an extension's end plane is not finite")
                })?;
            ends.push((edge, Surface::Plane(Plane::new(frame))));
        }
    }

    // What each edge is.
    let mut keys: Vec<Option<EdgeKey>> = vec![None; b.body.edges.len()];
    for (eref, (edge, _)) in &edges {
        let key = match *eref {
            ERef::Outline(flat, seg) => super::outline_key(flats, flat, seg),
            ERef::Sub(flat, seg, piece) => EdgeKey::Split { flat, seg, piece },
            ERef::Hole(flat, hole, seg) => EdgeKey::Hole { flat, hole, seg },
            ERef::Arc(bend, end) => EdgeKey::Arc { bend, end },
            ERef::Cut(cut, seg, piece) => EdgeKey::Cut { cut, seg, piece },
        };
        if let Some(slot) = usize::try_from(edge.0).ok().and_then(|e| keys.get_mut(e)) {
            *slot = Some(key);
        }
    }
    let keys: Vec<EdgeKey> = keys
        .into_iter()
        .collect::<Option<_>>()
        .ok_or_else(|| refuse(None, "an edge of the folded sheet is not accounted for"))?;
    if !checked {
        return Ok((b.body, roles, ends, keys));
    }
    let mut defects = craft3d_topo::check_remembered(&b.body, tolerance);
    if !defects.is_empty() {
        defects.truncate(crate::MAX_REPORTED);
        return Err(KernelError::InvalidBody {
            operation: "sheet metal",
            defects,
        });
    }
    Ok((b.body, roles, ends, keys))
}
