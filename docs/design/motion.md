# Motion over the timeline `t`

How a design is moved and looked at while it moves: the timeline `t`, the state shown at a moment of it, the trace
of a point, collisions drawn red, checks over the motion and the video of it. The code is
`crates/engine/src/motion.rs` (the state and the commands), `crates/engine/src/replay_cmds.rs` (the video),
`crates/ui-egui/src/shell/player.rs` and `shell/timeline.rs` (the player) and the overlay in
`crates/ui-egui/src/viewport.rs` and `direct.rs` (the traces). Assemblies, which this is for in the end, are
`assemblies.md` (track A); this document is track C's: phase 1 works on any part, phase 2 plays assemblies
(*Assemblies* below).

## Method

### The timeline

The **timeline** of a document is its variable `t` when that evaluates to a plain number (no unit). Formulas of `t`
are the drivers of the motion (a crank angle `360 deg * t`, per leg `crank + 90 deg`); `t` runs from 0 to 1. Nothing
is reserved: a document whose `t` is a length (a board's or a bar's thickness, as several Jigs have it, the
Jansen linkage leg among them) has no timeline, and every motion command says so with the way out (rename it with
`variable.rename`, then `variable.set {name: t, expr: 0}`). The rule is general: whether a document has a timeline
is read off its own variable's dimension, never off its name alone. The value the document stores is its rest
state, what everything that is not the motion sees.

### A moment of the motion (`view.motion`)

A moment is view state of an open document, as a scrub of the history is (`scrub.rs`): not saved, not undone, not a
change. `OpenDocument::shown` is, from the top: the motion's state at its `t`, else the scrubbed state, else the
document. So the viewport, `view.render`, `view.pick`, screenshots and the scene cache (keyed by the shown state's
`Arc`) follow it without knowing about it, while `document.inspect`, measures and exports see the document.

A state is made by a `Sweep`: the document (or the scrubbed state) is its reference, with a copy of the
document's regeneration cache; each moment is the reference with `t` set and its sketches carried there from the
reference's own solution (below), regenerated with the sweep's cache. Hence:

- **Only what depends on `t` is rebuilt.** A feature none of whose inputs changed is reused from the cache, its body
  the same `Arc`, so the session's store of meshes does not mesh it again. `view.motion` reports `rebuilt`: the
  features whose bodies differ from the document's. For a part this is honest work per frame: a feature driven by
  `t` is regenerated at every moment. Assemblies place their instances by transforms only; when they play `t` (phase
  2, after tracks A and B), a moment there costs a solve and transforms, no regeneration.
- **Branch continuity, and a moment that is a function of `t` alone.** Only the sketches carry state from one
  solve to the next (the solver starts from their stored points; every other feature is rebuilt from the document
  alone). Their state at `t` is defined without the path the view took: the reference's `t0` (the document's own
  `t`) and its sketches as last solved are grid point 0; grid point `k` (`t0 + k·MOTION_STEP`, `MOTION_STEP` =
  1/128, a crank driven by `360 deg * t` turns 2.8° a step) is grid point `k ∓ 1`, one step nearer to `t0`, settled
  at its `t`; and the moment `t` is the grid point between `t0` and `t` nearest to `t` (`k` is `(t − t0) /
  MOTION_STEP` rounded toward zero) settled at `t`. A grid point is made once per sweep and kept (the sketches only,
  at most 4096 steps from `t0`). So a linkage solved in a sketch is carried to any `t` in steps of at most 1/128
  and stays on its branch however far the view jumped, and a seek, a play, a trace, `motion.check` and the video
  give the same state at the same `t`, bit for bit (the solver and the regeneration are deterministic, and the
  cache keys a solve by everything it reads). A linkage whose branch turns faster than a step of 1/128 can still
  jump branches; the assembly solver of track B adds step cutting for assemblies. Before 2026-10-30 each moment
  settled from the one shown before it, so a state depended on the path and, in the window, on frame timing.
- **The window and frame timing.** The picture at a `t` does not depend on timing: frame timing only decides which
  values of `t` the player asks for (below); each is the state at its `t`.
- **On the calling thread.** `view.motion` makes its state in the command, on the calling thread, also in the web
  build, where the document's own regeneration and checks run elsewhere (`regen_elsewhere.rs`): a moment costs its
  driven features' regeneration there, synchronously. Playing heavy motions in the web build is phase 2's worker.
- An edit keeps the motion: the next `shown` sees another base and makes the state again from the edited document
  (a fresh sweep). An edit that takes the timeline away (renamed, given a unit) lets go of the motion.

### Collisions drawn red

With `collisions`, each state is checked for overlapping bodies: pairs of the visible bodies whose boxes (grown by
twice the tolerance) meet go to the kernel's interference query (`kernel::boolean::interference`, as
`document.interference`), on the shared thread pool, merged in pair order. Each state's answers are built in a map
of its own: from the memo where it has the pair, else asked. The memo keeps, for the next state, only that state's
pairs (both bodies in it), by the two bodies' addresses and the tolerance's bits (the bodies held, so an address is
not reused), so a pair `t` does not move is asked once over the motion, the memo is bounded by what one state
checks (2000 pairs) and holds no body that no state shows. It is dropped on `{t: null}` and when the base changes
(an edit, a scrub). How many answers are kept never changes what is found (before 2026-10-30 an overflowing memo
was cleared after the answers were looked up in it, and a pair whose answer was dropped read as clear). A pair the
kernel refuses is reported `unknown` with the reason, never clear and never red.

The red is put into the state itself: the copy shown gets `COLLISION_COLOR` as the appearance of each colliding
body's feature, and that feature's face colours are dropped (the nearest entry on a face's way back wins, so the
body's colour then holds for all its faces; `appearance.rs`). Because it is in the scene's face colours, every
renderer draws it: the CPU renderer (the oracle) and the GPU one through the same face colours, in the window, in
`view.render` and in `view.replay`. No renderer has a collision path of its own.

### Traces (`view.trace`)

A point is a vertex by its persistent name (resolved as a selection resolves it, `selection::Resolver`, in the body
of `feature` when given) or a point of a sketch (`{sketch, entity}`, from the sketch's solved points in its frame).
It is sampled at evenly spaced `t` over a range, by one `Sweep` in order, and fails the trace at the first `t` where
it is gone, naming that `t` (rule 4: a name that is gone is never retargeted). A trace is view state of one document
state, at most 32 per document, let go of at the next edit.

**In the background** (2026-11-02, after the developer's field test: a trace of 641 samples asked for while the
router played stopped the play for as long as the command built its configurations). In the window
(`Session::set_bake_in_background`) `view.trace` defaults to `background: true`: the point is first found at
`t[0]` in the command (a wrong name fails at once), then the samples are worked out by the motion's builder on a
copy of the motion's sweep (`TraceRun`, 32 samples a job, each job queued again behind what waits, so a machine
with one background worker still builds a play's configurations in between), with a configuration budget of its
own (256 MiB) and nothing handed back to the motion (taken in, the trace's configurations pushed a bake's frames
out of the budget). The command answers `pending: true`; `Motion::trace_jobs` keeps each pending or failed trace by
name with a serial, so only the last asked for a name is taken, a cleared one is never taken, names are reserved
when asked, and the 32-trace limit counts pending ones. A reset of the builder (`motion.bake {cancel}`,
`view.motion {t: null}`, a new sweep, a motion that cannot be shown, an edit: `check_trace_base`) fails the pending
traces with `let go of: …`. `view.trace {name, wait?}` reads one (its full answer, the error, or pending; `wait`
waits up to 600 s), `ui.get` lists them (`traces: [{name, pending, error, length, closed}]`), and the window says a
failed one in the timeline's problem line. Worked out in the command, a trace uses the motion's own sweep when it
runs over the state the view shows below the motion (the scrubbed one), else a sweep of its own, never replacing the
motion's. A trace's points are the same bit for bit whichever way it was worked out
(`crates/engine/tests/motion_ready.rs`, also after a bake and with a budget of one byte).

The window draws a trace as a polyline over the picture, tested against the depth buffer like a sketch (dimmed
behind a body), in the theme's `trace` colour, in both overlay paths (`viewport.rs` for the CPU raster and the
screenshot, `direct.rs` for the GPU drawing on the window's device). Every path that draws the window's picture
takes its marks from one function, `Shell::frame_marks`, and the traces drawn are part of the picture's key
(`FrameKey::traces`): until 2026-10-30 the two window paths each built their own list without the traces, so the
window showed none while `ui.screenshot` (which takes `Shell::marks`) did. With `grow: true` a trace is drawn only
as far as the moment the view shows (`TraceView::up_to`: the samples at or before `t` in the trace's own direction,
and the point between two samples where `t` lies), whole while no moment is shown, so the player draws it as it
goes. The trace's colour (a deep violet) and width (3 points) keep 3:1 against the plain background, the studio
look's greys and a lit body (`theme.rs`, `a_trace_stands_out_on_both_looks`). `file.export {trace}` writes it to SVG or DXF in its
plane: the plane through it within the document's tolerance (spanned by the point farthest from the centroid and
the point farthest from that line; every point checked against it), its normal turned to point along the world axis
it is most along, x the world axis least along the normal laid into the plane, y the normal crossed with x. A trace
that is not flat, or is a point or a straight line, is refused: it has no flat drawing. Flat is measured against
the document's tolerance and against a millionth of the trace's size (the greatest distance of a point from the
centroid), whichever is less, never below the rounding of its coordinates: a path far smaller than the tolerance is
not flat just because all of it lies within it. A trace is closed when it has more than 2 points and ends within the
tolerance of where it began (two samples at 0 and 1 are a segment there and back), in `view.trace` and in the
export alike. The export refuses `view`, `projection`, `hidden` and `tolerance`, which are about other drawings.

### Checks over the motion (`motion.check`)

Any assertion is checked over the motion, so interference and clearance over the motion are not measures of their
own: `overlap` with `is: false` gives the first `t` of a collision, `clearance` with a `min` the least clearance and
its `t`, and the same holds for any measure the document can assert (assemblies' measures by instance included,
once they move). A copy of the document in a session of its own is set to each `t` (one undo step merged over the
run, so the copy's history does not grow) as the `Sweep` makes it, and its assertions are measured as
`document.check` measures them; the answers are kept by what they depend on, so a measure whose subjects `t` does
not move is measured once. Per assertion: the worst status over the samples; `firstFailure`, `firstError` and
`firstNotProven`, each the first sample of that status with `after`, the last sample before it that passed (null
when none did), so the change begins in `(after, t]`; and the least and greatest value at the samples with their
`t`. `measure` is an assertion written inline and checked only there. It proves what held **at the samples**, no
more: a collision shorter than the spacing of the samples can be missed, so the result says `sampled: true` and
the samples it took; the summary and the guide say "at the samples", never "everywhere".

**Memory streams.** A sample keeps only what the next one needs: the copy's document state replaces the one before
(one merged undo step), its placement is held in place of the one before, and the answers the checks remember are
keyed on the bodies they measured by a `Weak` reference (`assert_check::Key`), which keeps the body's address from
being reused but not the body; an answer whose bodies are gone is dropped, since a new body has another address.
So a sample's bodies are freed once the next sample is placed, and the samples taken so far cost only their numbers.

### Video (`view.replay {motion}`)

`view.replay` with `motion` renders a frame per sample of `t` instead of a step per feature (with `collisions`, red
as above), framed on the box around every state so the camera does not jump, captioned `t = …`, written as PNG
frames that `craft3d-cli replay` turns into an MP4 or a GIF, as for the history. Memory does not grow with the
number of frames: the states are made twice rather than kept (each is a function of its `t`): once for the box
the camera is framed on, then a chunk of twice the render threads at a time, whose distinct bodies are meshed
(those of the chunk before that are still shown are kept, the others let go of; the meshes are found by the body's
address in a map), drawn in parallel and dropped. Frames whose meshes need more than 20 000 000 triangles at once
are refused with a coarser tolerance as the way out. The cost is a second regeneration of the driven features.

### The player

The history player gets a mode: History (the steps) or Motion (`t`). Playing the motion asks for the next moment
once the scene of the one asked for is there (no `view.motion` is waiting), by the time since the last
(`MOTION_SECONDS` = 4 s from 0 to 1 at speed 1, a frame's time capped at a quarter of a second), so a heavy model
plays slower rather than skipping its states. Loop starts again at 0; back and forth turns at 1 (and at 0 again with
loop); without either it stops at 1. A slider over `t`, back and forth and collisions sit above the player's
buttons; the caption over the viewport says `t = 0.250 · collision: Wall and Crank`. Because a moment is a function of its `t` alone, a slow machine shows fewer
moments of the same motion, never other ones. All of it is view state in
`UiState::timeline` (`motion`, `bounce`, `collisions`), so `ui.set` plays the motion for whoever watches. It is
offered only for a document with a timeline.

Since 2026-11-01 the player asks with `shapes: "ready"` (*Shapes in the background*): a moment never waits for a
configuration (except while the window is recorded frame by frame or captured, `Shell::exact_pictures`: then it asks
with `"exact"`, the viewport is not live and the scene is of full quality, so the frames never depend on how far the
builder got), so its clock runs at real time and the window's thread does a solve and a placement per moment (or
only a placement, for a baked frame), never a regeneration. It keeps its clock unrounded and asks for `t` rounded to
the frames of the last bake (`rounded_to_frames`, `MotionView::baked`), so a baked motion asks for nothing that is
not made ahead; a motion that builds as it plays (a moment provisional, configurations building) is baked in the
background at 120 frames, once. While it plays the viewport shows each moment as the quick first picture
(`Shell::scene_quality`, not while recorded frame by frame or captured): one scene per moment instead of two, and no display meshes made and kept for moments gone
the next frame; paused, the moment is refined as any picture is. While a moment is provisional or the builder works
the window polls (`Session::poll_motion`) every 33 ms, so a paused provisional moment settles by itself. The GPU
viewport takes over the uploaded vertices of the parts that did not change (`Onscreen::upload_parts_from`,
`PartsKey::reuse_from`: the same mesh at the same first face id, its faces looking the same; the material table,
whose brushed directions follow the poses, is uploaded anew, also when only poses moved), and keeps the parts of
the last upload of the other quality as a spare, so a moment in which one belt changed uploads that belt.

### Assemblies (phase 2)

An assembly has a timeline as a part has: its own variable `t`. Its drivers are formulas of `t` for the variables
that drive its joints (`Drive_angle = 360 deg * t`, the variable `joint.set` writes), and its moments are not
regenerated: each is a **solve** (`craft3d-assembly`), its instances placed by transforms
(`Sweep::placed_at`, `asm::AsmState::place_moment`). The exception is an instance whose parameters are formulas of
`t`: its configuration changes with `t`, so each moment builds that configuration (a regeneration of the source,
cached by its parameters' values like any configuration).

- **The grid, as for sketches.** The assembly at rest (its own `t`, solved from its placements as `assembly.solve`
  solves it) is grid point 0. Grid point `k` is grid point `k ∓ 1` carried to its `t`: the solver starts from that
  point's poses, solves at its driven values (it is there already) and moves the drives to the new ones by
  `Solver::advance`, which cuts the step while the solution jumps further than the motion predicts. The moment `t`
  is its grid point carried to `t` the same way. So a moment is a function of `t` alone (seek equals play, bit for
  bit), and the assembly stays on the branch it rests on: a four-bar's elbow stays up, whichever way and however far
  the view jumped. Each grid point keeps only poses and driven values (`asm::Carried`), never bodies, and is
  solved without placing them (`AsmState::solve_moment`): only the moment shown copies the bodies to their poses.
  A moment that cannot be carried from the one before it (the instances or the driven values taking part changed
  in between: an instance failed) is solved from the placements instead, and says so: `solveNote` in
  `view.motion`, `note` in `assembly.solve`. It may then lie on another branch.
- **What the view keeps is keyed on the files too.** The moment shown, the sweep (which holds a copy of the files
  read) and the traces are kept for the document state and for `AsmState::generation`, which `assembly.reload`
  raises without a new revision: after a reload the moment shown is made again from the files as they are.
- **Dead points.** Where a step cannot be solved (the mechanism reaches a limit position: a four-bar whose crank
  cannot go round, at `|B − D| = b + c`), the motion stops: that grid point and every one beyond it keep the last
  state reached and say where it stopped and why (`Stuck`). `view.motion` returns it as `deadPoint {t, why}` and
  shows the parts where they stopped; `motion.check` lists every dead point met over its samples (`deadPoints`).
  A dead point is reported at the last grid point reached, so within one step (1/128) before the true one.
- **What a moment shows, measures and draws.** The shown part of a moment is built as the assembly's own placement
  is (`build_shown`), so the scene, `view.render`, collisions (named by instance), `view.replay` and the tree read it
  as they read a part. `motion.check` measures an assembly's assertions at the moment's placement
  (`AsmState::hold`), not at a fresh solve from the placements, which could land on the other branch.
- **Traces through instances.** In an assembly `view.trace` names its point as a mate names geometry, through an
  instance (`Coupler/End2`, `Foot/F1:vertex(2)`): the origin of that frame, moved by the instance's pose
  (`asm::point_in`).
- **Costs: moments move meshes, they do not make them** (2026-10-30). A moment is one solve per grid point first
  reached plus one, and placing the bodies: a rigid copy of each B-rep (picking, collisions and measures read the
  bodies), whose mesh is its source's mesh moved. `build_shown` notes each placed body with its source and pose
  (`meshes::note_placed`, only a rigid motion that does not mirror: a mirror turns triangles inside out), and the
  store of meshes gives a noted body the mesh of the body at the end of its chain (subassemblies nest), made or
  found as any mesh, moved by the composed pose (`MeshStore::mesh`); its facts are the source's with the vertices
  moved, and the display rule's size reads the source's area (`meshes::measured_as`). So the scene of a placed body
  is always the moved mesh of its source, whether that was found in the store or made now (the store's rule), and
  no moment meshes or measures a placed body. A placed body's display mesh is thus its source's mesh moved in
  `f64`, not bit for bit the mesh the tessellator would make of the placed body; exports (`file.export`, the
  assembly's STL, 3MF, STEP) tessellate the placed bodies themselves and stay deterministic as before. The sources are meshed again only when a sampling changes: the quick
  first picture's tolerance follows the box around the bodies (`preview_size`), which moves with the mechanism, so
  when it steps to another rung of the ladder the sources are meshed once at it (the walker: 19 bodies over 61
  moments). The scene says so per body (`SceneBody::placement`: the source mesh and the
  pose), and the viewers draw an assembly as parts: the GPU renderer uploads each source mesh once
  (`Renderer::upload_parts`, `Onscreen::upload_parts`) and a moment of the same parts writes their poses, 48 bytes
  a part (`set_poses`; the vertex shader moves each part by its pose, `placed` in `scene.wgsl`), on the window's
  device (`direct.rs`) and the offscreen one (`backend.rs`). The CPU renderer and picking draw the scene's mesh,
  which is the moved meshes. Still uploaded per moment: the lines over the viewport (the bodies' edges), and the
  scene's joined mesh is still copied together on the CPU (no meshing).

### Configurations over a motion (`motion.bake`)

An instance whose parameters are formulas of `t` (the router's four belts: `xc = X_Travel_distance + 150 mm`) is a
new configuration at nearly every moment, and so is every grid point the moment is carried through (a grid point is
solved, and the solve reads the instance's geometry). What keeps that affordable (`crates/engine/src/asm.rs`,
`crates/engine/src/bake_cmds.rs`, 2026-11-01):

- **Incremental builds.** A part given parameters is regenerated from the regeneration cache of the last
  configuration built of its source (`AsmState::regen_caches`, one per source hash and length unit, only for
  sources given parameters), so only the features its changed parameters reach are rebuilt
  (`regenerate_cached_needing` with `Need::All`: the result is that of `regenerate`, body for body). Several
  configurations of one source built side by side each start from a copy of that cache; the cache kept afterwards
  is the last job's in the order the jobs were gathered (the instances' order, state after state), which changes
  only the time. How much this saves depends on the part: a
  hole driven by `t` at the end of a history rebuilds two features; the belt's `xc` reaches every feature but one
  tooth, so the belt gains nothing from it (findings).
- **Kept by memory, not by count.** Configurations are kept within a byte budget (`DEFAULT_CONFIG_BUDGET`: 1 GiB
  native, 128 MiB in the browser; `Session::set_config_budget`). Beyond it, first those only ever solved go (a
  grid point needs its configurations for its solve alone, once per sweep), then those placed with their bodies
  (`Config::shown`), each least recently used first, ties in key order, never one the placement just made uses;
  eviction runs at the top of a placement only, so a subassembly placed while its parent builds does not let go of
  what the parent has just built. A configuration's size is estimated (`built_bytes`) as every distinct body of
  its regeneration (`craft3d_regen::body_bytes`) and as much again for names and records: the router's belt (940
  faces; 0.74 MB of its own bodies and 1.23 MB of bodies later features took) is estimated at 4.0 MB and measured
  at 4.4 MB resident per regeneration kept. Before, at most 64 configurations beyond the last placement's were
  kept and then all of them were dropped at once, so the router's 120 frames rebuilt on every play. A motion's
  sweep holds its own copy of the files and configurations (`AsmSweep::asm`), so what a play builds stays with the
  sweep until an edit or `assembly.reload` makes a new one. The session's store of meshes keeps meshes by body
  within its own byte budget (held weakly: a mesh lives as long as its body), so a configuration kept is a mesh
  kept.
- **Made ahead.** `motion.bake {frames, collisions, limit}` makes the moments `k / frames` (both ends) ahead in the
  motion's sweep (the one `view.motion` then uses). For an assembly: first the grid points the frames are carried
  through, outward from the reference, 32 at a time (`BAKE_BATCH`): their configurations built side by side
  (`AsmState::prebuild`: the first step of a placement for each state, then every missing configuration in one
  batch), then the points solved one after the other; then the frames' own configurations, side by side, kept for
  the play. For a part: the moments themselves, regenerated side by side from copies of the sweep's cache and kept
  (`Sweep::kept`) within the same budget. With `collisions` each moment's collisions are found too and kept by `t`
  (`Sweep::collided`), which `view.motion {collisions: true}` takes instead of asking the kernel. A moment made
  ahead is the one `view.motion` would make (unbaked = baked, bit for bit, also with a budget of one byte:
  `crates/engine/tests/motion_bake.rs`). `limit` builds at most that many configurations or moments per call, for
  a host that bakes between its frames (the browser), which calls again until `done`; `overBudget` says how much of
  what the frames ask for the budget could not keep (those are built again when shown). The player asks for
  moments at frame timing, not at `k / frames`: a moment between the frames baked costs its own configurations,
  so the player rounds `t` to the baked frames (*The player*).
- **Shapes in the background** (`view.motion {shapes: "ready"}`, `crates/engine/src/motion_build.rs`,
  2026-11-01). A viewer that plays asks for each moment with `shapes: "ready"` (for that call only) and is
  answered at once (`Sweep::placed_ready`): when the grid point of `t` is solved and every configuration `t` asks
  for is kept, it is the exact moment (`placed_at`); otherwise a **provisional** one: each instance whose
  configuration is not kept is shown with the kept configuration of its own nearest in `t` (`AsmSweep::seen`
  remembers which configuration each instance asked for at which `t`; the configurations at rest are the last
  resort) and listed in `stale`, and the solve is carried from the solved grid point nearest `t` straight to `t`
  (`Solver::advance` cuts its steps). A provisional moment is never kept in the grid, so the grid stays exact (seek
  = play) and the moment, made again once what it lacked is in, is the exact one bit for bit
  (`crates/engine/tests/motion_ready.rs`). The rest is built by the session's **builder**, a few threads of its own
  (a quarter of the machine, at most 3; `Builder::available` is false in the browser, where `"ready"` is
  `"exact"`, made where it is asked for): the configurations a provisional moment lacks (urgent, at the front),
  **runs of the grid** (16 grid points solved one after the other on a copy of the motion's state, building what
  they ask for as a placement does, one run at a time toward the grid target: the last provisional moment's grid
  point or the farthest a bake reaches) and **runs of a bake's frames** (8 frames each, one run at a time). Results are taken in by
  `Session::poll_motion` (at the start of `view.motion` and `motion.bake`, and every frame of a window while a
  moment is provisional or the builder busy), configurations sorted by key, grid points only next to a solved one;
  what arrives only changes when a moment becomes exact, never what it is. Nothing a run does is shared with the
  session until it is taken in (a copy of `AsmState` and of the document), and a new sweep (an edit, a reload,
  `{t: null}`, `motion.bake {cancel: true}`) moves the builder to a new epoch: a run asks between its grid points
  and frames whether it was let go of and stops there, a configuration being built is let go of when it comes,
  and dropping the builder (closing the document) tells its threads to stop without waiting for them. A grid run
  that panics or cannot make a moment is recorded (`AsmSweep::grid_stopped`) and not started again for that
  sweep, and a provisional moment is made again only when something was taken in (or once the builder, busy when
  it was made, is idle), so nothing is asked for every poll.
- **Provisional only on screen** (2026-11-02, after an independent review). Only the answer of a `view.motion
  {shapes: "ready"}` and the window's own viewport (`Session::set_motion_live`, which the shell sets each frame)
  show a provisional moment. Every command runs with it off (`Session::execute`), so `view.render`, `view.pick`,
  exports, assertions and a `view.motion` without `"ready"` see the exact moment, made where it is asked for if it
  must be; so do `ui.screenshot`, `ui.pick`, `ui.fit`, `ui.highlight` and a frame-exact recording
  (`ui.record {step: true}`), during which the shell
  keeps it off and the player asks with `"exact"`. Before, `"ready"` stayed on once the player had set it, and a
  render or a recorded frame showed whatever the builder had reached.
- **The builder's budget** (2026-11-02, revised after a second review). Its threads take their place from the
  process-wide pool (`craft3d_math::pool`): a worker builds only while it holds one of the pool's `SHARED_THREADS`
  (7) and takes no helpers (`work_alone`), so a regeneration it runs (a pattern's unions, `derive::in_order`) runs
  on that one thread, and the background and the work someone waits for stay within the pool together. The trees
  of freeform booleans are bounded process-wide by bytes (`craft3d_kernel` › `freeform::HELD_TOGETHER`, 4 GiB):
  an operation whose next tree would take the process over it waits until another lets go, unless every other
  operation holding trees waits too; background threads (the builder, the assertions measured behind a command,
  `pool::mark_background`, and the helpers they lease, `Lease::helper_starts`) wait already at half of it. Each
  operation holds at most `HELD_BYTES` (2 GiB), so the trees counted stay below about 6 GiB plus the shelf (768
  MiB). That is an estimate, not a hard ceiling: a tree is counted once it is built (each waiting operation holds
  one more, at most about 22 MB), the memos trees grow later are not counted, a tree asked for outside a boolean
  counts only on the shelf, and the workers' cache budget is estimated from the configuration's bodies. Waiting
  changes when an operation runs, never its answer, and a pattern's side by side unions run as
  before (a count of operations at once, tried first, held them to three). The builder's own memory: at most
  `MAX_QUEUED` (256) configurations queued, each with a copy of its source document and of its source's
  regeneration cache (bodies shared); per worker the regeneration caches of at most 4 sources and 384 MiB
  (`MAX_CACHES`, `CACHE_BYTES`, estimated as the configuration each was last used for), let go of as soon as the
  epoch changes (a reset wakes idle workers); the configurations themselves within the configuration budget once
  taken in. A configuration's build asks between features whether it was let go of
  (`craft3d_regen::progress::with_stop`) and one cut short is never kept. What the builder hands in is collected
  once and kept by kind until taken, and the configurations a provisional placement leaves to the builder
  (`AsmState::deferred`) are set for that placement only (`Sweep::with_deferred`, empty again on an error or a
  panic), so no grid point is solved with configurations skipped.
- **Recordings in real time** (`ui.record` without `step`) record what the window shows as it plays, provisional
  moments and quick pictures included; a frame-exact recording (`step: true`) and pictures (`ui.screenshot`) show
  exact moments at full quality, and so do `ui.pick`, `ui.fit` and `ui.highlight`.
- **A bake in the window never blocks it.** `motion.bake {background: true}` (the window's default,
  `Session::set_bake_in_background`) only hands the bake to the builder and returns within milliseconds (the
  rest grid point solved; the grid and the frames in runs); `building` in `view.motion` says what is left;
  `cancel: true` drops it. The window bakes a motion that builds as it plays by itself, once, at 120 frames
  (`PLAY_FRAMES`, 30 a second over a play at speed 1). A bake in the command (scripts, tests) is as before, its
  batches pinned while they are built (`AsmState::pinned`) so a batch larger than the budget still completes with
  `limit` (before 2026-11-01 each call's pieces were let go of by the next and the bake never finished); a part's
  bake regenerates a batch at a time and stops once the budget is full.
- **Baked frames are placed without solving.** The solve of each frame of a bake (`AsmSweep::bake_ts`) is kept
  once it was placed exactly (`AsmState::solved_of`: the instances and mates as solved, without bodies and without
  holding the configurations, which are found again by key); showing the frame again places its bodies by those
  poses (`AsmState::show_solved`, the same placement), or solves as usual when a configuration it used is gone.
- **Placed copies are reused.** A body placed by the same source and the same pose as in the last placement is
  the same `Arc` (`PlacedCopies`): no copy, no moved mesh, no moved facts; the facts' names are shared between a
  source and its copies (`BodyFacts`' lists are `Arc`s). In the router most poses move a little with every moment,
  so this saves a quarter; the shared names saved more (a moment's quick scene 38 → 15 ms).
- **Slim configurations.** A motion's own state keeps the configurations of instances mated by their datums
  alone without the bodies (and names) that went into later features (`asm::slimmed`): a placement needs only
  what an instance shows, its datums and sketches; a mate on a face or an edge resolves its name through those
  bodies, so an instance mated so keeps its configurations whole (decided per placement; a slim one asked for
  whole is built again). At the top level only (a parent's mate may reach into a subassembly). The belt: 5.6 →
  2.1 MB estimated per configuration. The configurations only grid points used go as soon as their grid point is
  solved, except those a frame of a bake asks for (`AsmSweep::frame_keys`).

## Tests

`crates/engine/tests/motion.rs`: a crank's corner traced against its closed-form circle (every sample within 1e-9,
the polyline's length that of the inscribed polygon, closed, in the xy plane) and a sketch point driven by formulas
of `t`; a four-bar linkage in a sketch driven by `t` on its branch (closed form) over 97 samples and at moments
reached by a jump; a seek and a play of `view.motion` giving bit-identical sketches and meshes, and the same
sequence twice giving bit-identical bodies; a thumbnail saved during a red motion equal to one saved without; the
`after` of a first failure; red over faces painted upstream; a two-sample trace not closed and a tiny bent one not
flat; the export's refusals; a 65-frame replay in chunks against the closed form of its collisions; in
`motion.rs`, collisions found in full however few answers the memo keeps; the trace written to SVG and DXF and the refusals; a moment that rebuilds only the driven feature, leaves the
document as it was and is let go of when the timeline is renamed away; colliding bodies red in the scene's face
colours and in the CPU renderer's picture, the common volume in closed form; clearance over the motion against
`15 + 10 cos 2πt` (its first failure, the samples that fail, its minimum and maximum, every sample); the first
collision of an `overlap` written inline; the replay's frames, captions and collisions per frame; the refusals
without a timeline and with a `t` that is a length; bad parameters. `crates/ui-egui/tests/motion.rs`: a trace drawn
over the screenshot (and gone bit for bit when cleared), collisions red in the window's screenshot, the player over
`t` forward, back and forth and stopped by an edit, and not offered without a timeline; the player's step function
in `shell/player.rs`. `panic_hunt` runs the three commands with adversarial parameters. `crates/engine/tests/motion_meshes.rs`:
12 moments of a four-bar mesh no placed body (the tessellator's count per moment: none, or the two sources
once when the quick picture's tolerance steps to another rung), every body placed with the first moment's source
mesh, its mesh that mesh moved bit for bit. `crates/gpu/tests/parts.rs`: parts uploaded once and moved by poses
against the CPU renderer drawing them moved, near the origin and 1e7 away. `crates/ui-egui/tests/assembly.rs`: the
player plays an assembly forward and back and forth. `crates/engine/tests/motion_timing.rs` (ignored) times a
moment of an assembly, a whole motion through configurations twice (`CRAFT3D_MOTION_ASM`, baked first with
`CRAFT3D_MOTION_BAKE`) and the size of a configuration (`CRAFT3D_CONFIG_PART`). `crates/engine/tests/motion_ready.rs`: a provisional moment shows the stale belt (the one at rest) and settles to
the exact one bit for bit; a play with `shapes: "ready"` settles frame by frame to the exact play; a bake in the
background plays what a bake in the command plays, its frames shown again from their kept solves the same; bad
`shapes` and `background`. `crates/engine/tests/motion_bake.rs` also: a bake in pieces over a budget of one byte
gets done (the batch pinned). `crates/gpu/tests/parts.rs`: parts taken over from an earlier upload draw what a fresh
upload draws, pixel for pixel. The player's rounding to baked frames in `shell/player.rs`. `crates/engine/tests/motion_bake.rs`:
a baked assembly motion whose instance takes a parameter of `t` plays what an unbaked one plays, bit for bit, also
with a budget of one byte; a bake in pieces builds what one bake builds; a baked part shows its moments, `rebuilt`
and collisions as unbaked; the refusals. In `asm.rs`: configurations built one after the other from the last
one's cache equal a regeneration from nothing, feature for feature in their `Debug` (a hole driven by a parameter,
two features rebuilt per configuration; the belt's pattern and unions), and the budget lets the least recently used
go and the caches of sources with nothing kept.

## Findings and history

- 2026-10-29: first version (track C, phase 1): the timeline `t`, `view.motion` with collisions drawn red,
  `view.trace` with its SVG and DXF export, `motion.check` over any assertion, `view.replay {motion}`, the player's
  Motion mode. Waiting for tracks A and B: `t` playing an assembly (its drivers, a solve per moment with branch
  continuity, transforms only), the assembly tree, sliders for free motions, dragging along a joint, the context
  entry for mates, the web build's worker for assemblies, and the three Jigs converted.
- 2026-10-30: review fixes. Thumbnails set the motion aside as they set a scrub aside (a save during a red motion
  wrote red bodies at its `t` into the file, and the revision's cached thumbnail kept them). Collisions from a map
  per state (an overflowing memo dropped collisions). `motion.check` says "at the samples", `sampled: true`, `after`,
  `firstError`, `firstNotProven`. Moments are functions of `t` alone (the fixed-step grid above). The motion replay
  in chunks with a triangle budget and a map for the meshes. Trace: closed needs more than 2 points, flatness
  relative to the size too, the export refuses `tolerance`; the memo's key has the tolerance.
- 2026-10-30: assemblies merged (track A). An assembly shows its placed parts: `shown()` answers with the placed
  state before any motion or scrub, and every motion command refuses an assembly by name (the timeline check says
  that playing an assembly is phase 2), even when the assembly has a variable `t`; the test
  `an_assembly_shows_its_placed_parts_and_refuses_a_motion` holds both.
- 2026-10-30: phase 2 (asm-motion-p2). Assemblies play `t` (*Assemblies* above): a solve per grid point carried by
  `Solver::advance`, dead points with their `t`, traces through instances, `motion.check` at the moment's placement,
  collisions named by instance. Closed forms: a crank-rocker's coupler point against the intersection of two circles
  at 33 samples (1e-6, the branch never changes), seek equal to play bit for bit, the dead point of a non-Grashof
  four-bar at `cos θ = (a² + d² − (b + c)²) / 2ad` within a step (`crates/engine/tests/motion_assembly.rs`).
- 2026-10-30: motion-ux, after the developer watched the Jansen linkage walker played by a script ("a little
  stuttering"; the bar at the top flickered, the status bar too). The player plays an assembly (its Motion mode; an
  assembly has no History, `ui.set {timeline: {motion, playing}}` was refused for "no features to play"); moments
  move meshes instead of meshing (*Costs* above); the work indicator and the status bar are calm
  ([`engine.md`](engine.md) › *A calm work indicator and status bar*). Walker (88 bodies, 24 192 triangles; `motion_timing.rs`,
  release, on the Strix, 61 moments of `view.motion` plus the display scene), before → after: a moment mean 91.5 →
  25.0 ms, median 66.1 → 23.8 ms, longest 1543 → 33 ms; bodies meshed over the 61 moments 10 736 → 19; of a moment,
  `view.motion` itself (the solve and the rigid copies) 23 ms both times, now nearly all of it. The GPU gets 88
  poses of 48 bytes a moment instead of the 24 192 triangles (3.5 MB of vertices). Opening and the first display
  scene 1968 → 44 ms (the walker's 88 bodies are copies of a few sources).
- 2026-10-30 (asm-gaps): `motion.check` held memory for every moment it measured (the Jigs' gearboxes: about 25 MB
  a moment with `MALLOC_ARENA_MAX=2`, 300 MB without; `scripts/asm.py` ran it 17 moments at a time). The cause: the
  checks' memory of answers kept the bodies its keys named alive (an `Arc` per body, to keep the address from being
  reused), and every moment places fresh copies, so up to 256 answers' worth of moments stayed alive. The keys now
  hold the bodies weakly and drop answers whose bodies are gone (*Memory streams* above), which holds for every
  check, not only over a motion. Measured (`crates/engine/tests/motion_check_memory.rs`, two wheels of 258 faces,
  an overlap and a volume, peak resident memory after 9 and after 129 more moments, debug build on the Strix):
  before +60 MiB (35 → 95 MiB, about half a megabyte a moment, growing until 256 answers are kept), after +13 MiB
  (34 → 47 MiB, what the allocator keeps; the test holds it under 24 MiB).
- 2026-11-01 (motion-cache): motion through configurations (*Configurations over a motion* above). The router of
  `demo03-beltdrive` (49 instances, four belts of 940 faces whose `xc` follows `t`), 120 frames of `view.motion`
  plus the display scene, twice, release, on the developer's laptop (`motion_timing.rs`): before, pass 1 56.9 s
  (`view.motion` mean 324 ms), pass 2 35.4 s (151 ms: at most 64 configurations kept, then all dropped). After,
  without a bake: pass 1 56.8 s (286 ms; incremental builds do not help the belt, whose `xc` reaches every feature
  but one tooth), pass 2 23.4 s (`view.motion` mean 20.3 ms, max 32 ms; the rest of a frame is the scene). With
  `motion.bake {frames: 120}` first: the bake 14.5 s (323 configurations built side by side: the belt at the 129
  grid points and the 120 frames; 177 asked for by the frames, 4 of them over the 1 GiB budget), then pass 1 16.7 s
  (`view.motion` mean 20.0 ms, max 262 ms for the four not kept) and pass 2 10.8 s (`view.motion` mean 15.9 ms,
  max 21 ms; with the scene mean 90 ms). A bake before the grid was done in batches held every configuration at
  once (36.8 s, 99 of 341 kept within 512 MB). Resident memory after the bake 3.1 GB, during the play 4.0 GB (the
  scenes and meshes kept besides the configurations). Still owed: the display scene of a moment (75 to 175 ms of a
  frame here: the joined mesh copied together, and the configurations' meshes made on first show, which a bake
  could make too), and answering at once while configurations are built (proposed above).
- 2026-11-01 (motion-play): playback that does not wait for shapes (*Shapes in the background*, *The player*).
  The router (176 bodies, four belts whose `xc` follows `t`), on the developer's laptop (Intel iGPU, release):
  before, the window ran `view.motion` on its own thread and built each moment's configurations there (286 ms
  unbaked, "not responding" during a 17 s bake), and a baked frame cost `view.motion` 16 ms plus 75-175 ms of
  scenes (two per moment, each uploaded whole: the face colours' material table holds brushed directions that
  follow the poses, so parts were never moved, always uploaded). Measured in the window
  (`ui.get` sampled every 20 ms while it plays; the window draws only while the compositor shows it):
  first play, unbaked, 10 moments a second with the window's thread answering within 75 ms at worst (before: 1-3
  a second, 300-700 ms); headless (`view.motion {shapes: "ready"}` + the quick scene at 30 a second, the bake in
  the background): `view.motion` median 58-64 ms while the builder works, 44 ms once baked (35 ms on an idle
  machine, a baked frame from its kept solve less), a moment's quick scene 8-15 ms (was 38-51), a display scene
  10-20 ms (was 20-23; the shared facts' names). The background bake of 120 frames takes about 50 s with the
  builder's 3 threads (no extra threads from the pool, so the window's solve keeps them: without that the window's
  `view.motion` took 100-250 ms). Memory (resident, headless play as the window plays): before 3.0 GB after a bake
  and 3.9 GB while playing; after, 1.0 GB after the first pass, 2.1 GB after four (live: 1.4 GB, of which slim
  configurations about 0.4 GB for the frames, quick-picture meshes 0.35 GB, scenes 0.1 GB, the rest the open
  router and the allocator; a synchronous bake: 0.77 GB live after it, was 1.33 GB). Still owed: a moment of the
  window still takes three passes of its thread (the moment, the quick scene's upload, its edges as lines: 40-65
  ms together), so 30 a second needs the edges kept per part and moved like the parts (`segments_marked` rebuilds
  all 176 bodies' edges each moment) and the moment's solve off the window's thread; resident memory is twice
  the live (glibc keeps what the builds freed); `built_bytes` counts bodies only (regeneration caches, meshes per
  configuration and the open assembly's own `AsmState` beside the sweep's are not in the budget).
