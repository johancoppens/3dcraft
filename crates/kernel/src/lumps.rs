//! The shells of a solid sorted into lumps: each lump is one outer shell
//! and the cavities in it.
//!
//! One rule for every solid, however its shells came about (a boolean that
//! cut a body in two or left a void, a shell or an offset of a body with a
//! cavity, a pattern, a file): a shell that encloses a positive volume
//! (its faces point away from what it encloses) is the outside of a lump,
//! one that encloses a negative volume is a cavity. Which shells lie
//! inside which is told by points of one shell classified against each
//! other shell alone, by the ray casting the booleans use; in a valid
//! solid shells do not meet, so every point of a shell gives the same
//! answer, and the vote of the points that can tell is a guard against a
//! point that happens to lie on the other shell. A shell whose box is not
//! inside another's is inside nothing, at no cost. Then:
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
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Defect, ShellId, mass_properties, shell_alone};

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

/// The lumps of the solid `body`, in the order of their outer shells.
///
/// A body of one shell is one lump without cavities, and is not measured:
/// a shell turned inside out is a defect of the operation that made it,
/// not something to sort.
///
/// **Errors.** [`KernelError::InvalidBody`] (with
/// [`Defect::ShellNesting`]) for a sheet, a shell that encloses no volume,
/// and shells that do not nest as lumps and cavities do;
/// [`KernelError::Unsupported`] when a shell cannot be measured or no
/// point of it can be told inside or outside another.
pub fn lumps(body: &Body, tolerance: &Tolerance) -> Result<Vec<Lump>, KernelError> {
    let nesting = |shell: usize, reason: String| KernelError::InvalidBody {
        operation: OPERATION,
        defects: vec![Defect::ShellNesting {
            shell: ShellId(u32::try_from(shell).unwrap_or(u32::MAX)),
            reason,
        }],
    };
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
    struct Part {
        body: Body,
        volume: f64,
        lo: Vec3,
        hi: Vec3,
        points: Vec<Vec3>,
    }
    let mut parts = Vec::with_capacity(body.shells.len());
    for s in 0..body.shells.len() {
        let id = ShellId(u32::try_from(s).unwrap_or(u32::MAX));
        let alone = shell_alone(body, id).ok_or_else(|| {
            nesting(
                s,
                "refers to a face, loop, edge or vertex that does not exist".to_string(),
            )
        })?;
        let volume = mass_properties(&alone)
            .map_err(|_| KernelError::Unsupported {
                operation: OPERATION,
                reason: "the volume a shell encloses cannot be measured",
                faces: Vec::new(),
            })?
            .volume;
        if !volume.is_finite() || volume.abs() <= tolerance.linear.powi(3) {
            return Err(nesting(
                s,
                "encloses no volume, so it is neither the outside of a lump nor a cavity"
                    .to_string(),
            ));
        }
        let (lo, hi) = alone
            .held_bbox()
            .ok()
            .and_then(|b| b.corners())
            .ok_or_else(|| nesting(s, "has no bounded box".to_string()))?;
        let points = points_of(&alone);
        parts.push(Part {
            body: alone,
            volume,
            lo,
            hi,
            points,
        });
    }
    // inside[s]: the shells s lies inside.
    let margin = 2.0 * tolerance.linear;
    let mut inside: Vec<Vec<usize>> = vec![Vec::new(); parts.len()];
    for (s, part) in parts.iter().enumerate() {
        for (t, other) in parts.iter().enumerate() {
            let within = |a: Vec3, b: Vec3| {
                a.x >= b.x - margin && a.y >= b.y - margin && a.z >= b.z - margin
            };
            let boxed = s != t && within(part.lo, other.lo) && within(other.hi, part.hi);
            if !boxed {
                continue;
            }
            let found = crate::boolean::contains_points(&other.body, &part.points, tolerance)?;
            let (yes, no) = found.iter().fold((0, 0), |(yes, no), f| match f {
                Some(true) => (yes + 1, no),
                Some(false) => (yes, no + 1),
                None => (yes, no),
            });
            if yes == no {
                return Err(KernelError::Unsupported {
                    operation: OPERATION,
                    reason: "no point of a shell can be told inside or outside another shell",
                    faces: Vec::new(),
                });
            }
            if yes > no
                && let Some(list) = inside.get_mut(s)
            {
                list.push(t);
            }
        }
    }
    let depth = |s: usize| inside.get(s).map_or(0, Vec::len);
    let mut lumps: Vec<Lump> = Vec::new();
    let mut owner = vec![None; parts.len()];
    for (s, part) in parts.iter().enumerate() {
        let outer = part.volume > 0.0;
        if outer != (depth(s) % 2 == 0) {
            return Err(nesting(
                s,
                if outer {
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
        // The shell it lies in most deeply.
        let parent = inside
            .get(s)
            .into_iter()
            .flatten()
            .copied()
            .max_by_key(|&t| (depth(t), std::cmp::Reverse(t)));
        if let Some(t) = parent {
            let parent_outer = parts.get(t).is_some_and(|p| p.volume > 0.0);
            if parent_outer == outer || depth(t) + 1 != depth(s) {
                return Err(nesting(
                    s,
                    format!(
                        "lies in shell {t}, which is not the outside of a lump with a cavity there nor a cavity a lump can sit in"
                    ),
                ));
            }
        }
        if outer {
            if let Some(slot) = owner.get_mut(s) {
                *slot = Some(lumps.len());
            }
            lumps.push(Lump {
                outer: ShellId(u32::try_from(s).unwrap_or(u32::MAX)),
                cavities: Vec::new(),
            });
        }
    }
    // Cavities after every outer shell has its lump: a cavity's lump may
    // come later in the order of the shells.
    for s in 0..parts.len() {
        if parts.get(s).is_some_and(|p| p.volume > 0.0) {
            continue;
        }
        let parent = inside
            .get(s)
            .into_iter()
            .flatten()
            .copied()
            .max_by_key(|&t| (depth(t), std::cmp::Reverse(t)));
        let lump = parent
            .and_then(|t| owner.get(t).copied().flatten())
            .and_then(|l| lumps.get_mut(l))
            .ok_or_else(|| nesting(s, "is a cavity that lies in no lump".to_string()))?;
        lump.cavities
            .push(ShellId(u32::try_from(s).unwrap_or(u32::MAX)));
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
