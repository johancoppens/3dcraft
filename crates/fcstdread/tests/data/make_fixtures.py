# Makes the FreeCAD documents the tests of craft3d-fcstdread read, and
# what FreeCAD itself says of their objects (expected.json): volume, area
# and tight bounding box of each object's shape where the document puts
# it, and its colour.
#
# Colours live in GuiDocument.xml, which only FreeCAD's GUI writes, so
# this runs the GUI, headless, with a user home of its own:
#
#   FREECAD_USER_HOME=$(mktemp -d) QT_QPA_PLATFORM=offscreen \
#       CRAFT3D_FC_OUT=<this directory> FreeCAD.AppImage freecad make_fixtures.py
#
# The empty user home matters: with the developer's own, the GUI's start
# page makes thumbnails of his recent files (a large STEP file among
# them) with an external viewer, which ran away past 6 GB on 2026-11-01.
# Run it on a machine where that cannot hurt (the Strix), never beside a
# desktop session. The numbers are checked again headless by
# verify_fixtures.py (`cargo xtask freecad`).
#
# The files are this script's output, ours to commit (MIT, like the
# repository). FreeCAD runs as a separate program; nothing of it is linked.

import json
import math
import os

import FreeCAD as App
import FreeCADGui as Gui
import Part

OUT = os.environ["CRAFT3D_FC_OUT"]
App.ParamGet("User parameter:BaseApp/Preferences/Document").SetBool("CreateBackupFiles", False)
V = App.Vector
expected = {}


def rot(axis, degrees):
    return App.Rotation(V(*axis), degrees)


def hide(obj):
    obj.ViewObject.Visibility = False


def color(obj, rgb):
    obj.ViewObject.ShapeColor = rgb


def record(doc, objs, name):
    """What FreeCAD says of each object to be imported, in import order."""
    rows = []
    for obj in objs:
        shape = obj.Shape.copy()
        shape.Placement = obj.getGlobalPlacement()
        solids = shape.Solids if shape.Solids else [shape]
        for k, s in enumerate(solids):
            # The tight box (BoundBox may be loose on curved faces).
            bb = s.optimalBoundingBox(False, False)
            row = {
                "object": obj.Name,
                "label": obj.Label,
                "solid": k,
                "volume": s.Volume if s.Solids else 0.0,
                "area": s.Area,
                "faces": len(s.Faces),
                "bbox": [bb.XMin, bb.YMin, bb.ZMin, bb.XMax, bb.YMax, bb.ZMax],
            }
            try:
                mat = obj.ViewObject.ShapeAppearance[0]
                c = mat.DiffuseColor
                row["color"] = [int(round(c[0] * 255)), int(round(c[1] * 255)), int(round(c[2] * 255))]
            except Exception:
                pass
            rows.append(row)
    expected[name] = rows
    doc.saveAs(os.path.join(OUT, name))
    App.closeDocument(doc.Name)


def main():
    # 1. A box, placed and turned.
    doc = App.newDocument("box")
    box = doc.addObject("Part::Box", "Box")
    box.Length, box.Width, box.Height = 10, 20, 30
    box.Placement = App.Placement(V(5, 0, 0), rot((0, 0, 1), 30))
    box.Label = "Turned box"
    doc.recompute()
    record(doc, [box], "box.FCStd")

    # 2. A block with a hole through it: the cut is the final object, its
    # inputs hidden as the GUI hides them.
    doc = App.newDocument("cut")
    block = doc.addObject("Part::Box", "Block")
    block.Length, block.Width, block.Height = 40, 30, 10
    drill = doc.addObject("Part::Cylinder", "Drill")
    drill.Radius, drill.Height = 6, 30
    drill.Placement = App.Placement(V(20, 15, -10), App.Rotation())
    cut = doc.addObject("Part::Cut", "Cut")
    cut.Base, cut.Tool = block, drill
    cut.Label = "Plate with hole"
    doc.recompute()
    hide(block)
    hide(drill)
    color(cut, (0.0, 0.5, 1.0))
    record(doc, [cut], "cut.FCStd")

    # 3. A filleted and chamfered block (cylinders, tori, spheres at the
    # corners where three fillets meet).
    doc = App.newDocument("fillet")
    base = doc.addObject("Part::Box", "Base")
    base.Length, base.Width, base.Height = 30, 20, 10
    doc.recompute()
    fillet = doc.addObject("Part::Fillet", "Fillet")
    fillet.Base = base
    fillet.Edges = [(i, 2.0, 2.0) for i in range(1, 13)]
    doc.recompute()
    hide(base)
    fillet.Label = "Rounded block"
    record(doc, [fillet], "fillet.FCStd")

    # 4. A PartDesign body: a pad and a pocket, the body placed. Only the
    # body (its tip) is imported, not the features inside it.
    doc = App.newDocument("body")
    body = doc.addObject("PartDesign::Body", "Body")
    body.Placement = App.Placement(V(0, 0, 50), rot((1, 0, 0), 90))
    sketch = body.newObject("Sketcher::SketchObject", "Sketch")
    sketch.AttachmentSupport = (body.Origin.OriginFeatures[3], [""])  # XY plane
    sketch.MapMode = "FlatFace"
    import Sketcher

    pts = [V(0, 0, 0), V(40, 0, 0), V(40, 25, 0), V(0, 25, 0)]
    for i in range(4):
        sketch.addGeometry(Part.LineSegment(pts[i], pts[(i + 1) % 4]), False)
    pad = body.newObject("PartDesign::Pad", "Pad")
    pad.Profile = sketch
    pad.Length = 12
    doc.recompute()
    sketch2 = body.newObject("Sketcher::SketchObject", "Sketch001")
    sketch2.AttachmentSupport = (body.Origin.OriginFeatures[3], [""])
    sketch2.MapMode = "FlatFace"
    sketch2.AttachmentOffset = App.Placement(V(0, 0, 12), App.Rotation())
    sketch2.addGeometry(Part.Circle(V(20, 12.5, 0), V(0, 0, 1), 5), False)
    pocket = body.newObject("PartDesign::Pocket", "Pocket")
    pocket.Profile = sketch2
    pocket.Length = 8
    doc.recompute()
    body.Label = "Padded body"
    record(doc, [body], "body.FCStd")

    # 5. Splines: a profile bounded by a B-spline, revolved about z; and a
    # closed periodic B-spline extruded (a linear-extrusion surface).
    doc = App.newDocument("spline")
    spline = Part.BSplineCurve()
    spline.interpolate([V(0, 0, 0), V(8, 0, 4), V(5, 0, 10), V(9, 0, 16), V(0, 0, 20)])
    profile = Part.Face(Part.Wire([spline.toShape(), Part.LineSegment(V(0, 0, 20), V(0, 0, 0)).toShape()]))
    holder = doc.addObject("Part::Feature", "Profile")
    holder.Shape = profile
    rev = doc.addObject("Part::Revolution", "Revolve")
    rev.Source = holder
    rev.Axis = V(0, 0, 1)
    rev.Base = V(0, 0, 0)
    rev.Angle = 360
    rev.Solid = True
    rev.Label = "Vase"
    # The vase's volume by Pappus, integrated along the profile (Gauss
    # with 64 points per span between the interpolated points): an
    # oracle independent of both kernels' solid integrators.
    def pappus():
        import numpy

        xs, ws = numpy.polynomial.legendre.leggauss(64)
        knots = spline.getKnots()
        total = 0.0
        for a, b in zip(knots[:-1], knots[1:]):
            for x, w in zip(xs, ws):
                t = (a + b) / 2 + (b - a) / 2 * x
                p = spline.value(t)
                d = spline.getDN(t, 1)
                total += w * (b - a) / 2 * math.pi * p.x * p.x * d.z
        return abs(total)

    expected["vase_pappus"] = pappus()
    closed = Part.BSplineCurve()
    closed.interpolate([V(30, 0, 0), V(40, 5, 0), V(38, 12, 0), V(30, 10, 0), V(26, 4, 0)], PeriodicFlag=True)
    section = doc.addObject("Part::Feature", "Section")
    section.Shape = Part.Face(Part.Wire([closed.toShape()]))
    ext = doc.addObject("Part::Extrusion", "Extrude")
    ext.Base = section
    ext.Dir = V(0, 0, 7)
    ext.LengthFwd = 7
    ext.Solid = True
    ext.Label = "Blob"
    doc.recompute()
    hide(holder)
    hide(section)
    record(doc, [rev, ext], "spline.FCStd")

    # 6. Several bodies with colours and placements: two inside a placed
    # App::Part, primitives at the top, one hidden (not imported).
    doc = App.newDocument("multi")
    asm = doc.addObject("App::Part", "Assembly")
    asm.Placement = App.Placement(V(100, 0, 0), rot((0, 0, 1), 90))
    a = doc.addObject("Part::Box", "Red")
    a.Length, a.Width, a.Height = 5, 6, 7
    a.Placement = App.Placement(V(0, 10, 0), rot((1, 0, 0), 45))
    asm.addObject(a)
    b = doc.addObject("Part::Cylinder", "Green")
    b.Radius, b.Height = 3, 9
    b.Placement = App.Placement(V(20, 0, 0), App.Rotation())
    asm.addObject(b)
    sph = doc.addObject("Part::Sphere", "Ball")
    sph.Radius = 4
    sph.Placement = App.Placement(V(-20, 0, 0), App.Rotation())
    cone = doc.addObject("Part::Cone", "Cone")
    cone.Radius1, cone.Radius2, cone.Height = 5, 2, 8
    cone.Placement = App.Placement(V(0, -20, 0), rot((0, 1, 0), 30))
    tor = doc.addObject("Part::Torus", "Ring")
    tor.Radius1, tor.Radius2 = 10, 2
    tor.Placement = App.Placement(V(0, 0, 30), App.Rotation())
    ghost = doc.addObject("Part::Box", "Ghost")
    doc.recompute()
    hide(ghost)
    color(a, (1.0, 0.0, 0.0))
    color(b, (0.0, 1.0, 0.0))
    color(sph, (1.0, 1.0, 0.0))
    a.Label = "Red block"
    record(doc, [a, b, sph, cone, tor], "multi.FCStd")

    # Plain BRep files for craft3d-brep: a shape moved twice (a composite
    # location), and a periodic B-spline face.
    moved = Part.makeBox(1, 2, 3)
    moved.Placement = App.Placement(V(10, 0, 0), rot((0, 0, 1), 90))
    moved.transformShape(App.Placement(V(0, 5, 0), rot((1, 0, 0), 90)).toMatrix())
    moved = Part.makeCompound([moved])
    moved.Placement = App.Placement(V(1, 2, 3), rot((0, 1, 0), 60))
    moved.exportBrep(os.path.join(OUT, "moved.brp"))
    bb = moved.BoundBox
    expected["moved.brp"] = [{"volume": moved.Volume, "bbox": [bb.XMin, bb.YMin, bb.ZMin, bb.XMax, bb.YMax, bb.ZMax]}]
    # Points FreeCAD evaluates on the periodic curve, for the reading of
    # periodic knots.
    expected["periodic"] = {
        "points": [[t, list(closed.value(t))] for t in [closed.FirstParameter + (closed.LastParameter - closed.FirstParameter) * k / 7.0 for k in range(8)]]
    }
    Part.Face(Part.Wire([closed.toShape()])).exportBrep(os.path.join(OUT, "periodic.brp"))

    with open(os.path.join(OUT, "expected.json"), "w") as f:
        json.dump(expected, f, indent=1, sort_keys=True)


try:
    main()
except Exception as e:
    with open(os.path.join(OUT, "error.txt"), "w") as f:
        f.write(repr(e))
finally:
    os._exit(0)
