# Programs on 3DCraft

## For programs: `serve` and `mcp`

- `craft3d-cli mcp` is the MCP server for AI agents ([Install 3DCraft and connect an AI agent](install.md#connect-an-ai-agent-over-mcp)).
- `craft3d-cli serve` speaks a simple JSON-lines protocol on standard input and output, for your own
  programs: `{"id": 1, "method": "engine.execute", "params": {"command": "feature.block", "params": {...}}}`.
  The window speaks the same protocol on a TCP port (`craft3d --control`), with a token;
  [`control-protocol.md`](../control-protocol.md) describes it.
- `craft3d-cli score TASK --script FILE` scores a script against a task of 3DCraft's task set
  ([`docs/tasks/`](../tasks/)), the benchmark 3DCraft is measured by.
