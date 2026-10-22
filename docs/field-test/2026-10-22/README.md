# Second field test: what the first one did not touch (2026-10-22)

> Played by an agent the way the R1 testers will work: through an AI agent driving
> `craft3d-cli mcp --recovery-dir none` over stdio JSON-RPC, knowing 3DCraft only from `command_list`,
> the [user guide](../../guide/README.md) and the error messages. The scope is what the
> [first field test](../2026-10-12/README.md) left out (its section 5, and R1 › Open in
> [`roadmap.md`](../../roadmap.md)): history commands, conflicting and under- or over-constrained
> sketches, every sketch constraint kind, negative distances and held coordinates, other units, a change
> request on an example, and the guide's instructions taken literally. Every friction found is in the
> table of section 3, with what became of it: fixed on this branch (with a test), a line under R1 › Open
> in the roadmap, or left as it is.

## 1. How it was run

- The server: `craft3d-cli mcp --recovery-dir none`, with `--automation-read-root` and
  `--automation-write-root` set to a scratch directory for the file steps; built from `b6b7d32`.
- The driver: [`scripts/drive.py`](scripts/drive.py), a small MCP client that sends `initialize`, then
  one `tools/call` per step (`command_run`, and `command_list`, `command_batch`, `doc_inspect`,
  `doc_render_preview` where a script asks for them), and prints every result in full. The sessions are
  the JSON scripts next to it, run in this order:

| Script | What it plays |
|---|---|
| [`a1-constraints.json`](scripts/a1-constraints.json), [`a2-constraints.json`](scripts/a2-constraints.json) | Every one of the 18 constraint kinds of `sketch.constrain`, on free points, with `dof` read after each |
| [`b-conflicts.json`](scripts/b-conflicts.json) | Redundant, conflicting and impossible constraints, an impossible triangle, an under-constrained profile, wrong entity kinds and values; recovery |
| [`c1-history.json`](scripts/c1-history.json), [`c2-history.json`](scripts/c2-history.json) | Undo and redo, suppress, rollback with an insert, move, rename, `feature.edit`, delete, a redundant constraint turned conflicting by a variable |
| [`d1-units.json`](scripts/d1-units.json), [`d2-units.json`](scripts/d2-units.json) | An inch document, cm and m documents, mixed units, angles in degrees and radians, misspelled units, counts |
| [`e1-negative.json`](scripts/e1-negative.json), [`e2-negative.json`](scripts/e2-negative.json) | Negative and zero extrusions, `reversed`, a rectangle drawn backwards, held coordinates going negative and to zero, negative radii and walls |
| [`f-change-request.json`](scripts/f-change-request.json) | The [case](../../examples/case.json) changed as a customer asks: "1 mm more wall", "a grid of vents in the bottom", "rename and drive it by a variable" |
| [`g1-guide-chapter-2.json`](scripts/g1-guide-chapter-2.json), [`g2-guide-chapters-4-5-8.json`](scripts/g2-guide-chapters-4-5-8.json), [`g3-madeby.json`](scripts/g3-madeby.json) | The guide's own commands, as written |
| [`h-misc.json`](scripts/h-misc.json) | A batch with a failing feature in the middle, save outside the root, reopen, the guide's *notProven* example, an inch part with assertions in millimetres |

About 450 commands in all. **Nothing crashed, no command left the document in a state it did not
report, and every volume read was the closed form**: the equilateral triangle of side 40 extruded 3
(2078.4609690826537 = 40² √3/4 · 3), the guide's flange (40180.97003941347) and bracket (6491.205780556065;
8491.20578055609 with the longer leg), 0.5 in³ asserted as `8193.532 * (1 mm)^3`, a 2 in plate asserted as
`50.8 mm` long.

## 2. What worked well

- **Every constraint kind solved, and the bookkeeping was exact.** Each of `fixed`, `horizontal`,
  `vertical`, `parallel`, `perpendicular`, `distance` (a line, two points), `coincident`, `angle`,
  `equal`, `midpoint`, `pointOn` (a point on a line), `symmetric`, `concentric`,
  `radius`, `diameter`, `x`, `y` and `tangent` (a line to a circle) took off the degrees of freedom it
  should; the residuals were at most 7e-15.
- **Conflicts are found and named.** A second length on a held line: `"conflicting": ["c10"]`, the
  constraint kept with state `conflicting` and the sketch failed, what is made from it failing with
  `causedBy`. A duplicate is `redundant` and harmless. The impossible triangle 10, 10, 30 was refused
  the same way, and `sketch.edit` of the third side to 15 repaired it in one step. The redundant length
  that a later `variable.set` turned into a conflict was reported in that command's own `failed`.
- **Wrong kinds are refused with what was wanted:** *a radius constraint takes a circle or arc, got e5 (a
  line)*; *unknown constraint type `Horizontal`; the types are: …*; a negative length *cannot be
  negative: give its size, and draw the point on the side it is to lie on*.
- **History edits say what they broke.** `feature.suppress`, `feature.delete`, `feature.edit`,
  `history.rollback {"feature": null}` and `variable.set` each listed the features they made fail, with
  the reason, in their own result; `feature.move` before an input: *`Plate`, which makes F1:face(2) and
  F1:face(4), comes after this feature in the history; move it before (feature.move)*. Undo and redo
  name the step (`{"undone": "Add fillet"}`), and *there is nothing to redo* after a new change.
- **Units.** Inch, cm, m, ft and um documents; `1 in + 3 mm`, `2.54 cm + 0.01 m`, `1 ft - 1 in` exact;
  a length where a number is wanted and the reverse refused with both dimensions named; assertions read
  in the document's unit with bounds in any unit.
- **Negative values** do what a CAD user expects: a negative extrusion goes the other way (`reversed`
  with a negative distance comes back), a rectangle drawn from right to left is the same rectangle, held
  coordinates may be negative and may cross zero; a zero-length side is caught (*e5: the line has no
  length*) and a negative radius, height or block size is refused by name.
- **The guide is true where it shows numbers.** Chapter 2's session reproduced to the last digit (the
  thinnest wall 6.0 with `atLeast` 5.752661301804291), chapter 4's failed-features text word for word,
  chapter 5's volumes, chapter 7's *notProven* example exactly, and `throughAll` pointed the wrong way
  said *reversed goes the other way*.

## 3. Findings and what became of them

**Fixed** means on branch `r1-fieldtest-2` with a test (`crates/engine/tests/field_test_two.rs`,
`crates/automation/tests/mcp.rs` › `a_terse_batch_lists_the_features_that_were_added_but_failed`,
`crates/expr` unit tests) and, where the guide was wrong or silent, a guide change. **Roadmap** means a
line under R1 › Open. **Won't** means it stays as it is, with the reason.

| # | Area | Friction | Outcome |
|---|---|---|---|
| 1 | History | After `history.rollback` and an inserted chamfer, the boolean after it failed with *use that feature as the source, or give `Bevel` keep*: the parameter is `target`, and a chamfer has no `keep` | Fixed: *build on `Bevel` instead (feature.edit with "target": "Bevel")*, `keep`/`keepTool` offered only where the taker has it; a fillet, chamfer or face operation follows its faces by itself, and its message (if it ever gets one) points at its `edges` or `faces` |
| 2 | History | That insert breaks every feature after the marker that used the body; the user must re-point each one | Roadmap |
| 3 | History | Suppressing a boolean, fillet or chamfer fails everything after it; other modellers pass the body through | Roadmap; the summary of `feature.suppress` and the guide now say so |
| 4 | History | `feature.suppress`, `feature.move`, `feature.delete`, `history.rollback` summaries said nothing about what happens after them; `position` did not say it counts sketches | Fixed (summaries, guide chapter 4) |
| 5 | History | Guide: `feature.edit {"count": 8}` silently replaces the variable that drove the count | Fixed (guide: `"count": "n"`, and why) |
| 6 | Assertions | After a cut was added to `Case`, its assertions answered *the body of `Case` went into F6; name that feature*: an id where the agent works with names | Fixed: *went into F6, `Vented`*; the same in interference, `document.edges` and the selection |
| 7 | Assertions | An assertion on the part's body breaks at every feature added on top; it has to be re-pointed by hand | Roadmap (a subject that follows the chain); guide chapter 7 says how to re-point |
| 8 | Sketch | A conflict said *change or delete it* without the commands | Fixed: *(sketch.edit with a new value, or sketch.delete)* |
| 9 | Sketch | A conflict names the constraint that came last, not the ones it contradicts | Roadmap; the guide no longer claims it names *the constraints involved* |
| 10 | Sketch | `horizontal` and `vertical` on one line solve by shrinking it to a point, reported as satisfied, `dof` 2, `status` ok | Roadmap; guide chapter 11 says it shows only when something is made from it |
| 11 | Sketch | An under-constrained profile (`dof` 8) extrudes with no word anywhere, `document.check` included | Roadmap; guide chapter 4 says to check `dof` |
| 12 | Sketch | A constraint with a literal value it can never take (`-10 mm`, `10 deg` for a length) is added and fails the sketch, `dof: null`, instead of being refused | Roadmap |
| 13 | Sketch | *a angle constraint takes a line and a line, but 1 were given*; *`10 deg` is a angle* | Fixed (article and number) |
| 14 | Sketch on a face | On the bottom face of the case the sketch's x is world Y: a 4 × 3 vent grid came out 3 × 4 and off centre, valid and silent ([picture](img/case-vents-from-below.png)); the frame was only in `sketch.inspect` | Fixed in part: `sketch.create` returns `frame` and `normal`, the guide explains face frames; roadmap: a way to choose the axes |
| 15 | Units | `pi / 4 rad` is π/(4 rad) and `1/16 in` is 1/(16 in): accepted, `value` printed as if right (0.785…), dimension `angle^-1` / `length^-1` | Fixed: when a `/` is followed by a number with a unit and the result is an inverse, `variable.set` warns and a refusal of it written inline says how to write it (`1 in / 16`, `pi / 4 * 1 rad`); `1 / (2 mm)` and `10 mm / (360 deg)` are left alone |
| 16 | Units | `(pi / 4) rad` and `2 inch`, `3 MM`, `5 degrees`: *expected an operator or the end of the expression, found `rad`* | Fixed: *the unit `rad` must follow a number directly … multiply by one: `(pi / 4) * 1 rad`*; *`inch` is not a unit: write `in`* |
| 17 | Units | An angle variable's `value` is in radians while the document's angle unit is `deg`; nothing said so | Fixed: `angleUnit: "rad"` and `degrees` in the result |
| 18 | Units | A plain number as an angle parameter is degrees, inside `sin` it is radians, and `pi` is a plain number: `"angle": "2 * pi / n"` gives a pattern a few degrees wide, overlapping, no warning | Roadmap; guide chapter 4 explains it and says to write `360 deg / n` |
| 19 | Units | No command to change a document's length unit after `file.new` | Roadmap |
| 20 | Variables | "Make the wall 1 mm thicker" as `wall + 1 mm` was kept as a circular definition and failed the case, its shell and both assertions | Fixed: refused, *give its new value instead (it is `2 mm` now)*; a cycle through other variables is still kept, failing with *circular definition* (documented) |
| 21 | Variables | No `variable.list`; the variables are in `document.inspect` | Won't: documented in chapter 4 |
| 22 | Faces | A negative shell or thicken wall: *has to be more than this document's tolerance, got -2 mm* | Fixed: *has to be more than zero*, and for thicken that `side` chooses the side |
| 23 | Selection | The guide's `select.query {"kind": "edge", "where": {"madeBy": "Rounded"}}` returned nothing: an edge's name carried only the body's last feature | Fixed: `madeBy` also looks at the faces an edge or a vertex lies on |
| 24 | Names | A feature's name in a face name (`Cube:face(2)`) is refused, and ids shift when features are inserted, so a script with ids breaks | Roadmap |
| 25 | Batches | In a `terse` batch, a fillet in the middle that could not be built was nowhere in the report (`failed: 0`, the last result fine) | Fixed: `featuresFailed: [{step, feature, id, error, causedBy?}]`, the features a step left failing that still fail at the end, in the tool's description and the guide |
| 26 | MCP | Guide: `document.inspect {"measures": false}` next to `doc_inspect`, which takes no arguments | Fixed (guide); won't change the tool |
| 27 | Inspect | `document.inspect` shows parameters in the document's own words (`keep_tool`, `"target": 1`) rather than the commands' (`keepTool`, a name), and an assertion's `measured` as text there but a number in `assert.list` | Roadmap |
| 28 | History | `edit.undo` takes no count, `feature.move` no `before`/`after` | Won't: one step per call is what the guide says; the refusals list the parameters |

## 4. Details worth reading

**The change request (script `f`).** Starting from the case example:

1. "1 mm more wall": `variable.set {"name": "wall", "expr": "wall + 1 mm"}` broke the whole part (finding
   20). `"3 mm"` then passed both assertions (volume 24945.398 mm³, wall 3 mm with `atLeast` 2.95).
2. "A grid of 4 × 3 vents of 3 mm in the bottom": `select.query` with `context` found the faces facing
   up; a sketch on `F1:face(0)`, the bottom, a circle and `sketch.pattern` 4 × 3, extruded `throughAll`
   with `cut`. The extrusion went away from the part (*throughAll: there is nothing to go through along
   the sketch's normal … (reversed goes the other way)*), which was clear; `feature.edit {"reversed": true}`
   built it. The vents were then cut, but transposed and off centre, because the bottom face's sketch x
   is world Y (finding 14). Both case assertions went to *error*: they name `Case`, whose body went into
   the cut (findings 6 and 7). The count assertion on `{"hole": true, "radius": "vent_d / 2"}` passed with
   12.
3. "Drive the vent count by a variable": `sketch.pattern` with `replace` and `count: "vents_x"` failed
   while `vents_x` was not yet defined (*unknown variable*), and worked once it was.
4. "Rename `length` to `case_length`": the assertion's expression followed (`case_length * width …`),
   and setting it to 90 mm regenerated with the volume assertion passing.

**Rollback and insert (script `c2`).** `history.rollback {"feature": "Bossed"}`, a chamfer on the plate's
top edge, `history.rollback {"feature": null}`: the subtraction after the marker failed, because its
target's body now went into the chamfer. Reading the old message literally, the agent tried
`feature.edit {"source": "Bevel"}` and was refused (*`source` is not a parameter of a boolean*);
`"target": "Bevel"` repaired it. Meanwhile the fillet after the subtraction, which names faces rather
than a body, had followed the chamfer's body and built without the hole: correct as far as it goes,
flagged only through the failed subtraction.

**Suppress and delete (scripts `c1` and `c2`).** Suppressing the boss failed the union (*tool: `Boss` is
suppressed*) and with it everything after; suppressing the union too moved the failure one step on
(*target: `Bossed` is suppressed*). Deleting the union instead left the chamfer on the plate and the
boss as a separate body, with no failure: delete passes the body on, suppress does not (finding 3).

## 5. Pictures

| File | Shows |
|---|---|
| [`img/case-vents-from-below.png`](img/case-vents-from-below.png) | The case from below (`doc_render_preview`, orthographic): the vent grid meant as 4 across and 3 deep came out 3 across and 4 deep, off centre (finding 14) |
