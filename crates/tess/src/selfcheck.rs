//! Does the surface of a body pass through itself?
//!
//! `craft3d_topo::check` verifies how the entities of a body hang
//! together and that they lie on their geometry. It does not notice two
//! faces that pass through each other, or two shells that overlap. This
//! does, independently of the kernel: the body is tessellated and its mesh
//! is searched (`intersections.rs`).
//!
//! # The margin
//!
//! A mesh is not the body. Every point of the mesh of a face is within
//! the chordal tolerance `c` of the face, and the face is where its
//! geometry says within the modelling tolerance `t`. So two faces that
//! only touch (tangent faces, a sphere resting on a plane, a pin in a hole
//! of its own size) can have meshes that pass through each other, by at
//! most `c + c + 2t`. A crossing of the meshes deeper than
//!
//! ```text
//! margin(f, g) = deviation(f) + deviation(g) + 2 t
//! ```
//!
//! cannot come from the approximation: the faces themselves cross. Here
//! `deviation` is `c`, except for a planar face bounded by straight edges
//! only, whose mesh is the face itself: 0. Depth is measured as described
//! in `intersections.rs`: how far either surface could be moved along its
//! own normal without undoing the crossing. A face is at most its
//! deviation and the tolerance from its mesh, along that normal; so a
//! crossing of the meshes deeper than the margin is a crossing of the
//! faces. A finding deeper than its margin is **certain**; one within it
//! is reported as such and proves nothing either way.
//!
//! Two faces lying on each other are an *overlap*, with a depth measured
//! in their plane: how far into both the common area reaches. For flat
//! faces with straight edges "on each other" is within twice the
//! modelling tolerance, the distance at which the body itself cannot tell
//! two points apart. For curved faces only triangles that are in one
//! plane exactly count; that an approximation can bring about all the
//! same (2026-10-20): a strip of a curved face between two lines in a
//! flat face's plane, bulging out of it by less than the chord, gets no
//! point inside and is meshed as that plane. So an overlap deeper than
//! the margin is certain only where the faces themselves are at the same
//! place: the two surfaces are asked at points of the common area
//! (`faces_apart`), and where they are further apart than twice the
//! tolerance the overlap is of the meshes, not of the faces, and counts
//! as within the margin. That lowers it to a doubt, no more: like every
//! finding within the margin it is looked at again on finer meshes, and
//! one still there after the last ends as marginal, which
//! `document.check` counts but does not call a problem. A real overlap of
//! two faces has an area, and on a finer mesh the pairs of triangles
//! inside it lie where the surfaces are one, and are certain again; one
//! narrower than the finest mesh's triangles stays a doubt.
//!
//! Findings within the margin between curved faces are looked at again
//! with a mesh four times finer, a few times over; what is still within
//! the margin then is contact, or an overlap too shallow to tell from it.
//!
//! # Shells
//!
//! Shells that overlap partly cross, and are found as above. Shells that
//! are apart are checked for where they lie. A shell whose mesh encloses a
//! positive volume is an outside; one with a negative volume is a cavity.
//! For a point on each shell the winding number of every *other* shell is
//! computed (the sum of the solid angles of its triangles: 1 inside an
//! outside, -1 inside a cavity, 0 elsewhere; there is no ray, so nothing
//! that can graze an edge). Their total must be 0 for an outside (it is in
//! no material: not inside another outside, unless in a cavity of it) and
//! 1 for a cavity (it is in material). The point is the centre of one of
//! the largest triangles of the shell, the first that is further than the
//! margin from every other shell, so that the side it is on is not a
//! matter of the approximation.

use crate::body::tessellate_faces;
use crate::intersections::{Crossing, CrossingKind, Rules, common_area, contacts, raw_cap};
use crate::{Mesh, TessError, TessParams, tessellate_body_with_threads};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Portable;
use craft3d_math::{Bbox3, Vec3};
use craft3d_topo::{Body, BodyKind};
use std::fmt;

/// How to look for self-intersections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfCheckParams {
    /// The modelling tolerance of the body: `Tolerance::linear`.
    pub tolerance: f64,
    /// The chordal tolerance of the first mesh, as a fraction of the
    /// diagonal of the body's bounding box. Never below four times
    /// `tolerance`.
    pub relative_chord: f64,
    /// How many times findings within the margin are looked at again with
    /// a mesh four times finer. Also how many times a finer mesh is tried
    /// when the tessellator refuses a face at the first.
    pub refinements: u32,
    /// The most crossings to report; the certain ones come first.
    pub limit: usize,
    /// Threads to use; 0 means "as many as the machine offers". The
    /// result does not depend on it.
    pub threads: usize,
}

impl SelfCheckParams {
    /// The defaults for a body with this modelling tolerance.
    pub fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            relative_chord: 1e-3,
            refinements: 2,
            limit: 64,
            threads: 0,
        }
    }
}

impl Default for SelfCheckParams {
    /// For the default modelling tolerance of 1e-6.
    fn default() -> Self {
        Self::new(1e-6)
    }
}

/// Two faces whose meshes meet where they should not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCrossing {
    /// Face indices of the body.
    pub faces: [u32; 2],
    /// The shells those faces are in.
    pub shells: [Option<u32>; 2],
    /// Triangles of the mesh this was seen in.
    pub triangles: [usize; 2],
    /// About where.
    pub at: Vec3,
    pub kind: CrossingKind,
    /// How deep; see [`Crossing::depth`].
    pub depth: f64,
    /// The depth the approximation alone could explain.
    pub margin: f64,
    /// Deeper than the margin: the faces themselves cross or overlap.
    pub certain: bool,
}

/// What is wrong with where a shell lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellDefect {
    /// A shell that faces outward lies in the material of other shells:
    /// an outside within another outside.
    OuterInsideSolid,
    /// A shell that faces inward does not lie in the material of other
    /// shells: a cavity outside every outside, or inside another cavity.
    /// Also a body that is inside out.
    CavityOutsideSolid,
    /// It could not be told: the shell encloses no volume, the mesh of
    /// another shell is not closed, or the shell is nowhere further than
    /// the margin from another.
    Undetermined,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellFinding {
    /// Index of the shell in the body.
    pub shell: u32,
    pub defect: ShellDefect,
    /// The other shells the point was inside of (winding number not zero).
    pub inside: Vec<u32>,
    /// The point on the shell that was looked at.
    pub at: Vec3,
    /// False for [`ShellDefect::Undetermined`].
    pub certain: bool,
}

/// The result of [`body_self_intersections`].
#[derive(Clone, Debug, PartialEq)]
pub struct SelfCheck {
    /// At most `limit`, the certain ones first, then in mesh order.
    pub crossings: Vec<FaceCrossing>,
    /// All crossings deeper than their margin, also those beyond `limit`.
    pub certain_crossings: usize,
    /// All crossings within their margin.
    pub marginal_crossings: usize,
    /// More triangles met than were looked at.
    pub truncated: bool,
    pub shells: Vec<ShellFinding>,
    /// The chordal tolerance of the last mesh.
    pub chordal_tolerance: f64,
    /// The margin between two curved faces at that tolerance.
    pub margin: f64,
    /// Meshes tried: one, plus the refinements that were needed.
    pub passes: u32,
    pub triangles: usize,
    /// Triangles without area, which were left out.
    pub degenerate_triangles: usize,
    /// Faces meshed: all of them, or those near what changed
    /// ([`body_self_intersections_near`]).
    pub faces: usize,
    /// Places where the body touches itself: two of its vertices, or a
    /// vertex and an edge it is not an end of, at one place, of one shell
    /// or of two shells of a solid ([`Touch::between_shells`]). Always
    /// certain. At most `limit`.
    pub touching: Vec<Touch>,
}

pub use craft3d_topo::{Touch, Touched};

impl SelfCheck {
    /// No finding that is certain. Findings within the margin may remain.
    pub fn is_clean(&self) -> bool {
        self.certain_crossings == 0
            && !self.shells.iter().any(|s| s.certain)
            && self.touching.is_empty()
    }

    /// Nothing at all, not even within the margin.
    pub fn is_spotless(&self) -> bool {
        self.certain_crossings == 0
            && self.marginal_crossings == 0
            && self.shells.is_empty()
            && self.touching.is_empty()
    }
}

/// Where the shells of `body` touch (see [`Touch`]): vertices closer
/// than `near` to another vertex or to an edge they are not an end of, of
/// the same shell (a shell that touches itself, which no closed mesh can
/// be made of) or, in a solid, of another shell of it (two parts of one
/// body that share an edge or a corner: zero thickness, refused since
/// 2026-10-15). At most `limit`, in vertex order. On the B-rep, not the
/// mesh (`craft3d_topo::touches`).
pub fn body_touches_itself(body: &Body, near: f64, limit: usize) -> Result<Vec<Touch>, TessError> {
    craft3d_topo::touches(body, near, limit, craft3d_topo::TouchScope::All).map_err(|e| match e {
        craft3d_topo::TouchError::InvalidDistance(d) => TessError::InvalidTolerance(d),
        craft3d_topo::TouchError::TooMany => TessError::TooManyVertices,
        craft3d_topo::TouchError::InvalidBody(reason) => TessError::InvalidBody(reason),
    })
}

impl fmt::Display for SelfCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} certain and {} marginal crossings, {} shell findings ({} triangles at chord {:e}, \
             margin {:e}, {} passes)",
            self.certain_crossings,
            self.marginal_crossings,
            self.shells.len(),
            self.triangles,
            self.chordal_tolerance,
            self.margin,
            self.passes
        )?;
        for c in &self.crossings {
            let [fa, fb] = c.faces;
            writeln!(
                f,
                "  {} faces {fa} and {fb} {} at ({:e}, {:e}, {:e}), depth {:e} against a margin of {:e}",
                if c.certain { "certain:" } else { "marginal:" },
                match c.kind {
                    CrossingKind::Crossing => "cross",
                    CrossingKind::CoplanarOverlap => "overlap in one plane",
                },
                c.at.x,
                c.at.y,
                c.at.z,
                c.depth,
                c.margin
            )?;
        }
        for s in &self.shells {
            writeln!(
                f,
                "  shell {}: {:?}, inside shells {:?}, seen at ({:e}, {:e}, {:e})",
                s.shell, s.defect, s.inside, s.at.x, s.at.y, s.at.z
            )?;
        }
        for t in &self.touching {
            writeln!(
                f,
                "  touching: vertex {} and {:?} at ({:e}, {:e}, {:e}), {:e} apart, shells {} and {}",
                t.vertex, t.touches, t.at.x, t.at.y, t.at.z, t.distance, t.shells[0], t.shells[1]
            )?;
        }
        Ok(())
    }
}

/// How many of the largest triangles of a shell are tried as the point to
/// look from.
const SAMPLES: usize = 8;

/// A winding number further than this from a whole number is not one.
const WINDING_SLACK: f64 = 0.25;

/// Per face: is its mesh the face itself? True for a plane bounded by
/// straight edges only.
fn exact_faces(body: &Body) -> Vec<bool> {
    body.faces
        .iter()
        .map(|face| {
            matches!(face.surface, Surface::Plane(_))
                && face.loops.iter().all(|&l| {
                    body.loop_(l).is_some_and(|l| {
                        l.coedges.iter().all(|c| {
                            body.edge(c.edge)
                                .is_some_and(|e| matches!(e.curve, Curve3::Line(_)))
                        })
                    })
                })
        })
        .collect()
}

/// Per face: the shell it is in.
fn shell_of_faces(body: &Body) -> Vec<Option<u32>> {
    let mut out = vec![None; body.faces.len()];
    for (s, shell) in body.shells.iter().enumerate() {
        for face in &shell.faces {
            if let Some(slot) = out.get_mut(face.0 as usize) {
                *slot = u32::try_from(s).ok();
            }
        }
    }
    out
}

/// The solid angle of triangle `a b c` seen from `p`, with its sign:
/// positive when the triangle faces away from `p` (van Oosterom and
/// Strackee, "The solid angle of a plane triangle", IEEE Trans. Biomed.
/// Eng. 30, 1983).
fn solid_angle(p: Vec3, [a, b, c]: [Vec3; 3]) -> f64 {
    let (a, b, c) = (a - p, b - p, c - p);
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    let numerator = a.dot(b.cross(c));
    let denominator = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
    if numerator == 0.0 && denominator == 0.0 {
        return 0.0;
    }
    2.0 * numerator.patan2(denominator)
}

/// The distance from `p` to the triangle `a b c` (C. Ericson, "Real-Time
/// Collision Detection", 5.1.5: the nearest point by the region `p`
/// projects into).
fn distance_to_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> f64 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return p.distance(a);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return p.distance(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && d1 - d3 > 0.0 {
        return p.distance(a + ab * (d1 / (d1 - d3)));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return p.distance(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && d2 - d6 > 0.0 {
        return p.distance(a + ac * (d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    let (e1, e2) = (d4 - d3, d5 - d6);
    if va <= 0.0 && e1 >= 0.0 && e2 >= 0.0 && e1 + e2 > 0.0 {
        return p.distance(b + (c - b) * (e1 / (e1 + e2)));
    }
    let total = va + vb + vc;
    if total == 0.0 || !total.is_finite() {
        // No area: the nearest corner will do.
        return p.distance(a).min(p.distance(b)).min(p.distance(c));
    }
    p.distance(a + ab * (vb / total) + ac * (vc / total))
}

/// The triangles of one shell.
struct ShellMesh {
    corners: Vec<[Vec3; 3]>,
    /// Six times the enclosed volume, about `about`.
    volume: f64,
    /// The box of its triangles.
    bounds: Bbox3,
}

/// Where each shell lies relative to the others. Shells that `left_out`
/// marks are not wholly in the mesh: nothing is said of them, and a
/// point within `margin` of one of their `boxes` (which hold them) cannot
/// be placed.
fn shell_findings(
    body: &Body,
    mesh: &Mesh,
    shell_of: &[Option<u32>],
    margin: f64,
    left_out: &[bool],
    boxes: &[Bbox3],
) -> Vec<ShellFinding> {
    let mut findings = Vec::new();
    if body.kind != BodyKind::Solid || body.shells.is_empty() {
        return findings;
    }
    let about = mesh.bbox().center().unwrap_or(Vec3::ZERO);
    let size = mesh.bbox().diagonal();
    let mut shells: Vec<ShellMesh> = body
        .shells
        .iter()
        .map(|_| ShellMesh {
            corners: Vec::new(),
            volume: 0.0,
            bounds: Bbox3::EMPTY,
        })
        .collect();
    for (t, face) in mesh.face_ids.iter().enumerate() {
        let shell = shell_of.get(*face as usize).copied().flatten();
        let (Some(shell), Some(corners)) = (shell, mesh.corners(t)) else {
            continue;
        };
        if left_out.get(shell as usize).copied().unwrap_or(false) {
            continue;
        }
        if let Some(target) = shells.get_mut(shell as usize) {
            let [a, b, c] = corners.map(|p| p - about);
            target.volume += a.dot(b.cross(c));
            target.corners.push(corners);
            for p in corners {
                target.bounds.add(p);
            }
        }
    }

    for (s, shell) in shells.iter().enumerate() {
        let Ok(id) = u32::try_from(s) else {
            continue;
        };
        if shell.corners.is_empty() {
            continue;
        }
        let undetermined = |at: Vec3| ShellFinding {
            shell: id,
            defect: ShellDefect::Undetermined,
            inside: Vec::new(),
            at,
            certain: false,
        };
        let first = shell
            .corners
            .first()
            .map_or(Vec3::ZERO, |[a, b, c]| (*a + *b + *c) * (1.0 / 3.0));
        // A volume this small is a sheet folded shut, not a shell.
        if !shell.volume.is_finite() || shell.volume.abs() <= 1e-12 * size * size * size {
            findings.push(undetermined(first));
            continue;
        }
        let outward = shell.volume > 0.0;

        // The largest triangles first; the index breaks ties.
        let mut by_area: Vec<(f64, usize)> = shell
            .corners
            .iter()
            .enumerate()
            .map(|(k, [a, b, c])| ((*b - *a).cross(*c - *a).length_squared(), k))
            .collect();
        by_area.sort_by(|p, q| q.0.total_cmp(&p.0).then(p.1.cmp(&q.1)));

        let mut verdict = None;
        for &(_, k) in by_area.iter().take(SAMPLES) {
            let Some([a, b, c]) = shell.corners.get(k) else {
                continue;
            };
            let at = (*a + *b + *c) * (1.0 / 3.0);
            let mut total = 0_i64;
            let mut inside = Vec::new();
            // Near a shell that is not in the mesh, nothing can be said.
            let mut usable = !boxes.iter().any(|b| b.contains(at, margin));
            for (o, other) in shells.iter().enumerate() {
                if !usable {
                    break;
                }
                if o == s || other.corners.is_empty() {
                    continue;
                }
                // Further than the margin outside the box of a closed
                // shell: no part of it near, and its winding number 0.
                if !other.bounds.contains(at, margin) {
                    continue;
                }
                let mut angle = 0.0;
                let mut nearest = f64::INFINITY;
                for corners in &other.corners {
                    angle += solid_angle(at, *corners);
                    nearest = nearest.min(distance_to_triangle(at, *corners));
                }
                let winding = angle / (4.0 * std::f64::consts::PI);
                let whole = winding.round();
                // Too near to tell the side, or not a closed mesh.
                if nearest <= margin
                    || !whole.is_finite()
                    || (winding - whole).abs() > WINDING_SLACK
                {
                    usable = false;
                    break;
                }
                // A small whole number by the check above.
                let whole = whole as i64;
                total += whole;
                if whole != 0
                    && let Ok(o) = u32::try_from(o)
                {
                    inside.push(o);
                }
            }
            if usable {
                verdict = Some((at, total, inside));
                break;
            }
        }
        let Some((at, total, inside)) = verdict else {
            findings.push(undetermined(first));
            continue;
        };
        let defect = match (outward, total) {
            (true, 0) | (false, 1) => continue,
            (true, _) => ShellDefect::OuterInsideSolid,
            (false, _) => ShellDefect::CavityOutsideSolid,
        };
        findings.push(ShellFinding {
            shell: id,
            defect,
            inside,
            at,
            certain: true,
        });
    }
    findings
}

/// Tessellates `body` at a chordal tolerance derived from its size and
/// `params`, and reports where its surface passes through itself:
/// crossings and overlaps between faces (which includes shells that
/// overlap partly), and shells that lie inside each other the wrong way
/// round (an outside within another outside; a cavity outside every
/// outside). Every finding says whether it is certain or within the
/// margin the approximation allows; see [`SelfCheck::is_clean`].
///
/// The body should pass `craft3d_topo::check`: this looks at what that
/// does not. An error when the body cannot be tessellated, when it has no
/// extent, or for parameters that are negative or not finite.
pub fn body_self_intersections(
    body: &Body,
    params: &SelfCheckParams,
) -> Result<SelfCheck, TessError> {
    let chord = first_chord(body, params)?;
    let mut check = refined(body, params, chord, None)?;
    check.touching = body_touches_itself(body, 2.0 * params.tolerance, params.limit)?;
    Ok(check)
}

/// What a face is to [`body_self_intersections_near`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceRole {
    /// A face, or a piece of one, of a body that is known not to pass
    /// through itself: the faces of one group are not looked at against
    /// each other (nor a face against itself). The operands of a boolean
    /// are two groups; the instances of a pattern one each.
    Known(u32),
    /// A face an operation made or changed in a way that is not a piece
    /// of a known face: looked at against every face near it, itself
    /// included.
    New,
}

/// [`body_self_intersections`] of what an operation changed: `roles`
/// (one per face) says which faces are [`FaceRole::New`] and which
/// [`FaceRole::Known`], in which group.
///
/// Only pairs that can meet are looked at, and only the faces they can be
/// made of are meshed: a face is in when its box (`Body::face_bbox`,
/// grown by the first chordal tolerance and the modelling tolerance, so
/// that their meshes are inside) meets the box of a new face, or the
/// common part of the boxes of two groups. Pairs of faces of one group
/// are left out. Where the body has several shells, a shell whose box
/// meets one of those is meshed whole, so that where it lies can be
/// told; a shell that is not meshed whole has no shell finding, and a
/// point of another shell in its box cannot be placed (`Undetermined`).
/// The chordal tolerance is that of the whole body, so a finding is what
/// [`body_self_intersections`] finds for those faces; with every face new
/// the two are the same.
///
/// Errors as [`body_self_intersections`], and for `roles` of another
/// length than the faces.
pub fn body_self_intersections_near(
    body: &Body,
    params: &SelfCheckParams,
    roles: &[FaceRole],
) -> Result<SelfCheck, TessError> {
    if roles.len() != body.faces.len() {
        return Err(TessError::InvalidBody(format!(
            "{} roles for {} faces",
            roles.len(),
            body.faces.len()
        )));
    }
    let chord = first_chord(body, params)?;
    let scope = Scope::new(body, roles, chord, params.tolerance)?;
    let mut check = refined(body, params, chord, Some(&scope))?;
    check.touching = body_touches_itself(body, 2.0 * params.tolerance, params.limit)?;
    Ok(check)
}

/// The chordal tolerance of the first mesh.
fn first_chord(body: &Body, params: &SelfCheckParams) -> Result<f64, TessError> {
    let tolerance = params.tolerance;
    if !(tolerance.is_finite() && tolerance >= 0.0) {
        return Err(TessError::InvalidTolerance(tolerance));
    }
    if !(params.relative_chord.is_finite() && params.relative_chord > 0.0) {
        return Err(TessError::InvalidTolerance(params.relative_chord));
    }
    let size = body
        .bbox()
        .map_err(|e| TessError::InvalidBody(format!("the body has no bounding box: {e}")))?
        .diagonal();
    if !(size.is_finite() && size > 0.0) {
        return Err(TessError::InvalidBody("the body has no extent".into()));
    }
    Ok((params.relative_chord * size).max(4.0 * tolerance))
}

/// The work one check may do ([`craft3d_geom::allowance`] units, about a
/// microsecond each), all its passes together. A face that cannot be meshed
/// is refined round after round, and the check tried again finer: on faces
/// of a few hundred spans that went on for a minute and more before it
/// gave up (fuzzing, 2026-10-17). Past the bound the check ends with
/// [`TessError::Spent`], which a caller treats as a body it could not look
/// at.
pub const SELF_CHECK_WORK: u64 = 20_000_000;

/// The passes of the check: a first mesh at `chord`, finer ones where a
/// face cannot be meshed or something within the margin is left; bounded
/// by [`SELF_CHECK_WORK`].
fn refined(
    body: &Body,
    params: &SelfCheckParams,
    chord: f64,
    scope: Option<&Scope<'_>>,
) -> Result<SelfCheck, TessError> {
    craft3d_geom::allowance::within(SELF_CHECK_WORK, || {
        refined_within(body, params, chord, scope)
    })
}

fn refined_within(
    body: &Body,
    params: &SelfCheckParams,
    mut chord: f64,
    scope: Option<&Scope<'_>>,
) -> Result<SelfCheck, TessError> {
    let floor = 4.0 * params.tolerance;
    if scope.is_some_and(Scope::is_empty) {
        return Ok(SelfCheck {
            crossings: Vec::new(),
            certain_crossings: 0,
            marginal_crossings: 0,
            truncated: false,
            shells: Vec::new(),
            chordal_tolerance: chord,
            margin: 2.0 * chord + 2.0 * params.tolerance,
            passes: 0,
            triangles: 0,
            degenerate_triangles: 0,
            faces: 0,
            touching: Vec::new(),
        });
    }
    // A face can be too narrow for the first mesh: the polygon of a curved
    // edge then crosses a neighbouring edge that the curve itself only
    // comes near, and the tessellator refuses the face. A finer polygon
    // does not cross; that is not a finding about the body.
    let mut retries = 0_u32;
    let (mut check, mut unsettled) = loop {
        match one_pass(body, params, chord, scope) {
            Ok(found) => break found,
            Err(TessError::Face { .. })
                if retries < params.refinements && chord / 4.0 >= floor && chord / 4.0 > 0.0 =>
            {
                retries += 1;
                chord /= 4.0;
            }
            Err(error) => return Err(error),
        }
    };
    check.passes = retries.saturating_add(1);
    while unsettled && check.passes <= params.refinements {
        chord /= 4.0;
        if chord < floor || chord.is_nan() || chord <= 0.0 {
            break;
        }
        // A mesh too fine to make is not an error: the last one stands.
        let Ok((mut finer, still)) = one_pass(body, params, chord, scope) else {
            break;
        };
        finer.passes = check.passes.saturating_add(1);
        (check, unsettled) = (finer, still);
    }
    Ok(check)
}

/// A box with `lo <= hi` on every axis.
#[derive(Clone, Copy, Debug)]
struct Span {
    lo: Vec3,
    hi: Vec3,
}

impl Span {
    /// `bbox` grown by `pad` on every side; `None` for an empty box.
    fn of(bbox: &Bbox3, pad: f64) -> Option<Span> {
        let (lo, hi) = bbox.corners()?;
        let pad = Vec3::new(pad, pad, pad);
        Some(Span {
            lo: lo - pad,
            hi: hi + pad,
        })
    }

    fn meets(&self, other: &Span) -> bool {
        self.lo.x <= other.hi.x
            && other.lo.x <= self.hi.x
            && self.lo.y <= other.hi.y
            && other.lo.y <= self.hi.y
            && self.lo.z <= other.hi.z
            && other.lo.z <= self.hi.z
    }

    fn common(&self, other: &Span) -> Option<Span> {
        self.meets(other).then(|| Span {
            lo: self.lo.max(other.lo),
            hi: self.hi.min(other.hi),
        })
    }

    fn join(&self, other: &Span) -> Span {
        Span {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    fn bbox(&self) -> Bbox3 {
        Bbox3::from_points([self.lo, self.hi])
    }

    fn middle(&self, axis: usize) -> f64 {
        let (lo, hi) = match axis {
            0 => (self.lo.x, self.hi.x),
            1 => (self.lo.y, self.hi.y),
            _ => (self.lo.z, self.hi.z),
        };
        0.5 * (lo + hi)
    }
}

/// Most boxes in a leaf of a [`SpanTree`].
const SPAN_LEAF: usize = 8;

/// Boxes, for asking whether a box meets any of them in about the
/// logarithm of their number: a tree of boxes around groups of them, each
/// group halved at its middle along the longest side of its box. The
/// answer is the one asking every box gives; a mesh of a pattern of ten
/// thousand pieces asks it of every triangle, and asking every box took
/// minutes (fuzzing, 2026-10-17).
struct SpanTree {
    items: Vec<Span>,
    /// The box around `items[start..end]`, and the two halves when it is
    /// not a leaf. The root is the first.
    nodes: Vec<SpanNode>,
}

struct SpanNode {
    span: Span,
    start: usize,
    end: usize,
    halves: Option<(usize, usize)>,
}

impl SpanTree {
    fn new(mut items: Vec<Span>) -> Self {
        let around = |items: &[Span]| -> Option<Span> {
            let (first, rest) = items.split_first()?;
            Some(rest.iter().fold(*first, |all, b| all.join(b)))
        };
        let mut nodes = Vec::new();
        let Some(root) = around(&items) else {
            return Self { items, nodes };
        };
        nodes.push(SpanNode {
            span: root,
            start: 0,
            end: items.len(),
            halves: None,
        });
        // Each node is split once and its halves are smaller: this ends.
        let mut todo = vec![0_usize];
        while let Some(at) = todo.pop() {
            let Some(&SpanNode {
                span, start, end, ..
            }) = nodes.get(at)
            else {
                continue;
            };
            if end.saturating_sub(start) <= SPAN_LEAF {
                continue;
            }
            let size = span.hi - span.lo;
            let axis = if size.x >= size.y && size.x >= size.z {
                0
            } else if size.y >= size.z {
                1
            } else {
                2
            };
            let Some(group) = items.get_mut(start..end) else {
                continue;
            };
            group.sort_by(|a, b| a.middle(axis).total_cmp(&b.middle(axis)));
            let mid = start + (end - start) / 2;
            let (Some(left), Some(right)) = (
                items.get(start..mid).and_then(around),
                items.get(mid..end).and_then(around),
            ) else {
                continue;
            };
            let first = nodes.len();
            nodes.push(SpanNode {
                span: left,
                start,
                end: mid,
                halves: None,
            });
            nodes.push(SpanNode {
                span: right,
                start: mid,
                end,
                halves: None,
            });
            if let Some(node) = nodes.get_mut(at) {
                node.halves = Some((first, first + 1));
            }
            todo.push(first);
            todo.push(first + 1);
        }
        Self { items, nodes }
    }

    /// Whether `span` meets any of the boxes.
    fn meets(&self, span: &Span) -> bool {
        let mut todo = Vec::with_capacity(64);
        if !self.nodes.is_empty() {
            todo.push(0_usize);
        }
        while let Some(at) = todo.pop() {
            let Some(node) = self.nodes.get(at) else {
                continue;
            };
            if !node.span.meets(span) {
                continue;
            }
            match node.halves {
                Some((left, right)) => {
                    todo.push(left);
                    todo.push(right);
                }
                None => {
                    let group = self.items.get(node.start..node.end).unwrap_or_default();
                    if group.iter().any(|b| b.meets(span)) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Which faces [`body_self_intersections_near`] meshes and which pairs
/// it looks at.
struct Scope<'a> {
    roles: &'a [FaceRole],
    /// Per face: meshed.
    faces: Vec<bool>,
    /// Per shell: not meshed whole.
    left_out: Vec<bool>,
    /// The boxes of those shells.
    left_out_boxes: Vec<Bbox3>,
    /// Where pairs that are looked at can meet, grown by how far the
    /// depth of a crossing is followed.
    reach: SpanTree,
    /// The body with the holes of flat faces that are far from all of
    /// that filled, when there were such holes.
    trimmed: Option<Body>,
}

/// How far around where pairs can meet a mesh matters, in chordal
/// tolerances and modelling tolerances: the depth of a crossing is
/// followed until four times the margin (`intersections.rs`), and a
/// margin is two of each.
const REACH: f64 = 10.0;

impl<'a> Scope<'a> {
    /// The faces near what changed, for a first mesh at `chord` and the
    /// modelling tolerance `tolerance`.
    fn new(
        body: &Body,
        roles: &'a [FaceRole],
        chord: f64,
        tolerance: f64,
    ) -> Result<Self, TessError> {
        // Boxes are grown by this, so that what is in them holds the
        // meshes of what they hold.
        let pad = chord + tolerance;
        let mut boxes: Vec<Option<Span>> = Vec::with_capacity(body.faces.len());
        for face in 0..body.faces.len() {
            let id = u32::try_from(face).map_err(|_| TessError::TooManyVertices)?;
            let bbox = body
                .face_bbox(craft3d_topo::FaceId(id))
                .map_err(|e| TessError::InvalidBody(format!("face {face} has no box: {e}")))?;
            boxes.push(Span::of(&bbox, pad));
        }

        // Where something can meet something it is not known to keep
        // clear of: around every new face, and where the boxes of two
        // groups overlap.
        let mut zone: Vec<Span> = Vec::new();
        let mut groups: std::collections::BTreeMap<u32, Span> = std::collections::BTreeMap::new();
        for (role, bbox) in roles.iter().zip(&boxes) {
            let Some(bbox) = bbox else { continue };
            match role {
                FaceRole::New => zone.push(*bbox),
                FaceRole::Known(group) => {
                    groups
                        .entry(*group)
                        .and_modify(|all| *all = all.join(bbox))
                        .or_insert(*bbox);
                }
            }
        }
        // Sweep along x: each group against the ones that start before
        // it ends.
        let mut by_start: Vec<Span> = groups.into_values().collect();
        by_start.sort_by(|a, b| a.lo.x.total_cmp(&b.lo.x));
        for (i, a) in by_start.iter().enumerate() {
            for b in by_start.iter().skip(i + 1) {
                if b.lo.x > a.hi.x {
                    break;
                }
                if let Some(common) = a.common(b) {
                    zone.push(common);
                }
            }
        }
        let zones = SpanTree::new(zone.clone());
        let in_zone = |bbox: &Span| zones.meets(bbox);
        let mut faces: Vec<bool> = boxes
            .iter()
            .map(|bbox| bbox.as_ref().is_some_and(in_zone))
            .collect();

        // Where shells lie can be told only of shells meshed whole.
        let mut left_out = vec![false; body.shells.len()];
        let mut left_out_boxes = Vec::new();
        if body.shells.len() > 1 {
            for (s, shell) in body.shells.iter().enumerate() {
                let mut bbox: Option<Span> = None;
                for face in &shell.faces {
                    if let Some(Some(b)) = boxes.get(face.0 as usize) {
                        bbox = Some(bbox.map_or(*b, |all| all.join(b)));
                    }
                }
                let Some(bbox) = bbox else { continue };
                if in_zone(&bbox) {
                    for face in &shell.faces {
                        if let Some(slot) = faces.get_mut(face.0 as usize) {
                            *slot = true;
                        }
                    }
                } else if let Some(slot) = left_out.get_mut(s) {
                    *slot = true;
                    left_out_boxes.push(bbox.bbox());
                }
            }
        } else if let Some(shell) = body.shells.first()
            && !shell
                .faces
                .iter()
                .all(|f| faces.get(f.0 as usize).copied().unwrap_or(false))
            && let Some(slot) = left_out.get_mut(0)
        {
            *slot = true;
        }
        let far = REACH * (chord + tolerance);
        let reach: Vec<Span> = zone
            .iter()
            .map(|z| Span {
                lo: z.lo - Vec3::new(far, far, far),
                hi: z.hi + Vec3::new(far, far, far),
            })
            .collect();
        let reach = SpanTree::new(reach);
        let trimmed = trim_far_holes(body, &faces, &left_out, &reach)?;
        Ok(Self {
            roles,
            faces,
            left_out,
            left_out_boxes,
            reach,
            trimmed,
        })
    }

    /// Whether a triangle with corners `corners` can take part in a pair
    /// that is looked at, or in measuring how deep one is.
    fn reaches(&self, corners: &[Vec3; 3]) -> bool {
        let [a, b, c] = *corners;
        let span = Span {
            lo: a.min(b).min(c),
            hi: a.max(b).max(c),
        };
        self.reach.meets(&span)
    }

    fn is_empty(&self) -> bool {
        !self.faces.iter().any(|&f| f)
    }

    /// Whether triangles of faces `f` and `g` are looked at.
    fn pair(&self, f: u32, g: u32) -> bool {
        let role = |face: u32| self.roles.get(face as usize).copied();
        match (role(f), role(g)) {
            (Some(FaceRole::Known(a)), Some(FaceRole::Known(b))) => a != b,
            (Some(_), Some(_)) => true,
            _ => false,
        }
    }
}

/// `body` with the holes of its flat faces that lie wholly outside `reach`
/// filled, on the faces `faces` marks in shells `left_out` marks (whose
/// meshes are not looked at as wholes); `None` when there are none.
///
/// A hole far from where pairs can meet changes nothing there: the face
/// with it filled is the same face near every place that is looked at,
/// and where it differs (in the hole) nothing of another group and no
/// new face can be (it would be in `reach`). A long plate with a
/// thousand holes is then meshed with the few near a new one.
fn trim_far_holes(
    body: &Body,
    faces: &[bool],
    left_out: &[bool],
    reach: &SpanTree,
) -> Result<Option<Body>, TessError> {
    let shell_of = shell_of_faces(body);
    let mut trimmed: Option<Body> = None;
    for (index, face) in body.faces.iter().enumerate() {
        let wanted = faces.get(index).copied().unwrap_or(false);
        let loose = shell_of
            .get(index)
            .copied()
            .flatten()
            .is_some_and(|s| left_out.get(s as usize).copied().unwrap_or(false));
        if !(wanted && loose && face.loops.len() > 1 && matches!(face.surface, Surface::Plane(_))) {
            continue;
        }
        let mut boxes = Vec::with_capacity(face.loops.len());
        for &id in &face.loops {
            let bbox = body
                .loop_bbox(id)
                .map_err(|e| TessError::InvalidBody(format!("a loop has no box: {e}")))?;
            boxes.push(Span::of(&bbox, 0.0));
        }
        // On a plane the outer loop holds the others: its box is the
        // largest.
        let size = |b: &Option<Span>| b.map_or(0.0, |b| b.lo.distance(b.hi));
        let mut outer = 0;
        for (k, b) in boxes.iter().enumerate() {
            if size(b) > boxes.get(outer).map_or(0.0, size) {
                outer = k;
            }
        }
        let keep: Vec<bool> = boxes
            .iter()
            .enumerate()
            .map(|(k, b)| k == outer || b.is_none_or(|b| reach.meets(&b)))
            .collect();
        if keep.iter().all(|&k| k) {
            continue;
        }
        let target = trimmed.get_or_insert_with(|| body.clone());
        if let Some(slot) = target.faces.get_mut(index) {
            slot.loops = face
                .loops
                .iter()
                .zip(&keep)
                .filter(|(_, k)| **k)
                .map(|(id, _)| *id)
                .collect();
        }
    }
    Ok(trimmed)
}

/// Whether the surfaces of the two faces of `c`, an overlap of two
/// triangles of `mesh` in one plane, are further apart than `near`
/// somewhere in the triangles' common area: at its centroid, and halfway
/// from there to each of its corners. Each face lies within the
/// tolerance of its surface, so where the surfaces are more than twice
/// the tolerance apart no point is on both faces, whatever their meshes
/// do. A surface that cannot be asked (a projection that fails) is taken
/// to be at the place: the finding then stands, never one made up.
fn faces_apart(body: &Body, mesh: &Mesh, c: &Crossing, near: f64) -> bool {
    let [i, j] = c.triangles;
    let (Some(a), Some(b)) = (mesh.corners(i), mesh.corners(j)) else {
        return false;
    };
    let common = common_area(&a, &b);
    let Some(count) = u32::try_from(common.len()).ok().filter(|&n| n >= 3) else {
        return false;
    };
    let centroid = common.iter().fold(Vec3::ZERO, |sum, &p| sum + p) * (1.0 / f64::from(count));
    let surface = |face: u32| body.faces.get(face as usize).map(|f| &f.surface);
    let [face_f, face_g] = c.faces;
    let (Some(sf), Some(sg)) = (surface(face_f), surface(face_g)) else {
        return false;
    };
    let foot = |s: &Surface, p: Vec3| s.project(p).map(|(u, v)| s.eval(u, v));
    std::iter::once(centroid)
        .chain(common.iter().map(|&p| p.lerp(centroid, 0.5)))
        .any(|p| match (foot(sf, p), foot(sg, p)) {
            (Ok(qf), Ok(qg)) => qf.distance(qg) > near,
            _ => false,
        })
}

/// The check on one mesh, and whether something within the margin is
/// left that a finer mesh could settle.
fn one_pass(
    body: &Body,
    params: &SelfCheckParams,
    chord: f64,
    scope: Option<&Scope<'_>>,
) -> Result<(SelfCheck, bool), TessError> {
    let tolerance = params.tolerance;
    let exact = exact_faces(body);
    let shell_of = shell_of_faces(body);
    let tess = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = match scope {
        None => tessellate_body_with_threads(body, &tess, params.threads)?,
        Some(scope) => tessellate_faces(
            scope.trimmed.as_ref().unwrap_or(body),
            &tess,
            params.threads,
            &scope.faces,
        )?,
    };
    // Near what changed, only the triangles that can matter there are
    // searched; the whole mesh still says where shells lie.
    let near_only = match scope {
        None => None,
        Some(scope) => {
            let mut near = Mesh {
                positions: mesh.positions.clone(),
                normals: Vec::new(),
                triangles: Vec::new(),
                face_ids: Vec::new(),
            };
            for (t, (tri, face)) in mesh.triangles.iter().zip(&mesh.face_ids).enumerate() {
                if mesh.corners(t).is_some_and(|c| scope.reaches(&c)) {
                    near.triangles.push(*tri);
                    near.face_ids.push(*face);
                }
            }
            Some(near)
        }
    };
    let near_only = near_only.as_ref().unwrap_or(&mesh);
    let deviation = |face: u32| {
        if exact.get(face as usize).copied().unwrap_or(false) {
            0.0
        } else {
            chord
        }
    };
    let margin_of = |f: u32, g: u32| deviation(f) + deviation(g) + 2.0 * tolerance;
    // Two flat faces within twice the modelling tolerance of each other
    // are in one plane as far as the body is concerned.
    let near_of = |f: u32, g: u32| {
        if deviation(f) + deviation(g) > 0.0 {
            0.0
        } else {
            2.0 * tolerance
        }
    };
    let pair = |f: u32, g: u32| scope.is_none_or(|scope| scope.pair(f, g));
    let rules = Rules {
        margin: &margin_of,
        near: &near_of,
        reach: 2.0 * tolerance,
        looked_at: scope.map(|_| &pair as &(dyn Fn(u32, u32) -> bool + Sync)),
    };
    let found = contacts(near_only, &rules, params.threads, raw_cap(params.limit))?;
    let shell = |face: u32| shell_of.get(face as usize).copied().flatten();
    let mut unsettled = false;
    let mut crossings: Vec<FaceCrossing> = found
        .contacts
        .iter()
        .map(|c: &Crossing| {
            let [f, g] = c.faces;
            let margin = margin_of(f, g);
            let mut certain = c.depth > margin.max(found.rounding);
            // An overlap of meshes in one plane is an overlap of the faces
            // only where their surfaces are at the same place (module
            // documentation); the mesh of a flat face with straight edges
            // is the face, so only a curved face can be elsewhere.
            if certain
                && c.kind == CrossingKind::CoplanarOverlap
                && deviation(f) + deviation(g) > 0.0
                && faces_apart(body, near_only, c, 2.0 * tolerance)
            {
                certain = false;
            }
            unsettled |= !certain && deviation(f) + deviation(g) > 0.0;
            FaceCrossing {
                faces: c.faces,
                shells: [shell(f), shell(g)],
                triangles: c.triangles,
                at: c.at,
                kind: c.kind,
                depth: c.depth,
                margin,
                certain,
            }
        })
        .collect();
    let certain_crossings = crossings.iter().filter(|c| c.certain).count();
    let marginal_crossings = crossings.len() - certain_crossings;
    // Stable: the certain ones first, each group in mesh order.
    crossings.sort_by_key(|c| !c.certain);
    crossings.truncate(params.limit);
    let margin = 2.0 * chord + 2.0 * tolerance;
    let (left_out, left_out_boxes): (&[bool], &[Bbox3]) = match scope {
        None => (&[], &[]),
        Some(scope) => (&scope.left_out, &scope.left_out_boxes),
    };
    let check = SelfCheck {
        crossings,
        certain_crossings,
        marginal_crossings,
        truncated: found.truncated,
        shells: shell_findings(body, &mesh, &shell_of, margin, left_out, left_out_boxes),
        chordal_tolerance: chord,
        margin,
        passes: 1,
        triangles: mesh.triangles.len(),
        degenerate_triangles: found.degenerate,
        touching: Vec::new(),
        faces: scope.map_or(body.faces.len(), |scope| {
            scope.faces.iter().filter(|&&f| f).count()
        }),
    };
    Ok((check, unsettled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_angles_add_up_to_the_winding_number() {
        // A tetrahedron facing outward.
        let p = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let faces = [[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]];
        let winding = |at: Vec3| {
            faces
                .iter()
                .map(|f| solid_angle(at, f.map(|k| p[k])))
                .sum::<f64>()
                / (4.0 * std::f64::consts::PI)
        };
        assert!((winding(Vec3::new(0.1, 0.1, 0.1)) - 1.0).abs() < 1e-12);
        assert!(winding(Vec3::new(1.0, 1.0, 1.0)).abs() < 1e-12);
        assert!(winding(Vec3::new(-5.0, 0.2, 0.2)).abs() < 1e-12);
        // Just inside a face, an edge and a corner.
        assert!((winding(Vec3::new(1e-9, 0.3, 0.3)) - 1.0).abs() < 1e-6);
        assert!((winding(Vec3::new(1e-9, 1e-9, 0.3)) - 1.0).abs() < 1e-6);
        assert!((winding(Vec3::new(1e-9, 1e-9, 1e-9)) - 1.0).abs() < 1e-6);
        assert!(winding(Vec3::new(-1e-9, 0.3, 0.3)).abs() < 1e-6);
    }

    #[test]
    fn distances_to_a_triangle() {
        let t = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(0.0, 4.0, 0.0),
        ];
        let d = |x: f64, y: f64, z: f64| distance_to_triangle(Vec3::new(x, y, z), t);
        assert!((d(1.0, 1.0, 3.0) - 3.0).abs() < 1e-12, "over the inside");
        assert!((d(2.0, -2.0, 0.0) - 2.0).abs() < 1e-12, "beside an edge");
        assert!((d(-3.0, -4.0, 0.0) - 5.0).abs() < 1e-12, "beyond a corner");
        assert!((d(6.0, -1.0, 0.0) - 5.0_f64.sqrt()).abs() < 1e-12);
        assert!(
            (d(4.0, 4.0, 0.0) - 8.0_f64.sqrt()).abs() < 1e-12,
            "the long edge"
        );
        assert!((d(-1.0, 6.0, 2.0) - 3.0).abs() < 1e-12);
        assert_eq!(d(1.0, 1.0, 0.0), 0.0);
    }
}
