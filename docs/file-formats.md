# File formats

> **Last updated:** 2026-10-12 · **Status:** `.3dcraft` (version 4) and mesh export (STL, 3MF, OBJ) work and are tested; STEP is the plan.

| Format | Read | Write | Carries | Crates | Milestone |
|---|---|---|---|---|---|
| `.3dcraft` (native) | **yes (2026-10-12)** | **yes (2026-10-12)** | The design history: units, tolerance, variables, formulas, sketches, constraints, features, their order; imported bodies; optional caches | `format` | M2 |
| STL (binary, ASCII) | later | **yes (2026-10-12)** | Triangles only; no units, no colour | `meshio`, `io` | M3 |
| 3MF | later | **yes (2026-10-12)** | Indexed mesh per body, units, names, colours | `meshio`, `io` | M3 |
| OBJ | later | **yes (2026-10-12)** | Indexed mesh, normals, groups per body/face | `meshio`, `io` | M3 |
| STEP (`.step`, `.stp`) | M10 | yes | Exact B-rep geometry and topology, units, names; **no history** | `step`, `io` | M8 |

Formats live in standalone crates with their own format-level model and no workspace dependencies; the mapping to bodies and documents lives in `io` ([`architecture.md`](architecture.md) §3).

## Native format: `.3dcraft`

**Implemented today (format version 4; version 1, 2 and 3 files open unchanged):** a ZIP of stored entries with one entry, `manifest.json` (below). Unknown extra entries are ignored on load and not preserved on save. `blobs/`, `thumb.png`, the directory layout, autosave and crash recovery are the plan, not built.

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

**Scope.** Export solids and sheets as exact B-rep (`MANIFOLD_SOLID_BREP`, `ADVANCED_FACE`, `EDGE_CURVE`, analytic surfaces and `B_SPLINE_SURFACE_WITH_KNOTS`, …) with units and product names, in the AP214/AP242 "automotive design" family that every mainstream CAD system reads. Assemblies, colours and PMI come later. Import (M10) maps the same entity set back to `topo` bodies as `Import` features.

**Structure.** `craft3d-step` has two levels: a Part 21 layer (tokenizer, parser and writer for the exchange structure: header, entity instances, references, typed parameters) that knows nothing about CAD, and a typed layer for the entities we use. Unknown entities survive a parse → write round trip verbatim.

**Specs.** The exchange structure is ISO 10303-21; the entity definitions are in ISO 10303-42 (geometry and topology) and the application protocols (ISO 10303-214, -242). ISO documents are paid publications: work from copies the project legitimately has, from the freely published EXPRESS schemas and recommended practices of the CAx Implementor Forum, and from the files other systems write. Never paste text from a standard into the repository; cite the clause.

**Tests.** Writer output parses back to the same model; exported analytic solids have the expected entity counts; malformed-input sweep and fuzzing on the reader; and the external check ([`testing.md`](testing.md) oracle 8): an independent program opens the file and reports the same volume and bounding box. That last test is the one that matters: a STEP file is only correct if other CAD systems accept it.

**Tolerance on export.** The document's modelling tolerance is written as the file's uncertainty measure, and entity tolerances larger than it are tightened or reported, because receivers reject geometry whose gaps exceed the declared uncertainty.

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
