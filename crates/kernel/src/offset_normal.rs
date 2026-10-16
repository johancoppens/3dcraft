//! The normals the offsets of tangent faces share.
//!
//! Two faces that meet tangentially (a blend and the faces it rolls
//! along, the halves of a ring of blends) are tangent only as well as
//! they were fitted: their normals along the edge agree to about 1e-6,
//! and offsets by a distance `d` built each along its own normal part by
//! `d` times that, which at a document's tolerance of a micrometre on a
//! part of a few hundred millimetres is more than the tolerance. The
//! exact answer for a rolling-ball blend is known, though: the offset of
//! the blend is tangent to the offsets of its neighbours, so along the
//! edge it has their normal.
//!
//! **The turn.** Where a freeform face that moves lies along an edge with
//! one of the borders of its parameter range (an iso-curve: the borders
//! of a rolling-ball blend are), and the face across that edge moves by
//! the same distance and is tangent to it all along ([`TANGENT`]), the
//! normal the face is offset along is turned, along that border, to the
//! normal both faces share there: the neighbour's, when the neighbour is
//! not turned along that edge itself (it does not lie along it with a
//! border, or is not freeform), else the mean of the two. Across the face
//! the turns of its four borders are blended as a Coons patch is (each
//! linearly to the opposite border, less the bilinear blend of the
//! corners); at a corner where a border whose neighbour is not turned
//! meets one whose neighbour is, the former's turn holds (the corner is a
//! vertex of the unturned neighbour). The turn is of the size of the
//! mismatch, so the offset moves by `d` times that: the faces then meet
//! within the tolerance along the edge.
//!
//! **Bounded.** The turn moves the face's offset away from its exact
//! offset by `|d|` times the sine it turns by. That is held to
//! [`TURN_SHARE`] of the tolerance (at the border samples and on a grid
//! across the face, measured there), so that with the fit's own share the
//! face stays within half the tolerance of its exact offset; a turn that
//! would take more is refused as too far ([`TooFar`]): faces whose normals
//! part by more than that at this distance do not meet as tangent faces
//! within the tolerance, and calling them tangent would be a deviation
//! the document's tolerance does not allow.
//!
//! **Where.** The turn of a border is measured where the edge covers it;
//! past the ends of that part it falls linearly to nothing at the
//! border's ends, so that a part of the border along no edge (a blend's
//! surface continued past its face) or near other edges is moved by less
//! than the bound, not by the turn at the edge's end.

use craft3d_geom::{NurbsCurve, Surface, refine::interpolate};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId};
use std::collections::BTreeMap;

use crate::tweak::TANGENT;

/// Samples along a border for the turn, and to hold the faces tangent.
const ALONG: usize = 32;

/// The share of the tolerance a turn may move a face's offset by (the fit
/// takes another quarter: the face stays within half the tolerance of its
/// exact offset, two faces that meet within the tolerance of each other).
pub(crate) const TURN_SHARE: f64 = 0.25;

/// Points per direction of the grid across a face on which the blended
/// turn is measured against the bound.
const ACROSS: usize = 16;

/// A turn that would move these faces' offsets by more than
/// [`TURN_SHARE`] of the tolerance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TooFar {
    pub faces: Vec<FaceId>,
}

/// A border of a parameter range: `u` (true) or `v` fixed, at its start
/// (false) or end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Border {
    along_u_fixed: bool,
    end: bool,
}

const BORDERS: [Border; 4] = [
    Border {
        along_u_fixed: true,
        end: false,
    },
    Border {
        along_u_fixed: true,
        end: true,
    },
    Border {
        along_u_fixed: false,
        end: false,
    },
    Border {
        along_u_fixed: false,
        end: true,
    },
];

type Domain = ((f64, f64), (f64, f64));

impl Border {
    /// The parameters of the point of the border at `q` (the free one).
    fn at(self, ((u0, u1), (v0, v1)): Domain, q: f64) -> (f64, f64) {
        match (self.along_u_fixed, self.end) {
            (true, false) => (u0, q),
            (true, true) => (u1, q),
            (false, false) => (q, v0),
            (false, true) => (q, v1),
        }
    }

    /// The range of the free parameter.
    fn free(self, ((u0, u1), (v0, v1)): Domain) -> (f64, f64) {
        if self.along_u_fixed {
            (v0, v1)
        } else {
            (u0, u1)
        }
    }
}

/// The turn of one border: a curve of the turn over the free parameter
/// (continued past its ends as its end spans are), and whether the
/// neighbour holds its own normal there (it is not turned along this
/// edge).
#[derive(Clone, Debug)]
struct Side {
    turn: NurbsCurve,
    /// The part of the border the edge covers.
    range: (f64, f64),
    /// The whole border.
    border: (f64, f64),
    fixed: bool,
}

impl Side {
    /// The turn at `q`: measured where the edge covers the border, falling
    /// to nothing at the border's ends past that ([`fall`]), and going on
    /// as it is past an end of the border the edge reaches (a face that
    /// grows past that border has the edge go on along it).
    fn at(&self, q: f64) -> Vec3 {
        let ((lo, hi), (f0, f1)) = (self.range, self.border);
        let share = if q < lo {
            if lo > f0 {
                fall((q - f0) / (lo - f0))
            } else {
                1.0
            }
        } else if q > hi {
            if f1 > hi {
                fall((f1 - q) / (f1 - hi))
            } else {
                1.0
            }
        } else {
            1.0
        };
        self.turn.eval(q) * share
    }
}

/// The share of a border's turn at `x` of the way from where it falls to
/// nothing (0) to where its edge begins (1): the quintic smoothstep,
/// whose first and second derivatives vanish at both ends, so the turn
/// and the offset made along it are twice continuously differentiable
/// (what the fit of the offset asks between its breaks, `geom::offset`).
fn fall(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (10.0 + x * (-15.0 + 6.0 * x))
}

/// How the normal of one face is turned (see the module documentation):
/// add [`Turn::at`] to the unit normal `∂u × ∂v` and make it unit again.
#[derive(Clone, Debug)]
pub(crate) struct Turn {
    domain: Domain,
    sides: [Option<Side>; 4],
}

impl Turn {
    /// Whether the turn moves an offset by `distance` by at most `bound`
    /// over the box of parameters `over` (the domain and what the face
    /// grows into past it), measured on a grid across it.
    pub(crate) fn within(&self, over: Domain, distance: f64, bound: f64) -> bool {
        let ((u0, u1), (v0, v1)) = over;
        (0..=ACROSS).all(|i| {
            (0..=ACROSS).all(|j| {
                let (a, b) = (i as f64 / ACROSS as f64, j as f64 / ACROSS as f64);
                let moved = self.at(u0 + (u1 - u0) * a, v0 + (v1 - v0) * b).length();
                distance * moved <= bound
            })
        })
    }

    /// The turn at `(u, v)`, past the domain too: the blend goes on as the
    /// bilinear function it is, so the turn is as smooth there as inside.
    pub(crate) fn at(&self, u: f64, v: f64) -> Vec3 {
        let ((u0, u1), (v0, v1)) = self.domain;
        let s = (u - u0) / (u1 - u0);
        let t = (v - v0) / (v1 - v0);
        let value = |k: usize, q: f64| {
            self.sides
                .get(k)
                .and_then(Option::as_ref)
                .map_or(Vec3::ZERO, |side| side.at(q))
        };
        // Borders 0, 1: u fixed (functions of v); 2, 3: v fixed.
        let mut sum = value(0, v) * (1.0 - s) + value(1, v) * s;
        sum += value(2, u) * (1.0 - t) + value(3, u) * t;
        for (ku, kv, wu, wv, cu, cv) in [
            (0, 2, 1.0 - s, 1.0 - t, u0, v0),
            (1, 2, s, 1.0 - t, u1, v0),
            (0, 3, 1.0 - s, t, u0, v1),
            (1, 3, s, t, u1, v1),
        ] {
            let side = |k: usize| self.sides.get(k).and_then(Option::as_ref);
            let (a, b) = (value(ku, cv), value(kv, cu));
            let fixed = |k: usize| side(k).is_some_and(|s| s.fixed);
            // The corner value is a + b − k: the turn of the border whose
            // neighbour holds its normal, or the mean.
            let k = match (fixed(ku), fixed(kv)) {
                (true, false) => b,
                (false, true) => a,
                _ => (a + b) * 0.5,
            };
            sum = sum - k * (wu * wv);
        }
        if sum.is_finite() { sum } else { Vec3::ZERO }
    }
}

fn freeform(surface: &Surface) -> bool {
    matches!(surface, Surface::Bezier(_) | Surface::Nurbs(_))
}

/// The outward unit normal of `face` of `body` nearest `point`, and the
/// distance to its surface there.
fn outward(body: &Body, face: FaceId, point: Vec3) -> Option<(Vec3, f64)> {
    let face = body.face(face)?;
    let (u, v) = face.surface.project(point).ok()?;
    let normal = face.surface.normal(u, v)?;
    let off = face.surface.eval(u, v).distance(point);
    Some((if face.reversed { -normal } else { normal }, off))
}

/// The edges of `face` that lie along a border of its surface's
/// parameter range, per border.
fn along_borders(body: &Body, face: FaceId, tol: f64) -> BTreeMap<Border, Vec<EdgeId>> {
    let mut found: BTreeMap<Border, Vec<EdgeId>> = BTreeMap::new();
    let Some(data) = body.face(face) else {
        return found;
    };
    let domain = data.surface.domain();
    for coedge in data
        .loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| &l.coedges)
    {
        let Some(edge) = body.edge(coedge.edge) else {
            continue;
        };
        let points: Option<Vec<(Vec3, (f64, f64))>> = (1..=5)
            .map(|k| {
                let f = f64::from(k) / 6.0;
                let p = edge
                    .curve
                    .eval(edge.range.0 + (edge.range.1 - edge.range.0) * f);
                data.surface.project(p).ok().map(|uv| (p, uv))
            })
            .collect();
        let Some(points) = points else {
            continue;
        };
        for border in BORDERS {
            let on = points.iter().all(|(p, (u, v))| {
                let q = if border.along_u_fixed { *v } else { *u };
                let (bu, bv) = border.at(domain, q);
                data.surface.eval(bu, bv).distance(*p) <= tol
            });
            if on {
                let list = found.entry(border).or_default();
                if !list.contains(&coedge.edge) {
                    list.push(coedge.edge);
                }
            }
        }
    }
    found
}

/// The edges that lie along a border of the parameter range of a freeform
/// face of `moving`, with that face and border (the first such face, in
/// order, for an edge along borders of both its faces).
pub(crate) fn iso_edges(
    body: &Body,
    moving: &BTreeMap<FaceId, f64>,
    tolerance: &Tolerance,
) -> BTreeMap<EdgeId, crate::tweak::Iso> {
    let mut out = BTreeMap::new();
    for face in moving.keys() {
        let Some(data) = body.face(*face) else {
            continue;
        };
        if !freeform(&data.surface) {
            continue;
        }
        let ((u0, u1), (v0, v1)) = data.surface.domain();
        for (border, edges) in along_borders(body, *face, tolerance.linear) {
            let value = match (border.along_u_fixed, border.end) {
                (true, false) => u0,
                (true, true) => u1,
                (false, false) => v0,
                (false, true) => v1,
            };
            for edge in edges {
                out.entry(edge).or_insert(crate::tweak::Iso {
                    face: *face,
                    u_fixed: border.along_u_fixed,
                    value,
                });
            }
        }
    }
    out
}

/// The turns of the faces of `moving` (face and distance) that need one,
/// in the sense of `∂u × ∂v` of each face's surface.
pub(crate) fn turns(
    body: &Body,
    moving: &BTreeMap<FaceId, f64>,
    tolerance: &Tolerance,
) -> Result<BTreeMap<FaceId, Turn>, TooFar> {
    let bound = TURN_SHARE * tolerance.linear;
    let tol = tolerance.linear;
    let uses = crate::tweak::edge_faces(body);
    // Per freeform face that moves, the edges along its borders.
    let borders: BTreeMap<FaceId, BTreeMap<Border, Vec<EdgeId>>> = moving
        .keys()
        .filter(|f| body.face(**f).is_some_and(|d| freeform(&d.surface)))
        .map(|f| (*f, along_borders(body, *f, tol)))
        .collect();
    let lies_along = |face: FaceId, edge: EdgeId| {
        borders
            .get(&face)
            .is_some_and(|b| b.values().any(|edges| edges.contains(&edge)))
    };
    let mut out = BTreeMap::new();
    for (face, per_border) in &borders {
        let Some(data) = body.face(*face) else {
            continue;
        };
        let Some(distance) = moving.get(face) else {
            continue;
        };
        let domain = data.surface.domain();
        let sign = if data.reversed { -1.0 } else { 1.0 };
        let mut sides: [Option<Side>; 4] = [None, None, None, None];
        for (index, border) in BORDERS.iter().enumerate() {
            let Some([edge]) = per_border.get(border).map(Vec::as_slice) else {
                continue;
            };
            let Some(edge_data) = body.edge(*edge) else {
                continue;
            };
            let Some(neighbour) = uses
                .get(edge.0 as usize)
                .and_then(|faces| faces.iter().find(|f| *f != face).copied())
            else {
                continue;
            };
            if moving.get(&neighbour) != Some(distance) {
                continue;
            }
            let mutual = lies_along(neighbour, *edge);
            // The part of the border the edge covers.
            let free = |p: Vec3| {
                let (u, v) = data.surface.project(p).ok()?;
                Some(if border.along_u_fixed { v } else { u })
            };
            let (Some(a), Some(b)) = (
                free(edge_data.curve.eval(edge_data.range.0)),
                free(edge_data.curve.eval(edge_data.range.1)),
            ) else {
                continue;
            };
            let (lo, hi) = (a.min(b), b.max(a));
            let (f0, f1) = border.free(domain);
            let (lo, hi) = (lo.max(f0), hi.min(f1));
            if hi.is_nan() || lo.is_nan() || hi <= lo {
                continue;
            }
            let mut params = Vec::with_capacity(ALONG + 1);
            let mut values = Vec::with_capacity(ALONG + 1);
            let mut tangent = true;
            let mut largest = 0.0_f64;
            for k in 0..=ALONG {
                let q = lo + (hi - lo) * k as f64 / ALONG as f64;
                let (u, v) = border.at(domain, q);
                let point = data.surface.eval(u, v);
                let (Some(own), Some((theirs, off))) = (
                    data.surface.normal(u, v).map(|n| n * sign),
                    outward(body, neighbour, point),
                ) else {
                    tangent = false;
                    break;
                };
                if !(own.cross(theirs).length() <= TANGENT && own.dot(theirs) > 0.0 && off <= tol) {
                    tangent = false;
                    break;
                }
                let target = if mutual {
                    (own + theirs).normalized().unwrap_or(own)
                } else {
                    theirs
                };
                params.push(q);
                let turn = (target - own) * sign;
                largest = largest.max(turn.length());
                values.push(vec![turn.x, turn.y, turn.z]);
            }
            if !tangent {
                continue;
            }
            if distance.abs() * largest > bound {
                return Err(TooFar {
                    faces: vec![*face, neighbour],
                });
            }
            let Ok((knots, ctrl)) = interpolate(&params, 3.min(ALONG), &values) else {
                continue;
            };
            let ctrl: Option<Vec<Vec3>> = ctrl
                .iter()
                .map(|c| match c.as_slice() {
                    [x, y, z] => Some(Vec3::new(*x, *y, *z)),
                    _ => None,
                })
                .collect();
            let Some(turn) = ctrl.and_then(|c| NurbsCurve::polynomial(3, knots, c).ok()) else {
                continue;
            };
            if let Some(slot) = sides.get_mut(index) {
                *slot = Some(Side {
                    turn,
                    range: (lo, hi),
                    border: (f0, f1),
                    fixed: !mutual,
                });
            }
        }
        if sides.iter().any(Option::is_some) {
            let turn = Turn { domain, sides };
            // The blend across the face, measured on a grid.
            let ((u0, u1), (v0, v1)) = domain;
            for i in 0..=ACROSS {
                for j in 0..=ACROSS {
                    let (a, b) = (i as f64 / ACROSS as f64, j as f64 / ACROSS as f64);
                    let moved = turn.at(u0 + (u1 - u0) * a, v0 + (v1 - v0) * b).length();
                    if distance.abs() * moved > bound {
                        return Err(TooFar { faces: vec![*face] });
                    }
                }
            }
            out.insert(*face, turn);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A turn measured where an edge covers the middle half of a border
    /// (the rest of the border along other edges, or along none): the
    /// turn holds there, and past the ends of that part falls linearly to
    /// nothing at the border's ends; it is never more than at the edge.
    #[test]
    fn a_turn_past_the_part_of_a_border_its_edge_covers_falls_to_nothing() {
        let constant = Vec3::new(0.0, 0.0, 1e-6);
        let turn =
            NurbsCurve::polynomial(1, vec![0.25, 0.25, 0.75, 0.75], vec![constant, constant])
                .unwrap();
        let side = Side {
            turn,
            range: (0.25, 0.75),
            border: (0.0, 1.0),
            fixed: true,
        };
        for (q, share) in [
            (0.0, 0.0),
            (0.125, 0.5),
            (0.25, 1.0),
            (0.5, 1.0),
            (0.75, 1.0),
            (0.875, 0.5),
            (1.0, 0.0),
            (1.5, 0.0),
        ] {
            let got = side.at(q);
            assert!((got - constant * share).length() <= 1e-18, "{q}: {got:?}");
        }
        // An edge that covers the whole border: no falling off.
        let whole = Side {
            range: (0.0, 1.0),
            ..side
        };
        assert!((whole.at(0.0) - constant).length() <= 1e-18);
    }
}
