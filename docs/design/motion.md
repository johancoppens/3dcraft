# Motion over the timeline `t`

How a design is moved and looked at while it moves: the timeline `t`, the state shown at a moment of it, the trace
of a point, collisions drawn red, checks over the motion and the video of it. The code is
`crates/engine/src/motion.rs` (the state and the commands), `crates/engine/src/replay_cmds.rs` (the video),
`crates/ui-egui/src/shell/player.rs` and `shell/timeline.rs` (the player) and the overlay in
`crates/ui-egui/src/viewport.rs` and `direct.rs` (the traces). Assemblies, which this is for in the end, are
`assemblies.md` (track A, on its own branch until it merges); this document is track C's first part, which works on any part.

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

A state is made by a `Sweep`: the document (or the scrubbed state) copied once, with a copy of the document's
regeneration cache; each moment sets `t` on that copy, settles its sketches from where the previous moment left
them (`settle_sketches_cached`, as an edit does), and regenerates it with the sweep's cache. Hence:

- **Only what depends on `t` is rebuilt.** A feature none of whose inputs changed is reused from the cache, its body
  the same `Arc`, so the session's store of meshes does not mesh it again. `view.motion` reports `rebuilt`: the
  features whose bodies differ from the document's. For a part this is honest work per frame: a feature driven by
  `t` is regenerated at every moment. Assemblies place their instances by transforms only; when they play `t` (phase
  2, after tracks A and B), a moment there costs a solve and transforms, no regeneration.
- **Branch continuity.** Each moment starts from the one shown before it, sketches included, so a linkage solved in
  a sketch stays on its branch as `t` moves in small steps. A jump (the slider dragged far) starts from wherever the
  previous moment was; the assembly solver of track B adds the step cutting that keeps a branch over any jump.
- An edit keeps the motion: the next `shown` sees another base and makes the state again from the edited document
  (a fresh sweep). An edit that takes the timeline away (renamed, given a unit) lets go of the motion.

### Collisions drawn red

With `collisions`, each state is checked for overlapping bodies: pairs of the visible bodies whose boxes (grown by
twice the tolerance) meet go to the kernel's interference query (`kernel::boolean::interference`, as
`document.interference`), on the shared thread pool, merged in pair order. An answer is kept by the two bodies'
addresses (the bodies held, so an address is not reused), so a pair `t` does not move is asked once over the whole
motion. A pair the kernel refuses is reported `unknown` with the reason, never clear and never red.

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
that is not flat, or is a point or a straight line, is refused: it has no flat drawing.

### Checks over the motion (`motion.check`)

Any assertion is checked over the motion, so interference and clearance over the motion are not measures of their
own: `overlap` with `is: false` gives the first `t` of a collision, `clearance` with a `min` the least clearance and
its `t`, and the same holds for any measure the document can assert (assemblies' measures by instance included,
once they move). A copy of the document in a session of its own is set to each `t` (one undo step merged over the
run, so the copy's history does not grow), and its assertions are measured as `document.check` measures them; the
answers are kept by what they depend on, so a measure whose subjects `t` does not move is measured once. Per
assertion: the worst status over the samples, the first failure, and the least and greatest value with their `t`.
`measure` is an assertion written inline and checked only there. Sampling is what it is: a collision shorter than
the spacing of the samples can be missed, and the report says the samples it took.

### Video (`view.replay {motion}`)

`view.replay` with `motion` renders a frame per sample of `t` instead of a step per feature, each state made by one
`Sweep` in order (with `collisions`, red as above), meshed once per distinct body, framed on the box around every
state so the camera does not jump, captioned `t = …`, written as PNG frames that `craft3d-cli replay` turns into an
MP4 or a GIF, as for the history.

### The player

The history player gets a mode: History (the steps) or Motion (`t`). Playing the motion asks for the next moment
once the scene of the one asked for is there (no `view.motion` is waiting), by the time since the last
(`MOTION_SECONDS` = 4 s from 0 to 1 at speed 1, a frame's time capped at a quarter of a second), so a heavy model
plays slower rather than skipping its states. Loop starts again at 0; back and forth turns at 1 (and at 0 again with
loop); without either it stops at 1. A slider over `t`, back and forth and collisions sit above the player's
buttons; the caption over the viewport says `t = 0.250 · collision: Wall and Crank`. All of it is view state in
`UiState::timeline` (`motion`, `bounce`, `collisions`), so `ui.set` plays the motion for whoever watches. It is
offered only for a document with a timeline.

## Tests

`crates/engine/tests/motion.rs`: a crank's corner traced against its closed-form circle (every sample within 1e-9,
the polyline's length that of the inscribed polygon, closed, in the xy plane) and a sketch point driven by formulas
of `t`; the trace written to SVG and DXF and the refusals; a moment that rebuilds only the driven feature, leaves the
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
