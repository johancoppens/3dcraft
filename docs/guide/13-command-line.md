# 13. The command line and scripts

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
fails, saves nothing and exits with code 1. With `--keep-going` it carries on, prints
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

## Pictures: `render`

```sh
craft3d-cli render bracket.3dcraft --out bracket.png --size 800x600 --view iso
```

Draws the bodies into a PNG, on the window's light background (`--background dark` for a dark one).
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

For a DXF or SVG file, `convert` needs `--sketch`, `--face` or `--set`. A drawing of the model as seen
([chapter 12](12-files.md#drawings-of-the-model-as-seen)) is made with the command `file.export` instead:

```sh
craft3d-cli run bracket.3dcraft --cmd file.export --params '{"path": "bracket.svg", "view": "iso", "hidden": "dashed"}'
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
and checks each as `document.check` does. It lists every failing point with the values that reproduce
it, and exits with 1 when there is one (2 a usage mistake, 3 the file cannot be checked, 4 stopped by
`--time-limit-ms` before the end). `--deep` checks each point more thoroughly and takes longer.

## A video of the history: `replay`

```sh
craft3d-cli replay bracket.3dcraft --out build.mp4
```

Makes a video (MP4 or GIF) of the design being built, feature by feature. It needs `ffmpeg`; without it,
PNG frames.

## For programs: `serve` and `mcp`

- `craft3d-cli mcp` is the MCP server for AI agents ([chapter 1](01-install.md#connect-an-ai-agent-over-mcp)).
- `craft3d-cli serve` speaks a simple JSON-lines protocol on standard input and output, for your own
  programs: `{"id": 1, "method": "engine.execute", "params": {"command": "feature.block", "params": {...}}}`.
  The window speaks the same protocol on a TCP port (`craft3d --control`), with a token;
  [`control-protocol.md`](../control-protocol.md) describes it.
- `craft3d-cli score TASK --script FILE` scores a script against a task of 3DCraft's task set
  ([`docs/tasks/`](../tasks/)), the benchmark 3DCraft is measured by.

**Next:** [Troubleshooting](14-troubleshooting.md).
