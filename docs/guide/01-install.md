# 1. Install and connect an agent

## Build the two programs

There are no installers yet: 3DCraft is built from source. You need [Rust](https://rustup.rs) (rustup
installs the pinned toolchain by itself on the first build) and, on Linux, the windowing packages:

```sh
# Fedora
sudo dnf install libxkbcommon-devel wayland-devel libX11-devel libXrandr-devel libXi-devel mesa-libGL-devel gtk3-devel
# Debian, Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev

git clone https://github.com/johancoppens/3dcraft
cd 3dcraft
cargo build --release -p craft3d -p craft3d-cli
```

This gives two programs in `target/release/`:

- **`craft3d`**, the window. It shows a document and lets you look at it, click on it and open and save
  files. It does not model by itself: models change through commands.
- **`craft3d-cli`**, the headless tool: it runs commands, renders pictures, converts files and serves
  MCP (`craft3d-cli --help`, `craft3d-cli <subcommand> --help`). It needs no graphics card.

Check that it works:

```sh
target/release/craft3d-cli commands --filter feature.block
target/release/craft3d-cli run --new '{"units":"mm"}' \
    --cmd feature.block --params '{"size":["40 mm","20 mm","10 mm"]}' --out box.3dcraft
target/release/craft3d-cli render box.3dcraft --out box.png
target/release/craft3d box.3dcraft
```

The window needs a graphics adapter (a GPU, or a software one such as Mesa's lavapipe); everything in
`craft3d-cli` runs on any machine.

## Connect an AI agent over MCP

`craft3d-cli mcp` is an MCP server on standard input and output, so any MCP client that starts local
(stdio) servers can use it. There are two ways to run it.

**Headless** (the agent has a document of its own; you look at pictures it renders):

```sh
craft3d-cli mcp --automation-read-root /path/to/work --automation-write-root /path/to/work
```

**Bridged to the window** (the agent models in the window you are watching: recommended):

```sh
# 1. start the window with a control port and a private token file (created if missing)
target/release/craft3d --control 7878 --control-token-file ~/.config/craft3d-control.token \
    --automation-read-root /path/to/work --automation-write-root /path/to/work
# 2. the MCP server your client starts passes every message to that window
craft3d-cli mcp --bridge 7878 --control-token-file ~/.config/craft3d-control.token
```

The read and write roots are the only directories a command may read or write files in (`file.open`,
`file.save`, `file.export`, `file.import`, `view.render`); a path outside them, or any path when a root is
not given, is refused. Give the agent a working directory of its own. With `--bridge` the roots are the
window's.

Most MCP clients take a JSON configuration of this shape (the file and its location depend on the
client; use absolute paths):

```text
{
  "mcpServers": {
    "craft3d": {
      "command": "/path/to/3dcraft/target/release/craft3d-cli",
      "args": ["mcp", "--bridge", "7878", "--control-token-file", "/home/me/.config/craft3d-control.token"]
    }
  }
}
```

For a headless server, put `"args": ["mcp", "--automation-read-root", "/path/to/work",
"--automation-write-root", "/path/to/work"]` instead. One concrete example, for a client with a command
line to add servers (Claude Code):

```sh
claude mcp add craft3d -- /path/to/3dcraft/target/release/craft3d-cli mcp \
    --bridge 7878 --control-token-file /home/me/.config/craft3d-control.token
```

The server gives the agent these tools:

| Tool | What it does |
|---|---|
| `command_list` | The commands, with their parameters and a summary each (`filter` narrows it) |
| `command_run` | Runs one command: `{"id": "feature.block", "params": {...}}` |
| `command_batch` | Runs up to 256 commands in order (`terse: true` returns only failures and the last result) |
| `doc_inspect` | The whole document: variables, features with their status, bodies with volume, area, faces |
| `doc_render_preview` | A picture of the model (`view`, `projection`, `box`, `zoom`, `mode: "ids"`) |
| `selection_get` | What is selected, including what you clicked in the window |
| `control_call` | Bridged only: a method of the window (`ui.get`, `ui.highlight`, `ui.screenshot`, …) |

The server also tells the client, when it connects, how to start (`command_list`, then `file.new`). An
agent needs nothing else: the command summaries and the error messages are written to be read by it.

If the window is not running, a bridged server answers every request with an error that says how to
start it, and keeps serving: start the window and ask again.
