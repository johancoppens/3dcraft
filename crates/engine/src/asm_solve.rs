//! The engine's side of the assembly solver (track B's crate
//! `craft3d-assembly`, docs/design/assemblies.md › *Interface*): the mates
//! as `asm.rs` builds them, with the world as an instance index of its
//! own, put into the crate's `Problem`, and its `Solution` or `Diagnosis`
//! read back as poses, values and words.
//!
//! The world is one more instance, grounded at the identity, after the
//! assembly's own. When no mate reaches it, the first instance is
//! grounded where it is placed instead and the result says `floating`.

use craft3d_assembly as asm;
use craft3d_math::{Transform, Vec3};

/// A frame in an instance's own coordinates: an origin and three axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Frame {
    /// The frame moved by `t`.
    pub(crate) fn moved(self, t: &Transform) -> Frame {
        Frame {
            origin: t.point(self.origin),
            x: t.vector(self.x),
            y: t.vector(self.y),
            z: t.vector(self.z),
        }
    }

    fn to_solver(self) -> asm::Frame {
        asm::Frame::new(self.origin, asm::Mat3::from_cols(self.x, self.y, self.z))
    }
}

pub(crate) use asm::MateKind;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CouplingKind {
    Gear,
    Rack,
    Screw,
    /// `Σ cᵢ qᵢ = value` over `(mate, variable, cᵢ)`, each `qᵢ` in
    /// radians or model units.
    Linear(Vec<(usize, usize, f64)>),
}

/// Most variables a mate has.
pub(crate) const VARIABLES: usize = asm::MAX_MATE_VARIABLES;

/// One mate between two instances ([`WORLD`] is the world), each with its
/// frame in that instance's coordinates.
#[derive(Clone, Debug)]
pub(crate) struct MateSpec {
    pub name: String,
    pub kind: MateKind,
    pub a: (usize, Frame),
    pub b: (usize, Frame),
    /// Along `a`'s z, in model units.
    pub offset: f64,
    /// About `a`'s z, in radians.
    pub angle: f64,
    /// `b`'s frame turned over onto `a`'s (face to face).
    pub flip: bool,
    /// Per variable, in the kind's order.
    pub limits: [Option<(f64, f64)>; VARIABLES],
    pub driven: [Option<f64>; VARIABLES],
}

#[derive(Clone, Debug)]
pub(crate) struct CouplingSpec {
    pub kind: CouplingKind,
    pub mates: (usize, usize),
    pub value: f64,
}

/// A rigid pose: from the instance's coordinates to the assembly's.
pub(crate) type Pose = Transform;

/// The world, as an instance index in a [`MateSpec`].
pub(crate) const WORLD: usize = usize::MAX;

#[derive(Clone, Debug)]
pub(crate) struct Problem {
    pub instances: usize,
    pub names: Vec<String>,
    pub grounded: Vec<usize>,
    pub mates: Vec<MateSpec>,
    pub couplings: Vec<CouplingSpec>,
    pub start: Vec<Pose>,
    pub tolerance: f64,
    pub scale: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct Solution {
    pub poses: Vec<Pose>,
    /// Per mate, its variables' values (radians, model units).
    pub values: Vec<Vec<f64>>,
    /// Degrees of freedom left in the whole assembly.
    pub dof: usize,
    /// Each free motion in words ("Bar[3] can still turn about z …").
    pub free: Vec<String>,
    /// Per mate: its main free motion (its first variable) moves in some
    /// free motion of the solution, so driving it takes one degree of
    /// freedom and contradicts nothing ([`main_moves`]).
    pub main_free: Vec<bool>,
    /// The driven values solved for, in the order of the driven
    /// variables of the problem's mates.
    pub drives: Vec<f64>,
    /// Why a moment asked to be carried ([`advance`]) was solved plainly
    /// instead, or what the solver says of its answer
    /// ([`asm::Solution::note`]: a rest solve whose carried pose differs
    /// from the solve straight from the placement, or that was not carried).
    pub note: Option<String>,
}

/// Why the solve failed, in words that name the mates and instances.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Diagnosis(pub String);

fn pose_to_solver(t: &Transform) -> asm::Pose {
    let o = t.point(Vec3::ZERO);
    asm::Pose::new(
        asm::Mat3::from_cols(t.vector(Vec3::X), t.vector(Vec3::Y), t.vector(Vec3::Z)),
        o,
    )
}

fn pose_from_solver(p: &asm::Pose) -> Option<Transform> {
    let origin = p.point(Vec3::ZERO);
    let x = p.point(Vec3::X) - origin;
    let z = p.point(Vec3::Z) - origin;
    craft3d_math::Frame::new(origin, z, x).map(|f| Transform::from_frame(&f))
}

pub(crate) fn solve(p: &Problem) -> Result<Solution, Diagnosis> {
    solve_from(p, None, None)
}

/// How many rest solves [`SolveMemo`] keeps.
const MEMO_SOLVES: usize = 8;

/// The last rest solves ([`solve`]), by the exact problem: an assembly
/// placed again with nothing its solve reads changed (an edit to a part's
/// colour, a variable set and set back, the same placement for another
/// command) takes the answer it had, which the solver gives bit for bit
/// anyway (the same problem gives the same solution). A rest solve carries
/// a placement from the drive values it holds to its drives along its
/// branch (`asm::Solver::solve_at`), which costs a few solves more; this
/// spends it once per problem. The key is the SHA-256 of the problem's
/// every field as `Debug` writes it (each number in the shortest digits
/// that read back to it), so only an identical problem matches. It is the
/// assembly's derived state's, and the placements of its subassemblies
/// solve through it too: an assembly with more than eight subassemblies
/// that solve may let an answer go before it is asked again, and then
/// solves it again (the same answer, at the solve's cost).
#[derive(Clone, Default)]
pub(crate) struct SolveMemo {
    kept: std::collections::VecDeque<([u8; 32], Result<Solution, Diagnosis>)>,
}

impl SolveMemo {
    /// [`solve`], from the memo when `p` was solved last among the
    /// [`MEMO_SOLVES`] kept.
    pub(crate) fn solve(&mut self, p: &Problem) -> Result<Solution, Diagnosis> {
        let key = craft3d_doc::blob::sha256(format!("{p:?}").as_bytes());
        if let Some(at) = self.kept.iter().position(|(k, _)| *k == key)
            && let Some(hit) = self.kept.remove(at)
        {
            let answer = hit.1.clone();
            self.kept.push_back(hit);
            return answer;
        }
        let answer = solve(p);
        self.kept.push_back((key, answer.clone()));
        while self.kept.len() > MEMO_SOLVES {
            self.kept.pop_front();
        }
        answer
    }
}

/// Solves `p` as a moment of a motion: from its start poses, which a
/// solve at the driven values `from` holds, the drives are moved to the
/// problem's own along the branch ([`asm::Solver::advance`], which cuts
/// the step while the solution jumps). With `from` of another length
/// than the problem's drives, a plain solve.
pub(crate) fn advance(p: &Problem, from: &[f64]) -> Result<Solution, Diagnosis> {
    solve_from(p, Some(from), None)
}

/// [`advance`], the angles no drive sets started the full turns nearest
/// their values in `near` (per mate, as [`Solution::values`]): carried
/// from a solve whose joints had come many turns (a joint held by hand,
/// `drive_cmds.rs`), which the start poses alone do not say.
pub(crate) fn advance_near(
    p: &Problem,
    from: &[f64],
    near: &[Vec<f64>],
) -> Result<Solution, Diagnosis> {
    solve_from(p, Some(from), Some((near, Near::Every)))
}

/// [`advance`] for a moment of a motion carried from the moment before
/// (its values in `near`): the angles a coupling takes start the full
/// turns nearest their values there ([`asm::Solver::solve_at_near_coupled`]),
/// so a shaft that only its coupling turns follows it over any number of
/// turns; the others are measured as from a placement.
pub(crate) fn advance_coupled(
    p: &Problem,
    from: &[f64],
    near: &[Vec<f64>],
) -> Result<Solution, Diagnosis> {
    solve_from(p, Some(from), Some((near, Near::Coupled)))
}

/// Which angles a solve carried from another takes the turns of.
#[derive(Clone, Copy)]
enum Near {
    Every,
    Coupled,
}

fn solve_from(
    p: &Problem,
    from: Option<&[f64]>,
    near: Option<(&[Vec<f64>], Near)>,
) -> Result<Solution, Diagnosis> {
    if p.start.len() != p.instances {
        return Err(Diagnosis("a start pose is needed per instance".to_string()));
    }
    // The world after the instances, grounded at the identity.
    let world = p.instances;
    let index = |i: usize| if i == WORLD { world } else { i };
    let mut names = p.names.clone();
    names.push("world".to_string());
    let mut start: Vec<asm::Pose> = p.start.iter().map(pose_to_solver).collect();
    start.push(asm::Pose::IDENTITY);
    let mut grounded = p.grounded.clone();
    grounded.push(world);
    let mates = p
        .mates
        .iter()
        .map(|m| asm::MateSpec {
            name: m.name.clone(),
            kind: m.kind,
            a: (index(m.a.0), m.a.1.to_solver()),
            b: (index(m.b.0), m.b.1.to_solver()),
            offset: m.offset,
            angle: m.angle,
            flip: m.flip,
            limits: m.limits,
            driven: m.driven,
        })
        .collect();
    let couplings = p
        .couplings
        .iter()
        .map(|c| asm::CouplingSpec {
            kind: match &c.kind {
                CouplingKind::Gear => asm::CouplingKind::Gear,
                CouplingKind::Rack => asm::CouplingKind::Rack,
                CouplingKind::Screw => asm::CouplingKind::Screw,
                CouplingKind::Linear(terms) => asm::CouplingKind::Linear(terms.clone()),
            },
            mates: c.mates,
            value: c.value,
        })
        .collect();
    let problem = asm::Problem {
        instances: p.instances.saturating_add(1),
        names,
        grounded,
        mates,
        couplings,
        start,
        tolerance: p.tolerance,
        scale: p.scale,
    };
    let slots = problem.driven_slots().len();
    let note = from.filter(|f| f.len() != slots).map(|f| {
        format!(
            "solved from the placements, not carried from the moment before: {} driven values there, {slots} here",
            f.len()
        )
    });
    let solution = match from {
        Some(from) if from.len() == slots => {
            let mut solver =
                asm::Solver::new(&problem).map_err(|d| Diagnosis(d.message(&problem)))?;
            let to = solver.problem_drives();
            // The note of the solve it starts from stays with the moment
            // carried from it (the advance has none of its own).
            match near {
                Some((near, Near::Every)) => solver.solve_at_near(from, near),
                Some((near, Near::Coupled)) => solver.solve_at_near_coupled(from, near),
                None => solver.solve_at(from),
            }
            .and_then(|first| {
                let mut moved = solver.advance(&to)?;
                moved.note = moved.note.or(first.note);
                Ok(moved)
            })
            .map_err(|d| Diagnosis(d.message(&problem)))?
        }
        _ => asm::solve(&problem).map_err(|d| Diagnosis(d.message(&problem)))?,
    };
    let poses = solution
        .poses
        .iter()
        .take(p.instances)
        .map(pose_from_solver)
        .collect::<Option<Vec<Transform>>>()
        .ok_or_else(|| {
            Diagnosis("the solver returned a pose that is not a rigid motion".to_string())
        })?;
    Ok(Solution {
        poses,
        values: solution.values.clone(),
        dof: solution.dof,
        free: solution.free.iter().map(|f| f.describe(&problem)).collect(),
        main_free: main_moves(&problem, &solution.free),
        drives: solution.drives.clone(),
        note: note.or_else(|| solution.note.clone()),
    })
}

/// Per mate of `problem`: whether its first variable changes in one of
/// the free motions `free` (a basis of the Jacobian's null space) by at
/// least 10⁻³ of the fastest variable of that motion, rates of distances
/// taken at the problem's scale so that turns and shifts compare. A
/// driver of a variable the null space moves adds one independent
/// equation (its row is not in the Jacobian's row space), so it takes a
/// degree of freedom and contradicts nothing; a driver of one it does not
/// move repeats or contradicts what the mates already decide. Whether the
/// null space moves a variable does not depend on the basis.
fn main_moves(problem: &asm::Problem, free: &[asm::FreeMotion]) -> Vec<bool> {
    let unit = |mate: usize, var: usize| match problem
        .mates
        .get(mate)
        .and_then(|m| m.kind.variables().get(var))
    {
        Some(asm::VarKind::Angle) => 1.0,
        _ => problem.scale.max(f64::MIN_POSITIVE),
    };
    let mut out = vec![false; problem.mates.len()];
    for f in free {
        let fastest = f
            .variables
            .iter()
            .fold(0.0f64, |m, &(mt, v, r)| m.max((r / unit(mt, v)).abs()));
        if !(fastest.is_finite() && fastest > 0.0) {
            continue;
        }
        for &(m, v, r) in &f.variables {
            if v == 0
                && (r / unit(m, v)).abs() >= 1e-3 * fastest
                && let Some(slot) = out.get_mut(m)
            {
                *slot = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn frame(origin: Vec3, z: Vec3) -> Frame {
        let f = craft3d_math::Frame::from_z(origin, z).unwrap();
        Frame {
            origin: f.origin(),
            x: f.x(),
            y: f.y(),
            z: f.z(),
        }
    }

    #[test]
    fn a_fixed_mate_puts_face_against_face() {
        // A block's top (z = 10) against another's bottom (z = 0), which
        // faces down: aligned, not turned over.
        let spec = MateSpec {
            name: "Stack".to_string(),
            kind: MateKind::Fixed,
            a: (0, frame(Vec3::new(5.0, 5.0, 10.0), Vec3::Z)),
            b: (1, frame(Vec3::new(5.0, 5.0, 0.0), -Vec3::Z)),
            offset: 0.0,
            angle: 0.0,
            flip: true,
            limits: [None; VARIABLES],
            driven: [None; VARIABLES],
        };
        let p = Problem {
            instances: 2,
            names: vec!["A".to_string(), "B".to_string()],
            grounded: vec![0],
            mates: vec![spec],
            couplings: Vec::new(),
            start: vec![Transform::IDENTITY; 2],
            tolerance: 1e-6,
            scale: 10.0,
        };
        let s = solve(&p).unwrap();
        let moved = s.poses[1].point(Vec3::new(5.0, 5.0, 0.0));
        assert!(
            moved.distance(Vec3::new(5.0, 5.0, 10.0)) < 1e-6,
            "{moved:?}"
        );
        let top = s.poses[1].point(Vec3::new(5.0, 5.0, 10.0));
        assert!((top.z - 20.0).abs() < 1e-6, "{top:?}");
        assert_eq!(s.dof, 0);
    }

    /// The memo answers an identical problem as the solve does, keeps the
    /// last eight problems, and tells problems apart by any number in
    /// them (a start pose a ten-billionth of the scale away is another
    /// problem).
    #[test]
    fn the_memo_answers_an_identical_problem_as_the_solve_does() {
        let problem = |gap: f64| Problem {
            instances: 2,
            names: vec!["A".to_string(), "B".to_string()],
            grounded: vec![0],
            mates: vec![MateSpec {
                name: "Stack".to_string(),
                kind: MateKind::Fixed,
                a: (0, frame(Vec3::new(5.0, 5.0, 10.0), Vec3::Z)),
                b: (1, frame(Vec3::new(5.0, 5.0, 0.0), -Vec3::Z)),
                offset: 0.0,
                angle: 0.0,
                flip: true,
                limits: [None; VARIABLES],
                driven: [None; VARIABLES],
            }],
            couplings: Vec::new(),
            start: vec![
                Transform::IDENTITY,
                Transform::translation(Vec3::new(0.0, 0.0, gap)),
            ],
            tolerance: 1e-6,
            scale: 10.0,
        };
        let poses = |s: &Solution| {
            s.poses
                .iter()
                .map(|t| t.point(Vec3::new(1.0, 2.0, 3.0)))
                .collect::<Vec<_>>()
        };
        let mut memo = SolveMemo::default();
        for k in 0..10 {
            let p = problem(k as f64);
            assert_eq!(poses(&memo.solve(&p).unwrap()), poses(&solve(&p).unwrap()));
        }
        assert_eq!(memo.kept.len(), MEMO_SOLVES);
        // The last eight are kept, the first two let go of.
        let keys: Vec<[u8; 32]> = memo.kept.iter().map(|(k, _)| *k).collect();
        for k in 0..10 {
            let key = craft3d_doc::blob::sha256(format!("{:?}", problem(k as f64)).as_bytes());
            assert_eq!(keys.contains(&key), k >= 2, "problem {k}");
        }
        // A hit answers bit for bit, and moves to the back.
        let again = memo.solve(&problem(3.0)).unwrap();
        assert_eq!(poses(&again), poses(&solve(&problem(3.0)).unwrap()));
        assert_eq!(memo.kept.len(), MEMO_SOLVES);
        let last = craft3d_doc::blob::sha256(format!("{:?}", problem(3.0)).as_bytes());
        assert_eq!(memo.kept.back().map(|(k, _)| *k), Some(last));
        let near = craft3d_doc::blob::sha256(format!("{:?}", problem(3.0 + 1e-9)).as_bytes());
        assert_ne!(near, last);
    }
}
