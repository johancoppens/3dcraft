# The 3DCraft user guide

> **Last updated:** 2026-10-23, for the first release (R1). Every command named here exists, and every
> example model is rebuilt and checked by a test (`crates/engine/tests/guide_examples.rs`).

3DCraft is a parametric 3D CAD modeller in which **every action is a command** with an id and JSON
parameters. The same commands are run by an AI agent over MCP, by a script on the command line, or by a
program over a JSON channel; the window shows the model as it is built. This guide is written for the way
3DCraft is meant to be used first: **you describe a part, an AI agent builds it with 3DCraft's commands,
and you watch, point and check**. Building by hand (writing the commands yourself) is covered too, briefly.

You need no knowledge of Rust or of 3DCraft's source to read this guide.

| Chapter | What it covers |
|---|---|
| [1. Install and connect an agent](01-install.md) | Build the programs, connect an MCP client (headless, or bridged to the window) |
| [2. Working with an agent](02-agent.md) | What to ask, a full sample session, reading the agent's results, checking its work |
| [3. Watching and pointing](03-window.md) | The window: viewport, view cube, feature tree, clicking faces the agent can read |
| [4. Concepts](04-concepts.md) | History, variables and units, sketches and constraints, features, persistent names, failed features |
| [5. A first part, step by step](05-first-part.md) | An L-bracket with a rounded corner and two holes, command by command |
| [6. Modelling features](06-features.md) | Booleans, fillets and chamfers, shell, patterns, revolve, sweep and loft, threads, sheets, datums, Jigs and their parameters |
| [7. Measuring and assertions](07-measure.md) | `measure.*` queries and `assert.*` design rules checked at every regeneration |
| [8. Selection and named sets](08-selection.md) | `select.query` filters, the shared selection, sets kept in the document |
| [9. Files](09-files.md) | `.3dcraft` and its promise, STEP import and export, STL/3MF/OBJ, DXF/SVG for laser cutting |
| [10. Without an agent](10-by-hand.md) | `craft3d-cli run` and scripts, the JSON channel |
| [11. Troubleshooting](11-troubleshooting.md) | What refusals mean, failed features, undo, crash recovery |

The [example models](../examples/README.md) are small command scripts, each with a picture; every one of
them asserts its own volume, so you can see what a checked model looks like.
