# Making Jigs

*Turn a design into a Jig: publish the sizes others may set, switch options with conditions, prove it over
its whole range, and share it as one file.*

Any design whose sizes are variables can become a Jig ([Using a Jig](jigs.md) shows what its users get).
You design it as an ordinary `.3dcraft` document, with an agent or by hand, then go through six steps.

> **Ask an agent:** "Make this a Jig: width, depth and height between 50 and 200 mm, a wall between 1.2
> and 3 mm, and an option for a lid. Check it over the range and export it as box.3dcraftjig." The agent
> publishes the parameters, runs the range check, fixes what fails and exports the Jig.

## 1. Publish the variables others may set

Each published variable becomes a parameter, with a label, a description, a group, a range and a step.
Say your box has the variables `width` (a length), `lid` (0 or 1) and `corners` (1 or 2); these steps
publish them:

```json
[
  ["variable.publish", {"name": "width", "label": "Width", "description": "Outside width", "group": "Size",
                        "min": "40 mm", "max": "120 mm", "step": "1 mm"}],
  ["variable.publish", {"name": "lid", "label": "Thick lid", "group": "Finish", "kind": "boolean"}],
  ["variable.publish", {"name": "corners", "label": "Corners", "group": "Finish", "kind": "choice",
                        "choices": ["sharp", "round"]}]
]
```

- **Kinds.** `kind` is `number` (a value with its unit, the default), `integer`, `boolean` (0 or 1) or
  `choice` (1 for the first label; `choicesFrom` takes the labels from a table,
  [Formulas › Tables](formulas.md#tables)). The form shows a number as a slider, an integer with at most
  twelve values as a stepper (− and +), a boolean as a switch and a choice as a list.
- **A value, not a formula.** A published variable must hold a value (`40 mm`), not a formula. Other
  variables may be formulas of it: `radius = 0.5 mm + (corners - 1) * 4.5 mm`.
- **Ranges that follow.** Ranges and steps may be formulas of other parameters (`"max": "width - 2 * wall"`)
  and look values up in tables (`"max": "scales[scale].maxNotes"`).
- **Steps.** A number or an integer with a step takes only values on it, counted from its minimum when that
  is a value (`"min": "26.3 mm"`) and from 0 otherwise (no minimum, or one that is a formula), and the ends
  of its range. `param.set` refuses 13 for a step of 2 and names 12 and 14; the form rounds to the step.
  The step is for the form and `param.set` only: a value an assembly gives an instance of the part
  (`instance.add` with `params`, a value or a formula such as `sqrt(2) * 7 mm`) is held to the range, not
  to the step.
- `variable.unpublish` makes a parameter an ordinary variable again.

## 2. Switch options on and off

Conditions switch features: `feature.when {"feature": "Lid", "expr": "lid"}` builds the lid only when
*Thick lid* is switched on ([Formulas › Switching parts on and off](formulas.md#switching-parts-on-and-off)).

A colour and a density may follow a choice too: `body.appearance {"body": "Bar", "color":
["woods[wood].red", "woods[wood].green", "woods[wood].blue"]}` and `body.material {"body": "Bar", "name":
"Wood", "density": "woods[wood].density g/cm3"}` recolour and weigh the bar again when the wood changes.

## 3. Say what it is

Give it a title, a description (say how to export it), an author and a licence:

```json
[
  ["document.info", {"title": "Storage box", "description": "A box with a lid. Export it as STL to print.",
                     "author": "You", "licence": "CC-BY-4.0"}]
]
```

The licence is yours to choose. The Jigs on the start screen use CC BY 4.0 for their designs; 3DCraft
itself is under MIT.

## 4. Try it

The **Parameters** tab shows on any document with parameters, not only on a Jig. Move every parameter
across its range and watch for red parameters and failing checks.

## 5. Prove it over the whole range

`craft3d-cli check-range mybox.3dcraft` (or the command `document.sample`) builds the design at the
defaults, at the corners of the ranges, at every choice and at random points. It checks each, and lists
every combination that fails with the values that reproduce it
([The command line › Check a Jig over its range](command-line.md#check-a-jig-over-its-range)).

Fix the model, or narrow the range, until it passes. A check that could be neither proven nor disproven at
a point is counted apart (*with unproven assertions*); `--strict` treats it as a failure. With many
parameters, give `--max-corners 8` for a quick run.

## 6. Publish it

**File › Export as Jig…** (the command `jig.export`) writes the `.3dcraftjig`, with a picture of the
model. That is the file you hand out; keep the `.3dcraft` to go on designing.

## Good Jigs

Good Jigs have:

- few parameters, with clear labels and units;
- ranges that always build, and defaults that make a sensible part;
- a description that says how to export;
- checks ([Measuring and assertions](measure.md)) that guard what matters, such as a least wall.

**Next:** [The command line and scripts](command-line.md).
