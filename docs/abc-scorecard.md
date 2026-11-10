# ABC scorecard: chunk 0000

Made by `cargo xtask abc` in Heavy run 41467905116 on the Strix; the same chunk at the commit this branch starts from (main 55e74c9b with the same report), Heavy run 41397009712: 7699 imported, 36 invalid (the first scorecard, at ea2f589031 before spindle tori were read, Heavy run 41347412557: 7574 and 34). Model ids are the ABC dataset's (`<id>/<n>`: the model's file `<id>_<hash>_step_<n>`); the models themselves are never committed (their licence: [testing.md](testing.md) › *Real-world models*). A timeout is a model that gave no verdict in time: slow or hanging, not yet told apart. Why an edge is refused (the distances at samples, what the prover reaches): [design/step-import.md](design/step-import.md) › *Edges the ABC corpus refused*.

Commit f306d09c581, 2026-11-02 17:27; 10000 of the chunk's 10000 STEP files run, 6 at once, 60 s and 3G per model (docs/testing.md › Real-world models).

## Stage 1: import, check, mesh, measure

| Verdict | Models | % |
|---|---:|---:|
| invalid | 39 | 0.4 |
| ok | 8006 | 80.1 |
| refused | 1947 | 19.5 |
| timeout | 8 | 0.1 |

Read as valid solids (ready for stage 2): 7978.

| Where | Models |
|---|---:|
| invalid at check | 39 |
| refused at import | 1889 |
| refused at mesh | 58 |

### Top refusal reasons

| Reason | Models | Examples |
|---|---:|---|
| edge # could not be proven within the cap of its faces' surfaces, more than the | 994 | 00000001, 00000011, 00000012 |
| edge # lies #.#e-# off the surfaces of its faces, more than the cap #e-# on | 163 | 00000015, 00000030/10, 00000045/8 |
| edge # lies #.#e# off the surface of face #, more than its tolerance ; the | 128 | 00000026/6, 00000148/1, 00000192/1 |
| OFFSET_SURFACE: an offset of a B-spline surface is not read | 125 | 00000023/3, 00000036, 00000206 |
| EDGE_CURVE: an edge that starts where it ends, on a curve that does not close | 70 | 00000991/14, 00001160/1, 00001359/2 |
| OFFSET_SURFACE: an offset of an extruded curve is not read | 70 | 00000147, 00000191, 00000202 |
| data: the imported body cannot be meshed to see whether it passes through itself | 65 | 00000277, 00000558/9, 00000808/2 |
| edge # lies #.#e-# off the surface of face #, more than its tolerance ; the | 58 | 00000387/8, 00000775, 00000777/2 |
| face # cannot be tessellated: its refinement cannot go on: a triangle #.#e-# across, far within | 40 | 00000082, 00000514, 00000569 |
| vertex # lies #.#e-# from the ends of its edges, more than the cap #e-# on | 39 | 00000044/7, 00000196, 00000218/5 |
| B_SPLINE_CURVE_WITH_KNOTS: invalid geometry: the knot vector leaves an empty parameter domain | 38 | 00000647, 00000754/1, 00001122 |
| data: whether the imported body passes through itself could not be looked at within the work | 26 | 00000323/18, 00000945/4, 00001373/16 |
| is not a valid body : shell # of the sheet is closed : a closed | 24 | 00000402, 00001369/12, 00001570 |
| EDGE_CURVE: the edge runs against its curve although same_sense says it runs with it | 21 | 00000434, 00001076, 00001315 |
| ADVANCED_FACE: a face without an edge loop on a nurbs surface, which does not close all | 17 | 00000149/2, 00000193/2, 00000204/2 |
| is # bytes; at most # are read | 11 | 00000414, 00001075, 00005586 |
| patch # asks for a tolerance finer than its coordinates can resolve; use a larger tolerance | 11 | 00000195/1, 00000950, 00001018 |
| B_SPLINE_CURVE_WITH_KNOTS: invalid geometry: knots must be finite and non-decreasing | 9 | 00000225, 00000436/2, 00000681 |
| is not a valid body : loop # of face # does not lie where the | 7 | 00003691, 00004453/7, 00004456/10 |
| is not a valid body : the loops of face # do not bound a region: | 7 | 00000411, 00000964/3, 00002815 |

### Top problems of invalid imports

| Reason | Models | Examples |
|---|---:|---|
| cannot be measured: face #: not supported yet: a boundary integral that does not settle: the | 34 | 00000139/2, 00000220/7, 00000221/8 |
| cannot be measured: face #: not supported yet: a boundary that jumps across the parameters of | 2 | 00006086/2, 00008334/4 |
| cannot be measured: face # is not bounded by its loops | 1 | 00005387 |
| cannot be measured: face #: not supported yet: a boundary through two poles of a spline | 1 | 00002252/6 |
| cannot be measured: face #: not supported yet: a face too thin, or too nearly its | 1 | 00000702 |

### Crashes, timeouts, out of memory

| Model | Verdict | Detail |
|---|---|---|
| 00000644 | timeout | no verdict within 60 s |
| 00001054 | timeout | no verdict within 60 s |
| 00001760 | timeout | no verdict within 60 s |
| 00002501/3 | timeout | no verdict within 60 s |
| 00006380/3 | timeout | no verdict within 60 s |
| 00007501/6 | timeout | no verdict within 60 s |
| 00008058 | timeout | no verdict within 60 s |
| 00008460 | timeout | no verdict within 60 s |

## Stage 2: booleans (drill, slab, corner block; union, subtract, intersect)

| Verdict | Models | % |
|---|---:|---:|
| invalid | 4 | 0.1 |
| ok | 7793 | 97.7 |
| refused | 175 | 2.2 |
| timeout | 6 | 0.1 |

| Tool | Verdict | Models |
|---|---|---:|
| corner | invalid | 3 |
| corner | ok | 7917 |
| corner | refused | 52 |
| drill | invalid | 2 |
| drill | ok | 7885 |
| drill | refused | 85 |
| slab | invalid | 2 |
| slab | ok | 7848 |
| slab | refused | 122 |

### Top boolean refusals

| Reason | Models | Examples |
|---|---:|---|
| slab union: boolean does not support this: a freeform face that closes on itself and into | 25 | 00000324/19, 00000380/1, 00000722 |
| slab union: boolean does not support this: two curves leave a point in the same direction | 22 | 00000032/12, 00000761/8, 00001143 |
| drill union: boolean does not support this: two curves leave a point in the same direction | 20 | 00000078/19, 00001467/51, 00003450/1 |
| drill union: boolean does not support this: a freeform face that closes on itself and into | 17 | 00000407/4, 00001144, 00002192/1 |
| drill union: boolean does not support this: two points closer together than the tolerance can tell | 11 | 00000245, 00000857, 00001482/66 |
| drill union: the result would have zero thickness at : two of its parts touch there | 9 | 00001356, 00002306/6, 00002364/7 |
| slab union: boolean produced an invalid body: edge # is #.#e# away from the surface of | 6 | 00000530/2, 00003174, 00003938 |
| corner union: boolean does not support this: a freeform face that closes on itself and into | 5 | 00000826/2, 00005401/5, 00007946 |
| slab union: boolean does not support this: two points closer together than the tolerance can tell | 5 | 00005016/1, 00005378/19, 00006603/3 |
| corner union: the result would have zero thickness at : two of its parts touch there | 4 | 00000809/3, 00001109, 00003233/3 |
| drill union: boolean does not support this: a freeform face whose parametrisation stalls or folds somewhere | 3 | 00001470/54, 00001721, 00009846/2 |
| drill union: boolean does not support this: the freeform faces it meets need more memory for | 3 | 00003067/54, 00003068/55, 00003069/56 |
| drill union: boolean produced an invalid body: edge # could not be proven within its tolerance | 3 | 00001927, 00003348/1, 00007474 |
| drill union: boolean produced an invalid body: edge # is #.#e-# away from the surface of | 3 | 00000143/6, 00002833/18, 00002976 |
| slab union: boolean does not support this: an intersection curve could not be fitted within the | 3 | 00009287/5, 00009288/6, 00009780/5 |
| slab union: boolean does not support this: imprint lost: a curve in which two faces cross | 3 | 00002928, 00004655/6, 00005460/14 |
| corner union: boolean does not support this: two curves leave a point in the same direction | 2 | 00006696, 00009956/91 |
| drill union: boolean does not support this: a curve that is partly on and partly off | 2 | 00000033/13, 00008919/1 |
| drill union: boolean does not support this: a piece of a face too thin to find | 2 | 00004207, 00007346/1 |
| slab union: boolean does not support this: a point too close to a face boundary to | 2 | 00003321/9, 00007608/6 |

### Wrong booleans (identity or check)

| Reason | Models | Examples |
|---|---:|---|
| drill union check: cannot be measured: face #: not supported yet: a boundary integral that does | 2 | 00007037/22, 00009409 |
| slab union check: cannot be measured: face #: not supported yet: a boundary integral that does | 2 | 00001033, 00004752/3 |

### Boolean crashes, timeouts, out of memory

| Model | Verdict | Detail |
|---|---|---|
| 00003163/2 | timeout | no verdict within 180 s |
| 00003662/1 | timeout | no verdict within 180 s |
| 00003663/2 | timeout | no verdict within 180 s |
| 00003890/3 | timeout | no verdict within 180 s |
| 00004112 | timeout | no verdict within 180 s |
| 00004795/41 | timeout | no verdict within 180 s |

## Volumes against FreeCAD (a sample of stage 1's accepted models)

| Verdict | Models |
|---|---:|
| agree | 199 |
| disagree | 1 |

| Model | Ours | FreeCAD | Relative |
|---|---:|---:|---:|
| 00003315/3 | 1.985067e5 | 1.972927e5 | 6.153049e-3 |

