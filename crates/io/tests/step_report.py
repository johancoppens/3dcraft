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


def read(path):
    record = {"file": os.path.basename(path)}
    shape = Part.Shape()
    shape.read(path)
    record["shape"] = measure(shape)
    record["solids"] = [measure(s) for s in shape.Solids]
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
