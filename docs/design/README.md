# 3DCraft design documents

The detail documents behind the map in [`../architecture.md`](../architecture.md). Each one says how one part works now (**Method**) and keeps the dated history of what was found and changed in it (**Findings and history**).

| Document | What it holds |
|---|---|
| [`geometry.md`](geometry.md) | curves, surfaces, projection, intersections, poles, tangency, the marcher |
| [`topology.md`](topology.md) | the B-rep, `topo::check`, genus, the self-intersection check, mass properties, transforms |
| [`kernel.md`](kernel.md) | what every kernel operation promises, and the list of operations |
| [`booleans.md`](booleans.md) | imprint, classify, assemble; freeform operands; what is refused; speed; the corpus |
| [`blends.md`](blends.md) | chamfers and fillets: closed-form tools, rolling-ball blends, chains, meets, ends |
| [`sweeps.md`](sweeps.md) | profiles with splines, extrude and its ends, revolve, loft, sweep, helix, thread |
| [`face-operations.md`](face-operations.md) | offset face, shell, move face, draft, thicken, delete face |
| [`developable.md`](developable.md) | developable surfaces between two curves, the Gaussian curvature of any surface, unfolding faces flat with the map between pattern and part |
| [`names.md`](names.md) | persistent names: how they are made, followed and resolved |
| [`document.md`](document.md) | the document model, expressions, sketches and the solver, datums, appearance and material |
| [`regeneration.md`](regeneration.md) | the cache, long rows, failed features, the self-intersection check per operation |
| [`tessellator.md`](tessellator.md) | edges first, faces, seams, poles, angle, budgets, display and picture quality |
| [`rendering.md`](rendering.md) | the CPU oracle, the GPU renderer and its tolerances, see-through, section, pointing, the viewport |
| [`engine.md`](engine.md) | Session, commands, undo, threads, scenes, recovery, automation |
| [`selection.md`](selection.md) | the selection shared by the window and agents, picking through, and selecting by description (`select.query`) |
| [`origami.md`](origami.md) | crease patterns on a flat sheet: facets of the arrangement, rigid folding by loop closure, contact between facets, Kawasaki and Maekawa (necessary, not sufficient), crease patterns to SVG and DXF |
| [`verification.md`](verification.md) | checking a design by questions: distance, clearance, wall thickness, ray, through, symmetry, assertions kept in the document, and the proof of a Jig over its parameter range (`document.sample`, `check-range`) |
| [`sections-and-levels.md`](sections-and-levels.md) | what lies on one side of a plane: the volume and centroid below it cut off and integrated exactly, a section's area and second moments by Green's theorem, the level for a volume by Newton's method |
| [`plates.md`](plates.md) | plates (faces given a thickness on a side), the joints between them (square, bevel, mitre, touch, over/under), and the design of sheet metal on fold and unfold |
| [`sheetmetal.md`](sheetmetal.md) | sheet metal: flanges and bends with an inner radius and a K-factor (bends as linkage joints with a radius), the flat pattern with bend allowance and corner reliefs, the part as the plate of the folded skin, the bend table, manufacturability measures with the limits from the document |
| [`step-import.md`](step-import.md) | STEP files read into bodies: typed reading, sewing on topology only, the import feature and its data, names, oracles |
| [`files.md`](files.md) | `.3dcraft`, STEP, meshes, flat outlines (the formats themselves: [`../file-formats.md`](../file-formats.md)) |
| [`plan.md`](plan.md) | the design plan of 2026-10-12 as written then: goals, principles, the CAD holes, workspace, layering, the workspace sketch, the revision history of `architecture.md` |

## Where the old sections are

Until 2026-10-15 all of this was one file, `docs/architecture.md`. Its text was moved here verbatim; references in it such as "§5.3 › *Fillets as built*" are to the sections of that file:

| Old section | Now in |
|---|---|
| *Current state (2026-10-12)* | each bullet in the document of its part, under *From Current state*; the workspace, the end-to-end path and the kernel summary in [`plan.md`](plan.md) and [`kernel.md`](kernel.md) |
| §1 Goals and principles, §1.1 Avoiding the usual CAD holes, §2 Workspace layout, §3 Dependency layering | [`plan.md`](plan.md) |
| §4 Document model | [`document.md`](document.md) |
| §4.1 Persistent names (as built M6, bodies made from bodies, booleans, edges and vertices by their faces, copies of copies, primitives, the frame of a sketch on a face, references that follow a body) | [`names.md`](names.md) |
| §4.1 › *Splines in sketches*, *A point on a spline*, *Reference geometry* | [`document.md`](document.md) |
| §4.1 › *A feature that fails* | [`regeneration.md`](regeneration.md) |
| §4.1 › *Profiles and extrusions with splines*, *Revolved splines, closed splines and corners*, *Loft*, *Sweep along a path*, *Helix, and the sweep along it*, *Thread*, *Extrude ends* | [`sweeps.md`](sweeps.md) |
| §4.1 › *Chamfers*, *Fillets* | [`blends.md`](blends.md) |
| §4.1 › *The face operations' faces* | [`face-operations.md`](face-operations.md) |
| §4.2 Appearance and material per body | [`document.md`](document.md) |
| §5 Kernel, §5.3 Operations (the table and the rules) | [`kernel.md`](kernel.md) |
| §5.1 Geometry, *Intersections as built*, *Tacnodes and cusps* | [`geometry.md`](geometry.md) |
| §5.2 Topology | [`topology.md`](topology.md) |
| §5.3 › *Chamfers as built*, *Fillets as built*, *Blends next to a cylinder*, *Blends along freeform edges as built*, the blend findings, *Chains and meets of freeform blends* | [`blends.md`](blends.md) |
| §5.3 › *Face operations as built* | [`face-operations.md`](face-operations.md) |
| §5.3 › *Booleans as built* and everything under it (*Speed*, *Freeform faces*, *Rational faces*, poles, cusps, *Showcase findings*, *Coincident freeform faces*, *Freeform faces that close on themselves*, *The helmet's findings*) | [`booleans.md`](booleans.md) |
| §5.4 Sketch solver | [`document.md`](document.md) |
| §5.5 Regeneration (*Long rows*, *Never through itself*) | [`regeneration.md`](regeneration.md) |
| §6 Tessellator | [`tessellator.md`](tessellator.md) |
| §7 Rendering (§7.2 *See-through on the GPU*, *Section on the GPU*, …) | [`rendering.md`](rendering.md) |
| §8 Engine (§8.4 Runtime model, §8.5 The UI-swap seam), §10 Automation | [`engine.md`](engine.md) |
| §9 Files | [`files.md`](files.md) |
| §11 Testing and quality, §12 Cargo workspace sketch, §13 Open questions, Revision history | [`plan.md`](plan.md) |
