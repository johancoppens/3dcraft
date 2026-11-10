# The command line and scripts

*Everything an agent does, you can do yourself: run commands and scripts, render pictures, convert
files and check a Jig, with `craft3d-cli`.*

`craft3d-cli` runs in a terminal (on Windows: Command Prompt or PowerShell) and needs no window and no
graphics card. `craft3d-cli --help` lists its subcommands, and `craft3d-cli <subcommand> --help`
explains one. Every subcommand returns exit code 0 on success, 1 on a failure and 2 for a mistake in the
command line.

## Run commands: `run`

```sh
craft3d-cli run --new '{"units":"mm"}' \
    --cmd variable.set  --params '{"name":"width","expr":"60 mm"}' \
    --cmd feature.block --params '{"size":["width","width * 2 / 3","8 mm"],"name":"Plate"}' \
    --cmd document.measure \
    --out part.3dcraft
craft3d-cli run part.3dcraft --cmd variable.set --params '{"name":"width","expr":"80 mm"}' --out wider.3dcraft
```

`run` opens a file (or makes a new document with `--new`, the values of `file.new`), runs the commands in
order, prints one line of JSON per result, and saves with `--out`. It stops at the first command that
fails, saves nothing and exits with code 1. A `variable.set` whose expression cannot be worked out (a
name that is not defined, a length added to a plain number) counts as failing, although the variable is
set: everything it drives would fail after it. With `--keep-going` it carries on, prints
`{"failed": id, "step": n, "error": text}` for a failed command, and saves what was built.

> **Quotes on Windows:** in PowerShell and Command Prompt, the JSON in `--params` needs other quoting.
> A script file (below) avoids the problem altogether.

## Scripts

A script is a JSON list of steps, `[["id", {values}], …]`: the format of the
[example models](../examples/README.md) and of the JSON blocks in this guide.

```sh
craft3d-cli run --new '{"units":"mm"}' --script docs/examples/flange.json --out flange.3dcraft
craft3d-cli render flange.3dcraft --out flange.png
```

Edit the numbers at the top of a script (the `variable.set` steps) to get another size. `craft3d-cli
commands` lists every command with its values and a summary; `--filter fillet` narrows it.

Define a variable before a step uses it: a `variable.set` that names a variable not yet defined
cannot be worked out, and the script stops there.

## Pictures: `render`

```sh
craft3d-cli render bracket.3dcraft --out bracket.png --size 800x600 --view iso
```

Draws the bodies into a PNG, on the window's light background (`--background dark` for a dark one), on
the graphics card when there is one (`--renderer cpu` for the CPU renderer).
`--view` is `iso`, `front`, `back`, `left`, `right`, `top` or `bottom`; `--mode ids` gives every face its
own flat colour, to see which face is which.

## Converting files: `convert`

`craft3d-cli convert IN OUT` exports a saved `.3dcraft` file; the extension of `OUT` picks the format:

```sh
craft3d-cli convert bracket.3dcraft bracket.step
craft3d-cli convert bracket.3dcraft bracket.stl --quality fine
craft3d-cli convert panel.3dcraft panel.dxf --sketch Outline
craft3d-cli convert panel.3dcraft panel.svg --face 'F2:endCap()'
craft3d-cli convert box.3dcraft box.svg --set 'Cut sheet' --width '600 mm' --gap '8 mm'
```

A DXF or SVG file with `--sketch`, `--face` or `--set` is a flat outline at true size. Without them it
is a drawing of the model as seen ([Files, export and drawings](files.md#drawings-of-the-model-as-seen)), as
`file.export` makes it: `--view` picks the side (`iso`, the default, `front`, `back`, `left`, `right`,
`top` or `bottom`), `--projection` is `orthographic` (to scale, the default) or `perspective`, and
`--hidden dashed` draws what lies behind a face dashed instead of leaving it out:

```sh
craft3d-cli convert bracket.3dcraft bracket.svg --hidden dashed
craft3d-cli convert bracket.3dcraft bracket-front.dxf --view front
```

## Looking inside a file: `info`

`craft3d-cli info FILE` prints the whole state of a file as JSON, as `document.inspect` gives it: its
variables, features and their status, and every body with its volume, area and faces.

## Check a Jig over its range

```sh
craft3d-cli check-range box.3dcraft --samples 100 --seed 7
```

`check-range` proves a Jig (a document with parameters) over its parameter range: it builds the design
at the defaults, at the corners of the ranges, at every choice and at random points chosen by the seed,
and checks each as `document.check` does. Where a point narrows the range of a parameter it leaves at its
default (a choice that allows fewer holes), that parameter is moved into its new range, as the parameter
form moves it, and that point's line says so (`clamped holes 8 -> 4`); a choice the form does not offer
at the defaults is not tried; a switch whose maximum is another switch stays off while that one is off.

It prints a line per point as soon as that point is checked (so a run you stop still shows what it
found), then the totals: how many points passed, how many have an assertion that is **not proven**
(neither passed nor failed, for instance a clearance between curved faces whose least value could not be
pinned down), and how many failed. It lists every failing point, and every point with an unproven
assertion, with the values that reproduce it. A point with an unproven assertion does not count as
passed: a script that checks for *passed = points* should check *0 failed* instead, or use `--strict`.
It exits with 1 when a point failed (2 a usage mistake, 3 the file cannot be checked or the check was
killed, for instance for lack of memory, 4 stopped by `--time-limit-ms` before the end); `--strict` also fails the
points whose assertions are not proven. `--deep` checks each point more thoroughly and takes longer.

The corners of the ranges (every parameter at its low or high end) are all tried while there are at most
64 of them, whatever `--samples` says: a Jig with six parameters has 64 corners, with eight 256, of
which 64 are tried. For a quick run lower `--max-corners` (`--max-corners 8`) or leave the corners out
with `--no-corners`; the totals say how many of the corners were tried. With `--json` the result is
printed as JSON and the lines per point go to standard error.

## A video of the history: `replay`

```sh
craft3d-cli replay bracket.3dcraft --out build.mp4
```

Makes a video (MP4, WebM or GIF) of the design being built, feature by feature. It needs `ffmpeg`;
without it, PNG frames.

## Animations and videos

The same command renders animations: an exploded view, a mechanism's stroke, a parameter sweep, a
camera flying round the model. `view.replay` with `animate` gives variables *tracks*: each frame sets
them to their value at that moment and rebuilds only what they touch. A track goes `from` a value `to`
another, or through `keys` (`[{"t": 0, "value": 30}, {"t": 0.5, "value": 50}, ...]`, `t` from 0 at the
first frame to 1 at the last), with an `easing` (`linear`, `in`, `out`, `inOut`) and the variable's
`unit`. A track of the timeline `t` plays the document's motion ([Assemblies](assemblies.md) for
assemblies). `camera` moves the camera: `orbit` degrees about the vertical, `dolly` towards the model
(0.5 ends at half the distance), or `keys` of `eye`, `target`, `up` and `fov` it passes through
smoothly. `frames` or `duration` (seconds at `fps`) set the length, `hold` lingers on the last frame,
and `look` and `background` are those of `render`.

A box whose lid lifts off as the camera turns, then the box swept from 30 to 60 mm wide:

```json
[
  ["file.new", {}],
  ["variable.set", {"name": "t", "expr": "0"}],
  ["variable.set", {"name": "width", "expr": "40 mm"}],
  ["feature.block", {"name": "Box", "size": ["width", 30, 20]}],
  ["feature.block", {"name": "Lid", "size": ["width", 30, 3], "position": [0, 0, "20 mm + 25 mm * t"]}],
  ["view.replay", {"dir": "explode", "width": 320, "height": 240, "frames": 12,
                   "animate": [{"variable": "t", "from": 0, "to": 1, "easing": "inOut"}],
                   "camera": {"orbit": 30}}],
  ["view.replay", {"dir": "sweep", "width": 320, "height": 240, "frames": 12, "hold": 6,
                   "animate": [{"variable": "width", "from": 30, "to": 60, "unit": "mm"}]}]
]
```

Each replay writes `frame-00000.png` and on into its `dir`, with `captions.srt` (each frame's values),
and leaves the document as it was. `"video": "explode.mp4"` (or `.webm`, `.gif`) makes the video
straight away when `ffmpeg` is installed; otherwise the command says so before it draws anything, and
the PNG sequence is the way. From the command line, at full HD in the studio look:

```sh
craft3d-cli replay box.3dcraft --out explode.mp4 --size 1920x1080 \
    --params '{"animate": [{"variable": "t", "from": 0, "to": 1, "easing": "inOut"}],
               "duration": 4, "look": {}, "camera": {"orbit": 40, "dolly": 0.8}}'
```

Pictures and frames are drawn on the graphics card when the machine has one (`"renderer": "auto"`, the
default; the result says `"renderer": "gpu"` or `"cpu"`); `"renderer": "cpu"` (`--renderer cpu`) draws
them with the CPU renderer, the reference the card is checked against, and slower. A studio frame
averages 16 frames of samples (`look.accumulate`), as the window does before its picture is settled.

## For programs: `serve` and `mcp`

- `craft3d-cli mcp` is the MCP server for AI agents ([Install 3DCraft and connect an AI agent](install.md#connect-an-ai-agent-over-mcp)).
- `craft3d-cli serve` speaks a simple JSON-lines protocol on standard input and output, for your own
  programs: `{"id": 1, "method": "engine.execute", "params": {"command": "feature.block", "params": {...}}}`.
  The window speaks the same protocol on a TCP port (`craft3d --control`), with a token;
  [`control-protocol.md`](../control-protocol.md) describes it.
- `craft3d-cli score TASK --script FILE` scores a script against a task of 3DCraft's task set
  ([`docs/tasks/`](../tasks/)), the benchmark 3DCraft is measured by.

**Next:** [Troubleshooting](troubleshooting.md).
