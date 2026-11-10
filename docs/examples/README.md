# The guide's lessons

Seven parts to learn from, one technique each: an L-bracket, a bolt flange, a shelled case, a flanged
bushing, a coil spring, a hopper and an involute gear. Each is a command script (a JSON list of
`[command, params]` steps) that builds a `.3dcraft` document, driven by variables and ending with checks, at
least its volume worked out by hand, so it proves itself: `crates/engine/tests/guide_examples.rs` runs them
all at every change. The pictures are drawn by `craft3d-cli render` (`cargo xtask example-thumbs`). See the
[user guide](../guide/README.md).

**In the window:** the lessons are not on the start screen. It opens finished examples, Jigs and
assemblies instead, built into the program from their own repositories ([`published/`](published/), with
their licences in [`published/LICENSE.md`](published/LICENSE.md)). Build a lesson from a terminal, as
below, and open the file it writes; or let an agent run its steps in the window.

**From a terminal:** build one, draw it, change it:

```sh
craft3d-cli run --new '{"units":"mm"}' --script docs/examples/bracket.json --out bracket.3dcraft
craft3d-cli render bracket.3dcraft --out bracket.png
craft3d-cli run bracket.3dcraft --cmd variable.set --params '{"name":"leg_x","expr":"60 mm"}' \
    --cmd assert.list --out longer.3dcraft
```

An agent runs the same steps through `command_batch` (each `[id, params]` becomes `{"id": …, "params": …}`),
or opens a saved file.

| | |
|---|---|
| ![L-bracket](bracket.png) | **[L-bracket](bracket.json)**: an L profile on the `zx` plane with every corner written in the variables, extruded; a round in the inside corner (`feature.fillet` between two sides of the extrusion); a hole in each leg by a circle extruded `throughAll` with `cut`. Asserts its volume, two holes, and that they go through. The part of the guide's [A first part, command by command](../guide/first-part.md). |
| ![Bolt flange](flange.png) | **[Bolt flange](flange.json)**: a disc, a bore and a bolt hole as cylinders; the hole repeated `n` times round the axis by `feature.pattern`, subtracted with `feature.boolean`; a chamfer on the rim named as the first cylinder had it. A named set `bolt holes` (`set.define`) holds the holes of that size, and the assertions count them against `n` and check that they go through. |
| ![Shelled case](case.png) | **[Shelled case](case.json)**: a block with its four upright edges rounded (`feature.fillet`), hollowed by `feature.shell` with 2 mm walls and the top open; the inner corners get the radius less the wall. Asserts the volume (a closed form in all five variables) and a least wall thickness (the measure `thickness`). |
| ![Flanged bushing](bushing.png) | **[Flanged bushing](bushing.json)**: a stepped section turned a full turn about the sketch's x axis (`feature.revolve`), with a bore; an M12 thread on the shank by `feature.thread` (cosmetic: the size is taken from the shank's diameter, the body is unchanged). Asserts the volume by Pappus's rule and that the bore goes through. |
| ![Coil spring](spring.png) | **[Coil spring](spring.json)**: a helix of six turns (`datum.helix`) and a circle on the `zx` plane through its start, carried along it by its screw motion (`feature.sweep` with the helix as the path). Asserts the volume: the wire's area times the circumference of the coil, per turn. |
| ![Hopper](hopper.png) | **[Hopper](hopper.json)**: two squares, one on `xy` and one on an offset plane (`sketch.rect`, `sketch.create` with `offset`), joined by a ruled `feature.loft`. Asserts the volume of the frustum, h/3 (A₁ + A₂ + √(A₁A₂)). |
| ![Involute gear](gear.png) | **[Involute gear](gear.json)**: one tooth whose flanks are curves from formulas (`sketch.curve`: the involute `x = rb (cos(g + t) + t sin(g + t))`, its mirror for the other flank), closed by the tip arc and an arc inside the root circle; extruded, repeated `z` times round the axis (`feature.pattern`) and joined to the root disc. The flanks are fitted again whenever `module`, `z` or `alpha` change. Asserts the volume in closed form (the area an involute sweeps from the centre is `rb² t³ / 6`) and that it is one body. See [chapter 8](../guide/formulas.md#curves-from-formulas). |
