# Motion over the timeline `t`

How a design is moved and looked at while it moves: the timeline `t`, the state shown at a moment of it, the trace
of a point, collisions drawn red, checks over the motion and the video of it. The code is
`crates/engine/src/motion.rs` (the state and the commands), `crates/engine/src/replay_cmds.rs` (the video),
`crates/ui-egui/src/shell/player.rs` and `shell/timeline.rs` (the player) and the overlay in
`crates/ui-egui/src/viewport.rs` and `direct.rs` (the traces). Assemblies, which this is for in the end, are
`assemblies.md` (track A); this document is track C's first part, which works on any part.

## Method

### The timeline

The **timeline** of a document is its variable `t` when that evaluates to a plain number (no unit). Formulas of `t`
are the drivers of the motion (a crank angle `360 deg * t`, per leg `crank + 90 deg`); `t` runs from 0 to 1. Nothing
is reserved: a document whose `t` is a length (a board's or a bar's thickness, as several Jigs have it, the
strandbeest leg among them) has no timeline, and every motion command says so with the way out (rename it with
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

The window draws a trace as a polyline over the picture, tested against the depth buffer like a sketch (dimmed
behind a body), in the theme's `trace` colour, in both overlay paths (`viewport.rs` for the CPU raster and the
screenshot, `direct.rs` for the GPU drawing of the web build). `file.export {trace}` writes it to SVG or DXF in its
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
in `shell/player.rs`. `panic_hunt` runs the three commands with adversarial parameters.

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
