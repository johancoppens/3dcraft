# Start here: the 3DCraft user guide

> **For version 0.1.0** (the first release, October 2026). Every command named in this guide exists, and
> every example model is rebuilt and checked by a test at every change, so what you read here is what
> 3DCraft does.

![3DCraft's start screen: the logo, New document and Open, the example models as cards, and how to connect an AI agent](images/start-screen.png)

**3DCraft is a 3D design program for real parts**: brackets, boxes, gears, holders, spare parts. You
describe a part with its sizes, 3DCraft builds an exact solid model of it, and you save it, 3D-print it,
laser-cut it or send it to a workshop.

What makes it different: **every action in 3DCraft is a command** with a name and a few values, such as
`feature.block` with a size. That makes it easy for an **AI agent** (Claude, or another assistant that
speaks MCP) to model for you while you watch, and it means a model is a recipe that can be changed:
make the box 10 mm wider and everything that depends on the width follows.

You do not need to know anything about programming or CAD to start.

## Three ways to use 3DCraft

| | How | Best for |
|---|---|---|
| **With an AI agent** | You describe the part in your own words; the agent builds it in the 3DCraft window while you watch, point at faces and check the result. | Designing something new. This is what 3DCraft is made for. |
| **With a Jig** | You open a ready-made design (a box, a pulley, a welding table), set its sizes in a form, and export the result. No AI needed. | Getting a useful part in minutes, also in the browser. |
| **By hand** | You write the commands yourself, one by one or as a script, on the command line. | Repeating a design, automating, learning how it works. |

You can mix them: an agent can make a Jig for you, and you can open what an agent built and change its
numbers yourself.

## Where to go next

**New to 3DCraft?** Follow these three, in order:

1. [Install 3DCraft and connect an AI agent](01-install.md): the installers for Windows, macOS and Linux,
   the web version that needs no installing, and how to give your AI agent access.
2. [Your first five minutes](02-first-steps.md): open an example, look around, play its history, change a
   size and export it for a 3D printer.
3. [Working with an AI agent](03-agent.md): what to ask, what a session looks like, and how to check the
   agent's work.

**Using 3DCraft**

| Chapter | What you learn |
|---|---|
| [4. The window](04-window.md) | The start screen, tabs, the feature list, flying to a feature, the history player, the studio look, materials, surface analysis, pointing at faces, menus and keys |
| [5. Jigs](05-jigs.md) | Using a ready-made Jig (the Parameters tab), the Jigs collection, and making a Jig of your own |
| [6. How 3DCraft thinks](06-concepts.md) | The history, variables and units, sketches and constraints, features, names that stay put, failed features |
| [7. A first part, command by command](07-first-part.md) | An L-bracket with a rounded corner and two holes, built step by step |
| [8. Formulas, tables and conditions](08-formulas.md) | Choices made by formulas, tables of sizes, parts switched on and off, curves drawn from formulas (an involute gear) |
| [9. Modelling features](09-features.md) | Extrude, revolve, sweep and loft; text; booleans; fillets and chamfers; shell; patterns; threads; datums; sheets; developables and unfolding; surfaces from curve networks; plates; sheet metal |
| [10. Measuring and checking](10-measure.md) | Volume, distances and wall thickness, sections and floating levels, continuity across a seam, design rules (assertions) checked at every change, outputs |
| [11. Selection and named sets](11-selection.md) | Finding faces by what they are, the selection you share with the agent, sets kept in the document |
| [12. Files, export and drawings](12-files.md) | Saving, STEP, meshes for 3D printing, outlines for laser cutting, drawings of the model, the web version |
| [13. The command line and scripts](13-command-line.md) | `craft3d-cli`: run commands and scripts, render pictures, convert files, check a Jig |
| [14. Troubleshooting](14-troubleshooting.md) | What a refusal means, failed features, undo, crash recovery, reporting a problem |
| [16. Origami](16-origami.md) | Crease patterns (mountain, valley, cut), folding by angles and variables with the other creases solved, Kawasaki and Maekawa (and what they do not prove), crease patterns to SVG and DXF |
| [17. Assemblies](17-assemblies.md) | Instances of your parts and Jigs, sizes per instance, one body of a Jig, mates, subassemblies, packing, STEP with structure |

**Reference**

- [15. Reference](15-reference.md): every command by family, the MCP tools, keys and mouse, units and
  functions, the command-line programs.
- [Example models](../examples/README.md): seventeen parts (a bracket, a flange, a gear, a G2 fin, a fuel tank, a mouse, …), each a
  script with a picture that checks its own volume. The window's start screen builds ten of them with one
  click, and opens three published Jigs and three assemblies.
- [The Jigs collection](https://github.com/johancoppens/3dcraft-jigs): Jigs to download and use.
- [The field test](field-test.md): for testers of 0.1.0, how to install, connect an agent, five tasks to
  start with, and how to report what you find.

## Words used in this guide

| Word | Meaning |
|---|---|
| **Document** | One design, saved as a `.3dcraft` file. It holds the recipe, not just the shape. |
| **Feature** | One step of the recipe: a block, a hole, a rounded edge. The **feature list** shows them in order. |
| **Body** | A solid shape the features make. A document can hold several. |
| **Variable** | A named number with a unit (`width = 60 mm`) that features use, so the model follows when it changes. |
| **Parameter** | A variable a Jig shows in its form, with a label and a range. |
| **Jig** | A design made to be set by others through its parameters, shared as a `.3dcraftjig` file. |
| **Agent** | An AI assistant connected to 3DCraft through **MCP** (the Model Context Protocol), which lets it run 3DCraft's commands. |
| **Command** | One action, such as `feature.fillet` or `file.export`, with its values written as JSON (`{"radius": "3 mm"}`). |

In this guide, **File › Export…** means the *Export…* item of the *File* menu, and text in
`this font` is something you type or a name 3DCraft shows.
