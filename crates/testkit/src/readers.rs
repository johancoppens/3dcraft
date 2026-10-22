//! Independent readers of the colours in exported files: a 3MF package
//! (its stored ZIP container and the model part's base materials and
//! colour groups), a STEP file (its styled items) and an OBJ file with its
//! material library.
//!
//! Written from the specifications, not from the writers (`craft3d-meshio`,
//! `craft3d-zipstore`, `craft3d-io`), and sharing no code with them or with
//! the STEP reader (`craft3d-stepread`), so that a test reading an export
//! back with these checks the file and not the writer's idea of it:
//!
//! - ZIP: PKWARE's APPNOTE.TXT (the end of central directory record, the
//!   central directory's file headers, the local file headers); only stored
//!   entries (method 0) are read, which is what 3MF packages may use.
//! - 3MF: the 3MF Core Specification (resources, `basematerials` and
//!   `base` with `displaycolor`, objects' `pid` and `pindex`, triangles'
//!   `pid` and `p1`, the build) and the Materials and Properties Extension
//!   for `colorgroup` and `color`.
//! - STEP: ISO 10303-21 (the clear text encoding: entity instances,
//!   complex instances, parameters) and the presentation resources of
//!   ISO 10303-46 as AP214 and AP242 use them (`STYLED_ITEM` and the style
//!   assignment down to a `COLOUR_RGB`).
//! - OBJ and MTL: the Wavefront OBJ and MTL format descriptions (`mtllib`,
//!   `usemtl`, `newmtl`, `Kd`), with `Pr` and `Pm` of the PBR extension.

use std::collections::BTreeMap;

// ------------------------------------------------------------------- ZIP

fn u16_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    let b = bytes
        .get(at..at.checked_add(2).ok_or("offset overflow")?)
        .ok_or_else(|| format!("ZIP: no 2 bytes at {at}"))?;
    Ok(usize::from(u16::from_le_bytes([
        *b.first().ok_or("short")?,
        *b.get(1).ok_or("short")?,
    ])))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    let b = bytes
        .get(at..at.checked_add(4).ok_or("offset overflow")?)
        .ok_or_else(|| format!("ZIP: no 4 bytes at {at}"))?;
    let mut word = [0_u8; 4];
    word.copy_from_slice(b);
    usize::try_from(u32::from_le_bytes(word)).map_err(|e| e.to_string())
}

/// The entries of a ZIP archive whose entries are all stored (method 0),
/// by name, in the order of the central directory.
pub fn zip_stored_entries(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // The end of central directory record: signature 0x06054b50, at least
    // 22 bytes from the end (more with a comment).
    let eocd = (0..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&at| u32_at(bytes, at).ok() == Some(0x0605_4b50))
        .ok_or("ZIP: no end of central directory record")?;
    let count = u16_at(bytes, eocd + 10)?;
    let mut at = u32_at(bytes, eocd + 16)?;
    let mut entries = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        if u32_at(bytes, at)? != 0x0201_4b50 {
            return Err(format!("ZIP: no central file header at {at}"));
        }
        let method = u16_at(bytes, at + 10)?;
        let size = u32_at(bytes, at + 20)?;
        let name_len = u16_at(bytes, at + 28)?;
        let extra_len = u16_at(bytes, at + 30)?;
        let comment_len = u16_at(bytes, at + 32)?;
        let local = u32_at(bytes, at + 42)?;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or("ZIP: name past the end")?;
        let name = String::from_utf8(name.to_vec()).map_err(|e| e.to_string())?;
        if method != 0 {
            return Err(format!("ZIP: `{name}` is not stored (method {method})"));
        }
        if u32_at(bytes, local)? != 0x0403_4b50 {
            return Err(format!("ZIP: no local file header for `{name}`"));
        }
        let data = local + 30 + u16_at(bytes, local + 26)? + u16_at(bytes, local + 28)?;
        let content = bytes
            .get(data..data.checked_add(size).ok_or("size overflow")?)
            .ok_or_else(|| format!("ZIP: the data of `{name}` runs past the end"))?;
        entries.push((name, content.to_vec()));
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

// ------------------------------------------------------------------- XML

/// One start (or empty-element) tag of an XML document: its local name
/// (the part after a namespace prefix) and its attributes, entities
/// decoded. End tags are `/name` with no attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

impl Tag {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

/// The tags of an XML document in order, without text, comments,
/// processing instructions or declarations.
pub fn xml_tags(xml: &str) -> Result<Vec<Tag>, String> {
    let mut tags = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = rest.get(open + 1..).unwrap_or("");
        if let Some(after) = rest.strip_prefix("!--") {
            let end = after.find("-->").ok_or("XML: unclosed comment")?;
            rest = after.get(end + 3..).unwrap_or("");
            continue;
        }
        if rest.starts_with('?') || rest.starts_with('!') {
            let end = rest.find('>').ok_or("XML: unclosed declaration")?;
            rest = rest.get(end + 1..).unwrap_or("");
            continue;
        }
        // The tag runs to the first `>` outside a quoted value.
        let mut quote = None;
        let mut end = None;
        for (i, c) in rest.char_indices() {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (None, '>') => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let end = end.ok_or("XML: unclosed tag")?;
        let body = rest.get(..end).unwrap_or("").trim_end_matches('/');
        rest = rest.get(end + 1..).unwrap_or("");
        if let Some(name) = body.strip_prefix('/') {
            tags.push(Tag {
                name: format!("/{}", local_name(name.trim())),
                attributes: BTreeMap::new(),
            });
            continue;
        }
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = local_name(body.get(..name_end).unwrap_or(""));
        let mut attributes = BTreeMap::new();
        let mut text = body.get(name_end..).unwrap_or("");
        loop {
            text = text.trim_start();
            let Some(eq) = text.find('=') else { break };
            let key = text.get(..eq).unwrap_or("").trim().to_string();
            let after = text.get(eq + 1..).unwrap_or("").trim_start();
            let q = after.chars().next().ok_or("XML: attribute without value")?;
            if q != '"' && q != '\'' {
                return Err(format!("XML: unquoted value of `{key}`"));
            }
            let value_text = after.get(1..).unwrap_or("");
            let close = value_text.find(q).ok_or("XML: unclosed value")?;
            attributes.insert(
                local_name(&key),
                decode_entities(value_text.get(..close).unwrap_or("")),
            );
            text = value_text.get(close + 1..).unwrap_or("");
        }
        tags.push(Tag { name, attributes });
    }
    Ok(tags)
}

// ------------------------------------------------------------------- 3MF

/// An object of a 3MF model with the colour of each triangle.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreeMfObject {
    pub id: String,
    pub name: String,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    /// Per triangle: its colour (sRGB), from its own property or the
    /// object's, or `None` when it has neither.
    pub colors: Vec<Option<[u8; 3]>>,
    /// The object's own colour, from its `pid` and `pindex`.
    pub color: Option<[u8; 3]>,
}

/// `#RRGGBB` or `#RRGGBBAA` (the alpha is dropped).
fn srgb_hex(text: &str) -> Result<[u8; 3], String> {
    let hex = text
        .strip_prefix('#')
        .filter(|h| h.len() == 6 || h.len() == 8)
        .ok_or_else(|| format!("3MF: `{text}` is not an sRGB colour"))?;
    let byte = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|b| u8::from_str_radix(b, 16).ok())
            .ok_or_else(|| format!("3MF: `{text}` is not an sRGB colour"))
    };
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

/// The objects of a 3MF package (its bytes), with their triangles'
/// colours: base materials (`basematerials`, `displaycolor`) and colour
/// groups (`colorgroup`, `color`), by the object's `pid`/`pindex` and the
/// triangle's `pid`/`p1`.
pub fn read_3mf_colors(package: &[u8]) -> Result<Vec<ThreeMfObject>, String> {
    let entries = zip_stored_entries(package)?;
    // The model part is the target of the root relationship of the 3D
    // model type; `3D/3dmodel.model` by convention.
    let rels = entries
        .iter()
        .find(|(name, _)| name == "_rels/.rels")
        .ok_or("3MF: no _rels/.rels")?;
    let rels = xml_tags(&String::from_utf8_lossy(&rels.1))?;
    let target = rels
        .iter()
        .find(|t| {
            t.name == "Relationship"
                && t.get("Type")
                    .is_some_and(|ty| ty.ends_with("/3dmanufacturing/2013/01/3dmodel"))
        })
        .and_then(|t| t.get("Target"))
        .ok_or("3MF: no model relationship")?;
    let part = target.trim_start_matches('/');
    let model = entries
        .iter()
        .find(|(name, _)| name == part)
        .ok_or_else(|| format!("3MF: no part `{part}`"))?;
    let tags = xml_tags(&String::from_utf8_lossy(&model.1))?;

    // Property groups by id: the colours of their entries in order.
    let mut groups: BTreeMap<String, Vec<[u8; 3]>> = BTreeMap::new();
    let mut group: Option<String> = None;
    for tag in &tags {
        match tag.name.as_str() {
            "basematerials" | "colorgroup" => {
                let id = tag.get("id").ok_or("3MF: a group without an id")?;
                groups.insert(id.to_string(), Vec::new());
                group = Some(id.to_string());
            }
            "/basematerials" | "/colorgroup" => group = None,
            "base" | "color" => {
                let key = if tag.name == "base" {
                    "displaycolor"
                } else {
                    "color"
                };
                let color = srgb_hex(tag.get(key).ok_or("3MF: an entry without a colour")?)?;
                let id = group.as_ref().ok_or("3MF: an entry outside a group")?;
                groups.entry(id.clone()).or_default().push(color);
            }
            _ => {}
        }
    }
    let lookup = |pid: &str, index: &str| -> Result<[u8; 3], String> {
        let entries = groups
            .get(pid)
            .ok_or_else(|| format!("3MF: no property group {pid}"))?;
        let index: usize = index.parse().map_err(|_| format!("3MF: index `{index}`"))?;
        entries
            .get(index)
            .copied()
            .ok_or_else(|| format!("3MF: no entry {index} in group {pid}"))
    };

    let mut objects = Vec::new();
    let mut current: Option<ThreeMfObject> = None;
    let mut object_pid: Option<String> = None;
    for tag in &tags {
        match tag.name.as_str() {
            "object" => {
                object_pid = tag.get("pid").map(str::to_string);
                let color = match (tag.get("pid"), tag.get("pindex")) {
                    (Some(pid), Some(index)) => Some(lookup(pid, index)?),
                    _ => None,
                };
                current = Some(ThreeMfObject {
                    id: tag.get("id").unwrap_or("").to_string(),
                    name: tag.get("name").unwrap_or("").to_string(),
                    vertices: Vec::new(),
                    triangles: Vec::new(),
                    colors: Vec::new(),
                    color,
                });
            }
            "vertex" => {
                let object = current.as_mut().ok_or("3MF: a vertex outside an object")?;
                let coordinate = |key: &str| -> Result<f64, String> {
                    tag.get(key)
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| format!("3MF: vertex without {key}"))
                };
                object
                    .vertices
                    .push([coordinate("x")?, coordinate("y")?, coordinate("z")?]);
            }
            "triangle" => {
                let object = current
                    .as_mut()
                    .ok_or("3MF: a triangle outside an object")?;
                let corner = |key: &str| -> Result<usize, String> {
                    tag.get(key)
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| format!("3MF: triangle without {key}"))
                };
                object
                    .triangles
                    .push([corner("v1")?, corner("v2")?, corner("v3")?]);
                // A triangle's p1 is its property (with the triangle's pid,
                // else the object's); without one it has the object's.
                let color = match tag.get("p1") {
                    Some(index) => {
                        let pid = tag
                            .get("pid")
                            .or(object_pid.as_deref())
                            .ok_or("3MF: p1 without a pid")?;
                        Some(lookup(pid, index)?)
                    }
                    None => object.color,
                };
                object.colors.push(color);
            }
            "/object" => {
                objects.push(current.take().ok_or("3MF: </object> without <object>")?);
            }
            _ => {}
        }
    }
    Ok(objects)
}

// ------------------------------------------------------------------ STEP

/// A parameter of a STEP entity instance (ISO 10303-21).
#[derive(Clone, Debug, PartialEq)]
pub enum StepValue {
    Ref(u64),
    Number(f64),
    Text(String),
    Enum(String),
    List(Vec<StepValue>),
    /// A typed parameter, `NAME(...)`.
    Typed(String, Vec<StepValue>),
    /// `$` or `*`.
    Unset,
}

/// The entity instances of a STEP file's data section: per instance, its
/// parts (one for a simple instance, several for a complex one), each a
/// type name and its parameters.
pub type StepInstances = BTreeMap<u64, Vec<(String, Vec<StepValue>)>>;

struct StepParser<'a> {
    text: &'a [u8],
    at: usize,
}

impl StepParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.at += 1;
            }
            if self.text.get(self.at..self.at + 2) == Some(b"/*") {
                let rest = self.text.get(self.at + 2..).unwrap_or(&[]);
                let end = rest.windows(2).position(|w| w == b"*/");
                self.at = end.map_or(self.text.len(), |e| self.at + 2 + e + 2);
            } else {
                return;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!(
                "STEP: `{}` expected at byte {}",
                c as char, self.at
            ))
        }
    }

    fn keyword(&mut self) -> String {
        self.skip_space();
        let start = self.at;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'!')
        {
            self.at += 1;
        }
        String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or(&[])).to_uppercase()
    }

    fn list(&mut self, depth: usize) -> Result<Vec<StepValue>, String> {
        if depth > 64 {
            return Err("STEP: parameters nested too deep".into());
        }
        self.expect(b'(')?;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b')') {
            self.at += 1;
            return Ok(items);
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b')') => {
                    self.at += 1;
                    return Ok(items);
                }
                _ => return Err(format!("STEP: `,` or `)` expected at byte {}", self.at)),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<StepValue, String> {
        self.skip_space();
        match self.peek().ok_or("STEP: a parameter expected")? {
            b'#' => {
                self.at += 1;
                let start = self.at;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.at += 1;
                }
                let digits = String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or(&[]));
                Ok(StepValue::Ref(
                    digits.parse().map_err(|_| "STEP: bad reference")?,
                ))
            }
            b'\'' => {
                self.at += 1;
                let mut out = Vec::new();
                loop {
                    let c = self.peek().ok_or("STEP: unclosed string")?;
                    self.at += 1;
                    if c == b'\'' {
                        if self.peek() == Some(b'\'') {
                            self.at += 1;
                            out.push(b'\'');
                        } else {
                            break;
                        }
                    } else {
                        out.push(c);
                    }
                }
                Ok(StepValue::Text(String::from_utf8_lossy(&out).into_owned()))
            }
            b'.' => {
                self.at += 1;
                let start = self.at;
                while self.peek().is_some_and(|c| c != b'.') {
                    self.at += 1;
                }
                let name = String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or(&[]))
                    .into_owned();
                self.expect(b'.')?;
                Ok(StepValue::Enum(name))
            }
            b'$' | b'*' => {
                self.at += 1;
                Ok(StepValue::Unset)
            }
            b'(' => Ok(StepValue::List(self.list(depth)?)),
            b'"' => {
                // A binary: kept as text.
                self.at += 1;
                let start = self.at;
                while self.peek().is_some_and(|c| c != b'"') {
                    self.at += 1;
                }
                let bits = String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or(&[]))
                    .into_owned();
                self.expect(b'"')?;
                Ok(StepValue::Text(bits))
            }
            c if c == b'-' || c == b'+' || c.is_ascii_digit() => {
                let start = self.at;
                while self.peek().is_some_and(|c| {
                    c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'E' | b'e')
                }) {
                    self.at += 1;
                }
                let number = String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or(&[]));
                Ok(StepValue::Number(
                    number
                        .parse()
                        .map_err(|_| format!("STEP: bad number `{number}`"))?,
                ))
            }
            _ => {
                let name = self.keyword();
                if name.is_empty() {
                    return Err(format!("STEP: unexpected byte at {}", self.at));
                }
                Ok(StepValue::Typed(name, self.list(depth)?))
            }
        }
    }
}

/// The entity instances of a STEP file (ISO 10303-21 clear text).
pub fn step_instances(text: &str) -> Result<StepInstances, String> {
    let data = text.find("DATA;").ok_or("STEP: no DATA section")?;
    let mut parser = StepParser {
        text: text.as_bytes(),
        at: data + "DATA;".len(),
    };
    let mut instances = BTreeMap::new();
    loop {
        parser.skip_space();
        if parser.text.get(parser.at..parser.at + 6) == Some(b"ENDSEC") {
            break;
        }
        parser.expect(b'#')?;
        let start = parser.at;
        while parser.peek().is_some_and(|c| c.is_ascii_digit()) {
            parser.at += 1;
        }
        let id: u64 = String::from_utf8_lossy(parser.text.get(start..parser.at).unwrap_or(&[]))
            .parse()
            .map_err(|_| format!("STEP: bad instance name at byte {start}"))?;
        parser.expect(b'=')?;
        parser.skip_space();
        let mut parts = Vec::new();
        if parser.peek() == Some(b'(') {
            // A complex instance: `(A(...) B(...))`.
            parser.at += 1;
            loop {
                parser.skip_space();
                if parser.peek() == Some(b')') {
                    parser.at += 1;
                    break;
                }
                let name = parser.keyword();
                if name.is_empty() {
                    return Err(format!("STEP: #{id}: a part without a name"));
                }
                parts.push((name, parser.list(0)?));
            }
        } else {
            let name = parser.keyword();
            parts.push((name, parser.list(0)?));
        }
        parser.expect(b';')?;
        instances.insert(id, parts);
    }
    Ok(instances)
}

/// The parameters of `#id` when it is (or has a part) of type `name`.
pub fn step_params<'a>(
    instances: &'a StepInstances,
    id: u64,
    name: &str,
) -> Option<&'a [StepValue]> {
    instances
        .get(&id)?
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, params)| params.as_slice())
}

fn refs(value: Option<&StepValue>) -> Vec<u64> {
    match value {
        Some(StepValue::Ref(id)) => vec![*id],
        Some(StepValue::List(items)) => items.iter().flat_map(|v| refs(Some(v))).collect(),
        _ => Vec::new(),
    }
}

/// The first `COLOUR_RGB` reachable from `id` through references, in
/// parameter order: the colour a style assignment gives (presentation
/// style assignment, surface style usage, side style, fill area, fill
/// area style, its colour). Bounded in depth.
fn first_colour(instances: &StepInstances, id: u64, depth: usize) -> Option<[f64; 3]> {
    if depth > 16 {
        return None;
    }
    if let Some(params) = step_params(instances, id, "COLOUR_RGB") {
        let number = |k: usize| match params.get(k) {
            Some(StepValue::Number(v)) => Some(*v),
            _ => None,
        };
        return Some([number(1)?, number(2)?, number(3)?]);
    }
    for (_, params) in instances.get(&id)? {
        for param in params {
            for next in refs(Some(param)) {
                if let Some(colour) = first_colour(instances, next, depth + 1) {
                    return Some(colour);
                }
            }
        }
    }
    None
}

/// The colour (0 to 1 per channel) each styled item of a STEP file is
/// given: per `STYLED_ITEM` (and `OVER_RIDING_STYLED_ITEM`), its item
/// and the colour its styles reach.
pub fn step_item_colours(instances: &StepInstances) -> BTreeMap<u64, [f64; 3]> {
    let mut colours = BTreeMap::new();
    for &id in instances.keys() {
        let Some(params) = step_params(instances, id, "STYLED_ITEM")
            .or_else(|| step_params(instances, id, "OVER_RIDING_STYLED_ITEM"))
        else {
            continue;
        };
        // (name, styles, item)
        let Some(StepValue::Ref(item)) = params.get(2) else {
            continue;
        };
        let colour = refs(params.get(1))
            .into_iter()
            .find_map(|style| first_colour(instances, style, 0));
        if let Some(colour) = colour {
            colours.insert(*item, colour);
        }
    }
    colours
}

/// A face of a STEP solid or shell model, with what is needed to tell it
/// apart and its colour.
#[derive(Clone, Debug, PartialEq)]
pub struct StepFace {
    pub id: u64,
    /// The type of its surface (`PLANE`, `CYLINDRICAL_SURFACE`, ...).
    pub surface: String,
    /// A plane's point and its normal as the face faces (turned when the
    /// face's sense is false).
    pub plane: Option<([f64; 3], [f64; 3])>,
    /// Its own colour, else its solid's (0 to 1 per channel).
    pub colour: Option<[f64; 3]>,
}

/// A solid (`MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`) or shell model
/// (`SHELL_BASED_SURFACE_MODEL`) of a STEP file with its faces.
#[derive(Clone, Debug, PartialEq)]
pub struct StepSolid {
    pub id: u64,
    pub name: String,
    pub colour: Option<[f64; 3]>,
    pub faces: Vec<StepFace>,
}

fn point3(instances: &StepInstances, id: u64, name: &str) -> Option<[f64; 3]> {
    let params = step_params(instances, id, name)?;
    let StepValue::List(items) = params.get(1)? else {
        return None;
    };
    let number = |k: usize| match items.get(k) {
        Some(StepValue::Number(v)) => Some(*v),
        _ => None,
    };
    Some([number(0)?, number(1)?, number(2)?])
}

/// The solids and shell models of a STEP file, each face with its colour.
pub fn read_step_colours(text: &str) -> Result<Vec<StepSolid>, String> {
    let instances = step_instances(text)?;
    let colours = step_item_colours(&instances);
    let mut solids = Vec::new();
    for &id in instances.keys() {
        let (params, shell_index) =
            if let Some(p) = step_params(&instances, id, "MANIFOLD_SOLID_BREP") {
                (p, 1)
            } else if let Some(p) = step_params(&instances, id, "BREP_WITH_VOIDS") {
                (p, 1)
            } else if let Some(p) = step_params(&instances, id, "SHELL_BASED_SURFACE_MODEL") {
                (p, 1)
            } else {
                continue;
            };
        let name = match params.first() {
            Some(StepValue::Text(name)) => name.clone(),
            _ => String::new(),
        };
        let colour = colours.get(&id).copied();
        let mut shells = refs(params.get(shell_index));
        // A solid's cavities, through their oriented shells.
        if let Some(StepValue::List(voids)) = params.get(2) {
            for void in refs(Some(&StepValue::List(voids.clone()))) {
                if let Some(oriented) = step_params(&instances, void, "ORIENTED_CLOSED_SHELL") {
                    shells.extend(refs(oriented.get(2)));
                }
            }
        }
        let mut faces = Vec::new();
        for shell in shells {
            let face_ids = step_params(&instances, shell, "CLOSED_SHELL")
                .or_else(|| step_params(&instances, shell, "OPEN_SHELL"))
                .map(|p| refs(p.get(1)))
                .ok_or_else(|| format!("STEP: #{shell} is not a shell"))?;
            for face in face_ids {
                let params = step_params(&instances, face, "ADVANCED_FACE")
                    .or_else(|| step_params(&instances, face, "FACE_SURFACE"))
                    .ok_or_else(|| format!("STEP: #{face} is not a face"))?;
                let surface_id = match params.get(2) {
                    Some(StepValue::Ref(s)) => *s,
                    _ => return Err(format!("STEP: #{face} has no surface")),
                };
                let sense = !matches!(params.get(3), Some(StepValue::Enum(e)) if e == "F");
                let surface = instances
                    .get(&surface_id)
                    .and_then(|parts| parts.first())
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                let plane = step_params(&instances, surface_id, "PLANE").and_then(|p| {
                    let StepValue::Ref(placement) = p.get(1)? else {
                        return None;
                    };
                    let axes = step_params(&instances, *placement, "AXIS2_PLACEMENT_3D")?;
                    let (StepValue::Ref(origin), StepValue::Ref(axis)) =
                        (axes.get(1)?, axes.get(2)?)
                    else {
                        return None;
                    };
                    let origin = point3(&instances, *origin, "CARTESIAN_POINT")?;
                    let normal = point3(&instances, *axis, "DIRECTION")?;
                    let normal = if sense { normal } else { normal.map(|c| -c) };
                    Some((origin, normal))
                });
                faces.push(StepFace {
                    id: face,
                    surface,
                    plane,
                    colour: colours.get(&face).copied().or(colour),
                });
            }
        }
        solids.push(StepSolid {
            id,
            name,
            colour,
            faces,
        });
    }
    Ok(solids)
}

// ------------------------------------------------------------- OBJ + MTL

/// A material of an MTL library.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MtlMaterial {
    /// `Kd`, 0 to 1 per channel.
    pub diffuse: Option<[f64; 3]>,
    /// `Pr`.
    pub roughness: Option<f64>,
    /// `Pm`.
    pub metallic: Option<f64>,
}

/// The materials of an MTL library by name.
pub fn read_mtl(text: &str) -> Result<BTreeMap<String, MtlMaterial>, String> {
    let mut materials = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        let numbers: Vec<f64> = words.clone().filter_map(|w| w.parse().ok()).collect();
        match keyword {
            "newmtl" => {
                let name = words.next().ok_or("MTL: newmtl without a name")?;
                materials.insert(name.to_string(), MtlMaterial::default());
                current = Some(name.to_string());
            }
            "Kd" | "Pr" | "Pm" => {
                let name = current.as_ref().ok_or("MTL: a statement before newmtl")?;
                let entry = materials.entry(name.clone()).or_default();
                match (keyword, numbers.as_slice()) {
                    ("Kd", [r, g, b]) => entry.diffuse = Some([*r, *g, *b]),
                    ("Pr", [r]) => entry.roughness = Some(*r),
                    ("Pm", [m]) => entry.metallic = Some(*m),
                    _ => return Err(format!("MTL: bad `{line}`")),
                }
            }
            _ => {}
        }
    }
    Ok(materials)
}

/// An object of an OBJ file with its triangles' materials.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ObjObject {
    pub name: String,
    /// Corners, by index into the file's vertices (from 0).
    pub triangles: Vec<[usize; 3]>,
    /// Per triangle: the name of the material in use (the last `usemtl`
    /// before it, in this object or before).
    pub materials: Vec<Option<String>>,
}

/// An OBJ file read: its material libraries, vertices and objects (faces
/// before the first `o` go to an object without a name). Polygons are fanned
/// into triangles.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ObjFile {
    pub libraries: Vec<String>,
    pub vertices: Vec<[f64; 3]>,
    pub objects: Vec<ObjObject>,
}

pub fn read_obj(text: &str) -> Result<ObjFile, String> {
    let mut file = ObjFile::default();
    let mut material: Option<String> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        match words.next() {
            Some("mtllib") => file.libraries.extend(words.map(str::to_string)),
            Some("usemtl") => material = words.next().map(str::to_string),
            Some("o") => file.objects.push(ObjObject {
                name: words.collect::<Vec<_>>().join(" "),
                ..ObjObject::default()
            }),
            Some("v") => {
                let c: Vec<f64> = words.filter_map(|w| w.parse().ok()).collect();
                match c.as_slice() {
                    [x, y, z, ..] => file.vertices.push([*x, *y, *z]),
                    _ => return Err(format!("OBJ: bad `{line}`")),
                }
            }
            Some("f") => {
                let count = file.vertices.len();
                let corners: Vec<usize> = words
                    .map(|w| {
                        let index: i64 = w
                            .split('/')
                            .next()
                            .and_then(|i| i.parse().ok())
                            .ok_or_else(|| format!("OBJ: bad corner `{w}`"))?;
                        // From 1, or negative from the end.
                        let at = if index < 0 {
                            i64::try_from(count).map_err(|e| e.to_string())? + index
                        } else {
                            index - 1
                        };
                        usize::try_from(at)
                            .ok()
                            .filter(|&at| at < count)
                            .ok_or_else(|| format!("OBJ: corner `{w}` out of range"))
                    })
                    .collect::<Result<_, String>>()?;
                if file.objects.is_empty() {
                    file.objects.push(ObjObject::default());
                }
                let object = file.objects.last_mut().ok_or("OBJ: no object")?;
                let first = *corners.first().ok_or("OBJ: an empty face")?;
                for pair in corners.get(1..).unwrap_or(&[]).windows(2) {
                    if let [b, c] = pair {
                        object.triangles.push([first, *b, *c]);
                        object.materials.push(material.clone());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(file)
}

/// 0 to 1 to a byte, rounded: what an sRGB channel written as a fraction
/// of 255 reads back as.
pub fn unit_to_byte(value: f64) -> Option<u8> {
    let scaled = (value * 255.0).round();
    // Within 0..=255 before the cast.
    (value.is_finite() && (0.0..=255.0).contains(&scaled)).then_some(scaled as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_tags_read_attributes_entities_and_namespaces() {
        let tags = xml_tags(
            "<?xml version=\"1.0\"?><!-- a > b --><m:model a='1 &amp; 2' b=\"x>y\"><m:base displaycolor=\"#0A0B0C\"/></m:model>",
        )
        .unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name, "model");
        assert_eq!(tags[0].get("a"), Some("1 & 2"));
        assert_eq!(tags[0].get("b"), Some("x>y"));
        assert_eq!(tags[1].get("displaycolor"), Some("#0A0B0C"));
        assert_eq!(tags[2].name, "/model");
        assert_eq!(srgb_hex("#0A0B0CFF").unwrap(), [10, 11, 12]);
        assert!(srgb_hex("0A0B0C").is_err());
    }

    #[test]
    fn step_instances_read_simple_and_complex_instances() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
            #1=COLOUR_RGB('',1.,0.5,2.5E-1);\n\
            #2=FILL_AREA_STYLE_COLOUR('',#1);\n\
            #3=STYLED_ITEM('it''s',(#2),#4);\n\
            #4=(NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) LENGTH_UNIT());\n\
            /* a comment */ #5=X(LENGTH_MEASURE(2.),$,(#1,#2));\nENDSEC;\nEND-ISO-10303-21;\n";
        let instances = step_instances(text).unwrap();
        assert_eq!(instances.len(), 5);
        assert_eq!(
            step_params(&instances, 1, "COLOUR_RGB").unwrap()[3],
            StepValue::Number(0.25)
        );
        assert_eq!(
            step_params(&instances, 3, "STYLED_ITEM").unwrap()[0],
            StepValue::Text("it's".into())
        );
        assert_eq!(
            step_params(&instances, 4, "SI_UNIT").unwrap()[0],
            StepValue::Enum("MILLI".into())
        );
        assert_eq!(
            step_item_colours(&instances).get(&4),
            Some(&[1.0, 0.5, 0.25])
        );
        assert!(step_instances("DATA;\n#1=A(;\nENDSEC;").is_err());
    }

    #[test]
    fn obj_and_mtl_read_materials_per_triangle() {
        let mtl =
            read_mtl("newmtl a\nKd 1 0 0.5\nPr 0.25 # rough\nnewmtl b\nKd 0 0 0\nPm 1\n").unwrap();
        assert_eq!(mtl["a"].diffuse, Some([1.0, 0.0, 0.5]));
        assert_eq!(mtl["a"].roughness, Some(0.25));
        assert_eq!(mtl["b"].metallic, Some(1.0));
        let obj = read_obj(
            "mtllib x.mtl\no one\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nusemtl a\nf 1 2 3 4\no two\nf -1 -2 -3\nusemtl b\nf 1/1/1 2//2 3\n",
        )
        .unwrap();
        assert_eq!(obj.libraries, ["x.mtl"]);
        assert_eq!(obj.objects[0].triangles, [[0, 1, 2], [0, 2, 3]]);
        assert_eq!(obj.objects[1].triangles, [[3, 2, 1], [0, 1, 2]]);
        assert_eq!(
            obj.objects[1].materials,
            [Some("a".to_string()), Some("b".to_string())]
        );
        assert!(read_obj("v 0 0 0\nf 1 2 3\n").is_err());
        assert_eq!(unit_to_byte(0.784314), Some(200));
        assert_eq!(unit_to_byte(1.5), None);
    }
}
