# The 0.1.0 field test

*For the people who try 3DCraft 0.1.0 before anyone else: how to install it, connect your AI
agent, five tasks to start with, and how to tell us what you found.*

Thank you for testing. 3DCraft is made to be driven by an AI agent, so that is what we most want to hear
about: what you asked, what the agent built, where it got stuck. Using it by hand and in the browser
counts too. A refusal ("not built", "does not fit") is not a failure of your test: it is exactly what we
are looking for.

## 1. Install

Pick one (details and pictures in [chapter 1](01-install.md)):

| You have | Use | Can an agent drive it? |
|---|---|---|
| Windows (64-bit) | `3DCraft-0.1.0-x86_64.msi` | yes |
| macOS on Apple Silicon (M1 or later) | `3DCraft-0.1.0-aarch64.dmg` | yes |
| Linux (64-bit) | `3DCraft-0.1.0-x86_64.AppImage` | yes |
| A Chromebook, or no rights to install | the web address you were given | not yet |

You get the files or the web address from whoever invited you to the test. The installers are not
signed yet, so Windows and macOS warn the first time: [chapter 1 › Why your computer warns
you](01-install.md#why-your-computer-warns-you) shows the two clicks to get past it.

Check that it works: start 3DCraft and click the **L-bracket** card on the start screen. It is built in a
new tab.

## 2. Connect your AI agent

The agent talks to 3DCraft over MCP (the Model Context Protocol). The program `craft3d-cli` is the MCP
server; your AI tool starts it, and it passes the agent's commands on to the window you are watching.

**Step 1: start the window so that it listens.** The start screen's panel **Connect an AI agent** shows
the command line for your computer, with the full path of the program and of the token file filled in;
copy it from there and run it in a terminal. On a Mac it looks like this (one line):

```sh
/Applications/3DCraft.app/Contents/MacOS/craft3d --control 7878 \
    --control-token-file "/Users/me/Library/Application Support/3DCraft/control.token" \
    --automation-read-root /Users/me/Documents/3DCraft --automation-write-root /Users/me/Documents/3DCraft
```

On Linux with the AppImage, the program is the AppImage itself; on Windows, `craft3d.exe` in
`C:\Program Files\3DCraft`. The folder `3DCraft` in your Documents is the only folder the agent may read and
write files in; the window makes it the first time. The token file is made the first time too. A window
started from the menu does not listen; this one does.

**Step 2: tell your AI tool about the server.** The same panel has **Copy configuration** (the JSON most
tools take) and **Copy for Claude Code**. Paste it as below. In the examples, replace
`/full/path/to/craft3d-cli` and the token file's path by yours (the panel has them filled in):

| Your computer | `craft3d-cli` is |
|---|---|
| Windows | `C:\Program Files\3DCraft\craft3d-cli.exe` (in JSON, write every `\` twice) |
| macOS | `/Applications/3DCraft.app/Contents/MacOS/craft3d-cli` |
| Linux, AppImage | the AppImage itself, with `"cli"` as the first of the `args` (the panel does this) |
| Linux, tarball | `~/.local/bin/craft3d-cli`, written out in full |

**Claude Code** (in a terminal):

```sh
claude mcp add craft3d -- /full/path/to/craft3d-cli mcp \
    --bridge 7878 --control-token-file /home/me/.config/craft3d-control.token
```

**Claude Desktop** (Settings › Developer › Edit Config opens `claude_desktop_config.json`), **Cursor**
(`~/.cursor/mcp.json`) and **Gemini CLI** (`~/.gemini/settings.json`) all take the same shape:

```json
{
  "mcpServers": {
    "craft3d": {
      "command": "/full/path/to/craft3d-cli",
      "args": ["mcp", "--bridge", "7878", "--control-token-file",
               "/home/me/.config/craft3d-control.token"]
    }
  }
}
```

**VS Code** with GitHub Copilot's agent mode (`.vscode/mcp.json` in your work folder):

```json
{
  "servers": {
    "craft3d": {
      "type": "stdio",
      "command": "/full/path/to/craft3d-cli",
      "args": ["mcp", "--bridge", "7878", "--control-token-file",
               "/home/me/.config/craft3d-control.token"]
    }
  }
}
```

**OpenAI Codex CLI** (`~/.codex/config.toml`):

```toml
[mcp_servers.craft3d]
command = "/full/path/to/craft3d-cli"
args = ["mcp", "--bridge", "7878", "--control-token-file", "/home/me/.config/craft3d-control.token"]
```

On Windows, TOML strings in double quotes need every `\` written twice, as JSON does:
`command = "C:\\Program Files\\3DCraft\\craft3d-cli.exe"`.

Where a tool keeps its configuration changes between its versions; the shape above is what matters.
Restart the AI tool after changing it.

**Step 3: try it.** Ask: *"Make a 40 × 20 × 10 mm block in 3DCraft."* The block appears in the window. If
the agent says the window is not reachable, the window was not started as in step 1, or the port or token
file differ between step 1 and step 2.

## 3. Five tasks to start with

Links in this guide and in 3DCraft (**How it works**, Help › User guide) open in your web browser. Each
task takes 5 to 20 minutes. Give the agent the text in quotes, in your own words if you like, and in
your own language if you prefer. Then check its work as [chapter 3](03-agent.md#checking-the-agents-work)
shows: look at the window, click faces, and ask for the volume.

**Task 1, without an agent: your first five minutes.** Follow [chapter 2](02-first-steps.md): open an
example from the start screen, turn it around, play its history, change a size in the feature list and
export an STL for a 3D printer. Then do the same in the web version if you have its address.

**Task 2: an L-bracket.** *"In 3DCraft, make an L-shaped angle bracket of 4 mm stock, 25 mm wide: one leg
lies flat on the XY plane and runs 40 mm along +X, the other stands up along +Z to a height of 30 mm,
both legs measured to the outside of the corner, which is at the origin. Drive it by the variables leg_x,
leg_z, thickness and width. Tell me its volume."* The right answer is **6600 mm³**. Then ask it to round
the inside corner with a 4 mm radius, and to make the bracket 30 mm wide: the rounding should follow.

**Task 3: a strip with a counted row of holes.** *"Make a flat strip 120 × 20 × 3 mm (corner at the
origin, along +X, +Y, +Z) with a row of 5 mm holes along its centre line at a 10 mm pitch: as many holes
as fit with every hole centre at least 7.5 mm from the ends, the row centred on the strip. The number of
holes must follow the length."* That is **11 holes and 6552.05 mm³**. Now ask for a strip of 150 mm: **14
holes and 8175.33 mm³**, without the agent rebuilding anything by hand. If you want to see how it can be
done, this is one way, as a script (`craft3d-cli run --new '{"units":"mm"}' --script strip.json --out
strip.3dcraft`, [chapter 13](13-command-line.md)); its two assertions check the volume and the number of
holes at every change:

```json
[
 ["variable.set", {"name": "length", "expr": "120 mm"}],
 ["variable.set", {"name": "width", "expr": "20 mm"}],
 ["variable.set", {"name": "thickness", "expr": "3 mm"}],
 ["variable.set", {"name": "hole_d", "expr": "5 mm"}],
 ["variable.set", {"name": "pitch", "expr": "10 mm"}],
 ["variable.set", {"name": "margin", "expr": "7.5 mm"}],
 ["variable.set", {"name": "n", "expr": "floor((length - 2 * margin) / pitch) + 1"}],
 ["feature.block", {"size": ["length", "width", "thickness"], "name": "Strip"}],
 ["feature.cylinder", {"radius": "hole_d / 2", "height": "thickness", "position": ["(length - (n - 1) * pitch) / 2", "width / 2", 0], "name": "HoleTool"}],
 ["feature.pattern", {"source": "HoleTool", "count": "n", "step": ["pitch", 0, 0], "name": "HoleTools"}],
 ["feature.boolean", {"op": "subtract", "target": "Strip", "tool": "HoleTools", "name": "Perforated"}],
 ["assert.add", {"name": "volume", "measure": "volume", "of": "Perforated", "eq": "(length * width - n * pi / 4 * hole_d^2) * thickness"}],
 ["assert.add", {"name": "holes", "measure": "count", "of": {"kind": "face", "where": {"hole": true}}, "eq": "n"}]
]
```

**Task 4: a case, and a rule that guards it.** *"Make a moulded case: outside 70 × 50 × 25 mm (corner at
the origin), the top edges and the four upright corners rounded with a 2 mm radius, the bottom edges
sharp, hollowed to a 2 mm wall and open at the bottom. Drive it by variables. Then add an assertion that
the thinnest wall is at least 1.5 mm."* The volume is **17385.50 mm³**. At 2 mm the window may show the
assertion as *not proven* rather than passed: 3DCraft could not prove the bound at the case's sharp
inside corners, and says so instead of guessing. Now ask for a 1 mm wall: the window shows the assertion
failing, and the agent should tell you so ([chapter 10](10-measure.md#assertions)).

**Task 5: make a Jig of it.** *"Publish length and pitch of the strip from task 3 as parameters (length
from 40 to 300 mm, pitch from 8 to 20 mm) and export it as a Jig."* Open the `.3dcraftjig` in 3DCraft
(and in the web version, if you have it): the **Parameters** tab shows the two values; drag them and
watch the holes follow, then export an STL, or a DXF of the top face for a laser cutter ([chapter
5](05-jigs.md)).

After these, **try a part of your own**: something from your workshop, your classroom or your hobby. That
is the test that tells us most.

## 4. Tell us what you found

Write down, for each thing that went wrong or felt wrong: what you were trying to do, what you asked the
agent (copy the prompt), and what happened.

When a command failed, add the **bug report**: click **Copy details** in the status bar (or in the error
dialog of a file action) and paste it; or ask the agent *"give me the bug report"* (it runs
`session.report`). The report holds the 3DCraft version and the exact build it came from, your system,
your graphics card and how 3DCraft draws with it, and the command that failed with its values and the
whole message. **Read it before you share it**: it contains file paths and the values you or your agent
gave. Add the `.3dcraft` file if you can share it. [Chapter 14](14-troubleshooting.md#reporting-a-problem)
has the details.

Send it to whoever invited you to the test, or open an issue on GitHub if you were given access to the
repository. Things that went well are worth a line too: they tell us what not to break.

If 3DCraft ever closes by itself, start it again: it offers to recover your work ([chapter
14](14-troubleshooting.md#after-a-crash)). A crash is the most important thing you can report.

## 5. What 3DCraft sends: nothing

3DCraft has no telemetry, no account, no update check and no licence server; it does not connect to the
internet at all. The port the window listens on for your agent (7878 above) is open only to your own
computer, and only to a program that has the token file. Autosaves stay in 3DCraft's data folder on your
computer (Help › Open data folder). The bug report is copied to your clipboard and goes nowhere unless
you paste it somewhere.

The web version is a page that runs in your browser: it loads its own files from the site that serves
it, and your models are read from and saved to your computer, never uploaded.

**Your AI tool is another matter.** Everything the agent sees (your prompts, the commands it runs, the
numbers and pictures 3DCraft returns) goes to the AI provider your tool uses, under that provider's
terms. Do not give the agent designs you may not share with that provider.
