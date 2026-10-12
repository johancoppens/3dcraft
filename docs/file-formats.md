# File formats

> **Last updated:** 2026-10-12 · **Status:** design, pre-M0. Nothing is implemented; the table is the plan.

| Format | Read | Write | Carries | Crates | Milestone |
|---|---|---|---|---|---|
| `.3dcraft` (native) | yes | yes | The design history: units, tolerance, variables, formulas, sketches, constraints, features, their order; imported bodies; optional caches | `format` | M2 |
| STL (binary, ASCII) | later | yes | Triangles only; no units, no colour | `meshio`, `io` | M3 |
| 3MF | later | yes | Indexed mesh per body, units, names, colours | `meshio`, `io` | M3 |
| OBJ | later | yes | Indexed mesh, normals, groups per body/face | `meshio`, `io` | M3 |
| STEP (`.step`, `.stp`) | M10 | yes | Exact B-rep geometry and topology, units, names; **no history** | `step`, `io` | M8 |

Formats live in standalone crates with their own format-level model and no workspace dependencies; the mapping to bodies and documents lives in `io` ([`architecture.md`](architecture.md) §3).

## Native format: `.3dcraft`

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

All three are written from the tessellator's output for the selected bodies at a caller-chosen chordal tolerance (default from the document, overridable per export). Because the tessellator is watertight by construction, exported solids are closed manifolds; the exporter asserts this and fails with an error rather than writing a leaky mesh.

| Format | Notes | Spec |
|---|---|---|
| STL | Binary by default (80-byte header, triangle count, 50 bytes per triangle, little-endian f32), ASCII on request. No units in the format: export in the document's length unit and say so in the result. Vertices are not shared in STL; closedness is checked on our indexed mesh before writing | The original 3D Systems StereoLithography interface description (1989); de facto conventions |
| 3MF | An OPC (ZIP) package with `[Content_Types].xml`, `_rels/.rels` and `3D/3dmodel.model` (XML): indexed vertices and triangles per object, explicit unit, object names, base-material colours. Preferred mesh format: it has units and guarantees an indexed mesh | 3MF Core Specification (3MF Consortium, freely available) |
| OBJ | `v`, `vn`, `f` with one `o` per body and `g` per face; optional `.mtl` for colours. No units | Wavefront OBJ format description (public) |

The ZIP and XML writing needed for 3MF is part of `meshio` (STORE and DEFLATE writing is small; a permissively licensed pure-Rust compression crate is acceptable here, since it is not modelling code).

Tests: byte-exact expected files for a tetrahedron and a box; parse-back of our own output (minimal readers in tests) giving the same vertex/triangle counts and volume; closedness and orientation asserted; 3MF package structure validated against the spec's required parts; f32 rounding in STL does not open cracks (vertices shared in our mesh still compare equal after conversion).

## Later candidates

IGES and DXF/SVG (sketch import/export), glTF (viewing), AMF, and reading the mesh formats. None is planned before M10; each needs a roadmap entry first.
