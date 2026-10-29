# Assemblies

Instances of parts placed by mates, couplings and drivers, and moved over a motion. The decisions and the contract
between the three tracks are in `plan/assemblies.md` (local). This document holds what is built. The solver is the
crate `craft3d-assembly` (`crates/assembly`, L3, depends on `math` only), built 2026-10-29 (track B). Track A (the
document, its format and the engine's commands) adds its own sections below.

## Method (solver)

**Unknowns.** Every instance that is not grounded has six: a shift `u` of its reference point (its pose's
translation) in units of the problem's scale `L`, and a rotation vector `ω` in radians. A step applies them as
`t ← t + L u`, `R ← exp([ω]×) R`. The rotation is parametrised around the current pose at every step, so there is no
singular orientation (no Euler angles, no gimbal lock). Every mate's variables follow, in the order of
`MateKind::variables`: a revolute has an angle, a slider a distance, a cylindrical mate both, a planar mate `x`, `y`
and an angle. Angles are in radians and distances in units of `L`. Fixed and ball mates have no variables. All
unknowns are dimensionless, so one tolerance and one rank threshold serve every scale (tested at 1e-3, 1 and 1e6).

**Equations.** Every mate says that the frame on `b` is the frame on `a` moved by the mate's motion
`M(q) = T(x, y, offset + d) · Rz(angle + θ)`, in `a`'s frame. The variables a kind has are free, and the others are
zero. `flip` first turns `b`'s frame half a turn about its x axis. That gives six rows: the origins' difference over
`L`, and the rotation vector `log(G Eᵀ)` from the expected axes `E` to `b`'s axes `G`. A ball mate keeps only the
first three. The rotation residual is the logarithm, not the usual `½ Σ eᵢ × gᵢ`. That sine-type residual is zero at
a half turn too, so a flipped mate started from the identity would read as solved. The logarithm is zero only when
the axes agree. Near a half turn its axis is taken from the symmetric part of the matrix. Its Jacobian is exact, with
the inverse left Jacobian of SO(3): `J_l⁻¹(φ) = I − ½[φ]× + (1/θ² − cot(θ/2)/(2θ)) [φ]×²`. The rows are:

- `∂/∂ω_b = J_l⁻¹`;
- `∂/∂ω_a = −J_l⁻¹ R`;
- for the mate's angle, `−J_l⁻¹ R n`, where `n` is `a`'s z axis in the world.

A unit test checks every kind against central differences at three scales.

The other equations:
- **Couplings** are linear: `Σ cᵢ qᵢ = value`, divided by the largest dimensionless coefficient.
  - gear: `θa + ratio θb = 0`;
  - rack: `da − r θb = 0`;
  - screw: `da − lead θb / 2π = 0`. `a` and `b` may be the same cylindrical mate.
  - `Linear` is any tie, such as a planetary stage.
- **Drivers** are `q − value`.
- **Limits** are `q − bound`. A limit is active only while it holds its variable.

**Damped Newton (Levenberg–Marquardt).** The normal equations `(JᵀJ + μ D I) h = −Jᵀr` are assembled block-sparse.
The blocks are an instance's six unknowns and a mate's variables. They are factored as `L D Lᵀ` in minimum-degree
order, with the fill computed once per problem (`ldl.rs`). Each pivot block is eigen-decomposed by Jacobi rotations
(`eigen.rs`), and the directions at or below the rank threshold are left out (`D⁺`). Directions the equations do not
fix are not moved: a free part stays where it is, and the step is the least-squares step on the rest. A step longer
than half the scale or half a radian is shortened to that. A step is accepted when the sum of squares drops: `μ` is
then divided by 4, and otherwise multiplied by 8. The solve converges when every residual is within `ε = tolerance /
L`. It comes to rest when no damping lowers the sum of squares, or when four steps in a row lower it by less than one
part in 10⁹. It is capped at 200 iterations, and "did not converge" is an ordinary error.

**Limits** use an active set. The solve runs without them, then every limit the solution passes by more than the
tolerance is held at its nearer bound, and the solve runs again. There are at most as many passes as limits. A
driven value outside its limits is refused before solving (`OutOfLimits`).

**Rank threshold.** An eigenvalue of `JᵀJ` (or of `J Jᵀ`) counts as zero at or below `ε`. In words: a direction
whose singular value `σ ≤ √ε` can move by `√ε` of the scale while its residual stays within the tolerance to second
order, so as far as the tolerance can tell, it is free. The threshold follows from the tolerance and the scale
alone. A tolerance finer than double precision can resolve at that scale (`ε < 10⁴ f64::EPSILON`) is refused.

**Diagnosis.**
- **The rank and the free motions.** The rank of `J` is the sum of the pivots' ranks (for a positive semi-definite
  matrix the range of the coupling lies in that of the pivot). The degrees of freedom left are the unknowns minus the
  rank. Every zero eigenvector of a pivot gives a null vector of the whole by back substitution. Each null vector is
  turned into motions per instance: a `Slide` along a direction, or a `Turn` about an axis through a point, with a
  pitch. `FreeMotion::describe` puts it in words ("Bar[3] can still turn about z through (…)").
- **Redundancy and contradiction** come from `J Jᵀ`, factored in problem order: mates, couplings, drivers, then
  limits. A zero pivot direction at a group means that its rows repeat earlier ones. The null vector `y` (a left null
  vector of `J`) names the groups involved, and the last of them is the one that added nothing. At a solution every
  such dependency holds, and it is reported as `Solution::redundant`. A planar linkage built from revolutes in 3D
  always repeats three rows per loop.
- **At rest without meeting the tolerance**, the dependency with the largest `|yᵀr| / |y|` is the contradiction. The
  rank of `J` is then compared there with its rank at a generic nearby state: every unknown is moved by a fixed,
  irregular 1 % of the scale or 0.01 rad.
  - If the rank is lower where the solve stopped, the configuration lost it: a loop stretched or folded as far as it
    goes. That is `Infeasible`, with the loop's mates.
  - Otherwise the equations contradict each other everywhere. That is `OverConstrained`, with the groups.
  - The gap is the sum, over the groups involved, of their position misses (model units) and their angle misses
    (radians). For a loop that misses by `g`, the least-squares rest spreads `g` over its joints along the line, and
    the sum gives back `g`. The tests check this for a stretched four-bar and for two fixed mates 5 apart.

**Continuation over a motion** (`Solver::advance`, `Solver::sequence`). Each frame starts from the previous solution, and the first from a solve at the problem's own drives,
where its start poses are.
The drives go from the last values to the new ones in sub-steps, the first a whole step. A sub-step's first Newton
step from the last solution is its predictor. The tangent `−J⁺ (∂r/∂d) Δd` and the first Gauss–Newton step at the
new drives are the same step. The corrected solution is accepted when it lies within half the predicted motion (plus
`√ε`) of the prediction. A corrector that travels further has left the branch the prediction follows. A sub-step not
accepted is halved, and an accepted one doubles the next.
- **A dead point** is either of two things:
  - a frame where the rank drops: it is reported with its frame, drives, extra degrees of freedom and free motions,
    and the sequence goes on;
  - a drive that cannot get past a limit position: the sub-step shrank below `ε` of the drive without a solution. It
    is reported at the last value reached (`Diagnosis::DeadPoint`, with the cause), and the sequence stops there.
- **Jumps count as failures too.** A sub-step whose corrector converges but jumps off the branch fails like one that
  comes to rest. At a fold the corrector still converges past the limit, but further than predicted. When the
  sub-step shrinks to nothing, the result is a `DeadPoint`, and its cause is `BranchLost` or the solver's error.
- Angles are not wrapped: they follow the motion continuously, and a full crank turn ends at 2π.

**Determinism.** Fixed orders everywhere: blocks, groups, `BTreeSet` patterns and ties broken by index. No hash map
and nothing random. The generic state of the diagnosis is a fixed sequence. Solving twice gives the same bits.

**Never crash.** Everything a hostile problem can set is checked up front, with a reason:
- instance and mate indices, an instance mated to itself;
- frames that are not finite or not a right-handed set of axes (made orthonormal from z);
- offsets, angles and drives that are not finite, or so large that rounding exceeds the tolerance;
- limits that are not finite or reversed, and variables a kind does not have;
- couplings without the variable they need.

Sizes are capped (200 000 items), and so is the arithmetic of the symbolic and numeric factorisations (budgets).
Iterations, retries and sub-steps are capped too. A random-problem test (1500 problems with NaN, infinities, huge
numbers and wrong indices) never panics.

## Interface

```rust
pub struct Frame { pub origin: Vec3, pub axes: Mat3 }      // in the instance's own coordinates; z is the mate's axis
pub struct Pose { pub rotation: Mat3, pub translation: Vec3 } // world = rotation · local + translation
pub enum MateKind { Fixed, Revolute, Slider, Cylindrical, Planar, Ball }
pub struct MateSpec { name, kind, a: (usize, Frame), b: (usize, Frame), offset, angle, flip,
                      limits: [Option<(f64, f64)>; 3], driven: [Option<f64>; 3] }   // per variable
pub enum CouplingKind { Gear, Rack, Screw, Linear(Vec<(mate, variable, coefficient)>) }
pub struct CouplingSpec { kind, mates: (usize, usize), value }
pub struct Problem { instances, names, grounded, mates, couplings, start: Vec<Pose>, tolerance, scale }

pub fn solve(p: &Problem) -> Result<Solution, Diagnosis>;
pub fn solve_sequence(p: &Problem, frames: &[Vec<f64>]) -> Result<Sequence, Diagnosis>;
pub struct Solver;  // new(p), solve(), solve_at(drives), advance(drives), sequence(frames), last()
```

- **Units.** Angles are in radians and lengths in model units. `scale` is the assembly's size, and `tolerance` the
  length within which a residual counts as met. Variables are measured from the mate's zero: its two frames aligned
  up to `offset` and `angle`.
- **Drives.** The driven values are given per mate variable in `MateSpec::driven`. `advance` and `sequence` take new
  values in `Problem::driven_slots()` order (mate order, then variable order).
- **`Solution`** holds:
  - `poses` (one per instance) and `values` (every mate's variables, driven ones included);
  - `dof`, `rank` and `equations`;
  - `free` (a basis of the free motions);
  - `redundant` (equations that repeat others and hold);
  - `at_limit`, `drives` and `floating` (no instance is grounded);
  - `residual` (model units) and `iterations`.
- **`Diagnosis`** is one of `Invalid { reason }`, `OverConstrained { groups, gap }`,
  `Infeasible { blamed, loop_mates, gap }`, `OutOfLimits`, `DidNotConverge`, `BranchLost`, `TooLarge` or
  `DeadPoint { drives, cause }`.
  - A `Group` is a `Mate(i)`, a `Coupling(i)`, a `Driver { mate, variable }` or a `Limit { mate, variable }`.
  - `Diagnosis::message(&Problem)` and `Group::describe` name them by the problem's names.
- **`Sequence`** holds `frames`, `dead_points` (`DeadPoint { frame, drives, extra_dof, free }`) and
  `stopped: Option<(frame, Diagnosis)>`.
- **Under-constrained is not an error.** The solution carries `dof > 0` and the free motions. A part that nothing
  fixes stays where `start` puts it.

**The tests and what each pins** (`crates/assembly/tests/`; closed forms at scales 1e-3, 1 and 1e6 unless noted):
- **`linkages.rs`:**
  - a crank-rocker and a double-crank four-bar over a full turn: the rocker's joint and a coupler point against circle
    intersections, and back to the start after 2π;
  - the same with 60° frames, cut into sub-steps, on both branches (open and crossed);
  - a Grashof double-rocker and a non-Grashof triple-rocker stop at the closed-form dead point
    `cos θ* = (a² + d² − (b + c)²) / 2ad`;
  - an undriven four-bar has one free motion over three instances, and its redundancy is reported;
  - a slider-crank `x = r cos θ + √(l² − r² sin² θ)` over a turn;
  - driven by its slider, it stops at top dead centre `l + r`;
  - frame-by-frame `advance`, with wrong drive counts and NaN refused.
- **`couplings.rs`:**
  - a gear pair at its ratio, with the pitch points rolling without slip;
  - a rack moving `r θ`;
  - a screw advancing `lead · θ / 2π` (three leads after three turns);
  - a planetary stage through two linear couplings against Willis' formula (scale 1);
  - couplings naming a variable the mate does not have are refused.
- **`jansen.rs`:**
  - Jansen's leg (a = 38 … m = 15): the foot point against independent circle intersections (the strandbeest Jig's
    branches) at 72 crank angles over a turn, with rank 52 and no DOF left;
  - two legs half a turn apart on one crankshaft;
  - the benchmark (ignored; run in release).
- **`diagnosis.rs`:**
  - every mate kind's DOF and free motion, named per instance;
  - a floating assembly;
  - a repeated mate reported redundant;
  - two fixed mates 5 apart are `OverConstrained` with gap 5;
  - drivers against a gear coupling are `OverConstrained`, naming the coupling and both drivers;
  - a four-bar 1 short of closing is `Infeasible` with all four mates and gap 1;
  - a limit holds a free angle at its bound, and a driver beyond it is refused;
  - a flipped mate turns the part over from the identity;
  - thirteen malformed problems are refused with a reason;
  - determinism.
- **`hostile.rs`:** 1500 random problems never panic.
- **Unit tests:** the Jacobian against finite differences, the log map against exp, and the block factorisation.

**Benchmark.** Twelve Jansen legs on one crankshaft (74 instances, 109 revolutes), one degree per frame over a turn,
every foot checked: see the dev log entry of 2026-10-29 for the release timing on the Strix.

## Limits

- **Frames only.** The solver sees frames and does not know geometry. A face pair that should mate as tangent
  surfaces (a cam, a gear's flanks) is not a mate kind. Contact and interference are track C's measures, between
  bodies.
- **A ball mate has no variables in v1.** It cannot be driven, limited or coupled. Its three turns show only as free
  motions.
- **Limits are bounds held when passed.** A limit stays held for the rest of that solve, even when the solution would
  later move back inside it. Each new frame starts with every limit released.
- **Dead points need a drive to pass them.** A rank drop at a frame is reported only when the frame lands within about
  `ε` of it, which happens at a limit position the drive cannot pass. A change point the motion passes through
  between frames (the flat configuration of a parallelogram) is not detected. Continuation keeps the branch across
  it, because the corrector must stay near the predictor.
- **Which mates are blamed** is the dependency with the largest inconsistency, and its groups are those with a weight
  above 10⁻⁶ of the largest. With several independent contradictions, only the worst is named.
- **Infeasible against over-constrained** rests on comparing the rank at the stopping point with the rank at one fixed
  generic perturbation. A contrived problem whose rank happens to drop at that very perturbation would be misnamed.
- **Size.** The factorisation is sparse in minimum-degree order. Its arithmetic is capped at 2·10⁹ multiply-adds per
  factorisation and 4·10⁹ for the symbolic analysis, after which the problem is `TooLarge`. Assemblies of thousands
  of instances in long chains have not been measured.
