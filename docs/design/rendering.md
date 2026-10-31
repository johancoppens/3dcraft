# 3DCraft design: Rendering, the viewport and pointing

Pictures: the CPU reference renderer (`craft3d-render`, the display oracle), the wgpu renderer (`craft3d-gpu`), the viewport of the window (`ui-egui`, `apps/craft3d`) and pointing at faces, edges and vertices. Rule 9 of the map ([`../architecture.md`](../architecture.md) §1) and §7.

## Method

**One picture, two renderers.** Both take the same input (a mesh with face ids, edge polylines, a camera, `RenderParams`: colour, per-face colours, highlight, opacity, section) and give the same `Frame` (colour, face ids, view distance). The CPU renderer is the definition: a rasteriser with a depth buffer, smooth two-sided shading, supersampling and an id buffer, in `f64`. The GPU renderer draws the same picture with wgpu (offscreen and read back, or `Onscreen` on the window's own device into a texture shown as it is, multisampled), with reversed depth, and is held to the CPU within a documented tolerance: equal face ids and colours within a level outside a band of one pixel around feature edges, distances within a bound derived from `f32` rounding, a counted share inside the band. Parity tests run on every adapter of the machine and fail, not skip, without one.

**What a picture can show.** *See-through* (opacity below 1): the front surface blended over the nearest surface strictly behind it, exactly and order-independently; on the GPU two depth layers (depth peeling, with a key per sample when multisampled) and the CPU's own blend as a table. *A section plane*: samples on the cut side are left out, and the plane is drawn as a cap where the number of surfaces left out along the ray is odd (inside a closed body); on the GPU the count is a blend pass. *Highlights* and *colours of their own* per face; datums as translucent planes, lines and points. Without any of these the picture is the plain one, bit for bit. Pictures made without a window (`view.render`, `craft3d-cli render`, a Jig's `thumb.png`, the example thumbnails) are drawn on the viewport's light background since 2026-10-23 (`background: "dark"` keeps the renderer's dark grey).

**Pointing.** A position gives the persistent name of what is under it, by the CPU renderer's own arithmetic on the scene the viewport draws (`render::pick`, `Neighbourhood`), never by reading the GPU back: a vertex in sight within the pick radius, else the nearest edge in sight, else the face. `view.pick` asks it of a camera; the shell's click and `ui.pick` of the viewport's.

**The viewport.** The window draws on the GPU through `ui_egui::direct::Direct` on its own device, only when something the frame depends on changed; edges, sketches, axes and datums are lines drawn over it and tested against its depth. Where that cannot run, the shell falls back to the GPU on a device of its own read back, or to the CPU renderer (`--renderer cpu`), and says why. `ui.screenshot` always renders on the CPU: it is the oracle. The model's truth is the B-rep, not the picture: geometry is tested geometrically ([`../testing.md`](../testing.md)).

**Surface analysis** (2026-10-30, class-A stage 1; `craft3d_render::analysis`, on the GPU `scene.wgsl` › `analysis_of`). Zebra stripes, highlight lines, a curvature map with values per vertex from the exact surfaces, and draft, drawn in place of the plain shading by both renderers, with combs as overlay lines; view state only. The stripes are filtered over each sample's footprint in phase (2026-10-31), so where they are finer than a pixel they fade to their mean instead of a moiré. Method, tolerance and tests in [`surfacing.md`](surfacing.md) › *Stage 1: analysis*.

**The studio look** (2026-10-29, layer 1 of `plan/rendering.md`; `craft3d_render::studio`, on the GPU
`studio.rs` and `studio.wgsl`). A second look beside the plain one, view state only: image-based lighting
from an environment, soft shadows from its key light, ambient occlusion, a ground that catches the
shadow, a tone curve; see *The studio look* under the findings for the method, the sources and the
tolerance. The plain look is untouched, bit for bit. Its second layer (2026-10-30,
`craft3d_render::studio::polish`): screen-space reflections and a reflecting ground, contact shadows,
the key's specular shadow, a thin lens's depth of field, bloom and the accumulation of jittered frames;
see *Layer 2* under the findings. Layer 3 (materials) and layer 3.5 (solid textures, a pattern
evaluated in the model's space) are under the findings too.

**Headless GPU and animations** (2026-10-30). `view.render` and `view.replay` draw through
`craft3d_engine::pictures`: the CPU renderer, or a renderer the host gives the session
(`Session::set_picture_renderer`). The engine does not depend on `craft3d-gpu`; `craft3d-cli` (its
`render`, `run`, `replay`, `serve` and `mcp`) gives its sessions the wgpu renderer on an offscreen device
of its own, made at the first picture. `renderer: "auto"` (default) uses it and falls back to the CPU,
`"cpu"` keeps to the oracle, `"gpu"` insists; results say which drew. `view.replay` is an animation
renderer: variable tracks, the timeline, a camera path, the look; see *Headless GPU* and *Animations*
under the findings.

## Findings and history

> Moved verbatim from `docs/architecture.md` on 2026-10-15. Section numbers (§4.1, §5.3, …) in the text are those of that file before the split; [README.md](README.md) says where each one is now.

### From *Current state (2026-10-12)*

- **Desktop app and shell:** `ui-egui` is the shell as a library (serde state with the orbit camera, theme tokens, the viewport showing the renderer's image as a texture, a read-only feature tree, a status bar and a File menu); `apps/craft3d` puts it in an eframe window and adds the control listener (TCP on 127.0.0.1, token, the `serve` protocol plus `ui.*`), whose requests are answered on the one `Session`. Since 2026-10-14 the commands among them run on the window's engine thread (`apps/craft3d/src/worker.rs`): the session is handed over with the task and comes back with the reply and the scene the window will show already built, so the drawing thread never regenerates or meshes; meanwhile it draws the last scene with the work in progress (`Shell::ui_away`, `ui_egui::Work`, reported by regeneration per feature through `craft3d_regen::progress`, a per-thread sink that the layers below the app know nothing about), and only `ui.get` and `ui.quit` are answered out of turn ([`control-protocol.md`](../control-protocol.md)). `engine::Session::scene_with` gives the shell meshes, the bodies' edges as polylines (`tess::tessellate_edges`: the points the mesh has along each edge) and sketch polylines per document revision, at the display quality (`SceneQuality::Display`, §6: finer than the export's automatic tolerance, within a triangle budget; `state.quality` in `ui.get`/`ui.set`), which `ui.screenshot` renders too; a change of quality is another mesh of the same revision, so the frame keys and the direct path's upload carry the scene's tolerance as well. Four things about the viewport: (1) the sharp image is made on the shell's render thread, and the UI thread only makes the quick preview; `frames::Scheduler`, a state machine without threads or clocks, decides what is made and drops every result that is not the image of what is wanted now (camera, document revision, size, selection). `Shell::new` renders where it draws (wasm, tests), `Shell::threaded_with` is what the window uses. (2) **Which renderer (2026-10-13, direct path the same day):** the window lends the shell its own wgpu device (eframe's `RenderState`: `Shell::threaded_with_gpu`), and with `--renderer auto` (the default) the viewport is drawn on it by `ui_egui::direct::Direct` over `craft3d_gpu::Onscreen` (§7.2): bodies and the lines over them, multisampled, every frame at the viewport's full size, directly into a texture registered with egui (`register_native_texture`) and shown as it is. Nothing is read back for display, there is no preview and no render thread on this path (the draw is submitted from the drawing thread and costs it about a millisecond; `RenderStats::milliseconds` is that thread's time, not the GPU's), and a frame is drawn only when something it depends on changed (camera, document revision, size, selection, theme). The mesh and the lines are uploaded once per scene revision (the lines again when the selection or the theme changes; the axes are rewritten in place when the zoom changes their length). When the direct path cannot be set up (no render state, no pipelines on that device), or fails for good while running (a lost device, a driver that falls over), the shell goes on as before with `ui_egui::backend::BodyRenderer`: the wgpu renderer on a device of its own per thread, read back as the same `Frame` the CPU renderer gives, with the quality rules below, or the CPU reference renderer when there is no adapter or it was asked for (`--renderer cpu`, `CRAFT3D_RENDERER=cpu`); the reason is in `ui.get` (`renderer.fallback`) and the terminal. A frame the device cannot take (a target or a mesh too large for it) is made that way too, and the next one directly again. `ui.get` reports `renderer {backend, adapter, fallback, direct, msaa}` and `render.msaa`; the status bar says `GPU 1440×900 ·…`. On the readback and CPU paths each thread has a renderer of its own with its own wgpu instance and device (wgpu's OpenGL backend ties a device to its thread), the sharp image is made on the render thread while a preview is on screen, and the budgets (`REST_SAMPLES`, `MOVING_SAMPLES` in samples for the CPU; `GPU_REST_PIXELS`, `GPU_MOVING_PIXELS` in pixels for the readback, which runs at a few hundred MB/s on this laptop's Intel driver) shrink the image while moving (`MOVING_DIVISOR`) and on a large window. `ui.screenshot` always renders on the CPU: it is the oracle. The tests: `crates/ui-egui/tests/gpu_viewport.rs` draws one scene through the direct path on a device made as eframe makes the window's (`direct::headless_render_state`) and through the readback path, and compares with the oracle under the tolerances of §7.2 (pictures in `target/evidence/ui/`); `tests/gpu_fallback.rs` takes the adapter away. In the browser the viewport is drawn the same way on the page's device (*The browser: WebGPU and WebGL2* below). Measurements (2026-10-13, `cargo run --release -p craft3d-ui-egui --example frame_time`, the development machine's Intel Meteor Lake iGPU over Vulkan, while other builds ran; the numbers are in `docs/development.md` › The live viewer). (3) Edges and sketches are lines over the image: on the direct path drawn by the GPU (§7.2), on the other paths drawn by egui in vector form, each cut against the renderer's depth buffer pixel by pixel; either way the part of an edge behind a body is left out, the part of a sketch behind a body is drawn dimmed, the world axes are always drawn. (4) Open and save: the shell only asks (`FileAction`); the application shows the system's dialog (`rfd`, on its own thread) and runs `file.open` or `file.save` with a grant for the chosen file's directory, which the control channel cannot reach ([`control-protocol.md`](../control-protocol.md) › File access). `automation::bridge` is an MCP server that passes every message to that window (`mcp.message`). (5) The controls over the viewport (2026-10-16: the glass bar and the section plate with switches, segmented choices and Blender-like number fields, the view cube as rounded faces, capsules and corner circles with its names lying in the faces' planes as egui glyph meshes moved by each face's affine map, always without perspective, and the axis triad; [`development.md`](../development.md) › *The view cube and the controls over the viewport*) are egui widgets painted over the picture on every path, never by a renderer, so there is nothing of them for the CPU and GPU paths to agree on and they are not in `ui.screenshot`; zooming at the pointer finds the point under it with the same `Scene::pick_cut` a click uses, whichever renderer drew the picture.
- **One light theme, Mission (2026-10-21, the developer's choice; design in `target/work/theme/`, proposal D):** `theme::Tokens::mission()` is the one theme and `Tokens::default()`; the dark theme of phase 1 is gone. Soft-white panels, black chrome (menu bar, tabs, status bar), black plates over a light warm-grey viewport (`viewport_background` #EBE9E5), a light view cube without lines (the developer's choice after an outlined one read as a wireframe: `view_cube_base` off-white, every patch, face, capsule or corner, shaded by one rule, how it is turned from `view_cube_light`, the top lightest; dark names, a solid gold patch under the pointer, the soft shadow under it), edges in a dark warm grey (`edge` #58544E, softer than the design's #262422 so the shading carries the form; since the same day a body's edges are a shade of the body's own colour and `edge` is only the section cap's outline, see *Edges that read on every body*), gold for what is on (`accent` #E0A93E, black `knob` ink on it, `nav_on_opacity` 255), copper for the face highlight (#D9862C) and the selected tree row. New tokens: `plate_fill`, `plate_text`, `plate_text_dim` (the plates, which were `panel_fill`), `view_cube_base`, `view_cube_fade_full` (and `view_cube_glass_dim`, `view_cube_edge_opacity`, `view_cube_corner_opacity` are gone), `menu_fill`, `status_fill`, `chrome_text`, `chrome_text_dim`, and `chrome_ok`, `chrome_failed` (the last command in the status bar, which `ok` and `failed` do not read on at 4.5:1 on black). Nothing assumes a dark UI any more: `Tokens::apply` takes egui's light or dark visuals from how light `panel_fill` is (`theme::is_light`), the menu and status bars take theirs from `menu_fill` (`Tokens::chrome_visuals`), and a number field being typed into has its selection and cursor set for its black box. The plates are drawn at 240 of 255 so their light text keeps its contrast over the light viewport. A patch of the cube turning edge-on no longer vanishes at a hard cut of 0.25 (the developer: from the front with a small tilt the top and the side were gone): it is drawn down to 0.03 (`view_cube_fade`, against sub-pixel slivers) and fades in smoothly to full at 0.2 (`view_cube::patch_opacity`, `view_cube_fade_full`); a click still goes to the nearest drawn patch; the names keep their own `view_cube_label_hide`. `theme::tests` checks every text against the fill it is drawn on at 4.5:1 or better (`Tokens::text_pairs`; a plate is measured as it is blended over the viewport). The CPU/GPU parity tests run in this theme unchanged; the tests that pinned the cyan highlight, the dark background or an edge's ink on a face in shade were changed as expectations (dev log of 2026-10-21). Owed: a grey edge is hardly seen on a face in deep shade (`tests/thin_members.rs` draws its edges red to measure where they are hidden).
- **A second, cool accent: indigo (2026-10-24, the developer's choice from a mock-up of the start screen).** `accent` (amber) stays what acts: a button, the active tab, a toggle that is on. `info_accent` (#4C5AA6) is information and connection: the hover ring of an example on the start screen (thumbnails now rounded by `thumb_radius`), a 3-point bar before the start screen's headings and beside the *Connect an AI agent* block (`info_bar_width`). `info_ink` (#3C4890, 7.7:1 on the panels) is that hue as text: egui's `hyperlink_color` (the recent files, *How it works*, the About dialog's links to the guide and the source), which was egui's default blue, and `selection.stroke`, which is a text field's focus ring and also the colour of selected text (4.9:1 on the copper selection fill). `info_code` (#A9B3EC) is its light tint on a black plate: the program's name in the agent block's command lines; it is also the chrome's link colour, should a link appear there. The bars' corner radius is `info_bar_radius`; a heading's bar is as high as the body text's font size. `theme::tests` checks the three against what they are drawn on. Not done: an *agent connected* mark, since the shell is not told when a control client connects (the control server counts its connections, `apps/craft3d/src/control.rs`, but nothing hands that to the shell). The example scripts end with a `body.appearance` each, so the thumbnails, the guide's pictures and a click on the start screen show them in colour.
- **Rendering:** `render` is the CPU reference renderer (perspective and orthographic, depth buffer, smooth two-sided shading, id buffer, supersampling, PNG), with cameras that frame a box in perspective, orthographically, or as a close-up from outside a larger model, and zoom. It draws no edges itself (the viewer draws them over its image, tested against its depth buffer; `view.render` has none yet), and clips triangles to the GPU's near plane (since 2026-10-31, *Near-plane clipping* below). `gpu` is the wgpu renderer for the same inputs and the same result (`Frame`: colour, face ids, view distance), offscreen (`Offscreen`) or on a device and queue someone else owns (`Renderer`), and, as `Onscreen`, the viewport's renderer on the window's own device: the picture multisampled, the lines over it drawn and depth-tested on the GPU, into a texture that is shown without any readback; it matches the CPU renderer within the tolerances derived in §7.2, see-through bodies included (two depth layers, since 2026-10-14), and the desktop app's viewport draws with it when there is an adapter, with the readback path and the CPU renderer as the fallbacks (see *Desktop app and shell*).
- **Pointing (2026-10-13, faces):** a position in a picture gives the persistent name of the face under it, and a name is shown highlighted. Three layers, one answer. (1) `render::pick` is the CPU renderer's own arithmetic for one sample (the same projection, edge functions, depth key and tie rule as `render`, and it skips the triangles `render` skips), so at a pixel centre it returns the face `Frame::ids` holds there, with the point of the triangle that projects onto the position; it walks the triangles once and allocates nothing. (2) The engine's `Scene` carries the names: `SceneBody::face_names` and `face_aliases` are `faceNames` and `faceAliases` of `document.inspect`, face `k` of a body having id `first_face_id + k` in `Scene::mesh`; `Scene::pick` turns a hit into feature, body, name and point, and `Scene::faces_named` turns a name (with the feature when several bodies have it) into face ids. The command `view.pick` (`engine/src/pick_cmds.rs`) asks it of the scene and camera `view.render` uses (the framing is shared: `commands::Framing`), or of a camera given. (3) The shell asks the same of the scene it draws with the viewport's camera: a click (`Shell::pick`, also `ui.pick`) sets `UiState::picked` and `UiState::highlight`, and `ui.highlight` sets the highlight by name. **How the GPU viewport gives the same answer:** it does not read the GPU's id target back. The pick is computed on the CPU from the scene the viewport draws and the camera it draws with, and the GPU's picture has the same face at that pixel because its face ids equal the CPU's outside the band of §7.2 (that is what the parity tests assert); on a silhouette pixel the answer is the CPU renderer's. **The highlight** is `render::Highlight` in `RenderParams`: up to eight runs of face ids and a base colour (`theme::Tokens::face_highlight`). Both renderers shade those faces from that colour instead of the body's, with more ambient light (`SHADE_HIGHLIGHT_AMBIENT` 0.6, `SHADE_HIGHLIGHT_DIFFUSE` 0.4, so a highlighted face in the shade still shows; fifths on purpose, because a colour byte times a fifth is never half-way between two levels, where the GPU rounds to even and the CPU away from zero: with 0.55 the whole shaded side of a face was a level apart). On the GPU the runs are four `vec4` of the scene's uniform block compared with the vertex's face id, so a change of highlight uploads no mesh; ids are compared as floats and a highlight at or above 2²⁴ is refused (`GpuError::OutOfRange`), which sends that frame to the CPU. The highlight is part of the frame key of every path (preview, sharp image on the render thread, the direct path, `ui.screenshot`). Tests: `crates/render` (pick against the id buffer, exactly), `crates/gpu/tests/parity.rs` (16 highlighted scenes per adapter under the tolerances of §7.2), `crates/engine/tests/pointing.rs` (the acceptance test: 13 generated views of five bodies, a primitive, a boolean and patterns among them, 691 picks compared with `faceNames`; every one of the 58 faces picked from some side), `crates/ui-egui/tests/pointing.rs` and `gpu_viewport.rs` (clicks, `ui.pick`, `ui.highlight`, the direct path against the oracle). Not there: several highlights at once, a highlight in `view.render`.
- **See-through bodies (2026-10-13).** `RenderParams::opacity` (1: opaque). The CPU renderer keeps, per sample, the front fragment and the nearest fragment strictly behind it (two depth keys and two colours; a fragment that is neither is not shaded), and blends them once all triangles are drawn: `opacity * front + (1 - opacity) * behind`, the background where nothing is behind, no blend where the front face is highlighted. Why this definition: it is exact and order-independent (two minima), it needs no sorting, and a GPU can do the same with two passes (the scene, then the scene again with a fragment discarded unless its depth is greater than the first pass's at that sample) and one blend in the resolve, so the two renderers can agree under the tolerances of §7.2. Blending every layer in depth order was rejected: it needs sorted fragments or many passes, and several layers of grey read worse than one. At opacity 1 none of this runs and the frame is bit-identical to before (asserted). **The GPU half (2026-10-14)** is depth peeling with two layers, in both renderers of `gpu` (§7.2, *See-through on the GPU*): the scene is drawn twice, the second time keeping only what is behind the front, and a last pass blends the two per sample with a table of the CPU renderer's own blend. The window's device draws see-through frames itself; a device that cannot says so (`GpuError::SeeThrough`, which both viewport paths treat as "this frame another way") and the picture on screen is then the oracle's own. The overlay draws the hidden stretches of body edges dimmed for such a frame (`viewport::overlay_see_through`; on the GPU the same segments flagged *dimmed*, uploaded when the bodies become see-through or opaque again). Tests: `crates/render` (the formula on stacked squares in all six triangle orders, opacity 1 and values that are not numbers, a highlight), `crates/ui-egui/tests/see_through.rs` (`ui.set`, the screenshot against the frame, hidden edges, picks unchanged, the slider, the GPU viewport drawing the frames itself, and a device that cannot falling back and coming back); the GPU's tests are listed in §7.2.
- **Section plane (2026-10-14).** `RenderParams::section` (`None`: the whole picture, bit for bit) is a plane that cuts the picture open, to look inside a shell, a pocket or a cavity: a point, a normal that points into the half that is not drawn, whether the cut is capped, and the cap's colour (`render::Section`; `render::Cut` is the same plane made ready for one mesh). It is a view: no document, body or mesh changes, nothing is regenerated, and it costs passes of a frame, not a boolean. The definition is the CPU renderer's (`craft3d_render::render`, *Section*), per sample. (1) *The cut:* a vertex's distance from the plane is interpolated across its triangle like the position, and a sample of a triangle is left out where that is positive; a triangle that lies *in* the plane (parallel to it within a third of a degree and within 2⁻¹⁴ of the mesh's size of it) goes with its body, kept when it faces the cut half and left out when it faces away (normals point out of a body), so a plane laid on a face of a body cuts nothing and shows no speckles. (2) *The cap:* a body cut open looks solid because the plane is one more surface where it passes through material, in a colour of its own, unshaded (so that a cut reads as a cut whatever the light), with `SECTION_FACE` as its face id. The plane is inside a body at a sample when the number of surface samples left out there is odd: along the sample's ray those are the surfaces on the far side of the plane in the cut half, and a ray leaves a closed body as often as it enters it. A sample on an edge two triangles share counts for one of them (the top-left rule), and at a silhouette of the cut-away part for both or neither, which is the same parity. The cap then takes part in the depth test like any fragment, at the depth where the ray meets the plane (a triangle wins a tie), and in a see-through picture it is the front or the surface behind the front like any other. It stays within the box around the mesh. (3) *What is not closed:* the count is over all triangles together, so a mesh of open sheets is cut without a cap (`Section::cap: false`; the bodies of a document are closed solids and get one), and where two bodies overlap in space the plane through both shows no cap. Hatching was left out: it would have to be the same pattern to the pixel in both renderers. `render::pick_cut` and `Neighbourhood::new_cut` answer for the cut-open picture (a pick is what is seen; `cut_faces` says what was cut away in front of a cap, from which the engine names the body that is cut there), `Cut::clip_segment` cuts the lines over a picture and `Cut::outline` gives where the plane crosses the surface, which the viewport draws as edges. The engine resolves how a plane is *said* (`engine/src/section.rs`, `Session::section_plane`): a base plane, a datum plane, a flat face by its persistent name (its plane is read off the scene's mesh; the normal points out of the body), or a point and a normal, moved by an offset along the normal, with `flip` for the other half; `view.render` and `view.pick` take it as the parameter `section`, the window keeps it as `UiState::section` (resolved once per scene and state in `Shell::sync`; a name that no longer resolves switches it off and `ui.get` says why) and carries it to both of its drawing paths with the marks (`viewport::Marks::cut`), so the body edges end at the plane and the outline of the cut is drawn in both. The GPU half is §7.2, *Section on the GPU*.
- **Pointing at edges (2026-10-13):** the same three layers. (1) `render::Viewpoint` projects a point the way the renderer projects a vertex, and `render::Neighbourhood` keeps the triangles that can be under a position within some pixels of another, so that many positions close together are looked up (`Neighbourhood::pick`, the arithmetic of `render::pick`, shared code) for one pass over the mesh. (2) `SceneBody::edge_names` and `edge_faces` are `name` and `faces` of `document.edges` for the polylines of `SceneBody::edges` (one per edge of the B-rep, in its order). `Scene::point_at` is the rule ([`control-protocol.md`](../control-protocol.md), *Pointing*): the face under the position, and the nearest point in sight of any edge within the pick radius; a point is in sight when the surface the picture shows where the point lands is not nearer than the point (1e-7 of its distance plus the scene's size of slack: the points of an edge are points of the mesh, so this is rounding). `Scene::edges_named` turns an edge's own name, or two face names, into the edges of one body. (3) The shell's click and `ui.pick` call `point_at` with the viewport's camera; `UiState::picked` says which kind was hit and `UiState::highlight_edge` (an `EdgeRef`: own name, or the faces) is the edge shown. **The edge highlight is a line of the overlay**, not a shading: `viewport::Marks` holds the pieces of line of the highlighted edges, `viewport::overlay_marked` (CPU paths, `ui.screenshot`) and `direct::segments_marked` (the GPU's direct path) add them after everything else as `LineKind::Highlight`, depth-tested and dimmed where hidden, so the two paths agree on it exactly as they agree on every other line (§7.2). Cost: a pick is two passes over the mesh (the face, the neighbourhood) however many edges are near. Tests: `crates/engine/tests/pointing_edges.rs` (10 generated views of the five bodies: 1185 points of edges in sight picked and compared with `document.edges`, 776 points of hidden edges not picked through a face; a box alone against the closed form for a convex body; the radius measured; the nearest-edge rule on 2552 positions against a brute-force search; visibility there is a ray cast in space, another route than the pick's), `crates/ui-egui/tests/pointing_edges.rs` (37 edges clicked, `ui.pick`, `ui.highlight`, the screenshot, the direct path against the oracle along the highlighted lines).
- **Pointing at vertices (2026-10-13):** `SceneBody::vertices`, `vertex_names` and `vertex_faces` (the faces of the edges that end in a vertex) are what `document.edges` lists under `vertices`. `Scene::point_at` looks for the nearest vertex in sight within the radius before it settles for an edge (rule 4 of *Pointing* in [`control-protocol.md`](../control-protocol.md)), `Scene::vertices_named` turns a vertex's own name, or the names of faces that all meet in it, into the vertices of one body, and `UiState::highlight_vertex` (a `VertexRef`) is drawn as a dot of `viewport::Marks::points`, by `overlay_marked` and `segments_marked` alike. Tests: `crates/engine/tests/pointing_edges.rs` (the same 10 views: 461 picks on vertices in sight give the vertex `document.edges` lists, 156 hidden ones are not picked; a box against the closed form, seven corners of eight; vertex against edge and face at measured distances), `crates/ui-egui/tests/pointing_edges.rs` (clicks, `ui.highlight {vertex | corner}`, the dot on the direct path against the oracle).
- **Demo:** `cargo run --release -p craft3d-render --example teapot` renders the Utah teapot (32 patches) to `target/evidence/teapot.png`.

---

### 7. Rendering

Display is the one place with two implementations.

#### 7.1 Draw list

`render` and `gpu` consume the same data: meshes and edge polylines from `tess` with entity ids, a camera (orthographic or perspective), lights, a display mode (shaded, shaded with edges, wireframe, hidden-line later) and per-entity colours and selection state. Nothing about display leaks into `kernel` or `tess`.

#### 7.2 Backends

- **CPU (`render`).** The reference implementation: a software rasteriser with a depth buffer, flat/smooth shading, edge overlay, and an **id buffer** (which face or edge is under each pixel). Used by tests, the CLI (`craft3d-cli render`), MCP previews, `ui.screenshot`, and as the fallback in the viewport when no GPU is available or asked for (`--renderer cpu`). It favours exactness and readability over speed.
- **GPU (`gpu`).** wgpu (Vulkan, Metal, DX12, OpenGL, WebGPU) pipelines for the same draw list; picking reads the id buffer. The scene pass draws one sample per pixel into targets `ssaa` times larger per axis (an `Rgba8Unorm` colour target, an `Rg32Uint` target with the face id and the bits of the view distance, a `Depth32Float` buffer with near and far fitted to the mesh per frame, its depth reversed: see *Reversed depth* below); a resolve pass takes the rounded integer mean of the colour samples and the first sample's id and distance, exactly as the CPU renderer ends. Positions go to the GPU in `f32` relative to the centre of the mesh; the camera is computed in `f64` and narrowed last. That is the crate's `Offscreen` and `Renderer`, which read the frame back. **The viewport of the desktop app draws with the crate's `Onscreen`** (2026-10-13) on the window's own wgpu device, into a texture egui shows as it is: nothing is read back for display. Its scene pass draws the same shader into a colour target and depth buffer with 4 samples per pixel (MSAA; 1 where the adapter has no multisampling for the formats, and on wgpu's OpenGL backend, which resolved nothing out of a multisampled attachment on the development machine), shading once per pixel at its centre, exactly where the CPU renderer samples for `ssaa = 1`; a second pass draws the lines over the picture (the edges of the bodies, the world axes, the sketches, the sketch points) as screen-space quads inked by the distance of the pixel's centre to the segment (the CPU viewport's `draw_line`), each tested in the fragment shader against the depth buffer with the CPU viewport's rule (`depth_test.wgsl` is `viewport::surface_hides`, see *Edges behind thin members* below: the surface under the segment's point reconstructed from the four samples around it, an allowance for a kink between them, and the slack the GPU frame gets; a hidden edge is left out, a hidden sketch drawn dimmed), and resolves the samples into the shown texture. A third pass, only when asked, draws face ids and distances at one sample per pixel (for the tests, and for picking later). Two differences are known and documented in the crate (a third, a triangle that crosses the eye plane clipped by the GPU and dropped whole by the CPU renderer, is gone since 2026-10-31): a triangle with a corner that does not fit in `f32` is not drawn, and a sample exactly on an edge follows the GPU's top-left rule.
- **Reversed depth** (2026-10-14). The depth the GPU stores is 1 at the near plane and falls towards 0 far away, and every depth test keeps the greater (`Greater`, cleared to 0; on a tie the triangle drawn first stays, as before). In perspective it is `near / dist` (no far plane: the clip `z` is the near distance and `w` the view distance), in orthographic `(far - dist) / (far - near)`; the lines and the datum fills turn it back into a distance as `near / z` and `far - z (far - near)`. Why: near and far are fitted to the box around the mesh, and with the eye inside that box (zoomed into a building) the near plane must sit right in front of the eye, at 10⁻⁶ of the far distance. With the depth rising from 0 at the near plane to 1 at the far one, as it did, all the precision of a float near 0 sits at the near plane and a step of the buffer is `2⁻²⁴ · dist² / near` of distance: half a metre at 20 m in a 50 m model, so a screed 4 cm above its slab fought with it in stairs along every edge (measured: 574 face ids wrong outside the band on Vulkan, 833 on OpenGL, 22097 on lavapipe in `tests/depth_precision.rs`, against the CPU renderer). A float holds `near / dist` to the same relative precision at every distance (`2⁻²⁴ · dist`), so the near plane costs nothing further on and the same scene matches the CPU renderer within the tolerances of every other picture.
- **Parity tests** (`crates/gpu/tests/parity.rs`; the derivations are in `crates/gpu/tests/common/mod.rs`). 75 scenes (the Utah teapot, the sixteen reference solids of the test kit tilted at scales 1e-3, 1 and 1e6 and once ten million units from the origin, and hand-made awkward meshes), in perspective and orthographic, at supersampling 1 to 4, on every adapter of the machine. **The documented tolerance** is:
  - *The band.* The GPU's rasteriser snaps projected vertices to a grid of 2⁻⁸ pixel, so the renderers can disagree about a sample only within 2⁻⁸ pixel of a triangle's edge, and that shows only where the surface under the sample changes. The band is every pixel within one pixel of a projected *feature edge* (a face's border, an edge with one triangle, a silhouette edge) or of a change of face id in the CPU frame. It is computed from the mesh, the camera and the CPU frame, never from the GPU frame, and must leave more than half of the picture.
  - *Face ids:* equal outside the band. Inside it at most 1/128 of the band's pixels may differ (the probability that an edge passes within 2⁻⁸ pixel of a pixel's centre).
  - *Distance* outside the band: within `2⁻²⁰ · reach + 2⁻⁷ · G`, where `reach` is the eye's distance to the mesh centre plus the mesh diagonal (sixteen `f32` roundings of 2⁻²⁴ on numbers that large) and `G` is the largest difference to the eight neighbouring pixels in the CPU frame (the gradient, times one grid step, doubled).
  - *Colour* outside the band: at most 1/255 per channel, with no exceptions (both renderers round per sample; a value within the `f32` error of a half-integer rounds the other way), plus `2⁻⁷ · Gc` levels where the CPU picture itself changes by `Gc` between neighbours, which is a second level only next to a jump of 128 or more. The number of pixels that differ by that one level is bounded too, by the sum over the pixels of `2 · (2⁻⁷ · Gc + 0.004)`: a systematic error such as a truncated mean exceeds it. Inside the band at most 1/128 of the pixels may differ by more than a level.

  Measured on the three adapters of the development machine (Intel Vulkan, lavapipe, Intel OpenGL): no id differs outside the band, at most 0.07 % of a scene's pixels outside the band differ by one level and none by two, the largest distance error is 39 % of its tolerance, and at most 0.14 % of a band's pixels differ. The tests **fail** when no adapter is found, with the way to get one (`mesa-vulkan-drivers` gives lavapipe); they do not skip.
  - *The multisampled picture* (`crates/gpu/tests/onscreen.rs`, `compare_msaa` in the same `common/mod.rs`) is compared with the CPU frame at `ssaa = 1`, with the ids and distances of the capture pass, under the same tolerance with two exceptions that follow from multisampling: inside the band the colour is not compared at all (the samples lie on both sides of a feature edge and the resolved colour is their blend: the anti-aliasing; the number of such pixels is measured and must be more than a twentieth of the band), and on a *seam* (within a pixel of an interior triangle edge of one face) a pixel may mix two fragments, each shaded at the pixel centre from its own triangle's interpolated normal, so it is allowed `1 + Gc` levels, a pixel of the CPU picture's own gradient. Measured on the development machine: away from every edge not one pixel of the teapot or the block differs by a level, at most 6 seam pixels of a scene differ by more than a level, and about a tenth of the band is blended. The lines are checked on a block with lines behind, before, on and beside it (dimmed, left out, whole), and `crates/ui-egui/tests/gpu_viewport.rs` draws the viewport's own scene through the shell on a device made as eframe makes the window's and compares the shown picture with the CPU picture (bodies and inked lines) the same way, sampling every line of the overlay to check that the GPU hid, dimmed and drew what the CPU decided: 1812 of 1812 samples agree.
- **See-through on the GPU** (2026-10-14; `crates/gpu/src/peel.rs`, `peel.wgsl`, `peel_samples.wgsl`, `blend.wgsl`). `RenderParams::opacity` below 1 is drawn with two depth layers, as the CPU renderer defines it (*Current state*, *See-through bodies*): the front surface blended over the nearest surface strictly behind it. At opacity 1 (and at an opacity that is not a number) none of it runs: the passes, pipelines and targets are the ones of the opaque picture, and the frame is that picture bit for bit (asserted on every adapter).
  - *The passes.* (1) The front: the opaque scene pass with another fragment entry, whose alpha says whether the sample is seen through (1 for a body's face, 0 for a highlighted face; the target is cleared to alpha 0, so the background is not blended either), and whose depth buffer is kept. (2) The one behind: the same triangles into a colour target and a depth buffer of their own (`Greater` on the reversed depth), each fragment dropped unless it is behind the front. (3) The blend: one triangle over the picture that reads both layers per sample, blends them and writes the mean of the samples (the supersamples of the readback renderer, in place of its resolve pass; the multisamples of the viewport, in place of the hardware resolve), with the frame's ids and distances taken from the front as ever. Then, in the viewport, the lines: straight onto the shown texture at one sample (they are inked by distance, not by coverage), tested against the front's depth buffer as over an opaque picture. Per frame that is one more pass over the triangles and one more over the pixels; the extra targets (a colour layer and a depth buffer behind, and in the viewport a front layer and its keys, all with the samples of the renderer) are made for the first see-through frame of a size.
  - *The comparison of the layers, and its bias: strict, and none.* The front is the nearest fragment at its sample, so every other fragment that covers the sample is at or behind it: all that has to be told apart is the front itself (and what ties with it) from the rest. With one sample per pixel (the readback renderer; the viewport on OpenGL) the second pass reads the front's depth buffer as a texture and keeps a fragment when its own depth is less (further, with the depth reversed). Both passes rasterise the same triangles with the same vertex stage on the same device, so a triangle's depth at a sample is the same number in both and a surface is never its own "behind"; a bias would only hide a surface that is really there. Two surfaces closer together than `Depth32Float` resolves over the frame's depth range (about 10⁻⁷ of it) have equal depths and then nothing is behind the front, where the CPU renderer in `f64` still sees the second: measured on a wall 10⁻⁹ of its width thick, every pixel sees past both sheets on all adapters, and a wall 10⁻⁴ of its width thick shows its other side as on the CPU.
  - *With several samples per pixel* a fragment is shaded once per pixel and the depth test runs per sample on depths the shader does not see, so that comparison is not available: the fragment's own depth is the triangle's depth at the pixel centre. Making the pass run per sample does not help: **measured on Intel's Vulkan driver (Mesa 25.3.6), a per-sample fragment's depth differs from the depth buffer by far more than rounding** (every body pixel showed its own front as "behind"; lavapipe agreed with its depth buffer). The viewport therefore stores, next to the front's colour, a *key* of the triangle in front at each sample: the bits of its depth at the pixel centre, as the four bytes of an `Rgba8Unorm` target (the one format a multisampled viewport needs anyway; a byte goes in and comes out exactly). The second pass computes the same key for its own triangle (same triangle, same pixel, same arithmetic) and drops, through the fragment's sample mask, the samples whose front has that key; the depth test then keeps the nearest of what is left, per sample and exactly. Two triangles with the same corners have the same key, so a face that coincides with the front is not behind it, as on the CPU (asserted). No multisampled depth texture is read by the peeling, only by the lines as before, so the OpenGL limitation of the decisions document changes nothing here: on OpenGL the viewport has one sample and takes the first path.
  - *The blend is the CPU's own.* `opacity · front + (1 − opacity) · behind`, rounded, is a function of two bytes; the renderer tabulates it in `f64` with the CPU renderer's expression (a 256 × 256 `R8Unorm` texture, written when the opacity changes) and the blend pass looks it up per channel. So a sample whose two layers equal the CPU's has the CPU's colour exactly, also where the exact blend lies half-way between two levels (at opacity 0.1 that is a tenth of all pairs of levels) and `f32` would round the other way.
  - *The tolerance for a see-through picture*, derived in `crates/gpu/tests/common/mod.rs`, is the one above with two layers in place of one. Both layers are known from the CPU renderer alone: the front picture is its frame at opacity 1, the picture behind its frame at opacity 0. *The band* also holds every pixel within one pixel of a change of the face behind the front (where two surfaces behind the front pass through each other, which no mesh edge and no id of the front shows; the test finds that face from the definition with a small rasteriser of its own, for the band only). *Colour* outside the band: each layer is within its own `1 + ⌊2⁻⁷ · G⌋`, the exact blends therefore within the larger of the two, and rounding is monotone and commutes with adding an integer, so the blended levels differ by at most `1 + ⌊2⁻⁷ · max(G_front, G_behind)⌋`. *How many* differ at all: the probabilities of the two layers add. Ids and distances are the front's and keep their tolerances. Nothing was loosened: the same per-pixel limit, the same counted bound.
  - *Measured* (three adapters: Intel Vulkan, lavapipe, Intel OpenGL; `parity.rs`, 114 see-through scenes per adapter: the sixteen reference solids at opacities 0.1, 0.5 and 0.9, their three other rounds of scale and distance at one opacity each, the teapot, a plate with a hole and a body inside a hollow body at all three, a third of them with highlighted faces): no id differs outside the band, no colour by two levels, at most 0.11 % of a scene's pixels outside the band by one level (never more than 92 pixels), the largest distance error 23 % of its tolerance, at most 0.15 % of a band's pixels differ. The viewport's picture (`onscreen.rs`, nine scenes per adapter, four samples on Vulkan): not one pixel away from every edge differs by a level, at most one seam pixel by more than a level. `crates/gpu/tests/see_through.rs` holds the scenes where the comparison of the layers bites, for both renderers: one sheet in four views (nowhere its own "behind"), a wall of two sheets 10⁻² and 10⁻⁴ of its width apart, two sheets 10⁻⁹ apart, a face corner for corner on the front, a box seen 2° above its top face, lines over a see-through picture (hidden and dimmed, hidden and left out, in front), hostile opacities and a renderer told it cannot. `crates/ui-egui/tests/gpu_viewport.rs` draws the window's own scene see-through on a device made as eframe makes the window's: 0 of 408 223 pixels away from edges and lines differ, and 1769 of 1769 samples along the lines agree (a hidden, dimmed edge lies on a feature edge of what is behind the front, where a multisampled pixel mixes the faces that meet there: such a sample is compared with the dimmed ink over the range of the CPU picture's 3 × 3 pixels around it).
  - *A finding about the oracle, and the rule that removed it (2026-10-14).* The CPU renderer has a sample that lies exactly on an edge two triangles share inside both (its edge function is inclusive on both sides, so that a closed surface has no pinholes), and their depth keys there agree only up to rounding; where the second came out a hair greater, the surface was its own "behind" and the sample was drawn opaque. A square of two triangles seen head-on at 240 × 180, with pixel centres on its diagonal, had 8 such pixels in the CPU's see-through picture and none in the GPU's (whose rasteriser gives such a sample to one triangle). **The rule:** the triangles that share an edge or a vertex are one surface at a sample that lies exactly on it. The one in front there is the front as ever (nothing about the opaque picture changes: the same triangle wins, by the same comparison); the others are not behind it, and when one of them takes the front over, the old front does not become the one behind. "The same edge" is the coordinates of its two ends in space (of the vertex, for a sample on a vertex), kept per sample of a see-through picture as one number (`render::shared_place`). What changed: see-through pictures, only on such samples (the 8 pixels of that square are now the blend the other 14 392 are; the 30 see-through frames of the reference solids and the teapot that were hashed before and after are bit for bit the same, as are all opaque ones), and `a_surface_is_never_behind_itself` now compares that view like every other, on the CPU too. *Not taken: the top-left rule in the CPU rasteriser* (a sample on an edge belongs to one triangle outright, as on the GPU). It was tried: every existing test stayed green and every hashed picture the same, but by its definition it also drops a sample that lies exactly on the right or bottom edge of a mesh's outline, which changes opaque pictures at samples no two triangles share, and the oracle was not to change there; it remains the difference documented in `crates/gpu/src/lib.rs`. The count of a section's cut-away fragments does use the top-left rule: a count needs each surface once, and at an outline both rules give the same parity.
  - *What is refused.* A device on which the pipelines of the layers cannot be made keeps drawing opaque pictures and answers a see-through one with `GpuError::SeeThrough` and the reason; the viewport then draws that frame with its other renderer (`ui.get` › `render.renderer` says which). `Onscreen::without_see_through` and `Renderer::without_see_through` put a renderer in that state for the tests of the fallback. None of the adapters this was built on is in it.
  - *Cost* (release, Intel integrated graphics, the frame until the GPU is done, `examples/viewport_cost.rs`): at 1020 × 777 with four samples 1.7 ms opaque and 4.7 ms see-through for 12 800 triangles, 12.5 ms and 19.0 ms for 665 000; at 2560 × 1400, 5.0 and 9.1 ms, 15.8 and 25.7 ms. The CPU renderer takes 176 to 227 ms for the same see-through frames at 1020 × 777.
- **Section on the GPU** (2026-10-14; `crates/gpu/src/cut.rs`, `cut.wgsl`, `cut_flat.wgsl`, `cut_samples.wgsl`). `RenderParams::section` is drawn as the CPU renderer defines it (*Current state*, *Section plane*). Without a section none of it runs and none of it is built: the pipelines of a cut-open picture are made for the first one, the passes, entries and targets of a whole picture are the ones they were (`vs_main` and the fragment entries are untouched; the scene's uniform block grew by four vectors), and the frame is bit for bit the one before this was built (30 scenes, opaque and see-through, hashed on the CPU and on three adapters before and after: 120 of 120 equal; and asserted per adapter: the frame after a cut-open one equals a fresh renderer's).
  - *The cut* is a fragment `discard`: the vertex stage of a cut-open picture (`vs_cut`) adds the vertex's distance from the plane as one more interpolated value, and every fragment entry has a twin that drops the fragment by `Cut::clips` (the triangle's own normal, already a flat vertex attribute, says whether a triangle in the plane goes with its body).
  - *The count, and why it is a blend and not a stencil.* The cap needs, per sample, whether the number of fragments left out there is odd. The depth buffer is `Depth32Float` on every backend and has no stencil, and a stencil format beside it would be a second depth-stencil attachment, which no backend has. So the count is a pass of its own over the triangles, before the others and without a depth test, that keeps only the fragments that are cut away and writes them into an `R8Unorm` target whose blend is "one minus what was there" (source factor `OneMinusDst`, destination factor `Zero`): 0 and 1 are exact in a normalised byte, and the blend runs per sample where the target is multisampled. It is one more pass over the triangles per frame, with no shading.
  - *The cap* is one triangle in the plane, large enough to hold the box around the mesh (grown by the margin the cap may lie in, which the depth range of a cut-open frame is fitted to as well), drawn after the triangles of the mesh in every pass that draws them, through the same vertex stage and the same depth test (`Greater` on the reversed depth: a triangle of the mesh wins a tie, as on the CPU). Its fragment is kept where the count is odd and the point is within the box; its colour is the cap's as it is, and its distance in the frame is the distance of the point of the plane (not the screen-interpolated one of a triangle that large). One triangle, not a quad: no diagonal for a sample to lie on.
  - *With one sample per pixel* (the readback renderer, whose supersamples are pixels of a larger target; the viewport on OpenGL) the cap reads the count at its pixel. *With several* (the viewport: 4 on Vulkan) a fragment is shaded once per pixel, so whether it is cut away is decided at the pixel's centre for all of its samples, in the pass that counts and in the pass that draws alike (the same triangle, the same pixel, the same arithmetic: the two never disagree); the count is then per sample of the fragments left out there, and the cap takes the samples where it is odd through its sample mask, so a silhouette keeps its anti-aliasing and the picture has no hole between a wall and the cap. Along the cut itself the decision is per pixel, not per sample: that line is where the outline of the section is drawn as an edge.
  - *Seen through*, the cap is a surface of both layers, as on the CPU: in the front pass it is drawn with alpha 1 (seen through like a body's face) and, with several samples, with its key; in the pass behind the front it is kept where the count is odd *and* it is behind the front, by the comparison of §*See-through on the GPU* (its depth against the front's with one sample; its key against the front's with several: the cap's triangle goes through the same vertex stage in both passes, so it is never its own "behind").
  - *The viewport's lines* are cut on the CPU before they are uploaded (`viewport::Marks::kept`: an edge of a body ends at the plane; `Cut::outline` adds the outline of the cut as edges), and the cap is in the depth buffer they are tested against, so a line behind the cap is hidden like behind any face (asserted on a block: a line inside the body behind the cap is not drawn, a line in the cut half is).
  - *The tolerance for a cut-open picture* (derived in `crates/gpu/tests/common/mod.rs`, *Cut-open pictures*) is the one above, not loosened. The renderers can decide a cut differently only within the `f32` error of a distance from the plane (a few 10⁻⁷ of the mesh's size), which on screen is a strip along the line where the plane crosses the surface: **the band** also holds every pixel within one pixel of the projected outline of the section, which is known from the mesh and the plane alone. A miscount changes the parity only at that outline and at an edge with one triangle (at a shared edge a sample is in exactly one of the two triangles in either renderer; at a silhouette of the cut-away part in both or neither), so outside the band the cap's id is equal, its colour is equal exactly (unshaded: asserted byte for byte, the tolerance of one level is not used) and its distance is within the tolerance of any surface; inside the band the same 1/128 holds.
  - *Measured* (three adapters: Intel Vulkan, lavapipe, Intel OpenGL; `crates/gpu/tests/section.rs`). 322 cut-open scenes per adapter (the sixteen reference solids and two more, each across x, y and z at two offsets and across an oblique plane, opaque and see-through at opacities 0.1, 0.5 and 0.9, every other one with highlighted faces; the same solids at scales 10⁻³ and 10⁶ and ten million units from the origin; a block with a cavity and a plate with a drilled hole, both made by a boolean; the teapot without a cap and with one): no id differs outside the band, no colour by two levels, at most 26 pixels of a scene by one, the largest distance error is 22 % of its tolerance, and inside the band at most 2 ids and 5 colours of a scene differ. The viewport's picture (18 scenes per adapter, four samples on Vulkan): at most 7 pixels away from every edge differ by a level; on seams at most 40 by more than a level, all within the seam's tolerance, in one scene (the teapot with a third of its faces highlighted). *The cap's area* in an orthographic view along the normal, against the closed form of the cross-section (400 × 400 pixels; the allowed error is the pixels the outline passes through): a block across x, y, z and obliquely (6, 8, 12 and 3·√8) within 0.8 %, a cylinder across (π r²) within 0.013 %, a ball at distance d from its centre (π (r² − d²)) within 0.054 %, a hollow block (the frame, 24 − 8) within 0.25 % and through its wall (24) within 0.1 %; the CPU and every adapter count the same number of pixels in all eight.
  - *What is refused.* A device on which the pipelines of a cut-open picture cannot be made keeps drawing whole pictures and answers one with a section with `GpuError::Section` and the reason; the viewport then draws that frame with its other renderer, as for `GpuError::SeeThrough`. `Onscreen::without_section` and `Renderer::without_section` put a renderer in that state for the tests of the fallback. None of the adapters this was built on is in it. A plane whose numbers do not fit `f32` relative to the mesh is `GpuError::OutOfRange`, which the viewport treats the same way.
  - *Cost* (release, Intel integrated graphics while other work ran on the machine, the frame until the GPU is done, `examples/viewport_cost.rs`): at 1020 × 777 with four samples, 12 800 triangles: 3.8 ms whole, 7.2 ms cut open, 8.3 ms see-through, 16.1 ms both; 665 000 triangles: 26.6, 35.6, 48.1 and 58.0 ms; at 2560 × 1400: 34.1, 50.8, 79.2 and 99.5 ms. Moving the plane costs, besides the frame, its outline on the bodies and the lines over the picture made again: 0.2 ms for 12 800 triangles, 11 ms for 665 000. In the window (impeller, 105 000 triangles, 1020 × 754): the drawing thread spends 0.33 ms per frame on the whole picture and 0.35 to 0.47 ms on a cut-open one while the camera turns, and about 2 ms per frame while the plane moves. The CPU renderer takes 112 to 226 ms for the same cut-open frames at 1020 × 777 with four samples (a cut-open frame is cheaper there than a whole one: what is cut away is not shaded).

- **Colours of their own** (2026-10-15; `craft3d_render::FaceColors`, `render_colored`; on the GPU `Renderer::upload_colored`). A face with a colour of its own (a body's appearance, §4.2) is shaded from it instead of `RenderParams::color`, in exactly the same way (light, ambient term; a highlight wins over it); with no colours the picture is the old one bit for bit. On the GPU the colour travels with the mesh as one more vertex word (`VERTEX_BYTES` 44: red, green, blue and a flag byte; flat-interpolated), so a change of colour is a new upload, which a new document state is anyway; every pass (opaque, see-through, cut open) shades with it. Parity: the reference solids with every face, or every other face, in a palette of light, dark, saturated and grey colours, opaque and see-through, highlighted on top (`parity.rs` › `faces_in_colours_of_their_own_match_the_cpu_renderer`, the three adapters) under the tolerance above, not loosened. The metallic and roughness factors are shaded since 2026-10-22 (next).
- **Metallic and roughness** (2026-10-22; `craft3d_render::Finish`, per face in `FaceColors::set_look`). The appearance's two factors are shaded by a Blinn-Phong approximation of a metallic-roughness material, with the lights fixed to the camera (since 2026-10-24 a key and a fill light, see *Studio lighting* below). With `m` the metallic factor and `e` the highlight's exponent: intensity `ambient + (key · n·l + fill · n·f) · (1 − 0.3 m)` (one light, `ambient + diffuse · n·l · (1 − 0.3 m)`, until 2026-10-24) (`SHADE_METAL_DIFFUSE`: a metal reflects less diffusely), highlight `tint · 0.35 · k · (n·h)^e` with the tint white on a dielectric and the base colour on a metal (`255 (1 − m) + base · m`), `e = 2 / r⁴ − 2` from the roughness `r` (the Phong exponent of a Beckmann distribution of width `r²`), rounded and kept within 1 to 256 (`MAX_SHININESS`), and `k = √((e + 2) / 50)`, so that a smooth face's narrow highlight is brighter and a rough face's wide one dimmer. A roughness of about 0.45 gives the plain exponent 48 and `k = 1`. Both factors reach the renderers as bytes (0 to 255) and the exponent as an integer computed once on the CPU (`Finish::shininess`), so both renderers shade from the same numbers; on the GPU they are one more vertex word (`VERTEX_BYTES` 48: the metallic byte and the exponent above it, 0 for the plain highlight), flat-interpolated, and every pass shades with it; a highlighted face is shaded plainly. *A face without a finish is drawn bit for bit as before*: every factor the finish adds is then exactly 1 (or 255 for the tint), held by `the_plain_finish_adds_no_factor_to_the_shading` (renamed 2026-10-24; the plain shading written out in the test, against the new one over 80 802 normals, colours and the highlight's weights) and by the untouched expectations of the render and parity tests. Parity: the first eight reference solids with their faces in six finishes from a mirror to all rough, dielectric and metal and one without, opaque and seen through (`parity.rs` › `faces_with_metallic_and_roughness_match_the_cpu_renderer`, on the three adapters of the development machine, under the tolerance above, not loosened: at most 22 pixels a level apart outside the band). Picture: five spheres (plain, glossy red at roughness 0.15, matte red at 0.9, gold metal at 0.3, plain red), `target/evidence/render/col_finishes.png`.
- **Studio lighting: a key and a fill light** (2026-10-24, the developer's request: a face turned from the light was near black in every picture, the window, `view.render`, the thumbnails). One light fixed to the camera with ambient 0.2 and diffuse 0.8 became an ambient term, a key light and a fill light, all in `craft3d_render` (`SHADE_AMBIENT` 0.36, `SHADE_DIFFUSE` 0.64 for the key, `SHADE_FILL` 0.18; the directions `KEY_LIGHT` [−0.45, 0.6, 1] as before and `FILL_LIGHT` [1, −0.2, 0.4] in the camera's right, up and towards-the-viewer axes, `ViewAxes::light` and `ViewAxes::fill_light`). Intensity `ambient + (key · n·l + fill · n·f) · (1 − 0.3 m)`, the white or metal-tinted highlight from the key light alone, so a metal keeps one sharp highlight and its colour; a highlighted face gets no fill (`SHADE_HIGHLIGHT_FILL` 0: its ambient 0.6 already keeps it readable). The GPU takes the same numbers (`scene.wgsl`: the fill's direction and weight are one more `vec4` of the scene's uniform block, `SCENE_FLOATS` 76; the highlight's fill weight is `highlight_shade.z`). In the iso view a grey block's top, front and right faces are at 0.88, 0.73 and 0.57 of the base colour (they were 0.85, 0.66 and 0.26); the right face's relative luminance is 38 % of the top's, where it was 8 % (`a_grey_block_in_the_iso_view_reads_on_every_face`, which pins the three faces' pixels to closed forms worked out apart from the code). The ambient term is 9/25 so a face turned from both lights, shaded with it alone, never lands half-way between two levels (the rounding rule of the highlight's fifths). Every picture changed with it: the render and parity tests compare the two renderers with each other and with closed forms, not with stored pixels, so no tolerance moved; the example pictures, thumbnails and the guide's screenshots were made again.

- **Datums in the viewport** (2026-10-14; `engine/src/scene.rs` › `SceneDatum`, `ui-egui/src/viewport.rs` › `fill_datum_planes`, `Marks::datums`, `crates/gpu/src/fills.wgsl`). The scene carries every datum that is placed (built, not suppressed, not rolled back; a failed datum has no place and is not drawn, the tree marks it): a plane as the square `center ± u ± v`, centred on the foot of the model box's centre on the plane, half side `DATUM_PLANE_EXTENT` (0.4) of the box's diagonal, along the plane's own axes; an axis as the segment around the foot of the box's centre, `DATUM_AXIS_EXTENT` (0.6) of the diagonal to either side; a point. They are not in the box the camera frames. The window shows them when `UiState::show.datums` (default on) and hands them to both drawing paths with the marks, like the section: the outline of a plane and an axis are lines of the overlay and a point is a dot (kind `LineKind::Datum`, after the axes and before the sketches, the dot after the sketch points), tested against the bodies like a sketch (dimmed behind a body); a plane is also filled, translucent, *under* the lines: per pixel, the ray through the pixel's centre meets the plane inside the square, in front of the eye and not behind a body (the lines' depth test), and the pixel is blended with the datum colour at `Tokens::datum_fill_opacity`, rounded. On the CPU that is `fill_datum_planes` on the frame before the lines (`compose_with`, so the CPU viewport, the readback path and `ui.screenshot` have it); on the GPU it is a pipeline of its own in the lines pass, before the lines (`Onscreen::upload_overlay` with `Fill`s): one triangle over the picture per plane, whose fragment shader casts the same ray at the pixel's centre in camera coordinates and tests the same depth rule as the lines; a fragment covers all samples of its pixel, so a multisampled pixel resolves to the one value. Without datums nothing of it runs and the picture is the one before, bit for bit (asserted on the CPU; on the GPU the existing parity tests run unchanged). The names are written by egui over the viewport on both paths, beside a corner of a plane, the end of an axis, a point; they are not in `ui.screenshot`. A click passes through a datum to the face behind it. *The tolerance* (`crates/ui-egui/tests/gpu_viewport.rs` › `the_direct_path_draws_datums_and_matches_the_oracle`): the bodies, lines and seams as above; away from those, the fill is compared pixel by pixel except where the plane passes through a surface (where the CPU's and the GPU's depth tests can answer differently: between the lowest and highest threshold either rule can have at the pixel, from the CPU frame's 5 × 5 neighbourhood, widened by the error of the GPU's `f32` point at its grazing angle and by the slack; since 2026-10-18 a pixel is marginal where the rule's own decision turns within that window, and within the spread of the 5 × 5 pixels around it where they are not one plane); there a fill pixel may differ by one level more than a body pixel, because how a fixed-function blend rounds into an 8-bit target is the implementation's, and such pixels are not counted against the bodies' rounding bound. *Measured* on a plate with a post, a torus and a ball, two planes (one at 35° about a datum axis), the axis and a point, opaque and see-through at 0.5, at 1000 × 700 points: about 149 000 fill pixels compared per picture and 2 994 marginal ones left out; none differs by more than a level anywhere; fill pixels that differ by a level: Intel Vulkan 189 and 276, Intel OpenGL 59 and 60, lavapipe 76 106 and 61 714 (lavapipe's blend rounds the other way at about half of them); along the lines at most 2 of about 3 260 samples differ (the test allows 1 %; where, was not looked into). With the depth test of 2026-10-18 (Intel Vulkan): 134 351 and 133 253 fill pixels compared, 1 837 marginal left out, 151 and 231 differ by a level, none by more; the test now asserts the marginal pixels are under a twentieth of those compared.

- **Edges behind thin members on the direct path; the overlay's depth test made exact** (2026-10-18; `crates/gpu/src/depth_test.wgsl`, `viewport::surface_hides`, `Frame::sample`, `crates/ui-egui/tests/thin_members.rs`). The developer saw dark lines along the legs of the Garabit pier when zooming out in the window (orthographic, about 16 mm per pixel): the edges of stiffeners 9 mm behind the legs' webs, drawn through the web; `ui.screenshot` (the CPU) showed the webs solid at the same camera, and the offscreen `snapshot --renderer gpu` (the readback path, which uses the CPU's rule on the GPU frame) did not show them either. *The cause, measured* on a synthetic leg (a 9 mm web with stiffeners touching its back, the developer's view, 1 to 30 mm per pixel, both projections; the numbers are the test's print-out): the line shader of the direct path hid a point only beyond the furthest of the nine pixels around it plus the slack *plus the spread of the nine* (their furthest minus their nearest), an allowance added on 2026-10-13 to keep edges in hollows in sight. The spread of a face that slopes across the screen grows with the millimetres per pixel (the web's nine pixels spread 15 mm at 4 mm/px, 61 mm at 16, 115 mm at 30) while the stiffener edges stay 15 to 19 mm behind the web along the view, so from 4 mm/px on the shader drew every one of them (0 of 1831 points hidden) where the CPU's rule, without the spread, still hid them all. The CPU's rule failed further out in its turn, since the furthest of nine lies up to 1.4 px of slope beyond the surface under the point: at 8 mm/px it hid 412 of 912 points, at 16 and 30 none. Neither rule looked at where in its pixel the point lies. A second cause, in perspective only: `Frame::depth` was the distance interpolated evenly across the triangle on the screen (the chord of the hyperbola the true distance follows), which on a 4 m triangle lies hundreds of millimetres beyond the surface, so the CPU showed edges that far behind a big face while the GPU, testing its true depth, hid them (at 4 mm/px the CPU hid 1 of 1657 points, the GPU 34).
  - *The rule now*, one function on both paths: the surface under the point is the bilinear interpolation of the four samples around it, which is exact on a plane since a depth buffer is affine across a triangle on the screen, so an edge any amount behind a flat plate is told from one on it at any zoom. Where the surface is not one plane within the cell the reconstruction is off, and it matters where it comes out nearer than the surface: in the hollow where two faces meet, whose crease is further than the samples on either side. There the samples show it, as the *kink*: along a row or a column of the cell whose two samples lie on either side of such a crease, both second differences (at either sample, with its neighbour beyond the cell) say the sample lies further than the chord of its neighbours by more than the slack (rounding makes no hollow; a crease nearer to a sample than that is missed by less than the slack), and the larger of the two is how far apart the two faces' planes lie at the sample across the crease from the other; the interpolation at a point misses at most the weighted sum, over the samples across the crease from it, of that (bilinear weights, summing to at most one), and a line splits the four samples so that the point's side can be taken to hold a sample next to every sample across, so the largest such pair covers it (`the_kink_covers_a_crease_between_the_samples`: 20 000 creases at random angles, slopes and places, the point on the crease never hidden). A ridge, or a change of slope the other way, makes the reconstruction further than the surface, never strict; a step to another body makes one second difference of the pair say the opposite: neither gets an allowance, so nothing behind a plate shows along the outline of what stands in front of it (the datum fill of `tests/datums.rs` stays hidden up to the post's outline), and the allowance never reaches beyond the cell. Two simpler kinks were tried and dropped: the largest second difference of all eight lets a point behind a plate show within a pixel of a step or a ridge (by the step's own height), and the smaller of each pair reaches no further than the cell but misses a steep crease that crosses the cell through its columns only. The surface's change over `SUBPIXEL_GRAIN` of a pixel (1/16: a rasteriser snaps a triangle's corners to a grid of `2^-subPixelPrecisionBits` of a pixel before it rasterises, and 4 bits is the least Vulkan allows) covers how the rasteriser places the surface; that allowance is a sixteenth of the face's depth change per pixel, so it grows with the millimetres per pixel, and the pier's stiffeners (15 to 19 mm behind a web that changes 1.5 to 1.9 mm of depth per mm of picture) would show again at about 150 mm/px, where the web is 3 px wide. *The trade-off at a step:* within the cell a nearer body's outline crosses, the reconstruction lies between the two levels, so a point on the further level there is hidden too: a line on a far face stops up to a pixel short of a nearer body's outline, and a slot narrower than about two pixels loses its bottom edges (`a_step_hides_what_a_hollow_shows` pins both sides of the step). The rule before was lenient there (the furthest of nine) and strict in hollows; the GPU's spread rule lenient in both and increasingly so when zooming out. The point is hidden when it lies further than the surface plus the kink, that change and the slack of before. A sample where nothing was drawn among the four hides nothing; a second difference that needs one is left out. On the GPU the surface of a pixel is the mean of its samples' depths (the samples lie symmetric about the centre and the depth is affine, so on a plane that is the centre's value, where the CPU's sample is; any sample of nothing makes the pixel nothing), and the point tested is the point of the segment nearest to the pixel's centre, at that point's own depth, not the pixel's centre beside the line (which on a steep face lies in front of the line by the slope). `Frame::sample` says where in the pixel a frame's sample lies (`0.5 / ssaa`; the readback renderer's first supersample, the capture's centre), so a supersampled frame places its samples right. `Frame::depth` and `Hit::depth` are now the distance of the surface point under the sample, interpolated perspective-correct like the position, on the CPU and in the GPU's data target alike (`scene.wgsl` no longer rebuilds the screen-interpolated value); the parity tests and their tolerances are unchanged and green on the development machine's adapters.
  - *Measured after* (`thin_members.rs`: the direct path on a device made as the window's, against the oracle; the pictures in `target/work/stripes/`): every point of the stiffener edges that lies on the web alone is hidden on both paths at 1, 4, 8, 16 and 30 mm/px in orthographic and at 4 to 30 in perspective (234 to 1859 points per view), the web's outline is wholly in sight, the edge in the hollow of two walls is wholly in sight at 0.25 to 4 mm/px, and the two paths differ at not one point. `gpu_viewport.rs` (the viewport's own scene, 1812 line samples, the datum fills with their marginal pixels now derived from the rule itself) stays green. Not changed: the pick's *in sight* rule (`Scene::point_at`), its own arithmetic at the exact position.
- **The depth buffer on OpenGL loses the key: the fragment writes its own** (2026-10-19; `crates/gpu/src/renderer.rs` › `fragment_depth`, `scene_source`; `scene.wgsl` › `depth_key`, `stored_depth`; `Onscreen::read_depth`, `depth_read.wgsl`; `crates/gpu/tests/depth_precision.rs`). The first gate on the Strix Halo (AMD Radeon 8060S, radeonsi, Mesa 26.2) failed `inside_a_building_lines_on_the_faces_stay_whole` on its OpenGL adapter: 309 of 1689 pixels of the lines hidden by their own face, at the far end of the screed; its Vulkan adapter (RADV), lavapipe, and the developer's Intel MTL on Vulkan and OpenGL passed. *Measured* with a readback of the depth buffer as the lines' test reads it (`read_depth`: one pass that copies the depth texture through the lines' own bind group into an integer target, since wgpu's OpenGL backend cannot copy a depth texture to a buffer), against the distance the same fragment wrote to the data target, at one sample per pixel, the eye along the screed's edge (near plane 50 µm, far 50 m, the key `near / dist` 1 to 2 millionths over the screed): on the Radeon's OpenGL every one of 82 834 covered pixels lay beyond the window's slack of 0.12 mm, the surface read 2 to 3 % *nearer* than written (−1.00 m at 43 m, −0.75 at 35, −0.54 at 30, −0.33 at 23: a constant +3·10⁻⁸ on the key, one spacing of an `f32` next to 1, halved); on the Intel's OpenGL 66 955 pixels beyond the slack, up to 0.60 m, −5 cm at 43 m, −2 cm at 30 (a smaller, per-vertex rounding). *The cause* is the backend's clip space: OpenGL's depth runs from −1 to 1 where WebGPU's runs 0 to 1, and wgpu's shader translation (naga, `ADJUST_COORDINATE_SPACE`) converts the vertex stage's `z` by `z * 2 - w` in `f32`, after which the GPU maps `z / w` back to 0..1. Our reversed key is tiny far away (`near / far`, 10⁻⁶ at the least with the eye inside the model's box, since `depth_range` clamps the near plane at a millionth of the far distance), and `2 near - dist` has only the precision of `dist` (a 43 m `dist` is spaced 3.8 µm, against a `2 near` of 99 µm: 2 % lost at the vertex), then `z / w` next to −1 is spaced 6·10⁻⁸ (2.6 % of the key): whatever key is chosen, the depth that comes out of OpenGL's conversion is exact to about 2⁻²⁵ in 0..1, and a key of 10⁻⁶ is told to 3 %, a metre at 43 m. No choice of key escapes it (one spread over 0..1 needs a near plane at the nearest *drawn* point, not at the box's corner behind the eye, and still needs `near ≥ 2⁻²⁵ · dist² / slack` = 0.55 m here), only the window-space depth written by the fragment shader does, or `ARB_clip_control`, which wgpu does not use. Intel passed by luck: the depth test allows the surface's change over a sixteenth of a pixel, 0.3 to 0.5 m where the screed's rows are metres apart along the view, which swallowed Intel's 5 cm and not the Radeon's metre. *The fix*, on the OpenGL backend only (`fragment_depth`: `device.adapter_info().backend == Gl`; the shader sources are a template): every fragment entry of the scene passes (`scene.wgsl`, the see-through and the cut-open ones) writes `@builtin(frag_depth) = depth_key(dist)`, the key of `vertex` computed from the perspective-correct distance, exact to the `f32` division; the see-through passes compare `stored_depth` (what the fragment wrote, where it did) with the front's depth instead of the rasteriser's `clip.z`. The other backends keep the rasteriser's depth: their clip space is 0..1, their depth was within the test's allowance, and writing the depth in the fragment shader costs the early depth test (the Intel's Vulkan went from 133 to 184 ms per frame on twelve dense spheres over each other at 1600 × 1000, 594 000 triangles, four samples; its OpenGL from 30 to 57; the Radeon's OpenGL from 2.0 to 2.1, within noise, its Vulkan unchanged at 1.9). The count pass of a section writes no depth (a `frag_depth` output needs a depth attachment). *Measured after* (`inside_a_building_the_depth_buffer_holds_the_fragments_distance`, the new oracle: at every pixel with four drawn neighbours the depth read agrees with the distance written within the slack plus the change over the grain; also pinned by the lines test): the Radeon's OpenGL worst 0.004 mm (f32 rounding) and 0 of 80 949 pixels beyond, 0 of 1689 line pixels hidden; the Intel's OpenGL the same 0.004 mm (0.60 m before), its id band in `inside_a_building_every_pixel_matches_the_cpu_renderer` 4 → 0; RADV worst 3.6 mm and lavapipe 0.07 mm (the rasteriser's placement, within the grain's allowance), Intel Vulkan 0.8 mm, none beyond. Everything else in `craft3d-gpu`, `craft3d-render` and `craft3d-ui-egui` green on both machines. Not changed: the multisampled peel's key stays the rasteriser's `clip.z` bits (a key, not a depth; the OpenGL renderer has one sample). Both variants of every shader composition are parsed and validated by naga in `renderer.rs` › `template_tests`, so a machine without an OpenGL adapter still checks the OpenGL shaders. Two consequences to keep in mind: with several samples per pixel a fragment-written depth is one number for all the pixel's samples (the key at the pixel's centre), so a multisampled OpenGL renderer, not reachable today (`Onscreen::new` gives OpenGL one sample), would decide the depth test per pixel centre along silhouettes and intersections; and the planned web build on WebGL2 (ES 3.0) is this backend too, where the early depth test is lost for good: conservative depth (`@early_depth_test(less_equal)`, wgpu's `SHADER_EARLY_DEPTH_TEST`) needs GL 4.2 or ES 3.1. Open: on desktop OpenGL the early depth test could be kept with that promise and a clip `z` biased in front of the key; not needed at the Radeon's numbers, worth it only if an OpenGL-only machine shows the cost.
- **Specks in the GPU's cap: the count of a section is per sample** (2026-10-19; `crates/gpu/src/cut.wgsl` › `vs_parity`, `ParityOut`, `CutOut::side`; `renderer::scene_source_for`; `crates/gpu/tests/cap_specks.rs`). When the display tolerance went on the engine's ladder the mesh of the part of `crates/ui-egui/tests/section.rs` changed by a few percent and the direct path's cap missed one pixel per cut where the CPU oracle shows the cap on the pixel and two all around (z: 4 px from the cut's outline, blended with the cavity's wall; x: 8 px inside, dark); the test's own comments had met the same stray pixel at other window sizes and display qualities since 2026-10-14. *Measured* (`speck_probe`, a scratch test on the same part, two cuts through the ball's centre, four views, three sizes, Intel MTL Vulkan and lavapipe): the window's renderer (four samples) against the oracle at one sample lacked the cap at 30 pixels over 22 of the 24 views, one to five per view, every one a blend of three samples of the cap with one of the wall behind, at the same pixels on both adapters; the same views at four times the resolution with one sample per pixel (so the standard sample positions of a multisampled pixel are pixel centres) had the GPU's cap equal to the CPU's at every pixel clear of the outline, in both directions; and at every one of the 30 pixels all four sample positions showed the cap at one sample. So neither the rasteriser's edge rule nor the sample positions nor the geometry differ from the CPU at those samples; what differs is where the decision *cut away or not* is taken: once per fragment, at the pixel's centre, for all the samples the fragment covers, in the count pass and the drawing passes alike. The removed half of the cavity's wall is seen edge-on along its silhouette, in sliver triangles a fraction of a pixel wide, and for a fragment of one the pixel's centre lies outside the triangle, where its distance from the plane is *extrapolated* and crosses zero within that fraction of a pixel; the sample the sliver covers is then not counted (or not cut) although the surface at the sample's own point is above the plane, the count there comes out even, and the cap is left out at one sample. Away from the outline, since a sliver's extrapolated distance changes sign anywhere along its silhouette, not only where the surface meets the plane. *The fix*, in the multisampled shader composition only (`scene_source_for(fragment_depth, multisampled, per_sample, ..)` fills the markers SIDE_SAMPLE and SIDE_CENTROID; with one sample per pixel the centre is the sample and they are empty; `per_sample` is `Onscreen::new`'s reading of the adapter's `DownlevelFlags::MULTISAMPLED_SHADING`, and a device that cannot shade per sample keeps the count at the pixel's centre, with the specks, rather than no cut-open picture; `renderer.rs` › `the_cut_composition_for_a_device_without_per_sample_shading_validates` checks that fallback with naga's capability of the same name absent): the count pass has a vertex stage of its own, `vs_parity`, whose distance from the plane is `@interpolate(perspective, sample)`, so `fs_parity` runs per sample and a sample is counted by exactly the surfaces that are cut away at its own point, as the CPU counts (`render` › `owns_sample`); the passes that draw take `CutOut::side` at the `centroid` of the covered samples, never beyond the triangle, so a sliver wholly above the plane is cut at every sample it covers, and along the cut a surface is cut per fragment within the pixel as before (the outline of the cap is drawn over it as lines). The drawing passes stay at one shading per pixel: per-sample shading of every cut-open frame would cost up to four times the fragment work for a sub-pixel gain at the outline only. *Cost* (release, the frame until the picture is read back, twelve dense spheres over each other cut through their middles, 594 000 triangles, 2560 × 1400, four samples): the Radeon 8060S (RADV) 5.7 → 6.4 ms per frame (the count pass at sample rate), its OpenGL (one sample, unchanged) 4.8 → 4.9; the Intel MTL (Vulkan) 139 before, 156 and 108 in two runs after (the integrated GPU's run-to-run spread exceeds the difference), its OpenGL 33 → 25. *After*: 0 of the 30 pixels on Intel Vulkan and lavapipe (OpenGL has one sample and had none), the one-sample comparison unchanged at 0, `crates/ui-egui/tests/section.rs` green as it is, the gpu section parity tests unchanged. *Pinned* by `cap_specks.rs`: the 24 views against the oracle with the window's renderer (no pixel of a flat stretch of the cap missing, on every adapter), two of them at four times the resolution and one sample (the cap equal pixel for pixel, either way), and a block seen straight from above with the diagonal of its top face through the centres of the pixels along it (a sample exactly on a shared edge is counted once: the cap whole at one sample per pixel, where the sample is the centre, as the CPU's `owns_edge` and the GPU's fill rule both give such a sample to exactly one of the two triangles; the window's renderer draws it too, its samples off the diagonal). The template test of `renderer.rs` asserts the per-sample interpolation appears in the multisampled cut composition and nowhere else (the `sample` qualifier needs GLSL 4.00 or ES 3.20, and the OpenGL renderer, at one sample, never compiles it).
- **Edges that read on every body: a shade of the body, a stronger outline, faded where they crowd** (2026-10-21, the developer's request after the Mission theme: on the Garabit pier's dark red the one grey edge colour read as grey seams, its lattice girders as a grey mush from afar; `viewport.rs` › `EdgeInk`, `surface_steps_back`, `fade_short`, `edge_fade`, `EdgeDensity`; `theme.rs` › `Tokens::edge_colors`; `crates/gpu/src/lines.wgsl`, `edge_tiles.wgsl`, `distance.wgsl`; `tests/edges.rs`, `tests/gpu_viewport.rs`; pictures in `target/work/edges/`). Three rules, the same on both paths. *Colour*: an edge of a body is the body's colour (its appearance, else the theme's body colour) with every sRGB channel scaled by `edge_shade` 0.4 (hue kept) when that contrasts with the body's colour by `edge_min_contrast` 1.5 or more (WCAG ratio of relative luminances), else lifted `edge_lift` 0.45 of the way to white: the pier's red gets dark red edges (2.2:1), the rover's white dark grey ones, black wheels light grey ones; the outline is the colour scaled by `edge_silhouette_shade` 0.25. No colour is special-cased; a highlighted edge keeps the highlight colour and the outline of a section's cap the theme's `edge`. *Outline*: a stretch of an edge in sight is the body's outline where the surface on either side, read 1.5 and 3 pixels across (`OUTLINE_NEAR`, `OUTLINE_FAR`) from the depth buffer the lines are tested against and carried on to the edge along the line through the two (in the reciprocal of the distance in perspective, so a plane reaches the edge exactly), lies behind the edge by more than `OUTLINE_STEP` 2 pixels' worth of the view at the edge's distance plus half its own change per pixel (`OUTLINE_SLOPE`) and the depth test's slack, or shows nothing: the background, another body, or a part of the same body further back (a flange before its web). It is drawn `edge_silhouette_width` 1.6 points wide against `edge_width` 1. The CPU decides per piece of a pixel (the depth test's pieces), the GPU per pixel at the point of the segment under it; the CPU draws the stretches of one segment as one line whose pixels take the look of the stretch at their foot, cut square where the edge goes behind a body (`EdgeInk::joined`, `cut`), as the GPU draws a segment, so a stretch does not ink over the next. *Fade*: an edge keeps `fade_short(extent) · min(1, edge_dense_cap / density)` of its opacity, lifted to at least `edge_fade_floor` 0.05, where *extent* is the diagonal of the edge's box projected (a straight edge's length; smooth from 0 at `edge_fade_short` 3 points to 1 at `edge_fade_long` 12) and *density* the share of a tile of `edge_tile` 16 points that the ink of the edges in sight covers at `edge_width` (one 1-px edge across a tile: 1/16), blended bilinearly between the tiles' centres so it changes smoothly as edges move; with the cap at 0.15 the edges of a crowded tile together cover no more of it than the cap, however many they are (a smooth step between two thresholds still let ten overlapping faded edges blacken a lattice; the ratio does not), and up close nothing is crowded and every edge is crisp. The CPU measures the density in closed form (each line's length per tile times its ink across, in four strands across its width, plus its round ends), the GPU per pixel: an ink pass of the edges in sight into an `R16Float` target summed by blending (`fs_ink`), then the mean per tile (`edge_tiles.wgsl`, an `R32Uint` target holding `f32` bits); a device that cannot blend into `R16Float` fades for length only (`Onscreen::edge_density`). *GPU cost*: the lines are now one instance per segment (68 bytes, the quad made from the vertex index; 264 bytes per segment before), and the depth buffer is turned into one distance per pixel once per frame (`distance.wgsl`, the mean of the samples exactly as `surface_at` computed it per read, so the depth test reads one texel where it read four), which more than pays for the outline's four extra surfaces and the ink pass. *Measured* (release, the pier, 57 k faces, 419 k triangles, 1280 × 800, Intel MTL Vulkan, four samples, `frame_time --open`): the direct path while orbiting, until the GPU is done, 102 ms before and 40 ms after (two runs each); the CPU path (`--cpu`, the drawing thread's quick preview with its overlay) 95-138 before and 152-222 after on a machine loaded by other agents, the outline test and the density per preview (owed in the roadmap); the CPU's sharp image (`snapshot`, ssaa 2) unchanged: pier from afar 655 → 646 ms, close up 6052 → 6033 ms, rover 295 → 309 ms. *Parity* (`gpu_viewport.rs`, Intel Vulkan; revised after review the same day, when three mutations of the GPU stayed green: lines 3 px wide crowd a tile on their own, so nearly every sample was faded and held only to a range, curved edges were never sampled, and the light plate's edges were within the tolerance of the theme's). Now: (1) the direct-path scenes draw with the fades off, every edge segment is sampled (a curved edge's neighbouring segments no longer count as crowding, only the ends where the CPU cut a stretch are avoided), and the bodies of the new scene have colours far from the theme's and from each other (a very dark post, a red ball, a blue plate, a green bead, a comb of yellow plates); each body's edge colours must be among the samples, which agree within 8 levels at 99 % or more. (2) A scene of its own at the theme's widths, fades on (a comb of 24 plates a pixel apart and 36 specks a few pixels across), compares every pixel along the edges in sight with three CPU pictures, the oracle and the same without the fade for shortness or for crowding, where the oracle and the unfaded picture differ by 24 levels or more: the GPU's pixel must be nearest the oracle at two thirds of them (measured 96 % of 702 for short edges, 80 % of 10 917 for crowded ones). (3) The density tiles agree within 0.025 + a tenth of the density (measured 0.019 at 0.14 and 0.080 at 0.90 in the comb). *Mutations* of the GPU that now fail: the theme's body colour for every edge (the coloured scene), no fade for shortness (2 % nearest the oracle), no fade for crowding (2.5 %). The overlay's shaders (lines, fills, distances, depth read, edge tiles) are validated by naga without an adapter for one and four samples, and translated to GLSL ES 3.10 (`onscreen.rs` › `the_overlay_shaders_validate_without_an_adapter`; not 3.00, WebGL2: the depth test's array of arrays, there since 2026-10-18, needed 3.10; one array since 2026-10-22, and translated to GLSL ES 3.00 since 2026-10-26, *The browser*). *Expectations changed* (dev log): `depth.rs` (an edge in sight may be several stretches end to end, cut where its outline starts or stops; with no surface beside it every edge is an outline; the top edge of the block from above is drawn in the block's outline colour), `thin_members.rs` (its probe colour is now black edges on light-red bodies with the fades off, since an edge's colour follows its body), `gpu_viewport.rs` (the oracle renders the bodies' own colours; outline width 4 beside the edge width 3). *Owed*: an edge on a face in deep shade can still be close to that face's colour (the contrast is taken against the body's colour at full light); the window's fallback paths paint the overlay with egui's painter, which fades a line per piece of half a tile rather than per pixel.
- **A frame that differed once: one sample of the surface behind the front, on the Strix's Radeon over OpenGL** (2026-10-22; `crates/gpu/tests/parity.rs` › `the_same_input_gives_the_same_frame`, `soak_the_same_input_gives_the_same_frame`; `common/mod.rs` › `frame_difference`). In one gate of about sixty (run 39677202007, attempt 1, 2026-10-21) the teapot drawn twice on one device came back different on the adapter `AMD Radeon 8060S Graphics (radeonsi, strix_halo, ACO), Gl, Mesa 26.2.0-devel`; the rerun was green. The log held both frames in full, so they were taken apart afterwards (pictures in `target/evidence/gpu/strix-same-frame-2026-10-21-*.png`): **one pixel of 172 800 differed**, (263, 111) on the bright rim of the lid, colour (208, 214, 224) in the first frame and (162, 167, 175) in the second, face id and distance equal. The first frame is the CPU renderer's and every local adapter's value. At `ssaa = 2` that pixel is the mean of four samples, all well inside triangles of the lid (the nearest triangle edge 0.011 pixel away, three steps of the rasteriser's 2⁻⁸ grid; one surface behind them, 0.1 units further, `render` at opacity 0 shades it (40, 41, 43)). Exactly one of the four, the sample at (0, 1) of the pixel, having that colour of the surface behind gives the second frame's three channels to the level (648 / 4 = 162, 666 / 4 = 166.5 → 167, 698 / 4 = 174.5 → 175, the resolve rounding half up); no other single sample does, and the background does not. So in that one frame the front fragment of one sample was lost (or its write not seen by the resolve pass) and the triangle behind it, drawn earlier in the mesh's order, stayed. Nothing in the renderer explains it: the targets are cleared by every pass that writes them, the readback waits for the queue (`PollType::wait_indefinitely`) before mapping, a stale readback would have given the identical previous frame, the uploads go in order on one queue, the GPU crate has no hash map, clock or random number, and the adapter of both frames is the same. What remains is the driver or the hardware (a development snapshot of Mesa, with the scene pass on OpenGL writing its depth in the fragment shader, see *The depth buffer on OpenGL loses the key*), on a GPU the gate's parallel tests, a local language model and a speech model share. **Not reproduced on the development machine** (Intel Meteor Lake: Vulkan, OpenGL and lavapipe): 300 runs of the test in processes of their own (150 of them with the CPU loaded), 3 × 2999 frames compared in one process, and 6 threads × 1499 frames on all three adapters at once with the CPU loaded, none differed. **What changed:** the test is not loosened and retries nothing. It now runs every comparison (same device, new device, after another picture of the *same* size, whose pixels are then in every target the teapot's passes reuse, and after one of another size) and fails with all that differed; each difference writes `target/evidence/gpu/same-frame-<adapter>-<what>.png` (the two frames and the pixels that differ in red) and `.txt` (every differing pixel with both colours, ids and distances) instead of printing two 5 MB arrays. `soak_the_same_input_gives_the_same_frame` (ignored; `CRAFT3D_GPU_SOAK` frames, default 2000, by two threads per adapter at once) and the Heavy workflow's task `gpu-soak` (20 000 frames per thread on the Strix, the evidence kept as the run's artifact) are for measuring the rate there. Owed: a soak on the Strix; if it shows the difference on OpenGL only, a reduced case for Mesa, and whether the gate should draw on the Radeon's Vulkan adapter alone is the developer's to decide.
- **The Radeon loses one of several writes of a sample; every sample is now written once** (2026-10-22, the soak owed above; `crates/gpu/src/renderer.rs` › `Step`, `Drawing`, `draw_in_steps`; `scene.wgsl` and `cut.wgsl`, the position `@invariant`; `crates/gpu/tests/parity.rs` › `two_steps_give_the_one_step_frame`, `soak_the_same_input_gives_the_same_frame`). *Measured, before:* the Heavy task `gpu-soak` on the Strix (run 39821584698, 20 000 teapots per thread, two threads per adapter) found the difference on **both** of the Radeon's adapters, not on OpenGL alone: 115 and 98 frames of 19 999 on Vulkan (RADV), 49 and 58 on OpenGL (radeonsi), 0 on llvmpipe; always one pixel, always one of six silhouette pixels of the lid's rim and the spout ((263, 111), (285, 151), (203, 158), (217, 160), ...), always one sample of the surface behind the front. *Bisected* with one Heavy run per batch of variants (runs 39837081999, 39838254166, 39840098043, 39840778972, 39842573815; 5 000 frames per variant, in counts of differing frames Vulkan / OpenGL): one thread per adapter 21 / 8, Vulkan alone 24, OpenGL alone 5 (not concurrency between devices or queues); the mesh uploaded once 25 / 5 (not the upload); no hierarchical depth (`RADV_DEBUG=nohiz`, `AMD_DEBUG=nohyperz`) 36 / 7, no primitive binning 12 / 6, no fast clears 22 / 8, no NGG culling 20 / 2, no NGG 22 / 9, invariant geometry 22 / 10, no out-of-order rasterisation 18 / 7: none of these; **no delta colour compression (`RADV_DEBUG=nodcc`, `AMD_DEBUG=nodcc`) 0 / 0, and 0 / 0 in 15 000**. At `ssaa = 1` on the full 960 × 720 (where the readback's id and distance are those of the sample itself) a frame kept the surface behind in the colour target while the data target had the front, and another the other way round: the depth test chose the front every time, and one of the two targets ended with the colour an earlier, further fragment had written there before. Drawing the depth of the whole mesh first and then the colour where the depth is equal, so that no sample is written twice, gave 0 / 0. Storage usage on the targets (another kind of compression on RADV) did not help (16 on Vulkan). So the cause is below the API: with delta colour compression on, a later write of a sample in a pass is now and then overtaken by an earlier one, on Mesa 26.2.0-devel on the 8060S (gfx1151) for both RADV and radeonsi, which share the surface code (`ac_surface`); nothing in WebGPU or Vulkan lets a correct program see it, as the order of a pass's writes to one sample is the primitive order. *Reduced:* the scene pass alone as a small wgpu program (the teapot's triangles, the same vertex arithmetic and reversed depth, the face id as the colour, read back at the full size) did not show it with a trivial fragment shader; with 64 rounds of arithmetic in the fragment shader it does on RADV, about 1 frame in 7 000 (6 of 39 998, 5 of 39 998 without the sampling pass), always the same 3 neighbouring texels, and 0 with `nodcc`; not yet on radeonsi (where one worker of the program ended with its last 25 frames returning a block of texels in the cleared colour, another symptom, not looked into; the program is `plan/rad_repro.rs` in the developer's checkout, run as an example of `craft3d-gpu`). *The way around it, in our code:* every pass over the triangles of the offscreen renderer (the opaque scene, the front and the layer behind it of a see-through picture, the cut-open scene and its caps) is drawn in two steps on a device with 32-bit float depth and a stencil (`Depth32FloatStencil8`, the feature asked for where the adapter has it): first the depth alone with the colour targets masked (`Greater`, written), then the same pipelines' colour where the depth is `Equal` and the stencil still 0, which the write raises (`IncrementClamp`), so each sample is written once, by the first fragment drawn of those at the nearest depth; that is the one step's tie rule, so the picture is the same bit for bit (`two_steps_give_the_one_step_frame`: the teapot opaque, in orthographic, seen through, cut open, both, and a square drawn twice, on every adapter here and on the Strix). Both steps run the same shader module and entry point, so a fragment's depth is the same number (the position is `@invariant`; on OpenGL the fragment writes the depth with the same arithmetic) and a fragment discarded in one is discarded in the other. A device without the feature draws in one step as before. *Measured, after* (run 39842573815, 19 999 frames per thread, two threads per adapter): opaque, see-through and cut open **0 on each of the Radeon's four workers**; drawn in one step the same build still loses writes (see-through 36 and 31 of 4 999 on Vulkan, 6 and 6 on OpenGL), so the soak keeps measuring the driver (`CRAFT3D_GPU_SOAK_ONE_STEP=1`). Every test of `craft3d-gpu` and `craft3d-ui-egui` passes on the Strix. The cost: the triangles go through the vertex stage twice; 20 000 teapots of 480 × 360 at `ssaa = 2` took 42.6 s instead of about 40 s per thread on the Radeon. *Open:* the viewport (`onscreen.rs`, multisampled) still draws in one step; its pictures are not compared bit for bit across frames, but the same lost writes can show there as a flickering sample on a silhouette. A report to Mesa with the reduced program is the developer's to decide.
- **The one-step reference lost writes too; the test now tells whose frame differs** (2026-10-30; `crates/gpu/tests/parity.rs` › `two_steps_give_the_one_step_frame`). The test compared the two-step frame with a frame drawn in one step, the very path that loses writes on the Radeon, and failed three times in a week on Vulkan (RADV): one pixel, (263, 111) on the lid seen through, about 20 levels. Now, per view and adapter, the two-step frame (production's) is held to oracle 7 against the CPU renderer strictly; the two frames are still compared bit for bit, but a pixel where they differ is the one-step frame's lost write when the one-step frame drawn again (three times at most) gives the two-step value there, since a lost write does not repeat at one pixel (at most one see-through teapot in about 140, so three in a row below one in a million); such pixels are printed with their evidence and do not fail. A difference the one-step frame repeats every time is the two-step frame's and fails, and the two-step frame must be nowhere further from the CPU frame than the one-step frame as drawn (`Parity::worse_than`, every count of oracle 7).

#### 7.3 What the oracle is for

In PhotoCraft the CPU compositor defines what the image *is*. Here the model's truth is the B-rep and its mass properties, not the picture. The CPU renderer is the oracle for **display** only: it lets agents and the developer see a model without a GPU, and it catches GPU regressions. Geometric correctness is tested geometrically ([`testing.md`](../testing.md)).

---

### The browser: WebGPU and WebGL2 (2026-10-22)

The browser build (`apps/craft3d-web`) draws the viewport with the same `Onscreen` renderer and shaders on the page's device; checked in headless Chrome (SwiftShader) on WebGPU (4 samples) and WebGL2 (1 sample) against the CPU renderer's picture of the same flange. Two things WebGL2 (wgpu's GL backend on OpenGL ES 3.0) does not take that desktop OpenGL does:

- **Arrays of arrays.** GLSL ES 3.0 has none, and naga refuses `array<array<…>>` there; `depth_test.wgsl` held its 4 × 4 samples that way, so every line pipeline failed validation. One array indexed by `row * 4 + column` now; `crates/gpu/tests/webgl_shaders.rs` keeps arrays of arrays out of every shader.
- **A depth texture without a nearest filter is incomplete.** OpenGL ES 3.0 (§3.8.13) takes a depth texture whose magnification filter is not `NEAREST` for incomplete, and `texelFetch` from it reads 0; wgpu-hal's GL backend binds no sampler to a texture that is only loaded from, so the default linear filter stays. Every surface read as nothing and no line was hidden. The distance pass now reads a single-sampled depth buffer with `textureSampleLevel` through a nearest sampler at the texel's centre (the texel itself; the multisampled path is unchanged; the parity tests on Vulkan and OpenGL pass unchanged). The see-through (peel) and section (cut) passes still loaded depth textures, so on WebGL2 the page handed those pictures to the CPU renderer until 2026-10-26 (next item).
- **See-through and the section on WebGL2 (2026-10-26; `crates/gpu/src/peel.rs` › `sampled_front`, `peel_bind`; `renderer.rs` › `scene_source_for`, the markers `{{FRONT_SAMPLER}}` and `{{FRONT_DEPTH}}` of `peel.wgsl` and `cut_flat.wgsl`; `cut.rs` › `cap_layout`, `cap_bind`; `crates/gpu/tests/webgl2.rs`).** The developer's report: in the web build the See-through slider made every frame a CPU frame, slow on the page's one thread, because most Linux browsers have WebGPU off and run WebGL2. *What the web build drew on WebGL2 before:* the opaque picture and every line over it on the GPU (the shaders of the overlay take GLSL ES 3.00 since the depth test's array of arrays went on 2026-10-22, and the distance pass samples the depth buffer), see-through and cut-open frames on the CPU (`page.rs` called `without_gpu_see_through` and `without_gpu_section` on the GL backend). The pipelines themselves were built; what failed was the read: the pass behind the front (`fs_behind`, `fs_behind_cut`, `fs_cap_behind`) took the front's depth with `textureLoad`, which WebGL2 answers with 0 for a depth texture whose filter is linear, so nothing was ever behind the front. *The fix*, on the OpenGL backend only (the one whose fragments write their depth, `fragment_depth`, which always has one sample per pixel): the front's depth is sampled through a nearest sampler (group 1, binding 2, beside the front at 0 and a section's count at 1) at the texel's centre, `(floor(clip.xy) + 0.5) / size`, which gives the texel itself, as the distance pass does; the layouts and bind groups of the pass behind and the cap behind carry the sampler there. Vulkan, Metal, DX12 and WebGPU keep the load: their shader sources, layouts and bind groups are what they were, so their pictures are unchanged bit for bit; the multisampled peel (keys, not depths) is untouched. The page no longer hands either picture to the CPU. *Verified* three ways. (1) naga, without an adapter (`renderer.rs` › `template_tests::what_opengl_runs_translates_to_glsl_es_300`, `onscreen.rs` › `the_overlay_shaders_validate_without_an_adapter`): every single-sampled composition in OpenGL's variant (scene, scene + peel, scene + peel + cut + cut_flat), the blend and the overlay's shaders at one sample are validated with no optional capability and translated to GLSL ES 3.00 with `is_webgl`, entry point by entry point, as wgpu does in a browser (the overlay was translated to ES 3.10 before); and in naga's IR the front's depth is never loaded and always sampled in OpenGL's variant, loaded and never sampled in every other, and the distance pass samples the depth buffer at one sample (`template_tests::reads_of`). This is the test that failed before the fix (the OpenGL variant loaded `front_depth`). (2) A real OpenGL ES 3.0 context (`tests/webgl2.rs`): the test runs itself again in a child with `WGPU_BACKEND=gl MESA_GL_VERSION_OVERRIDE=3.0 MESA_GLES_VERSION_OVERRIDE=3.0` (wgpu takes a desktop context when EGL offers one; with the desktop version capped below 3.3 it falls back to OpenGL ES, and Mesa then reports ES 3.0 and GLSL ES 3.00, so wgpu translates every shader to `#version 300 es`), asserts the adapter is `OpenGL ES 3.0` without compute shaders, requests the device with `Limits::downlevel_webgl2_defaults()` as egui-wgpu does in a browser, and compares the viewport's picture with the CPU oracle under §7.2's tolerances: the teapot and a block opaque, the teapot, the block and a hollow block see-through at 0.1, 0.5 and 0.9 (every other with faces highlighted, one orthographic), the hollow block cut open across z and obliquely, opaque and at 0.5, and the lines over an opaque and a see-through picture (hidden, dimmed, left out). *Measured* on the Strix Halo (Radeon 8060S, radeonsi, Mesa 26.2, `OpenGL ES 3.0 Mesa 26.2.0-devel`): in all 15 pictures not one pixel off by a level away from the edges and seams, no id outside the band, at most one id and one colour in the band (block at 0.5 highlighted; the hollow cut obliquely at 0.5), the lines exactly as on the other backends. Mesa does not apply ES 3.0's rule for depth textures, so the same test passed before the fix with the same numbers: on Mesa it pins the shading language, the limits and the sampled read's picture (identical to the load's), not the rule; (1) pins that. (3) A browser: the developer opened the CubeSat in his own Linux browser with `?webgl` (2026-10-26): see-through and a section are drawn on the GPU and turn smoothly. Not tried yet: Windows (Chrome and Edge run WebGL2 on ANGLE over Direct3D 11) and macOS Safari; the recipe for a headless check is that of 2026-10-22 (`development.md` › *Web build*, headless Chrome with `--use-angle=swiftshader` and `?webgl`). *What a failure costs in a browser:* `guarded` captures nothing on wasm (WebGPU's error scopes can only be awaited), so a pipeline the browser refuses is not an `Err` from `Onscreen` but an error on the device's `on_uncaptured_error`, which the page answers with `Shell::leave_gpu` (`apps/craft3d-web/src/page.rs`): the *whole* viewport goes to the CPU renderer from then on, not just that frame as on the desktop. The see-through pipelines are built in `Onscreen::new`, so a browser that refused them would leave the GPU at the first frame; the cut-open ones are built for the first picture with a section (`Onscreen::ensure_cut`), so a refusal there moves the viewport to the CPU when a section is first switched on. Building them eagerly would not turn the refusal into an `Err` either (the same uncaptured path), only move it to the start, so they stay lazy; accepted for R1. *Cost*: the sampled read is one texture fetch as the load was; the early depth test was already lost on this backend (fragment-written depth). *Review* (2026-10-26, the coordinator's): the sampler is `Onscreen`'s own nearest one, the one the distance pass uses (`depth_sampler`); the readback renderer keeps one with its see-through pipelines.

Errors WebGPU reports later go to the page's `on_uncaptured_error` handler (wgpu's default panics, which ends a WebAssembly page); the page then drops the direct path for the CPU renderer (`Shell::leave_gpu`).

---

### The studio look (2026-10-29)

Layer 1 of `plan/rendering.md` (the developer's decision of 2026-10-29). `craft3d_render::studio` is the
definition; `craft3d_gpu` repeats it in `studio.wgsl` on the readback renderer (`Renderer::draw_studio`,
`Offscreen::render_studio`) and on the viewport's (`Onscreen::set_studio`). View state only: `ui.set
{"look": …}` (`ui_egui::state::LookState`), `view.render`'s `look` and `craft3d-cli render --look …`
parse the same object (`craft3d_engine::look::LookSpec`); the plain look stays the default everywhere,
so every picture, thumbnail and test of the plain look is unchanged bit for bit.

**Sources** (clean-room: no code of Blender's Eevee or Cycles, which are GPL, was read or transliterated):

- B. Karis, *Real Shading in Unreal Engine 4*, SIGGRAPH 2013 Physically Based Shading course: the split
  sum, GGX importance sampling of the half vector, the prefiltered environment with `n = v = r`.
- Google Filament, *Physically Based Rendering in Filament*: GGX `D`, the height-correlated Smith
  visibility (after E. Heitz, *Understanding the Masking-Shadowing Function*, JCGT 2014), perceptual
  roughness `α = r²`, the DFG table's layout.
- Google Filament, *Physically Based Rendering in Filament*, § Energy loss in specular reflectance, after
  C. Kulla and A. Conty, *Revisiting Physically Based Shading at Imageworks*, SIGGRAPH 2017: the energy
  compensation `1 + F0 (1 / (A + B) − 1)` of the specular part (the diffuse part is not reduced by the
  energy it adds; C. J. Fdez-Agüera's *A Multiple-Scattering Microfacet Model for Real-Time Image-Based
  Lighting*, JCGT 2019, which also takes it from the diffuse part, is not implemented).
- M. Colbert and J. Křivánek, *GPU-Based Importance Sampling*, GPU Gems 3 ch. 20: filtered importance
  sampling (each sample reads the source at the level matching its solid angle).
- R. Ramamoorthi and P. Hanrahan, *An Efficient Representation for Irradiance Environment Maps*,
  SIGGRAPH 2001: nine spherical harmonics, convolved with `π, 2π/3, π/4`.
- S. Lagarde and C. de Rousiers, *Moving Frostbite to Physically Based Rendering*, SIGGRAPH 2014:
  specular occlusion from ambient occlusion.
- J. Jimenez, X. Wu, A. Pesce, A. Jarabo, *Practical Real-Time Strategies for Accurate Indirect
  Occlusion*, SIGGRAPH 2016 course (GTAO): horizons per slice and the closed-form cosine-weighted arc.
- L. Williams, *Casting Curved Shadows on Curved Surfaces*, 1978 (shadow maps); W. Reeves, D. Salesin,
  R. Cook 1987 (percentage-closer filtering); R. Fernando, *Percentage-Closer Soft Shadows*, NVIDIA 2005.
- T. Sobotka's AgX as described by B. Wrensch, *Minimal AgX Implementation* (2023: the inset and outset
  matrices, the log2 range of 16.5 stops, the sixth-order fit; credited in `ATTRIBUTION.md`); K. Narkowicz,
  *ACES Filmic Tone Mapping Curve* (2016); IEC 61966-2-1 (sRGB). Wrensch's snippet ends with `pow(x, 2.2)`
  after the outset matrix, for a framebuffer that then encodes to sRGB; both steps are skipped here and the
  outset value is the display value, as for a display of gamma 2.2, which the sigmoid is fitted for. The
  two agree from the mid-tones up (0.5 becomes 0.504) and differ in the darks, where sRGB's linear toe is:
  a display value of 0.1 (byte 26) would be 0.073 with the steps (byte 19), 0.02 would be 0.002. The
  shadows of the studio look are therefore a little lighter than a renderer that keeps the steps.
- G. Ward, *Real Pixels*, Graphics Gems II (1991), and the Radiance file-format notes: the `.hdr` reader.
- The environments: three 1k HDRs from Poly Haven, CC0 (`assets/environments/LICENSE.md`,
  `ATTRIBUTION.md`): `studio` (Studio Small 09), `outdoor` (Kloofendal 48d Partly Cloudy, Pure Sky),
  `workshop` (Abandoned Workshop 02). 4.8 MB together; the desktop programs carry them in the binary
  (`studio::bundled`), the web page fetches the one chosen from `environments/` beside it (`cargo xtask
  package --formats web` copies them there; the WebAssembly module does not grow by them).

**Method.** At load (`Prepared::new`, once per environment and process, cached):

1. Level 0: the picture resampled to 512 × 256 (an average of K × K bilinear lookups per texel, K the
   ratio of the widths) and scaled so that its mean luminance over the sphere is 1: every environment
   lights a model alike and the exposure means the same for all.
2. The key light: the brightest texel of the 64 × 32 level, then within 0.5 rad of it the excess of each
   texel of level 0 over `max(4, peak / 4)` times the mean luminance. Its sum (times the solid angle) is
   the key's irradiance, its luminance-weighted direction the key's direction, `√(Ω / π)` its angular
   radius. An even environment has none and casts no shadow.
3. The diffuse light: level 0 minus the key, projected on nine spherical harmonics, convolved with the
   clamped cosine. The key is added back as a directional light with its shadow; together they are the
   environment, so no light is counted twice.
4. Levels 1 to 5 (256 × 128 down to 16 × 8) for roughness `i / 5`: 128 GGX samples per texel (Hammersley),
   each reading a box-filtered chain of level 0 at its solid angle.
5. The split-sum table, 32 × 32 (`n·v` across, roughness down, at texel centres), 512 samples per texel.

Everything either renderer reads is stored as `f32`: both read the same numbers, with the same manual
bilinear and trilinear lookups (`textureLoad`, never a sampler).

Per sample (the CPU's `render_studio`; the GPU's passes in brackets):

- the nearest surface by the plain renderer's rules (the same coverage, key and ties);
- [shadow map] the key's view: a square over the sphere around the mesh, 1024², the depth from the light
  as `f32` bits; the CPU rasterises it with the top-left rule;
- [geometry, occlusion] GTAO over the samples' view distances and normals: six slices, six steps per
  side, a radius of 0.12 of the sphere around the mesh (at most a quarter of the picture's height); a
  sample counts only when it rises above the tangent plane by more than a sine of 0.05 (else a tilted
  flat face, sampled at texel centres a little off the slice, darkens itself by 0.1 %); the sum is
  divided by the same sum for empty slices (`cos γ + γ sin γ` per slice), so a flat open face is 1
  exactly with any number of slices;
- [background] where no surface is: the environment at roughness 0.45; where the ray meets the ground
  (the plane at the bottom of the mesh's box, seen from above) it is darkened by
  `(E + K l_z vis) / (E + K l_z)`, `E` the luminance of the diffuse light on an upward face, `K` the
  key's, `vis` the shadow there: a shadow catcher, no geometry;
- [bodies] the light (the formula in `studio/mod.rs`): with `F0 = 0.04 (1 − m) + base m` and `(A, B)` from
  the table, specular = prefiltered(reflection, `r · 5`) `· (F0 A + B) ·` multiple scattering `·` specular
  occlusion; diffuse = `base (1 − m)(1 − Fs) / π · (E_sh(n) · ao + E_key n·l vis)`. The key's shadow
  (PCSS: a 4 × 4 blocker search over the widest penumbra, then a 5 × 5 filter as wide as the gap times
  the key's angular size, 1 to 24 texels; normal offset 1.5 texels, bias 2) darkens only the key's
  diffuse light: the specular light keeps the key in the prefiltered map (layer 2's contact shadows and
  screen-space reflections will refine this).
- exposure `2^EV`, the tone curve (AgX by default, ACES, or linear for the tests), the sRGB encoding,
  rounded per sample, averaged per pixel as in the plain picture.

A face without a roughness of its own is at 0.5, not metal; a highlighted face is its highlight colour at
0.5. See-through and cut-open pictures: *See-through and cut-open pictures in the studio look* (2026-10-30).

**Plain background** (`Look::plain_background`, `look.background` `plain`, `craft3d-cli render
--backdrop plain`): the environment lights the model as before, and the picture's own background colour
(`RenderParams::background`, the theme's) stands behind it instead of the environment: decoded to linear
light, times the ground's shadow factor, encoded to sRGB again, without exposure or tone curve, so that
where no shadow falls it is the colour exactly (`FrameLight::background_bytes`; on the GPU one more `vec4`
of the uniform block, `STUDIO_FLOATS` 76). Held by `a_plain_background_is_the_pictures_colour` and by one
of the GPU parity scenes. Its reflection in a glossy ground is layer 2.

**Degrading.** A studio look whose passes cannot be built or made ready on the viewport's device is drawn
in the plain look, with the reason kept (`Onscreen::studio_failure`); the viewport keeps its GPU renderer
(before, the error retired the direct path for good). The CPU fallback of the browser's viewport draws
the simpler look (no shadows, no occlusion, `backend::cpu_look`): full PCSS and GTAO on its one thread
would hold the page up. The window prepares an environment on a thread of its own
(`LookSpec::prepared_if_ready`) and draws plain until it is ready, then again (the frame key has the
environment's readiness; the window looks again every 100 ms while it waits); `ui.screenshot` and
`view.render` wait for it. The split-sum table does not depend on the environment and is computed once
per process (`brdf::shared_lut`).

**WebGL2.** The simpler path (`studio::simpler`): in the browser on WebGL2 the viewport draws the studio
look without shadows and without occlusion (`Onscreen::studio_as_drawn` turns them off, so the CPU
reference of that frame is the same look with them off); image-based lighting, the ground's background
and the tone curve are the same. Every studio shader translates to GLSL ES 3.00 (`renderer.rs`,
`template_tests`, *scene + studio*); the passes use integer targets only, which WebGL2 renders.

**Tests.** `crates/render/tests/studio.rs`: the furnace (white dielectric and metal, roughness 0 to 1, at
20 angles, within 0.98 to 1.02 of the incoming light; the picture of a white rough sphere at exposure −1
within the bytes of 0.49 to 0.51 everywhere); a Lambertian surface under `1 + 0.5 z` against the closed
form `π + π/3 n_z` (within 0.2 %); a mirror sphere under a smooth environment against the environment
itself along the reflection (within 2 levels); the shadow of a ball lit from straight above is the disc of
its radius on the ground; determinism; bad environments and numbers are errors. `brdf.rs`: the split sum
against an independent midpoint quadrature over the hemisphere and against the closed form of a smooth
surface (`A = 1 − (1 − n·v)⁵`, `B = (1 − n·v)⁵`); published tables were not at hand offline, so the
reference points are these two independent methods. `ao.rs`: an open plane is 1, a groove's floor is
occluded, a step on the side a tilted normal leans to occludes more than the same step on the other side
(0.84 against 1.0; a sign error in the slice's normal angle γ gives 1.0 against 0.80, worked out apart
from the code). The key light's energy (`the_key_and_the_rest_carry_the_environments_light`, a bright cap
and the three bundled environments, against brute force over the mirror level with exact solid angles):
the mean over all normals of `irradiance(n) + K max(n·l, 0)` is a quarter of the environment's power
`P`, within 0.2 % (neither the nine harmonics nor the key's single direction change a mean over the
sphere, so this pins the key's scale: its solid angles, the excess's factor, its sum); and at sixteen
normals each within a bound derived in the test: `3/32 (P − K)` for nine harmonics of the rest (the
largest error of a point light's nine-term irradiance, at the horizon, `|1/4 + c/2 + 5/32 (3c² − 1) −
max(c, 0)|` at `c = 0`, and any light is a sum of point lights), plus `K (1 − cos ρ)` on the key's lit side,
`2 K sin(ρ/2)` across its horizon, 0 on its dark side, with `ρ = 2 KEY_CONE` (the excess lies within
`KEY_CONE` of the brightest direction, so within twice that of the key's direction), plus 0.2 % of `P`.
A band of a few percent at every normal cannot be derived, and is not met either: measured (Strix run
40976593867) the largest difference is 1.5 % of the mean irradiance `P/4` for `outdoor`, 6.8 % for the
bright cap (straight up), 7.1 % for `studio` and 15 % for `workshop` (straight up: 1.32 against 0.86,
the nine harmonics of a bright window at the horizon), each within its derived bound, which is four to
eight times larger. `crates/gpu/tests/studio.rs`: the GPU against the CPU (below), the same input gives the same
frame, the viewport's renderer against the CPU, the cost.

**Tolerance, GPU against CPU** (`crates/gpu/tests/studio.rs`): face ids equal outside the band (as for
the plain look); outside the band at most 1/1024 of the covered pixels more than one level apart and
none more than 4 levels. Why looser than the plain look's single level: the GPU draws its own shadow map, and
a texel along a triangle's border in the light's view may be covered by one renderer and not the other
(the band of the light's view is not the picture's); a PCF tap on such a texel moves the shadow of a pixel
by one tap, 1/25 of the key's light; the occlusion reads neighbouring samples, so a sample that the two
renderers cover differently (inside the picture's band) moves the occlusion of pixels around it that are
outside the band. Both are counted, not bounded per pixel. *Measured* (Strix, run 40927686269,
2026-10-29; the provisional 1/64 and 48 were set before any measurement): the readback renderer on its
three adapters (Radeon 8060S over Vulkan and over GL, llvmpipe), ten scenes in the four test looks, never
more than one level apart outside the band (no pixel over one); the viewport's renderer (four samples on
Vulkan, one on GL) on six scenes, at most 2 levels and at most 2 pixels over one level (the torus: 2 of
7 541 covered, 1/3 770). The bounds are those numbers with a margin of two (the largest difference)
and of about four (the share); ids equal everywhere outside the band.

**Cost** (`studio_frames_on_a_thousand_faces`, Strix run 40933205051, 2026-10-29): a grid of the
reference solids, 1 002 faces and 190 096 triangles, at 1020×777, on the Strix's Radeon 8060S. The
viewport's renderer, until the GPU is done: plain 1.03 ms, studio 3.94 ms, studio without shadows and
occlusion 2.13 ms (Vulkan, four samples); 0.58, 2.67 and 0.74 ms (GL, one sample). The readback renderer
with the read back: 13.5 ms plain, 16.0 ms studio (Vulkan). The CPU renderer: 183 ms for the studio
frame. llvmpipe, for scale: 49 ms plain, 186 ms studio in the viewport. (A second run, 40976593867,
after the review's changes, on a quieter machine: 0.26, 0.89 and 0.28 ms on Vulkan, the CPU frame 162
ms.)

**Preparing an environment** (`preparing_an_environment_costs`, the same run, test profile, Strix CPU):
the split-sum table 24 ms once per process; `studio` 400 ms, `outdoor` 123 ms, `workshop` 129 ms
(decoding the 1024×512 `.hdr` about 6 ms of each). The window spends it on its own thread and draws
plain meanwhile; the browser's page spends it on its one thread when the fetched picture arrives (a
hitch of that order, once per environment and session).

**Looked at** (2026-10-29, `target/evidence/render-rl1/`: flange, gear, bracket and a gear reducer, plain
and studio, the gear in the other environments and in ACES): no banding in the background, no shadow
acne, no occlusion halos. Two findings, left as they are for the developer: in `studio` (Studio Small 09)
the key the extraction finds is low (13° above the horizon) and weak beside the rest of the light
(irradiance 0.70 against about 3), so its shadows are faint; `outdoor` gives clear ones. And AgX lifts and
desaturates dark saturated colours, as it is made to: a navy housing (sRGB 86, 108, 145) reads paler in
the studio look than in the plain one (AgX alone maps it to about 100, 119, 149 before the light's
specular part is added); ACES keeps more of its colour (about 65, 92, 137). A punchier default (ACES, or
AgX with a saturation look) is a choice of taste, not a fix.

### Layer 2: polish (2026-10-30)

Layer 2 of `plan/rendering.md`. `craft3d_render::studio::polish` is the definition; the GPU repeats it on
the same buffers (`studio.wgsl`). View state only, every effect optional: `ui.set {"look": …}`,
`view.render`'s `look` and `craft3d-cli render` take `reflections`, `contactShadows`, `specularShadow`,
`groundReflection` (0 to 1), `bloom` (0 to 1), `dof` (`{fstop, focus}`, focus a distance along the view,
a point, `"picked"`: the window's last pick, or `"centre"`) and `accumulate` (frames). The look as
parsed (`LookSpec`, `LookState`) starts from `Look::polished()`: everything on but the ground's
reflection and the lens, one frame
for `view.render` and 16 for the window; `Look::default()`, which the tests of layer 1 use, has all of
layer 2 off, and with it off every picture is layer 1's bit for bit (the specular light is added in the
same order of operations as before, and the plain background is shown from its colour as layer 1
shows it, not divided by the exposure and multiplied again, which could move a byte at a fractional
EV; `the_plain_background_keeps_its_colour_at_any_exposure`).

**Sources** (clean-room: nothing of Blender's Eevee or Cycles, which are GPL, was read or transliterated):

- M. McGuire and M. Mara, *Efficient GPU Screen-Space Ray Tracing*, JCGT 3(4), 2014: a ray's ends
  projected, the depth interpolated as `1/z` across the screen, a hit where the ray passes behind the
  depth buffer by at most an assumed thickness. We step uniformly (32 steps) instead of their DDA, then
  halve the step 5 times.
- J. Jimenez, *Next Generation Post Processing in Call of Duty: Advanced Warfare*, SIGGRAPH 2014 course
  *Advances in Real-Time Rendering*: bloom's 13-tap downsampling (five overlapping boxes weighted 1/2
  and 1/8) and the 3 × 3 tent upsampling, summed level by level; the same talk's gather for depth of
  field (a scatter written as a gather, a tap weighed by whether its blur reaches the pixel).
- The thin-lens model (any optics textbook; Pharr, Jakob and Humphreys, *Physically Based Rendering*,
  4th ed., § 5.2.3): the circle of confusion `A · m · |z − zf| / z` with `m = f / (zf − f)`.
- B. Karis, *High-Quality Temporal Supersampling*, SIGGRAPH 2014 (Advances in Real-Time Rendering):
  sub-pixel jitter on a Halton (2, 3) sequence, frames averaged; with a still camera no reprojection or
  history clamping is needed, so the average is a plain mean that converges to the supersampled picture.
- B. Karis, *Real Shading in Unreal Engine 4*, 2013: widening the GGX lobe for a light of some size
  (here by half the key's angular radius), for the key's specular light.
- Filament's GGX `D` and height-correlated Smith `V`, Schlick's Fresnel, as in layer 1.

**Method**, per frame `k`, at the samples of the picture (`render_studio_frame`):

1. *Jitter.* Frame 0 is the picture without accumulation; frame `k ≥ 1` moves every sample by the Halton
   (2, 3) point `k` minus ½, in samples (`polish::jitter`; `View::jitter`, applied where a point is
   projected and where a sample's ray is made).
2. *Shading* (`FrameLight::shade_parts`): layer 1's, with two additions. **Contact shadows**: from the
   surface point a ray towards the key, 12 steps over 0.05 of the mesh box's diagonal, projected; it is
   hidden (the key's visibility 0) where it lies behind the picture's surface by more than half a sample at
   that depth (the tessellation must not shadow itself; scale-free) and less than 0.02 of the diagonal
   (an occluder's assumed thickness). **Specular shadow**: the key is part of the prefiltered
   environment, so where the key is hidden (shadow map times contact, `vis`) its share of the specular
   light is taken out: `share = lum(E · n·l · D V F (α')) / lum(specular)` clamped to 0..1 with
   `α' = r² + radius / 2` (at least 0.002), and the specular light times `1 − (1 − vis) · share`. The
   sample keeps its linear light and its specular weight `W = Fs · ms · so · keep` per channel, with
   its roughness (−1 where there is no surface), as `f32`, as the GPU stores them.
3. *Reflections* (`polish::Reflect`), when asked for. On a surface with roughness under 0.4: the mirror
   ray from the point, up to one diagonal (stopped at a tenth of its depth before the eye plane), traced
   across the screen (above); a hit counts only on a surface facing the ray. The light becomes
   `L + c · W · (L_hit − env)`, `env` the prefiltered environment the shading used, `c = (1 − r / 0.4) ·
   fade` with `fade` going to 0 over the last 0.05 of the picture's height at its border. Where the
   ray finds nothing the environment stays: that is the fallback. On the **ground** (where a background
   sample's ray meets it): the ray mirrored about the ground, traced the same way, its light mixed in by
   `groundReflection · F(cos)`, `F` a dielectric's Schlick Fresnel at `F0 = 0.04`; nothing found, the
   environment along the mirrored ray (at the background's blur).
4. *Depth of field* (`polish::Lens`, `dof_gather`), perspective pictures only (an orthographic camera
   has no lens). The camera is a full-frame one (24 mm high) whose lens, focused at `zf`, has the
   picture's field: image distance `v = 12 mm / tan(fov/2)`, focal length `f = v zf / (zf + v)`,
   aperture `A = f / N`, in model units through the document's length unit. The blur's radius in samples
   is `A h |z − zf| / (4 tan(fov/2) z zf)` (`coc_radius`; the background at `z = ∞`), at most 1/40 of
   the picture's height. The gather's reach is the most blur in the frame (the box's corners and the
   background); the taps are 4 rings of 8 i at `reach √((i − 1)² + f (2i − 1)) / 4` (ring `i` in its band
from `(i − 1) / 4` to `i / 4` of the reach, spread by area within it, so the frames cover the disc
evenly; 2026-10-30 review, it was `reach (i − 1 + f) / 4`), turned by the golden angle per
   frame, `f` 1 on frame 0 and the Halton (5) point after, so frames together cover the disc. A tap at
   distance `d` with blur `c` (the smaller of its and the centre's when it lies behind the centre)
   weighs `clamp(c − d + ½, 0, 1) / max(c, ½)²`; the centre `1 / max(c₀, ½)²`. A sample in focus with
   nothing in front of it keeps its light exactly.
5. *Bloom* (`polish::bloom`): 6 levels down (sizes halved, rounded up; reads clamped to the border),
   summed up again (`up_i = D_i + tent(up_{i+1})`), `B = (L + tent(up_1)) / levels`, mixed in as
   `(1 − s) L + s B`. Every filter's weights add up to 1.
6. Exposure, the tone curve, sRGB, bytes per sample, averaged per pixel as before. The plain background
   (`look.background` plain, layer 1) enters the buffers as its colour's linear light over the exposure,
   marked by the roughness −2 (`PLAIN_BACKDROP`): it reflects nothing, the lens and bloom treat it as any
   light, and it is shown as `srgb(L · 2^EV)` without the tone curve, so where nothing changed it, it is its
   colour.

A still of `n` frames (`render_studio`) sums each frame's pixel bytes exactly (integers) and rounds
once, `(Σ + n/2) / n`; ids and depths are frame 0's, so picking and the id parity are those of a single
frame. At most 256 frames.

**Rules and why they are general.** Every length is a fraction of the mesh box's diagonal or a number
of samples at the depth concerned, so the effects look the same at any scale and size; the lens alone
is tied to the length unit, on purpose (a real lens blurs a small part seen close more). The ray
tests are the published method's, with its known limits: what is not on the picture cannot be
reflected (the environment stands in), and a ray passing just behind a surface within the assumed
thickness is taken as a hit (`a_mirror_plane_reflects…` counts how often: 0 of 15 776 there).

**Tests** (`crates/render/tests/polish.rs`): a mirror floor under a red box against the mirrored rays
worked out apart from the renderer (every pixel whose mirrored ray enters the box's front face shows the
box: 716 of 716; of the pixels whose ray misses the box grown by 0.06, none does); contact shadows on
an open floor change nothing, and beside a box darken only the floor within the ray's reach of it; the
specular shadow only takes light away; the lens's radius against the textbook circle of confusion of a
50 mm f/2 lens at 1 m for a point at 2 m (to 1e-9, and the same in metres), and a lone bright sample
spread over 64 frames reaches the radius (9.49 for 9, within 1.5); the plane of focus keeps its picture
bit for bit while a far box blurs; bloom keeps an even picture even (to 1e-5) and a picture's energy
within a band (below); accumulation: frame 0 is the picture without it, the same frames give the same
picture, the change from 16 to 32 frames is smaller than from 4 to 8 and under 1 level on average;
bad numbers (an f-number of 0 or NaN, a focus behind the eye, infinite units, 0 frames) leave the lens
out rather than fail.

**Bloom's energy.** Away from the border the pyramid keeps the light's energy exactly: summed over
the texels below, the 13-tap filter gives every source texel a quarter of its light whatever its
place among the pairs it halves (the middle box 1/8, the four outer boxes 1/8, for either parity of
its row and column), and the tent gives every coarse texel four fine texels' worth
(`one_level_of_bloom_keeps_an_inner_spot_s_energy`: one level down and up, a spot at each of the four
phases, to 1e-6). What changes the sum is the border: reads are clamped to it, which counts a border
texel more than once, and a level of odd size rounds up; on the small levels most texels are at the
border. So a lone bright texel in a 96 × 80 picture keeps measured 0.998, 0.983 and 0.942 of its
light for strengths 0.04, 0.3 and 1 (its light reaches the border on the 6 × 5, 3 × 3 and 2 × 2
levels); the test holds that band, `|ratio − 1| ≤ 0.1 s`, as a regression bound. An even picture
stays exactly even. (Until the 2026-10-30 review this paragraph blamed the 13-tap weights for the
loss; they are exact in the interior.) An empty picture gives an empty one (`bloom`, `bloom_down`,
`bloom_tent`; they panicked on a zero size before).

**On the GPU** (`studio.wgsl`, `polish.wgsl`, `accumulate.wgsl`; `craft3d_gpu::studio`). Every studio mesh
pipeline draws through `vs_studio`, which moves clip x and y by the frame's jitter (and `across()` adds it),
so geometry and bodies agree. The geometry pass runs whenever occlusion, contact shadows, reflections or
the lens need it; contact shadows and the specular shadow are in the bodies' shading, which reads the
geometry. With a layer-2 effect that needs the light afterwards (`Look::post`), the background and the
bodies write the linear light and the weight with the roughness to two `Rgba32Uint` targets (`f32` bits),
then full-screen passes: `fs_reflect`, `fs_lens` (its taps come from `polish::dof_taps` in the uniform
block, never recomputed in the shader), `fs_bloom_down` / `fs_bloom_up` (levels as `f32` bits, the CPU's
sizes, clamped reads), `fs_display`, then the resolve. The readback renderer writes the face ids and
distances in a pass of their own (`fs_data`): light, weight and data together would pass wgpu's 32 bytes
per sample. A still of `n` frames is drawn as frames 1 to n − 1, then 0 (ids and depths from frame 0); each
frame's resolved bytes are summed exactly in a ping-pong `Rgba32Uint` pair, then `(Σ + n/2) / n`; the
shadow map is drawn once per picture. **The viewport** (`Onscreen`): with post effects or more than one
frame, it draws at one sample per pixel through the deferred path (the jittered frames smooth the edges
instead of MSAA; the bodies are drawn depth-only into the multisampled depth for the lines), and every
draw with the same mesh, camera, params and look adds frame `k`, up to `accumulate` (16 by default);
anything changing starts again at frame 0. `Onscreen::converging` tells the shell to draw again
(`Direct::frame` redraws and asks egui for another frame while it is true). The window's other paths and
`ui.screenshot` draw one frame unless the look gives `accumulate`. `ui.screenshot {window: true}` does
not count the window's frames while `Shell::converging` (`Desk::ask_capture`), so the picture of the
window is the settled still and not one of a timing-dependent number of frames; its 8 s deadline still
bounds the wait, with an error of its own (2026-10-30 review). On the CPU a still may not pass
`MAX_SAMPLES` (2²⁶) samples over all its frames (`RenderError::TooManyFrames`, refused before anything
is drawn): one still never costs more than the largest single picture.

**WebGL2.** `studio_as_drawn` also turns off reflections, the ground's reflection, contact shadows and the
lens (they loop over the geometry); bloom, the specular shadow (with no shadows there, it changes nothing)
and the accumulation stay. Every new shader translates to GLSL ES 3.00 (`template_tests`, *scene + studio
+ polish*, and `accumulate.wgsl`). Not yet run in a browser.

**Tolerance, GPU against CPU** (`crates/gpu/tests/polish.rs`, the band rule of layer 1; measured on the
Strix's Radeon 8060S over Vulkan and GL and on llvmpipe, three scenes, Check runs 40973956505 and
40976484395): ids equal everywhere outside the band. Readback: each effect alone (contact shadows,
specular shadow, ground reflection, lens, bloom, 8 frames) at most 4 of 175 721 pixels more than one level
apart and 2 to 3 levels at most; reflections 8 of 175 721 (4 of 44 183 in one scene), one pixel 50 levels
apart (a reflected ray that finds a surface on one renderer and passes just beside it on the other: a hit
decision in `f32` against `f64`); all together 3 of 175 721, at most 6 levels (9 on GL). The viewport, 1, 2
and 8 frames: 20 of 44 197 at worst, 48 levels.

**Pixels in doubt (2026-10-30 review).** The large differences were all decisions of a traced ray that the
GPU's `f32` took otherwise, so instead of a bound wide enough for them (it was 96 levels for looks that
trace a ray and for the viewport, 16 for the rest) the CPU marks them and the comparison leaves them out,
as it leaves out the band. `polish::trace` marks a ray (`Traced::doubt`) where a decision lies within
`TRACE_DOUBT · ε_f32 · M` of its boundary, `M` the largest magnitude the compared numbers come from, and
taking it the other way changes the outcome: a step's hit test (`z ≥ zs`, the thickness's limit); a step's
position near a sample's border where the sample across it decides that test otherwise; the first
halving in doubt (its depth test, or its position near a border with the sample across deciding
otherwise) only when a sample the ray passes over in the interval it halved shows another face than the
hit (within one face the light changes by the shading's slope, not by a jump); the hit position near a
border only when the sample across shows another face; and the facing test within `TRACE_DOUBT · ε` of 0.
`TRACE_DOUBT` is 128: counting the roundings (the depth read, the point rebuilt and projected, the
`1/z` interpolation with WGSL's 2.5-ULP divisions, FMA contraction) gives about 32, and a sweep of the
margin (32, 64, 128, 256, 512; Check run 40998600287) found the three pixels the GPU decided otherwise
between 32 and 128: the rasteriser's interpolation of the depth, which the APIs leave unspecified, errs
more than the count. `render_studio_doubt` gives each pixel the share of its light from samples in doubt
(over its samples and frames, through the lens's gather by its weights; bloom's spread, at most its
strength, is not counted). *Measured* (Check run 41000166136, the three adapters): with pixels whose share
passes 1/64 left out, at most 6 levels anywhere (3 for looks without a traced ray) and at most 1/6 000 of
the pixels more than one level apart; left out, at most 1.8 % of the pixels (the polished look with the
lens and 8 frames on the scene whose mirror ball and plate reflect three other parts; 0.36 % for
reflections alone in one frame) and a mean share of 0.14 % of the light. That is more than the 1/4 000
the review hoped for: a mirror reflecting curved parts has many rays whose hit lies at another face's
edge, and accumulation and the lens take a pixel out when any of its frames or enough of its taps were in
doubt. *Bounds:* outside the band and the pixels in doubt, at most 1/512 of the pixels more than one
level apart and at most 8 levels; pixels in doubt at most 1/32, mean share at most 1/512. The same input
gives the same frame bit for bit, accumulated too, on every adapter.

**Cost** (`polish_frames_on_a_thousand_faces`, 1 002 faces, 190 096 triangles, 1020 × 777, the Strix's
Radeon 8060S): the viewport over Vulkan (4 samples for layer 1) 0.93 ms for layer 1, 1.69 ms polished
(`Look::polished`), 2.80 ms with the lens, 1.70 ms a frame accumulating 16; over GL 0.88, 2.58, 3.80 and
2.60 ms. The readback renderer with its read back: 17.5 ms, 20.9 ms, 26.0 ms, and 90.2 ms for a still of
16 frames (Vulkan). The CPU renderer: 704 ms for the polished frame (183 ms for layer 1's).

**Looked at** (2026-10-30, `target/evidence/render-rl2/`: `panel-*-{plain,layer1,layer2,layer2-dof}.png` of the
flange, the gear, a metal flange (roughness 0.2), the gear-reducer Jig and the turbofan, and the side by
side `cmp-*.png`; the GPU's against the CPU's in `gpu/`). The metal flange reflects the ground and its
own rim; the lens blurs the front rim with the centre sharp; no banding, no acne from the contact
shadows. Two findings. (1) The ground's reflection, at full Fresnel strength, lifted the whole
background (the top of the picture from 161 to 190): the ground has no edge, and towards the horizon it
is seen at a grazing angle where a dielectric reflects nearly all of the sky. It also showed the
method's limit plainly under the gear reducer: only what is on the picture can be reflected, so the
reflection is in pieces (the undersides of parts are never on the picture). So `Look::polished()` leaves
it off (`groundReflection` 0); a ground of finite extent (a studio floor fading out around the model)
and a rougher, blurred reflection would be the general fix. (2) The CPU takes 0.58 s for the polished
flange at 480 × 360 against 0.16 s for layer 1, and 11 s for 16 frames with the lens: fine for
`view.render` and the tests, not for a CPU viewport, which is why the window's other paths draw one frame.

### The bodies' edges: shown or not, and under the lens (2026-10-30)

The developer looked at the turbofan in the studio look with depth of field: the black edges, drawn
after the picture is shaded, stay sharp where the model behind them is blurred, and could not be hidden.

**`show.edges`** (`ui.set {"show": {"edges": false}}`, the *Edges* toggle on the display bar) hides the
bodies' edges and the outline of a section's cap with them, in every look, on both drawing paths and in
`ui.screenshot`, as `show.sketches` and `show.datums` do: `Marks::hide_edges` leaves the bodies' edges
out of the CPU overlay (`build_overlay`, `Marks::outline`) and out of the GPU's segments
(`direct::segments_marked`). A highlighted or selected edge or vertex is drawn either way: it is feedback
on what the pointer or the selection names, not a line of the model. Shown, nothing changes, bit for bit.

**The studio look's `look.edges`: `soft` (the default), `sharp`, `off`.** Two ways were open: draw the
edges into the shaded picture before the lens and bloom, so they blur with it, or keep drawing them over
the finished picture and make them follow the lens there. *Decision: the second, as a look option.*
*Reasons.* (1) The edges are an overlay of the window, not part of the picture the studio look makes:
`view.render` and `craft3d-cli render` draw no edges, and the lens pass of both renderers
(`studio::polish`, `polish.wgsl`) gathers from the shaded colour and its depth only. Drawing edges into
it would add a second line rasteriser inside the CPU's studio frame and a line pass into the GPU's
studio target before the lens, each with its own depth handling, and make the edges' hiding test
(`depth_test.wgsl`, the CPU's `stretches`) part of the lens's input: a much larger change to the oracle
for a line that is a few pixels wide. (2) A thin line spread over a circle of confusion mostly
disappears into the blur anyway: a line `w` pixels wide spread over a disc of radius `c ≫ w` leaves about
`2 w / (π c)` of its ink at its middle. So `soft` keeps each edge pixel's share `w / (w + π c / 2)`
(`viewport::EdgeBlur::fade`), `c` the lens's blur radius at that pixel's distance along the view, from
the same `Lens` the polish pass uses (`a h / (4 half zf) · |z − zf| / z`, capped at `DOF_MAX_COC`): 1 at
the focus, fading with the blur. The distance along a line goes evenly in its reciprocal, as the GPU
interpolates it (`EdgeInk::depth`, `EdgeBlur::fade_along`; `lens_fade` in `lines.wgsl` takes the same
three numbers in `more.yzw`). (3) The developer may want edges crisp for a technical look or gone for a
clean one: `sharp` is the plain look's lines, `off` leaves them out of the studio look; the *Edges*
toggle hides them in every look. *Default `soft`*: without a lens it is `sharp` bit for bit, so only a
picture with depth of field changes. The option is the window's (`LookState::edges`, not the engine's
`LookSpec`), since only the window draws edges. The plain look ignores it.

*Limit.* `soft` fades, it does not spread: an edge far from the focus becomes faint, not wide. Bloom
does not reach the edges either. Both would need the edges in the picture before the lens, the first
way, should it ever be wanted.

*Tests.* `crates/ui-egui/tests/show_edges.rs` (toggling changes only the pixels near an edge, and back
restores the same bytes; a highlighted edge stays; both paths honour the marks; `look.edges` checked,
plain look unchanged; `soft` = `sharp` without a lens; the fade's formula); `gpu_viewport.rs`
`the_direct_path_fades_edges_with_the_lens_as_the_oracle_does` (along the edges, where the CPU's soft
and sharp pictures differ by 24 levels or more, at least two thirds of the GPU's pixels are nearest the
soft one); the layout tests with the *Edges* chip. With it the 1280 × 800 window's display bar is too
narrow for the three toggles' names with the section open: those three show their icons (their names as
tooltips) before any other chip loses its name.

### Layer 3: materials (2026-10-30)

Layer 3 of `plan/rendering.md`: the layers of a material beyond its colour, metallic and roughness,
kept in the document (`craft3d_doc::Optics` on `Appearance` and `BodyAppearance`, format 54) and
shaded by the studio look in both renderers. `craft3d_render::studio::layers` is the definition (the
closed forms), `FrameLight::shade_layered` and `studio::refract` the shading; the GPU repeats them
(`studio.wgsl`). A face without layers (`FaceColors::optics_slot` 0) is shaded as before bit for bit:
every layer enters as a factor of 1 or a term of 0, and the plain look does not read them.

**Sources** (clean-room: nothing of Blender's Eevee or Cycles, which are GPL, was read or transliterated):

- B. Burley, *Physically-Based Shading at Disney*, SIGGRAPH 2012 course: the clear coat as a second,
  fixed-index specular lobe over the base, and the anisotropy parameter.
- C. Kulla and A. Conty, *Revisiting Physically Based Shading at Imageworks*, SIGGRAPH 2017: layering a
  coat by weakening the base by the coat's directional albedo, and `αt = α (1 + a)`, `αb = α (1 − a)`.
- Google Filament, *Physically Based Rendering in Filament*: § Anisotropic model (the anisotropic GGX
  `D` and the anisotropic height-correlated visibility, after E. Heitz 2014; the bent normal for
  image-based light), § Clear coat model (the coat at `F0` = 0.04, the base times `1 − Fc`), §
  Transmission and refraction (the screen-space refraction of a solid or thin object, the transmitted
  light tinted by the base colour and absorbed by Beer–Lambert's law).
- B. Walter, S. Marschner, H. Li, K. Torrance, *Microfacet Models for Refraction through Rough
  Surfaces*, EGSR 2007: the rough dielectric interface. Its transmitted lobe is not traced here (the
  refracted ray is the smooth one); named as the method for blurred refraction.
- The wrap profile is the normalised wrapped cosine `max(0, n·l + w) / (1 + w)²`; its spherical
  harmonic weights were derived here (below), checked against quadrature.

**Data.** `Optics`: `anisotropy` (−1 to 1) and its `direction` (`Parametric`, or `Axis([x, y, z])`),
`clearcoat` and `clearcoat_roughness` (0 to 1), `transmission` (0 to 1), `ior` (1 to 3, default 1.5),
`thin_walled`, `absorption` (an sRGB colour or none) after `absorption_distance` millimetres
(default 10, whatever the document's unit, as the density is in kg/m³), `subsurface` (0 to 1) and
`subsurface_color` (the base colour when none). The manifest writes `optics` only when it is not the
default and each field only when it is not its default; an unknown field is refused, a value out of
range too (`Optics::check`). Format 54, `v53_to_v54` an identity step (the developer's pre-release
policy: older files have no layers, which is what a missing field reads as). The engine turns it into
`craft3d_render::studio::layers::Optics` per face (`appearance::render_optics`): `f32` numbers (both
renderers read the same), the absorption as coefficients `σ = −ln(c) / d` per model unit, and the
tangent field from the face's surface (`appearance::tangent_field`): a plane's first axis; around the
axis of a cylinder, cone, sphere or torus (`axis × (p − origin)`, the `u` direction of each). A
freeform face takes its `u` direction at the middle of its domain as a fixed axis: an approximation;
the general method, per-point tangents from the tessellator's parameters, is open. A field that gives
no tangent at a point (a fixed axis along the normal; a point on the axis) shades that point without
anisotropy (`TANGENT_EPSILON`, 10⁻⁴ of the field's length: scale-free).

**Method** (per sample, after layer 1's light and before layer 2's reflections; `shade_layered`):

1. *Anisotropy* `a` with the tangent `t` (the field projected on the tangent plane) and `b = n × t`.
   The environment along Filament's bent reflection: `n' = normalize(mix(n, n_a, |a| · saturate(5r)))`
   with `n_a` the normal the stretched lobe sees. The key light is part of the prefiltered
   environment, so its isotropic share there (the lobe widened by the key's size, estimated at the bent
   normal, where that light is centred) is taken out, `env · (1 − share)`, and the key's anisotropic
   lobe added instead: `E · n·l · vis · D_aniso V_aniso F` with `αt, αb` from `α' = r² + radius/2`
   (as layer 2's specular shadow widens it), at least 0.002 each. No reflection replaces the bent
   lobe (its weight is 0): it is no mirror.
2. *Subsurface* `w`: the diffuse light becomes `(1 − w) · base · E₀ + w · c_s · E_w` (times the usual
   `(1 − m)(1 − Fs) / π`), `E_w` the irradiance through the wrapped kernel: the nine harmonics weighed
   per band by `1`, `(1 + 3w/2 − w³/2) / (1 + w)²` and `(1 − w)²` against the clamped cosine's (from
   `2π ∫ k(t) P_l(t) dt` over `−w ≤ t ≤ 1`; `the_wrap_bands_are_the_kernels_harmonics` checks them
   against quadrature), the key through the wrapped cosine with its shadow where `n·l + w > 0`. Both
   kernels integrate to π, so a white surface in a furnace stays white at any `w`.
3. *Transmission* `T`: the diffuse light times `1 − T`; the light behind times
   `T (1 − m)(1 − Fs) · base` (`Shaded::transmit`), added by `refract::pass_through`.
4. *Clear coat* `c` of roughness `r_c`: with `(A, B)` the split sum at `n·v, r_c`, `Fc = c (0.04 A + B)`;
   everything so far (and the transmitted factor) times `1 − Fc`, plus the environment along the mirror
   direction at `r_c` times `Fc` times its specular occlusion. The reflections then replace the coat's
   light (weight `Fc`, roughness `r_c`): the coat is the sharpest layer. The coat is weakened by the
   view direction's albedo only (Filament's simplification of Kulla–Conty's `(1 − E(μo))(1 − E(μi))`,
   the incoming direction being integrated by the environment); the base's `F0` is not corrected for
   the coat's index.

**Through transmissive faces, in screen space** (`refract.rs`; Filament's approximation, documented as
such). A face with `T > 0` is *transmissive*: it casts no shadow (the key's shadow map is drawn
without it) and is left out of the picture it sees. When a picture has one, the frame is drawn twice
more: the picture without the transmissive faces (its own occlusion and contact shadows), and the
nearest back faces of the transmissive faces (by the triangle's own normal against the direction from
the eye). A transmissive sample then adds `transmit · exp(−σ L) · behind(x)`:

- thin-walled, or no back face behind it (an open sheet): `x` is the sample itself, `L` 0;
- solid: the view ray refracted at the front surface (Snell, index 1 to `ior`), along it
  `L = Δ · cos θi / cos θt`, `Δ` the view ray's length between the front surface and the back face
  under the sample: the path a slab with parallel faces would give; its end projected to the sample
  it lies in (`floor`). Off the picture, the background along the view ray from that end.

For a slab of parallel faces in an orthographic picture this is exact: the ray leaves parallel to the
view, so what it sees lies behind its exit point, which is offset by the textbook
`t sin θ (1 − cos θ / √(n² − sin² θ))`. Elsewhere its limits are the method's: one transmissive layer
(glass behind glass is not seen), the back face taken as parallel to the front, only what is on the
picture refracted, no blur for a rough interface. A picture with transmission always takes layer 2's
path through the buffers (the light behind is light, not bytes). *Doubt*: the exit's sample is a
decision the GPU's `f32` may take otherwise; within `TRACE_DOUBT · ε_f32 · max(w, h)` samples of a
sample's border with another face across it (or of the picture's border), the sample is marked as
layer 2 marks a traced ray, and `render_studio_doubt` merges both marks.

**Exports** (what each keeps; the `.3dcraft` file keeps all):

| | keeps | drops |
|---|---|---|
| OBJ (material library) | colour `Kd`, `Pr`, `Pm`, clear coat `Pc`, `Pcr`, anisotropy's size `aniso` (with `anisor 0.25` for a negative one), transmission as dissolve `d = 1 − T` and filter `Tf` (the absorption colour, else white), `Ni` | the anisotropy's direction, the absorption's length, thin-walled, subsurface, a solid texture (its mean colour is `Kd`, layer 3.5) |
| 3MF (core base materials) | the display colour, with alpha `1 − T` (`#RRGGBBAA`) for a transmissive one | metallic, roughness, every other layer, a solid texture (its mean colour is the display colour) |
| STEP (AP214 presentation) | the colour; a transmissive one's `SURFACE_STYLE_TRANSPARENT` (`T`) in a `SURFACE_STYLE_RENDERING_WITH_PROPERTIES` beside the fill | metallic, roughness, every other layer, a solid texture (its mean colour is the colour) |

**Presets** (`crates/engine/src/material_presets.json`, data read as a command's parameters):
`steel`, `aluminium-brushed`, `abs`, `glass`, `car-paint`, `wood`, each an appearance and a material
(name and density). `material.preset` gives one to a body in one undo step, any parameter beside it
replacing the preset's number; `material.presets` lists them. There is no material panel in the
window yet; the presets are commands only.

**Tests** (`crates/render/tests/layers.rs`): the white furnace for every layer, dielectric and metal,
roughness 0 to 1, at 20 angles (anisotropy −0.8 to 0.95, clear coats, subsurface, transmission with
the light behind the furnace's own): within 0.98 to 1.02, as layer 1's; the clear coat weakens the
base by exactly `1 − Fc` and adds `Fc` times the environment at its roughness (to 10⁻⁹); the
anisotropic highlight of a point key behind the viewer: the ratio of its half-maximum widths along and
across the tangent against the closed form of GGX's `D`,
`α √((√2 − 1) / (1 − √2 α²))` for `αt` over the same for `αb` (within 3 %; the visibility and `n·l`,
which the closed form leaves out, move it by about 1 %), and the widest of 72 directions is the
tangent's (the one across it for a negative anisotropy), for four tangents; a glass slab of thickness
`t` and index `n` turned by `θ` in front of a red and blue plate, orthographic: the plate's edge seen
through it moves by the closed-form offset within 1.5 pixels, not at all when thin-walled; a
layered scene is the same twice and marking doubt does not change it. `layers.rs` unit tests: the wrap
bands against quadrature, Snell, the fields, sanitising. Engine: `tests/materials.rs` (the presets,
the parameters said back, saving and opening, bad values refused, the exports' keys), the scene's
wire form carries the layers, a format round trip and the v54 fixture.

**On the GPU** (`crates/gpu`: `studio.wgsl`, `mesh.rs`, `studio.rs`). Each vertex carries one more
word (52 bytes a vertex): its face's slot in the low 16 bits (`FaceColors::optics_slot`, 0 for none)
and bit 16 when the face is transmissive (`mesh::optics_word`). The distinct layers go into a material
table uploaded with the mesh (`mesh::optics_table`, an `Rgba32Uint` texture of six texels a row, `f32`
bits: the numbers, the field's axis, the origin of a field around an axis relative to the mesh's
origin), bound in the studio's main group and read with `textureLoad` only. `studio_shade` is
`shade_layered` term for term (the frame from the field at the fragment's point, the bent reflection
and the key's share at the bent normal, the wrapped harmonics by band, the coat, `Shaded::transmit`);
a fragment of slot 0 takes the same arithmetic as before. The shadow pass discards transmissive
fragments. With transmission (`Passes::transmit`, which turns `post` on, and the viewport's deferred
path) the readback renderer and the viewport draw, before the frame, the picture without the
transmissive faces (`fs_geometry_opaque`, its own occlusion, the background and `fs_studio_behind`
into a light target) and the nearest back faces of the transmissive faces (`fs_backs`: transmissive
and `flat · rel > 0` in perspective, `flat · forward` in orthographic, which on a triangle's plane is
the sign of the CPU's `flat · (corner − eye)`; their view distance, infinity for none). The frame's
`fs_studio_post` then adds `transmit · exp(−σL) · behind(exit)` as `refract::pass_through` does,
before the reflections. WebGL2 keeps transmission (the passes use no loop over texels and translate
to GLSL ES 3.00; `studio_as_drawn` leaves the layers alone), so the CPU reference of a WebGL2 frame
is the one of `studio_as_drawn`'s look, as before.

*Tolerance* (`crates/gpu/tests/layers.rs`, run 41022429587 and 41023163195, the Radeon 8060S on RADV
and on radeonsi, llvmpipe; two scenes, each layer alone and all together, readback and viewport):
layer 2's comparison (the band, the pixels in doubt at `TRACE_DOUBT` left out), with the band of what
transmissive faces show added: where a sample sees the picture behind straight, that picture's band;
where it refracts, the samples whose ray leaves within one sample of a border between two faces
behind (`render_studio_doubt` with a margin of one sample). Measured: ids equal; at most 3.4 · 10⁻⁵ of
the pixels over one level; at most 6 levels (RADV's orthographic scene, terms without layers alike;
1 on the others); the transmissive faces' pixels at most 3; at most 0.73 % in doubt. Bounds: 1/4096,
8 levels, 5 for the transmissive pixels, 1/64 in doubt. On the viewport (one step) a pixel that
differs between two draws of the same picture is a lost write, left out and counted (none measured,
at most 4). Every term is compared on the viewport in the polished look with its traced effects, a
single frame for a layer alone (so the way layer 3 feeds the reflections, the coat's weight `fc · so`
and roughness `rc`, the anisotropy's weight 0, is held on one frame) and four for all together. One
reflection is decided otherwise on RADV's viewport outside the marked samples (the plate, which has no
layer, reflecting the post with anisotropy around its axis at its silhouette, 13 levels at one pixel;
1 untraced, 1 on llvmpipe and radeonsi): the CPU marks that ray at 1024 epsilons of its boundary and
not at 256 (run 41029229083), so RADV's viewport arithmetic errs there by more than `TRACE_DOUBT`'s
128, which was measured on the readback renderer. The viewport comparison leaves out at most one pixel
over the bound per picture, and only one the CPU's doubt at `8 · TRACE_DOUBT` marks (`WIDE_DOUBT`,
`MOST_WIDE`); raising `TRACE_DOUBT` itself for one ray would widen every comparison of layer 2 and 3.
A light-based doubt (a hit whose neighbour on the same face reads light more than a level apart) was
tried first and dropped: it did not mark this ray (its decision is not near a sample's border) and
raised layer 2's doubt to 3.5 %, past its bound.

*The plain background through glass* (2026-10-30). The plain background is shown as it is, without
the tone curve (`PLAIN_BACKDROP`); seen through a transmissive face it used to enter that face's light
as its colour over the exposure and went through the tone curve with it, so clear glass on white came
out grey under AgX, and a refracted ray that left the picture showed the environment instead of the
plain colour. Now both renderers give the plain colour (times `ground_factor`, on the picture and off
it) as the light the tone curve shows as that colour, `FrameLight::untoned` = `tonemap::undisplay` of
its display value over the exposure (`untoned` in `studio.wgsl`; the GPU marks the plain background
in the light's `w` of the picture behind). `undisplay` inverts each curve: Linear by sRGB, ACES by its
quadratic, AgX by its inset matrix (the inverse of the outset one), halvings of its monotonic sigmoid
and the outset matrix; AgX does not reach a bright saturated colour (it turns them towards white by
design) and gives a paler one. Held by `undisplay_gives_back_every_byte_the_curve_reaches` and
`clear_glass_on_a_plain_white_background_stays_white` (CPU, and GPU against 250 on every adapter). Cost (1 002 faces, 1020×777, readback with the readback itself, every seventh part solid
glass): RADV 22.5 → 23.5 ms layer 1, 18.5 → 27.5 ms polished; radeonsi 18.8 → 24.8 and 24.7 → 28.1 ms;
the CPU 2.0 s.

### Layer 3.5: solid textures (2026-10-30)

The developer's decision of 2026-10-30: a material can carry a 3D pattern evaluated in the model's
space, with no mapping of the surface. B-rep faces have no tidy parametrisation to hang a picture on,
and a cut or a section shows the pattern running through the body, as in a real material.
`craft3d_render::studio::pattern` is the definition (`f64`); the GPU repeats it in `studio.wgsl`
(`pattern_of`, `noise3`, `cellular3`, `pattern_value`, `pattern_color`) in `f32` from the
interpolated position. The document keeps it as `craft3d_doc::SolidTexture` in a material's
`Optics` (`pattern`, format 57), so it travels wherever the layers of layer 3 travel (body and face
appearances, the rule by which faces of later features inherit them, the scene's wire form, the
replay).

**Sources** (clean-room, from the literature; no code of any renderer was read):

- K. Perlin, *An Image Synthesizer*, SIGGRAPH 1985 (solid textures, turbulence, marble as
  `sin(x + turbulence)`), and *Improving Noise*, SIGGRAPH 2002 (the quintic fade `6t⁵ − 15t⁴ + 10t³`
  and the twelve edge gradients chosen by a hash's low four bits).
- S. Worley, *A Cellular Texture Basis Function*, SIGGRAPH 1996 (the nearest of one jittered feature
  point per cell, over the 27 cells about the point).
- D. Ebert, F. K. Musgrave, D. Peachey, K. Perlin, S. Worley, *Texturing & Modeling: A Procedural
  Approach*, 3rd ed., 2003: fBm (octaves of lacunarity 2 and gain ½), wood as rings about an axis
  wobbled by noise, early and late wood.
- K. Perlin's reference implementation published with *Improving Noise* (`ImprovedNoise`, 2002):
  the choice of the gradient by the hash's low four bits (`grad`).
- M. Jarzynski and M. Olano, *Hash Functions for GPU Rendering*, JCGT 2020: `pcg3d`, three `u32`
  mixed by wrapping products, sums, xors and shifts only, so every CPU and GPU computes the same
  bits. No driver's noise, `fract`-of-`sin` hash or texture is used.

**The noise.** A lattice point's hash is `pcg3d(cell + offset(seed, octave))` (the offset is itself
`pcg3d(seed, octave, constant)`: another seed or octave is the same field translated by 2³² cells
at most, far beyond any part). Gradient noise: Perlin's improved noise with that hash, 0 at every
lattice point, about −1 to 1 (variance 0.073). fBm and turbulence over 1 to 6 octaves. Cellular
noise: the distance to the nearest feature point and a number in [0, 1) of its cell (a second
`pcg3d` of the cell's hash; the hash's top 24 bits are exact in `f32`). The pattern's local
coordinates (`Pattern::local`: the point minus the origin, in the pattern's frame, over the scale)
are clamped to ±2²⁰ features so that the cells fit an `i32` on both renderers; a pattern is
deterministic everywhere and sharp on the GPU to about 10⁴ features from its origin (there `f32`
resolves a thousandth of a feature; at 2²⁰ only an eighth).

*Platform-free on the CPU.* The CPU's picture and the mean colour the document stores must be the
same bits everywhere (rule 5): the standard library's `sin` and `atan2` are the platform's and may
differ in the last bit, so the pattern uses its own (`sin_pi`, the Taylor series after folding into
±π/2; `atan2_exact`, two angle halvings with the exact `√` and a series), made of `+`, `×`, `/` and
`√` only, within 10⁻¹⁴ of the standard ones. The GPU uses its own `sin` and `atan2` (in doubt as any
`f32` arithmetic). The one platform function left is the sRGB decode's `powf` of the ramp's bytes,
shared with every colour the renderer draws: a 1-ulp difference there can move a mean colour's byte
only where it lies exactly on a rounding boundary.

**Kinds** (`Pattern::value`, a place `t` on the ramp, 0 to 1):

- `wood`: `r = √(x² + y²) + distortion · fBm(0.35x, 0.35y, 0.03z)` (at most two octaves: where a face
  cuts the rings at a grazing angle, a board's face, a radial wobble is stretched along the face by
  about one over the angle, and finer octaves become teeth) about the axis (`z`), in rings of the
  scale; `t = smoothstep(0.35, 0.85, f) (1 − smoothstep(0.9, 1, f))` of `f = fract(r)`: early wood
  widening into late wood, back to early wood at the ring's border, continuous (no jump to alias);
  plus `grain · 0.18 · noise(12x, 12y, 0.15z)` (fine streaks along the axis) and minus
  `rays · 0.7 · smoothstep(0.25, 0.45, noise(48 x/r, 48 y/r, 0.6 z + 0.3 r))` (medullary rays:
  constant along a radius, thin across it);
- `marble`: `t = (1 − |sin 2π u|)⁴ + 0.1 (1 + fBm(2q))`, `u = z + distortion · turbulence(q / 2)`
  (veins across the axis, faint clouds between them);
  the sine takes `u`'s place in its period (`−sin(π(2 fract(u) − 1))`), so the GPU's `sin` gets an
  argument within ±π, where it is accurate;
- `granite`: the cell's number, quantised to the ramp's stops (each crystal one mineral's colour),
  blended with the second nearest cell's within a thin band along their border:
  `t = t₂ + (t₁ − t₂)(½ + ½ smoothstep(0, 0.08, F₂ − F₁))`, so the pattern has no jump for the two
  renderers to decide apart (a hard border was decided otherwise by RADV at single pixels, by up to
  71 levels, run 41070463971);
- `speckle`: a dot of radius `spot · (½ + ½ v)` cells about each feature point; `spot` 0 is no
  dots (both renderers test the radius: `smoothstep` of an empty range is undefined in WGSL);
- `noise`: `½ + 0.6 (1 + distortion) fBm(q)`: cast iron, stone, a variation of roughness;
- `brushed`: `½ + 0.9 fBm(x, y, z/50)` over at most three octaves, plus `grain` scratches: streaks
  along the axis;
- `carbon`: a 2×2 twill. Tows of the scale's width; cell `(i, j)` of the weave's coordinates
  `(a, b)` shows the warp (along `a`) where `k = (i + j) mod 4` is 0 or 1, the weft where it is 2
  or 3, each floating over two cells; the brightness is `amp · sin(π · across) · sin(π · along / 2)`
  (`along` the place in the float, 0 to 2; `amp` 1 for the warp, 0.55 for the weft): round across
  the tow and along its float, 0 at every border, so the weave has no jump either. With `wrap` (the default) the weave is wrapped around
  the axis: `a` along it, `b` around it in tows, their number around rounded to a multiple of 4 for
  each radius (so the weave closes on a tube without a seam). Without it, the weave lies in the
  plane across the axis (a plate whose normal is the axis). *Where an object-space weave looks
  wrong, honestly:* on a face that is neither a surface of revolution about the axis nor a plane
  across it (a plate's edges, a tube's end, a doubly curved shell) the weave is the solid pattern's
  section there, stretched or polar, not a cloth laid on the face; and where the radius crosses a
  rounding of the tow count (an end face) the weave changes count with a visible step. Draping a
  weave on a freeform face needs a parametrisation of the face, which is what this layer avoids;
  it is owed as a texture mapped by the tessellator's parameters.

The ramp's colours (2 to 4, sRGB) are spaced evenly and blended in linear light (the GPU's table
holds them decoded, as `f32`, and the CPU reads the same numbers). A roughness ramp
`roughness: [at 0, at 1]` replaces the appearance's roughness; without one the appearance's is kept.
With `alongAxis` (wood and brushed by default), a face's anisotropy runs along the pattern's axis.
In the studio look the texture replaces the base colour and roughness in `shade_layered` (and
`studio_shade`) before every other layer, so the clear coat, the reflections and the glass see the
textured surface. The plain look shows the appearance's colour. The point is the one the picture
draws: a part placed by a pose (*Parts moved by poses*) is textured where it is placed, so in a
motion its texture stays in the world while the part moves (owed: the part's own space, its pose's
inverse applied to the point). A section's cap and a see-through picture are in the plain look, as
for layers 1 to 3.

**Defaults** (`craft3d_engine::solid_texture::fitted`, applied by the command and stored, so a later
edit of the body does not move the grain, and a body moved by an earlier feature slides through its
texture; a mode that follows the body, fitting at drawing time as `render_pattern` already does for
a missing axis or origin and moving with the body's placement, is owed): the axis along the longest side of the body's box (ties
in x, y, z order); wood's tilted towards the thinnest side by 0.06 (a board is never sawn exactly
along the pith, and the tilt is what makes the rings show as arches on its face) with the pith 0.6
of the middle side from the centre towards the thinnest side, outside the body; every other kind
about the box's centre. The scale: wood a twentieth of the middle side (rings that read at a
part's size), marble the longest side over 4, granite over 80, speckle over 50, noise over 6,
brushed over 100, carbon a twelfth of the middle side. A file written by hand without an axis or an
origin gets them from the body's box at drawing time. Without a colour the appearance takes the
texture's mean colour (`Pattern::mean_color`: 24³ points of a cube of 12 features, averaged in
linear light), which the plain look and every export show.

**Doubt** (the GPU-vs-CPU rule of the other layers, for a function of the point): a sample is in
doubt where moving its point by `PATTERN_DOUBT` (provisional 8) `f32` epsilons of the magnitudes the
GPU computes it from (the eye and the point relative to the mesh's centre, the point relative to the
pattern's origin) along any axis moves a channel of the texture's colour, or its roughness, by more
than one level (`Pattern::in_doubt`, marked by `render_studio_doubt`), or by `SUBPIXEL_DOUBT`, a
256th of a sample, if that is more: the GPU's rasteriser snaps the vertices to its sub-pixel grid
(8 bits on the adapters measured; Vulkan requires 4), so the point its fragment interpolates is not
quite the CPU's. Steep places are where this happens: a ring's late wood, a speckle's rim, the fine
streaks of brushing. A pattern finer than a sample aliases (there is no filtering of a procedural
texture by the pixel's footprint; owed: band-limiting each octave by it, Ebert et al. ch. 2) and is
then mostly in doubt: brushing at a scale of 1.6 pixels left 5.7 % of the pixels in doubt (run
41072278069), past the bound of 1/64, so the defaults keep features at several pixels (brushing's
scale is the longest side over 100, its scratches three times finer). Every kind is continuous (no jump anywhere but where the carbon weave's tow
count changes with the radius, on an end face), so the doubt stays a thin set.

*Measured* (Check runs 41093289950 and 41094540615, 2026-10-30; the two layer-3 scenes, each kind
on two parts, readback and viewport, RADV, radeonsi, llvmpipe). Ids equal. radeonsi and llvmpipe
within 1 level everywhere compared. RADV's orthographic scene differs at one plate pixel, (168,
150), in every term, layers or not: 5 to 6 levels without a layer (what layer 3 recorded), 7
(brushed, carbon) and 8 (noise, the bound itself) with a texture on the plate there; the same
decision of the scene's, worth more levels on the texture's colour, not the texture's arithmetic
(the other two adapters agree there within 1). The comparison now has a baseline term without any
layer, and leaves out (and counts, at most 4 a picture; 2 measured) the pixels that baseline already
misses by more than half the bound (`excuse_shared` in `crates/gpu/tests/layers.rs`). With them out:
RADV's readback at most 2 levels with a texture, its viewport at most 6 (granite on the ring); at
most 5 · 10⁻⁵ of the pixels over one level; in doubt at most 1.3 %. Layer 3's bounds hold with their
margins. The run prints its numbers (`MEASURE`, `WORST`) when green with `-- --nocapture`.

**Cost** (`crates/ui-egui/tests/solid_texture_cost.rs`, the turbofan, 262 458 triangles, five bodies
textured with carbon, cast iron, oak, marble and walnut, 1280×800, readback with the read back, on
the Strix): RADV 8.66 → 9.29 ms (layer 1), 6.94 → 7.48 ms (the simpler WebGL2 look); radeonsi (GL,
the path closest to WebGL2) 4.43 → 4.63 ms and 2.95 → 3.15 ms; llvmpipe unchanged within noise (194,
84 ms). About 0.2 to 0.6 ms a frame.

**Exports.** OBJ's material library, 3MF's base materials and STEP's presentation cannot hold a
procedural texture: each exports the appearance's colour, which the commands set to the texture's
mean (the tables of layer 3 above: the `pattern` is in the *drops* column of every format). Baking a
texture into a picture (`texture.bake`, a PNG per face in the tessellator's parameters) is owed.

**Tests.** `crates/render/tests/pattern.rs`: the hash, the noise, fBm and the cellular noise bit for
bit against an independent evaluation of the same published formulas; the statistics (mean within
0.01 of 0, variance 0.055 to 0.09, fBm's variance 1.33 times the noise's for four octaves as
independent octaves add, cellular values uniform, the nearest point about half a cell away); zero on
the lattice; no period from 1 to 256 cells along any axis and no correlation between seeds (|ρ| <
0.1); wood without noise against the closed form along a radius at scales 0.004, 1.5 and 3000, its
rings one scale apart; every kind on its ramp with its roughness ramp, its mean colour between the
ends, a ramp of one colour its own mean; sanitising; patterned pictures deterministic, different
from plain, marking doubt changing nothing. `crates/gpu/tests/layers.rs`: each kind on two parts of
the two layer-3 scenes, readback and viewport, on every adapter, with layer 3's tolerance. Engine
(`tests/solid_textures.rs`): the presets fitted to a board (walnut's rings 4 apart on a board 80
wide, the pith 48 below its centre, the axis `[1, 0, 0.06]`), a pattern said back given again, saved
and opened, a face's own pattern, a cut through an oak block whose cut faces carry the block's one
texture, the exports' mean colour, bad parameters refused; the format round trip and the v57
fixture.
### See-through and cut-open pictures in the studio look (2026-10-30)

Layers 1 to 3 drew a see-through (`RenderParams::opacity` below 1) or cut-open (`RenderParams::section`)
picture in the plain look, so switching either on dropped the studio look. Both are now studio
pictures on both renderers. `craft3d_render::studio::render_studio_frame` is the definition; the
surfaces are those of the plain picture exactly (the same rasterisation rules,
`studio::rasterise_layers`), only their light is the studio's.

**Method.**

1. *The surfaces.* Per sample the front and, when see-through, the nearest surface strictly behind it,
   by the rules of the plain renderer (`render_analysed`): the same coverage and depth keys, the same
   rule for an edge or vertex the front lies on, what the section cuts away left out, the cap where
   the count of the surfaces left out is odd (counted over every triangle, so that every picture of
   a frame has the same cap), at the plane's own point under the sample's jittered position. The cap
   is the face `SECTION_FACE` with the plane's normal. `the_studio_picture_has_the_plain_pictures_surfaces`
   holds the ids and distances equal to the plain picture's.
2. *The cap is lit like a face*: its colour, no finish of its own (roughness 0.5, not metal), its
   occlusion and contact shadows from the buffers like any surface. `the_cap_is_lit_as_a_face_in_its_plane`:
   the cap of a box cut in two against the face of the half box in the same plane, same colour (and
   the same box around the mesh), within one level over the cap's inside.
3. *What is cut away casts no shadow.* The light's view is drawn of what the section leaves
   (`ShadowMap::draw_cut`: a texel of a triangle is left out where its distance from the plane,
   interpolated linearly in the orthographic light view, says `Cut::clips`). The cap is not drawn into
   it: a point of the kept half that the light reaches through the cap's plane meets a face of the kept
   half first (the segment from a point outside the bodies to a point inside one crosses its boundary
   within the kept half-space), so the cap would add nothing but self-shadowing.
   `what_is_cut_away_casts_no_shadow`: under the cut-away half of a ball lit from above the ground is
   lit (visibility above 0.97 where the whole ball gives under 0.5), under the kept half the shadow is
   unchanged bit for bit.
4. *A see-through front over the surface behind it, in light.* Both are shaded completely, each with
   its own occlusion and contact shadows (the surface behind reads its own buffer, `shade_all(&peeled)`),
   and blended before the screen-space passes: `light = α · front + (1 − α) · behind`, `α` the opacity;
   the front's reflection weight times `α` (the reflections replace the front's share of the specular
   light only); a highlighted face is not seen through, as in the plain picture. The surface behind
   where there is none is the background along the ray (the plain background as the light the tone
   curve shows as its colour, `FrameLight::seen`, as layer 3's glass shows it). One layer behind, as
   the plain picture keeps one. The studio look blends radiance, the plain look blends display bytes,
   so the same opacity looks different in the two looks (through a tone curve the behind's share is
   not `1 − α` of the bytes). `a_see_through_front_is_blended_over_what_is_behind_in_light`: two
   sheets, the linear curve, no shadows: every pixel of the front is
   `srgb(α · lin(front alone) + (1 − α) · lin(behind alone))` within a level.
5. *Shadows through a see-through body.* Every body is seen through at the same opacity, so a shadow
   lets `pass = 1 − α` of the key through: the key's visibility `v` becomes `pass + (1 − pass) · v`
   (`FrameLight::see`), on the surfaces (with the contact shadow) and on the ground. One layer is
   counted, as the shadow map holds one (a ray through two walls of a see-through body would keep
   `(1 − α)²`: owed, roadmap › Open); with opaque bodies `pass` is 0 and the light is bit for bit
   as before, pinned by `studio_bytes.rs` (the checksums of an opaque, whole picture in layer 1's look
   and the polished one, a transmissive face among them, as main fef3410c drew them). `a_see_through_ball_lets_its_share_of_the_key_through` holds the ground's visibility to
   that formula (to 10⁻⁹).
6. *Glass in a see-through picture.* A face that lets light through (layer 3) shows the surface behind
   it as a thin sheet would (`light + transmit · behind` before the blend): layer 3's picture behind
   the transmissive faces is not drawn, since the surface behind every front is drawn already. It still
   casts no shadow. `glass_in_a_see_through_picture_passes_the_surface_behind_then_blends`: a thin glass
   sheet over another sheet is `α · (front + t · behind) + (1 − α) · behind`, each term
   `FrameLight::shade_layered`'s, within a level; highlighted, the glass is not seen through.
7. Both kinds take the path through the screen-space buffers (as layer 3's transmission does), so the
   viewport draws them on its deferred path (one sample per pixel, frames accumulated). A see-through
   picture keeps a second surface per sample, so the CPU takes at most half `MAX_SAMPLES` for it
   (`studio::max_samples`; `view.render`'s frame cap follows it).
8. *Doubt.* Where a surface other than the front's triangle lies within `TRACE_DOUBT · ε_f32 · M` of
   the front's view distance (a coplanar face, a face touching another), the GPU may take it for the
   surface behind or not; `render_studio_doubt` marks those samples (`peel_doubt`), as layer 2 marks a
   traced ray.

**On the GPU** (`studio.wgsl`, `studio.rs`, `renderer.rs` › `encode_studio`, `StudioOnscreen::encode_peel`).
The section's uniforms are the scene's (`scene.wgsl` › `section`); every pass over the surfaces on the
buffers' path (`fs_geometry`, `fs_geometry_opaque`, `fs_studio_post`, `fs_studio_behind`, `fs_backs`,
`fs_studio_data` and the new ones) drops what `cut_drops` says: a fragment of the mesh whose distance
from the plane, interpolated from its corners' origin-relative distances (`VertexOut::side`, as the
CPU interpolates `Cut::side`; not `eye + rel` in `f32`, which loses it far from the origin), says
`Cut::clips` (without a section the plane is none and nothing is
dropped), or a fragment of the cap where the count is even or outside the cap's box. The count is a
pass of its own before the others (`fs_studio_parity` through the jittered vertex stage, into an
`R8Unorm` target with the plain cut's "one minus what was there" blend), read in group 2 of those
passes (`layout_parity`, its bind group made once with the targets; a texel of 0 for a whole
picture). The cap is the plain cut's triangle
(`cut::cap_vertices`), drawn after the mesh in every pass that draws the surfaces; the shadow pass
discards what is cut away by the distance interpolated from its corners. A see-through picture draws,
after the front's geometry, the surface behind it into layer 3's targets behind: its geometry
(`fs_geometry_peel`: a fragment whose view distance is strictly greater than the front's at its sample,
read from the front's geometry; both are the same vertex stage's numbers, so the front is never its
own surface behind), its occlusion, the background and its light (`fs_studio_peel`, the same test with
the front's geometry in `behind_map`); the front's `fs_studio_post` then blends over it (`see` in the
uniform block: the opacity, the pass, the flags). The GPU does not apply the CPU's rule for a sample
exactly on an edge the front lies on (the neighbouring triangle's distance there can differ in `f32`
and come out "behind"); such samples lie in the band of the changes of the face behind.

*Tolerance* (`crates/gpu/tests/studio_cut.rs`): layer 2's comparison with the plain look's band of a
cut-open, see-through picture (`common::band_of`: silhouettes, the section's outline, the changes of
the face behind). Measured (run 41142710329; readback and viewport on RADV, radeonsi and llvmpipe; a
housing over parts in perspective, its ball glass and the post's faces highlighted, and a plate,
globe, cone and a block standing on the plate in orthographic; seen through, cut open, both; layer 1
and polished): ids equal; at most 2 pixels over one level, 2 levels at most; at most 0.27 % of the
pixels in doubt, all where two faces in one plane lie behind a see-through front (the block's bottom
on the plate's top: 21 to 22 levels apart there, the other face's light), mean share 0.24 %. Bounds:
1/4096, 8 levels, 1/256 in doubt and in the mean share. A first run found the cap's light 7 to 36
levels off on every adapter: the cap's triangle carries no normals, so the GPU shaded it with the
view direction; `normal_of` now gives the cap the plane's normal. A second found the coplanar faces
behind the front decided otherwise in `f32` (90 pixels, 22 levels): `peel_doubt` marks a sample where
a fragment lies within the margin of the surface behind as well as of the front.

*Cost* (`see_through_and_cut_open_frames_on_a_thousand_faces`, run 41142710329: 1 002 faces, 190 096
triangles, 1020 × 777, the viewport until the GPU is done; a new camera every draw, so every draw is
frame 0). RADV (4 samples when opaque in layer 1, else the deferred path at one): layer 1 opaque 1.30
ms, see-through 2.52, cut open 1.81, both 2.60; polished 2.22, 3.32, 2.30, 3.06. radeonsi (GL, one
sample): layer 1 1.25, 2.65, 1.81, 2.62; polished 2.95, 3.97, 3.05, 3.86. Seen through costs about
1 to 1.4 ms more (the surface behind: its geometry, occlusion and light), cut open 0.1 to 0.6 ms (the
count and the cap); llvmpipe, for scale, 135 to 241 ms a frame either way.

**Looked at** (2026-10-30, `target/evidence/studio-cut/`, the CPU renderer, polished, 8 frames;
`crates/engine/tests/studio_cut_evidence.rs`, ignored, run on the Strix): `turbofan-cut.png` and
`turbofan-cut-studio.png` (the turbofan of the benchmark cut through its axis, the upper half away:
caps lit, the casing's shadow on the ground has the cut's outline), `turbofan-see-through.png`,
`gears-see-through.png` (a housing at opacity 0.4 over two gears: the gears lit and shadowed through
it, the housing's shadow on the ground lighter), `gears-cut.png`, `gears-cut-see-through.png`. A finding
that is the method's, not a fault: where a body lies inside another (gears modelled inside a solid
housing block) the count is even and the cap is left open there, as in the plain look.

The viewport draws the bodies' depth for the lines without what the section cuts away (the plain cut's
`depth_only` pipelines), and the plain see-through passes are not drawn under a studio picture; a
device without them draws studio see-through pictures all the same (refused only in the plain look). The
edges follow the lens in these pictures too (`Backend::edge_blur` no longer leaves them out).

### Parts moved by poses (2026-10-30)

An assembly's scene says per body which source mesh it is and the rigid motion that places it
(`SceneBody::placement`, [`motion.md`](motion.md) › *Costs*). The GPU renderer takes such a scene as parts
(`Renderer::upload_parts`, `Onscreen::upload_parts`): each part packed as a mesh of its own (vertices relative to
its own centre, its face ids from where its body's start in the scene), every pipeline that draws a mesh with a
second vertex buffer stepped per instance holding the part's pose (three rows of four `f32` at shader locations 8 to 10, after the vertex's optics and analysis value at 6 and 7: 11 attributes, within WebGL2's 16; the rotation and the
translation relative to the scene's origin, worked out in `f64`, `mesh::pose_bytes`), bound per chunk at the
part's offset (no first-instance, which WebGL2 lacks), and `placed` in `scene.wgsl` moving the position and both
normals before every vertex entry point. A mesh uploaded whole is one part at the identity, which moves nothing
(products by 1 and 0 are exact). A moment of the same parts is `set_poses`: 48 bytes a part and the box of the
placed triangles (`drawn_bounds`, what the depth range, the section and the studio light are framed on, as on the
CPU). Held by `crates/gpu/tests/parts.rs` against the CPU renderer drawing the moved mesh (oracle 7's tolerances),
near the origin and 1e7 away, after uploading and after moving.

### Headless GPU (2026-10-30)

`craft3d_engine::pictures` holds the seam. `PictureRenderer` has the CPU renderer's three entry points as
one call with a `Shading` (`Plain`, `Analysed(values)`, `Studio(look, env)`), plus `describe` (the
adapter, making the device the first time). A `Painter` takes the session's renderer out for one command
and gives it back: `renderer: "gpu"` fails at once when there is none or it cannot make a device;
`"auto"` draws on it and, on its first error, draws the rest of the command on the CPU and says why
(`rendererFallback`). The host's renderer and video encoder sit in the session in a `Hosted` slot that a
clone of the session does not copy (the device stays with the session the host gave it to), behind a
mutex so the session stays `Sync`. Thumbnails stay on the CPU: the same bytes on every machine.

`apps/craft3d-cli/src/gpu.rs` is the GPU side: `craft3d_gpu::Offscreen` made on first use (its error
kept, so it is not asked again), the scene's mesh uploaded once per key (a replay gives every state a key
unique in the process, so the frames of an orbit reuse one upload) and drawn with `render_uploaded` or
`render_uploaded_studio`. A panic inside wgpu is caught there, the device is let go of, and `auto`
continues on the CPU. A cut-open picture is drawn in the plain look by both renderers (as
`render_studio` does on the CPU). The CPU's limit of 2^26 samples over a still's frames bounds the CPU's
time, not memory, so it is the CPU's alone: on the GPU a still averages up to 256 frames at any size.

Review fixes (2026-10-30): under `auto` the painter asks the renderer at once whether it can draw
(`describe`, which makes the device), so a replay sizes its studio frames for the CPU (at most the CPU's
cap) when there is no adapter. A failure in the middle of a command (an error, a lost device, a panic,
which `Painter::draw` catches) hands the rest to the CPU, and a studio still is then averaged over as many
frames as the CPU can at that size (`cpuAccumulate` in the result) instead of the command failing; the
result's `renderer` is `mixed` with the count of each. A renderer that panicked is let go of, never put
back. The painter puts the host's renderer back into the session when it is dropped, so a command that
fails, or one the host's last-resort guard catches, does not lose it. The CLI's renderer makes its
device again once after it was lost (`GpuError::Device`), so a long-lived `serve` recovers. A picture of
face ids (`mode: "ids"`) is always the CPU's, since agents read its pixels. ffmpeg runs with a time
limit (a minute plus a second a frame), its paths prefixed `file:` (a name like `-x.mp4` or `tcp://…`
is a file) and a `%` of the frames' directory written `%%` in the input pattern. The camera spline is
Catmull-Rom in time (tangents over the keys' times), so a short stretch between uneven keys is not
overshot by the long one beside it. Held by `crates/engine/tests/pictures.rs` (fake renderers that
cannot draw, fail half-way, or panic) and `animation.rs`'s unit tests.

A replay on the GPU draws its frames one at a time on the command's thread and encodes the PNGs a batch
at a time on every core; on the CPU each frame is drawn and written on its own core as before.

**Parity.** `apps/craft3d-cli/src/gpu/tests.rs` gives a session a renderer that draws each picture on
the GPU and on the CPU from the same mesh, camera and parameters, and holds the two to the GPU crate's
own rules: the plain look to `crates/gpu/tests/common` (oracle 7), the studio look to the layer-2 rules of
`crates/gpu/tests/polish.rs` (outside the band and the pixels in doubt, at most 1/512 over one level,
none over 8). It runs `view.render` in the plain and studio looks (default layer 2, an environment
with 4 frames, without reflections) and `view.replay` of the history (orbiting, studio) and of an
animation. The rendering is the one the GPU crate's tests already hold; what this adds is that the
commands give the GPU the same inputs as the CPU.

### Animations (2026-10-30)

`view.replay` has three kinds of frames: the history (as before), the motion (`motion`), and an
animation (`animate`). An animation has `frames` states (or `duration` × `fps`; 2 s by default); state
`k` is at `s = k / (frames − 1)`. Each variable track gives its variable a value at `s`: from `from` to
`to`, or through keys `[{t, value}]` with `t` rising from 0 to 1, between two keys moved by its easing
(`linear`, `in` u², `out` 1 − (1 − u)², `inOut` 3u² − 2u³; per stretch, so the eased ones come to rest at every key), the ends held outside the keys. The value is
written with the track's `unit` and must keep the variable's dimension (checked on the first state, so
`20` for a length is refused with the advice to give `"unit": "mm"`). Each state is the document with
the values set, regenerated from a copy of the document's regeneration cache, so only what the variables
touch is rebuilt; it is a function of its values alone (sketches are solved from the document's own
solution), so the two passes over the states (one for the box the camera is framed on, one to draw a
chunk at a time, as for the motion) give the same states, and what is held at once is one chunk. A track
of the document's timeline `t` goes through `motion::Sweep`, so a linkage stays on its branch and an
assembly's parts are placed by its solve; an assembly takes that track only (its other variables are
its drivers' business), and with no track it is shown at its own `t`.

**The camera path.** `camera.keys` give `eye`, `target`, `up` and `fov` at places `t`; a field a key
leaves out is the framed camera's. Eye and target follow a Catmull-Rom spline in time through the keys
(it passes through each key; a key's tangent is the chord between its neighbours over the time between
them, one-sided at the ends), the up and the field of view are
interpolated linearly (the up normalised). `orbit` turns the camera about the vertical through its
target by that many degrees over the replay and `dolly` scales its distance to the target (and an
orthographic camera's height) from 1 to the factor; `easing` paces the keys' segments and both helpers.
The path runs over every frame, the hold included. With an orbit the base camera is framed so that the
model stays in the picture all round (`orbit_camera`, as before). A frame whose camera has no direction
(eye on target, or looking along its up) is an error that names the frame. Without a path that moves, a
state held for several frames is drawn once and written several times.

**Pacing and settling.** `fps` and `duration` (or `frames`) pace an animation; `framesPerStep` and
`hold` the history. A studio frame averages `look.accumulate` frames, 16 when not given
(`look::VIEWPORT_ACCUMULATE`, the window's settled picture), on the CPU at most what its sample limit
allows at the size. The window's own settling count is the same field: `ui.set {"look": {"accumulate": n}}`
sets how many frames the viewport averages, which `ui.screenshot {window: true}` waits for.

**Video.** The engine starts no program. `view.replay {video}` (`.mp4`, `.webm`, `.gif`) asks the
session's `VideoEncoder` (the host's, as the renderer), which `craft3d-cli` implements with an external
`ffmpeg` (`PATH` or `CRAFT3D_FFMPEG`; H.264 or VP9 when that ffmpeg has them, the captions as a subtitle
track), the way FreeCAD is used as an optional tool: nothing is linked. It is asked before a frame is
drawn, and a missing ffmpeg is an error that names the PNG sequence (`dir` alone) as the alternative.

**Measured** (Check run 41059222246, the Strix's Radeon 8060S over Vulkan/RADV, 1920 × 1080, the
benchmark's engine block, 69 048 triangles, `view.render` to a PNG, the first picture not counted):
plain 0.045 s a frame on the GPU against 1.05 s on the CPU (22 fps against 0.96); studio, one frame,
0.050 s against 7.25 s (20 fps against 0.14); studio with 16 frames averaged 0.20 s (5 fps; the CPU
refuses 16 at this size). A replay of the history orbiting, PNG files written: 199 frames in 7.7 s plain
(26 fps) and 36.8 s in the studio look with 16 frames each (5.4 fps).


### The interface's font (2026-10-31)

The developer chose Inter (SIL OFL 1.1) for the interface, in place of egui's Ubuntu-Light. Like the built-in fonts
(`docs/design/text.md` › *Built-in fonts*) it is never committed and never in the program or the web module:
`craft3d_font::UI_FONTS` pins `InterVariable.ttf` of Inter 4.1 and its licence (`Inter-OFL.txt`) by URL and SHA-256,
`cargo xtask fonts` fetches it with the others, and `cargo xtask package` copies it into every package's `fonts`
folder and the web site's `fonts/`. A text in a model cannot name it (`craft3d_font::builtin` does not know it;
`shipped_font` does). The shell asks for it on its first frame (`crate::fonts::UiFonts`, through
`craft3d_engine::fonts::load`): the desktop reads it where it finds the built-in fonts and checks the sum; the browser
lists it for the page, which fetches `fonts/InterVariable.ttf` as it fetches a text's font and registers it, and the
shell swaps it in on the frame it arrives (a failure is logged and not said in the status bar). One variable file
gives two faces through egui's variation coordinates: weight 400 for running text and weight 500 as the family
`medium`, which `TextStyle::Heading` uses. Monospace text (`TextStyle::Monospace`, the command lines on the start
screen) is JetBrains Mono, the built-in `mono`, when it is here: read at start on the desktop, used in the browser
once a text fetched it. egui's own fonts (`default_fonts` stays on) are what the interface has until then, on a
checkout without `cargo xtask fonts`, and behind Inter in every family: Inter lacks ✔ ⏸ ⌖ (the tree's marks and the
zoom tooltip), which egui's emoji fonts draw; ☰ and the fold arrows are painted, not glyphs. The web module does
not grow (the file is fetched, 880 KB, 460 KB gzip).

Inter is wider than Ubuntu-Light, so two places that assumed the old widths now measure: the start screen's card
titles are one row cut with an ellipsis at the card's width (`shell/start.rs`, the whole title in the tooltip), and
the studio popover's *Ground* slider takes the room its value text leaves (`shell/look_panel.rs`; superseded 2026-10-31, below), which on a phone
had pushed `reflection 0.00` past the popover's edge. Evidence: `tests/ui_audit_evidence.rs` with `--ignored`
before and after, every picture looked at (start screen, part, assembly, studio and analysis popovers, section,
drawer, Parameters, error; 1600 × 1000, 1024 × 700, 390 × 844, 820 × 1180).

- 2026-10-31 (r1-studio-panel): the studio popover's ground reflection is a row of its own (*Reflection*), and every
  slider in a popover has one width (`Tokens::popover_slider_width`), so no slider takes "the room its value text
  leaves" any more. The popovers are drawn on the plates' surface (`Tokens::plate_visuals`) and hang from the camera
  plate (`docs/development.md` › the controls over the viewport).
### Near-plane clipping (2026-10-31)

The CPU renderer left out every triangle with a corner at or behind the eye plane, so a camera close to a surface or inside the model drew what lay behind those triangles where the GPU, which clips, drew the triangles themselves (found with the surface analysis seen close on a sphere: one large triangle near the pole skipped, the sphere's far side showing through). The CPU renderer is the oracle, so it now clips (`crates/render/src/clip.rs`).

- *The near plane* is one function for both renderers: `craft3d_render::depth_range` of `craft3d_render::depth_bounds`, the box of the drawn triangles (`section_bounds`: those with every corner finite; the GPU's `GpuMesh::bounds` is the same box), grown by the cap's margin when a section draws its cap, and the GPU's `depth_range` and `cut::cap_bounds` now call them. Near is the box's nearest corner less a margin, at least a millionth of the farthest. A vertex no triangle uses, even an infinite one, does not move it (the first version took `mesh.bbox()`, every position: one infinite vertex made the box infinite and nothing was clipped, the defect again). A camera outside the box by more than the margin clips nothing, and the mesh is used as given, borrowed, bit for bit; one within the margin or inside clips at the same distance on both renderers.
- *Clipping* is Sutherland–Hodgman against `dist ≥ near` per triangle (`dist` the distance along the view direction, the clip-space `w`): a corner that is not a number leaves the triangle as it was (the projection then leaves it out), none in front drops it, one or two give a triangle or a quadrilateral fanned from its first corner in the triangle's winding.
- *A new corner* is the point `t = (near − d₀) / (d₁ − d₀)` of an edge, its ends taken in the order of their positions' bits (then indices), so the triangles of a shared edge get the very same point, also where two faces meet along a sharp edge with vertices of their own walked the other way (a tessellated body's case; ordered by index, `P + (Q − P)t` and `Q + (P − Q)t'` differed in their last bits: pinholes, and the see-through rule that knows a shared edge by its points' bits lost it). One vertex per edge of the mesh, kept in a `BTreeMap` (deterministic). Every attribute is the same interpolation of the edge's ends, exact since along a straight edge in space the perspective-correct interpolation is the linear one: the normal, the point (so the section's distance from its plane too) and the curvature map's value. Clipped triangles keep their face id and remember the mesh's triangle they come from (`ClippedMesh::origin`), which the studio look's peeling compares.
- *Where*: the plain, analysed and see-through pictures (`render_analysed`), the studio look's layers and its peel doubt (with the whole scene's box: `Raster::depth`), picking (`pick`, `pick_cut`, `cut_faces`, `pick_all`) and `Neighbourhood`, which clip each triangle as they look at it (`clip_corners`, the same points, no copy of the mesh), and the curvature combs of `view.render` (`analysis::draw_lines`), whose segments are cut at a millionth of their farther end's distance instead of being left out whole. The light's view of the studio shadows is orthographic. The analysis and the section are placed by the mesh as given. The cap of a section is clipped at the same near plane (`View::cap_at` refuses a point nearer than it, as the GPU clips its cap's triangle), so an eye within a millionth of the far distance of the cut plane draws no cap pixel the GPU leaves out. `View::project` takes any distance in front of the eye (it cut at 10⁻¹² absolute, which at a small enough scale is in front of the near plane, a millionth of the farthest distance); results that do not fit a float are still refused.
- *Picking from inside*, 4.9 million triangles (a sphere at 2·10⁻⁶), release, per pick: outside 112 ms, inside 130 ms before (a copy of the clipped mesh per pick) and 131 ms after (clipped as looked at): the copy was not what it cost; a pick looks at every triangle either way.
- *The GPU's origin* (positions go to it relative to a centre) is now the centre of the triangles' corners, not of every position: a far vertex no triangle used cost the drawn ones their `f32` precision (found by the parity scene with such a vertex: every pixel's shading off).
- *Tests*: `render/tests/near_clip.rs`: a floor standing 1 above it looking level, at 10⁻³, 1, 10³ and 10⁶ model units: every row below the horizon is the floor, every row above is empty, the depth at each pixel the closed form `s / (r / (h/2) · tan(fov/2))` to 10⁻⁹, and a pick there on the floor at the closed-form distance (the picture was empty before); inside a closed block at 10⁻³ and 1, near a corner and against a wall, every pixel a face; a comb through the eye plane drawn in front; triangles wholly behind the eye, at the eye, with a NaN or a huge corner, with values per vertex: no panic; the same picture twice. `clip.rs`: whole, gone, triangle and quadrilateral; a sharp edge with vertices of its own cut at one point, bit for bit; a camera outside the box borrows the mesh; the near plane ignores vertices no triangle uses and grows by the cap's margin. `gpu/tests/parity.rs` › `a_camera_close_to_or_inside_the_model_matches_the_cpu_renderer` (skimming a sphere just above its pole; inside it; 0.025 above its pole looking down, the case that found it, also with an infinite and a far vertex no triangle uses; a sphere of radius 10⁻³ from inside; inside a block near its corner) and `gpu/tests/section.rs` › `a_camera_inside_a_cut_open_body_matches_the_cpu_renderer` (inside a capped section, and just outside the box within the cap's margin): on every adapter within the usual tolerances; without the clipping 1513 ids differ in the block and 5918 distances over the pole. The tolerances needed nothing.


### Grainy highlights on a class-A tank (2026-10-31)

The coordinator saw a grainy, dithered pattern inside the clear coat's highlights on the class-A tank in the studio environment. Isolated: the pattern is the same bit for bit with screen-space reflections, ambient occlusion, the specular shadow, all shadows off, and with one frame or eight (`speck/isolate.png` in the run's evidence), and a car-paint sphere shows it too; it is the environment picture itself. `assets/environments/studio.hdr` (1024 × 512) has two octagonal softboxes with egg-crate grids in front of them, and a clear coat of roughness 0.04 on a gently curved panel is a mirror that magnifies them: the grid is reflected. Not a renderer defect. The tank's pictures use the workshop environment (windows, no grid) with the ACES curve, as the thumbnails do since METHOD v4, and the key light raised (`look.light` [200, 78]) so its shadow falls under the body and it stands on the ground. Owed, if wanted: a studio picture with plain softboxes, or the studio environment turned so the grids are not what a class-A panel reflects.
