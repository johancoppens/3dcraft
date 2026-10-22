#!/usr/bin/env python3
"""The second field test's driver: runs a list of steps over `craft3d-cli mcp`
(stdio JSON-RPC, as an MCP client does) and prints every result, after a
file.new {"units": "mm"} unless --no-new is given.

Step forms: ["cmd.id", {params}]   -> the tool command_run
            ["@inspect", {}]       -> doc_inspect
            ["@list", {...}]       -> command_list
            ["@batch", {...}]      -> command_batch
            ["@preview", {..., "out": path}] -> doc_render_preview, the PNG saved
            ["@include", "docs/examples/case.json"] -> that script's steps, here
            ["#", "comment"]

Usage, from the repository root:
  CRAFT3D_CLI=target/debug/craft3d-cli python3 docs/field-test/2026-10-22/scripts/drive.py \\
      docs/field-test/2026-10-22/scripts/c1-history.json [--max N] [--no-new]
Env: CRAFT3D_CLI (the binary), FT2_ROOT (the server's read and write root,
and its working directory; without it the server may read and write nothing)."""
import base64, json, os, subprocess, sys

CLI = os.environ.get("CRAFT3D_CLI")
args = sys.argv[1:]
script = args[0]
mx = int(args[args.index("--max") + 1]) if "--max" in args else 1500
steps = []
for st in json.load(open(script)):
    if st[0] == "@include":
        steps += json.load(open(st[1]))
    else:
        steps.append(st)
root = os.environ.get("FT2_ROOT")
cmdline = [CLI, "mcp", "--recovery-dir", "none"]
if root:
    cmdline += ["--automation-read-root", root, "--automation-write-root", root]
p = subprocess.Popen(cmdline, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd=root or None)
n = [0]


def call(method, params):
    n[0] += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n[0], "method": method, "params": params}) + "\n")
    p.stdin.flush()
    line = p.stdout.readline()
    if not line:
        print("!!! server closed (crash?) exit", p.poll())
        sys.exit(1)
    return json.loads(line)


call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "ft2", "version": "0"}})
p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")


def tool(name, a):
    r = call("tools/call", {"name": name, "arguments": a})
    if "error" in r:
        return True, json.dumps(r["error"]), []
    res = r["result"]
    texts = [c.get("text") for c in res.get("content", []) if c.get("type") == "text"]
    imgs = [c for c in res.get("content", []) if c.get("type") == "image"]
    return res.get("isError", False), "\n".join(t for t in texts if t), imgs


if "--no-new" not in args:
    tool("command_run", {"id": "file.new", "params": {"units": "mm"}})
for i, st in enumerate(steps):
    k = st[0]
    prm = st[1] if len(st) > 1 else {}
    if k == "#":
        print(f"\n## {prm}")
        continue
    if k == "@inspect":
        e, t, im = tool("doc_inspect", prm)
    elif k == "@list":
        e, t, im = tool("command_list", prm)
    elif k == "@batch":
        e, t, im = tool("command_batch", prm)
    elif k == "@preview":
        out = prm.pop("out", None)
        e, t, im = tool("doc_render_preview", prm)
        if im and out:
            open(out, "wb").write(base64.b64decode(im[0]["data"]))
    else:
        e, t, im = tool("command_run", {"id": k, "params": prm})
    tag = "ERR" if e else "ok "
    print(f"[{i}] {tag} {k} {json.dumps(prm)[:220]}\n    -> {t[:mx]}")
p.stdin.close()
p.wait()
