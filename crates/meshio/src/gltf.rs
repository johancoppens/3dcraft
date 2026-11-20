//! glTF 2.0 binary (`.glb`) writer: one file with a JSON chunk and a binary
//! chunk, the format browsers, game engines and renderers load. Triangles
//! for showing a model, as STL is for printing one; not for making it.
//!
//! Written from the Khronos glTF 2.0 specification alone (no glTF library,
//! in the code or in its tests): the asset, the scene, nodes, meshes and
//! their primitives, accessors, buffer views and the buffer, materials with
//! `pbrMetallicRoughness`, `extras` for application data, the rules for
//! data alignment, and the GLB container (a 12-byte header, then a JSON
//! chunk padded with spaces and a binary chunk padded with zeros, every
//! number a little-endian `u32`); the ratified material extensions
//! `KHR_materials_clearcoat`, `KHR_materials_ior` and
//! `KHR_materials_transmission`; RFC 8259 for the JSON text.
//!
//! What the writer settles, the same for every caller:
//!
//! - **Metres, +Y up.** glTF's lengths are metres and its up axis is +Y,
//!   its front facing +Z. Positions are given in [`Scene::unit`] with +Z
//!   up, as 3DCraft holds a model; each is scaled to metres and turned a
//!   quarter turn about +X, `(x, y, z)` to `(x, z, −y)`, in the coordinates
//!   themselves, so a reader that ignores node transforms still gets the
//!   model upright and to scale, and the nodes carry none. A turn, not a
//!   mirror: triangles keep their winding, and the model's front (what is
//!   seen from −Y) faces glTF's +Z.
//! - **One node per [`Node`], one primitive per [`Group`].** A node has a
//!   mesh of the same name whose primitives are its groups, in order; a
//!   group without triangles is left out, and a node without any has no
//!   mesh. `extras` are written where they are given.
//! - **Every primitive has its own accessors:** `POSITION` (32-bit floats,
//!   with the `min` and `max` the specification asks for, exactly the
//!   written values), `NORMAL` (unit vectors) and the indices, as `u16` when
//!   the group has at most 65,535 vertices and else `u32` (the largest value
//!   of the type, the primitive restart value, is never used). Three buffer
//!   views hold them all: the positions, the normals (both with a stride of
//!   12 bytes, as views that several accessors share must state) and the
//!   indices, each accessor at a multiple of 4 bytes.
//! - **Materials are metallic-roughness:** the base colour given as sRGB
//!   is written linear, as `baseColorFactor` is (the sRGB transfer function
//!   of IEC 61966-2-1), with alpha 1; the clear coat, transmission and index
//!   of refraction go in their ratified extensions, listed in
//!   `extensionsUsed` and never in `extensionsRequired`, so a reader without
//!   them still shows the core material.
//! - **Numbers** are the shortest decimal that reads back as the same
//!   `f64`, without an exponent and never `-0`; a 32-bit float is written
//!   as the `f64` it widens to, so `min` and `max` read back bit for bit.
//!   The same scene gives the same bytes.

use crate::{MeshError, Unit};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A value in an object's `extras`, the place glTF gives application data.
#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    Text(String),
    Number(u64),
    /// Written as the shortest decimal that reads back as the same `f64`;
    /// `null` when it is not finite (JSON has no such number).
    Real(f64),
    Texts(Vec<String>),
    Numbers(Vec<u64>),
}

/// Application data of a node or a group, written as a JSON object with
/// the keys in this order (a key met a second time is left out).
pub type Extras = Vec<(String, Extra)>;

/// Triangles drawn with one material: one primitive of its node's mesh.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    pub extras: Extras,
    /// An index into [`Scene::materials`]; `None`: glTF's default material.
    pub material: Option<u32>,
    /// In [`Scene::unit`], right-handed with +Z up.
    pub positions: Vec<[f64; 3]>,
    /// One per position, or none at all. A vector that is zero or not
    /// finite means "none here": the mean of the unit normals of the
    /// group's triangles around the position is written instead (+Z of the
    /// model for a position on no triangle with an area).
    pub normals: Vec<[f64; 3]>,
    /// Counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
}

/// A node, with a mesh whose primitives are its groups.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Node {
    pub name: String,
    pub extras: Extras,
    pub groups: Vec<Group>,
}

/// A metallic-roughness material.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PbrMaterial {
    pub name: String,
    /// sRGB, as 3DCraft keeps colours; written linear.
    pub color: [u8; 3],
    /// 0 to 1.
    pub metallic: f64,
    /// 0 to 1.
    pub roughness: f64,
    /// Both sides drawn: for an open surface, whose inside can be seen.
    pub double_sided: bool,
    /// `KHR_materials_clearcoat`: its strength and roughness, 0 to 1.
    pub clearcoat: Option<(f64, f64)>,
    /// `KHR_materials_transmission`: the share of light let through, 0 to 1.
    pub transmission: Option<f64>,
    /// `KHR_materials_ior`: the index of refraction, 1 or more.
    pub ior: Option<f64>,
}

/// What a glTF file holds.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Scene {
    /// `asset.generator`: the program that wrote the file, and its version.
    pub generator: String,
    /// `asset.extras`: what the file as a whole says beyond glTF (written
    /// only when there is something).
    pub asset_extras: Extras,
    /// The unit of the positions, which are written in metres.
    pub unit: Unit,
    pub nodes: Vec<Node>,
    pub materials: Vec<PbrMaterial>,
}

/// The extensions a material can use, by name.
pub const KHR_MATERIALS_CLEARCOAT: &str = "KHR_materials_clearcoat";
pub const KHR_MATERIALS_IOR: &str = "KHR_materials_ior";
pub const KHR_MATERIALS_TRANSMISSION: &str = "KHR_materials_transmission";

/// `componentType` and `target` codes of the specification.
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// The GLB header's magic (`glTF`) and version, and the chunk types
/// (`JSON`, `BIN\0`), as little-endian words.
const MAGIC: u32 = 0x4654_6C67;
const VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

const TOO_LARGE: MeshError =
    MeshError::TooLarge("a GLB file of 4 GiB or more (its header counts bytes in 32 bits)");

/// A byte of an sRGB colour as linear light, 0 to 1: the sRGB transfer
/// function (IEC 61966-2-1), since glTF's `baseColorFactor` is linear.
pub fn srgb_to_linear(byte: u8) -> f64 {
    let c = f64::from(byte) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        libm::pow((c + 0.055) / 1.055, 2.4)
    }
}

/// The length of a GLB file with a JSON chunk of `json` bytes and a binary
/// chunk of `bin` bytes (none when 0), each padded to 4 bytes: an error
/// when the header's 32-bit length cannot hold it.
pub fn glb_length(json: u64, bin: u64) -> Result<u32, MeshError> {
    let padded = |n: u64| n.checked_next_multiple_of(4);
    let json = padded(json).ok_or(TOO_LARGE)?;
    let bin = match bin {
        0 => 0,
        n => padded(n).and_then(|n| n.checked_add(8)).ok_or(TOO_LARGE)?,
    };
    let total = json
        .checked_add(20)
        .and_then(|n| n.checked_add(bin))
        .ok_or(TOO_LARGE)?;
    u32::try_from(total).map_err(|_| TOO_LARGE)
}

/// A group to be written: where it is in the scene and how wide its
/// indices are.
struct Planned<'a> {
    object: usize,
    group: &'a Group,
    wide: bool,
}

/// Where a group's data went in the binary chunk (bytes from the start of
/// each buffer view) and the box of its written positions.
struct Placed {
    positions: usize,
    normals: usize,
    indices: usize,
    min: [f32; 3],
    max: [f32; 3],
}

/// Writes `scene` as a GLB file (see the module's documentation).
///
/// Refused, naming the node and group or the material: a position that is
/// not finite or that lies farther from the origin in metres than a 32-bit
/// float holds; a triangle that refers to a vertex that does not exist; a
/// list of normals that is not one per position; a material index past
/// [`Scene::materials`]; a material number that is not finite or out of
/// its range; and a file of 4 GiB or more, checked before the binary
/// chunk is made.
pub fn write_glb(scene: &Scene) -> Result<Vec<u8>, MeshError> {
    check_materials(&scene.materials)?;
    let scale = scene.unit.metres();
    let mut planned: Vec<Planned> = Vec::new();
    let mut vertices: u64 = 0;
    let mut index_bytes: u64 = 0;
    for (object, node) in scene.nodes.iter().enumerate() {
        for (index, group) in node.groups.iter().enumerate() {
            if group.triangles.is_empty() {
                continue;
            }
            let wide = check_group(scene.materials.len(), scale, object, index, group)?;
            let width: u64 = if wide { 4 } else { 2 };
            let bytes = u64::try_from(group.triangles.len())
                .ok()
                .and_then(|t| t.checked_mul(3 * width))
                .and_then(|b| b.checked_next_multiple_of(4))
                .ok_or(TOO_LARGE)?;
            vertices = u64::try_from(group.positions.len())
                .ok()
                .and_then(|v| vertices.checked_add(v))
                .ok_or(TOO_LARGE)?;
            index_bytes = index_bytes.checked_add(bytes).ok_or(TOO_LARGE)?;
            planned.push(Planned {
                object,
                group,
                wide,
            });
        }
    }
    // The binary chunk's size, known before anything is made: a file too
    // large for the header is refused without the memory for it.
    let binary = vertices
        .checked_mul(24)
        .and_then(|v| v.checked_add(index_bytes))
        .ok_or(TOO_LARGE)?;
    glb_length(0, binary)?;
    let capacity = usize::try_from(binary).map_err(|_| TOO_LARGE)?;
    let mut bin: Vec<u8> = Vec::with_capacity(capacity);
    let mut placed: Vec<Placed> = Vec::with_capacity(planned.len());
    for plan in &planned {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let at = bin.len();
        for &p in &plan.group.positions {
            let q = to_gltf(p, scale);
            for ((lo, hi), c) in min.iter_mut().zip(max.iter_mut()).zip(q) {
                *lo = lo.min(c);
                *hi = hi.max(c);
            }
            for c in q {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        placed.push(Placed {
            positions: at,
            normals: 0,
            indices: 0,
            min,
            max,
        });
    }
    let normals_view = bin.len();
    for (plan, place) in planned.iter().zip(placed.iter_mut()) {
        place.normals = bin.len() - normals_view;
        for n in normals_of(plan.group) {
            for c in n {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    let indices_view = bin.len();
    for (plan, place) in planned.iter().zip(placed.iter_mut()) {
        place.indices = bin.len() - indices_view;
        for k in plan.group.triangles.iter().flatten() {
            if plan.wide {
                bin.extend_from_slice(&k.to_le_bytes());
            } else {
                // Below 65,535: the group has at most that many vertices,
                // and every index was checked against their count.
                bin.extend_from_slice(&(*k as u16).to_le_bytes());
            }
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
    }
    let json = json_text(
        scene,
        &planned,
        &placed,
        normals_view,
        indices_view,
        bin.len(),
    );
    glb(json, bin)
}

/// The JSON chunk's text.
fn json_text(
    scene: &Scene,
    planned: &[Planned],
    placed: &[Placed],
    normals_view: usize,
    indices_view: usize,
    total: usize,
) -> String {
    let mut out = String::new();
    out.push_str("{\"asset\":{\"version\":\"2.0\",\"generator\":");
    push_string(&mut out, &scene.generator);
    if !scene.asset_extras.is_empty() {
        out.push_str(",\"extras\":");
        push_extras(&mut out, &scene.asset_extras);
    }
    out.push('}');
    let used = extensions_used(&scene.materials);
    if !used.is_empty() {
        out.push_str(",\"extensionsUsed\":[");
        for (i, name) in used.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_string(&mut out, name);
        }
        out.push(']');
    }
    out.push_str(",\"scene\":0,\"scenes\":[{");
    if !scene.nodes.is_empty() {
        out.push_str("\"nodes\":[");
        for i in 0..scene.nodes.len() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "{i}");
        }
        out.push(']');
    }
    out.push_str("}]");
    // Which nodes have a mesh: those with a group written, in node order.
    let mut has_mesh = vec![false; scene.nodes.len()];
    for plan in planned {
        if let Some(slot) = has_mesh.get_mut(plan.object) {
            *slot = true;
        }
    }
    if !scene.nodes.is_empty() {
        out.push_str(",\"nodes\":[");
        let mut mesh = 0_usize;
        for (object, node) in scene.nodes.iter().enumerate() {
            if object > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            push_string(&mut out, &node.name);
            if has_mesh.get(object).copied().unwrap_or(false) {
                let _ = write!(out, ",\"mesh\":{mesh}");
                mesh += 1;
            }
            if !node.extras.is_empty() {
                out.push_str(",\"extras\":");
                push_extras(&mut out, &node.extras);
            }
            out.push('}');
        }
        out.push(']');
    }
    if !planned.is_empty() {
        out.push_str(",\"meshes\":[");
        let mut accessor = 0_usize;
        for (m, groups) in planned.chunk_by(|a, b| a.object == b.object).enumerate() {
            if m > 0 {
                out.push(',');
            }
            let name = groups
                .first()
                .and_then(|g| scene.nodes.get(g.object))
                .map_or("", |n| n.name.as_str());
            out.push_str("{\"name\":");
            push_string(&mut out, name);
            out.push_str(",\"primitives\":[");
            for (p, plan) in groups.iter().enumerate() {
                if p > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{{\"attributes\":{{\"POSITION\":{},\"NORMAL\":{}}},\"indices\":{}",
                    accessor,
                    accessor + 1,
                    accessor + 2
                );
                accessor += 3;
                if let Some(material) = plan.group.material {
                    let _ = write!(out, ",\"material\":{material}");
                }
                if !plan.group.extras.is_empty() {
                    out.push_str(",\"extras\":");
                    push_extras(&mut out, &plan.group.extras);
                }
                out.push('}');
            }
            out.push_str("]}");
        }
        out.push(']');
    }
    if !scene.materials.is_empty() {
        out.push_str(",\"materials\":[");
        for (i, material) in scene.materials.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_material(&mut out, material);
        }
        out.push(']');
    }
    if !planned.is_empty() {
        out.push_str(",\"accessors\":[");
        for (i, (plan, place)) in planned.iter().zip(placed).enumerate() {
            if i > 0 {
                out.push(',');
            }
            let count = plan.group.positions.len();
            let _ = write!(
                out,
                "{{\"bufferView\":0,\"byteOffset\":{},\"componentType\":{FLOAT},\"count\":{count},\"type\":\"VEC3\",\"min\":",
                place.positions
            );
            push_numbers(&mut out, place.min.map(f64::from));
            out.push_str(",\"max\":");
            push_numbers(&mut out, place.max.map(f64::from));
            let _ = write!(
                out,
                "}},{{\"bufferView\":1,\"byteOffset\":{},\"componentType\":{FLOAT},\"count\":{count},\"type\":\"VEC3\"}}",
                place.normals
            );
            let kind = if plan.wide {
                UNSIGNED_INT
            } else {
                UNSIGNED_SHORT
            };
            let _ = write!(
                out,
                ",{{\"bufferView\":2,\"byteOffset\":{},\"componentType\":{kind},\"count\":{},\"type\":\"SCALAR\"}}",
                place.indices,
                plan.group.triangles.len().saturating_mul(3)
            );
        }
        out.push(']');
        let _ = write!(
            out,
            ",\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{normals_view},\"byteStride\":12,\"target\":{ARRAY_BUFFER}}},\
             {{\"buffer\":0,\"byteOffset\":{normals_view},\"byteLength\":{},\"byteStride\":12,\"target\":{ARRAY_BUFFER}}},\
             {{\"buffer\":0,\"byteOffset\":{indices_view},\"byteLength\":{},\"target\":{ELEMENT_ARRAY_BUFFER}}}]",
            indices_view - normals_view,
            total - indices_view
        );
        let _ = write!(out, ",\"buffers\":[{{\"byteLength\":{total}}}]");
    }
    out.push('}');
    out
}

/// The GLB container: the header, the JSON chunk padded with spaces and,
/// when there is one, the binary chunk padded with zeros.
fn glb(json: String, mut bin: Vec<u8>) -> Result<Vec<u8>, MeshError> {
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let size = |n: usize| u64::try_from(n).map_err(|_| TOO_LARGE);
    let total = glb_length(size(json.len())?, size(bin.len())?)?;
    let json_length = u32::try_from(json.len()).map_err(|_| TOO_LARGE)?;
    let bin_length = u32::try_from(bin.len()).map_err(|_| TOO_LARGE)?;
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    for word in [MAGIC, VERSION, total, json_length, CHUNK_JSON] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&bin_length.to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

/// Checks a group that has triangles; true when its indices need 32 bits.
fn check_group(
    materials: usize,
    scale: f64,
    object: usize,
    group_index: usize,
    group: &Group,
) -> Result<bool, MeshError> {
    let bad = |what: String| MeshError::BadGroup {
        object,
        group: group_index,
        what,
    };
    let count = group.positions.len();
    if !group.normals.is_empty() && group.normals.len() != count {
        return Err(bad(format!(
            "it has {} normals for {count} positions: give one per position, or none",
            group.normals.len()
        )));
    }
    if group.material.is_some_and(|m| m as usize >= materials) {
        return Err(MeshError::BadMaterial { object });
    }
    for (t, triangle) in group.triangles.iter().enumerate() {
        if let Some(k) = triangle.iter().find(|&&k| k as usize >= count) {
            return Err(bad(format!(
                "triangle {t} refers to vertex {k}, and there are {count}"
            )));
        }
    }
    let limit = f64::from(f32::MAX);
    for (vertex, p) in group.positions.iter().enumerate() {
        if p.iter().any(|c| !c.is_finite()) {
            return Err(bad(format!(
                "vertex {vertex} is not finite (NaN or infinity)"
            )));
        }
        // Finite, so its product is finite or infinite, never NaN.
        if p.iter().any(|c| (c * scale).abs() > limit) {
            return Err(bad(format!(
                "vertex {vertex} lies farther from the origin, in metres, than a 32-bit float holds"
            )));
        }
    }
    if u32::try_from(count).is_err() {
        return Err(MeshError::TooLarge(
            "more than 2^32 - 1 vertices in one group of a glTF file",
        ));
    }
    // 65,535 in 16 bits is the primitive restart value, never an index:
    // 16 bits hold the indices of up to 65,535 vertices.
    Ok(count > usize::from(u16::MAX))
}

/// Every material number finite and in its range.
fn check_materials(materials: &[PbrMaterial]) -> Result<(), MeshError> {
    let unit = |v: f64| (0.0..=1.0).contains(&v);
    for (material, m) in materials.iter().enumerate() {
        let what = if !unit(m.metallic) {
            "the metallic factor must be from 0 to 1"
        } else if !unit(m.roughness) {
            "the roughness factor must be from 0 to 1"
        } else if m.clearcoat.is_some_and(|(c, r)| !(unit(c) && unit(r))) {
            "the clear coat's strength and roughness must be from 0 to 1"
        } else if m.transmission.is_some_and(|t| !unit(t)) {
            "the transmission must be from 0 to 1"
        } else if m.ior.is_some_and(|i| !(i.is_finite() && i >= 1.0)) {
            "the index of refraction must be a number of 1 or more"
        } else {
            continue;
        };
        return Err(MeshError::BadMaterialValue { material, what });
    }
    Ok(())
}

/// A position in glTF's axes and metres, as written: `(x, z, −y)` times
/// the scale, in 32 bits, `-0` as `0`. In range: checked before.
fn to_gltf(p: [f64; 3], scale: f64) -> [f32; 3] {
    let [x, y, z] = p.map(|c| c * scale);
    [x as f32 + 0.0, z as f32 + 0.0, -y as f32 + 0.0]
}

/// What a node's points become in the file's 32-bit floats (in metres):
/// how many of its points fall on another point (`merged`; a point is told
/// by its exact position, so the copies of one point on several faces are
/// one point) and how many of its triangles get two corners on one place
/// there and so fold to a line or a point (`folded`). Both 0 for every
/// model of ordinary size near the origin; a model far from it, or
/// thousands of metres across with millimetre detail, loses that much in
/// glTF. Points that are not finite (which [`write_glb`] refuses) are left
/// out of the count.
pub fn points_in_32_bits(node: &Node, unit: Unit) -> (usize, usize) {
    let scale = unit.metres();
    let exact = |p: [f64; 3]| p.map(|c| (c + 0.0).to_bits());
    let written = |p: [f64; 3]| to_gltf(p, scale).map(f32::to_bits);
    let mut points: BTreeMap<[u64; 3], [u32; 3]> = BTreeMap::new();
    for group in &node.groups {
        for &p in &group.positions {
            if p.iter().all(|c| c.is_finite()) {
                points.entry(exact(p)).or_insert_with(|| written(p));
            }
        }
    }
    let places: BTreeSet<[u32; 3]> = points.values().copied().collect();
    let merged = points.len() - places.len();
    let mut folded = 0;
    for group in &node.groups {
        for triangle in &group.triangles {
            let corners = triangle.map(|k| {
                group
                    .positions
                    .get(k as usize)
                    .filter(|p| p.iter().all(|c| c.is_finite()))
                    .map(|&p| written(p))
            });
            if let [Some(a), Some(b), Some(c)] = corners
                && (a == b || b == c || c == a)
            {
                folded += 1;
            }
        }
    }
    (merged, folded)
}

/// `v` made unit length, scaled first so that neither its square
/// underflows nor overflows; `None` for a zero vector or one that is not
/// finite.
fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    if v.iter().any(|c| !c.is_finite()) {
        return None;
    }
    let largest = v.iter().fold(0.0_f64, |m, c| m.max(c.abs()));
    if largest <= 0.0 {
        return None;
    }
    let s = v.map(|c| c / largest);
    let length = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
    Some(s.map(|c| c / length))
}

/// The normals a group writes, in glTF's axes, 32 bits each: its own where
/// it gives one, else the mean of the unit normals of its triangles around
/// the position, else +Z of the model (+Y in glTF).
fn normals_of(group: &Group) -> Vec<[f32; 3]> {
    let given = |k: usize| group.normals.get(k).copied().and_then(unit);
    let count = group.positions.len();
    let mut around: Vec<[f64; 3]> = Vec::new();
    if (0..count).any(|k| given(k).is_none()) {
        around = vec![[0.0; 3]; count];
        for triangle in &group.triangles {
            let [Some(a), Some(b), Some(c)] =
                triangle.map(|k| group.positions.get(k as usize).copied())
            else {
                continue;
            };
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let Some(n) = unit(cross) else {
                continue;
            };
            for k in triangle {
                if let Some(sum) = around.get_mut(*k as usize) {
                    for (s, c) in sum.iter_mut().zip(n) {
                        *s += c;
                    }
                }
            }
        }
    }
    (0..count)
        .map(|k| {
            let [x, y, z] = given(k)
                .or_else(|| around.get(k).copied().and_then(unit))
                .unwrap_or([0.0, 0.0, 1.0]);
            [x as f32 + 0.0, z as f32 + 0.0, -y as f32 + 0.0]
        })
        .collect()
}

/// The extensions the materials use, in alphabetical order: what a file
/// of them lists in `extensionsUsed`.
pub fn extensions_used(materials: &[PbrMaterial]) -> Vec<&'static str> {
    let mut used = Vec::new();
    if materials.iter().any(|m| m.clearcoat.is_some()) {
        used.push(KHR_MATERIALS_CLEARCOAT);
    }
    if materials.iter().any(|m| m.ior.is_some()) {
        used.push(KHR_MATERIALS_IOR);
    }
    if materials.iter().any(|m| m.transmission.is_some()) {
        used.push(KHR_MATERIALS_TRANSMISSION);
    }
    used
}

fn push_material(out: &mut String, m: &PbrMaterial) {
    out.push_str("{\"name\":");
    push_string(out, &m.name);
    out.push_str(",\"pbrMetallicRoughness\":{\"baseColorFactor\":");
    let [r, g, b] = m.color.map(srgb_to_linear);
    push_numbers(out, [r, g, b, 1.0]);
    out.push_str(",\"metallicFactor\":");
    push_number(out, m.metallic);
    out.push_str(",\"roughnessFactor\":");
    push_number(out, m.roughness);
    out.push('}');
    if m.double_sided {
        out.push_str(",\"doubleSided\":true");
    }
    let mut extensions = Vec::new();
    if let Some((strength, roughness)) = m.clearcoat {
        let mut text = format!("\"{KHR_MATERIALS_CLEARCOAT}\":{{\"clearcoatFactor\":");
        push_number(&mut text, strength);
        text.push_str(",\"clearcoatRoughnessFactor\":");
        push_number(&mut text, roughness);
        text.push('}');
        extensions.push(text);
    }
    if let Some(ior) = m.ior {
        let mut text = format!("\"{KHR_MATERIALS_IOR}\":{{\"ior\":");
        push_number(&mut text, ior);
        text.push('}');
        extensions.push(text);
    }
    if let Some(transmission) = m.transmission {
        let mut text = format!("\"{KHR_MATERIALS_TRANSMISSION}\":{{\"transmissionFactor\":");
        push_number(&mut text, transmission);
        text.push('}');
        extensions.push(text);
    }
    if !extensions.is_empty() {
        out.push_str(",\"extensions\":{");
        out.push_str(&extensions.join(","));
        out.push('}');
    }
    out.push('}');
}

/// `text` as a JSON string (RFC 8259, section 7): the quotation mark, the
/// reverse solidus and the control characters U+0000 to U+001F escaped,
/// every other character itself, in UTF-8.
fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A number as JSON: the shortest decimal that reads back as the same
/// `f64`, without an exponent, `-0` as `0`. One that is not finite, which
/// the writer refuses before it gets here, as `0`.
fn push_number(out: &mut String, value: f64) {
    let value = if value.is_finite() { value + 0.0 } else { 0.0 };
    let _ = write!(out, "{value}");
}

fn push_numbers<const N: usize>(out: &mut String, values: [f64; N]) {
    out.push('[');
    for (i, v) in values.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_number(out, v);
    }
    out.push(']');
}

fn push_extras(out: &mut String, extras: &[(String, Extra)]) {
    out.push('{');
    let mut first = true;
    for (i, (key, value)) in extras.iter().enumerate() {
        if extras.iter().take(i).any(|(seen, _)| seen == key) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        push_string(out, key);
        out.push(':');
        match value {
            Extra::Text(text) => push_string(out, text),
            Extra::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Extra::Real(x) if x.is_finite() => push_number(out, *x),
            Extra::Real(_) => out.push_str("null"),
            Extra::Texts(list) => {
                out.push('[');
                for (k, text) in list.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    push_string(out, text);
                }
                out.push(']');
            }
            Extra::Numbers(list) => {
                out.push('[');
                for (k, n) in list.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{n}");
                }
                out.push(']');
            }
        }
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts of a GLB file, read by the container's rules: the header's
    /// version and length, the JSON chunk's text (padding kept) and the
    /// binary chunk.
    fn chunks(bytes: &[u8]) -> (u32, u32, String, Option<Vec<u8>>) {
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(&bytes[..4], b"glTF");
        let json_length = word(12) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let json = String::from_utf8(bytes[20..20 + json_length].to_vec()).unwrap();
        let rest = 20 + json_length;
        let bin = (rest < bytes.len()).then(|| {
            let length = word(rest) as usize;
            assert_eq!(&bytes[rest + 4..rest + 8], b"BIN\0");
            assert_eq!(rest + 8 + length, bytes.len());
            bytes[rest + 8..].to_vec()
        });
        (word(4), word(8), json, bin)
    }

    fn triangle_scene() -> Scene {
        Scene {
            generator: "3DCraft test".into(),
            asset_extras: Vec::new(),
            unit: Unit::Meter,
            nodes: vec![Node {
                name: "t".into(),
                extras: vec![("feature".into(), Extra::Text("F1".into()))],
                groups: vec![Group {
                    extras: vec![("face".into(), Extra::Text("a".into()))],
                    material: Some(0),
                    positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.5, 0.0]],
                    normals: Vec::new(),
                    triangles: vec![[0, 1, 2]],
                }],
            }],
            materials: vec![PbrMaterial {
                name: "#ff00ff".into(),
                color: [255, 0, 255],
                metallic: 0.0,
                roughness: 0.5,
                double_sided: true,
                clearcoat: Some((1.0, 0.25)),
                transmission: None,
                ior: None,
            }],
        }
    }

    #[test]
    fn one_triangle_is_written_exactly_as_specified() {
        let bytes = write_glb(&triangle_scene()).unwrap();
        let (version, length, json, bin) = chunks(&bytes);
        assert_eq!((version, length as usize), (2, bytes.len()));
        let expected = concat!(
            "{\"asset\":{\"version\":\"2.0\",\"generator\":\"3DCraft test\"},",
            "\"extensionsUsed\":[\"KHR_materials_clearcoat\"],",
            "\"scene\":0,\"scenes\":[{\"nodes\":[0]}],",
            "\"nodes\":[{\"name\":\"t\",\"mesh\":0,\"extras\":{\"feature\":\"F1\"}}],",
            "\"meshes\":[{\"name\":\"t\",\"primitives\":[{\"attributes\":{\"POSITION\":0,\"NORMAL\":1},",
            "\"indices\":2,\"material\":0,\"extras\":{\"face\":\"a\"}}]}],",
            "\"materials\":[{\"name\":\"#ff00ff\",\"pbrMetallicRoughness\":{\"baseColorFactor\":[1,0,1,1],",
            "\"metallicFactor\":0,\"roughnessFactor\":0.5},\"doubleSided\":true,",
            "\"extensions\":{\"KHR_materials_clearcoat\":{\"clearcoatFactor\":1,\"clearcoatRoughnessFactor\":0.25}}}],",
            "\"accessors\":[{\"bufferView\":0,\"byteOffset\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",",
            "\"min\":[0,0,-0.5],\"max\":[2,0,0]},",
            "{\"bufferView\":1,\"byteOffset\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"},",
            "{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}],",
            "\"bufferViews\":[{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36,\"byteStride\":12,\"target\":34962},",
            "{\"buffer\":0,\"byteOffset\":36,\"byteLength\":36,\"byteStride\":12,\"target\":34962},",
            "{\"buffer\":0,\"byteOffset\":72,\"byteLength\":8,\"target\":34963}],",
            "\"buffers\":[{\"byteLength\":80}]}"
        );
        // Padded with spaces to a multiple of 4.
        assert_eq!(json.trim_end_matches(' '), expected);
        assert!(json.len().is_multiple_of(4));
        assert!(json.len() - expected.len() < 4);
        // The binary chunk: the positions turned to +Y up, the normals
        // (the triangle's own: +Z of the model, +Y in glTF), the indices.
        let mut want: Vec<u8> = Vec::new();
        let floats: [f32; 18] = [
            0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, -0.5, // positions
            0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, // normals
        ];
        for f in floats {
            want.extend_from_slice(&f.to_le_bytes());
        }
        for k in [0_u16, 1, 2, 0] {
            want.extend_from_slice(&k.to_le_bytes());
        }
        assert_eq!(bin.unwrap(), want);
        // Deterministic.
        assert_eq!(write_glb(&triangle_scene()).unwrap(), bytes);
    }

    #[test]
    fn strings_are_escaped_as_json_requires() {
        let mut out = String::new();
        push_string(
            &mut out,
            "a\"b\\c/\n\r\t\u{8}\u{c}\u{0}\u{1}\u{1f} \u{7f}é\u{2028}😀",
        );
        assert_eq!(
            out,
            "\"a\\\"b\\\\c/\\n\\r\\t\\b\\f\\u0000\\u0001\\u001f \u{7f}é\u{2028}😀\""
        );
        // In a whole file: a name with every kind of character, and keys
        // met twice written once.
        let mut scene = triangle_scene();
        scene.nodes[0].name = "x\u{0}\"y".into();
        scene.nodes[0].extras = vec![
            ("k".into(), Extra::Numbers(vec![1, 2])),
            ("k".into(), Extra::Text("again".into())),
            ("l\n".into(), Extra::Texts(vec!["p".into(), "q\"".into()])),
            ("n".into(), Extra::Number(7)),
        ];
        let (_, _, json, _) = chunks(&write_glb(&scene).unwrap());
        assert!(json.contains("\"name\":\"x\\u0000\\\"y\""), "{json}");
        assert!(
            json.contains("\"extras\":{\"k\":[1,2],\"l\\n\":[\"p\",\"q\\\"\"],\"n\":7}"),
            "{json}"
        );
    }

    #[test]
    fn numbers_are_shortest_round_trip_decimals_without_exponents() {
        let mut out = String::new();
        push_numbers(
            &mut out,
            [-0.0, 1e-7, 0.1 + 0.2, 1e21, f64::from(0.1_f32), f64::NAN],
        );
        assert_eq!(
            out,
            "[0,0.0000001,0.30000000000000004,1000000000000000000000,0.10000000149011612,0]"
        );
    }

    #[test]
    fn linear_colour_follows_the_srgb_transfer_function() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert_eq!(srgb_to_linear(255), 1.0);
        // The linear segment, and the curve: 128 is 0.2158605 in every
        // table of the function.
        assert_eq!(srgb_to_linear(10), 10.0 / 255.0 / 12.92);
        assert!((srgb_to_linear(128) - 0.215_860_500_2).abs() < 1e-9);
        let mut last = -1.0;
        for byte in 0..=255 {
            let value = srgb_to_linear(byte);
            assert!(value > last && (0.0..=1.0).contains(&value), "{byte}");
            last = value;
        }
    }

    #[test]
    fn the_header_counts_bytes_in_32_bits() {
        assert_eq!(glb_length(100, 0), Ok(120));
        assert_eq!(glb_length(101, 0), Ok(124));
        assert_eq!(glb_length(4, 8), Ok(40));
        assert_eq!(glb_length(4, 9), Ok(44));
        // The largest file: 4 GiB less 4 bytes, a multiple of 4.
        let most = u64::from(u32::MAX) - 3 - 32;
        assert_eq!(glb_length(4, most), Ok(u32::MAX - 3));
        assert_eq!(glb_length(4, most + 1), Err(TOO_LARGE));
        assert_eq!(glb_length(u64::MAX, 0), Err(TOO_LARGE));
        assert_eq!(glb_length(4, u64::MAX - 1), Err(TOO_LARGE));
    }

    #[test]
    fn an_empty_scene_and_a_node_without_triangles_are_valid_files() {
        let bytes = write_glb(&Scene::default()).unwrap();
        let (_, length, json, bin) = chunks(&bytes);
        assert_eq!(length as usize, bytes.len());
        assert_eq!(
            json.trim_end(),
            "{\"asset\":{\"version\":\"2.0\",\"generator\":\"\"},\"scene\":0,\"scenes\":[{}]}"
        );
        assert!(bin.is_none());
        let scene = Scene {
            nodes: vec![Node {
                name: "empty".into(),
                extras: Vec::new(),
                groups: vec![Group {
                    positions: vec![[0.0; 3]],
                    ..Group::default()
                }],
            }],
            ..Scene::default()
        };
        let (_, _, json, bin) = chunks(&write_glb(&scene).unwrap());
        assert_eq!(
            json.trim_end(),
            "{\"asset\":{\"version\":\"2.0\",\"generator\":\"\"},\"scene\":0,\"scenes\":[{\"nodes\":[0]}],\"nodes\":[{\"name\":\"empty\"}]}"
        );
        assert!(bin.is_none());
    }

    /// The box and the colour read back from data that would hide a writer
    /// starting its `min` or `max` at 0, or leaving a colour in sRGB: every
    /// coordinate away from 0 (and every written z below it), and a colour
    /// of neither 0 nor 255 in any channel.
    #[test]
    fn min_and_max_are_the_datas_and_the_colour_is_linear() {
        let mut scene = triangle_scene();
        // (x, y, z) is written (x, z, −y): (1, 3, −2), (4, 3, −2), (1, 6, −5).
        scene.nodes[0].groups[0].positions =
            vec![[1.0, 2.0, 3.0], [4.0, 2.0, 3.0], [1.0, 5.0, 6.0]];
        scene.materials[0].color = [128, 64, 200];
        let (_, _, json, bin) = chunks(&write_glb(&scene).unwrap());
        assert!(json.contains("\"min\":[1,3,-5],\"max\":[4,6,-2]"), "{json}");
        let bin = bin.unwrap();
        let float = |at: usize| f32::from_le_bytes(bin[at..at + 4].try_into().unwrap());
        assert_eq!([float(24), float(28), float(32)], [1.0, 6.0, -5.0]);
        let linear = [128_u8, 64, 200].map(srgb_to_linear);
        let mut factor = String::new();
        push_numbers(&mut factor, [linear[0], linear[1], linear[2], 1.0]);
        assert!(
            json.contains(&format!("\"baseColorFactor\":{factor}")),
            "{json}"
        );
        // Linear, not the bytes over 255: 128 is 0.216, not 0.502.
        assert!((linear[0] - 0.215_860_5).abs() < 1e-6 && (linear[0] - 128.0 / 255.0).abs() > 0.2);
        assert!(!json.contains(&format!("{}", 128.0 / 255.0)), "{json}");
    }

    #[test]
    fn asset_extras_are_written_with_real_numbers() {
        let mut scene = triangle_scene();
        scene.asset_extras = vec![
            ("documentUnit".into(), Extra::Text("in".into())),
            ("scale".into(), Extra::Real(0.0254)),
            (
                "documentAxes".into(),
                Extra::Texts(vec!["X".into(), "-Z".into(), "Y".into()]),
            ),
            ("nothing".into(), Extra::Real(f64::NAN)),
        ];
        let (_, _, json, _) = chunks(&write_glb(&scene).unwrap());
        assert!(
            json.starts_with(
                "{\"asset\":{\"version\":\"2.0\",\"generator\":\"3DCraft test\",\"extras\":{\"documentUnit\":\"in\",\"scale\":0.0254,\"documentAxes\":[\"X\",\"-Z\",\"Y\"],\"nothing\":null}},"
            ),
            "{json}"
        );
    }

    #[test]
    fn points_that_fall_together_in_32_bits_are_counted() {
        // A millimetre cube's corners a hundred kilometres out along x, in
        // millimetres: 32-bit floats there are 7.8 mm apart in x (and fine
        // in y and z, near 0), so its eight corners are four places and
        // every triangle with a side along x folds.
        let corner = |i: u32| {
            [
                1e8 + f64::from(i & 1),
                f64::from((i >> 1) & 1),
                f64::from((i >> 2) & 1),
            ]
        };
        let far = Node {
            name: "far".into(),
            extras: Vec::new(),
            groups: vec![Group {
                positions: (0..8).map(corner).collect(),
                triangles: vec![[0, 2, 3], [0, 3, 1], [4, 5, 7], [4, 7, 6]],
                ..Group::default()
            }],
        };
        assert_eq!(points_in_32_bits(&far, Unit::Millimeter), (4, 4));
        // Near the origin nothing falls together; a point on two faces (two
        // copies, one in each group) is one point.
        let mut near = far.clone();
        for p in &mut near.groups[0].positions {
            p[0] -= 1e8;
        }
        near.groups.push(near.groups[0].clone());
        assert_eq!(points_in_32_bits(&near, Unit::Millimeter), (0, 0));
        // Nor in metres, where the same model is a thousand times larger.
        assert_eq!(points_in_32_bits(&near, Unit::Meter), (0, 0));
    }

    /// A strip of `n` vertices along x, `n − 2` triangles.
    fn strip(n: u32) -> Group {
        Group {
            positions: (0..n)
                .map(|k| [f64::from(k / 2), f64::from(k % 2), 0.0])
                .collect(),
            triangles: (0..n - 2)
                .map(|k| {
                    if k.is_multiple_of(2) {
                        [k, k + 1, k + 2]
                    } else {
                        [k + 1, k, k + 2]
                    }
                })
                .collect(),
            ..Group::default()
        }
    }

    #[test]
    fn indices_take_32_bits_past_65535_vertices() {
        for (n, kind) in [(65_535, "5123"), (65_536, "5125")] {
            let scene = Scene {
                nodes: vec![Node {
                    name: "strip".into(),
                    extras: Vec::new(),
                    groups: vec![strip(n)],
                }],
                ..Scene::default()
            };
            let bytes = write_glb(&scene).unwrap();
            let (_, _, json, bin) = chunks(&bytes);
            assert!(
                json.contains(&format!(
                    "\"componentType\":{kind},\"count\":{}",
                    3 * (n - 2)
                )),
                "{n}"
            );
            let bin = bin.unwrap();
            let start = 24 * n as usize;
            let width = if n > 65_535 { 4 } else { 2 };
            let last = &bin[start + (3 * (n as usize - 2) - 1) * width..][..width];
            let mut word = [0_u8; 4];
            word[..width].copy_from_slice(last);
            assert_eq!(u32::from_le_bytes(word), n - 1);
            assert!(bin.len().is_multiple_of(4));
        }
    }

    #[test]
    fn positions_are_metres_with_y_up_and_normals_unit_length() {
        // A triangle in the model's xz plane facing −y (the front), in
        // inches: in glTF it stands in the xy plane facing +z.
        let scene = Scene {
            unit: Unit::Inch,
            nodes: vec![Node {
                name: "front".into(),
                extras: Vec::new(),
                groups: vec![Group {
                    positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 10.0]],
                    // One given (not unit length), one zero, one not finite.
                    normals: vec![[0.0, -3.0, 0.0], [0.0; 3], [f64::NAN, 0.0, 0.0]],
                    triangles: vec![[0, 1, 2]],
                    ..Group::default()
                }],
            }],
            ..Scene::default()
        };
        let (_, _, json, bin) = chunks(&write_glb(&scene).unwrap());
        let bin = bin.unwrap();
        let float = |at: usize| f32::from_le_bytes(bin[at..at + 4].try_into().unwrap());
        let read = |at: usize| [float(at), float(at + 4), float(at + 8)];
        assert_eq!(read(12), [0.254, 0.0, 0.0]);
        assert_eq!(read(24), [0.0, 0.254, 0.0]);
        for k in 0..3 {
            assert_eq!(read(36 + 12 * k), [0.0, 0.0, 1.0], "normal {k}");
        }
        assert!(
            json.contains("\"min\":[0,0,0],\"max\":[0.2540000081062317,0.2540000081062317,0]"),
            "{json}"
        );
    }

    #[test]
    fn bad_scenes_are_errors_that_say_where() {
        let bad_group = |change: fn(&mut Group)| {
            let mut scene = triangle_scene();
            scene.nodes.insert(0, triangle_scene().nodes[0].clone());
            change(&mut scene.nodes[1].groups[0]);
            write_glb(&scene)
        };
        let error = |r: Result<Vec<u8>, MeshError>| r.unwrap_err().to_string();
        assert_eq!(
            error(bad_group(|g| g.positions[1][2] = f64::NAN)),
            "object 1, group 0: vertex 1 is not finite (NaN or infinity)"
        );
        assert_eq!(
            error(bad_group(|g| g.positions[2][0] = 1e39)),
            "object 1, group 0: vertex 2 lies farther from the origin, in metres, than a 32-bit float holds"
        );
        assert_eq!(
            error(bad_group(|g| g.triangles[0] = [0, 3, 1])),
            "object 1, group 0: triangle 0 refers to vertex 3, and there are 3"
        );
        assert_eq!(
            error(bad_group(|g| g.normals = vec![[0.0, 0.0, 1.0]])),
            "object 1, group 0: it has 1 normals for 3 positions: give one per position, or none"
        );
        assert_eq!(
            bad_group(|g| g.material = Some(1)),
            Err(MeshError::BadMaterial { object: 1 })
        );
        // Far in metres, not in the unit: micrometres hold what metres do
        // not.
        let mut micro = triangle_scene();
        micro.unit = Unit::Micron;
        micro.nodes[0].groups[0].positions[2][0] = 1e39;
        assert!(write_glb(&micro).is_ok());
        for (change, what) in [
            (
                (|m: &mut PbrMaterial| m.metallic = 1.5) as fn(&mut PbrMaterial),
                "the metallic factor must be from 0 to 1",
            ),
            (
                |m| m.roughness = f64::NAN,
                "the roughness factor must be from 0 to 1",
            ),
            (
                |m| m.clearcoat = Some((0.5, -0.1)),
                "the clear coat's strength and roughness must be from 0 to 1",
            ),
            (
                |m| m.transmission = Some(f64::INFINITY),
                "the transmission must be from 0 to 1",
            ),
            (
                |m| m.ior = Some(0.9),
                "the index of refraction must be a number of 1 or more",
            ),
        ] {
            let mut scene = triangle_scene();
            change(&mut scene.materials[0]);
            assert_eq!(
                write_glb(&scene),
                Err(MeshError::BadMaterialValue { material: 0, what })
            );
        }
        // A group without triangles is left out, whatever it holds.
        let mut ignored = triangle_scene();
        ignored.nodes[0].groups.push(Group {
            positions: vec![[f64::NAN; 3]],
            material: Some(9),
            ..Group::default()
        });
        assert_eq!(
            write_glb(&ignored).unwrap(),
            write_glb(&triangle_scene()).unwrap()
        );
    }

    #[test]
    fn transmission_and_ior_go_to_their_extensions() {
        let mut scene = triangle_scene();
        scene.materials[0].clearcoat = None;
        scene.materials[0].transmission = Some(0.75);
        scene.materials[0].ior = Some(1.45);
        scene.materials[0].double_sided = false;
        let (_, _, json, _) = chunks(&write_glb(&scene).unwrap());
        assert!(
            json.contains(
                "\"extensionsUsed\":[\"KHR_materials_ior\",\"KHR_materials_transmission\"]"
            ),
            "{json}"
        );
        assert!(
            json.contains("\"roughnessFactor\":0.5},\"extensions\":{\"KHR_materials_ior\":{\"ior\":1.45},\"KHR_materials_transmission\":{\"transmissionFactor\":0.75}}}"),
            "{json}"
        );
        assert!(!json.contains("doubleSided"));
    }
}
