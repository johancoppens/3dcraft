# 11. Troubleshooting

3DCraft does not crash on bad input, and does not build a wrong solid when it cannot build the right one:
it says no, with a reason. Reading the reason is nearly always the way forward.

## A command fails

The document is unchanged. The message starts with the command and usually the parameter:

| Message (shortened) | What to do |
|---|---|
| *there is no feature named 'Sheet'; features: Block* | A name is wrong or the feature is not made yet; the message lists what there is |
| *unknown plane 'CutPlane'; use xy, yz or zx, or the name of a datum plane* | Same, for planes |
| *give either distance … or about and angle …, not both* | Two ways of placing something were mixed; use two datums |
| *variable.delete: \`t\` is still used by the feature \`Plate\` (F1: size.z), …* | Change those first, or give `"force": true` to delete it anyway |
| *file.save: '/tmp/x.3dcraft' is outside the write root …* | Save or export beneath the directory the server or window was given |
| *variable.set: \`wall + 1 mm\` uses \`wall\` itself …* | Give the new value (`3 mm`); the message says what it is now |
| *the unit \`rad\` must follow a number directly …*, *\`inch\` is not a unit: write \`in\`* | Units are `um mm cm m in ft deg rad`, in lower case, right after a number |
| *an angle constraint takes a line and a line, but 1 entity was given* | Give the entities the message lists, in that order |

`command_list` (or `craft3d-cli commands --filter …`) shows what parameters a command takes.

## A feature fails

The feature is in the history but has no body: its status is `failed`, with `error`; features made from it
fail with `causedBy` naming the first one. `document.check` lists them all. Fix the first:

- **A size that does not fit** (a fillet radius, a shell thickness): the message gives the largest that
  fits. Make it smaller, or `edit.undo`.
- **A reference that was lost** (an upstream edit took away the face or edge a feature names): the
  message names the name it lost and the feature that made it. Point the feature at the right entity
  with `feature.edit` (`edges`, `faces`, `sketch`, `target`), or undo the upstream edit.
- **Something made from a suppressed feature**: switch it on (`feature.suppress {"suppressed": false}`).
- ***The body of `Bossed` already went into `Bevel`***: another feature took that body first, most often
  one inserted at the rollback marker. Build on the newer feature: the message gives the `feature.edit`
  (`{"feature": "Drilled", "target": "Bevel"}` for a boolean; for a fillet, a chamfer or a face operation
  its `edges` or `faces` named by faces of the newer body).
- **Not built yet**: some cases are refused as *not supported* (a fillet of a radius that would make a
  torus reach its own axis, a shell at a vertex where four faces meet, a boolean the kernel cannot settle
  within the tolerance). The message says which faces; change the design around it (a slightly different
  size, another order of features), and report it: every refusal is work owed.

Over MCP a failed feature is **not** a tool error (the command did what was asked: it added the feature),
so an agent must read `status` in each result, or run `document.check` after a batch. A change that makes
*other* features fail (`variable.set`, `feature.edit`, `feature.suppress`, `feature.delete`, …) lists them
in its result under `failed`, each with its `error`.

## A broken assertion

The model was built, but does not meet what was asked. `assert.list` shows what was measured and what was
expected. Either the design changed in a way you did not want (undo it), or the assertion needs to be
written differently (`assert.edit`). *notProven* is not a failure: the value found is fine but the proof
did not reach the bound (chapter 7).

## Undo

`edit.undo` undoes one change (a batch of ten steps is ten changes); `edit.redo` redoes it. Undo also
brings back a deleted feature or variable. The selection is not part of the history and is not undone.

## A sketch does not do what you meant

`sketch.inspect` shows every entity with its solved coordinates, every constraint with its residual and
state, and `dof`, the freedom left. A point drawn with plain numbers may be moved by the solver; write its
coordinates as expressions (`"0 mm"`) to hold it there. A constraint that contradicts the ones before it
fails the sketch and is named (`"conflicting": ["c10"]`, state `conflicting`), and so are the ones it
contradicts (`"conflicts": [{"constraint": "c10", "contradicts": ["c3"]}]`). Change one of them
(`sketch.edit` with `constraint` and `value`) or delete it (`sketch.delete` with `constraint`). A
contradiction the solver can meet by shrinking a line to nothing (a line both `horizontal` and
`vertical`) solves, but the sketch command's result lists the line under `collapsed` with the constraints
that did it, and `document.check` reports it as a problem.

## Pictures

If a face is hard to identify, render with `mode: "ids"` (one flat colour per face, with a legend) or ask
for a close-up with `projection: "orthographic"` and a `box`. `view.render` and `doc_render_preview` use
the CPU renderer, which is the reference for what the window shows.

## The window

- **It does not start**: it needs a graphics adapter; install Mesa's lavapipe for a software one, or work
  headless with `craft3d-cli`.
- **The agent's changes do not show**: the MCP server must run with `--bridge PORT --control-token-file
  FILE`, the same port and token file the window was started with.
- **After a crash**: start the window again and choose *Recover* in the recovery bar. A window started
  for tests or scripts should get `--recovery-dir none`, so that what it leaves behind does not appear in
  your own recovery bar.

## Logs

The window writes its log to `~/.config/craft3d/logs/craft3d.log` on Linux (`RUST_LOG=debug` for more).

## Reporting a problem

When a command fails in the window, the status bar at the bottom shows the command in red, its message,
and a **Copy details** button. It copies a short plain-text report to the clipboard: the 3DCraft version
and the commit it was built from, your system, the graphics card and how 3DCraft draws with it, and the
command that failed with its parameters and the whole message. A window shows the same text, so you can
read it first: **it contains file paths and the values you or your agent gave, so read it before you post
it publicly.** Paste it into your bug report, together with what you were trying to do (and the
`.3dcraft` file, if you can share it). In the web version the browser only lets a page copy when it is
served over https or from localhost; elsewhere, select the text in that window and copy it yourself.

An agent gets the same report from the command `session.report` (its `text`), so you can also ask it:
*"something went wrong; give me the bug report"*. When the agent works in the window, the report names
the graphics card too.
