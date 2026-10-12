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
//! plane exactly count, which no approximation brings about: both faces
//! were then cut from the same geometry at the same places.
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

use crate::intersections::{Crossing, CrossingKind, Rules, contacts, raw_cap};
use crate::{Mesh, TessError, TessParams, tessellate_body_with_threads};
use craft3d_geom::{Curve3, Surface};
use craft3d_math::Vec3;
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
}

impl SelfCheck {
    /// No finding that is certain. Findings within the margin may remain.
    pub fn is_clean(&self) -> bool {
        self.certain_crossings == 0 && !self.shells.iter().any(|s| s.certain)
    }

    /// Nothing at all, not even within the margin.
    pub fn is_spotless(&self) -> bool {
        self.certain_crossings == 0 && self.marginal_crossings == 0 && self.shells.is_empty()
    }
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
    2.0 * numerator.atan2(denominator)
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
}

/// Where each shell lies relative to the others.
fn shell_findings(
    body: &Body,
    mesh: &Mesh,
    shell_of: &[Option<u32>],
    margin: f64,
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
        })
        .collect();
    for (t, face) in mesh.face_ids.iter().enumerate() {
        let shell = shell_of.get(*face as usize).copied().flatten();
        let (Some(shell), Some(corners)) = (shell, mesh.corners(t)) else {
            continue;
        };
        if let Some(target) = shells.get_mut(shell as usize) {
            let [a, b, c] = corners.map(|p| p - about);
            target.volume += a.dot(b.cross(c));
            target.corners.push(corners);
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
            let mut usable = true;
            for (o, other) in shells.iter().enumerate() {
                if o == s || other.corners.is_empty() {
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
    let floor = 4.0 * tolerance;
    let mut chord = (params.relative_chord * size).max(floor);
    // A face can be too narrow for the first mesh: the polygon of a curved
    // edge then crosses a neighbouring edge that the curve itself only
    // comes near, and the tessellator refuses the face. A finer polygon
    // does not cross; that is not a finding about the body.
    let mut retries = 0_u32;
    let (mut check, mut unsettled) = loop {
        match one_pass(body, params, chord) {
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
        let Ok((mut finer, still)) = one_pass(body, params, chord) else {
            break;
        };
        finer.passes = check.passes.saturating_add(1);
        (check, unsettled) = (finer, still);
    }
    Ok(check)
}

/// The check on one mesh, and whether something within the margin is
/// left that a finer mesh could settle.
fn one_pass(
    body: &Body,
    params: &SelfCheckParams,
    chord: f64,
) -> Result<(SelfCheck, bool), TessError> {
    let tolerance = params.tolerance;
    let exact = exact_faces(body);
    let shell_of = shell_of_faces(body);
    let tess = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body_with_threads(body, &tess, params.threads)?;
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
    let rules = Rules {
        margin: &margin_of,
        near: &near_of,
        reach: 2.0 * tolerance,
    };
    let found = contacts(&mesh, &rules, params.threads, raw_cap(params.limit))?;
    let shell = |face: u32| shell_of.get(face as usize).copied().flatten();
    let mut unsettled = false;
    let mut crossings: Vec<FaceCrossing> = found
        .contacts
        .iter()
        .map(|c: &Crossing| {
            let [f, g] = c.faces;
            let margin = margin_of(f, g);
            let certain = c.depth > margin.max(found.rounding);
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
    let check = SelfCheck {
        crossings,
        certain_crossings,
        marginal_crossings,
        truncated: found.truncated,
        shells: shell_findings(body, &mesh, &shell_of, margin),
        chordal_tolerance: chord,
        margin,
        passes: 1,
        triangles: mesh.triangles.len(),
        degenerate_triangles: found.degenerate,
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
