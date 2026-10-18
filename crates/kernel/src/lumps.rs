//! The shells of a solid sorted into lumps: each lump is one outer shell
//! and the cavities in it.
//!
//! One rule for every solid, however its shells came about (a boolean that
//! cut a body in two or left a void, a shell or an offset of a body with a
//! cavity, a pattern, a file): a shell that encloses a positive volume
//! (its faces point away from what it encloses) is the outside of a lump,
//! one that encloses a negative volume is a cavity. The volumes come from
//! one integration of the body's faces summed per shell
//! (`craft3d_topo::shell_volumes`), and a volume within the integrator's
//! own bound of zero has no sign to read.
//!
//! Which shells lie inside which is told by a few points of one shell
//! classified against each other shell alone, by the ray casting the
//! booleans use, each other shell prepared once with the points of every
//! shell that can be in it; a shell with a point outside the box of
//! another is not in it, at no cost of rays. This assumes that shells do
//! not cross, which is the self-intersection check's to find
//! (`tess::body_self_intersections`), not proven here: a point on another
//! shell is refused as shells that meet, and points of one shell that
//! disagree are refused as unsupported. Then:
//!
//! - every shell lies inside an even number of others when it is the
//!   outside of a lump and an odd number when it is a cavity (a lump may
//!   sit in a cavity of another, a cavity in a lump in a cavity, …);
//! - the shell a shell lies in most deeply (the one it is in that is in
//!   the most others) is the other kind: a cavity is in the material of a
//!   lump, never in another cavity's void or outside every lump;
//! - a cavity belongs to that lump.
//!
//! What breaks this is not a solid: `KernelError::InvalidBody` with
//! `Defect::ShellNesting` and the shell.

use crate::KernelError;
use crate::boolean::{Operand, Place, classify_points};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{
    Body, BodyKind, Defect, FaceId, MassError, ShellId, ShellVolume, shell_alone, shell_volumes,
};

const OPERATION: &str = "sorting shells into lumps";

/// How many points of a shell vote on where it lies.
const VOTERS: usize = 8;

/// One lump of a solid: the shell that bounds it from outside and the
/// cavities in it, by index into the body's shells, each in index order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lump {
    pub outer: ShellId,
    pub cavities: Vec<ShellId>,
}

fn shell_id(s: usize) -> ShellId {
    ShellId(u32::try_from(s).unwrap_or(u32::MAX))
}

fn nesting(shell: usize, reason: String) -> KernelError {
    KernelError::InvalidBody {
        operation: OPERATION,
        defects: vec![Defect::ShellNesting {
            shell: shell_id(shell),
            reason,
        }],
    }
}

/// Unsupported, naming the first face of shell `shell`.
fn unsupported(body: &Body, shell: usize, reason: &'static str) -> KernelError {
    KernelError::Unsupported {
        operation: OPERATION,
        reason,
        faces: body
            .shells
            .get(shell)
            .and_then(|s| s.faces.first())
            .map(|&f: &FaceId| vec![(Operand::A, f)])
            .unwrap_or_default(),
    }
}

/// Whether each shell of the solid `body` is the outside of a lump (its
/// volume, `volumes[s]` as [`craft3d_topo::shell_volumes`] gives it, is
/// positive) or a cavity (negative).
///
/// **Errors.** [`KernelError::ShellUnmeasured`] for a shell that could not
/// be measured, with its index and why; [`KernelError::Unsupported`] for a
/// volume within its bound of zero (or not finite), whose sign cannot be
/// told; [`KernelError::InvalidBody`] for a list that does not fit the
/// shells.
pub fn outer_shells(
    body: &Body,
    volumes: &[Result<ShellVolume, MassError>],
) -> Result<Vec<bool>, KernelError> {
    if volumes.len() != body.shells.len() {
        return Err(nesting(
            0,
            format!("{} volumes for {} shells", volumes.len(), body.shells.len()),
        ));
    }
    volumes
        .iter()
        .enumerate()
        .map(|(s, measured)| {
            let measured = measured
                .as_ref()
                .map_err(|error| KernelError::ShellUnmeasured {
                    operation: OPERATION,
                    shell: shell_id(s),
                    error: error.clone(),
                })?;
            if !(measured.volume.is_finite() && measured.volume.abs() > measured.bound) {
                return Err(unsupported(
                    body,
                    s,
                    "the volume a shell encloses is within the measure's own bound of zero, so whether it is a lump or a cavity cannot be told",
                ));
            }
            Ok(measured.volume > 0.0)
        })
        .collect()
}

/// The lumps of the solid `body`, in the order of their outer shells.
///
/// A body of one shell is one lump without cavities, and is not measured:
/// a shell turned inside out is a defect of the operation that made it,
/// not something to sort.
///
/// **Errors.** [`KernelError::InvalidBody`] (with
/// [`Defect::ShellNesting`]) for a sheet, shells that meet, and shells
/// that do not nest as lumps and cavities do;
/// [`KernelError::ShellUnmeasured`] and [`KernelError::Unsupported`] (as
/// [`outer_shells`], and when the points of a shell cannot tell whether it
/// lies in another) when the sorting cannot be done.
pub fn lumps(body: &Body, tolerance: &Tolerance) -> Result<Vec<Lump>, KernelError> {
    if body.kind == BodyKind::Solid && body.shells.len() > 1 {
        lumps_measured(body, tolerance, &shell_volumes(body))
    } else {
        lumps_measured(body, tolerance, &[])
    }
}

/// [`lumps`] with the shells' volumes given (`volumes[s]`, as
/// [`craft3d_topo::shell_volumes`] or `mass_with_shells` gives them; not
/// looked at for a body of one shell).
pub fn lumps_measured(
    body: &Body,
    tolerance: &Tolerance,
    volumes: &[Result<ShellVolume, MassError>],
) -> Result<Vec<Lump>, KernelError> {
    if body.kind != BodyKind::Solid {
        return Err(nesting(0, "a sheet has no lumps".to_string()));
    }
    if body.shells.len() <= 1 {
        return Ok((0..body.shells.len())
            .map(|_| Lump {
                outer: ShellId(0),
                cavities: Vec::new(),
            })
            .collect());
    }
    let outer = outer_shells(body, volumes)?;
    struct Part {
        body: Body,
        lo: Vec3,
        hi: Vec3,
        points: Vec<Vec3>,
    }
    let mut parts = Vec::with_capacity(body.shells.len());
    for s in 0..body.shells.len() {
        let alone = shell_alone(body, shell_id(s)).ok_or_else(|| {
            nesting(
                s,
                "refers to a face, loop, edge or vertex that does not exist".to_string(),
            )
        })?;
        // A box that certainly holds the shell: a point of another shell
        // outside it is outside the shell. (Not the other way round: boxes
        // of curved faces reach past them, so a shell inside another may
        // have a box that is not inside the other's.)
        let (lo, hi) = alone
            .bbox()
            .ok()
            .and_then(|b| b.corners())
            .ok_or_else(|| nesting(s, "has no bounded box".to_string()))?;
        let points = points_of(&alone);
        parts.push(Part {
            body: alone,
            lo,
            hi,
            points,
        });
    }
    // inside[s]: the shells s lies inside. Each other shell t is prepared
    // once, for the points of every shell its box holds.
    let margin = 2.0 * tolerance.linear;
    let mut inside: Vec<Vec<usize>> = vec![Vec::new(); parts.len()];
    for (t, other) in parts.iter().enumerate() {
        let held = |p: &Vec3| {
            p.x >= other.lo.x - margin
                && p.y >= other.lo.y - margin
                && p.z >= other.lo.z - margin
                && p.x <= other.hi.x + margin
                && p.y <= other.hi.y + margin
                && p.z <= other.hi.z + margin
        };
        let candidates: Vec<usize> = parts
            .iter()
            .enumerate()
            .filter(|&(s, part)| s != t && part.points.iter().all(held))
            .map(|(s, _)| s)
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let points: Vec<Vec3> = candidates
            .iter()
            .filter_map(|&s| parts.get(s))
            .flat_map(|part| part.points.iter().copied())
            .collect();
        let found = classify_points(&other.body, &points, tolerance)?;
        let mut at = 0_usize;
        for &s in &candidates {
            let count = parts.get(s).map_or(0, |p| p.points.len());
            let votes = found.get(at..at.saturating_add(count)).unwrap_or_default();
            at = at.saturating_add(count);
            let (mut yes, mut no) = (0_usize, 0_usize);
            for place in votes {
                match place {
                    Place::Inside => yes += 1,
                    Place::Outside => no += 1,
                    Place::OnBoundary => {
                        return Err(nesting(
                            s,
                            format!(
                                "meets shell {t}: a point of it lies on that shell, and shells that meet are not lumps and cavities"
                            ),
                        ));
                    }
                    Place::Unknown => {}
                }
            }
            if yes > 0 && no > 0 {
                return Err(unsupported(
                    body,
                    s,
                    "points of a shell disagree on whether it lies inside another shell (the shells may cross)",
                ));
            }
            if yes == 0 && no == 0 {
                return Err(unsupported(
                    body,
                    s,
                    "no point of a shell can be told inside or outside another shell",
                ));
            }
            if yes > 0
                && let Some(list) = inside.get_mut(s)
            {
                list.push(t);
            }
        }
    }
    let depth = |s: usize| inside.get(s).map_or(0, Vec::len);
    let is_outer = |s: usize| outer.get(s).copied().unwrap_or(false);
    // The shell each lies in most deeply.
    let parent = |s: usize| {
        inside
            .get(s)
            .into_iter()
            .flatten()
            .copied()
            .max_by_key(|&t| (depth(t), std::cmp::Reverse(t)))
    };
    let mut lumps: Vec<Lump> = Vec::new();
    let mut owner = vec![None; parts.len()];
    for s in 0..parts.len() {
        let outward = is_outer(s);
        if outward != (depth(s) % 2 == 0) {
            return Err(nesting(
                s,
                if outward {
                    format!(
                        "faces outward but lies inside {} other shells, which puts it in the material of a lump or in a cavity's void the wrong way round",
                        depth(s)
                    )
                } else if depth(s) == 0 {
                    "is a cavity (its faces point into what it encloses) that lies in no part of the body"
                        .to_string()
                } else {
                    format!(
                        "is a cavity (its faces point into what it encloses) but lies inside {} other shells, which puts it in another cavity's void",
                        depth(s)
                    )
                },
            ));
        }
        if let Some(t) = parent(s)
            && (is_outer(t) == outward || depth(t) + 1 != depth(s))
        {
            return Err(nesting(
                s,
                format!(
                    "lies in shell {t}, which is not the outside of a lump with a cavity there nor a cavity a lump can sit in"
                ),
            ));
        }
        if outward {
            if let Some(slot) = owner.get_mut(s) {
                *slot = Some(lumps.len());
            }
            lumps.push(Lump {
                outer: shell_id(s),
                cavities: Vec::new(),
            });
        }
    }
    // Cavities after every outer shell has its lump: a cavity's lump may
    // come later in the order of the shells.
    for s in (0..parts.len()).filter(|&s| !is_outer(s)) {
        let lump = parent(s)
            .and_then(|t| owner.get(t).copied().flatten())
            .and_then(|l| lumps.get_mut(l))
            .ok_or_else(|| nesting(s, "is a cavity that lies in no lump".to_string()))?;
        lump.cavities.push(shell_id(s));
    }
    Ok(lumps)
}

/// Points on a shell (given as a body of its own) that vote on where it
/// lies: some of its vertices, and a point of each face without loops
/// (a whole sphere or torus has no vertex).
fn points_of(shell: &Body) -> Vec<Vec3> {
    let mut points: Vec<Vec3> = shell
        .faces
        .iter()
        .filter(|face| face.loops.is_empty())
        .map(|face| face.surface.eval(0.0, 0.0))
        .collect();
    let n = shell.vertices.len();
    // Spread over the shell, not the first few, which one face may hold.
    let step = (n / VOTERS).max(1);
    points.extend(
        shell
            .vertices
            .iter()
            .step_by(step)
            .take(VOTERS)
            .map(|v| v.point),
    );
    points
}
