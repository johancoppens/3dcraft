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

## The chapters

The guide has six parts. Read the first part in order; after that, go to what you need.

**Start**

1. [Install 3DCraft and connect an AI agent](install.md): the installers for Windows, macOS and Linux,
   the web version that needs no installing, and how to let an AI agent work in the window.
2. [Your first five minutes](first-steps.md): open an example, look around, play its history, set a Jig
   and export the result.
3. [Working with an AI agent](agent.md): what to ask, what a session looks like, how to check the agent's
   work, and five tasks to try first.

**Using it**

4. [The window](window.md): the start screen, menus, tabs, the feature list, the history player, the
   studio look, surface analysis, and pointing at things.
5. [Using a Jig](jigs.md): the Parameters tab, its limits, and the Jigs on the start screen.
6. [Assemblies](assemblies.md): parts placed together, joined by mates, played as a motion.
7. [Files, export and drawings](files.md): saving, STEP, meshes for 3D printing, outlines for laser
   cutting, cut lists, drawings, the web version, autosave.

**Modelling**

8. [How 3DCraft thinks](concepts.md): the history, variables and units, sketches and constraints,
   features, names that stay put, failed features.
9. [A first part, command by command](first-part.md): an L-bracket with a rounded corner and two holes.
10. [Formulas, tables and conditions](formulas.md): choices made by formulas, tables of sizes, parts
    switched on and off, curves drawn from formulas.
11. [Modelling features](features.md): extrude, revolve, sweep, loft, text, booleans, patterns, fillets,
    shells, threads, datums, sheets, colours and materials.
12. [Sheet metal](sheet-metal.md): a part bent from one sheet, its flat pattern, and whether a press brake
    can bend it.
13. [Surfaces and developables](surfaces.md): sheets that bend without stretching, surfaces through
    curves, smooth seams, plates.
14. [Origami](origami.md): crease patterns folded by angles you give, flat folds, Kawasaki and Maekawa.
15. [Selection and named sets](selection.md): finding faces by what they are, the selection you share
    with the agent, sets kept in the document.

**Checking**

16. [Measuring and assertions](measure.md): volumes, distances, walls, checks kept in the document, outputs.
17. [Engineering checks](engineering-checks.md): sections, fill levels, floating, forces and stresses,
    checks over a motion.

**Building on 3DCraft**

18. [Making Jigs](making-jigs.md): publish a design's sizes as a form, prove it over its range, share it.
19. [The command line and scripts](command-line.md): `craft3d-cli` runs commands and scripts, renders
    pictures, converts files and checks a Jig.
20. [Programs on 3DCraft](programs.md): your own program on the engine, through `serve`, the window's
    control channel, MCP or the Rust library.

**Help**

21. [Troubleshooting](troubleshooting.md): what a refusal means, failed features, undo, crash recovery,
    reporting a problem.
22. [Reference](reference.md): every command by family, keys and mouse, units and functions, the
    programs and their options.

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
