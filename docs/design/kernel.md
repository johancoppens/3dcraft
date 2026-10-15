# 3DCraft design: The kernel

What every modelling operation of `craft3d-kernel` promises, and the list of operations; the methods themselves are in their own documents ([`../architecture.md`](../architecture.md) §5).

## Method

**An operation is a pure function.** Bodies, parameters and the tolerance in; out a new body with, for every face, edge and vertex, its origin (which input entities, in which role), or a `KernelError` naming the entities and the reason. No operation reads the document, meshes, or registers a command: `regen` calls the kernel and turns origins into names ([`names.md`](names.md)), `engine` registers the commands. There is no cancellation token yet.

**The rules every operation keeps.** The result passes `topo::check`, or the operation returns an error; a case that is not built is refused with its reason (`Unsupported`, or the operation's own error), never answered with a body that is valid in its counts and wrong in space. Results do not depend on the number of threads or on hash order. No length is built in: every threshold is the tolerance or a ratio.

**The operations**, by method:

| Method | Modules | Detail |
|---|---|---|
| Primitives | `primitives`: block, cylinder, cone, sphere, torus, each built in a fixed order of entities | [`names.md`](names.md) |
| Profiles and sweeps | `profile`, `spline2`, `extrude`, `extrude_to`, `revolve`, `loft`, `sweep`, `helix`, `thread` | [`sweeps.md`](sweeps.md) |
| Booleans | `boolean` (and `contains_points`) | [`booleans.md`](booleans.md) |
| Blends | `blend` (with `blend::rolling`), front ends `chamfer` and `fillet` | [`blends.md`](blends.md) |
| Face operations | `tweak`, `offset_face`, `shell`, `move_face`, `draft`, `thicken`, `delete_face` | [`face-operations.md`](face-operations.md) |
| Arrangement | transform, mirror and pattern: `topo::transform` and `merge`, joined by the boolean where instances touch (built in `regen`) | [`names.md`](names.md) |

The checks the kernel cannot make itself (a body that passes through itself far from what changed) are made by regeneration on a mesh ([`regeneration.md`](regeneration.md)).

## Findings and history

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

### From *Current state (2026-10-12)*

- **Kernel:** `kernel::primitives` builds block, cylinder, cone, sphere and torus as B-rep. `kernel::profile` finds the closed regions of a set of 2D curves (loops of lines, arcs and, since 2026-10-13, splines joined end to end, circles; even-odd nesting gives holes and islands; open, branching, crossing or touching curves are errors that name them). `kernel::extrude` sweeps profiles along a plane's normal into one solid with a shell per profile, checked before it is returned, and reports for every face, edge and vertex where it came from (`Origins`). `kernel::extrude_to` (2026-10-13) ends an extrusion on the surface of a face instead (§4.1 › *Extrude ends*). `kernel::boolean` is the union, difference and intersection of two solids (whose faces were planes and cylinders when this was written; since 2026-10-13 any analytic or freeform face: [`booleans.md`](booleans.md)), with coincident faces and smooth edges handled, and a source for every face, edge and vertex of the result; everything else is `KernelError::Unsupported` with the faces and the reason (§5.3 › Booleans as built). It rests on `geom::intersect` (closed forms of planes and cylinders when this was written; `meet_surfaces` for every pair since 2026-10-13). `kernel::revolve` turns profiles about an axis in their plane through a full turn or part of one (a line sweeps a plane, a cylinder or a cone, an arc a sphere or a torus; a curve on the axis sweeps nothing) and reports origins in the same roles as an extrusion; a profile that crosses the axis and an arc that would sweep a spindle torus are errors that name the curves. `kernel::chamfer` (2026-10-14) replaces edges of a solid by faces: straight edges between planes and circular edges (whole or part) between a plane, a cylinder or a cone about the circle's axis, with one distance, two, or a distance and an angle, chains of tangent edges, mitres and flat corners where chamfers meet (§5.3 › *Chamfers as built*). `kernel::fillet` (2026-10-14) rounds the same edges with a radius: a cylinder along a straight edge between planes at any angle, a torus along a circular edge, a piece of a ball where three straight fillets of one radius meet, chains with their tools joined into one; both blends are front ends of `kernel::blend` (§5.3 › *Fillets as built*). Since 2026-10-14 both also take a straight edge next to a cylinder along it (worked out in the plane across the edge) and, as a spline surface within the tolerance built in place (`kernel::blend::rolling`), any other edge whose ends are corners of three faces, or that is closed (§5.3 › *Blends next to a cylinder*, *Blends along freeform edges*). `kernel::offset_face` (2026-10-14) moves faces of a solid along their normals with the neighbours following, by a local operation that keeps every entity (`kernel::tweak`), and `kernel::shell` makes a solid hollow with walls of one thickness and faces left open, the hollow being that offset applied to every face and taken away by the boolean (§5.3 › *Face operations as built*). `kernel::delete_face` (2026-10-14) takes faces out of a solid and closes it again: holes, bosses and pockets filled from the faces round them, a chamfer or a fillet collapsed into the edge between its neighbours (§5.3 › *Face operations as built*). Since 2026-10-14 a spline is revolved too (a rational B-spline surface of revolution, a full turn in two halves), a spline that closes on itself is a profile on its own and a spline with corners (a polyline) is one as well, each cut into pieces that sweep one face each (§4.1 › *Revolved splines, closed splines and corners*).

### 5. Kernel

#### 5.3 Operations (`kernel`)

Each operation is a pure function: bodies and parameters in, `Result<OpResult, KernelError>` out, where `OpResult` holds the new body and the **origin map** (for every result entity: which input entities and which role produced it), which feeds persistent naming.

| Group | Operations | Milestone |
|---|---|---|
| Primitives | box, cylinder, cone, sphere, torus | M3 |
| From sketches | profile detection (closed regions from sketch entities), extrude, revolve | M6 |
| Loft | a solid through a row of closed profiles: ruled, smooth, closed into a ring (`kernel::loft`, ✅ 2026-10-14, see *Loft*) | M9 |
| Booleans | union, subtract, intersect on solids; imprint and split as operations of their own *(planned)* | M7 |
| Local | chamfer (✅ 2026-10-14, see *Chamfers as built*), fillet with a constant radius (✅ 2026-10-14, see *Fillets as built*), offset face, shell and delete face (✅ 2026-10-14, see *Face operations as built*), move face, thicken and draft (✅ 2026-10-15, the same), replace face *(planned)* | M9 |
| Sweeps | a closed profile carried along a path in a plane: lines, arcs, mitred corners, splines; open or closed (`kernel::sweep`, ✅ 2026-10-14, see *Sweep along a path*); along a helix by its screw motion, and the ISO metric thread cut by it (`kernel::helix`, `kernel::thread`, ✅ 2026-10-14, see *Helix, and the sweep along it* and *Thread*) | M9 |
| Arrangement | transform, mirror, linear and circular pattern | M6–M9 |

**Rules for every operation:**
- **Signature:** bodies, parameters and the tolerance in, `Result<…, KernelError>` out. *(Planned, not built: a `CancelToken`, serialisable parameter structs, one `OpResult` type.)*
- **Never an invalid body.** The result passes `topo::check` or the operation returns `Err`. Errors name the entities and the reason ("faces F3/cap and F5/side are tangent along a curve; tangent booleans are not supported yet").
- **Deterministic:** no dependence on thread count or hash order.
- **Not registered by itself:** `engine` registers commands; `kernel` stays engine-agnostic.
