# Changelog

All notable changes to 3DCraft are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until 1.0, a minor version may change behaviour; files saved by any released version keep opening in every later one.

## [Unreleased]

## [0.1.1] - 2026-11-12

A fix for the Windows packages. On Linux, macOS and the web, 0.1.1 is 0.1.0 with a new version number.

### Fixed

- **The Windows programs no longer need Microsoft's Visual C++ runtime.** In 0.1.0 both of them, the window and `craft3d-cli`, needed it, and many computers do not have it: on such a PC 3DCraft stopped at once with *"The code execution cannot proceed because VCRUNTIME140.dll was not found"*. They now carry that runtime inside them. If you have 0.1.0 on Windows, install 0.1.1 over it: the new `.msi` removes 0.1.0 by itself and installs 0.1.1 in its place, and the new zip unpacks into a folder of its own, named with the new version, from which you start `craft3d.exe` (the guide's *Installing a newer version*).

### Known limitations

- Windows: a console window opens beside 3DCraft's window.
- The known limitations of 0.1.0 still hold.

## [0.1.0] - 2026-11-11

The first release for people outside the project. 3DCraft is a parametric 3D CAD modeller built with and for AI: you describe a part, an AI agent builds it through 3DCraft's commands, and you watch it appear in the window, point at faces and check the result. Ready-made designs called Jigs need no AI at all: set their sizes in a form and export. Everything can also be done from the command line or a script, and 3DCraft runs in the browser too.

### Modelling

- **A history that rebuilds.** Variables with units and formulas, and an ordered list of features: change a value and the model rebuilds, only from the feature that changed. Undo and redo, roll back to any step, suppress, edit, move, rename and delete features; insert a feature in the middle and the ones after it follow. A feature that fails says why (often with the largest size that fits) and can be repaired; features made from it name it as the cause.
- **Features.** Blocks, cylinders, cones, spheres and tori; extrude, revolve, sweep (along a path, a helix, or two rails that carry and scale the profile, also closed into a ring) and loft from sketches; patterns, mirror and move; shell, draft, offset, move and delete faces, thicken; ISO metric threads (cosmetic or modelled); datum planes, axes, points and helices; sheet bodies; several bodies in one document.
- **Fillets and chamfers** on edges, chosen by name, by description or by pointing at them.
- **Names that stay put.** Faces and edges are named by how they were made, so a fillet does not jump to another edge when an earlier feature changes, and a face that is gone is reported instead of silently replaced.
- **Units.** Millimetres, inches and the other length units per document, angles in degrees or radians; the length unit can be changed later while keeping the sizes. Mass, time, force, pressure and stress, moment and energy in expressions (`1000 N / (100 * (1 mm)^2)` is 10 MPa), so a design rule can say `"max": "235 MPa"`; an output can carry a label and the unit it is shown in. Named sets can pick flat faces by their plate thickness (`thickness`). Litres (`L`, `mL`) for volumes. A document's tolerance can be set (`document.setTolerance`).
- **Colours and materials** per body and per face, with metallic and roughness, following the face through later edits and kept in 3MF, OBJ and STEP exports. A face's own colour wins over its body's; the button on a body's card that clears its face colours (`face.appearance` with `clear`) takes them all away in one step.

### Sketches

- **Sketches with a constraint solver.** Lines, arcs, circles, rectangles, paths, splines (also through computed points, and closed splines that stay smooth where they close) and patterns, held by dimensions and geometric constraints and solved by 3DCraft's own solver; coordinates can be formulas.
- **Clear diagnostics.** A sketch that cannot be solved names the constraint at fault and the ones it contradicts; a line squeezed to nothing and a sketch with freedom left are reported.
- **Curves from formulas** (`sketch.curve`): `x(t)` and `y(t)` in the variables, fitted again whenever they change, as in the involute gear example; points on them and tangents to them can be constrained.
- Sketches on a face, with the x axis turned to a direction you choose.

### Text

- **Text in sketches** (`sketch.text`), straight or along a curve (`sketch.textOnCurve`): a template that can hold formulas, cap height or em size, anchor, alignment, rotation, line spacing and wrapping to a width, as closed profiles of exact curves with kerning, ready to extrude, emboss or engrave. Text and any sketch wrap onto a cylinder or a cone (`feature.extrude` with `wrap`), embossed or engraved along the surface at true size. Letters are named by glyph, so a feature on one fails by name instead of jumping to another.
- **Fonts.** Noto Sans, JetBrains Mono and a single-stroke font drawn for 3DCraft are built in and carried by the installers (fetched when first needed on the web); `font.add` keeps another font in the document when its licence allows it. The measure `textExtent` gives a text's size.

### Booleans

- **Union, subtract and intersect** between bodies of any shape, including curved and freeform faces, and cuts by sheets and between sheets.
- **Sew** (`feature.boolean` with `op: sew`): sheets that meet along their edges, even tangent or in curvature, joined into one sheet or a closed solid without cutting them.
- When an operation cannot be done exactly it refuses with a clear message instead of producing a broken solid.

### Sheets, plates, sheet metal and origami

- **Plates.** Give faces (flat, developable or freeform) a thickness, inside, outside or in the middle, with square, bevel, mitre, touch and over/under joints between neighbouring plates.
- **Developable surfaces and unfolding.** A developable surface between two curves, unfolded flat at true size with a map between the pattern and the part, and a measure of how developable a face is. Rails with corners (a fan from the corner, a crease where both rails turn) and closed rails (a tube), such as a sea kayak's sheer panels.
- **Sheet metal.** A base with flanges, bent by inner radius and K-factor, with rectangular, round and tear corner reliefs; corners closed by overlap, butt or mitre; holes and cut-outs on any flat; a bend table; checks for bend radius, flange length, hole-to-bend distance and relief; the flat pattern exported with its bend lines. Holes across a bend, and a later through cut carried back onto the flat pattern. Flanges inset on an edge (several tabs), on a flange's side, on a hole's edge or with an outline of their own, with bend reliefs at their ends. A press-brake check (`sheetMetal.bendSequence`) that searches an order of bends and proves the punch and die clear the part, or checks an order you give, also for parts with holes across their bends. Louvres and tabs: a flange on the edge of a cut-out in any flat. The measures `flatExtent` and `flatArea` for a flat pattern's size. Sharp bends (an inner radius of 0); curved flanges along arcs and circles of the base (a round pan, a tray with rounded corners), with the stretch of their approximated flat pattern proven (`bendStrain`); jogs that step a flat; formed louvres pressed by a tool (`louvreStretch`); and hems (flanges bent 180 degrees), which the press-brake check presses flat in two strokes.
- **Cutting.** Plates and sheet-metal parts recognised for cutting: a cut list grouped by thickness and material, identical parts counted and sizes given to the document's tolerance (`cut.list`, JSON or CSV), and every part's outline to DXF or SVG, per part or on one sheet, with a kerf (`cut.export`).
- **Origami.** A crease pattern of mountains, valleys and cuts folded by angles that follow variables, or folded flat with the order of the layers worked out; Kawasaki and Maekawa checks; the crease pattern exported to SVG and DXF. Thick panels on a membrane, slits and slots, a flat-folded stack folded on, and patterns of thousands of creases.

### Freeform surfacing

- **Analysis.** Zebra stripes, highlight lines, a curvature map and a draft check drawn on the exact surfaces, curvature combs on edges and iso-curves, and the measures `draft` and `curvature` at a point (the Analysis toggle in the window).
- **Surfaces from curve networks.** Gordon surfaces through crossing u and v curves (`feature.surface.network`), Coons patches (`feature.surface.coons`) and lofts with guide curves, as exact B-splines.
- **Continuity across seams.** An edge held in position, tangent (G1) or in curvature (G2) to a plane, cylinder, cone, sphere or a spline face, and `measure.continuity`, which proves G0, G1 or G2 with bounded gaps (also as an assertion). A curvature seam spreads its change over the sheet with the least variation of curvature, so the surface stays fair beside the seam.
- **Trimmed faces and N-sided fills.** A face cut out of an existing surface by curves on it, drawn on it or projected onto it, such as an opening or a label area (`feature.surface.trim`), and a loop of 3 to 6 curves filled with a smooth sheet that meets its neighbours tangent or in curvature (`feature.surface.nsided`).
- Zebra stripes and highlight lines stay clean where they are finer than a pixel: they fade to grey instead of breaking into moiré.

### Assemblies and motion

- **Assemblies** as their own documents (`.3dcraftasm`): instances of other `.3dcraft` files by reference (the assembly follows the part when you edit it) or packed into one file, configurations by a part's public parameters, one body picked from a multi-body part, and rigid or flexible subassemblies. A part whose feature fails marks its instance with the reason and fails the assembly's check.
- **Mates, joints and couplings**, solved by 3DCraft's own solver: fixed, revolute and the other mates between named faces of instances (a face that is gone fails its mate by name), free joints set by value or dragged, gears, racks and screws coupling two mates (a screw can also tie a cylindrical mate's slide to its own turn, as a thread), and linear couplings of any number of motions (a planetary stage). A joint's solved value and an instance's pose as measures, usable in checks and formulas (`measure.joint`, `measure.pose`). A part drawn in another length unit keeps its size. Measures and checks name an instance's bodies by path (`Instance/Body`, also into subassemblies); clearance, overlap and mass per instance; STEP export with the assembly's structure.
- **Motion over a timeline `t`.** Play a mechanism in the window, see collisions in red, draw the trace of a point (and export it to SVG or DXF), and check any assertion over the whole motion (`motion.check`). Dead points are reported with their `t`.

### Measuring and design checks

- **Measures.** Volume, area, mass and inertia, distances, clearance, wall thickness, holes that go through, rays and symmetry; what lies below a plane, the section by a plane and its moments, and the level that holds a given volume. The floating position (level and tilt) of a hull for a given weight and centre of gravity, with its stability. A section's torsion constant, shear centre and warping constant, for any section with holes.
- **Assertions** kept in the document ("this wall is at least 2 mm"), checked at every rebuild and shown in the window, optionally only when a condition holds. A bound is met within the document's tolerance (an angle within its angular tolerance), and a bound finer than that is warned about; the line over the feature list counts the checks that pass, fail or are not proven. Read-only **outputs** computed from measures and formulas.
- **Selections.** Queries ("every 8 mm hole"), a selection shared between you and the agent, and named sets.

### Formulas and Jigs

- **Formulas that decide.** Comparisons, `and`, `or` and `not`, `if`, `choose` and `clamp`, and the usual functions in any expression; tables of values looked up by row or key (`belt["GT2"].pitch`); features switched on and off by a condition, so a yes-or-no parameter adds or removes a lid. A part switched off is left out of the exports, cut lists and measures that name it, so one list serves every option, and a check on it alone is inactive.
- **Jigs: designs anyone can set without AI.** Publish the variables that drive a design as parameters (number, whole number, yes or no, a choice; with a label, group, range and step) and export it as a `.3dcraftjig` (File › Export as Jig…). Opening a Jig shows its **Parameters** tab: drag or type values and the model follows; values are kept in range and the form says so; a value that breaks the model is marked, with *Back to last good*. A Jig is never overwritten: Save saves the result as a model, and *Open for editing* makes an editable copy.
- `craft3d-cli check-range` proves a Jig over its whole parameter range. The separate 3DCraft Jigs collection has Jigs to try, from a laser-cut box and a GT2 pulley to a planetary gearbox and a sundial for your place.

### Files and export

- **The native `.3dcraft` format**, with a promise: every file a released version saves opens in every later version and rebuilds to the same bodies.
- **Import** STEP: solids, sheets and assemblies, with colours; FreeCAD documents (`.FCStd`) and BRep files: the visible final objects' solids, with their names, placements and colours.
- **Export** STEP (the exact geometry), STL, 3MF and OBJ (meshes at Coarse, Normal or Fine quality, with colours), and DXF and SVG: the flat outline of a sketch or of faces at true size, a whole cut sheet of a named set laid out to your laser's width, flat and crease patterns, or a drawing of the model as seen (orthographic to scale or perspective, hidden lines left out or dashed), its edges written as the lines, arcs, circles, ellipses and splines they are.
- An export dialog that first asks what goes into the file and offers only the formats that hold it. Bodies hidden in the window are left out of a 3D export of the model and of the cut list, and the dialog says which.
- Saving from the window never replaces a file you were not asked about, and an assembly is saved as `.3dcraftasm`, on the desktop and in the browser.

### The window and installing

- **The window.** A tab per document with its own view; a feature list with an eye per feature and a double click (or F) that flies the camera to it; a history player like a video player; a view cube, see-through bodies and a section plane; File, Edit, View and Help menus with keyboard shortcuts (F1), and light or dark panels; a status bar that says what was done. It draws with your graphics card, or on the processor when there is none. The interface is set in Inter, with JetBrains Mono for code.
- **A start screen** with New part, New assembly and Open, your recent files, the panel that connects an AI agent, and twenty examples built in, so they open offline and in the browser, under three tabs: ten finished parts (a steel bookend and a T-slot profile marked *Start here*, then a moka pot, a ship propeller, a snap-fit enclosure, a hydraulic cylinder, a spiral staircase, a solitaire ring, a flat-pack bookcase and a mailer box), six Jigs that open on their form (a laser-cut box, a Gridfinity bin, a dovetail drawer, a sheet-metal enclosure, a planetary gearbox and a sundial for your place) and four assemblies that open with their player, ready to play (a toggle clamp, a Jansen walker, a scissor jack and a radial engine). An example opens in a tab named after it and counts as unchanged until you edit it; Save asks where. *More examples* opens the tab's part of the galleries on the website. The examples are MIT, the Jigs and assemblies CC BY 4.0 (`EXAMPLES-LICENSE.md`).
- **Check for updates**: Help › Check for updates…, a link beside the version on the start screen, and the About dialog open the website in your browser, which says whether there is a newer version; 3DCraft itself connects nowhere. The AppImage carries update information, so a tool such as AppImageLauncher can update it. The guide says how to install a newer version of each package.
- **Autosave and crash recovery.** Unsaved work is saved in the background, and the status bar says when for the document in front; after a crash the window offers to recover it. Closing the window or a tab asks before unsaved changes are lost, naming the document.
- **Clear errors and bug reports.** A failed file action shows a dialog with what went wrong; after any failed command the status bar offers *Copy details*, a short report to paste into a bug report.
- **Installers** for Linux (an AppImage, and a tarball whose `install.sh` installs 3DCraft into `~/.local` and whose `uninstall.sh` removes exactly what it installed), macOS (a `.dmg` for Apple Silicon) and Windows (a `.msi` installer, and a zip), each with the fonts 3DCraft draws with. The `.msi`, the tarball and the macOS app register the `.3dcraft`, `.3dcraftjig` and `.3dcraftasm` file types; the AppImage brings its menu entry and file types through a tool such as AppImageLauncher.
- **A user guide** for first-time users, in 22 chapters and six parts, with pictures of this version's window: installing (and installing a newer version) and connecting an agent, your first five minutes, working with an AI agent with five tasks to try, the window, Jigs, assemblies and files; how 3DCraft thinks, a first part, formulas, features, sheet metal, surfaces, origami and selection; measuring and engineering checks; making Jigs, the command line and programs on 3DCraft; troubleshooting and a reference. Seven lessons, from an L-bracket to an involute gear, check their own volume. Help › User guide opens the guide of your version on 3dcraft.dev.

### Rendering

- **A studio look**: image-based lighting from three bundled environments, soft shadows, ambient occlusion, a ground that catches shadows and reflections, contact shadows, depth of field and bloom, refined while the camera stands still; a view setting that never changes the document.
- **Materials** kept in the document: anisotropic metal, clear coat paint, glass with refraction and subsurface wrap, with six presets (`material.presets`); solid textures (wood, marble, granite, speckle, noise, brushed metal, carbon) evaluated in the model's space; exports carry what their formats hold.
- Both looks are drawn by the graphics card and by the processor alike, the processor's picture being the reference the graphics card is tested against. Pictures without a window (`view.render`, `view.replay`, `craft3d-cli render`) use the graphics card when there is one (`renderer: auto|cpu|gpu`).
- **Animations** (`view.replay`): variables moved along tracks with easing, the motion timeline, a camera path with orbit and dolly, the look, frames per second and duration, written as PNG frames or, through an installed ffmpeg, as MP4, WebM or GIF. The window records itself too (`ui.record`), choosing an encoder your ffmpeg actually has.
- **The same model on every computer.** 3DCraft computes its sines, cosines and other functions itself instead of using the system's maths library, so a document rebuilds to the same bodies whatever the platform.

### Web

- **3DCraft in the browser**: the same program as a web page (WebGPU, or WebGL2, see-through and sections included), with nothing to install. Open, save, import and export with your system's file dialogs in Chrome, Edge and on Chromebooks, as downloads elsewhere; use Jigs (a Jig on the same site opens with `?open=`), play assemblies and open the start screen's examples, which are built in. The model is rebuilt, measured and exported in the background, so the page stays responsive.

### AI agents and the command line

- **Drive it with an AI agent.** A built-in MCP server (`craft3d-cli mcp`) that any MCP client can start: headless, or bridged to the open window so you see every step as the agent works. The **Connect an AI agent** panel on the start screen gives the command for Claude Code and the configuration for Claude Desktop, Cursor, VS Code or any other MCP client, ready to copy.
- **Allow AI agents.** One switch, on the start screen and behind the agents pill in the status bar, lets agents connect to the window with no flags and nothing secret to copy. They read and write files in a work folder you choose; the window asks you before an agent reaches any other folder, and never offers your home folder itself or the folders of keys and settings. The **Agent** tab of the side panel shows what agents did to the document, newest first, and a feature an agent made carries a mark (✦); `agent.log` gives an agent the same.
- **Everything is a command**, with a name and plain JSON parameters; the same commands run from the command line, a script or the JSON control channel, and the agent can light up faces, fly the camera, play the history, take pictures of the window, record it as a video (`ui.record`) and get the bug report (`session.report`).
- **The command line.** `craft3d-cli` runs commands and scripts, prints a document's state, renders pictures, converts files, draws a model as seen to DXF or SVG, makes a video of how a design is built, scores a design against a task, proves a Jig over its range, and serves MCP and a JSON-lines protocol.

### Known limitations

- Tested on Linux. The macOS and Windows packages are new and less tested: each is built and started on its system, but the full test suite has not passed on those systems yet.
- The installers are not signed, so Windows and macOS warn the first time (the guide shows how to open them).
- macOS: Apple Silicon only (M1 or later), and double-clicking a `.3dcraft` file there starts 3DCraft without opening the file (use File › Open).
- Windows: `craft3d.exe` and `craft3d-cli.exe` show a generic icon in Explorer; the Start menu entry, the file types and the window have 3DCraft's.
- Linux: the packages need glibc 2.35 or newer, as on Ubuntu 22.04 and later.
- 3DCraft does not update itself: Check for updates opens the website, and you install a newer version as the guide says.
- The window has no toolbar of modelling buttons: you model through commands, sent by an AI agent, a Jig's form or a script of your own, and the window is for watching, pointing, checking and exporting.
- An AI agent cannot drive the web version yet.
- Some modelling cases are refused rather than built (the message says which): among them some booleans and freeform fillets, curved sheet-metal bends along a spline and some sharp ones, curved origami creases, and text wrapped onto a surface other than a cylinder or a cone. Text is laid out glyph by glyph with kerning; shaping for complex scripts (such as Arabic or the Indic scripts) is for a later version. Every refusal reported helps.

[Unreleased]: https://github.com/johancoppens/3dcraft/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/johancoppens/3dcraft/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/johancoppens/3dcraft/releases/tag/v0.1.0
