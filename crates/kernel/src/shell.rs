//! Shell: a solid made hollow, with walls of one thickness behind every
//! face except the ones left open.
//!
//! **Method.** The hollow is a body of its own: the solid with every
//! face that keeps a wall moved into the material by the thickness
//! ([`crate::offset_face`]: flat faces to parallel planes, cylinders,
//! cones and spheres about their axes, the edges between them found
//! again), and every open face moved outward by the same amount, so that
//! the hollow reaches through it and the opening is as wide as the
//! hollow is there. The shell is the solid less that body
//! ([`crate::boolean`]), which crosses the open faces squarely and
//! touches nothing else. Without open faces the hollow lies inside the
//! solid and the result has two shells: the skin and a cavity.
//!
//! The offset is the local operation, so the hollow has, face for face,
//! the faces of the solid: a face of the result is a piece of the skin
//! of the solid or the face of the hollow behind one, and is reported so.
//!
//! **Never a wrong solid.** A thickness at which a face of the hollow
//! vanishes (half the width of a wall, the radius of a rounded edge) is
//! refused with the face or the edge and the largest thickness that
//! fits. After the boolean the result is held against what a shell is:
//! every face that keeps a wall is there as skin and as a face of the
//! hollow, and the hollow comes out through every open face.
//!
//! **An open face with a blend along it** (a box rounded all round, open
//! at its top; 2026-10-22). A blend tangent to an open face cannot follow
//! it outward, so such a face (it has to be flat) is moved inward like
//! every other face, the blends along it keep their walls (offset, or
//! collapsed where the thickness reaches their radius), and the hollow is
//! opened through it by the prism on the hollow's face behind it, along
//! the face's normal, as far past the face as the other open faces are
//! moved: the lip is the blend's wall, of the thickness all along, ending
//! square to the open face where the face's rim was.

use crate::KernelError;
use crate::boolean::{BooleanOp, BooleanOptions, EdgeSource, Operand, VertexSource, boolean_with};
use crate::offset_blend::Fillet;
use crate::offset_face::{Blends, FaceOffset, offset_for_known, refuse};
use crate::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_geom::Surface;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, EdgeId, FaceId, VertexId, edge_faces};
use std::collections::{BTreeMap, BTreeSet};

const OPERATION: &str = "shell";

/// Which of the two skins of a shell an entity is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skin {
    /// The skin of the solid that was made hollow.
    Outer,
    /// The hollow: its entities are those of the solid, moved inward.
    Inner,
}

/// A shell and where each of its entities came from: `faces[i]` says of
/// `FaceId(i)` of `body` which face of the solid it is a piece of (outer
/// skin) or lies behind (the hollow), and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct Shelled {
    pub body: Body,
    pub faces: Vec<(Skin, FaceId)>,
    pub edges: Vec<Option<(Skin, EdgeId)>>,
    pub vertices: Vec<Option<(Skin, VertexId)>>,
}

const LOST: &str = "the shell lost track of a face of the hollow; this is a bug";

/// Normals nearer to each other than this sine are one direction (as
/// delete face takes them).
const TANGENT: f64 = 1e-6;

/// Whether `a` and `b` are tangent across `edge` (at its middle).
fn tangent_across(body: &Body, a: FaceId, b: FaceId, edge: EdgeId) -> bool {
    let Some(e) = body.edge(edge) else {
        return false;
    };
    let at = e.curve.eval(0.5 * (e.range.0 + e.range.1));
    match (
        body.face(a).and_then(|f| f.outward(at)),
        body.face(b).and_then(|f| f.outward(at)),
    ) {
        (Some(na), Some(nb)) => na.cross(nb).length() <= TANGENT && na.dot(nb) > 0.0,
        _ => false,
    }
}

/// The faces of `body` whose face of the hollow would vanish at
/// `thickness` and that the hollow does without: convex rolling-ball
/// blends whose radius the thickness reaches (within the tolerance), a
/// cylinder, a sphere or a torus's tube whose radius shrinks inward, or a
/// fillet face [`crate::offset_blend`] recognises, that keep a wall and
/// run along faces that keep one too, tangent to them: a strip (its two
/// sides tangent to the faces beside it) or a corner (tangent across
/// every edge to such blends).
///
/// **Why the hollow is then the hollow of the body with those blends
/// collapsed.** The hollow is the solid eroded by a ball of radius `t`:
/// the points at least `t` from its outside. A rolling-ball blend of
/// radius `r` is, there, the solid opened by a ball of radius `r` (eroded
/// and grown back by it). For `t ≥ r` the erosion by `t` of the opening
/// is the erosion of the solid itself: the opening lies in the solid, and
/// a point whose ball of radius `t` fits in the solid has a ball of
/// radius `t − r` round it that does, which grown by `r` lies in the
/// opening, so its ball of radius `t` fits there too. So exactly the
/// blends whose radius the thickness reaches are left out, every other
/// face keeps its own (a fillet of a larger radius is rebuilt, one of
/// the other kind grows). `open` holds the faces moved outward only: an
/// open face with a blend along it keeps a wall in the hollow and is
/// opened afterwards (see the module documentation), so the blends along
/// it may vanish like any other.
fn vanishing(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    fillets: &BTreeMap<FaceId, Fillet>,
    tolerance: &Tolerance,
) -> BTreeSet<FaceId> {
    let tol = tolerance.linear;
    let mut found: BTreeSet<FaceId> = BTreeSet::new();
    for (index, face) in body.faces.iter().enumerate() {
        let Ok(id) = u32::try_from(index).map(FaceId) else {
            continue;
        };
        if open.contains(&id) {
            continue;
        }
        // Inward, along the surface's own normal.
        let along = if face.reversed { thickness } else { -thickness };
        let radius = match &face.surface {
            Surface::Cylinder(c) => Some(c.radius()),
            Surface::Sphere(s) => Some(s.radius()),
            Surface::Torus(t) => Some(t.radii().1),
            Surface::Bezier(_) | Surface::Nurbs(_) | Surface::Plane(_) | Surface::Cone(_) => None,
        };
        if radius.is_some_and(|r| r + along <= tol) {
            found.insert(id);
        }
    }
    // The fillet faces among the freeform ones, and the faces each rolls
    // along, read off it.
    let mut supports: BTreeMap<FaceId, [FaceId; 2]> = BTreeMap::new();
    for (face, fillet) in fillets {
        if !open.contains(face) && fillet.ball.convex && fillet.ball.radius - thickness <= tol {
            found.insert(*face);
            supports.insert(*face, fillet.supports);
        }
    }
    // Only blends: a strip whose sides, the edges along which its ball
    // rolls (lines along a cylinder's axis, circles about a torus's,
    // the edges to the faces a fillet face rolls along), are tangent to
    // faces that keep a wall; a ball (a sphere) whose every edge it
    // shares tangentially with such blends. A face left without what
    // makes it one goes, until none does.
    let uses = edge_faces(body);
    let other = |edge: EdgeId, face: FaceId| -> Option<FaceId> {
        match uses.get(edge.0 as usize)?.as_slice() {
            [a, b] if *a == face && *b != face => Some(*b),
            [a, b] if *b == face && *a != face => Some(*a),
            _ => None,
        }
    };
    let rolls_along = |face: FaceId, edge: EdgeId| -> Option<bool> {
        let f = body.face(face)?;
        let e = body.edge(edge)?;
        let (at, along) = e.curve.derivatives(0.5 * (e.range.0 + e.range.1));
        let along = along.normalized()?;
        let parallel = |direction: Vec3| {
            direction
                .normalized()
                .is_some_and(|d| d.cross(along).length() <= TANGENT)
        };
        Some(match &f.surface {
            Surface::Cylinder(c) => parallel(c.frame().z()),
            Surface::Torus(t) => {
                let frame = t.frame();
                parallel(frame.z().cross(at - frame.origin()))
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => supports
                .get(&face)
                .is_some_and(|s| other(edge, face).is_some_and(|o| s.contains(&o))),
            Surface::Sphere(_) | Surface::Plane(_) | Surface::Cone(_) => false,
        })
    };
    for _ in 0..=found.len() {
        let before = found.len();
        let kept: BTreeSet<FaceId> = found
            .iter()
            .copied()
            .filter(|face| {
                let Some(f) = body.face(*face) else {
                    return false;
                };
                let edges: Vec<EdgeId> = f
                    .loops
                    .iter()
                    .filter_map(|l| body.loop_(*l))
                    .flat_map(|l| l.coedges.iter().map(|c| c.edge))
                    .collect();
                let tangent = |edge: EdgeId| {
                    other(edge, *face)
                        .is_some_and(|o| !open.contains(&o) && tangent_across(body, *face, o, edge))
                };
                if matches!(f.surface, Surface::Sphere(_)) {
                    return !edges.is_empty()
                        && edges.iter().all(|e| {
                            tangent(*e) && other(*e, *face).is_some_and(|o| found.contains(&o))
                        });
                }
                let sides: Vec<EdgeId> = edges
                    .iter()
                    .copied()
                    .filter(|e| rolls_along(*face, *e) == Some(true))
                    .collect();
                sides.len() == 2 && sides.iter().all(|e| tangent(*e))
            })
            .collect();
        found = kept;
        if found.len() == before {
            break;
        }
    }
    found
}

/// Whether a face is tangent to one of its neighbours across an edge they
/// share: a blend runs along it.
fn blend_along(body: &Body, face: FaceId) -> bool {
    let uses = edge_faces(body);
    let Some(f) = body.face(face) else {
        return false;
    };
    f.loops
        .iter()
        .filter_map(|l| body.loop_(*l))
        .flat_map(|l| l.coedges.iter().map(|c| c.edge))
        .any(|edge| {
            uses.get(edge.0 as usize).is_some_and(|faces| {
                faces
                    .iter()
                    .any(|o| *o != face && tangent_across(body, face, *o, edge))
            })
        })
}

const LIP_CURVES: &str = "an open face with a blend along it is opened by the prism on the hollow's face behind it, whose edges have to be lines and circles: this one has another curve";

/// The prism on the flat face `face` of `hollow`, along its outward
/// normal from its plane to `height` past it: what opens the hollow
/// through an open face with a blend along it.
fn prism(
    hollow: &Body,
    face: FaceId,
    height: f64,
    open: FaceId,
    tolerance: &Tolerance,
) -> Result<Body, KernelError> {
    let refused = |reason: &'static str| refuse(OPERATION, vec![open], None, reason);
    let Some(f) = hollow.face(face) else {
        return Err(refused(LOST));
    };
    let Surface::Plane(plane) = &f.surface else {
        return Err(refused(
            "an open face with a blend along it has to be flat to be opened",
        ));
    };
    let frame = plane.frame();
    // Its z out of the hollow.
    let frame: Frame = if f.reversed {
        frame
            .reversed_at(frame.origin())
            .ok_or_else(|| refused(LOST))?
    } else {
        *frame
    };
    let flat = |p: Vec3| {
        let (a, b, _) = frame.local(p);
        [a, b]
    };
    let mut curves = Vec::new();
    for l in f.loops.iter().filter_map(|l| hollow.loop_(*l)) {
        for coedge in &l.coedges {
            let Some(edge) = hollow.edge(coedge.edge) else {
                return Err(refused(LOST));
            };
            let (Some(start), Some(end)) = (hollow.vertex(edge.start), hollow.vertex(edge.end))
            else {
                return Err(refused(LOST));
            };
            let (start, end) = (flat(start.point), flat(end.point));
            let curve = match &edge.curve {
                craft3d_geom::Curve3::Line(_) => Curve2::Line { start, end },
                craft3d_geom::Curve3::Circle(c) => {
                    let (a, b, _) = frame.local(c.frame().origin());
                    let center = [a, b];
                    let turns = c.frame().z().dot(frame.z());
                    if edge.start == edge.end {
                        Curve2::Circle {
                            center,
                            radius: c.radius(),
                        }
                    } else if turns > 0.0 {
                        Curve2::Arc { center, start, end }
                    } else {
                        Curve2::Arc {
                            center,
                            start: end,
                            end: start,
                        }
                    }
                }
                craft3d_geom::Curve3::Ellipse(_) | craft3d_geom::Curve3::Nurbs(_) => {
                    return Err(refused(LIP_CURVES));
                }
            };
            curves.push(SketchCurve {
                id: u64::from(coedge.edge.0) + 1,
                curve,
            });
        }
    }
    let profiles = detect_profiles(&curves, tolerance).map_err(|_| refused(LIP_CURVES))?;
    Ok(crate::extrude::extrude(&profiles, &frame, 0.0, height, tolerance)?.body)
}

fn skin(operand: Operand) -> Skin {
    match operand {
        Operand::A => Skin::Outer,
        Operand::B => Skin::Inner,
    }
}

/// `body` made hollow with walls of `thickness`, open at the faces
/// `open` (see the module documentation).
///
/// **Errors.** [`KernelError::FaceOpTooFar`] for a thickness at which an
/// edge or a face of the hollow would vanish, with the largest that
/// fits; [`KernelError::FaceOp`] for what is not built (the faces the
/// offset refuses) or not a face; [`KernelError::FaceOpBoolean`] when the
/// boolean refuses the hollow.
pub fn shell(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    shell_with(body, thickness, open, &Blends::default(), tolerance)
}

/// [`shell`], told how faces were made (`blends`: the radius each fillet
/// face was made with); a fillet's face of the hollow is the fillet
/// rebuilt between the faces of the hollow it rolls along
/// (`crate::offset_blend`).
pub fn shell_with(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    blends: &Blends,
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    shell_with_capped(body, thickness, open, blends, tolerance, None)
}

/// [`shell_with`] held to the document's cap on an entity's own tolerance
/// (`loosest`; `None` for the default, `topo::DEFAULT_LOOSEST` times the
/// linear tolerance): a body with loose entities above it is refused.
pub fn shell_with_capped(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    blends: &Blends,
    tolerance: &Tolerance,
    loosest: Option<f64>,
) -> Result<Shelled, KernelError> {
    crate::loose_op::run(
        OPERATION,
        body,
        tolerance,
        loosest,
        |at| shell_at(body, thickness, open, blends, at),
        |r| {
            let outer = |skin: Skin| skin == Skin::Outer;
            let sources = crate::loose_op::Sources {
                faces: r
                    .faces
                    .iter()
                    .map(|(skin, f)| outer(*skin).then_some(*f))
                    .collect(),
                edges: r
                    .edges
                    .iter()
                    .map(|e| e.and_then(|(skin, e)| outer(skin).then_some(e)))
                    .collect(),
                vertices: r
                    .vertices
                    .iter()
                    .map(|v| v.and_then(|(skin, v)| outer(skin).then_some(v)))
                    .collect(),
            };
            (&mut r.body, sources)
        },
    )
}

/// The operation itself, at the tolerance given.
fn shell_at(
    body: &Body,
    thickness: f64,
    open: &[FaceId],
    blends: &Blends,
    tolerance: &Tolerance,
) -> Result<Shelled, KernelError> {
    if !(thickness.is_finite() && thickness > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "thickness",
            value: thickness,
        });
    }
    for (index, face) in open.iter().enumerate() {
        if body.face(*face).is_none() {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "there is no such face in the body",
            ));
        }
        if open.iter().take(index).any(|earlier| earlier == face) {
            return Err(refuse(
                OPERATION,
                vec![*face],
                None,
                "the face is given twice",
            ));
        }
    }
    if open.len() >= body.faces.len() {
        return Err(refuse(
            OPERATION,
            Vec::new(),
            None,
            "every face is left open: there is nothing to leave a wall behind",
        ));
    }
    // Open faces with a blend along them are moved inward like the faces
    // that keep a wall and opened afterwards (see the module
    // documentation); the others are moved outward.
    let lipped: BTreeSet<FaceId> = open
        .iter()
        .copied()
        .filter(|face| blend_along(body, *face))
        .collect();
    let outward: Vec<FaceId> = open
        .iter()
        .copied()
        .filter(|face| !lipped.contains(face))
        .collect();
    // Blends whose radius the thickness reaches leave no face in the
    // hollow: it is the hollow of the body with them collapsed into the
    // edges and corners they round (see `vanishing`).
    // The fillet faces among the freeform faces, recognised once: the
    // hollow's offset takes them as they are when nothing vanishes.
    let freeform = body
        .faces
        .iter()
        .any(|f| matches!(f.surface, Surface::Bezier(_) | Surface::Nurbs(_)));
    let fillets = if freeform {
        let all: BTreeSet<FaceId> = (0..body.faces.len())
            .filter_map(|i| u32::try_from(i).ok().map(FaceId))
            .collect();
        crate::offset_blend::fillets(body, &all, &blends.radii, tolerance)
    } else {
        BTreeMap::new()
    };
    let vanish = vanishing(body, thickness, &outward, &fillets, tolerance);
    let collapsed = if vanish.is_empty() {
        None
    } else {
        let faces: Vec<FaceId> = vanish.iter().copied().collect();
        crate::delete_face::delete_faces(body, &faces, tolerance).ok()
    };
    // The faces that keep a wall but have none behind them.
    let gone: BTreeSet<FaceId> = if collapsed.is_some() {
        vanish.clone()
    } else {
        BTreeSet::new()
    };
    // The body the hollow is offset from, and per face of it the face of
    // `body` it is (the same numbers when nothing vanishes).
    let (inner_of, inner_faces, inner_edges, inner_vertices) = match &collapsed {
        Some(c) => (
            &c.body,
            c.faces.clone(),
            c.edges.clone(),
            c.vertices.clone(),
        ),
        None => (
            body,
            (0..body.faces.len())
                .filter_map(|i| u32::try_from(i).ok().map(FaceId))
                .collect(),
            (0..body.edges.len())
                .map(|i| u32::try_from(i).ok().map(EdgeId))
                .collect(),
            (0..body.vertices.len())
                .map(|i| u32::try_from(i).ok().map(VertexId))
                .collect(),
        ),
    };
    let offsets: Vec<FaceOffset> = inner_faces
        .iter()
        .enumerate()
        .filter_map(|(index, original)| {
            u32::try_from(index).ok().map(|face| FaceOffset {
                face: FaceId(face),
                distance: if outward.contains(original) {
                    thickness
                } else {
                    -thickness
                },
            })
        })
        .collect();
    let inner_blends = Blends {
        radii: inner_faces
            .iter()
            .enumerate()
            .filter_map(|(index, original)| {
                let radius = blends.radii.get(original)?;
                Some((FaceId(u32::try_from(index).ok()?), *radius))
            })
            .collect(),
    };
    let known = if collapsed.is_none() {
        Some(fillets)
    } else {
        None
    };
    let hollow = offset_for_known(
        inner_of,
        &offsets,
        &inner_blends,
        known,
        OPERATION,
        tolerance,
    )?;
    // Both passed the checker a moment ago.
    let options = BooleanOptions {
        trust_operands: true,
        ..BooleanOptions::default()
    };
    // The hollow opened through each open face with a blend along it, and
    // per face, edge and vertex of it what it is of `body`: a face of the
    // prism is the hollow behind that open face.
    let mut hollow = hollow;
    let mut inner_faces: Vec<FaceId> = inner_faces;
    let mut inner_edges: Vec<Option<EdgeId>> = inner_edges;
    let mut inner_vertices: Vec<Option<VertexId>> = inner_vertices;
    for open_face in &lipped {
        let Some(behind) = inner_faces
            .iter()
            .position(|f| f == open_face)
            .and_then(|i| u32::try_from(i).ok())
            .map(FaceId)
        else {
            return Err(refuse(OPERATION, vec![*open_face], None, LOST));
        };
        let opening = prism(&hollow, behind, 2.0 * thickness, *open_face, tolerance)?;
        let opened = boolean_with(&hollow, &opening, BooleanOp::Union, tolerance, &options)
            .map_err(|error| KernelError::FaceOpBoolean {
                operation: OPERATION,
                error: Box::new(error),
            })?;
        let mut faces = Vec::with_capacity(opened.faces.len());
        for source in &opened.faces {
            faces.push(match source.operand {
                Operand::A => inner_faces
                    .get(source.face.0 as usize)
                    .copied()
                    .ok_or_else(|| refuse(OPERATION, vec![*open_face], None, LOST))?,
                Operand::B => *open_face,
            });
        }
        inner_edges = opened
            .edges
            .iter()
            .map(|source| match source {
                EdgeSource::Kept {
                    operand: Operand::A,
                    edge,
                } => inner_edges.get(edge.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        inner_vertices = opened
            .vertices
            .iter()
            .map(|source| match source {
                VertexSource::Kept {
                    operand: Operand::A,
                    vertex,
                } => inner_vertices.get(vertex.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        inner_faces = faces;
        hollow = opened.body;
    }
    let cut =
        boolean_with(body, &hollow, BooleanOp::Subtract, tolerance, &options).map_err(|error| {
            KernelError::FaceOpBoolean {
                operation: OPERATION,
                error: Box::new(error),
            }
        })?;
    let inner = |face: FaceId| inner_faces.get(face.0 as usize).copied();
    let mut faces: Vec<(Skin, FaceId)> = Vec::with_capacity(cut.faces.len());
    for source in &cut.faces {
        let face = match source.operand {
            Operand::A => source.face,
            Operand::B => inner(source.face).ok_or(KernelError::FaceOp {
                operation: OPERATION,
                faces: Vec::new(),
                edge: None,
                reason: LOST,
            })?,
        };
        faces.push((skin(source.operand), face));
    }
    // What a shell is.
    for index in 0..body.faces.len() {
        let Ok(face) = u32::try_from(index).map(FaceId) else {
            continue;
        };
        let has = |which: Skin| faces.contains(&(which, face));
        if open.contains(&face) {
            // The hollow comes out through it: the face is gone, or the
            // hollow cuts a hole in it. (Behind an open face between
            // walls that stand round it, as the floor of a pocket, the
            // hollow is wider than the opening, and a piece of the face
            // moved out is the hollow's ceiling there.)
            let crossed = cut
                .edges
                .iter()
                .any(|source| matches!(source, EdgeSource::Intersection { a, .. } if *a == face));
            if has(Skin::Outer) && !crossed {
                return Err(refuse(
                    OPERATION,
                    vec![face],
                    None,
                    "the hollow does not come out through this open face: it is closed off behind it",
                ));
            }
        } else if !has(Skin::Outer) || (!has(Skin::Inner) && !gone.contains(&face)) {
            return Err(refuse(
                OPERATION,
                vec![face],
                None,
                "the wall behind this face did not come out whole: the hollow cuts through it or misses it",
            ));
        }
    }
    // An edge or a vertex of the hollow is the one of the solid it lies
    // behind, or none where blends vanished (the edge they collapsed
    // into, named by its faces).
    let edges = cut
        .edges
        .iter()
        .map(|source| match source {
            EdgeSource::Kept {
                operand: Operand::A,
                edge,
            } => Some((Skin::Outer, *edge)),
            EdgeSource::Kept {
                operand: Operand::B,
                edge,
            } => inner_edges
                .get(edge.0 as usize)
                .copied()
                .flatten()
                .map(|e| (Skin::Inner, e)),
            EdgeSource::Intersection { .. } => None,
        })
        .collect();
    let vertices = cut
        .vertices
        .iter()
        .map(|source| match source {
            VertexSource::Kept {
                operand: Operand::A,
                vertex,
            } => Some((Skin::Outer, *vertex)),
            VertexSource::Kept {
                operand: Operand::B,
                vertex,
            } => inner_vertices
                .get(vertex.0 as usize)
                .copied()
                .flatten()
                .map(|v| (Skin::Inner, v)),
            VertexSource::New { .. } => None,
        })
        .collect();
    Ok(Shelled {
        body: cut.body,
        faces,
        edges,
        vertices,
    })
}
