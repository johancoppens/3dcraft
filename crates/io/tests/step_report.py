# Run by the STEP checks (crates/io/tests/step_freecad.rs and
# crates/engine/tests/step_freecad.rs) inside FreeCAD's command-line
# interpreter (docs/testing.md, oracle 8). Reads the STEP files listed one
# per line in CRAFT3D_FC_IN and appends what FreeCAD makes of each to
# CRAFT3D_FC_OUT as one JSON object per line, flushed after each so that a
# file that crashes FreeCAD can be told from the ones before it.
#
# Only FreeCAD's documented Python API is used. Nothing is judged here: the
# tests compare the numbers with ours.
import json
import os

import FreeCAD
import Part

try:
    import Import
except Exception:  # The names are then reported as unavailable.
    Import = None


def vector(v):
    return [v.x, v.y, v.z]


def box(b):
    return [b.XMin, b.YMin, b.ZMin, b.XMax, b.YMax, b.ZMax]


def measure(shape):
    record = {
        "valid": bool(shape.isValid()),
        "type": shape.ShapeType,
        "area": shape.Area,
        "bbox": box(shape.BoundBox),
        "faces": len(shape.Faces),
        "edges": len(shape.Edges),
        "vertices": len(shape.Vertexes),
        "shells": len(shape.Shells),
        "closed": [bool(s.isClosed()) for s in shape.Shells],
        "surfaces": sorted(set(f.Surface.TypeId for f in shape.Faces)),
    }
    try:
        record["volume"] = shape.Volume
        record["centre"] = vector(shape.CenterOfMass)
    except Exception as error:  # A compound has no centre of mass.
        record["volume_error"] = str(error)
    # Edges that are really degenerated (a pole, an apex) or seams: what
    # FreeCAD added to faces we wrote without them.
    degenerated = 0
    seams = 0
    for face in shape.Faces:
        for edge in face.Edges:
            try:
                if edge.Degenerated:
                    degenerated += 1
                elif edge.isSeam(face):
                    seams += 1
            except Exception:
                pass
    record["degenerated"] = degenerated
    record["seams"] = seams
    return record


# Five-point Gauss-Legendre on [-1, 1]: exact to degree 9.
GAUSS = (
    (-0.9061798459386640, 0.2369268850561891),
    (-0.5384693101056831, 0.4786286704993665),
    (0.0, 0.5688888888888889),
    (0.5384693101056831, 0.4786286704993665),
    (0.9061798459386640, 0.2369268850561891),
)
def breaks(geometry, method, a, b):
    """Where to cut [a, b] for the quadrature: at the knots of a spline
    (it is a polynomial between them), each span in four."""
    inner = []
    if hasattr(geometry, method):
        inner = [k for k in getattr(geometry, method)() if a < k < b]
    cuts = [a] + sorted(set(inner)) + [b]
    pieces = 4 if inner or hasattr(geometry, method) else 16
    out = []
    for lo, hi in zip(cuts, cuts[1:]):
        out.extend(lo + (hi - lo) * i / pieces for i in range(pieces))
    return out + [b]


def nodes(geometry, method, a, b):
    cuts = breaks(geometry, method, a, b)
    for lo, hi in zip(cuts, cuts[1:]):
        mid, half = (lo + hi) / 2.0, (hi - lo) / 2.0
        for x, w in GAUSS:
            yield mid + half * x, half * w


def wire_area(wire):
    """The area a closed plane wire encloses: half the length of the
    integral of r x dr along it."""
    total = FreeCAD.Vector()
    for edge in wire.OrderedEdges:
        a, b = edge.ParameterRange
        sign = -1.0 if edge.Orientation == "Reversed" else 1.0
        for t, w in nodes(edge.Curve, "getKnots", a, b):
            total += edge.valueAt(t).cross(edge.derivative1At(t)) * (w * sign)
    return total.Length / 2.0


def quadrature_volume(solid):
    """The volume of a solid by the divergence theorem, a third of the flux
    of r through its faces, by Gauss-Legendre on the curves and surfaces
    FreeCAD imported: a measure that does not go through FreeCAD's own
    integration of faces, which is approximate for a plane face bounded by
    a spline. It takes milliseconds and no memory to speak of, and gives
    FreeCAD's own volume to 1e-13 where that one is exact (a block, a
    cylinder, a slot). A plane face may have any wires; a curved face must
    be a whole patch of its surface, or this raises."""
    total = 0.0
    for face in solid.Faces:
        (u0, u1, v0, v1) = face.ParameterRange
        surface = face.Surface
        if surface.TypeId == "Part::GeomPlane":
            area = wire_area(face.OuterWire)
            for wire in face.Wires:
                if not wire.isSame(face.OuterWire):
                    area -= wire_area(wire)
            u, v = (u0 + u1) / 2.0, (v0 + v1) / 2.0
            total += face.valueAt(u, v).dot(face.normalAt(u, v)) * area
            continue
        if len(face.Wires) != 1 or len(face.Edges) not in (3, 4):
            raise ValueError("a curved face that is not a whole patch")
        vs = list(nodes(surface, "getVKnots", v0, v1))
        for u, wu in nodes(surface, "getUKnots", u0, u1):
            for v, wv in vs:
                du, dv = face.derivative1At(u, v)
                point = face.valueAt(u, v)
                flux = abs(point.dot(du.cross(dv)))
                side = point.dot(face.normalAt(u, v))
                total += (flux if side > 0 else -flux) * wu * wv
    return total / 3.0


def read(path):
    record = {"file": os.path.basename(path)}
    shape = Part.Shape()
    shape.read(path)
    record["shape"] = measure(shape)
    record["solids"] = [measure(s) for s in shape.Solids]
    # Only where asked for by the file's name: most bodies have curved
    # faces that are trimmed.
    if "spline" in record["file"]:
        try:
            record["quadrature_volume"] = sum(
                quadrature_volume(s) for s in shape.Solids
            )
        except Exception as error:
            record["quadrature_error"] = str(error)
    record["names"] = None
    if Import is not None:
        document = FreeCAD.newDocument("StepCheck")
        try:
            Import.insert(path, document.Name)
            document.recompute()
            # The objects that carry a shape, and the containers FreeCAD
            # makes for a product with several solids.
            record["names"] = [
                o.Label
                for o in document.Objects
                if o.TypeId in ("Part::Feature", "App::Part")
            ]
        finally:
            FreeCAD.closeDocument(document.Name)
    return record


with open(os.environ["CRAFT3D_FC_IN"]) as listing:
    paths = [line.strip() for line in listing if line.strip()]
with open(os.environ["CRAFT3D_FC_OUT"], "a") as out:
    out.write(json.dumps({"version": FreeCAD.Version()[:3]}) + "\n")
    out.flush()
    for path in paths:
        try:
            record = read(path)
        except Exception as error:
            record = {"file": os.path.basename(path), "error": str(error)}
        out.write(json.dumps(record) + "\n")
        out.flush()
