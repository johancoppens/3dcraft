//! The `ui.*` methods of the control channel (docs/control-protocol.md):
//! what an agent uses to look at the window it is driving, frame the camera
//! and caption a step. They read and change view state only; the document
//! is changed with commands.
//!
//! This is the body of the methods; the transport (JSON lines over TCP, the
//! token) belongs to the application.

use crate::shell::{FlyTo, Shell};
use crate::viewport::{self, MAX_SIDE, MIN_SIDE, Quality};
use craft3d_engine::Session;
use serde_json::{Map, Value, json};

/// A method, as the protocol's `methods` lists it.
pub struct UiMethod {
    pub name: &'static str,
    pub params: &'static str,
    pub summary: &'static str,
}

/// Every `ui.*` method, in a fixed order.
pub const UI_METHODS: &[UiMethod] = &[
    UiMethod {
        name: "ui.get",
        params: "{}",
        summary: "Returns the window as it is, one key each: state, viewport, document, scene, features, assertions, renderer, render, section, lastCommand, report, recovery, problem, work, workShown, traces, tabs, closing, quitting, selection, parameters, recent, startScreen, agents, timeline, export, lens and appearance. state: the UI state, the fields ui.set sets (camera, panels, selectedFeature, notice, quality, highlight, highlightEdge, highlightVertex, transparency, section, show, dialogs, timeline, parameters, export, look, analysis, appearance) and picked, the last click in the viewport. viewport: its width and height in points and pixelsPerPoint. document: the active document (index, open: how many are, path, dirty, units {length, angle}), null without one. scene: what the viewport's scene holds (revision, bodies, sketches, triangles, the chordal tolerance it was meshed with, the angle criterion in degrees (null for none), coarsened: whether the display quality was coarsened to fit its triangle budget, refining: true while the viewport shows the quick first picture and the display-quality mesh is still being built, bbox, problems; notices: what it says without the picture being wrong, a face colour whose face is gone), null without a scene. features: the feature list's rows (id, name, kind, state, status, error, causedBy, selected, hidden, madeSelection, zoomable, when, passes: what it passes on and leaves out, as its tooltip says, else null). assertions: the assertions' rows (name, measure, status, measured, expected, message). renderer: which renderer the viewport uses (backend gpu or cpu, the adapter, fallback: why it fell back, direct: whether the GPU draws on the window's own device, msaa: with how many samples). render: how the viewport was last rendered (renderer, width, height, ssaa, msaa, triangles, milliseconds, sharp, count), null before the first. lastCommand: the last command the window ran itself ({label, ok, text}, as the status bar says it), else null. report: the plain-text bug report Copy details copies (session.report's, with the window's renderer). recovery: autosave and recovery as recovery.list gives them but without looking at the disk again (the documents a crashed 3DCraft left behind, when a document was last autosaved, the autosave problem if there is one). problem: why the viewport shows no picture or not the right one, else the scene's first notice (a face colour whose face is gone, which colours nothing; scene.notices lists them all), else null. work: what the engine thread is doing while a command runs (label, phase, feature, name, step, steps, seconds), else null. workShown: what the work indicator in the status bar says while it shows (only for work that has run 0.35 s, and for 0.3 s after), else null. traces: the traces of the active document (view.trace) as [{name, pending: being worked out in the background, error: why one asked for in the background could not be made, length, closed}]. recent: the recent files of the File menu that lie beneath the session's roots. startScreen: true while the start screen shows (no document open). agents: whether AI agents may drive the window and who does (available: the desktop app, false in the browser; allowed: the switch Allow AI agents is on and the window listens; forced: by --control; port; connected: how many clients; clients: their names; workFolder: the folder agents may read and write files in; error: why the switch could not be turned on), read-only: the person turns it on and off. agentLog: what agents did to the active document, as agent.log gives it with limit 100 (entries newest first: client, timeMs, command, label, ok, message, created, changed, intent; features: who made and last changed each). section: where the section plane of state.section lies while it is on (point, normal, into the half that is cut away, and range, the offsets between which the plane passes through the bodies), or lost, why the section was switched off when its plane no longer exists (a face or a datum plane that is gone); null when it is off. selection: what is selected, as select.get gives it (items, count, summary, dropped). tabs: the open documents as the row of tabs above the viewport shows them, in the order of file.list (document: the number file.activate takes, name: the file's name without its extension or Untitled n, path, dirty: unsaved changes, marked on the tab, active, asksToClose: closing it from its tab asks Save, Don't Save or Cancel first, as file.close refuses it without discard, jigInUse: a published Jig opened to be used, which asks only when its parameters changed, whether to save them as a model); closing: that question while the window asks it ({document, name, path, quitting: true when the window is closing, jig: true for a Jig in use, asked as Save as model…, Close or Cancel}), else null; quitting: true while the window is closing and asks about the documents with unsaved changes one after the other. A click on a tab runs file.activate, its close button file.close. timeline: the history timeline (steps; shown, the step the view shows; scrub, where view.scrub put it, or null; rollback, the real marker's step; problem) and its player (playing, speed, loop and orbit, as state.timeline has them; caption, the step's caption over the viewport while it is scrubbed or playing, else null; playQuality as set; playingLight: true while the viewport draws lighter because a motion plays, never while the window is captured or recorded; heavy: what the motion's recent moments cost and the bake hint {msPerMoment: the median time the view.motion of a moment took in the window, over the last 15 moments played (null before 8; not the frame around it), moments: how many are weighed, provisional: the share of them shown provisionally while their configurations were built in the background, heavy: the judgement (heavy from a median of 100 ms, light again only below 70 ms), suggestBake: the player shows \"This motion is heavy — bake first?\" (heavy, not baked, not dismissed for this document in this session), baking: {progress: 0 to 1, auto: started by itself, for a motion that is not heavy but shown provisionally} while the bake runs (motion.bake at 120 frames, in the background builder or in pieces between frames), else null, dismissed}). parameters: the Parameters tab beside the feature list (the values are param.list's): jig (the document has public parameters), inUse (a published Jig opened from a .3dcraftjig file to be used: only param.set changes it; the tab offers Open for editing, jig.edit), tab (parameters or features: the one shown; a Jig opens on parameters; state.parameters.open sets it), count, busy (a param.set of the form waits or runs), live ({name, value, dragging}: the field being dragged or stepped, before its value is committed), note ({name, text}: what the form said under a field it clamped to its range or snapped to its step, such as \"clamped to 120 mm\", for a few seconds) and trouble ({name, message, broke}: the last commit was refused (broke false), or broke the model (broke true: a feature fails or an assertion is broken that did not in the last good state; a banner over the viewport says so with Back to last good)). export: the open export dialog ({whats: [{what, label, formats}]: each choice of what goes into the file with the formats that hold it, none for a selection no format holds; problem: why not; namesFile: it asks the file's name itself (the browser without file dialogs); file: the name with the format's extension}), null while it is closed (state.export is what it chose last). lens: where the studio look's lens focuses as the viewport draws it ({focus: look.dof.focus as given; point: the point it focuses on (the plane square to the view through it), null for a distance; distance: how far along the view from the eye; fallback: why it focuses on the model's centre instead (nothing selected, a selection none of which is shown, what was selected gone, no point clicked yet), else null; noBlur: why it blurs nothing (an orthographic view, a focus behind the camera), else null}), null without a lens, outside the studio look or while an analysis is shown. appearance: whether the panels are light or dark now (state.appearance resolved: system follows the desktop's or the browser's preference, and is light when there is none).",
    },
    UiMethod {
        name: "ui.set",
        params: r#"{"view": "iso|front|back|right|left|top|bottom", "camera": {"target": [x, y, z], "distance": number, "yaw": degrees, "pitch": degrees, "projection": "perspective|orthographic", "fovY": degrees, "autoFit": boolean}, "panels": {"tree": boolean, "status": boolean, "recovery": boolean, "treeWidth": points = 260, "drawer": boolean = false (the side panel's drawer in the narrow layout), "lookOptions": boolean = true (the studio look's popover open while the look is on; switching the look on opens it), "analysisOptions": boolean = true (the analysis' popover open while one is on; switching one on opens it)}, "selectedFeature": id or null (selects that feature, as ui.select does; null takes the features out of the selection), "notice": string, "quality": {"chordal": fraction of the model's size = 2e-4, "maxTriangles": integer = 1000000, "angle": degrees = 12}, "highlight": {"feature": id or name, "face": face name} or null, "highlightEdge": {"feature": id or name, "edge": edge name, "between": [face name, face name]} or null, "highlightVertex": {"feature": id or name, "vertex": vertex name, "corner": [face name, ...]} or null, "transparency": number from 0 to 0.9 = 0, "section": {"on": boolean, "plane": "xy|yz|zx" or a datum plane or a face name, "feature": id or name, "point": [x, y, z], "normal": [x, y, z], "offset": number, "flip": boolean}, "show": {"datums": boolean = true, "sketches": boolean = true}, "dialogs": {"about": boolean = false, "shortcuts": boolean = false}, "timeline": {"playing": boolean, "speed": 0.5|1|2 = 1, "loop": boolean, "orbit": boolean, "motion": boolean, "bounce": boolean, "collisions": boolean, "playQuality": "auto|full" = "auto"}, "parameters": {"open": boolean, "collapsed": [group, ...]}, "export": {"open": boolean, "what": "model|selection|everyPart" or {"set": name} or {"sheetMetal": part} or {"fold": fold}, "format": "step|stl|3mf|obj|dxf|svg", "quality": "coarse|normal|fine", "drawing": {"view": "current|iso|front|back|left|right|top|bottom", "perspective": boolean, "dashed": boolean}, "width": string, "gap": string, "name": string}, "look": {"mode": "plain|studio" = "plain", "environment": "studio|outdoor|workshop" = "studio", "exposure": stops from -10 to 10 = 0, "rotation": degrees = 0, "ground": boolean = true, "ao": boolean = true, "shadows": boolean = true, "tonemap": "agx|aces|linear" = "agx", "light": [azimuth, elevation] or null = null, "reflections": boolean = true, "contactShadows": boolean = true, "specularShadow": boolean = true, "groundReflection": 0 to 1 = 0, "bloom": 0 to 1 = 0.04, "dof": {"fstop": positive f-number, clamped to 0.5 to 64 = 2.8, "focus": distance | [x, y, z] | picked (the last click) | "selection" (the centre of what is selected) | "centre" = "centre"} or null = null, "accumulate": frames from 1 to 256 = 16}, "analysis": {"mode": "zebra|highlight|curvature|draft|none" = "none", "stripes": number from 1 to 512 = 16, "direction": degrees = 0, "width": dark share 0 to 1 = 0.5, "offset": periods = 0, "pull": [x, y, z] = [0, 0, 1], "minDraft": degrees = 1, "curvature": "gaussian|mean|max" = "gaussian", "range": [low, high] or null = from the values, "legend": boolean = true, "combs": {"edges": boolean or [edge name, ...], "iso": [{"face": face name, "feature": id or name, "u" or "v": 0 to 1}], "scale": number or null, "density": 2 to 400 = 40} or null} or null (off), "appearance": "system|light|dark" = "system"}"#,
        summary: "Changes the fields given and leaves the rest. An unknown field or a wrong type is an error, and then nothing has changed. Numbers out of range are clamped, except in analysis, which refuses them (as view.render does). quality is how finely the viewport's mesh follows the model: a chordal tolerance of chordal times the model's size and an angle criterion of angle degrees (no chord of an edge turns its tangent, no side of a triangle the surface's normal, by more than that unless it is shorter than the tolerance, so small holes stay round; 0 for none), coarsened until the scene has at most maxTriangles triangles but never beyond the quick first picture's tolerance (1e-3 of the diagonal of the model's box), which the window shows while the mesh is refined. highlight is the face shown highlighted, highlightEdge the edge and highlightVertex the vertex (ui.highlight sets them too, and says whether they exist); transparency is how see-through the bodies are drawn (0: opaque; up to 0.9): the front surface is blended over the one behind it, so that inner faces, holes and cavities show, and the hidden parts of edges are drawn dimmed; ui.screenshot shows it too, and a pick still gives the front-most face. section cuts the viewport's picture open with a plane, to look inside a shell, a pocket or a cavity; it is a view, the document does not change. The plane is plane (xy, yz or zx, a datum plane by its feature's id or name, or a flat face by its persistent name, with feature to say which body) or point and normal, moved by offset along its normal; what lies on the side the normal points to is not drawn (a face's normal points out of its body, so a negative offset cuts into it) and flip keeps the other half. Where the plane passes through a body the cut is closed with a flat cap in a colour of its own, edges end at the plane and the outline of the cut is drawn as edges. Setting plane replaces point and normal and the other way round; on with no plane at all is xy. A plane that means nothing (an unknown name, a face that is not flat, a normal without direction) is an error and changes nothing; ui.get says under section where the plane lies and between which offsets it passes through the model. ui.screenshot and ui.pick follow the section: a pick gives what is seen, never a face that is cut away, and on the cap kind is section. show says what the viewport draws besides the bodies and their edges: datums false hides the datum planes, axes and points, sketches false the sketches (their lines and points); the toggles over the viewport set the same fields, and ui.screenshot follows them. timeline is the history player under the feature list: playing true plays the history step by step for the person watching (from the first step when the view is at the end), each step shown once its model is built and then for at least 0.8 s divided by speed (0.5, 1 or 2); loop starts again after the last step, orbit turns the camera slowly meanwhile; it stops by itself at the end, on an edit of the document or another tab; motion true plays the motion over the timeline t instead (bounce back and forth, collisions drawn red); playQuality auto (the default) draws the viewport lighter while a motion plays (the studio look in one frame, without its accumulated frames, depth of field, ambient occlusion, screen-space reflections and contact shadows; the full look again, settling, once it stops), full draws it as when it stands still; pictures of the window (ui.screenshot) and recordings (ui.record, in real time too) are drawn at the full quality either way; playing true without a document, or without features when the motion cannot play (an assembly plays its motion), is refused. dialogs opens or closes the windows of the Help menu: about (About 3DCraft) and shortcuts (the keyboard shortcuts, also F1). parameters: open true shows the Parameters tab of the side panel instead of the feature list, collapsed the groups of parameters folded shut. export is the export dialog of File, Export…: open true opens it as the menu does (what the document offers is read then: the selection, its named sets, its sheet-metal parts and folds), false closes it as Cancel does; what and format choose as its buttons do and are refused when the document does not offer them or the format cannot hold what is chosen (a sheet or a pattern goes only to dxf or svg), and the name follows the format's extension; quality, drawing, width and gap are its options. Set while it is closed, what, width, gap and name are what the next opening chooses (what when the document offers it); what the dialog chose is kept for the next export only once a file was written. Exporting (its Export… button, then the system's save dialog) is the person's: no method presses it. look is how the bodies are lit, view state only (the document does not change): mode plain is the shading of every picture so far; studio lights them by an environment (studio, outdoor or workshop, bundled), image-based (reflections and diffuse light from it, metallic and roughness shaded by a physically based model), with soft shadows from its brightest direction (or from light, azimuth and elevation in degrees), ambient occlusion and a ground under the bodies that catches their shadow, tone mapped (agx, aces, or linear) at exposure stops; rotation turns the environment about z. The Studio toggle of the display bar switches mode; view.render and craft3d-cli render take the same look. Its second layer: reflections of the model in its glossy faces (roughness under 0.4) and in the ground (groundReflection, 0 to 1), the environment where a reflected ray finds nothing on the picture; contactShadows, short shadows where parts meet; specularShadow, the key light's shadow on the reflections too; bloom, the share of the light spread around bright parts; dof, the depth of field of a thin lens with fstop (a smaller f-number blurs more; the popover's slider stops at the full stops f/1.4 to f/22) focused at focus (a distance along the view, a point [x, y, z], picked: the point of the last click, which a later click moves; selection: the centre of the box of what is selected, in the viewport or the feature list, which follows the selection; centre: the model's centre; picked without a click and selection with nothing shown selected focus on the centre, and ui.get's lens says so; while the lens is on the selected faces keep their own colour, so the part in focus shows its sharpness, and the selection is drawn as its outline); accumulate, the frames the viewport averages while the camera stands still, each with its samples moved within the pixel and the lens turned, so the picture settles to a smooth still (ui.screenshot renders one frame unless accumulate is given; with window: true it waits until the viewport has averaged its frames, the environment is prepared and no popover is still fading in, within its 8 seconds: accumulate is that count, 16 unless set, so ui.set {look: {accumulate: 4}} settles sooner; view.replay averages the same 16 per studio frame unless its look says otherwise). A see-through or cut-open picture is lit the same way (the surface behind a see-through front lit as the front, the cap of a section as a face of its colour); in the browser on WebGL2 the studio look has no shadows, no occlusion, no reflections, no contact shadows and no depth of field. ui.screenshot follows it. appearance is the panels' colours: system (the default: the desktop's light or dark setting, in the browser prefers-color-scheme), light or dark; the viewport, its pictures and ui.screenshot stay the same in both, and the chrome, menus and popovers stay dark. picked, the last click, cannot be set (ui.pick with select makes one).",
    },
    UiMethod {
        name: "ui.pick",
        params: r#"{"x": number, "y": number, "width": integer = the viewport's, "height": integer = the viewport's, "radius": number = the click's (4 points), "select": boolean = false, "all": boolean = false, "shift": boolean = false, "alt": boolean = false}"#,
        summary: "Says what the viewport shows at a position, as the command view.pick does, for the viewport's mesh and camera and cut open by the viewport's section when that is on (then kind is section where the cap of the section is in front, with section: {feature, body, point, depth}, the body that is cut there and the point of the plane): kind (vertex when a vertex is near, otherwise edge when an edge is near, otherwise face), the face under the position (the body as feature and name, the face's persistent name as document.inspect lists it under faceNames, with faceIndex and aliases, and the point in space) and, when its nearest point in sight is within radius pixels, edge: {feature, body, name, index, faces, point, depth, distance}, the edge as document.edges lists it (name is its own persistent name or null, faces the names of the faces it lies between), or vertex, with the same fields, when a vertex in sight is within radius pixels (faces: the faces that meet in it; index: its index under vertices of document.edges). x and y are in pixels from the top-left corner of the viewport (the picture ui.screenshot makes; width and height when that picture was made at another size), the centre of pixel (i, j) at (i + 0.5, j + 0.5). radius is in pixels of that picture, 0 to 64, 0 for faces only; without it, it is what a click uses. hit is false when nothing is there, with outside true when the position is not in the viewport. With select it counts as a click of the mouse: state.picked becomes this pick and what it points at is highlighted, the vertex (state.highlightVertex), or else the edge (state.highlightEdge), or else the face (state.highlight); a pick on nothing clears them. A click in the viewport does the same, so state.picked of ui.get is what the developer pointed at last. A click also selects (ui.select, select.get): what it points at (the vertex, else the edge, else the face) replaces the selection, nothing clears it; with shift (Shift held) it is toggled instead; a click again on the same spot (within 3 pixels, the camera and the model unchanged), or with alt (Alt held), selects the next face behind the last one along the ray, going round to the front after the last, and highlights it; the result then has selection, as select.get gives it. With all, hits lists every face along the ray front to back, as view.pick gives it (the faces a right click in the viewport lists to choose from).",
    },
    UiMethod {
        name: "ui.highlight",
        params: r#"{"face": face name or null, "edge": edge name, "between": [face name, face name], "vertex": vertex name, "corner": [face name, ...], "feature": id or name = the body that has it}"#,
        summary: "Shows a face, an edge or a vertex highlighted in the viewport; one of face, edge, between, vertex and corner. face is a face's persistent name from document.inspect (faceNames, or an alias). edge is an edge's own persistent name (name in document.edges); between names an edge by the two faces it lies between, as document.edges takes it (the same name twice for a seam), and means every edge between them. vertex is a vertex's own persistent name (name under vertices in document.edges); corner names a vertex by faces that meet in it (one to 16 names) and means every vertex in which all of them meet. feature says which body when several have one of that name. face: null removes every highlight. Returns the highlight as it was set (highlight: {feature, face} for a face, highlightEdge: {feature, edge, between} for an edge, highlightVertex: {feature, vertex, corner} for a vertex; the feature by id), the name of the body and how many faces carry the name (faces; a boolean can cut a face into pieces that keep its name) or how many edges or vertices are meant (edges, vertices). A name no body has is an error and changes nothing.",
    },
    UiMethod {
        name: "ui.select",
        params: r#"{"items": [{"kind": "body|face|edge|vertex|feature|sketch|datum", "name": string, "feature": id or name}], "names": [string], "mode": "replace|add|remove|toggle|clear" = "replace"}"#,
        summary: "Sets the selection the window shows, as the command select.set does (the same selection: the window's clicks change it too), and returns it as select.get does. Selected bodies and faces light up in the selection colour, their outlines and selected edges and vertices are drawn over the picture (dimmed where something is in front), the status bar counts them (\"3 bodies, 2 faces selected\") and the feature list marks the first selected feature. ui.get gives it under selection. select.query with then selects by description.",
    },
    UiMethod {
        name: "ui.fit",
        params: r#"{"view": "iso|front|back|right|left|top|bottom", "feature": id or name, "selection": boolean = false}"#,
        summary: "Frames everything (bodies and sketches), from the named view when one is given, and keeps following the model as it grows. With feature it frames instead what that feature made: the faces whose persistent names say it made them, wherever they are now (a hole's wall in the plate it was cut into), else its body, its sketch or its datum; with selection: true, what is selected (ui.select, select.get); either from the view that sees it best (the current one or one of the eight iso views, not hidden behind the rest of the model) and with at least a fifth of the diagonal of its body around it. Then the camera stays where it is put as the model grows (autoFit false), a hidden sketch or datum framed is shown again, and a section that cuts away all of it is switched off. This is what a double click on a row of the feature list does in the window, and F with something selected, there as a flight of 0.3 s. An error (nothing of the feature is shown, nothing is selected) changes nothing.",
    },
    UiMethod {
        name: "ui.screenshot",
        params: r#"{"path": string, "width": integer = the viewport's, "height": integer = the viewport's, "scale": number = the window's pixels per point, "window": boolean = false}"#,
        summary: "A PNG of the viewport as it is framed now: bodies by the CPU reference renderer (the oracle, whichever renderer the window uses), meshed at the display quality the viewport shows, sketches and axes over them. Written under the write root when path is given, otherwise returned as base64. With window: true (the desktop window only; then only path may be given) it is instead a frame of the window, panels and the controls over the viewport included, at the window's size in pixels: painted by the window's renderer into a texture and read back, once the viewport's picture is done (its frames averaged, the studio look's environment prepared, no popover or drawer still fading or sliding in), so it is taken also when the window is covered, minimised or on another workspace (where the window has no such texture, it is the next frame the compositor lets it draw).",
    },
    UiMethod {
        name: "ui.record",
        params: r#"{"path": string, "fps": number = 30, "step": boolean = false, "includeUi": boolean = true, "accumulate": integer = 1, "quality": "screen|web" = "screen", "start": true} | {"stop": true} | {}"#,
        summary: "Records the desktop window's own frames, panels and controls included (includeUi false: the viewport only), painted by its renderer into a texture of the window's size and read back, so it records also when the window is covered, minimised or on another workspace. path, under the write root, is a directory that gets frame-000001.png, frame-000002.png, ... or a video file (.mp4, .mkv, .mov, .webm, .gif, .avi, .m4v) written by an external ffmpeg (CRAFT3D_FFMPEG, else the one on PATH; an error when there is none) with the best encoder it has that works for the container (mp4: libx264, libopenh264, VAAPI/QSV/NVENC H.264, mpeg4; webm: VP9, SVT-AV1, AOM AV1, VP8), refused at the start when none does; quality screen (the default: crisp text, colour at full resolution where the encoder can, which some players do not play) or web (4:2:0, plays everywhere); a 4:2:0 encoder gets odd sides padded to even by repeating the last column or row; PNG frames are lossless. accumulate (1 to 256, 1 by default) is how many frames of the studio look's still a frame-exact frame waits for the viewport to average. Every frame has the size the window had when the recording started. Real time (step false): a frame every 1/fps seconds, the window showing what is recorded, what the person does in it included. Frame-exact (step: true): nothing is drawn until ui.advance, which draws its frames at a fixed dt each, every one once the window has settled (no command running, the scene built, the viewport's picture done); the person's input to the window is ignored meanwhile. {stop: true} ends it, once the frames are written (and ffmpeg has finished), and says how many; {} says how it stands (recording, path, format, fps, step, includeUi, accumulate, quality, width, height, encoder, encoded, frames, skipped, time). One recording at a time. The desktop window only.",
    },
    UiMethod {
        name: "ui.advance",
        params: r#"{"frames": integer = 1, "dt": number = 1/fps, "timeout": seconds = 30}"#,
        summary: "With a frame-exact recording (ui.record with step: true): draws and records frames frames (1 to 100000), each dt seconds (above 0, at most 60) of the window's clock after the last, so the history player, a flight, the orbit and a motion advance by exactly that; each frame waits until the window has settled (no command running, the scene built, the viewport's picture done) and is drawn then. Answered once the last frame is drawn, or after timeout seconds (above 0, at most 110) with the frames drawn so far (complete false, left the frames still to ask for): frames (those drawn), unsettled (frames whose picture had not settled within 4 s, drawn as they were), recorded (the recording's total) and time (the window's clock). The requests after it wait for it, but ui.get, closing the window and ui.record {stop: true}, which ends it at once with an error saying how many frames were drawn. A window that does not settle for the next frame within 60 s ends it with an error. An error without a frame-exact recording.",
    },
    UiMethod {
        name: "ui.param",
        params: r#"{"name": string, "value": number or true/false or a choice's label}"#,
        summary: "Sets one public parameter as the Parameters tab does: a number (in the parameter's unit, param.list unit) is first clamped to its range and snapped to its step, a whole-number kind rounded, and the form says so under the field (note, as ui.get parameters gives it); then param.set runs (in the desktop window on its engine thread, the reply coming once it is done; through MCP's control_call it is queued as the form's commit and the reply says queued: true instead of result). Returns sent (the value given to param.set), note, result (param.set's result, or {error}) and parameters (as ui.get). A value that breaks the model marks the field and a banner over the viewport offers Back to last good (parameters.trouble, broke: true); the viewport shows the document as it is. To set a value exactly, or one out of range to see it refused, use the command param.set.",
    },
    UiMethod {
        name: "ui.notice",
        params: r#"{"text": string}"#,
        summary: "Shows a one-line caption over the viewport; an empty text removes it.",
    },
    UiMethod {
        name: "ui.quit",
        params: r#"{"save": "discard|ask|save" = "discard"}"#,
        summary: "Closes the application. discard (the default): at once, dropping the unsaved changes and their autosaves. ask: as the window's close button does: at once when no document has unsaved changes, otherwise the window asks the person, one document at a time, Save, Don't Save or Cancel (ui.get: closing, with quitting: true, while it asks), and closes once each was saved or dropped; Cancel keeps it. The reply says quitting (closing now) and, while it asks, asking: the document it asks about (null while it first makes that document the active one, so the person sees it; ui.get's closing has it then). save: saves every document with unsaved changes to its own file through the automation write root, as file.save does, then closes; refused, with nothing saved, when one was never saved (save it with file.save and a path first) or its file lies outside the write root (a file the person chose is not the channel's to overwrite); a save that fails is an error. Not through control_call.",
    },
];

/// The params of a method: an object, or nothing at all.
#[derive(Clone, Copy)]
struct Fields<'a>(Option<&'a Map<String, Value>>);

impl<'a> Fields<'a> {
    fn get(&self, key: &str) -> Option<&'a Value> {
        self.0?.get(key)
    }
}

/// How `ui.quit` closes the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitMode {
    /// At once, unsaved changes and their autosaves dropped (the default).
    Discard,
    /// As the window's close button: the person is asked.
    Ask,
    /// Every document with unsaved changes saved to its file first.
    Save,
}

/// The `save` of `ui.quit`'s params. Never panics.
pub fn quit_mode(params: Option<&Value>) -> Result<QuitMode, String> {
    let fields = fields("ui.quit", params, &["save"])?;
    match fields.get("save") {
        None | Some(Value::Null) => Ok(QuitMode::Discard),
        Some(Value::String(mode)) => match mode.as_str() {
            "discard" => Ok(QuitMode::Discard),
            "ask" => Ok(QuitMode::Ask),
            "save" => Ok(QuitMode::Save),
            other => {
                let other: String = other.chars().take(40).collect();
                Err(format!(
                    "ui.quit: save is \"discard\", \"ask\" or \"save\", not \"{other}\""
                ))
            }
        },
        Some(_) => Err("ui.quit: save is \"discard\", \"ask\" or \"save\"".to_string()),
    }
}

/// The params of `method` as an object whose keys are all in `allowed`.
fn fields<'a>(
    method: &str,
    params: Option<&'a Value>,
    allowed: &[&str],
) -> Result<Fields<'a>, String> {
    let map = match params {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map),
        Some(_) => return Err(format!("{method}: parameters must be a JSON object")),
    };
    if let Some(unknown) = map
        .into_iter()
        .flat_map(Map::keys)
        .find(|key| !allowed.contains(&key.as_str()))
    {
        let unknown: String = unknown.chars().take(40).collect();
        let expected = if allowed.is_empty() {
            "it takes no parameters".to_string()
        } else {
            format!("its parameters are: {}", allowed.join(", "))
        };
        return Err(format!(
            "{method}: unknown parameter `{unknown}`; {expected}"
        ));
    }
    Ok(Fields(map))
}

/// A whole number of pixels within the limits of an image side.
fn side(method: &str, map: Fields<'_>, key: &str) -> Result<Option<u32>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (MIN_SIDE..=MAX_SIDE).contains(n))
            .map(Some)
            .ok_or_else(|| {
                format!("{method}: `{key}` must be a whole number from {MIN_SIDE} to {MAX_SIDE}")
            }),
    }
}

impl Shell {
    /// Runs `work` as a capture ([`Shell::exact_pictures`]): the session
    /// shows the exact moment of a playing motion, and the viewport goes
    /// back to what it was afterwards.
    fn exactly<R>(
        &mut self,
        session: &mut Session,
        work: impl FnOnce(&mut Self, &mut Session) -> R,
    ) -> R {
        let capturing = self.capturing;
        self.set_capturing(true);
        session.set_motion_live(false);
        let out = work(self, session);
        self.set_capturing(capturing);
        session.set_motion_live(!self.exact_pictures());
        out
    }

    /// Runs a `ui.*` method. `None` when `method` is not one of them.
    /// Never panics, whatever the params hold.
    pub fn handle_method(
        &mut self,
        session: &mut Session,
        method: &str,
        params: Option<&Value>,
    ) -> Option<Result<Value, String>> {
        // The active tab's view first: a method reads and changes that,
        // and what it puts back when it refuses is that too.
        self.follow_active_tab(session);
        // The fields that take face, edge or vertex names may write a
        // feature by its name, as a command's do (`Plate:face(1)`): looked
        // up in the active document, so the state holds ids.
        let name_keys: &[&str] = match method {
            "ui.highlight" => &["face", "edge", "between", "vertex", "corner"],
            "ui.set" => &["highlight", "highlightEdge", "highlightVertex", "section"],
            _ => &[],
        };
        let with_ids;
        let params = match params {
            Some(given) if !name_keys.is_empty() => {
                match session.with_feature_ids(name_keys, given) {
                    Ok(value) => {
                        with_ids = value;
                        Some(&with_ids)
                    }
                    Err(e) => return Some(Err(format!("{method}: {e}"))),
                }
            }
            other => other,
        };
        let result = match method {
            "ui.get" => self.ui_get(session, params),
            "ui.set" => self.ui_set(session, params),
            // What an agent sees, picks, frames or highlights is the
            // exact moment of a playing motion, at full quality: never the
            // provisional moment (a face of a substitute configuration) or
            // the quick picture the viewport shows while it plays.
            "ui.fit" => self.exactly(session, |shell, s| shell.ui_fit(s, params)),
            "ui.screenshot" => self.exactly(session, |shell, s| shell.ui_screenshot(s, params)),
            "ui.pick" => self.exactly(session, |shell, s| shell.ui_pick(s, params)),
            "ui.highlight" => self.exactly(session, |shell, s| shell.ui_highlight(s, params)),
            "ui.select" => self.ui_select(session, params),
            "ui.notice" => self.ui_notice(params),
            "ui.quit" => self.ui_quit(params),
            "ui.param" => self.ui_param(session, params),
            "ui.record" | "ui.advance" => Err(format!(
                "{method}: recording is done by the desktop window (craft3d --control), which paints its own frames"
            )),
            _ => return None,
        };
        Some(result)
    }

    /// `ui.quit` as the shell answers it: `discard` and `ask`. `save`
    /// needs the files of the documents, which only the desktop
    /// application writes ([`quit_mode`]).
    fn ui_quit(&mut self, params: Option<&Value>) -> Result<Value, String> {
        match quit_mode(params)? {
            QuitMode::Discard => {
                self.request_quit();
                Ok(json!({ "quitting": true }))
            }
            QuitMode::Ask => {
                if self.request_window_close() {
                    return Ok(json!({ "quitting": true }));
                }
                self.advance_quit();
                Ok(json!({
                    "quitting": self.quit_requested(),
                    "asking": self.closing(),
                }))
            }
            QuitMode::Save => {
                Err("ui.quit: save: \"save\" is done by the desktop window".to_string())
            }
        }
    }

    fn state_json(&self) -> Result<Value, String> {
        serde_json::to_value(&self.state).map_err(|e| e.to_string())
    }

    fn ui_get(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        self.ui_get_with(Some(session), params)
    }

    /// `ui.get` while the session is away doing work on the host's engine
    /// thread: the state as the window shows it (the scene and document
    /// of the last sync), with `work` saying what is being done.
    pub fn ui_get_away(&mut self, params: Option<&Value>) -> Result<Value, String> {
        self.ui_get_with(None, params)
    }

    fn ui_get_with(
        &mut self,
        session: Option<&mut Session>,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        fields("ui.get", params, &[])?;
        // The agent log only while the session is here (not while a
        // command runs on the engine thread).
        let (scene, status, agent_log) = match session {
            Some(session) => {
                self.sync_trace_states(session);
                (
                    self.sync(session),
                    session.document_status(),
                    session.agent_log_json(100),
                )
            }
            None => (
                self.shown_scene_arc(),
                self.shown_status().cloned(),
                Value::Null,
            ),
        };
        let [width, height] = self.viewport_size();
        let vec3 = |v: craft3d_math::Vec3| json!([v.x, v.y, v.z]);
        let scene_json = scene.as_deref().map_or(Value::Null, |scene| {
            json!({
                "revision": scene.revision,
                "bodies": scene.bodies.len(),
                "sketches": scene.sketches.len(),
                "triangles": scene.triangles(),
                "tolerance": scene.tolerance,
                "angle": scene.max_angle.map(f64::to_degrees),
                "coarsened": scene.coarsened,
                "refining": scene.preview,
                "bbox": scene.bbox.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
                "problems": scene.problems,
                "notices": scene.notices,
            })
        });
        let features: Vec<Value> = self
            .tree_rows()
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "name": row.name,
                    "kind": row.kind,
                    "state": row.state.name(),
                    "status": row.status,
                    "error": row.error,
                    "causedBy": row.caused_by.as_ref().map(|(id, name)| json!({"id": id, "name": name})),
                    "selected": row.selected,
                    "hidden": row.hidden,
                    "madeSelection": row.made_selection,
                    "zoomable": row.zoomable,
                    "when": row.when,
                    "passes": row.passes,
                })
            })
            .collect();
        let assertions: Vec<Value> = self
            .assertion_rows()
            .iter()
            .map(|row| {
                json!({
                    "name": row.name,
                    "measure": row.measure,
                    "status": row.status.name(),
                    "measured": row.measured,
                    "expected": row.expected,
                    "message": row.message,
                })
            })
            .collect();
        let document = status.map_or(Value::Null, |status| {
            json!({
                "index": status.index,
                "open": status.open,
                "path": status.path,
                "dirty": status.dirty,
                "units": { "length": status.length_unit, "angle": status.angle_unit },
            })
        });
        let report = self.renderer_report();
        let renderer = json!({
            "backend": report.backend.name(),
            "adapter": report.adapter,
            "fallback": report.fallback,
            "direct": report.direct,
            "msaa": report.msaa,
        });
        let render = self.render_stats().map_or(Value::Null, |stats| {
            json!({
                "renderer": stats.renderer.name(),
                "width": stats.width,
                "height": stats.height,
                "ssaa": stats.ssaa,
                "msaa": stats.msaa,
                "triangles": stats.triangles,
                "milliseconds": stats.milliseconds,
                "sharp": stats.sharp,
                "count": self.render_count(),
            })
        });
        Ok(json!({
            "state": self.state_json()?,
            "viewport": {
                "width": width,
                "height": height,
                "pixelsPerPoint": self.pixels_per_point(),
            },
            "document": document,
            "scene": scene_json,
            "features": features,
            "assertions": assertions,
            "renderer": renderer,
            "render": render,
            "section": self.section_report(),
            "lastCommand": self.last_command.as_ref().map(|report| json!({
                "label": report.label, "ok": report.ok, "text": report.text,
            })),
            "report": self.error_report(),
            "recovery": self.recovery().to_json(),
            "problem": self.problem(),
            "work": self.work(),
            "workShown": self.work_shown(),
            "traces": self.traces_json(),
            "tabs": self.tabs(),
            "closing": self.closing(),
            "quitting": self.quitting(),
            "selection": self.selection().json(),
            "parameters": self.parameters_report(),
            "recent": self.recent_json(),
            "startScreen": self.start_shown(),
            "agents": serde_json::to_value(self.agents_status_for_viewer()).unwrap_or(Value::Null),
            "agentLog": agent_log,
            "timeline": self.timeline_json(),
            "export": self.export_report(),
            "lens": self.lens_focus().map(|lens| lens.json()),
            "appearance": if self.resolved_dark() { "dark" } else { "light" },
        }))
    }

    fn ui_set(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let patch = match params {
            None | Some(Value::Null) => return Err("ui.set: nothing to set".to_string()),
            Some(patch) => patch,
        };
        // `analysis: null` turns the analysis off: no mode and no combs.
        let off;
        let patch = match patch {
            Value::Object(map) if map.get("analysis").is_some_and(Value::is_null) => {
                let mut map = map.clone();
                map.insert(
                    "analysis".into(),
                    serde_json::json!({"mode": "none", "combs": null}),
                );
                off = Value::Object(map);
                &off
            }
            _ => patch,
        };
        let before = self.state.clone();
        let lost_before = self.section_lost().map(str::to_string);
        self.state
            .apply_patch(patch)
            .map_err(|e| format!("ui.set: {e}"))?;
        // A highlight set here is the agent's, not the last click's light:
        // it stays when the selection changes, and is drawn under the
        // studio look's lens.
        let agent_highlight = ["highlight", "highlightEdge", "highlightVertex"]
            .iter()
            .any(|key| patch.get(*key).is_some());
        // The side panel's width is held to the tokens' range here, as it
        // is when the panel is drawn, so `ui.get` says the width in use.
        if patch.pointer("/panels/treeWidth").is_some() {
            let t = &self.tokens;
            let wanted = self.state.panels.tree_width;
            self.state.panels.tree_width = if wanted.is_finite() {
                wanted.clamp(t.tree_min_width, t.tree_max_width)
            } else {
                t.tree_width
            };
        }
        // A first section switched on without a place cuts through the
        // middle of the model, not at the origin; one switched on again
        // keeps its plane and offset.
        if let Some(section) = patch.get("section").and_then(Value::as_object)
            && section.get("on") == Some(&Value::Bool(true))
            && !before.section.on
            && before.section.plane.is_none()
            && before.section.normal.is_none()
            && before.section.point.is_none()
            && !["normal", "point", "feature"]
                .iter()
                .any(|k| section.contains_key(*k))
        {
            self.section_through_the_middle(section.contains_key("offset"));
        }
        // The look is checked (an unknown environment or tone curve
        // changes nothing) and kept as parsed: clamped, turned within 0 to
        // 360 degrees.
        // The analysis likewise.
        if patch.get("analysis").is_some() {
            match self.state.analysis.spec() {
                Ok(spec) => self.state.analysis = crate::state::AnalysisState::of(&spec),
                Err(why) => {
                    self.state = before;
                    return Err(format!("ui.set: {why}"));
                }
            }
        }
        if patch.get("look").is_some() {
            match self.state.look.checked() {
                Ok(look) => self.state.look = look,
                Err(why) => {
                    self.state = before;
                    return Err(format!("ui.set: {why}"));
                }
            }
        }
        // The export dialog first, before anything with an effect beyond
        // the state (selecting a feature): what it refuses leaves nothing
        // changed. One it opened goes again when the rest is refused.
        let had_export = self.export_dialog().is_some();
        if let Some(export) = patch.get("export")
            && let Err(why) = self.ui_set_export(session, &before.export, export)
        {
            self.state = before;
            return Err(format!("ui.set: export: {why}"));
        }
        let opened_export = !had_export && self.export_dialog().is_some();
        let sets_section = patch.get("section").is_some();
        if sets_section {
            self.clear_section_lost();
        }
        // The selected feature is the selection's first feature: setting
        // it selects that feature, null takes the features out.
        if let Some(chosen) = patch.get("selectedFeature") {
            let request = match chosen {
                Value::String(id) => json!({"items": [{"kind": "feature", "feature": id}]}),
                _ => {
                    let features: Vec<Value> = self
                        .selection()
                        .items
                        .iter()
                        .filter(|item| !item.kind.is_entity())
                        .map(craft3d_engine::SelectionItem::json)
                        .collect();
                    json!({"items": features, "mode": "remove"})
                }
            };
            // A feature that does not exist is dropped below, as before.
            if session.execute("select.set", &request).is_err() {
                self.state.selected_feature = None;
            }
        }
        // A selection that names no feature is dropped here, not later.
        let scene = self.sync(session);
        // Nothing to play: refused, and nothing has changed.
        // An assembly has no features of its own but plays its motion.
        if self.state.timeline.playing
            && !before.timeline.playing
            && !self.has_steps()
            && !self.playing_motion()
        {
            if opened_export {
                self.discard_export_dialog();
            }
            self.state = before;
            self.sync(session);
            return Err(if scene.is_some() {
                "ui.set: timeline.playing: the document has no features to play; add one first"
                    .to_string()
            } else {
                "ui.set: timeline.playing: there is no document; open one first".to_string()
            });
        }
        // A section whose plane means nothing is refused, not switched
        // off behind the caller's back: nothing has changed.
        if sets_section
            && scene.is_some()
            && let Some(why) = self.section_lost().map(str::to_string)
        {
            if opened_export {
                self.discard_export_dialog();
            }
            self.state = before;
            self.sync(session);
            if lost_before.is_none() {
                self.clear_section_lost();
            }
            return Err(format!("ui.set: {why}"));
        }
        if agent_highlight {
            self.highlight_set_elsewhere();
        }
        Ok(json!({ "state": self.state_json()? }))
    }

    fn ui_fit(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let map = fields("ui.fit", params, &["view", "feature", "selection"])?;
        let view = match map.get("view") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => Some(name.as_str()),
            Some(_) => return Err("ui.fit: `view` must be a string".to_string()),
        };
        let feature = match map.get("feature") {
            None | Some(Value::Null) => None,
            Some(Value::String(feature)) => Some(feature.clone()),
            Some(_) => return Err("ui.fit: `feature` must be a feature's id or name".to_string()),
        };
        let selection = match map.get("selection") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(selection)) => *selection,
            Some(_) => return Err("ui.fit: `selection` must be true or false".to_string()),
        };
        let to = match (feature, selection) {
            (Some(_), true) => {
                return Err("ui.fit: give `feature` or `selection`, not both".to_string());
            }
            (Some(feature), false) => Some(FlyTo::Feature(feature)),
            (None, true) => Some(FlyTo::Selection),
            (None, false) => None,
        };
        let mut camera = self.state.camera;
        if let Some(name) = view {
            camera.set_view(name).map_err(|e| format!("ui.fit: {e}"))?;
        }
        let before = (self.state.camera, self.state.show, self.state.section.on);
        self.state.camera = camera;
        if let Some(to) = to {
            let Some(mut scene) = self.sync(session) else {
                self.state.camera = before.0;
                return Err("ui.fit: there is no document; open one first".to_string());
            };
            // A hidden target is shown again and a scrub before it let go
            // of, first (`shell/timeline.rs`).
            match self.reveal_now(session, &scene, &to) {
                Ok(false) => {}
                Ok(true) => {
                    if let Some(again) = self.sync(session) {
                        scene = again;
                    }
                }
                Err(why) => {
                    self.state.camera = before.0;
                    return Err(format!("ui.fit: {why}"));
                }
            }
            if let Err(why) = self.fly_to(&scene, &to, None) {
                (self.state.camera, self.state.show, self.state.section.on) = before;
                return Err(format!("ui.fit: {why}"));
            }
            // A section switched off to show it is resolved again.
            self.sync(session);
            return Ok(json!({ "camera": self.state_json()?.get("camera") }));
        }
        let before = before.0;
        if !self.fit(session) {
            self.state.camera = before;
            return Err(
                "ui.fit: there is nothing to frame; add a feature or a sketch first".to_string(),
            );
        }
        Ok(json!({ "camera": self.state_json()?.get("camera") }))
    }

    fn ui_select(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.select";
        fields(method, params, &["items", "names", "mode"])?;
        let params = params.cloned().unwrap_or(Value::Null);
        let result = session.execute("select.set", &params).map_err(|e| {
            format!(
                "{method}: {}",
                e.message().trim_start_matches("select.set: ")
            )
        })?;
        self.sync(session);
        Ok(result)
    }

    fn ui_notice(&mut self, params: Option<&Value>) -> Result<Value, String> {
        let map = fields("ui.notice", params, &["text"])?;
        let Some(Value::String(text)) = map.get("text") else {
            return Err("ui.notice: `text` must be a string (empty to remove it)".to_string());
        };
        self.state
            .apply_patch(&json!({ "notice": text }))
            .map_err(|e| format!("ui.notice: {e}"))?;
        Ok(json!({ "notice": self.state.notice }))
    }

    fn ui_pick(&mut self, session: &mut Session, params: Option<&Value>) -> Result<Value, String> {
        let method = "ui.pick";
        let map = fields(
            method,
            params,
            &[
                "x", "y", "width", "height", "radius", "select", "all", "shift", "alt",
            ],
        )?;
        let number = |key: &str| match map.get(key) {
            None | Some(Value::Null) => Err(format!("{method}: missing parameter `{key}`")),
            Some(value) => value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{method}: `{key}` must be a finite number")),
        };
        let (x, y) = (number("x")?, number("y")?);
        let [viewport_width, viewport_height] = self.viewport_size();
        let width = side(method, map, "width")?.unwrap_or(viewport_width);
        let height = side(method, map, "height")?.unwrap_or(viewport_height);
        let radius = match map.get("radius") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|r| (0.0..=craft3d_engine::MAX_PICK_RADIUS).contains(r))
                    .ok_or_else(|| {
                        format!(
                            "{method}: `radius` must be a number of pixels from 0 to {}",
                            craft3d_engine::MAX_PICK_RADIUS
                        )
                    })?,
            ),
        };
        let flag = |key: &str| match map.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(on)) => Ok(*on),
            Some(_) => Err(format!("{method}: `{key}` must be true or false")),
        };
        let select = flag("select")?;
        let all = flag("all")?;
        let (shift, alt) = (flag("shift")?, flag("alt")?);
        if (shift || alt) && !select {
            return Err(format!(
                "{method}: `shift` and `alt` are keys held with a click; give `select`: true too"
            ));
        }
        let scene = self.sync(session);
        let Some(scene) = scene else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        // A click is in the viewport's own pixels.
        let stretch = f64::from(viewport_width) / f64::from(width);
        let (px, py) = if select {
            (
                x * stretch,
                y * f64::from(viewport_height) / f64::from(height),
            )
        } else {
            (x, y)
        };
        let size = if select {
            [viewport_width, viewport_height]
        } else {
            [width, height]
        };
        let pick_radius = if select {
            radius.map(|radius| radius * stretch)
        } else {
            radius
        };
        let pointed = self
            .pick(Some(&scene), px, py, Some(size), pick_radius, select)
            .map_err(|e| format!("{method}: {e}"))?;
        let hits = if all {
            let [w, h] = size;
            let hits = scene
                .pick_all_cut(
                    &self.state.camera.to_render(),
                    w,
                    h,
                    px,
                    py,
                    self.section().as_ref(),
                )
                .map_err(|e| format!("{method}: {e}"))?;
            Some(hits)
        } else {
            None
        };
        if select {
            // A click selects too (Shift: toggles; Alt or the same spot
            // again: the next face behind).
            self.click_select(Some(&scene), px, py, &pointed, (shift, alt, false));
            self.apply_selection_requests(session);
            self.sync(session);
        }
        let outside = !(x >= 0.0 && y >= 0.0 && x < f64::from(width) && y < f64::from(height));
        let mut result = json!({
            "x": x,
            "y": y,
            "width": width,
            "height": height,
        });
        if let Some(map) = result.as_object_mut() {
            pointed.describe(map);
            if pointed.kind().is_none() {
                map.insert("outside".into(), outside.into());
            }
            if let Some(hits) = &hits {
                map.insert(
                    "hits".into(),
                    hits.iter()
                        .map(craft3d_engine::ScenePick::json)
                        .collect::<Vec<_>>()
                        .into(),
                );
            }
            if select {
                let state = self.state_json()?;
                for key in ["picked", "highlight", "highlightEdge", "highlightVertex"] {
                    map.insert(key.into(), state.get(key).cloned().unwrap_or(Value::Null));
                }
                map.insert("selection".into(), self.selection().json());
            }
        }
        Ok(result)
    }

    fn ui_highlight(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        // What an agent points at stays lit when the selection changes.
        self.highlight_set_elsewhere();
        let map = fields(
            method,
            params,
            &["face", "edge", "between", "vertex", "corner", "feature"],
        )?;
        let feature = match map.get("feature") {
            None | Some(Value::Null) => None,
            Some(Value::String(feature)) => Some(feature.clone()),
            // An id may be given as a number, as the commands take it.
            Some(Value::Number(id)) if id.is_u64() => Some(id.to_string()),
            Some(_) => {
                return Err(format!(
                    "{method}: `feature` must be a feature id (3 or \"F3\") or name"
                ));
            }
        };
        let of_edge = map.get("edge").is_some() || map.get("between").is_some();
        let of_vertex = map.get("vertex").is_some() || map.get("corner").is_some();
        if of_edge || of_vertex {
            if map.get("face").is_some() || (of_edge && of_vertex) {
                return Err(format!(
                    "{method}: give one of `face`, `edge`, `between`, `vertex` and `corner`"
                ));
            }
            return if of_edge {
                self.highlight_edge(session, &map, feature)
            } else {
                self.highlight_vertex(session, &map, feature)
            };
        }
        let face = match map.get("face") {
            None => {
                return Err(format!(
                    "{method}: missing parameter `face`: a face name from document.inspect, or null to remove the highlight (or `edge` or `between` for an edge, `vertex` or `corner` for a vertex)"
                ));
            }
            Some(Value::Null) => {
                if feature.is_some() {
                    return Err(format!(
                        "{method}: `feature` says which body has `face`; without a face there is nothing to highlight"
                    ));
                }
                self.state.highlight = None;
                self.state.highlight_edge = None;
                self.state.highlight_vertex = None;
                return Ok(json!({ "highlight": null, "faces": 0 }));
            }
            Some(Value::String(face)) => face,
            Some(_) => return Err(format!("{method}: `face` must be a face name or null")),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .faces_named(feature.as_deref(), face)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown =
            craft3d_render::Highlight::new(self.tokens.face_highlight, found.ids.iter().copied());
        self.state.highlight = Some(crate::state::FaceRef {
            feature: Some(found.feature.clone()),
            face: found.face.clone(),
        });
        // One thing is pointed at.
        self.state.highlight_edge = None;
        self.state.highlight_vertex = None;
        let mut result = json!({
            "highlight": { "feature": found.feature, "face": found.face },
            "name": found.body,
            "faces": found.ids.len(),
        });
        // More pieces than the renderers take runs of: said, not hidden.
        if let (Some(map), (_, left_out @ 1..)) = (result.as_object_mut(), shown) {
            map.insert("facesNotShown".into(), left_out.into());
        }
        Ok(result)
    }

    /// `ui.highlight` for an edge: `edge` or `between`.
    fn highlight_edge(
        &mut self,
        session: &mut Session,
        map: &Fields<'_>,
        feature: Option<String>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        let edge = match map.get("edge") {
            None => None,
            Some(Value::String(edge)) => Some(edge.as_str()),
            Some(_) => {
                return Err(format!(
                    "{method}: `edge` must be an edge name from document.edges (`face`: null removes the highlight)"
                ));
            }
        };
        let between: Vec<String> = match map.get("between") {
            None => Vec::new(),
            Some(Value::Array(names)) => match names.as_slice() {
                [Value::String(a), Value::String(b)] => vec![a.clone(), b.clone()],
                _ => return Err(format!("{method}: `between` takes two face names")),
            },
            Some(_) => return Err(format!("{method}: `between` takes two face names")),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .edges_named(feature.as_deref(), edge, &between)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown = crate::state::EdgeRef {
            feature: Some(found.feature.clone()),
            edge: found.edge.clone(),
            between: found.between.clone(),
        };
        let result = json!({
            "highlightEdge": shown,
            "name": found.body,
            "edges": found.edges.len(),
        });
        self.state.highlight_edge = Some(shown);
        // One thing is pointed at.
        self.state.highlight = None;
        self.state.highlight_vertex = None;
        Ok(result)
    }

    /// `ui.highlight` for a vertex: `vertex` or `corner`.
    fn highlight_vertex(
        &mut self,
        session: &mut Session,
        map: &Fields<'_>,
        feature: Option<String>,
    ) -> Result<Value, String> {
        let method = "ui.highlight";
        let vertex = match map.get("vertex") {
            None => None,
            Some(Value::String(vertex)) => Some(vertex.as_str()),
            Some(_) => {
                return Err(format!(
                    "{method}: `vertex` must be a vertex name from document.edges (`face`: null removes the highlight)"
                ));
            }
        };
        let wrong = || {
            format!(
                "{method}: `corner` takes one to {} face names",
                craft3d_engine::MAX_CORNER_FACES
            )
        };
        let corner: Vec<String> = match map.get("corner") {
            None => Vec::new(),
            Some(Value::Array(names))
                if !names.is_empty() && names.len() <= craft3d_engine::MAX_CORNER_FACES =>
            {
                names
                    .iter()
                    .map(|name| name.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or_else(wrong)?
            }
            Some(_) => return Err(wrong()),
        };
        let Some(scene) = self.sync(session) else {
            return Err(match self.problem() {
                Some(problem) => format!("{method}: {problem}"),
                None => format!("{method}: no document is open; run file.new or file.open first"),
            });
        };
        let found = scene
            .vertices_named(feature.as_deref(), vertex, &corner)
            .map_err(|e| format!("{method}: {e}"))?;
        let shown = crate::state::VertexRef {
            feature: Some(found.feature.clone()),
            vertex: found.vertex.clone(),
            corner: found.corner.clone(),
        };
        let result = json!({
            "highlightVertex": shown,
            "name": found.body,
            "vertices": found.vertices.len(),
        });
        self.state.highlight_vertex = Some(shown);
        // One thing is pointed at.
        self.state.highlight = None;
        self.state.highlight_edge = None;
        Ok(result)
    }

    /// `ui.screenshot {window: true, path?}` as the desktop window answers
    /// it: `image` is the window's own frame (panels, controls and all,
    /// as egui drew it), saved as a PNG under the write root when `path`
    /// is given, otherwise returned as base64.
    pub fn window_picture(
        &self,
        session: &mut Session,
        image: &egui::ColorImage,
        path: Option<&str>,
    ) -> Result<Value, String> {
        let method = "ui.screenshot";
        let [width, height] = image.size;
        let mut rgb = Vec::with_capacity(image.pixels.len().saturating_mul(3));
        for pixel in &image.pixels {
            rgb.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
        }
        let (w, h) = (
            u32::try_from(width).map_err(|e| format!("{method}: {e}"))?,
            u32::try_from(height).map_err(|e| format!("{method}: {e}"))?,
        );
        let png = craft3d_render::encode_png(w, h, &rgb).map_err(|e| format!("{method}: {e}"))?;
        let mut result = json!({
            "width": width,
            "height": height,
            "window": true,
            "bytes": png.len(),
        });
        if let Some(map) = result.as_object_mut() {
            match path {
                Some(path) => {
                    session
                        .write_output(path, &png)
                        .map_err(|e| format!("{method}: {e}"))?;
                    map.insert("path".into(), path.into());
                }
                None => {
                    map.insert("mime".into(), "image/png".into());
                    map.insert("base64".into(), craft3d_engine::encode_base64(&png).into());
                }
            }
        }
        Ok(result)
    }

    /// Whether the request `line` is `ui.screenshot {window: true}`, and
    /// then its path or why its parameters are wrong. A path the session
    /// may not write is refused here, before the window is captured.
    pub fn window_picture_request(
        session: &Session,
        line: &str,
    ) -> Option<Result<Option<String>, String>> {
        let method = "ui.screenshot";
        let request: Value = serde_json::from_str(line).ok()?;
        if request.get("method").and_then(Value::as_str) != Some(method) {
            return None;
        }
        let params = request.get("params");
        if params.and_then(|p| p.get("window")) != Some(&Value::Bool(true)) {
            return None;
        }
        let checked = (|| {
            let map = fields(method, params, &["path", "window"])?;
            match map.get("path") {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(path)) => {
                    session
                        .workspace()
                        .resolve_write(path)
                        .map_err(|e| format!("{method}: {e}"))?;
                    Ok(Some(path.clone()))
                }
                Some(_) => Err(format!("{method}: `path` must be a string")),
            }
        })();
        Some(checked)
    }

    fn ui_screenshot(
        &mut self,
        session: &mut Session,
        params: Option<&Value>,
    ) -> Result<Value, String> {
        let method = "ui.screenshot";
        let map = fields(
            method,
            params,
            &["path", "width", "height", "scale", "window"],
        )?;
        match map.get("window") {
            None | Some(Value::Null | Value::Bool(false)) => {}
            Some(Value::Bool(true)) => {
                return Err(format!(
                    "{method}: `window` is a picture of the desktop window with its panels and controls, which only the window (craft3d --control) takes"
                ));
            }
            Some(_) => return Err(format!("{method}: `window` must be a boolean")),
        }
        let path = match map.get("path") {
            None | Some(Value::Null) => None,
            Some(Value::String(path)) => Some(path.as_str()),
            Some(_) => return Err(format!("{method}: `path` must be a string")),
        };
        let [viewport_width, viewport_height] = self.viewport_size();
        let width = side(method, map, "width")?.unwrap_or(viewport_width);
        let height = side(method, map, "height")?.unwrap_or(viewport_height);
        let scale = match map.get("scale") {
            None | Some(Value::Null) => f64::from(self.pixels_per_point()),
            Some(value) => value
                .as_f64()
                .filter(|s| (0.25..=8.0).contains(s))
                .ok_or_else(|| format!("{method}: `scale` must be a number from 0.25 to 8"))?,
        };
        // Refuse a path the session may not write before rendering.
        if let Some(path) = path {
            session
                .workspace()
                .resolve_write(path)
                .map_err(|e| format!("{method}: {e}"))?;
        }
        let scene = self.sync(session);
        let studio = self.studio_now();
        let picture = viewport::picture_lit(
            scene.as_deref(),
            &self.state.camera,
            Quality {
                width,
                height,
                ssaa: crate::shell::REST_SSAA,
            },
            &self.tokens,
            self.state.selected_feature.as_deref(),
            self.face_highlight(scene.as_deref()),
            &self.marks(scene.as_deref()),
            self.opacity(),
            (studio, self.analysis_view.clone()),
            scale,
        )
        .map_err(|e| format!("{method}: {e}"))?;
        let png = craft3d_render::encode_png(picture.width, picture.height, &picture.rgb)
            .map_err(|e| format!("{method}: {e}"))?;
        let mut result = json!({
            "width": picture.width,
            "height": picture.height,
            "triangles": picture.triangles,
            "coverage": picture.coverage,
            "overlayCoverage": picture.overlay_coverage,
            "bytes": png.len(),
        });
        if let Some(map) = result.as_object_mut() {
            match path {
                Some(path) => {
                    session
                        .write_output(path, &png)
                        .map_err(|e| format!("{method}: {e}"))?;
                    map.insert("path".into(), path.into());
                }
                None => {
                    map.insert("mime".into(), "image/png".into());
                    map.insert("base64".into(), craft3d_engine::encode_base64(&png).into());
                }
            }
        }
        Ok(result)
    }
}
