//! The assembly solver: where the instances of an assembly are, given the
//! mates between them, the couplings between mate variables and the values
//! the drivers set.
//!
//! The interface is pure data. A [`Problem`] holds frames on instances (in
//! each instance's own coordinates), [`MateSpec`]s between them,
//! [`CouplingSpec`]s and start poses; [`solve`] gives a [`Solution`] (a
//! pose per instance, every mate variable's value, the degrees of freedom
//! left and the motions still free) or a [`Diagnosis`] that names the mates
//! at fault. [`Solver::advance`] and [`solve_sequence`] follow a motion over
//! a sequence of driven values on one branch and report dead points.
//!
//! Unknowns are instance poses with the rotation parametrised locally
//! around the current pose (no gimbal singularity) and the mates'
//! variables; the equations are solved by damped Newton
//! (Levenberg–Marquardt) on a block-sparse factorisation whose pivots are
//! eigen-decomposed, which also gives the rank, the free motions and the
//! equations that repeat others. See `docs/design/assemblies.md`.

mod analysis;
mod eigen;
mod ldl;
mod newton;
mod report;
mod solver;
mod system;
mod types;

pub use solver::Solver;
pub use types::{
    CouplingKind, CouplingSpec, DeadPoint, Diagnosis, Frame, FreeMotion, Gap, Group,
    InstanceMotion, MAX_MATE_VARIABLES, Mat3, MateKind, MateSpec, Motion, Pose, Problem,
    Redundancy, Sequence, Solution, VarKind,
};

/// Solves `p` from its start poses at its own driven values.
pub fn solve(p: &Problem) -> Result<Solution, Diagnosis> {
    Solver::new(p)?.solve()
}

/// Solves `p` over a sequence of driven values (each in
/// [`Problem::driven_slots`] order), each frame from the one before.
/// `Err` only when the problem itself is malformed; a frame that cannot be
/// solved stops the sequence ([`Sequence::stopped`]).
pub fn solve_sequence(p: &Problem, frames: &[Vec<f64>]) -> Result<Sequence, Diagnosis> {
    let mut s = Solver::new(p)?;
    Ok(s.sequence(frames))
}
