# 3DCraft design: Files

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

## From *Current state (2026-10-12)*

- **Mesh export:** `meshio` writes binary and ASCII STL, 3MF and OBJ; `io` welds a tessellated mesh, classifies it as solid or surface and encodes one or more objects. FreeCAD reads the box, the teapot and all fourteen tessellated reference solids (primitives, hemisphere, and the solids with trimmed faces) back as closed solids with the same counts, area and volume (`cargo xtask freecad`). No readers, no colours.
- **Native format:** `format` reads and writes `.3dcraft`: a stored ZIP (`zipstore`, with a reader hardened against hostile archives) holding a pretty-printed `manifest.json`, now at `format_version` 4 (version 1 to 3 files still open, through the migration chain). The file's types are separate from `doc`'s and converted by exhaustive destructuring, so a new document field does not compile until it is saved and loaded (verified by adding one). Saves are atomic and refuse an invalid document; loads validate. Not built: `thumb.png`, `blobs/`, the directory layout, autosave and crash recovery.
- **STEP export (2026-10-13):** `step` reads and writes ISO 10303-21 with its own model (records with parameters, simple and complex instances; nothing about CAD in it); `io::step` maps bodies to the AP214 B-rep entities (`MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `ADVANCED_FACE`, every curve and surface kind incl. rational B-splines, vertex loops for whole spheres and tori and for an apex inside a face) with the document's units, uncertainty and names, after checking each body; `file.export` and `craft3d-cli convert` write `.step`/`.stp`. For that `io` now depends on `geom`, `topo` and `kernel` (the latter only for `boolean::contains_points`, to tell which lump a cavity is in). FreeCAD reads 448 exported files as valid solids of our volume and area (`cargo xtask freecad`). No import yet, no assemblies, colours or names of faces. The conventions are in [`file-formats.md`](../file-formats.md) and in the module documentation.
- **DXF and SVG export (2026-10-13):** `drawing` writes ASCII DXF (AutoCAD 2000) and SVG 1.1 from its own 2D model (chains of lines, circular and elliptic arcs and B-splines; whole circles and ellipses; a unit; layers); `io::drawing` maps a sketch (its curves joined into chains) and the loops of flat faces (seen from outside the body) onto it; `file.export` writes them for a `sketch` or a `face`. Every curve stays what it is. FreeCAD reads both formats back with the closed-form length and area ([`file-formats.md`](../file-formats.md) › Flat outlines).


## 9. Files

Detail in [`file-formats.md`](../file-formats.md).

- **Native `.3dcraft`** (`format`): a bundle (ZIP of STORE entries, or a directory with the same layout) with a versioned `manifest.json` holding the document, and content-addressed blobs for imported bodies and optional regeneration caches. Atomic saves, autosave and crash recovery through the same writer.
- **STEP** (`step` + `io`): export of exact B-rep (AP214 geometry and topology) is built; import later. STEP carries the result, not the history.
- **Meshes** (`meshio` + `io`): STL, 3MF and OBJ export from `tess` output at a caller-chosen tolerance.
- **Flat outlines** (`drawing` + `io`, 2026-10-13): DXF and SVG export of a sketch or of a flat face, as true curves at true size, for laser cutting.

---
