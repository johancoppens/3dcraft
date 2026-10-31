# Changelog

All notable changes to 3DCraft are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until 1.0, a minor version may change behaviour; files saved by any released version keep opening in every later one.

## [Unreleased]

## [0.1.0] - unreleased

The first release for people outside the project. 3DCraft is a parametric 3D CAD modeller built for AI: you describe a part, an AI agent builds it through 3DCraft's commands, and you watch it appear in the window, point at faces and check the result. Ready-made designs called Jigs need no AI at all: set their sizes in a form and export. Everything can also be done from the command line or a script, and 3DCraft runs in the browser too.

Every item below is in `main` as of 2026-10-30. Branches still merging for this release add their lines here when they land (the release checklist in `docs/development.md`).

### Modelling

- **A history that rebuilds.** Variables with units and formulas, and an ordered list of features: change a value and the model rebuilds, only from the feature that changed. Undo and redo, roll back to any step, suppress, edit, move, rename and delete features; insert a feature in the middle and the ones after it follow. A feature that fails says why (often with the largest size that fits) and can be repaired; features made from it name it as the cause.
- **Features.** Blocks, cylinders, cones, spheres and tori; extrude, revolve, sweep (along a path or a helix) and loft from sketches; patterns, mirror and move; shell, draft, offset, move and delete faces, thicken; ISO metric threads (cosmetic or modelled); datum planes, axes, points and helices; sheet bodies; several bodies in one document.
- **Fillets and chamfers** on edges, chosen by name, by description or by pointing at them.
- **Names that stay put.** Faces and edges are named by how they were made, so a fillet does not jump to another edge when an earlier feature changes, and a face that is gone is reported instead of silently replaced.
- **Units.** Millimetres, inches and the other length units per document, angles in degrees or radians; the length unit can be changed later while keeping the sizes. Mass, time, force, pressure and stress, moment and energy in expressions (`1000 N / (100 * (1 mm)^2)` is 10 MPa), so a design rule can say `"max": "235 MPa"`; an output can carry a label and the unit it is shown in. Named sets can pick flat faces by their plate thickness (`thickness`).
- **Colours and materials** per body and per face, with metallic and roughness, following the face through later edits and kept in 3MF, OBJ and STEP exports.

### Sketches

- **Sketches with a constraint solver.** Lines, arcs, circles, rectangles, paths, splines (also through computed points) and patterns, held by dimensions and geometric constraints and solved by 3DCraft's own solver; coordinates can be formulas.
- **Clear diagnostics.** A sketch that cannot be solved names the constraint at fault and the ones it contradicts; a line squeezed to nothing and a sketch with freedom left are reported.
- **Curves from formulas** (`sketch.curve`): `x(t)` and `y(t)` in the variables, fitted again whenever they change, as in the involute gear example; points on them and tangents to them can be constrained.
- Sketches on a face, with the x axis turned to a direction you choose.

### Text

- **Text in sketches** (`sketch.text`), straight or along a curve (`sketch.textOnCurve`): a template that can hold formulas, cap height or em size, anchor, alignment, rotation, line spacing and wrapping to a width, as closed profiles of exact curves with kerning, ready to extrude, emboss or engrave. Letters are named by glyph, so a feature on one fails by name instead of jumping to another.
- **Fonts.** Noto Sans, JetBrains Mono and a single-stroke font drawn for 3DCraft are built in and carried by the installers (fetched when first needed on the web); `font.add` keeps another font in the document when its licence allows it. The measure `textExtent` gives a text's size.

### Booleans

- **Union, subtract and intersect** between bodies of any shape, including curved and freeform faces, and cuts by sheets and between sheets.
- When an operation cannot be done exactly it refuses with a clear message instead of producing a broken solid.

### Sheets, plates, sheet metal and origami

- **Plates.** Give faces (flat, developable or freeform) a thickness, inside, outside or in the middle, with square, bevel, mitre, touch and over/under joints between neighbouring plates.
- **Developable surfaces and unfolding.** A developable surface between two curves, unfolded flat at true size with a map between the pattern and the part, and a measure of how developable a face is. Rails with corners (a fan from the corner, a crease where both rails turn) and closed rails (a tube), such as a sea kayak's sheer panels.
- **Sheet metal.** A base with flanges, bent by inner radius and K-factor, with rectangular, round and tear corner reliefs; corners closed by overlap, butt or mitre; holes and cut-outs on any flat; a bend table; checks for bend radius, flange length, hole-to-bend distance and relief; the flat pattern exported with its bend lines. Holes across a bend, and a later through cut carried back onto the flat pattern. Flanges inset on an edge (several tabs), on a flange's side, on a hole's edge or with an outline of their own, with bend reliefs at their ends. A press-brake check (`sheetMetal.bendSequence`) that searches an order of bends and proves the punch and die clear the part.
- **Origami.** A crease pattern of mountains, valleys and cuts folded by angles that follow variables, or folded flat with the order of the layers worked out; Kawasaki and Maekawa checks; the crease pattern exported to SVG and DXF. Thick panels on a membrane, slits and slots, a flat-folded stack folded on, and patterns of thousands of creases.

### Freeform surfacing

- **Analysis.** Zebra stripes, highlight lines, a curvature map and a draft check drawn on the exact surfaces, curvature combs on edges and iso-curves, and the measures `draft` and `curvature` at a point (the Analysis toggle in the window).
- **Surfaces from curve networks.** Gordon surfaces through crossing u and v curves (`feature.surface.network`), Coons patches (`feature.surface.coons`) and lofts with guide curves, as exact B-splines.
- **Continuity across seams.** An edge held in position, tangent (G1) or in curvature (G2) to a plane, cylinder, cone, sphere or a spline face, and `measure.continuity`, which proves G0, G1 or G2 with bounded gaps (also as an assertion). A curvature seam spreads its change over the sheet with the least variation of curvature, so the surface stays fair beside the seam.

### Assemblies and motion

- **Assemblies** as their own documents (`.3dcraftasm`): instances of other `.3dcraft` files by reference (the assembly follows the part when you edit it) or packed into one file, configurations by a part's public parameters, one body picked from a multi-body part, and rigid or flexible subassemblies.
- **Mates, joints and couplings**, solved by 3DCraft's own solver: fixed, revolute and the other mates between named faces of instances (a face that is gone fails its mate by name), free joints set by value or dragged, gears, racks and screws coupling two mates, and linear couplings of any number of motions (a planetary stage). A joint's solved value and an instance's pose as measures, usable in checks and formulas (`measure.joint`, `measure.pose`). A part drawn in another length unit keeps its size. Clearance, overlap and mass per instance; STEP export with the assembly's structure.
- **Motion over a timeline `t`.** Play a mechanism in the window, see collisions in red, draw the trace of a point (and export it to SVG or DXF), and check any assertion over the whole motion (`motion.check`). Dead points are reported with their `t`.

### Measuring and design checks

- **Measures.** Volume, area, mass and inertia, distances, clearance, wall thickness, holes that go through, rays and symmetry; what lies below a plane, the section by a plane and its moments, and the level that holds a given volume. The floating position (level and tilt) of a hull for a given weight and centre of gravity, with its stability. A section's torsion constant, shear centre and warping constant, for any section with holes.
- **Assertions** kept in the document ("this wall is at least 2 mm"), checked at every rebuild and shown in the window, optionally only when a condition holds; read-only **outputs** computed from measures and formulas.
- **Selections.** Queries ("every 8 mm hole"), a selection shared between you and the agent, and named sets.

### Formulas and Jigs

- **Formulas that decide.** Comparisons, `and`, `or` and `not`, `if`, `choose` and `clamp`, and the usual functions in any expression; tables of values looked up by row or key (`belt["GT2"].pitch`); features switched on and off by a condition, so a yes-or-no parameter adds or removes a lid.
- **Jigs: designs anyone can set without AI.** Publish the variables that drive a design as parameters (number, whole number, yes or no, a choice; with a label, group, range and step) and export it as a `.3dcraftjig` (File › Export as Jig…). Opening a Jig shows its **Parameters** tab: drag or type values and the model follows; values are kept in range and the form says so; a value that breaks the model is marked, with *Back to last good*. A Jig is never overwritten: Save saves the result as a model, and *Open for editing* makes an editable copy.
- `craft3d-cli check-range` proves a Jig over its whole parameter range. Three Jigs to try (a laser-cut box, a welding table and a GT2 pulley) are in the separate 3DCraft Jigs collection.

### Files and export

- **The native `.3dcraft` format**, with a promise: every file a released version saves opens in every later version and rebuilds to the same bodies.
- **Import** STEP: solids, sheets and assemblies, with colours.
- **Export** STEP (the exact geometry), STL, 3MF and OBJ (meshes at Coarse, Normal or Fine quality, with colours), and DXF and SVG: the flat outline of a sketch or of faces at true size, a whole cut sheet of a named set laid out to your laser's width, flat and crease patterns, or a drawing of the model as seen (orthographic to scale or perspective, hidden lines left out or dashed), its edges written as the lines, arcs, circles, ellipses and splines they are.
- An export dialog that first asks what goes into the file and offers only the formats that hold it.

### The window and installing

- **The window.** A start screen with the example models as cards, recent files and how to connect an agent; a tab per document with its own view; a feature list with an eye per feature and a double click (or F) that flies the camera to it; a history player like a video player; a view cube, see-through bodies and a section plane; File, Edit (Undo, Redo) and Help menus with keyboard shortcuts (F1). It draws with your graphics card, or on the processor when there is none. The interface is set in Inter, with JetBrains Mono for code.
- **Autosave and crash recovery.** Unsaved work is saved in the background; after a crash the window offers to recover it. Closing the window or a tab asks before unsaved changes are lost.
- **Clear errors and bug reports.** A failed file action shows a dialog with what went wrong; after any failed command the status bar offers *Copy details*, a short report to paste into a bug report.
- **Installers** for Linux (AppImage, and a tarball with an install script), macOS (a `.dmg` for Apple Silicon) and Windows (a `.msi` installer, and a zip), with the `.3dcraft` and `.3dcraftjig` file types.
- **A user guide** for first-time users: three ways to use 3DCraft, your first five minutes, working with an AI agent, the window, Jigs, formulas, features, measuring, files and troubleshooting, with pictures, origami and assemblies, a reference, and ten example models (from an L-bracket to an involute gear) that check their own volume.

### Rendering

- **A studio look**: image-based lighting from three bundled environments, soft shadows, ambient occlusion, a ground that catches shadows and reflections, contact shadows, depth of field and bloom, refined while the camera stands still; a view setting that never changes the document.
- **Materials** kept in the document: anisotropic metal, clear coat paint, glass with refraction and subsurface wrap, with six presets (`material.presets`); exports carry what their formats hold.
- Both looks are drawn by the graphics card and by the processor alike, the processor's picture being the reference the graphics card is tested against. Pictures without a window (`view.render`, `view.replay`, `craft3d-cli render`) use the graphics card when there is one (`renderer: auto|cpu|gpu`).
- **Animations** (`view.replay`): variables moved along tracks with easing, the motion timeline, a camera path with orbit and dolly, the look, frames per second and duration, written as PNG frames or, through an installed ffmpeg, as MP4, WebM or GIF.

### Web

- **3DCraft in the browser**: the same program as a web page (WebGPU, or WebGL2, see-through and sections included), with nothing to install. Open, save, import and export with your system's file dialogs in Chrome, Edge and on Chromebooks, as downloads elsewhere; use Jigs (a Jig on the same site opens with `?open=`); build the examples. Design checks are measured in the background so the page stays responsive.

### AI agents and the command line

- **Drive it with an AI agent.** A built-in MCP server (`craft3d-cli mcp`) that any MCP client can start: headless, or bridged to the open window so you see every step as the agent works. The start screen shows the configuration for your computer, with *Copy configuration* and *Copy for Claude Code*.
- **Everything is a command**, with a name and plain JSON parameters; the same commands run from the command line, a script or the JSON control channel, and the agent can light up faces, fly the camera, play the history, take pictures of the window, record it as a video (`ui.record`) and get the bug report (`session.report`).
- **The command line.** `craft3d-cli` runs commands and scripts, prints a document's state, renders pictures, converts files, draws a model as seen to DXF or SVG, makes a video of how a design is built, scores a design against a task, proves a Jig over its range, and serves MCP and a JSON-lines protocol.

### Known limitations

- The installers are not signed, so Windows and macOS warn the first time (the guide shows how to open them). macOS is Apple Silicon only, and double-clicking a `.3dcraft` file there starts 3DCraft without opening the file (use File › Open).
- An AI agent cannot drive the web version yet. The web version rebuilds the model beside the page, but a click that selects, measures and exports still compute on the page, so they can take a moment on a big model.
- Some modelling cases are refused rather than built (the message says which): among them some booleans and freeform fillets, curved and sharp sheet-metal bends, curved origami creases, a sweep along two rails, and text wrapped onto a curved surface. Text is laid out glyph by glyph with kerning; shaping for complex scripts (such as Arabic or the Indic scripts) is for a later version. Every refusal reported helps.

[Unreleased]: https://github.com/johancoppens/3dcraft/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/johancoppens/3dcraft/releases/tag/v0.1.0
