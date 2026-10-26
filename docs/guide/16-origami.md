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
- **pass through each other** or **lie on each other**: two facets meet. Folding a crease to the full 180
  degrees lays two facets on one another, which a sheet without thickness cannot be, so fold to just under
  180 degrees.

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

## Exporting the crease pattern

`file.export` with `fold` writes the flat crease pattern as `.svg` or `.dxf`, in the sketch's own axes, the
way crease patterns are drawn: the outline in black on the layer `boundary`, **mountains red and
dash-dot** on `mountain`, **valleys blue and dashed** on `valley`, and cuts black on `cut`. Print it and
score along the lines, or send the cuts to a laser cutter.

## Limits

- Creases are straight, and the sheet's outline is made of straight lines. A curved crease bends the paper
  beside it, which rigid folding does not do.
- The sheet has no thickness.
- A cut must divide the sheet; a slit that ends inside a facet is not built yet.
- At most 400 crease angles are solved at once; give more of them an angle in a larger pattern.

How it works, and the sources: [docs/design/origami.md](../design/origami.md).
