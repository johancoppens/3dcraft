//! `file.export` to glTF 2.0 binary (`.glb`), for showing a model in a
//! browser, a game engine or a renderer: every body a node of the scene
//! named after the feature that made it, every name of its faces a group of
//! triangles that says which faces it holds (or, with `primitives` looks,
//! every look of it one group, for viewers), a material per look, in metres
//! with +Y up, the document's unit and the turn back to it in
//! `asset.extras` (docs/file-formats.md › glTF). The command itself is in
//! `commands.rs`; the face groups are made by `craft3d_io::gltf`, the file
//! by `craft3d_meshio::gltf`.

use crate::commands::{Run, appearance_name, export_tess, fail, length_unit, mesh_unit, meshes};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_doc::Appearance;
use craft3d_io::gltf::{GltfBody, Primitives};
use craft3d_meshio::gltf::{Extra, Node, PbrMaterial, Scene};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const ID: &str = "file.export";

/// Whether `file.export` writes glTF: `format` `glb`, or without it a
/// `.glb` name (any case). glTF's text form (`.gltf`, its data in files
/// beside it) is not written: asked for by its name or its format, or a
/// `.gltf` name with `format` glb (GLB bytes under the text form's name),
/// it is refused with the name to use.
pub(crate) fn wanted(format: Option<&str>, path: &str) -> Result<bool, CommandError> {
    let text_form = || {
        fail(format!(
            "{ID}: glTF is written as one binary file (.glb, its JSON and its data together), not as .gltf with its data beside it; use a .glb name or `format` glb"
        ))
    };
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let text_name = extension.as_deref() == Some("gltf");
    match format {
        Some("glb" | "gltf") if text_name => text_form(),
        Some("glb") => Ok(true),
        Some("gltf") => text_form(),
        Some(_) => Ok(false),
        None if text_name => text_form(),
        None => Ok(extension.as_deref() == Some("glb")),
    }
}

/// The layers of a look glTF's core and the extensions written cannot
/// hold, by name: anisotropy (its direction needs tangents the mesh does
/// not have), subsurface scattering (no ratified extension), a solid
/// texture (`pattern`; the base colour is its mean, as the commands set
/// it) and, for a body that lets light through and is not thin-walled or
/// tints it, `volume` (the refraction through its thickness and the
/// absorption, which `KHR_materials_volume` takes with a thickness the
/// mesh does not give: glTF draws it as a thin sheet).
fn left_out(appearance: &Appearance) -> Vec<&'static str> {
    let o = &appearance.optics;
    let mut out = Vec::new();
    if o.anisotropy != 0.0 {
        out.push("anisotropy");
    }
    if o.subsurface > 0.0 {
        out.push("subsurface");
    }
    if o.pattern.is_some() {
        out.push("pattern");
    }
    if o.transmission > 0.0 && (!o.thin_walled || o.absorption.is_some()) {
        out.push("volume");
    }
    out
}

/// The glTF material of a look: its colour, metallic (0 without one, a
/// dielectric) and roughness (the pictures' 0.5 without one), the clear
/// coat, and for a body that lets light through the transmission and the
/// index of refraction; both sides drawn for an open mesh.
fn material(appearance: &Appearance, double_sided: bool) -> PbrMaterial {
    let o = &appearance.optics;
    let mut name = appearance_name(appearance);
    if double_sided {
        name.push_str(" double-sided");
    }
    let lets_through = o.transmission > 0.0;
    PbrMaterial {
        name,
        color: appearance.color,
        metallic: appearance.metallic.unwrap_or(0.0),
        roughness: appearance
            .roughness
            .unwrap_or(craft3d_render::studio::DEFAULT_ROUGHNESS),
        double_sided,
        clearcoat: (o.clearcoat > 0.0).then_some((o.clearcoat, o.clearcoat_roughness)),
        transmission: lets_through.then_some(o.transmission),
        ior: lets_through.then_some(o.ior),
    }
}

/// What of the looks in a file glTF cannot hold: each layer, with the
/// bodies that have it.
type LeftOut = BTreeMap<&'static str, BTreeSet<String>>;

/// The file's materials, one per look and side, in the order asked for.
#[derive(Default)]
struct Looks {
    keys: Vec<(Appearance, bool)>,
    materials: Vec<PbrMaterial>,
}

impl Looks {
    /// The index of the material of `appearance`, added when it is not
    /// there yet.
    fn index(&mut self, appearance: &Appearance, double_sided: bool) -> u32 {
        let key = (*appearance, double_sided);
        let at = match self.keys.iter().position(|k| *k == key) {
            Some(at) => at,
            None => {
                self.keys.push(key);
                self.materials.push(material(appearance, double_sided));
                self.materials.len().saturating_sub(1)
            }
        };
        // Past u32 the writer refuses it as a material that does not exist.
        u32::try_from(at).unwrap_or(u32::MAX)
    }

    /// The materials a primitive of `nodes` uses, in their order, the
    /// primitives' indices renumbered to them, and what of those looks
    /// glTF cannot hold, with the bodies (nodes) whose primitives use them.
    /// A material no primitive uses is not written (a body without faces,
    /// a face all of whose triangles have no area, a body's own look where
    /// every face has one of its own), so the file has no object nothing
    /// refers to, and `leftOut` names only what is in it.
    fn used(self, nodes: &mut [Node]) -> (Vec<PbrMaterial>, LeftOut) {
        let mut users: Vec<BTreeSet<String>> = vec![BTreeSet::new(); self.materials.len()];
        for node in nodes.iter() {
            for material in node.groups.iter().filter_map(|g| g.material) {
                if let Some(users) = users.get_mut(material as usize) {
                    users.insert(node.name.clone());
                }
            }
        }
        let mut renumbered: Vec<Option<u32>> = vec![None; self.materials.len()];
        let mut kept = Vec::new();
        let mut left = LeftOut::new();
        for (((material, (appearance, _)), users), slot) in self
            .materials
            .into_iter()
            .zip(self.keys)
            .zip(users)
            .zip(renumbered.iter_mut())
        {
            if users.is_empty() {
                continue;
            }
            for layer in left_out(&appearance) {
                left.entry(layer).or_default().extend(users.iter().cloned());
            }
            *slot = u32::try_from(kept.len()).ok();
            kept.push(material);
        }
        for group in nodes.iter_mut().flat_map(|n| n.groups.iter_mut()) {
            group.material = group
                .material
                .and_then(|m| renumbered.get(m as usize).copied().flatten());
        }
        (kept, left)
    }
}

/// Writes the glTF file at `path` (resolved for writing here) from every
/// body, or from those `bodies` names, at the quality asked for, with a
/// primitive per face group or per look (`primitives`).
pub(crate) fn export(session: &mut Session, p: &Params, path: &str) -> Run {
    let primitives = match p.str("primitives")? {
        None => Primitives::Faces,
        Some(name) => Primitives::from_name(name).ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: unknown primitives `{}`; use faces (a primitive per name of a body's faces, the names in its extras) or looks (a primitive per look of a body: far fewer draw calls for a viewer, and no face names in the file)",
                name.chars().take(40).collect::<String>()
            ))
        })?,
    };
    let resolved = session.workspace.resolve_write(path)?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let written = crate::export_bodies::written(
        p,
        &open.bodies_doc(),
        &regen,
        open.history.current().is_assembly(),
    )?;
    let unit = mesh_unit(open.history.current().units.length);
    let document_unit = length_unit(open.history.current());
    // For showing, not printing: the automatic tolerance, never held to
    // what a printed curve needs.
    let (tess, quality, degrees) = export_tess(p, &regen, None)?;
    let bodies = meshes(open, ID, &tess, written.only.as_deref())?;
    // What has no look of its own is drawn as the pictures draw it, not
    // in glTF's default material (white metal).
    let plain = Appearance::color(craft3d_render::RenderParams::default().color);
    let mut looks = Looks::default();
    let mut nodes = Vec::with_capacity(bodies.len());
    let mut closed = 0_usize;
    // Points that fall on others in the file's 32-bit floats, and what is
    // said of each body where they do.
    let (mut merged_points, mut folded_triangles) = (0_usize, 0_usize);
    let mut warnings: Vec<String> = Vec::new();
    for body in &bodies {
        let name = &body.name;
        // An open mesh (a sheet) shows its inside: both sides drawn.
        let solid = body
            .mesh
            .weld()
            .map_err(|t| {
                CommandError::new(format!(
                    "{ID}: {name}: the mesh is inconsistent at triangle {t}"
                ))
            })?
            .is_closed();
        if solid {
            closed += 1;
        }
        // The body's own look, for the faces without one of their own.
        let own = looks.index(&body.appearance.unwrap_or(plain), !solid);
        let faces: Vec<Option<u32>> = body
            .faces
            .iter()
            .map(|a| a.map(|a| looks.index(&a, !solid)))
            .collect();
        let face_names: Vec<String> = regen
            .feature(body.id)
            .and_then(|f| f.names.as_ref())
            .map(|names| names.faces.iter().map(ToString::to_string).collect())
            .unwrap_or_default();
        let node = craft3d_io::gltf::node(GltfBody {
            name,
            mesh: &body.mesh,
            face_names: &face_names,
            face_materials: &faces,
            material: Some(own),
            extras: vec![("feature".to_string(), Extra::Text(body.id.to_string()))],
            primitives,
        })
        .map_err(|e| CommandError::new(format!("{ID}: {name}: {e}")))?;
        let (merged, folded) = craft3d_meshio::gltf::points_in_32_bits(&node, unit);
        if merged > 0 {
            merged_points += merged;
            folded_triangles += folded;
            warnings.push(format!(
                "{name}: in the file's 32-bit floats, in metres, {merged} of its points fall on others and {folded} of its triangles fold to a line (no area): glTF holds positions so; 3MF and OBJ write every point as the mesh has it, or move the model nearer the origin"
            ));
        }
        nodes.push(node);
    }
    let (materials, left) = looks.used(&mut nodes);
    let triangles: usize = nodes
        .iter()
        .flat_map(|n| &n.groups)
        .map(|g| g.triangles.len())
        .sum();
    let draw_calls: usize = nodes.iter().map(|n| n.groups.len()).sum();
    let scene = Scene {
        generator: format!("3DCraft {}", crate::report::VERSION),
        // What a file alone needs to give the document's coordinates back:
        // the document's (x, y, z) are the file's (X, −Z, Y) over `scale`,
        // in `documentUnit`.
        asset_extras: vec![
            (
                "documentUnit".to_string(),
                Extra::Text(document_unit.to_string()),
            ),
            ("scale".to_string(), Extra::Real(unit.metres())),
            (
                "documentAxes".to_string(),
                Extra::Texts(vec!["X".to_string(), "-Z".to_string(), "Y".to_string()]),
            ),
        ],
        unit,
        nodes,
        materials,
    };
    let bytes = craft3d_meshio::gltf::write_glb(&scene)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let size = bytes.len();
    session.write_resolved(&resolved, bytes)?;
    let left: Vec<Value> = left
        .iter()
        .map(|(layer, bodies)| json!({"layer": layer, "bodies": bodies}))
        .collect();
    let mut out = json!({
        "path": path,
        "bytes": size,
        "bodies": scene.nodes.len(),
        // How many of the meshes are closed and consistently oriented.
        "closed": closed,
        "triangles": triangles,
        // A primitive per face group (the face names in the file) or per
        // look; each primitive is a draw call of a viewer.
        "primitives": primitives.name(),
        "drawCalls": draw_calls,
        "faceNames": primitives == Primitives::Faces,
        "materials": scene.materials.len(),
        "extensions": craft3d_meshio::gltf::extensions_used(&scene.materials),
        "leftOut": left,
        "tolerance": tess.chordal_tolerance,
        "angle": degrees,
        "quality": quality,
        // glTF's lengths are metres and its up axis +Y: the document's
        // unit times `scale`, its +Z turned to +Y.
        "unit": "m",
        "documentUnit": document_unit,
        "scale": unit.metres(),
        "upAxis": "+y",
    });
    if let Some(map) = out.as_object_mut() {
        if primitives == Primitives::Faces {
            map.insert("faceGroups".into(), draw_calls.into());
        }
        if !warnings.is_empty() {
            map.insert("mergedPoints".into(), merged_points.into());
            map.insert("foldedTriangles".into(), folded_triangles.into());
            map.insert("warnings".into(), warnings.into());
        }
    }
    written.report(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_meshio::gltf::Group;

    fn group(material: u32) -> Group {
        Group {
            material: Some(material),
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![[0, 1, 2]],
            ..Group::default()
        }
    }

    #[test]
    fn a_material_no_primitive_uses_is_not_written_and_not_reported() {
        let mut looks = Looks::default();
        let brushed = Appearance {
            optics: craft3d_doc::Optics {
                anisotropy: 0.5,
                ..craft3d_doc::Optics::default()
            },
            ..Appearance::color([1, 2, 3])
        };
        let glassy = Appearance {
            optics: craft3d_doc::Optics {
                subsurface: 0.5,
                ..craft3d_doc::Optics::default()
            },
            ..Appearance::color([4, 5, 6])
        };
        // A body's own look nothing uses (every face has one), the brushed
        // look of a body without triangles, and the waxy look in use.
        let unused = looks.index(&Appearance::color([9, 9, 9]), false);
        let brushed = looks.index(&brushed, false);
        let used = looks.index(&glassy, false);
        assert_eq!((unused, brushed, used), (0, 1, 2));
        let mut nodes = vec![
            Node {
                name: "Empty".into(),
                ..Node::default()
            },
            Node {
                name: "Waxy".into(),
                groups: vec![group(used), group(used)],
                ..Node::default()
            },
        ];
        let (materials, left) = looks.used(&mut nodes);
        assert_eq!(materials.len(), 1);
        assert_eq!(materials[0].color, [4, 5, 6]);
        assert!(nodes[1].groups.iter().all(|g| g.material == Some(0)));
        let left: Vec<(&str, Vec<String>)> = left
            .into_iter()
            .map(|(layer, bodies)| (layer, bodies.into_iter().collect()))
            .collect();
        assert_eq!(left, [("subsurface", vec!["Waxy".to_string()])]);
    }

    #[test]
    fn a_gltf_name_is_refused_whatever_the_format_says() {
        assert_eq!(wanted(None, "a.GLB").ok(), Some(true));
        assert_eq!(wanted(Some("glb"), "a.bin").ok(), Some(true));
        assert_eq!(wanted(None, "a.stl").ok(), Some(false));
        assert_eq!(wanted(Some("stl"), "a.glb").ok(), Some(false));
        for (format, path) in [
            (None, "a.gltf"),
            (Some("gltf"), "a.glb"),
            (Some("glb"), "a.GLTF"),
        ] {
            let refused = wanted(format, path).unwrap_err().to_string();
            assert!(
                refused.contains("one binary file (.glb"),
                "{format:?} {path}: {refused}"
            );
        }
    }
}
