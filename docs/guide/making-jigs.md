# Making Jigs

## Making a Jig

Any design whose sizes are variables can become a Jig. You design it as an ordinary `.3dcraft` document
(with an agent, or by hand), then:

1. **Publish the variables** others may set. Each becomes a parameter, with a label, a description, a
   group, a range and a step:

   ```json
   [
     ["variable.publish", {"name": "width", "label": "Width", "description": "Outside width", "group": "Size",
                           "min": "40 mm", "max": "120 mm", "step": "1 mm"}],
     ["variable.publish", {"name": "lid", "label": "Thick lid", "group": "Finish", "kind": "boolean"}],
     ["variable.publish", {"name": "corners", "label": "Corners", "group": "Finish", "kind": "choice",
                           "choices": ["sharp", "round"]}]
   ]
   ```

   `kind` is `number` (a value with its unit, the default), `integer`, `boolean` (0 or 1) or `choice` (1
   for the first label; `choicesFrom` takes the labels from a table, [Formulas, tables and conditions](formulas.md#tables)).
   A published variable must hold a value (`40 mm`), not a formula; other variables may be formulas of it
   (`radius = 0.5 mm + (corners - 1) * 4.5 mm`). Ranges and steps may be formulas of other parameters
   (`"max": "width - 2 * wall"`) and look values up in tables (`"max": "scales[scale].maxNotes"`). A
   number or an integer with a step takes only values on it, counted from its minimum when that is a
   value (`"min": "26.3 mm"`) and from 0 otherwise (no minimum, or one that is a formula), and the ends
   of its range: `param.set` refuses 13 for a step of 2 and names 12 and 14, and the form rounds to the
   step. The step is for the form and `param.set` only: a value an assembly gives an instance of the part
   (`instance.add` with `params`, a value or a formula such as `sqrt(2) * 7 mm`) is held to the range, not
   to the step. `variable.unpublish` makes a parameter an ordinary variable again.
2. **Switch options on and off** with conditions: `feature.when {"feature": "Lid", "expr": "lid"}` builds
   the lid only when the *Thick lid* box is ticked ([Formulas, tables and conditions](formulas.md#switching-parts-on-and-off)).
   A colour and a density may follow a choice too: `body.appearance {"body": "Bar", "color":
   ["woods[wood].red", "woods[wood].green", "woods[wood].blue"]}` and `body.material {"body": "Bar",
   "name": "Wood", "density": "woods[wood].density g/cm3"}` recolour and reweigh the bar when the
   wood changes.
3. **Say what it is**: a title, a description (say how to export it), an author and a licence:

   ```json
   [
     ["document.info", {"title": "Storage box", "description": "A box with a lid. Export it as STL to print.",
                        "author": "You", "licence": "CC-BY-4.0"}]
   ]
   ```

4. **Try it** on the **Parameters** tab, which a document with parameters shows too: drag every
   parameter across its range and watch for red fields.
5. **Prove it over the whole range** with `craft3d-cli check-range mybox.3dcraft` (or the command
   `document.sample`): it builds the design at the defaults, at the corners of the ranges, at every choice
   and at random points, checks each, and lists every combination that fails with the values to
   reproduce it ([The command line and scripts](command-line.md#check-a-jig-over-its-range)). Fix the model or narrow the
   range until it passes. An assertion that could be neither proven nor disproven at a point is counted
   apart (*with unproven assertions*); `--strict` treats it as a failure. With many parameters, give
   `--max-corners 8` for a quick run.
6. **Publish it**: **File › Export as Jig…** (the command `jig.export`) writes the `.3dcraftjig`, with a
   picture of the model. That is the file you hand out; keep the `.3dcraft` to go on designing.

> **Ask an agent:** "Make this a Jig: width, depth and height between 50 and 200 mm, a wall between 1.2
> and 3 mm, and an option for a lid. Check it over the range and export it as box.3dcraftjig." The agent
> publishes the parameters, runs the range check, fixes what fails and exports the Jig.

**Good Jigs** have few parameters, clear labels with units, ranges that always build, defaults that make a
sensible part, a description that says how to export, and checks (assertions, [Measuring and assertions](measure.md))
that guard what matters, such as a minimum wall.
