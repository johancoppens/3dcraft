//! An independent reader of glTF 2.0 binary files (GLB), for the tests
//! that read exports back. Written from the Khronos glTF 2.0
//! specification, sharing no code with the writer (`craft3d_meshio::gltf`)
//! or with `craft3d-io`; the JSON text is parsed by `serde_json`. It reads
//! what the tests need (the default scene's nodes, their meshes' triangle
//! primitives with positions, normals and indices, and the materials) and
//! refuses, on the way, what the specification forbids there, so that a
//! file it reads is one any reader may rely on:
//!
//! - the GLB container: the magic `glTF`, version 2, the header's length
//!   the file's, every chunk a multiple of 4 bytes long, the JSON chunk
//!   first (padded with spaces) and the binary chunk second, the buffer at
//!   most as long as the binary chunk and at most 3 bytes shorter;
//! - buffer views within the buffer, a stride from 4 to 252 and a multiple
//!   of 4;
//! - accessors within their view, each element at a multiple of its
//!   component's size from the buffer's start, and of 4 for a vertex
//!   attribute;
//! - `POSITION`: three floats, with `min` and `max` given and equal to the
//!   data's, component by component;
//! - `NORMAL`: three floats of unit length (within 1e-5);
//! - the indices: unsigned integers, three per triangle, each below the
//!   number of vertices and never the largest value of its type;
//! - the attributes of a primitive all of one count.

use serde_json::Value;

/// A glTF file as the tests see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Gltf {
    pub version: String,
    pub generator: String,
    pub extensions_used: Vec<String>,
    pub extensions_required: Vec<String>,
    /// The default scene's nodes, in its order.
    pub nodes: Vec<GltfNode>,
    pub materials: Vec<GltfMaterial>,
    /// The whole JSON chunk, for anything else.
    pub json: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfNode {
    pub name: String,
    /// `extras`, `Null` when there are none.
    pub extras: Value,
    /// True when the node has a `matrix`, `translation`, `rotation` or
    /// `scale`.
    pub transformed: bool,
    pub children: usize,
    /// Its mesh's name, when it has a mesh.
    pub mesh: Option<String>,
    pub primitives: Vec<GltfPrimitive>,
}

/// A primitive of triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfPrimitive {
    /// As the file holds them: metres, +Y up (32-bit floats widened).
    pub positions: Vec<[f64; 3]>,
    /// Empty when the primitive has none.
    pub normals: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    pub material: Option<usize>,
    pub extras: Value,
    /// The `min` and `max` of the position accessor.
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// The indices' `componentType`.
    pub index_type: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfMaterial {
    pub name: String,
    /// Linear, with alpha.
    pub base_color: [f64; 4],
    pub metallic: f64,
    pub roughness: f64,
    pub double_sided: bool,
    /// `extensions`, `Null` when there are none.
    pub extensions: Value,
}

const JSON: u32 = 0x4E4F_534A;
const BIN: u32 = 0x004E_4942;

fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    let end = at.checked_add(4).ok_or("GLB: offset overflow")?;
    let b = bytes
        .get(at..end)
        .ok_or_else(|| format!("GLB: no 4 bytes at {at}"))?;
    let mut w = [0_u8; 4];
    w.copy_from_slice(b);
    Ok(u32::from_le_bytes(w))
}

fn index(value: &Value, key: &str) -> Result<Option<usize>, String> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("`{key}` is not an index: {v}")),
    }
}

fn required(value: &Value, key: &str) -> Result<usize, String> {
    index(value, key)?.ok_or_else(|| format!("`{key}` is missing in {value}"))
}

fn numbers(value: Option<&Value>) -> Option<Vec<f64>> {
    value?.as_array()?.iter().map(Value::as_f64).collect()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// One element of an accessor, its components as numbers.
struct Accessor {
    component: u64,
    width: usize,
    values: Vec<f64>,
    min: Option<Vec<f64>>,
    max: Option<Vec<f64>>,
}

impl Accessor {
    fn count(&self) -> usize {
        self.values.len() / self.width.max(1)
    }

    fn vec3(&self) -> Vec<[f64; 3]> {
        self.values
            .chunks_exact(3)
            .map(|c| [c.first(), c.get(1), c.get(2)].map(|v| v.copied().unwrap_or(f64::NAN)))
            .collect()
    }
}

/// Reads accessor `at`; `vertex` for a vertex attribute (4-byte alignment).
fn accessor(
    json: &Value,
    bin: &[u8],
    buffer: usize,
    at: usize,
    vertex: bool,
) -> Result<Accessor, String> {
    let a = json
        .get("accessors")
        .and_then(|list| list.get(at))
        .ok_or_else(|| format!("no accessor {at}"))?;
    let name = |what: &str| format!("accessor {at}: {what}");
    let view_at = index(a, "bufferView")?.ok_or_else(|| name("no bufferView"))?;
    let component = a
        .get("componentType")
        .and_then(Value::as_u64)
        .ok_or_else(|| name("no componentType"))?;
    let size: usize = match component {
        5121 => 1,
        5123 => 2,
        5125 | 5126 => 4,
        other => return Err(name(&format!("componentType {other} is not read here"))),
    };
    let width: usize = match a.get("type").and_then(Value::as_str) {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        other => return Err(name(&format!("type {other:?} is not read here"))),
    };
    let count = required(a, "count")?;
    if count == 0 {
        return Err(name("count 0"));
    }
    let offset = index(a, "byteOffset")?.unwrap_or(0);
    let view = json
        .get("bufferViews")
        .and_then(|list| list.get(view_at))
        .ok_or_else(|| name("its bufferView does not exist"))?;
    if required(view, "buffer")? != 0 {
        return Err(name("a buffer other than the GLB's"));
    }
    let view_offset = index(view, "byteOffset")?.unwrap_or(0);
    let view_length = required(view, "byteLength")?;
    let view_end = view_offset
        .checked_add(view_length)
        .filter(|&end| end <= buffer)
        .ok_or_else(|| name("its bufferView runs past the buffer"))?;
    let stride = index(view, "byteStride")?;
    if let Some(stride) = stride
        && !(stride.is_multiple_of(4) && (4..=252).contains(&stride))
    {
        return Err(name(&format!("byteStride {stride}")));
    }
    let start = view_offset
        .checked_add(offset)
        .ok_or_else(|| name("overflow"))?;
    if !offset.is_multiple_of(size) || !start.is_multiple_of(size) {
        return Err(name("not aligned to its component's size"));
    }
    if vertex && !(offset.is_multiple_of(4) && start.is_multiple_of(4)) {
        return Err(name("a vertex attribute not aligned to 4 bytes"));
    }
    let element = size * width;
    let step = stride.unwrap_or(element);
    if step < element {
        return Err(name("a stride shorter than an element"));
    }
    let end = step
        .checked_mul(count - 1)
        .and_then(|n| n.checked_add(element))
        .and_then(|n| n.checked_add(start))
        .ok_or_else(|| name("overflow"))?;
    if end > view_end {
        return Err(name("runs past its bufferView"));
    }
    let mut values = Vec::with_capacity(count.saturating_mul(width));
    for i in 0..count {
        for c in 0..width {
            let at = start + i * step + c * size;
            let bytes = bin
                .get(at..at + size)
                .ok_or_else(|| name("past the binary chunk"))?;
            let value = match (component, bytes) {
                (5126, [a, b, c, d]) => f64::from(f32::from_le_bytes([*a, *b, *c, *d])),
                (5125, [a, b, c, d]) => f64::from(u32::from_le_bytes([*a, *b, *c, *d])),
                (5123, [a, b]) => f64::from(u16::from_le_bytes([*a, *b])),
                (5121, [a]) => f64::from(*a),
                _ => return Err(name("a component of the wrong size")),
            };
            values.push(value);
        }
    }
    Ok(Accessor {
        component,
        width,
        values,
        min: numbers(a.get("min")),
        max: numbers(a.get("max")),
    })
}

/// A glTF 2.0 binary file read back (see the module's documentation).
pub fn read_glb(bytes: &[u8]) -> Result<Gltf, String> {
    if bytes.get(..4) != Some(b"glTF".as_slice()) {
        return Err("GLB: no `glTF` magic".to_string());
    }
    if word(bytes, 4)? != 2 {
        return Err(format!("GLB: version {}", word(bytes, 4)?));
    }
    if usize::try_from(word(bytes, 8)?).ok() != Some(bytes.len()) {
        return Err(format!(
            "GLB: the header says {} bytes, the file has {}",
            word(bytes, 8)?,
            bytes.len()
        ));
    }
    let mut chunks: Vec<(u32, &[u8])> = Vec::new();
    let mut at = 12_usize;
    while at < bytes.len() {
        let length = usize::try_from(word(bytes, at)?).map_err(|e| e.to_string())?;
        let kind = word(bytes, at + 4)?;
        if !length.is_multiple_of(4) {
            return Err(format!(
                "GLB: a chunk of {length} bytes, not a multiple of 4"
            ));
        }
        let end = at
            .checked_add(8)
            .and_then(|s| s.checked_add(length))
            .ok_or("GLB: overflow")?;
        let data = bytes.get(at + 8..end).ok_or("GLB: a chunk past the end")?;
        chunks.push((kind, data));
        at = end;
    }
    let Some(&(JSON, text)) = chunks.first() else {
        return Err("GLB: the first chunk is not JSON".to_string());
    };
    let bin: &[u8] = match chunks.get(1) {
        None => &[],
        Some(&(BIN, data)) => data,
        Some(_) => return Err("GLB: the second chunk is not BIN".to_string()),
    };
    let trimmed = text
        .iter()
        .rposition(|&b| b != b' ')
        .and_then(|last| text.get(..=last))
        .ok_or("GLB: an empty JSON chunk")?;
    let json: Value = serde_json::from_slice(trimmed).map_err(|e| format!("JSON: {e}"))?;
    let asset = json.get("asset").ok_or("no asset")?;
    let version = asset
        .get("version")
        .and_then(Value::as_str)
        .ok_or("no asset.version")?
        .to_string();
    if version != "2.0" {
        return Err(format!("asset.version {version}"));
    }
    let generator = asset
        .get("generator")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let buffer = match json.get("buffers").and_then(Value::as_array) {
        None => 0,
        Some(buffers) => {
            let [only] = buffers.as_slice() else {
                return Err("more than one buffer".to_string());
            };
            if only.get("uri").is_some() {
                return Err("the buffer has a uri: not the GLB's".to_string());
            }
            let length = required(only, "byteLength")?;
            if length > bin.len() || length + 3 < bin.len() {
                return Err(format!(
                    "the buffer is {length} bytes, the binary chunk {}",
                    bin.len()
                ));
            }
            length
        }
    };
    let mut materials = Vec::new();
    for m in json
        .get("materials")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let pbr = m.get("pbrMetallicRoughness");
        let factor = |key: &str| {
            pbr.and_then(|p| p.get(key))
                .map_or(Some(1.0), Value::as_f64)
                .ok_or_else(|| format!("material: {key} is not a number"))
        };
        let colour = match pbr.and_then(|p| p.get("baseColorFactor")) {
            None => vec![1.0; 4],
            some => numbers(some).ok_or("baseColorFactor is not numbers")?,
        };
        let base_color: [f64; 4] = colour
            .try_into()
            .map_err(|_| "baseColorFactor is not four numbers".to_string())?;
        materials.push(GltfMaterial {
            name: m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            base_color,
            metallic: factor("metallicFactor")?,
            roughness: factor("roughnessFactor")?,
            double_sided: m
                .get("doubleSided")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            extensions: m.get("extensions").cloned().unwrap_or(Value::Null),
        });
    }
    let scene_at = required(&json, "scene")?;
    let scene = json
        .get("scenes")
        .and_then(|s| s.get(scene_at))
        .ok_or("the scene does not exist")?;
    let mut nodes = Vec::new();
    for at in scene
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let at = at
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("a node index")?;
        let node = json
            .get("nodes")
            .and_then(|n| n.get(at))
            .ok_or_else(|| format!("node {at} does not exist"))?;
        let mut read = GltfNode {
            name: node
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            extras: node.get("extras").cloned().unwrap_or(Value::Null),
            transformed: ["matrix", "translation", "rotation", "scale"]
                .iter()
                .any(|k| node.get(*k).is_some()),
            children: node
                .get("children")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            mesh: None,
            primitives: Vec::new(),
        };
        if let Some(mesh_at) = index(node, "mesh")? {
            let mesh = json
                .get("meshes")
                .and_then(|m| m.get(mesh_at))
                .ok_or_else(|| format!("mesh {mesh_at} does not exist"))?;
            read.mesh = Some(
                mesh.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
            for p in mesh
                .get("primitives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                read.primitives
                    .push(primitive(&json, bin, buffer, p, materials.len())?);
            }
        }
        nodes.push(read);
    }
    Ok(Gltf {
        version,
        generator,
        extensions_used: strings(json.get("extensionsUsed")),
        extensions_required: strings(json.get("extensionsRequired")),
        nodes,
        materials,
        json,
    })
}

fn primitive(
    json: &Value,
    bin: &[u8],
    buffer: usize,
    p: &Value,
    materials: usize,
) -> Result<GltfPrimitive, String> {
    if p.get("mode").is_some_and(|m| m.as_u64() != Some(4)) {
        return Err(format!("a primitive that is not triangles: {p}"));
    }
    let attributes = p
        .get("attributes")
        .ok_or("a primitive without attributes")?;
    let position = accessor(json, bin, buffer, required(attributes, "POSITION")?, true)?;
    if (position.component, position.width) != (5126, 3) {
        return Err("POSITION is not three floats".to_string());
    }
    let positions = position.vec3();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in &positions {
        for ((lo, hi), c) in min.iter_mut().zip(max.iter_mut()).zip(p) {
            *lo = lo.min(*c);
            *hi = hi.max(*c);
        }
    }
    let declared = |bound: &Option<Vec<f64>>| -> Result<[f64; 3], String> {
        bound
            .clone()
            .ok_or("POSITION without min or max")?
            .try_into()
            .map_err(|_| "POSITION's min or max is not three numbers".to_string())
    };
    let (declared_min, declared_max) = (declared(&position.min)?, declared(&position.max)?);
    if declared_min != min || declared_max != max {
        return Err(format!(
            "POSITION's min {declared_min:?} and max {declared_max:?} are not the data's {min:?} and {max:?}"
        ));
    }
    let normals = match index(attributes, "NORMAL")? {
        None => Vec::new(),
        Some(at) => {
            let normal = accessor(json, bin, buffer, at, true)?;
            if (normal.component, normal.width) != (5126, 3) {
                return Err("NORMAL is not three floats".to_string());
            }
            if normal.count() != position.count() {
                return Err("NORMAL and POSITION differ in count".to_string());
            }
            let normals = normal.vec3();
            if let Some(n) = normals
                .iter()
                .find(|n| ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() > 1e-5)
            {
                return Err(format!("a normal of length other than 1: {n:?}"));
            }
            normals
        }
    };
    let indices = accessor(json, bin, buffer, required(p, "indices")?, false)?;
    let largest = match indices.component {
        5121 => f64::from(u8::MAX),
        5123 => f64::from(u16::MAX),
        5125 => f64::from(u32::MAX),
        other => return Err(format!("indices of componentType {other}")),
    };
    if indices.width != 1 || !indices.values.len().is_multiple_of(3) {
        return Err("indices that are not three scalars per triangle".to_string());
    }
    let vertices = positions.len();
    let mut triangles = Vec::with_capacity(indices.values.len() / 3);
    for t in indices.values.chunks_exact(3) {
        let mut corner = [0_usize; 3];
        for (slot, &k) in corner.iter_mut().zip(t) {
            if k == largest {
                return Err("an index that is the primitive restart value".to_string());
            }
            // A whole number from the file's integer type.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let k = k as usize;
            if k >= vertices {
                return Err(format!("index {k} of {vertices} vertices"));
            }
            *slot = k;
        }
        triangles.push(corner);
    }
    let material = index(p, "material")?;
    if material.is_some_and(|m| m >= materials) {
        return Err("a material that does not exist".to_string());
    }
    Ok(GltfPrimitive {
        positions,
        normals,
        triangles,
        material,
        extras: p.get("extras").cloned().unwrap_or(Value::Null),
        min,
        max,
        index_type: indices.component,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GLB file of `json` and `bin`, framed by hand.
    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
        let mut out = b"glTF".to_vec();
        for word in [2, total as u32, json.len() as u32] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        if !bin.is_empty() {
            out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&bin);
        }
        out
    }

    /// The corner tetrahedron in metres: four points, four triangles.
    fn tetrahedron(min: &str, index: u16, offset: u32) -> Vec<u8> {
        let mut bin = Vec::new();
        for p in [
            [0.0_f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ] {
            for c in p {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for k in [0_u16, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, index] {
            bin.extend_from_slice(&k.to_le_bytes());
        }
        let json = format!(
            "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"hand\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
             \"nodes\":[{{\"name\":\"tetra\",\"mesh\":0,\"extras\":{{\"k\":1}}}}],\
             \"meshes\":[{{\"name\":\"m\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"material\":0}}]}}],\
             \"materials\":[{{\"name\":\"plain\"}}],\
             \"accessors\":[{{\"bufferView\":0,\"byteOffset\":{offset},\"componentType\":5126,\"count\":4,\"type\":\"VEC3\",\"min\":{min},\"max\":[1,1,1]}},\
             {{\"bufferView\":1,\"componentType\":5123,\"count\":12,\"type\":\"SCALAR\"}}],\
             \"bufferViews\":[{{\"buffer\":0,\"byteLength\":48}},{{\"buffer\":0,\"byteOffset\":48,\"byteLength\":24}}],\
             \"buffers\":[{{\"byteLength\":72}}]}}"
        );
        glb(&json, &bin)
    }

    #[test]
    fn a_file_made_by_hand_reads_back() {
        let read = read_glb(&tetrahedron("[0,0,0]", 3, 0)).unwrap();
        assert_eq!(
            (read.version.as_str(), read.generator.as_str()),
            ("2.0", "hand")
        );
        let node = &read.nodes[0];
        assert_eq!(
            (node.name.as_str(), node.mesh.as_deref()),
            ("tetra", Some("m"))
        );
        assert_eq!(node.extras["k"], 1);
        let p = &node.primitives[0];
        assert_eq!(p.positions[3], [0.0, 0.0, 1.0]);
        assert_eq!(p.triangles, [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]);
        assert!(p.normals.is_empty());
        assert_eq!(p.index_type, 5123);
        // A material without factors has the specification's defaults.
        let m = &read.materials[0];
        assert_eq!(
            (m.base_color, m.metallic, m.roughness),
            ([1.0; 4], 1.0, 1.0)
        );
    }

    #[test]
    fn what_the_specification_forbids_is_refused() {
        // A min that is not the data's.
        let wrong = read_glb(&tetrahedron("[0,0,-1]", 3, 0)).unwrap_err();
        assert!(wrong.contains("min"), "{wrong}");
        // An index past the vertices, and the restart value.
        assert!(
            read_glb(&tetrahedron("[0,0,0]", 4, 0))
                .unwrap_err()
                .contains("index 4")
        );
        assert!(
            read_glb(&tetrahedron("[0,0,0]", u16::MAX, 0))
                .unwrap_err()
                .contains("restart")
        );
        // A vertex attribute off its alignment (and past its view).
        assert!(read_glb(&tetrahedron("[0,0,0]", 3, 2)).is_err());
        // The container.
        let good = tetrahedron("[0,0,0]", 3, 0);
        let mut magic = good.clone();
        magic[0] = b'g' ^ 0x20;
        assert!(read_glb(&magic).unwrap_err().contains("magic"));
        let mut long = good.clone();
        long.extend_from_slice(&[0; 4]);
        assert!(read_glb(&long).unwrap_err().contains("header"));
        let mut version = good;
        version[4] = 1;
        assert!(read_glb(&version).unwrap_err().contains("version"));
    }
}
