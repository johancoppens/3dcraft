# 1. Install 3DCraft and connect an AI agent

*Get 3DCraft running on your computer (or in your browser) and give your AI agent access to it.*

## What you get

3DCraft comes as two programs, and every installer has both:

- **`craft3d`**, the **window**. It shows your design, lets you look around, click on faces, play the
  history, set a Jig's parameters, and open, save and export files.
- **`craft3d-cli`**, the **command line**. It runs commands and scripts, renders pictures, converts files,
  and is the **MCP server** your AI agent connects through. It needs no graphics card.

There is also a **web version**: 3DCraft in your browser, with nothing to install (see
[below](#in-the-browser-nothing-to-install)).

## Download

Download the installer for your computer from the release page you were given. The files carry the
version number, here 0.1.0:

| Your computer | Download | Where `craft3d-cli` ends up |
|---|---|---|
| Windows (64-bit) | `3DCraft-0.1.0-x86_64.msi`, or `3dcraft-0.1.0-x86_64-windows.zip` | `C:\Program Files\3DCraft\craft3d-cli.exe` (on your `PATH`), or the unzipped folder |
| macOS (Apple Silicon: M1 or later) | `3DCraft-0.1.0-aarch64.dmg` | `/Applications/3DCraft.app/Contents/MacOS/craft3d-cli` |
| Linux (64-bit Intel or AMD) | `3DCraft-0.1.0-x86_64.AppImage`, or `3dcraft-0.1.0-x86_64-linux.tar.gz` | the AppImage itself (`… cli`), or `~/.local/bin/craft3d-cli` |
| Any computer, nothing to install | the web version | none: the web version has no MCP |

### Why your computer warns you

This first release is **not signed**: 3DCraft does not yet have the certificates from Apple and Microsoft
that tell your computer who made a program. So the first time you open it, Windows or macOS says it
does not know the program and stops it. That is expected, and you only have to get past it once. The
steps below are all it takes; nothing is switched off for other programs.

## Windows

1. Double-click `3DCraft-0.1.0-x86_64.msi`.
2. A blue box appears: **"Windows protected your PC"**. Click **More info**, check that the app is
   `3DCraft-0.1.0-x86_64.msi` with publisher *Unknown publisher*, and click **Run anyway**.
3. The installer may ask for an administrator's permission (it installs for all users into
   `C:\Program Files\3DCraft`).

3DCraft is now in the Start menu, `.3dcraft` and `.3dcraftjig` files open with it, and `craft3d-cli`
works in a new terminal window.

> **Prefer not to install?** Unzip `3dcraft-0.1.0-x86_64-windows.zip` anywhere and start `craft3d.exe`
> from the folder. SmartScreen asks the same question the first time (More info › Run anyway).

## macOS

1. Open `3DCraft-0.1.0-aarch64.dmg` and drag **3DCraft** onto the **Applications** folder beside it.
2. Open 3DCraft from Applications. macOS refuses the first time, with a message such as *"3DCraft"
   cannot be opened because Apple cannot check it for malicious software*, or *"3DCraft" Not Opened*.
   Click **Done** (or **OK**). Do not move it to the Bin.
3. Allow it once, in one of two ways:
   - **System Settings › Privacy & Security**: scroll down to *"3DCraft" was blocked to protect your
     Mac* and click **Open Anyway**, then confirm with your password and click **Open**. This works on
     every recent macOS, and it is the only way on macOS 15 (Sequoia) and later.
   - On macOS 14 (Sonoma) and earlier you can instead **right-click** (or Control-click) 3DCraft in
     Applications, choose **Open**, and click **Open** in the question that follows.

From then on 3DCraft opens normally.

> **Terminal users:** `xattr -dr com.apple.quarantine /Applications/3DCraft.app` does the same as
> step 3. To use `craft3d-cli` by name, link it onto your `PATH`:
> `ln -s /Applications/3DCraft.app/Contents/MacOS/craft3d-cli /usr/local/bin/`.

## Linux

Linux does not warn about unsigned programs, but a downloaded file may not run until you allow it.
Both packages need a Linux from about 2022 or newer (glibc 2.35, as in Ubuntu 22.04).

**AppImage** (one file, nothing installed):

1. Make it executable: in a file manager, right-click › Properties › Permissions › *Allow executing file
   as program*; or in a terminal:

   ```sh
   chmod +x 3DCraft-0.1.0-x86_64.AppImage
   ```

2. Start it: double-click it, or `./3DCraft-0.1.0-x86_64.AppImage` for the window and
   `./3DCraft-0.1.0-x86_64.AppImage cli --version` for the command line. A link named `craft3d-cli` that
   points at the AppImage starts the command line too.

If it complains about FUSE, install your distribution's `fuse` (or `libfuse2`) package, or run it with
`APPIMAGE_EXTRACT_AND_RUN=1`.

**Tarball** (installs for your user into `~/.local`, with a menu entry and the `.3dcraft` file type):

```sh
tar -xzf 3dcraft-0.1.0-x86_64-linux.tar.gz
cd 3dcraft-0.1.0-x86_64-linux
./install.sh            # or ./install.sh /some/prefix; uninstall.sh removes it again
```

Make sure `~/.local/bin` is on your `PATH` (most distributions do this already).

## In the browser: nothing to install

The web version is 3DCraft as a web page. Open the address you were given in a recent Chrome, Edge,
Firefox or Safari. It can:

- open, show, save, import and export `.3dcraft`, `.3dcraftjig` and STEP files (drop a file on the page,
  or use the File menu);
- **use Jigs**: set a Jig's parameters and export the result, so you can make a laser-cut box or a
  3D-printable pulley without installing anything ([chapter 5](05-jigs.md));
- build the example models from its start screen.

It has no MCP and no command line, so **an AI agent cannot drive it yet**: for working with an agent,
install one of the programs above. The web version keeps nothing on a server: files are read from and
written to your computer ([chapter 12](12-files.md#files-in-the-web-version)). It draws with WebGPU where the
browser has it and WebGL2 otherwise.

## Check that it works

Start **3DCraft** from your Start menu, Applications folder or app launcher. The window opens on the
start screen. Click an example card, such as **L-bracket**: it is built in a new tab. That is all it
takes; [chapter 2](02-first-steps.md) shows you around.

To check the command line too, open a terminal and type (write the full path of `craft3d-cli` from the
table above when it is not on your `PATH`):

```sh
craft3d-cli --version
craft3d-cli run --new '{"units":"mm"}' \
    --cmd feature.block --params '{"size":["40 mm","20 mm","10 mm"]}' --out box.3dcraft
craft3d-cli render box.3dcraft --out box.png
```

This makes a 40 × 20 × 10 mm block, saves it as `box.3dcraft` and draws it into `box.png`.

> **The window needs a graphics adapter**: a graphics card, or a software one such as Mesa's lavapipe on
> Linux. Without one it says so when it starts. Everything in `craft3d-cli` runs on any computer.

## Connect an AI agent over MCP

An AI agent works with 3DCraft through **MCP** (the Model Context Protocol), which most AI tools support:
Claude Code, Claude Desktop and other MCP clients. `craft3d-cli mcp` is the MCP server; your AI tool
starts it, and through it the agent can run every 3DCraft command.

There are two ways to connect. The first is the one to use: **the agent models in the window you are
watching**.

### Bridged to the window (recommended)

1. **Allow AI agents.** Click the agent pill at the right end of the status bar (it says **Agents
   off**; on the start screen the same panel is under **Connect an AI agent**, and **Help › Connect an
   AI agent…** opens it too) and turn on the switch beside **Connect an AI agent**. 3DCraft remembers
   your choice for the next start. The pill then says **Waiting for an agent**.
2. **Add 3DCraft to your AI tool, once.** With the switch on, the panel shows what to copy, under
   **Connect**:
   - for **Claude Code**, click **Copy command** and paste the line into a terminal:

     ```sh
     claude mcp add 3dcraft -- /full/path/to/craft3d-cli mcp --bridge
     ```

   - for **Claude Desktop**, **Cursor** and other tools configured with JSON, click **Copy MCP
     configuration** and paste it into the tool's configuration (Claude Desktop calls it
     `claude_desktop_config.json`, Cursor `~/.cursor/mcp.json`):

     ```text
     {
       "mcpServers": {
         "3dcraft": {
           "command": "/full/path/to/craft3d-cli",
           "args": ["mcp", "--bridge"]
         }
       }
     }
     ```

   - **Other clients** has a ready configuration for each of **Claude Desktop**, **Cursor**, **VS Code**
     (its `.vscode/mcp.json` takes `"servers"` with `"type": "stdio"`) and **Other (stdio)**, the bare
     command and its arguments for any client that starts a server over stdio; choosing one copies it.

   The path is filled in for your computer (on Linux with the AppImage, the AppImage itself, followed
   by `cli`). Nothing secret is in it: there is no password to copy or keep.
3. **Ask the agent something** ([chapter 3](03-agent.md)): "Make a 40 × 20 × 10 mm block in 3DCraft".
   The block appears in the window. The pill turns indigo, **Claude Code connected**, and while the
   agent works it names each command it runs, with a turning mark (**Claude Code · feature.block**),
   so you always see when it acts. Click the pill to see who is connected and since when, to
   **Disconnect all**, or to turn agents off.

> **Which files can the agent touch?** Only the folders the panel names: the **Work folder**,
> `~/Documents/3DCraft` (made the first time; **Change…** picks another folder, which
> 3DCraft remembers, **Open folder** shows it), and, while **Also the open document's folder** is on,
> the folder of the document you have open, so the agent can export next to it. `file.open`,
> `file.save`, `file.export`, `file.import` and `view.render` refuse any other path, also through a
> shortcut (a symbolic link) that leads out of those folders. Everything you open or save yourself from
> the File menu stays yours to choose.

**3DCraft asks you first.** When your AI tool connects from a project folder (Claude Code opened in
`~/projects/bracket`, say), the pill turns amber and 3DCraft asks: *"Claude Code wants to connect. It is
working in this folder. 3DCraft can let it read and write files there, besides the work folder"*, with the
folder underneath. **Allow this folder** allows it until you close 3DCraft, or for every agent from now
on when **Remember for this folder** is ticked (it then appears in the panel, where **Remove** takes it
back); **Only the work folder** and **Refuse** keep it out, and the agent stays connected with the work
folder only. The buttons wait a moment before they take a click, so a click meant for something else
never answers.
Then "make a new document with a 10 mm cube and save it in our folder" just works. When the agent wants
a file anywhere else, 3DCraft shows the file, *"Claude Code asks to write a file"*, with **Deny**,
**Allow this folder** (for every agent from now on) and **Allow once** (that file, for that one request);
the agent waits for you (up to two minutes) and is told when you say no. Only you can answer these
questions: the agent cannot answer them for itself. 3DCraft never
offers your home folder itself, the folders of your keys, settings and programs, or the system's folders.
The folder of the document you have open counts only for a document you opened or saved yourself.

If no window allows agents, the agent's requests get an answer that says to turn on **Allow AI agents**,
and it can carry on as soon as you do. With several 3DCraft windows open, the agent works in the one you
used last (click into it before you ask).

**Why a switch, and is it safe?** With the switch off, 3DCraft does not listen at all. With it on, it
listens only to programs on your own computer, and only to those that know a secret token which 3DCraft
keeps in a file that only you can read; your AI tool's `craft3d-cli` reads it there by itself. A web page
open in your browser cannot drive 3DCraft: it cannot read that file, and 3DCraft ignores web requests.

### Headless (no window)

The agent gets a document of its own and shows you pictures it renders:

```sh
craft3d-cli mcp --automation-read-root /path/to/work --automation-write-root /path/to/work
```

In the client's configuration, put `"args": ["mcp", "--automation-read-root", "/path/to/work",
"--automation-write-root", "/path/to/work"]`. Useful on a server, or to let the agent work while you do
something else; open the file it saved in the window afterwards.

### What the agent gets

The server gives the agent these tools, and tells it how to start when it connects:

| Tool | What it does |
|---|---|
| `command_list` | The commands, with their parameters and a summary each (`filter` narrows it; `brief: true` gives the first sentence of each summary, `id` one command whole) |
| `command_run` | Runs one command: `{"id": "feature.block", "params": {...}}` |
| `command_batch` | Runs up to 256 commands in order (`terse: true` returns only failures and the last result) |
| `doc_inspect` | The whole document: variables, features with their status, bodies with volume, area, faces |
| `doc_render_preview` | A picture of the model (`view`, `projection`, `box`, `zoom`, `mode: "ids"`) |
| `selection_get` | What is selected, including what you clicked in the window |
| `control_call` | Bridged only: a method of the window (`ui.get`, `ui.highlight`, `ui.screenshot`, …) |

The agent needs nothing else: the command summaries and the error messages are written for it to read.

## Build from source

To build 3DCraft yourself you need [Rust](https://rustup.rs) (rustup installs the right toolchain by
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

**Next:** [Your first five minutes](02-first-steps.md).
