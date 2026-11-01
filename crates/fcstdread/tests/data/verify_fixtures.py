# Checks expected.json against FreeCAD, headless (FreeCADCmd): each
# committed document opened again, each listed object's shape where the
# document puts it measured, and compared with what make_fixtures.py
# recorded. Writes one line per object to CRAFT3D_FC_OUT: `ok <file>
# <object>` or `bad <file> <object> <what>`.
#
# Run by `cargo test -p craft3d-fcstdread --test freecad -- --ignored`
# with CRAFT3D_FREECAD set (`cargo xtask freecad`). FreeCAD runs as a
# separate program; nothing of it is linked.

import json
import os

import FreeCAD as App

HERE = os.path.dirname(os.path.abspath(__file__))
lines = []
try:
    expected = json.load(open(os.path.join(HERE, "expected.json")))
    for name, rows in sorted(expected.items()):
        if not name.endswith(".FCStd"):
            continue
        doc = App.openDocument(os.path.join(HERE, name))
        for row in rows:
            obj = doc.getObject(row["object"])
            if obj is None:
                lines.append("bad %s %s missing" % (name, row["object"]))
                continue
            shape = obj.Shape.copy()
            shape.Placement = obj.getGlobalPlacement()
            solids = shape.Solids if shape.Solids else [shape]
            s = solids[row["solid"]]
            bb = s.optimalBoundingBox(False, False)
            box = [bb.XMin, bb.YMin, bb.ZMin, bb.XMax, bb.YMax, bb.ZMax]
            what = []
            if abs(s.Volume - row["volume"]) > 1e-9 * max(1.0, abs(row["volume"])) and s.Solids:
                what.append("volume %r %r" % (s.Volume, row["volume"]))
            if abs(s.Area - row["area"]) > 1e-9 * max(1.0, row["area"]):
                what.append("area %r %r" % (s.Area, row["area"]))
            if len(s.Faces) != row["faces"]:
                what.append("faces %d %d" % (len(s.Faces), row["faces"]))
            if any(abs(a - b) > 1e-6 for a, b in zip(box, row["bbox"])):
                what.append("bbox %r %r" % (box, row["bbox"]))
            lines.append(("bad %s %s %s" % (name, row["object"], "; ".join(what))) if what else ("ok %s %s" % (name, row["object"])))
        App.closeDocument(doc.Name)
except Exception as e:
    lines.append("bad exception %r" % (e,))
with open(os.environ["CRAFT3D_FC_OUT"], "w") as f:
    f.write("\n".join(lines) + "\n")
