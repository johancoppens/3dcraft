//! The commands of assemblies (docs/design/assemblies.md): start one,
//! place instances of other documents in it, mate them, solve, pack the
//! sources in, and read what came of it. What they derive is in `asm.rs`.

use crate::asm::{self, Built, Placed};
use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session};
use craft3d_doc::{
    Assembly, Blob, Coupling, CouplingKind, DocError, Document, Expr, Instance, InstanceMode, Mate,
    MateKind, MateRef, ParamOverride, Placement, Source,
};
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("assembly.new", "New Assembly", r#"{"units": "um|mm|cm|m|in|ft" = "mm", "tolerance": number = 1e-6}"#,
        "Creates an empty assembly (saved as a .3dcraftasm file) and makes it the active document. An assembly places instances of other documents (instance.add) and joins them with mates (mate.add); it has variables, assertions and outputs as a part has, and no features of its own.", doc: false, assembly_new),
    spec!("instance.add", "Add Instance", r#"{"source": path of a .3dcraft or .3dcraftasm file, "name": string = the file's stem with [n], "body": feature name or id of the source = all its bodies, "params": {name: expr} = none, "position": [number, number, number] = [0, 0, 0], "rotation": [number, number, number] (a rotation vector, degrees) = [0, 0, 0], "mode": "rigid|flexible" = "rigid", "pinned": boolean = false, "visible": boolean = true, "colour": [r, g, b] = the source's}"#,
        "Adds an instance of a source document to the active assembly (one undo step): a reference, never a copy; the source is read from the read root, and its path is kept relative to the assembly's file, with the SHA-256 of its bytes. body picks one body of a multi-body source (a Jig), by the feature that made it: the instance shows only that body, where the source puts it, and fails by its name when it is gone. params sets the source's public parameters, as expressions of the assembly's variables: each distinct set (a configuration) is regenerated once and shared by every instance that uses it, whichever body they pick. pinned keeps the instance to the source's bytes of now. Returns the instance as assembly.inspect shows it.", doc: true, instance_add),
    spec!("instance.edit", "Edit Instance", r#"{"name": string, "rename": string, "source": path, "body": feature name or id or null, "params": {name: expr or null}, "position": [number, number, number], "rotation": [number, number, number], "mode": "rigid|flexible", "pinned": boolean, "visible": boolean, "colour": [r, g, b] or null}"#,
        "Changes an instance (one undo step): what is given replaces what it had; params merges, a null removes an override; pinned true pins it to the source's bytes of now, false follows the file again (and takes its new hash). A rename rewrites the mates that name it. Returns the instance.", doc: true, instance_edit),
    spec!("instance.delete", "Delete Instance", r#"{"name": string}"#,
        "Removes an instance (one undo step). An instance a mate still names is refused, with the mates named: delete them first.", doc: true, instance_delete),
    spec!("instance.open", "Open Instance Source", r#"{"name": string}"#,
        "Opens an instance's source document in its own tab, as file.open does, so it is edited there (an assembly never edits a part's history); a packed source opens as a new document without a file. The assembly follows the file once it is saved (assembly.reload).", doc: true, instance_open),
    spec!("mate.add", "Add Mate", r#"{"kind": "fixed|revolute|slider|cylindrical|planar|ball", "a": "Instance/Feature:name" or "world/xy|yz|zx|x|y|z|origin", "b": same, "name": string = Mate<n>, "offset": length expr = 0, "angle": angle expr = 0, "limits": [expr, expr], "flip": boolean = false}"#,
        "Joins two instances (one undo step). a and b name geometry through an instance, by the persistent names of the source (Bar[1]/F1:face(3)), or a datum of the source by its name or id (Bar[1]/Pivot); world/... is the world. Each gives a frame: a plane at a planar face's centroid with its outward normal, the axis of a cylindrical or conical face at the point nearest its centroid, the centre and axis of a circular edge, a point at a vertex, a datum's own. b's frame is put against a's, turned over (flip keeps it as it is), shifted by offset along and turned by angle about a's z; the kind says what stays free. A name that is gone fails the mate by that name; it never retargets. A free motion is driven by the variable <mate>_angle (or _distance) when it exists (joint.set). Returns the mate with its status.", doc: true, mate_add),
    spec!("mate.edit", "Edit Mate", r#"{"name": string, "rename": string, ...the parameters of mate.add, "offset"/"angle"/"limits": null to remove}"#,
        "Changes a mate (one undo step): what is given replaces what it had. Returns the mate with its status.", doc: true, mate_edit),
    spec!("mate.delete", "Delete Mate", r#"{"name": string}"#,
        "Removes a mate and the variables that drive it (one undo step). A mate a coupling still names is refused.", doc: true, mate_delete),
    spec!("coupling.add", "Add Coupling", r#"{"kind": "gear|rack|screw", "mates": [mate, mate], "value": expr (the ratio, the radius or the lead), "name": string = Coupling<n>}"#,
        "Ties two mates' motions (one undo step): gear θa = −ratio·θb, rack d = radius·θ, screw d = lead·θ/360°. Returns the coupling.", doc: true, coupling_add),
    spec!("coupling.delete", "Delete Coupling", r#"{"name": string}"#,
        "Removes a coupling (one undo step).", doc: true, coupling_delete),
    spec!("joint.set", "Set Joint", r#"{"mate": string, "value": expr or null, "which": "angle|distance|x|y" = the mate's main free motion}"#,
        "Drives a mate's free motion (one undo step): sets the assembly variable <mate>_<which> to value (an angle or a length expression), creating it, so it can be used in formulas and published as a parameter (variable.publish); null removes the variable and leaves the motion free. Returns the mate with its status.", doc: true, joint_set),
    spec!("assembly.solve", "Solve Assembly", r#"{}"#,
        "Solves the active assembly as it is and says how: ok, each mate's status and the value of its free motion, each instance's degrees of freedom left and pose, floating when no mate reaches the world (the first instance is then held where it is placed), and the error that names the mates that contradict each other.", doc: true, assembly_solve),
    spec!("assembly.inspect", "Inspect Assembly", r#"{"measures": boolean = false}"#,
        "Describes the active assembly: its instances (source, hash, whether the file changed since, pinned, body, parameters, configuration, status, pose, the features of the shown part that hold its bodies), its mates and couplings, the number of distinct configurations and how many regenerations they cost so far, and the packed sources; with measures each instance's placed volume and box.", doc: true, assembly_inspect),
    spec!("assembly.reload", "Reload Sources", r#"{}"#,
        "Reads the active assembly's source files again (and those of its subassemblies) and lists the instances whose source changed: an instance follows its file unless it is pinned, and only a changed source is regenerated. Not an undo step.", doc: true, assembly_reload),
    spec!("assembly.pack", "Pack Sources", r#"{}"#,
        "Packs every source document into the assembly (one undo step): each instance then refers to its source by hash, and the saved .3dcraftasm file carries the sources, subassemblies packed with their own sources, so it rebuilds anywhere. Returns the packed sources.", doc: true, assembly_pack),
    spec!("assembly.unpack", "Unpack Sources", r#"{"folder": path = "parts", "overwrite": boolean = false}"#,
        "Writes each packed source document as a file into folder (beneath the write root, relative to the assembly's file) and makes its instances refer to that file (one undo step). Returns the files written.", doc: true, assembly_unpack),
];

/// The assembly of the active document, or an error that says it is a
/// part.
fn assembly_of<'a>(session: &'a Session, command: &str) -> Result<&'a Assembly, CommandError> {
    session
        .document()?
        .history
        .current()
        .assembly
        .as_ref()
        .ok_or_else(|| {
            CommandError::new(format!(
                "{command}: the active document is a part, not an assembly; start one with assembly.new or open a .3dcraftasm file"
            ))
        })
}

fn assembly_new(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("assembly.new", params, &["units", "tolerance"])?;
    let mut given = Map::new();
    for key in ["units", "tolerance"] {
        if let Some(v) = p.get(key) {
            given.insert(key.to_string(), v.clone());
        }
    }
    let result = crate::commands::file_new(session, &Value::Object(given))
        .map_err(|e| CommandError::new(e.message().replace("file.new", "assembly.new")))?;
    let open = session.document_mut()?;
    let mut doc = Document::clone(open.history.current());
    doc.assembly = Some(Assembly::default());
    *open = OpenDocument::new(doc, None);
    let mut result = result;
    if let Some(map) = result.as_object_mut() {
        map.insert("assembly".into(), true.into());
    }
    Ok(result)
}

impl Session {
    /// The folder the active assembly's source paths are relative to:
    /// its file's, or for one not saved yet the read root (the working
    /// folder, absolute, with unrestricted access).
    pub(crate) fn asm_base(&self) -> Result<String, CommandError> {
        let open = self.document()?;
        if let Some(path) = &open.path {
            return Ok(asm::dir_of(path));
        }
        Ok(self.unsaved_base())
    }

    fn unsaved_base(&self) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        if matches!(self.workspace.read, crate::Access::Unrestricted) {
            return std::path::absolute(".")
                .ok()
                .and_then(|p| p.to_str().map(str::to_string))
                .unwrap_or_default();
        }
        String::new()
    }

    /// A source path as the request gave it, written as [`asm::join`]
    /// writes paths: absolute with unrestricted access, beneath the root
    /// otherwise.
    fn full_source_path(&self, path: &str) -> Result<String, CommandError> {
        let resolved = self.workspace.resolve_read(path)?;
        let kept = crate::workspace::kept_path(&self.workspace.read, path, &resolved);
        asm::join("", &kept).map_err(CommandError::new)
    }

    /// Reads one file for an assembly, by its full path.
    fn read_source_file(&self, full: &str) -> Result<Blob, String> {
        let resolved = self
            .workspace
            .resolve_read(full)
            .map_err(|e| e.message().to_string())?;
        self.read_input(full, &resolved, craft3d_format::MAX_FILE_BYTES)
            .map(Blob::new)
    }

    /// Reads the source files of the active assembly that are not read
    /// yet (all of them with `again`), with those of its subassemblies,
    /// to [`asm::MAX_DEPTH`] levels, each file once. Returns the paths
    /// whose bytes changed.
    pub(crate) fn read_sources(&mut self, again: bool) -> Result<Vec<String>, CommandError> {
        let base = self.asm_base()?;
        let open = self.document()?;
        let doc = Arc::clone(open.history.current());
        if !doc.is_assembly() {
            return Ok(Vec::new());
        }
        let mut files = open.asm.files.clone();
        let mut changed = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        // (document, the folder its sources are relative to, depth)
        let mut stack: Vec<(Arc<Document>, String, usize)> = vec![(doc, base, 0)];
        while let Some((doc, base, depth)) = stack.pop() {
            let Some(assembly) = &doc.assembly else {
                continue;
            };
            if depth > asm::MAX_DEPTH {
                continue;
            }
            for instance in &assembly.instances {
                let (blob, sub_base) = match &instance.source {
                    Source::Packed { hash } => match assembly.packed(hash) {
                        Some(blob) => (blob.clone(), base.clone()),
                        None => continue,
                    },
                    Source::File { path, .. } => {
                        let Ok(full) = asm::join(&base, path) else {
                            continue;
                        };
                        if !seen.insert(full.clone()) || seen.len() > asm::MAX_SOURCE_FILES {
                            continue;
                        }
                        let read = match files.get(&full) {
                            Some(found) if !again => found.clone(),
                            before => {
                                let now = self.read_source_file(&full);
                                let differs = match (before, &now) {
                                    (Some(Ok(a)), Ok(b)) => a.sha256() != b.sha256(),
                                    (Some(_), _) => true,
                                    (None, _) => false,
                                };
                                if differs {
                                    changed.push(full.clone());
                                }
                                files.insert(full.clone(), now.clone());
                                now
                            }
                        };
                        match read {
                            Ok(blob) => (blob, asm::dir_of(&full)),
                            Err(_) => continue,
                        }
                    }
                };
                // A subassembly's own sources, below it.
                if let Ok(sub) = craft3d_format::from_bytes(blob.bytes())
                    && sub.is_assembly()
                {
                    stack.push((Arc::new(sub), sub_base, depth.saturating_add(1)));
                }
            }
        }
        let open = self.document_mut()?;
        if open.asm.files != files {
            open.asm.files = files;
            open.asm.invalidate();
            open.assertions = crate::assert_check::Checks::default();
        }
        Ok(changed)
    }

    /// Refuses a source that contains, through its subassemblies, the
    /// active assembly's own file or itself: a subassembly cycle. Depth
    /// limited ([`asm::MAX_DEPTH`]).
    fn refuse_cycle(&self, full: &str, blob: &Blob) -> Result<(), String> {
        let own = self.document().ok().and_then(|o| {
            o.path
                .as_deref()
                .map(|p| asm::join("", p).unwrap_or_else(|_| p.to_string()))
        });
        let mut chain: Vec<String> = own.into_iter().collect();
        self.cycle_from(full, blob, &mut chain, 0)
    }

    fn cycle_from(
        &self,
        full: &str,
        blob: &Blob,
        chain: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), String> {
        if chain.iter().any(|c| c == full) {
            return Err(format!(
                "`{full}` contains itself through its subassemblies: a subassembly cycle is refused"
            ));
        }
        if depth > asm::MAX_DEPTH {
            return Err(format!(
                "subassemblies nest deeper than {} levels below `{full}`",
                asm::MAX_DEPTH
            ));
        }
        let Ok(doc) = craft3d_format::from_bytes(blob.bytes()) else {
            return Ok(());
        };
        let Some(assembly) = &doc.assembly else {
            return Ok(());
        };
        chain.push(full.to_string());
        let base = asm::dir_of(full);
        for instance in &assembly.instances {
            let Source::File { path, .. } = &instance.source else {
                continue;
            };
            let Ok(sub) = asm::join(&base, path) else {
                continue;
            };
            if let Ok(bytes) = self.read_source_file(&sub) {
                self.cycle_from(&sub, &bytes, chain, depth.saturating_add(1))?;
            }
        }
        chain.pop();
        Ok(())
    }

    /// The active assembly placed.
    pub(crate) fn placed(&mut self) -> Result<Arc<Placed>, CommandError> {
        self.document_mut()?
            .placed()
            .ok_or_else(|| CommandError::new("the active document is not an assembly"))
    }
}

fn instance_json(placed: &Placed, instance: &Instance) -> Value {
    let st = placed.instance(&instance.name);
    let (kind, path) = match &instance.source {
        Source::File { path, .. } => ("file", Value::from(path.clone())),
        Source::Packed { .. } => ("packed", Value::Null),
    };
    let params: Map<String, Value> = instance
        .params
        .iter()
        .map(|p| (p.name.clone(), Value::from(p.expr.source())))
        .collect();
    let bodies: Vec<Value> = st
        .map(|s| {
            s.bodies
                .iter()
                .map(|id| Value::from(id.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let pose = st.map(|s| {
        let o = s.pose.point(craft3d_math::Vec3::ZERO);
        let x = s.pose.vector(craft3d_math::Vec3::X);
        let z = s.pose.vector(craft3d_math::Vec3::Z);
        let y = s.pose.vector(craft3d_math::Vec3::Y);
        json!({ "origin": [o.x, o.y, o.z], "x": [x.x, x.y, x.z], "y": [y.x, y.y, y.z], "z": [z.x, z.y, z.z] })
    });
    json!({
        "name": instance.name,
        "source": { "kind": kind, "path": path, "hash": instance.source.hash() },
        "builtFrom": st.and_then(|s| s.hash.clone()),
        "changed": st.is_some_and(|s| s.changed),
        "pinned": instance.pinned,
        "body": instance.body,
        "params": params,
        "configuration": st.and_then(|s| s.config.clone()),
        "subassembly": st.and_then(|s| s.built.as_ref()).is_some_and(|b| matches!(b.as_ref(), Built::Assembly(_))),
        "mode": instance.mode.name(),
        "visible": instance.visible,
        "colour": instance.colour,
        "position": instance.placement.position,
        "rotation": instance.placement.rotation,
        "status": if st.is_some_and(|s| s.error.is_none()) { "ok" } else { "failed" },
        "error": st.and_then(|s| s.error.clone()),
        "pose": pose,
        "bodies": bodies,
    })
}

fn mate_json(placed: &Placed, mate: &Mate) -> Value {
    let st = placed.mates.iter().find(|m| m.name == mate.name);
    let failed = st.and_then(|s| s.error.clone());
    let status = match (&failed, &placed.solve_error) {
        (Some(_), _) => "failed",
        (None, Some(_)) => "unsolved",
        (None, None) => "ok",
    };
    json!({
        "name": mate.name,
        "kind": mate.kind.name(),
        "a": mate.a.to_string(),
        "b": mate.b.to_string(),
        "offset": mate.offset.as_ref().map(Expr::source),
        "angle": mate.angle.as_ref().map(Expr::source),
        "limits": mate.limits.as_ref().map(|(a, b)| [a.source(), b.source()]),
        "flip": mate.flip,
        "free": mate.kind.free(),
        "variables": mate.kind.free().iter().map(|w| asm::joint_variable(&mate.name, w)).collect::<Vec<_>>(),
        "driven": st.is_some_and(|s| s.driven),
        "value": st.and_then(|s| s.free),
        "status": status,
        "error": failed,
    })
}

fn coupling_json(c: &Coupling) -> Value {
    json!({
        "name": c.name,
        "kind": c.kind.name(),
        "mates": [c.mates.0, c.mates.1],
        "value": c.value.source(),
    })
}

fn solve_json(placed: &Placed, assembly: &Assembly) -> Value {
    let dof: Vec<Value> = placed
        .instances
        .iter()
        .filter(|i| i.error.is_none())
        .zip(placed.dof.iter().map(Some).chain(std::iter::repeat(None)))
        .map(|(i, d)| json!({ "instance": i.name, "dof": d }))
        .collect();
    let failed: Vec<Value> = placed
        .instances
        .iter()
        .filter_map(|i| Some(json!({ "instance": i.name, "error": i.error.clone()? })))
        .chain(
            placed
                .mates
                .iter()
                .filter_map(|m| Some(json!({ "mate": m.name, "error": m.error.clone()? }))),
        )
        .collect();
    json!({
        "ok": placed.solve_error.is_none() && failed.is_empty(),
        "error": placed.solve_error,
        "floating": placed.floating,
        "failed": failed,
        "dof": dof,
        "mates": assembly.mates.iter().map(|m| mate_json(placed, m)).collect::<Vec<_>>(),
        "stand_in": "the assembly solver of track B has not landed; a forward-kinematics stand-in solved this (no couplings, open chains only)",
    })
}

/// The one instance named `name`, as JSON, after an edit.
fn one_instance(session: &mut Session, name: &str) -> Run {
    let placed = session.placed()?;
    let assembly = assembly_of(session, "instance")?;
    let instance = assembly
        .instance(name)
        .ok_or_else(|| CommandError::new(format!("there is no instance `{name}`")))?;
    Ok(instance_json(&placed, instance))
}

fn one_mate(session: &mut Session, name: &str) -> Run {
    let placed = session.placed()?;
    let assembly = assembly_of(session, "mate")?;
    let mate = assembly
        .mate(name)
        .ok_or_else(|| CommandError::new(format!("there is no mate `{name}`")))?;
    Ok(mate_json(&placed, mate))
}

fn vec3_param(p: &Params, key: &str) -> Result<Option<[f64; 3]>, CommandError> {
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let wrong = || {
        CommandError::new(format!(
            "{}: `{key}` must be three finite numbers [x, y, z]",
            p.command()
        ))
    };
    let Value::Array(items) = value else {
        return Err(wrong());
    };
    let numbers: Vec<f64> = items
        .iter()
        .map(|v| v.as_f64().filter(|n| n.is_finite() && n.abs() < 1e12))
        .collect::<Option<Vec<f64>>>()
        .ok_or_else(wrong)?;
    match numbers.as_slice() {
        [x, y, z] => Ok(Some([*x, *y, *z])),
        _ => Err(wrong()),
    }
}

fn colour_param(p: &Params, key: &str) -> Result<Option<[u8; 3]>, CommandError> {
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let wrong = || {
        CommandError::new(format!(
            "{}: `{key}` must be [r, g, b], each a whole number from 0 to 255",
            p.command()
        ))
    };
    let Value::Array(items) = value else {
        return Err(wrong());
    };
    let bytes: Vec<u8> = items
        .iter()
        .map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(wrong)?;
    match bytes.as_slice() {
        [r, g, b] => Ok(Some([*r, *g, *b])),
        _ => Err(wrong()),
    }
}

fn mode_param(p: &Params) -> Result<Option<InstanceMode>, CommandError> {
    match p.str("mode")? {
        None => Ok(None),
        Some(text) => InstanceMode::from_name(text).map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{}: `mode` must be rigid or flexible, got `{}`",
                p.command(),
                text.chars().take(40).collect::<String>()
            ))
        }),
    }
}

/// `params`: `{name: expr}`, a null for one to remove (with `nulls`).
fn overrides_param(
    p: &Params,
    nulls: bool,
) -> Result<Option<Vec<(String, Option<String>)>>, CommandError> {
    let Some(value) = p.get("params") else {
        return Ok(None);
    };
    let Value::Object(map) = value else {
        return fail(format!(
            "{}: `params` must be an object of public parameter names and expressions, such as {{\"length\": \"80 mm\"}}",
            p.command()
        ));
    };
    let mut out = Vec::new();
    for (name, v) in map {
        if v.is_null() && nulls {
            out.push((name.clone(), None));
        } else {
            out.push((name.clone(), Some(p.expr_of("params", v)?)));
        }
    }
    Ok(Some(out))
}

/// The default name of an instance of `path`: its file's stem with the
/// first free `[n]`.
fn default_instance_name(assembly: &Assembly, path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file);
    let stem: String = stem
        .chars()
        .filter(|c| !c.is_control() && *c != '/')
        .take(48)
        .collect();
    let stem = if stem.trim().is_empty() {
        "Part".to_string()
    } else {
        stem.trim().to_string()
    };
    (1..=assembly.instances.len().saturating_add(1))
        .map(|n| format!("{stem}[{n}]"))
        .find(|name| assembly.instance(name).is_none())
        .unwrap_or_else(|| format!("{stem}[{}]", assembly.instances.len().saturating_add(1)))
}

fn instance_add(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "instance.add";
    let p = Params::new(
        ID,
        params,
        &[
            "source", "name", "body", "params", "position", "rotation", "mode", "pinned",
            "visible", "colour",
        ],
    )?;
    let assembly = assembly_of(session, ID)?.clone();
    let path = p.required_str("source")?;
    let base = session.asm_base()?;
    let full = session.full_source_path(path)?;
    let blob = session
        .read_source_file(&full)
        .map_err(|e| CommandError::new(format!("{ID}: cannot read the source `{path}`: {e}")))?;
    // Something a document reads as, or nothing is added.
    let source_doc = craft3d_format::from_bytes(blob.bytes())
        .map_err(|e| CommandError::new(format!("{ID}: `{path}` is not a 3DCraft document: {e}")))?;
    let relative = asm::relative_to(&base, &full);
    craft3d_doc::assembly::check_source_path(&relative)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    session
        .refuse_cycle(&full, &blob)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let hash = blob.sha256();
    let name = match p.str("name")? {
        Some(name) => name.to_string(),
        None => default_instance_name(&assembly, path),
    };
    let body = p.str("body")?.map(str::to_string);
    if let Some(body) = &body
        && source_doc.is_assembly()
    {
        return fail(format!(
            "{ID}: `{path}` is an assembly; body picks one body of a part (`{body}`)"
        ));
    }
    let overrides = overrides_param(&p, false)?.unwrap_or_default();
    let placement = Placement {
        position: vec3_param(&p, "position")?.unwrap_or_default(),
        rotation: vec3_param(&p, "rotation")?.unwrap_or_default(),
    };
    let instance = Instance {
        name: name.clone(),
        source: Source::File {
            path: relative,
            hash: hash.clone(),
        },
        body,
        params: overrides
            .into_iter()
            .filter_map(|(name, expr)| {
                Some(ParamOverride {
                    name,
                    expr: Expr::new(expr?),
                })
            })
            .collect(),
        pinned: p.bool("pinned")?.unwrap_or(false).then(|| hash.clone()),
        mode: mode_param(&p)?.unwrap_or_default(),
        visible: p.bool("visible")?.unwrap_or(true),
        colour: colour_param(&p, "colour")?,
        placement,
    };
    {
        let open = session.document_mut()?;
        open.asm.files.insert(full, Ok(blob));
        open.asm.invalidate();
    }
    edit(session, ID, &format!("Add instance {name}"), |doc| {
        let assembly = doc.assembly_mut()?;
        if assembly.instance(&instance.name).is_some() {
            return Err(DocError::Invalid(format!(
                "there is an instance `{}` already",
                instance.name
            )));
        }
        assembly.instances.push(instance);
        Ok(())
    })?;
    session.read_sources(false)?;
    one_instance(session, &name)
}

fn instance_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "instance.edit";
    let p = Params::new(
        ID,
        params,
        &[
            "name", "rename", "source", "body", "params", "position", "rotation", "mode", "pinned",
            "visible", "colour",
        ],
    )?;
    let assembly = assembly_of(session, ID)?.clone();
    let name = p.required_str("name")?.to_string();
    let old = assembly
        .instance(&name)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no instance `{name}`")))?
        .clone();
    let mut instance = old.clone();
    let base = session.asm_base()?;
    // A new source, read now.
    if let Some(path) = p.str("source")? {
        let full = session.full_source_path(path)?;
        let blob = session.read_source_file(&full).map_err(|e| {
            CommandError::new(format!("{ID}: cannot read the source `{path}`: {e}"))
        })?;
        craft3d_format::from_bytes(blob.bytes()).map_err(|e| {
            CommandError::new(format!("{ID}: `{path}` is not a 3DCraft document: {e}"))
        })?;
        let relative = asm::relative_to(&base, &full);
        session
            .refuse_cycle(&full, &blob)
            .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        instance.source = Source::File {
            path: relative,
            hash: blob.sha256(),
        };
        let open = session.document_mut()?;
        open.asm.files.insert(full, Ok(blob));
        open.asm.invalidate();
    }
    if p.has("body") {
        instance.body = p.str("body")?.map(str::to_string);
    }
    if let Some(changes) = overrides_param(&p, true)? {
        for (k, v) in changes {
            match v {
                None => instance.params.retain(|o| o.name != k),
                Some(expr) => match instance.params.iter_mut().find(|o| o.name == k) {
                    Some(o) => o.expr = Expr::new(expr),
                    None => instance.params.push(ParamOverride {
                        name: k,
                        expr: Expr::new(expr),
                    }),
                },
            }
        }
    }
    if let Some(position) = vec3_param(&p, "position")? {
        instance.placement.position = position;
    }
    if let Some(rotation) = vec3_param(&p, "rotation")? {
        instance.placement.rotation = rotation;
    }
    if let Some(mode) = mode_param(&p)? {
        instance.mode = mode;
    }
    if let Some(visible) = p.bool("visible")? {
        instance.visible = visible;
    }
    if p.has("colour") {
        instance.colour = colour_param(&p, "colour")?;
    }
    if let Some(pinned) = p.bool("pinned")? {
        // Pinned to the bytes the instance is built from now; unpinned,
        // it follows its file and takes the file's hash.
        let placed = session.placed()?;
        let now = placed.instance(&name).and_then(|s| s.hash.clone());
        if pinned {
            instance.pinned = Some(now.ok_or_else(|| {
                CommandError::new(format!(
                    "{ID}: `{name}` has no source read to pin it to; assembly.reload reads it"
                ))
            })?);
        } else {
            instance.pinned = None;
            if let (Source::File { hash, .. }, Some(now)) = (&mut instance.source, now) {
                *hash = now;
            }
        }
    }
    let rename = p.str("rename")?.map(str::to_string);
    if let Some(new) = &rename {
        instance.name = new.clone();
    }
    let shown = instance.name.clone();
    edit(session, ID, &format!("Edit instance {name}"), |doc| {
        let assembly = doc.assembly_mut()?;
        if let Some(new) = &rename
            && *new != name
            && assembly.instance(new).is_some()
        {
            return Err(DocError::Invalid(format!(
                "there is an instance `{new}` already"
            )));
        }
        let slot = assembly
            .instances
            .iter_mut()
            .find(|i| i.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no instance `{name}`")))?;
        *slot = instance;
        if let Some(new) = &rename {
            for mate in &mut assembly.mates {
                for r in [&mut mate.a, &mut mate.b] {
                    if r.instance == name {
                        r.instance = new.clone();
                    }
                }
            }
        }
        Ok(())
    })?;
    session.read_sources(false)?;
    one_instance(session, &shown)
}

fn instance_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "instance.delete";
    let p = Params::new(ID, params, &["name"])?;
    let name = p.required_str("name")?.to_string();
    let assembly = assembly_of(session, ID)?;
    if assembly.instance(&name).is_none() {
        return fail(format!("{ID}: there is no instance `{name}`"));
    }
    let using: Vec<String> = assembly
        .mates
        .iter()
        .filter(|m| m.a.instance == name || m.b.instance == name)
        .map(|m| format!("`{}`", m.name))
        .collect();
    if !using.is_empty() {
        return fail(format!(
            "{ID}: the mates {} name `{name}`; delete them first (mate.delete)",
            using.join(", ")
        ));
    }
    edit(session, ID, &format!("Delete instance {name}"), |doc| {
        let assembly = doc.assembly_mut()?;
        assembly.instances.retain(|i| i.name != name);
        assembly.drop_unused_packed();
        Ok(())
    })?;
    Ok(json!({ "deleted": name }))
}

fn instance_open(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "instance.open";
    let p = Params::new(ID, params, &["name"])?;
    let name = p.required_str("name")?.to_string();
    let assembly = assembly_of(session, ID)?.clone();
    let instance = assembly
        .instance(&name)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no instance `{name}`")))?;
    match &instance.source {
        Source::File { path, .. } => {
            let full = asm::join(&session.asm_base()?, path)
                .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
            let mut result = crate::commands::file_open(session, &json!({ "path": full }))
                .map_err(|e| CommandError::new(e.message().replace("file.open", ID)))?;
            if let Some(map) = result.as_object_mut() {
                map.insert("instance".into(), name.into());
            }
            Ok(result)
        }
        Source::Packed { hash } => {
            let blob = assembly.packed(hash).ok_or_else(|| {
                CommandError::new(format!("{ID}: the packed source {hash} is missing"))
            })?;
            let doc = craft3d_format::from_bytes(blob.bytes()).map_err(|e| {
                CommandError::new(format!("{ID}: the packed source does not read: {e}"))
            })?;
            let index = session.add_document(OpenDocument::new(doc, None))?;
            Ok(json!({ "document": index, "path": null, "instance": name, "packed": true }))
        }
    }
}

/// Reads `a` or `b` of a mate.
fn reference(p: &Params, key: &str) -> Result<Option<MateRef>, CommandError> {
    match p.str(key)? {
        None => Ok(None),
        Some(text) => MateRef::parse(text)
            .map(Some)
            .map_err(|e| CommandError::new(format!("{}: `{key}`: {e}", p.command()))),
    }
}

fn kind_param(p: &Params) -> Result<Option<MateKind>, CommandError> {
    match p.str("kind")? {
        None => Ok(None),
        Some(text) => MateKind::from_name(text).map(Some).ok_or_else(|| {
            CommandError::new(format!(
                "{}: `kind` must be fixed, revolute, slider, cylindrical, planar or ball, got `{}`",
                p.command(),
                text.chars().take(40).collect::<String>()
            ))
        }),
    }
}

fn limits_param(p: &Params) -> Result<Option<(Expr, Expr)>, CommandError> {
    let Some(value) = p.get("limits") else {
        return Ok(None);
    };
    let wrong = || {
        CommandError::new(format!(
            "{}: `limits` must be [lower, upper], two expressions",
            p.command()
        ))
    };
    let Value::Array(items) = value else {
        return Err(wrong());
    };
    match items.as_slice() {
        [lo, hi] => Ok(Some((
            Expr::new(p.expr_of("limits", lo)?),
            Expr::new(p.expr_of("limits", hi)?),
        ))),
        _ => Err(wrong()),
    }
}

/// The first free name `<prefix><n>`.
fn next_name(prefix: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..=100_000usize)
        .map(|n| format!("{prefix}{n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| format!("{prefix}0"))
}

const MATE_KEYS: [&str; 9] = [
    "kind", "a", "b", "name", "offset", "angle", "limits", "flip", "rename",
];

fn mate_add(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "mate.add";
    let p = Params::new(ID, params, &MATE_KEYS[..8])?;
    let assembly = assembly_of(session, ID)?;
    let kind = kind_param(&p)?.ok_or_else(|| p.missing("kind"))?;
    let a = reference(&p, "a")?.ok_or_else(|| p.missing("a"))?;
    let b = reference(&p, "b")?.ok_or_else(|| p.missing("b"))?;
    let name = match p.str("name")? {
        Some(name) => name.to_string(),
        None => next_name("Mate", |n| {
            assembly.mate(n).is_some() || assembly.coupling(n).is_some()
        }),
    };
    let mate = Mate {
        name: name.clone(),
        kind,
        a,
        b,
        offset: p.expr("offset")?.map(Expr::new),
        angle: p.expr("angle")?.map(Expr::new),
        limits: limits_param(&p)?,
        flip: p.bool("flip")?.unwrap_or(false),
    };
    edit(session, ID, &format!("Add mate {name}"), |doc| {
        let assembly = doc.assembly_mut()?;
        if assembly.mate(&mate.name).is_some() || assembly.coupling(&mate.name).is_some() {
            return Err(DocError::Invalid(format!(
                "the name `{}` is taken",
                mate.name
            )));
        }
        assembly.mates.push(mate);
        Ok(())
    })?;
    one_mate(session, &name)
}

fn mate_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "mate.edit";
    let p = Params::new(ID, params, &MATE_KEYS)?;
    let assembly = assembly_of(session, ID)?;
    let name = p.required_str("name")?.to_string();
    let mut mate = assembly
        .mate(&name)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no mate `{name}`")))?
        .clone();
    if let Some(kind) = kind_param(&p)? {
        mate.kind = kind;
    }
    if let Some(a) = reference(&p, "a")? {
        mate.a = a;
    }
    if let Some(b) = reference(&p, "b")? {
        mate.b = b;
    }
    if p.has("offset") {
        mate.offset = p.expr("offset")?.map(Expr::new);
    }
    if p.has("angle") {
        mate.angle = p.expr("angle")?.map(Expr::new);
    }
    if p.has("limits") {
        mate.limits = limits_param(&p)?;
    }
    if let Some(flip) = p.bool("flip")? {
        mate.flip = flip;
    }
    let rename = p.str("rename")?.map(str::to_string);
    if let Some(new) = &rename {
        mate.name = new.clone();
    }
    let shown = mate.name.clone();
    edit(session, ID, &format!("Edit mate {name}"), |doc| {
        if let Some(new) = &rename
            && *new != name
        {
            let assembly = doc.assembly()?;
            if assembly.mate(new).is_some() || assembly.coupling(new).is_some() {
                return Err(DocError::Invalid(format!("the name `{new}` is taken")));
            }
            // The variables that drive it follow its name.
            for which in ["angle", "distance", "x", "y"] {
                let old_var = asm::joint_variable(&name, which);
                if doc.variables.iter().any(|v| v.name == old_var) {
                    doc.rename_variable(&old_var, &asm::joint_variable(new, which))?;
                }
            }
        }
        let assembly = doc.assembly_mut()?;
        let slot = assembly
            .mates
            .iter_mut()
            .find(|m| m.name == name)
            .ok_or_else(|| DocError::Invalid(format!("there is no mate `{name}`")))?;
        *slot = mate;
        if let Some(new) = &rename {
            for c in &mut assembly.couplings {
                for m in [&mut c.mates.0, &mut c.mates.1] {
                    if *m == name {
                        *m = new.clone();
                    }
                }
            }
        }
        Ok(())
    })?;
    one_mate(session, &shown)
}

fn mate_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "mate.delete";
    let p = Params::new(ID, params, &["name"])?;
    let name = p.required_str("name")?.to_string();
    let assembly = assembly_of(session, ID)?;
    if assembly.mate(&name).is_none() {
        return fail(format!("{ID}: there is no mate `{name}`"));
    }
    let using: Vec<String> = assembly
        .couplings
        .iter()
        .filter(|c| c.mates.0 == name || c.mates.1 == name)
        .map(|c| format!("`{}`", c.name))
        .collect();
    if !using.is_empty() {
        return fail(format!(
            "{ID}: the couplings {} name `{name}`; delete them first (coupling.delete)",
            using.join(", ")
        ));
    }
    edit(session, ID, &format!("Delete mate {name}"), |doc| {
        doc.assembly_mut()?.mates.retain(|m| m.name != name);
        for which in ["angle", "distance", "x", "y"] {
            let var = asm::joint_variable(&name, which);
            if doc.variables.iter().any(|v| v.name == var) {
                doc.remove_variable(&var)?;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "deleted": name }))
}

fn coupling_add(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "coupling.add";
    let p = Params::new(ID, params, &["kind", "mates", "value", "name"])?;
    let assembly = assembly_of(session, ID)?;
    let kind = match p.str("kind")? {
        None => return Err(p.missing("kind")),
        Some(text) => CouplingKind::from_name(text).ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: `kind` must be gear, rack or screw, got `{}`",
                text.chars().take(40).collect::<String>()
            ))
        })?,
    };
    let mates = match p.get("mates") {
        Some(Value::Array(items)) => match items.as_slice() {
            [Value::String(a), Value::String(b)] => (a.clone(), b.clone()),
            _ => return fail(format!("{ID}: `mates` must be two mate names")),
        },
        Some(_) => return fail(format!("{ID}: `mates` must be two mate names")),
        None => return Err(p.missing("mates")),
    };
    let value = p.expr("value")?.ok_or_else(|| p.missing("value"))?;
    let name = match p.str("name")? {
        Some(name) => name.to_string(),
        None => next_name("Coupling", |n| {
            assembly.mate(n).is_some() || assembly.coupling(n).is_some()
        }),
    };
    let coupling = Coupling {
        name: name.clone(),
        kind,
        mates,
        value: Expr::new(value),
    };
    let shown = coupling_json(&coupling);
    edit(session, ID, &format!("Add coupling {name}"), |doc| {
        doc.assembly_mut()?.couplings.push(coupling);
        Ok(())
    })?;
    Ok(shown)
}

fn coupling_delete(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "coupling.delete";
    let p = Params::new(ID, params, &["name"])?;
    let name = p.required_str("name")?.to_string();
    if assembly_of(session, ID)?.coupling(&name).is_none() {
        return fail(format!("{ID}: there is no coupling `{name}`"));
    }
    edit(session, ID, &format!("Delete coupling {name}"), |doc| {
        doc.assembly_mut()?.couplings.retain(|c| c.name != name);
        Ok(())
    })?;
    Ok(json!({ "deleted": name }))
}

fn joint_set(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "joint.set";
    let p = Params::new(ID, params, &["mate", "value", "which"])?;
    let name = p.required_str("mate")?.to_string();
    let assembly = assembly_of(session, ID)?;
    let mate = assembly
        .mate(&name)
        .ok_or_else(|| CommandError::new(format!("{ID}: there is no mate `{name}`")))?;
    let free = mate.kind.free();
    let which = match p.str("which")? {
        Some(w) if free.contains(&w) => w.to_string(),
        Some(w) => {
            return fail(format!(
                "{ID}: a {} mate has no free motion `{}`; it has {}",
                mate.kind.name(),
                w.chars().take(40).collect::<String>(),
                if free.is_empty() {
                    "none".to_string()
                } else {
                    free.join(", ")
                }
            ));
        }
        None => match free.first() {
            Some(w) => (*w).to_string(),
            None => {
                return fail(format!(
                    "{ID}: a {} mate has no free motion to set",
                    mate.kind.name()
                ));
            }
        },
    };
    if !p.has("value") {
        return Err(p.missing("value"));
    }
    let value = p.expr("value")?;
    let var = asm::joint_variable(&name, &which);
    edit(session, ID, &format!("Set {var}"), |doc| match &value {
        Some(expr) => doc.set_variable(&var, expr),
        None => {
            if doc.variables.iter().any(|v| v.name == var) {
                doc.remove_variable(&var)
            } else {
                Ok(())
            }
        }
    })?;
    let mut result = one_mate(session, &name)?;
    if let Some(map) = result.as_object_mut() {
        map.insert("variable".into(), var.into());
    }
    Ok(result)
}

fn assembly_solve(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assembly.solve";
    Params::new(ID, params, &[])?;
    let assembly = assembly_of(session, ID)?.clone();
    let placed = session.placed()?;
    Ok(solve_json(&placed, &assembly))
}

/// The volume of each instance's placed bodies, and the centre of the
/// box around them.
fn measured(placed: &Placed, name: &str) -> Value {
    let Some(st) = placed.instance(name) else {
        return Value::Null;
    };
    let mut volume = 0.0;
    let mut extent = craft3d_math::Bbox3::EMPTY;
    for id in &st.bodies {
        let Some(body) = placed.regen.feature(*id).and_then(|f| f.own_body()) else {
            continue;
        };
        if let Ok(m) = craft3d_topo::mass_properties(body) {
            volume += m.volume;
        }
        if let Some((lo, hi)) = body.held_bbox().ok().and_then(|b| b.corners()) {
            extent.add(lo);
            extent.add(hi);
        }
    }
    let corners = extent
        .corners()
        .map(|(lo, hi)| json!([[lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]]));
    json!({ "volume": volume, "box": corners })
}

fn assembly_inspect(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assembly.inspect";
    let p = Params::new(ID, params, &["measures"])?;
    let measures = p.bool("measures")?.unwrap_or(false);
    let assembly = assembly_of(session, ID)?.clone();
    let placed = session.placed()?;
    let configurations: std::collections::BTreeSet<&String> = placed
        .instances
        .iter()
        .filter_map(|i| i.config.as_ref())
        .collect();
    let builds = session.document()?.asm.builds;
    Ok(json!({
        "instances": assembly.instances.iter().map(|i| {
            let mut entry = instance_json(&placed, i);
            if let (true, Some(map)) = (measures, entry.as_object_mut()) {
                map.insert("measured".into(), measured(&placed, &i.name));
            }
            entry
        }).collect::<Vec<_>>(),
        "mates": assembly.mates.iter().map(|m| mate_json(&placed, m)).collect::<Vec<_>>(),
        "couplings": assembly.couplings.iter().map(coupling_json).collect::<Vec<_>>(),
        "configurations": configurations.len(),
        "regenerations": builds,
        "packed": assembly.packed.iter().map(|b| json!({ "hash": b.sha256(), "bytes": b.len() })).collect::<Vec<_>>(),
        "bodies": placed.regen.bodies().count(),
        "solve": solve_json(&placed, &assembly),
    }))
}

fn assembly_reload(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assembly.reload";
    Params::new(ID, params, &[])?;
    assembly_of(session, ID)?;
    let changed = session.read_sources(true)?;
    let assembly = assembly_of(session, ID)?.clone();
    let placed = session.placed()?;
    let instances: Vec<Value> = placed
        .instances
        .iter()
        .filter(|i| i.changed)
        .map(|i| Value::from(i.name.clone()))
        .collect();
    let failed: Vec<Value> = placed
        .instances
        .iter()
        .filter_map(|i| Some(json!({ "instance": i.name, "error": i.error.clone()? })))
        .collect();
    Ok(json!({
        "filesChanged": changed,
        "changed": instances,
        "failed": failed,
        "instances": assembly.instances.len(),
    }))
}

/// The bytes of `doc` with every instance's source packed into it, its
/// subassemblies packed the same way first (to [`asm::MAX_DEPTH`]).
fn packed_document(
    session: &Session,
    doc: &Document,
    base: &str,
    depth: usize,
) -> Result<Document, String> {
    if depth > asm::MAX_DEPTH {
        return Err(format!(
            "subassemblies nest deeper than {} levels",
            asm::MAX_DEPTH
        ));
    }
    let mut doc = doc.clone();
    let Some(assembly) = doc.assembly.as_mut() else {
        return Ok(doc);
    };
    let files = &session
        .document()
        .map_err(|e| e.message().to_string())?
        .asm
        .files;
    let mut packs = Vec::new();
    for instance in &mut assembly.instances {
        let (blob, sub_base) = match &instance.source {
            Source::Packed { .. } => continue,
            Source::File { path, .. } => {
                let full = asm::join(base, path)?;
                let blob = match files.get(&full) {
                    Some(Ok(blob)) => blob.clone(),
                    _ => session.read_source_file(&full).map_err(|e| {
                        format!(
                            "instance `{}`: its source `{path}` cannot be read: {e}",
                            instance.name
                        )
                    })?,
                };
                (blob, asm::dir_of(&full))
            }
        };
        let blob = match craft3d_format::from_bytes(blob.bytes()) {
            Ok(sub) if sub.is_assembly() => {
                let packed = packed_document(session, &sub, &sub_base, depth.saturating_add(1))?;
                Blob::new(craft3d_format::to_bytes(&packed).map_err(|e| e.to_string())?)
            }
            Ok(_) => blob,
            Err(e) => return Err(format!("instance `{}`: {e}", instance.name)),
        };
        let hash = blob.sha256();
        if let Some(pinned) = &mut instance.pinned {
            *pinned = hash.clone();
        }
        instance.source = Source::Packed { hash };
        packs.push(blob);
    }
    for blob in packs {
        assembly.pack(blob);
    }
    assembly.drop_unused_packed();
    Ok(doc)
}

fn assembly_pack(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assembly.pack";
    Params::new(ID, params, &[])?;
    assembly_of(session, ID)?;
    session.read_sources(false)?;
    let base = session.asm_base()?;
    let doc = Arc::clone(session.document()?.history.current());
    let packed = packed_document(session, &doc, &base, 0)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let new = packed.assembly.clone().unwrap_or_default();
    edit(session, ID, "Pack sources", |doc| {
        doc.assembly = Some(new);
        Ok(())
    })?;
    let assembly = assembly_of(session, ID)?;
    Ok(json!({
        "packed": assembly.packed.iter().map(|b| json!({ "hash": b.sha256(), "bytes": b.len() })).collect::<Vec<_>>(),
    }))
}

/// A file name for a packed source: its first instance's name, with what
/// a file name cannot hold left out, and the first free number.
fn unpacked_name(
    instance: &str,
    taken: &std::collections::BTreeSet<String>,
    assembly: bool,
) -> String {
    let stem: String = instance
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let stem = stem.trim_matches('_');
    let stem = if stem.is_empty() { "part" } else { stem };
    let extension = if assembly {
        craft3d_format::ASSEMBLY_EXTENSION
    } else {
        "3dcraft"
    };
    (1..=10_000usize)
        .map(|n| {
            if n == 1 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem}_{n}.{extension}")
            }
        })
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| format!("{stem}_x.{extension}"))
}

fn assembly_unpack(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "assembly.unpack";
    let p = Params::new(ID, params, &["folder", "overwrite"])?;
    let folder = p.str("folder")?.unwrap_or("parts").to_string();
    let overwrite = p.bool("overwrite")?.unwrap_or(false);
    craft3d_doc::assembly::check_source_path(&folder)
        .map_err(|e| CommandError::new(format!("{ID}: `folder`: {e}")))?;
    let assembly = assembly_of(session, ID)?.clone();
    if assembly.packed.is_empty() {
        return fail(format!("{ID}: the assembly has no packed sources"));
    }
    let base = session.asm_base()?;
    // One file per packed source, named after its first instance.
    let mut taken = std::collections::BTreeSet::new();
    let mut plan: Vec<(String, String, String)> = Vec::new(); // (hash, relative path, full)
    for instance in &assembly.instances {
        let Source::Packed { hash } = &instance.source else {
            continue;
        };
        if plan.iter().any(|(h, ..)| h == hash) {
            continue;
        }
        let Some(blob) = assembly.packed(hash) else {
            continue;
        };
        let is_asm = craft3d_format::from_bytes(blob.bytes()).is_ok_and(|d| d.is_assembly());
        let file = unpacked_name(&instance.name, &taken, is_asm);
        taken.insert(file.clone());
        let relative = format!("{folder}/{file}");
        let full =
            asm::join(&base, &relative).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
        plan.push((hash.clone(), relative, full));
    }
    let mut written = Vec::new();
    for (hash, relative, full) in &plan {
        let Some(blob) = assembly.packed(hash) else {
            continue;
        };
        let resolved = session.workspace.resolve_write(full)?;
        if !overwrite && file_exists(session, &resolved) {
            return fail(format!(
                "{ID}: `{relative}` exists already; pass {{\"overwrite\": true}} to replace it"
            ));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !session.writes_to_memory()
            && let Some(parent) = resolved.parent()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                CommandError::new(format!("{ID}: cannot make the folder of `{relative}`: {e}"))
            })?;
        }
        session.write_resolved(&resolved, blob.bytes().to_vec())?;
        let open = session.document_mut()?;
        open.asm.files.insert(full.clone(), Ok(blob.clone()));
        written.push(json!({ "path": relative, "hash": hash }));
    }
    session.document_mut()?.asm.invalidate();
    edit(session, ID, "Unpack sources", |doc| {
        let assembly = doc.assembly_mut()?;
        for instance in &mut assembly.instances {
            if let Source::Packed { hash } = &instance.source
                && let Some((_, relative, _)) = plan.iter().find(|(h, ..)| h == hash)
            {
                instance.source = Source::File {
                    path: relative.clone(),
                    hash: hash.clone(),
                };
            }
        }
        assembly.drop_unused_packed();
        Ok(())
    })?;
    Ok(json!({ "written": written }))
}

fn file_exists(session: &Session, resolved: &std::path::Path) -> bool {
    if session.writes_to_memory() {
        return session.memory_file(resolved).is_some();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        resolved.exists()
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

/// Rewrites the active assembly's source paths for a save into the
/// folder `new_base` (as [`asm::join`] writes it), so they stay relative
/// to its file. Not an undo step of its own: part of the save.
pub(crate) fn rebased(doc: &Document, old_base: &str, new_base: &str) -> Option<Document> {
    let assembly = doc.assembly.as_ref()?;
    if old_base == new_base {
        return None;
    }
    let mut doc = doc.clone();
    let mut changed = false;
    if let Some(a) = doc.assembly.as_mut() {
        for (instance, before) in a.instances.iter_mut().zip(&assembly.instances) {
            if let (Source::File { path, .. }, Source::File { path: old, .. }) =
                (&mut instance.source, &before.source)
                && let Ok(full) = asm::join(old_base, old)
            {
                let new = asm::relative_to(new_base, &full);
                if new != *path && craft3d_doc::assembly::check_source_path(&new).is_ok() {
                    *path = new;
                    changed = true;
                }
            }
        }
    }
    changed.then_some(doc)
}
