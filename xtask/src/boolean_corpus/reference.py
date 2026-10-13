# Run by `cargo xtask boolean-corpus` inside FreeCAD's command-line
# interpreter (docs/testing.md, oracle 8). Reads pairs of solids, one JSON
# object per line, from CRAFT3D_FC_IN; builds each with FreeCAD's Part
# primitives and computes common, cut (A - B) and fuse; appends what FreeCAD
# measures to CRAFT3D_FC_OUT as `<id> TAB <json>`, one line per pair, flushed
# after each so that a pair that hangs or crashes FreeCAD can be told from
# the ones before it.
#
# Only FreeCAD's documented Python API is used. Nothing is judged here: the
# xtask decides which answers are reliable.
import json
import os
import signal
import sys

import FreeCAD
import Part

# Seconds one pair may take. SIGALRM is left at its default action, which
# ends the process: the geometry kernel does not return to Python while it
# works, so a Python handler would never run.
LIMIT = int(os.environ.get("CRAFT3D_FC_LIMIT", "120"))


def unit(v):
    n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) ** 0.5
    return [v[0] / n, v[1] / n, v[2] / n]


def domed_block(dx, dy, height, spline):
    """A block dx x dy on the xy plane under a roof that is a bicubic
    Bezier patch (or, with `spline`, the same patch as a B-spline surface)
    over the regular 4 x 4 net with the control point (i, j) at
    (dx i / 3, dy j / 3, height (1 + 0.15 (((7 i + 3 j) mod 5) - 2))): the
    domed block of 3DCraft's test kit. Four planar walls under the roof's
    own edges, a floor, one shell, six faces."""
    poles = [
        [
            FreeCAD.Vector(
                dx * i / 3.0,
                dy * j / 3.0,
                height * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) - 2)),
            )
            for j in range(4)
        ]
        for i in range(4)
    ]
    if spline:
        surface = Part.BSplineSurface()
        surface.buildFromPolesMultsKnots(
            poles, [4, 4], [4, 4], [0.0, 1.0], [0.0, 1.0], False, False, 3, 3
        )
    else:
        surface = Part.BezierSurface()
        surface.increase(3, 3)
        for i in range(4):
            for j in range(4):
                surface.setPole(i + 1, j + 1, poles[i][j])
    roof = Part.Face(surface)
    ground = [
        FreeCAD.Vector(0.0, 0.0, 0.0),
        FreeCAD.Vector(dx, 0.0, 0.0),
        FreeCAD.Vector(dx, dy, 0.0),
        FreeCAD.Vector(0.0, dy, 0.0),
    ]
    top = [poles[0][0], poles[3][0], poles[3][3], poles[0][3]]

    def roof_edge(p, q):
        # The roof's own edge between two of its corners, so that the
        # wall below shares it.
        for edge in roof.Edges:
            a, b = edge.Vertexes[0].Point, edge.Vertexes[-1].Point
            if ((a - p).Length < 1e-9 and (b - q).Length < 1e-9) or (
                (a - q).Length < 1e-9 and (b - p).Length < 1e-9
            ):
                return edge
        raise ValueError("the roof has no edge between %s and %s" % (p, q))

    floor_edges = [Part.makeLine(ground[k], ground[(k + 1) % 4]) for k in range(4)]
    uprights = [Part.makeLine(ground[k], top[k]) for k in range(4)]
    faces = []
    for k in range(4):
        n = (k + 1) % 4
        wire = Part.Wire(
            [floor_edges[k], uprights[n], roof_edge(top[k], top[n]), uprights[k]]
        )
        faces.append(Part.Face(wire))
    faces.append(Part.Face(Part.Wire(floor_edges)))
    faces.append(roof)
    solid = Part.Solid(Part.Shell(faces))
    if len(solid.Shells) != 1 or len(solid.Faces) != 6 or not solid.isValid():
        raise ValueError("the domed block did not close")
    return solid


def build(solid):
    kind = solid["kind"]
    if kind == "block":
        shape = Part.makeBox(*solid["size"])
    elif kind == "cylinder":
        shape = Part.makeCylinder(solid["radius"], solid["height"])
    elif kind == "cone":
        shape = Part.makeCone(solid["radius"], 0.0, solid["height"])
    elif kind == "sphere":
        shape = Part.makeSphere(solid["radius"])
    elif kind == "torus":
        shape = Part.makeTorus(solid["major"], solid["minor"])
    elif kind in ("dome", "spline"):
        shape = domed_block(*solid["size"], spline=(kind == "spline"))
    else:
        raise ValueError("unknown kind " + kind)
    # The frame as 3DCraft makes it: z normalised, x made perpendicular to
    # it, y = z cross x.
    o = solid["origin"]
    z = unit(solid["z"])
    h = solid["x"]
    d = h[0] * z[0] + h[1] * z[1] + h[2] * z[2]
    x = unit([h[0] - z[0] * d, h[1] - z[1] * d, h[2] - z[2] * d])
    y = [
        z[1] * x[2] - z[2] * x[1],
        z[2] * x[0] - z[0] * x[2],
        z[0] * x[1] - z[1] * x[0],
    ]
    matrix = FreeCAD.Matrix(
        x[0], y[0], z[0], o[0],
        x[1], y[1], z[1], o[1],
        x[2], y[2], z[2], o[2],
        0.0, 0.0, 0.0, 1.0,
    )
    shape.Placement = FreeCAD.Placement(matrix)
    return shape


def spline_edges(shape):
    """Whether an edge is neither a line nor a conic: an approximated curve."""
    for edge in shape.Edges:
        try:
            name = edge.Curve.TypeId
        except Exception:
            # A degenerated edge (the pole of a sphere) has no curve.
            continue
        if name not in (
            "Part::GeomLine",
            "Part::GeomCircle",
            "Part::GeomEllipse",
            "Part::GeomHyperbola",
            "Part::GeomParabola",
        ):
            return True
    return False


def measure(shape, result):
    solids = shape.Solids
    volume = 0.0
    moment = [0.0, 0.0, 0.0]
    faces_in_solids = 0
    for solid in solids:
        v = solid.Volume
        c = solid.CenterOfMass
        volume += v
        moment[0] += v * c.x
        moment[1] += v * c.y
        moment[2] += v * c.z
        faces_in_solids += len(solid.Faces)
    out = {"v": volume, "a": shape.Area if solids else 0.0}
    if solids and volume != 0.0:
        out["c"] = [moment[0] / volume, moment[1] / volume, moment[2] / volume]
    if not result:
        # An operand: one solid by construction, and its box is not needed.
        if len(solids) != 1 or not shape.isValid():
            raise ValueError("operand is not one valid solid")
        return out
    if solids and volume != 0.0:
        b = shape.BoundBox
        out["bb"] = [float("%.9g" % t) for t in (b.XMin, b.YMin, b.ZMin, b.XMax, b.YMax, b.ZMax)]
    out["n"] = [len(solids), len(shape.Shells)]
    out["ok"] = bool(shape.isValid())
    if result:
        if len(shape.Faces) > faces_in_solids:
            # Faces that bound no solid: not a solid result. (Fewer is
            # fine: two solids that share a face.)
            out["loose"] = len(shape.Faces) - faces_in_solids
        if solids:
            out["t"] = float("%.3g" % shape.getTolerance(1))
            out["len"] = float("%.9g" % shape.Length)
            if spline_edges(shape):
                out["spl"] = True
    return out


def attempt(make):
    try:
        return measure(make(), True)
    except BaseException as error:  # FreeCAD raises its own exception types
        return {"err": ("%s: %s" % (type(error).__name__, error))[:200]}


def main():
    with open(os.environ["CRAFT3D_FC_IN"]) as source:
        cases = [json.loads(line) for line in source if line.strip()]
    with open(os.environ["CRAFT3D_FC_OUT"], "a") as out:
        version = FreeCAD.Version()
        out.write(
            "#version\tFreeCAD %s.%s.%s (%s, %s), OCCT %s\n"
            % (version[0], version[1], version[2], version[3], version[7][:12]
               if len(version) > 7 else "?", getattr(Part, "OCC_VERSION", "?"))
        )
        out.flush()
        for case in cases:
            out.write("#start\t%s\n" % case["id"])
            out.flush()
            signal.alarm(LIMIT)
            try:
                a = build(case["a"])
                b = build(case["b"])
                ref = {"a": measure(a, False), "b": measure(b, False)}
            except BaseException as error:
                ref = {"err": ("%s: %s" % (type(error).__name__, error))[:200]}
            else:
                ref["common"] = attempt(lambda: a.common(b))
                ref["cut"] = attempt(lambda: a.cut(b))
                ref["fuse"] = attempt(lambda: a.fuse(b))
            signal.alarm(0)
            out.write("%s\t%s\n" % (case["id"], json.dumps(ref, separators=(",", ":"))))
            out.flush()
        out.write("#done\n")


# FreeCAD runs a script a second time, as another kind of file, when the
# first run raised: once is enough.
if not getattr(sys, "_craft3d_boolean_corpus", False):
    sys._craft3d_boolean_corpus = True
    main()
