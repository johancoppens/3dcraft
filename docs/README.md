# 3DCraft documentation

> **Last updated:** 2026-10-12 · **Status:** M0 in progress; the first crates exist (see `architecture.md` › Current state)

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

Everything here describes the **target design** until the code exists. A doc that says *(planned)* is a specification for the milestone that builds it, not a description of something you can run.

## Relation to PhotoCraft's docs

`../docs_photocraft/` is a local reference copy of PhotoCraft's `docs/` (gitignored; not part of this repository). 3DCraft follows the same principles, so the docs here were derived from it. Do not edit that directory and do not link to it from code.

**Adopted and adapted**

| PhotoCraft doc | Here | What changed |
|---|---|---|
| `architecture.md` | `architecture.md` | Same shape (principles, layered workspace, engine seam, snapshots, CPU oracle + wgpu). Pixels, layers and PSD replaced by B-rep, feature history and STEP. "Avoiding GIMP's hole" became "Avoiding the usual CAD holes" |
| `development.md` | `development.md` | Build/run/test loops, CLI, control channel, MCP, offscreen snapshots, GPU fallback. Added *Watching along* |
| `development.md` › Testing strategy, Test corpora, Rendering fidelity | `testing.md` | The PSD composite oracle has no CAD equivalent; replaced by analytic, topological and round-trip oracles. Corpus rules (pinned, sha256, never committed) kept |
| `contributing.md` | `contributing.md` | Same rules; the command checklist no longer starts from a menu catalogue |
| `control-protocol.md`, `mcp.md` | `control-protocol.md` | Same envelope, auth, capability-scoped file roots and limits; a much smaller method set |
| `file-format-parity.md`, `development.md` › Native format | `file-formats.md` | Native bundle design kept (manifest + content-addressed blobs, atomic saves); formats are STEP and meshes |
| `roadmap.md` | `roadmap.md` | Milestone table with DoD and a current-focus list; no parity percentages |
| `decisions.md` | `decisions.md` | Same purpose; also lists open decisions |

**Not adopted (yet), and why**

| PhotoCraft doc | Reason |
|---|---|
| `target-app-parity.md`, `gaps.md`, `parity-checklist.md`, `ui-parity.md`, `context-menu-*.md`, `brush-parity.md`, `hardware-parity.md` | They measure distance to one target app (Photoshop). 3DCraft is AI-first with a minimal UI and has no target app ([decisions](decisions.md#taken)) |
| `scorecard.md` and the `perf/` budgets | Worth having once there is something to measure; planned from M4 ([roadmap](roadmap.md)) |
| `ui-design.md`, `color-picker.md`, `shape-strokes.md` | Phase 1 has (almost) no UI. The two rules that matter now (tokens, verify visually) are in `AGENTS.md` |
| `localization*.md` | No UI text to translate yet |
| `releasing.md`, `release-playbook.md`, `contributors.md` | No releases yet; they depend on the private `craftrules` repository, which is not checked out next to this project |
| `plugins.md` | No plug-in API planned in phase 1 |
| `*-performance.md`, `camera-raw-*.md`, `nikon-nef-development.md`, `ora.md`, `pdn.md`, `gpu-composite-reuse.md` | Image-editing specifics |
