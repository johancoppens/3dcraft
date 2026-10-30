//! The command registry: every action 3DCraft can take.
//!
//! To add a command: write its `fn(&mut Session, &Value) -> Result<Value,
//! CommandError>`, add a `CommandSpec` to `SPECS`, and add tests (behaviour,
//! undo, bad parameters). `tests/panic_hunt.rs` fuzzes every entry of
//! `SPECS` automatically.

use crate::mass::mass_of;
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session, base64};
use craft3d_doc::{
    Block, Boolean, BooleanOp, Cone, Cylinder, DocError, Document, Expr, Extrude, ExtrudeEnd,
    Feature, FeatureId, FeatureKind, LengthUnit, Orientation, PatternLayout, PersistentName,
    PlaneBy, Sphere, Tolerance, Torus, Units,
};
use craft3d_io::MeshFormat;
use craft3d_math::Vec3;
use craft3d_regen::{Regen, Status};
use craft3d_render::{Camera, RenderParams};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::BodyKind;
use serde_json::{Value, json};

pub(crate) type Run = Result<Value, CommandError>;

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, CommandError> {
    Err(CommandError::new(message))
}

pub(crate) fn doc_error(command: &str) -> impl Fn(DocError) -> CommandError + '_ {
    move |e| CommandError::new(format!("{command}: {e}"))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $summary:literal, doc: $needs:literal, $run:path) => {
        CommandSpec {
            id: $id,
            label: $label,
            params: $params,
            summary: $summary,
            needs_document: $needs,
            names: &[],
            run: $run,
        }
    };
    ($id:literal, $label:literal, $params:literal, $summary:literal, doc: $needs:literal, names: [$($name:literal),* $(,)?], $run:path) => {
        CommandSpec {
            id: $id,
            label: $label,
            params: $params,
            summary: $summary,
            needs_document: $needs,
            names: &[$($name),*],
            run: $run,
        }
    };
}
pub(crate) use spec;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("file.new", "New Document", r#"{"units": "um|mm|cm|m|in|ft" = "mm", "tolerance": number = 1e-6, "loosest": number = 1000 × tolerance}"#,
        "Creates an empty document and makes it the active one. The tolerance is in the document's length unit, from 1e-12 to 1e3. loosest is the cap on the tolerance any vertex or edge may have of its own (geometry that is not exact to the document's tolerance, as imported files bring it), at least the tolerance: an operation on a body with an entity looser than it refuses.", doc: false, file_new),
    spec!("file.open", "Open", r#"{"path": string}"#,
        "Opens a .3dcraft file from the read root and makes it the active document.", doc: false, file_open),
    spec!("file.save", "Save", r#"{"path": string = the document's own path, "overwrite": boolean = false, "thumbnail": boolean = true for a Jig}"#,
        "Saves the active document as a .3dcraft file (atomically) under the write root. A recovered document (recovery.open) is not saved over its original file when that file changed since the recovery was taken, unless overwrite is true. With thumbnail (the default for a Jig, a document with public parameters) the file also holds thumb.png, a 400 x 300 picture of the bodies from the iso view drawn by the CPU renderer (the same document gives the same picture on one platform and build; drawn once per change), for a gallery; the result says thumbnail: true when it was written, and thumbnailError why not (no bodies yet): the document is saved all the same.", doc: true, file_save),
    spec!("file.close", "Close", r#"{"document": integer = the active one, "discard": boolean = false}"#,
        "Closes the active document, or the open document of that number (file.list). Refuses unsaved changes unless discard is true. The active document stays active when another closes; when it closes, the last open one becomes active. Returns how many are open and which is active.", doc: true, file_close),
    spec!("file.export", "Export", r#"{"path": string, "format": "step|stl|stl-ascii|3mf|obj|dxf|svg" = from the extension, "tolerance": number = by quality, "quality": "coarse|normal|fine" = "normal", "angle": degrees = by quality, "bodies": [id or name of the feature that made the body] = every body, "sketch": id or name, "fold": id or name of a fold, "sheetMetal": id or name of a sheet-metal part or a list of them (a sheet), "face": face name, "feature": id or name = the body that has the face, "faces": [face name or {"face", "feature", "xAxis", "layer"}], "set": name of a named set, "trace": name of a trace (view.trace), "xAxis": [x, y, z] = level, y up the face, "origin": "model|corner" = model, "gap": expr = 5 mm, "width": expr = no limit, "construction": boolean = false, "view": "iso|front|back|left|right|top|bottom|current" = iso, "projection": "orthographic|perspective" = orthographic, "hidden": "omit|dashed" = omit, "stroke": number = 0.01 (SVG line width in mm)}"#,
        "Writes a file under the write root. step (.step, .stp) is the exact geometry of every body, with the document's units, its tolerance as the file's uncertainty and the bodies' names. stl, 3mf and obj are meshes of every body: quality coarse, normal (the default) or fine sets how closely they follow the model, a chordal tolerance of 2, 1 or 0.25 thousandths of the side of a square with the bodies' area and an angle criterion of 20, 10 or 5 degrees (no chord turns an edge, no triangle side a surface's normal, by more, so small holes and thin bent tubes stay round); tolerance (in document units) and angle (degrees, 0 for none, else 1 to 90) set either one instead, and a tolerance given without quality or angle has no angle criterion, as before 2026-10-23. The result says tolerance, angle and quality, and closed counts the meshes that are watertight. A mesh of more than 20000000 triangles, all bodies together, is refused (choose a coarser quality). 3mf carries the colours of bodies and faces; obj carries them in a material library written beside it (name.obj, name.mtl; mtl in the result, null when nothing has a colour; with colours the name must have no white space or #, and must not end in .mtl). bodies writes only the bodies it names (to step, stl, 3mf and obj), each by the id or name of the feature that made it, as document.inspect and select.get name them, with their names and colours, in history order; a name that is not a body now (unknown, failed, suppressed, or taken by a later feature) is an error that lists the bodies there are. dxf and svg are a flat outline for laser cutting and drawings, in the document's unit at true size: of a sketch (sketch: its lines, arcs and circles, the instances of its patterns included, joined into chains, in the sketch's axes; construction geometry only with construction, on a layer of its own), of a trace (trace: the name of a path view.trace made, as one polyline in its plane, x along the world axis least along the plane's normal) or of a flat face of a body (face: a name from document.inspect, faceNames, or the name the face had in an earlier body, followed to its newest form as features follow names; its outer boundary and its holes, seen from outside the body, so never mirrored: outer boundaries run counter-clockwise; feature says which body when several have a face of that name). A face is drawn with y up the face (the model's +z projected onto its plane) and, on a face that lies flat, x along the model's +x; xAxis gives x another direction (projected onto the plane); the origin is the model's origin dropped onto the plane, or with origin corner the lower left corner of the outline. faces puts several flat faces, of one body or of several, on one sheet: each in its own frame and on a layer of its own named after it (or its layer), set left to right in the order given by their bounding boxes, gap apart, in rows of at most width (a new row above the last). set does the same for every flat face a named set (set.define) holds now, in document order, the faces of one body on one layer named after it (faces given the same layer in faces share it too): one sheet of all the parts to cut, which follows the design as the set does; a body hidden in the view is drawn too (visibility is view data). The result lists under notFlat the faces of the set that are not flat and under others how many items of each other kind it holds (body, edge, ...); neither is drawn. Every curve stays what it is: lines, circular arcs and circles, ellipses, splines; nothing becomes a polyline. dxf is AutoCAD 2000 ASCII with $INSUNITS; svg has width and height in mm, cm or in and y pointing up as in the model. tolerance for svg is how far the cubic Béziers written for a spline of a degree above 3 or a rational one may lie from it (default: the document's tolerance); the result says how many there were. Without sketch, face or faces a dxf or svg file is a drawing of the model as seen (a projected view): every edge of the bodies that are not hidden, as the curve it is in the drawing (a line, a circle or arc where its plane is square to the view, else an ellipse or elliptic arc, a spline as a spline; one seen edge on as a line), and the silhouettes of curved faces as straight pieces within tolerance (the chordal tolerance here, automatic by default), on the layer visible (silhouettes on silhouette); what lies behind a face is left out, or with hidden dashed drawn dashed on the layer hidden. view is a named view, or current: the camera of the window showing the document (iso without one; the result says so in note); the result gives the camera used; projection orthographic is to scale (one drawing unit is one model unit, in the document's unit), perspective is true size only at the depth of the camera's target. The section of the window is not applied and a scrubbed view is not followed: the bodies of the document as it is, whole. The result counts the visible, silhouette and hidden pieces (lines, arcs, elliptic arcs and splines; a whole circle or ellipse one). With fold, a .dxf or .svg file is the crease pattern of a fold, flat, in its sketch's axes: the sheet's outline on the layer boundary (black, continuous), mountains on mountain (red, dash-dot), valleys on valley (blue, dashed) and cuts on cut (black, continuous), each crease line whole; the result counts the creases of each kind. With sheetMetal, a .dxf or .svg file is the flat pattern of a sheet-metal part (feature.sheetMetal), in its sketch's axes: the outline on the layer outline (black, continuous: the base, the bends' strips at their bend allowance and the flanges, reliefs and holes cut), each bend's line (the middle of its strip) on bendUp (blue, dashed) or bendDown (red, dash-dot); the result counts the bends of each way and gives the flat area. With sheetMetal a list of parts, their flat patterns go side by side on one sheet as a set's faces do (gap apart, default 5 mm, in rows of at most width), each in its sketch's axes moved, the outlines sharing the layer outline and the bend lines bendUp and bendDown; the result lists each part with where it went (at, bbox).", doc: true, names: ["face", "faces"], file_export),
    spec!("edit.undo", "Undo", "{}", "Undoes the last change to the document.", doc: true, edit_undo),
    spec!("edit.redo", "Redo", "{}", "Redoes the change last undone.", doc: true, edit_redo),
    spec!("variable.set", "Set Variable", r#"{"name": string, "expr": expression, "wait": boolean = false}"#,
        "Defines a variable or replaces its expression, e.g. {\"name\": \"width\", \"expr\": \"40 mm\"}. An expression has numbers with units (um mm cm m in ft, deg rad), variables, pi, + - * / ^, the comparisons < <= > >= == != (1 or 0; == within a relative 1e-12), && || ! (0 is false, anything else true) and the functions sin, cos, tan, asin, acos, atan, atan2, sqrt, abs, min, max, floor, ceil, round, trunc, mod, exp, ln, log10, pow, sinh, cosh, tanh, if, clamp and choose. Conditions: if(length > 200 mm, 4, 2) takes only the branch chosen, so the other may fail (if(n > 0, total / n, 0 mm)); both branches have one dimension. clamp(wall, 1.2 mm, 5 mm) keeps a value between two bounds; choose(i, a, b, ...) is the i-th value, counted from 1. floor, ceil, round and trunc take a plain number (floor(span / fingerW)) or a value and a positive step of its dimension (round(width, 0.5 mm)); a length alone is refused, as its whole number would depend on the unit. round takes ties to the even whole number (2.5 to 2, 3.5 to 4); mod(a, b) is a - b * floor(a / b), with the sign of b, and b = 0 is an error. A name is ASCII letters, digits and _, not starting with a digit, and not a word the expressions use themselves (pi, a unit such as m or in, a function such as mod or min): such a name is refused with what it clashes with and a name to use instead (module for a gear's m). Returns the variable, failed (the features that fail now and did not before) and, when the document has assertions, their summary (assertions): it waits up to two seconds for those the change touched and counts the rest as pending, with whenPending saying how to have them; wait: true waits until every assertion is measured, for 60 s at most (what is not measured by then stays pending, with whenPending; where the host works a piece at a time between its frames, the web page, they are all measured in the command, without that ceiling).", doc: true, variable_set),
    spec!("variable.delete", "Delete Variable", r#"{"name": string, "force": boolean = false}"#,
        "Deletes a variable. A variable still in use is refused, with every user named (variables, features and sketches with the parameter, assertions, named sets); with force it is deleted anyway, and what used it fails until the variable is defined again (edit.undo brings it back); the result lists what fails now under failed. Where the host regenerates a piece at a time between its frames (the web page), the regeneration comes after the command: failed is left out then, and deferred: true says so.", doc: true, variable_delete),
    spec!("variable.rename", "Rename Variable", r#"{"name": string, "to": string}"#,
        "Renames a variable and every reference to it in variables and features.", doc: true, variable_rename),
    spec!("feature.block", "Block", r#"{"size": [expr, expr, expr], "position": [expr, expr, expr] = [0, 0, 0], "axis": [expr, expr, expr] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic, "when": expr = always on}"#,
        "Adds a rectangular block with one corner at position. axis is the direction of its local z (three expressions, plain numbers or lengths as a position takes them, so it can follow parameters: [\"cos(a)\", \"sin(a)\", 0] or [\"dx\", \"dy\", 0]; only where it points counts; one of no length fails the feature, and so does one that mixes lengths with plain numbers other than 0), angle a turn about that axis. Its faces are F1:face(0) to F1:face(5) (F1 its id): 0 bottom, 1 top, 2 front, 3 back, 4 left, 5 right in its own axes (low z, high z, low y, high y, low x, high x).", doc: true, feature_block),
    spec!("feature.cylinder", "Cylinder", r#"{"radius": expr, "height": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [expr, expr, expr] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic, "when": expr = always on}"#,
        "Adds a cylinder along axis (expressions, as for feature.block) with its base centred at position. Its faces are F1:face(0) the wall, F1:face(1) the base and F1:face(2) the top (F1 its id).", doc: true, feature_cylinder),
    spec!("feature.sphere", "Sphere", r#"{"radius": expr, "position": [expr, expr, expr] = [0, 0, 0], "name": string = automatic, "when": expr = always on}"#,
        "Adds a sphere centred at position.", doc: true, feature_sphere),
    spec!("feature.cone", "Cone", r#"{"radius": expr, "height": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [expr, expr, expr] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic, "when": expr = always on}"#,
        "Adds a cone along axis (expressions, as for feature.block): its base circle is centred at position, its apex is height further along the axis. Its faces are F1:face(0) the side and F1:face(1) the base (F1 its id).", doc: true, feature_cone),
    spec!("feature.torus", "Torus", r#"{"major": expr, "minor": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [expr, expr, expr] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic, "when": expr = always on}"#,
        "Adds a ring torus about axis (expressions, as for feature.block), centred at position: major is the radius of the ring, minor of the tube.", doc: true, feature_torus),
    spec!("feature.extrude", "Extrude", r#"{"sketch": id or name, "distance": expr, "throughAll": boolean = false, "upTo": face name, "symmetric": boolean = false, "reversed": boolean = false, "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic, "when": expr = always on}"#,
        "Adds a solid by sweeping the closed profiles of a sketch along its normal: loops of lines and arcs, and circles; a loop inside another is a hole. Give one end: distance (a negative one goes the other way; symmetric puts half on each side of the sketch plane), throughAll (past every body the features before this one have built that lies in the way; symmetric goes through on both sides) or upTo (a face name from document.inspect: the solid ends on that face's surface, flat or curved, and follows it when the face moves). reversed goes against the sketch's normal. With cut, join or intersect (a feature with a body) the solid is combined with that body at once: two features are added in one undo step, the extrusion and a boolean, and the result is the boolean's (the one to build on; name applies to it) with the extrusion under sweep. A hole through a plate: sketch a circle on its top, extrude with throughAll, reversed and cut.", doc: true, names: ["upTo"], feature_extrude),
    spec!("feature.edit", "Edit Feature", r#"{"feature": id or name, "sketch": id or name, "path": id or name, "sections": [sketch, or {sketch, start}; ...], "ruled": boolean, "closed": boolean, "source": id or name, "target": id or name, "tool": id or name, "op": "union|subtract|intersect", "size": [expr; 3] or a thread's "M10" or expr or null, "position": [expr; 3], "axis": [expr; 3] or datum axis or "x|y" or line id, "angle": expr, "radius": expr, "height": expr, "major": expr, "minor": expr, "distance": expr, "throughAll": boolean, "upTo": face name, "symmetric": boolean, "reversed": boolean, "translate": [expr; 3], "pivot": [expr; 3], "point": [expr; 3], "normal": [expr; 3], "plane": datum plane id or name, "step": [expr; 3] (a circular pattern: expr along its axis, or null for a plain turn), "count": expr, "keep": boolean, "keepTool": boolean, "edges": [[face name, face name], ...], "second": expr, "chain": boolean, "faces": [face name, ...], "thickness": expr, "side": "outside|inside|both" (a plate's: "in|out|mid"), "neutral": plane, "open": [face name, ...], "pitch": expr (or null for a thread: the coarse pitch of its size), "turns": expr, "leftHanded": boolean, "length": expr, "cosmetic": boolean, "modeled": boolean, "a": sketch or {edges}, "b": sketch or {edges}, "at": [expr; 3], "joints": [joint, ...] (a plate's, as feature.plate takes them), "u": [curve, ...], "v": [curve, ...] (a network surface's; its "edges" are its conditions, as feature.surface.network takes them)}"#,
        "Changes parameters of a feature; give only the ones to change. Which ones a feature has depends on its kind (document.inspect shows them under params). The end of an extrusion is changed by giving the new one: distance, throughAll or upTo. A mirror is pointed at a datum plane with plane, and back at a plane of its own with normal or point; a circular pattern at a datum axis with axis (its name or id), and back at a line of its own with axis as a direction or point; a circular pattern becomes a screw with step (one length along the axis) and a plain turn again with step null. What a feature is made from is changed the same way, which is how a feature that lost it is repaired: sketch for an extrusion, a revolution or a sweep (its profile), path for a sweep, axis for a revolution (x, y, a line of the sketch or a datum axis), source for a moved, mirrored or patterned body, target, tool and op for a boolean, edges for a chamfer or a fillet (which is also how its body is chosen: the body of the feature the faces are of); the feature keeps its id and its place in the history, and the names of its faces are those of a feature built that way from the start. A chamfer's size: distance alone changes the first distance and keeps the form; with second it becomes two distances, with angle a distance and an angle (give second the same as distance for equal ones). A fillet's radius is changed with radius. An offset's faces and distance are changed with faces and distance (the faces also choose its body, as the edges of a chamfer do), a moved face's with faces, translate, pivot, axis and angle, a thickened one's with faces, thickness and side, a plate's with faces, thickness, side and joints (the whole list), a draft's with faces, neutral, angle and reversed, a deletion's faces with faces; a shell's with source, thickness and open. A datum plane's distance or angle and the position of a datum point at coordinates are changed here; what a datum is placed by, with datum.plane, datum.axis or datum.point and replace.", doc: true, names: ["upTo", "edges", "faces", "neutral", "open", "plane", "joints"], feature_edit),
    spec!("feature.rename", "Rename Feature", r#"{"feature": id or name, "name": string}"#,
        "Renames a feature.", doc: true, feature_rename),
    spec!("feature.suppress", "Suppress Feature", r#"{"feature": id or name, "suppressed": boolean = true}"#,
        "Switches a feature off (or back on) without deleting it. The features after it go on as if it were not there: one that changes a body (a boolean, fillet, chamfer, face operation, shell, thread, pattern, or a transform or mirror without keep) passes that body on unchanged, so what is made from it is made from the body as it was before (document.inspect says passesOn); one that makes a body of its own passes nothing on, a boolean with that body as its tool passes its target on, a union with it as its target passes its tool on, and a subtraction or intersection with it as its target passes nothing on and still takes its tool (passesNothing says why there is no body). A fillet, chamfer or face operation leaves out the faces and edges that are not there (of a body that is not made, or that only the suppressed feature makes) and, with none left, passes on the body it would have changed. The appearance, material and face colours given to the suppressed feature apply to the body it passes on only where that body has none of its own (the source's own wins). A sketch, datum or extrusion up to a face only the suppressed feature makes fails naming it. feature.when switches a feature by a condition instead. The result lists the features that fail under failed.", doc: true, feature_suppress),
    spec!("feature.move", "Reorder Feature", r#"{"feature": id or name, "position": integer}"#,
        "Moves a feature to a position in the history: its index in the features of document.inspect, sketches and datums included; 0 is first. A feature moved before what it is made from fails and says so.", doc: true, feature_move),
    spec!("feature.delete", "Delete Feature", r#"{"feature": id or name}"#,
        "Deletes a feature (edit.undo brings it back). A feature that named it (as its target, tool, source or sketch) fails. The result lists the features that fail under failed. Where the host regenerates a piece at a time between its frames (the web page), the regeneration comes after the command: failed is left out then, and deferred: true says so.", doc: true, feature_delete),
    spec!("history.rollback", "Roll Back", r#"{"feature": id or name or null}"#,
        "Regenerates only up to and including this feature; null removes the marker. While the marker is set, a feature added by any command goes in right after it, and the marker moves onto the new feature. A feature added there that takes a body (a fillet, a chamfer, a move, a boolean's target, a union's tool) takes it before the features after the marker that named that body (a boolean's target or tool, the source of a transform, mirror, pattern, shell or split): those are pointed at the new feature, so they go on with the body it changed, and the new feature's result lists them under repointed ([{feature, param}]). One that would then name it twice (a boolean of the two bodies an inserted union joined), and one that used the tool of an inserted subtraction or intersection, are left as they are and fail with \"already went into\", saying which feature.edit points them on. A fillet, a chamfer or a face operation names faces, not a body, and follows them into the new feature's body by itself. What was re-pointed is kept in the document (saved with it): feature.delete of the inserted feature gives every such parameter that still names it the feature it named before, as if it had never been inserted (restored: [{feature, param, names}] in its result), and feature.edit of it gives back those re-pointed from a body it no longer takes (restored in its result) and re-points nothing else: a later feature that names a body it takes now was pointed there by the user. A parameter given back only while the feature it named before still comes before it in the history: one moved after it is left naming the inserted feature and listed under notRestored with why. A parameter a command points elsewhere itself (feature.edit of the later feature) is the user's from then on: deleting or editing the inserted feature leaves it as it is, also when it was pointed back. document.inspect lists what is kept under inserted. The result lists the features that fail now and did not before under failed. Where the host regenerates a piece at a time between its frames (the web page), the regeneration comes after the command: failed is left out then, and deferred: true says so.", doc: true, history_rollback),
    spec!("document.inspect", "Inspect", r#"{"measures": boolean = true, "named": boolean = false}"#,
        "Returns the whole document state (with named: true each feature's parameters also under named, in the commands' words: keys as the commands spell them, keepTool, and features by name, where they differ from params): units, variables with values, features with status (a failed one with the reason under error, which starts with the parameter and names the feature and the name it lost, and, when the reason is that something it is made from failed, under causedBy the feature at the start of that chain: the one to fix; a datum with where it lies, under datum), bodies with counts (shells: one per lump and one per cavity; a body a cut fell apart in, or one with a void, has several), mass properties (volume, area, centroid; with a material: mass and inertia) and bbox, appearance and material, and faceNames (and, per face, faceAliases: the other names a face answers to when a boolean or a pattern merged faces of several operands into it), faceAppearances (the colours face.appearance gave single faces) and lostFaces (those whose face an edit took away, which colour nothing), inserted (when there are any: each feature inserted at the rollback marker that took a body, with repointed: [{feature, param, was}], the parameters of later features it pointed at itself and the feature each named before; a parameter pointed elsewhere by a command since is dropped from it, as the user's own choice), assertions (when there are any: each with status passed, failed, notProven, error or pending (still being measured after two seconds), what was measured and what was expected, as assert.list), undo history. With measures: false the bodies are listed without what takes measuring them (volume, area, centroid, mass, inertia, bbox and bboxTolerance; a body with a spline face can take seconds the first time), the rest as always: the quick way to read the features, their status and the face names of a large model.", doc: true, document_inspect),
    spec!("document.check", "Check", r#"{"deep": boolean = false, "feature": id or name = every feature}"#,
        "Checks every body for validity and lists every problem: failed variables, failed features (with causedBy as in document.inspect), defects of bodies, each body held to the document's cap on any entity's own tolerance, broken assertions, and a sketch line the solver shrank to a point under two directions or more (horizontal and vertical, or a direction and a perpendicular or an angle: a contradiction it reports as holding; with line and constraints). ok is true when there is none. warnings (never part of ok), each with kind: assertion (one not proven either way), collapsedLine (a line with no length under one direction), lostFace (a face colour whose face is gone: it colours nothing, as lostFaces in document.inspect says, with feature and face), underConstrained (one entry for every sketch with degrees of freedom left, listed under sketches with feature, name and dof), plainAngle (a parameter read as an angle, in a document whose angle unit is degrees, whose value is a plain number with a pi in it that has no unit and is not inside sin, cos or tan, itself or in a variable it uses: `2 * pi / n` is a few degrees there; with feature, name, parameter and expr; the value is kept). loose lists the bodies with entities that have a tolerance of their own (edges, vertices, the largest), when there are. With feature only that feature is checked (whether it failed, its own body, and deep its boolean), its status is given under status (ok, failed, suppressed, rolledBack), and a feature that is suppressed or after the rollback marker is a problem (nothing is built to check): what an edit at the end of a long history did, without checking the rest again. deep also looks for bodies that pass through themselves (faces crossing, shells overlapping or nested the wrong way), on a fine mesh, and that the shells of a body of several are the outsides of lumps and their cavities, each cavity in the material of one lump, from the exact geometry: slower, and what it reports is certain (deeper than the mesh could be off); doubts are counted under marginal; and for two shells of a body that touch at a vertex or the middle of an edge (zero thickness). deep also holds the body of every boolean feature to what every exact boolean of its target and tool keeps (volumes within bounds, the target unchanged out of the tool's reach, seeded points; counted under booleansChecked), and meshes every body with loose entities at a thousandth of its size, every point of a face's mesh on its surface or, on its boundary, within the tolerance of the edge or vertex it lies on (counted under looseMeshesChecked).", doc: true, document_check),
    spec!("document.measure", "Measure", r#"{"feature": id or name = all bodies, "bound": boolean = false}"#,
        "Returns volume, area and centroid, per body and in total, in document units. With bound also volumeBound and areaBound, per body and summed: how far the volume and the area can be off by the integrator's own account (every sum refined until two in a row agree; an estimate, not a guarantee). A body with a material (body.material) also has its mass (kg) and its inertia tensor about its centroid (kg times the length unit squared, along the model's axes), and its appearance (body.appearance); the total has the mass when every body has one.", doc: true, document_measure),
    spec!("view.render", "Render", r#"{"path": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the whole model, "mode": "shaded|ids" = "shaded", "section": {plane, feature, point, normal, offset, flip, cap} = none, "tolerance": number = the picture's, "background": "light|dark" = "light", "look": {mode: "plain|studio" = "studio", environment: "studio|outdoor|workshop" = "studio", exposure: stops = 0, rotation: degrees = 0, ground: boolean = true, ao: boolean = true, shadows: boolean = true, tonemap: "agx|aces|linear" = "agx", light: [azimuth, elevation] = the environment's, background: "environment|plain" = "environment", reflections: boolean = true, contactShadows: boolean = true, specularShadow: boolean = true, groundReflection: 0 to 1 = 0, bloom: 0 to 1 = 0.04, dof: {fstop: number = 2.8, focus: distance or [x, y, z] = the model's centre} = null, accumulate: frames 1 to 256 = 1} = the plain look}"#,
        "Renders the bodies with the CPU renderer, on the window's light viewport background (235, 233, 229; background: dark for the renderer's dark grey). Writes a PNG under the write root when path is given, otherwise returns it as base64. Without tolerance the bodies are meshed for the picture: between a quarter and half a pixel of the picture where it shows the model nearest, never finer than the floor (2e-4 of the diagonal of the box around the bodies), at most 12 degrees per chord, within a total budget of a million triangles (coarser, and coarsened true, when the picture would have more), so a close-up box, a zoom or a larger picture is finer; the same params give the same picture, and the same view, size, zoom and box (or camera, in view.pick) give the same tolerance. With tolerance, at that chordal tolerance alone. A body too fine for the tessellator at it is meshed coarser rather than refused. The result says the tolerance (document units), maxAngle (degrees, or null) and coarsened (true when the tolerance was coarsened for the triangle budget or for a body). orthographic shows lengths across the view to scale. box frames the region between two opposite corners (document units) instead of the whole model, seen from outside the model. zoom magnifies about the centre of the picture (2 shows half as much, twice as large; from 0.01 to 1000). section cuts the picture open with a plane, to look inside a shell, a pocket or a cavity; the document does not change. It is an object: the plane is plane (xy, yz or zx, a datum plane by id or name, or a flat face by its persistent name, with feature to say which body when several have that face) or point and normal ([x, y, z] each), moved by offset (a number, default 0) along its normal; what lies on the side the normal points to is not drawn (a face's normal points out of its body; flip: true keeps the other half). Where the plane passes through the inside of a body the cut is closed with a flat cap in a colour of its own (cap: false leaves it open). The result then has section: {point, normal, range}, the plane as it was used and the offsets between which it passes through the model. look lights the bodies: the studio look (an environment, soft shadows, ambient occlusion, a ground that catches shadows, a tone curve), and its second layer: reflections of the model in its glossy faces and in the ground (the environment where a reflected ray finds nothing on the picture), contact shadows, the key light's shadow on the specular light, bloom, depth of field of a thin lens with fstop focused at focus (a distance along the view or a point; the model's centre by default; a lens of the document's length unit), and accumulate frames averaged into a still, each with its samples moved within the pixel and the lens turned (refused when the frames' samples together, width × height × 4 × frames, pass 2^26). A see-through or cut-open picture is drawn in the plain look.", doc: true, names: ["section"], view_render),
];

/// The command id most like `id`, for "did you mean".
pub(crate) fn closest(id: &str) -> Option<&'static str> {
    // Shared prefix length plus shared suffix length is a cheap similarity
    // that favours a typo inside an otherwise right id.
    let score = |candidate: &str| {
        let prefix = candidate
            .bytes()
            .zip(id.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = candidate
            .bytes()
            .rev()
            .zip(id.bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        prefix + suffix
    };
    Session::commands()
        .iter()
        .map(|c| (score(c.id), c.id))
        .filter(|&(s, _)| s * 2 >= id.len().max(4))
        .max_by_key(|&(s, _)| s)
        .map(|(_, id)| id)
}

// ---------------------------------------------------------------- helpers

/// Applies an edit to the active document as one undo step.
pub(crate) fn edit<T>(
    session: &mut Session,
    command: &'static str,
    label: &str,
    change: impl FnOnce(&mut Document) -> Result<T, DocError>,
) -> Result<T, CommandError> {
    edit_merging(session, command, label, None, false, change)
}

/// [`edit`] as a step with a `key`, folded into the last undo step when
/// `merge` is true and that step has the same key
/// (`History::apply_merging`).
pub(crate) fn edit_merging<T>(
    session: &mut Session,
    command: &'static str,
    label: &str,
    key: Option<&str>,
    merge: bool,
    change: impl FnOnce(&mut Document) -> Result<T, DocError>,
) -> Result<T, CommandError> {
    // Every edit ends with the sketches solved and their solution stored,
    // in the same undo step: a changed variable moves the sketches that
    // use it, and the next solve starts from where they are now.
    // The condition of the features a command with `when` adds.
    let when = session.new_when.clone();
    let open = session.document_mut()?;
    // A Jig in use changes only through its parameters.
    if open.jig_in_use && command != "param.set" {
        return Err(crate::jig_cmds::in_use_refusal(command));
    }
    let cache = &mut open.cache;
    open.history
        .apply_merging(label, key, merge, |doc| {
            // A file from before image places were stored: where the
            // images are now, taken before the change, so the solve after
            // it can carry the sketches along from there. Added only when
            // the change changed something: an edit that does nothing
            // stays one that does nothing (no undo step, nothing to save),
            // so neither the places nor the settle that would write them
            // are added then (the coordinates are the file's solution).
            let pending = craft3d_regen::image_places_to_record(doc, cache);
            let before = (!pending.is_empty()).then(|| doc.clone());
            let first_new = doc.next_feature_id;
            let misused = crate::output_cmds::misuse(doc);
            // What features inserted at the rollback marker re-pointed: a
            // parameter the command points elsewhere itself is the user's
            // choice from now on (`Document::forget_hand_edits`).
            let recorded = (!doc.inserted.is_empty()).then(|| doc.recorded());
            let out = change(doc)?;
            // An assembly's bodies are its instances': a command that
            // would give it a feature of its own is refused whole.
            if doc.is_assembly() && !doc.features.is_empty() {
                return Err(DocError::Invalid(format!(
                    "an assembly has no features of its own; open the part with instance.open and edit it there ({command} is for parts)"
                )));
            }
            if let Some(recorded) = &recorded {
                doc.forget_hand_edits(recorded);
            }
            // Regeneration stays one way: nothing it reads may use an
            // output (docs/design/verification.md › *Outputs*).
            crate::output_cmds::refuse_misuse(&misused, doc)?;
            if let Some(when) = &when {
                for feature in doc.features.iter_mut().filter(|f| f.id.0 >= first_new) {
                    feature.when.get_or_insert_with(|| when.clone());
                }
            }
            if let Some(before) = before {
                if *doc == before {
                    return Ok(out);
                }
                craft3d_regen::record_image_places(doc, pending);
            }
            craft3d_regen::settle_sketches_cached(doc, cache);
            Ok(out)
        })
        .map_err(doc_error(command))
}

/// Finds a feature by id (`3` or `"F3"`) or by name.
pub(crate) fn feature_ref(
    doc: &Document,
    p: &Params,
    command: &str,
    key: &str,
) -> Result<FeatureId, CommandError> {
    let value = p.get(key).ok_or_else(|| p.missing(key))?;
    let by_id = |n: u64| {
        doc.feature(FeatureId(n))
            .map(|f| f.id)
            .ok_or_else(|| CommandError::new(format!("{command}: there is no feature F{n}")))
    };
    match value {
        Value::Number(n) => match n.as_u64() {
            Some(n) => by_id(n),
            None => fail(format!(
                "{command}: `{key}` must be a feature id or name, got {n}"
            )),
        },
        Value::String(text) => {
            if let Some(feature) = doc.feature_by_name(text) {
                return Ok(feature.id);
            }
            match text
                .strip_prefix('F')
                .and_then(|digits| digits.parse::<u64>().ok())
            {
                Some(n) => by_id(n),
                None => {
                    let names: Vec<&str> = doc
                        .features
                        .iter()
                        .map(|f| f.name.as_str())
                        .take(20)
                        .collect();
                    fail(format!(
                        "{command}: there is no feature named `{text}`; features: {}",
                        if names.is_empty() {
                            "none".to_string()
                        } else {
                            names.join(", ")
                        }
                    ))
                }
            }
        }
        _ => fail(format!(
            "{command}: `{key}` must be a feature id (3 or \"F3\") or name"
        )),
    }
}

pub(crate) fn exprs(values: [String; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

pub(crate) fn origin() -> [String; 3] {
    ["0".to_string(), "0".to_string(), "0".to_string()]
}

fn feature_json(feature: &Feature) -> Value {
    json!({ "id": feature.id.to_string(), "name": feature.name })
}

/// Adds a feature to the history where a modeller does: at the end, or,
/// with the history rolled back, right after the rollback marker, which
/// then moves onto the new feature. What came after the marker stays
/// rolled back, after the new feature. Every command that adds a feature
/// goes through here.
pub(crate) fn insert_feature(
    doc: &mut Document,
    name: Option<&str>,
    kind: FeatureKind,
) -> Result<FeatureId, DocError> {
    let id = doc.add_feature(name, kind)?;
    if let Some(marker) = doc.rollback {
        let at = doc
            .features
            .iter()
            .position(|f| f.id == marker)
            .ok_or(DocError::UnknownFeature(marker))?;
        doc.move_feature(id, at.saturating_add(1))?;
        doc.set_rollback(Some(id))?;
    }
    Ok(id)
}

/// The regeneration a feature about to be inserted at the rollback marker
/// is followed against ([`repoint_inserted`]): the document's as it is,
/// rolled back to the marker. `None` without a marker, or for a kind that
/// takes no body.
pub(crate) fn before_insert(
    session: &mut Session,
    kind: &FeatureKind,
) -> Result<Option<std::sync::Arc<craft3d_regen::Regen>>, CommandError> {
    let open = session.document_mut()?;
    if open.history.current().rollback.is_none() || kind.sources().is_empty() {
        return Ok(None);
    }
    Ok(Some(open.regen()))
}

/// After `id` was inserted at the rollback marker: the features after it
/// that took a body it now takes first go on with its body instead
/// ([`Document::repoint_after`]). What it takes is what regeneration builds
/// it from at its place, `before` (the document rolled back to the marker,
/// before it was inserted): its faces followed to the body they are of
/// there. Returns what this insertion re-pointed, for the result
/// (`repointed: [{feature, param, warning?}]`), `None` for nothing.
///
/// A boolean's `tool` pointed at a feature that grew from more than one
/// body (a union inserted before it) now holds the other bodies' material
/// too, so it cuts, joins or keeps more than it did: that is said in a
/// `warning` on the entry.
pub(crate) fn repoint_inserted(
    doc: &mut Document,
    id: FeatureId,
    before: Option<&craft3d_regen::Regen>,
) -> Result<Option<Value>, DocError> {
    let (Some(before), Some(feature)) = (before, doc.feature(id)) else {
        return Ok(None);
    };
    // It fails where it stands: nothing is taken from the features after
    // it. It is kept as inserted all the same, so that an edit that makes
    // it take a body re-points them then.
    let taken = taken_at(doc, before, feature).unwrap_or_default();
    let changed = doc.repoint_after(id, &taken)?;
    doc.record_inserted(id, &changed);
    Ok(repointed_json(doc, id, &taken, &changed))
}

/// The bodies `feature` grows from where it stands, after `earlier` (the
/// regeneration of the features before it): its faces followed to the
/// body they are of there ([`craft3d_regen::followed_feature`]). `None`
/// when it fails there.
fn taken_at(
    doc: &Document,
    earlier: &craft3d_regen::Regen,
    feature: &craft3d_doc::Feature,
) -> Option<Vec<FeatureId>> {
    match craft3d_regen::followed_feature(doc, &earlier.features, feature) {
        Ok(Some(followed)) => Some(followed.kind.grows_from()),
        Ok(None) => Some(feature.kind.grows_from()),
        Err(_) => None,
    }
}

/// The regeneration of the features before `id` (the document rolled back
/// to the one before it): what an inserted feature that is edited is
/// followed against, as it was when it was inserted.
pub(crate) fn regen_before(
    session: &mut Session,
    id: FeatureId,
) -> Result<std::sync::Arc<craft3d_regen::Regen>, CommandError> {
    let open = session.document_mut()?;
    let doc = open.history.current();
    let at = doc
        .features
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| CommandError::new(format!("there is no feature {id}")))?;
    let Some(previous) = at.checked_sub(1).and_then(|i| doc.features.get(i)) else {
        return Ok(std::sync::Arc::new(craft3d_regen::Regen {
            variables: Vec::new(),
            features: Vec::new(),
        }));
    };
    let mut rolled = (**doc).clone();
    rolled.rollback = Some(previous.id);
    Ok(std::sync::Arc::new(craft3d_regen::regenerate_cached(
        &rolled,
        &mut open.cache,
    )))
}

/// After the inserted feature `id` was edited (followed against
/// `earlier`, the features before it): the parameters it re-pointed from
/// a body it no longer takes get that body back. Nothing else is
/// re-pointed: a later feature that names a body it takes now was pointed
/// there by the user. Returns `restored` and `kept` for the result.
fn repoint_edited(
    doc: &mut Document,
    id: FeatureId,
    earlier: &craft3d_regen::Regen,
) -> Result<(Option<Value>, Option<Value>), DocError> {
    let Some(feature) = doc.feature(id) else {
        return Ok((None, None));
    };
    // One that fails where it stands now is not one that no longer takes
    // a body: what it re-pointed stays recorded, for an edit that makes it
    // work again.
    let Some(taken) = taken_at(doc, earlier, feature) else {
        return Ok((None, None));
    };
    let released: Vec<FeatureId> = doc
        .inserted(id)
        .map(|entry| {
            let mut was: Vec<FeatureId> = entry
                .repointed
                .iter()
                .map(|r| r.was)
                .filter(|w| !taken.contains(w))
                .collect();
            // Only to keep the list short: `restore_repointed` reads it
            // as a set.
            was.sort();
            was.dedup();
            was
        })
        .unwrap_or_default();
    let restored = doc.restore_repointed(id, Some(&released));
    Ok((
        restored_json(doc, &restored.given),
        kept_json(doc, &restored.kept),
    ))
}

/// `restored: [{feature, param, names}]`: the parameters given back the
/// feature they named before an inserted feature re-pointed them.
pub(crate) fn restored_json(
    doc: &Document,
    given: &[(FeatureId, String, FeatureId)],
) -> Option<Value> {
    if given.is_empty() {
        return None;
    }
    let called = |f: FeatureId| {
        doc.feature(f)
            .map_or_else(|| f.to_string(), |f| f.name.clone())
    };
    Some(
        given
            .iter()
            .map(|(later, param, names)| {
                json!({"feature": called(*later), "param": param, "names": called(*names)})
            })
            .collect::<Vec<_>>()
            .into(),
    )
}

/// `notRestored: [{feature, param, was, why}]`: the parameters an inserted
/// feature re-pointed that are not given back, because the feature they
/// named before no longer comes before them in the history (moved after
/// them); they still name the inserted feature.
fn kept_json(doc: &Document, kept: &[(FeatureId, String, FeatureId)]) -> Option<Value> {
    if kept.is_empty() {
        return None;
    }
    let called = |f: FeatureId| {
        doc.feature(f)
            .map_or_else(|| f.to_string(), |f| f.name.clone())
    };
    Some(
        kept.iter()
            .map(|(later, param, was)| {
                let (later, was) = (called(*later), called(*was));
                json!({"feature": later, "param": param, "was": was,
                    "why": format!("{was} now comes after {later} in the history, so it cannot be given back; point {param} of {later} with feature.edit")})
            })
            .collect::<Vec<_>>()
            .into(),
    )
}

/// `repointed: [{feature, param, warning?}]` of what inserting (or
/// editing) `id` re-pointed; `None` for nothing.
fn repointed_json(
    doc: &Document,
    id: FeatureId,
    taken: &[FeatureId],
    changed: &[(FeatureId, &'static str, FeatureId)],
) -> Option<Value> {
    if changed.is_empty() {
        return None;
    }
    let called = |doc: &Document, f: FeatureId| {
        doc.feature(f)
            .map_or_else(|| f.to_string(), |f| f.name.clone())
    };
    let inserted = called(doc, id);
    let listed: Vec<Value> = changed
        .iter()
        .map(|&(later, param, was)| {
            let mut entry = json!({"feature": called(doc, later), "param": param});
            let others: Vec<String> = taken
                .iter()
                .filter(|&&t| t != was)
                .map(|&t| format!("`{}`", called(doc, t)))
                .collect();
            if param == "tool"
                && !others.is_empty()
                && let Some(map) = entry.as_object_mut()
            {
                map.insert(
                    "warning".into(),
                    format!(
                        "the tool of `{}` was `{}` and is now `{inserted}`, which holds {} too: it takes more than it did",
                        called(doc, later),
                        called(doc, was),
                        others.join(", ")
                    )
                    .into(),
                );
            }
            entry
        })
        .collect();
    Some(listed.into())
}

pub(crate) fn add_feature(
    session: &mut Session,
    command: &'static str,
    name: Option<&str>,
    kind: FeatureKind,
) -> Run {
    let label = format!("Add {}", kind.type_name());
    let before = before_insert(session, &kind)?;
    let (id, repointed) = edit(session, command, &label, |doc| {
        let id = insert_feature(doc, name, kind)?;
        let repointed = repoint_inserted(doc, id, before.as_deref())?;
        Ok((id, repointed))
    })?;
    let mut result = feature_result(session, id)?;
    with_repointed(&mut result, repointed);
    Ok(result)
}

/// The `cut`, `join` and `intersect` parameters of a sweep: the body the
/// swept solid is combined with at once, and how.
pub(crate) const COMBINE: [(&str, BooleanOp); 3] = [
    ("cut", BooleanOp::Subtract),
    ("join", BooleanOp::Union),
    ("intersect", BooleanOp::Intersect),
];

pub(crate) fn combine_param(
    doc: &Document,
    p: &Params,
    command: &str,
) -> Result<Option<(BooleanOp, FeatureId)>, CommandError> {
    let mut found = None;
    for (key, op) in COMBINE {
        if !p.has(key) {
            continue;
        }
        if found.is_some() {
            return fail(format!(
                "{command}: give one of `cut`, `join` and `intersect`, not several"
            ));
        }
        found = Some((op, feature_ref(doc, p, command, key)?));
    }
    Ok(found)
}

/// Adds a sweep (an extrusion, a revolution) and, when it is to be
/// combined with a body at once, the boolean that does it, as one undo
/// step. The name goes to what the caller builds on: the boolean when
/// there is one, else the sweep.
pub(crate) fn add_sweep(
    session: &mut Session,
    command: &'static str,
    name: Option<&str>,
    kind: FeatureKind,
    combine: Option<(BooleanOp, FeatureId)>,
) -> Run {
    let Some((op, target)) = combine else {
        return add_feature(session, command, name, kind);
    };
    let label = format!("Add {} and {}", kind.type_name(), op.name());
    let boolean = FeatureKind::Boolean(Boolean {
        op,
        target,
        tool: target,
        keep_tool: false,
    });
    let before = before_insert(session, &boolean)?;
    let (sweep, combined, repointed) = edit(session, command, &label, |doc| {
        let sweep = insert_feature(doc, None, kind)?;
        let combined = insert_feature(
            doc,
            name,
            FeatureKind::Boolean(Boolean {
                op,
                target,
                tool: sweep,
                keep_tool: false,
            }),
        )?;
        let repointed = repoint_inserted(doc, combined, before.as_deref())?;
        Ok((sweep, combined, repointed))
    })?;
    let mut result = feature_result(session, combined)?;
    with_repointed(&mut result, repointed);
    let swept = feature_result(session, sweep)?;
    if let Some(map) = result.as_object_mut() {
        map.insert("sweep".into(), swept);
    }
    Ok(result)
}

/// `repointed` (what this command's insertion re-pointed) in a result.
fn with_repointed(result: &mut Value, repointed: Option<Value>) {
    if let (Some(map), Some(listed)) = (result.as_object_mut(), repointed) {
        map.insert("repointed".into(), listed);
    }
}

/// What a feature command returns: the feature and how it regenerated, so
/// the caller sees at once whether the change worked.
pub(crate) fn feature_result(session: &mut Session, id: FeatureId) -> Run {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let Some(feature) = doc.feature(id) else {
        return Ok(json!({ "id": id.to_string() }));
    };
    let mut result = feature_json(feature);
    if let (Some(map), Some(outcome)) = (result.as_object_mut(), regen.feature(id)) {
        let (status, error) = status_json(&outcome.status);
        map.insert("status".into(), status.into());
        if let Some(error) = error {
            map.insert("error".into(), error.into());
        }
        if let Some(root) = craft3d_regen::root_cause(doc, &regen.features, id) {
            map.insert("causedBy".into(), root.to_string().into());
        }
        add_switch(map, doc, &regen.features, feature, outcome);
        if let Some(report) = &outcome.sweep {
            map.insert("path".into(), crate::sweep_path_cmds::report_json(report));
        }
        if let Some(report) = &outcome.thread {
            map.insert("thread".into(), crate::helix_cmds::thread_json(report));
        }
        if let Some(made) = &outcome.developed {
            crate::develop_cmds::report_json(map, made);
        }
        if let Some(made) = &outcome.offset {
            map.insert("offset".into(), crate::offset_cmds::offset_json(made));
        }
    }
    crate::assert_cmds::add_summary(session, &mut result);
    Ok(result)
}

/// The condition of a feature (`when`, as written) and, for one switched
/// off or with an operand switched off, what it passes on to the features
/// after it: `passesOn` the id of the feature whose body it passes on
/// unchanged, `passesNothing` why it passes no body on (as it should),
/// `cannotPass` why the body it would pass on cannot be had; and
/// `leftOut`, the faces and edges it names that are not there because a
/// feature is off ([`craft3d_regen::left_out`]: `{name, why}`, an edge as
/// `a / b`), which it leaves out.
pub(crate) fn add_switch(
    map: &mut serde_json::Map<String, Value>,
    doc: &Document,
    results: &[craft3d_regen::FeatureResult],
    feature: &Feature,
    outcome: &craft3d_regen::FeatureResult,
) {
    if let Some(when) = &feature.when {
        map.insert("when".into(), when.source().into());
    }
    match &outcome.passed {
        Some(craft3d_regen::Passed::Body(from)) => {
            map.insert("passesOn".into(), from.to_string().into());
        }
        Some(craft3d_regen::Passed::Nothing(why)) => {
            map.insert("passesNothing".into(), why.clone().into());
        }
        Some(craft3d_regen::Passed::Broken(why)) => {
            map.insert("cannotPass".into(), why.clone().into());
        }
        None => {}
    }
    let left = craft3d_regen::left_out(doc, results, feature.id);
    if !left.is_empty() {
        let listed: Vec<Value> = left
            .into_iter()
            .map(|(name, why)| json!({ "name": name, "why": why }))
            .collect();
        map.insert("leftOut".into(), listed.into());
    }
}

/// What a feature passes on and leaves out, in a sentence for the feature
/// list's tooltip: `None` for a feature built as usual from everything it
/// names.
pub(crate) fn passes_text(
    doc: &Document,
    results: &[craft3d_regen::FeatureResult],
    feature: &Feature,
    outcome: &craft3d_regen::FeatureResult,
) -> Option<String> {
    let mut parts = Vec::new();
    match &outcome.passed {
        Some(craft3d_regen::Passed::Body(from)) => {
            let by = doc
                .feature(*from)
                .map_or_else(|| from.to_string(), |f| f.name.clone());
            parts.push(format!("Passes on the body of {by} unchanged."));
        }
        Some(craft3d_regen::Passed::Nothing(why)) => {
            parts.push(format!("Makes no body: {why}."));
        }
        Some(craft3d_regen::Passed::Broken(why)) => {
            parts.push(format!("Cannot pass its body on: {why}"));
        }
        None => {}
    }
    let left = craft3d_regen::left_out(doc, results, feature.id);
    if !left.is_empty() {
        let listed: Vec<String> = left
            .iter()
            .map(|(name, why)| format!("{name} ({why})"))
            .collect();
        parts.push(format!("Left out, not there: {}.", listed.join("; ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn status_json(status: &Status) -> (&'static str, Option<&str>) {
    match status {
        Status::Ok => ("ok", None),
        Status::Suppressed => ("suppressed", None),
        Status::Inactive => ("inactive", None),
        Status::RolledBack => ("rolledBack", None),
        Status::Failed(reason) => ("failed", Some(reason)),
    }
}

/// The features of the active document as it is now that fail, by id,
/// in history order: what an edit is compared with to say which features
/// it made fail ([`add_failed`]). Of this revision, never of an earlier
/// one (an undo or a redo does not regenerate, and a regeneration of the
/// revision before it would blame the next edit for what the undo did);
/// when the document was not regenerated since, it is now, which the
/// regeneration cache keeps to what changed.
///
/// Also on a host that regenerates a piece at a time: an edit that
/// regenerates in the command (a feature or sketch command,
/// `variable.set`) compares with this, so the state before it must be
/// regenerated too, or every feature that failed already would be
/// reported as made to fail. An edit that leaves its regeneration to the
/// host takes [`failing_unless_deferred`].
pub(crate) fn failing(session: &mut Session) -> Result<Vec<FeatureId>, CommandError> {
    failing_waiting(session, true)
}

/// [`failing`] for an edit that leaves its regeneration to a host that
/// does it a piece at a time ([`Session::regeneration_deferred`]):
/// empty there without regenerating, as [`add_failed`] then says
/// nothing either but `deferred`.
pub(crate) fn failing_unless_deferred(
    session: &mut Session,
) -> Result<Vec<FeatureId>, CommandError> {
    failing_waiting(session, false)
}

/// [`failing`] when `wait` (an edit's `wait: true`, whose result is
/// final: [`add_failed_waiting`]) and [`failing_unless_deferred`]
/// otherwise.
pub(crate) fn failing_waiting(
    session: &mut Session,
    wait: bool,
) -> Result<Vec<FeatureId>, CommandError> {
    if !wait && session.regeneration_deferred() {
        session.document()?;
        return Ok(Vec::new());
    }
    let open = session.document_mut()?;
    if open.history.current().features.is_empty() {
        return Ok(Vec::new());
    }
    let regen = open.regen();
    Ok(regen
        .features
        .iter()
        .filter(|f| matches!(f.status, Status::Failed(_)))
        .map(|f| f.id)
        .collect())
}

/// Adds `failed` to the result of an edit: the features that fail now
/// and did not `before` it, other than `except` (the feature the result
/// is about, which carries its own status), each as `{feature, id,
/// error}` and, when it failed because something it is made from did,
/// `causedBy`. Left out when there are none. A failed feature is not an
/// error of the command (it did what was asked), so this is how the
/// caller learns what its change broke.
///
/// For a host that regenerates a piece at a time
/// ([`Session::regeneration_deferred`]) the document is not regenerated
/// here: the result says `deferred: true` instead, and the host shows
/// what fails once the regeneration is done.
pub(crate) fn add_failed(
    session: &mut Session,
    before: &[FeatureId],
    except: Option<FeatureId>,
    result: &mut Value,
) -> Result<(), CommandError> {
    add_failed_waiting(session, before, except, result, false)
}

/// [`add_failed`], regenerating the document all the same when `wait`:
/// an edit given `wait: true` reports what fails, never `deferred`, on
/// every host (its result is final, as its assertions' summary is).
pub(crate) fn add_failed_waiting(
    session: &mut Session,
    before: &[FeatureId],
    except: Option<FeatureId>,
    result: &mut Value,
    wait: bool,
) -> Result<(), CommandError> {
    if !wait && session.regeneration_deferred() {
        session.document()?;
        if let Some(map) = result.as_object_mut() {
            map.insert("deferred".into(), Value::Bool(true));
        }
        return Ok(());
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let mut failed = Vec::new();
    for outcome in &regen.features {
        let Status::Failed(reason) = &outcome.status else {
            continue;
        };
        if before.contains(&outcome.id) || except == Some(outcome.id) {
            continue;
        }
        let name = doc.feature(outcome.id).map_or("", |f| f.name.as_str());
        let mut entry = json!({
            "feature": name,
            "id": outcome.id.to_string(),
            "error": reason,
        });
        if let (Some(map), Some(root)) = (
            entry.as_object_mut(),
            craft3d_regen::root_cause(doc, &regen.features, outcome.id),
        ) {
            map.insert("causedBy".into(), root.to_string().into());
        }
        failed.push(entry);
    }
    if let (false, Some(map)) = (failed.is_empty(), result.as_object_mut()) {
        map.insert("failed".into(), Value::Array(failed));
    }
    Ok(())
}

pub(crate) fn vec3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

pub(crate) fn length_unit(doc: &Document) -> &'static str {
    doc.units.length.symbol()
}

// ------------------------------------------------------------------- file

pub(crate) fn file_new(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.new", params, &["units", "tolerance", "loosest"])?;
    let length = match p.str("units")? {
        None => LengthUnit::default(),
        Some(symbol) => LengthUnit::from_symbol(symbol).ok_or_else(|| {
            CommandError::new(format!(
                "file.new: unknown length unit `{symbol}`; use one of um, mm, cm, m, in, ft"
            ))
        })?,
    };
    let mut document = Document::new(Units {
        length,
        ..Units::default()
    });
    if let Some(linear) = p.number("tolerance")? {
        if !Tolerance::linear_in_range(linear) {
            return fail(format!(
                "file.new: `tolerance` must be between {:e} and {:e} {}, the range of a modelling tolerance in the document's length unit; got {linear:e}",
                Tolerance::LEAST_LINEAR,
                Tolerance::GREATEST_LINEAR,
                length.symbol()
            ));
        }
        document.tolerance = Tolerance {
            linear,
            ..Tolerance::default()
        };
    }
    if let Some(loosest) = p.number("loosest")? {
        if !(loosest.is_finite() && loosest >= document.tolerance.linear) {
            return fail(format!(
                "file.new: `loosest` must be a finite length at least the tolerance {:e} {}; got {loosest:e}",
                document.tolerance.linear,
                length.symbol()
            ));
        }
        document.tolerance.loosest = Some(loosest);
    }
    let index = session.add_document(OpenDocument::new(document, None))?;
    Ok(json!({ "document": index, "units": length.symbol() }))
}

/// What a file on disk looks like now; nothing for a file in memory, or
/// where there is no file system.
fn file_stamp(in_memory: bool, resolved: &std::path::Path) -> Option<crate::recovery::FileStamp> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if in_memory {
            None
        } else {
            crate::recovery::FileStamp::of(resolved)
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (in_memory, resolved);
        None
    }
}

/// Reads a `.3dcraft` file from disk; where there is no file system,
/// says so.
fn load_from_disk(
    resolved: &std::path::Path,
) -> Result<(craft3d_doc::Document, Vec<String>), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        craft3d_format::load_noted(resolved).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = resolved;
        Err("there is no file system here, only files in memory".to_string())
    }
}

/// Writes a `.3dcraft` file to disk; where there is no file system, says
/// so.
pub(crate) fn save_to_disk(
    resolved: &std::path::Path,
    document: &craft3d_doc::Document,
    thumbnail: Option<&[u8]>,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        craft3d_format::save_with_thumbnail(resolved, document, thumbnail)
            .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (resolved, document, thumbnail);
        Err("there is no file system here, only files in memory".to_string())
    }
}

pub(crate) fn file_open(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.open", params, &["path"])?;
    let path = p.required_str("path")?;
    let resolved = session.workspace.resolve_read(path)?;
    let in_memory = session.reads_from_memory();
    let loaded = if in_memory {
        session
            .read_input(path, &resolved, craft3d_format::MAX_FILE_BYTES)
            .and_then(|bytes| craft3d_format::from_bytes_noted(&bytes).map_err(|e| e.to_string()))
    } else {
        load_from_disk(&resolved)
    };
    let (document, read) =
        loaded.map_err(|e| CommandError::new(format!("file.open: cannot open `{path}`: {e}")))?;
    // An assembly is a .3dcraftasm file, and a .3dcraftasm file holds an
    // assembly (docs/decisions.md, 2026-10-29).
    let asm_path = craft3d_format::is_assembly_path(path)
        || craft3d_format::is_assembly_path(&resolved.to_string_lossy());
    if document.is_assembly() != asm_path {
        return fail(if asm_path {
            format!("file.open: `{path}` is named as an assembly (.3dcraftasm) and holds a part")
        } else {
            format!(
                "file.open: `{path}` holds an assembly, which is saved as a .3dcraftasm file only"
            )
        });
    }
    let features = document.features.len();
    let kept = crate::workspace::kept_path(&session.workspace.read, path, &resolved);
    let mut open = OpenDocument::new(document, Some(kept.clone()));
    open.disk = file_stamp(in_memory, &resolved);
    open.file = file_on_disk(in_memory, &resolved);
    // A published Jig is opened to be used: by the name of the file read.
    let jig = craft3d_format::is_jig_path(path)
        || craft3d_format::is_jig_path(&resolved.to_string_lossy());
    open.jig_in_use = jig;
    let index = session.add_document(open)?;
    let mut result = json!({ "document": index, "path": kept, "features": features });
    // An assembly reads its sources as it opens: what it shows follows
    // them (a source that cannot be read fails its instances, by name).
    if session.document()?.history.current().is_assembly() {
        session.read_sources(false)?;
        if let Some(map) = result.as_object_mut() {
            map.insert("assembly".into(), true.into());
        }
    }
    if let (true, Some(map)) = (jig, result.as_object_mut()) {
        map.insert("jigInUse".into(), true.into());
    }
    // What reading the file changed (a tolerance outside the range a
    // document may have, from a file saved before 2026-10-15).
    if let (false, Some(map)) = (read.is_empty(), result.as_object_mut()) {
        map.insert("warnings".into(), read.into());
    }
    Ok(result)
}

/// The absolute path of the file at `resolved`, for
/// [`crate::DocumentStatus::file`]; `None` for a file in memory.
fn file_on_disk(in_memory: bool, resolved: &std::path::Path) -> Option<String> {
    if in_memory {
        return None;
    }
    std::path::absolute(resolved)
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
}

fn file_save(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.save", params, &["path", "overwrite", "thumbnail"])?;
    let overwrite = p.bool("overwrite")?.unwrap_or(false);
    let thumbnail = p.bool("thumbnail")?;
    let in_use = session.document()?.jig_in_use;
    let path = match (p.str("path")?, &session.document()?.path) {
        (Some(path), _) => path.to_string(),
        (None, _) if in_use => {
            return fail(
                "file.save: this is a Jig in use and its .3dcraftjig file is never saved over; save it as a model: give a `path` ending in .3dcraft",
            );
        }
        (None, Some(own)) => own.clone(),
        (None, None) => return fail("file.save: this document has no file yet; give a `path`"),
    };
    if craft3d_format::is_jig_path(&path)
        || session
            .workspace
            .resolve_write(&path)
            .is_ok_and(|resolved| craft3d_format::is_jig_path(&resolved.to_string_lossy()))
    {
        return fail(
            "file.save: a .3dcraftjig file is a published Jig and is written by jig.export (File, Export as Jig…), never saved over; give a `path` ending in .3dcraft",
        );
    }
    let is_assembly = session.document()?.history.current().is_assembly();
    let asm_path = craft3d_format::is_assembly_path(&path);
    if is_assembly && !asm_path {
        return fail(format!(
            "file.save: an assembly is saved as a .{} file; give a `path` ending in .{}",
            craft3d_format::ASSEMBLY_EXTENSION,
            craft3d_format::ASSEMBLY_EXTENSION
        ));
    }
    if !is_assembly && asm_path {
        return fail(format!(
            "file.save: a .{} file holds an assembly, and this document is a part; give a `path` ending in .3dcraft",
            craft3d_format::ASSEMBLY_EXTENSION
        ));
    }
    let resolved = session.workspace.resolve_write(&path)?;
    let access = session.workspace.write.clone();
    // An assembly's source paths stay relative to its file: saved into
    // another folder, they are rewritten for it. The rewritten document is
    // what is written; the open one takes it (with every undo state) only
    // once the file is written, together with its new path.
    let rebase = if is_assembly {
        let old_base = session.asm_base()?;
        let new_base = crate::asm::dir_of(&crate::workspace::kept_path(&access, &path, &resolved));
        Some((old_base, new_base))
    } else {
        None
    };
    let in_memory = session.writes_to_memory();
    let open = session.document_mut()?;
    // A recovered document is older than its original file when that
    // file was written after the recovery was taken (by another 3DCraft,
    // or by a save the crashed one made after its last autosave).
    if let Some(recovered) = open.recovered
        && !overwrite
        && (open.path.as_deref() == Some(path.as_str())
            || open.path == Some(crate::workspace::kept_path(&access, &path, &resolved)))
        && let Some(now) = file_stamp(in_memory, &resolved)
        && recovered.original != Some(now)
    {
        return fail(format!(
            "file.save: `{path}` changed on disk since this recovered document was autosaved, so saving would replace a file that may be newer; save under another `path`, or pass {{\"overwrite\": true}} to replace it (at the window: File, Save As, and choose the file again)"
        ));
    }
    let cannot = |e: String| CommandError::new(format!("file.save: cannot save `{path}`: {e}"));
    // A Jig is saved with a picture for a gallery, unless told not to;
    // one that cannot be drawn (no bodies yet) is saved without.
    let wants_thumbnail = thumbnail.unwrap_or_else(|| open.history.current().is_jig());
    let (png, thumbnail_error) = if wants_thumbnail {
        match thumbnail_png(session) {
            Ok(png) => (Some(png), None),
            Err(e) => (None, Some(e.message().to_string())),
        }
    } else {
        (None, None)
    };
    let open = session.document_mut()?;
    let mut document = std::sync::Arc::clone(open.history.current());
    if let Some((old_base, new_base)) = &rebase
        && let Some(rebased) = crate::asm_cmds::rebased(&document, old_base, new_base)
    {
        document = std::sync::Arc::new(rebased);
    }
    if in_memory {
        let bytes =
            craft3d_format::to_bytes_with_thumbnail(&document, png.as_deref().map(Vec::as_slice))
                .map_err(|e| cannot(e.to_string()))?;
        session
            .write_resolved(&resolved, bytes)
            .map_err(|e| cannot(e.message().to_string()))?;
    } else {
        save_to_disk(&resolved, &document, png.as_deref().map(Vec::as_slice)).map_err(cannot)?;
    }
    let open = session.document_mut()?;
    if let Some((old_base, new_base)) = &rebase {
        open.history
            .rewrite_all(|doc| crate::asm_cmds::rebased(doc, old_base, new_base));
    }
    open.saved_revision = Some(open.history.revision());
    let kept = crate::workspace::kept_path(&access, &path, &resolved);
    open.path = Some(kept.clone());
    open.disk = file_stamp(in_memory, &resolved);
    open.file = file_on_disk(in_memory, &resolved);
    open.recovered = None;
    // Saved as a model: it is edited like any other from now on.
    open.jig_in_use = false;
    let mut result = json!({ "path": kept });
    if let Some(map) = result.as_object_mut()
        && wants_thumbnail
    {
        map.insert("thumbnail".into(), png.is_some().into());
        if let Some(error) = thumbnail_error {
            map.insert("thumbnailError".into(), error.into());
        }
    }
    Ok(result)
}

fn file_close(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.close", params, &["document", "discard"])?;
    let discard = p.bool("discard")?.unwrap_or(false);
    session.document()?;
    let count = session.documents.len();
    let index = match p.integer("document", 0, u64::MAX)? {
        None => session.active,
        Some(n) => match usize::try_from(n).ok().filter(|&i| i < count) {
            Some(index) => index,
            None => {
                return fail(format!(
                    "file.close: there is no document {n}; {count} are open (file.list)"
                ));
            }
        },
    };
    let Some(open) = session.documents.get(index) else {
        return fail("file.close: there is no document to close");
    };
    if open.close_asks() && !discard && open.jig_in_use {
        return fail(
            "file.close: this Jig's parameters were changed; save the result as a model (file.save with a path ending in .3dcraft), or pass {\"discard\": true}",
        );
    }
    if open.close_asks() && !discard {
        return fail(
            "file.close: the document has unsaved changes; save it, or pass {\"discard\": true}",
        );
    }
    session.documents.remove(index);
    // The active document stays active when another one closes; when it
    // closes itself, the last one becomes active, as before.
    session.active = if index < session.active {
        session.active - 1
    } else if index == session.active {
        session.documents.len().saturating_sub(1)
    } else {
        session.active
    };
    Ok(json!({ "open": session.documents.len(), "active": session.active }))
}

/// A default chordal tolerance that suits the size of the model: a
/// thousandth of the side of a square with the bodies' total area.
pub(crate) fn automatic_tolerance(regen: &Regen) -> f64 {
    let bodies: Vec<&std::sync::Arc<craft3d_topo::Body>> =
        regen.bodies().map(|(_, body)| body).collect();
    crate::mass::measure_ahead(&bodies);
    let area: f64 = regen
        .bodies()
        .filter_map(|(_, body)| mass_of(body).ok())
        .map(|m| m.area)
        .sum();
    if area.is_finite() && area > 0.0 {
        area.sqrt() * 1e-3
    } else {
        1e-3
    }
}

/// How finely a mesh export follows the model (`file.export {quality}`):
/// a chordal tolerance as a multiple of the automatic one (a thousandth
/// of the side of a square with the bodies' area) and an angle criterion
/// in degrees, as the viewport has (no chord turns an edge's tangent, no
/// triangle side the surface's normal, by more than it: small holes and
/// thin bent tubes stay round).
pub const EXPORT_QUALITIES: [(&str, f64, f64); 3] = [
    ("coarse", 2.0, 20.0),
    ("normal", 1.0, 10.0),
    ("fine", 0.25, 5.0),
];

/// The largest angle criterion `file.export {angle}` takes, in degrees.
pub const MAX_EXPORT_ANGLE: f64 = 90.0;

/// The smallest angle criterion `file.export {angle}` takes (besides 0,
/// none), in degrees.
pub const MIN_EXPORT_ANGLE: f64 = 1.0;

/// The most triangles a mesh export holds, all bodies together: past it
/// `file.export` refuses, asking for a coarser mesh, rather than write a
/// file no program can open.
pub const EXPORT_MAX_TRIANGLES: usize = 20_000_000;

/// The tessellation of a mesh export, from `quality`, `tolerance` and
/// `angle` (see [`EXPORT_QUALITIES`]). Without `tolerance` the quality
/// sets both (normal by default); with it, the chordal tolerance is that
/// and the angle is `angle`, or the quality's when `quality` is given,
/// or none (an export with a tolerance makes the mesh it always made).
/// Returns the parameters, the quality's name and the angle in degrees.
fn export_tess(
    p: &Params,
    regen: &Regen,
) -> Result<(TessParams, &'static str, Option<f64>), CommandError> {
    let names: Vec<&str> = EXPORT_QUALITIES.iter().map(|(name, ..)| *name).collect();
    let quality = match p.str("quality")? {
        None => None,
        Some(asked) => Some(
            EXPORT_QUALITIES
                .iter()
                .find(|(name, ..)| *name == asked)
                .ok_or_else(|| {
                    CommandError::new(format!(
                        "file.export: unknown quality `{}`; use {}",
                        asked.chars().take(40).collect::<String>(),
                        names.join(", ")
                    ))
                })?,
        ),
    };
    let (name, scale, degrees) = quality
        .copied()
        .or_else(|| EXPORT_QUALITIES.get(1).copied())
        .unwrap_or(("normal", 1.0, 10.0));
    let angle = match p.number("angle")? {
        Some(a) if a != 0.0 && !(MIN_EXPORT_ANGLE..=MAX_EXPORT_ANGLE).contains(&a) => {
            return fail(format!(
                "file.export: `angle` is in degrees: 0 for none, or from {MIN_EXPORT_ANGLE} to {MAX_EXPORT_ANGLE} (a finer angle makes meshes too large to use)"
            ));
        }
        Some(a) => Some(a),
        None => None,
    };
    let given = p.number("tolerance")?;
    let chordal_tolerance = match given {
        Some(t) if t > 0.0 && t.is_finite() => t,
        Some(_) => return fail("file.export: `tolerance` must be positive"),
        None => automatic_tolerance(regen) * scale,
    };
    let degrees = match (angle, given, quality) {
        (Some(a), ..) => a,
        (None, Some(_), None) => 0.0,
        _ => degrees,
    };
    let max_angle = (degrees > 0.0).then(|| degrees.to_radians());
    Ok((
        TessParams {
            chordal_tolerance,
            max_angle,
            max_triangles: Some(EXPORT_MAX_TRIANGLES),
            ..TessParams::default()
        },
        name,
        (degrees > 0.0).then_some(degrees),
    ))
}

/// One mesh per body, with the name of the feature that made it.
/// One body's mesh, with the appearance of the body and of each of its
/// faces (`appearance`).
struct BodyMesh {
    name: String,
    mesh: Mesh,
    /// The body's own (for an export's object; a picture colours faces).
    appearance: Option<craft3d_doc::Appearance>,
    faces: Vec<Option<craft3d_doc::Appearance>>,
}

/// Of the bodies of `only` when given (in history order), else of every
/// body.
fn meshes(
    open: &mut OpenDocument,
    command: &str,
    tess: &TessParams,
    only: Option<&[FeatureId]>,
) -> Result<Vec<BodyMesh>, CommandError> {
    let regen = open.regen();
    let doc = &open.bodies_doc();
    let values = crate::appearance::Values::new(doc);
    let mut out = Vec::new();
    for (id, body) in regen.bodies() {
        if only.is_some_and(|only| !only.contains(&id)) {
            continue;
        }
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        // What is left of the export's triangle budget, for this body.
        let used: usize = out.iter().map(|m: &BodyMesh| m.mesh.triangles.len()).sum();
        let mut params = *tess;
        params.max_triangles = tess.max_triangles.map(|cap| cap.saturating_sub(used));
        let mesh = tessellate_body(body, &params).map_err(|e| match e {
            craft3d_tess::TessError::OverBudget { .. } => CommandError::new(format!(
                "{command}: the mesh would have more than {} triangles; choose a coarser quality, a larger tolerance or a larger angle",
                tess.max_triangles.unwrap_or(0)
            )),
            e => CommandError::new(format!("{command}: {name}: {e}")),
        })?;
        out.push(BodyMesh {
            name,
            mesh,
            appearance: crate::appearance::body_appearance_in(&values, &regen, id),
            faces: crate::appearance::face_appearances_in(&values, &regen, id, body.faces.len()),
        });
    }
    if out.is_empty() {
        return fail(format!(
            "{command}: there are no bodies; add a feature, or check document.inspect for failed features"
        ));
    }
    Ok(out)
}

fn file_export(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "file.export",
        params,
        &[
            "path",
            "format",
            "tolerance",
            "sketch",
            "fold",
            "sheetMetal",
            "face",
            "faces",
            "set",
            "feature",
            "construction",
            "xAxis",
            "origin",
            "gap",
            "width",
            "bodies",
            "view",
            "projection",
            "hidden",
            "quality",
            "angle",
            "stroke",
            "trace",
        ],
    )?;
    let path = p.required_str("path")?;
    if let Some(format) = crate::drawing_cmds::wanted(p.str("format")?, path) {
        // An explicit null is no `bodies`, as for STEP and the meshes.
        if p.get("bodies").is_some() {
            return fail(
                "file.export: `bodies` is for STEP and mesh files; an outline (.dxf, .svg) takes `sketch`, `face`, `faces` or `set`",
            );
        }
        if let Some(key) = ["quality", "angle"]
            .into_iter()
            .find(|key| p.get(key).is_some())
        {
            return fail(format!(
                "file.export: `{key}` is for mesh files (.stl, .3mf, .obj); an outline keeps every curve as it is"
            ));
        }
        let resolved = session.workspace.resolve_write(path)?;
        return crate::drawing_cmds::export(session, &p, path, &resolved, format);
    }
    // The other formats hold every body, or those named in `bodies`.
    let outline_only = || match [
        "sketch",
        "fold",
        "sheetMetal",
        "face",
        "faces",
        "set",
        "feature",
        "construction",
        "xAxis",
        "origin",
        "gap",
        "width",
        "view",
        "projection",
        "hidden",
        "stroke",
        "trace",
    ]
    .into_iter()
    .find(|key| p.has(key))
    {
        Some(key) => fail(format!(
            "file.export: `{key}` is for a drawing (the outline of a sketch or a face, or the model as seen), which is a .dxf or .svg file; the other formats hold every body"
        )),
        None => Ok(()),
    };
    if crate::step_cmds::wanted(p.str("format")?, path) {
        outline_only()?;
        if let Some(key) = ["tolerance", "quality", "angle"]
            .into_iter()
            .find(|key| p.get(key).is_some())
        {
            return fail(format!(
                "file.export: `{key}` is for mesh formats; STEP is exact and carries the document's tolerance"
            ));
        }
        let resolved = session.workspace.resolve_write(path)?;
        let open = session.document_mut()?;
        let regen = open.regen();
        let only = crate::export_bodies::chosen(&p, &open.bodies_doc(), &regen)?;
        if open.history.current().is_assembly() {
            return crate::asm_export::step(session, path, &resolved, only.as_deref());
        }
        return crate::step_cmds::export(session, path, &resolved, only.as_deref());
    }
    let format = match p.str("format")? {
        Some("stl") => MeshFormat::StlBinary,
        Some("stl-ascii") => MeshFormat::StlAscii,
        Some("3mf") => MeshFormat::ThreeMf,
        Some("obj") => MeshFormat::Obj,
        Some(other) => return fail(format!("file.export: unknown format `{other}`; use step, stl, stl-ascii, 3mf, obj, dxf or svg")),
        None => std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(MeshFormat::from_extension)
            .ok_or_else(|| {
                CommandError::new(format!(
                    "file.export: cannot tell the format from `{path}`; use a .step, .stp, .stl, .3mf, .obj, .dxf or .svg name, or pass `format`"
                ))
            })?,
    };
    outline_only()?;
    let resolved = session.workspace.resolve_write(path)?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let only = crate::export_bodies::chosen(&p, &open.bodies_doc(), &regen)?;
    let (tess, quality, degrees) = export_tess(&p, &regen)?;
    let meshes = meshes(open, "file.export", &tess, only.as_deref())?;
    let unit = match open.history.current().units.length {
        LengthUnit::Micrometre => craft3d_meshio::Unit::Micron,
        LengthUnit::Millimetre => craft3d_meshio::Unit::Millimeter,
        LengthUnit::Centimetre => craft3d_meshio::Unit::Centimeter,
        LengthUnit::Metre => craft3d_meshio::Unit::Meter,
        LengthUnit::Inch => craft3d_meshio::Unit::Inch,
        LengthUnit::Foot => craft3d_meshio::Unit::Foot,
    };
    let mut model = craft3d_meshio::Model {
        unit,
        objects: Vec::new(),
        materials: Vec::new(),
    };
    let mut triangles = 0;
    let mut closed = 0;
    for body in &meshes {
        let name = &body.name;
        let mut material = |appearance: Option<craft3d_doc::Appearance>| {
            appearance.map(|a| export_material(&mut model.materials, &a))
        };
        // A body without a colour whose faces have some: the rest of its
        // faces in the colour the pictures draw it in, since in 3MF a
        // triangle without a material of its own has its object's, and an
        // object whose triangles have materials needs one. In OBJ every
        // body without a colour has it when any body has one, since there
        // a material holds until the next `usemtl`.
        let plain =
            || craft3d_doc::Appearance::color(craft3d_render::RenderParams::default().color);
        let coloured = |b: &BodyMesh| b.appearance.is_some() || b.faces.iter().any(Option::is_some);
        let own = body.appearance.or_else(|| {
            let needed = if format == MeshFormat::Obj {
                meshes.iter().any(coloured)
            } else {
                body.faces.iter().any(Option::is_some)
            };
            needed.then(plain)
        });
        let own = material(own);
        let faces: Vec<Option<u32>> = body.faces.iter().map(|a| material(*a)).collect();
        let mut object = craft3d_io::to_object_with_materials(&body.mesh, name, |face| {
            faces.get(face as usize).copied().flatten()
        })
        .map_err(|e| CommandError::new(format!("file.export: {name}: {e}")))?;
        object.material = own;
        triangles += object.triangles.len();
        if object.kind == craft3d_meshio::ObjectKind::Solid {
            closed += 1;
        }
        model.objects.push(object);
    }
    // OBJ's colours go to a material library beside it, named after it
    // (a document without colours has no materials and needs none).
    let (bytes, library) = if format == MeshFormat::Obj && !model.materials.is_empty() {
        let (name, at) = obj_library(path)?;
        let (obj, mtl) = craft3d_meshio::write_obj_with_mtl(&model, &name)
            .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
        (obj.into_bytes(), mtl.map(|mtl| (at, mtl)))
    } else {
        let bytes = craft3d_io::encode(&model, format)
            .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
        (bytes, None)
    };
    let written = bytes.len();
    let library = match library {
        Some((at, mtl)) => {
            let library = session.workspace.resolve_write(&at)?;
            write_with_library(session, &resolved, bytes, &library, mtl.into_bytes())?;
            Some(at)
        }
        None => {
            session.write_resolved(&resolved, bytes)?;
            None
        }
    };
    let open = session.document_mut()?;
    Ok(json!({
        "path": path,
        "mtl": library,
        "bytes": written,
        "bodies": model.objects.len(),
        // How many of the meshes are closed and consistently oriented.
        "closed": closed,
        "triangles": triangles,
        "tolerance": tess.chordal_tolerance,
        "angle": degrees,
        "quality": quality,
        "unit": length_unit(open.history.current()),
    }))
}

/// The material library of the OBJ file at `path`: its name as the OBJ
/// refers to it (the OBJ's file name with `.mtl` for its extension) and
/// its path beside the OBJ. Refused, rather than renamed, for a name the
/// `mtllib` statement cannot hold (white space or `#`: it takes a list of
/// names, and `#` starts a comment), so that two OBJ files never share a
/// library and no file the person did not name is replaced; refused too
/// when the library would be the OBJ itself (`x.mtl` written as OBJ,
/// compared without case for file systems that ignore it).
fn obj_library(path: &str) -> Result<(String, String), CommandError> {
    let at = std::path::Path::new(path);
    let stem = at
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem.is_empty()
        || stem
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '#')
    {
        return fail(format!(
            "file.export: `{path}` has colours, which go to a material library named after the file, and an OBJ cannot name a library with white space or `#` in its name; choose a file name without them (e.g. `{}.obj`)",
            if stem.is_empty() {
                "parts".to_string()
            } else {
                stem.chars()
                    .map(|c| {
                        if c.is_whitespace() || c.is_control() || c == '#' {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect()
            }
        ));
    }
    let name = format!("{stem}.mtl");
    let own = at
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if own == name.to_lowercase() {
        return fail(format!(
            "file.export: `{path}` would be its own material library ({name}); give the OBJ file another extension, e.g. `.obj`"
        ));
    }
    let beside = at.with_file_name(&name).to_string_lossy().into_owned();
    Ok((name, beside))
}

/// Writes an OBJ file and its material library: the library first, so
/// that an OBJ never names one that is not there, after checking that
/// both places can take a file; when the OBJ still cannot be written the
/// library is put back as it was (removed, or its former content
/// restored), so a failed export leaves nothing behind. In memory the two
/// files are put side by side, and the library taken out again when the
/// OBJ does not fit.
fn write_with_library(
    session: &mut Session,
    obj_path: &std::path::Path,
    obj: Vec<u8>,
    library_path: &std::path::Path,
    library: Vec<u8>,
) -> Result<(), CommandError> {
    if session.writes_to_memory() {
        let before = session.memory_file(library_path).cloned();
        session.write_resolved(library_path, library)?;
        if let Err(e) = session.write_resolved(obj_path, obj) {
            let _ = session.forget_output(library_path);
            if let Some(bytes) = before {
                let _ = session.write_resolved(library_path, bytes);
            }
            return Err(e);
        }
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        write_with_library_on_disk(obj_path, &obj, library_path, &library)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (obj, library);
        fail(format!(
            "file.export: cannot write `{}`: there is no file system here, only files in memory",
            obj_path.display()
        ))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_with_library_on_disk(
    obj_path: &std::path::Path,
    obj: &[u8],
    library_path: &std::path::Path,
    library: &[u8],
) -> Result<(), CommandError> {
    for path in [obj_path, library_path] {
        if path.is_dir() {
            return fail(format!(
                "file.export: cannot write `{}`: it is a directory",
                path.display()
            ));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
            && !parent.is_dir()
        {
            return fail(format!(
                "file.export: cannot write `{}`: there is no directory `{}`",
                path.display(),
                parent.display()
            ));
        }
    }
    let before = std::fs::read(library_path).ok();
    crate::workspace::write_atomic(library_path, library)?;
    if let Err(e) = crate::workspace::write_atomic(obj_path, obj) {
        let _ = match before {
            Some(bytes) => crate::workspace::write_atomic(library_path, &bytes),
            None => {
                std::fs::remove_file(library_path).map_err(|e| CommandError::new(e.to_string()))
            }
        };
        return Err(e);
    }
    Ok(())
}

/// The index of the material of `appearance` in an export's `materials`,
/// added when it is not there yet: one material per appearance, named by
/// its colour (with its metallic and roughness when it has them).
fn export_material(
    materials: &mut Vec<craft3d_meshio::Material>,
    appearance: &craft3d_doc::Appearance,
) -> u32 {
    let o = &appearance.optics;
    let layered = craft3d_meshio::Material {
        name: String::new(),
        color: appearance.color,
        metallic: appearance.metallic,
        roughness: appearance.roughness,
        clearcoat: (o.clearcoat > 0.0).then_some((o.clearcoat, o.clearcoat_roughness)),
        anisotropy: (o.anisotropy != 0.0).then_some(o.anisotropy),
        transmission: (o.transmission > 0.0).then_some(o.transmission),
        ior: (o.transmission > 0.0).then_some(o.ior),
        transmission_filter: o.absorption.filter(|_| o.transmission > 0.0),
    };
    let same = |m: &craft3d_meshio::Material| {
        craft3d_meshio::Material {
            name: String::new(),
            ..m.clone()
        } == layered
    };
    if let Some(at) = materials.iter().position(same) {
        return u32::try_from(at).unwrap_or(u32::MAX);
    }
    let mut name = appearance.hex();
    if let Some(metallic) = appearance.metallic {
        name.push_str(&format!(" metallic {metallic}"));
    }
    if let Some(roughness) = appearance.roughness {
        name.push_str(&format!(" roughness {roughness}"));
    }
    for (what, v) in [
        ("anisotropy", o.anisotropy),
        ("clearcoat", o.clearcoat),
        ("transmission", o.transmission),
        ("subsurface", o.subsurface),
    ] {
        if v != 0.0 {
            name.push_str(&format!(" {what} {v}"));
        }
    }
    let material = craft3d_meshio::Material { name, ..layered };
    materials.push(material);
    u32::try_from(materials.len().saturating_sub(1)).unwrap_or(u32::MAX)
}

// ------------------------------------------------------------------- edit

fn edit_undo(session: &mut Session, params: &Value) -> Run {
    Params::new("edit.undo", params, &[])?;
    match session.document_mut()?.history.undo() {
        Some(label) => Ok(json!({ "undone": label })),
        None => fail("edit.undo: there is nothing to undo"),
    }
}

fn edit_redo(session: &mut Session, params: &Value) -> Run {
    Params::new("edit.redo", params, &[])?;
    match session.document_mut()?.history.redo() {
        Some(label) => Ok(json!({ "redone": label })),
        None => fail("edit.redo: there is nothing to redo"),
    }
}

// -------------------------------------------------------------- variables

/// A variable with its value, as commands return it.
fn variable_json(regen: &Regen, name: &str, unit: &str) -> Value {
    match regen.variables.iter().find(|v| v.name == name) {
        None => json!({ "name": name }),
        Some(v) => match &v.value {
            Ok(value) => {
                let mut out = json!({
                    "name": v.name, "expr": v.expr, "value": value.value,
                    "dimension": value.dimension.to_string(), "lengthUnit": unit,
                });
                if let Some(map) = out.as_object_mut() {
                    // An angle's value is in radians, whatever the
                    // document's angle unit: say so, and give degrees.
                    if value.dimension.angle != 0 {
                        map.insert("angleUnit".into(), "rad".into());
                    }
                    if value.dimension == craft3d_expr::Dimension::ANGLE {
                        map.insert("degrees".into(), value.value.to_degrees().into());
                    }
                    // `1/16 in` is 1 / (16 in): almost never what was meant.
                    let hint = value.dimension.unit_hint(&v.expr);
                    if !hint.is_empty() {
                        map.insert(
                            "warning".into(),
                            format!("`{}` is {}{hint}", v.expr, value.dimension.described()).into(),
                        );
                    } else if let Some(degrees) = degrees_in_radians(&v.expr) {
                        map.insert("warning".into(), degrees.into());
                    }
                }
                out
            }
            Err(error) => json!({ "name": v.name, "expr": v.expr, "error": error }),
        },
    }
}

/// A plain number inside `sin`, `cos` or `tan` that looks like degrees
/// (more than a full turn in radians: `sin(30)`), which those functions
/// take as radians: the warning to give, naming the unit to write. Only a
/// literal argument is looked at; the meaning does not change (a plain
/// number in them stays radians, as files rely on).
pub(crate) fn degrees_in_radians(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    for function in ["sin", "cos", "tan"] {
        let pattern = format!("{function}(");
        let mut from = 0;
        while let Some(found) = source.get(from..).and_then(|rest| rest.find(&pattern)) {
            let at = from + found;
            from = at + pattern.len();
            // `asin(`, `my_sin(`: another name.
            let before = at.checked_sub(1).and_then(|i| bytes.get(i)).copied();
            if before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let Some(inner) = source.get(from..) else {
                continue;
            };
            let Some(close) = inner.find(')') else {
                continue;
            };
            let argument = inner.get(..close).unwrap_or("").trim();
            if let Ok(number) = argument.parse::<f64>()
                && number.is_finite()
                && number.abs() > std::f64::consts::TAU
            {
                return Some(format!(
                    "`{function}({argument})` takes {argument} as radians ({:.0} degrees); for degrees write `{function}({argument} deg)`",
                    number.to_degrees()
                ));
            }
        }
    }
    None
}

fn variable_set(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.set", params, &["name", "expr", "wait"])?;
    let wait = crate::assert_cmds::wait_param(&p)?;
    let name = p.required_str("name")?;
    // A name the expression language uses itself (`m` the metre, `mod`
    // the function) is refused first, with what it clashes with and what
    // to call the variable instead.
    craft3d_expr::check_variable_name(name)
        .map_err(|e| CommandError::new(format!("variable.set: `name`: {e}")))?;
    let expr = p.expr("expr")?.ok_or_else(|| p.missing("expr"))?;
    // "Make the wall 1 mm thicker" written as `wall + 1 mm` can never be
    // worked out, and kept it would fail everything the variable drives:
    // refused, with the value to write instead.
    if craft3d_expr::Expr::parse(&expr).is_ok_and(|parsed| parsed.variables().contains(&name)) {
        let open = session.document_mut()?;
        let regen = open.regen();
        let now = regen
            .variables
            .iter()
            .find(|v| v.name == name)
            .map_or_else(String::new, |v| format!(" (it is `{}` now)", v.expr));
        return fail(format!(
            "variable.set: `{expr}` uses `{name}` itself, so it could never be worked out; give \
             its new value instead{now}"
        ));
    }
    // A public parameter is set by a form, which can only set a value.
    if let Some(variable) = session.document()?.history.current().variable(name)
        && variable.public.is_some()
        && !crate::param_cmds::is_value(&expr)
    {
        return fail(format!(
            "variable.set: `{name}` is a public parameter, so its expression must be a value (a number with its unit), not the formula `{expr}`; variable.unpublish it first to give it a formula"
        ));
    }
    // Regenerated before the edit too: variable.set regenerates in the
    // command, so its `failed` must not count what failed already.
    let before = failing(session)?;
    let doc_before = std::sync::Arc::clone(session.document()?.history.current());
    edit(session, "variable.set", &format!("Set {name}"), |doc| {
        doc.set_variable(name, &expr)
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let mut result = variable_json(&regen, name, length_unit(open.history.current()));
    add_parameter_warnings(&doc_before, open.history.current(), &mut result);
    add_failed_waiting(session, &before, None, &mut result, wait)?;
    crate::assert_cmds::add_summary_waiting(session, &mut result, wait);
    Ok(result)
}

fn variable_delete(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.delete", params, &["name", "force"])?;
    let name = p.required_str("name")?;
    let force = p.bool("force")?.unwrap_or(false);
    // An output goes as a variable does, refused while it is used.
    if session.document()?.history.current().output(name).is_some() {
        let users = crate::output_cmds::users(session.document()?.history.current(), name);
        if !force && !users.is_empty() {
            return fail(format!(
                "variable.delete: the output `{name}` is still used by {}; change those first, or give force: true to delete it anyway (they are then unknown or in error)",
                users.join(", ")
            ));
        }
        edit(
            session,
            "variable.delete",
            &format!("Delete output {name}"),
            |doc| doc.remove_output(name).map(|_| ()),
        )?;
        let mut result = json!({ "deleted": name, "output": true });
        if let (false, Some(map)) = (users.is_empty(), result.as_object_mut()) {
            map.insert("usedBy".into(), users.into());
        }
        return Ok(result);
    }
    let users = variable_users(session.document()?.history.current(), name);
    if !force && !users.is_empty() {
        return fail(format!(
            "variable.delete: `{name}` is still used by {}; change those first, or give force: true to delete it anyway (they then fail until it is defined again)",
            users.join(", ")
        ));
    }
    let before = if users.is_empty() {
        Vec::new()
    } else {
        failing_unless_deferred(session)?
    };
    let doc_before = std::sync::Arc::clone(session.document()?.history.current());
    edit(
        session,
        "variable.delete",
        &format!("Delete {name}"),
        |doc| doc.remove_variable(name),
    )?;
    let mut result = json!({ "deleted": name });
    add_parameter_warnings(
        &doc_before,
        session.document()?.history.current(),
        &mut result,
    );
    if !users.is_empty() {
        if let Some(map) = result.as_object_mut() {
            map.insert("usedBy".into(), users.into());
        }
        add_failed(session, &before, None, &mut result)?;
    }
    Ok(result)
}

/// Adds `parameterWarnings` to the result of a designer's edit: the public
/// parameters it made invalid (`variable.set` and `variable.delete` do
/// not refuse that; `param.set` and `variable.publish` do).
fn add_parameter_warnings(before: &Document, after: &Document, result: &mut Value) {
    let warnings = crate::param_cmds::invalidated(before, after);
    if let (false, Some(map)) = (warnings.is_empty(), result.as_object_mut()) {
        map.insert("parameterWarnings".into(), warnings.into());
    }
}

/// Whether the expression text `source` refers to the variable `name`.
fn mentions(source: &str, name: &str) -> bool {
    craft3d_expr::refers_to_variable(source, name)
}

/// Whether a string inside the JSON `value` (a stored query or what an
/// assertion measures) mentions the variable `name`, leaving out the keys
/// whose strings are names and not expressions. The JSON was read from a
/// stored text, so its nesting is bounded; `depth` caps it anyway.
fn json_mentions(value: &Value, name: &str, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    match value {
        Value::String(text) => mentions(text, name),
        Value::Array(items) => items
            .iter()
            .any(|item| json_mentions(item, name, depth + 1)),
        // Under a key that names something, a string or a list of
        // strings is names (bodies, kinds, a plane, an axis); an object
        // there (a subject query) is looked into like the rest.
        Value::Object(map) => map
            .iter()
            .any(|(key, inner)| !names_under(key, inner) && json_mentions(inner, name, depth + 1)),
        _ => false,
    }
}

/// Whether `inner`, under `key` of a stored query or measure, is names
/// (bodies, kinds, a plane, an axis), not expressions.
fn names_under(key: &str, inner: &Value) -> bool {
    let names = match inner {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        _ => false,
    };
    JSON_NAMES.contains(&key) && names
}

/// The names the expressions in the JSON `value` (a stored query or
/// measure) use, into `out`, leaving out the keys whose strings are names
/// as [`json_mentions`] does.
pub(crate) fn json_expr_names(
    value: &Value,
    out: &mut std::collections::BTreeSet<String>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    match value {
        Value::String(text) => {
            out.extend(craft3d_expr::names_in(text).into_iter().map(str::to_string))
        }
        Value::Array(items) => {
            for item in items {
                json_expr_names(item, out, depth + 1);
            }
        }
        Value::Object(map) => {
            for (key, inner) in map {
                if !names_under(key, inner) {
                    json_expr_names(inner, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// The keys of a stored query or measure whose strings are names.
const JSON_NAMES: &[&str] = &[
    "name",
    "type",
    "material",
    "color",
    "set",
    "in",
    "madeBy",
    "copyOf",
    "kind",
    "surface",
    "along",
    "perpendicular",
    "of",
    "from",
    "to",
    "measure",
    "face",
    "edge",
    "feature",
    "axis",
    "plane",
    "about",
    "measured",
];

/// Everything in `doc` that uses the variable `name`, one short phrase
/// each, in document order: other variables, features and sketches (with
/// the parameters), assertions and named sets.
pub(crate) fn variable_users(doc: &Document, name: &str) -> Vec<String> {
    variable_users_kinds(doc, name)
        .into_iter()
        .map(|(_, user)| user)
        .collect()
}

/// [`variable_users`], each with whether it may use an output: an output
/// or an assertion, which are computed after regeneration (true), or an
/// input of it (false).
pub(crate) fn variable_users_kinds(doc: &Document, name: &str) -> Vec<(bool, String)> {
    let mut users: Vec<(bool, String)> = Vec::new();
    for variable in &doc.variables {
        if variable.name != name && mentions(variable.expr.source(), name) {
            users.push((false, format!("the variable `{}`", variable.name)));
        }
    }
    for variable in &doc.variables {
        let Some(public) = &variable.public else {
            continue;
        };
        let bounds: Vec<&str> = public
            .bounds()
            .into_iter()
            .filter(|(_, expr)| expr.is_some_and(|e| mentions(e.source(), name)))
            .map(|(what, _)| what)
            .collect();
        if !bounds.is_empty() {
            users.push((
                false,
                format!("the parameter `{}` ({})", variable.name, bounds.join(", ")),
            ));
        }
    }
    for feature in &doc.features {
        // In the formulas of a curve the parameter is not the variable of
        // its name.
        let formulas = feature
            .kind
            .formula_exprs()
            .into_iter()
            .filter(|_| name != craft3d_doc::FORMULA_PARAMETER);
        let params: Vec<String> = feature
            .exprs()
            .into_iter()
            .chain(formulas)
            .filter(|(_, expr)| mentions(expr.source(), name))
            .map(|(param, _)| param)
            .collect();
        if !params.is_empty() {
            let what = if matches!(feature.kind, FeatureKind::Sketch(_)) {
                "sketch"
            } else {
                "feature"
            };
            users.push((
                false,
                format!(
                    "the {what} `{}` ({}: {})",
                    feature.name,
                    feature.id,
                    params.join(", ")
                ),
            ));
        }
    }
    for assertion in &doc.assertions {
        let in_bound = assertion
            .exprs()
            .iter()
            .any(|expr| mentions(expr.source(), name));
        let in_measure = serde_json::from_str::<Value>(&assertion.measure)
            .is_ok_and(|measure| json_mentions(&measure, name, 0));
        if in_bound || in_measure {
            users.push((true, format!("the assertion `{}`", assertion.name)));
        }
    }
    for set in &doc.sets {
        if serde_json::from_str::<Value>(&set.query)
            .is_ok_and(|query| json_mentions(&query, name, 0))
        {
            users.push((false, format!("the set `{}`", set.name)));
        }
    }
    for body in &doc.bodies {
        if body
            .exprs()
            .iter()
            .any(|expr| mentions(expr.source(), name))
        {
            users.push((false, format!("the colour or material of {}", body.feature)));
        }
    }
    for output in &doc.outputs {
        let uses = match &output.def {
            craft3d_doc::OutputDef::Expr(e) => mentions(e.source(), name),
            craft3d_doc::OutputDef::Measure(text) => serde_json::from_str::<Value>(text)
                .is_ok_and(|measure| json_mentions(&measure, name, 0)),
        };
        if output.name != name && uses {
            users.push((true, format!("the output `{}`", output.name)));
        }
    }
    users
}

fn variable_rename(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.rename", params, &["name", "to"])?;
    let (name, to) = (p.required_str("name")?, p.required_str("to")?);
    craft3d_expr::check_variable_name(to)
        .map_err(|e| CommandError::new(format!("variable.rename: `to`: {e}")))?;
    let output = session.document()?.history.current().output(name).is_some();
    edit(
        session,
        "variable.rename",
        &format!("Rename {name}"),
        |doc| {
            if output {
                doc.rename_output(name, to)?;
            } else {
                doc.rename_variable(name, to)?;
            }
            crate::assert_cmds::rename_variable_in_assertions(doc, name, to)
        },
    )?;
    Ok(json!({ "name": to }))
}

// --------------------------------------------------------------- features

/// The `axis` and `angle` parameters shared by the primitives that can be
/// turned; absent means upright.
fn orientation(p: &Params) -> Result<Orientation, CommandError> {
    let default = Orientation::default();
    Ok(Orientation {
        axis: p.expr3("axis")?.map(exprs).unwrap_or(default.axis),
        angle: p.expr("angle")?.map(Expr::new).unwrap_or(default.angle),
    })
}

pub(crate) fn required(p: &Params, key: &str) -> Result<Expr, CommandError> {
    Ok(Expr::new(p.expr(key)?.ok_or_else(|| p.missing(key))?))
}

fn position(p: &Params) -> Result<[Expr; 3], CommandError> {
    Ok(exprs(p.expr3("position")?.unwrap_or_else(origin)))
}

fn feature_block(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.block",
        params,
        &["size", "position", "axis", "angle", "name"],
    )?;
    let size = p.expr3("size")?.ok_or_else(|| p.missing("size"))?;
    let kind = FeatureKind::Block(Block {
        position: position(&p)?,
        size: exprs(size),
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.block", p.str("name")?, kind)
}

fn feature_cylinder(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.cylinder",
        params,
        &["radius", "height", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Cylinder(Cylinder {
        position: position(&p)?,
        radius: required(&p, "radius")?,
        height: required(&p, "height")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.cylinder", p.str("name")?, kind)
}

fn feature_sphere(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.sphere", params, &["radius", "position", "name"])?;
    let kind = FeatureKind::Sphere(Sphere {
        position: position(&p)?,
        radius: required(&p, "radius")?,
    });
    add_feature(session, "feature.sphere", p.str("name")?, kind)
}

fn feature_cone(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.cone",
        params,
        &["radius", "height", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Cone(Cone {
        position: position(&p)?,
        radius: required(&p, "radius")?,
        height: required(&p, "height")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.cone", p.str("name")?, kind)
}

fn feature_torus(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.torus",
        params,
        &["major", "minor", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Torus(Torus {
        position: position(&p)?,
        major: required(&p, "major")?,
        minor: required(&p, "minor")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.torus", p.str("name")?, kind)
}

fn feature_extrude(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.extrude",
        params,
        &[
            "sketch",
            "distance",
            "throughAll",
            "upTo",
            "symmetric",
            "reversed",
            "cut",
            "join",
            "intersect",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, "feature.extrude", "sketch")?;
    doc.profile_source(sketch)
        .map_err(doc_error("feature.extrude"))?;
    let combine = combine_param(doc, &p, "feature.extrude")?;
    let Some(end) = extrude_end(&p, "feature.extrude")? else {
        return fail(
            "feature.extrude: missing parameter `distance` (or `throughAll` or `upTo`: where the extrusion ends)",
        );
    };
    let symmetric = p.bool("symmetric")?.unwrap_or(false);
    symmetric_fits(&end, symmetric, "feature.extrude")?;
    let kind = FeatureKind::Extrude(Extrude {
        sketch,
        // An end that is not a distance keeps one, unused.
        distance: Expr::new(p.expr("distance")?.unwrap_or_else(|| "0".to_string())),
        symmetric,
        end,
        reversed: p.bool("reversed")?.unwrap_or(false),
    });
    add_sweep(session, "feature.extrude", p.str("name")?, kind, combine)
}

/// The end of an extrusion as the parameters give it: one of `distance`,
/// `throughAll` (true) and `upTo`. `None` when none of them is given.
fn extrude_end(p: &Params, command: &str) -> Result<Option<ExtrudeEnd>, CommandError> {
    let distance = p.expr("distance")?.is_some();
    let through = p.bool("throughAll")?;
    let up_to = match p.str("upTo")? {
        None => None,
        Some(text) => Some(
            text.parse::<PersistentName>()
                .map_err(|reason| CommandError::new(format!("{command}: `upTo`: {reason}")))?,
        ),
    };
    if up_to.as_ref().is_some_and(|name| !name.is_face()) {
        return fail(format!(
            "{command}: `upTo` must name a face (see faceNames in document.inspect)"
        ));
    }
    let given =
        usize::from(distance) + usize::from(through == Some(true)) + usize::from(up_to.is_some());
    if given > 1 {
        return fail(format!(
            "{command}: give one of `distance`, `throughAll` and `upTo`, not several"
        ));
    }
    Ok(match (up_to, through) {
        (Some(face), _) => Some(ExtrudeEnd::UpToFace { face }),
        (None, Some(true)) => Some(ExtrudeEnd::ThroughAll),
        // `throughAll: false` on its own asks for a distance again.
        (None, Some(false)) => Some(ExtrudeEnd::Distance),
        (None, None) => distance.then_some(ExtrudeEnd::Distance),
    })
}

/// An extrusion up to a face has one end: it cannot be symmetric.
fn symmetric_fits(end: &ExtrudeEnd, symmetric: bool, command: &str) -> Result<(), CommandError> {
    if symmetric && matches!(end, ExtrudeEnd::UpToFace { .. }) {
        return fail(format!(
            "{command}: `symmetric` does not go with `upTo`: an extrusion up to a face goes one way"
        ));
    }
    Ok(())
}

/// The parameters `feature.edit` can change, by name.
/// The parameters of a thread that `feature.edit` takes as null.
const THREAD_NULLS: [&str; 2] = ["size", "pitch"];

const EDITABLE: [&str; 52] = [
    "sketch",
    "sections",
    "ruled",
    "closed",
    "source",
    "target",
    "tool",
    "op",
    "size",
    "position",
    "axis",
    "radius",
    "height",
    "major",
    "minor",
    "angle",
    "distance",
    "throughAll",
    "upTo",
    "symmetric",
    "reversed",
    "translate",
    "pivot",
    "point",
    "normal",
    "plane",
    "step",
    "count",
    "keep",
    "keepTool",
    "edges",
    "second",
    "chain",
    "path",
    "faces",
    "thickness",
    "side",
    "neutral",
    "open",
    "pitch",
    "turns",
    "leftHanded",
    "length",
    "cosmetic",
    "modeled",
    "corners",
    "a",
    "b",
    "at",
    "joints",
    "u",
    "v",
];

fn feature_edit(session: &mut Session, params: &Value) -> Run {
    let mut allowed = vec!["feature"];
    allowed.extend(EDITABLE);
    let p = Params::new("feature.edit", params, &allowed)?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.edit",
        "feature",
    )?;
    // A thread's size and pitch may be given as null: back to the size
    // of the face, the coarse pitch of the size.
    let is_thread = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Thread(_))
    );
    // Validate the shape of everything given before touching anything.
    // An axis given as a string is a datum axis, for a circular pattern.
    // (For every other kind an axis is a direction, and a string is
    // refused as one.)
    let circular = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Pattern(craft3d_doc::Pattern {
            layout: PatternLayout::Circular { .. },
            ..
        }))
    );
    let new_axis = match p.get("axis") {
        Some(Value::String(_)) if circular => Some(crate::datum_cmds::datum_ref(
            session.document()?.history.current(),
            &p,
            "feature.edit",
            "axis",
            "datumAxis",
        )?),
        _ => None,
    };
    // The axis of a revolution is one of the sketch's own, a line of the
    // sketch or a datum axis, as feature.revolve takes it.
    let revolution = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Revolve(_))
    );
    let new_revolve_axis = match p.get("axis") {
        Some(value) if revolution => Some(crate::sweep_cmds::revolve_axis(
            session.document()?.history.current(),
            "feature.edit",
            value,
        )?),
        _ => None,
    };
    // The sections of a loft, as feature.loft takes them.
    let new_sections = match p.get("sections") {
        Some(value) => Some(crate::loft_cmds::loft_sections(
            session.document()?.history.current(),
            "feature.edit",
            value,
        )?),
        None => None,
    };
    // What the feature is made from: another sketch, another body. Each
    // has to be there now; whether it can be used (it comes earlier, it
    // has a body nothing else took) is for regeneration to say, as when
    // the feature was added.
    let mut made_from: [(&str, Option<FeatureId>); 5] = [
        ("sketch", None),
        ("source", None),
        ("target", None),
        ("tool", None),
        ("path", None),
    ];
    for (key, found) in &mut made_from {
        if p.get(key).is_some() {
            let doc = session.document()?.history.current();
            let other = feature_ref(doc, &p, "feature.edit", key)?;
            if other == id {
                return fail(format!(
                    "feature.edit: `{key}` is the feature itself; a feature cannot be made from itself"
                ));
            }
            if *key == "sketch" {
                // An offset is made from a sketch; everything else that
                // takes one also takes an offset of one.
                let offset = matches!(
                    doc.feature(id).map(|f| &f.kind),
                    Some(FeatureKind::OffsetProfile(_))
                );
                if offset {
                    doc.sketch(other).map_err(doc_error("feature.edit"))?;
                } else {
                    doc.profile_source(other)
                        .map_err(doc_error("feature.edit"))?;
                }
            }
            if *key == "path" {
                crate::sweep_path_cmds::path_ref(doc, other, "feature.edit")?;
            }
            *found = Some(other);
        }
    }
    let [
        (_, new_sketch),
        (_, new_source),
        (_, new_target),
        (_, new_tool),
        (_, new_path),
    ] = made_from;
    let new_op = match p.str("op")? {
        None => None,
        Some(op) => Some(BooleanOp::from_name(op).ok_or_else(|| {
            CommandError::new(format!(
                "feature.edit: unknown op `{op}`; use union, subtract or intersect"
            ))
        })?),
    };
    for key in [
        "size",
        "position",
        "axis",
        "translate",
        "pivot",
        "point",
        "normal",
        "step",
    ] {
        if key == "axis" && (new_axis.is_some() || new_revolve_axis.is_some()) {
            continue;
        }
        // The step of a circular pattern is one length along its axis
        // (a screw), checked with the kind.
        if key == "step" && circular {
            continue;
        }
        // The size of a thread is a designation (`M10`), checked with
        // the kind.
        if key == "size" && (is_thread || matches!(p.get("size"), Some(Value::String(_)))) {
            continue;
        }
        p.expr3(key)?;
    }
    for key in [
        "radius",
        "height",
        "major",
        "minor",
        "angle",
        "distance",
        "count",
        "second",
        "thickness",
    ] {
        p.expr(key)?;
    }
    for key in [
        "symmetric",
        "reversed",
        "keep",
        "keepTool",
        "chain",
        "modeled",
        "cosmetic",
    ] {
        p.bool(key)?;
    }
    let new_end = extrude_end(&p, "feature.edit")?;
    let new_plane = match p.get("plane") {
        None => None,
        Some(_) => Some(crate::datum_cmds::datum_ref(
            session.document()?.history.current(),
            &p,
            "feature.edit",
            "plane",
            "datumPlane",
        )?),
    };
    let new_neutral = match p.get("neutral") {
        None => None,
        Some(value) => Some(crate::datum_cmds::plane_value(
            session.document()?.history.current(),
            "feature.edit",
            "neutral",
            value,
        )?),
    };
    let given: Vec<&str> = EDITABLE
        .into_iter()
        .filter(|key| {
            p.get(key).is_some()
                || (is_thread && THREAD_NULLS.contains(key) && p.has(key))
                // A circular pattern's step as null: a plain turn again.
                || (circular && *key == "step" && p.has(key))
        })
        .collect();
    if given.is_empty() {
        return fail(format!(
            "feature.edit: give at least one parameter to change ({})",
            EDITABLE.join(", ")
        ));
    }
    let mut kind = session
        .document()?
        .history
        .current()
        .feature(id)
        .map(|f| f.kind.clone())
        .ok_or_else(|| CommandError::new(format!("feature.edit: there is no feature {id}")))?;
    // Each kind lists the parameters it has; a given one outside the list
    // is an error rather than ignored.
    let (name, own): (&str, Vec<(&str, Slot)>) = match &mut kind {
        FeatureKind::Sketch(_) => {
            return fail(
                "feature.edit: a sketch is changed with the sketch.* commands (sketch.edit, sketch.constrain, ...)",
            );
        }
        FeatureKind::Block(b) => (
            "block",
            vec![
                ("position", Slot::Triple(&mut b.position)),
                ("size", Slot::Triple(&mut b.size)),
                ("axis", Slot::Triple(&mut b.orientation.axis)),
                ("angle", Slot::One(&mut b.orientation.angle)),
            ],
        ),
        FeatureKind::Cylinder(c) => (
            "cylinder",
            vec![
                ("position", Slot::Triple(&mut c.position)),
                ("radius", Slot::One(&mut c.radius)),
                ("height", Slot::One(&mut c.height)),
                ("axis", Slot::Triple(&mut c.orientation.axis)),
                ("angle", Slot::One(&mut c.orientation.angle)),
            ],
        ),
        FeatureKind::Sphere(s) => (
            "sphere",
            vec![
                ("position", Slot::Triple(&mut s.position)),
                ("radius", Slot::One(&mut s.radius)),
            ],
        ),
        FeatureKind::Cone(c) => (
            "cone",
            vec![
                ("position", Slot::Triple(&mut c.position)),
                ("radius", Slot::One(&mut c.radius)),
                ("height", Slot::One(&mut c.height)),
                ("axis", Slot::Triple(&mut c.orientation.axis)),
                ("angle", Slot::One(&mut c.orientation.angle)),
            ],
        ),
        FeatureKind::Torus(t) => (
            "torus",
            vec![
                ("position", Slot::Triple(&mut t.position)),
                ("major", Slot::One(&mut t.major)),
                ("minor", Slot::One(&mut t.minor)),
                ("axis", Slot::Triple(&mut t.orientation.axis)),
                ("angle", Slot::One(&mut t.orientation.angle)),
            ],
        ),
        FeatureKind::Extrude(e) => (
            "extrude",
            vec![
                ("sketch", Slot::Elsewhere),
                ("distance", Slot::One(&mut e.distance)),
                ("symmetric", Slot::Flag(&mut e.symmetric)),
                ("reversed", Slot::Flag(&mut e.reversed)),
                // Applied below: the three name one thing, the end.
                ("throughAll", Slot::Elsewhere),
                ("upTo", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Revolve(r) => (
            "revolve",
            vec![
                ("sketch", Slot::Elsewhere),
                ("axis", Slot::Elsewhere),
                ("angle", Slot::One(&mut r.angle)),
                ("symmetric", Slot::Flag(&mut r.symmetric)),
            ],
        ),
        FeatureKind::Loft(l) => (
            "loft",
            vec![
                ("sections", Slot::Elsewhere),
                ("ruled", Slot::Flag(&mut l.ruled)),
                ("closed", Slot::Flag(&mut l.closed)),
            ],
        ),
        FeatureKind::Sweep(_) => (
            "sweep",
            vec![("sketch", Slot::Elsewhere), ("path", Slot::Elsewhere)],
        ),
        FeatureKind::Fill(_) => ("fill", vec![("sketch", Slot::Elsewhere)]),
        // Its creases and fixed point are changed by fold.edit.
        FeatureKind::Fold(_) => ("fold", vec![("sketch", Slot::Elsewhere)]),
        FeatureKind::SheetMetal(_) => ("sheet-metal part", vec![("sketch", Slot::Elsewhere)]),
        FeatureKind::Split(_) => ("split", vec![("source", Slot::Elsewhere)]),
        FeatureKind::Developable(_) => (
            "developable",
            vec![("a", Slot::Elsewhere), ("b", Slot::Elsewhere)],
        ),
        FeatureKind::SurfaceNetwork(_) => (
            "network surface",
            vec![
                ("u", Slot::Elsewhere),
                ("v", Slot::Elsewhere),
                ("edges", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Unfold(u) => (
            "unfold",
            vec![
                ("faces", Slot::Elsewhere),
                ("at", Slot::Triple(&mut u.at)),
                ("angle", Slot::One(&mut u.angle)),
            ],
        ),
        // What an import is, is the file: import it again to change it.
        FeatureKind::Import(_) => ("import", Vec::new()),
        FeatureKind::OffsetProfile(o) => (
            "offset profile",
            vec![
                ("sketch", Slot::Elsewhere),
                ("distance", Slot::One(&mut o.distance)),
                ("corners", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Transform(t) => (
            "transform",
            vec![
                ("source", Slot::Elsewhere),
                ("translate", Slot::Triple(&mut t.translate)),
                ("pivot", Slot::Triple(&mut t.pivot)),
                ("axis", Slot::Triple(&mut t.axis)),
                ("angle", Slot::One(&mut t.angle)),
                ("keep", Slot::Flag(&mut t.keep)),
            ],
        ),
        FeatureKind::Boolean(b) => (
            "boolean",
            vec![
                ("op", Slot::Elsewhere),
                ("target", Slot::Elsewhere),
                ("tool", Slot::Elsewhere),
                ("keepTool", Slot::Flag(&mut b.keep_tool)),
            ],
        ),
        FeatureKind::Chamfer(c) => (
            "chamfer",
            vec![
                // Applied below: together they are the size's form.
                ("edges", Slot::Elsewhere),
                ("distance", Slot::Elsewhere),
                ("second", Slot::Elsewhere),
                ("angle", Slot::Elsewhere),
                ("chain", Slot::Flag(&mut c.chain)),
            ],
        ),
        FeatureKind::Fillet(f) => (
            "fillet",
            vec![
                // Applied below, with the edges of a chamfer.
                ("edges", Slot::Elsewhere),
                ("radius", Slot::One(&mut f.radius)),
                ("chain", Slot::Flag(&mut f.chain)),
            ],
        ),
        FeatureKind::OffsetFace(o) => (
            "offset face",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("distance", Slot::One(&mut o.distance)),
            ],
        ),
        FeatureKind::Draft(d) => (
            "draft",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("neutral", Slot::Elsewhere),
                ("angle", Slot::One(&mut d.angle)),
                ("reversed", Slot::Flag(&mut d.reversed)),
            ],
        ),
        FeatureKind::Thicken(t) => (
            "thicken",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("side", Slot::Elsewhere),
                ("thickness", Slot::One(&mut t.thickness)),
            ],
        ),
        FeatureKind::Plate(p) => (
            "plate",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("side", Slot::Elsewhere),
                ("joints", Slot::Elsewhere),
                ("thickness", Slot::One(&mut p.thickness)),
            ],
        ),
        FeatureKind::MoveFace(m) => (
            "move face",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("translate", Slot::Triple(&mut m.translate)),
                ("pivot", Slot::Triple(&mut m.pivot)),
                ("axis", Slot::Triple(&mut m.axis)),
                ("angle", Slot::One(&mut m.angle)),
            ],
        ),
        FeatureKind::DeleteFace(_) => (
            "delete face",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Shell(s) => (
            "shell",
            vec![
                ("source", Slot::Elsewhere),
                ("thickness", Slot::One(&mut s.thickness)),
                // Applied below.
                ("open", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Mirror(m) => (
            "mirror",
            vec![
                ("source", Slot::Elsewhere),
                ("point", Slot::Triple(&mut m.point)),
                ("normal", Slot::Triple(&mut m.normal)),
                // Applied below: a datum plane instead of point and normal.
                ("plane", Slot::Elsewhere),
                ("keep", Slot::Flag(&mut m.keep)),
            ],
        ),
        FeatureKind::DatumPoint(datum) => (
            "datum point",
            match &mut datum.at {
                craft3d_doc::PointRef::At { position } => {
                    vec![("position", Slot::Triple(position))]
                }
                _ => {
                    return fail(
                        "feature.edit: a datum point at a vertex or a centre has no parameter to change here; give it another definition with datum.point and replace",
                    );
                }
            },
        ),
        FeatureKind::Thread(thread) => (
            "thread",
            vec![
                // Applied below: a designation or an expression, and
                // either may be null (back to the face's size, the
                // coarse pitch).
                ("size", Slot::Elsewhere),
                ("pitch", Slot::Elsewhere),
                ("length", Slot::One(&mut thread.length)),
                ("leftHanded", Slot::Flag(&mut thread.left_handed)),
                // Applied below: either says whether it is cut.
                ("modeled", Slot::Elsewhere),
                ("cosmetic", Slot::Elsewhere),
            ],
        ),
        FeatureKind::DatumHelix(helix) => {
            let mut slots = match &mut helix.start {
                craft3d_doc::HelixStart::Radius { radius } => vec![("radius", Slot::One(radius))],
                craft3d_doc::HelixStart::Point { .. } => Vec::new(),
            };
            slots.extend([
                ("pitch", Slot::One(&mut helix.pitch)),
                // Applied below: either one is how far the helix goes.
                ("turns", Slot::Elsewhere),
                ("height", Slot::Elsewhere),
                ("leftHanded", Slot::Flag(&mut helix.left_handed)),
            ]);
            ("datum helix", slots)
        }
        FeatureKind::DatumAxis(_) => {
            return fail(
                "feature.edit: a datum axis has no parameter to change here; give it another definition with datum.axis and replace",
            );
        }
        FeatureKind::DatumPlane(datum) => (
            "datum plane",
            match &mut datum.by {
                PlaneBy::Offset { distance, .. } => vec![("distance", Slot::One(distance))],
                PlaneBy::Angle { angle, .. } => vec![("angle", Slot::One(angle))],
                PlaneBy::ThreePoints { .. } => {
                    return fail(
                        "feature.edit: a datum plane through three points has no parameter to change here; give it other points with datum.plane and replace",
                    );
                }
            },
        ),
        FeatureKind::Pattern(pattern) => {
            let mut slots = vec![
                ("source", Slot::Elsewhere),
                ("count", Slot::One(&mut pattern.count)),
            ];
            match &mut pattern.layout {
                PatternLayout::Linear { step } => {
                    slots.push(("step", Slot::Triple(step)));
                    ("linear pattern", slots)
                }
                PatternLayout::Circular {
                    point,
                    axis,
                    angle,
                    step,
                    ..
                } => {
                    slots.push(("point", Slot::Triple(point)));
                    slots.push(("axis", Slot::Triple(axis)));
                    slots.push(("angle", Slot::One(angle)));
                    slots.push(("step", Slot::Maybe(step)));
                    ("circular pattern", slots)
                }
            }
        }
    };
    let names: Vec<&str> = own.iter().map(|(key, _)| *key).collect();
    if let Some(unused) = given.iter().find(|key| !names.contains(key)) {
        return fail(format!(
            "feature.edit: `{unused}` is not a parameter of a {name}; its parameters are: {}",
            names.join(", ")
        ));
    }
    for (key, slot) in own {
        match slot {
            // Applied below: a datum axis instead of point and axis.
            Slot::Triple(_) if key == "axis" && new_axis.is_some() => {}
            Slot::Triple(target) => {
                if let Some(value) = p.expr3(key)? {
                    *target = exprs(value);
                }
            }
            Slot::One(target) => {
                if let Some(value) = p.expr(key)? {
                    *target = Expr::new(value);
                }
            }
            // Given as null, the parameter is taken away.
            Slot::Maybe(target) => {
                if p.has(key) {
                    *target = p.expr(key)?.map(Expr::new);
                }
            }
            Slot::Flag(target) => {
                if let Some(value) = p.bool(key)? {
                    *target = value;
                }
            }
            Slot::Elsewhere => {}
        }
    }
    match (&mut kind, new_axis) {
        (
            FeatureKind::Pattern(craft3d_doc::Pattern {
                layout: PatternLayout::Circular { about, .. },
                ..
            }),
            new_axis,
        ) => match new_axis {
            Some(_) if p.get("point").is_some() => {
                return fail(
                    "feature.edit: give either `axis` as a datum axis or `axis` as a direction and `point`, not both",
                );
            }
            Some(axis) => *about = Some(axis),
            // A line of its own again when a direction or point is given.
            None if p.get("axis").is_some() || p.get("point").is_some() => *about = None,
            None => {}
        },
        (_, Some(_)) => {
            return fail(
                "feature.edit: `axis` as a datum axis goes with a circular pattern; this feature's `axis` is a direction [x, y, z]",
            );
        }
        (_, None) => {}
    }
    if let FeatureKind::Thread(thread) = &mut kind {
        if p.has("size") {
            thread.size = match p.get("size") {
                Some(size) => Some(crate::helix_cmds::size_value("feature.edit", size)?),
                None => None,
            };
        }
        if p.has("pitch") {
            thread.pitch = p.expr("pitch")?.map(Expr::new);
        }
        if let Some(modeled) = crate::helix_cmds::modeled_value("feature.edit", &p)? {
            thread.modeled = modeled;
        }
    }
    if let FeatureKind::DatumHelix(helix) = &mut kind {
        match (p.expr("turns")?, p.expr("height")?) {
            (Some(_), Some(_)) => {
                return fail(
                    "feature.edit: give either `turns` or `height`: both say how far the helix goes",
                );
            }
            (Some(turns), None) => {
                helix.extent = craft3d_doc::HelixExtent::Turns {
                    turns: Expr::new(turns),
                };
            }
            (None, Some(height)) => {
                helix.extent = craft3d_doc::HelixExtent::Height {
                    height: Expr::new(height),
                };
            }
            (None, None) => {}
        }
    }
    if let FeatureKind::Mirror(mirror) = &mut kind {
        // A plane of its own again when point or normal is given.
        match new_plane {
            Some(_) if p.get("normal").is_some() || p.get("point").is_some() => {
                return fail(
                    "feature.edit: give either `plane` (a datum plane) or `normal` and `point`, not both",
                );
            }
            Some(plane) => mirror.plane = Some(plane),
            None if p.get("normal").is_some() || p.get("point").is_some() => mirror.plane = None,
            None => {}
        }
    }
    // What the feature is made from. (A parameter the kind does not have
    // was refused above.)
    match &mut kind {
        FeatureKind::Extrude(extrusion) => {
            extrusion.sketch = new_sketch.unwrap_or(extrusion.sketch);
        }
        FeatureKind::Revolve(revolution) => {
            revolution.sketch = new_sketch.unwrap_or(revolution.sketch);
            revolution.axis = new_revolve_axis.unwrap_or(revolution.axis);
        }
        FeatureKind::Loft(loft) => {
            if let Some(sections) = new_sections {
                loft.sections = sections;
            }
            if loft.closed && loft.sections.len() < 3 {
                return fail("feature.edit: a closed loft passes through at least 3 sketches");
            }
        }
        FeatureKind::Fill(fill) => fill.sketch = new_sketch.unwrap_or(fill.sketch),
        FeatureKind::Fold(fold) => fold.sketch = new_sketch.unwrap_or(fold.sketch),
        FeatureKind::SheetMetal(part) => part.sketch = new_sketch.unwrap_or(part.sketch),
        FeatureKind::OffsetProfile(offset) => {
            offset.sketch = new_sketch.unwrap_or(offset.sketch);
            if let Some(corners) = crate::offset_cmds::corners_param(&p, "feature.edit")? {
                offset.corners = corners;
            }
        }
        FeatureKind::Sweep(sweep) => {
            sweep.sketch = new_sketch.unwrap_or(sweep.sketch);
            sweep.path = new_path.unwrap_or(sweep.path);
            if sweep.sketch == sweep.path {
                return fail(
                    "feature.edit: `sketch` and `path` are the same sketch; the path is drawn in a sketch of its own",
                );
            }
        }
        FeatureKind::Transform(transform) => {
            transform.source = new_source.unwrap_or(transform.source);
        }
        FeatureKind::Mirror(mirror) => mirror.source = new_source.unwrap_or(mirror.source),
        FeatureKind::Pattern(pattern) => pattern.source = new_source.unwrap_or(pattern.source),
        FeatureKind::Split(split) => split.source = new_source.unwrap_or(split.source),
        FeatureKind::Boolean(boolean) => {
            boolean.op = new_op.unwrap_or(boolean.op);
            boolean.target = new_target.unwrap_or(boolean.target);
            boolean.tool = new_tool.unwrap_or(boolean.tool);
            if boolean.target == boolean.tool {
                return fail(
                    "feature.edit: `target` and `tool` are the same feature; a body cannot be combined with itself",
                );
            }
        }
        FeatureKind::Chamfer(chamfer) => crate::blend_cmds::edit(&p, chamfer)?,
        FeatureKind::Fillet(fillet) => crate::blend_cmds::edit_fillet(&p, fillet)?,
        FeatureKind::OffsetFace(offset) => crate::face_cmds::edit_offset(&p, offset)?,
        FeatureKind::DeleteFace(deletion) => crate::face_cmds::edit_delete(&p, deletion)?,
        FeatureKind::MoveFace(moving) => crate::face_cmds::edit_move(&p, moving)?,
        FeatureKind::Thicken(layer) => crate::face_cmds::edit_thicken(&p, layer)?,
        FeatureKind::Developable(sheet) => {
            let doc = session.document()?.history.current();
            crate::develop_cmds::edit_developable(doc, &p, sheet)?;
        }
        FeatureKind::SurfaceNetwork(sheet) => {
            let doc = session.document()?.history.current();
            crate::surface_cmds::edit(doc, &p, sheet)?;
        }
        FeatureKind::Unfold(pattern) => {
            if let Some(value) = p.get("faces") {
                pattern.faces =
                    crate::face_cmds::faces_value("feature.edit", "faces", value, "unfold", false)?;
            }
        }
        FeatureKind::Plate(made) => {
            let doc = session.document()?.history.current();
            crate::plate_cmds::edit_plate(doc, &p, made)?;
        }
        FeatureKind::Draft(tilted) => {
            crate::face_cmds::edit_draft(&p, tilted)?;
            tilted.neutral = new_neutral.unwrap_or_else(|| tilted.neutral.clone());
        }
        FeatureKind::Shell(shell) => {
            shell.source = new_source.unwrap_or(shell.source);
            crate::face_cmds::edit_shell(&p, shell)?;
        }
        _ => {}
    }
    crate::blend_cmds::of_one_body("feature.edit", session, &kind)?;
    if let (FeatureKind::Extrude(extrusion), Some(end)) = (&mut kind, new_end) {
        extrusion.end = end;
    }
    if let FeatureKind::Extrude(extrusion) = &kind {
        symmetric_fits(&extrusion.end, extrusion.symmetric, "feature.edit")?;
    }
    let before = failing(session)?;
    // A feature inserted at the rollback marker re-points the features
    // after it as it is edited: followed against the features before it.
    let inserted = match session.document()?.history.current().inserted(id) {
        Some(entry) if !entry.repointed.is_empty() => Some(regen_before(session, id)?),
        _ => None,
    };
    let (restored, kept) = edit(session, "feature.edit", "Edit feature", |doc| {
        doc.feature_mut(id)?.kind = kind;
        match &inserted {
            Some(earlier) => repoint_edited(doc, id, earlier),
            None => Ok((None, None)),
        }
    })?;
    let mut result = feature_result(session, id)?;
    if let Some(map) = result.as_object_mut() {
        if let Some(restored) = restored {
            map.insert("restored".into(), restored);
        }
        if let Some(kept) = kept {
            map.insert("notRestored".into(), kept);
        }
    }
    add_failed(session, &before, Some(id), &mut result)?;
    Ok(result)
}

/// Where a parameter of a feature lives.
enum Slot<'a> {
    One(&'a mut Expr),
    /// A parameter a feature may be without: null takes it away.
    Maybe(&'a mut Option<Expr>),
    Triple(&'a mut [Expr; 3]),
    Flag(&'a mut bool),
    /// A parameter that is not one field: handled where the kind is known.
    Elsewhere,
}

fn feature_rename(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.rename", params, &["feature", "name"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.rename",
        "feature",
    )?;
    let name = p.required_str("name")?;
    edit(session, "feature.rename", "Rename feature", |doc| {
        // What assertions name by name is named by id first, so they keep
        // naming the feature.
        crate::assert_cmds::assertions_by_id(doc);
        doc.rename_feature(id, name)
    })?;
    feature_result(session, id)
}

fn feature_suppress(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.suppress", params, &["feature", "suppressed"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.suppress",
        "feature",
    )?;
    let suppressed = p.bool("suppressed")?.unwrap_or(true);
    let label = if suppressed {
        "Suppress feature"
    } else {
        "Unsuppress feature"
    };
    let before = failing(session)?;
    edit(session, "feature.suppress", label, |doc| {
        doc.feature_mut(id)?.suppressed = suppressed;
        Ok(())
    })?;
    let mut result = feature_result(session, id)?;
    add_failed(session, &before, Some(id), &mut result)?;
    Ok(result)
}

fn feature_move(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.move", params, &["feature", "position"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.move",
        "feature",
    )?;
    let position = p
        .integer("position", 0, u64::from(u32::MAX))?
        .ok_or_else(|| p.missing("position"))?;
    // Bounded by the check above.
    let before = failing(session)?;
    edit(session, "feature.move", "Reorder feature", |doc| {
        doc.move_feature(id, position as usize)
    })?;
    let mut result = feature_result(session, id)?;
    add_failed(session, &before, Some(id), &mut result)?;
    Ok(result)
}

fn feature_delete(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.delete", params, &["feature"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.delete",
        "feature",
    )?;
    let before = failing_unless_deferred(session)?;
    let (removed, given, kept) = edit(session, "feature.delete", "Delete feature", |doc| {
        // What inserting it at the marker re-pointed is given back first
        // (`remove_feature` would too, without saying what).
        let restored = doc.restore_repointed(id, None);
        let given = restored_json(doc, &restored.given);
        let kept = kept_json(doc, &restored.kept);
        Ok((doc.remove_feature(id)?, given, kept))
    })?;
    let mut result = json!({ "deleted": feature_json(&removed) });
    if let Some(map) = result.as_object_mut() {
        if let Some(given) = given {
            map.insert("restored".into(), given);
        }
        if let Some(kept) = kept {
            map.insert("notRestored".into(), kept);
        }
    }
    add_failed(session, &before, None, &mut result)?;
    Ok(result)
}

fn history_rollback(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("history.rollback", params, &["feature"])?;
    if !p.has("feature") {
        return Err(p.missing("feature"));
    }
    let target = match p.get("feature") {
        None => None,
        Some(_) => Some(feature_ref(
            session.document()?.history.current(),
            &p,
            "history.rollback",
            "feature",
        )?),
    };
    let before = failing_unless_deferred(session)?;
    edit(session, "history.rollback", "Roll back", |doc| {
        doc.set_rollback(target)
    })?;
    let mut result = json!({ "rollback": target.map(|id| id.to_string()) });
    add_failed(session, &before, None, &mut result)?;
    Ok(result)
}

// --------------------------------------------------------------- document

fn document_inspect(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("document.inspect", params, &["measures", "named"])?;
    let measures = p.bool("measures")?.unwrap_or(true);
    let words = p.bool("named")?.unwrap_or(false);
    let index = session.active;
    let open = session.document_mut()?;
    let regen = open.regen();
    // An assembly is described by the part that shows its placed bodies,
    // one feature per body named after its instance (`asm.rs`).
    let doc = &open.bodies_doc();
    let unit = length_unit(doc);

    let variables: Vec<Value> = regen
        .variables
        .iter()
        .map(|v| {
            let mut entry = variable_json(&regen, &v.name, unit);
            // A public parameter: how a form shows it (param.list has it
            // evaluated).
            if let (Some(map), Some(public)) = (
                entry.as_object_mut(),
                doc.variable(&v.name).and_then(|d| d.public.as_ref()),
            ) {
                map.insert(
                    "public".into(),
                    serde_json::to_value(public).unwrap_or(Value::Null),
                );
            }
            entry
        })
        .collect();
    let mut features = Vec::new();
    let mut bodies = Vec::new();
    // Measured only when the measures are asked for: first those with a
    // material together with their inertia (each body's faces integrated
    // once), then the box's tolerance, from the area of every body.
    if measures {
        crate::appearance_cmds::measure_all_ahead(
            doc,
            &regen,
            doc.features.iter().filter_map(|feature| {
                let body = regen.feature(feature.id).and_then(|o| o.own_body())?;
                Some((feature.id, body))
            }),
        );
    }
    let bbox_tess = measures.then(|| TessParams {
        chordal_tolerance: automatic_tolerance(&regen),
        ..TessParams::default()
    });
    let values = crate::appearance::Values::new(doc);
    for feature in &doc.features {
        let outcome = regen.feature(feature.id);
        let (status, error) = outcome.map_or(("unknown", None), |o| status_json(&o.status));
        let mut entry = json!({
            "id": feature.id.to_string(),
            "name": feature.name,
            "suppressed": feature.suppressed,
            "status": status,
            "params": serde_json::to_value(&feature.kind).unwrap_or(Value::Null),
        });
        // The parameters in the commands' words beside the document's own.
        if words
            && let Some(map) = entry.as_object_mut()
            && let Some(params) = map.get("params")
        {
            // Only where they differ: most features say the same in both.
            let named = in_command_words(params, doc, 0);
            if named != *params {
                map.insert("named".into(), named);
            }
        }
        if let (Some(map), Some(error)) = (entry.as_object_mut(), error) {
            map.insert("error".into(), error.into());
        }
        if let (Some(map), Some(outcome)) = (entry.as_object_mut(), outcome) {
            add_switch(map, doc, &regen.features, feature, outcome);
        }
        // Hidden in the view (`feature.visible`): said only when it is.
        if let Some(map) = entry.as_object_mut()
            && doc.is_hidden(feature.id)
        {
            map.insert("hidden".into(), true.into());
        }
        // Failed because something it is made from failed: the feature at
        // the start of that chain, the one to fix.
        if let (Some(map), Some(root)) = (
            entry.as_object_mut(),
            craft3d_regen::root_cause(doc, &regen.features, feature.id),
        ) {
            map.insert("causedBy".into(), root.to_string().into());
        }
        // A sketch: how far it is constrained. sketch.inspect has the rest.
        if let (Some(map), Some(solved)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.sketch.as_ref()),
        ) {
            map.insert("dof".into(), solved.dof.map_or(Value::Null, Value::from));
        }
        // Reference geometry: where it lies.
        if let (Some(map), Some(datum)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.datum.as_ref()),
        ) {
            map.insert("datum".into(), crate::datum_cmds::datum_json(datum));
        }
        // A sweep: how its path was followed.
        if let (Some(map), Some(report)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.sweep.as_ref()),
        ) {
            map.insert("path".into(), crate::sweep_path_cmds::report_json(report));
        }
        // A thread: what it is.
        if let (Some(map), Some(report)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.thread.as_ref()),
        ) {
            map.insert("thread".into(), crate::helix_cmds::thread_json(report));
        }
        // An offset of the profiles of a sketch: its outline.
        if let (Some(map), Some(outcome)) = (entry.as_object_mut(), outcome)
            && let Some(made) = &outcome.offset
        {
            map.insert("offset".into(), crate::offset_cmds::offset_json(made));
        }
        // Faces or edges given by the names of an earlier body, followed
        // to the newest one: the parameters as the feature was built.
        if let (Some(map), Some(built)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.followed.as_ref()),
        ) {
            map.insert(
                "followed".into(),
                serde_json::to_value(&built.kind).unwrap_or(Value::Null),
            );
        }
        // A body that went into a later feature is no longer listed under
        // bodies; this says where it went.
        if let (Some(map), Some(taker)) =
            (entry.as_object_mut(), outcome.and_then(|o| o.consumed_by))
        {
            // The splits of a body took it together: all of them, listed.
            let takers: Vec<String> = if craft3d_regen::taken_by_its_splits(doc, feature.id, taker)
            {
                craft3d_regen::splits_of(doc, &regen.features, feature.id)
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            } else {
                vec![taker.to_string()]
            };
            map.insert("bodyUsedBy".into(), takers.join(", ").into());
        }
        features.push(entry);
        if let Some(body) = outcome.and_then(|o| o.own_body()) {
            let mut entry = json!({
                "feature": feature.id.to_string(),
                "name": feature.name,
                "kind": match body.kind { BodyKind::Solid => "solid", BodyKind::Sheet => "sheet" },
                "valid": true,
                "faces": body.faces.len(),
                "edges": body.edges.len(),
                "vertices": body.vertices.len(),
                "shells": body.shells.len(),
            });
            if measures {
                crate::appearance_cmds::measure_ahead(doc, &regen, feature.id, body);
                if let (Some(map), Ok(mass)) = (entry.as_object_mut(), mass_of(body)) {
                    map.insert("volume".into(), mass.volume.into());
                    map.insert("area".into(), mass.area.into());
                    map.insert("centroid".into(), mass.centroid.map_or(Value::Null, vec3));
                }
            }
            if let Some(map) = entry.as_object_mut() {
                crate::appearance_cmds::physical_fields(
                    map,
                    &values,
                    &regen,
                    (feature.id, body),
                    unit,
                    measures,
                );
            }
            // What each face is called by the features that come after:
            // a name made of where the face came from, not of its index.
            // A face a boolean merged from faces of both operands answers
            // to the names of the others too: listed per face as aliases,
            // only when there are any.
            if let (Some(map), Some(names)) = (
                entry.as_object_mut(),
                outcome.and_then(|o| o.names.as_ref()),
            ) {
                let listed: Vec<String> = names.faces.iter().map(ToString::to_string).collect();
                map.insert("faceNames".into(), listed.into());
                if !names.aliases.is_empty() {
                    let aliases: Vec<Vec<String>> = (0..names.faces.len())
                        .map(|face| {
                            names
                                .aliases_of(face)
                                .iter()
                                .map(ToString::to_string)
                                .collect()
                        })
                        .collect();
                    map.insert("faceAliases".into(), json!(aliases));
                }
            }
            // From a tessellation: exact for flat-faced bodies, inside the
            // true box by at most the tolerance for curved ones.
            if let (Some(map), Some(bbox_tess)) = (entry.as_object_mut(), &bbox_tess)
                && let Some((lo, hi)) = crate::mass::mesh_box_of(body, bbox_tess)
            {
                map.insert("bbox".into(), json!([vec3(lo), vec3(hi)]));
                map.insert("bboxTolerance".into(), bbox_tess.chordal_tolerance.into());
            }
            bodies.push(entry);
        }
    }
    let face_colors: Vec<Value> = doc
        .faces
        .iter()
        .map(|entry| {
            json!({
                "feature": entry.feature.to_string(),
                "face": entry.face.to_string(),
                "color": entry.appearance.hex(),
            })
        })
        .collect();
    let lost_faces = crate::appearance_cmds::lost_faces_json(doc, &regen);
    let mut out = json!({
        "document": index,
        "path": open.path,
        "dirty": open.dirty(),
        "units": { "length": unit, "angle": match doc.units.angle {
            craft3d_doc::AngleUnit::Degree => "deg",
            craft3d_doc::AngleUnit::Radian => "rad",
        }},
        // `loosest` (the cap on any entity's own tolerance) only where the
        // document sets one: absent means 1000 times `linear`.
        "tolerance": match doc.tolerance.loosest {
            Some(loosest) => json!({ "linear": doc.tolerance.linear, "angular": doc.tolerance.angular, "loosest": loosest }),
            None => json!({ "linear": doc.tolerance.linear, "angular": doc.tolerance.angular }),
        },
        "variables": variables,
        "features": features,
        "rollback": doc.rollback.map(|id| id.to_string()),
        "bodies": bodies,
        "ok": regen.is_ok(),
        "history": {
            "undo": open.history.undo_labels().collect::<Vec<_>>(),
            "redo": open.history.redo_labels().collect::<Vec<_>>(),
        },
    });
    // The colours given to single faces, and those whose face is gone:
    // only when there are any.
    if let Some(map) = out.as_object_mut() {
        if !face_colors.is_empty() {
            map.insert("faceAppearances".into(), face_colors.into());
        }
        if !lost_faces.is_empty() {
            map.insert("lostFaces".into(), lost_faces.into());
        }
        // The named sets, as kept (set.list says what each holds now).
        if !doc.sets.is_empty() {
            let sets: Vec<Value> = doc
                .sets
                .iter()
                .map(|set| {
                    let query: Value = serde_json::from_str(&set.query).unwrap_or(Value::Null);
                    json!({
                        "name": set.name,
                        "kind": query.get("kind").cloned().unwrap_or(Value::Null),
                        "where": query.get("where").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();
            map.insert("sets".into(), sets.into());
        }
        // The features inserted at the rollback marker and the parameters
        // each re-pointed, with what they named before: only when there
        // are any.
        if !doc.inserted.is_empty() {
            let inserted: Vec<Value> = doc
                .inserted
                .iter()
                .map(|entry| {
                    let repointed: Vec<Value> = entry
                        .repointed
                        .iter()
                        .map(|r| json!({"feature": r.feature.to_string(), "param": r.param, "was": r.was.to_string()}))
                        .collect();
                    json!({"feature": entry.feature.to_string(), "repointed": repointed})
                })
                .collect();
            map.insert("inserted".into(), inserted.into());
        }
        // What a gallery shows of it (document.info): only when set.
        if !doc.info.is_empty() {
            map.insert(
                "info".into(),
                serde_json::to_value(&doc.info).unwrap_or(Value::Null),
            );
        }
    }
    // The assertions and how each stands: only when there are any; the
    // outputs as they are computed, likewise.
    let (states, outputs): (Vec<crate::assert_cmds::Checked>, Vec<Value>) = match session
        .checks_and_outputs(crate::assert_check::Wait::For(
            crate::assert_check::EDIT_WAIT,
        )) {
        Ok((checked, resolved)) => (
            checked,
            crate::output_cmds::states(session.document()?.history.current(), &resolved)
                .iter()
                .map(crate::output_cmds::state_json)
                .collect(),
        ),
        Err(_) => (Vec::new(), Vec::new()),
    };
    if let (false, Some(map)) = (outputs.is_empty(), out.as_object_mut()) {
        map.insert("outputs".into(), outputs.into());
    }
    if let (false, Some(map)) = (states.is_empty(), out.as_object_mut()) {
        let doc = session.document()?.history.current();
        let listed: Vec<Value> = states
            .iter()
            .map(|c| {
                let a = &c.state;
                // Its condition, as assert.list gives it.
                let when = doc
                    .assertion(&a.name)
                    .and_then(|stored| stored.when.as_ref())
                    .map(|w| w.source().to_string());
                // What was measured as assert.list gives it: the number
                // (or yes or no) and its unit apart, not one line of text.
                json!({
                    "name": a.name, "measure": a.measure, "status": a.status.name(),
                    "measured": c.value, "unit": c.unit,
                    "expected": a.expected, "message": a.message,
                    "when": when,
                })
            })
            .collect();
        map.insert("assertions".into(), listed.into());
    }
    Ok(out)
}

fn document_measure(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("document.measure", params, &["feature", "bound"])?;
    let with_bound = p.bool("bound")?.unwrap_or(false);
    let only = match p.get("feature") {
        None => None,
        Some(_) => Some(feature_ref(
            session.document()?.history.current(),
            &p,
            "document.measure",
            "feature",
        )?),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let values = crate::appearance::Values::new(doc);
    let unit = length_unit(doc);
    let mut bodies = Vec::new();
    let (mut volume, mut area, mut moment) = (0.0, 0.0, Vec3::ZERO);
    let (mut volume_bound, mut area_bound) = (0.0, 0.0);
    // The total mass, while every body has one.
    let mut total_mass = Some(0.0);
    // With the bound each body is integrated once, for its measures and
    // their bound together; without, measured ahead and remembered.
    if !with_bound {
        crate::appearance_cmds::measure_all_ahead(
            doc,
            &regen,
            regen
                .bodies()
                .filter(|(id, _)| only.is_none_or(|o| o == *id)),
        );
    }
    for (id, body) in regen
        .bodies()
        .filter(|(id, _)| only.is_none_or(|o| o == *id))
    {
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let failed = |e: craft3d_topo::MassError| {
            CommandError::new(format!("document.measure: {name}: {e}"))
        };
        let (mass, bound) = if with_bound {
            let (mass, bound) = craft3d_topo::mass_bound(body).map_err(failed)?;
            (mass, Some(bound))
        } else {
            crate::appearance_cmds::measure_ahead(doc, &regen, id, body);
            (mass_of(body).map_err(failed)?, None)
        };
        volume += mass.volume;
        area += mass.area;
        if let Some(c) = mass.centroid {
            moment += c * mass.volume;
        }
        let mut entry = json!({
            "feature": id.to_string(), "name": name,
            "volume": mass.volume, "area": mass.area, "centroid": mass.centroid.map_or(Value::Null, vec3),
        });
        if let Some(map) = entry.as_object_mut() {
            if let Some(bound) = bound {
                volume_bound += bound.volume;
                area_bound += bound.area;
                map.insert("volumeBound".into(), bound.volume.into());
                map.insert("areaBound".into(), bound.area.into());
            }
            crate::appearance_cmds::physical_fields(map, &values, &regen, (id, body), unit, true);
            total_mass = match (total_mass, map.get("mass").and_then(Value::as_f64)) {
                (Some(sum), Some(m)) => Some(sum + m),
                _ => None,
            };
        }
        bodies.push(entry);
    }
    if let (Some(id), true) = (only, bodies.is_empty()) {
        let status = regen
            .feature(id)
            .map_or("unknown", |o| status_json(&o.status).0);
        return fail(format!(
            "document.measure: feature {id} has no body (its status is {status})"
        ));
    }
    let mut result = json!({
        "bodies": bodies,
        "volume": volume,
        "area": area,
        // Bodies are summed as they are: overlapping bodies count twice.
        "centroid": if volume != 0.0 { vec3(moment * (1.0 / volume)) } else { Value::Null },
        "lengthUnit": unit,
        "areaUnit": format!("{unit}^2"),
        "volumeUnit": format!("{unit}^3"),
    });
    if let (Some(map), true) = (result.as_object_mut(), with_bound) {
        map.insert("volumeBound".into(), volume_bound.into());
        map.insert("areaBound".into(), area_bound.into());
    }
    // The total mass when every body has a material.
    if let (Some(map), Some(mass), false) = (result.as_object_mut(), total_mass, bodies.is_empty())
    {
        map.insert("mass".into(), mass.into());
        map.insert("massUnit".into(), "kg".into());
    }
    Ok(result)
}

fn document_check(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("document.check", params, &["deep", "feature"])?;
    let deep = p.bool("deep")?.unwrap_or(false);
    let only = match p.get("feature") {
        None => None,
        Some(_) => Some(feature_ref(
            session.document()?.history.current(),
            &p,
            "document.check",
            "feature",
        )?),
    };
    check_document(session, deep, only, true)
}

/// `document.check` of the active document: of every feature, or of
/// `only`. Without `measure` the bodies are not measured (their mass
/// properties integrated from the exact faces, which shows an unbounded
/// or otherwise unmeasurable face): `document.sample` measures at the
/// defaults and, with `deep`, at every point, as measuring costs several
/// times a regeneration.
pub(crate) fn check_document(
    session: &mut Session,
    deep: bool,
    only: Option<craft3d_doc::FeatureId>,
    measure: bool,
) -> Run {
    let measure = measure || deep;
    let open = session.document_mut()?;
    // A deep check holds every boolean to its operands, also those the
    // cache let go of.
    let regen = open.regen_needing(if deep {
        craft3d_regen::Need::All
    } else {
        craft3d_regen::Need::Bodies
    });
    let doc = open.history.current();
    let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| {
            CommandError::new("document.check: the document's tolerance is not positive and finite")
        })?;
    let mut problems = Vec::new();
    let mut loose: Vec<Value> = Vec::new();
    for variable in regen.variables.iter().filter(|_| only.is_none()) {
        if let Err(error) = &variable.value {
            problems.push(json!({ "variable": variable.name, "problem": error }));
        }
    }
    // A public parameter out of its range or not of its kind (variable.set
    // does not check them; param.set does).
    if only.is_none() {
        for (name, problem) in crate::param_cmds::parameter_problems(doc) {
            problems.push(json!({ "parameter": name, "problem": problem }));
        }
        // A colour or a density that follows the variables and does not
        // evaluate (the body is drawn or weighed without it).
        if !doc.bodies.is_empty() {
            let values = crate::appearance::evaluation(doc);
            for (feature, problem) in crate::appearance::property_errors(doc, values.as_ref()) {
                let name = doc
                    .feature(feature)
                    .map_or_else(|| feature.to_string(), |f| f.name.clone());
                problems.push(json!({
                    "feature": feature.to_string(), "name": name, "problem": problem,
                }));
            }
        }
    }
    let mut checked = 0;
    let mut marginal = 0_usize;
    let mut booleans_checked = 0_usize;
    let mut loose_meshes = 0_usize;
    let mut status = None;
    for outcome in &regen.features {
        if only.is_some_and(|id| id != outcome.id) {
            continue;
        }
        let name = doc
            .feature(outcome.id)
            .map_or_else(|| outcome.id.to_string(), |f| f.name.clone());
        if only.is_some() {
            // What was asked about is not built: say so rather than "ok,
            // no bodies".
            // One switched off is not built, also when it passes a body on
            // (that body is the one of the feature that made it, and is
            // checked with that feature).
            let off = match &outcome.status {
                Status::Suppressed => Some("is suppressed"),
                Status::Inactive => Some("is inactive (its condition is 0)"),
                Status::RolledBack => Some("is after the rollback marker"),
                Status::Ok | Status::Failed(_) => None,
            };
            let passes = match &outcome.passed {
                Some(craft3d_regen::Passed::Body(from)) => {
                    let by = doc
                        .feature(*from)
                        .map_or_else(|| from.to_string(), |f| f.name.clone());
                    format!(
                        "; it passes the body of `{by}` ({from}) on unchanged, which is checked with the feature that made it"
                    )
                }
                _ => String::new(),
            };
            let not_built = match (off, &outcome.passed) {
                (Some(why), _) => Some(format!("{why}: nothing is built to check{passes}")),
                (None, Some(craft3d_regen::Passed::Nothing(why))) => Some(format!(
                    "passes no body on ({why}): nothing is built to check"
                )),
                (None, _) => None,
            };
            if let Some(why) = not_built {
                problems.push(json!({
                    "feature": outcome.id.to_string(), "name": name,
                    "problem": format!("the feature {why}"),
                }));
            }
            status = Some(match &outcome.status {
                Status::Ok => "ok",
                Status::Failed(_) => "failed",
                Status::Suppressed => "suppressed",
                Status::Inactive => "inactive",
                Status::RolledBack => "rolledBack",
            });
        }
        if let Status::Failed(reason) = &outcome.status {
            let mut problem =
                json!({ "feature": outcome.id.to_string(), "name": name, "problem": reason });
            if let (Some(map), Some(root)) = (
                problem.as_object_mut(),
                craft3d_regen::root_cause(doc, &regen.features, outcome.id),
            ) {
                map.insert("causedBy".into(), root.to_string().into());
            }
            problems.push(problem);
        }
        // Every boolean of the history, its body taken by a later feature
        // or not.
        if deep && outcome.status == Status::Ok {
            booleans_checked +=
                boolean_check(doc, &regen, outcome, &name, &tolerance, &mut problems);
        }
        // Asked about one feature that is off: what it passes on is not
        // its own, and is not checked here.
        let passed_only = only.is_some() && outcome.is_off();
        if let Some(body) = outcome.own_body().filter(|_| !passed_only) {
            checked += 1;
            // Entities with a tolerance of their own (a boolean on loose
            // operands, an import) are said, not hidden.
            if body.is_tolerant() {
                loose.push(json!({
                    "feature": outcome.id.to_string(), "name": name,
                    "edges": body.tolerances.edges().count(),
                    "vertices": body.tolerances.vertices().count(),
                    "largest": body.working_tolerance(doc.tolerance.linear),
                }));
            }
            for defect in
                craft3d_topo::check_capped(body, &tolerance, doc.tolerance.loosest_linear())
            {
                problems.push(json!({
                    "feature": outcome.id.to_string(), "name": name, "problem": format!("invalid body: {defect}"),
                }));
            }
            // An unbounded or otherwise unmeasurable face shows up here.
            if measure {
                crate::appearance_cmds::measure_ahead(doc, &regen, outcome.id, body);
            }
            if measure && let Err(error) = mass_of(body) {
                problems.push(json!({
                    "feature": outcome.id.to_string(), "name": name, "problem": format!("cannot be measured: {error}"),
                }));
            }
            // Several shells: the outsides of lumps and their cavities,
            // nested as they must be (craft3d_kernel::lumps).
            // A body that is not lumps and cavities is a problem; one whose
            // shells cannot be sorted (a shell that cannot be measured,
            // points of a shell that disagree or that no ray can place) is
            // a doubt, counted under marginal. A volume that cannot be told
            // from zero is no doubt: where the shell lies decides.
            if deep && body.kind == BodyKind::Solid && body.shells.len() > 1 {
                let volumes = crate::mass::shells_of(body);
                match craft3d_kernel::lumps::lumps_measured(body, &tolerance, &volumes) {
                    Ok(_) => {}
                    Err(error @ craft3d_kernel::KernelError::InvalidBody { .. }) => {
                        problems.push(json!({
                            "feature": outcome.id.to_string(), "name": name,
                            "problem": format!("its shells are not lumps and cavities: {error}"),
                        }));
                    }
                    Err(_) => marginal = marginal.saturating_add(1),
                }
            }
            // A fold folded flat lies on itself by design, in one plane:
            // whether it passes through itself is the order of its layers,
            // which it was built with (docs/design/origami.md).
            let layered = matches!(
                outcome.developed,
                Some(craft3d_regen::Developed::FlatFold(_))
            );
            if deep {
                if !layered {
                    let params = craft3d_tess::SelfCheckParams::new(doc.tolerance.linear);
                    match craft3d_tess::body_self_intersections(body, &params) {
                        Ok(found) => {
                            marginal = marginal
                                .saturating_add(found.marginal_crossings)
                                .saturating_add(
                                    found.shells.iter().filter(|s| !s.certain).count(),
                                );
                            if !found.is_clean() {
                                problems.push(json!({
                                    "feature": outcome.id.to_string(), "name": name,
                                    "problem": format!("the body passes through or touches itself: {found}"),
                                }));
                            }
                        }
                        Err(error) => problems.push(json!({
                            "feature": outcome.id.to_string(), "name": name,
                            "problem": format!("cannot be searched for self-intersections: {error}"),
                        })),
                    }
                }
                // A body with loose entities: its mesh's boundary points
                // held to the tolerance of the edge or vertex they lie on
                // (`craft3d_tess::off_surfaces`), every other point to
                // rounding.
                if body.is_tolerant() {
                    match loose_mesh_problem(body, doc.tolerance.linear) {
                        Ok(found) => {
                            loose_meshes += 1;
                            if let Some(problem) = found {
                                problems.push(json!({
                                    "feature": outcome.id.to_string(), "name": name,
                                    "problem": problem,
                                }));
                            }
                        }
                        Err(_) => marginal = marginal.saturating_add(1),
                    }
                }
                // Two shells that touch, a vertex or the middle of an edge
                // of one on a face of the other: zero thickness, the rule
                // regeneration holds every body to
                // (craft3d_kernel::boolean::parts_touch); the mesh above
                // does not see it.
                match craft3d_kernel::boolean::parts_touch(body, &tolerance) {
                    Ok(Some(touch)) => {
                        let [a, b] = touch.faces;
                        problems.push(json!({
                            "feature": outcome.id.to_string(), "name": name,
                            "problem": format!(
                                "the body has parts that touch at ({:e}, {:e}, {:e}): faces {} and {} (zero thickness)",
                                touch.at.x, touch.at.y, touch.at.z, a.0, b.0
                            ),
                        }));
                    }
                    Ok(None) => {}
                    Err(_) => marginal = marginal.saturating_add(1),
                }
            }
        }
    }
    // Lines shrunk to a point under directions (a contradiction the
    // solver reports as holding) and sketches with freedom left.
    let mut sketch_warnings = Vec::new();
    crate::sketch_warnings::check_sketches(doc, &regen, only, &mut problems, &mut sketch_warnings);
    // Angles written as plain numbers with pi in them: degrees there.
    sketch_warnings.extend(crate::angle_warnings::plain_angles(doc, only));
    // A face colour whose face is gone colours nothing: a warning, never a
    // problem (the bodies are what they are; R1 item 57).
    if only.is_none() {
        for lost in crate::appearance_cmds::lost_faces_json(doc, &regen) {
            sketch_warnings.push(json!({
                "kind": "lostFace",
                "feature": lost.get("feature"),
                "face": lost.get("face"),
                "warning": lost.get("notice"),
            }));
        }
    }
    // Broken assertions are problems of the design, like failed
    // features (only when the whole document is checked); those that are
    // not proven either way are warnings.
    let assertions = if only.is_none() {
        crate::assert_cmds::summary(session, crate::assert_check::Wait::All)
    } else {
        None
    };
    let mut warnings: Vec<Value> = assertions
        .as_ref()
        .and_then(|s| s.get("unproven"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|b| json!({"kind": "assertion", "assertion": b.get("name"), "warning": b.get("message")}))
                .collect()
        })
        .unwrap_or_default();
    warnings.extend(sketch_warnings);
    if let Some(broken) = assertions
        .as_ref()
        .and_then(|s| s.get("broken"))
        .and_then(Value::as_array)
    {
        for b in broken {
            problems.push(json!({
                "assertion": b.get("name"),
                "problem": b.get("message"),
                "measured": b.get("measured"),
                "expected": b.get("expected"),
            }));
        }
    }
    let mut result =
        json!({ "ok": problems.is_empty(), "bodiesChecked": checked, "problems": problems });
    if let (Some(s), Some(map)) = (assertions, result.as_object_mut()) {
        map.insert("assertions".into(), s);
    }
    if let (false, Some(map)) = (warnings.is_empty(), result.as_object_mut()) {
        map.insert("warnings".into(), warnings.into());
    }
    if let (false, Some(map)) = (loose.is_empty(), result.as_object_mut()) {
        map.insert("loose".into(), loose.into());
    }
    if let (Some(status), Some(map)) = (status, result.as_object_mut()) {
        map.insert("status".into(), status.into());
    }
    if let (true, Some(map)) = (deep, result.as_object_mut()) {
        map.insert("deep".into(), true.into());
        map.insert("marginal".into(), marginal.into());
        map.insert("booleansChecked".into(), booleans_checked.into());
        map.insert("looseMeshesChecked".into(), loose_meshes.into());
    }
    Ok(result)
}

/// Keys whose integer values are features in a feature's parameters.
const FEATURE_KEYS: &[&str] = &[
    "target", "tool", "sketch", "source", "path", "plane", "feature",
];

/// A feature's parameters as `document.inspect` lists them (`params`,
/// the document's own form) in the words of the commands (`named`): keys
/// in camelCase as the commands spell them (`keepTool`, not `keep_tool`),
/// and a feature referred to by its name instead of its number
/// (`"target": "Plate"`, not `"target": 1`). The same structure
/// otherwise; `params` stays as it was for the agents that read it.
fn in_command_words(value: &Value, doc: &Document, depth: usize) -> Value {
    // Our own structures nest a few levels; deeper is cut off, not
    // followed.
    if depth > 32 {
        return Value::Null;
    }
    let camel = |key: &str| -> String {
        let mut out = String::with_capacity(key.len());
        let mut up = false;
        for c in key.chars() {
            if c == '_' {
                up = true;
            } else if up {
                out.extend(c.to_uppercase());
                up = false;
            } else {
                out.push(c);
            }
        }
        out
    };
    let name_of = |v: &Value| -> Option<Value> {
        let id = v.as_u64()?;
        doc.feature(FeatureId(id))
            .map(|f| Value::from(f.name.clone()))
    };
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, v)| {
                    let key = camel(key);
                    let v = if FEATURE_KEYS.contains(&key.as_str()) {
                        match v {
                            Value::Array(items) => Value::Array(
                                items
                                    .iter()
                                    .map(|item| name_of(item).unwrap_or_else(|| item.clone()))
                                    .collect(),
                            ),
                            other => name_of(other)
                                .unwrap_or_else(|| in_command_words(other, doc, depth + 1)),
                        }
                    } else {
                        in_command_words(v, doc, depth + 1)
                    };
                    (key, v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| in_command_words(v, doc, depth + 1))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The mesh check of a body with loose entities (`document.check` deep):
/// the body meshed at a thousandth of its size, every point of a face's
/// mesh within rounding of the face's surface, or, on the face's
/// boundary, within the tolerance of the edge or vertex it lies on
/// (`craft3d_tess::off_surfaces`). Rounding is a billionth of the body's
/// size, at least a hundredth of the document's tolerance. `Ok(None)`
/// when every point is where it may be, the problem when not; `Err` when
/// the body cannot be meshed or has no size.
pub(crate) fn loose_mesh_problem(
    body: &craft3d_topo::Body,
    linear: f64,
) -> Result<Option<String>, String> {
    let size = body.bbox().map_err(|e| e.to_string())?.diagonal();
    if !(size.is_finite() && size > 0.0) {
        return Err("the body has no size".into());
    }
    let params = TessParams {
        chordal_tolerance: (1e-3 * size).max(linear),
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).map_err(|e| e.to_string())?;
    let rounding = (1e-9 * size).max(0.01 * linear);
    let off = craft3d_tess::off_surfaces(body, &mesh, linear, rounding);
    let Some(worst) = off
        .iter()
        .max_by(|a, b| (a.distance - a.allowed).total_cmp(&(b.distance - b.allowed)))
    else {
        return Ok(None);
    };
    Ok(Some(format!(
        "its mesh has {} points off their faces' surfaces by more than the tolerance of the edge or vertex they lie on (the worst {:e} off face {}, {:e} allowed)",
        off.len(),
        worst.distance,
        worst.face.0,
        worst.allowed
    )))
}

/// `document.check` deep for a boolean feature: its body held to what
/// every exact boolean of its target and tool keeps
/// (`craft3d_kernel::boolean::verify` in full: volumes, the result's
/// reach, the faces far from the other operand, seeded points). Each
/// violation is a problem of the feature, naming the operand's face where
/// it is about one. 1 when the feature was checked, 0 when it is not a
/// boolean or has no body.
fn boolean_check(
    doc: &craft3d_doc::Document,
    regen: &Regen,
    outcome: &craft3d_regen::FeatureResult,
    name: &str,
    tolerance: &craft3d_math::Tolerance,
    problems: &mut Vec<Value>,
) -> usize {
    use craft3d_kernel::boolean::{BooleanOp, Operand, VerifyOptions, Violation, verify};
    let Some(feature) = doc.feature(outcome.id) else {
        return 0;
    };
    // As it was built: with its names followed, where they were.
    let kind = outcome
        .followed
        .as_deref()
        .map_or(&feature.kind, |f| &f.kind);
    let craft3d_doc::FeatureKind::Boolean(boolean) = kind else {
        return 0;
    };
    let source = |id| {
        regen
            .features
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.body.as_ref().map(|body| (body, r.names.as_deref())))
    };
    let (Some(result), Some((a, a_names)), Some((b, b_names))) = (
        outcome.body.as_ref(),
        source(boolean.target),
        source(boolean.tool),
    ) else {
        return 0;
    };
    let op = match boolean.op {
        craft3d_doc::BooleanOp::Union => BooleanOp::Union,
        craft3d_doc::BooleanOp::Subtract => BooleanOp::Subtract,
        craft3d_doc::BooleanOp::Intersect => BooleanOp::Intersect,
    };
    let found = match verify(a, b, op, result, tolerance, &VerifyOptions::default()) {
        Ok(found) => found,
        Err(error) => {
            problems.push(json!({
                "feature": outcome.id.to_string(), "name": name,
                "problem": format!("the boolean check cannot run: {error}"),
            }));
            return 1;
        }
    };
    for violation in &found.violations {
        let mut problem = json!({
            "feature": outcome.id.to_string(), "name": name,
            "problem": format!("not the {} of its target and tool: {violation}", boolean.op.name()),
        });
        if let (Violation::FarFace { operand, face, .. }, Some(map)) =
            (violation, problem.as_object_mut())
        {
            let names = match operand {
                Operand::A => a_names,
                Operand::B => b_names,
            };
            if let Some(face_name) = names.and_then(|n| n.faces.get(face.0 as usize)) {
                map.insert("face".into(), face_name.to_string().into());
            }
        }
        problems.push(problem);
    }
    1
}

// ------------------------------------------------------------------- view

/// The `box` of `view.render`: two opposite corners, in any order.
fn view_box(value: &Value, command: &str) -> Result<craft3d_math::Bbox3, CommandError> {
    const SHAPE: &str = "`box` must be two opposite corners, [[x, y, z], [x, y, z]], \
                         of numbers within ±1e12 that are not the same point";
    let corner = |value: &Value| -> Option<Vec3> {
        let [x, y, z] = value.as_array()?.as_slice() else {
            return None;
        };
        let number = |v: &Value| v.as_f64().filter(|n| n.is_finite() && n.abs() <= 1e12);
        Some(Vec3::new(number(x)?, number(y)?, number(z)?))
    };
    let corners = value
        .as_array()
        .and_then(|corners| match corners.as_slice() {
            [a, b] => Some((corner(a)?, corner(b)?)),
            _ => None,
        });
    match corners {
        Some((a, b)) if a != b => Ok(craft3d_math::Bbox3::from_points([a, b])),
        _ => fail(format!("{command}: {SHAPE}")),
    }
}

/// How `view.render` looks at the model: the parameters `view`,
/// `projection`, `zoom` and `box`, shared with `view.pick`, which answers
/// what is at a position of the picture these parameters give.
pub(crate) struct Framing {
    command: &'static str,
    pub view: &'static str,
    toward_eye: Vec3,
    up: Vec3,
    pub orthographic: bool,
    pub zoom: f64,
    pub look_at: Option<craft3d_math::Bbox3>,
}

impl Framing {
    /// The keys of the parameters it reads.
    pub const KEYS: [&'static str; 4] = ["view", "projection", "zoom", "box"];

    pub fn from_params(p: &Params, command: &'static str) -> Result<Self, CommandError> {
        const VIEWS: [(&str, Vec3, Vec3); 7] = [
            ("iso", Vec3::new(1.0, -1.0, 0.8), Vec3::Z),
            ("front", Vec3::new(0.0, -1.0, 0.0), Vec3::Z),
            ("back", Vec3::new(0.0, 1.0, 0.0), Vec3::Z),
            ("right", Vec3::new(1.0, 0.0, 0.0), Vec3::Z),
            ("left", Vec3::new(-1.0, 0.0, 0.0), Vec3::Z),
            ("top", Vec3::new(0.0, 0.0, 1.0), Vec3::Y),
            ("bottom", Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        ];
        let name = p.str("view")?.unwrap_or("iso");
        let Some(&(view, toward_eye, up)) = VIEWS.iter().find(|(view, ..)| *view == name) else {
            return fail(format!(
                "{command}: unknown view `{name}`; use iso, front, back, left, right, top or bottom"
            ));
        };
        let orthographic = match p.str("projection")?.unwrap_or("perspective") {
            "perspective" => false,
            "orthographic" => true,
            other => {
                return fail(format!(
                    "{command}: unknown projection `{other}`; use perspective or orthographic"
                ));
            }
        };
        let zoom = match p.number("zoom")? {
            None => 1.0,
            Some(zoom) if (0.01..=1000.0).contains(&zoom) => zoom,
            Some(_) => {
                return fail(format!(
                    "{command}: `zoom` must be a number from 0.01 to 1000"
                ));
            }
        };
        let look_at = p
            .get("box")
            .map(|value| view_box(value, command))
            .transpose()?;
        Ok(Self {
            command,
            view,
            toward_eye,
            up,
            orthographic,
            zoom,
            look_at,
        })
    }

    /// The direction from the model towards the eye, and up.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn direction(&self) -> (Vec3, Vec3) {
        (self.toward_eye, self.up)
    }

    /// The camera for a picture of `aspect` (width over height) of a model
    /// that fills `model`.
    pub fn camera(&self, model: &craft3d_math::Bbox3, aspect: f64) -> Result<Camera, CommandError> {
        let framed = self.look_at.unwrap_or(*model);
        if self.orthographic {
            Camera::framing_orthographic(&framed, self.toward_eye, self.up, aspect)
        } else {
            // A close-up is taken from outside the whole model.
            Camera::framing_clear_of(&framed, model, self.toward_eye, self.up, 30.0, aspect)
        }
        .and_then(|camera| camera.zoomed(self.zoom))
        .ok_or_else(|| {
            CommandError::new(if self.look_at.is_some() {
                format!(
                    "{}: cannot frame this `box` from this view: seen from there it has no size",
                    self.command
                )
            } else {
                format!("{}: cannot frame the model", self.command)
            })
        })
    }
}

/// The parameters of `view.render`.
const VIEW_RENDER_PARAMS: &[&str] = &[
    "path",
    "width",
    "height",
    "view",
    "projection",
    "zoom",
    "box",
    "mode",
    "section",
    "tolerance",
    "background",
    "look",
];

/// The background of `view.render` and of thumbnails by default: the
/// window's viewport in its theme, Mission (a light warm grey), so a
/// picture looks like what the window shows.
pub const VIEWPORT_BACKGROUND: [u8; 3] = [235, 233, 229];
/// The background of `view.render` with `background: "dark"`: the CPU
/// renderer's own (`RenderParams::default`).
pub const DARK_BACKGROUND: [u8; 3] = [30, 33, 40];

fn view_render(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("view.render", params, VIEW_RENDER_PARAMS)?;
    let path = p.str("path")?;
    let resolved = path
        .map(|path| session.workspace.resolve_write(path))
        .transpose()?;
    let (mut result, png) = render_picture(session, &p)?;
    let Some(map) = result.as_object_mut() else {
        return Ok(result);
    };
    match (path, resolved) {
        (Some(path), Some(resolved)) => {
            session.write_resolved(&resolved, png)?;
            map.insert("path".into(), path.into());
        }
        _ => {
            map.insert("mime".into(), "image/png".into());
            map.insert("base64".into(), base64::encode(&png).into());
        }
    }
    Ok(result)
}

/// Size of the thumbnail a Jig is saved with, in pixels.
pub(crate) const THUMBNAIL_SIZE: (u32, u32) = (400, 300);

/// The thumbnail of the active document: a PNG of the bodies from the
/// default view (iso, perspective, the whole model) at
/// [`THUMBNAIL_SIZE`], with the CPU renderer, so the same document gives
/// the same bytes on one platform and build (the renderer's maths is not
/// promised bit for bit across platforms). Kept per revision: saving an
/// unchanged document again does not draw it again.
pub(crate) fn thumbnail_png(
    session: &mut Session,
) -> Result<std::sync::Arc<Vec<u8>>, CommandError> {
    let revision = session.document()?.history.revision();
    if let Some((drawn, png)) = &session.document()?.thumbnail
        && *drawn == revision
    {
        return Ok(std::sync::Arc::clone(png));
    }
    let (width, height) = THUMBNAIL_SIZE;
    let params = json!({ "width": width, "height": height });
    let p = Params::new("view.render", &params, VIEW_RENDER_PARAMS)?;
    // The document as it is, not where the view is in its history or
    // its motion: a scrubbed view and a moment of the motion (with its
    // collisions drawn red) are set aside while the thumbnail is drawn,
    // so the picture saved into the file and kept for this revision is
    // the document's.
    let open = session.document_mut()?;
    let scrub = std::mem::take(&mut open.scrub);
    let motion = std::mem::take(&mut open.motion);
    let drawn = render_picture(session, &p);
    let open = session.document_mut()?;
    open.scrub = scrub;
    open.motion = motion;
    let png = std::sync::Arc::new(drawn?.1);
    session.document_mut()?.thumbnail = Some((revision, std::sync::Arc::clone(&png)));
    Ok(png)
}

/// The picture of `view.render` with what it says about it; its `path`
/// is left to the caller.
fn render_picture(session: &mut Session, p: &Params) -> Result<(Value, Vec<u8>), CommandError> {
    let width = p.integer("width", 16, 4096)?.unwrap_or(800);
    let height = p.integer("height", 16, 4096)?.unwrap_or(600);
    let framing = Framing::from_params(p, "view.render")?;
    let (view, orthographic, zoom, look_at) = (
        framing.view,
        framing.orthographic,
        framing.zoom,
        framing.look_at,
    );
    let ids = match p.str("mode")?.unwrap_or("shaded") {
        "shaded" => false,
        "ids" => true,
        other => {
            return fail(format!(
                "view.render: unknown mode `{other}`; use shaded or ids"
            ));
        }
    };
    // The picture's scene: at the tolerance given, or at a quarter to half
    // a pixel where the picture shows the model, never finer than the
    // display's floor, within a total budget (`Session::picture_scene`).
    // A section is resolved against it: a face's name means the face of
    // that mesh.
    let tolerance = match p.number("tolerance")? {
        Some(t) if t > 0.0 => Some(t),
        Some(_) => return fail("view.render: `tolerance` must be positive"),
        None => None,
    };
    let background = match p.str("background")?.unwrap_or("light") {
        "light" => VIEWPORT_BACKGROUND,
        "dark" => DARK_BACKGROUND,
        other => {
            let other: String = other.chars().take(40).collect();
            return fail(format!(
                "view.render: unknown background `{other}`; use light or dark"
            ));
        }
    };
    let look = crate::look::look_param("view.render", p.get("look"))?;
    let aspect = width as f64 / height as f64;
    let pixel = |model: &craft3d_math::Bbox3| {
        let camera = framing.camera(model, aspect).ok()?;
        crate::scene::pixel_footprint(&camera, height as u32, model)
    };
    let Some(scene) = session.picture_scene(tolerance, pixel)? else {
        return fail("view.render: no document is open; run file.new or file.open first");
    };
    if let Some(problem) = scene.problems.first() {
        return fail(format!("view.render: {problem}"));
    }
    if scene.bodies.is_empty() {
        return fail(
            "view.render: there are no bodies; add a feature, or check document.inspect for failed features",
        );
    }
    let section = match p.get("section") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let (spec, cap) = crate::section::section_param("view.render", value)?;
            let plane = session
                .section_plane(&scene, &spec)
                .map_err(|e| CommandError::new(format!("view.render: {e}")))?;
            Some((plane, cap))
        }
    };
    let all: &Mesh = &scene.mesh;
    let colors: &craft3d_render::FaceColors = &scene.face_colors;
    let legend: Vec<Value> = scene
        .bodies
        .iter()
        .map(|body| json!({ "body": body.name, "firstFaceId": body.first_face_id }))
        .collect();
    // Both are within 16..=4096.
    let render_params = RenderParams {
        width: width as u32,
        height: height as u32,
        background,
        section: section.map(|(plane, cap)| craft3d_render::Section {
            cap,
            ..plane.section(crate::section::SECTION_CAP_COLOR)
        }),
        ..RenderParams::default()
    };
    // Framed on the bodies' B-rep, as the tolerance was chosen.
    let model = scene.bodies_box;
    let framed = look_at.unwrap_or(model);
    let camera = framing.camera(&model, aspect)?;
    let prepared = look
        .prepared()
        .map_err(|e| CommandError::new(format!("view.render: look: {e}")))?;
    let mm_per_unit = session
        .document()?
        .history
        .current()
        .units
        .length
        .as_expr_unit()
        .factor();
    let frame = match &prepared {
        Some(env) if !ids => craft3d_render::studio::render_studio(
            all,
            colors,
            &camera,
            &render_params,
            &look.resolved(&model, mm_per_unit, None, 1),
            env,
        ),
        _ => craft3d_render::render_colored(all, colors, &camera, &render_params),
    }
    .map_err(|e| CommandError::new(format!("view.render: {e}")))?;
    let png = if ids {
        craft3d_render::encode_png(frame.width, frame.height, &frame.false_color())
    } else {
        frame.to_png()
    }
    .map_err(|e| CommandError::new(format!("view.render: {e}")))?;

    let mut result = json!({
        "width": width, "height": height, "view": view,
        "projection": if orthographic { "orthographic" } else { "perspective" },
        "zoom": zoom,
        "box": framed.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
        "triangles": all.triangles.len(),
        "coverage": frame.coverage(),
        "bodies": legend,
        "tolerance": scene.tolerance,
        "maxAngle": scene.max_angle.map(f64::to_degrees),
        "coarsened": scene.coarsened,
    });
    if look.studio
        && let Some(map) = result.as_object_mut()
    {
        map.insert("look".into(), look.json());
    }
    if let (Some((plane, _)), Some(map)) = (&section, result.as_object_mut()) {
        map.insert(
            "section".into(),
            json!({
                "point": vec3(plane.origin),
                "normal": vec3(plane.normal),
                // Where the plane passes through what is drawn: the mesh.
                "range": plane.range(&all.bbox()),
            }),
        );
    }
    Ok((result, png))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod loose_mesh_tests {
    use super::loose_mesh_problem;
    use craft3d_math::{Tolerance, Vec3};
    use craft3d_testkit::loose::{SPLIT_TOP_SIDES, exploded_from, split_top_block};

    /// A sewn body with its faces up to 300 tolerances apart: its mesh is
    /// held to the tolerances it recorded; the same body with its table
    /// cleared has boundary points off its surfaces, and says so.
    #[test]
    fn a_loose_mesh_is_held_to_its_entities_tolerances() {
        for scale in [1e-3, 1.0, 1e6] {
            let tol = Tolerance::new(1e-6 * scale, 1e-9).unwrap();
            let cap = 1000.0 * tol.linear;
            let expected = split_top_block(scale, true).unwrap();
            let whole = split_top_block(scale, false).unwrap();
            let moves: Vec<Vec3> = (0..expected.faces.len())
                .map(|f| {
                    let a = 0.7 * f as f64 + 0.3;
                    Vec3::new(a.cos(), a.sin(), (1.3 * a).cos()) * (300.0 * tol.linear * 0.5)
                })
                .collect();
            let loose = exploded_from(
                [&expected, &whole],
                |f| SPLIT_TOP_SIDES.contains(&f),
                &moves,
                None,
            )
            .unwrap();
            let sewn = craft3d_topo::sew(&loose, &tol, cap, cap).unwrap().body;
            assert!(sewn.is_tolerant());
            assert_eq!(loose_mesh_problem(&sewn, tol.linear), Ok(None));
            let mut bare = sewn.clone();
            bare.tolerances = craft3d_topo::Tolerances::default();
            let problem = loose_mesh_problem(&bare, tol.linear).unwrap().unwrap();
            assert!(problem.contains("points off their faces"), "{problem}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod export_budget_tests {
    use super::{TessParams, meshes};
    use crate::{Session, Workspace};
    use serde_json::json;

    /// The export's triangle budget holds for all bodies together, and a
    /// mesh over it is refused saying what to do.
    #[test]
    fn a_mesh_export_over_its_triangle_budget_is_refused() {
        let mut session = Session::new(Workspace::none());
        session.execute("file.new", &json!({})).unwrap();
        for k in 0..3 {
            session
                .execute(
                    "feature.block",
                    &json!({"size": [1, 1, 1], "position": [k * 3, 0, 0]}),
                )
                .unwrap();
        }
        let open = session.document_mut().unwrap();
        let tess = |cap| TessParams {
            chordal_tolerance: 0.01,
            max_triangles: Some(cap),
            ..TessParams::default()
        };
        // Three cubes of 12 triangles: 36 fit, 35 do not.
        let made = meshes(open, "file.export", &tess(36), None).unwrap();
        assert_eq!(
            made.iter().map(|m| m.mesh.triangles.len()).sum::<usize>(),
            36
        );
        let Err(error) = meshes(open, "file.export", &tess(35), None) else {
            panic!("over the budget");
        };
        assert!(
            error.message().contains("more than 35 triangles")
                && error.message().contains("coarser quality"),
            "{error}"
        );
    }
}
