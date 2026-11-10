# Contributing to 3DCraft

Thank you for taking the time. 3DCraft is built by one developer working with AI agents, under tests a
machine can check (the [README](README.md) says how it started). This page says how to report a problem
and how to propose a change. The rules for the code itself are in [`AGENTS.md`](AGENTS.md) and
[`docs/contributing.md`](docs/contributing.md).

## Reporting a problem

Open an issue in this repository: [new issue](https://github.com/johancoppens/3dcraft/issues/new/choose),
*Report a problem*. Say:

- **what you did**: the steps, the commands or the script, and on which model;
- **what happened**, and **what you expected**;
- **the details 3DCraft copies**: **Copy details** in the error dialog, or in the status bar after a failed
  command; an AI agent gets the same report from the command `session.report`. The report holds file paths
  and the values you gave: read it before you post it;
- **the model or the script**, if you can share it (GitHub takes a `.zip`: put the `.3dcraft` file in one).

A refusal ("not supported", "not built") is worth reporting too: in 3DCraft a refusal is work still owed.
How to find the details is in the user guide, [Troubleshooting](docs/guide/14-troubleshooting.md) ›
*Reporting a problem*.

**Security problems are not reported in an issue.** See [`SECURITY.md`](SECURITY.md).

## Proposing a change

Changes are welcome, but **open an issue first**: what you want to change and why. A pull request follows
once the developer has agreed on the issue. That keeps you from building something that does not fit, such
as a generator for one kind of product, which belongs in a separate project on top of 3DCraft (rule 14 of
[`AGENTS.md`](AGENTS.md), *Platform, not applications*).

Small improvements to the docs and the examples (a wrong number, an unclear sentence, a broken link) may
come as a pull request directly.

A pull request holds the change with its tests, and its description says:

- what changed and why, with a link to the issue;
- what you ran: at least `cargo fmt --all --check`, `cargo clippy -p <crates> --all-targets -- -D warnings`
  and `cargo test -p <crates>` for the crates you touched; `cargo xtask ci` runs everything, if your machine
  can take it ([`docs/development.md`](docs/development.md)). Run `cargo xtask fonts` once before the tests:
  it fetches the built-in fonts, which are never committed, and the text tests fail without them;
- every test expectation you changed, with its reason, under the four conditions of
  [`docs/testing.md`](docs/testing.md) › *Changing an expectation*. Where the docs say an expectation change
  is "logged in the dev log", from outside it goes here.

The developer runs the full gate (`cargo xtask ci`, then `cargo xtask freecad` and the web module's size,
`cargo xtask web-size`) on the workshop's build server, the Strix, before a change is merged: `main` only moves
to a commit the gate passed.
Everything in the repository is written in English: code, comments, commit messages, docs and messages.

## Clean-room

Nothing is copied or transliterated from other geometry kernels (OpenCascade, CGAL and the like) or from
proprietary software: no code, shaders, icons or other assets. Work from public specifications, textbooks,
papers and the observed behaviour of other programs, and cite the source in a comment where an algorithm
comes from one. Reading another kernel's documentation to learn what an operation should do is fine;
pasting or transliterating its code is not. A third-party asset needs a permissive licence, its licence
file next to it and a row in [`ATTRIBUTION.md`](ATTRIBUTION.md).

## Licence

3DCraft is licensed under the [MIT licence](LICENSE). Every contribution is under it too: by sending a
contribution you agree that it is published under the MIT licence of this repository.

## Where the rules are

- [`AGENTS.md`](AGENTS.md): the golden rules (never panic, everything is a command, the history is the
  truth, determinism, clean-room, tests), the layering and the checklist before a change is done. It was
  written for the AI agents and the developer who built 3DCraft; its first part says what in it holds for
  everyone. If you work with an AI agent, point it at that file.
- [`docs/contributing.md`](docs/contributing.md): the rules in short, and the checklists for adding a
  command, a feature type or a geometry kind.
- [`docs/development.md`](docs/development.md) (building, running, checking) and
  [`docs/testing.md`](docs/testing.md) (the oracles, and how an expectation may change).
