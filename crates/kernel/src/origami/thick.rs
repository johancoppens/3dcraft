//! Thick panels: a folded crease pattern built as one solid panel per
//! facet, the hinges kept at the zero-thickness sheet
//! (docs/design/origami.md › *Thick panels*).
//!
//! This is the *membrane* model of thick origami (the hinges on one
//! surface of the panels, as a tape or a fabric joins rigid panels:
//! Zirbel et al., "Accommodating thickness in origami-based deployable
//! arrays", *Journal of Mechanical Design* 135, 2013, which names it
//! among the methods it compares): every facet becomes a prism of the
//! given thickness on one side of the sheet (its back or its front),
//! its outline set back by the clearance from every crease and every cut
//! it lies on (the boundary is not set back). The kinematics are those of
//! the zero-thickness pattern exactly, since every hinge stays on the
//! sheet, so every fold the sheet makes the panels make; what thickness
//! adds is room: two panels beside a crease meet when the crease folds
//! towards their side by more than `2 atan(c / T)` (clearance `c`,
//! thickness `T`): the corner of one at the clearance and the far face
//! reaches the other there, `c (1 + cos ρ) = T sin ρ`. Folding away from
//! the panels they never meet below 180 degrees. Every pair of panels is
//! checked as solids (their faces against each other with the
//! zero-thickness contact test, and one corner of each in the other), so
//! a fold whose panels meet anywhere, at a crease, round a vertex or far
//! apart, is refused naming the two facets.
//!
//! What it is not: the offset-panel technique (Edmondson et al. 2014)
//! and the tapered panels of Tachi (2011), which move the hinges off the
//! sheet so that panels of any thickness fold flat, change the
//! kinematics; they are owed.

use super::collide::{Placed, meet, polygon_face};
use super::{Arrangement, FacetMeeting, FoldError, Folded, Meeting, P2, cross, dot, lift, sub};
use crate::boxtree::BoxTree;
use crate::extrude::{OpResult, Origin, Origins, Role, extrude};
use crate::profile::{Loop2, Profile, Segment, Shape};
use craft3d_math::{Bbox3, Frame, Tolerance, Transform, Vec3};
use std::collections::BTreeMap;

/// Which side of the sheet the panels lie on, as the sketch's normal
/// points (valleys fold towards the front).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelSide {
    Back,
    Front,
}

/// The panels' thickness and the clearance they keep from every crease
/// and cut, in the sheet's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panels {
    pub thickness: f64,
    pub clearance: f64,
    pub side: PanelSide,
}

/// A panel's outline: per ring (outer first) its corners and the
/// half-edge each edge comes from.
type Rings = Vec<(Vec<P2>, Vec<usize>)>;

/// The most corners one panel's outline may have.
const MAX_CORNERS: usize = 4096;

/// Each ring of facet `f` (outer first) set back by `clearance` from its
/// creases and cuts, as points and the half-edge each edge comes from.
fn shrunk(arr: &Arrangement, f: usize, clearance: f64, tol: f64) -> Result<Rings, FoldError> {
    let facet = arr.facets.get(f).ok_or(FoldError::Broken {
        reason: "a facet is missing",
    })?;
    let refuse = |reason: &'static str| FoldError::Clearance { facet: f, reason };
    let mut out = Vec::with_capacity(1 + facet.holes.len());
    for cycle in std::iter::once(&facet.outer).chain(&facet.holes) {
        if cycle.len() > MAX_CORNERS {
            return Err(FoldError::TooMany {
                what: "corners of one panel",
                limit: MAX_CORNERS,
            });
        }
        let ring = arr.polygon(cycle);
        let n = ring.len();
        if n < 3 || n != cycle.len() {
            return Err(refuse("its outline has fewer than three corners"));
        }
        // Per edge: its start, unit direction, left normal and offset.
        let mut lines = Vec::with_capacity(n);
        for (i, &h) in cycle.iter().enumerate() {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                return Err(refuse("an edge is missing"));
            };
            let d = sub(b, a);
            let len = dot(d, d).sqrt();
            if !(len.is_finite() && len > tol) {
                return Err(refuse("an edge is shorter than the tolerance"));
            }
            let u = [d[0] / len, d[1] / len];
            let offset = if matches!(
                arr.edges.get(h / 2).map(|e| e.kind),
                Some(super::EdgeKind::Boundary)
            ) {
                0.0
            } else {
                clearance
            };
            lines.push((a, u, [-u[1], u[0]], offset));
        }
        // Each corner where the offset lines of the edges into it and out
        // of it meet.
        let mut points = Vec::with_capacity(n);
        for i in 0..n {
            let (Some(&(_, u1, n1, o1)), Some(&(a2, u2, n2, o2))) =
                (lines.get((i + n - 1) % n), lines.get(i))
            else {
                return Err(refuse("an edge is missing"));
            };
            let p1 = [a2[0] + n1[0] * o1, a2[1] + n1[1] * o1];
            let p2 = [a2[0] + n2[0] * o2, a2[1] + n2[1] * o2];
            let den = cross(u1, u2);
            let corner = if den.abs() <= 1e-12 {
                // The two edges run on along one line (or turn back): one
                // offset moves the corner square to it; two different
                // offsets (a crease running on into the boundary) or an
                // edge that turns back (a slit's tip) have no mitre.
                if dot(u1, u2) > 0.0 && (o1 - o2).abs() <= tol * 1e-3 {
                    p2
                } else if dot(u1, u2) > 0.0 {
                    return Err(refuse(
                        "a crease or a cut runs on straight into the boundary there, where a set-back needs a step",
                    ));
                } else {
                    return Err(refuse(
                        "a cut ends inside it (a slit or a slot), which a panel of no width there cannot have",
                    ));
                }
            } else {
                // p1 + t u1 = p2 + s u2.
                let t = cross(sub(p2, p1), u2) / den;
                [p1[0] + u1[0] * t, p1[1] + u1[1] * t]
            };
            if !(corner[0].is_finite() && corner[1].is_finite()) {
                return Err(refuse("a corner is not finite"));
            }
            points.push(corner);
        }
        // Every edge keeps its direction and some length.
        for i in 0..n {
            let (Some(&a), Some(&b), Some(&(_, u, _, _))) =
                (points.get(i), points.get((i + 1) % n), lines.get(i))
            else {
                return Err(refuse("an edge is missing"));
            };
            if dot(sub(b, a), u) <= tol {
                return Err(refuse("the clearance takes away a whole edge of it"));
            }
        }
        out.push((points, cycle.clone()));
    }
    // No two edges of the outline cross, and every hole stays inside.
    let edges: Vec<(P2, P2)> = out
        .iter()
        .flat_map(|(r, _)| {
            let n = r.len();
            (0..n).filter_map(move |i| Some((*r.get(i)?, *r.get((i + 1) % n)?)))
        })
        .collect();
    for (i, &(a, b)) in edges.iter().enumerate() {
        for &(c, d) in edges.iter().skip(i + 1) {
            if proper_crossing(a, b, c, d) {
                return Err(refuse("the clearance makes its outline cross itself"));
            }
        }
    }
    if let Some((outer, _)) = out.first() {
        for (hole, _) in out.iter().skip(1) {
            if let Some(&p) = hole.first()
                && !super::inside_loops(p, [outer.as_slice()])
            {
                return Err(refuse("the clearance takes it past one of its holes"));
            }
        }
    }
    Ok(out)
}

fn proper_crossing(a: P2, b: P2, c: P2, d: P2) -> bool {
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    if den == 0.0 {
        return false;
    }
    let w = sub(c, a);
    let (t, v) = (cross(w, s) / den, cross(w, r) / den);
    t > 1e-12 && t < 1.0 - 1e-12 && v > 1e-12 && v < 1.0 - 1e-12
}

/// The thick panels of a folded crease pattern in the sketch's plane
/// `plane`: one prism per facet (its outline set back by the clearance
/// from its creases and cuts, `thickness` thick on the side `side`), a
/// solid of one shell per panel. Refused when the clearance does not fit
/// a facet, or when two panels meet (naming the facets). Names are an
/// extrusion's, each panel's outline segments keyed by the curve and,
/// per half-edge, its piece and side (`2 piece + side`, as a cut's two
/// sides are named on the sheet), its caps by the curves of its outer
/// boundary with a number when two panels have the same.
pub fn thick_panels(
    arr: &Arrangement,
    folded: &Folded,
    plane: &Frame,
    panels: &Panels,
    tolerance: &Tolerance,
) -> Result<OpResult, FoldError> {
    let tol = tolerance.linear;
    let (t, c) = (panels.thickness, panels.clearance);
    if !(t.is_finite() && t > tol) {
        return Err(FoldError::Panels {
            reason: "the thickness must be larger than the tolerance",
        });
    }
    if !(c.is_finite() && c > tol) {
        return Err(FoldError::Panels {
            reason: "the clearance must be larger than the tolerance: with none the panels touch along every crease",
        });
    }
    if folded.transforms.len() != arr.facets.len() {
        return Err(FoldError::Build(
            "the fold does not match the facets".to_string(),
        ));
    }
    let (from, to) = match panels.side {
        PanelSide::Back => (-t, 0.0),
        PanelSide::Front => (0.0, t),
    };
    let depth = if panels.side == PanelSide::Back {
        -t
    } else {
        t
    };

    let mut rings_of = Vec::with_capacity(arr.facets.len());
    for f in 0..arr.facets.len() {
        rings_of.push(shrunk(arr, f, c, tol)?);
    }

    // The panels as solids, checked against each other first (in the
    // sheet's coordinates, as the fold placed the facets).
    let mut next_id = 0usize;
    let mut faces: Vec<Vec<Placed>> = Vec::with_capacity(arr.facets.len());
    let mut boxes = Vec::with_capacity(arr.facets.len());
    for (f, rings) in rings_of.iter().enumerate() {
        let tf = *folded.transforms.get(f).ok_or(FoldError::Broken {
            reason: "a facet has no place",
        })?;
        let flat: Vec<Vec<P2>> = rings.iter().map(|(r, _)| r.clone()).collect();
        let lid = Transform::translation(Vec3::new(0.0, 0.0, depth)).then(&tf);
        let mut mine = Vec::new();
        let fail = || FoldError::Broken {
            reason: "a panel's face could not be placed",
        };
        mine.push(polygon_face(flat.clone(), tf, &mut next_id).ok_or_else(fail)?);
        mine.push(polygon_face(flat.clone(), lid, &mut next_id).ok_or_else(fail)?);
        for ring in &flat {
            let n = ring.len();
            for i in 0..n {
                let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else {
                    continue;
                };
                let along = lift(sub(b, a));
                let len = along.length();
                let frame = Frame::new(lift(a), along.cross(Vec3::Z), along).ok_or_else(fail)?;
                let side = Transform::from_frame(&frame).then(&tf);
                let quad = vec![vec![[0.0, 0.0], [len, 0.0], [len, depth], [0.0, depth]]];
                mine.push(polygon_face(quad, side, &mut next_id).ok_or_else(fail)?);
            }
        }
        let corners = mine.iter().flat_map(|p| p.corners()).collect::<Vec<_>>();
        let grow = Vec3::new(tol, tol, tol);
        let b = Bbox3::from_points(corners.iter().copied());
        let (lo, hi) = b.corners().ok_or_else(fail)?;
        boxes.push(Bbox3::from_points([lo - grow, hi + grow]));
        faces.push(mine);
    }
    let tree = BoxTree::new(boxes.clone());
    let mut near = Vec::new();
    for i in 0..faces.len() {
        let Some(mine) = boxes.get(i) else { continue };
        let mut budget = usize::MAX;
        tree.search(|b| boxes_meet(b, mine), &mut near, &mut budget);
        near.retain(|&j| j > i);
        near.sort_unstable();
        for &j in &near {
            let (Some(a), Some(b)) = (faces.get(i), faces.get(j)) else {
                continue;
            };
            let found = a
                .iter()
                .flat_map(|p| b.iter().map(move |q| (p, q)))
                .find_map(|(p, q)| meet(p, q, tol, tolerance.angular));
            let found = match found {
                Some(m) => Some(m),
                None => contained(folded, &rings_of, i, j, depth, tol)
                    .or_else(|| contained(folded, &rings_of, j, i, depth, tol)),
            };
            if let Some((kind, at)) = found {
                return Err(FoldError::PanelsMeet(Meeting {
                    facets: [i, j],
                    kind,
                    at,
                }));
            }
        }
    }

    // The body: one extrusion per panel in the facet's folded plane.
    let world = |p: Vec3| plane.point(p.x, p.y, p.z);
    let mut bodies = Vec::with_capacity(arr.facets.len());
    let mut origins = Origins::default();
    let mut used: BTreeMap<Vec<u64>, u32> = BTreeMap::new();
    for (f, rings) in rings_of.iter().enumerate() {
        let tf = folded.transforms.get(f).ok_or(FoldError::Broken {
            reason: "a facet has no place",
        })?;
        let frame = {
            let n = tf.vector(Vec3::Z);
            let x = tf.vector(Vec3::X);
            Frame::new(
                world(tf.point(Vec3::ZERO)),
                plane.vector(n.x, n.y, n.z),
                plane.vector(x.x, x.y, x.z),
            )
            .ok_or_else(|| FoldError::Build("a panel's plane is not finite".to_string()))?
        };
        let mut loops = Vec::with_capacity(rings.len());
        for (ring, cycle) in rings {
            let n = ring.len();
            let mut segments = Vec::with_capacity(n);
            for (i, &h) in cycle.iter().enumerate() {
                let (Some(&a), Some(&b), Some(e)) =
                    (ring.get(i), ring.get((i + 1) % n), arr.edges.get(h / 2))
                else {
                    return Err(FoldError::Broken {
                        reason: "an edge is missing",
                    });
                };
                segments.push(Segment {
                    source: e.source,
                    piece: e
                        .piece
                        .saturating_mul(2)
                        .saturating_add(u32::try_from(h % 2).unwrap_or(1)),
                    reversed: false,
                    start: a,
                    end: b,
                    shape: Shape::Line,
                });
            }
            loops.push(Loop2 { segments });
        }
        let mut loops = loops.into_iter();
        let profile = Profile {
            outer: loops.next().ok_or(FoldError::Broken {
                reason: "a panel has no outline",
            })?,
            holes: loops.collect(),
        };
        let made = extrude(&[profile], &frame, from, to, tolerance)
            .map_err(|e| FoldError::Build(format!("panel of facet {f}: {e}")))?;
        let mut o = made.origins;
        // Two panels whose outer boundaries have the same curves: their
        // caps numbered in facet order.
        let sources = o
            .faces
            .iter()
            .find(|x| x.role == Role::StartCap)
            .map(|x| x.sources.clone())
            .unwrap_or_default();
        let piece = used.entry(sources).or_insert(0);
        for x in &mut o.faces {
            if matches!(x.role, Role::StartCap | Role::EndCap) {
                *x = Origin::of_piece(x.role, &x.sources, *piece);
            }
        }
        *piece = piece.saturating_add(1);
        origins.faces.extend(o.faces);
        origins.edges.extend(o.edges);
        origins.vertices.extend(o.vertices);
        bodies.push(made.body);
    }
    let refs: Vec<&craft3d_topo::Body> = bodies.iter().collect();
    let (body, _) = craft3d_topo::merge(&refs).map_err(|e| FoldError::Build(e.to_string()))?;
    Ok(OpResult { body, origins })
}

/// Whether a corner of panel `a` lies inside panel `b` (further than the
/// tolerance from its faces): the one way two solids meet with no faces
/// meeting.
fn contained(
    folded: &Folded,
    rings_of: &[Rings],
    a: usize,
    b: usize,
    depth: f64,
    tol: f64,
) -> Option<(FacetMeeting, Vec3)> {
    let p = *rings_of.get(a)?.first()?.0.first()?;
    let x = folded.transforms.get(a)?.point(lift(p));
    let q = folded.transforms.get(b)?.inverse()?.point(x);
    let (lo, hi) = (depth.min(0.0), depth.max(0.0));
    if !(q.z > lo + tol && q.z < hi - tol) {
        return None;
    }
    let rings: Vec<Vec<P2>> = rings_of.get(b)?.iter().map(|(r, _)| r.clone()).collect();
    let flat = [q.x, q.y];
    (super::inside_loops(flat, rings.iter().map(Vec::as_slice))
        && super::ring_distance(&rings, flat) > tol)
        .then_some((FacetMeeting::Through, x))
}

fn boxes_meet(a: &Bbox3, b: &Bbox3) -> bool {
    match (a.corners(), b.corners()) {
        (Some((alo, ahi)), Some((blo, bhi))) => {
            alo.x <= bhi.x
                && blo.x <= ahi.x
                && alo.y <= bhi.y
                && blo.y <= ahi.y
                && alo.z <= bhi.z
                && blo.z <= ahi.z
        }
        _ => false,
    }
}
