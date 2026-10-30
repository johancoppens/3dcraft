# 17. Assemblies

*Put parts together: place instances of your documents (and Jigs) in an assembly, give each its own sizes, join them
with mates, and export the whole as one STEP file with its structure.*

An **assembly** is a document of its own, saved as a `.3dcraftasm` file. It holds no features: it holds
**instances** of other documents (parts, Jigs, or other assemblies) and **mates** that say how they sit against each
other. An instance is a reference, not a copy: you edit a part in its own document, and the assembly follows when you
save it.

## Instances

Start an assembly and place a part twice:

```json
[
  ["assembly.new", {"units": "mm"}],
  ["instance.add", {"source": "bar.3dcraft", "name": "Bar[1]"}],
  ["instance.add", {"source": "bar.3dcraft", "name": "Bar[2]", "position": [0, 0, 10]}]
]
```

The `source` is read from the folder 3DCraft may read from, and kept as a path relative to the assembly's file, with
the SHA-256 of its bytes. `position` and `rotation` (a rotation vector in degrees) say where an instance starts; mates
move it from there.

**Sizes per instance.** A part with public parameters (a Jig's, `variable.publish`) can be placed with other values:
`"params": {"len": "80 mm"}`, or an expression of the assembly's own variables (`"params": {"len": "long"}`). Each
different set of values is a *configuration*, built once however many instances use it.

**One body of a multi-body part.** `"body": "Gear"` shows only the body the feature `Gear` made, where the part puts
it. This is how the bodies of an existing Jig become parts of an assembly. If the part changes so that `Gear` makes no
body any more, the instance says so by name; it never shows another body instead.

**Following the part.** `assembly.reload` reads the sources again and lists the instances whose part changed.
`"pinned": true` keeps an instance to the part as it is now: those bytes are kept in the assembly's file, so the
instance shows them whatever the part's file holds later, and `assembly.pack` packs them. `instance.open` opens an instance's part in its own tab.

## Mates

A mate joins geometry of two instances. It names that geometry through the instance, with the part's own names
(`Bar[1]/F1:face(1)` is the top of the block `F1` of the instance `Bar[1]`), or the world (`world/xy`, `world/z`,
`world/origin`):

```json
[
  ["mate.add", {"kind": "fixed", "a": "world/xy", "b": "Bar[1]/F1:face(0)", "name": "Ground"}],
  ["mate.add", {"kind": "revolute", "a": "Bar[1]/F1:face(1)", "b": "Bar[2]/F1:face(0)", "name": "Hinge"}],
  ["joint.set", {"mate": "Hinge", "value": "30 deg"}]
]
```

Each side gives a **frame**: a flat face gives its plane at the face's centre with its outward normal, a round face
its axis, a circular edge its centre and axis, a corner a point, a datum of the part its own. The second frame is put
against the first, facing it (`"flip": true` keeps it the same way round), moved by `offset` and turned by `angle`.

| Kind | What stays free |
|---|---|
| `fixed` | nothing |
| `revolute` | a turn about the axis (`<mate>_angle`) |
| `slider` | a slide along it (`<mate>_distance`) |
| `cylindrical` | both |
| `planar` | sliding in the plane and turning in it |
| `ball` | turning about the point |

`joint.set` sets a free motion: it makes the assembly variable `Hinge_angle`, which you can use in formulas and
publish as a parameter (it then shows as a slider). `assembly.solve` says whether the mates are met and what each
instance can still do. A mate on a face that is gone from the part fails with that face's name and never moves to
another face.

Gears, racks and screws couple two mates: `coupling.add {"kind": "gear", "mates": ["A", "B"], "value": 2}`. A
coupling whose mate fails, or whose value does not evaluate, fails by name and `assembly.solve` is not ok.

Closed loops (a four-bar linkage) are solved as a whole, and `assembly.solve` says which mates contradict each other
and by how much when they cannot all be met.

## In the window

The left panel of an assembly is its tree: the instances (a subassembly's own nested under it), then the mates with
their kind and value, and how many degrees of freedom are left. Click a row to select the instance, double-click to
fly to it, the eye hides it and **Open** opens its part in a tab of its own.

Every joint you can set by hand is a slider on the **Parameters** tab. In the viewport, **Alt and a drag** on a part
moves it along its free motion: drag the crank and the linkage follows. Both are `joint.set`.

To join two parts, select a face of each (Ctrl-click the second), right-click and choose *Revolute between these*
(or *Fixed*, *Slider*, *Cylindrical*, *Planar*, *Ball*): that is `mate.add`.

## Motion

An assembly plays over the timeline `t` as a part does ([chapter 4](04-window.md)): give it a variable `t` and drive
a joint by a formula of it.

```json
[
  ["variable.set", {"name": "t", "expr": "0"}],
  ["variable.set", {"name": "Drive_angle", "expr": "360 deg * t"}],
  ["view.motion", {"t": 0.25}]
]
```

Each moment is solved from the one before it, so the linkage stays the way it was assembled (elbow up stays up),
and the same `t` always shows the same state. Where the mechanism cannot go further (a dead point), `view.motion`
says so with the `t` it stopped at, and `motion.check` lists every dead point it meets. A trace names its point
through an instance, as a mate does: `view.trace {"point": "Coupler/End2"}`.

## Subassemblies

An instance of a `.3dcraftasm` is a **subassembly**. By default it moves as one block (`"mode": "rigid"`);
`"mode": "flexible"` lets its own joints move in the parent. Mates reach into it through its instances:
`Leg/Arm/F1:face(1)`. An assembly that would contain itself is refused.

## Packing, export and checks

- `assembly.pack` puts copies of all parts into the `.3dcraftasm` file, so it opens anywhere; `assembly.unpack`
  writes them out as files again.
- `file.export` to `.step` writes the assembly with its structure: each part once, placed by every instance. STL, 3MF
  and OBJ hold the placed bodies. Invisible instances are left out of all of them.
- Checks name instances: `assert.add {"name": "gap", "measure": "clearance", "from": "Bar[1]", "to": "Bar[2]", "min": "1 mm"}`,
  `overlap` for interference and `mass` work as in a part ([chapter 10](10-measure.md)). Renaming an instance renames
  it in its checks; an instance a check names is not deleted until the check is.
- `assembly.inspect` lists the instances, their configurations, the mates and how they stand.
