# 10. Measuring and checking

*Ask the design questions (how heavy, how thick, does it fit) and keep the answers that matter as
checks that run at every change.*

An agent should check a design by asking it questions, not by looking at a picture. 3DCraft answers on
the exact geometry, and can keep the questions in the document as **assertions** that are checked again
after every change.

## Measures

| Command | Answers |
|---|---|
| `document.inspect`, `document.measure` | Volume, area, centroid, bounding box of every body; mass and inertia with a material |
| `measure.distance {"from", "to"}` | The least distance between faces or bodies, and where |
| `measure.clearance {"from", "to"}` | The gap between bodies; an overlap has none |
| `measure.thickness {"of"}` | The thinnest wall of a body (or from chosen faces), and where |
| `measure.through {"of"}` | Whether holes (cylindrical or conical faces) go clear through along their axis |
| `measure.ray {"from", "direction"}` | Every place a line meets the bodies, entering or leaving |
| `measure.symmetry {"of", "plane"}` | How far the bodies are from symmetric in a plane |
| `document.interference` | For each pair of bodies: clear, touch or overlap (with the common volume) |

A subject (`of`, `from`, `to`) is a body by its feature's name, a list of them, or a query as
`select.query` takes it ([chapter 11](11-selection.md)): `{"hole": true, "radius": "4 mm"}` for the 8 mm holes,
`{"kind": "face", "where": {"normal": {"along": "z"}}}` for the faces facing up.

What is searched for (a thickness, a distance) comes with what is **proven**: `atLeast`, a bound nothing
is below, beside the value found at a real place. On the flange of [chapter 3](03-agent.md#a-full-session): `"thickness": 6.0,
"atLeast": 5.75`.

## Assertions

`assert.add` keeps a measure and what it must be:

```json
[
 ["assert.add", {"name": "volume", "measure": "volume", "of": "Case", "eq": "(length * width - (4 - pi) * corner_r^2) * height - ((length - 2 * wall) * (width - 2 * wall) - (4 - pi) * (corner_r - wall)^2) * (height - wall)"}],
 ["assert.add", {"name": "walls", "measure": "thickness", "of": "Case", "min": "wall - 0.1 mm"}],
 ["assert.add", {"name": "holes go through", "measure": "through", "of": {"hole": true}}],
 ["assert.add", {"name": "six bolt holes", "measure": "count", "of": {"set": "bolt holes"}, "eq": 6}]
]
```

- Measures: `volume`, `area`, `mass`, `count`, `extent` (size along `axis`), `distance`, `clearance`,
  `thickness`, `through`, `interference` (none overlap), `symmetric`.
- Bounds: `eq` (with `tol`, by default a millionth of the value), or `min` and/or `max`; `is` for the
  yes-or-no measures. Bounds are expressions in the document's variables, so they follow the design: the
  case's volume is right for any `wall`.
- An assertion may be written before the model meets it, even before its subject exists (it is in
  *error* until then).
- An assertion names a feature's body as it is at that feature. A feature added on top of it later (a
  hole cut into `Case`, giving `Vented`) takes that body, and the assertion turns to *error*: *the body
  of `Case` went into F6, `Vented`; name that feature*. Point it at the new end of the chain, with the
  bound changed if the change was meant: `assert.edit {"name": "volume", "of": "Vented", "eq": "…"}`.
  Queries (`{"hole": true}`, a named set) look at whatever bodies there are and need no edit.

Each assertion has a status: **passed**; **failed** (with what was measured and expected);
**notProven** (a thickness or distance whose found value is within the bound but whose proven bound is
not: a warning, never a pass); **error** (it could not be measured: its subject names nothing, or a body
is missing); **pending** (still being measured, on a large model). They are measured off to the side and
only when what they depend on changed, so they never slow modelling down or stop a feature.

Where they show: in the result of every feature command, `variable.set` and `param.set` (`assertions:
{count, passed, broken, unproven, pending}`), in `assert.list` (each with its message), in `document.check` (a broken one makes
`ok` false), in `document.inspect`, and in the window's tree. `assert.edit` and `assert.delete` change
and remove them.

**Pending after an edit.** An edit's result waits up to two seconds for the assertions it touched; the
rest count under `pending`, and `whenPending` then says how to have them. A script that checks its
assertions after a change either gives the edit `"wait": true` (`param.set`, `variable.set`), which waits
until every assertion is measured (a minute at most: what takes longer stays `pending`, with
`whenPending`), or runs `assert.list` next, which waits by default:

```json
[
  ["param.set", {"values": {"teeth": 31}, "wait": true}],
  ["assert.list", {}]
]
```

A real *notProven*: with the bound written as `"min": "wall - 0.01 mm"`, the case's thickness answers
*thickness: not proven: found 2 mm (at least 1.95750083 mm), expected ≥ 1.99 mm*. The wall is 2 mm, but
the proof only reaches 1.9575 mm near the rounded corners; a bound of `wall - 0.1 mm` passes.

One trap: in an expression a power binds to the number with its unit, so `50 cm^3` would mean
(50 cm)³; it is refused wherever a bound is read. Write `50 * (1 cm)^3`, or a plain number in the
document's unit (mm³ in a mm document).

**Next:** [Selection and named sets](11-selection.md).
