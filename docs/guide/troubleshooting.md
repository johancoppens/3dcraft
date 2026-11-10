# Troubleshooting

*What to do when 3DCraft says no, when something does not show, after a crash, and how to report a
problem.*

3DCraft does not crash on bad input, and does not build a wrong solid when it cannot build the right one:
it says no, with a reason. Reading the reason is nearly always the way forward; when an agent works for
you, ask it to read the reason to you.

## Quick answers

| Problem | Look at |
|---|---|
| The window does not start | [The window](#the-window) |
| The agent's changes do not show in the window | [The window](#the-window) |
| A feature has a ⚠ in the feature list | [A feature fails](#a-feature-fails) |
| An assertion is red | [A broken assertion](#a-broken-assertion) |
| A Jig's field turned red | [Jigs](#jigs) |
| The web version is slow, blank or will not save | [The web version](#the-web-version) |
| 3DCraft closed unexpectedly | [After a crash](#after-a-crash) |
| Something is wrong and you want to tell us | [Reporting a problem](#reporting-a-problem) |

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
| *the unit \`rad\` must follow a number, a parenthesis or \`pi\` directly …*, *\`inch\` is not a unit: write \`in\`* | Units are `um mm cm m in ft deg rad`, in lower case, right after a number, a `)` or `pi` (`(360 / n) deg`); after a variable, multiply by one (`width * 1 mm`) |
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
- **Something made from a suppressed or inactive feature** (a sketch on a face only the suppressed feature
  made, an extrusion of a sketch that is off): switch it on (`feature.suppress {"suppressed": false}`, or
  set the parameter its condition reads), or give the dependent feature the same condition
  (`feature.when`). A fillet, chamfer or face operation does not fail: it leaves out the edges and faces
  that are not there.
- **A part that should disappear stays**: its condition is on a feature that changes the body (a
  transform, a fillet), which passes the body on unchanged when it is off. Put the condition on the
  feature that makes the part ([Formulas, tables and conditions](formulas.md#switching-parts-on-and-off)).
- ***The body of `Bossed` already went into `Bevel`***: another feature took that body first (one inserted at the
  rollback marker re-points the features after it where it can, so this is left for a boolean of the two
  bodies an inserted union joined, the tool of an inserted subtraction, or a body named twice). Build on the newer feature: the message gives the `feature.edit`
  (`{"feature": "Drilled", "target": "Bevel"}` for a boolean; for a fillet, a chamfer or a face operation
  its `edges` or `faces` named by faces of the newer body).
- **Not built yet**: some cases are refused as *not supported* (a fillet of a radius that would make a
  torus reach its own axis, a shell at a vertex where four faces meet, a boolean the kernel cannot settle
  within the tolerance). The message says which faces; change the design around it (a slightly different
  size, another order of features), and report it: every refusal is work owed.

Over MCP a failed feature is **not** a tool error of `command_run` (the command did what was asked: it
added the feature), so an agent reads `status` in each result; a `command_batch` that leaves a feature it
broke or edited failing is an error, listed under `featuresFailed`. A change that makes
*other* features fail (`variable.set`, `feature.edit`, `feature.suppress`, `feature.delete`, …) lists them
in its result under `failed`, each with its `error`.

## A broken assertion

The model was built, but does not meet what was asked. `assert.list` shows what was measured and what was
expected. Either the design changed in a way you did not want (undo it), or the assertion needs to be
written differently (`assert.edit`). *notProven* is not a failure: the value found is fine but the proof
did not reach the bound ([Measuring and assertions](measure.md)).

## Undo

**Edit › Undo** (Ctrl+Z) undoes one change, yours or the agent's, and **Edit › Redo** (Ctrl+Shift+Z or
Ctrl+Y) redoes it; an agent does the same with `edit.undo` and `edit.redo` (a batch of ten steps is ten
changes). Undo also brings back a deleted feature or variable. The selection is not part of the history
and is not undone. In a Jig, **↺**, **Reset all** and **Back to last good** are there as well.

## A sketch does not do what you meant

`sketch.inspect` shows every entity with its solved coordinates, every constraint with its residual and
state, and `dof`, the freedom left. A point drawn with plain numbers may be moved by the solver; write its
coordinates as expressions (`"0 mm"`) to hold it there. In one `sketch.path`, once a corner is written
with an expression, its corners given as plain numbers are held too. A constraint that contradicts the ones before it
fails the sketch and is named (`"conflicting": ["c10"]`, state `conflicting`), and so are the ones it
contradicts (`"conflicts": [{"constraint": "c10", "contradicts": ["c3"]}]`). Change one of them
(`sketch.edit` with `constraint` and `value`) or delete it (`sketch.delete` with `constraint`). A
contradiction the solver can meet by shrinking a line to nothing (a line both `horizontal` and
`vertical`) solves, but the sketch command's result lists the line under `collapsed` with the constraints
that did it (`contradiction: true`), and `document.check` reports it as a problem. A line with no length
under one direction only is a warning there (`kind: "collapsedLine"`).

## Pictures

If a face is hard to identify, render with `mode: "ids"` (one flat colour per face, with a legend) or ask
for a close-up with `projection: "orthographic"` and a `box`. `view.render` and `doc_render_preview` use
the CPU renderer, which is the reference for what the window shows.

## Jigs

- **A field turned red** and the banner says *This value breaks the model*: that combination of values
  cannot be built (the reason is under the field). Click **Back to last good**, or change the value back.
  Tell the Jig's maker: a Jig's ranges should always build.
- **A value jumped** after you typed it: it was outside the parameter's range, or between two steps. The
  note under the field (*clamped to …*, *rounded to …*) says what was set.
- **An agent cannot change the Jig's features**: in a Jig only the parameters change. Use **Open for
  editing** to get a model you can change.

## The window

- **It does not start**, and a terminal says *no graphics adapter can show it*: 3DCraft's window is drawn
  through Vulkan, Metal, DirectX 12 or OpenGL. On a computer without a suitable graphics card, install a
  software one: on Linux Mesa's lavapipe (`mesa-vulkan-drivers`) or llvmpipe for OpenGL; on Windows update
  the graphics driver. Everything else works without it: `craft3d-cli` can still model, render and export.
- **The agent's changes do not show**: the window must have been started with `--control PORT
  --control-token-file FILE`, and the agent's MCP server with `--bridge PORT --control-token-file FILE`,
  the same port and the same file ([Install 3DCraft and connect an AI agent](install.md#connect-an-ai-agent-over-mcp)). A window
  started from the menu does not listen.
- **The agent cannot open or save a file**: the path is outside the folders the window was given
  (`--automation-read-root`, `--automation-write-root`); the error says so. Save from the File menu
  yourself, or start the window with a root that holds the file.
- **The model looks rough while it is still being refined**: a heavy model first shows a quick picture
  and then the sharp one; the status bar says `refining` meanwhile.

## The web version

- **It is slow on a big model**: in the browser 3DCraft rebuilds the model beside the page, in a worker
  of its own, and measures the checks (assertions and outputs) in another, so the page stays usable while
  they run: the last picture stays, the view turns, and a new value cancels the rebuild that was running.
  Selecting a face, measuring and exporting still rebuild the model on the page the first time after a
  change, so they can take a moment on a big model. Where the browser refuses the workers, the page
  rebuilds a feature at a time with the page drawn in between; `?pageregen` in the address does that on
  purpose.
- **The page stays blank or the model does not show**: the page needs WebGPU or WebGL2. Try a recent
  Chrome, Edge or Firefox; add `?webgl` to the address to use WebGL2 instead of WebGPU.
- **Save makes a download** instead of asking where: that is how Firefox and Safari work
  ([Files, export and drawings](files.md#files-in-the-web-version)).
- **Copy details does not copy**: browsers let a page use the clipboard only over https (or from your own
  computer); select the text in the details window and copy it yourself.

## After a crash

Start 3DCraft again: a bar under the menu says *3DCraft did not close properly* and offers what was
autosaved. **Recover** opens it as an unsaved document (save it under a name); **Discard** throws it away
(click twice); **Later** keeps it for next time. The autosaves are in 3DCraft's data folder (**Help ›
Open data folder**). The web version has no autosave: save often.

A window started for tests or scripts should get `--recovery-dir none`, so that what it leaves behind does
not appear in your own recovery bar.

## Reporting a problem

![An error dialog with Copy details](images/error-dialog.png)

When a file action fails, a dialog says what went wrong, with **Copy details**. When a command fails in
the window, the status bar at the bottom shows the command in red, its message,
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

Report problems, with what you were trying to do and the copied details, on
[GitHub](https://github.com/johancoppens/3dcraft/issues), or to whoever gave you 3DCraft. Every refusal
you report helps: in 3DCraft, a refusal is work still to do.

## 4. Tell us what you found

Write down, for each thing that went wrong or felt wrong: what you were trying to do, what you asked the
agent (copy the prompt), and what happened.

When a command failed, add the **bug report**: click **Copy details** in the status bar (or in the error
dialog of a file action) and paste it; or ask the agent *"give me the bug report"* (it runs
`session.report`). The report holds the 3DCraft version and the exact build it came from, your system,
your graphics card and how 3DCraft draws with it, and the command that failed with its values and the
whole message. **Read it before you share it**: it contains file paths and the values you or your agent
gave. Add the `.3dcraft` file if you can share it. [Troubleshooting](troubleshooting.md#reporting-a-problem)
has the details.

Send it by mail to the developer, or through the shared form you were given with the invitation. Things that went well are worth a line too: they tell us what not to break.

If 3DCraft ever closes by itself, start it again: it offers to recover your work ([Troubleshooting](troubleshooting.md#after-a-crash)). A crash is the most important thing you can report.

**See also:** [the reference](reference.md), for every command, key and unit.
