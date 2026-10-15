# Development guide

> **Last updated:** 2026-10-12 · **Status:** M3 done. *Works today:* everything under [What works today](#what-works-today). Every other section is *(planned)*: it describes what M0–M4 must deliver. Move commands up as they start to exist.

## What works today

```sh
cargo test --workspace                          # about 1100 tests, one binary after the other: about 8 minutes
cargo xtask ci                                  # fmt --check + clippy -D warnings + tests (binaries side by side) + doctests + layers + wasm: about 7 minutes on 12 cores, 5 of them the boolean corpus (3996 cases against FreeCAD); under 4 without it
cargo xtask layers                              # dependency layering, and that every crate opts in to the never-crash lints
cargo xtask stats                               # lines and tests per crate
CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo xtask freecad   # FreeCAD reads our exports back
CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo xtask boolean-corpus --check   # regenerate the boolean corpus with FreeCAD and compare with the pinned one
cargo build --release -p craft3d-cli            # the CLI: target/release/craft3d-cli
```

Model a part from the command line, look at it, export it:

```sh
craft3d-cli commands --filter feature           # what can I do? ids, params, summaries
craft3d-cli run --new '{"units":"mm"}' \
    --cmd variable.set     --params '{"name":"width","expr":"60 mm"}' \
    --cmd feature.block    --params '{"name":"Plate","size":["width","width * 2 / 3","8 mm"]}' \
    --cmd feature.cylinder --params '{"name":"Post","position":["width / 4","width / 3","8 mm"],"radius":"width / 10","height":"30 mm"}' \
    --cmd document.measure \
    --out part.3dcraft                          # one JSON line per command
craft3d-cli info part.3dcraft                   # the whole state as JSON: features with status, bodies with volume
craft3d-cli render part.3dcraft --out part.png --size 1000x700 --view iso
craft3d-cli convert part.3dcraft part.3mf       # or .stl, .obj
craft3d-cli run part.3dcraft --cmd variable.set --params '{"name":"width","expr":"80 mm"}' --out wider.3dcraft
craft3d-cli run --new '{"units":"mm"}' --script long-build.json --out build.3dcraft --keep-going
```

`run` stops at the first command that fails, saves nothing and exits with 1. A long script is better run with `--keep-going`: a failing command gets the line `{"failed": id, "step": n, "error": text}`, the next one runs (a command that fails leaves the document as it was), `--out` saves what was built, and the exit code is still 1, with the first failure on stderr. `render` (and `view.render`) meshes the model for the picture: between a quarter and half a pixel of the picture where it shows the model nearest, never finer than the floor (2e-4 of the diagonal of the box around the bodies), at most 12 degrees per chord, within a total budget of a million triangles; its JSON says the tolerance it used and whether it had to coarsen it.

Drive it as a server (see [`control-protocol.md`](control-protocol.md)):

```sh
craft3d-cli serve --automation-read-root work --automation-write-root work     # JSON lines on stdin/stdout
craft3d-cli mcp   --automation-read-root work --automation-write-root work     # MCP on stdin/stdout
claude mcp add craft3d -- "$PWD/target/release/craft3d-cli" mcp \
    --automation-read-root "$PWD/work" --automation-write-root "$PWD/work"
```

A contact sheet of every reference solid, to look at what the tessellator makes of them: `cargo run --release -p craft3d-render --example solids -- target/evidence/solids.png`.

The teapot (Bézier patches, outside the document path) still has its own examples: `cargo run --release -p craft3d-render --example teapot` and `cargo run --release -p craft3d-io --example export_teapot`.

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on first use. If `cargo` is not on `PATH`, it is in `~/.cargo/bin`. CI (`.github/workflows/ci.yml`) runs `cargo xtask ci` on Linux and `cargo test --workspace` on macOS and Windows. It does **not** run on every push (developer's decision, 2026-10-12: nobody uses the software yet and a run costs about 25 runner-minutes): it runs every Monday morning and on demand with `gh workflow run CI && gh run watch`. The local `cargo xtask ci` before every commit is therefore the gate; run the workflow by hand after changes that are likely to differ per platform (file paths, threads, anything in `format`, `engine::workspace` or the CLI) and before anything that must be certain.

External oracle: FreeCAD runs as a separate program to check exported files ([`testing.md`](testing.md) oracle 8). Point `CRAFT3D_FREECAD` at a FreeCAD AppImage (run as `<AppImage> freecadcmd <script>`) or a `FreeCADCmd` executable. The tests (`crates/io/tests/freecad.rs` for meshes, `crates/drawing/tests/freecad.rs` and `crates/engine/tests/drawing_freecad.rs` for DXF and SVG, `crates/io/tests/step_freecad.rs` and `crates/engine/tests/step_freecad.rs` for STEP) are `#[ignore]`d, so plain `cargo test` and CI skip them; `cargo xtask freecad` runs them (about two minutes after the build: one launch of FreeCAD per test, and the STEP checks take nearly all of it) and fails if FreeCAD is missing. The STEP checks print a table per file (valid, volume and area against ours, centre, bounding box, counts, what FreeCAD added) and leave it with the files and FreeCAD's raw report in `target/tmp/step-io/` and `target/tmp/step-engine-files/` (`report.md`, `report.jsonl`); the booleans the kernel refused and the bodies we could not measure ourselves are in `target/tmp/step-engine/refused.md`. One of them alone: `CRAFT3D_FREECAD=… cargo test --release -p craft3d-engine --test step_freecad -- --ignored --nocapture`. After a deliberate change of the STEP mapping, `CRAFT3D_UPDATE_GOLDEN=1 cargo test -p craft3d-io --test step golden` rewrites the two golden files; read the diff. A script handed to FreeCAD must not be called `freecad.py`: that name shadows FreeCAD's own module and the script silently does nothing.

The boolean corpus ([`testing.md`](testing.md) oracle 8) is the other use of FreeCAD. The pinned file `crates/kernel/tests/fixtures/boolean_corpus.json` is read by `cargo test -p craft3d-kernel --test boolean_corpus`, which needs no FreeCAD and takes about a minute (17 s for the primitives; building a domed block measures it, 0.1 s for a Bézier roof and 0.3 s for a NURBS one). `cargo xtask boolean-corpus` regenerates the file (about a minute: it deals the cases out to one FreeCAD process per core, `--jobs N` to change that) and `--check` regenerates into `target/tmp/boolean-corpus/` and compares case by case; with the same FreeCAD build the result is byte-identical, with another one the measured numbers differ in their last digits and the corpus has to be re-pinned and the score reviewed. `--seed N` makes another corpus, `--only <part of an id>` prints the matching cases without writing anything. A pair on which FreeCAD crashes or takes more than two minutes is marked unreliable and the rest carries on. After the kernel learns something, raise `AGREEING_FLOOR` in `crates/kernel/tests/boolean_corpus.rs` to the count the test prints.

## Prerequisites

- Rust stable (the version is pinned in `Cargo.toml` when M0 scaffolds). Add the web target with `rustup target add wasm32-unknown-unknown`.
- Linux, macOS or Windows. The developer's machine is Fedora Linux; that is the Tier-1 platform until others are tested.
- Linux windowing, GL and GTK development packages (needed from M4, when the desktop app exists):
  - Fedora/RHEL: `sudo dnf install libxkbcommon-devel wayland-devel libX11-devel libXrandr-devel libXi-devel mesa-libGL-devel gtk3-devel`
  - Debian/Ubuntu: `sudo apt install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev`
- Optional: `cargo install cargo-nextest --locked` for parallel test runs.

## Build and run

```sh
cargo test --workspace                                     # everything
cargo xtask ci                                             # fmt + clippy + tests + layers + wasm
cargo xtask stats                                          # tests and lines per crate
cargo run --release -p craft3d -- path/to/model.3dcraft    # desktop viewer (M4)
cargo run --release -p craft3d -- --control 7878 --control-token-file .private/control.token \
  --automation-read-root work --automation-write-root work
```

Geometry code is slow at `opt-level 0`, so the workspace profile builds dependencies at `opt-level 2`. Use `--release` for anything interactive or timed.

**Autosave and crash recovery.** The app autosaves documents with unsaved changes into `3dcraft/recovery` in the per-user state directory (`~/.local/state` on Linux), 30 s after a change, and a start after a crash or a kill shows a bar with what was left there: Recover, Discard (asks twice), Later. The status bar says `autosaved 2 min ago`, or why autosave failed. `--recovery-dir DIR|none` and `--autosave-interval SECONDS` (or `CRAFT3D_RECOVERY_DIR`, `CRAFT3D_AUTOSAVE_INTERVAL`) change that; closing the window, or `ui.quit`, is a clean end and leaves nothing. **A window started for a test or by an agent, and ended by killing it, would leave its documents in the developer's own recovery directory: start it with `--recovery-dir none` or a directory of the test's own** (`apps/craft3d/tests/hidden_window.rs` and `killed_window.rs` do). The recovery directory is described in [`file-formats.md`](file-formats.md), the three `recovery.*` commands in [`control-protocol.md`](control-protocol.md).

The dev profile builds with `debug = "line-tables-only"`: backtraces keep file and line, and test binaries link faster. To inspect local variables in a debugger, build with `CARGO_PROFILE_DEV_DEBUG=true`.

### Faster test loops

```sh
cargo test -p <crate> --lib --tests        # skip examples and doctests
cargo test -p <crate> --test <file>        # one integration-test file
cargo test -p <crate> <name> -- --nocapture
cargo test --doc --workspace               # the doctests alone
```

`cargo xtask ci` builds the tests once (`cargo test --workspace --no-run`) and runs the test binaries side by side, slowest first by the durations of the previous run (`target/ci-durations.txt`), each binary's output printed in one piece when it is done; the doctests follow through cargo. `cargo test --workspace` runs the binaries one after the other and takes about twice as long. GPU tests each open their own wgpu device, so the binaries of `crates/gpu` run one after the other on one worker. The long random tests (`panic_hunt`, `boolean_random`, `meet_random`) run their cases on every core through `craft3d_testkit::parallel::map_parallel`: independent cases, each with its own seed, results in the order of the inputs. `panic_hunt` prints where its time goes per command with `--nocapture`.

## Headless CLI

`apps/craft3d-cli` builds the binary `craft3d-cli`:

```sh
craft3d-cli convert bracket.3dcraft bracket.step              # native -> STEP
craft3d-cli convert bracket.3dcraft bracket.3mf --tolerance 0.01
craft3d-cli info bracket.3dcraft                              # JSON: units, variables, feature tree with status, bodies, mass properties
craft3d-cli run --new '{"units":"mm"}' \
    --cmd feature.primitive.box --params '{"size":["40 mm","20 mm","10 mm"]}' \
    --cmd document.measure \
    --out box.3dcraft
craft3d-cli run bracket.3dcraft --cmd variable.set --params '{"name":"width","expr":"55 mm"}' --out wider.3dcraft
craft3d-cli render bracket.3dcraft --out shot.png --size 1200x800 --view iso    # CPU renderer, no GPU needed
craft3d-cli replay bracket.3dcraft --out bracket.mp4 --size 1280x720 --frames-per-step 6 --hold 60   # how it is built, as a video
craft3d-cli commands --filter fillet                          # the command registry with params docs
craft3d-cli serve                                             # JSON lines on stdio (see control-protocol.md)
craft3d-cli mcp --automation-read-root work --automation-write-root work
```

`run` prints one JSON line per command result. A script file (`--script steps.json`: `[["<id>", {…}], …]`) is the same list of commands; it is the format for agent-written scenarios and demos.

### A showcase video

`craft3d-cli replay` shows how a document is built: the history is rolled forward from nothing one feature at a time, and every step that changes a body becomes a frame, drawn by the CPU renderer from one camera framed on the finished model (the engine command `view.replay`, [`control-protocol.md`](control-protocol.md)). The document is not changed.

```sh
craft3d-cli replay part.3dcraft --out part.mp4 --size 1280x720 --frames-per-step 6 --hold 60 --orbit 60
craft3d-cli replay part.3dcraft --out part.gif --size 480x360 --frames-per-step 3 --hold 20 --fps 15
craft3d-cli replay part.3dcraft --out frames/ --from Plate --to Holes --every   # PNG frames and captions.srt only
```

- `.mp4` and `.gif` are made by `ffmpeg`, found on `PATH` or named by `CRAFT3D_FFMPEG`; the engine itself starts no program. Without ffmpeg the command fails with exit code 1 and keeps the frames in `OUT.frames/` (`frame-%05d.png` at the given fps, with `captions.srt`), which `--keep-frames` also keeps after a video is made.
- The captions (each step's place in the history, feature name and type) are a subtitle track in the MP4 (players switch it on) when the ffmpeg build has the `mov_text` encoder (the result says `captionsTrack`; Fedora's `ffmpeg-free` lacks it) and `captions.srt` beside the frames; they are not drawn into the picture, because the renderer has no text. The result JSON lists every step with the frames it got.
- `--frames-per-step` slows the build down, `--hold` lingers on the finished model, `--orbit DEG` turns the camera over the whole replay (the model is then framed so that it stays in the picture all round), `--from`/`--to` pick a range of the history, `--params` passes any other `view.replay` param (`projection`, `zoom`, `box`, `tolerance`).
- Time: opening and regenerating the document once, then about the CPU renderer's time per distinct picture, spread over the cores (each step reuses what the document's regeneration built; nothing is rebuilt). The turbofan (156 features, 113 steps that change a body) takes about 4.5 s beyond opening it on 14 cores: 3 s to tessellate the model once (as `view.render` does) and 1.5 s for the 113 frames at 800x600 (0.13 to 0.25 s of CPU each, many at once); stepping through the history costs 0.1 s.

## Driving the app programmatically

Start the app with a private token file. It then accepts authenticated JSON lines on `127.0.0.1:7878`:

```sh
TOKEN=$(tr -d '\r\n' < .private/control.token)
printf '%s\n' "{\"id\":\"auth\",\"method\":\"auth\",\"params\":{\"token\":\"$TOKEN\"}}" \
              '{"id":1,"method":"engine.execute","params":{"command":"feature.extrude","params":{"sketch":"Sketch1","distance":"25 mm"}}}' \
              '{"id":2,"method":"ui.screenshot","params":{"path":"evidence/shot.png"}}' | nc 127.0.0.1 7878
```

See [`control-protocol.md`](control-protocol.md) for every method. `.private/` is gitignored.

**Pointing at a face.** Click a face in the viewport (the left button, without dragging): it is highlighted, and `ui.get` has what was clicked under `state.picked` (the body's feature, the face's persistent name as `document.inspect` lists it, the point), so an agent can be told "this face" by clicking it. The other way round, an agent shows which face it means with `ui.highlight` and asks what is at a pixel of a screenshot with `ui.pick`:

```sh
printf '%s\n' "{\"id\":\"auth\",\"method\":\"auth\",\"params\":{\"token\":\"$TOKEN\"}}" \
              '{"id":1,"method":"ui.pick","params":{"x":412.5,"y":300.5}}' \
              '{"id":2,"method":"ui.highlight","params":{"face":"F1:face(1)","feature":"Plate"}}' \
              '{"id":3,"method":"ui.get"}' | nc 127.0.0.1 7878
```

**Pointing at an edge.** Click within 4 points of an edge that is in sight: the edge is highlighted instead of the face, and `state.picked` has `kind: "edge"` and `edge` (its body, its own name where it has one, the faces it lies between, its place in `document.edges`, the point). An agent points at an edge with `ui.highlight {"edge": "F1:edge(7)"}` or, for an edge without a name of its own (after a boolean), `ui.highlight {"between": ["F3:copy(0)/F1:face(1)", "F3:copy(1)/F2:face(0)"]}`; `ui.pick` takes `radius` (pixels; 0 for faces only). A click within 4 points of a vertex in sight is a click on the vertex (`kind: "vertex"`, `vertex`), which then wins over the edges that end there; an agent points at one with `ui.highlight {"vertex": "F1:vertex(4)"}` or `{"corner": [face, face, face]}`, the faces that meet in it.

Without a window the command `view.pick` answers the same question about the picture `view.render` makes ([`control-protocol.md`](control-protocol.md), *Pointing*).

## The viewer today

`craft3d [FILE]` shows the active document: bodies shaded with their edges, sketches as lines (dimmed where a body is in front of them), datum planes as translucent squares with their outline and name, datum axes as lines and datum points as dots (`ui.set {"show": {"datums": false}}` hides them, `{"show": {"sketches": false}}` the sketches; the *Sketches* and *Datums* toggles over the viewport, above *Section*, do the same), the feature tree and a status bar. The tree marks every feature's state with a mark and a colour (✔ built, ⚠ failed with its reason under it and on hover, ⏸ suppressed, ○ rolled back), and a feature that failed because one it is made from failed says which (*caused by Plate*; a click selects it); `ui.get` › `features` holds the same. The bodies and the lines are drawn by the wgpu renderer (`crates/gpu`) on the window's own device when the machine has a graphics adapter (multisampled, 4 samples per pixel, straight into the texture that is shown: nothing is read back and every frame is the sharp one, also while the model is turned) and by the CPU reference renderer otherwise; `--renderer cpu` (or `CRAFT3D_RENDERER=cpu`) asks for the CPU renderer, to compare the two. The status bar's `GPU 1440×900 …` or `CPU …` says which made the image on screen, `ui.get` reports it with the adapter's name, whether it draws directly on the window's device and with how many samples (`renderer.direct`, `renderer.msaa`) and, when the direct GPU path was wanted and could not be used, why (`renderer.fallback`), and the terminal prints it at start (`craft3d: viewport renderer: gpu (…; direct, 4 samples)`). `ui.screenshot` is always rendered by the CPU renderer: it is the oracle the GPU viewport is checked against. **The viewport's mesh is finer than the one `view.render`, previews and `file.export` make by default** (2026-10-13): the shell asks the engine for the *display quality* (`ui.get` › `state.quality`; `ui.set {"quality": {"chordal": fraction, "maxTriangles": integer}}`), a chordal tolerance of `chordal` times the model's size (the side of a square with the bodies' total area; 2e-4 by default, against 1e-3 for the automatic tolerance, with an angle criterion of `angle` degrees, 12 by default, that keeps small holes round), coarsened until the scene has at most `maxTriangles` (1,000,000) triangles and every body is within the tessellator's reach, but never beyond the quick first picture's tolerance (1e-3 of the diagonal of the model's box); `ui.get` › `scene.tolerance` and `scene.coarsened` say what it got. `ui.screenshot` renders that same scene, so it shows what the window shows; `--renderer cpu` follows the same rule. Measured on the flange of the demo (a disc of radius 40 mm with six bolt holes of radius 4 mm; `crates/ui-egui/tests/gpu_viewport.rs` › `the_flange_holes_are_round_at_the_display_quality`, pictures and crops of one hole in `target/evidence/ui/viewport-flange-*.png`, the window's own in `flange-8082-*.png`): before, 820 triangles and a chord error of 0.66 px in a 1020 × 754 window (1.4 px at 2560 × 1400: a visibly twelve-sided hole); after, 4328 triangles and 0.02 px (0.04 px). The scene is tessellated in about 10 ms either way (the cost is per face, not per triangle, on such a model), the GPU viewport's frame on the drawing thread takes 0.2 to 0.5 ms either way and 1.7 to 5 ms until the GPU is done, the first frame after the change (the upload) 2.2 ms; the CPU viewport's sharp image at 1020 × 754 (ssaa 2) takes 74 ms before and 92 ms after, its preview while orbiting 7 ms either way. A plate with 1024 holes (1030 faces, `crates/engine/tests/scene_bench.rs`): 24,588 triangles in 666 ms at the automatic tolerance, 122,892 in 776 ms at the display quality; the tessellator's cost beyond its cost per face is about 1.5 µs per triangle, which is what the budget of a million bounds (about 1.5 s per edit at worst). At scale (`viewport_cost`, a scene of 665,104 triangles against one of 12,796, both at 1020 × 777): the CPU viewport's sharp image takes 176 against 123 ms and its preview while moving 30 against 8 ms (0.08 and 0.03 µs per triangle: the cost is per sample), the GPU draws it in 16 to 18 ms after a one-time upload of 60 ms; the budget adds at most about 80 ms to a CPU sharp image and 30 ms to a preview. Drag to orbit, right-drag or shift-drag to pan, wheel to zoom, F to frame everything. File menu: New (Ctrl+N), Open (Ctrl+O), Save (Ctrl+S), Save As (Ctrl+Shift+S), with the system's file dialog (on Linux through xdg-desktop-portal, so a portal backend has to be running, as it is on GNOME and KDE); the title shows the file's name and a `*` for unsaved changes. Everything else is done with commands over the control channel.

**Tabs of the open documents (2026-10-14).** A row of tabs under the menu bar, above the tree and the viewport, has one tab per open document in the order of `file.list`: the file's name without its extension, or *Untitled 1*, *Untitled 2*, … (counted in the row) for one never saved; a dot in the attention colour before the name for unsaved changes; the active one lighter with an underline; its path on hover. A click on a tab makes that document the active one with `file.activate`, run on the engine thread like every command, so the window keeps drawing (and shows the progress plate) while a heavy document's scene is built; Ctrl+Tab and Ctrl+Shift+Tab go to the next and the previous document, round. The × on a tab (or a middle click on it, or Ctrl+W and File › Close for the active one) closes it with `file.close`; the document that was active stays active, and when it is the one closed its neighbour in the row is shown. A document with something to lose (what `file.close` refuses without `discard`: unsaved changes and a file or features) asks first, in a dialog over the window: *Save* (to its own file, or Save As's dialog for one without a file, and closed once saved), *Don't Save* (`file.close {"discard": true}`), *Cancel* (also Escape or a click beside it). Over the control channel `file.close` is as it was: it closes the active document and refuses unsaved changes without `discard`. The tabs shrink to fit, between 72 and 200 points (`Tokens::tab_min_width`, `tab_max_width`), and when even the narrowest do not fit the row scrolls, with the active tab brought into view. The File menu lists the open documents too, the active one checked. `ui.get` › `tabs` and `closing` hold what the row and the dialog show ([`control-protocol.md`](control-protocol.md)). `cargo run --release -p craft3d-ui-egui --example window_picture -- --idle --active 1 --edit 2 --ask 2 a.3dcraft b.3dcraft c.3dcraft untitled OUT.png` paints the window with several documents (`untitled`: a new one with a block), one of them changed and its close question open.

**Coarse first, then sharp; switching tabs (2026-10-14).** After an edit, on opening a document or on going back to one whose scene was let go, the window shows a quick first picture at once (1e-3 of the model's box, built on the engine thread) and the status bar says `refining ·` until the display-quality mesh, built on a thread of its own (`apps/craft3d/src/refine.rs`), takes its place; commands meanwhile run as usual, and `ui.screenshot` waits for the display mesh. Every open document's scenes are kept (1.5 GiB at most, least recently shown out first), so switching tabs shows the model in well under a millisecond and never meshes it again. `cargo test --release -p craft3d-engine --test display_speed -- --nocapture` prints regeneration, first picture and display scene for the showcase's spring and screw, and for every `.3dcraft` in `CRAFT3D_DISPLAY_MODELS`; `cargo run --release -p craft3d-engine --example tolerance_sweep -- FILE.3dcraft` prints triangles and time at a row of tolerances; `window_picture --first` paints the window with the first picture, and `--set '{"quality": {"chordal": 1e-4}, "camera": {...}}'` with another display quality or a camera.

**The window never waits for a command (2026-10-14).** Commands (from the control channel, MCP, or a file chosen in the Open and Save As dialogs) run on the window's engine thread, which also regenerates and meshes; the window keeps drawing, can be moved, turned and closed, and shows over the viewport a spinner, what is being done (`file.open: regenerating J2 (17 of 20), 3 s`) and a bar for the features done so far. `ui.get` › `work` says the same and is answered at once while work runs ([`control-protocol.md`](control-protocol.md)). Opening the developer's aircraft (`plane.3dcraft`: a revolved fuselage, lofted wings, tailplane and fin, five unions) froze the window for about 8 s before, long enough for GNOME's *not responding* dialog. `cargo run --release -p craft3d-engine --example regen_profile -- FILE.3dcraft` times every feature regeneration rebuilds, the measuring of the model and the meshing, and prints a hash of the display mesh; `cargo run --release -p craft3d-ui-egui --example window_picture -- FILE.3dcraft OUT.png` paints the whole shell, panels included, as it looks while work runs (egui's own wgpu renderer into a texture: GNOME on Wayland lets no other program take a screenshot).

The window never waits for a renderer. On the GPU the frame is submitted from the drawing thread and the device finishes it on its own (the status bar's milliseconds are the drawing thread's share: uploading what changed, encoding, submitting). On the CPU, and on the GPU when the direct path is not available (then the GPU renders on a device of its own and reads the frame back, as it did before 2026-10-13), the sharp image is made on a render thread while a quick, smaller preview is on screen, and the status bar's milliseconds are the render thread's. `cargo run --release -p craft3d-ui-egui --example frame_time` times whole frames on the drawing thread for the window's path (`-- --readback` for the GPU read back, `-- --cpu` for the CPU renderer, `-- --inline` to make their sharp image on the drawing thread), and for the direct path also the time until the GPU is done with the frame; `cargo run --release -p craft3d-ui-egui --example viewport_cost` prints the parts of one image ([`architecture.md`](architecture.md) › Current state › Desktop app and shell). Measured on the development machine (an Intel Meteor Lake iGPU over Vulkan, 2026-10-13, other builds running; `frame_time`, the drawing thread's time per frame while orbiting, and for the direct path the time until the GPU is done with it): at 1280 × 800 with 84 triangles the direct path takes 0.2 ms on the thread and 2.1 ms until the GPU is done, against 9.6 ms for the GPU read back (a half-size preview) and 5.8 ms for the CPU (idem); with 12 796 triangles 0.4 and 2.5 ms against 12.6 and 8.9 ms; at 2560 × 1400 with 12 796 triangles 0.5 and 6.2 ms against 24.1 and 21.4 ms, and the sharp image after the camera stops, which the direct path has at once, took the readback path 142 ms (1843 × 1085, ssaa 2) and the CPU 125 ms (2048 × 1205, ssaa 1). The first frame after a scene change uploads the mesh and the lines: 0.7 ms for 12 796 triangles, about 60 ms for 665 104 triangles (80 MB of vertices), which `viewport_cost` prints; that model is then drawn in 13 ms at 1020 × 777 and 18 ms at 2560 × 1400 (GPU done).

**The view cube (2026-10-14).** In the bottom-right corner of the viewport: a small cut gem that turns with the camera. It is a cube with its edges and corners cut off, 26 facets in all: six faces named after the standard views (`top`, `front`, `right`, `back`, `left`, `bottom`, each tinted with the colour of the axis it is across), twelve facets where an edge was and eight where a corner was; the three axes stick out of the faces they point through, X red, Y green, Z blue as at the origin. A click on a facet turns the camera, in a quarter of a second, to look at the model from that side and frames it: a face gives a standard view, an edge facet the view half-way between two of them (45°), a corner facet the view from that corner. A click on the facet that already faces you gives the view from the other side. A drag on the cube orbits the camera exactly as a drag in the viewport does. The cube is drawn the way the viewport is, in perspective or orthographic, and a click on the ring around it (which carries the projection's name) switches between the two. Under it stay two small buttons, **Fit** and **iso**, and the see-through slider. None of it is a pick: the cube and the buttons are widgets over the viewport. It replaces the axis gizmo of 2026-10-13 and the row of view buttons before that. Shell code only (`crates/ui-egui/src/view_cube.rs` for where the facets are and what a click means, `Shell::view_cube` for the mouse and the paint, `Shell::advance_turn` for the turn; sizes and colours in `theme::Tokens`: `view_cube_size`, `view_cube_dot`, `view_cube_opacity`, `axes`); the camera it sets is `state.camera`, which an agent sets with `ui.set {"view": ...}` or `{"camera": {"yaw", "pitch", "projection"}}` as before, so nothing was added to the control protocol. A camera set through `ui.set` during a turn ends the turn. The cube is not in `ui.screenshot` (that is the viewport's picture without widgets). Tests: `crates/ui-egui/tests/view_cube.rs` (a synthetic click on every one of the 26 facets in both projections and again for the opposite side, the turn frame by frame, a drag compared with `CameraState::orbit`, the ring, the buttons, a viewport too small for it).

**See-through (2026-10-13).** Left of the buttons under the view cube is a slider, *See-through*, from opaque to nearly clear (`state.transparency`, 0 to 0.9, also `ui.set {"transparency": 0.5}`): the bodies are drawn with the surface behind the front one showing through, and the hidden parts of their edges dimmed, so a pocket or a cavity inside a part can be looked at. The window's device draws it (since 2026-10-14: the scene twice, for the front and for what is behind it, and a blend), so turning a see-through model is as quick as turning an opaque one: measured in the window at 1020 × 754 with 16 000 triangles, 0.4 to 1.2 ms of the drawing thread per frame either way (it was 492 ms per frame on the CPU), and until the GPU is done 4.7 ms against 1.7 ms opaque for 12 800 triangles and 19 ms against 12.5 ms for 665 000 (`cargo run --release -p craft3d-ui-egui --example viewport_cost`, Intel integrated graphics). Picks and highlights work as ever. `ui.get` › `render.renderer` says which renderer drew the frame on screen: `cpu` only on a device that cannot draw the layers, or with `--renderer cpu`.

**The section plane (2026-10-14).** In the row above that slider, beside the view cube: *Section* cuts the picture open with a plane, so that the inside of a shell, a pocket or a cavity can be looked at; where the plane passes through material the cut is closed with a flat orange cap, so a solid looks solid. While it is on, *X*, *Y* and *Z* choose the axis the plane is across (it starts in the middle of the model), the slider moves it across the model's extent, and *Flip* keeps the other half. It is a view like the camera (`state.section`, also `ui.set {"section": {"on": true, "plane": "zx", "offset": 0, "flip": true}}`, where the plane may also be a datum plane, a flat face by its name or a point and a normal): the document does not change and nothing is regenerated. The window's device draws it (one more pass over the triangles, which counts what is cut away, and one triangle for the cap); a click on a cut-open model gives what is seen, and on the cap `state.picked.kind` is `section`. `ui.screenshot` and `view.render {"section": {…}}` cut the same way, the latter without a window.

## Watching along

The developer follows development and automated testing live. Three ways, cheapest first:

1. **Test output.** Run tests in the foreground with `-- --nocapture` when the developer is watching, and say (in Dutch) what the run is expected to show before starting it. Geometry tests print the numbers they compare (expected, actual, tolerance), not just pass/fail.
2. **Evidence images.** Tests and scripts that render write PNGs under `target/evidence/…` ([`testing.md`](testing.md)). After a visual change, name the files to open. `craft3d-cli render` produces a picture of any document with the CPU renderer: no window, no GPU, no focus stealing.
3. **The live viewer** (exists since 2026-10-12; today: `cargo build --release -p craft3d -p craft3d-cli`, then `target/release/craft3d --control 7878 --control-token-file .private/control.token --automation-write-root target/work`, and for a scripted demo `cargo run --release -p craft3d --example demo -- --port 7878 --token-file .private/control.token --delay-ms 1500 --part plate`; the `--bridge`/`--step-delay` flags of `run` in the sentence below do not exist, `craft3d-cli mcp --bridge` does). Launch `craft3d --control <port>` once; the window stays on the developer's screen. Agents then build and edit the model through the control channel, and the viewport updates after every command: the developer sees the part grow feature by feature. Use `ui.set` to frame the camera on what changed and `ui.notice` to show a one-line caption of the current step. Run scripted demos with a delay between steps (`craft3d-cli run --script demo.json --bridge 127.0.0.1:7878 --step-delay 800ms`) so they can be followed.

Rules:

- Never launch a second window when one is already running under control; reuse it.
- Don't steal focus repeatedly: prefer offscreen renders for checks the developer did not ask to watch.
- Screenshots capture only our own window.
- Demo models are original or public-domain designs. No personal data in files, file names or requests; when fetching anything, use a generic `Craft3d-dev` User-Agent.

## Offscreen UI snapshots (no window)

```sh
cargo run --release -p craft3d-ui-egui --example snapshot -- \
    --out ui.png --size 1440x900 --scale 2 --open bracket.3dcraft \
    --script '[["ui.set", {"view": "iso"}]]' --renderer auto
```

The viewport as the window would show it, without a window: the shell runs headlessly (egui with no backend) with the renderer the window would use (the GPU when there is one, `--renderer cpu` for the CPU), the image it uploads as the viewport's texture is taken, and the lines it draws over it are inked on it. The panels are not in the picture (egui's own drawing needs a backend; that is not built). `--size` is the viewport in points, `--scale` the pixels per point. `--script` is a list of control-protocol calls (`[method, params]`), applied in order: the `ui.*` methods and `engine.execute {"command", "params"}`. It prints which renderer it used and how long the image took. For the oracle's picture of the same state use `ui.screenshot` or `craft3d-cli render`.

## MCP (agents)

`craft3d-cli mcp` serves MCP on stdio using `crates/automation`:

- **Headless:** `craft3d-cli mcp --automation-read-root <dir> --automation-write-root <dir>`. It drives an in-process engine session and has no file authority when a root is omitted.
- **Live app:** start `craft3d --control 7878 --control-token-file <private-path> …`, then run `craft3d-cli mcp --bridge 127.0.0.1:7878 --control-token-file <private-path>`. The developer sees every step in the window.

Claude Code (`.mcp.json` in the repo root, or `claude mcp add`):

```json
{
  "mcpServers": {
    "craft3d": {
      "command": "/path/to/3dcraft/target/release/craft3d-cli",
      "args": ["mcp", "--automation-read-root", "/absolute/path/to/work", "--automation-write-root", "/absolute/path/to/work"]
    },
    "craft3d-live": {
      "command": "/path/to/3dcraft/target/release/craft3d-cli",
      "args": ["mcp", "--bridge", "127.0.0.1:7878", "--control-token-file", "/private/path/craft3d-control.token"]
    }
  }
}
```

A typical agent loop:

1. `doc_new {units:"mm"}`
2. `command_list {filter:"sketch"}`
3. `command_run {id:"sketch.create", params:{plane:"xy"}}`, then entities and constraints
4. `command_run {id:"feature.extrude", params:{sketch:"Sketch1", distance:"25 mm"}}`
5. `doc_inspect` to verify feature status, volume and face count; `doc_render_preview` to look
6. `doc_save {path:"bracket.3dcraft"}`, `doc_export {path:"bracket.step"}`

## Native format (.3dcraft)

See [`file-formats.md`](file-formats.md). In short: a ZIP of STORE entries (or a directory with the same layout) holding `manifest.json` (the document: variables, sketches, constraints, features, order), content-addressed `blobs/` for imported bodies and optional regeneration caches, and `thumb.png`. Saves are atomic (write a temp file, fsync, rename). Autosave writes snapshots to a recovery directory on a background thread.

## Graphics startup and device loss

`--renderer cpu` (`CRAFT3D_RENDERER=cpu`) starts with the CPU renderer. Otherwise the viewport is drawn on the window's own wgpu device (the one eframe draws the window with); when its pipelines cannot be made there, or the graphics driver fails while that is tried, the viewport's renderer opens a wgpu device of its own on each of its two threads instead (the drawing thread for previews, the render thread for sharp images) and reads the frames back; when no adapter is found for that either, the viewport is rendered on the CPU; `ui.get` (`renderer.fallback`) and the terminal say why in both cases. A device that is lost while running, or any GPU error that is not the frame's own fault (a lost device, a failed readback, a driver that falls over inside a render), ends that path for the rest of the session: the direct path gives way to the CPU renderer on its threads, the readback path's thread renders on the CPU from then on; the status bar shows `CPU …`. A frame the device cannot take (a target or a mesh too large for it) is rendered the next way down, and the next one on the GPU again. `cargo test -p craft3d-ui-egui --test gpu_fallback` runs the no-adapter case (in a child process with `WGPU_ADAPTER_NAME` set to a name no adapter has).

The window itself is drawn by eframe on wgpu, and eframe 0.36 can only present through wgpu or OpenGL. So the window needs *some* adapter: a GPU, or a software implementation, which wgpu takes when it is the only one. With Mesa's lavapipe (`mesa-vulkan-drivers`) or llvmpipe installed the app runs without a GPU (and the viewport's renderer then runs on that software adapter too); try it with `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json WGPU_BACKEND=vulkan craft3d`. With no adapter at all the app does not start: it prints what to install and exits with code 1. A window that needs no adapter would take a presentation path of our own (a CPU rasteriser for egui's output and a software surface), which is a second windowing stack and was not built. Nothing else needs an adapter: `craft3d-cli render`, `run`, `serve` and `mcp` work on any machine.

## Logs

The desktop app writes `log` records to standard error and to `logs/craft3d.log` in the settings directory (Linux `~/.config/craft3d/logs/`), rotating the previous two runs. `RUST_LOG` takes env_logger-style directives (`RUST_LOG=warn,craft3d_kernel=trace`). The crash guard's panic report lands there.

## Environment variables

| Variable | Effect |
|---|---|
| `CRAFT3D_CONTROL_PORT` | Same as `--control <port>` |
| `CRAFT3D_CONTROL_TOKEN_FILE` | Read, or create for a server, the control bearer-token file |
| `CRAFT3D_AUTOMATION_READ_ROOT` / `CRAFT3D_AUTOMATION_WRITE_ROOT` | Desktop app only: automation file roots. Headless CLI modes require the flags |
| `CRAFT3D_RECOVERY_DIR` | Desktop app only: where documents with unsaved changes are autosaved and where a start looks for what a crash left behind; `none` turns autosave off. Same as `--recovery-dir`. Default: `3dcraft/recovery` in the per-user state directory (`$XDG_STATE_HOME` or `~/.local/state`; `~/Library/Application Support`; `%LOCALAPPDATA%`). `craft3d-cli serve` and `mcp` take the flag only, and do not autosave without it. Tests and scripts that start the app pass a directory of their own |
| `CRAFT3D_AUTOSAVE_INTERVAL` | Desktop app only: seconds after a change until a document is autosaved (0.01 to 86400, default 30). Same as `--autosave-interval` |
| `CRAFT3D_RENDERER` | `auto` (the GPU when there is an adapter, else the CPU; the default) or `cpu`: what shades the viewport. Same as `--renderer` |
| `CRAFT3D_CONFIG_DIR` | Settings and log directory override (tests, portable use) |
| `CRAFT3D_THREADS` | Worker thread count (determinism tests run with 1, 2 and N) |
| `CRAFT3D_FFMPEG` | Path to `ffmpeg`, used only by `craft3d-cli replay` to make an MP4 or a GIF; default: the first `ffmpeg` on `PATH` |
| `CRAFT3D_FREECAD` | Path to a FreeCAD executable or AppImage, used only by opt-in export-validation tests |
| `WGPU_BACKEND` | Pick the wgpu backend (`vulkan`, `metal`, `dx12`, `gl`) |
| `RUST_LOG` | Log levels (see [Logs](#logs)) |

## Debugging geometry

- `craft3d-cli info --check model.3dcraft` prints the validity report of every body (which check failed on which entity).
- `document.inspect {body, detail:"topology"}` dumps faces, loops, edges and vertices with their persistent names, surface kinds and tolerances as JSON.
- `view.render` with `mode:"ids"` writes a false-colour image where each face has its own colour, with a legend in the JSON result: the quickest way to see which face a name refers to.
- `view.render` with `projection:"orthographic"` gives a picture to scale; `box:[[x,y,z],[x,y,z]]` frames a region instead of the whole model, and `zoom` magnifies about the centre: for looking at a detail.
- Export the smallest failing case as a test in the crate that owns the bug (`crates/<name>/tests/regressions.rs`), not as a file in the repo.

## Web build *(later milestone)*

`apps/craft3d-web` will run the same app in the browser through eframe's web runner (WebGPU, WebGL2 fallback), built with trunk; the only JavaScript is the glue wasm-bindgen generates. Until then `cargo xtask wasm` keeps L0–L7 compiling for `wasm32-unknown-unknown`.

## Dev log

`log/devlog.md` (gitignored) gets a terse entry at the end of every task: date, what landed, numbers (tests, timings with machine and load), what is still open. It is how the next agent session picks up.
