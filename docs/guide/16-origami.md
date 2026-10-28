# 16. Origami: crease patterns and folding

*Draw a crease pattern on a flat sheet, mark each crease mountain, valley or cut, and fold it: by angles you
give, by a variable you slide, with the other creases solved. Check the pattern's vertices, and export the
pattern for printing or cutting.*

## A sheet and its creases

A fold starts from a **sketch**: a closed outline of straight lines is the sheet (holes are allowed), and
straight lines inside it are the creases. Draw the creases as **construction lines**
(`"construction": true`), so the sketch stays a plain outline for anything else that uses it. Each crease
in `feature.fold` says which line it is (its entity id from `sketch.line`), its **kind** and, if you like,
its **angle**:

| Kind | Seen from the front (the side the sketch's normal points to) |
|---|---|
| `valley` | the paper folds towards you; the crease sinks |
| `mountain` | the paper folds away from you; the crease rises |
| `cut` | the sheet is cut through along the line |

A crease runs from the boundary, a cut or another crease to the boundary, a cut or another crease. Where
creases cross or end on each other is a **vertex**; the creases divide the sheet into **facets**, and each
facet stays flat and turns about the creases it shares with its neighbours. That is *rigid folding*: the
paper does not bend or stretch between the creases.

## One fold, driven by a variable

A card folded in two, by the variable `fold`:

```json
[
  ["file.new", {"units": "mm"}],
  ["variable.set", {"name": "fold", "expr": "60 deg"}],
  ["sketch.create", {"plane": "xy", "name": "Paper"}],
  ["sketch.polyline", {"sketch": "Paper", "points": [[0, 0], [100, 0], [100, 100], [0, 100]], "closed": true}],
  ["sketch.line", {"sketch": "Paper", "from": [50, 0], "to": [50, 100], "construction": true}],
  ["feature.fold", {"sketch": "Paper", "name": "Card", "fixed": [25, 50],
    "creases": [{"line": "e11", "kind": "valley", "angle": "fold"}]}]
]
```

The angle is a fold angle: 0 is flat, 180 degrees is folded flat, and the kind says which way. `fixed` is
a point inside the facet that stays where it is (without it, the largest facet stays). Change `fold` with
`variable.set`, or make it a parameter and drag its slider, and the card opens and closes. The result is a
**sheet** (no volume, see [chapter 9](09-features.md#sheets)): `document.measure` gives its area, still
100 × 100 mm², whatever the angle.

## Solved creases: a vertex with four creases

Leave a crease's angle out and 3DCraft solves it: the facets round every vertex must meet again, which
fixes the angles of the others. A vertex with four creases has one degree of freedom, so one angle drives
all four. Here the creases leave the middle at 0, 60, 120 and 240 degrees:

```json
[
  ["file.new", {"units": "mm"}],
  ["variable.set", {"name": "drive", "expr": "40 deg"}],
  ["sketch.create", {"plane": "xy", "name": "Vertex"}],
  ["sketch.polyline", {"sketch": "Vertex", "points": [[-50, -50], [50, -50], [50, 50], [-50, 50]], "closed": true}],
  ["sketch.line", {"sketch": "Vertex", "from": [0, 0], "to": [50, 0], "construction": true}],
  ["sketch.line", {"sketch": "Vertex", "from": [0, 0], "to": [28.867513459481287, 50], "construction": true}],
  ["sketch.line", {"sketch": "Vertex", "from": [0, 0], "to": [-28.867513459481287, 50], "construction": true}],
  ["sketch.line", {"sketch": "Vertex", "from": [0, 0], "to": [-28.867513459481287, -50], "construction": true}],
  ["feature.fold", {"sketch": "Vertex", "name": "Twist", "creases": [
    {"line": "e11", "kind": "valley", "angle": "drive"},
    {"line": "e14", "kind": "mountain"},
    {"line": "e17", "kind": "valley"},
    {"line": "e20", "kind": "valley"}]}],
  ["fold.inspect", {"fold": "Twist"}]
]
```

`fold.inspect` lists every crease with the angle each of its pieces folds by: at a drive of 40 degrees the
crease opposite the driving one folds by 40 degrees too, and the two others by about 20.6 degrees, one up
and one down. This is the closed form of such a vertex (Huffman; Tachi): tan(ρ₂/2) = −½ tan(ρ₁/2) here.
The kinds choose how the vertex folds: marked the other way round, it would fold another way, or not at
all.

When a fold cannot be made, the feature fails and says why:

- **not rigidly foldable at these angles**: the angles you gave cannot all hold; give fewer, and let the
  others be solved;
- **nothing determines the fold angles of …**: too few angles are given (a crease on no closed loop of
  creases, or a vertex with more than four creases and one angle); give those creases an angle;
- **would fold the other way from its kind**: at these angles the pattern only folds with that crease
  turned the other way;
- **pass through each other** or **lie on each other**: two facets meet. Below 180 degrees a sheet without
  thickness may not touch itself; at exactly 180 degrees it is *folded flat* (next section), where facets
  lie on one another in an order of layers;
- **folds by 180 degrees while other creases fold by less**: facets folded onto each other are layered only
  when the whole sheet is folded flat. Give every crease 180 degrees (or leave the others to be solved),
  or fold to just under 180.

## Folding flat

Give the creases 180 degrees and the sheet is **folded flat**: every facet comes down onto the plane of
the sketch, face over, and many lie on top of one another. A crease without an angle then folds by 180
degrees too, the way its kind says. The question is no longer the angles but the **order of the
layers**: which facet lies above which, so that no facet passes through another. 3DCraft works it out:

- two facets joined by a crease are ordered by its kind: seen from the front, a valley brings their
  fronts together, a mountain their backs;
- a facet that lies across a fold may not slip between the fold's two facets (*taco-tortilla*);
- two folds along the same line nest or stay apart, never interleave (*taco-taco*);
- three facets that lie on one spot are in one order there (*transitivity*).

These are the published rules for flat-folded states (Justin; Bern and Hayes; Akitaya, Demaine and Ku).
Here is the waterbomb base of the next section, folded flat:

```json
[
  ["file.new", {"units": "mm"}],
  ["sketch.create", {"plane": "xy", "name": "Square"}],
  ["sketch.polyline", {"sketch": "Square", "points": [[-50, -50], [50, -50], [50, 50], [-50, 50]], "closed": true}],
  ["sketch.line", {"sketch": "Square", "from": [-50, 0], "to": [50, 0], "construction": true}],
  ["sketch.line", {"sketch": "Square", "from": [-50, -50], "to": [50, 50], "construction": true}],
  ["sketch.line", {"sketch": "Square", "from": [50, -50], "to": [-50, 50], "construction": true}],
  ["feature.fold", {"sketch": "Square", "name": "Base", "creases": [
    {"line": "e11", "kind": "mountain", "angle": "180 deg"},
    {"line": "e14", "kind": "valley"},
    {"line": "e17", "kind": "valley"}]}],
  ["fold.inspect", {"fold": "Base"}]
]
```

`fold.inspect` says `flat: true` and gives the layers: per facet its `layer` (0 has nothing below it) and
`faceDown`, and under `layers.above` every pair of facets that overlap, upper first. The body itself lies
exactly in the sketch's plane (`document.measure` gives the area of the square, 100 × 100 mm², as always);
only the picture lifts each layer a little off the one below it, so that you can see them.

When the kinds do not fold flat, the feature fails and says so: **cannot fold flat with this
mountain-valley assignment**, naming the facets that cannot be layered (mark all three creases valleys
and try), or **does not fold flat** when the facets round a vertex do not meet again (Kawasaki fails
there). Deciding whether a crease pattern folds flat is a hard problem in general (NP-hard), so the
search for an order has a limit; past it the fold fails with **not decided within the work allowed**,
never a guess.

One honest note: a pattern that folds flat need not be reachable by *rigid* folding, with the facets
staying flat all the way. Sliding a fold's angle towards 180 degrees may stop short (the fold cannot be
followed, or two facets touch) while the flat state at exactly 180 degrees exists.

## Creases from a sketch pattern

A crease line that a sketch pattern copies (`sketch.pattern`) gives creases of its copies too: name one
copy as the sketch names it, `e11[1]`, or the line and every copy with `e11[*]`. With `"alternate":
true` the kinds alternate along the copies, the given kind on the line itself: a pleat (accordion) whose
number of folds is a variable:

```json
[
  ["file.new", {"units": "mm"}],
  ["variable.set", {"name": "n", "expr": "4"}],
  ["variable.set", {"name": "a", "expr": "30 mm"}],
  ["variable.set", {"name": "fold", "expr": "40 deg"}],
  ["sketch.create", {"plane": "xy", "name": "Strip"}],
  ["sketch.polyline", {"sketch": "Strip", "points": [["0 mm", "0 mm"], ["n * a", "0 mm"], ["n * a", "a"], ["0 mm", "a"]], "closed": true}],
  ["sketch.line", {"sketch": "Strip", "from": ["a", "0 mm"], "to": ["a", "a"], "construction": true}],
  ["sketch.pattern", {"sketch": "Strip", "entities": ["e11"], "count": "n - 1", "step": ["a", "0 mm"]}],
  ["feature.fold", {"sketch": "Strip", "name": "Pleats", "fixed": [15, 15], "creases": [
    {"line": "e11[*]", "kind": "valley", "angle": "fold", "alternate": true}]}],
  ["fold.inspect", {"fold": "Pleats"}]
]
```

`fold.inspect` lists `e11` (valley), `e11[1]` (mountain) and `e11[2]` (valley); set `fold` to `180 deg`
and the strip lies flat in four layers. A crease that names one copy (`e11[1]` with its own kind or angle,
through `fold.edit` with `set`) wins over `e11[*]` for that copy. A crease line whose copies are not all
creases is refused, naming the copies left out: they would otherwise be dropped without a word. A copy is
named by its number; there is no `e11[last]`, because a name must keep naming the same line when the
pattern's count changes (draw the last one as a line of its own if it needs another kind).

## Checking a crease pattern: Kawasaki and Maekawa

`fold.check` looks at every vertex inside the sheet and tells you, for each, two classic conditions for
folding **flat**:

- **Kawasaki**: going round the vertex, the angles between the creases, taken alternately, add up to 180
  degrees each;
- **Maekawa**: the numbers of mountains and of valleys at the vertex differ by exactly 2.

For each vertex you get where it is, its creases, both sums and how far off they are, and the numbers of
mountains and valleys. **These are two small, local checks, and they are honest about what they do not
say**: a vertex that fails either one cannot fold flat, but passing both at every vertex does *not* prove
that the whole pattern folds flat. Whether a crease pattern folds flat is a hard problem in general
(NP-hard, Bern and Hayes 1996), and 3DCraft does not claim to answer it. The result repeats this in its
`note`.

The waterbomb base, both diagonals valleys and one middle line a mountain, passes both checks at its one
vertex:

```json
[
  ["file.new", {"units": "mm"}],
  ["sketch.create", {"plane": "xy", "name": "Square"}],
  ["sketch.polyline", {"sketch": "Square", "points": [[-50, -50], [50, -50], [50, 50], [-50, 50]], "closed": true}],
  ["sketch.line", {"sketch": "Square", "from": [-50, 0], "to": [50, 0], "construction": true}],
  ["sketch.line", {"sketch": "Square", "from": [-50, -50], "to": [50, 50], "construction": true}],
  ["sketch.line", {"sketch": "Square", "from": [50, -50], "to": [-50, 50], "construction": true}],
  ["feature.fold", {"sketch": "Square", "name": "Waterbomb", "creases": [
    {"line": "e11", "kind": "mountain", "angle": "36.03846874550981 deg"},
    {"line": "e14", "kind": "valley", "angle": "49.40777113928209 deg"},
    {"line": "e17", "kind": "valley"}]}],
  ["fold.inspect", {"fold": "Waterbomb"}],
  ["fold.check", {"fold": "Waterbomb"}],
  ["file.export", {"path": "waterbomb.svg", "fold": "Waterbomb"}]
]
```

A vertex with six creases has three degrees of freedom, so it needs three angles: here the middle line
(both of its halves) and one diagonal are given, and `fold.inspect` gives the other diagonal: 49.41
degrees for both halves, as the base's symmetry says. Give fewer, or none at all, and the fold fails with
*nothing determines the fold angles of …*: a crease pattern with solved creases folds only when the
angles you give decide how (even the flat sheet, with every given angle 0, does not decide which way the
others would go). `fold.check` and the export only read the pattern, so they work whether the fold
does or not.

The same checks are assertion measures, so a crease pattern made from variables can be held to them:
`kawasakiFails` and `maekawaFails` count the interior vertices that fail (assert `eq 0`), and
`flatFoldable` says whether the pattern with its kinds folds flat (every crease at 180 degrees, with an
order of the layers), whatever angles the fold has now:

```json
[
  ["assert.add", {"name": "Kawasaki holds at every interior vertex", "measure": "kawasakiFails", "of": "Waterbomb", "eq": "0"}],
  ["assert.add", {"name": "Maekawa holds at every interior vertex", "measure": "maekawaFails", "of": "Waterbomb", "eq": "0"}],
  ["assert.add", {"name": "It folds flat", "measure": "flatFoldable", "of": "Waterbomb", "is": true}]
]
```

The first two are necessary for folding flat and prove nothing more; `flatFoldable` is decided, or the
assertion is an error when the search for an order of the layers runs out of work.

## Exporting the crease pattern

`file.export` with `fold` writes the flat crease pattern as `.svg` or `.dxf`, in the sketch's own axes, the
way crease patterns are drawn: the outline in black on the layer `boundary`, **mountains red and
dash-dot** on `mountain`, **valleys blue and dashed** on `valley`, and cuts black on `cut`. Print it and
score along the lines, or send the cuts to a laser cutter. An SVG's lines are a laser's hairline (0.01 mm)
by default, too thin to print: `"stroke": 0.3` gives them a width in millimetres.

## Limits

- Creases are straight, and the sheet's outline is made of straight lines. A curved crease bends the paper
  beside it, which rigid folding does not do.
- The sheet has no thickness: folded flat, its layers lie in one plane, ordered (the picture draws them
  apart). Layers that fold on by other angles (a flat-folded stack folded again by 90 degrees) are not
  built yet.
- A cut must divide the sheet; a slit that ends inside a facet is not built yet.
- At most 400 crease angles are solved at once; give more of them an angle in a larger pattern.

How it works, and the sources: [docs/design/origami.md](../design/origami.md).
