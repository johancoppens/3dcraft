# Changelog

All notable changes to 3DCraft are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until 1.0, a minor version may change behaviour; files saved by any released version keep opening in every later one.

## [Unreleased]

### Added

- **Switch features on and off by a condition.** `feature.when` gives a feature a condition such as `lid` or `profile == 2`; a yes/no or choice parameter of a Jig then switches parts of the model on and off. Such a feature is shown greyed as *inactive*.

### Changed

- **Suppressing a feature no longer breaks what comes after it.** A suppressed boolean, fillet, chamfer, shell or pattern passes the body on unchanged, as other modellers do; a boolean whose tool is suppressed keeps its target. Only what the suppressed feature itself made (a fillet's rounded face) is gone, and a feature that used it says so.

- **Files in the browser.** In Chrome, Edge and on Chromebooks, Open, Save As, Import and Export show your system's file dialog, and Save writes back to the file the document came from. Elsewhere files are downloads, and the status bar says where each file went.

## [0.1.0] - 2026-10-23

The first release for people outside the project. 3DCraft is a parametric 3D CAD modeller built for AI: you describe a part, an AI agent builds it through 3DCraft's commands, and you watch it appear in the window, point at faces and check the result. Everything can also be done from the command line or a script.

### Added

- **Drive it with an AI agent.** A built-in MCP server (`craft3d-cli mcp`) that any MCP client can start: headless, or bridged to the open window so you see every step as the agent works. Every action is a command with a name and plain JSON parameters; the same commands run from the command line, a script or the JSON control channel.
- **Parametric modelling.** Variables with units and formulas, and an ordered feature history: change a value and the model rebuilds. Undo and redo, roll back, suppress, edit, rename and delete features; a feature that fails says why and can be repaired.
- **Sketches with a constraint solver.** Lines, arcs, circles, rectangles, splines and patterns, held by dimensions and geometric constraints and solved by 3DCraft's own solver; a sketch that cannot be solved names the constraint at fault.
- **Features.** Blocks, cylinders, cones, spheres and tori; extrude, revolve, sweep and loft from sketches; patterns, mirror and move; shell, draft, thicken and other face operations; threads and helices; datum planes, axes and points; sheet bodies; several bodies in one document.
- **Booleans.** Union, subtract and intersect between bodies of any shape, including curved and freeform faces. When an operation cannot be done exactly it refuses with a clear message instead of producing a broken solid.
- **Fillets and chamfers** on edges, chosen by name or by pointing at them; names stay stable when an earlier feature changes, so a fillet does not jump to another edge.
- **Measuring and design checks.** Volume, area, mass and inertia, distances, clearance, thickness and symmetry; assertions kept in the document ("this wall is at least 2 mm") are checked at every rebuild. Selection queries and named sets.
- **Colours and materials** per body and per face, shown in the window and kept in exports.
- **Files.** The native `.3dcraft` format, with a promise: every file a released version saves opens in every later version. Import STEP; export STEP (the exact geometry), STL, 3MF and OBJ (meshes, with colours), and DXF and SVG (flat outlines of a sketch or a face, for laser cutting and drawings).
- **Autosave and crash recovery.** Unsaved work is saved in the background; after a crash the window offers to recover it. Closing the window asks before unsaved changes are lost.
- **The window.** A viewer with a view cube, a feature tree, tabs for open documents, clicking to point at faces, edges and corners, see-through bodies and a section plane, and Open, Save, Import and Export in the File menu. It draws with your graphics card, or on the processor when there is none.
- **Copy details.** When a command fails, the status bar offers *Copy details*: a short report (version, platform, graphics, the command and the error) to paste into a bug report. An agent gets the same report from the `session.report` command.
- **Installers.** Linux (AppImage, and a tarball with an install script), macOS (a `.dmg` for Apple Silicon) and Windows (a `.msi` installer, and a zip). They are not signed yet: the user guide explains the warning your system shows and how to get past it.
- **3DCraft in the browser.** The same program as a web page (WebGPU, or WebGL2), with nothing to install; files stay in the page's memory.
- **A user guide** (`docs/guide/`) written for working with an AI agent, from installing and connecting an agent to modelling, measuring, files and troubleshooting, with nine example models that check their own volume.

[Unreleased]: https://github.com/johancoppens/3dcraft/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/johancoppens/3dcraft/releases/tag/v0.1.0
