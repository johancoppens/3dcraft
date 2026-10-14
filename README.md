# 3DCraft

A parametric 3D CAD modeller for AI, in pure Rust, with its own kernel, constraint solver, tessellator and renderers.
Every action is a command, so the same engine can be driven from a UI, the CLI, a JSON control
channel or an MCP server.

![A part modelled with six commands: a plate, a post with a knob, and a peg](docs/img/part.png)

**Status: the kernel is being made professional.** A part is modelled from primitive solids (block, cylinder,
sphere, cone, torus) and from sketches: lines, polylines, circles, arcs, paths of lines and arcs and B-splines
under sixteen kinds of constraint, solved by 3DCraft's own solver, extruded (to a distance, through all, or up
to a face, flat or curved) or revolved into solids, which are moved, mirrored, patterned and combined by
booleans (union, subtract, intersect), with variables and formulas throughout. Datum planes, axes and points
place sketches, mirrors, revolves and patterns where no face is. Documents are saved as `.3dcraft`, reopened
and changed (only what an edit touches is regenerated), rendered on the CPU or the GPU, and exported as STEP,
STL, 3MF or OBJ, and a sketch or a flat face as DXF or SVG, all through commands: from the command line, a
JSON-lines server or an MCP server. A native window shows the model while an agent builds it, also over MCP.
Freeform (Bézier and B-spline) faces take part in booleans and are measured against FreeCAD on a corpus of
3996 cases, and FreeCAD reads back what the commands build as valid solids of our volume. There are no
fillets, no lofts and no sweeps yet; [`docs/roadmap.md`](docs/roadmap.md) says what comes next.

```sh
cargo build --release -p craft3d-cli
target/release/craft3d-cli run --new '{}' \
    --cmd variable.set  --params '{"name":"width","expr":"60 mm"}' \
    --cmd feature.block --params '{"size":["width","width * 2 / 3","8 mm"]}' \
    --cmd document.measure --out part.3dcraft
target/release/craft3d-cli render part.3dcraft --out part.png
target/release/craft3d-cli convert part.3dcraft part.3mf
target/release/craft3d-cli commands          # everything it can do
```

The viewer, after an agent built this bracket in it through the MCP bridge (36 commands):

![The 3DCraft window showing an L bracket with a tab, its sketches drawn over it](docs/img/viewer.png)

```sh
cargo build --release -p craft3d -p craft3d-cli
target/release/craft3d --control 7878 --control-token-file .private/control.token &
claude mcp add craft3d -- "$PWD/target/release/craft3d-cli" mcp \
    --bridge 7878 --control-token-file "$PWD/.private/control.token"
```

## Showcase

Sixteen models built with the commands that exist today; the first eight were driven step by step into the
window while the developer watched, the others were built headless. The pictures are the CPU renderer's
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
| ![Socket head cap screw](docs/showcase/screw.png) **Socket head cap screw M10 × 40**: head and shank, a hex socket with its drill point, rounds and chamfers, and `feature.thread`: an ISO metric thread of 26 mm cut through the lead chamfer by the groove of the basic profile swept along a helix, with a run-out of a quarter turn. | |

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
