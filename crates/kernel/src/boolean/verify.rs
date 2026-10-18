//! A cheap check of a boolean's result against what every exact boolean
//! of the same operands satisfies, beside `craft3d_topo::check` (which
//! only says the result is a valid solid, not that it is the right one).
//!
//! Given the target A, the tool B, the operation and the result R:
//!
//! 1. **Volumes.** Union: `max(V(A), V(B)) ≤ V(R) ≤ V(A) + V(B)`.
//!    Subtract: `V(A) − V(B) ≤ V(R) ≤ V(A)`. Intersect:
//!    `V(A) + V(B) − V(H) ≤ V(R) ≤ min(V(A), V(B))`, with `H` the box
//!    around both (which holds `A ∪ B`); and `V(R) ≥ 0` for every
//!    operation. Each is held to an allowance that is derived, not tuned:
//!    the three volumes' own error bounds (`craft3d_topo::mass_bound`, the
//!    integrator's account of what its sums were held to), and the volume
//!    within the tolerance of the operands' boundaries, `2 t (S(A) +
//!    S(B))` (the first term of the volume of the band of half-width `t`
//!    around a surface of area `S`): the boolean decides what lies within
//!    the linear tolerance `t` of a face of the other operand (coincident,
//!    touching, one vertex), so the result may differ from the exact set
//!    by what lies that close to the boundary of either operand, and by
//!    nothing else. The further terms of that band's volume (`π t²` times
//!    the length of the edges, `t³` per vertex) are smaller than the first
//!    by the ratio of the tolerance to the width of a face and are left
//!    out. A body that cannot be measured leaves the volumes unchecked
//!    (reported, never a violation).
//! 2. **Where the result can be.** Every vertex of R, and the middle of
//!    every edge, lies on the boundary of A or of B, inside the closure of
//!    the result: so within the tolerance of the box of A or of B for a
//!    union, of the box of A for a difference, and of both boxes for an
//!    intersection.
//! 3. **Outside the tool's reach the result is the target.** A face of A
//!    whose box is further than the tolerance from the box of B is
//!    outside B, and so (union, subtract) in R as it is: a face of R on
//!    the same surface (the same one, or one within the tolerance of it at
//!    the face's points, facing the same way), with the same loops and the
//!    same edges, each with the same ends and middle within the tolerance.
//!    For a union the same holds for the faces of B far from A. (For an
//!    intersection such a face is outside the result, which 2 already
//!    checks: no vertex of R is there.)
//! 4. **Membership.** At a few seeded points (in the box around both, and
//!    in the box where the two overlap), whether each is in R is the set
//!    formula of whether it is in A and in B (`a ∨ b`, `a ∧ ¬b`, `a ∧ b`).
//!    A point within twice the tolerance of any of the three boundaries
//!    (where [`super::contains_points`] does not tell) is passed over.
//!
//! With a sheet: a sheet trimmed by a solid is held to the same rules (a
//! sheet's volume is zero, and it has no inside, so no point is
//! compared); a solid cut by a sheet is held to rules 1 and 2 of a
//! difference, no larger than the solid and within its box, since the
//! part a cut keeps of a face far from the sheet is the side it is on.
//!
//! Every rule holds for any pair of solids and any surface; nothing in it
//! knows what kind of faces it is given. It costs a box per face, a match
//! per face far from the other operand (no intersections), a ray per
//! point and body, and with the volumes three mass integrations.

use super::{BooleanOp, Operand, contains_points};
use crate::KernelError;
use craft3d_geom::Surface;
use craft3d_math::{Bbox3, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, FaceId, VertexId, mass_bound};
use std::fmt;

/// What [`verify`] looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Measure the volumes of A, B and R (rule 1): three mass
    /// integrations, which on freeform faces cost more than the rest.
    pub volumes: bool,
    /// How many seeded points to classify (rule 4); half of them in the
    /// box where A and B overlap.
    pub points: usize,
}

impl VerifyOptions {
    /// What every boolean checks of its own result before returning it:
    /// rules 2 and 3, a few per cent of the boolean's time (measured on
    /// the bench models, docs/design/booleans.md).
    pub const ALWAYS: VerifyOptions = VerifyOptions {
        volumes: false,
        points: 0,
    };
}

/// What every boolean checks of its own result: [`VerifyOptions::ALWAYS`],
/// or everything when the environment variable `CRAFT3D_BOOLEAN_CHECK`
/// is `full` (read once per process): a sweep of a whole test suite with
/// the full check on every boolean any operation makes, fillets and
/// shells included (docs/testing.md). The switch is for test runs only:
/// with it a boolean may be refused in one process and not in another, so
/// a document must never depend on it.
pub fn checked_by_every_boolean() -> VerifyOptions {
    static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let full = *FULL
        .get_or_init(|| std::env::var("CRAFT3D_BOOLEAN_CHECK").is_ok_and(|value| value == "full"));
    if full {
        VerifyOptions::default()
    } else {
        VerifyOptions::ALWAYS
    }
}

impl Default for VerifyOptions {
    fn default() -> Self {
        VerifyOptions {
            volumes: true,
            points: 16,
        }
    }
}

/// Which volume bound is broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeRule {
    /// A union lost volume: less than the larger operand.
    UnionLoses,
    /// A union gained volume: more than both operands together.
    UnionGains,
    /// A difference added volume: more than the target.
    SubtractAdds,
    /// A difference took away more than the tool.
    SubtractTakesTooMuch,
    /// An intersection is larger than the smaller operand.
    IntersectGains,
    /// An intersection is smaller than the operands' overlap must be.
    IntersectLoses,
    /// A negative volume.
    Negative,
}

impl VolumeRule {
    fn text(self) -> &'static str {
        match self {
            VolumeRule::UnionLoses => "a union lost volume (less than the larger operand)",
            VolumeRule::UnionGains => "a union gained volume (more than both operands together)",
            VolumeRule::SubtractAdds => "a subtraction added volume (more than the target)",
            VolumeRule::SubtractTakesTooMuch => {
                "a subtraction took away more than the tool's volume"
            }
            VolumeRule::IntersectGains => "an intersection is larger than the smaller operand",
            VolumeRule::IntersectLoses => {
                "an intersection is smaller than the operands' overlap must be"
            }
            VolumeRule::Negative => "the result has a negative volume",
        }
    }
}

/// One thing the result does that no exact boolean of its operands does.
#[derive(Clone, Debug, PartialEq)]
pub enum Violation {
    /// `result` is beyond `limit` by more than `allowance`.
    Volume {
        rule: VolumeRule,
        result: f64,
        limit: f64,
        allowance: f64,
    },
    /// A vertex of the result, or the middle of an edge (`vertex` is then
    /// `None`), where the result cannot be.
    OutOfReach { vertex: Option<VertexId>, at: Vec3 },
    /// A face of an operand far from the other that is not in the result
    /// as it was: `problem` says what differs.
    FarFace {
        operand: Operand,
        face: FaceId,
        problem: &'static str,
    },
    /// A point whose membership of the result is not the set formula of
    /// its membership of the operands.
    Membership {
        at: Vec3,
        in_a: bool,
        in_b: bool,
        in_result: bool,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Volume {
                rule,
                result,
                limit,
                allowance,
            } => write!(
                f,
                "{}: volume {result:e} against {limit:e}, allowance {allowance:e}",
                rule.text()
            ),
            Violation::OutOfReach { vertex, at } => match vertex {
                Some(v) => write!(
                    f,
                    "vertex {} at ({}, {}, {}) is where the result cannot be",
                    v.0, at.x, at.y, at.z
                ),
                None => write!(
                    f,
                    "an edge passes through ({}, {}, {}), where the result cannot be",
                    at.x, at.y, at.z
                ),
            },
            Violation::FarFace {
                operand,
                face,
                problem,
            } => {
                let which = match operand {
                    Operand::A => "target",
                    Operand::B => "tool",
                };
                write!(
                    f,
                    "face {} of the {which}, out of the other's reach, is not in the result as it was: {problem}",
                    face.0
                )
            }
            Violation::Membership {
                at,
                in_a,
                in_b,
                in_result,
            } => {
                let word = |inside: bool| if inside { "inside" } else { "outside" };
                write!(
                    f,
                    "the point ({}, {}, {}) is {} the target and {} the tool, and {} the result",
                    at.x,
                    at.y,
                    at.z,
                    word(*in_a),
                    word(*in_b),
                    word(*in_result)
                )
            }
        }
    }
}

/// The three volumes and what they were held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub a: f64,
    pub b: f64,
    pub result: f64,
    pub allowance: f64,
}

/// What [`verify`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verification {
    /// Empty when the result is consistent with every rule.
    pub violations: Vec<Violation>,
    /// The volumes, when they were measured.
    pub volumes: Option<Volumes>,
    /// Why the volumes were not checked, when they were asked for and one
    /// of the bodies could not be measured.
    pub unmeasured: Option<String>,
    /// Faces of the operands far from the other one that were matched.
    pub far_faces: usize,
    /// Points whose membership was compared (those near no boundary).
    pub points: usize,
    /// Why no point was compared, when one of the bodies could not be
    /// classified.
    pub unclassified: Option<String>,
}

impl Verification {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks `result`, the boolean `op` of `a` and `b`, against the rules of
/// the module documentation. An error for a tolerance that is not
/// positive and finite and for a body whose box cannot be had (geometry
/// that is not finite); everything else it finds is in the
/// [`Verification`].
pub fn verify(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    result: &Body,
    tolerance: &Tolerance,
    options: &VerifyOptions,
) -> Result<Verification, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    let (boxes_a, boxes_b) = (face_boxes(a)?, face_boxes(b)?);
    let (box_a, box_b) = (boxes_a.whole, boxes_b.whole);
    let mut found = Verification::default();
    // A solid cut by a sheet keeps a part of the solid on one side of the
    // sheet: within the solid's box, no larger than the solid, and
    // nothing of it is known to be kept far from the sheet (the side
    // there is the side of the cut). Rules 2 and 1 as for a difference;
    // no faces far from the sheet and no membership (the sheet has no
    // inside).
    if a.kind == BodyKind::Solid && b.kind == BodyKind::Sheet {
        if options.volumes {
            volumes(
                a,
                b,
                BooleanOp::Subtract,
                result,
                (&box_a, &box_b),
                tol,
                &mut found,
            );
        }
        out_of_reach(
            BooleanOp::Subtract,
            result,
            (&box_a, &box_b),
            tol,
            &mut found.violations,
        );
        return Ok(found);
    }
    if options.volumes {
        volumes(a, b, op, result, (&box_a, &box_b), tol, &mut found);
    }
    let index = out_of_reach(op, result, (&box_a, &box_b), tol, &mut found.violations);
    let mut far = |of: &Body, boxes: &Boxes, operand: Operand, other: &Bbox3| {
        far_faces((of, boxes, operand), other, result, &index, tol, &mut found);
    };
    match op {
        BooleanOp::Union => {
            far(a, &boxes_a, Operand::A, &box_b);
            far(b, &boxes_b, Operand::B, &box_a);
        }
        BooleanOp::Subtract => far(a, &boxes_a, Operand::A, &box_b),
        BooleanOp::Intersect => {}
    }
    membership(
        a,
        b,
        op,
        result,
        (&box_a, &box_b),
        tolerance,
        options,
        &mut found,
    );
    Ok(found)
}

/// A body or a face whose box cannot be had: geometry that is not finite,
/// or a face that is not bounded, which `craft3d_topo::check` refuses.
fn unboxed() -> KernelError {
    KernelError::Unsupported {
        operation: "boolean check",
        reason: "a body or a face without a finite box (run craft3d_topo::check)",
        faces: Vec::new(),
    }
}

/// Rule 1.
fn volumes(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    result: &Body,
    (box_a, box_b): (&Bbox3, &Bbox3),
    tol: f64,
    found: &mut Verification,
) {
    let measured = [a, b, result].map(mass_bound);
    let [Ok((ma, ba)), Ok((mb, bb)), Ok((mr, br))] = measured else {
        let why = measured
            .iter()
            .zip(["target", "tool", "result"])
            .find_map(|(m, which)| m.as_ref().err().map(|e| format!("the {which}: {e}")));
        found.unmeasured = why;
        return;
    };
    let allowance = ba.volume + bb.volume + br.volume + 2.0 * tol * (ma.area.abs() + mb.area.abs());
    let (va, vb, vr) = (ma.volume, mb.volume, mr.volume);
    found.volumes = Some(Volumes {
        a: va,
        b: vb,
        result: vr,
        allowance,
    });
    let hull = hull_volume(box_a, box_b);
    // (rule, the limit, whether the result must be at least it)
    let bounds: Vec<(VolumeRule, f64, bool)> = match op {
        BooleanOp::Union => vec![
            (VolumeRule::UnionLoses, va.max(vb), true),
            (VolumeRule::UnionGains, va + vb, false),
        ],
        // A solid cut by a sheet (the volume of a sheet is zero): no more
        // than the solid, and what is cut away is not bounded by the tool.
        BooleanOp::Subtract if b.kind == BodyKind::Sheet => {
            vec![(VolumeRule::SubtractAdds, va, false)]
        }
        BooleanOp::Subtract => vec![
            (VolumeRule::SubtractAdds, va, false),
            (VolumeRule::SubtractTakesTooMuch, va - vb, true),
        ],
        BooleanOp::Intersect => {
            let mut bounds = vec![(VolumeRule::IntersectGains, va.min(vb), false)];
            if let Some(hull) = hull {
                bounds.push((VolumeRule::IntersectLoses, va + vb - hull, true));
            }
            bounds
        }
    };
    for (rule, limit, at_least) in bounds
        .into_iter()
        .chain([(VolumeRule::Negative, 0.0, true)])
    {
        let beyond = if at_least { limit - vr } else { vr - limit };
        // A NaN is beyond everything.
        if beyond.is_nan() || beyond > allowance {
            found.violations.push(Violation::Volume {
                rule,
                result: vr,
                limit,
                allowance,
            });
        }
    }
}

/// The volume of the box around both boxes.
fn hull_volume(a: &Bbox3, b: &Bbox3) -> Option<f64> {
    let (a0, a1) = a.corners()?;
    let (b0, b1) = b.corners()?;
    let (lo, hi) = (a0.min(b0), a1.max(b1));
    let d = hi - lo;
    Some(d.x * d.y * d.z)
}

/// The box of every face of a body, and of the body (theirs together,
/// with the vertices): one pass over the faces, by the rules of
/// `Body::face_bbox`.
struct Boxes {
    whole: Bbox3,
    faces: Vec<Bbox3>,
}

fn face_boxes(body: &Body) -> Result<Boxes, KernelError> {
    let mut whole = Bbox3::EMPTY;
    let mut faces = Vec::with_capacity(body.faces.len());
    for index in 0..body.faces.len() {
        let bbox = body
            .face_bbox(FaceId(u32::try_from(index).unwrap_or(u32::MAX)))
            .map_err(|_| unboxed())?;
        if let Some((lo, hi)) = bbox.corners() {
            whole.add(lo);
            whole.add(hi);
        }
        faces.push(bbox);
    }
    for vertex in &body.vertices {
        if !vertex.point.is_finite() {
            return Err(unboxed());
        }
        whole.add(vertex.point);
    }
    Ok(Boxes { whole, faces })
}

/// The result's edges by the middle of each (sorted along x), the faces
/// each edge bounds, and the faces without loops: how a face of an
/// operand finds its counterpart in the result without a box per face of
/// the result.
struct Index {
    middles: Vec<(Vec3, usize)>,
    faces_of: Vec<Vec<FaceId>>,
    loopless: Vec<FaceId>,
}

impl Index {
    /// The faces of the result with an edge whose middle is within `tol`
    /// of `at`.
    fn faces_near(&self, at: Vec3, tol: f64) -> Vec<FaceId> {
        let from = self.middles.partition_point(|(m, _)| m.x < at.x - tol);
        let mut faces = Vec::new();
        for &(middle, edge) in self.middles.get(from..).unwrap_or_default() {
            if middle.x > at.x + tol {
                break;
            }
            if middle.distance(at) <= tol {
                faces.extend(self.faces_of.get(edge).into_iter().flatten().copied());
            }
        }
        faces.sort();
        faces.dedup();
        faces
    }
}

/// Rule 2, and the [`Index`] of the result.
fn out_of_reach(
    op: BooleanOp,
    result: &Body,
    (box_a, box_b): (&Bbox3, &Bbox3),
    tol: f64,
    violations: &mut Vec<Violation>,
) -> Index {
    let allowed = |p: Vec3| {
        let (in_a, in_b) = (box_a.contains(p, tol), box_b.contains(p, tol));
        match op {
            BooleanOp::Union => in_a || in_b,
            BooleanOp::Subtract => in_a,
            BooleanOp::Intersect => in_a && in_b,
        }
    };
    for (index, vertex) in result.vertices.iter().enumerate() {
        if !allowed(vertex.point) {
            violations.push(Violation::OutOfReach {
                vertex: Some(VertexId(u32::try_from(index).unwrap_or(u32::MAX))),
                at: vertex.point,
            });
        }
    }
    let mut middles = Vec::with_capacity(result.edges.len());
    for (index, edge) in result.edges.iter().enumerate() {
        let middle = edge.curve.eval(0.5 * (edge.range.0 + edge.range.1));
        if !allowed(middle) {
            violations.push(Violation::OutOfReach {
                vertex: None,
                at: middle,
            });
        }
        middles.push((middle, index));
    }
    middles.sort_by(|p, q| p.0.x.total_cmp(&q.0.x).then(p.1.cmp(&q.1)));
    let mut faces_of = vec![Vec::new(); result.edges.len()];
    let mut loopless = Vec::new();
    for (index, face) in result.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(index).unwrap_or(u32::MAX));
        if face.loops.is_empty() {
            loopless.push(id);
        }
        for coedge in face
            .loops
            .iter()
            .filter_map(|&l| result.loop_(l))
            .flat_map(|l| &l.coedges)
        {
            if let Some(faces) = faces_of.get_mut(coedge.edge.0 as usize) {
                faces.push(id);
            }
        }
    }
    Index {
        middles,
        faces_of,
        loopless,
    }
}

/// How far apart two boxes are: zero where they overlap or touch, and
/// infinite where either is empty.
fn gap(a: &Bbox3, b: &Bbox3) -> f64 {
    let (Some((a0, a1)), Some((b0, b1))) = (a.corners(), b.corners()) else {
        return f64::INFINITY;
    };
    let along = |a0: f64, a1: f64, b0: f64, b1: f64| (b0 - a1).max(a0 - b1).max(0.0);
    Vec3::new(
        along(a0.x, a1.x, b0.x, b1.x),
        along(a0.y, a1.y, b0.y, b1.y),
        along(a0.z, a1.z, b0.z, b1.z),
    )
    .length()
}

/// An edge as its ends (in order) and its middle.
type Signature = (Vec3, Vec3, Vec3);

/// The edges of `face`'s loops, loop by loop.
fn signatures(body: &Body, face: FaceId) -> Option<Vec<Vec<Signature>>> {
    let face = body.face(face)?;
    let mut loops = Vec::with_capacity(face.loops.len());
    for &id in &face.loops {
        let mut edges = Vec::new();
        for coedge in &body.loop_(id)?.coedges {
            let edge = body.edge(coedge.edge)?;
            let start = body.vertex(edge.start)?.point;
            let end = body.vertex(edge.end)?.point;
            let middle = edge.curve.eval(0.5 * (edge.range.0 + edge.range.1));
            let (start, end) = if start.x < end.x || (start.x == end.x && start.y <= end.y) {
                (start, end)
            } else {
                (end, start)
            };
            edges.push((start, end, middle));
        }
        loops.push(edges);
    }
    Some(loops)
}

fn same_edge(p: &Signature, q: &Signature, tol: f64) -> bool {
    let near = |x: Vec3, y: Vec3| x.distance(y) <= tol;
    near(p.2, q.2) && ((near(p.0, q.0) && near(p.1, q.1)) || (near(p.0, q.1) && near(p.1, q.0)))
}

const INCOMPARABLE: &str = "a surface that cannot be compared at the face's points";
const ANOTHER_SURFACE: &str = "another surface";
const TURNED: &str = "turned inside out";
/// What [`differs`] can say once the edges matched.
const AFTER_THE_EDGES: [&str; 3] = [INCOMPARABLE, ANOTHER_SURFACE, TURNED];

/// What differs between `face` of `of` and `candidate` of `result`, or
/// `None` when they are the same face up to the tolerance.
fn differs(
    of: &Body,
    face: FaceId,
    result: &Body,
    candidate: FaceId,
    tol: f64,
) -> Option<&'static str> {
    let (Some(f), Some(g)) = (of.face(face), result.face(candidate)) else {
        return Some("a face that is not in the body");
    };
    // Counted before anything is evaluated: a face with many loops next
    // to a far face (a plate around its holes) is passed over at once.
    let count = |body: &Body, face: &craft3d_topo::Face| -> usize {
        face.loops
            .iter()
            .filter_map(|&l| body.loop_(l))
            .map(|l| l.coedges.len())
            .sum()
    };
    if f.loops.len() != g.loops.len() {
        return Some("another number of loops");
    }
    if count(of, f) != count(result, g) {
        return Some("another number of edges");
    }
    let (Some(mine), Some(theirs)) = (signatures(of, face), signatures(result, candidate)) else {
        return Some("a face whose loops refer to something missing");
    };
    if mine.len() != theirs.len() {
        return Some("another number of loops");
    }
    let mine: Vec<Signature> = mine.into_iter().flatten().collect();
    let theirs: Vec<Signature> = theirs.into_iter().flatten().collect();
    if mine.len() != theirs.len() {
        return Some("another number of edges");
    }
    // Each of the face's edges matched to its own edge of the candidate.
    let mut used = vec![false; theirs.len()];
    for edge in &mine {
        let matched = theirs
            .iter()
            .zip(used.iter_mut())
            .find(|(other, used)| !**used && same_edge(edge, other, tol));
        match matched {
            Some((_, used)) => *used = true,
            None => return Some("an edge that is not there or has moved"),
        }
    }
    if f.surface == g.surface {
        return (f.reversed != g.reversed).then_some(TURNED);
    }
    // Not the same surface: the candidate's points must lie on the face's
    // surface, with the same outward normal there.
    for &(start, _, middle) in &theirs {
        for p in [start, middle] {
            let (Some(on_f), Some(on_g)) = (foot(&f.surface, p), foot(&g.surface, p)) else {
                return Some(INCOMPARABLE);
            };
            if on_f.0.distance(p) > tol || on_g.0.distance(p) > tol {
                return Some(ANOTHER_SURFACE);
            }
            let outward = |n: Vec3, reversed: bool| if reversed { -n } else { n };
            if outward(on_f.1, f.reversed).dot(outward(on_g.1, g.reversed)) <= 0.0 {
                return Some(TURNED);
            }
        }
    }
    None
}

/// The point of `surface` nearest `p` and the normal there.
fn foot(surface: &Surface, p: Vec3) -> Option<(Vec3, Vec3)> {
    let (u, v) = surface.project(p).ok()?;
    Some((surface.eval(u, v), surface.normal(u, v)?))
}

/// Rule 3 for the faces of `of` (the operand `operand`, with its face
/// boxes) far from `other`: each is matched to a face of the result that
/// has an edge where its first edge is (or, for a face without loops, to
/// the result's faces without loops).
fn far_faces(
    (of, boxes, operand): (&Body, &Boxes, Operand),
    other: &Bbox3,
    result: &Body,
    index: &Index,
    tol: f64,
    found: &mut Verification,
) {
    const MISSING: &str = "no face of the result is where it was";
    for (at, bbox) in boxes.faces.iter().enumerate() {
        let id = FaceId(u32::try_from(at).unwrap_or(u32::MAX));
        if gap(bbox, other) <= tol {
            continue;
        }
        let first = of
            .face(id)
            .and_then(|face| face.loops.first())
            .and_then(|&l| of.loop_(l))
            .and_then(|l| l.coedges.first())
            .and_then(|c| of.edge(c.edge));
        let candidates = match first {
            Some(edge) => {
                index.faces_near(edge.curve.eval(0.5 * (edge.range.0 + edge.range.1)), tol)
            }
            None => index.loopless.clone(),
        };
        let mut problem = Some(MISSING);
        for candidate in candidates {
            match differs(of, id, result, candidate, tol) {
                None => {
                    problem = None;
                    break;
                }
                // A difference is the face's own when the candidate is on
                // its surface, or has its edges and differs only in the
                // surface or the side (those differences come after the
                // edges are matched); the first such is reported. A
                // neighbour that shares an edge with it says nothing.
                Some(what)
                    if problem == Some(MISSING)
                        && (AFTER_THE_EDGES.contains(&what)
                            || of
                                .face(id)
                                .zip(result.face(candidate))
                                .is_some_and(|(f, g)| f.surface == g.surface)) =>
                {
                    problem = Some(what);
                }
                Some(_) => {}
            }
        }
        match problem {
            None => found.far_faces += 1,
            Some(problem) => found.violations.push(Violation::FarFace {
                operand,
                face: id,
                problem,
            }),
        }
    }
}

/// A seeded stream of numbers in [0, 1) (SplitMix64): the same points on
/// every run.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    fn point_in(&mut self, lo: Vec3, hi: Vec3) -> Vec3 {
        let (x, y, z) = (self.next(), self.next(), self.next());
        Vec3::new(
            lo.x + x * (hi.x - lo.x),
            lo.y + y * (hi.y - lo.y),
            lo.z + z * (hi.z - lo.z),
        )
    }
}

/// Rule 4.
#[allow(clippy::too_many_arguments)]
fn membership(
    a: &Body,
    b: &Body,
    op: BooleanOp,
    result: &Body,
    (box_a, box_b): (&Bbox3, &Bbox3),
    tolerance: &Tolerance,
    options: &VerifyOptions,
    found: &mut Verification,
) {
    if options.points == 0 {
        return;
    }
    let (Some((a0, a1)), Some((b0, b1))) = (box_a.corners(), box_b.corners()) else {
        return;
    };
    let (hull_lo, hull_hi) = (a0.min(b0), a1.max(b1));
    let (over_lo, over_hi) = (a0.max(b0), a1.min(b1));
    let overlap = over_lo.x < over_hi.x && over_lo.y < over_hi.y && over_lo.z < over_hi.z;
    let mut seeded = Seeded(0x3dc4_af7b_00c4_ec4b);
    let points: Vec<Vec3> = (0..options.points)
        .map(|i| {
            if overlap && i % 2 == 1 {
                seeded.point_in(over_lo, over_hi)
            } else {
                seeded.point_in(hull_lo, hull_hi)
            }
        })
        .collect();
    // A body the ray casting does not take (one it refuses, or one that
    // is not valid) tells nothing here; the reason is kept.
    let mut untold = None;
    let mut classified = |body: &Body, which: &str| match contains_points(body, &points, tolerance)
    {
        Ok(inside) => Some(inside),
        Err(e) => {
            untold.get_or_insert_with(|| format!("the {which}: {e}"));
            None
        }
    };
    let told = (
        classified(a, "target"),
        classified(b, "tool"),
        classified(result, "result"),
    );
    found.unclassified = untold;
    let (Some(in_a), Some(in_b), Some(in_r)) = told else {
        return;
    };
    for (((&at, a), b), r) in points.iter().zip(in_a).zip(in_b).zip(in_r) {
        let (Some(in_a), Some(in_b), Some(in_result)) = (a, b, r) else {
            continue;
        };
        found.points += 1;
        let expected = match op {
            BooleanOp::Union => in_a || in_b,
            BooleanOp::Subtract => in_a && !in_b,
            BooleanOp::Intersect => in_a && in_b,
        };
        if in_result != expected {
            found.violations.push(Violation::Membership {
                at,
                in_a,
                in_b,
                in_result,
            });
        }
    }
}
