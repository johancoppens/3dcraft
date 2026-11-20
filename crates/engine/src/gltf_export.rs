//! `file.export` to glTF 2.0 binary (`.glb`), for showing a model in a
//! browser, a game engine or a renderer: every body a node of the scene
//! named after the feature that made it, every name of its faces a group of
//! triangles that says which faces it holds, a material per look, in
//! metres with +Y up (docs/file-formats.md › glTF). The command itself is
//! in `commands.rs`; the face groups are made by `craft3d_io::gltf`, the
//! file by `craft3d_meshio::gltf`.

use crate::commands::{Run, appearance_name, export_tess, fail, length_unit, mesh_unit, meshes};
use crate::params::Params;
use crate::{CommandError, Session};
use craft3d_doc::Appearance;
use craft3d_io::gltf::GltfBody;
use craft3d_meshio::gltf::{Extra, PbrMaterial, Scene};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const ID: &str = "file.export";

/// Whether `file.export` writes glTF: `format` `glb`, or without it a
/// `.glb` name (any case). glTF's text form (`.gltf`, its data in files
/// beside it) is not written: asked for, it is refused with the name to use.
pub(crate) fn wanted(format: Option<&str>, path: &str) -> Result<bool, CommandError> {
    let text_form = || {
        fail(format!(
            "{ID}: glTF is written as one binary file (.glb, its JSON and its data together), not as .gltf with its data beside it; use a .glb name or `format` glb"
        ))
    };
    if let Some(format) = format {
        return match format {
            "glb" => Ok(true),
            "gltf" => text_form(),
            _ => Ok(false),
        };
    }
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("glb") => Ok(true),
        Some("gltf") => text_form(),
        _ => Ok(false),
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

/// The file's materials, one per look and side, and what of the looks
/// was left out, with the bodies that had it.
#[derive(Default)]
struct Looks {
    keys: Vec<(Appearance, bool)>,
    materials: Vec<PbrMaterial>,
    left_out: BTreeMap<&'static str, BTreeSet<String>>,
}

impl Looks {
    /// The index of the material of `appearance` on `body`, added when it
    /// is not there yet.
    fn index(&mut self, appearance: &Appearance, double_sided: bool, body: &str) -> u32 {
        for layer in left_out(appearance) {
            self.left_out
                .entry(layer)
                .or_default()
                .insert(body.to_string());
        }
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
}

/// Writes the glTF file at `path` (resolved for writing here) from every
/// body, or from those `bodies` names, at the quality asked for.
pub(crate) fn export(session: &mut Session, p: &Params, path: &str) -> Run {
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
    let (mut triangles, mut closed, mut groups) = (0_usize, 0_usize, 0_usize);
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
        let own = looks.index(&body.appearance.unwrap_or(plain), !solid, name);
        let faces: Vec<Option<u32>> = body
            .faces
            .iter()
            .map(|a| a.map(|a| looks.index(&a, !solid, name)))
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
        })
        .map_err(|e| CommandError::new(format!("{ID}: {name}: {e}")))?;
        triangles += node.groups.iter().map(|g| g.triangles.len()).sum::<usize>();
        groups += node.groups.len();
        nodes.push(node);
    }
    let scene = Scene {
        generator: format!("3DCraft {}", crate::report::VERSION),
        unit,
        nodes,
        materials: looks.materials,
    };
    let bytes = craft3d_meshio::gltf::write_glb(&scene)
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let size = bytes.len();
    session.write_resolved(&resolved, bytes)?;
    let left: Vec<Value> = looks
        .left_out
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
        "faceGroups": groups,
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
    written.report(&mut out);
    Ok(out)
}
