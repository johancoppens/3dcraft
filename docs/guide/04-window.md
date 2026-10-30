# 4. The window

*Everything you see in the 3DCraft window, and how to look around, play the history and point at things.*

The window (`craft3d`) is where you watch the agent work, look at a design and show the agent what you
mean. It does not model on its own: every change to a design comes in as a command, from an agent through
the MCP bridge ([chapter 1](01-install.md#connect-an-ai-agent-over-mcp)), from a Jig's parameter form
([chapter 5](05-jigs.md)) or from a program on the control channel. What the window does by itself is
open, save, import and export files, and let you look, play, click and select.

![The window with the L-bracket, marked 1 to 9](images/window.png)

| | Part | What it is for |
|---|---|---|
| 1 | **File**, **Edit** and **Help** menus | Files, recent files, Jigs; Undo and Redo; the guide, the keys, About |
| 2 | **Tabs** | One per open document |
| 3 | **Feature list** | The history of the design, in order, with each step's status |
| 4 | **Eye** | Hides a feature's body, sketch or datum in the view |
| 5 | **Assertions** | The checks the design must pass |
| 6 | **History player** | Watch the design being built, step by step |
| 7 | **View buttons** | **Fit**, **Iso**, **Persp**, **Ortho** and **Studio** |
| 8 | **View cube** | Click a side, edge or corner to look from there |
| 9 | **Display bar** | **Sketches**, **Datums**, **See-through** and **Section** |

At the bottom is the **status bar**: the document's units, its file, when it was autosaved, the last
command and its answer, and which renderer drew the picture. While 3DCraft is working, a thin bar runs
across the top of the viewport and the status bar says what it is doing, on the right: *Opening
welding-table… 41/82* while a file's features are built, then *Checking 14 assertions… 5/14* while its
assertions are measured (the model is already there to look at). Until a newly opened file has
something to show, the middle of the viewport says *Opening welding-table…*.

## The start screen

Without a document, the window shows the **start screen** (picture in [chapter 2](02-first-steps.md)):

- **New document** and **Open…**;
- **Examples**: the [example models](../examples/README.md) as cards with a picture. A click builds one in
  a new tab, from the same command script an agent would send;
- **Recent**: the files you opened or saved last;
- **Connect an AI agent**: the two command lines that connect an agent to this window, with this window's
  own port and token file when it listens (it was started with `--control`). **Copy configuration** puts
  the MCP configuration on the clipboard, **Copy for Claude Code** the `claude mcp add` line, and **How it
  works** opens the part of this guide that explains it. In the browser, the start screen says that an
  agent cannot drive 3DCraft there yet.

The start screen comes back when you close the last tab.

## Menus and tabs

**Tabs** along the top, one per open document; a dot marks unsaved changes. **Ctrl+Tab** goes to the next
tab and **Ctrl+Shift+Tab** to the previous one; **Ctrl+W** closes one, asking first when it has unsaved
changes (**Save**, **Don't Save**, **Cancel**). On a Mac, ⌘ takes the place of Ctrl.

The **File** menu:

| Item | Keys | Does |
|---|---|---|
| **New** | Ctrl+N | A new, empty document in a new tab |
| **Open…** | Ctrl+O | Opens a `.3dcraft` document or a `.3dcraftjig` Jig (a STEP file is imported with Import…) |
| **Open for editing** | | For a Jig in use: a new untitled model of it, to change as you like ([chapter 5](05-jigs.md)) |
| **Save** | Ctrl+S | Saves to the document's file (asks for a name the first time) |
| **Save As…** | Ctrl+Shift+S | Saves under a new name |
| **Import…** | Ctrl+I | Reads a STEP file into the document |
| **Export…** | Ctrl+E | Writes STEP, STL, 3MF, OBJ, DXF or SVG ([chapter 12](12-files.md)) |
| **Export as Jig…** | | Publishes a document with parameters as a `.3dcraftjig` ([chapter 5](05-jigs.md)) |
| **Recent files** | | The last ten files opened or saved, by you or by an agent, newest first; **Clear recent** empties the list |
| **Close** | Ctrl+W | Closes the active tab |

Under them, the open documents: click one to switch to it.

A recent file that is not there any more is greyed out. The list is kept in `3dcraft/recent-files.json`
in your state folder (next to the autosaves), and two windows open at once each keep what the other
added. The browser keeps no recent files.

The **Edit** menu:

| Item | Keys | Does |
|---|---|---|
| **Undo** | Ctrl+Z | Undoes the last change to the active document; the item names it (**Undo Set teeth**) |
| **Redo** | Ctrl+Shift+Z or Ctrl+Y | Redoes the change last undone |

An item is greyed out when there is nothing to undo or redo, and while a parameter is being dragged or set. Every change counts, whoever made it: you,
or an agent over MCP. While you type into a field, Ctrl+Z undoes your typing instead. In a Jig in use
the only changes are those of its parameters, so Undo takes back the last parameter you set.

The **Help** menu:

- **User guide** opens this guide in your browser.
- **Keyboard shortcuts** (also **F1**) lists every key and mouse action ([chapter 15](15-reference.md#keys-and-mouse)).
- **Open data folder** shows 3DCraft's folder: the autosaves, what can be recovered after a crash, and the
  recent files (not in the browser).
- **About 3DCraft** gives the version and the commit it was built from, the platform, the licence (MIT),
  links to the guide and the source, and the renderer in use. **Copy system info** copies those lines
  for a bug report.

## The feature list

The left panel shows the history of the design, one row per step: the name, the kind of feature and its
state. Each row has a mark:

| Mark | Means |
|---|---|
| ✔ | Built |
| ⚠ | Failed: the reason is under it; *caused by …* when it failed because a feature it is made from failed (click that one) |
| ⏸ | Suppressed: switched off by hand |
| ○ | After the rollback marker: not built for now |

A feature switched off by a condition (*inactive*, [chapter 8](08-formulas.md#switching-parts-on-and-off))
is greyed, with a *when* mark; its condition shows when you hold the pointer over it.

- **Click a row** to select the feature: the faces it made light up in the view (a fillet's round, not the
  whole part). A pattern or a mirror lights the copies it made.
- **The eye** at the end of a row hides that feature's body, sketch or datum in the view; click it again
  to show it. The model itself does not change: the hidden part is still built, measured and exported.
- **Double-click a row**, or click the **⌖** that shows on the row under the pointer, to **fly to** the
  feature: the camera turns to the view that shows it best, with some of its body around it. A feature
  that failed, is suppressed or shows nothing has no ⌖. A double click on a hidden feature shows it
  again and flies to it.
- When you click something in the view, the list scrolls to the feature that made it.

Under the features are the **assertions**, failed ones in red with their message. While there are none, a
short hint says how to add one.

**A Jig** has two tabs here: **Parameters** (its form) and **Features** (this list, to look at).

## The history player

![The history player](images/player.png)

At the foot of the feature list is the **timeline**: drag it to see the design as it was after any step
of its history. Features after that step are greyed, and the step is named at the top of the view, such as
`Step 3/6  Join`. Looking back never changes the document.

Under the timeline is a **player**, like a video player's:

| Button | Key | Does |
|---|---|---|
| ⏮ First step | Home | Shows the first step |
| ⏴ Step back | ← | One step back |
| ▶ Play / ⏸ Pause | Space | Plays the history step by step, or pauses |
| ⏵ Step forward | → | One step forward |
| ⏭ To end | End | Back to the whole design |
| **1×** | | The speed: 0.5×, 1× or 2× (click to change) |
| Loop | | Starts again at the first step after the last |
| Orbit | | Turns the camera slowly while it plays |

In a narrow feature panel the speed, Loop and Orbit move to a second row, under the other five.

The keys work while the pointer is over the view or the feature list and you are not typing in a field.
Each step is shown once its model is built, and for at least 0.8 s at 1×, so a heavy model never skips a
step. Playing stops at the end, when the document changes or when you switch tabs.

**Roll back here** puts the *rollback marker* on the step shown: the design is then built only up to that
step, and new features go in right after it, a way to insert something early in the history. **To end**
takes the marker away again. While you look back, clicks in the view do not select; go **To end** first.

### Playing a motion

A design with a **timeline** can also be played as a motion. The timeline is a variable `t`, a plain number
from 0 to 1 (`variable.set {"name": "t", "expr": "0"}`); formulas of `t` move the design, such as a crank angle
`360 deg * t`. A `t` with a unit (a thickness called `t`) is not a timeline: rename it first.

With a timeline, **History** and **Motion** appear above the player. In Motion the slider runs over `t`, and the
player plays it: from 0 to 1 in 4 s at 1×, **Loop** starts again at 0, **Back and forth** turns round at the
end, and **Collisions** draws the parts that overlap in red. The view names the moment, such as
`t = 0.500 · collision: Wall and Crank`. As with the history, nothing in the document changes; only the
features that use `t` are rebuilt for each moment, so a heavy model plays slower rather than skipping.

An agent does the same with `view.motion {"t": 0.5, "collisions": true}`, draws a point's path with
`view.trace` (exported with `file.export {"path": "path.svg", "trace": "Trace 1"}`), checks assertions over the
motion with `motion.check` ([Measure and check](10-measure.md#over-a-motion)), and makes a video of it with
`view.replay {"dir": "frames", "motion": {"frames": 120}}`.

## Looking around

| Do | To |
|---|---|
| Drag with the left button | Orbit |
| Drag with the right or middle button, or Shift and the left | Pan |
| Wheel, or pinch on a touchpad | Zoom about the point under the pointer |
| **F** | Fly to the selection, or frame the whole model when nothing is selected |
| **Fit** | Frame the whole model |
| **Iso**, or a click on the view cube | A standard view (Iso looks from the front-right corner) |
| **Persp** / **Ortho** | Perspective, or orthographic (to scale, no vanishing point) |
| **Studio** | The studio look on or off (see *The studio look* below) |

The **display bar** at the bottom of the view:

- **Sketches** and **Datums** show or hide sketches and construction planes, axes and points.
- **See-through** makes the bodies transparent (drag the percentage), to look at pockets and cavities.
- **Section** cuts the view open with a plane across X, Y or Z; **Offset** moves it and **Flip side**
  keeps the other half.

None of this changes the document. Each tab remembers its own camera, section and see-through. A heavy
model first shows a quick picture, then the sharp one (the status bar says `refining`).

### The studio look

**Studio**, beside Persp and Ortho at the top right, switches between the plain look, which every
picture had so far and which is best for modelling, and the studio look, which shows what a part will look like: its colours,
how metal and how rough its faces are (Appearance), lit by a photographed environment. Three come with
3DCraft: `studio` (soft boxes, the default), `outdoor` (sky and sun) and `workshop`. The light falls
from the environment's brightest direction and casts soft shadows on the model and on a ground under
it; corners and grooves darken where little light reaches. A see-through or cut-open view is drawn in
the plain look.

An agent sets the same look with `ui.set`, and pictures from the command line take it too:

```text
ui.set {"look": {"mode": "studio", "environment": "outdoor", "exposure": 0.5, "rotation": 90}}
craft3d-cli render part.3dcraft --out part.png --look studio --environment workshop
```

`exposure` makes the picture brighter in stops (negative darker), `rotation` turns the environment
about the vertical axis, `ground`, `ao` and `shadows` switch the ground, the darkening in corners and
the shadows off, `tonemap` chooses how bright light is fitted into the picture (`agx`, the
default, or `aces`), and `background` `plain` keeps the window's own background colour behind the
model while the environment still lights it (`environment`, the default, shows the environment). The look is a view: the document does not change. In the browser on older
graphics (WebGL2) the studio look has no shadows and no darkening in corners.

The studio look also has a second layer of polish, on by default:

- glossy faces (roughness under 0.4) reflect the model itself where it is in the picture, and the
  environment elsewhere (`reflections`); the ground can reflect it too (`groundReflection` from 0 to
  1, off by default: towards the horizon the ground then mirrors the sky and the background
  brightens, and the ground reflects only what is in the picture, so under a model whose
  undersides you cannot see the reflection comes out in pieces);
- short, dark contact shadows where parts touch or stand on each other (`contactShadows`);
- a face in the key light's shadow loses that light's reflection too (`specularShadow`);
- bright parts glow a little into their surroundings (`bloom`, the share of the light spread, 0.04 by
  default).

Depth of field is off until you ask for it: `dof` is a thin camera lens with an f-number (`fstop`,
2.8 by default; smaller is more blur) focused on a point. `"focus": "picked"` focuses on the point you
click last, so clicking a part brings it into focus; a number is a distance along the view, `[x, y,
z]` a point, and without one the lens focuses on the model's centre. The blur follows the
document's length unit, as a real camera would: a small part seen close up has a shallow depth of
field. An orthographic view has no lens and stays sharp.

While the camera stands still the window keeps refining the picture: it averages 16 frames
(`accumulate`, 1 to 256), each with its samples moved a little within the pixel and the lens turned,
so edges smooth out and the blur fills in; moving the camera starts again from a sharp first frame.
`view.render` and `craft3d-cli render` draw one frame unless `accumulate` (`--accumulate`) asks for
more; they refuse a still whose frames together would take too long (all frames' samples over
2^26, about 34 frames of the default 800 × 600 picture). `ui.screenshot` with `window: true` waits
until the window has averaged its frames, so the picture is the settled one.

```text
ui.set {"look": {"mode": "studio", "dof": {"fstop": 2, "focus": "picked"}, "bloom": 0.1}}
craft3d-cli render part.3dcraft --out part.png --look studio --fstop 4 --accumulate 32
```

In the browser on WebGL2 the second layer's reflections, contact shadows and depth of field are left
out.

## Pointing at things

A click selects what is under the pointer: a corner (vertex) when one is within a few points, else an
edge, else the face.

| Do | To |
|---|---|
| Click | Select a vertex, an edge or a face |
| Shift+click | Add to or take from the selection |
| Click again at the same place, or Alt+click | Select the next face behind |
| Right click | List every face under the pointer, to choose one |
| Double-click in the view | Select the whole body |
| Ctrl+drag | Select what lies inside a box |
| Click empty space, or Escape | Clear the selection |

**The agent sees your selection.** The selection belongs to the document, not to the window, so the agent
reads it: each item with its name, the body, the feature that made it, its shape (a hole's radius and
axis, a face's direction) and the size that drives it. So you can say:

> Make the hole I selected 6.5 mm.

and the agent finds the size behind that hole and changes it. The other way round, the agent can show
you what it means: it can light up a face, an edge or a corner in your view, set the selection you see,
fly the camera to a feature, play the history, and take a picture of the window to look at itself
(`ui.highlight`, `ui.select`, `ui.fit`, `ui.set`, `ui.screenshot` over `control_call`;
[`control-protocol.md`](../control-protocol.md) has the details).

## When something goes wrong

When a command fails, the status bar shows it in red with its message, and a **Copy details** button that
copies a short report for a bug report ([chapter 14](14-troubleshooting.md#reporting-a-problem)). When a
file action you started yourself fails (Open, Save, Import, Export), a dialog says what went wrong:

![An error dialog with OK and Copy details](images/error-dialog.png)

When 3DCraft was not closed properly (a crash, a power cut), the next start shows a bar under the menu
that offers to **Recover** what was autosaved ([chapter 14](14-troubleshooting.md#after-a-crash)).

## Without the window

An agent working headless looks at its work with `doc_render_preview` (the command `view.render`): the
same picture `craft3d-cli render` makes. `view.pick` answers "what is at this pixel of that picture", as a
click does in the window.

**Next:** [Jigs](05-jigs.md).
