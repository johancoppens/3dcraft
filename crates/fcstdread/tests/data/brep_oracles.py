# Shapes FreeCAD makes, written as BRep files with what FreeCAD says of
# them, for `tests/freecad.rs` (opt-in, `cargo xtask freecad`): periodic
# B-spline curves of several degrees and first multiplicities (the
# alignment of their poles), and a solid on a spindle torus (half a disc
# that reaches across its axis, turned about it).
#
# Headless (FreeCADCmd), writing into CRAFT3D_FC_OUT (a directory):
# `<name>.brp` per shape and `oracles.json`. FreeCAD runs as a separate
# program; nothing of it is linked.

import json
import math
import os

import FreeCAD as App
import Part

V = App.Vector
OUT = os.environ["CRAFT3D_FC_OUT"]
result = {}
try:
    poles = [V(30, 0, 0), V(38, 3, 0), V(41, 9, 0), V(37, 15, 0), V(30, 13, 0), V(25, 8, 0), V(26, 3, 0)]
    for degree in (2, 3, 4, 5):
        for m1 in (1, 2):
            if m1 > degree:
                continue
            n = len(poles)
            # Periodic: the multiplicities of all knots but the last add up
            # to the poles, the last equal to the first.
            inner = [m1] + [1] * (n - m1)
            mults = inner + [m1]
            knots = [0.0]
            for k in range(1, len(mults)):
                knots.append(knots[-1] + 1.0 + 0.37 * (k % 3))
            c = Part.BSplineCurve()
            c.buildFromPolesMultsKnots(poles, mults, knots, True, degree)
            name = "periodic_d%d_m%d" % (degree, m1)
            Part.Face(Part.Wire([c.toShape()])).exportBrep(os.path.join(OUT, name + ".brp"))
            a, b = c.FirstParameter, c.LastParameter
            ts = [a + (b - a) * k / 11.0 for k in range(12)]
            result[name] = {"points": [[t, list(c.value(t))] for t in ts]}
    # Half a disc of radius 3 about (2, 0, 0) in the xz plane, cut along
    # the z axis, turned about it: the apple of a spindle torus (major 2,
    # minor 3), a valid solid.
    h = math.sqrt(9.0 - 4.0)
    arc = Part.ArcOfCircle(Part.Circle(V(2, 0, 0), V(0, 1, 0), 3), -math.acos(-2.0 / 3.0), math.acos(-2.0 / 3.0))
    e1 = arc.toShape()
    p0, p1 = e1.Vertexes[0].Point, e1.Vertexes[-1].Point
    face = Part.Face(Part.Wire([e1, Part.LineSegment(p1, p0).toShape()]))
    apple = face.revolve(V(0, 0, 0), V(0, 0, 1), 360)
    solid = Part.Solid(Part.Shell(apple.Faces)) if apple.ShapeType != "Solid" else apple
    solid.exportBrep(os.path.join(OUT, "apple.brp"))
    result["apple.brp"] = {"volume": solid.Volume, "area": solid.Area, "h": h, "valid": solid.isValid()}
except Exception as e:
    result["error"] = repr(e)
with open(os.path.join(OUT, "oracles.json"), "w") as f:
    json.dump(result, f, indent=1, sort_keys=True)
