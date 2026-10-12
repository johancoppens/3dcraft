# 3DCraft documentation

> **Last updated:** 2026-10-12 · **Status:** M2 done; see [`roadmap.md`](roadmap.md) for the milestones and `architecture.md` › Current state for what exists

Start with [`AGENTS.md`](../AGENTS.md), then read in this order.

| Doc | What it holds |
|---|---|
| [`architecture.md`](architecture.md) | Goals and principles, crate map and layers, document model, kernel, tessellator, renderers, engine seam |
| [`development.md`](development.md) | Prerequisites, build and run, CLI, driving the app, watching along, environment variables |
| [`testing.md`](testing.md) | The AI test loop, the oracles, test kinds per crate, evidence |
| [`contributing.md`](contributing.md) | Rules and the "add a command" / "add a feature type" checklists |
| [`control-protocol.md`](control-protocol.md) | JSON control channel, headless server, MCP tools |
| [`file-formats.md`](file-formats.md) | `.3dcraft`, STEP, STL, 3MF, OBJ: scope, specs, tests |
| [`roadmap.md`](roadmap.md) | Milestones with definition of done, current focus |
| [`decisions.md`](decisions.md) | Decisions taken and open |

The docs describe the **target design**, and each says at its top what is built today. Where a doc says *(planned)*, it is a specification for the milestone that builds it, not a description of something you can run.
