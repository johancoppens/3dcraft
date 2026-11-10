# Your first five minutes

*Open an example, look around, watch how it was built, try a Jig and export a part for a 3D printer.
No AI agent needed yet.*

You need 3DCraft installed, or the web version open in your browser ([Install 3DCraft and connect an AI agent](install.md)).

## 1. Open an example

Start 3DCraft. Without a document, the window shows its **start screen**:

![The start screen](images/start-screen.png)

Click the **L-bracket** card. 3DCraft builds the bracket in a new tab, from the same command script an
agent would send, and shows it:

![The window with the L-bracket; the numbers are explained in The window](images/window.png)

On the left is the **feature list**: the steps that made the bracket, in order (a sketch of the L, the
extrusion, the rounded corner, the holes), and under it the **assertions**, the checks the bracket must
pass (its volume, two holes, holes that go through). A ✔ means it is fine.

## 2. Look around

| Do this | To |
|---|---|
| Drag with the left mouse button | Turn the model (orbit) |
| Drag with the right button (or Shift and the left) | Move it (pan) |
| Turn the wheel, or pinch on a touchpad | Zoom in and out, about the point under the pointer |
| Click a side or corner of the **view cube** (top right) | Look from the front, the top, a corner, … |
| Press **F**, or click **Fit** | See the whole model again |

Now click the **Rounded** row in the feature list: the round in the inside corner lights up.
**Double-click** it (or click the **⌖** that appears on the row under the pointer) and the camera flies
to it. Press **F** to see everything again.

> **Tip:** the **eye** at the end of a row hides that feature's body or sketch in the view; click it again
> to show it. The model itself does not change.

## 3. Watch how it was built

Under the feature list is the **history player**, like a video player:

![The history player: the timeline, first step, step back, play, step forward, to end, speed, loop and orbit](images/player.png)

1. Click **⏮** (first step): the view shows the model after the first step, just the sketch.
2. Press **Space** (or click ▶): the bracket is built again, step by step, with the name of each step at
   the top of the view.
3. Click the **orbit** button (the last one) and play again: the camera turns slowly while it plays.
4. Click **⏭** (to end) to see the finished part.

Playing back never changes the document; it only shows it as it was.

## 4. Export it for a 3D printer

1. Open the **File** menu and choose **Export…** (or press Ctrl+E; ⌘E on a Mac).
2. In the export dialog, keep **The model** and choose the format **STL** or **3MF** (3MF keeps the
   size unit and colours; most slicers read both). The **quality**: **Normal** is right for printing.
   The model is what you see: a body you hid (the eye in the feature list) is left out, and the dialog
   says which under **The model**, as the status line does after the export.

   ![The export dialog for a mesh: Coarse, Normal or Fine](images/export-mesh.png)

3. Click **Export…**, choose where in your system's save dialog and click Save.

Open the file in your slicer (Cura, PrusaSlicer, Bambu Studio, …). The bracket is in millimetres, 40 mm
long, as designed.

> **Other formats:** `.step` for another CAD program, `.dxf` or `.svg` for a laser cutter or a drawing
> ([Files, export and drawings](files.md)).

## 5. Try a Jig

A **Jig** is a design you change by setting its sizes in a form. Get one from the
[Jigs collection](https://github.com/johancoppens/3dcraft-jigs): in its `dist` folder, download
`laser-box.3dcraftjig` (a laser-cut box), `gt2-pulley.3dcraftjig` (a timing pulley) or
`welding-table.3dcraftjig` (a welding table), and open it with **File › Open…** (or double-click the
file once 3DCraft is installed).

The Jig opens on its **Parameters** tab:

![The laser-cut box Jig on its Parameters tab](images/jig-parameters.png)

1. Drag across the **Width** field: the box follows as you drag.
2. Click the **Height** field's value, press Enter, type `60 mm` and press Enter again.
3. Click **↺** next to a field to put it back, or **Reset all**.
4. Export the result (**File › Export…**). For the box, pick **Sheet of set: Cut sheet** and the
   format SVG or DXF: all six panels on one sheet, at true size, ready for the laser cutter.

[Using a Jig](jigs.md) tells you everything about Jigs.

## 6. Save

**File › Save** (Ctrl+S) saves the document as a `.3dcraft` file. A Jig is never overwritten: saving it
saves your result as a new `.3dcraft` model. If you forget to save, 3DCraft keeps an autosave and offers
to recover it after a crash ([Troubleshooting](troubleshooting.md)).

## What next?

- **Design something new with an AI agent:** connect one ([Install 3DCraft and connect an AI agent](install.md#connect-an-ai-agent-over-mcp))
  and read [Working with an AI agent](agent.md).
- **Get to know the window:** [The window](window.md).
- **The other examples:** each card on the start screen is one; the [example models](../examples/README.md)
  page shows them all with what they teach.
