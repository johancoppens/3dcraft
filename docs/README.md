# 3DCraft documentation

> **Last updated:** 2026-10-23 (0.1.0) · **Status:** see [`roadmap.md`](roadmap.md) › *Current focus* and *Milestones* for what is done and open, and [`architecture.md`](architecture.md) for how it is built (the map; the details per part in [`design/`](design/README.md))

**Using 3DCraft?** Start with the [user guide](guide/README.md): what 3DCraft is, installing it, a first model in five minutes, working with an AI agent, Jigs, and the reference. The [example models](examples/README.md) are small parts that check their own volume. What changed per version is in [`CHANGELOG.md`](../CHANGELOG.md).

**Working on 3DCraft?** Start with [`CONTRIBUTING.md`](../CONTRIBUTING.md) (how to report a problem and send a change) and [`AGENTS.md`](../AGENTS.md), then read in this order.

| Doc | What it holds |
|---|---|
| [`guide/`](guide/README.md) | The user guide: start here, install and connect an agent, first steps, working with an agent, the window, Jigs, concepts, a first part, formulas, features, measuring, selection, files, the command line, troubleshooting, reference |
| [`examples/`](examples/README.md) | Eighteen example models as command scripts, each with a picture and its volume asserted; thirteen of them are cards on the start screen, beside the published Jigs and assemblies in `examples/published/` |
| [`architecture.md`](architecture.md) | The map: the rules that always hold, crates and layers, data, pipelines, the kernel's methods, known structural debt |
| [`design/`](design/README.md) | One detail document per part (geometry, topology, booleans, blends, sweeps, face operations, developable surfaces, surfacing, names, document, regeneration, tessellator, rendering, engine, motion, selection, origami, verification, sections and levels, plates, sheet metal, cutting, assemblies, STEP import, text, files): how it works now, then its dated findings |
| [`development.md`](development.md) | Prerequisites, build and run, checks and the gate on the Strix, CLI, driving the app, watching along, packaging, the web build, example thumbnails, environment variables |
| [`testing.md`](testing.md) | The AI test loop, the oracles, test kinds per crate, the task set, the guide's checked examples, performance and web-size budgets, evidence |
| [`contributing.md`](contributing.md) | Rules and the "add a command" / "add a feature type" / "add a geometry kind" checklists |
| [`control-protocol.md`](control-protocol.md) | JSON control channel, headless server, MCP tools, every `ui.*` method |
| [`file-formats.md`](file-formats.md) | `.3dcraft`, `.3dcraftjig` and `.3dcraftasm`, STEP, FreeCAD documents, STL, 3MF, OBJ, DXF and SVG: scope, specs, the format promise, tests |
| [`roadmap.md`](roadmap.md) | Current focus, one line per milestone, a section per milestone (Goal, Done, Open, Acceptance), the boolean checklist, ideas, projects on 3DCraft |
| [`decisions.md`](decisions.md) | Decisions taken and open |
| [`tasks/`](tasks/) | The task set: 22 ordinary parts with descriptions in English and Dutch, closed-form expectations and reference solutions, scored by `craft3d-cli score` ([`testing.md`](testing.md) › *The task set*) |
| [`abc-scorecard.md`](abc-scorecard.md) | The latest scorecard of real-world STEP models (one chunk of the ABC dataset): ids and results only, made by `cargo xtask abc` ([`testing.md`](testing.md) › *Real-world models*) |
| [`showcase/`](showcase/) | Larger models built by agents as stress tests of the kernel (`.3dcraft` files, their build steps and pictures); several tests open them |
| [`field-test/2026-10-12/`](field-test/2026-10-12/README.md) | What an agent that only used 3DCraft over MCP built, and what it ran into |
| [`field-test/2026-10-22/`](field-test/2026-10-22/README.md) | The second field test: history, conflicting sketches, every constraint kind, negative values, units, a change request and the guide taken literally; what was fixed and what is open |

The docs describe the **target design**, and each says at its top what is built today. Where a doc says *(planned)*, it is a specification for the milestone that builds it, not a description of something you can run.
