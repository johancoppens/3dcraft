# 3DCraft

A parametric 3D CAD modeller for AI, in pure Rust, with its own kernel, tessellator and renderers.
Every action is a command, so the same engine can be driven from a UI, the CLI, a JSON control
channel or an MCP server.

![The Utah teapot, tessellated and rendered by 3DCraft's CPU reference renderer](docs/img/teapot.png)

**Status: very early.** What exists today is the first vertical slice: Bézier patches, a
tolerance-driven tessellator and a CPU reference renderer. The CAD kernel, document model, file
formats and engine are designed in [`docs/`](docs/README.md) and not built yet;
[`docs/roadmap.md`](docs/roadmap.md) says what comes next.

```sh
cargo test --workspace
cargo run --release -p craft3d-render --example teapot -- --out target/evidence/teapot.png
```

Agents and contributors: read [`AGENTS.md`](AGENTS.md) first. Licence: [MIT](LICENSE).
