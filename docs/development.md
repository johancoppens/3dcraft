# Development guide

> **Last updated:** 2026-10-12 · **Status:** M3 done. *Works today:* everything under [What works today](#what-works-today). Every other section is *(planned)*: it describes what M0–M4 must deliver. Move commands up as they start to exist.

## What works today

```sh
cargo test --workspace                          # about 430 tests, three to four minutes (the tessellator's oracle tests take two)
cargo xtask ci                                  # fmt --check + clippy -D warnings + tests + layers + wasm
cargo xtask layers                              # dependency layering, and that every crate opts in to the never-crash lints
cargo xtask stats                               # lines and tests per crate
CRAFT3D_FREECAD=/path/to/FreeCAD.AppImage cargo xtask freecad   # FreeCAD reads our exports back
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
```

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

External oracle: FreeCAD runs as a separate program to check exported files ([`testing.md`](testing.md) oracle 8). Point `CRAFT3D_FREECAD` at a FreeCAD AppImage (run as `<AppImage> freecadcmd <script>`) or a `FreeCADCmd` executable. The tests (`crates/io/tests/freecad.rs`) are `#[ignore]`d, so plain `cargo test` and CI skip them; `cargo xtask freecad` runs them and fails if FreeCAD is missing. A script handed to FreeCAD must not be called `freecad.py`: that name shadows FreeCAD's own module and the script silently does nothing.

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

The dev profile builds with `debug = "line-tables-only"`: backtraces keep file and line, and test binaries link faster. To inspect local variables in a debugger, build with `CARGO_PROFILE_DEV_DEBUG=true`.

### Faster test loops

```sh
cargo test -p <crate> --lib --tests        # skip examples and doctests
cargo test -p <crate> --test <file>        # one integration-test file
cargo test -p <crate> <name> -- --nocapture
cargo nextest run --workspace              # parallel across binaries
cargo test --doc --workspace               # nextest does not run doctests
```

A full `cargo test` stays the final check. GPU tests each open their own wgpu device; when nextest is used they go in a group of limited size (`.config/nextest.toml`).

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
craft3d-cli commands --filter fillet                          # the command registry with params docs
craft3d-cli serve                                             # JSON lines on stdio (see control-protocol.md)
craft3d-cli mcp --automation-read-root work --automation-write-root work
```

`run` prints one JSON line per command result. A script file (`--script steps.json`: `[["<id>", {…}], …]`) is the same list of commands; it is the format for agent-written scenarios and demos.

## Driving the app programmatically

Start the app with a private token file. It then accepts authenticated JSON lines on `127.0.0.1:7878`:

```sh
TOKEN=$(tr -d '\r\n' < .private/control.token)
printf '%s\n' "{\"id\":\"auth\",\"method\":\"auth\",\"params\":{\"token\":\"$TOKEN\"}}" \
              '{"id":1,"method":"engine.execute","params":{"command":"feature.extrude","params":{"sketch":"Sketch1","distance":"25 mm"}}}' \
              '{"id":2,"method":"ui.screenshot","params":{"path":"evidence/shot.png"}}' | nc 127.0.0.1 7878
```

See [`control-protocol.md`](control-protocol.md) for every method. `.private/` is gitignored.

## The viewer today

`craft3d [FILE]` shows the active document: bodies shaded by the CPU renderer with their edges, sketches as lines (dimmed where a body is in front of them), the feature tree and a status bar. Drag to orbit, right-drag or shift-drag to pan, wheel to zoom, F to frame everything. File menu: New (Ctrl+N), Open (Ctrl+O), Save (Ctrl+S), Save As (Ctrl+Shift+S), with the system's file dialog (on Linux through xdg-desktop-portal, so a portal backend has to be running, as it is on GNOME and KDE); the title shows the file's name and a `*` for unsaved changes. Everything else is done with commands over the control channel.

The window never waits for the renderer: the sharp image is made on a render thread while a quick preview is on screen. `cargo run --release -p craft3d-ui-egui --example frame_time` times whole frames on the drawing thread (`-- --inline` for the old behaviour); the status bar's milliseconds are the render thread's.

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
    --script '[["ui.set", {"view": "iso"}]]'
```

Renders the full UI headlessly with the real wgpu viewport. `--script` is a list of control-protocol calls (`[method, params]`), applied in order.

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

`--safe-gpu` starts with the CPU renderer for one launch. If the wgpu device cannot be created, or is lost while running, the viewport switches to the CPU renderer for the rest of the session and a notice says so. The app must start and show a model on a machine with no usable GPU.

Today (2026-10-12): the model is always rendered on the CPU, but the window itself is drawn by eframe on wgpu, and eframe 0.36 can only present through wgpu or OpenGL. So the window needs *some* adapter: a GPU, or a software implementation, which wgpu takes when it is the only one. With Mesa's lavapipe (`mesa-vulkan-drivers`) or llvmpipe installed the app runs without a GPU; try it with `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json WGPU_BACKEND=vulkan craft3d`. With no adapter at all the app does not start: it prints what to install and exits with code 1. A window that needs no adapter would take a presentation path of our own (a CPU rasteriser for egui's output and a software surface), which is a second windowing stack and was not built. Nothing else needs an adapter: `craft3d-cli render`, `run`, `serve` and `mcp` work on any machine. `--safe-gpu` and device-loss handling do not exist yet.

## Logs

The desktop app writes `log` records to standard error and to `logs/craft3d.log` in the settings directory (Linux `~/.config/craft3d/logs/`), rotating the previous two runs. `RUST_LOG` takes env_logger-style directives (`RUST_LOG=warn,craft3d_kernel=trace`). The crash guard's panic report lands there.

## Environment variables

| Variable | Effect |
|---|---|
| `CRAFT3D_CONTROL_PORT` | Same as `--control <port>` |
| `CRAFT3D_CONTROL_TOKEN_FILE` | Read, or create for a server, the control bearer-token file |
| `CRAFT3D_AUTOMATION_READ_ROOT` / `CRAFT3D_AUTOMATION_WRITE_ROOT` | Desktop app only: automation file roots. Headless CLI modes require the flags |
| `CRAFT3D_CPU_VIEWPORT=1` | Force the CPU renderer in the viewport |
| `CRAFT3D_CONFIG_DIR` | Settings and log directory override (tests, portable use) |
| `CRAFT3D_THREADS` | Worker thread count (determinism tests run with 1, 2 and N) |
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
