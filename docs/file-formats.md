# File formats

> **Last updated:** 2026-10-13 · **Status:** `.3dcraft` (version 6), mesh export (STL, 3MF, OBJ) and STEP export work and are tested; STEP import is the plan.

| Format | Read | Write | Carries | Crates | Milestone |
|---|---|---|---|---|---|
| `.3dcraft` (native) | **yes (2026-10-12)** | **yes (2026-10-12)** | The design history: units, tolerance, variables, formulas, sketches, constraints, features, their order; imported bodies; optional caches | `format` | M2 |
| STL (binary, ASCII) | later | **yes (2026-10-12)** | Triangles only; no units, no colour | `meshio`, `io` | M3 |
| 3MF | later | **yes (2026-10-12)** | Indexed mesh per body, units, names, colours | `meshio`, `io` | M3 |
| OBJ | later | **yes (2026-10-12)** | Indexed mesh, normals, groups per body/face | `meshio`, `io` | M3 |
| STEP (`.step`, `.stp`) | M10 | **yes (2026-10-13)** | Exact B-rep geometry and topology, units, uncertainty, names; **no history** | `step`, `io` | M8 |

Formats live in standalone crates with their own format-level model and no workspace dependencies; the mapping to bodies and documents lives in `io` ([`architecture.md`](architecture.md) §3).

## Native format: `.3dcraft`

**Implemented today (format version 6; version 1 to 5 files open unchanged; 5 added the `transform`, `mirror` and `pattern` feature kinds and names that are copies of names; 6 added the `spline` kind of sketch entity, written `{"type": "spline", "degree": 3, "control": [1, 2, 3, 4]}` with the ids of its control points, its knots being clamped and uniform by definition):** a ZIP of stored entries with one entry, `manifest.json` (below). Unknown extra entries are ignored on load and not preserved on save. `blobs/`, `thumb.png`, the directory layout, autosave and crash recovery are the plan, not built.

A `.3dcraft` file is a ZIP of STORE (uncompressed) entries, or a directory with the same layout (useful in git and for debugging):

```text
manifest.json            the document, versioned
blobs/<blake3>.zst       content-addressed, zstd-compressed objects: imported bodies, cached regeneration results
thumb.png                preview (CPU renderer)
```

**`manifest.json`** holds `format_version` and the whole `craft3d-doc` document as JSON: readable, diffable and writable by an agent in an emergency.

- **What is stored is what the user wrote.** Expressions are stored as source text (`"width / 2 + 3 mm"`), not as evaluated numbers. Values are recomputed on load.
- **Order is stored explicitly:** the feature list in application order, the variable table in evaluation order, each sketch's constraints in the order they were added, and each sketch's last solved state (so a reopened sketch does not flip to another solution branch).
- **Ids are stable.** Feature and sketch-entity ids are never reused within a document, because persistent names are built from them.
- **Caches are optional.** A cached body is keyed by the hash of everything it was derived from. Loading verifies the key; a missing, stale or damaged cache is regenerated silently. Deleting `blobs/` of a document without imports must not change the model.
- **Imported bodies are data.** A STEP import becomes an `Import` feature whose body is a blob: it cannot be regenerated, so it is not a cache and is never garbage-collected while referenced.
- **Version history.** 1 (2026-10-12): variables; block, cylinder, sphere. 2 (2026-10-12): adds cone and torus, and an optional `orientation` (`axis`, `angle`) on block, cylinder, cone and torus. 3 (2026-10-12): adds the `sketch` feature kind. 4 (2026-10-12): adds the `extrude` feature kind (`sketch`: the id of the sketch feature, `distance`: an expression, `symmetric`) and a second kind of sketch plane, `{"type": "face", "face": {"feature": 2, "role": "endCap", "sources": [7, 8, 9]}, "offset": expr}`: a flat face of an earlier body by its persistent name. Nothing in an older file changed meaning in any step; the number was raised so that an older build reports "a newer 3DCraft is needed" instead of an unknown field or kind.
- **A sketch in the file** is a feature of type `sketch` with a `plane` (`{"type": "world", "plane": "xy|yz|zx", "offset": expr}`), `entities` in the order they were drawn (`{"id", "construction", "geometry"}`, where geometry is a `point` with `x` and `y`, a `line` with `start` and `end`, a `circle` with `center` and `radius`, or an `arc` with `center`, `start` and `end`, all referring to point entities by id), `constraints` in the order they were added (`{"id", "kind": {"type": …}}` with entity ids and, for dimensions, a `value` expression), and the next free entity and constraint ids. The point coordinates and circle radii are plain numbers in the document's length unit: they are the last solution, and the solver starts from them.
- **Versioning.** Every change to the meaning of existing fields bumps `format_version` and adds a migration (`format/src/migrate.rs`) with a fixture of the old version in the tests. New optional fields use `#[serde(default)]`. A file from a *newer* version opens with an error that says so, never with silent data loss.
- **Completeness is enforced at compile time:** `format` destructures every `doc` struct exhaustively, so a new document field cannot be forgotten ([`AGENTS.md`](../AGENTS.md) §8).
- **Atomic saves:** write to a temporary file in the same directory, fsync, rename over the target. A crash or a full disk leaves the previous file intact.
- **Autosave and crash recovery** reuse the writer, from a snapshot on a background thread, into a recovery directory keyed per launch.

Tests: serde round trip of every feature type; save → load → regenerate gives identical bodies; cache-stripped files; migration fixtures; truncation at every offset and fuzzing never panic.

The body blob encoding (how a B-rep is serialised) is defined in M2 alongside `topo`, versioned independently inside the blob header.

## STEP

**As built (2026-10-13): export.** `file.export` with a `.step` or `.stp` name (or `"format": "step"`) writes every body of the document as an exact B-rep, with the document's length unit, its angle unit, its linear tolerance as the file's uncertainty, and each body under its feature's name. `craft3d-cli convert part.3dcraft part.step` does the same. Import is M10.

**Structure.** `craft3d-step` is the Part 21 layer and nothing else: a file is a header (records) and one data section (entity instances, simple or complex), and a record is a keyword with parameters (integer, real, string, enumeration, binary, reference, list, typed parameter, omitted `$`, derived `*`). It reads and writes that model and knows no entity. Reals are written as the shortest text that reads back as the same `f64` (`1.`, `0.25`, `1.E-7`); strings as printable ASCII with `''`, `\\` and the `\X2\`, `\X4\` directives for everything else (read: also `\X\`, `\S\`, `\P`, raw UTF-8 and raw ISO 8859-1). Unknown entities survive a read → write round trip as the instances they are; comments and white space do not. A reader gets every reference resolved (an unresolved one is an error) and can ask for a reference cycle before it follows references. Not read: a second data section, the `ANCHOR`/`REFERENCE`/`SIGNATURE` sections of edition 3, `&SCOPE` of edition 1; each is an error that says so. Hostile input is an error with byte offset, line and column; nesting (64), instances, parameters and string length are capped (`ReadLimits`). Which entities to write, and what their attributes mean, is `craft3d_io::step`: there is no typed entity layer in `craft3d-step`, the mapping builds records directly.

**Entities written.** The schema declared is `AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }` (AP214; [`decisions.md`](decisions.md) D-open-8).

| 3DCraft | STEP |
|---|---|
| body | `PRODUCT` → `PRODUCT_DEFINITION_FORMATION` → `PRODUCT_DEFINITION` → `PRODUCT_DEFINITION_SHAPE` → `SHAPE_DEFINITION_REPRESENTATION`, and `PRODUCT_RELATED_PRODUCT_CATEGORY('part')`; one `APPLICATION_CONTEXT`, `APPLICATION_PROTOCOL_DEFINITION`, `PRODUCT_CONTEXT`, `PRODUCT_DEFINITION_CONTEXT` per file |
| solid body | `ADVANCED_BREP_SHAPE_REPRESENTATION` with one item per lump |
| lump | `MANIFOLD_SOLID_BREP` of a `CLOSED_SHELL`; with cavities `BREP_WITH_VOIDS`, each cavity an `ORIENTED_CLOSED_SHELL` (`.F.`) of a `CLOSED_SHELL` |
| sheet body | `MANIFOLD_SURFACE_SHAPE_REPRESENTATION` of a `SHELL_BASED_SURFACE_MODEL` of `OPEN_SHELL`s |
| face, loop, coedge, edge, vertex | `ADVANCED_FACE` with `FACE_OUTER_BOUND` / `FACE_BOUND`; `EDGE_LOOP`; `ORIENTED_EDGE`; `EDGE_CURVE`; `VERTEX_POINT` on a `CARTESIAN_POINT` |
| a whole sphere or torus; a cone's apex inside a face | a `FACE_BOUND` of a `VERTEX_LOOP` |
| plane, cylinder, cone, sphere, torus | `PLANE`, `CYLINDRICAL_SURFACE`, `CONICAL_SURFACE`, `SPHERICAL_SURFACE`, `TOROIDAL_SURFACE` on an `AXIS2_PLACEMENT_3D` |
| Bézier patch, NURBS surface | `B_SPLINE_SURFACE_WITH_KNOTS`; rational: the complex instance with `RATIONAL_B_SPLINE_SURFACE` |
| line, circle, ellipse, NURBS curve | `LINE` (with a `VECTOR` of magnitude 1), `CIRCLE`, `ELLIPSE`, `B_SPLINE_CURVE_WITH_KNOTS` (rational: the complex instance with `RATIONAL_B_SPLINE_CURVE`) |
| units, tolerance | one context: `GEOMETRIC_REPRESENTATION_CONTEXT(3)`, `GLOBAL_UNIT_ASSIGNED_CONTEXT` (length: `SI_UNIT` with a prefix, or `CONVERSION_BASED_UNIT` `'INCH'` 25.4 mm / `'FOOT'` 304.8 mm; plane angle: radian or `'DEGREE'`; steradian), `GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT` with one `UNCERTAINTY_MEASURE_WITH_UNIT` |

Instances are numbered from 1 in the order they are made and refer only to earlier ones; the same document gives the same bytes (`FILE_NAME.time_stamp` is empty for that reason). Geometry is not shared between entities: every placement has its own point and directions.

**Conventions.** The module documentation of `crates/io/src/step.rs` has each one with how sure it is; in short:

- Surfaces and curves are parametrised by ISO 10303-42 exactly as `craft3d-geom` does (normal `∂u × ∂v`: away from the axis or centre), so frames and radii go over unchanged. A cone gets a positive semi-angle: one that narrows along its frame's z is written on the frame seen from the other side. An ellipse gets its longer semi-axis first, on a placement turned a quarter if need be. Angles are in the document's angle unit.
- An edge runs from its start to its end vertex with the curve's parameter (`same_sense` `.T.`, always) and is the part of its curve between its vertices; an `ORIENTED_EDGE` is `.T.` unless the coedge is reversed; `ADVANCED_FACE.same_sense` is `.T.` unless the face is reversed; bounds are `.T.` (our loops already run with the face on their left seen from outside).
- `FACE_OUTER_BOUND` is written first, and only where a loop is the outer one: the one loop of a face; on a plane the loop that runs counter-clockwise; elsewhere the one loop that does not go around the surface, when none does. A cylinder wall between two circles has two plain `FACE_BOUND`s.
- Faces on closed surfaces are written without seam edges, as they are in the kernel. Part 42 does not ask for seams; readers add their own.
- A cavity's `CLOSED_SHELL` has its faces turned over (so its normals point away from the volume it encloses, as a closed shell's must), and `ORIENTED_CLOSED_SHELL` `.F.` turns them back. A shell is a cavity when it encloses a negative volume, and belongs to the innermost lump that contains a point of it.

**Refused**, with an error that names the body: a body that fails `topo::check` at the document's tolerance; a face without loops on anything but a sphere or a torus; a planar face without exactly one counter-clockwise loop; a solid of several shells where one cannot be measured, encloses nothing, or is a cavity outside every lump; geometry that is not finite. Not written at all: assemblies (every body is a part of its own in the world frame), colours, PMI, face and edge names, parameter-space curves.

**Specs.** The exchange structure is ISO 10303-21; the entity definitions are in ISO 10303-42 (geometry and topology) and the application protocols (ISO 10303-214, -242). ISO documents are paid publications: work from copies the project legitimately has, from the freely published EXPRESS schemas and recommended practices of the CAx Implementor Forum, and from the files other systems write. Never paste text from a standard into the repository; cite the clause. The mapping as built was written from memory of the standard and checked against what FreeCAD makes of the files; the clause numbers in the code are marked as approximate.

**Tests.** In `craft3d-step`: every real reads back bit for bit (20 000 random bit patterns); a file with one of everything reads back as the same model and its text is pinned; every string directive; hostile input (some 40 malformed files, each with the expected error and line), truncation at every offset, 20 000 random damages, limits, cycles. In `craft3d-io` (`tests/step.rs`): 26 bodies at three scales read back as the same instance graph and pass a small independent interpreter (closed loops, each edge of a closed shell used once each way, every vertex on its edge's curve and on its face's surface); each convention above; golden text for a block and a cylinder (`tests/golden/`, `CRAFT3D_UPDATE_GOLDEN=1` rewrites them); what is refused. In `craft3d-engine`: `tests/step.rs` and the acceptance scenario. And the external check ([`testing.md`](testing.md) oracle 8): FreeCAD imports the files and reports the same solids. That last test is the one that matters: a STEP file is only correct if other CAD systems accept it.

**Tolerance on export.** The document's modelling tolerance is written as the file's uncertainty measure; the body is checked with that tolerance before it is written, so no vertex is further from its edge, and no edge from its face, than the file declares. (There are no per-entity tolerances in `topo` yet to tighten or report.)

## Meshes: STL, 3MF, OBJ

All three are written from the tessellator's output at a caller-chosen chordal tolerance. `io` merges vertices at bit-identical positions (the tessellator makes shared edges exact, so no distance tolerance is involved) and marks the object a solid only when the merged mesh is closed and consistently oriented; an open mesh is exported as a surface (3MF `type="surface"`). *Planned with bodies:* a body that claims to be a solid but tessellates open is an error rather than a leaky file.

Today: one object per file, no colours or materials, OBJ without normals (`vn`) or groups per face.

| Format | Notes | Spec |
|---|---|---|
| STL | Binary by default (80-byte header, triangle count, 50 bytes per triangle, little-endian f32), ASCII on request. No units in the format: export in the document's length unit and say so in the result. Vertices are not shared in STL; closedness is checked on our indexed mesh before writing | The original 3D Systems StereoLithography interface description (1989); de facto conventions |
| 3MF | An OPC (ZIP) package with `[Content_Types].xml`, `_rels/.rels` and `3D/3dmodel.model` (XML): indexed vertices and triangles per object, explicit unit, object names, base-material colours. Preferred mesh format: it has units and guarantees an indexed mesh | 3MF Core Specification (3MF Consortium, freely available) |
| OBJ | `v`, `vn`, `f` with one `o` per body and `g` per face; optional `.mtl` for colours. No units | Wavefront OBJ format description (public) |

The ZIP and XML writing needed for 3MF is part of `meshio`: a stored (uncompressed) ZIP writer with CRC-32, no dependency. Files are larger than they need to be (the teapot at tolerance 0.002 is 11.9 MB as 3MF, 6.5 MB as STL); DEFLATE is a later improvement.

Tests (in place): exact expected text for ASCII STL, OBJ and the 3MF model part; binary STL and the ZIP container parsed back by minimal readers in the tests; FreeCAD reading a box (solid, volume 24) and the teapot in all four encodings. The FreeCAD test also covers 32-bit rounding in STL: FreeCAD merges the rounded points to exactly the vertex count of our welded mesh.

## Later candidates

IGES and DXF/SVG (sketch import/export), glTF (viewing), AMF, and reading the mesh formats. None is planned before M10; each needs a roadmap entry first.
