# 3DCraft

**A parametric 3D CAD modeller built for AI.** Describe a part, and an AI agent builds it in the 3DCraft
window while you watch, point at faces and check the result. Or open a ready-made **Jig**, set its sizes
in a form and export it, no AI needed. Or write the commands yourself.

![The 3DCraft window: four documents open, the feature list and history player of a socket head cap screw with a cut thread, the view cube and the display bar](docs/img/viewer.png)

Every action in 3DCraft is a command with a name and a few values, so the same engine is driven from the
window, the command line, a script, a JSON control channel or an MCP server. It is written in pure Rust,
with its own B-rep kernel, constraint solver, tessellator and renderers: no OpenCascade, no Parasolid, no
third-party geometry of any kind. Licence: [MIT](LICENSE).

## Get started

- **[Install 3DCraft](docs/guide/01-install.md)**: installers for Windows, macOS and Linux (version
  0.1.0), or the web version with nothing to install.
- **[Your first five minutes](docs/guide/02-first-steps.md)**: open an example, play its history, try a
  Jig, export for a 3D printer.
- **[The user guide](docs/guide/README.md)**: working with an AI agent, the window, Jigs, formulas,
  modelling, measuring, files and a reference of every command.
- **[Example models](docs/examples/README.md)** that check their own volume, and the
  **[Jigs collection](https://github.com/johancoppens/3dcraft-jigs)**: a laser-cut box, a welding table
  and a GT2 pulley.
- **What changed:** [`CHANGELOG.md`](CHANGELOG.md).

## What it does

- **Works with your AI agent.** A built-in MCP server lets Claude or any other MCP client model in the
  window you are watching; the agent reads what you selected, and can light up faces, fly the camera and
  play the history for you.
- **Parametric and smart.** Variables and formulas drive everything, with conditions, tables and parts
  switched on and off; sketches under sixteen kinds of constraint, solved by 3DCraft's own solver; curves
  drawn from formulas (an involute gear is exact). Change one number and the model rebuilds, only from
  the feature the change touches.
- **Features.** Extrude, revolve, loft, sweep along a path or a helix, booleans, mirror and patterns,
  chamfers and fillets, shell, draft, offset, move and delete face, thicken, ISO metric threads, datums
  and sheets.
- **Stable references.** Every face, edge and vertex has a persistent name made of where it came from, so
  a fillet stays on its edge through upstream edits; a feature that loses its edge fails, names what it
  lost and is repaired, never silently moved.
- **Checked, not hoped.** Volumes, distances, wall thickness and clearances from the exact geometry, and
  design rules kept in the document that are checked at every change. The boolean kernel agrees with a
  reference kernel on a corpus of about twelve thousand cases with none wrong; what it cannot do yet it
  refuses by name rather than answering badly.
- **Jigs.** Publish a design's sizes as parameters and share it as a `.3dcraftjig`: anyone sets them in a
  form, on the desktop or in the browser, and `check-range` proves the design over its whole range.
- **Files.** `.3dcraft` (the history: it rebuilds bit for bit, and every released version's files keep
  opening); STEP import and export; STL, 3MF and OBJ for printing; DXF and SVG of outlines, whole cut
  sheets for a laser, and drawings of the model with hidden lines.
- **A native window and a web version.** egui on wgpu (Vulkan, Metal, DX12 or OpenGL) on the desktop,
  WebGPU or WebGL2 in the browser; a start screen, tabs, a view cube, section and see-through, a history
  player, and a Parameters tab for Jigs.
- **Never crashes.** A bad file, a bad parameter or a degenerate model gives an error you can act on, and
  an autosave brings your work back after a crash.

## Build from source

```sh
cargo build --release -p craft3d -p craft3d-cli
target/release/craft3d-cli run --new '{"units":"mm"}' \
    --cmd variable.set  --params '{"name":"width","expr":"60 mm"}' \
    --cmd feature.block --params '{"size":["width","width * 2 / 3","8 mm"]}' \
    --out part.3dcraft
target/release/craft3d-cli render part.3dcraft --out part.png
target/release/craft3d part.3dcraft
```

The window, and an agent driving it over MCP:

```sh
target/release/craft3d --control 7878 --control-token-file .private/control.token &
claude mcp add craft3d -- "$PWD/target/release/craft3d-cli" mcp \
    --bridge 7878 --control-token-file "$PWD/.private/control.token"
```

Contributors and agents working on 3DCraft: read [`AGENTS.md`](AGENTS.md) and [`docs/`](docs/README.md).

## One variable, three screws

The [parametric screw](docs/showcase/pscrew.steps.json) is fifteen commands. Its head, hex socket, rounds,
chamfer and thread all follow the nominal diameter `d`; the thread takes the ISO 261 size and coarse pitch of
whatever diameter the shank has. Setting `d` to 8, 10 and 12 mm gives an M8 × 1.25, an M10 × 1.5 and an
M12 × 1.75, each a valid solid regenerated in about three seconds, thread cut and all:

```sh
target/release/craft3d-cli run --new '{"units":"mm"}' --script docs/showcase/pscrew.steps.json \
    --cmd variable.set --params '{"name":"d","expr":"12 mm"}' --out m12.3dcraft
```

![Three socket head cap screws of the same model, M8, M10 and M12, at the same scale](docs/img/screw-sizes.png)

## Showcase

Twenty-four models built with the commands that exist today; the first eight were driven step by step into
the window while the developer watched, the others were built headless or by a script. The pictures are the CPU renderer's
(`craft3d-cli render`). The `.3dcraft` files and the step scripts are in
[`docs/showcase/`](docs/showcase/); any of them is rebuilt headless with

```sh
target/release/craft3d-cli run --new '{"units":"mm"}' --script docs/showcase/gear.steps.json --out gear.3dcraft
```

| | |
|---|---|
| ![Spur gear](docs/showcase/gear.png) **Spur gear**, module 2, 24 teeth: the involute flanks as one closed polyline of 336 points, extruded; hub, bore and keyway by booleans. | ![Gear pair](docs/showcase/gearpair.png) **Gear pair** 24:12 in mesh: the pinion turned by half a tooth, its shaft joined; `document.check` with `deep` finds no interference. |
| ![Timing belt drive](docs/showcase/belt.png) **Timing-belt drive** 20:40: toothed pulleys with flanges and bores; the belt is one sketch with an outer and an inner loop (arcs around the pulleys, tangent runs between them), extruded. | ![Turbine rotor](docs/showcase/turbine.png) **Turbine rotor**: hub by revolve, a cambered blade section extruded radially, its tip cut round by an intersect, seven of them by a circular pattern, joined to the hub; a shroud ring. |
| ![Wind turbine](docs/showcase/windmill.png) **Wind turbine**: tubular tower by revolve, foundation, nacelle, spinner cone, three blades pitched by `feature.transform` and patterned about the rotor axis, everything one body. | ![Boat hull](docs/showcase/hull.png) **Boat hull** from three profiles (side, plan, section) extruded and intersected, hollowed by the same three a wall thinner, thwarts across. Faceted: it was built before sketches had splines; a loft would make it fair. |
| ![Impeller](docs/showcase/impeller.png) **Impeller**: a blade between two splines, extruded into B-spline walls, nine of them by a circular pattern, joined with a disc and a hub and bored: one body of 33 faces. | ![Pillar under a roller](docs/showcase/pillar.png) **Pillar**: a leaf of a spline and a line extruded up to the wall of a roller lying across it (`upTo`), so that its top follows the roller; seen from behind. |
| ![Bearing housing](docs/showcase/housing.png) **Bearing housing**: half a base with a spline for its ends, mirrored in a datum plane and joined; a pedestal between two splines extruded up to the sleeve's wall; bored; two oil holes drilled from a datum plane turned 30° about the sleeve's axis (a datum axis); three bolt holes by a pattern, mirrored in the same datum plane. FreeCAD reads its STEP export as one closed solid of 20 faces. | ![Lofted hull](docs/showcase/lofthull.png) **Lofted hull**: four stations along the keel, each a spline under a deck line, joined by `feature.loft` into one smooth skin; hollowed by a second loft through the same stations a wall's thickness smaller (`cut`). Seven faces. |
| ![Twisted blade](docs/showcase/blade.png) **Twisted blade**: an aerofoil of two splines at four heights, shorter and turned further towards the tip (48° in all), lofted and joined to a round foot. | ![Vase](docs/showcase/vase.png) **Vase**: one sketch, one feature. Its section is two splines (the outside and the inside of the wall) closed by a foot, a rim and the axis, turned a full turn: the walls are rational B-spline surfaces of revolution, exact, in two halves each. |
| ![Bent pipe with a flange](docs/showcase/pipe.png) **Bent pipe with a flange**: a ring (the wall and the bore) carried along a line, a quarter bend and a line by `feature.sweep`: two cylinders and a piece of a torus inside and out, exact and without a seam; a flange extruded and joined. | ![Handle on a plate](docs/showcase/handle.png) **Handle on a plate**: a circle swept along a path of three lines and two bends drawn in a sketch that stands on the plate, joined to the plate at once (`join`). |
| ![Gasket](docs/showcase/gasket.png) **Gasket**: an obround section (two lines, two half circles) carried round a closed path, a rounded rectangle: a ring without ends, its flat faces one face each all the way round. | ![Hose](docs/showcase/hose.png) **Hose**: a ring swept along a spline. Along a spline the faces are B-spline surfaces within half the document's tolerance of the true sweep; the feature says how far off they were found to be (1.4e-7 mm here, through 85 stations). |
| ![Tray, cut open](docs/showcase/tray.png) **Shelled tray, cut open**: a block with its upright edges rounded, made hollow by `feature.shell` (3 mm walls, open at the top; the inner rounds have the radius less the wall), a bored boss sketched on the floor of the hollow, then made taller and its bore wider by `feature.offsetFace`. Its volume is the closed form to the last digit. Rendered with the section plane. | ![Coil spring](docs/showcase/spring.png) **Coil spring**: a wire of 1.5 mm on a plane square to a helix (`datum.helix`, six turns of pitch 5), carried along it by its screw motion (`feature.sweep` with the helix as its path): one face per quarter of the wire per half turn, B-spline surfaces 1.1e-9 mm from the true sweep through 217 stations; its volume is the wire's area times the helix's length to 4e-13. |
| ![Socket head cap screw](docs/showcase/screw.png) **Socket head cap screw M10 × 40**: head and shank, a hex socket with its drill point, rounds and chamfers, and `feature.thread`: an ISO metric thread of 26 mm cut through the lead chamfer by the groove of the basic profile swept along a helix, with a run-out of a quarter turn. | ![Parametric screw](docs/showcase/pscrew.png) **Parametric screw**: the same screw driven by one variable `d`: head `1.5 d`, socket `0.8 d` (three slabs turned 60° and intersected), rounds and chamfer given by the faces of the first body and followed through every boolean, and a thread that takes the ISO size and coarse pitch of whatever diameter the shank has. |
| ![Aircraft](docs/showcase/aircraft.png) **Aircraft**: a fuselage turned from a spline, wings lofted through four cambered sections (tapered, swept back, rising towards the tip and twisted 3.5° there), mirrored; a tailplane and a fin; five unions of freeform faces into one solid. | ![Voronoi helmet](docs/showcase/helmet.png) **Voronoi helmet**: a 4 mm shell turned from two splines, 70 Voronoi cells (computed on the sphere) cut through it as radial frustums: one solid. Not the organic look of a lattice tool: its struts are flat and its rims sharp. |
| ![Modernist villa](docs/showcase/villa.png) **Modernist villa**, in metres: slabs, glazing, columns, a cantilevered floor and two upper wings at an angle under flared roofs (ruled lofts), a pool cut into the terrace, curved loungers (a closed spline extruded), filleted sofas, turned planters and a fire bowl: 49 bodies. | ![Wind turbine rotor](docs/showcase/rotor.png) **Wind turbine rotor**, in metres (tolerance 0.1 mm: `--new '{"units":"m","tolerance":1e-4}'`): each 22 m blade one smooth loft through eight cambered sections, from a thick round root to a slender tip (chord 3 m to 0.55 m, thickness 62 % to 12 %, twist 20° to 0°), three by a circular pattern, united with a revolved spinner. |

## First field test: an agent that only knew the command list

A separate AI session got nothing but the MCP connection: no source, no docs, only what `command_list`
and the error messages say. It built these, and checked every volume against a closed form or its own
computation:

![A plate with rounded corners and four holes, a hollow post with two ribs and a hexagonal head](docs/field-test/2026-10-12/img/04_plate_post_hex_head.png)

A constrained plate sketch with four holes, a hollow post on its top face, ribs sketched on a side face
and a hexagonal head: four bodies, each to the predicted volume, following the variables when the plate
is made longer, and bit-identical after save and reopen.

![Two spur gears, 30 and 50 teeth, in mesh](docs/field-test/2026-10-12/img/09_gear_pair_iso.png)

Two spur gears (module 2, 30 and 50 teeth, bore with keyway) in mesh at centre distance 80. The
50-tooth gear was built over MCP alone in 220 commands; its volume equals the script's own area
integral to 12 digits, and the two outlines clear each other by 0.007 mm on the line of action.

Nothing crashed and no command gave a wrong result silently. The same report lists what got in the way:
no patterns (a gear is 700 literal points), no booleans, a cap face that is renamed when its outline
changes, fixed perspective views that cannot zoom, and heavy payloads for bulk geometry over MCP.
[The full findings](docs/field-test/2026-10-12/README.md), with the generator scripts.

The tessellator handles any face the kernel can describe. These are the reference solids of the test
suite, each checked against its closed-form volume and read back by FreeCAD as a closed solid:

![Sixteen reference solids: primitives, solids with holes, trimmed spheres and cylinders, freeform roofs](docs/img/solids.png)

The kernel also evaluates freeform surfaces; here is the Utah teapot (32 Bézier patches),
tessellated and drawn by the same renderer:

![The Utah teapot, tessellated and rendered by 3DCraft's CPU reference renderer](docs/img/teapot.png)

Agents and contributors: read [`AGENTS.md`](AGENTS.md) first. Licence: [MIT](LICENSE).
