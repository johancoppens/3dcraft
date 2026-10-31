//! Two sheets sewn along the boundary edges they share, without
//! intersecting them (`feature.boolean {"op": "sew"}`, 2026-10-31).
//!
//! A union of two sheets ([`super::two_sheets`]) intersects every pair of
//! their faces first, so that it can refuse sheets that cross. Sheets
//! that meet tangentially along an edge they share (a side held in
//! curvature to a crown, a blend to its faces) cannot be intersected
//! there: the surfaces touch along the whole edge, and the marcher
//! refuses ("tangent or nearly coincident over too large an area"). The
//! sewing here takes the edges as they are: the faces of both sheets go
//! into one body, and `craft3d_topo::sew` joins the free edges that run
//! between the same places within the tolerance, the two uses against
//! each other (a consistent orientation), proving every joined edge on
//! both faces' surfaces over its whole curve. Nothing is moved and no
//! face is cut.
//!
//! What it does not look at is whether the faces also pass through each
//! other away from those edges; regeneration holds every boolean's result
//! to `selfcheck::crossing_near` (where pieces of the two operands can
//! meet), which refuses a sewn body that passes through itself there.
//!
//! The result: a sheet, or a solid when every shell closes and encloses
//! a positive volume (as a union of sheets that closes,
//! [`super::two_sheets::closed_union`]). Refused: an operand that is not
//! a sheet, an operand with loose entities, sheets that share no edge, a
//! gap beyond the tolerance, a closed shell facing inward, and a result
//! the checker does not pass.

use super::{
    BooleanResult, EdgeSource, FaceSource, Operand, Stop, VertexSource, checked, two_sheets,
};
use crate::{KernelError, MAX_REPORTED};
use craft3d_math::Tolerance;
use craft3d_topo::{Body, BodyKind, Coedge, EdgeId, FaceId, Loop, Shell, VertexId};

const OPERATION: &str = "sew";

fn refused(reason: &'static str) -> KernelError {
    KernelError::Unsupported {
        operation: OPERATION,
        reason,
        faces: Vec::new(),
    }
}

/// The disjoint union of `a` and `b` in one sheet: `a`'s entities first,
/// with their ids, then `b`'s, each id moved past `a`'s.
fn both(a: &Body, b: &Body) -> Option<Body> {
    let shift = |n: usize| u32::try_from(n).ok();
    let (dv, de, dl, df) = (
        shift(a.vertices.len())?,
        shift(a.edges.len())?,
        shift(a.loops.len())?,
        shift(a.faces.len())?,
    );
    let mut out = a.clone();
    out.kind = BodyKind::Sheet;
    out.vertices.extend(b.vertices.iter().cloned());
    for edge in &b.edges {
        let mut edge = edge.clone();
        edge.start = VertexId(edge.start.0.checked_add(dv)?);
        edge.end = VertexId(edge.end.0.checked_add(dv)?);
        out.edges.push(edge);
    }
    for l in &b.loops {
        let coedges = l
            .coedges
            .iter()
            .map(|c| {
                Some(Coedge {
                    edge: EdgeId(c.edge.0.checked_add(de)?),
                    reversed: c.reversed,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        out.loops.push(Loop { coedges });
    }
    for face in &b.faces {
        let mut face = face.clone();
        for l in &mut face.loops {
            l.0 = l.0.checked_add(dl)?;
        }
        out.faces.push(face);
    }
    for shell in &b.shells {
        let faces = shell
            .faces
            .iter()
            .map(|f| Some(FaceId(f.0.checked_add(df)?)))
            .collect::<Option<Vec<_>>>()?;
        out.shells.push(Shell { faces });
    }
    Some(out)
}

/// `a` and `b`, two sheets, sewn along the boundary edges they share
/// (see the module documentation). Faces of the result are the faces of
/// `a` and then of `b`, in order, each with its surface and side; an edge
/// both sheets had is `a`'s.
pub fn sew_sheets(a: &Body, b: &Body, tolerance: &Tolerance) -> Result<BooleanResult, KernelError> {
    if a.kind != BodyKind::Sheet || b.kind != BodyKind::Sheet {
        return Err(refused(
            "sewing joins two sheets along edges they share; a solid has no free edge (unite solids with union)",
        ));
    }
    if a.is_tolerant() || b.is_tolerant() {
        return Err(refused(
            "a sheet with entities looser than the document's tolerance is not sewn yet (owed: sewing at the operands' largest tolerance, as the booleans work on loose bodies)",
        ));
    }
    for operand in [a, b] {
        let defects = checked(operand, tolerance);
        if !defects.is_empty() {
            return Err(KernelError::InvalidBody {
                operation: OPERATION,
                defects: defects.into_iter().take(MAX_REPORTED).collect(),
            });
        }
    }
    let joined = both(a, b).ok_or(refused("the sheets are too large to be sewn into one body"))?;
    // Edges must coincide within the tolerance, and nothing looser is
    // accepted: the cap is the tolerance itself.
    let sewn = craft3d_topo::sew(&joined, tolerance, tolerance.linear, tolerance.linear).map_err(
        |e| match e {
            craft3d_topo::SewError::Gap { .. } => refused(
                "the sheets' edges lie further apart than the tolerance where they would be sewn",
            ),
            craft3d_topo::SewError::TooMany { .. } => {
                refused("too many free edges meet in one place to sew them")
            }
            _ => refused("the sheets could not be sewn (a dangling entity or a bad distance)"),
        },
    )?;
    let (na_edges, na_vertices, na_faces) = (a.edges.len(), a.vertices.len(), a.faces.len());
    // Some edge of `a` and some edge of `b` became one.
    let mut owner: Vec<Option<EdgeSource>> = vec![None; sewn.body.edges.len()];
    let mut shared = false;
    for (index, (into, _)) in sewn.edges.iter().enumerate() {
        let source = if index < na_edges {
            EdgeSource::Kept {
                operand: Operand::A,
                edge: EdgeId(u32::try_from(index).unwrap_or(u32::MAX)),
            }
        } else {
            EdgeSource::Kept {
                operand: Operand::B,
                edge: EdgeId(u32::try_from(index - na_edges).unwrap_or(u32::MAX)),
            }
        };
        let Some(slot) = owner.get_mut(into.0 as usize) else {
            return Err(refused("sewing lost track of an edge; this is a bug"));
        };
        match slot {
            None => *slot = Some(source),
            Some(EdgeSource::Kept {
                operand: Operand::A,
                ..
            }) if index >= na_edges => shared = true,
            Some(_) => {}
        }
    }
    if !shared {
        return Err(refused(
            "the sheets share no boundary edge: none of their free edges run between the same places within the tolerance, used against each other (sheets that face opposite ways across an edge are not sewn: turn one)",
        ));
    }
    let edges = owner
        .into_iter()
        .map(|s| {
            s.ok_or(refused(
                "sewing left an edge without a source; this is a bug",
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut vertex_owner: Vec<Option<VertexSource>> = vec![None; sewn.body.vertices.len()];
    for (index, into) in sewn.vertices.iter().enumerate() {
        let source = if index < na_vertices {
            VertexSource::Kept {
                operand: Operand::A,
                vertex: VertexId(u32::try_from(index).unwrap_or(u32::MAX)),
            }
        } else {
            VertexSource::Kept {
                operand: Operand::B,
                vertex: VertexId(u32::try_from(index - na_vertices).unwrap_or(u32::MAX)),
            }
        };
        let Some(slot) = vertex_owner.get_mut(into.0 as usize) else {
            return Err(refused("sewing lost track of a vertex; this is a bug"));
        };
        if slot.is_none() {
            *slot = Some(source);
        }
    }
    let vertices = vertex_owner
        .into_iter()
        .map(|s| {
            s.ok_or(refused(
                "sewing left a vertex without a source; this is a bug",
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let faces: Vec<FaceSource> = (0..sewn.body.faces.len())
        .map(|i| {
            let (operand, face) = if i < na_faces {
                (Operand::A, i)
            } else {
                (Operand::B, i - na_faces)
            };
            FaceSource {
                operand,
                face: FaceId(u32::try_from(face).unwrap_or(u32::MAX)),
            }
        })
        .collect();
    let mut body = sewn.body;
    body.shells.retain(|s| !s.faces.is_empty());
    two_sheets::closed_union(&mut body).map_err(|stop| match stop {
        Stop::Unsupported { reason, .. } => refused(reason),
        Stop::Error(e) => e,
    })?;
    let defects = checked(&body, tolerance);
    if !defects.is_empty() {
        return Err(KernelError::InvalidBody {
            operation: OPERATION,
            defects: defects.into_iter().take(MAX_REPORTED).collect(),
        });
    }
    Ok(BooleanResult {
        body,
        merged: faces.iter().map(|f| vec![*f]).collect(),
        faces,
        edges,
        vertices,
        raised: Vec::new(),
    })
}
