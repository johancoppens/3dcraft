# Run by tests/freecad.rs inside FreeCAD's command-line interpreter. Loads the
# mesh file named in CRAFT3D_FC_IN and writes what FreeCAD makes of it, one
# key=value per line, to CRAFT3D_FC_OUT.
#
# With CRAFT3D_FC_LIST instead (the STEP harness, for the meshes of
# documents built by commands): a file with one mesh file per line; what
# FreeCAD makes of each is appended to CRAFT3D_FC_OUT as one JSON object
# per line, flushed after each so that a file that crashes FreeCAD can be
# told from the ones before it.
import json
import os

import Mesh


def report(path):
    mesh = Mesh.Mesh(path)
    box = mesh.BoundBox
    return {
        "facets": mesh.CountFacets,
        "points": mesh.CountPoints,
        "area": mesh.Area,
        "volume": mesh.Volume,
        "solid": bool(mesh.isSolid()),
        "bbox": [box.XMin, box.YMin, box.ZMin, box.XMax, box.YMax, box.ZMax],
    }


if "CRAFT3D_FC_LIST" in os.environ:
    with open(os.environ["CRAFT3D_FC_LIST"]) as listing:
        paths = [line.strip() for line in listing if line.strip()]
    with open(os.environ["CRAFT3D_FC_OUT"], "a") as out:
        for path in paths:
            try:
                record = report(path)
            except Exception as error:
                record = {"error": str(error)}
            record["file"] = os.path.basename(path)
            out.write(json.dumps(record) + "\n")
            out.flush()
else:
    found = report(os.environ["CRAFT3D_FC_IN"])
    lines = [
        "facets=%d" % found["facets"],
        "points=%d" % found["points"],
        "area=%r" % found["area"],
        "volume=%r" % found["volume"],
        "solid=%d" % (1 if found["solid"] else 0),
        "bbox=%r %r %r %r %r %r" % tuple(found["bbox"]),
    ]
    with open(os.environ["CRAFT3D_FC_OUT"], "w") as out:
        out.write("\n".join(lines) + "\n")
