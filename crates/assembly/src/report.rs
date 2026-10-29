//! Messages for people: diagnoses and free motions in words, naming the
//! instances and mates by the problem's names.

use crate::types::{Diagnosis, FreeMotion, Group, Motion, Problem};
use craft3d_math::Vec3;

/// A unit direction as a world axis when it is one (`x`, `-z`, …), else
/// its components.
fn direction(v: Vec3) -> String {
    let axes = [
        (Vec3::X, "x"),
        (Vec3::Y, "y"),
        (Vec3::Z, "z"),
        (-Vec3::X, "-x"),
        (-Vec3::Y, "-y"),
        (-Vec3::Z, "-z"),
    ];
    for (a, name) in axes {
        if v.dot(a) > 1.0 - 1e-9 {
            return name.to_string();
        }
    }
    format!("({:.4}, {:.4}, {:.4})", v.x, v.y, v.z)
}

fn point(v: Vec3) -> String {
    format!("({:.6}, {:.6}, {:.6})", v.x, v.y, v.z)
}

impl Group {
    /// The group in words.
    pub fn describe(&self, p: &Problem) -> String {
        let var = |m: usize, v: usize| {
            p.mates
                .get(m)
                .and_then(|s| s.kind.variables().get(v))
                .map_or("variable", |k| k.label())
        };
        match *self {
            Group::Mate(m) => p.mate_name(m),
            Group::Coupling(c) => format!("coupling {c}"),
            Group::Driver { mate, variable } => {
                format!(
                    "the driver of {}.{}",
                    p.mate_name(mate),
                    var(mate, variable)
                )
            }
            Group::Limit { mate, variable } => {
                format!("the limit of {}.{}", p.mate_name(mate), var(mate, variable))
            }
        }
    }
}

impl FreeMotion {
    /// The motion in words, one clause per instance it moves: "Bar[3] can
    /// still turn about z through (…)".
    pub fn describe(&self, p: &Problem) -> String {
        let parts: Vec<String> = self
            .instances
            .iter()
            .map(|im| {
                let name = p.instance_name(im.instance);
                match im.motion {
                    Motion::Slide { direction: d } => {
                        format!("{name} can still slide along {}", direction(d))
                    }
                    Motion::Turn {
                        axis,
                        point: at,
                        pitch,
                    } if pitch.abs() <= 1e-9 * p.scale => {
                        format!(
                            "{name} can still turn about {} through {}",
                            direction(axis),
                            point(at)
                        )
                    }
                    Motion::Turn {
                        axis,
                        point: at,
                        pitch,
                    } => format!(
                        "{name} can still screw about {} through {} ({pitch:.6} per radian)",
                        direction(axis),
                        point(at)
                    ),
                }
            })
            .collect();
        if parts.is_empty() {
            "only mate variables can still change".to_string()
        } else {
            parts.join(", together with: ")
        }
    }
}

impl Diagnosis {
    /// The diagnosis in words, naming instances and mates by `p`'s names.
    pub fn message(&self, p: &Problem) -> String {
        let list = |groups: &[Group]| -> String {
            groups
                .iter()
                .map(|g| g.describe(p))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Diagnosis::Invalid { reason } => format!("the assembly is malformed: {reason}"),
            Diagnosis::OverConstrained { groups, gap } => format!(
                "over-constrained: {} contradict each other (they miss by {} in length and {} rad)",
                list(groups),
                gap.length,
                gap.angle
            ),
            Diagnosis::Infeasible {
                blamed,
                loop_mates,
                gap,
            } => {
                let mates: Vec<String> = loop_mates.iter().map(|&m| p.mate_name(m)).collect();
                format!(
                    "the loop through {} does not close by {} (and {} rad); {} cannot be met",
                    mates.join(", "),
                    gap.length,
                    gap.angle,
                    blamed.describe(p)
                )
            }
            Diagnosis::OutOfLimits {
                mate,
                variable,
                value,
                limits,
            } => format!(
                "{} is driven to {value}, outside its limits {} to {}",
                Group::Driver {
                    mate: *mate,
                    variable: *variable
                }
                .describe(p),
                limits.0,
                limits.1
            ),
            Diagnosis::DidNotConverge {
                iterations,
                residual,
            } => format!("did not converge in {iterations} iterations (residual {residual})"),
            Diagnosis::BranchLost { drives } => format!(
                "the motion cannot be followed on one branch past the drives {drives:?} (a branch point)"
            ),
            Diagnosis::TooLarge { reason } => format!("the assembly is too large: {reason}"),
            Diagnosis::DeadPoint {
                drives,
                loop_mates,
                cause,
            } => {
                let mates: Vec<String> = loop_mates.iter().map(|&m| p.mate_name(m)).collect();
                format!(
                    "a dead point at the drives {drives:?}{}: the drive cannot go further ({})",
                    if mates.is_empty() {
                        String::new()
                    } else {
                        format!(" in the loop through {}", mates.join(", "))
                    },
                    cause.message(p)
                )
            }
        }
    }
}
