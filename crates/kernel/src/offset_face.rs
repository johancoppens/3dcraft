//! Offset face: faces of a solid moved along their normals, with the
//! faces next to them following so that the body stays closed.
//!
//! A flat face moves to the parallel plane at the distance; a cylinder, a
//! sphere and a torus's tube grow or shrink by it, and a cone keeps its
//! angle and moves its apex along its axis; a freeform face becomes a
//! spline surface in the same parameters, held to a quarter of the
//! tolerance of its exact offset at the middles and quarters of its
//! sample steps (measured there, not bounded between), once the exact
//! offset is proven not to fold ([`craft3d_geom::offset`]); continued
//! past its borders where the face grows past them, and turned, by no
//! more than another quarter of the tolerance, along borders where it is
//! tangent to a neighbour ([`crate::offset_normal`]). Positive is outward, away
//! from the material: a boss grows and a hole narrows; negative is into
//! the material. The neighbours are not moved: their boundaries are, to
//! where their surfaces meet the moved one ([`crate::tweak`], which says
//! what is built and how a wrong solid is kept out).
//!
//! **Method: a local operation, not a tool and the boolean.** Inward, the
//! body cut by the half-space behind a moved plane would be the answer;
//! outward, the new material is bounded by the neighbours' continued
//! surfaces, which no tool that is easy to make has as its faces, and
//! which is the same computation as closing the body directly. So the
//! body is closed directly in both directions: one route, the faces stay
//! the faces they were (a block whose top moves has six faces, not
//! seven), and every entity keeps its number, which is what its name
//! follows (docs/decisions.md, 14 October 2026).
//!
//! **Too far.** A distance at which an edge shrinks to nothing, a face is
//! turned inside out, a hole leaves its face or the faces at a vertex no
//! longer meet is refused with the entity and the largest distance that
//! fits, found by halving. When no distance at all can be made, the
//! geometry is one that is not built (a neighbour whose continuation does
//! not meet the moved face in a line or a circle), and that is said
//! instead.

use crate::KernelError;
use crate::offset_normal::Turn;
use crate::tweak::{self, TweakError};
use craft3d_geom::offset::OffsetError;
use craft3d_geom::{Cone, Cylinder, NurbsSurface, Plane, Sphere, Surface, Torus};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, BodyKind, EdgeId, Face, FaceId, VertexId};
use std::collections::{BTreeMap, BTreeSet};

const OPERATION: &str = "offset face";

/// Most faces one offset takes.
pub const MAX_FACES: usize = 4096;

/// One face to move and how far along its outward normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceOffset {
    pub face: FaceId,
    pub distance: f64,
}

/// A body with faces moved and where each of its entities came from:
/// `faces[i]` is the face of the body given that `FaceId(i)` of `body`
/// is, and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct Offset {
    pub body: Body,
    pub faces: Vec<FaceId>,
    pub edges: Vec<Option<EdgeId>>,
    pub vertices: Vec<Option<VertexId>>,
}

/// Why a surface has no offset.
pub(crate) enum NoOffset {
    /// The offset is nothing: a radius that reaches zero.
    Gone,
    /// A freeform face's offset folds over, or cannot be shown not to:
    /// the distance reaches a radius of curvature on that side.
    Folds,
    /// A freeform face's normal is not defined everywhere.
    NoNormal,
    /// A freeform face's offset could not be approximated.
    NoFit,
    /// Refused by the offset with this reason.
    Refused(&'static str),
}

/// How far a freeform face's offset may be from the exact one, as a share
/// of the tolerance (as the rolling-ball blends are held).
const OFFSET_SHARE: f64 = 0.25;

pub(crate) const FOLDS: &str = "the offset of the freeform face would fold over: the distance reaches a radius of curvature of the face on the side it moves to (or comes so near one that it cannot be shown not to)";
pub(crate) const NO_NORMAL: &str = "the freeform face has a pole or a border or corner drawn into a point, where its normal and so its offset are not defined";
/// The work every face operation may do ([`craft3d_geom::allowance`]
/// units, about half a microsecond each in release), its tries at the
/// whole distance and its halving for the largest that fits together;
/// [`face_op_work`] adds what the faces moved and the body classified
/// against call for. The dearest face operations of the action matrix's
/// rotor: a start cap of a blade moved 0.17 mm, 82 million units on the
/// first blade and 275 and 293 million on the other two (the same
/// geometry turned: the classification casts its rays along fixed axes
/// and splits the pieces of the blades' surfaces where they lie at a
/// slant, six times as many there); the bound is half as much again as
/// the dearest, so that every copy gets the same verdict. A search on
/// blend faces of a few hundred spans ran for minutes before it ended
/// (fuzzing, 2026-10-17) and now ends at the bound.
pub const FACE_OP_WORK: u64 = 450_000_000;

/// What [`face_op_work`] adds per span of a freeform face that moves (its
/// offset is proven regular and fitted span by span) and per edge on a
/// face that moves (each is made again on its faces, and the body is
/// classified beside it).
pub const FACE_OP_WORK_PER_SPAN: u64 = 20_000;
pub const FACE_OP_WORK_PER_EDGE: u64 = 1_000_000;
/// What [`face_op_work`] adds per span of every freeform face of the body:
/// the points beside every edge made again are classified against all of
/// them.
pub const FACE_OP_WORK_PER_BODY_SPAN: u64 = 50_000;

pub(crate) const SPENT: &str = "it needs more work than one operation is allowed: each try of it is dear here (offsets of freeform faces of many spans, and the checks of the body they make), and the search for a size that fits did not end within the bound";
pub(crate) const TURN_TOO_FAR: &str = "the faces are tangent here only to within an angle that, at this distance, parts their offsets by more than a quarter of the tolerance: they do not meet as tangent faces within it";
pub(crate) const NO_FIT: &str =
    "the offset of the freeform face could not be approximated within the tolerance";

/// The surface at `distance` from that of `face`, along its outward
/// normal. A freeform face's is a B-spline surface within a share of the
/// tolerance of the exact offset ([`craft3d_geom::offset`]) of its
/// surface continued by `reach` past its borders ([`continued`]).
pub(crate) fn offset_surface(
    face: &Face,
    distance: f64,
    (reach, growth): (f64, Growth),
    turn: Option<&Turn>,
    tolerance: &Tolerance,
) -> Result<Surface, NoOffset> {
    // Along the surface's own normal.
    let along = if face.reversed { -distance } else { distance };
    match &face.surface {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            frame
                .at(frame.origin() + frame.z() * along)
                .map(|frame| Surface::Plane(Plane::new(frame)))
                .ok_or(NoOffset::Gone)
        }
        Surface::Cylinder(cylinder) => Cylinder::new(*cylinder.frame(), cylinder.radius() + along)
            .map(Surface::Cylinder)
            .map_err(|_| NoOffset::Gone),
        Surface::Sphere(sphere) => Sphere::new(*sphere.frame(), sphere.radius() + along)
            .map(Surface::Sphere)
            .map_err(|_| NoOffset::Gone),
        Surface::Torus(torus) => {
            let (major, minor) = torus.radii();
            Torus::new(*torus.frame(), major, minor + along)
                .map(Surface::Torus)
                .map_err(|_| NoOffset::Gone)
        }
        Surface::Cone(cone) => {
            // The same angle about the same axis; the radius at every
            // height changes by the distance over the cosine.
            let (slope, cosine) = (cone.half_angle().tan(), cone.half_angle().cos());
            if !(cosine > 0.0 && slope != 0.0) {
                return Err(NoOffset::Gone);
            }
            let radius = cone.radius() + along / cosine;
            let frame = cone.frame();
            let (frame, radius) = if radius >= 0.0 {
                (*frame, radius)
            } else {
                // The height the frame stands at is past the new apex:
                // stand it where the radius is what it was.
                let lift = (cone.radius() - radius) / slope;
                let Some(lifted) = frame.at(frame.origin() + frame.z() * lift) else {
                    return Err(NoOffset::Gone);
                };
                (lifted, cone.radius())
            };
            Cone::new(frame, radius, cone.half_angle())
                .map(Surface::Cone)
                .map_err(|_| NoOffset::Gone)
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => {
            let over = reached(&face.surface, reach, growth, tolerance)
                .map(|(_, over)| over)
                .ok_or(NoOffset::NoFit)?;
            let turned = |u: f64, v: f64| turn.map_or(craft3d_math::Vec3::ZERO, |t| t.at(u, v));
            craft3d_geom::offset::offset_along(
                &face.surface,
                along,
                OFFSET_SHARE * tolerance.linear,
                over,
                &turned,
            )
            .map(Surface::Nurbs)
            .map_err(|error| match error {
                OffsetError::Folds { .. } | OffsetError::Unproven { .. } => NoOffset::Folds,
                OffsetError::NoNormal { .. } => NoOffset::NoNormal,
                OffsetError::Invalid(craft3d_geom::GeomError::Unsupported(why)) => {
                    NoOffset::Refused(why)
                }
                OffsetError::Spent => NoOffset::Refused(SPENT),
                OffsetError::NoFit | OffsetError::Invalid(_) => NoOffset::NoFit,
            })
        }
    }
}

/// Which borders of a surface's parameter range to go on past: the start
/// and the end in u, the start and the end in v.
pub(crate) type Growth = [bool; 4];

/// Every border.
const ALL_BORDERS: Growth = [true; 4];

/// A freeform surface as a NURBS surface, and the box of parameters that
/// reaches about `reach` past each of its borders in `growth` along it,
/// in a direction in which it does not close on itself. `None` for
/// another kind of surface.
fn reached(
    surface: &Surface,
    reach: f64,
    growth: Growth,
    tolerance: &Tolerance,
) -> Option<(NurbsSurface, craft3d_geom::offset::Domain)> {
    let nurbs = match surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok()?,
        _ => return None,
    };
    let ((u0, u1), (v0, v1)) = nurbs.domain();
    if !(reach > 0.0 && reach.is_finite()) {
        return Some((nurbs, ((u0, u1), (v0, v1))));
    }
    // How far the parameter has to go past a border for the surface to
    // reach that far: by the fastest speed across it along that border.
    let past = |along_u: bool, at: f64| {
        let speed = (0..=8)
            .map(|k| {
                let f = f64::from(k) / 8.0;
                let (_, du, dv) = if along_u {
                    nurbs.derivatives(at, v0 + (v1 - v0) * f)
                } else {
                    nurbs.derivatives(u0 + (u1 - u0) * f, at)
                };
                if along_u { du.length() } else { dv.length() }
            })
            .fold(0.0_f64, f64::max);
        if speed > 0.0 && speed.is_finite() {
            reach / speed
        } else {
            0.0
        }
    };
    let tol = tolerance.linear;
    let [grow_u0, grow_u1, grow_v0, grow_v1] = growth;
    let (closes_u, closes_v) = (nurbs.closes(true, tol), nurbs.closes(false, tol));
    let by = |grow: bool, closes: bool, along_u: bool, at: f64| {
        if grow && !closes {
            past(along_u, at)
        } else {
            0.0
        }
    };
    let over = (
        (
            u0 - by(grow_u0, closes_u, true, u0),
            u1 + by(grow_u1, closes_u, true, u1),
        ),
        (
            v0 - by(grow_v0, closes_v, false, v0),
            v1 + by(grow_v1, closes_v, false, v1),
        ),
    );
    Some((nurbs, over))
}

/// A freeform surface continued past its borders by about `reach` along
/// it ([`reached`]) as the functions its end spans are, so every point of
/// it keeps its parameters: exactly where the weights allow, else within
/// a share of the tolerance; another surface, or one that cannot be
/// continued, as it is.
pub(crate) fn continued(
    surface: &Surface,
    reach: f64,
    growth: Growth,
    tolerance: &Tolerance,
) -> Surface {
    let Some((nurbs, (u, v))) = reached(surface, reach, growth, tolerance) else {
        return surface.clone();
    };
    if (u, v) == nurbs.domain() {
        return surface.clone();
    }
    let tol = tolerance.linear;
    match nurbs.extended(u, v, tol) {
        Ok(wider) => Surface::Nurbs(wider),
        // A rational span continued far may need weights that are not
        // positive: the continued surface within a share of the
        // tolerance instead (its offset by nothing).
        Err(_) => craft3d_geom::offset::offset_over(surface, 0.0, OFFSET_SHARE * tol, (u, v))
            .map_or_else(|_| surface.clone(), Surface::Nurbs),
    }
}

/// How far the freeform faces are continued in the search for one that is
/// far enough, in multiples of the largest distance, growing.
const REACHES: [f64; 3] = [1.0, 4.0, 16.0];

/// Most tries of one offset in that search.
const MOST_TRIES: usize = 8;

/// Halvings in the search for the largest distance that fits.
const HALVINGS: usize = 48;

pub(crate) fn refuse(
    operation: &'static str,
    faces: Vec<FaceId>,
    edge: Option<EdgeId>,
    reason: &'static str,
) -> KernelError {
    KernelError::FaceOp {
        operation,
        faces,
        edge,
        reason,
    }
}

/// The body with every face of `offsets` at `fraction` of its distance.
///
/// A freeform face that moves, or one next to a face that moves, may have
/// to go on past the borders of its surface (inward on a ball the band
/// between two planes grows toward the poles): when faces do not meet
/// where a freeform one ends, it is tried again with the freeform faces
/// among them continued, and when the same ones fail again, further
/// ([`REACHES`]).
fn at(
    body: &Body,
    offsets: &[FaceOffset],
    fraction: f64,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    let largest = offsets
        .iter()
        .map(|o| (o.distance * fraction).abs())
        .fold(0.0_f64, f64::max);
    let freeform = |face: &FaceId| {
        body.face(*face)
            .is_some_and(|data| matches!(data.surface, Surface::Bezier(_) | Surface::Nurbs(_)))
    };
    let moving: BTreeMap<FaceId, f64> = offsets
        .iter()
        .map(|o| (o.face, o.distance * fraction))
        .collect();
    // The freeform faces continued, past which borders, and how far (a
    // step of REACHES): first those the vertices are seen to pass.
    let mut grow = borders_passed(body, &moving);
    let mut level = 0;
    let mut last = None;
    for _ in 0..MOST_TRIES {
        if craft3d_geom::allowance::pass() {
            return Err(TweakError::Unsupported {
                faces: Vec::new(),
                edge: None,
                reason: SPENT,
            });
        }
        let reach = REACHES.get(level).map_or(0.0, |times| times * largest);
        let mut wider = body.clone();
        for (face, growth) in &grow {
            if moving.contains_key(face) {
                continue;
            }
            if let Some(slot) = wider.faces.get_mut(face.0 as usize) {
                slot.surface = continued(&slot.surface, reach, *growth, tolerance);
            }
        }
        let reach_of = |face: FaceId| match grow.get(&face) {
            Some(growth) => (reach, *growth),
            None => (0.0, [false; 4]),
        };
        let got = surfaced(&wider, offsets, fraction, &reach_of, operation, tolerance);
        match got {
            Err(error @ (TweakError::Unsupported { .. } | TweakError::Vanishes { .. }))
                if ends_on_freeform(&error) =>
            {
                let named: Vec<FaceId> = match &error {
                    TweakError::Unsupported { faces, .. } | TweakError::Vanishes { faces, .. } => {
                        faces.iter().copied().filter(freeform).collect()
                    }
                    TweakError::Kernel(_) => Vec::new(),
                };
                // The faces named go on past every border; when they did
                // already, all of them go on further.
                let mut changed = false;
                for face in named {
                    let slot = grow.entry(face).or_insert([false; 4]);
                    if *slot != ALL_BORDERS {
                        *slot = ALL_BORDERS;
                        changed = true;
                    }
                }
                if !changed {
                    level += 1;
                }
                last = Some(error);
                if level >= REACHES.len() || grow.is_empty() {
                    break;
                }
            }
            other => return other,
        }
    }
    Err(last.unwrap_or(TweakError::Unsupported {
        faces: Vec::new(),
        edge: None,
        reason: tweak::FREEFORM,
    }))
}

/// Per freeform face, the borders of its surface's parameter range that a
/// vertex of it passes when the faces of `moving` move by their
/// distances: the new place of every vertex next to a moving face is
/// foreseen where the tangent planes of its faces, moved, meet (one step
/// of [`tweak`]'s iteration, a nearly tangent plane adding nothing), and a
/// face whose surface has its nearest point to it on a border, not square
/// below it, has to go on past that border.
fn borders_passed(body: &Body, moving: &BTreeMap<FaceId, f64>) -> BTreeMap<FaceId, Growth> {
    let uses = tweak::edge_faces(body);
    let mut around: Vec<BTreeSet<FaceId>> = vec![BTreeSet::new(); body.vertices.len()];
    for (index, edge) in body.edges.iter().enumerate() {
        for vertex in [edge.start, edge.end] {
            if let (Some(list), Some(faces)) = (around.get_mut(vertex.0 as usize), uses.get(index))
            {
                list.extend(faces.iter().copied());
            }
        }
    }
    let mut out: BTreeMap<FaceId, Growth> = BTreeMap::new();
    for (vertex, faces) in body.vertices.iter().zip(&around) {
        if !faces.iter().any(|f| moving.contains_key(f)) {
            continue;
        }
        let at = vertex.point;
        let mut basis: Vec<craft3d_math::Vec3> = Vec::with_capacity(3);
        let mut solved: Vec<f64> = Vec::with_capacity(3);
        for face in faces {
            let Some(data) = body.face(*face) else {
                continue;
            };
            let Some(normal) = data
                .surface
                .project(at)
                .ok()
                .and_then(|(u, v)| data.surface.normal(u, v))
            else {
                continue;
            };
            let normal = if data.reversed { -normal } else { normal };
            let mut rest = normal;
            let mut right = moving.get(face).copied().unwrap_or(0.0);
            for (axis, c) in basis.iter().zip(&solved) {
                let along = normal.dot(*axis);
                right -= along * c;
                rest = rest - *axis * along;
            }
            let own = rest.length();
            if own <= tweak::TANGENT || basis.len() >= 3 {
                continue;
            }
            basis.push(rest * (1.0 / own));
            solved.push(right / own);
        }
        let mut to = at;
        for (axis, c) in basis.iter().zip(&solved) {
            to += *axis * *c;
        }
        if !to.is_finite() {
            continue;
        }
        for face in faces.iter().filter(|f| {
            body.face(**f)
                .is_some_and(|d| matches!(d.surface, Surface::Bezier(_) | Surface::Nurbs(_)))
        }) {
            let Some(data) = body.face(*face) else {
                continue;
            };
            let Ok((u, v)) = data.surface.project(to) else {
                continue;
            };
            let Some(normal) = data.surface.normal(u, v) else {
                continue;
            };
            // Held by a border: the nearest point is not square below it
            // (by any amount: a vertex just past a border is past it).
            let off = to - data.surface.eval(u, v);
            let aside = (off - normal * off.dot(normal)).length();
            if aside <= tweak::SQUARE * off.length() + 1e-15 * (1.0 + to.length()) {
                continue;
            }
            let ((u0, u1), (v0, v1)) = data.surface.domain();
            let passed = [u <= u0, u >= u1, v <= v0, v >= v1];
            let slot = out.entry(*face).or_insert([false; 4]);
            for (s, p) in slot.iter_mut().zip(passed) {
                *s |= p;
            }
        }
    }
    out
}

/// Whether `error` is about freeform faces that no longer meet where one
/// of them ends: what continuing them may mend.
fn ends_on_freeform(error: &TweakError) -> bool {
    match error {
        TweakError::Unsupported { reason, .. } | TweakError::Vanishes { reason, .. } => {
            *reason == tweak::FREEFORM || *reason == tweak::LOST
        }
        TweakError::Kernel(_) => false,
    }
}

/// [`at`] with each freeform face that moves continued by `reach_of` it.
fn surfaced(
    body: &Body,
    offsets: &[FaceOffset],
    fraction: f64,
    reach_of: &dyn Fn(FaceId) -> (f64, Growth),
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, TweakError> {
    let mut surfaces: BTreeMap<FaceId, Surface> = BTreeMap::new();
    let moving: BTreeMap<FaceId, f64> = offsets
        .iter()
        .map(|o| (o.face, o.distance * fraction))
        .collect();
    let turns = crate::offset_normal::turns(body, &moving, tolerance).map_err(|far| {
        TweakError::Vanishes {
            faces: far.faces,
            edge: None,
            reason: TURN_TOO_FAR,
        }
    })?;
    for offset in offsets {
        if craft3d_geom::allowance::pass() {
            return Err(TweakError::Unsupported {
                faces: vec![offset.face],
                edge: None,
                reason: SPENT,
            });
        }
        let Some(face) = body.face(offset.face) else {
            return Err(TweakError::Unsupported {
                faces: vec![offset.face],
                edge: None,
                reason: "there is no such face in the body",
            });
        };
        let faces = vec![offset.face];
        let surface = offset_surface(
            face,
            offset.distance * fraction,
            reach_of(offset.face),
            turns.get(&offset.face),
            tolerance,
        );
        let surface = surface.map_err(|no| {
            match no {
                NoOffset::Gone => TweakError::Vanishes {
                    faces,
                    edge: None,
                    reason: "the radius of the face would reach zero: the face would vanish",
                },
                // Nearer, the face bends less far than the distance.
                NoOffset::Folds => TweakError::Vanishes {
                    faces,
                    edge: None,
                    reason: FOLDS,
                },
                NoOffset::NoNormal => TweakError::Unsupported {
                    faces,
                    edge: None,
                    reason: NO_NORMAL,
                },
                NoOffset::NoFit => TweakError::Unsupported {
                    faces,
                    edge: None,
                    reason: NO_FIT,
                },
                NoOffset::Refused(why) => TweakError::Unsupported {
                    faces,
                    edge: None,
                    reason: why,
                },
            }
        })?;
        surfaces.insert(offset.face, surface);
    }
    let iso = crate::offset_normal::iso_edges(body, &moving, tolerance);
    tweak::resurface_along(body, &surfaces, &iso, operation, tolerance)
}

/// [`offset_faces`] in the words of `operation`, for the operations that
/// are made of it.
pub(crate) fn offset_for(
    body: &Body,
    offsets: &[FaceOffset],
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "the body is a sheet: offsetting, shelling or thickening a sheet is not built yet (only the faces of a solid can be moved)",
        ));
    }
    if offsets.len() > MAX_FACES {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "more faces than one operation takes",
        ));
    }
    let mut largest = 0.0_f64;
    for (index, offset) in offsets.iter().enumerate() {
        if !offset.distance.is_finite() {
            return Err(KernelError::InvalidSize {
                name: "distance",
                value: offset.distance,
            });
        }
        if offsets
            .iter()
            .take(index)
            .any(|earlier| earlier.face == offset.face)
        {
            return Err(refuse(
                operation,
                vec![offset.face],
                None,
                "the face is given twice",
            ));
        }
        largest = largest.max(offset.distance.abs());
    }
    if offsets.is_empty() || largest <= tol {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "there is nothing to move: no face, or a distance that cannot be told from none at this tolerance",
        ));
    }
    let defects = craft3d_topo::check(body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    let unfolded = |fraction: f64| {
        offsets.iter().all(|offset| {
            let Some(face) = body.face(offset.face) else {
                return true;
            };
            if !matches!(face.surface, Surface::Bezier(_) | Surface::Nurbs(_)) {
                return true;
            }
            let along = offset.distance * fraction * if face.reversed { -1.0 } else { 1.0 };
            craft3d_geom::offset::regular(&face.surface, along).is_ok()
        })
    };
    let moved: Vec<FaceId> = offsets.iter().map(|o| o.face).collect();
    fitting_with(
        |fraction| at(body, offsets, fraction, operation, tolerance),
        Some(&unfolded),
        largest,
        operation,
        tolerance,
        face_op_work(body, &moved),
    )
}

/// `attempt(1.0)`, the whole of an operation whose size is `largest`
/// (a length: how far the farthest thing moves); when that goes too far,
/// [`KernelError::FaceOpTooFar`] with the largest part of it that fits,
/// found by halving (`attempt(fraction)`), or, when no part of it fits,
/// [`KernelError::FaceOp`] with what is not built.
pub(crate) fn fitting(
    attempt: impl Fn(f64) -> Result<Body, TweakError>,
    largest: f64,
    operation: &'static str,
    tolerance: &Tolerance,
    bound: u64,
) -> Result<Body, KernelError> {
    fitting_with(attempt, None, largest, operation, tolerance, bound)
}

/// [`fitting`], where a freeform face's offset that folds over is told by
/// `unfolded(fraction)` (true when every freeform face's offset at that
/// part of its distance is proven not to fold): its limit is found by
/// halving on that alone. Close to a fold the offset forms a cusp, and a
/// body built there costs more the closer it comes, without bound; the
/// limit is the surface's, not the body's. When the whole is proven
/// regular after all (the fold lay where a surface was continued), the
/// halving builds the body as for any other limit.
///
/// **Bounded.** The whole search is held to [`FACE_OP_WORK`]: the tries
/// and the proofs stop when it is reached, and the operation is refused
/// with [`SPENT`]. A try during which the bound was reached is not
/// believed, whatever it gave (a check cut short could have let a body
/// through), so a body that comes out is one whose try ran to its end.
pub(crate) fn fitting_with(
    attempt: impl Fn(f64) -> Result<Body, TweakError>,
    unfolded: Option<&dyn Fn(f64) -> bool>,
    largest: f64,
    operation: &'static str,
    tolerance: &Tolerance,
    bound: u64,
) -> Result<Body, KernelError> {
    craft3d_geom::allowance::within(bound, || {
        bounded_fitting(attempt, unfolded, largest, operation, tolerance)
    })
}

/// The spans of a freeform surface (its Bézier pieces); none for another.
fn spans(surface: &Surface) -> u64 {
    let distinct = |knots: &[f64], (lo, hi): (f64, f64)| -> u64 {
        let mut count = 0_u64;
        let mut last = f64::NAN;
        for &k in knots {
            if k > lo && k < hi && k != last {
                count += 1;
                last = k;
            }
        }
        count + 1
    };
    match surface {
        Surface::Bezier(_) => 1,
        Surface::Nurbs(s) => {
            let (u, v) = s.domain();
            distinct(s.knots_u(), u).saturating_mul(distinct(s.knots_v(), v))
        }
        _ => 0,
    }
}

/// The work bound of a face operation on `body` that moves `moved`:
/// [`FACE_OP_WORK`], and more the more freeform spans and edges the faces
/// that move have, so that a shell of many faces is not held to what one
/// offset may do.
pub(crate) fn face_op_work(body: &Body, moved: &[FaceId]) -> u64 {
    let moved_spans: u64 = moved
        .iter()
        .filter_map(|f| body.face(*f))
        .map(|f| spans(&f.surface))
        .sum();
    let body_spans: u64 = body.faces.iter().map(|f| spans(&f.surface)).sum();
    let moved_edges: usize = moved
        .iter()
        .filter_map(|f| body.face(*f))
        .flat_map(|f| f.loops.iter().filter_map(|l| body.loop_(*l)))
        .map(|l| l.coedges.len())
        .sum();
    FACE_OP_WORK
        .saturating_add(moved_spans.saturating_mul(FACE_OP_WORK_PER_SPAN))
        .saturating_add((moved_edges as u64).saturating_mul(FACE_OP_WORK_PER_EDGE))
        .saturating_add(body_spans.saturating_mul(FACE_OP_WORK_PER_BODY_SPAN))
}

fn bounded_fitting(
    attempt: impl Fn(f64) -> Result<Body, TweakError>,
    unfolded: Option<&dyn Fn(f64) -> bool>,
    largest: f64,
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    let tol = tolerance.linear;
    let first = attempt(1.0);
    let spent = |faces: Vec<FaceId>, edge: Option<EdgeId>| refuse(operation, faces, edge, SPENT);
    if craft3d_geom::allowance::exhausted() {
        let faces = match &first {
            Err(TweakError::Unsupported { faces, .. } | TweakError::Vanishes { faces, .. }) => {
                faces.clone()
            }
            _ => Vec::new(),
        };
        return Err(spent(faces, None));
    }
    let (faces, edge, reason) = match first {
        Ok(made) => return Ok(made),
        Err(TweakError::Kernel(error)) => return Err(error),
        Err(TweakError::Unsupported {
            faces,
            edge,
            reason,
        }) => return Err(refuse(operation, faces, edge, reason)),
        Err(TweakError::Vanishes {
            faces,
            edge,
            reason,
        }) => (faces, edge, reason),
    };
    // The largest part of the distance that can be made, by halving, to
    // within the tolerance (closer to where a face vanishes or folds, each
    // try is a body more nearly degenerate, and dearer).
    let folds = match unfolded {
        Some(proven) if reason == FOLDS && !proven(1.0) => Some(proven),
        _ => None,
    };
    let (mut fits, mut fails) = (0.0_f64, 1.0_f64);
    for _ in 0..HALVINGS {
        let middle = 0.5 * (fits + fails);
        if middle * largest <= tol || (fails - fits) * largest <= tol {
            break;
        }
        let holds = match folds {
            Some(proven) => proven(middle),
            None => attempt(middle).is_ok(),
        };
        if craft3d_geom::allowance::exhausted() {
            return Err(spent(faces, edge));
        }
        if holds {
            fits = middle;
        } else {
            fails = middle;
        }
    }
    if fits == 0.0 {
        // Not a matter of how far.
        return Err(refuse(operation, faces, edge, reason));
    }
    Err(KernelError::FaceOpTooFar {
        operation,
        faces,
        edge,
        reason,
        value: largest,
        largest: fits * largest,
    })
}

/// `body` with the faces of `offsets` moved along their outward normals
/// and the faces next to them following (see the module documentation).
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a distance at which an
/// edge or a face would vanish, with the largest that fits;
/// [`KernelError::FaceOp`] for what is not built or not a face;
/// [`KernelError::InvalidBody`] when the body given, or (a bug, or input
/// at the edge of tolerance) the result fails the checker.
pub fn offset_faces(
    body: &Body,
    offsets: &[FaceOffset],
    tolerance: &Tolerance,
) -> Result<Offset, KernelError> {
    offset_for(body, offsets, OPERATION, tolerance).map(same_numbers)
}

/// What every face operation that moves faces asks of its input: a
/// usable tolerance, a solid that passes the checker, and faces of it,
/// each once and not too many.
pub(crate) fn usable(
    body: &Body,
    faces: &[FaceId],
    operation: &'static str,
    tolerance: &Tolerance,
) -> Result<(), KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0 && tolerance.angular.is_finite() && tolerance.angular > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if body.kind != BodyKind::Solid {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "the body is a sheet: offsetting, shelling or thickening a sheet is not built yet (only the faces of a solid can be moved)",
        ));
    }
    if faces.is_empty() {
        return Err(refuse(operation, Vec::new(), None, "no face is given"));
    }
    if faces.len() > MAX_FACES {
        return Err(refuse(
            operation,
            Vec::new(),
            None,
            "more faces than one operation takes",
        ));
    }
    for (index, face) in faces.iter().enumerate() {
        if body.face(*face).is_none() {
            return Err(refuse(
                operation,
                vec![*face],
                None,
                "there is no such face in the body",
            ));
        }
        if faces.iter().take(index).any(|earlier| earlier == face) {
            return Err(refuse(
                operation,
                vec![*face],
                None,
                "the face is given twice",
            ));
        }
    }
    let defects = craft3d_topo::check(body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation,
            defects: defects.into_iter().take(crate::MAX_REPORTED).collect(),
        });
    }
    Ok(())
}

/// What a body made by [`crate::tweak`] is: every entity under its own
/// number.
pub(crate) fn same_numbers(made: Body) -> Offset {
    let numbered = |count: usize| (0..count).map(|i| u32::try_from(i).unwrap_or(u32::MAX));
    Offset {
        faces: numbered(made.faces.len()).map(FaceId).collect(),
        edges: numbered(made.edges.len())
            .map(|i| Some(EdgeId(i)))
            .collect(),
        vertices: numbered(made.vertices.len())
            .map(|i| Some(VertexId(i)))
            .collect(),
        body: made,
    }
}
