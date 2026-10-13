# 3DCraft

A parametric 3D CAD modeller for AI, in pure Rust, with its own kernel, constraint solver, tessellator and renderers.
Every action is a command, so the same engine can be driven from a UI, the CLI, a JSON control
channel or an MCP server.

![A part modelled with six commands: a plate, a post with a knob, and a peg](docs/img/part.png)

**Status: the kernel is being made professional.** A part is modelled from primitive solids (block, cylinder,
sphere, cone, torus) and from sketches: lines, polylines, circles, arcs and paths of lines and arcs under sixteen kinds of
constraint, solved by 3DCraft's own solver, extruded or revolved into solids, which are moved, mirrored,
patterned and combined by booleans (union, subtract, intersect), with variables and formulas throughout.
Documents are saved as `.3dcraft`, reopened and changed (only what an edit touches is regenerated), rendered
on the CPU or the GPU, and exported as STEP, STL, 3MF or OBJ, all through commands: from the command line,
a JSON-lines server or an MCP server. A native window shows the model while an agent builds it, also over
MCP. Freeform (Bézier and B-spline) faces take part in booleans and are measured against FreeCAD on a corpus
of 3996 cases. There are no fillets, no lofts and no splines in sketches yet; [`docs/roadmap.md`](docs/roadmap.md)
says what comes next.

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

Six models built with the commands that exist today, each driven step by step into the window while the
developer watched; the pictures are the CPU renderer's. The `.3dcraft` files and the step scripts are in
[`docs/showcase/`](docs/showcase/); any of them is rebuilt headless with

```sh
target/release/craft3d-cli run --new '{"units":"mm"}' --script docs/showcase/gear.steps.json --out gear.3dcraft
```

| | |
|---|---|
| ![Spur gear](docs/showcase/gear.png) **Spur gear**, module 2, 24 teeth: the involute flanks as one closed polyline of 336 points, extruded; hub, bore and keyway by booleans. | ![Gear pair](docs/showcase/gearpair.png) **Gear pair** 24:12 in mesh: the pinion turned by half a tooth, its shaft joined; `document.check` with `deep` finds no interference. |
| ![Timing belt drive](docs/showcase/belt.png) **Timing-belt drive** 20:40: toothed pulleys with flanges and bores; the belt is one sketch with an outer and an inner loop (arcs around the pulleys, tangent runs between them), extruded. | ![Turbine rotor](docs/showcase/turbine.png) **Turbine rotor**: hub by revolve, a cambered blade section extruded radially, its tip cut round by an intersect, seven of them by a circular pattern, joined to the hub; a shroud ring. |
| ![Wind turbine](docs/showcase/windmill.png) **Wind turbine**: tubular tower by revolve, foundation, nacelle, spinner cone, three blades pitched by `feature.transform` and patterned about the rotor axis, everything one body. | ![Boat hull](docs/showcase/hull.png) **Boat hull** from three profiles (side, plan, section) extruded and intersected, hollowed by the same three a wall thinner, thwarts across. Faceted, since sketches have no splines yet; a loft would make it fair. |

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
