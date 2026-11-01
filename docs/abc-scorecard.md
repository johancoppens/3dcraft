# ABC scorecard: chunk 0000

Made by `cargo xtask abc` in Heavy run 41347412557 (import 1304 s, booleans 1433 s on the Strix). Model ids are the ABC dataset's (`<id>/<n>`: the model's file `<id>_<hash>_step_<n>`); the models themselves are never committed (their licence: [testing.md](testing.md) › *Real-world models*). A timeout is a model that gave no verdict in time: slow or hanging, not yet told apart.

Commit 9aca23b396, 2026-11-01 22:18; 10000 of the chunk's 10000 STEP files run, 6 at once, 60 s and 3G per model (docs/testing.md › Real-world models).

## Stage 1: import, check, mesh, measure

| Verdict | Models | % |
|---|---:|---:|
| invalid | 34 | 0.3 |
| ok | 7574 | 75.7 |
| refused | 2386 | 23.9 |
| timeout | 6 | 0.1 |

Read as valid solids (ready for stage 2): 7548.

| Where | Models |
|---|---:|
| invalid at check | 34 |
| refused at import | 2347 |
| refused at mesh | 39 |

### Top refusal reasons

| Reason | Models | Examples |
|---|---:|---|
| edge # could not be proven within the cap of its faces' surfaces, more than the | 1020 | 00000001, 00000011, 00000012 |
| edge # lies #.#e-# off the surfaces of its faces, more than the cap #e-# on | 647 | 00000010, 00000015, 00000027/7 |
| TOROIDAL_SURFACE: invalid geometry: torus major radius must be larger than its minor radius | 192 | 00000182/12, 00000231, 00000278 |
| OFFSET_SURFACE: an offset of a B-spline surface is not read | 120 | 00000023/3, 00000036, 00000206 |
| OFFSET_SURFACE: an offset of an extruded curve is not read | 68 | 00000147, 00000191, 00000202 |
| EDGE_CURVE: an edge that starts where it ends, on a curve that does not close | 66 | 00000991/14, 00001160/1, 00001359/2 |
| data: the imported body cannot be meshed to see whether it passes through itself | 52 | 00000277, 00000808/2, 00000980/3 |
| B_SPLINE_CURVE_WITH_KNOTS: invalid geometry: the knot vector leaves an empty parameter domain | 30 | 00000647, 00001122, 00001123/1 |
| vertex # lies #.#e-# from the ends of its edges, more than the cap #e-# on | 25 | 00000196, 00000232, 00000612 |
| is not a valid body : shell # of the sheet is closed : a closed | 22 | 00001369/12, 00001570, 00001573/3 |
| EDGE_CURVE: the edge runs against its curve although same_sense says it runs with it | 21 | 00000434, 00001076, 00001315 |
| data: whether the imported body passes through itself could not be looked at within the work | 18 | 00000323/18, 00000945/4, 00001373/16 |
| ADVANCED_FACE: a face without an edge loop on a nurbs surface, which does not close all | 17 | 00000149/2, 00000193/2, 00000204/2 |
| file.export: Part #: face # cannot be tessellated: its refinement cannot go on: a triangle #.#e-# | 13 | 00000082, 00003161, 00003724 |
| is # bytes; at most # are read | 11 | 00000414, 00001075, 00005586 |
| B_SPLINE_CURVE_WITH_KNOTS: invalid geometry: knots must be finite and non-decreasing | 9 | 00000225, 00000436/2, 00000681 |
| is not a valid body : loop # of face # does not lie where the | 7 | 00003691, 00004453/7, 00004456/10 |
| is not a valid body : the loops of face # do not bound a region: | 7 | 00000411, 00000964/3, 00002815 |
| file.export: Part #: patch # asks for a tolerance finer than its coordinates can resolve; use | 6 | 00000950, 00001018, 00001550 |
| file.export: M# Socket head cap screw-#: face # cannot be tessellated: its refinement cannot go on: | 4 | 00000569, 00001761, 00003254 |

### Top problems of invalid imports

| Reason | Models | Examples |
|---|---:|---|
| cannot be measured: face #: not supported yet: a boundary integral that does not settle: the | 32 | 00000139/2, 00000220/7, 00000221/8 |
| cannot be measured: face # is not bounded by its loops | 1 | 00005387 |
| cannot be measured: face #: not supported yet: a boundary that jumps across the parameters of | 1 | 00008334/4 |

### Crashes, timeouts, out of memory

| Model | Verdict | Detail |
|---|---|---|
| 00001760 | timeout | no verdict within 60 s |
| 00002501/3 | timeout | no verdict within 60 s |
| 00003794/20 | timeout | no verdict within 60 s |
| 00007501/6 | timeout | no verdict within 60 s |
| 00008058 | timeout | no verdict within 60 s |
| 00008460 | timeout | no verdict within 60 s |

## Stage 2: booleans (drill, slab, corner block; union, subtract, intersect)

| Verdict | Models | % |
|---|---:|---:|
| invalid | 3 | 0.0 |
| ok | 7447 | 98.7 |
| refused | 94 | 1.2 |
| timeout | 4 | 0.1 |

| Tool | Verdict | Models |
|---|---|---:|
| corner | invalid | 2 |
| corner | ok | 7512 |
| corner | refused | 30 |
| drill | invalid | 2 |
| drill | ok | 7495 |
| drill | refused | 47 |
| slab | invalid | 1 |
| slab | ok | 7482 |
| slab | refused | 61 |

### Top boolean refusals

| Reason | Models | Examples |
|---|---:|---|
| slab union: boolean does not support this: two curves leave a point in the same direction | 16 | 00000032/12, 00001143, 00001795/13 |
| drill union: boolean does not support this: two points closer together than the tolerance can tell | 10 | 00000245, 00001482/66, 00002605 |
| drill union: boolean does not support this: two curves leave a point in the same direction | 7 | 00000078/19, 00001467/51, 00003461/12 |
| drill union: the result would have zero thickness at : two of its parts touch there | 7 | 00001356, 00002306/6, 00002364/7 |
| slab union: boolean produced an invalid body: edge # is #.#e# away from the surface of | 6 | 00000530/2, 00003174, 00003938 |
| corner union: the result would have zero thickness at : two of its parts touch there | 4 | 00000809/3, 00001109, 00003233/3 |
| drill union: boolean does not support this: a freeform face whose parametrisation stalls or folds somewhere | 3 | 00001470/54, 00001721, 00009846/2 |
| drill union: boolean does not support this: the freeform faces it meets need more memory for | 3 | 00003067/54, 00003068/55, 00003069/56 |
| drill union: boolean produced an invalid body: edge # is #.#e-# away from the surface of | 3 | 00000143/6, 00002833/18, 00002976 |
| slab union: boolean does not support this: an intersection curve could not be fitted within the | 3 | 00009287/5, 00009288/6, 00009780/5 |
| slab union: boolean does not support this: two points closer together than the tolerance can tell | 3 | 00005016/1, 00008826/6, 00008924/6 |
| corner union: boolean does not support this: two curves leave a point in the same direction | 2 | 00006696, 00009956/91 |
| drill union: boolean does not support this: a curve that is partly on and partly off | 2 | 00000033/13, 00008919/1 |
| corner subtract: the result would have zero thickness at : two of its parts touch there | 1 | 00008105/1 |
| corner union: boolean does not support this: two faces that go through each other by between | 1 | 00002543/5 |
| corner union: boolean does not support this: two points closer together than the tolerance can tell | 1 | 00008805/1 |
| drill intersect: boolean produced an invalid body: edge # is #.#e-# away from the surface of | 1 | 00005644 |
| drill intersect: tool: the result cannot be meshed to see whether it passes through itself | 1 | 00004425 |
| drill subtract: tool: the result cannot be meshed to see whether it passes through itself | 1 | 00006399 |
| drill union: boolean does not support this: a freeform face that closes into a point too | 1 | 00000314/9 |

### Wrong booleans (identity or check)

| Reason | Models | Examples |
|---|---:|---|
| drill union check: cannot be measured: face #: not supported yet: a boundary integral that does | 2 | 00007037/22, 00009409 |
| slab union check: cannot be measured: face #: not supported yet: a boundary integral that does | 1 | 00004752/3 |

### Boolean crashes, timeouts, out of memory

| Model | Verdict | Detail |
|---|---|---|
| 00003662/1 | timeout | no verdict within 180 s |
| 00003663/2 | timeout | no verdict within 180 s |
| 00004112 | timeout | no verdict within 180 s |
| 00004795/41 | timeout | no verdict within 180 s |

## Volumes against FreeCAD (a sample of stage 1's accepted models)

| Verdict | Models |
|---|---:|
| agree | 196 |
| disagree | 4 |

| Model | Ours | FreeCAD | Relative |
|---|---:|---:|---:|
| 00000961 | 2.773611e3 | 2.794391e3 | 7.436367e-3 |
| 00003938 | 7.935472e3 | 7.918261e3 | 2.173493e-3 |
| 00005776/1 | 2.986023e4 | 2.976634e4 | 3.154065e-3 |
| 00008074/1 | 9.817477e5 | 9.891263e5 | 7.459719e-3 |

