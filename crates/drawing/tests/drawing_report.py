# Run by tests/freecad.rs (and by the engine's tests/drawing_freecad.rs)
# inside FreeCAD's command-line interpreter. Imports every .dxf and .svg
# file of the directory named in CRAFT3D_FC_IN and writes what FreeCAD makes
# of each, one line of tab-separated key=value per file, to CRAFT3D_FC_OUT:
# the edges it got
# (how many, of which curve, their total length and bounding box), the wires
# they join into, and the area inside (the largest closed wire less the
# others: a plate with holes).
import os
import traceback

import FreeCAD
import Part


def measure(document):
    edges = []
    for item in document.Objects:
        shape = getattr(item, "Shape", None)
        if shape is None or shape.isNull():
            continue
        edges.extend(shape.Edges)
    kinds = {}
    for edge in edges:
        kind = type(edge.Curve).__name__
        kinds[kind] = kinds.get(kind, 0) + 1
    box = FreeCAD.BoundBox()
    for edge in edges:
        box.add(edge.BoundBox)
    wires = [Part.Wire(group) for group in Part.sortEdges(edges)]
    closed = [wire for wire in wires if wire.isClosed()]
    areas = sorted((abs(Part.Face(wire).Area) for wire in closed), reverse=True)
    return {
        "edges": len(edges),
        "kinds": kinds,
        "length": sum(edge.Length for edge in edges),
        "bbox": [box.XMin, box.YMin, box.XMax, box.YMax] if edges else [],
        "wires": len(wires),
        "closed": len(closed),
        "area": areas[0] - sum(areas[1:]) if areas else 0.0,
    }


def read(path, name):
    document = FreeCAD.newDocument(name)
    if path.lower().endswith(".dxf"):
        # FreeCAD's own (C++) DXF reader; the older Python one needs a
        # library it downloads on first use.
        import Import

        Import.readDXF(path, name)
    else:
        import importSVG

        importSVG.insert(path, name)
    document.recompute()
    return measure(document)


def text(value):
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, list):
        return " ".join(repr(v) for v in value)
    if isinstance(value, dict):
        return ",".join("%s:%d" % (k, value[k]) for k in sorted(value))
    return " ".join(str(value).split())


folder = os.environ["CRAFT3D_FC_IN"]
lines = []
for index, file_name in enumerate(sorted(os.listdir(folder))):
    if not file_name.lower().endswith((".dxf", ".svg")):
        continue
    try:
        report = read(os.path.join(folder, file_name), "d%d" % index)
    except Exception:
        report = {"error": traceback.format_exc()}
    report["file"] = file_name
    lines.append("\t".join("%s=%s" % (key, text(report[key])) for key in sorted(report)))
with open(os.environ["CRAFT3D_FC_OUT"], "w") as out:
    out.write("\n".join(lines) + "\n")
