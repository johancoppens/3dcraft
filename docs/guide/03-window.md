# 3. Watching and pointing

The window (`craft3d`) is where you watch the agent work and show it what you mean. It does not model by
itself: every change comes in as a command, from the agent through the MCP bridge (chapter 1) or from a
program on the control channel. What it does on its own is open, save and close files, and let you look,
click and select.

![The 3DCraft window](../img/viewer.png)

## What you see

- **Tabs** along the top, one per open document; a dot marks unsaved changes. Ctrl+Tab goes to the next,
  Ctrl+W closes one (asking first if it has unsaved changes). The **File** menu has New, Open, Save and
  Save As (Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S).
- **The feature tree** on the left: the history, in order. Each feature has a mark: ✔ built, ⚠ failed
  (the reason under it; *caused by …* when it failed because a feature it is made from failed: click that
  one), ⏸ suppressed, ○ after the rollback marker. Under the features, the **assertions**, failed ones in
  the failed colour with their message.
- **The viewport**: bodies shaded with their edges, sketches as lines, datum planes as translucent squares.
  The model updates after every command the agent runs; a heavy model first shows a quick picture, then
  the sharp one (the status bar says `refining`).
- **The status bar** at the bottom: units, the file, when it was autosaved, the last command and its
  answer, and which renderer drew the picture.

## Moving around

| Do | To |
|---|---|
| Drag with the left button | Orbit |
| Drag with the right or middle button, or Shift and the left | Pan |
| Wheel, or a pinch on a touchpad | Zoom about the point under the pointer |
| F, or **Fit** | Frame the whole model |
| **Iso**, or a click on a face, edge or corner of the view cube | A standard view |
| **Persp** / **Ortho** | Perspective or orthographic (to scale) |

The bar at the bottom of the viewport switches **Sketches** and **Datums** on and off, makes the bodies
**See-through** (to look at pockets and cavities), and turns on a **Section**: a plane across X, Y or Z,
moved by **Offset**, cutting the picture open (**Flip side** keeps the other half). None of this changes
the document.

## Pointing at things

A click selects what is under the pointer: a vertex if one is within a few points, else an edge, else the
face. Shift+click adds or removes, a double click selects the whole body, a click on empty space clears
the selection, Ctrl+drag selects everything inside a box. A click again at the same place (or Alt+click)
selects the next face behind; a right click lists all the faces under the pointer.

**The agent sees your selection.** The selection belongs to the document, not to the window, so the
agent reads it with the MCP tool `selection_get` (or the command `select.get`): each item with its
persistent name, the body, the feature that made it, its surface (a cylinder's radius and axis, a plane's
normal) and the parameter that drives it with the edit that changes it. So you can say:

> Make the hole I selected 6.5 mm.

and the agent finds the hole's `drivers` and runs the `feature.edit` it lists. The other way round, the
agent can show you what it means: through the bridge, `control_call` with `ui.highlight` (a face, an edge
or a vertex, by name) lights it up in the viewport, `ui.select` sets the selection you see, and
`ui.screenshot` takes a picture of the viewport for the agent to look at.

```text
control_call {"method": "ui.highlight", "params": {"face": "F1:face(1)", "feature": "Plate"}}
control_call {"method": "ui.get"}     -> the camera, what you clicked last (state.picked), the selection
```

## Without the window

An agent working headless looks at its work with `doc_render_preview` (or the command `view.render`): the
same CPU renderer that `craft3d-cli render` uses. `view.pick` answers "what is at this pixel of that
picture", as a click does in the window.
