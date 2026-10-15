//! Modelling operations on B-rep bodies. Each is a pure function that
//! returns a body which passes `craft3d_topo::check`, or an error that says
//! why it cannot (docs/architecture.md §5.3).
//!
//! So far: primitive solids, closed profiles from sketch curves
//! ([`profile`]), their extrusion ([`extrude`]), their revolution
//! ([`revolve`]), the solid through a row of them ([`loft`]), the solid one
//! of them sweeps along a path ([`sweep`]), booleans ([`boolean`]),
//! chamfers ([`chamfer`]), fillets ([`fillet`]) and the face operations
//! ([`offset_face`], [`shell`]).

mod blend;
pub mod boolean;
pub mod chamfer;
pub mod delete_face;
pub mod draft;
pub mod extrude;
pub mod extrude_to;
pub mod fillet;
pub mod helix;
pub mod loft;
pub mod move_face;
pub mod offset_face;
pub mod primitives;
pub mod profile;
pub mod revolve;
pub mod shell;
mod spline2;
pub mod sweep;
pub mod thicken;
pub mod thread;
mod tweak;
pub mod work;

use craft3d_geom::GeomError;
use craft3d_topo::Defect;
use std::fmt;

/// The most curve ids, or defects, an error lists; the rest are left out.
pub const MAX_REPORTED: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    /// A size that must be positive and finite is not.
    InvalidSize {
        name: &'static str,
        value: f64,
    },
    Geometry(GeomError),
    /// There is nothing to make a profile from, or nothing to extrude.
    NoProfile,
    /// More curves than profile detection accepts.
    TooManyCurves {
        count: usize,
        limit: usize,
    },
    /// Two curves were given the same id.
    DuplicateCurveId {
        id: u64,
    },
    /// A curve that is not a usable piece of boundary on its own.
    DegenerateCurve {
        id: u64,
        reason: &'static str,
    },
    /// These curves have an end that meets no other curve (at most
    /// [`MAX_REPORTED`] of them, ascending).
    OpenProfile {
        curves: Vec<u64>,
    },
    /// More than two curve ends meet at one place; these curves are
    /// involved (at most [`MAX_REPORTED`], ascending).
    BranchingProfile {
        curves: Vec<u64>,
    },
    /// Two curves cross, touch or overlap away from an end they share; the
    /// same curve twice for a spline that crosses or touches itself.
    CrossingCurves {
        curves: [u64; 2],
    },
    /// A closed loop that encloses no area (at most [`MAX_REPORTED`] of its
    /// curves, ascending).
    ZeroAreaLoop {
        curves: Vec<u64>,
    },
    /// The extrusion does not run from a lower to a higher offset, or is
    /// thinner than the tolerance.
    InvalidOffsets {
        from: f64,
        to: f64,
    },
    /// A profile that `detect_profiles` would not have produced.
    InvalidProfile {
        reason: &'static str,
    },
    /// The axis of a revolution is not a line in the sketch plane.
    InvalidAxis {
        reason: &'static str,
    },
    /// The revolution does not turn from a lower to a higher angle, turns
    /// through more than a full turn, or through too little to move
    /// anything further than the tolerance.
    InvalidAngles {
        from: f64,
        to: f64,
    },
    /// The profiles have points on both sides of the axis of revolution;
    /// these curves reach across to the side with less of them (at most
    /// [`MAX_REPORTED`], ascending).
    CrossesAxis {
        curves: Vec<u64>,
    },
    /// A curve the operation cannot sweep yet.
    UnsupportedCurve {
        id: u64,
        reason: &'static str,
    },
    /// The operation built a body that fails the validity checker (at most
    /// [`MAX_REPORTED`] defects). The input is at the edge of what the
    /// tolerance allows, or this is a bug in the kernel.
    InvalidBody {
        operation: &'static str,
        defects: Vec<Defect>,
    },
    /// The operation does not handle this geometry or this kind of contact
    /// (yet). `reason` is a fixed text per cause; `faces` are the faces
    /// involved (at most [`MAX_REPORTED`]), each with the operand it is a
    /// face of.
    Unsupported {
        operation: &'static str,
        reason: &'static str,
        faces: Vec<(boolean::Operand, craft3d_topo::FaceId)>,
    },
    /// An extrusion up to a face has no end there; `reason` says why (the
    /// face is parallel to the direction, behind the sketch plane, or the
    /// profile misses it).
    NoEnd {
        reason: &'static str,
    },
    /// The operation leaves nothing: an intersection of bodies that do not
    /// overlap, a body minus one that contains it.
    EmptyResult {
        operation: &'static str,
    },
    /// A chamfer distance for which the cut line would leave `face`, one
    /// of the two faces of `edge`: `largest` is the distance at which it
    /// does.
    ChamferTooLarge {
        edge: craft3d_topo::EdgeId,
        face: craft3d_topo::FaceId,
        distance: f64,
        largest: f64,
    },
    /// An edge that cannot be chamfered, or is not built yet, or a chamfer
    /// that did not come out as one: the edge and the face it is about,
    /// when there is one.
    Chamfer {
        edge: Option<craft3d_topo::EdgeId>,
        face: Option<craft3d_topo::FaceId>,
        reason: &'static str,
    },
    /// The boolean refused the tool of a chamfered edge.
    ChamferBoolean {
        edge: craft3d_topo::EdgeId,
        error: Box<KernelError>,
    },
    /// A fillet radius for which the line along which the ball touches
    /// `face`, one of the two faces of `edge`, would leave that face:
    /// `largest` is the radius at which it does.
    FilletTooLarge {
        edge: craft3d_topo::EdgeId,
        face: craft3d_topo::FaceId,
        radius: f64,
        largest: f64,
    },
    /// An edge that cannot be filleted, or is not built yet, or a fillet
    /// that did not come out as one: the edge and the face it is about,
    /// when there is one.
    Fillet {
        edge: Option<craft3d_topo::EdgeId>,
        face: Option<craft3d_topo::FaceId>,
        reason: &'static str,
    },
    /// The boolean refused the tool of a filleted edge.
    FilletBoolean {
        edge: craft3d_topo::EdgeId,
        error: Box<KernelError>,
    },
    /// A face operation (offset face, shell) that is refused:
    /// the faces and the edge it is about, when there are.
    FaceOp {
        operation: &'static str,
        faces: Vec<craft3d_topo::FaceId>,
        edge: Option<craft3d_topo::EdgeId>,
        reason: &'static str,
    },
    /// A face operation whose distance or thickness `value` is so large
    /// that `edge`, or a face of `faces`, would vanish (`reason` says
    /// which and how): `largest` is a size that still fits.
    FaceOpTooFar {
        operation: &'static str,
        faces: Vec<craft3d_topo::FaceId>,
        edge: Option<craft3d_topo::EdgeId>,
        reason: &'static str,
        value: f64,
        largest: f64,
    },
    /// The boolean refused a step of a face operation.
    FaceOpBoolean {
        operation: &'static str,
        error: Box<KernelError>,
    },
    /// A loft cannot be made through these sections (counted from 0;
    /// empty when it is not about particular ones). `reason` counts them
    /// from 1, as the user does.
    Loft {
        sections: Vec<usize>,
        reason: String,
    },
    /// A profile cannot be swept along this path. `curves` are the curves
    /// of the path it is about (empty when it is not about particular
    /// ones); `reason` names them as the sketch does (`e7`).
    Sweep {
        curves: Vec<u64>,
        reason: String,
    },
}

fn id_list(ids: &[u64]) -> String {
    let parts: Vec<String> = ids.iter().map(u64::to_string).collect();
    parts.join(", ")
}

fn face_op_entities(
    f: &mut fmt::Formatter<'_>,
    faces: &[craft3d_topo::FaceId],
    edge: Option<craft3d_topo::EdgeId>,
) -> fmt::Result {
    if let Some(edge) = edge {
        write!(f, " at edge {}", edge.0)?;
    }
    for (index, face) in faces.iter().take(MAX_REPORTED).enumerate() {
        let lead = if index == 0 { " (face " } else { ", " };
        write!(f, "{lead}{}", face.0)?;
    }
    if !faces.is_empty() {
        write!(f, ")")?;
    }
    Ok(())
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::InvalidSize { name, value } => {
                write!(f, "{name} must be a positive, finite length, got {value}")
            }
            KernelError::Geometry(e) => write!(f, "{e}"),
            KernelError::NoProfile => {
                write!(f, "no closed profile: there are no curves to make one from")
            }
            KernelError::TooManyCurves { count, limit } => write!(
                f,
                "{count} curves are more than profile detection accepts ({limit})"
            ),
            KernelError::DuplicateCurveId { id } => {
                write!(f, "two curves have the same id {id}")
            }
            KernelError::DegenerateCurve { id, reason } => {
                write!(f, "curve {id} cannot bound a profile: {reason}")
            }
            KernelError::OpenProfile { curves } => write!(
                f,
                "the profile is not closed: an end of curve {} meets no other curve; \
                 join the ends or remove the curve",
                id_list(curves)
            ),
            KernelError::BranchingProfile { curves } => write!(
                f,
                "more than two curve ends meet at one place (curves {}); \
                 a profile boundary cannot branch",
                id_list(curves)
            ),
            KernelError::CrossingCurves { curves: [a, b] } if a == b => write!(
                f,
                "curve {a} crosses or touches itself, or has a cusp; a profile boundary \
                 is a simple curve"
            ),
            KernelError::CrossingCurves { curves } => write!(
                f,
                "curves {} cross, touch or overlap away from a shared end; \
                 split them there or move them apart",
                id_list(curves)
            ),
            KernelError::ZeroAreaLoop { curves } => write!(
                f,
                "the loop of curves {} encloses no area: it folds back onto itself, \
                 or crosses itself into parts that cancel",
                id_list(curves)
            ),
            KernelError::InvalidOffsets { from, to } => write!(
                f,
                "an extrusion runs from a lower to a higher offset, further apart than \
                 the tolerance; got from {from} to {to}"
            ),
            KernelError::InvalidProfile { reason } => write!(f, "invalid profile: {reason}"),
            KernelError::InvalidAxis { reason } => {
                write!(f, "the axis of revolution is not usable: {reason}")
            }
            KernelError::InvalidAngles { from, to } => write!(
                f,
                "a revolution turns from a lower to a higher angle, through at most a full \
                 turn and far enough to move the profile by more than the tolerance; got \
                 from {from} to {to} radians"
            ),
            KernelError::CrossesAxis { curves } => write!(
                f,
                "the profile crosses the axis of revolution: curves {} reach across it; \
                 keep the profile on one side (it may touch the axis)",
                id_list(curves)
            ),
            KernelError::UnsupportedCurve { id, reason } => {
                write!(f, "curve {id} cannot be revolved yet: {reason}")
            }
            KernelError::InvalidBody { operation, defects } => {
                write!(f, "{operation} produced an invalid body:")?;
                for defect in defects {
                    write!(f, " {defect};")?;
                }
                Ok(())
            }
            KernelError::Unsupported {
                operation,
                reason,
                faces,
            } => {
                write!(f, "{operation} does not support this: {reason}")?;
                for (index, (operand, face)) in faces.iter().enumerate() {
                    let body = match operand {
                        boolean::Operand::A => "first",
                        boolean::Operand::B => "second",
                    };
                    let lead = if index == 0 { " (" } else { ", " };
                    write!(f, "{lead}face {} of the {body} body", face.0)?;
                }
                if !faces.is_empty() {
                    write!(f, ")")?;
                }
                Ok(())
            }
            KernelError::NoEnd { reason } => write!(f, "the extrusion has no end: {reason}"),
            KernelError::EmptyResult { operation } => {
                write!(f, "{operation} leaves nothing: the result is empty")
            }
            KernelError::ChamferTooLarge {
                edge,
                face,
                distance,
                largest,
            } => write!(
                f,
                "the chamfer of edge {} is too large for face {}: the cut line at {distance} \
                 leaves the face; the distance has to stay below {largest}",
                edge.0, face.0
            ),
            KernelError::Chamfer { edge, face, reason } => {
                write!(f, "chamfer")?;
                if let Some(edge) = edge {
                    write!(f, " of edge {}", edge.0)?;
                }
                if let Some(face) = face {
                    write!(f, " (face {})", face.0)?;
                }
                write!(f, ": {reason}")
            }
            KernelError::ChamferBoolean { edge, error } => {
                write!(f, "chamfer of edge {}: {error}", edge.0)
            }
            KernelError::FilletTooLarge {
                edge,
                face,
                radius,
                largest,
            } => write!(
                f,
                "the fillet of edge {} is too large for face {}: with a radius of {radius} the \
                 line where it touches the face leaves the face; the radius has to stay below \
                 {largest}",
                edge.0, face.0
            ),
            KernelError::Fillet { edge, face, reason } => {
                write!(f, "fillet")?;
                if let Some(edge) = edge {
                    write!(f, " of edge {}", edge.0)?;
                }
                if let Some(face) = face {
                    write!(f, " (face {})", face.0)?;
                }
                write!(f, ": {reason}")
            }
            KernelError::FilletBoolean { edge, error } => {
                write!(f, "fillet of edge {}: {error}", edge.0)
            }
            KernelError::FaceOp {
                operation,
                faces,
                edge,
                reason,
            } => {
                write!(f, "{operation}")?;
                face_op_entities(f, faces, *edge)?;
                write!(f, ": {reason}")
            }
            KernelError::FaceOpTooFar {
                operation,
                faces,
                edge,
                reason,
                value,
                largest,
            } => {
                write!(f, "{operation}")?;
                face_op_entities(f, faces, *edge)?;
                write!(
                    f,
                    ": {value} is too far: {reason}; the largest that fits is below {largest}"
                )
            }
            KernelError::FaceOpBoolean { operation, error } => {
                write!(f, "{operation}: {error}")
            }
            KernelError::Loft { reason, .. } | KernelError::Sweep { reason, .. } => {
                write!(f, "{reason}")
            }
        }
    }
}

impl std::error::Error for KernelError {}

impl From<GeomError> for KernelError {
    fn from(e: GeomError) -> Self {
        KernelError::Geometry(e)
    }
}
