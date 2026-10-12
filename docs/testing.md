# Testing: the AI loop and its oracles

> **Last updated:** 2026-10-12 · **Status:** the loop is in use. In place: oracle 1 (closed-form mass properties, `testkit::solids`), oracle 2 (validity checker with seeded defects), oracle 3 (boolean identities on seeded random pairs), oracle 4 (tessellation against the exact body, for patches), oracle 6 (constraint solver), oracle 7 (CPU side) and oracle 8 (FreeCAD). The rest is design

3DCraft is written by AI agents while the developer watches. That only works if an agent can tell, without a human, whether what it built is right. This document says how.

## The loop

```text
 1. write the test      state what must be true, before or with the code
 2. run it              the narrowest command that exercises it
 3. check               read the actual output: numbers, the validity report, the picture
 4. improve             fix the code (not the test), then go to 2
 5. widen               run the crate's suite, then the gate in AGENTS.md §5
 6. record              devlog entry: what landed, numbers, what is still open
```

Rules of the loop:

- **A test that never failed proves nothing.** See it fail for the right reason first (or break the code on purpose once), then make it pass.
- **Read the output.** "exit code 0" is not a check. For geometry, look at the numbers against the expected values; for anything visual, open the PNG.
- **Never get green by weakening.** Don't loosen a tolerance, delete an assertion, mark a test `#[ignore]` or special-case the fixture. If the behaviour really is out of scope, return an `Unsupported` error, test *that*, and write the limitation in the dev log.
- **Tolerances are stated, not tuned.** Every approximate comparison names its tolerance and why (modelling tolerance, chordal tolerance, floating-point bound). A tolerance changed to make a test pass needs a justification in the commit.
- **Changing an expectation is a recorded decision, not a fix.** When a test fails there are two suspects, the code and the expectation, and the agent wrote both. An expectation may change only when all of these hold:
  1. **The reason is a fact about the mathematics, the specification or the measuring method**, stated in a comment at the assertion, and true whatever the code prints. "The lip of the spout folds inward, so outward there means towards the symmetry plane" is a reason. "The code returns 7.99" is not.
  2. **The new expectation is not weaker than it has to be.** Prefer replacing a wrong claim with a different, checkable one (normals agree across a seam) over deleting it or widening a tolerance.
  3. **Something independent of the code under test still pins the behaviour down:** a closed form, a second computation by another route, an external program, or a picture a human looked at. Where no such oracle exists (wording of an error message, a default value), say so in the test's comment: it is then a regression test, not a proof.
  4. **It is logged.** The dev log entry lists every expectation that was changed and why, under its own heading, so the developer can review them without reading the diff.

  If any of the four is missing, the code is the suspect and the test stays.
- **Shrink failures.** A failure found on a big model becomes a small synthetic regression test before it is fixed.
- **Stop and ask** when three honest attempts fail the same way, or when making a test pass would need a decision listed as open in [`decisions.md`](decisions.md).

## Why CAD is checkable

PhotoCraft's main oracle is free: every PSD contains Photoshop's own rendering. A CAD kernel has no such file. It has something better: **mathematics**. Most of what a kernel produces has properties that are known exactly or must hold by construction.

### Oracle 1: closed-form mass properties

`topo` computes volume, area, centroid and inertia from the exact B-rep. For shapes with textbook formulas the expected value is known to machine precision.

| Shape | Checks |
|---|---|
| Box a×b×c | V = abc, A = 2(ab+bc+ca), centroid at the centre |
| Cylinder r, h | V = πr²h, A = 2πr(r+h) |
| Cone, sphere, torus | V = ⅓πr²h; V = 4/3πr³, A = 4πr²; V = 2π²Rr², A = 4π²Rr |
| Extrusion of a profile | V = profile area × height (profile area from Green's theorem on the sketch) |
| Revolution of a profile | Pappus: V = 2π × centroid distance × profile area |
| Box with a through hole | V = abc − πr²h |
| Filleted box edge | V = abc − (1 − π/4) r² × edge length |

`testkit` *(planned)* provides these as reference solids with their expected values, at several scales and after arbitrary rigid transforms (mass properties must be invariant or transform predictably).

### Oracle 2: topological invariants

Every body an operation returns must pass `topo::check`: Euler–Poincaré (V − E + F − (L − F) = 2(S − G)), each edge of a solid used exactly twice in opposite directions, closed loops, curves on their surfaces and vertices on their curves within tolerance, consistent orientation. Expected entity counts are asserted where known (a box has 6 faces, 12 edges, 8 vertices; a box minus a through cylinder has 7 faces).

### Oracle 3: algebraic identities

Checked on generated inputs (proptest), by comparing mass properties and validity:

- `vol(A ∪ B) = vol(A) + vol(B) − vol(A ∩ B)`
- `vol(A − B) = vol(A) − vol(A ∩ B)`
- `A ∪ B` equals `B ∪ A`; `A ∩ A = A`; `A − A` is empty; `A ∪ ∅ = A`
- an operation followed by its inverse transform leaves mass properties unchanged
- pattern of *n* disjoint copies has *n* × the volume

As built for the booleans (M7, `crates/kernel/tests/boolean_random.rs`): seeded pairs (our own SplitMix64, no proptest) of blocks, cylinders, L-shaped extrusions and slots with a hole, in three placements (each turned at random; both turned alike, so axes and faces are parallel but nothing coincides; on a half-unit grid along the world axes, so faces are flush, touch and share edges), at scales 1e-3, 1 and 1e6 with the tolerance scaled along. Per pair the six booleans (both orders) must each give a body that passes the checker, an empty result, or `Unsupported`; then the volume identities above must hold to 1e-9, with the two area identities `area(A ∪ B) + area(A ∩ B) = area(A − B) + area(B − A) = area A + area B` where nothing is coincident, and `(A − B) ∪ (A ∩ B)` must have the volume of A (its pieces meet in coincident faces, which is what that union exercises).

The identities compare the booleans with each other: a kernel that missed every intersection would satisfy them. So the intersection is also checked against the operands' own definitions: exactly for two axis-aligned boxes, and by sampling 3000 points (within five standard deviations) for every pair. The test run does 120 pairs per placement and scale; the long run (`cargo test --release -p craft3d-kernel --test boolean_random -- --ignored --nocapture`) does 1500 and prints, per placement and scale, how many pairs were solved and how many were refused for which reason. `BOOLEAN_PAIR="seed placement scale"` with the ignored test `one_pair` reproduces one pair. Boolean results also go through the tessellator (`crates/testkit/tests/boolean_tess.rs`, there because `kernel` and `tess` share a layer): the mesh is closed and its volume converges to the closed form.

### Oracle 4: tessellation against the exact body

- the mesh of a solid is a closed, consistently oriented 2-manifold (every edge shared by exactly two triangles, opposite directions)
- every mesh vertex lies on its face's surface within floating-point error; sampled triangle interiors lie within the chordal tolerance
- mesh volume and area converge to the B-rep values as the tolerance shrinks, at the expected rate
- identical output for 1, 2 and N threads (byte-equal)

### Oracle 5: determinism and round trips

- regenerate twice → byte-identical bodies
- incremental regeneration after an edit = full regeneration from scratch (built: `Session::regeneration_matches_a_full_one()`; the `run` helper of the engine suites asserts it after every command, and `crates/engine/tests/regen_cache.rs` adds a seeded random walk of edits and the counts of rebuilt features)
- save → load → regenerate → same bodies (compared by a canonical hash) and same feature statuses
- strip every cache from a `.3dcraft` → still opens to the same model
- replay a recorded command script → same document
- undo/redo returns to the exact previous snapshot
- STEP export → (our import, once it exists) → same mass properties and entity counts

### Oracle 6: constraint solver

After a solve, every constraint residual is within tolerance; the reported degrees of freedom match a hand count for known sketches (a fully constrained rectangle has 0); redundant and conflicting constraints are identified on purpose-built cases; re-solving a solved sketch moves nothing.

In place since M5, at two levels that were written independently of each other. `crates/solve/tests` has a catalogue of 26 sketches with coordinates and degrees of freedom worked out by hand in a comment at each case, solved at scales 1e-3, 1 and 1e6, plus every constraint kind alone, purpose-built redundant and conflicting cases, branch stepping and a seeded robustness run. `crates/engine/tests/sketch.rs` drives the same solver through the commands with its own hand-computed sketches (an L profile, a 3-4-5 triangle drawn on either side, a circle on a line), through a change of a variable, undo, and save and reopen.

### Oracle 7: renderers against each other

The CPU renderer has golden images for a small set of scenes (reviewed by the developer when first blessed, because a golden image is only as right as the eyes that accepted it). The GPU renderer is compared with the CPU renderer on the same scenes: id buffers equal away from silhouettes, colours within tolerance. See [`architecture.md`](architecture.md) §7.

In place since M4 for the GPU half (`crates/gpu/tests`). `parity.rs` renders 75 scenes with both renderers on every adapter the machine has and asserts the tolerances of [`architecture.md`](architecture.md) §7.2, each derived in `tests/common/mod.rs` from two facts (the GPU computes in `f32`, and its rasteriser snaps vertices to 2⁻⁸ pixel) rather than tuned: face ids equal outside a band around feature edges, distances within a bound made of the `f32` rounding and the local gradient, colours within one level of 255, and a limit on how many pixels may differ at all. The same file checks exact pixel counts that need no second renderer and that the same input gives the same frame bit for bit on the same adapter. `robustness.rs` holds the GPU renderer to the CPU renderer's errors for the CPU renderer's own bad inputs and covers what only a GPU has (no adapter, a destroyed device, a mesh of another renderer, a target format that cannot take the picture). The tests need an adapter and **fail without one**; a software rasteriser is enough (`mesa-vulkan-drivers` for lavapipe), and `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` narrow the run to one. Pictures (CPU, GPU and their difference side by side) and the measured numbers go to `target/evidence/gpu/`. The renderer was checked by seeding twenty breakages one at a time (a flipped axis, a wrong tie rule, a near plane inside the model, swapped channels, an id that loses a bit, a truncated mean, ...); when a new kind of mistake becomes possible, seed it and see that a test fails.

### Oracle 8: external tools (optional, never a dependency)

For file formats the final judge is another program. Where available on the machine, test helpers may shell out to an independent tool to validate an exported file and read back its volume and bounding box (for example an open-source CAD kernel's command line for STEP, a slicer or mesh checker for 3MF/STL). These tests are opt-in (`#[ignore]`, run by `cargo xtask freecad`), fail with a clear message when opted in without the tool, and the tool is never linked, vendored or read for implementation ideas. In use since the first mesh export: on its first run FreeCAD's point count exposed near-duplicate vertices on collapsed and reversed patch edges that our own tests had not caught. FreeCAD is the approved tool ([`decisions.md`](decisions.md)); tests find it through the `CRAFT3D_FREECAD` environment variable.

## Test kinds per crate

| Crate | Must have |
|---|---|
| `math`, `geom` | closed-form unit tests; proptest on predicates and evaluation/inversion round trips (`project(evaluate(u,v)) = (u,v)`); degenerate inputs (zero length, coincident, tangent, NaN) return `Err` or a documented result |
| `topo` | checker tests with deliberately broken bodies (each defect is detected); mass properties (oracle 1) |
| `expr` | parser round trip, dimension errors, cycles, evaluation order stability; fuzz target |
| `doc`, `format` | serde round trip, older-version fixtures with migrations, malformed-input sweep (truncate at every offset), fuzz target |
| `solve` | oracle 6 |
| `kernel` | oracles 1–3 for every operation; a table of supported geometry-kind pairs, each with a test; failure cases return useful errors |
| `tess` | oracle 4 |
| `regen` | oracle 5; persistent names survive upstream edits in a catalogue of scenarios (the "topological naming suite") |
| `step`, `meshio` | byte-level writer tests against hand-written expected files, parse-back, malformed-input sweep, fuzz target, oracle 8 |
| `render`, `gpu` | oracle 7 |
| `engine` | per-command tests (behaviour, undo/redo, disabled states, bad params); `panic_hunt` |
| `automation` | scripted agent tasks end to end over MCP (build a bracket, change a variable, export, verify volume) |

## The topological naming suite

Because stable references are the property most likely to rot silently, they get a dedicated suite (version 1 since M6: `crates/engine/tests/naming.rs`, with a sketch on a face and a boss extruded from it as the downstream feature; the scenario with a cut waits for booleans). Each scenario builds a model, attaches a downstream feature to a face or edge, edits something upstream, and asserts the downstream feature still refers to the entity a human would expect, or fails with `Missing` (never silently moves). Scenarios: change an extrude distance; add a sketch entity before the referenced one; reorder two independent features; split a referenced face with a cut; delete the feature that made the referenced edge.

## Corpora

Real files are useful oracles but are large binaries, so they never go into this repository. When we adopt a corpus (for example public STEP test models for import, M10) it follows PhotoCraft's rules: fetched into `corpus/` (gitignored) at a pinned revision, verified against a committed sha256 manifest, tests behind a `corpus` cargo feature that fails loudly when the corpus is missing, and each file's licence checked and recorded before the pin is added.

## Evidence the developer can see

A test that renders writes its images under `target/evidence/<crate>/<test>/` (actual, expected, diff). Agents report the paths of the images worth looking at, and for a milestone demo leave a short script the developer can replay ([`development.md`](development.md) › Watching along).

## Performance tests

Benchmarks (criterion or simple timed examples) start with the first heavy algorithm: tessellation of a body with thousands of faces, a boolean between two such bodies, full and incremental regeneration of a 100-feature history. Numbers are recorded in the dev log with the machine and its load. Budgets and a generated scorecard follow PhotoCraft's model once there is a baseline (M4+).
