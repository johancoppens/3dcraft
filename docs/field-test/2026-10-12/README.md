# Field test: an agent models with 3DCraft over MCP (2026-10-12)

> Written by the agent that did the test, in a session separate from the one that develops 3DCraft. Copied here unchanged apart from this note. The models it mentions were written to `target/work`, which is not part of the repository; the pictures (`img/`) and generator scripts (`scripts/`) are next to this file. The findings are tracked in [`roadmap.md`](../../roadmap.md).

An agent session that only *used* 3DCraft as a modeller, without touching the
source. It drove the running desktop app (`craft3d --control 7878`, read and
write root `target/work`) through the MCP bridge (`craft3d-cli mcp --bridge`),
and for two models through `craft3d-cli run --script`. The only knowledge of
the app came from `command_list` (36 commands) and from the error messages.

Nothing crashed and no command produced a wrong result silently. Every model
below passed `document.check`, and every volume matched a closed-form or
independently computed value.

## 1. What was built

| Model | Channel | Checks |
|---|---|---|
| Plate 60 x 40 x 8 with a post (block + cylinder) | MCP | volumes 19200 and pi*6^2*30 |
| Same, sketch driven: fully constrained plate sketch with 4 holes, hollow post on the plate's end cap | MCP | 18571.68 and 2544.69 mm^3, both exact; `plate_l` 60 -> 80 regenerates, holes and post follow |
| + rounded corners (4 lines + 4 arcs, tangent and radius constraints), 2 ribs sketched on a side face with an offset | MCP | plate 18400.00, ribs 1280.00 mm^3; `corner_r` and `plate_l` edits regenerate correctly |
| + hexagonal head on the post's end cap (points on a construction circle, 5 `equal`, 1 `horizontal`) | MCP | 1389.20 mm^3 = (3*sqrt(3)/2*10^2 - pi*3^2)*6 |
| Save, reopen, export of the above | MCP | reopened file gives bit-identical volume, area and centroid for all 4 bodies; 3MF and STL closed, 472 triangles |
| Spur gear m=2, z=30, bore 15 with keyway 5 x 2.3 | CLI script, then `file.open` over MCP | 51973.00895 mm^3, equal to the script's own area integral |
| Spur gear m=2, z=50, same bore | MCP only, 220 commands | 152256.53233 mm^3, equal to 12 digits |
| Both gears meshing at centre distance 80 | CLI script, then MCP | outlines checked outside the app: 0 crossings, smallest gap 0.007 mm on the line of action |

Files are in `target/work` (`plate_post.*`, `gear_m2_z30.*`, `gear_m2_z50.*`,
`gear_pair_*`). The generator scripts are in `scripts/` next to this file.

## 2. What worked well

- **Self-describing commands.** The `params` notation and summaries in
  `command_list` were enough to model without reading code or docs.
- **Actionable errors.** Mid-edit states reported exactly what was wrong:
  - `sketch PlateSketch: the profile is not closed: an end of e19, e22, e25, e28 meets nothing`
  - `plane: Plate no longer has a face F4:endCap(e5,e6,e7,e8); its faces are: F4:side(e19), ...`
  - downstream features: `... failed; fix it first (sketch.inspect)`
  The list of existing face names in the second message is what made the
  repair a single `sketch.edit`.
- **No silent retargeting.** When a face name disappeared, the dependent
  sketch failed instead of moving to another face.
- **Solver bookkeeping.** Every constraint removed exactly the expected number
  of degrees of freedom, including tangent between a line and an arc that
  share an end point (a configuration that is degenerate in naive
  formulations). Dimensions with `direction: horizontal|vertical` stayed on
  the intended side when `plate_l` changed.
- **Robust with big, under-constrained sketches.** A sketch with 1409 entities
  and 1300 remaining degrees of freedom solved, extruded and round-tripped
  without trouble. `craft3d-cli run` built the 30-tooth gear (139 commands,
  a body of 486 faces) in 0.2 s.
- **Predictable entity ids.** `sketch.polyline` with n points creates n points
  then n-1 lines (n lines when closed); `sketch.arc`/`sketch.circle` on an
  existing centre creates one entity. Ids could be predicted for a whole
  script and always matched.
- **Round trip and export.** Save/reopen was bit-identical; all meshes closed.

## 3. Findings worth a look

1. **End-cap face names are fragile under outline edits.** The name lists
   every edge of the outer loop (`F4:endCap(e5,e6,e7,e8)`), so rounding the
   corners of the plate renamed the cap to
   `F4:endCap(e19,e22,e25,e28,e30,e32,e34,e36)` and broke the sketch on it,
   although "the top of the plate" is conceptually the same face. Correct
   under the no-silent-retarget rule, but any outline change breaks everything
   built on a cap. (Inner loops are not part of the name: adding holes did
   not rename it.)
2. **The frame of a sketch on a side face moves with upstream dimensions.**
   Its origin is the start point of the generating line. On `F4:side(e19)`
   the origin was world x = 5 = `corner_r`, so the rib dimensions had to
   subtract `corner_r` to stay put when the corner radius changes.
3. **A sketch cannot reference geometry of other features.** The post, the
   ribs and the head each had to be re-dimensioned from a fixed origin point
   with variable expressions (`plate_l / 2`, ...). Projected edges/vertices
   or a "coincident with body vertex" constraint would remove this.
4. **Two disjoint profiles in one extrude give one body with two lumps.**
   `Ribs` is a single valid solid made of two separate pieces. Fine, but
   worth deciding whether that is intended (one body per profile, or a
   documented multi-lump body).
5. **No way to list or switch open documents.** `file.new` and `file.open`
   add a document and make it active; `file.close` closes the active one and
   reports `{"open": n}`. The registry has no command to list the open
   documents or to activate another one, so earlier documents (five were
   open at the end, one of them unsaved) can only be reached by closing the
   ones on top.
6. **`view.render` / `doc_render_preview`: the fixed views are perspective and
   cannot zoom.** In `top` view the side walls of off-centre bodies are
   visible, and checking a detail (gear mesh zone) needed a 2048 px render
   cropped by hand. An orthographic option and a zoom or look-at box would
   make visual checks much easier. `mode: ids` was useful as a way to tell
   bodies apart.
7. **Automatic tessellation tolerance is coarse for small holes.** It was
   reported as 0.10 to 0.21 mm for these models; the bore of diameter 15 looks
   clearly polygonal in renders and exports. `bbox` in `document.inspect` is
   mesh based too (cylinder of radius 6 at x = 30: min x 24.08 instead of
   24.0), within the reported `bboxTolerance`.
8. **MCP is heavy for bulk geometry.** `command_batch` is limited to 256
   steps, and each result repeats sketch id, name, status, dof and all new
   entity ids. Building the 50-tooth gear took three batches and very large
   payloads both ways; the same model through `craft3d-cli run --script` is
   one small call. A terse result mode for batches, or one command that adds
   many sketch entities at once, would help.
9. **A valid profile is not a correct design.** The first 50-tooth profile had
   a notch below the root circle (for z >= 42 the base circle lies inside the
   root circle). It was a closed, valid profile and would have extruded
   without complaint; it was caught by reading the numbers. Not an app bug,
   but a reminder that agent-built geometry needs an independent check.

## 4. Missing capabilities that shaped the models

- **Patterns** (circular, linear): the gears are 480 and 700 literal points
  instead of one tooth times z. Module and tooth count are therefore not
  variables; changing them means re-running the script.
- **Booleans and cut extrusion**: plate, post, ribs and head stay separate
  bodies; a hole is only possible as an inner loop of the same sketch; overlap
  or clearance between bodies cannot be checked inside the app.
- **Free-form sketch curves**: involute flanks are 6 line segments each
  (deviation about 0.015 mm at z=30, 0.01 mm at z=50). Tip and root circles
  are true arcs.
- **Revolve, fillet, chamfer, edge and vertex references**: no shaft, no root
  fillet on the gears.

## 5. Not tested

History commands (`feature.suppress`, `feature.move`, `history.rollback`),
over-constrained and conflicting sketches, variable cycles, `symmetric`,
`midpoint`, `angle`, `parallel`, `perpendicular` and `concentric`
constraints, negative extrude distances, units other than mm, extreme scales,
OBJ export, and reading the exports back in FreeCAD.

## 6. Caveats about the models themselves

- Keyway 5 x 2.3 mm (hub) for a 15 mm shaft was taken from memory of
  DIN 6885-1 (key 5 x 5 for shafts over 12 up to 17 mm); verify against the
  standard before manufacturing.
- Gears: pressure angle 20 deg, face width 20 mm and no profile shift were the
  agent's choices. For z=30 the flank below the base circle is a radial line.
- The 50-tooth MCP build used coordinates rounded to 8 decimals; the CLI
  builds used full double precision.

## 7. Scripts

- `scripts/gear_z30_cli.py OUT.json`: command script for the 30-tooth gear
  (valid only while the base circle is outside the root circle, z <= 41).
- `scripts/gear_pair_cli.py OUT.json`: general version, handles both cases,
  builds the meshing pair and writes the outlines for the clearance check.
  Run with `craft3d-cli run --script OUT.json --out target/work/NAME.3dcraft`.
- `scripts/gear_z50_mcp_payload.py a1|a2|b|info`: prints the MCP batch
  payloads used for the pure-MCP 50-tooth build.

## 8. Pictures

All rendered by the app's CPU renderer (`doc_render_preview` or `view.render`),
in `img/`:

| File | Shows |
|---|---|
| `01_plate_post_primitives.png` | block + cylinder |
| `02_plate_post_sketch_driven.png` | constrained plate sketch with 4 holes, hollow post on the end cap |
| `03_plate_post_rounded_ribs.png` | rounded corners, two ribs from a side-face sketch |
| `04_plate_post_hex_head.png` | hexagonal head on the post (final plate model) |
| `05_gear_z30_iso.png`, `06_gear_z30_top.png` | gear m=2, z=30 |
| `07_gear_z50_iso.png`, `08_gear_z50_top.png` | gear m=2, z=50 (built over MCP only) |
| `09_gear_pair_iso.png` | both gears at centre distance 80 |
| `10_gear_pair_top_ids.png` | top view, `mode: ids`, 2048 px; note the perspective |
| `11_gear_pair_mesh_zoom.png` | mesh zone cropped from the previous picture and enlarged 4x |
