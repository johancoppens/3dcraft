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
  - screw: `da − lead θb / 2π = 0`, so `lead` is per turn of 2π radians (the plan's 360°). `a` and `b` may be the
    same cylindrical mate.
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
is deterministic.

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
  - frame-by-frame `advance`, with wrong drive counts and NaN refused.
- **`couplings.rs`:**
  - a gear pair at its ratio, with the pitch points rolling without slip;
  - a rack moving `r θ`;
  - a screw advancing `lead · θ / 2π` (three leads after three turns);
  - a planetary stage through two linear couplings against Willis' formula (scale 1);
  - couplings naming a variable the mate does not have are refused.
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
  instance by name), regenerated once. Instances of one configuration share it whichever body they pick. The
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
`assembly.solve`, `assembly.inspect` (`measures`: each instance's volume and box), `assembly.reload`,
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
undo after Save As keeps paths relative to the new folder.

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
the order of the instances.

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
limits by name), and the instances then stay where they are placed.

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
rename; a mate on a hidden body; the shown scenes after the workspace narrows. `regen_elsewhere.rs` has the assembly
built here by a host that regenerates elsewhere, `recovery.rs` the unsaved assembly that asks, is autosaved and comes
back with its sources, `crates/io/tests/step_freecad.rs` FreeCAD reading an assembly (`cargo xtask freecad`).
`crates/engine/tests/panic_hunt.rs` runs every command with adversarial parameters. `crates/doc/src/assembly.rs` and
`crates/engine/src/asm.rs` have the unit tests of the rules and the paths. The format: `crates/format/tests/versions.rs`
(the fixture) and `crates/engine/tests/format_versions.rs` (its regeneration).

### In the window (track C, phase 2)

- **The tree** (`Session::assembly_outline`, `crates/engine/src/asm_outline.rs`; drawn in `ui-egui/src/shell.rs`
  `tree`): the left panel of a `.3dcraftasm` is headed *Assembly* and lists its instances, each followed by its
  subassembly's (nested, read from the subassembly's placement; a flexible one's own instances are placed here and
  selectable, a rigid one's are not: their bodies are the subassembly's), then the mates with their mark, kind,
  value and `driven`, and the degrees of freedom left (or why the solve failed). Marks, a click to select, a double
  click to fly and an eye per row work as for features; the eye issues `instance.edit {visible}` and *Open* issues
  `instance.open` (the source in a tab of its own). It is read from the placement the view shows: a moment of the
  motion when one is shown.
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
  255. The variants compared (a neutral slate, this indigo, a muted bronze, the slate without a hairline) are in
  `plan/ui-audit/mates-v2/` (local).
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
  below), not an edit; while it holds the joint the slider says so under it (*View only · Esc releases*, or where
  it stopped and why in a few words: *Stops at 138.6° (dead point) · Esc releases*, the whole reason in its
  tooltip); a double click on its knob, Esc (after an open popover), a moment of the motion or an edit lets go.
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
  limit) the last value reached is found by halving the step (30 times) and shown, and kept (`Walk::limit`): a
  later value past it is shown there without a solve; `stopped` says where and why. When even that pose cannot be
  placed the command fails and the view stays as it was, never the document's pose instead. At most 720 grid
  steps either way (10 turns, a solve without bodies each); further is refused at once. The viewport,
  `view.render`, `view.pick`, screenshots and the outline show the held pose (`shown`, `shown_regen` and
  `shown_placed` take it before a moment of the motion); measures, checks and exports see the document. In the
  browser the joint is held in the worker: the host keeps the ask (`Drive::remote`), the view's stand-in and the
  job of scenes carry it (`ViewJob::drive`, its value as its bits), the worker holds it with `view.drive` before it
  makes the scenes (the same joint at another value goes on from the solves it keeps; another joint, or none, lets
  go), and its answer crosses with the facts (`ViewFacts::drive`); one the worker refuses crosses as an error
  (`Drive::refused`), and the page lets go of it and says why once (`view.drive {}`); the page places nothing
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
- 2026-10-29: the stand-in replaced by track B's `craft3d-assembly` (on main since 3c885c35): closed loops, couplings, limits and the diagnosis now come from the solver; the engine maps its frames, drives and the world onto the crate's `Problem`.
- 2026-10-30 (asm-motion-p2): the window bug reported with track A (`joint.set` at 60°, 150° and 240° drew one
  picture, `ui.fit` found nothing to frame) does not reproduce on main 765efa6e: not in real shell frames (CPU, with
  and without the refiner), not at the window's desk with its engine thread and refiner and its own session (the
  user's authority narrowed to the automation roots, one source by absolute path)
  (`ui-egui/tests/assembly.rs`, `apps/craft3d/src/app.rs` `the_window_follows_joint_set_on_an_assembly`). The likely
  cause, fixed in track A's review before the merge, is the base of an unsaved assembly after the workspace narrows
  (`the_shown_scenes_follow_the_joints_after_the_workspace_narrows`); that is not proven, since the bug was never
  caught in a test. The two tests stay as its guard; what neither covers is the real eframe/wgpu frame loop.
