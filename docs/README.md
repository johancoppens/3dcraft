# 3DCraft documentation

> **Last updated:** 2026-10-15 · **Status:** see [`roadmap.md`](roadmap.md) › *Current focus* and *Milestones* for what is done and open, and [`architecture.md`](architecture.md) for how it is built (the map; the details per part in [`design/`](design/README.md))

**Using 3DCraft?** Read the [user guide](guide/README.md) and the [example models](examples/README.md). To work on 3DCraft, start with [`AGENTS.md`](../AGENTS.md), then read in this order.

| Doc | What it holds |
|---|---|
| [`guide/`](guide/README.md) | The user guide: install, working with an agent over MCP, the window, concepts, features, measuring, files, troubleshooting |
| [`examples/`](examples/README.md) | Nine example models as command scripts, each with a picture and its volume asserted |
| [`architecture.md`](architecture.md) | The map: the rules that always hold, crates and layers, data, pipelines, the kernel's methods, known structural debt |
| [`design/`](design/README.md) | One detail document per part (geometry, topology, booleans, blends, sweeps, face operations, names, document, regeneration, tessellator, rendering, engine, files): how it works now, then its dated findings |
| [`development.md`](development.md) | Prerequisites, build and run, CLI, driving the app, watching along, environment variables |
| [`field-test/2026-10-12/`](field-test/2026-10-12/README.md) | What an agent that only used 3DCraft over MCP built, and what it ran into |
| [`testing.md`](testing.md) | The AI test loop, the oracles, test kinds per crate, evidence |
| [`contributing.md`](contributing.md) | Rules and the "add a command" / "add a feature type" checklists |
| [`control-protocol.md`](control-protocol.md) | JSON control channel, headless server, MCP tools |
| [`file-formats.md`](file-formats.md) | `.3dcraft`, STEP, STL, 3MF, OBJ: scope, specs, tests |
| [`roadmap.md`](roadmap.md) | Current focus, one line per milestone, a section per milestone (Goal, Done, Open, Acceptance), the boolean checklist, ideas, projects on 3DCraft |
| [`decisions.md`](decisions.md) | Decisions taken and open |

The docs describe the **target design**, and each says at its top what is built today. Where a doc says *(planned)*, it is a specification for the milestone that builds it, not a description of something you can run.
