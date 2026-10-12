# Development guide

> **Last updated:** 2026-10-12 · **Status:** M0 in progress. *Works today:* everything under [What works today](#what-works-today). Every other section is *(planned)*: it describes what M0–M4 must deliver. Move commands up as they start to exist.

## What works today

```sh
cargo test --workspace                          # 35 tests
cargo xtask ci                                  # fmt --check + clippy -D warnings + tests + layers + wasm
cargo xtask layers                              # dependency layering
cargo xtask wasm                                # every library crate checks for wasm32-unknown-unknown
cargo run --release -p craft3d-render --example teapot -- \
    --out target/evidence/teapot.png --ids target/evidence/teapot-ids.png \
    --size 1280x800 --tolerance 0.002 --ssaa 2  # the Utah teapot on the CPU renderer; prints a JSON summary
```

Rust is installed with rustup (stable, with `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target); if `cargo` is not on `PATH`, it is in `~/.cargo/bin`.

External oracle: FreeCAD may be run as a separate program to check exported files ([`testing.md`](testing.md) oracle 8). Point `CRAFT3D_FREECAD` at the executable or AppImage; tests that need it are opt-in and skip with a message when it is unset.

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

## Watching along

The developer follows development and automated testing live. Three ways, cheapest first:

1. **Test output.** Run tests in the foreground with `-- --nocapture` when the developer is watching, and say (in Dutch) what the run is expected to show before starting it. Geometry tests print the numbers they compare (expected, actual, tolerance), not just pass/fail.
2. **Evidence images.** Tests and scripts that render write PNGs under `target/evidence/…` ([`testing.md`](testing.md)). After a visual change, name the files to open. `craft3d-cli render` produces a picture of any document with the CPU renderer: no window, no GPU, no focus stealing.
3. **The live viewer** (from M4). Launch `craft3d --control <port>` once; the window stays on the developer's screen. Agents then build and edit the model through the control channel, and the viewport updates after every command: the developer sees the part grow feature by feature. Use `ui.set` to frame the camera on what changed and `ui.notice` to show a one-line caption of the current step. Run scripted demos with a delay between steps (`craft3d-cli run --script demo.json --bridge 127.0.0.1:7878 --step-delay 800ms`) so they can be followed.

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
3. `command_run {id:"sketch.create", params:{plane:"XY"}}`, then entities and constraints
4. `command_run {id:"feature.extrude", params:{sketch:"Sketch1", distance:"25 mm"}}`
5. `doc_inspect` to verify feature status, volume and face count; `doc_render_preview` to look
6. `doc_save {path:"bracket.3dcraft"}`, `doc_export {path:"bracket.step"}`

## Native format (.3dcraft)

See [`file-formats.md`](file-formats.md). In short: a ZIP of STORE entries (or a directory with the same layout) holding `manifest.json` (the document: variables, sketches, constraints, features, order), content-addressed `blobs/` for imported bodies and optional regeneration caches, and `thumb.png`. Saves are atomic (write a temp file, fsync, rename). Autosave writes snapshots to a recovery directory on a background thread.

## Graphics startup and device loss

`--safe-gpu` starts with the CPU renderer for one launch. If the wgpu device cannot be created, or is lost while running, the viewport switches to the CPU renderer for the rest of the session and a notice says so. The app must start and show a model on a machine with no usable GPU.

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
- Export the smallest failing case as a test in the crate that owns the bug (`crates/<name>/tests/regressions.rs`), not as a file in the repo.

## Web build *(later milestone)*

`apps/craft3d-web` will run the same app in the browser through eframe's web runner (WebGPU, WebGL2 fallback), built with trunk; the only JavaScript is the glue wasm-bindgen generates. Until then `cargo xtask wasm` keeps L0–L7 compiling for `wasm32-unknown-unknown`.

## Dev log

`log/devlog.md` (gitignored) gets a terse entry at the end of every task: date, what landed, numbers (tests, timings with machine and load), what is still open. It is how the next agent session picks up.
