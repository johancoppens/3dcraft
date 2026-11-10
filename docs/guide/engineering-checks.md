# Engineering checks

*Questions of engineering, asked of the exact geometry and kept as checks: a section's area and
stiffness, a tank's fill level, where a hull floats and whether it stays upright, a stress, and what
happens over a motion.*

These build on the measures, checks and outputs of [Measuring and assertions](measure.md): read that
first.

## Below a plane, sections and levels

Three questions about a plane through closed bodies, answered on the exact geometry (the part below
the plane is cut off by the kernel and integrated; the section's moments come from the curves in which
the plane cuts the bodies). The plane is given as `measure.symmetry` takes it (`xy`, `yz`, `zx`, a datum
plane, `{"point", "normal"}`) or as a flat face by its name (`"F1:face(1)"`, with its outward normal),
moved by `offset` along its normal; the default is the xy plane. *Below* is the side the normal points
away from.

| Command | Answers |
|---|---|
| `measure.below {"of", "plane", "offset"}` | The `volume` below, its `centroid`, the `surface` below, and the `section` |
| `measure.section {"of", "plane", "offset"}` | The section: `area`, `centroid`, `ix` and `iy` (second moments about its centroid along `xAxis` and `yAxis`), `ixy`, the principal `major` and `minor` with `majorAxis`; its torsion: `torsion` (J), `shearCentre`, `shearOffset`, `warping` ([below](#a-sections-torsion-and-shear-centre)) |
| `measure.levelFor {"of", "plane", "volume"}` | The `offset` at which the volume below is `volume`, and what `measure.below` gives there |
| `measure.floating {"of", "plane", "volume", "centerOfGravity"}` | The level and tilt a body floats at: the equilibria, each stable, unstable or neutral ([Floating](#floating)) |

```json
[
 ["feature.block", {"size": [4000, 1300, 1000], "position": [-2000, -650, 0], "name": "Tank"}],
 ["measure.below", {"of": "Tank", "offset": "300 mm"}],
 ["measure.levelFor", {"of": "Tank", "volume": "1.56 * (1 m)^3"}],
 ["measure.section", {"of": "Tank", "plane": "yz"}]
]
```

The first answers `volume` 1.56e9 (mm³), `centroid` [0, 0, 150]; the second, the fill level for 1.56 m³ of
liquid, `offset` 300 (the kernel finds it by Newton's method, `steps` saying how many levels it measured);
the third, the cross-section square to x: `area` 1.3e6, with `xAxis` y and `yAxis` z, `ix` = 1300 × 1000³/12
(about the axis along y) and `iy` = 1000 × 1300³/12.

- `offset` is a length, `volume` a volume, both expressions of the variables; `xAxis` sets the
  section's x axis (square to the normal; by default the world axis most square to it).
- Errors you can act on: *outside (0, total)* (no plane leaves that volume below), *a sheet has no
  inside* (close it first), *did not converge* after 64 steps.
- Bodies that overlap are counted once each.
- Floating objects: a body of mass m in a liquid of density ρ settles where `levelFor` of m/ρ puts the
  plane. A Jig computes what it needs from the centroid and the section's moments; the developer
  documentation has a worked example ([`design/sections-and-levels.md`](../design/sections-and-levels.md)).

### A section's torsion and shear centre

`measure.section` also twists the section: `torsion` is its St
Venant torsion constant J (the length unit to the fourth power), `shearCentre` the point a sideways load
must pass through for the beam not to twist (a point in space; `shearOffset` is it from the centroid
along `xAxis` and `yAxis`), and `warping` the warping constant Cw (the unit to the sixth power). They
hold for any section: closed tubes, open channels, sections with cutouts or holes, several pieces.
3DCraft solves the warping of the section by finite elements and refines until J and the shear centre
change by less than `torsionTolerance` (default 1e-4, relative); `torsionSolve` says whether it got there
(`converged`), how much J changed in the last round, and how many elements it took (at most
`maxElements`, default 60000). A solve that stops at the cap says `converged: false`; it never claims
more. `torsion: false` leaves the torsion out (it costs from a few tens of milliseconds to a few tenths
of a second).

```json
[
 ["feature.block", {"name": "Outer", "size": ["500 mm", "100 mm", "150 mm"]}],
 ["feature.block", {"name": "Inner", "size": ["502 mm", "94 mm", "142 mm"], "position": ["-1 mm", "3 mm", "4 mm"]}],
 ["feature.boolean", {"name": "Box", "op": "subtract", "target": "Outer", "tool": "Inner"}],
 ["measure.section", {"of": "Box", "plane": "yz", "offset": "250 mm"}]
]
```

The welded box beam, walls of 3 and 4 mm: `torsion` about 5.58e6 mm⁴, 1.4% above Bredt's thin-walled
formula (which is an approximation; the solve is not), and `shearCentre` at the centroid, as the
symmetry says. Cut a slot in one wall and the section through it is open: J drops by three orders of
magnitude and the shear centre moves away from the slot.

### Sections and levels as checks

As assertions: `volumeBelow`, `centroidBelow` (with `axis`), `sectionArea`, `sectionCentroid` (with
`axis`), `sectionInertia` (with `about`: `x`, `y`, `xy`, `major` or `minor`), `sectionTorsion` (J),
`sectionShearCentre` (with `axis`: a coordinate of the shear centre; a solve that stops at its cap is
`notProven`), `sectionExtent` (with
`axis`: `x`, `y`, `z` or `[x, y, z]`; how far the section reaches along it, the length of a hull's
waterline at its level) and `levelFor` (with `volume`), each with the same `of`, `plane`, `offset`
(not for `levelFor`) and `xAxis` (not for `sectionExtent`):

```json
[
 ["assert.add", {"name": "stiff enough", "measure": "sectionInertia", "of": "Tank", "plane": "yz", "about": "minor", "min": "1e11"}],
 ["assert.add", {"name": "fill level", "measure": "levelFor", "of": "Tank", "volume": "1.56 * (1 m)^3", "max": "350 mm"}]
]
```

## Floating

`measure.floating` finds the plane a floating body settles at when its centre of
gravity is known too: the level *and* the tilt at which the volume below is `volume` (or `mass` /
`density`, kg and kg/m³) and the centroid below lies straight under the centre of gravity. `plane` is the
upright waterplane (default xy, up +z); `heel` and `trim` tilt the start.

```json
[
 ["feature.block", {"size": [1000, 200, 200], "position": [-500, -100, 0], "name": "Pontoon"}],
 ["measure.floating", {"of": "Pontoon", "mass": 20, "density": 1000, "centerOfGravity": [0, 0, 50]}],
 ["measure.floating", {"of": "Pontoon", "mass": 20, "density": 1000, "centerOfGravity": [0, 0, 90]}]
]
```

The first answers one equilibrium, upright at `offset` 100, `stable`, with its two metacentric heights
under `principal` (`gm` 33.3 mm about the length, 833 mm about the width). The second has its centre of
gravity too high: upright is an equilibrium but `unstable`, and the two it falls to follow, heeled 32.3°
to each side (the box's angle of loll); `floats` is the index of the first stable one. Each equilibrium
gives `heel`, `trim` and `tilt` in degrees, `normal`, `centroid`, `arm` (how far the centre of gravity is
off the vertical through the centroid) and what `measure.below` gives there. The search starts from the
upright plane (or `heel`, `trim`), finds the nearest equilibrium by Newton's method and, where that one is
unstable, the ones the body falls to from it; other equilibria are not searched. `neutral` says that the
first one turns freely in some direction (a sphere with its centre of gravity at its centre).

### Floating, as assertions and outputs

What `measure.floating` finds can be asserted too, so a design that
floats upright keeps doing so as it changes: `floatingOffset` (the waterplane's offset along its normal),
`floatingHeel` and `floatingTrim` (its angles from the upright plane; write the bounds with a unit, `"3 deg"`),
`floatingGM` and `floatingGML` (the metacentric heights for a heel, a turn about the plane's x axis, and for a
trim, about its y axis) and `floatingStable` (`is`: stable both ways; *notProven* when it is neutral within measurement), all of the
equilibrium reached from the start. They take the parameters of `measure.floating`, and every value may be a formula of the variables and
the outputs, so the centre of gravity can be computed by the design:

```json
[
 ["feature.block", {"size": [2000, 600, 400], "position": [-1000, -300, 0], "name": "Barge"}],
 ["variable.set", {"name": "KG", "expr": "180 mm"}],
 ["variable.set", {"name": "xg", "expr": "64 mm"}],
 ["variable.output", {"name": "zG", "expr": "KG"}],
 ["assert.add", {"name": "upright", "measure": "floatingGM", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"], "min": "0 mm"}],
 ["assert.add", {"name": "trim", "measure": "floatingTrim", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"], "min": "-3 deg", "max": "3 deg"}],
 ["variable.measure", {"name": "GMf", "measure": "floatingGM", "of": "Barge", "mass": 300, "density": 1000, "centerOfGravity": ["xg", 0, "zG"]}]
]
```

The barge settles 2.86° down towards +x, where its centre of gravity lies (a trim of tan⁻¹ 0.05, the wall-sided closed form) with a GM of
66.75 mm, so both pass. With `KG` at 300 mm its GM is negative and `upright` fails, and so does `trim`:
the barge then trims by 3.16°, past its bound of 3°. A search for the
equilibrium that does not settle makes such an assertion *notProven*, with the reason, never passed, and
an output of it unknown; a displacement more than the body holds is an error (it sinks). Each assertion
solves the equilibrium itself, and again only when what it depends on changes.

### Stability as outputs: a hull's GM

Some answers are formulas of measures. A boat's stability, the metacentric height GM = KB + BM − KG,
comes from where it floats (the draught T), the height of the centre of what is under water (KB), and the
waterplane's moment (I, so BM = I / V). As outputs ([Measuring and assertions › Outputs](measure.md#outputs-measures-in-formulas)),
a box hull's GM is worked out after every change, and checked:

```json
[
 ["variable.set", {"name": "KG", "expr": "180 mm"}],
 ["variable.set", {"name": "V", "expr": "0.3 * (1 m)^3"}],
 ["feature.block", {"size": ["2000 mm", "600 mm", "400 mm"], "position": ["-1000 mm", "-300 mm", "0 mm"], "name": "Hull"}],
 ["variable.measure", {"name": "T", "measure": "levelFor", "of": "Hull", "volume": "V"}],
 ["variable.measure", {"name": "KB", "measure": "centroidBelow", "of": "Hull", "offset": "T", "axis": "z"}],
 ["variable.measure", {"name": "I", "measure": "sectionInertia", "of": "Hull", "offset": "T", "about": "x", "xAxis": "x"}],
 ["variable.output", {"name": "BM", "expr": "I / V"}],
 ["variable.output", {"name": "GM", "expr": "KB + BM - KG"}],
 ["assert.add", {"name": "stable", "measure": "value", "expr": "GM", "min": "0 mm"}],
 ["output.list", {}]
]
```

The hull floats at `T` = 250 mm (the level below which it holds 0.3 m³), `KB` = 125 mm, `I` = 2000 × 600³/12
mm⁴, so `BM` = 120 mm and `GM` = 65 mm: the assertion *stable* passes. Set `KG` to 300 mm and GM is −55 mm,
and *stable* fails; only the assertion is measured again, since the waterline does not depend on `KG`.

## Forces, stresses and moments

Forces, stresses and moments have units of their own:

- `g` and `kg` for a mass, `s` for a time;
- `N` and `kN` for a force; `Pa`, `kPa`, `MPa` and `GPa` for a pressure or a stress; `J` for an energy;
- a force unit followed by a length unit for a moment: `20 N m`, `5 kN mm`.

They are derived from mass, length and time (a newton is kg·m/s²), so a load over an area is a stress:
`1000 N / (100 * (1 mm)^2)` is 10 MPa, and `1 MPa` is `1 N / (1 mm)^2`. Unlike `mm` or `deg`, these
symbols are not reserved: a count may still be called `N` and a spacing `s`. They are units only right
after a number, a parenthesis or `pi` (`N * 5 N` is N times five newtons).

A formula with units in it has the dimension they give: a load in newtons over an area is a stress. So
a stress needs no unit in its name:

```json
[
 ["variable.set", {"name": "load", "expr": "1000 N"}],
 ["feature.block", {"name": "Bar", "size": ["100 mm", "10 mm", "10 mm"]}],
 ["variable.measure", {"name": "area", "measure": "sectionArea", "of": "Bar", "plane": "yz", "offset": "50 mm"}],
 ["variable.output", {"name": "stress", "expr": "load / area", "label": "Bending stress", "unit": "MPa"}],
 ["variable.output", {"name": "moment", "expr": "load * 100 mm", "label": "Bending moment"}],
 ["assert.add", {"name": "below yield", "measure": "value", "expr": "stress", "max": "235 MPa"}],
 ["output.list", {}]
]
```

`output.list` (and the Parameters tab) shows *Bending stress 10 MPa* and *Bending moment 100 N m*. An
output without a `unit` is shown in its dimension's: a force in N, a pressure or a stress in MPa, a
moment (or an energy) in N m, a mass in kg, anything else in the document's length unit to its power.
A `unit` is a unit of the expressions or a product and quotient of them: `N/mm²`, `kN m`, `kN*mm`,
`kg/m^3`, `mm/m`. It must have the output's dimension: `"unit": "mm"` on a stress is refused, with both
dimensions named, and a unit that stops fitting after an edit (the load written without its unit)
makes the output *unknown* with the reason. An assertion's bounds take the same units (`"max": "235
MPa"`); a plain number bounding a stress is in MPa, a force in N, a mass in kg and a density in kg/m³
(as a floating measure's `density`). A measured `mass` is a mass: `mass + 1` is a unit mismatch, write
`mass + 1 kg`. Mind that a unit belongs to the number just before it: `1000 N / 100 mm^2` divides by
(100 mm)², 10 000 mm², and is 0.1 MPa; write `1000 N / (100 * (1 mm)^2)`. A unit's symbol alone, an
undefined `N`, is a variable: write `1 N`. The set filter `thickness` ([Selection and named sets](selection.md#named-sets))
is a flat face's plate thickness, not this measure's thinnest place.

## Over a motion

A design that moves (its timeline `t`, see [The window](window.md#playing-a-motion)) is checked over the
whole motion with `motion.check`: every assertion is measured at evenly spaced moments of `t` (samples), on a
copy, and the result says per assertion whether it held at the samples, the first sample it failed at
(`firstFailure.t`, with `after`, the last sample before it that passed: the failure begins between the two),
and its least and greatest value at the samples with their `t`. It proves what held at the samples, no more:
a collision shorter than the spacing of the samples can be missed, so the result says `sampled: true`; take
more samples, or a narrower `t` around a suspect moment, to look closer. Any measure works: `overlap` with
`is` false gives the first sample that collides, `clearance` the least gap at the samples and where it is. `measure` checks an assertion written in place without keeping it.

```json
[
 ["variable.set", {"name": "t", "expr": "0"}],
 ["feature.block", {"name": "Wall", "size": ["10 mm", "10 mm", "10 mm"], "position": ["-10 mm", "0 mm", "0 mm"]}],
 ["feature.block", {"name": "Slider", "size": ["10 mm", "10 mm", "10 mm"],
   "position": ["15 mm + 10 mm * cos(360 deg * t)", "0 mm", "0 mm"]}],
 ["assert.add", {"name": "gap", "measure": "clearance", "from": "Wall", "to": "Slider", "min": "6 mm"}],
 ["motion.check", {"samples": 33}],
 ["motion.check", {"measure": {"measure": "overlap", "of": "Wall", "to": "Slider", "is": false}}]
]
```

The first check fails at `t` = 0.4375 with the least clearance 5 mm at `t` = 0.5 (the slider swings to within
5 mm of the wall); the second passes. A collision shorter than the spacing of the samples can be missed: give
more `samples` where it matters.

**Next:** [Making Jigs](making-jigs.md).
