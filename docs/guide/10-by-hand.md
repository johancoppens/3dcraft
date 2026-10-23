# 10. Without an agent

Everything an agent does, you can do yourself with the same commands.

## `craft3d-cli run`

```sh
craft3d-cli run --new '{"units":"mm"}' \
    --cmd variable.set  --params '{"name":"width","expr":"60 mm"}' \
    --cmd feature.block --params '{"size":["width","width * 2 / 3","8 mm"],"name":"Plate"}' \
    --cmd document.measure \
    --out part.3dcraft
craft3d-cli run part.3dcraft --cmd variable.set --params '{"name":"width","expr":"80 mm"}' --out wider.3dcraft
```

`run` opens a file (or makes a new document with `--new`, the params of `file.new`), runs the commands in
order, prints one line of JSON per result, and saves with `--out`. It stops at the first command that
fails, saves nothing and exits with code 1; with `--keep-going` it carries on, prints
`{"failed": id, "step": n, "error": text}` for a failed command and saves what was built.

## Scripts

A script is a JSON list of steps, `[["id", {params}], …]`, the format of the
[examples](../examples/README.md) and of the JSON blocks in this guide:

```sh
craft3d-cli run --new '{"units":"mm"}' --script docs/examples/flange.json --out flange.3dcraft
craft3d-cli render flange.3dcraft --out flange.png
```

Edit the numbers at the top of a script (the `variable.set` steps) to get another size. `craft3d-cli
commands` lists every command with its parameters and summary (`--filter fillet` narrows it).

## Other subcommands

- `craft3d-cli info FILE`: the state of a file as JSON (as `document.inspect`).
- `craft3d-cli render FILE --out PNG [--size WxH] [--view iso|front|back|left|right|top|bottom]`.
- `craft3d-cli convert IN OUT`: chapter 9.
- `craft3d-cli replay FILE --out build.mp4`: a video of the history built feature by feature (needs
  `ffmpeg`; without it, PNG frames).
- `craft3d-cli score TASK --script FILE`: scores a script against a task of the task set
  (`docs/tasks/`), the benchmark 3DCraft is measured by.
- `craft3d-cli check-range FILE [--samples N] [--seed S] [--deep]`: proves a Jig (a document
  with public parameters) over its parameter range: the defaults, the corners of the ranges and
  seeded random points, each regenerated and checked as `document.check` checks; it lists every
  failing point with the values that reproduce it, and exits with 1 when there is one.
- `craft3d-cli serve`: the JSON-lines protocol on standard input and output, for programs:
  `{"id": 1, "method": "engine.execute", "params": {"command": "feature.block", "params": {...}}}`.
  The window speaks the same protocol on a TCP port (`craft3d --control`), with a token; see
  [`control-protocol.md`](../control-protocol.md).
