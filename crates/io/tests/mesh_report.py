# Run by tests/freecad.rs inside FreeCAD's command-line interpreter. Loads the
# mesh file named in CRAFT3D_FC_IN and writes what FreeCAD makes of it, one
# key=value per line, to CRAFT3D_FC_OUT.
import os

import Mesh

mesh = Mesh.Mesh(os.environ["CRAFT3D_FC_IN"])
box = mesh.BoundBox
lines = [
    "facets=%d" % mesh.CountFacets,
    "points=%d" % mesh.CountPoints,
    "area=%r" % mesh.Area,
    "volume=%r" % mesh.Volume,
    "solid=%d" % (1 if mesh.isSolid() else 0),
    "bbox=%r %r %r %r %r %r" % (box.XMin, box.YMin, box.ZMin, box.XMax, box.YMax, box.ZMax),
]
with open(os.environ["CRAFT3D_FC_OUT"], "w") as out:
    out.write("\n".join(lines) + "\n")
