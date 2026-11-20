# Run by tests/freecad.rs inside FreeCAD's command-line interpreter. Imports
# the glTF file named in CRAFT3D_FC_IN with FreeCAD's own glTF reader
# (Import.insert, Open CASCADE's), which turns glTF's metres and +Y up into
# FreeCAD's millimetres and +Z up, and writes what it made of it to
# CRAFT3D_FC_OUT, one key=value per line: the objects' labels, how many faces
# (one per triangle), their area, the volume they enclose (by the divergence
# theorem over the faces, each with its outward normal, so that it does not
# matter how FreeCAD groups them into shells) and their bounding box.
import json
import os

import FreeCAD
import Import

doc = FreeCAD.newDocument("gltf")
Import.insert(os.environ["CRAFT3D_FC_IN"], doc.Name)
labels = []
faces = []
for obj in doc.Objects:
    shape = getattr(obj, "Shape", None)
    if shape is None or shape.isNull() or not shape.Faces:
        continue
    labels.append(obj.Label)
    faces.extend(shape.Faces)

area = 0.0
volume = 0.0
box = None
for face in faces:
    u0, u1, v0, v1 = face.ParameterRange
    normal = face.normalAt(0.5 * (u0 + u1), 0.5 * (v0 + v1))
    centre = face.CenterOfMass
    area += face.Area
    volume += face.Area * centre.dot(normal) / 3.0
    if box is None:
        box = FreeCAD.BoundBox(face.BoundBox)
    else:
        box.add(face.BoundBox)

lines = [
    "labels=%s" % json.dumps(labels),
    "faces=%d" % len(faces),
    "area=%r" % area,
    "volume=%r" % volume,
    "bbox=%r %r %r %r %r %r"
    % ((box.XMin, box.YMin, box.ZMin, box.XMax, box.YMax, box.ZMax) if box else (0, 0, 0, 0, 0, 0)),
]
with open(os.environ["CRAFT3D_FC_OUT"], "w") as out:
    out.write("\n".join(lines) + "\n")
