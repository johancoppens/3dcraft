# 3DCraft
A custom 3D CAD modeller for AI
Following the principles of https://github.com/storytold *craft
In the first phase, (almost) no UI, menus and the like. Only load/save.
Every action is a command, so you can drive the same engine from the UI, the CLI, a JSON control channel or an MCP server.
Custom CAD kernel and tessellator
Pure Rust
A GPU compositor on wgpu (Metal, Vulkan, DX12, WebGPU), copy-on-write tiles and multithreaded filters. No Electron, no web view, no waiting.
Custom file format to store the design history: sketches, constraints, formulas, variables, and the order in which formulas and constraints were applied
Custom document model
Export to STEP
Export to STL/3MF/OBJ meshes
Clean-room: implemented from public specs and observed behaviour only. No proprietary code, shaders or assets.
Two compositors: a CPU compositor serves as the reference oracle, and a wgpu compositor puts the canvas on the GPU. They are tested against each other.
AI writes tests > runs them > checks > improves
The developer watches along during development and automated testing.
The developer prompts in Dutch during development, but everything in the repo is done in English.

