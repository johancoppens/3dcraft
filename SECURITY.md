# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through GitHub's private
vulnerability reporting: the **Security** tab of this repository, *Report a vulnerability*
([direct link](https://github.com/johancoppens/3dcraft/security/advisories/new)). Say what is affected,
how to reproduce it (a file, a script or the steps), and what it lets an attacker do. The developer answers
there; please leave time for a fix before you make the problem public.

## What counts as security

- **A security problem** is a way for a file, a script, a web page or an AI agent to reach what it must
  not. For example: a `.3dcraft`, `.3dcraftjig`, `.3dcraftasm`, STEP, FreeCAD or font file that makes
  3DCraft read, write or run anything beyond failing with an error; an agent that reads or writes files
  outside the folders it was given, or gets past the window's question before it reaches another folder; a
  connection to the window's control channel or MCP bridge without its token; the control token leaking.
- **A bug, for the [issues](https://github.com/johancoppens/3dcraft/issues)**: a crash, a wrong result, a
  file that does not open or is written wrong, a refusal, a hang or a large use of memory. 3DCraft must
  never crash, so a crash is a serious bug, but it is reported as an issue ([`CONTRIBUTING.md`](CONTRIBUTING.md))
  unless it also lets the input do one of the things above.

## Supported versions

Security fixes go into the latest release only. Until 1.0, older versions get no fixes: update to the
latest release.
