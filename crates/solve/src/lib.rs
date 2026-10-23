//! 2D geometric constraint solver for sketches.
//!
//! The input is plain numbers: a [`Sketch`] of points, lines, circles and
//! arcs with the constraints on them, in the order they were added, and the
//! current coordinates as the starting guess. [`solve`] returns the solved
//! coordinates with a diagnosis: the degrees of freedom left, and for every
//! constraint whether it is satisfied, redundant or conflicting.
//!
//! ```
//! use craft3d_math::Tolerance;
//! use craft3d_solve::{Constraint, ConstraintStatus, Sketch, SolveOptions, solve};
//!
//! let mut sketch = Sketch::new();
//! let a = sketch.add_point(0.0, 0.0);
//! let b = sketch.add_point(9.0, 1.0);
//! let line = sketch.add_line(a, b);
//! sketch.add_constraint(Constraint::Fixed { point: a, at: None });
//! sketch.add_constraint(Constraint::Horizontal { line });
//! sketch.add_constraint(Constraint::Distance { a, b, value: 10.0 });
//!
//! let solution = solve(&sketch, &SolveOptions::new(Tolerance::default())).unwrap();
//! let [x, y] = solution.point(b).unwrap();
//! assert!((x - 10.0).abs() < 1e-9 && y.abs() < 1e-9);
//! assert_eq!(solution.dof, 0);
//! assert!(solution.constraints.iter().all(|c| c.status == ConstraintStatus::Satisfied));
//! ```
//!
//! # Method
//!
//! A point is also held on a B-spline whose control points are points of
//! the sketch ([`Constraint::PointOnSpline`]): one equation, the signed
//! distance from the curve at the foot of the point, with no unknown for
//! the curve's parameter (it is eliminated by the projection).
//!
//! The unknowns are the point coordinates and the circle radii. Every
//! constraint is one or two scalar equations; an arc adds the built-in
//! equation that its two ends are equally far from its centre. The system
//! is solved by a damped Newton iteration whose steps are the minimum-norm
//! solution of the linearised equations, so whatever is not constrained
//! moves as little as possible.
//!
//! The Jacobian is factorised row by row in the order the constraints were
//! added. A row that is linearly dependent on the earlier rows marks its
//! constraint as redundant (it holds anyway) or conflicting (it does not),
//! which is how the later constraint gets the blame. The degrees of freedom
//! are the number of unknowns minus the number of independent rows at the
//! solution.
//!
//! When the iteration on all constraints does not converge, they are added
//! in order instead, each batch from the solution of the ones before it,
//! and the first that cannot be added is blamed and left out.
//! "Conflicting" therefore means "this solver could not satisfy it together
//! with the earlier ones from here"; it is a diagnosis, not a proof.
//!
//! A constraint that has no value where the sketch is (the direction of a
//! line without length) is left out in the same way and reported as
//! conflicting, with the reason in [`ConstraintReport::undefined`].
//!
//! The whole of this is one pass. [`solve`] repeats the pass from its own
//! result until the result no longer changes, because a pass reads the
//! mirror choices and the undefined constraints off its starting guess and
//! its result is a better guess: a constraint blamed too early, or
//! undefined where the sketch was drawn, gets its turn in the next pass.
//!
//! A solve that ends with a conflict, or does not converge, is tried once
//! more from the starting guess with every coordinate that an `x`, `y` or
//! `Fixed` constraint holds at a value already at that value (the start of
//! a fresh build: a held end of an arc that jumped is then on the side of
//! the circle its values pick, not the side it was on). That result is
//! kept only when it has no conflict.
//!
//! Before the first pass, image points that have moved since the starting
//! guess was a solution ([`Sketch::image_starts`]: a count, step or angle
//! of their pattern changed) carry the sketch along by continuation from
//! their old places to their new ones, and the passes start from where
//! that path ends: what hangs on an image follows it instead of landing on
//! another root of its equations.
//!
//! # Guarantees
//!
//! * Never panics, for any input; every loop is bounded ([`MAX_UNKNOWNS`],
//!   [`MAX_EQUATIONS`], [`SolveOptions::max_iterations`],
//!   [`SolveOptions::work_limit`]).
//! * Deterministic: the same input gives a bit-identical result.
//! * Stable: an `Ok` result is one that solving again leaves alone. Solving
//!   the entities of a [`Solution`] with the same constraints returns `Ok`
//!   with the same statuses and, in every case tried (the catalogue at
//!   three scales and some five thousand random sketches, most of them
//!   with conflicts), the same coordinates bit for bit; never a move as
//!   large as the tolerance. A sketch for which no such result is found
//!   within a few passes is [`SolveError::DidNotConverge`].
//! * Mirror solutions are told apart by the starting guess, so a sketch
//!   re-solved from its stored solution stays on its branch.

mod carry;
mod dual;
mod equations;
mod error;
mod factor;
mod model;
mod solver;
mod spline;

pub use equations::{CROWDED, MAX_IMAGE_DEPTH};
pub use error::{Site, SolveError};
pub use model::{
    Carried, Constraint, ConstraintReport, ConstraintStatus, Entity, EntityId, EntityKind,
    ImageMap, ImageStart, Sketch, Solution, SolveOptions, Spline,
};

/// The highest degree of a spline a point can be constrained onto.
pub const MAX_SPLINE_DEGREE: usize = spline::MAX_SPLINE_DEGREE;
/// Most entities in a sketch.
pub const MAX_ENTITIES: usize = 100_000;
/// Most unknowns (two per point, one per circle).
pub const MAX_UNKNOWNS: usize = 20_000;
/// Most constraints, and most scalar equations.
pub const MAX_EQUATIONS: usize = 20_000;
/// Most stored numbers in the factorisation (about 250 MB).
pub const MAX_FACTOR_ENTRIES: usize = 16_000_000;
/// One pass takes at most this many times
/// [`SolveOptions::max_iterations`] Newton steps over all its runs, and a
/// solve is at most seven passes.
pub const RUNS_PER_SOLVE: usize = 32;
/// Most constraints one pass of the search for non-convergent conflicts
/// blames before the solve fails with [`SolveError::DidNotConverge`].
pub const MAX_BLAMED: usize = 8;

/// Solves `sketch` from its current coordinates.
///
/// A sketch with conflicting or undefined constraints is not an error: the
/// result satisfies the others and names those. Errors are for input that
/// is not a sketch (unknown ids, wrong kinds, values that are not finite)
/// and for a solve that does not get there within its bounds.
pub fn solve(sketch: &Sketch, options: &SolveOptions) -> Result<Solution, SolveError> {
    solver::solve(sketch, options)
}
