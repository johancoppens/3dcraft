# 1. Install and connect an agent

## Install 3DCraft

3DCraft is two programs:

- **`craft3d`**, the window. It shows a document and lets you look at it, click on it and open and save
  files. It does not model by itself: models change through commands.
- **`craft3d-cli`**, the headless tool: it runs commands, renders pictures, converts files and serves
  MCP, the connection your AI agent uses (`craft3d-cli --help`, `craft3d-cli <subcommand> --help`). It
  needs no graphics card.

Every installer has both. Download the one for your computer from the release page you were given
(the files are named after the version, here 0.1.0):

| Your computer | Download | Where `craft3d-cli` ends up |
|---|---|---|
| Linux (64-bit Intel or AMD) | `3DCraft-0.1.0-x86_64.AppImage`, or `3dcraft-0.1.0-x86_64-linux.tar.gz` | the AppImage itself (`… cli`), or `~/.local/bin/craft3d-cli` |
| macOS (Apple Silicon: M1 or later) | `3DCraft-0.1.0-aarch64.dmg` | `/Applications/3DCraft.app/Contents/MacOS/craft3d-cli` |
| Windows (64-bit) | `3DCraft-0.1.0-x86_64.msi`, or `3dcraft-0.1.0-x86_64-windows.zip` | `C:\Program Files\3DCraft\craft3d-cli.exe` (on `PATH`), or the unzipped folder |
| Any computer, nothing to install | the web version (below) | none: the web version has no MCP |

### Why your computer warns you

This first release is **not signed**: we have not yet bought the certificates from Apple and Microsoft
that tell your computer who made a program. So the first time you open 3DCraft, Windows or macOS
says it does not know the program and stops it. That is expected, and you only have to get past it
once. The steps below are all it takes; nothing is switched off for other programs.

### Windows

1. Double-click `3DCraft-0.1.0-x86_64.msi`.
2. A blue box appears: **"Windows protected your PC"** (Microsoft Defender SmartScreen prevented an
   unrecognised app from starting). Click **More info**, check that the app is `3DCraft-0.1.0-x86_64.msi`
   with publisher *Unknown publisher*, and click **Run anyway**.
3. The installer may ask for an administrator's permission (it installs for all users into
   `C:\Program Files\3DCraft`). Then 3DCraft is in the Start menu, `.3dcraft` files open with it, and
   `craft3d-cli` works in a new terminal window.

Prefer not to install? Unzip `3dcraft-0.1.0-x86_64-windows.zip` anywhere and start `craft3d.exe` from
the folder; SmartScreen asks the same question the first time (More info › Run anyway).

### macOS

1. Open `3DCraft-0.1.0-aarch64.dmg` and drag **3DCraft** onto the **Applications** folder beside it.
2. Open 3DCraft from Applications. macOS refuses the first time, with a message such as *"3DCraft"
   cannot be opened because Apple cannot check it for malicious software*, or *"3DCraft" Not Opened*.
   Click **Done** (or **OK**); do not move it to the Bin.
3. Allow it once, in one of two ways:
   - **System Settings › Privacy & Security**: scroll down to *"3DCraft" was blocked to protect your
     Mac* and click **Open Anyway**, then confirm with your password and click **Open**. This works on
     every recent macOS, and it is the only way on macOS 15 (Sequoia) and later.
   - On macOS 14 (Sonoma) and earlier you can instead **right-click** (or Control-click) 3DCraft in
     Applications, choose **Open**, and click **Open** in the question that follows.

From then on 3DCraft opens normally. If you prefer the terminal, this does the same as step 3:
`xattr -dr com.apple.quarantine /Applications/3DCraft.app`. To use `craft3d-cli` by name, link it onto
your `PATH`, for example `ln -s /Applications/3DCraft.app/Contents/MacOS/craft3d-cli /usr/local/bin/`.

### Linux

Linux does not warn about unsigned programs, but a downloaded file is not allowed to run until you say
so.

- **AppImage** (one file, nothing installed): make it executable, then start it.

  ```sh
  chmod +x 3DCraft-0.1.0-x86_64.AppImage
  ./3DCraft-0.1.0-x86_64.AppImage                # the window
  ./3DCraft-0.1.0-x86_64.AppImage cli --version  # craft3d-cli
  ```

  In a file manager: right-click › Properties › Permissions › *Allow executing file as program*. A link
  named `craft3d-cli` that points at the AppImage starts the command line too. If it complains about
  FUSE, install your distribution's `fuse` (or `libfuse2`) package, or run it with
  `APPIMAGE_EXTRACT_AND_RUN=1`.
- **Tarball** (installs for your user into `~/.local`, with a menu entry and the `.3dcraft` file type):

  ```sh
  tar -xzf 3dcraft-0.1.0-x86_64-linux.tar.gz
  cd 3dcraft-0.1.0-x86_64-linux
  ./install.sh            # or ./install.sh /some/prefix; uninstall.sh removes it again
  ```

  Make sure `~/.local/bin` is on your `PATH` (most distributions do this already).

Both need a Linux from about 2022 or newer (glibc 2.35, as in Ubuntu 22.04).

### In the browser: nothing to install

The web version is 3DCraft as a web page: open the address you were given in a recent Chrome, Edge,
Firefox or Safari. It opens, shows, saves, imports and exports `.3dcraft` and STEP files (drop a file on
the page, or use the File menu), so it is the quickest way to look at a model someone sent you. It has
no MCP and no command line, so **an AI agent cannot drive it**: for working with an agent, install one
of the programs above. Files you open stay in the page's memory: save before you close the tab. It
draws with WebGPU where the browser has it and WebGL2 otherwise (add `?webgl` to the address to force
WebGL2).

### Build from source

To build 3DCraft yourself you need [Rust](https://rustup.rs) (rustup installs the pinned toolchain by
itself on the first build) and, on Linux, the windowing packages:

```sh
# Fedora
sudo dnf install libxkbcommon-devel wayland-devel libX11-devel libXrandr-devel libXi-devel mesa-libGL-devel gtk3-devel
# Debian, Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev

git clone https://github.com/johancoppens/3dcraft
cd 3dcraft
cargo build --release -p craft3d -p craft3d-cli
```

This puts both programs in `target/release/`.

## Check that it works

In a terminal (write the full path of `craft3d-cli` from the table above when it is not on your `PATH`;
for a build from source it is `target/release/craft3d-cli`):

```sh
craft3d-cli --version
craft3d-cli commands --filter feature.block
craft3d-cli run --new '{"units":"mm"}' \
    --cmd feature.block --params '{"size":["40 mm","20 mm","10 mm"]}' --out box.3dcraft
craft3d-cli render box.3dcraft --out box.png
craft3d box.3dcraft
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
craft3d --control 7878 --control-token-file ~/.config/craft3d-control.token \
    --automation-read-root /path/to/work --automation-write-root /path/to/work
# 2. the MCP server your client starts passes every message to that window
craft3d-cli mcp --bridge 7878 --control-token-file ~/.config/craft3d-control.token
```

The window is started from a terminal here, so that it gets these options: `craft3d` is next to
`craft3d-cli` (on macOS `/Applications/3DCraft.app/Contents/MacOS/craft3d`, on Windows
`craft3d.exe`; the AppImage without `cli` is the window).

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
      "command": "/full/path/to/craft3d-cli",
      "args": ["mcp", "--bridge", "7878", "--control-token-file", "/home/me/.config/craft3d-control.token"]
    }
  }
}
```

For a headless server, put `"args": ["mcp", "--automation-read-root", "/path/to/work",
"--automation-write-root", "/path/to/work"]` instead. One concrete example, for a client with a command
line to add servers (Claude Code):

```sh
claude mcp add craft3d -- /full/path/to/craft3d-cli mcp \
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
