# Assemblies

Instances of parts placed by mates, couplings and drivers, and moved over a motion. The decision is in
[`../decisions.md`](../decisions.md) (2026-10-29, *Assemblies in R1*). This document holds what is built. The solver is the
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
  - screw: `da − lead θb / 2π = 0`, so `lead` is per turn of 2π radians (the plan's 360°). `a` and `b` may be the
    same cylindrical mate (a thread); the document's check takes one mate twice for a screw on a cylindrical mate only
    and refuses it, saying so, for any other mate or coupling (2026-11-10, A-020: it refused it for every kind).
  - `Linear` is any tie, such as a planetary stage.
- **Drivers** are `q − value`.
- **Limits** are `q − bound`. A limit is active only while it holds its variable.

Couplings act on the variables as the solve carries them: unwrapped, measured from the mate's zero. The start value of
an angle is its measured angle in (−π, π], with three exceptions:
- a driven angle is taken the whole turns nearest its drive;
- a limited one is taken the whole turns nearest its range (a lid hinge limited to 0..270° and placed at 200° starts
  at 200°, not at −160°, which would be snapped to the bound 0);
- every other angle stays in (−π, π].

A coupling's phase is therefore fixed by the representative in (−π, π] of each start. Exactly at π this is +π, and it
is deterministic. Two consequences (2026-11-14, A-030):
- A moment of a motion is solved from the poses of the moment before, which say nothing of turns: first as always,
  every angle in (−π, π], so a moment that converged before converges to the same bits. Only when that fails are the
  angles a coupling takes started at the whole turns nearest their values in the moment before
  (`Solver::solve_at_near_coupled`); every other equation is periodic in an angle. Started in (−π, π], a screw that
  only its coupling turns was walked back half a radian an iteration at every moment, and a motion of two lead
  screws, one driven, stopped after 15.7 turns ("did not converge in 200 iterations").
- A rest solve takes a coupled angle's turns from its couplings where the placement says them. When the direct solve
  fails, the placement is solved again from the turns its couplings say (`System::coupled_start_turns`: an angle no
  drive sets that is the only such angle of a coupling starts the whole turns that meet it best where the placement
  has the rest, so a screw placed with its nut 27 leads along starts at 27 turns), tried only when that moves a start.
  A driven angle a coupling takes holds the whole turns that meet its couplings best (`System::start_drives`), and
  the placement is carried from there to its drive (*The rest solve is carried too*, below): a lead screw placed at 0
  with its nut and driven 16.8 turns winds there with its nut. When the carry turns a drive more than half a turn,
  the angles no equation counts are taken back to the whole turns nearest their start (their equations are periodic:
  no pose moves). So a joint no equation counts reads what the path to it gave: a nut's free turn on its screw reads
  9.42 rad after 1.5 turns, where the direct solve converges and the carried one agrees with it (the direct one is
  kept, as before), and within ±π after 3 turns, where the direct solve fails and the carried one is used.

**Damped Newton (Levenberg–Marquardt).** The normal equations `(JᵀJ + μ D I) h = −Jᵀr` are assembled block-sparse.
The blocks are an instance's six unknowns and a mate's variables. They are factored as `L D Lᵀ` in minimum-degree
order, with the fill computed once per problem (`ldl.rs`). Each pivot block is eigen-decomposed by Jacobi rotations
(`eigen.rs`), and the directions at or below the rank threshold are left out (`D⁺`). Directions the equations do not
fix get no step of their own: a free part that nothing touches stays where it is, and the step is the least-squares
step on the rest. A step that closes a misfit can still move a free joint with the parts it closes; a rest solve then
carries it back (*Free joints keep their start values*, below). A step longer than half the scale or half a radian
is shortened to that. A step is accepted when the sum of squares drops: `μ` is
then divided by 4, and otherwise multiplied by 8. The solve converges when every residual is within `ε = tolerance /
L`. It comes to rest when no damping lowers the sum of squares, or when four steps in a row lower it by less than one
part in 10⁹. It is capped at 200 iterations, and "did not converge" is an ordinary error.

**Limits** use an active set. The solve runs without them, then every limit the solution passes by more than the
tolerance is held at its nearer bound, and the solve runs again. There are at most as many passes as limits. A
driven value outside its limits is refused before solving (`OutOfLimits`).

**Rank threshold.** An eigenvalue of `JᵀJ` (or of `J Jᵀ`) counts as zero at or below `ε`. In words: a direction
whose singular value `σ ≤ √ε` can move by `√ε` of the scale while its residual stays within the tolerance to second
order, so as far as the tolerance can tell, it is free. The threshold follows from the tolerance and the scale
alone. It is `ε` and not `ε²` because the rank is read from `JᵀJ`, which squares the singular values. Its rounding
floor is about `10⁻¹⁶` times its largest entry, so `σ² ≤ ε²` could not be told from rounding at the finer
tolerances. Two cases weaken the threshold:
- **A hub with many mates** (a crankshaft with dozens of pins, a frame) sums all their rows into one pivot, so its
  entries and its rounding grow with the number of mates;
- **a short link held by ball mates at both ends** turns about the line between them with a singular value
  proportional to its length over the scale: below `√ε` of the scale it reads as one degree of freedom too many or
  too few. A tolerance finer than double precision can resolve at that scale (`ε < 10⁴ f64::EPSILON`) is refused.

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
  rank of that contradiction's own rows (its groups only) is compared there with their rank a little way along the
  motions `J` leaves free there. The step is a fixed, irregular combination of the null vectors, its largest
  component 2 % of the scale or 0.02 rad.
  - Moving along real motions keeps what the mates keep everywhere: a planar linkage stays planar, and a linkage
    elsewhere in the assembly is not counted.
  - So the rank goes up only when the configuration itself lost it: a loop stretched or folded as far as it goes,
    which bends there. That is `Infeasible`, with the loop's mates.
  - An earlier version perturbed every unknown at random and compared the global rank. That left the plane, so a
    regular planar four-bar anywhere in the assembly made a contradiction read as `Infeasible`. The regression test
    is `a_contradiction_beside_a_planar_four_bar_is_still_over_constrained`.
  - Otherwise the equations contradict each other everywhere. That is `OverConstrained`, with the groups.
  - The gap is the sum, over the groups involved, of their position misses (model units) and their angle misses
    (radians). For a loop that misses by `g`, the least-squares rest spreads `g` over its joints along the line, and
    the sum gives back `g`. The tests check this for a stretched four-bar and for two fixed mates 5 apart.

**Continuation over a motion** (`Solver::advance`, `Solver::sequence`). Each frame starts from the previous solution, and the
first from a solve at the problem's own drives, where its start poses are.
The drives go from the last values to the new ones in sub-steps, the first a whole step.
- **The predictor** is the secant of the last two solutions, scaled to the new drives, when the drives go on in the
  same direction (within 10⁻⁶ of parallel). Otherwise it is the tangent: one undamped, unshortened Gauss–Newton step
  at the new drives (from a solution, that step is `−J⁺ (∂r/∂d) Δd`).
- **The corrector** is the damped Newton solve, started from the predictor.
- **Acceptance.** The sub-step is accepted when the corrected solution lies within half the predicted motion (plus
  100 ε) of the prediction. A corrector that travels further has left the branch the prediction follows.
- **Step size.** A sub-step not accepted is halved, and an accepted one doubles the next.

Because the predictor is not shortened, a gear train of ratio 1000 turned 2 rad, or a rack driving a pinion of radius
L/1000 through hundreds of turns, takes one sub-step. The secant carries a branch through a point where branches
cross: a parallelogram four-bar keeps its rocker parallel to its crank through the flat position, whether or not a
frame lands on it (tested at 5° and 7° frames). On the flat position itself the Jacobian is singular, and the rocker
is fixed there only to about `√ε`.
- **A dead point** is either of two things:
  - a frame where the rank drops: it is reported with its frame, drives, extra degrees of freedom and free motions,
    and the sequence goes on;
  - a drive that cannot get past a limit position: the sub-step shrank below `ε` of the drive without a solution. It
    is reported at the last value reached (`Diagnosis::DeadPoint`, with the cause), and the sequence stops there.
  - Both name the loop (`loop_mates`) as the union of two sets: the mates whose rows came to depend on a driver's
    there (the drive at its reach, as at a slider's top dead centre), and the mates that the motions free there move
    (a loop that lost rank on its own, such as a coupler and rocker in line). A mate counts as moved when its
    variables change at 10⁻³ of the fastest or more, or when it has no variables and sits on a moving instance.
- **Running out of work is not a dead point.** More than 4096 sub-steps, or more than 20 000 Newton iterations in one
  advance, is `DidNotConverge`.
- **Jumps count as failures too.** A sub-step whose corrector converges but jumps off the branch fails like one that
  comes to rest. At a fold the corrector still converges past the limit, but further than predicted. When the
  sub-step shrinks to nothing, the result is a `DeadPoint`, and its cause is `BranchLost` or the solver's error.
- Angles are not wrapped: they follow the motion continuously, and a full crank turn ends at 2π.

**The rest solve is carried too** (`Solver::solve`, `solve_at`; A-016). A placement's poses hold driven values of their
own (`System::start_drives`: each driven variable as the placement has it, a driven angle at the whole turns nearest
its drive, or that meet its couplings best when a coupling takes it).
- When they differ from the drives by more than the tolerance, and the drives may take them (`check_drives`: within
  their limits and reach), the placement is also solved at its own values and carried from there to the drives along
  its branch (`Solver::carry_rest`). One Newton solve from the placement at other drive values can land on the mirrored
  loop and say nothing: a Jansen leg put together at 100° and solved at 260°, the walker's legs at a crank start of
  180°, a near-change-point crank-rocker solved past the pose where its circuits pass closest.
- **The rule** (the third review, the coordinator's decision): an answer without a note is the one solve straight from
  the placement, bit for bit; wherever the carried and the direct solution differ, the answer says so. A step size can
  always be beaten by a geometry nearer its change point; the note cannot. So nothing the direct solve (0.1.2) got
  right comes out otherwise without a note. A note marks a disagreement between the two solves, not a guarantee: near
  a change point the carry itself can land on the other branch (*How the carry steps*, below), and the pose it shows is
  then the wrong one, with the note.
  - The direct solve is made first, as before. When the carried solution is the same one (within √ε, over the
    instances and mate variables the direct solution's free motions do not move, an angle no equation counts within a
    turn: `System::distance_fixed`), the direct one is kept, bit for bit, without a note.
  - When they differ and the parts as placed meet their mates at the values they hold (the closing solve moved them by
    at most √ε), the carried solution is the answer, and the note says "carried from the placement, the assembly rests
    elsewhere than a solve straight from the placement gives; the carried pose is shown". When the parts do not meet
    their mates there (a leg placed
    with its pin away from its crank's), which branch the closing solve lands on is not known: the carried solution is
    the answer and the note says the branch is not certain. The Jansen legs and the walker at a crank start of 180°
    that 0.1.2 put on their mirrored loop carry one of these notes too: two methods disagree, and the answer says which
    was taken.
  - When the carry stops short from a placement that meets its mates (at a limit position, or at a turn its sub-steps
    cannot follow: which of the two is not proven), the direct solution is the answer and the note says it was not
    carried past those drive values and the branch is not checked. When the carry runs out of its allowance, the note
    says the direct solution is shown and the branch is not checked. Any other failure (a placement that cannot be
    closed at its own values, a carry from one that does not meet its mates that stops) leaves the direct solution and
    says nothing, as before: on the review's 95,000 probe solves a note there would have fallen on 634 right answers.
  - Where the direct solve fails, the coupled turns above are tried first (as on main, without a note), then the
    carry. Its answer says so: "the solve straight from the placement failed; the pose carried from the placement is
    shown" from a placement that meets its mates (the crank-rocker of `linkages.rs`), that the branch is not certain
    from one that does not; a coupled drive carried whole turns with its nut (A-030), which has no branch to doubt,
    says nothing. Without the note a failed direct solve near a change point would become a silent pose the carry may
    have put on the other branch.
- When the placement holds the drives, it is one solve, as before, bit for bit. A moment of a motion and a joint held
  by hand start from a solved state that holds its own drives, and are solved as before.
- **How the carry steps** (`Pace::REST`). It is a motion's continuation with these differences, so that it follows its
  branch where a motion's frames, close together, need not:
  - A sub-step changes a drive by at most a tenth of a radian or of the scale (`REST_STEP`), the first by a sixteenth of
    that, and it doubles from there. A branch can turn sharply where nothing is near singular: the third review's
    offset slider-crank (crank 1, rod 1.52, offset 0.5) turns at a crank of 270° with its circuits 0.1 of the scale
    apart, and a half-radian sub-step from 254° to 277° landed on the other circuit with the corrector converging fast
    and the prediction met. A tenth resolves such turns at the review's slider-cranks and at four-bars of coupler
    3.0002 (circuits 0.044 apart at a crank of 180°); at coupler 3.00002 (1.4·10⁻³ apart) the carry still lands on the
    other circuit now and then (below), and at 3.0000002 (1.4·10⁻⁴ apart, where even a motion in 1° frames can) often:
    the note on every difference is what holds there.
  - The secant moves each part by its rigid motion scaled (`System::extrapolate`: `(P P_prevᵀ)^k P`, the screw motion
    about the same axis), not its translation and turn apart, which cut the chord of a part turning about an axis far
    from its origin: a prediction that does not depend on where a part's origin is, exact for a part turning about a
    fixed axis (a lead screw carried 40 turns takes no Newton step but the last).
  - **Its sub-steps keep away from singular poses** (`REST_REACH`). The corrector's first two Newton steps say how fast
    it converges: `ω = 2 |h₂| / |h₁|²` is the constant of its quadratic convergence, and `1/ω` estimates how far the
    solution is from a singular pose, where another branch may come near. The next sub-step moves at most a quarter
    of `1/ω`, and a sub-step that moved more than `1/ω` from the solution it reached is cut. So the carry nears a pose
    where two branches pass close in steps that shrink with the distance, and follows its own branch round the sharp
    turn there: the second review's crank-rocker of coupler 3.02, whose circuits pass 0.44 apart at a crank of 180°,
    crossed in half-radian sub-steps without it in 61 of 863 exact placements. `ω` grows only near a singular
    Jacobian: a sharp turn at moderate conditioning (the slider-crank above) is left to the short step.
  - The corrector starts from a damping of 10⁻¹² of the largest diagonal (`MU_CARRY`), a thousandth of a solve's. At
    10⁻⁹ every direction whose eigenvalue is near 10⁻⁷ of the largest converges only linearly (a rate of 0.01 in the
    published walker), and `ω` read from the steps then grows as the prediction gets better: the walker carried 60°
    took 325 sub-steps, from 10⁻¹² 20.
  - **A decision the developer can revisit: the lighter damping against jumps.** With 10⁻¹² the carry lands on the
    other circuit more readily near a change point. Measured by the fourth review on four-bars of coupler 3.00002: 8
    jumps with 10⁻¹², none with 10⁻⁹; among them 4 exact placements the direct solve had right (circuit −1 at 80° driven
    to 187°, 217° and 247°; circuit +1 at 280° driven to 127°), each shown with the note; the probe took 45 s with
    10⁻⁹ against 36 s. With 10⁻⁹ the published walker carried to 180° runs out of its allowance (and is shown straight
    from the placement, on its mirrored loop, with the note that the branch is not checked); with 10⁻¹² it takes 54
    sub-steps, fifty legs on one crankshaft 0.43–0.78 s instead of giving up after 1.9–3.1 s. Kept at 10⁻¹² (the
    coordinator, 2026-11-14), since every jump carries a note by the rule.
  - A sub-step that succeeds after a cut one keeps its length instead of doubling straight back.
  - A sub-step whose prediction meets the tolerance takes no Newton step, so the solution may drift within the
    tolerance over many of them; one Gauss–Newton step at the end brings it to the accuracy a solve ends at.
- The carry spends at most 2000 Newton iterations (ten solves), a sub-step's corrector at most 50, and at most twice
  the sub-steps its span needs at full length plus 384 (`REST_SUBSTEPS`): the published walker carried to a crank start
  of 180° takes 54 in all, at ±120° 37, at ±60° 20, at either end of its leg scale's range the same; the radial engine
  carried more than half a turn 98. A carry that cannot fit what is left of the way in what is left of its allowance,
  even doubling every sub-step, stops at once (the same outcome, without the work; a stop that assumed no growth was
  measured and refused: it gave up on carries that then grow, 298 slider-cranks among them). A limit position is
  found by halving to ε of the drives' change (√ε was tried and refused: a Jansen leg carried from 250° to 80° passes a
  near-singular pose at 193° in finer steps).
- In the engine (`asm_solve::SolveMemo`), the last eight rest solves are kept by their exact problem, so an assembly
  placed again with nothing its solve reads changed takes the answer it had, which the solver would give bit for bit.
  The memo is the assembly's derived state's and its subassemblies solve through it too: one with more than eight
  subassemblies that solve may let an answer go before it is asked again.

**Free joints keep their start values** (`Solver::keep_free`; A-036). A Newton step is the least-squares step in all
the unknowns alike: instance shifts and turns and joint values. A placement with a misfit (a part placed turned
against its mate on a turret that nothing drives) is closed by every unknown that can close it, each a share: the free
swivel, the turret and the part each turned a third of the misfit. So a free joint ended where nothing asked it to be
(the task lamp's turret swivel 6.6° from its start, the solve ok).
- After a rest solve from a placement, a joint is kept when it alone carries a free motion: the free motions move it in
  a way no other joint they move does (its row of rates in the free motions is not in the span of the other joints'
  rows, to 10⁻⁶ of its length, as a null vector's component below 10⁻⁶ of its largest counts as none). A free swivel,
  a free hinge, a free slider are kept. Joints that move only together (a free loop's, a free screw and its nut) are
  left as the solve leaves them: which of them to keep is a choice the document does not make. The choice depends
  neither on the mates' order nor on the instances' nor on the assembly's size, down to the 10⁻⁶ the null vectors
  resolve.
- When a kept joint is more than the tolerance from its start value, the kept joints are carried back to their start
  values together, with the solution, along its branch, as drives of a problem that holds them (the rest carry's
  pace). When they cannot be, the solution is the solve's and its note says so, naming the mates. When nothing moved,
  nothing is done: bit for bit as before.
- A free motion that moves no joint variable (a ball's turns, a floating part) has no value to keep and stays as the
  solve leaves it.

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
  - `residual` (model units) and `iterations`;
  - `note`: what a reader should know about the answer, in words (a rest solve whose carried pose differs from the
    one solve straight from the placement, or that was not carried to its drives; a free joint not carried back to its
    start value). A rest solve without a note is that one solve, bit for bit.
- **`Diagnosis`** is one of `Invalid { reason }`, `OverConstrained { groups, gap }`,
  `Infeasible { blamed, loop_mates, gap }`, `OutOfLimits`, `DidNotConverge`, `BranchLost`, `TooLarge` or
  `DeadPoint { drives, loop_mates, cause }`.
  - A `Group` is a `Mate(i)`, a `Coupling(i)`, a `Driver { mate, variable }` or a `Limit { mate, variable }`.
  - `Diagnosis::message(&Problem)` and `Group::describe` name them by the problem's names.
- **`Sequence`** holds `frames`, `dead_points` (`DeadPoint { frame, drives, extra_dof, loop_mates, free }`) and
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
  - frame-by-frame `advance`, with wrong drive counts and NaN refused;
  - a crank-rocker of coupler 3.02 placed exactly on its circuit −1 at 110° and solved at rest at 275°, at three
    scales: on circuit −1 by the closed form, with the note that the solve straight from the placement failed (it
    does not converge there).
- **`couplings.rs`:**
  - a gear pair at its ratio, with the pitch points rolling without slip;
  - a rack moving `r θ`;
  - a screw advancing `lead · θ / 2π` (three leads after three turns);
  - a planetary stage through two linear couplings against Willis' formula (scale 1);
  - couplings naming a variable the mate does not have are refused;
  - two lead screws on one bed, one driven: the other at the driven one's angle and the bed a lead a turn, at rest at
    3 to 40 turns and moment by moment for 40 turns (A-030), the nuts' free turn on their screws read within half a
    turn of where they were placed; the same saved with the bed 27 leads up: nothing turned round, every joint as
    placed.
- **`jansen.rs`:**
  - Jansen's leg (a = 38 … m = 15): the foot point against independent circle intersections (the `strandbeest-leg` Jig's
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
- **`rest.rs`** (A-016, A-036):
  - crank-rockers with couplers 3.5, 3.02, 3.002 and 3.0002 (the last three near the change point, their circuits
    0.44, 0.14 and 0.044 apart at a crank of 180°), put together on either circuit at every 40°, exactly or with the
    crank alone turned 90° on, solved at rest at every 60°, with the bodies' origins at the world's or at their pivots:
    the coupler's far pin on the circuit it was put together in by the closed form, every one of 5,184, and a note
    wherever the carried and the direct solve differ (the parent of the fix: 252 exact and 150 turned placements on the
    other circuit);
  - **the rule** (`keeps_the_rule`): an answer without a note is the direct solve's (`solve_at_near` with no turns to
    take), bit for bit; one whose note says the solves disagree is not; one whose note says it was not carried is.
    Kept, with every answer on its circuit by the closed form or noted, by the third review's offset slider-crank
    (rods 1.52 and 1.502, three scales, exact and crank-turned), couplers 3.00002 (three scales), an RSSR spatial
    four-bar on its two assembly modes, a five-bar driven by both cranks, 4, 8 and 12 Jansen legs on one crankshaft
    (exact, and placed at a crank of their own), and four-bars placed off by 3·10⁻¹⁰ to 3·10⁻³ of the scale; at
    coupler 3.0000002, where continuation cannot resolve the circuits, only the rule;
  - a Jansen leg put together at 30°, 100°, 190° and 250° and solved at rest at every 10° of a turn: the foot against
    the circle intersections on the Jig's branches (before, 260° and 270° from 100° were on the mirrored loop, which
    the note now says the direct solve lands on); its crank alone turned to the drive, or by every 10° with the drive
    where it was put together: the leg's pins closed on its branch (before, 27 of 432 on the mirrored loop);
  - the same pose with the instances numbered the other way round;
  - a Grashof double-rocker driven to its other circuit: solved, with a note that it was not carried past the drive
    values where the carry stopped;
  - a turret on a free swivel with a part on it placed turned (0.12, −0.3 and 2.5 rad, fixed and on a free revolute):
    the swivel keeps 0, the part turns back onto its mate (before, the swivel took a third of the misfit); the same in
    other instance orders; a driven arm on the turret: the swivel's value and turn bit for bit;
  - the review's swivel and elbow: a free swivel and a free elbow, the arm and forearm placed turned: the swivel keeps
    0 and the elbow its placed value (−0.7 of the misfit), the forearm's end where that puts it;
  - a free double-rocker placed with its input out of reach: closed, without a note (its four joints move only
    together, so none is kept);
  - a free screw and its nut beside a kept swivel, in both mate orders and at two sizes: neither the screw nor the nut
    is kept, the swivel is.
- **`hostile.rs`:** 1500 random problems never panic.
- **Unit tests:** the Jacobian against finite differences, the log map against exp, and the block factorisation.

**Benchmark.** Twelve Jansen legs on one crankshaft (74 instances, 109 revolutes), one degree per frame over a turn,
every foot checked against the closed form: 5.2 ms mean and 5.6 ms worst per frame in release on the Strix
(2026-10-29), against a target of 16 ms.

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
  between frames (the flat configuration of a parallelogram) is not reported. The secant keeps the branch across
  it.
- **Which mates are blamed** is the dependency with the largest inconsistency, and its groups are those with a weight
  above 10⁻⁶ of the largest. With several independent contradictions, only the worst is named.
- **Infeasible against over-constrained** rests on comparing the contradiction's rank at the stopping point with its
  rank one fixed step along the free motions. A contrived problem whose rank happens to drop at that very step would
  be misnamed. A loop that is stretched where nothing is free (every motion driven) reads as over-constrained.
- **Size.** The factorisation is sparse in minimum-degree order. Its arithmetic is capped at 2·10⁹ multiply-adds per
  factorisation and 4·10⁹ for the symbolic analysis, after which the problem is `TooLarge`. Assemblies of thousands
  of instances in long chains have not been measured.

## The document and the engine (track A)

The document kind, its format, the engine's derived state, its commands, pack and export (track A, 2026-10-29).

### The document (`crates/doc/src/assembly.rs`)

An assembly is a `Document` whose `assembly` is set: it keeps the document's variables, tables, assertions, outputs,
named sets and info as a part does, and has **no features of its own** (a command that would add one is refused whole,
`commands::edit_merging`). It is saved as a `.3dcraftasm` file, the same container and format versions as `.3dcraft`;
a `.3dcraft` file never holds an assembly and a `.3dcraftasm` file never a part (`file.open` and `file.save` refuse the
wrong extension both ways).

- **`Instance`**: a unique `name` (not `world`, no `/`, which separates it from what follows in a reference); a
  `source`, either `File {path, hash}` (the path relative to the assembly's file with `/` between its parts, `..`
  allowed, and the SHA-256 of the bytes when it was added or last followed) or `Packed {hash}` (one of the documents
  the file carries); an optional `body` (the name or id of the source feature whose body the instance shows); `params`
  (public parameter overrides, as expressions of the assembly's variables); an optional `pinned` hash; `mode` (rigid or
  flexible, for a subassembly); `visible`; `colour`; `placement` (a position and a rotation vector in degrees: the
  solver's start and the pose of an instance nothing constrains).
- **`Mate`**: a `name` that is a variable name (so `<name>_angle` is one too), a `kind` (fixed, revolute, slider,
  cylindrical, planar, ball), references `a` and `b` written `Instance/name` (`Bar[2]/F1:face(1)`, `world/xy`), an
  optional `offset` and `angle` expression, `limits`, and `flip`.
- **`Coupling`**: a `kind` (gear, rack, screw), the two mates by name and a value expression.
- **`packed`**: the source documents carried in the file, each the bytes of a `.3dcraft` or `.3dcraftasm` file, in
  ascending order of their SHA-256, each once; written as the archive entries `blobs/<sha256>.3dcraft` beside the
  imported STEP files' `blobs/<sha256>.step`, read back and checked against their hash.

`Assembly::validate` holds the rules (unique names, references to instances that exist, no mate joining an instance
to itself, couplings naming mates, hashes in lowercase hex, relative paths, the packed list in order and complete).
Every assembly command edits through `asm_cmds::asm_edit`, which validates in the same step.

### Format (version 52)

`manifest.rs` and `convert.rs` carry the assembly exhaustively (a new field does not compile until it is written).
The version was raised so that an older build refuses an assembly by naming the version it needs instead of an
unknown field; a file of an older version is a part and holds none, so the migration changes nothing. The fixture of
the version is `v52.3dcraftasm`, a packed assembly (two configurations of one source, a picked body, a pinned and a
hidden instance, a fixed, a revolute and a slider mate, a joint variable, a rack coupling), written by the engine of
its own build on the Strix.

### Derived state (`crates/engine/src/asm.rs`)

What an open assembly shows is derived and may be thrown away (rule 2). `OpenDocument::asm` keeps:

- **the files read** (`AsmState::files`), by their path as `asm::join` writes it from the read root. Reading is the one
  part that touches files: `Session::read_sources` (on `file.open`, `instance.add`, `instance.edit`,
  `assembly.reload`, `assembly.pack`) goes through the workspace's read root and the platform's file access
  (`resolve_read`, `read_input`: the session's memory in the browser), recursively through subassemblies to
  `MAX_DEPTH` (8) levels with a seen-set, at most `MAX_SOURCE_FILES` (1000) files. Everything after it is pure.
  **Following the sources** (2026-11-06, `asm_follow.rs`): `Session::read_sources_of` reads the sources of any open
  document, all of them (`assembly.reload`), only the missing ones (opening, adding) or those that look changed
  (`Reread::Stale`: another size or modification time than `AsmState::stamps` holds; in memory, every one, compared
  by hash). `file.save` makes every open assembly read what changed, the file it wrote whatever its stamp says (a
  file system with coarse times); `file.activate` the one it activates; `assembly.follow` all of them, which the
  window runs when it gets the focus back. A file read again with the same bytes changes nothing, so nothing is
  placed or built again; a changed one invalidates the placement and only its configurations are built, when the
  assembly is shown (not an undo step: the document does not change). A source that changed and cannot be read again
  (gone, unreadable, refused by the roots) keeps its last bytes in `files`, and its instances their bodies; the
  reason is kept in `AsmState::unfollowed` (and listed by `assembly.reload` under `unread`, by `file.save` under
  `warnings`) and read again at the next look. `InstanceState::held` says why an instance does not show what its file
  holds now, pinned or not read again (`assembly.inspect` › `held`, the outline's `held`, the Instances panel's ⚠ and
  tooltip); in the browser the outline comes from the worker, which does not know `unfollowed`, so there only a
  pinned instance is marked. `InstanceState::feature_failed` (2026-11-10, A-023) says which feature of an
  instance's part failed in its configuration, with the part's own message (`asm::failed_in`: the first failed
  feature; for a subassembly the first of its instances with one, by its name there, so a failure inside a
  subassembly surfaces at the top): the instance is placed with the bodies from before it, as the part shows them,
  and `error` stays for an instance that is not placed. It is `assembly.inspect` › `featureFailed`, the outline's
  `feature_failed` (carried from the worker in the browser), the Instances panel's ⚠ and tooltip and a problem of the
  assembly's `document.check` (before, a lamp's turret showed four intermediate bodies and the check was ok). Before, only `assembly.reload` read a source again, and the window never ran it: a part
  saved in its own tab did not show in the assembly's (the developer, 2026-11-06). Measured on the pi case (two
  parts, three instances; test profile): a check that finds nothing 0.07 ms (it parsed each part to see whether it
  is a subassembly, 4 ms, until it took the parsed sources of the placement), the save of `case.3dcraft` 137 ms
  with the follow, the placement after it 0.96 s (one configuration built again, the first placement 6.6 s).
  An assembly without a file reads its sources from the folder `Session::asm_base` gives (the read root's top, or
  the current folder with access to every file, as the command line has) and is placed from that same folder
  (`AsmState::unsaved_base`, set by `read_sources`) until it is saved. Before 2026-10-30 its placement joined the
  paths to an empty folder, so with access to every file each source was read under its absolute path and looked
  for under a relative one: `instance.add` on an unsaved assembly in `craft3d-cli` failed.
- **the documents parsed**, by hash, and **the configurations built**, by key: the source's SHA-256 and each
  overridden parameter's name and the text it was set to; for a subassembly also the folder its own sources are read
  from (two copies of one file in two folders are two configurations). A subassembly's placement depends on the files
  read for its own sources, which the key does not hold, so every change of the files read drops the subassemblies'
  entries (parts stay: their key is their bytes), and a subassembly whose own sources changed is `changed` too. A configuration is the source with its public parameters set
  as `param.set` sets them (the same checks: a parameter that is not public, or a value outside its range, fails the
  instance by name; but not the step, which is the form's and `param.set`'s: a value the assembly computes is no
  slider, 2026-11-10, A-011), regenerated once. Instances of one configuration share it whichever body they pick. The
  configurations not built yet are regenerated side by side (`interference_cmds::in_order` on the shared thread pool)
  and merged in the order of their keys; a subassembly's are built one after the other, each building its own parts
  side by side. `assembly.inspect` reports the number of distinct configurations and how many regenerations they cost
  so far (`regenerations`).
- **the placement** of the current revision (and generation of the files read): one `Placed`.

The placement, in order:

1. each instance's source bytes: the file read, or the packed document. A **pinned** instance keeps the bytes it is
   pinned to packed in the assembly (`instance.add {pinned: true}` and `instance.edit {pinned: true}` pack them,
   `drop_unused_packed` keeps them), so when its file changes it uses that copy; an older file without it fails naming
   both hashes; an unpinned one
   follows the file and is marked `changed` (its stored hash is the one it was added with until `instance.edit
   {pinned: false}` takes the new one). A part in another length unit than the assembly is regenerated in the assembly's
   unit as large as it is (2026-10-30): its parameters set first in its own unit (a value given carries its unit),
   then converted as `document.setUnit {keep: sizes}` converts a document and proven the same way, every feature's
   status and every body's volume, area, centroid and box (`unit_cmds::in_unit`, two regenerations more per such
   configuration); what that refuses fails the instance by name. The configuration's key holds the unit it is built
   in. A subassembly in another unit is refused by name: its own mates' offsets and placements would have to be
   converted too, which is owed. A body is never scaled: it is moved by rigid transforms only. A hash already on the chain of assemblies above is a cycle,
   refused by name; `instance.add` and `instance.edit` refuse a source that contains the assembly's own file or itself
   before anything is changed (`refuse_cycle`).
2. the configurations built or taken from the cache;
3. each instance's picked body: **by the source feature's name or id, never another one**. A feature that is gone, or
   whose body went into a later feature or failed, fails the instance by its name; the instance shows nothing then.
4. the **frames** of each mate, in the instance's own coordinates (`frame_in`), from the plan's table: a planar face's
   plane at the face's centroid (area-weighted, over a mesh at a thousandth of the body's size) with its outward normal
   as z and the surface frame's x; a cylindrical or conical face's axis at the point of the axis nearest the face's
   centroid; a circular edge's centre and axis; a vertex as a point with the world's axes; a datum plane, axis or point
   of the source by its name or id. Anything else (a sphere, a freeform face, a straight edge) fails the mate naming
   what it is. A face of a body the source hides fails the mate (the instance shows only the visible bodies) unless the
   instance picks that body. The name is resolved by `selection::Resolver::locate`, the same resolution as a selection: its aliases
   and the short names of caps hold, and **a name that is gone fails the mate with that name; it never retargets**
   (rule 4 across files). Through a subassembly the name goes on through its instance (`Sub/Inner/F1:face(0)`) and the
   frame is moved by that instance's pose in it. The world has `world/xy`, `yz`, `zx` (planes), `x`, `y`, `z` (axes)
   and `origin`, each a frame at the origin: `xy`, `z` and `origin` have z along the world's z and x along its x;
   `yz` and `x` have z along x and x along y; `zx` and `y` have z along y and x along z (a cyclic turn of the axes,
   so each is right-handed). A world reference is known without reading anything, so `MateRef::check` refuses any
   other name when the mate is made or edited (`mate.add {"a": "world/-x"}` is an error naming the seven), never
   later in the solve.
5. the **solve** (`asm_solve`, see *Solver* below): the mates as the contract's `MateSpec` (b's frame turned over
   about its x unless `flip`, `offset` along and `angle` about a's z, the main free motion driven by the assembly
   variable `<mate>_<motion>` when it exists), the couplings, a start pose per instance; when no mate reaches the
   world the first instance is held where it is placed and the result says `floating`.
6. the bodies **placed by transforms only** (`Body::transformed`): a part `Placed::doc` with one feature per placed
   body (named after its instance, `Instance/Body` when it shows several) and its regeneration `Placed::regen`, with
   the source's names, its fixed appearance and material, and the instance's colour over them.

`OpenDocument::regen` and `OpenDocument::shown`/`shown_regen` of an assembly are that part and its regeneration, so the
scene, `view.render`, picking, selection, `file.export` of meshes and the assertions read an assembly as they read any
part. A picked face of an assembly is `F<n>:<the source's name>`; `assembly.inspect` lists which features of the shown
part hold each instance's bodies, so a frontend writes the mate reference `Instance/<name>`.

### Subassemblies

An instance whose source is an assembly is a subassembly, placed recursively with the same caches.

- **Rigid** (the default): it moves as one block; its bodies are its instances' bodies where it placed them.
- **Flexible**: its instances join the parent's solve as `Sub/Inner`, starting where the subassembly puts them, with its
  own mates (by those names), its couplings, and its world taken as where the subassembly is placed; the subassembly
  itself is not solved, so its free motions stay free in the parent; it places the same as rigid while nothing in
  the parent moves its instances. A flexible subassembly's own flexible subassemblies come along the same way, to
  any depth.
- **Its drives** (`asm::flex`, 2026-10-30). A free motion the subassembly drives itself (its `<mate>_angle` variable,
  a formula of its own `t`) is **free in the parent unless the parent drives it**: the instance sets, in `params`,
  a public parameter of the subassembly that the drive depends on through its formulas (the drive's variable
  itself, published, or a `swing` or `t` it follows). So a leg that plays its own motion is also a part of a
  machine that turns it from outside, and a parent that wants the leg's own motion passes its `t` on
  (`"params": {"t": "t"}`). The rule is read off the formulas, never the names: each driven motion carries the set
  of variables its value depends on (`NamedSpec::drive_deps`, followed through the formulas with a seen-set, cut at
  10 000 names); the parent keeps the drive when it sets one of them, and then the drive depends, for the parent's
  own parent, on what the parent's values for them depend on. Before, a flexible subassembly brought its drivers
  with the values of its own file, so a driven leg in a parent that drove its crank was over-constrained. Its
  couplings were dropped; they now come along with its mates (`Placed::coupling_specs`, mate indices into its
  specs, moved past the parent's own).

### Commands (`crates/engine/src/asm_cmds.rs`)

`assembly.new`, `instance.add`, `instance.edit` (a rename rewrites the mates that name it), `instance.delete` (refused
while a mate names it), `instance.open` (the source in its own tab, so it is edited there; a packed one as a new
document), `mate.add`, `mate.edit` (a rename takes its variables along), `mate.delete` (with its variables; refused while
a coupling names it), `coupling.add`, `coupling.delete`, `joint.set` (sets or removes the assembly variable
`<mate>_angle`, `_distance`, `_x` or `_y`, which formulas may use and `variable.publish` may publish),
`assembly.solve`, `assembly.inspect` (`measures`: each instance's volume and box), `assembly.reload`, `assembly.follow`,
`assembly.pack`, `assembly.unpack`; `measure.joint` and `measure.pose` (`asm_measure.rs`, *Measures* below). A mate whose name or geometry fails is kept and reported failed by name, as a
feature is; so is a coupling whose mate takes no part or whose value does not evaluate (`status`, `error`, and in
`assembly.solve`'s `failed`). A rename of an instance also rewrites the assertions and outputs that name it (`Inst`,
`Inst/...`), and is refused onto a name a check names already; `instance.delete` is refused while a check names it, so
checks never pass to another instance that later takes the name. `assembly.new` keeps the tab's number and the
session's body budget.

A free motion's variable is written `<mate>_angle`, not `<mate>.angle` as the plan has it: a dot in a name is a table
lookup in expressions (`scales[scale].maxNotes`).

**Linear couplings** (2026-10-30). `coupling.add {"kind": "linear", "terms": [{"mate", "motion", "coefficient"},
...], "value"}` ties any number of free motions: `Σ coefficient · <mate>_<motion> = value`, the solver's
`CouplingKind::Linear` (a planetary stage of three revolutes by Willis, two sliders, a slider and a revolute). The
document keeps the terms (`Coupling::terms`, `CouplingTerm`; a linear coupling has no `mates`) and the format writes
them as `terms` without a new version (a field with a default; a build before it refuses the file by the unknown
field). A term's `motion` is the mate's main free motion unless given, its coefficient 1. Units: each motion is in
base units (radians, millimetres) and each length motion's coefficient is taken to the assembly's length unit for
the solver; every term's product must have the value's whole dimension (length, mass and time; `2 kg` or `3 s` as a
coefficient of a distance is refused), an angle counting as a pure number as radians do (so a rack is
`d − 5 mm · θ = 0`), and a value of 0 fits any dimension (the first term's then sets it). A term
whose mate lacks the motion, terms that disagree in dimension, or a value that does not evaluate fail the coupling by
name, as for the other kinds. A mate rename takes its terms along; a mate a term names is not deleted.

### Pack and unpack

`assembly.pack` replaces every file source by the packed document, its subassemblies packed first the same way (to
`MAX_DEPTH`), and a pinned hash by the packed bytes' hash; the file then rebuilds anywhere. A pinned instance is packed
with the bytes it is pinned to, never with what its file holds now; when neither the file nor the assembly holds them
the pack is refused by name. `assembly.unpack {folder}`
writes each packed document beneath the write root, relative to the assembly's file, named after its first instance,
refuses to overwrite unless asked, and refers to the files again with the same hashes. Saving an assembly into another
folder rewrites its source paths so they stay relative to its file: the document written is rewritten, and the open
one takes the rewrite only once the file is written, for its current state and every state undo and redo reach
(`History::rewrite_all`, no undo step of its own), together with its new path. A failed write changes nothing; an
undo after Save As keeps paths relative to the new folder. Each source is followed to its file on disk and written as
the path from the folder the file is written into (`asm_cmds::Relinker`). The file: an absolute path as it is; a path
relative to the root beneath the folder that source was last read beneath (`AsmState::read_from`, per source: what
`resolve_read` made of the path without the path's parts, recorded at `instance.add`, `instance.edit` and every reading
of the sources; session state beside the document, kept through undo, redo and tab switches, gone with the document);
for one not read in this session, beneath the folder of the assembly's file without its relative path's parts, else
beneath the first read root (the working folder with access to every file). The folder written into: the path's
folder when it is absolute, else the write root on disk joined with it. So an assembly an agent made beneath its folder
and never saved is saved by the person at the window (whose Save As has no read root) with paths to the agent's files,
two sources read beneath two folders (the agent given another folder between them) each lead to their own, and with a
read root that is not the write root a relative save leads to the files read. Each path is made by
`asm_paths::relink`, which compares two folders only from the same start (the root, one Windows drive in either case,
one share `\\server\share`) and refuses, never writes another path, where none leads to the file: two drives or
shares, a source that climbs above the start of the file system, a path longer than a source may have. Every path is
made from the texts given; a document keeps the path it was given, as before. The file system's spelling of a folder
(`workspace::canonical`, links followed) is used only to decide whether two folder texts name one place, and only
where it is a plain Unix or drive path (`asm_paths::plain_spelling`; a share, a verbatim `\\?\` path and the browser
fall back to the texts as given): a source read beneath a root spelled one way (`alias`) is written as the write root
spells it where the two are one place. Then the checks: the last line of defence, that the folder written into
joined with the new path is the file the old folder joined with the old path was (both written by `asm::join`; in
memory, where every path is a name in one store, the two names); that from the folder written into as the file
system spells it the new path leads to that file too (a folder reached through a link one level deeper or higher
would make `..` climb elsewhere on disk: refused in words, saying the folder's own path); and that this session,
reading a path kept relative beneath its read root, would not read another file by it. A path whose first part is
`..` (not one that starts with two dots, `..x`) climbs out of the root and is not read at all: the root refuses it,
the bytes read stay, and the save's answer says so (`notFollowed`, and a warning the status bar counts). A
refused source refuses the save, naming the instance and the ways out (another folder, `instance.edit` with a
`source`, `assembly.pack`), with nothing written; a source that climbs above the start of the file system is kept as it
is only in a save into the assembly's own folder. An assembly without a file whose sources were never read in this
session, saved where there is no read root, is refused so (the folder is not known). A state undo or redo reaches whose
sources cannot be rewritten keeps its paths. The files the open assembly read are carried over to the paths it reads
them by after the save (`Relinker::carry`: the same files, so the same bytes and stamps), and the save does not read the
saved assembly's own sources again (its follow of the other open assemblies skips it): its instances stay placed with
whatever access the save had, and a source changed on disk is taken in by the next follow (the window's focus,
`assembly.follow`), not by the save.

### Export (`crates/engine/src/asm_export.rs`, `crates/io/src/step_assembly.rs`)

`file.export` of an assembly to STEP writes its structure: each distinct body of a configuration once, as a product
with its B-rep as for a part, and one product named after the file that places them, each placement a
`NEXT_ASSEMBLY_USAGE_OCCURRENCE` with a `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` whose `ITEM_DEFINED_TRANSFORMATION`
maps the part's origin frame onto its pose (the structure `craft3d-stepread` reads, so `file.import` of the file gives
one body per placement where the assembly put it). Subassemblies are written flat. An invisible instance, and a
subassembly's invisible ones, are left out, as the mesh exports and the scene leave them out. STL, 3MF and OBJ hold the placed
bodies, named after their instances.

### Measures

The assertions of an assembly are measured on the shown part, so a subject is an instance by its name: `clearance`
(`from`, `to`), `overlap` (interference, with the common volume in its message) and `mass` (from the source bodies'
fixed materials) work as on a part. Subjects are stored by name, never by the shown part's feature ids, which follow
the order of the instances. One rule names them (2026-11-10, A-007, A-010, A-031): a name of a shown body as before;
else, by `asm::body_paths`, every placed body has a path `Instance/Body` (the body by the name of the source feature
that made it; for a subassembly's bodies `Sub/` and their paths in it, `Sub/Inner/Body`), and a name is every body
whose path it is or begins up to a `/`: an instance of several bodies or a subassembly by its name, a one-body
part's body as `Cube/Block`, `Sub/Inner`. The pose measures take a path into a rigid subassembly
(`Placed::pose_of`: the inner instance's pose followed by the subassembly's). The `measure.*` commands measure an
assembly on its shown part with its placement, as its assertions are (before, they looked up names in the assembly's
own document, which has no features).

**What the solve says** (`asm_measure.rs`, 2026-10-30). The measures read the placement the checks are taken on
(`Ctx::placed`, handed through the assertion jobs; a moment's placement in `motion.check`):

- `jointAngle {mate}` (radians, an angle) and `jointDistance {mate, motion}` (the assembly's length unit; the mate's
  first length motion unless `motion` names `distance`, `x` or `y`): the solved value of a free motion, driven or
  not (`MateState::values`, every variable of the mate);
- `position {instance | point, axis}`: an instance's origin, or the origin of the frame a mate would take from
  geometry through an instance (`point`, as `view.trace` names it);
- `rotation {instance, axis}`: a component of the instance's rotation vector (the log of its rotation,
  `Mat3::log`), or without `axis` the angle it is turned by.

A mate that takes no part, an instance that is not placed or a solve that failed is an error saying why. The key
of an answer holds the values and poses it reads (`asm_measure::key_text`), so a joint set again measures again.
`variable.measure` makes them outputs, which formulas of outputs and the assertions' bounds use: this is how the
solved values of free motions are readable in formulas (the assembly's own variables are evaluated before the
solve and cannot read it). `measure.joint {mate}` (angles in degrees) and `measure.pose {instance}` (position,
rotation vector in degrees as `instance.add` takes it, angle, axis and the three axes) or `{point}` answer the same
as commands. A rename of a mate or an instance takes the checks that name it along; a mate a check names is not
deleted.

### The solver in the engine (`crates/engine/src/asm_solve.rs`)

The engine hands track B's `craft3d-assembly` its problem: the instances that are placed (a flexible subassembly's
own instances among them), each mate with its two frames in its instances' coordinates, its offset and angle, its main
free motion's limits, and every free motion that has its variable (`<mate>_angle`, `_distance`, `_x`, `_y`) driven to
that variable's value; the couplings by mate index. The world is one more instance after them, grounded at the
identity, and a mate to `world/...` is a mate to it; when no mate reaches the world, the first instance is grounded
where it is placed and the result says `floating`. A mate of the engine puts `b`'s frame against `a`'s turned over
(face to face), so it is handed to the solver with `flip` negated. The solution's poses place the instances, each
mate's variables are its values (`assembly.solve` reports the main one), the degrees of freedom and the free motions
in words are reported as they come; a `Diagnosis` fails the solve with its message (the mates, couplings, drivers and
limits by name), and the instances then stay where they are placed. The solution's note (a rest solve whose carried
pose differs from the solve straight from the placement or that was not carried, a free joint not kept) is
`assembly.solve`'s `note`. The last eight rest solves are kept by their exact problem (`SolveMemo`, in the assembly's derived state):
an assembly placed again with nothing its solve reads changed takes the answer it had.

### Regeneration elsewhere

A host that regenerates elsewhere (the web page's worker, `Session::set_regen_elsewhere`) has an assembly placed there
too (since 2026-11-03; before, `regen_elsewhere()` was false for an assembly and the page placed it a piece at a time).
The state crosses with the sources this session read for it (`view_elsewhere::state_bytes`: the `.3dcraft` file, the
assembly's path and folder, and each file of `AsmState::files` with its bytes or why it could not be read; a packed
assembly carries its sources in the file), and the other side installs them in its own `AsmState` as reading them would
have (`view_elsewhere::install`), so it places from the same bytes: the same configurations, poses and scenes, bit for
bit. Its motion's moments are made there as well, and its bake. The host keeps a stand-in per view for the scenes, and
reads the outline and the joints a drag moves from what the other side said of the view (`ViewFacts`); while the next
moment is made the tree lists the last one of the same state. `joint.set` there answers without solving (`status:
"pending"`, `deferred: true`); the checking worker measures the assertions of the assembly's state with its sources;
a read that needs the placement and was not placed on the page runs in the worker. What still places on the page: an
edit run there that reports what it solved (`instance.add`, `mate.add`, `variable.set`, `assembly.solve`,
`assembly.inspect`), and `view.trace` and `motion.check`, which make their own sweep. Without the worker (none, or
`?pageregen`) the page places it a piece at a time, as before ([`engine.md`](engine.md) › *Motion and assemblies in
the worker*).

### Tests (track A)

`crates/engine/tests/assembly.rs`: a stack placed by fixed mates at 1e-3, 1 and 1e6, saved and reopened; two
configurations of one source built once each and only the changed one rebuilt; two bodies picked from one source with
one regeneration and a body that is gone failing by name; an edited source followed after a reload and a removed face
failing its mate by name; a rigid subassembly and a mate through it, and cycles refused; a flexible subassembly solved
with its parent; pack and unpack equality with the source file deleted; the STEP export read back by our import, and
the mesh exports, each imported body's box against its instance's; clearance, overlap and mass in closed form; a
revolute driven by `joint.set`; the refusals; a subassembly following its own part and its folder; a failed save and an
undo after Save As; couplings that fail by name; a pin through a file change, a pack and a reopen; checks through a
rename; a mate on a hidden body; the shown scenes after the workspace narrows; the assembly's `document.check` not ok
for an instance that is not placed (a value past its part's range, a source gone or not a document), a failed mate,
coupling or solve, and each inside a rigid and a flexible subassembly, and two subassemblies down in each of the
four pairs of modes, and answering as before for one that is right, also with a hinge at either end of its limits, a
fixed mate given twice, nothing held to the world, a pinned instance whose file changed and a part without a body.
`regen_elsewhere.rs` has the assembly
built here by a host that regenerates elsewhere, `recovery.rs` the unsaved assembly that asks, is autosaved and comes
back with its sources, `crates/io/tests/step_freecad.rs` FreeCAD reading an assembly (`cargo xtask freecad`).
`crates/engine/tests/assembly_save_paths.rs` saves between folders relative to the root and absolute ones (the report of
2026-11-18), in memory, under hostile paths, the refusals, the way back for a file saved wrongly, and the person's
Save As at the window of an assembly an agent made (one and two read roots, a part added by its absolute path, an undone
addition, two tabs of two folders);
`crates/engine/src/asm_paths.rs` has the rewritten paths of Unix and Windows (drives, shares) as text.
`crates/engine/tests/panic_hunt.rs` runs every command with adversarial parameters. `crates/doc/src/assembly.rs` and
`crates/engine/src/asm.rs` have the unit tests of the rules and the paths, `crates/engine/src/asm_solve.rs` the memo of
rest solves. The format: `crates/format/tests/versions.rs` (the fixture) and `crates/engine/tests/format_versions.rs`
(its regeneration). `crates/engine/tests/published_assemblies.rs` pins the published example assemblies (and the
walker at six crank starts, and its leg opened alone at three leg scales) to goldens recorded on the parent of the
assembly-trust fixes; what A-016 and A-036 change is declared there entry by entry and pinned to a second golden: the
walker at 180° (two legs' boxes, three assertions that now pass, two measures over the motion, the note) and the leg
alone at another leg scale (four parts' poses, two joints back at the values they were placed at); and the check of
each published assembly as 0.2.0 answered it.

### In the window (track C, phase 2)

- **The tree** (`Session::assembly_outline`, `crates/engine/src/asm_outline.rs`; drawn in `ui-egui/src/shell.rs`
  `tree`): the left panel of a `.3dcraftasm` is headed *Assembly* and lists its instances, each followed by its
  subassembly's (nested, read from the subassembly's placement; a flexible one's own instances are placed here and
  selectable, a rigid one's are not: their bodies are the subassembly's), then the mates with their mark, kind,
  value and `driven`, and the degrees of freedom left (or why the solve failed). Marks, a click to select, a double
  click to fly and an eye per row work as for features; the eye issues `instance.edit {visible}` and *Open* issues
  `instance.open` (the source in a tab of its own). It is read from the placement the view shows: a moment of the
  motion when one is shown. **Its verdict is the check's** (2026-11-20): the outline carries what `document.check`
  lists for the assembly (`AssemblyOutline::problems`, `asm::wrong_in` of the placement shown), and the heading
  (`AssemblyVerdict`, `ui.get` › `assembly`) says *Solved* with the degrees of freedom left only while that is
  empty; else *Not solved* when the solve failed, or how many problems it lists (*2 failed*), each in the check's
  words on hover. Each is on the row it names: an instance that is not placed is red with why (a flexible
  subassembly's too, from the subassembly's own placement, and a flexible subassembly inside a flexible one is a
  row of its own with its instances under it), one placed although a feature of its part failed is marked ⚠; a
  subassembly with something wrong in itself (its mates, its couplings, its own solve unless it is flexible:
  `asm::wrong_within`, the tail of what `wrong_inside` lists) is marked ⚠ with those lines under its row
  (`OutlineInstance::inside`); a failed mate is red; a coupling that takes no part is a red row after the mates,
  with why (one that takes part is not listed). A form of the assembly's public parameters says the heading's
  words in the place of *Valid*, and weighs a commit by the check's problems ([`document.md`](document.md) ›
  *Good and broken are sets*).
- **Joints** (`ui-egui/src/shell/joints.rs`): a mate whose main free motion can be set by hand and leaves the
  assembly solved (`OutlineMate::settable`) is a slider on the Parameters tab, between its limits or a turn either
  way (a distance: the model's size either way). Settable means: it has a free motion, no formula of other
  variables drives it (a driver of `t` is played, not dragged), and either a value drives it already (setting
  another keeps the degrees of freedom as they are) or the solve leaves it free: its first variable moves in the
  null space of the solve's Jacobian (`asm_solve::main_moves`, at 10⁻³ of the fastest variable of a free motion,
  distances at the solver's scale), so a driver of it adds one independent equation and contradicts nothing.
  Whether the null space moves a variable does not depend on the basis the solver found. So a closed loop of one
  degree of freedom offers the joints of the loop that move to first order where it rests while none is driven
  (a joint at a stationary point of the loop is not offered there), and only its driven one once one is; a
  second driver in the loop, which made the assembly *Not solved*, is never offered. **Alt and a drag** on a part
  moves it along the first such mate of its instance (in the assembly's order): an angle by the sweep of the
  pointer about the joint's axis on the plane across it (unwrapped frame to frame, so a drag goes on past half a
  turn), a distance by the point of the axis nearest the pointer's ray; the value moves the way the part does
  (`JointHandle::sign`: −1 when the part is side a, whose frame the value is measured in). A part held to the world
  (by a chain of fixed mates, or as the first instance of a floating assembly) is not dragged. Both issue
  `joint.set` through the shell's queue, where a later value of the same joint takes the place of an earlier one,
  so the window never waits for solves it would throw away; every value after a drag's first carries `merge`
  (folded into the drag's undo step: one drag is one undo step, as a parameter slider's).
- **Mate markers** (2026-11-05; `ui-egui/src/mates.rs`, `shell/mate_view.rs`): per mate a small glyph at its
  frame, drawn while `show.mates` is on (the default; the display bar's first toggle is *Mates* in an assembly, in
  the place of *Sketches*, which an assembly has none of). The frame is `OutlineMate::frame`: side a's frame of the
  solve's mate moved by its instance's pose (side b's when a is the world), read from the assembly's own entries of
  `Placed::specs` (`NamedSpec::own`: a flexible subassembly's mates are there too, under their own names, and a mate
  of the assembly that failed must never take one of theirs), so no face is measured again per frame; a failed mate
  has none. `OutlineMate::parts` names the two instances. It crosses to the page with the outline (`view_elsewhere`,
  bit for bit). The glyph: a small ring at the origin in the frame's plane, the axis an arrow along z with a short,
  narrow head, and by kind three quarters of a turn about the axis with its head (revolute), the axis on below the
  origin with a head there too (slider), both (cylindrical), a square in the plane (planar), a crossed box (fixed),
  three circles (ball); circles and arcs in 64 pieces a turn, so they stay round at the size drawn. A frame whose
  axes are not finite and square, or a length that is none, gives no marker (never a guessed one). Its length is
  `tokens.mate_size` points at the camera's target (`Projector::footprint` at the camera's distance, as the world
  axes keep their size), so an orbit leaves the lines as they are and only a zoom or a resize makes them again; it
  is part of the frame key, and the markers go with the parts while a motion plays. The segments are
  `Marks::mates` (and `mates_hot` for the one pointed at or chosen), drawn by both paths of the viewport from that
  one list, after the datums, tested like a datum but dimmed less behind a body (`tokens.mate_hidden_opacity`; on
  the GPU a segment's own hidden opacity, `Segment::hidden_opacity`): a mate's origin lies on or in a body almost
  always. The GPU's picture is compared with the oracle (`gpu_viewport.rs`,
  `the_direct_path_draws_mate_markers_and_matches_the_oracle`, the markers' samples held to their own 99 %), opaque:
  over the four-bar's see-through bars the two pictures differ at 4 pixels beside the bars' edges also without
  markers, on Intel and AMD alike, a gap of the see-through oracle that is owed.
- **The markers' look** (2026-11-06, the developer: the first look was far too heavy). At rest a marker is quiet:
  1.25 pt lines (`tokens.mate_width`, times the pixels per point) in a muted indigo (`tokens.mate`, #6870B0, the
  info accent's family and lighter than the sketches', which an assembly does not show), 40 pt long, on a hairline
  of the light halo colour 2.25 pt wide (`tokens.mate_rest_halo_width`, `LineKind::MateRestHalo`): on the light
  viewport it is not seen, on a dark part (a brass gear) it gives the line its edge. The marker pointed at or
  chosen is the one the eye finds: the selection's family in a deep copper (`tokens.mate_hot`, #B05C14), 2 pt wide,
  over a 3.5 pt light halo (`LineKind::MateHalo`, only under the hot marker). Only a chosen mate tints its parts,
  in a very pale copper (`tokens.mate_part`, #F6E4CC), and not a part held to the world (`OutlineInstance::grounded`:
  a housing, a held ring gear); pointing at one lights its marker alone. The tint replaces the part's colour; a
  blend over it (both renderers, with parity) is owed after R1. Contrast: the
  resting colour is 3.8:1 on the viewport and 3.3:1 on a body's grey, the hot colour 3.9:1 and 3.4:1, both 3:1 or
  more against the halo (`theme.rs`, `mate_markers_read_over_the_viewport_and_a_body`). The lines are opaque, not
  translucent: each segment is blended on its own on both paths, so a translucent arc would bead at every joint of
  its pieces; the muted tone and the thin line make it quiet instead. Behind a body a marker is drawn at 120 of
  255. The variants compared were a neutral slate, this indigo, a muted bronze and the slate without a
  hairline.
- **Pointing at mates**: the pointer on a mate's row on the Assembly tab, or within `tokens.mate_hit` points of a
  marker's line where the marker is in sight, sets `sidePanel.mateHover`: the marker is drawn hot and its two parts
  are tinted `tokens.mate_part` while no face is lit otherwise (a flexible subassembly on a side: its instances; a
  failed mate too, its parts are where they are; not under the studio look's lens with something selected). Where a
  body's face is in front of the marker the pointer and a click are the face's (its depth against the marker's), so
  faces around a joint are still picked. The tooltip names the mate and its kind; markers as near as the nearest,
  within a pixel (two coaxial joints), are named together in the mates' order, and the first is pointed at. A click
  on a row or a marker chooses the mate (`sidePanel.mate`, the row marked; a click on its row again, elsewhere in the
  viewport or another document lets it go) and a click on a marker picks nothing.
- **Joint sliders on the Assembly tab** (2026-11-05): under each mate row whose joint can be set by hand
  (`settable`, as on the Parameters tab) is the same kit slider (`Shell::joint_slider`, one function for both tabs),
  named by its motion (*Angle*, *Distance*, *X*), in degrees for an angle, between the mate's limits or a turn
  either way, through the same queue and `merge`: one drag is one undo step. The Parameters tab keeps its sliders.
  Every other mate with one value to hold (a revolute's angle, a slider's distance, a cylindrical's angle) that is
  solved has a slider too (`Shell::drive_slider`, 2026-11-06): one a formula of `t` drives, one that follows another
  through a coupling or a closure. Dragging it holds the joint by hand for the view (*Driving a joint by hand*,
  below), not an edit; while it holds the joint the slider says so under it (*View only · Esc*, or why it stopped short in a word: *Dead point · Esc*, *Limit · Esc*; the value it stopped at is the slider's, the whole reason its tooltip); a double click on its knob, Esc (after an open popover), a moment of the motion or an edit lets go.
- **Driving a joint by hand** (`view.drive`, `engine/src/drive_cmds.rs`, 2026-11-06): view state of the open
  document (`OpenDocument::drive`, kept for its document state and the history's revision, so an undo and a redo
  let go too), as a moment of the motion is; the document, its undo history and its dirty
  state never change. The assembly is placed with a `Hold` (`asm.rs`): the held mate's main free motion (or the one
  named) is driven at the value, and the drivers that would contradict it are let go of: none when the mate's own
  variable drives it (the value takes its place) or the solve leaves it free (`MateState::main_free`); else the
  first other driver, in the assembly's order, whose release frees it (each tried with a solve without bodies), or
  all of them; when none frees it the command says the joint is held by its mates. The others follow: couplings
  (the planetary's carrier turned by hand turns the sun by Zr/Zs on it), closures (a four-bar's rocker turns the
  crank). The solve is carried from the document's placement along a fixed grid of the held value (5° for an
  angle, 1/72 of the assembly's size for a distance), each step from the one before and kept for the next value of
  the same joint, then from the last grid value to the value: so a pose is a function of the value alone (as a
  moment is of `t`), and a periodic joint is never carried past a dead point onto the mirrored branch a whole turn
  further, which one long advance did (a crank-rocker's rocker asked for +180° landed on the mirror at −180°).
  Each solve starts its joints the full turns nearest those of the solve it is carried from (`Carried::values`,
  `Solver::solve_at_near`): the poses alone say an angle only within a turn, and a coupled shaft many turns round
  (the reducer's input at ten times its output) started a turn off left its coupling a multiple of 2π·ratio
  unmet ("did not converge", residual ±16, at 560° of the output). Where a step cannot be solved (a dead point, a
  limit) the last value reached is found by halving that whole grid step (30 times, over the step whatever value was asked, so where it stops does not depend on the value that found it first) and shown, and kept (`Walk::limit`): a
  later value past it is shown there without a solve; `stopped` says where and why. When even that pose cannot be
  placed the command fails and the view stays as it was, never the document's pose instead. At most 720 grid
  steps either way (10 turns, a solve without bodies each); further is refused at once. With collisions on (the switch of `view.motion` and the player) the held pose is checked as a
  moment is (`motion::collisions`, the motion's memo of pairs), lazily when the pose is shown or answered, once per
  pose and switch (`Driven::collisions`), the colliding bodies red (`motion::marked`); the window's player counts
  them and its caption names them (*Hinge held · collision: Tray and Lid*). A slider being dragged asks with
  `later` (the pose alone: the check costs about a second on the developer's Pi case, the pose 15–50 ms) and the
  drag's end asks `view.drive {}`, which checks; in the browser the switch goes with the ask (`DriveAsk`'s fourth
  field) and the worker checks. The viewport,
  `view.render`, `view.pick`, screenshots and the outline show the held pose (`shown`, `shown_regen` and
  `shown_placed` take it before a moment of the motion); measures, checks and exports see the document. In the
  browser the joint is held in the worker: the host keeps the ask (`Drive::remote`), the view's stand-in and the
  job of scenes carry it (`ViewJob::drive`, its value as its bits), the worker holds it with `view.drive` before it
  makes the scenes (the same joint at another value goes on from the solves it keeps; another joint, or none, lets
  go), and its answer crosses with the facts (`ViewFacts::drive`); one the worker refuses crosses as an error
  with the request it answers (`Drive::refused`: mate, motion and the value's bits), and the page lets go of it and says why once (`view.drive {}`); the page places nothing
  (`engine/tests/view_drive.rs`).
- **Selections** of an assembly are made in its placed part, whose features are numbered in instance order (an
  instance that is deleted or fails gives its id to the next). Each item is anchored to the name of the placed
  feature it was made on (its instance: `Crank`, `Crank/Bar`) and found by that name at every check, never by the
  id (rule 4): an instance that is not placed for now hides it, one that is gone drops it. Checks are keyed on the
  revision and the generation of the files read (`assembly.reload` changes the placement without a revision).
- **Rows** of the tree carry their instance's source (`InstanceState::source`), also inside a rigid
  subassembly, and the whole tree holds at most 2,000 rows at any depth.
- **Mates from two faces**: with two faces of two of the assembly's own instances selected, a right click offers
  *Revolute between these*, *Fixed*, *Slider*, *Cylindrical*, *Planar* and *Ball* above *Select other*; one issues
  `mate.add` with the faces named through their instances.
- **The web build** is the same shell: the tree, the sliders and the motion work on the page; an assembly is solved
  and placed in the page's worker, its moments made there (*Regeneration elsewhere* above, since 2026-11-03).
- `.3dcraftasm` is among the extensions Open offers, on the desktop and the page.

### Findings and history (track A)

- 2026-11-20 (window-verdict): the window told a person *Solved* and *Valid* while `document.check` on the same
  assembly said not ok (since 0.2.1, below). The Assembly tab's heading read the solve of what was placed alone, so
  an assembly whose plate its part refused (a corner of 40 mm past its range of 30) and whose mate on it failed was
  *Solved · 0 DOF*, with two red rows under it; a coupling that takes no part (a linear one on a motion its revolute
  lacks) was in no outline, so the heading said *Solved · 1 DOF* and nothing anywhere said why `assembly.solve` was
  not ok; a flexible subassembly listed only what its parent placed, so its instance that was not placed showed
  nowhere; a subassembly's own mates and solve were shown nowhere (*instance `Sub`: its mate `Ground`: …*); and a
  form of the assembly's public parameters weighed only red instance rows against the last good state, so a value
  that failed a feature of an instance's part (the row is ⚠, not red) or past a hinge's limit (the solve) left it
  *Valid*, as did an assembly that opened unsolved. Principle broken: one truth; the window had a second, weaker
  rule. Rule: the window's verdict on an assembly is what the check lists for it, and each problem is shown where
  the person looks for the thing it names. Built: `AssemblyOutline::problems` is `asm::wrong_in` of the placement the
  view shows (at rest the document's, which the check reads; a moment of a motion or a held joint while one
  is shown, which the check does not see); `AssemblyVerdict::of` makes the heading from it (*Solved*,
  *Solved · n DOF* only while it is empty; *Not solved* when the solve failed; else *n failed*, the check's words one
  per line on hover), and `ui.get` › `assembly` gives it (`{ok, text, problems}`, `null` for a part). The outline
  carries the couplings (`OutlineCoupling`), and the Mates section draws one that takes no part as a red row after
  the mates with why (`Shell::coupling_row_hits`); one that takes part is not listed, so no assembly that is right
  looks different. A row of a placed subassembly carries `inside`, `asm::wrong_within` (split out of `wrong_inside`,
  which lists the same in the same order: its mates, its couplings, its solve unless flexible), shown under the row
  in the failure colour and on hover, the row's source marked ⚠ (`ui.get` › `features[].inside`); its instances'
  problems are on their own rows. A flexible subassembly's rows are its own instances in its order (`push_flexed`):
  each placed in the parent as the parent holds it, one it could not place from its own placement with why (before,
  not listed), and a flexible subassembly of its own as a row (before, the list went from the outer one straight to
  the inner one's instances) with its instances under it; any placed in the parent that this walk misses are still
  listed after. A form of the assembly's parameters weighs a commit by those problems, each keyed by what it names
  (`instance Sub`, `instance Sub 2` for a second one of the same instance, `mate Ground`, `solve`), so a value that
  adds one *Breaks the model* and one that only changes a reason does not; and its header says the heading's words
  in the place of *Valid* while the check lists anything, whatever the last good state (`ui.get` › `parameters` ›
  `status`). Not changed: what a command answers, the file format, a part's form (still weighed against its last
  good state alone, so a part that opens with a failed feature says *Valid*: whether it should not is a question
  for parts, open), and the Checks section, which says a broken assertion of the assembly itself while the heading
  may say *Solved* (the heading speaks of the placement; whether it should also weigh the assertions, which
  `document.check` does, is open). The outline crosses to the page with its new
  lists (`view_elsewhere`, bit for bit). Tests: `crates/ui-egui/tests/assembly_verdict.rs` (each case before and
  after, the problems in `ui.get` equal to the check's, a right, a floating, a redundant and a limited assembly still
  *Solved* and *Valid*), `state::tests::an_assembly_is_solved_only_while_its_check_lists_nothing`, and the crossing
  in `view_elsewhere::tests`.
- 2026-11-19 (assembly-check): an assembly's `document.check` answered `ok` with no problem while an instance was not
  placed: one plate given a corner radius of 40 mm where its part's range ends at 30, one fixed mate on it.
  `assembly.inspect` gave the instance `error` (*its configuration failed: nothing was changed: `corner`: its value
  40 mm is outside its range, from 0.5 mm to 30 mm*), the mate failed (*its instance `Plate` is not placed: …*),
  `assembly.solve` listed both under `failed`, and the check said `{"bodiesChecked": 0, "ok": true, "problems": []}`.
  The same in 0.1.3 and 0.2.0. Cause: the check asked an assembly one question only, whether an instance was placed
  although a feature of its part had failed (`featureFailed`, A-023); the placement's other failures, which the
  assembly's own commands report, were never asked about, and a part's checks (its features' statuses, its bodies)
  see nothing of them: the shown part holds only the bodies that were placed, all built. Principle broken: an agent
  that trusts the check got a green light for an assembly that is not what it says. Rule: the check is not ok while
  `assembly.solve` is not, nor while a subassembly's own `assembly.solve` would not be (its own solve left out when
  it is flexible). It now lists (`asm::wrong_in`), in the form of a part's problems, each with the key that names it
  and `problem`: each instance that is not placed, whatever the reason (its configuration failed, its source is
  gone or holds no document, a cycle, a pinned copy missing, the body it picks gone), as `instance` with *instance
  `Plate` is not placed: …*; each placed although a feature of its part failed, as before; each mate and coupling
  that takes no part, as `mate` and `coupling` with *mate `Ground`: …*, a driver that does not evaluate among them;
  the solve when it failed, as `"solve": "failed"` with *the assembly does not solve: …*, mates that contradict
  each other or a driver past its limits; and through a subassembly each of these inside it, as `instance` with its
  instance's name (*instance `Sub`: its instance `Plate` is not placed: …*, *… its mate `Ground`: …*, *… it does
  not solve: …*), to any depth; a flexible one's own solve is left out, its mates being solved in its parent's, where
  a contradiction is the parent's. Each reason is the one `assembly.inspect` and `assembly.solve` give. Not asked:
  `held` (a pinned instance, or a source that changed and could not be read again, which keeps the version read
  before and is said in `assembly.inspect` and the Instances panel), and `floating` (no failure in `assembly.solve`
  either). An assembly that is right answers as before, to the byte: a rigid and a flexible subassembly, a mate into
  one, a driven hinge and a gear coupling (`a_right_assembly_checks_as_before`), and the four published assemblies
  the program carries, pinned to what 0.2.0 answered (`the_published_assemblies_check_ok_as_before`). The two of the
  examples' repository the program does not carry were measured with 0.2.0: the task lamp (no subassembly) and the
  CoreXY printer answer ok, `assembly.solve` ok with nothing failed, and each of the printer's four rigid
  subassemblies, opened at the values its instance gives it, solves with nothing failed: nothing the check now asks
  about is there. Measured since by the review on the build of the fix (Check 44622852028, from a scratch branch
  never merged): all six, as published (format 69) and as rebuilt at 0.2.0 (format 77), at their defaults and at
  t = 0 to 1 by a quarter, each check ok with no problem and solve with nothing failed. `document.sample` and
  `craft3d-cli check-range` check each point with this check, so a point of an assembly where an instance is not
  placed fails now. `document.inspect` (`craft3d-cli info`, MCP's `doc_inspect`) gave the same green light by
  another door: its `ok` is `Regen::is_ok`, nothing failed to build (no variable, no feature), and an assembly's
  regeneration is the placed part, whose features are the placed bodies, all built, so only a failed variable of
  the assembly made it false. It keeps that meaning and reads it for an assembly as its placement: false also when
  `wrong_in` finds anything (all of it a failure to build or to be placed), so it agrees with the check on an
  assembly; a part's answer and a right assembly's are unchanged, and a part with a broken assertion still answers
  ok there, whether the checks pass being `document.check`'s to say (the command's summary, the MCP tool, `info
  --help` and `control-protocol.md` now say what `ok` means; the protocol's example had a failed feature beside
  `"ok": true`). It lists no reasons of its own for an assembly: `document.check`, `assembly.inspect` and
  `assembly.solve` give them. The test of A-023 held the wrong ok itself: at a corner the
  part takes, its mate on `Plate/F1:face(1)` names a face of the blank, whose body went into the fillet and is not
  shown (the plate's faces are `F2:copy(0)/F1:face(k)`), and fails (*no body has a face `F1:face(1)`*), while the
  check said ok; its expectation was changed to that problem, and to ok once the mate names the plate's face. A
  mate does not follow a short name to the face it became, as a feature that takes the body and `file.export` do
  ([`names.md`](names.md) › *References that follow a body*): whether it should is a question, not part of this
  fix. A check that measures an instance that is not placed (a clearance on it) said *there is no feature named
  `Plate`*; it now says why (*instance `Plate` is not placed: …*, `Placed::pose_of` in `verify_cmds::subject`). The
  window did not take its verdict from the check; it does since 2026-11-20 (window-verdict, above).

- 2026-11-18 (asm-save-path): an assembly whose sources were relative to the working folder (the window's read root),
  saved under an absolute path, got source paths that climbed to the file system's root and then named the path from
  the working folder (`../../../../../../../../../model/v2/rod.3dcraft` from `/home/…/work/abs`), and `file.save`
  answered ok; the open assembly then said its source `/model/v2/rod.3dcraft` was outside the read root, and the file
  opened with every instance failing. In 0.1.0 to 0.1.3. Cause: the rebase joined the old folder with each source and
  took the path from the new folder by comparing parts as text (`asm::relative_to`, which drops the empty part an
  absolute path starts with), so a folder relative to the root and one from the file system's root were compared as
  if both started at one place; and a path it could not write (another drive) silently kept the old one. Principle
  broken: a save must never write a source path that no longer leads to the file it led to. Rule: each source is
  followed to its file on disk and written from the folder written into, checked by text before anything is written,
  and a path that cannot be written refuses the save (*Pack and unpack*). The command line
  did it right because with access to every file every path is kept absolute (`workspace::kept_path`), so both
  folders were of one kind. What a file saved wrongly holds is a path that names the file from the working folder,
  which the file does not record: no repair on opening is sound (a path that climbs to the root can be meant), so
  such a file is mended by pointing each instance at its file again (`instance.edit {"name", "source"}`) and saving.
  A source that is not there beneath the root now says "there is no such file" (as `file.open` does), not the
  system's text; "outside the read root" stays for a path that leaves it. The case the bug was met by is the person's
  Save As at the window of an assembly an agent made and never saved: that save has no read root, so the folder each
  source was read beneath is recorded where it is read (`AsmState::read_from`), and the files read are carried to the
  paths the saved assembly reads them by; before, the save also read the saved assembly's sources again with its own
  access (none) and they showed as failed until the next `assembly.follow`. The review (review-asm-save,
  `crates/engine/tests/review_asm_save.rs`) found two more saves that said ok and wrote paths that did not lead to the
  files: two sources read beneath two folders (the agent given another folder between them; one folder for the whole
  assembly could not be right for both), and a relative save with a read root that is not the write root
  (`craft3d-cli serve --read-root a --write-root b`, paths written as if the file were beneath the read root). Both
  lead to their files now (a folder per source; the folder written into on disk), and a last check by text before the
  write refuses any path that would not. After the second, the open assembly holds a path that climbs out of its
  read root: it keeps showing the part, read before, but cannot follow it until opened again by its full path, and
  the save says so (`notFollowed`). The second round of that review (`review_asm_save_2.rs`) found the check's text
  comparison wrong in three places: a folder named `..x` taken for one that climbs out of the root (the session then
  followed one part while the file named another), a save into a folder reached through a link (the path led there by
  the link's spelling, not on disk), and a root spelled two ways (`alias` and its target: the agent's own session
  refused the path written, and the reverse refused an honest save). A path now climbs only when its first part is
  `..`; two folder texts are one place where the file system spells both as one plain path; and a save into a folder
  reached through a link, where the path written would not lead to the file on disk, is refused in words. Its third
  round found that keeping a document's path in the file system's spelling (as a first answer did) broke Windows:
  `asm::join` writes a share (a mapped drive's) or a verbatim path as another path, which was then kept, shown and read
  from; that was taken out, and no resolved spelling is kept, shown or written. Open, as on main: an assembly opened
  by an absolute path through a link to a folder at another depth joins its source paths with the link's text, so it
  does not find them (it never did); saving it there is refused, saving it by the folder's own path works. Still
  refused, where the folder is truly not known: an assembly without a file whose sources were never read in this
  session (opened from bytes, or recovered where they cannot be read) saved where there is no read root. A recovered
  assembly without a file reads its sources beneath the first read root of the session it is recovered in (the
  working folder with access to every file): found when that is the folder they were added beneath, as at the
  window's next start with the same folders; the recovery record does not hold the folder. Open: (1) A save by the
  person at the window follows the other open assemblies with the save's access (no read root): their sources cannot
  be stamped, so they are read, the read is refused, and they keep their last bytes marked as not followed, a mark
  that can stay until the file changes or `assembly.reload` (its stamp is not taken again). (2) `asm::join` writes a
  Windows share (`\\server\share\…`) as `/server/share/…`, so an assembly on a share cannot read its sources;
  `asm_paths::relink` compares shares written with backslashes, but reads `//server/share` as a path from the root
  (as Unix does), and the paths are read through `join` (a bug of its own). Tests:
  `crates/engine/tests/assembly_save_paths.rs`, `crates/engine/tests/review_asm_save.rs`,
  `crates/engine/src/asm_paths.rs`.
- 2026-11-14 (assembly-trust, A-016 and A-036, for 0.2):
  - A-016: the rest solve was one Newton solve from the placement at the document's drives. A mechanism put together
    at one drive value and solved at another could converge to the mirrored loop, the solve ok with no degree of
    freedom left: the Jansen walker at a crank start of 180° (two legs on the mirror, three assertions failed at rest
    and over the motion), a lone Jansen leg put together at 100° and solved at 260°, near-change-point crank-rockers
    solved past the pose where their circuits pass closest (965 of 10,368 exact placements in `rest.rs`). Cause: the
    placement is a solution at the drive values its poses hold, not at the document's; one damped Newton solve across
    that difference is not a path. Rule: the rest solve is also carried from the values the placement holds, and the
    carried answer replaces the direct one where they differ (the Method's *The rest solve is carried too*).
  - A-036: the task lamp with its turret swivel left free rested with the swivel at 6.6° from 0 where it was placed,
    and the solve said nothing. Cause: the Newton step is the least-squares step in all unknowns alike, so a misfit is
    closed by everything that can turn about the swivel's axis, a third each. Rule: after a rest solve the free joints
    that alone carry a free motion are carried back to their start values (*Free joints keep their start values*).
  - The first review (on `release/0.1`): a carried answer from a placement whose mates do not hold is as blind as the
    direct one, so it now says the branch is not certain; notes only on evidence; the choice of kept joints made
    independent of the mates' order and the assembly's size; the published assemblies pinned to goldens of the parent.
  - The second review's blocker, measured: the carry, in half-radian sub-steps checked against half their predicted
    motion as a motion's are, crossed a crank-rocker's circuits near the pose where they pass closest and replaced a
    right 0.1.2 answer by the mirrored circuit without a note (couplers 3.02 and 3.002: 61 of 863 and 67 of 864 exact
    placements). Neither a tighter continuity check (a twentieth of the motion: right for these couplers, but by the
    luck of a secant that cut the chord of parts turning far from their origins; with the rigid secant, 21 of 864 at
    3.002 crossed) nor the corrector's contraction (fast on the other branch too) sees it: a long step lands on the other
    branch's straight continuation, past the sharp turn of its own. What sees it is how near the solution is to a
    singular pose, which the corrector's quadratic convergence gives (`1/ω`, *How the carry steps*): sub-steps bounded
    by it, a sub-step that moved further than it at its end cut, and a first sub-step short (a placement ten degrees
    before the pose crossed in its first). Measured: every one of 31,104 four-bars of the review's probe and 41,472 of
    a wider one (coupler 3.0002 too, three scales, the bodies' origins at the world's or at their pivots) on its
    circuit; every answer the parent got right the same line, iterations included (the direct solve kept); notes then
    on placements whose mates do not hold. The review's other probes: the Jansen sweep 5,184 and the disturbed legs
    432 all on their branch (the parent: 161 and 9 off), 76 lead-screw placements within 2.4·10⁻⁸ of the closed form
    (47 failed on the parent), gear trains and the 72 moments of a geared motion and the 1,728 motion sequences bit
    for bit.
  - Moments of a motion and joints held by hand are not carried and keep no free joints: they start from a solved
    state that holds its own drives, and are solved exactly as before.
  - The first review's skip (no carry when the direct solve moved nothing further than one sub-step may) is gone: a
    direct solve near a change point can cross to the other circuit in less than half a radian, and the second
    review asked for a test, not a heuristic. The carry now runs wherever the placement holds other drive values.
  - Cost, measured (release; rest solves of Jansen legs on one crankshaft placed at crank 0 and solved at 0.5 to 4.5
    rad, medians of five, load 2 to 18): 74 instances 10–17 ms on main's base, 25–95 ms here (10 to 51 sub-steps);
    302 instances 0.35–0.40 s against 1.9–3.1 s, the carry giving up at its allowance of sub-steps and the direct
    solve standing (without the allowance 2.9–5.4 s, out of its 2000 iterations at 1,494 sub-steps); solved where
    placed, the same. A sub-step that succeeds after a cut no longer doubles straight back, and the engine keeps the
    last eight rest solves by their exact problem. In the engine (development build, six placements of the published
    radial engine and walker at other drive values, two runs): 2.10–2.32 and 1.98–2.33 s a placement on main's base,
    2.16–2.20 and 2.20–2.40 s here, within the noise of a loaded machine.
  - The third review, off the earlier grids (some 63,000 new rest solves): answers 0.1.2 got right came out on the
    other circuit without a note: an offset slider-crank (crank 1, rod 1.52 or 1.502, offset 0.5; 25 exact
    placements), four-bars of coupler 3.00002 (23) and 3.0000002 (94). `ω` stayed small across the slider-crank's
    sharp turn at 270° (it grows only near a singular Jacobian) and a half-radian sub-step stepped over it. Rules: the
    step is a tenth of a radian (`REST_STEP`), and wherever the carried and the direct solution differ the answer says
    so (*The rule*), since a step can always be beaten by a geometry nearer its change point. Found on the way: the
    corrector's damping of 10⁻⁹ made the walker's Newton steps converge linearly, so `ω` grew as its predictions got
    better and its sub-steps shrank without end (325 for 60°, out of the allowance at 180°); `MU_CARRY` (10⁻¹²) gives
    20 and 54. A stopped carry no longer claims a dead point it did not prove ("not carried past … the branch is not
    checked"), and one out of its allowance says so. Measured on the reviews' probes, 134,751 rest solves and the
    1,728 motion sequences: no answer that differs from the direct solve's lacks a note (but the five free-joint cases,
    where A-036 keeps joints as designed); an answer without a note that is not the closed form is the direct one, as
    0.1.2's (399 four-bars: 341 at coupler 3.0000002, 58 non-Grashof ones placed with the crank turned, whose carry
    stopped and so says nothing); notes on right
    answers: 4,504 "carried" and 4,046 "not certain" (each where 0.1.2 was wrong), 148 "not carried past" (non-Grashof
    four-bars and five-bars driven across a fold, where 0.1.2 was right and the carry stopped at the fold). Cost:
    twelve legs on one crankshaft 27–100 ms against 10–16 ms; fifty legs 0.43–0.78 s against 0.34–0.39 s, the carry
    now getting there.
  - The fourth review (fingerprints of every pose and joint value over 78,355 rest solves): no answer that differs from
    the direct one without a note but the five free-joint cases; the 399 wrong without a note bit for bit the parent's;
    5,506 answers 0.1.2 had wrong right now, each with a note. Its three changes: a note where the direct solve failed
    and the carry answered from a placement that meets its mates (5 on right answers in its probes, the crank-rocker
    of `linkages.rs` among them); the "carried" note reworded to claim only the disagreement (in 93 exact placements
    0.1.2 had right the carry itself jumped: 89 at coupler 3.0000002, 4 at 3.00002); the damping's trade-off written
    down (*How the carry steps*).
- 2026-11-14 (assembly-trust, A-030): two lead screws whose nuts ride one bed, screw A driven 24 turns over the
  motion, screw B turned by nothing but its screw coupling: `assembly.solve` was ok at `t = 0`, `motion.check`
  stopped at `t = 0.65` (15.7 turns, "did not converge in 200 iterations"), and the rest solve failed from 16.8 turns
  (`t = 0.7`). Cause: each moment is solved from the poses of the moment before, and a pose says an angle only within a
  turn, so B started in (−π, π] with its coupling a whole number of leads unmet; the solve walked it back at most half a
  radian an iteration, 100 rad in 200. The rest solve started A the turns nearest its drive and B at 0, the same
  walk. The principle broken: an angle whose turns an equation counts (a coupling) was not carried with its turns.
  Rule: a moment starts the angles a coupling takes at the turns they had; a rest solve that fails is solved again
  from the turns the placement's couplings say, or carried (the Method's *Couplings*). The motion now runs its 73
  samples, screw B within 2·10⁻⁴° of `−24 · 360° · t`, and the rest solves at 16.8 and 48 turns. Second review
  (measured): a first version started every moment's coupled angles at their turns and so moved the numbers of any
  geared motion once a shaft had turned more than half a turn (a gear train: 45 of 72 moments by up to 2·10⁻¹⁴; the
  radial engine's least and greatest values by up to 3·10⁻⁷ through its clearance search), and it carried any failed
  rest solve, so a crank-rocker that had failed with "did not converge" came out on its other circuit. Now a moment
  tries the start it always had first, and only a coupling's turns are carried: every moment and rest solve that
  converged before gives the same bits (the published assemblies are pinned to goldens of the parent,
  `engine/tests/published_assemblies.rs`), and that crank-rocker is again the error it was
  (`linkages.rs`; for 0.2 A-016's carry, above, solves it on its own circuit).

- 2026-10-30 (asm-gaps, the gaps the assemblies of three Jigs found): world references checked when a mate is
  made; linear couplings; the measures `jointAngle`, `jointDistance`, `position`, `rotation` and the commands
  `measure.joint`, `measure.pose`; a flexible subassembly's drives free in the parent unless it sets what they
  depend on, and its couplings carried; an unsaved assembly placed from where its sources were read; a part in
  another length unit regenerated in the assembly's; `overlap`
  checks only the pairs whose boxes meet (a sweep along x, `verify_cmds::meeting_pairs`), at most 2000 of them, an
  error beyond that instead of 500 pairs of any kind; `motion.check`'s memory (`motion.md`).

- 2026-10-29: first version (track A): the document kind, format 52, instances with configurations, picked
  bodies, pins and subassemblies, cross-file names and frames, a stand-in solver, placement into the scene, pack and
  unpack, the structured STEP export, measures by instance.
- 2026-10-30 (review fixes): subassembly configurations keyed by their folder and dropped when the files read change;
  the save rebase applied only after the write, to every undo state; assemblies built here by a host that regenerates
  elsewhere; an unsaved assembly asks before closing and is autosaved, and a recovered one reads its sources; coupling
  status; pinned bytes kept packed and used by `assembly.pack`; the STEP export skips invisible instances; checks
  follow instance renames; mates on hidden source bodies fail. FreeCAD (`cargo xtask freecad`) read the first STEP
  assembly export as no shape: each part's origin frame sat in a separate shape representation its definition does
  not name; it is now an item of the part's B-rep representation, which the occurrences relate (as OpenCASCADE's own
  writer does), and FreeCAD reads the three placed solids to 1e-15. The window's scene not following `joint.set` was not
  reproduced in the engine (test `the_shown_scenes_follow_the_joints_after_the_workspace_narrows`: absolute open,
  workspace narrowed to a root, display refined elsewhere, Preview and Display scenes, stepwise and not).
- 2026-10-29: the stand-in replaced by track B's `craft3d-assembly` (on main since 3233785f): closed loops, couplings, limits and the diagnosis now come from the solver; the engine maps its frames, drives and the world onto the crate's `Problem`.
- 2026-10-30 (asm-motion-p2): the window bug reported with track A (`joint.set` at 60°, 150° and 240° drew one
  picture, `ui.fit` found nothing to frame) does not reproduce on main 24ad5692: not in real shell frames (CPU, with
  and without the refiner), not at the window's desk with its engine thread and refiner and its own session (the
  user's authority narrowed to the automation roots, one source by absolute path)
  (`ui-egui/tests/assembly.rs`, `apps/craft3d/src/app.rs` `the_window_follows_joint_set_on_an_assembly`). The likely
  cause, fixed in track A's review before the merge, is the base of an unsaved assembly after the workspace narrows
  (`the_shown_scenes_follow_the_joints_after_the_workspace_narrows`); that is not proven, since the bug was never
  caught in a test. The two tests stay as its guard; what neither covers is the real eframe/wgpu frame loop.
