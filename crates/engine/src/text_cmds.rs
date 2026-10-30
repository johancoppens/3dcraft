//! Fonts and texts: `font.list`, `font.add`, `sketch.text`,
//! `sketch.textOnCurve`, `sketch.textEdit`, and the measure `textExtent`
//! (docs/design/text.md).

use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::sketch_cmds::{PointArg, entity_id, sketch_ref, sketch_result};
use crate::verify_cmds::{Ctx, Dim, Outcome};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    Blob, DocError, DocFont, Document, EntityId, Expr, FeatureId, Geometry, MAX_DOC_FONTS,
    SketchError, SketchText, TextAlign, TextAnchor, TextPath, TextSide, TextSizeOf, TextTemplate,
};
use craft3d_font::{FontFile, STROKE};
use craft3d_regen::SketchResult;
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("font.list", "List Fonts", r#"{}"#,
        "Lists the fonts a text can be set in: the built-in ones (sans: Noto Sans; mono: JetBrains Mono; stroke: the single-stroke font for engraving, CNC, lasers and plotters, open strokes rather than outlines), whether each is installed here, and the document's own fonts with their family, hash, embedding permission and whether the document keeps their bytes.", doc: false, font_list),
    spec!("font.add", "Add Font", r#"{"path": string, "builtin": "sans|mono", "name": string = the font's family, "replace": boolean = false}"#,
        "Adds a font to the document, from a font file (path, OpenType or TrueType) or a built-in one (builtin), under name, which texts use as their font. The document keeps the font's bytes with their SHA-256, so it opens the same everywhere; a font whose embedding permission (OS/2 fsType) is restricted, preview-and-print or bitmap-only is not kept: the document records that it needs that font by name and hash, the bytes are used for this session only, and a reopened document asks for the font again (font.add it once more). A name that is taken by another font is refused unless replace. Returns the font's family, style, embedding and whether it is kept.", doc: true, font_add),
    spec!("sketch.text", "Sketch Text", r#"{"sketch": id or name, "text": string, "size": length expr, "font": string = "sans", "sizeOf": "cap|em" = "cap", "at": point id or [x, y] = [0, 0], "anchor": "baseline|top|middle|bottom" = "baseline", "align": "left|center|right" = "left", "rotation": angle expr = 0, "lineSpacing": number expr = 1, "width": length expr, "construction": boolean = false}"#,
        "Adds a text to a sketch: closed profiles (the outlines of its glyphs, overlapping contours united by the font's fill rule) that extrude, cut and emboss as any profile does; in the stroke font, open strokes for engraving and DXF or SVG export. text may hold formulas in braces, evaluated at every regeneration (\"Part {partNo}\"; a length is written in the document's unit, an angle in its angle unit; {{ and }} write a brace). size is the height of the capitals (sizeOf cap) or the em (sizeOf em). The text stands on the point at (an existing point, which constraints can then place, or a new one), its first baseline, top, middle or last baseline there (anchor), its lines left, centred or right of it (align), turned by rotation; lines break at newlines, and with width also at spaces (a longer word between letters); lineSpacing scales the font's line height. A built-in font is added to the document when first used. Each contour is a curve named e7.glyph(i,U+XXXX).contour(j): the text entity, the place of the character in the text (every character counts), the character and the contour; a face swept from it is F2:side(e7.glyph(0,U+0041).contour(0)). A changed text fails references to a glyph it no longer has; it never hands them another. Returns the text, as evaluated, its lines, width and height.", doc: true, sketch_text),
    spec!("sketch.textOnCurve", "Sketch Text on Curve", r#"{"sketch": id or name, "curve": entity id, "text": string, "size": length expr, "font": string = "sans", "sizeOf": "cap|em" = "cap", "start": length expr = 0, "offset": length expr = 0, "side": "left|right" = "left", "reversed": boolean = false, "align": "left|center|right" = "left", "anchor": "baseline|top|middle|bottom" = "baseline", "lineSpacing": number expr = 1, "construction": boolean = false}"#,
        "Adds a text that follows a line, arc, circle or spline of the sketch (curve): each glyph is set upright on the curve, its baseline along the tangent at its middle, so a glyph keeps its exact shape. start is the length along the curve (from its start; a circle starts at angle 0 and runs counter-clockwise) where the text starts (align center: where its middle is; right: where it ends); offset lifts the baseline off the curve, square to it. side left stands the text on the left of the curve as it runs (outside a counter-clockwise circle), right on its right, the glyphs then running the other way so they still read; reversed runs the curve the other way. On an open curve the text must fit between its ends. Otherwise as sketch.text, whose names and fonts it has.", doc: true, sketch_text_on_curve),
    spec!("sketch.textEdit", "Edit Sketch Text", r#"{"sketch": id or name, "entity": text id, "text": string, "font": string, "size": length expr, "sizeOf": "cap|em", "anchor": "baseline|top|middle|bottom", "align": "left|center|right", "rotation": angle expr, "lineSpacing": number expr, "width": length expr or null, "start": length expr, "offset": length expr, "side": "left|right", "reversed": boolean}"#,
        "Changes a text of a sketch: any of the fields sketch.text and sketch.textOnCurve set (width null removes it). The glyphs are laid out again; a face or edge named after a glyph the text no longer has fails by its name.", doc: true, sketch_text_edit),
];

/// The names a size, an anchor, an alignment and a side are given by.
fn size_of(p: &Params, command: &str) -> Result<Option<TextSizeOf>, CommandError> {
    Ok(match p.str("sizeOf")? {
        None => None,
        Some("cap") => Some(TextSizeOf::Cap),
        Some("em") => Some(TextSizeOf::Em),
        Some(other) => {
            return fail(format!(
                "{command}: `sizeOf` is \"cap\" or \"em\", got {:?}",
                other.chars().take(20).collect::<String>()
            ));
        }
    })
}

fn anchor(p: &Params, command: &str) -> Result<Option<TextAnchor>, CommandError> {
    Ok(match p.str("anchor")? {
        None => None,
        Some("baseline") => Some(TextAnchor::Baseline),
        Some("top") => Some(TextAnchor::Top),
        Some("middle") => Some(TextAnchor::Middle),
        Some("bottom") => Some(TextAnchor::Bottom),
        Some(other) => {
            return fail(format!(
                "{command}: `anchor` is baseline, top, middle or bottom, got {:?}",
                other.chars().take(20).collect::<String>()
            ));
        }
    })
}

fn align(p: &Params, command: &str) -> Result<Option<TextAlign>, CommandError> {
    Ok(match p.str("align")? {
        None => None,
        Some("left") => Some(TextAlign::Left),
        Some("center" | "centre") => Some(TextAlign::Center),
        Some("right") => Some(TextAlign::Right),
        Some(other) => {
            return fail(format!(
                "{command}: `align` is left, center or right, got {:?}",
                other.chars().take(20).collect::<String>()
            ));
        }
    })
}

fn side(p: &Params, command: &str) -> Result<Option<TextSide>, CommandError> {
    Ok(match p.str("side")? {
        None => None,
        Some("left") => Some(TextSide::Left),
        Some("right") => Some(TextSide::Right),
        Some(other) => {
            return fail(format!(
                "{command}: `side` is left or right, got {:?}",
                other.chars().take(20).collect::<String>()
            ));
        }
    })
}

fn template(
    p: &Params,
    command: &str,
    required: bool,
) -> Result<Option<TextTemplate>, CommandError> {
    let text = if required {
        Some(p.required_str("text")?)
    } else {
        p.str("text")?
    };
    text.map(|t| {
        TextTemplate::parse(t).map_err(|e| CommandError::new(format!("{command}: `text`: {e}")))
    })
    .transpose()
}

fn font_name(p: &Params) -> Result<String, CommandError> {
    let font = p.str("font")?.unwrap_or("sans");
    if font.trim().is_empty() || font.len() > craft3d_doc::MAX_FONT_NAME {
        return Err(p.missing("font"));
    }
    Ok(font.to_string())
}

/// A font read for the document: its record, without a name yet.
fn read_font(command: &str, bytes: Vec<u8>) -> Result<DocFont, CommandError> {
    let info = {
        let file =
            FontFile::parse(&bytes).map_err(|e| CommandError::new(format!("{command}: {e}")))?;
        file.info()
    };
    let blob = Blob::new(bytes);
    let keep = info.embedding.allows_storing();
    Ok(DocFont {
        name: info.family.clone(),
        family: info.family,
        style: info.subfamily,
        sha256: blob.sha256(),
        embedding: info.embedding.name().to_string(),
        blob: keep.then(|| blob.clone()),
        session: (!keep).then_some(blob),
    })
}

/// Puts `font` into the document: a new name, the same font again (a
/// font that may not be kept gets its bytes for this session), or, with
/// `replace`, another font under a name that is taken.
fn put_font(doc: &mut Document, font: DocFont, replace: bool) -> Result<(), DocError> {
    if let Some(have) = doc.fonts.iter_mut().find(|f| f.name == font.name) {
        if have.sha256 == font.sha256 {
            if have.bytes().is_none() {
                have.session = font.session;
            }
            return Ok(());
        }
        if !replace {
            return Err(DocError::Invalid(format!(
                "a font is called `{}` already ({}, sha256 {}); give another name, or replace",
                font.name, have.family, have.sha256
            )));
        }
        *have = font;
        return Ok(());
    }
    if doc.fonts.len() >= MAX_DOC_FONTS {
        return Err(DocError::Invalid(format!(
            "a document keeps at most {MAX_DOC_FONTS} fonts"
        )));
    }
    doc.fonts.push(font);
    Ok(())
}

/// The built-in font `name` as a font of the document, when a text asks
/// for one the document does not have yet.
fn builtin_font(
    command: &str,
    doc: &Document,
    name: &str,
) -> Result<Option<DocFont>, CommandError> {
    if name == STROKE || doc.fonts.iter().any(|f| f.name == name) {
        return Ok(None);
    }
    if craft3d_font::builtin(name).is_none() {
        let known: Vec<&str> = doc
            .fonts
            .iter()
            .map(|f| f.name.as_str())
            .chain(craft3d_font::BUILTIN.iter().map(|f| f.name))
            .chain([STROKE])
            .collect();
        return fail(format!(
            "{command}: there is no font `{}`; the fonts are {} (font.add adds one)",
            name.chars().take(60).collect::<String>(),
            known.join(", ")
        ));
    }
    let bytes =
        crate::fonts::bytes(name).map_err(|e| CommandError::new(format!("{command}: {e}")))?;
    let mut font = read_font(command, bytes.to_vec())?;
    font.name = name.to_string();
    Ok(Some(font))
}

fn font_list(session: &mut Session, params: &Value) -> Run {
    Params::new("font.list", params, &[])?;
    let mut builtin: Vec<Value> = craft3d_font::BUILTIN
        .iter()
        .map(|f| {
            let installed = crate::fonts::ready(f.name) || crate::fonts::bytes(f.name).is_ok();
            json!({
                "name": f.name,
                "file": f.file,
                "installed": installed,
                "licence": "SIL Open Font Licence 1.1",
            })
        })
        .collect();
    builtin.push(json!({
        "name": STROKE,
        "installed": true,
        "singleStroke": true,
        "licence": "drawn for 3DCraft (MIT)",
    }));
    let document: Vec<Value> = match session.document() {
        Ok(open) => open
            .history
            .current()
            .fonts
            .iter()
            .map(|f| {
                json!({
                    "name": f.name,
                    "family": f.family,
                    "style": f.style,
                    "sha256": f.sha256,
                    "embedding": f.embedding,
                    "kept": f.blob.is_some(),
                    "available": f.bytes().is_some(),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    Ok(json!({"builtin": builtin, "document": document}))
}

fn font_add(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "font.add";
    let p = Params::new(COMMAND, params, &["path", "builtin", "name", "replace"])?;
    let replace = p.bool("replace")?.unwrap_or(false);
    let mut font = match (p.str("path")?, p.str("builtin")?) {
        (Some(_), Some(_)) => {
            return fail(format!("{COMMAND}: give a `path` or a `builtin`, not both"));
        }
        (None, None) => {
            return fail(format!(
                "{COMMAND}: give a font file (`path`) or a built-in font (`builtin`: sans or mono)"
            ));
        }
        (Some(path), None) => {
            let resolved = session.workspace.resolve_read(path)?;
            let limit = u64::try_from(craft3d_font::MAX_FONT_BYTES).unwrap_or(u64::MAX);
            let bytes = session
                .read_input(path, &resolved, limit)
                .map_err(|e| CommandError::new(format!("{COMMAND}: {e}")))?;
            read_font(COMMAND, bytes)?
        }
        (None, Some(name)) => {
            if craft3d_font::builtin(name).is_none() {
                return fail(format!(
                    "{COMMAND}: `builtin` is sans or mono (the stroke font needs no adding), got {:?}",
                    name.chars().take(20).collect::<String>()
                ));
            }
            let bytes = crate::fonts::bytes(name)
                .map_err(|e| CommandError::new(format!("{COMMAND}: {e}")))?;
            let mut font = read_font(COMMAND, bytes.to_vec())?;
            font.name = name.to_string();
            font
        }
    };
    if let Some(name) = p.str("name")? {
        if name.trim().is_empty() || name.len() > craft3d_doc::MAX_FONT_NAME || name == STROKE {
            return fail(format!(
                "{COMMAND}: `name` is 1 to {} bytes and not `{STROKE}`",
                craft3d_doc::MAX_FONT_NAME
            ));
        }
        font.name = name.to_string();
    }
    if font.name.trim().is_empty() {
        font.name = format!("font-{}", font.sha256.chars().take(8).collect::<String>());
    }
    let summary = json!({
        "name": font.name,
        "family": font.family,
        "style": font.style,
        "sha256": font.sha256,
        "embedding": font.embedding,
        "kept": font.blob.is_some(),
    });
    let kept = font.blob.is_some();
    let embedding = font.embedding.clone();
    edit(session, COMMAND, "Add font", |doc| {
        put_font(doc, font, replace)
    })?;
    let mut out = summary;
    if !kept && let Some(map) = out.as_object_mut() {
        map.insert(
            "warning".into(),
            format!(
                "the font's embedding permission is {embedding}: the document records that it needs this font, but does not keep it; a reopened document asks for it again"
            )
            .into(),
        );
    }
    Ok(out)
}

/// The fields of a new text from the parameters.
fn text_fields(
    p: &Params,
    command: &str,
) -> Result<(TextTemplate, String, Expr, TextSizeOf), CommandError> {
    let text = template(p, command, true)?.unwrap_or_default();
    let font = font_name(p)?;
    let size = Expr::new(p.expr("size")?.ok_or_else(|| p.missing("size"))?);
    let size_of = size_of(p, command)?.unwrap_or_default();
    Ok((text, font, size, size_of))
}

/// What a command that made or changed a text returns about it.
fn laid_out(solved: Option<&SketchResult>, id: EntityId) -> Value {
    let Some(laid) = solved.and_then(|s| s.text(id)) else {
        return Value::Null;
    };
    json!({
        "text": laid.text,
        "lines": laid.lines,
        "width": laid.width,
        "height": laid.height,
        "capHeight": laid.cap_height,
        "curves": laid.curves.len(),
    })
}

fn finish(session: &mut Session, sketch: FeatureId, before: &[FeatureId], text: EntityId) -> Run {
    let regen = session.document_mut()?.regen();
    let solved = regen.feature(sketch).and_then(|f| f.sketch.as_deref());
    let own = json!({
        "entity": text.to_string(),
        "layout": laid_out(solved, text),
    });
    sketch_result(session, sketch, Some(before), own)
}

fn sketch_text(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.text";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "text",
            "size",
            "font",
            "sizeOf",
            "at",
            "anchor",
            "align",
            "rotation",
            "lineSpacing",
            "width",
            "construction",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let (text, font, size, size_of) = text_fields(&p, COMMAND)?;
    let typed = session.typed();
    let at = match p.get("at") {
        Some(v) => PointArg::parse(&typed, COMMAND, "at", v)?,
        None => PointArg::New(0.0, 0.0, Default::default()),
    };
    let anchor = anchor(&p, COMMAND)?.unwrap_or_default();
    let align = align(&p, COMMAND)?.unwrap_or_default();
    let rotation = Expr::new(p.expr("rotation")?.unwrap_or_else(|| "0".into()));
    let line_spacing = Expr::new(p.expr("lineSpacing")?.unwrap_or_else(|| "1".into()));
    let width = p.expr("width")?.map(Expr::new);
    let construction = p.bool("construction")?.unwrap_or(false);
    let added = builtin_font(COMMAND, session.document()?.history.current(), &font)?;
    let before = crate::commands::failing(session)?;
    let entity = edit(session, COMMAND, "Add text", |doc| {
        if let Some(f) = added {
            put_font(doc, f, false)?;
        }
        doc.edit_sketch(id, |s| {
            let point = at.place(s)?;
            s.add_text(
                SketchText {
                    text,
                    font,
                    size,
                    size_of,
                    at: point,
                    anchor,
                    align,
                    rotation,
                    line_spacing,
                    width,
                    path: None,
                },
                construction,
            )
        })
    })?;
    finish(session, id, &before, entity)
}

fn sketch_text_on_curve(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.textOnCurve";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "curve",
            "text",
            "size",
            "font",
            "sizeOf",
            "start",
            "offset",
            "side",
            "reversed",
            "align",
            "anchor",
            "lineSpacing",
            "construction",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let curve = entity_id(
        COMMAND,
        "curve",
        p.get("curve").ok_or_else(|| p.missing("curve"))?,
    )?;
    let (text, font, size, size_of) = text_fields(&p, COMMAND)?;
    let path = TextPath {
        curve,
        start: Expr::new(p.expr("start")?.unwrap_or_else(|| "0".into())),
        offset: Expr::new(p.expr("offset")?.unwrap_or_else(|| "0".into())),
        side: side(&p, COMMAND)?.unwrap_or_default(),
        reversed: p.bool("reversed")?.unwrap_or(false),
    };
    let anchor = anchor(&p, COMMAND)?.unwrap_or_default();
    let align = align(&p, COMMAND)?.unwrap_or_default();
    let line_spacing = Expr::new(p.expr("lineSpacing")?.unwrap_or_else(|| "1".into()));
    let construction = p.bool("construction")?.unwrap_or(false);
    let added = builtin_font(COMMAND, session.document()?.history.current(), &font)?;
    let before = crate::commands::failing(session)?;
    let entity = edit(session, COMMAND, "Add text on curve", |doc| {
        if let Some(f) = added {
            put_font(doc, f, false)?;
        }
        doc.edit_sketch(id, |s| {
            // The text is placed by its curve; its point is the curve's
            // first (a circle's or arc's centre, a line's start).
            let at = s
                .entity(curve)?
                .geometry
                .points()
                .first()
                .copied()
                .ok_or_else(|| SketchError::Invalid(format!("{curve} has no points")))?;
            s.add_text(
                SketchText {
                    text,
                    font,
                    size,
                    size_of,
                    at,
                    anchor,
                    align,
                    rotation: Expr::new("0"),
                    line_spacing,
                    width: None,
                    path: Some(path),
                },
                construction,
            )
        })
    })?;
    finish(session, id, &before, entity)
}

fn sketch_text_edit(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.textEdit";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch",
            "entity",
            "text",
            "font",
            "size",
            "sizeOf",
            "anchor",
            "align",
            "rotation",
            "lineSpacing",
            "width",
            "start",
            "offset",
            "side",
            "reversed",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let entity = entity_id(
        COMMAND,
        "entity",
        p.get("entity").ok_or_else(|| p.missing("entity"))?,
    )?;
    let text = template(&p, COMMAND, false)?;
    let font = p.has("font").then(|| font_name(&p)).transpose()?;
    let size = p.expr("size")?.map(Expr::new);
    let size_of = size_of(&p, COMMAND)?;
    let anchor = anchor(&p, COMMAND)?;
    let align = align(&p, COMMAND)?;
    let rotation = p.expr("rotation")?.map(Expr::new);
    let line_spacing = p.expr("lineSpacing")?.map(Expr::new);
    let width = match p.get("width") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => Some(Some(Expr::new(p.expr("width")?.unwrap_or_default()))),
    };
    let start = p.expr("start")?.map(Expr::new);
    let offset = p.expr("offset")?.map(Expr::new);
    let side = side(&p, COMMAND)?;
    let reversed = p.bool("reversed")?;
    let added = match &font {
        Some(f) => builtin_font(COMMAND, session.document()?.history.current(), f)?,
        None => None,
    };
    let before = crate::commands::failing(session)?;
    edit(session, COMMAND, "Edit text", |doc| {
        if let Some(f) = added {
            put_font(doc, f, false)?;
        }
        doc.edit_sketch(id, |s| {
            let mut t = match &s.entity(entity)?.geometry {
                Geometry::Text(t) => (**t).clone(),
                other => {
                    return Err(SketchError::Invalid(format!(
                        "{entity} is a {}, not a text",
                        other.type_name()
                    )));
                }
            };
            let along = [&start, &offset].iter().any(|v| v.is_some())
                || side.is_some()
                || reversed.is_some();
            if along && t.path.is_none() {
                return Err(SketchError::Invalid(format!(
                    "{entity} is not along a curve: start, offset, side and reversed are for a text made by sketch.textOnCurve"
                )));
            }
            if let Some(v) = text {
                t.text = v;
            }
            if let Some(v) = font {
                t.font = v;
            }
            if let Some(v) = size {
                t.size = v;
            }
            if let Some(v) = size_of {
                t.size_of = v;
            }
            if let Some(v) = anchor {
                t.anchor = v;
            }
            if let Some(v) = align {
                t.align = v;
            }
            if let Some(v) = rotation {
                t.rotation = v;
            }
            if let Some(v) = line_spacing {
                t.line_spacing = v;
            }
            if let Some(v) = width {
                t.width = v;
            }
            if let Some(path) = &mut t.path {
                if let Some(v) = start {
                    path.start = v;
                }
                if let Some(v) = offset {
                    path.offset = v;
                }
                if let Some(v) = side {
                    path.side = v;
                }
                if let Some(v) = reversed {
                    path.reversed = v;
                }
            }
            if let Some(fault) = s.text_fault(&t) {
                return Err(SketchError::Invalid(fault));
            }
            s.entity_mut(entity)?.geometry = Geometry::Text(Box::new(t));
            Ok(())
        })
    })?;
    finish(session, id, &before, entity)
}

/// A text as `sketch.inspect` writes it: its definition and, when its
/// sketch solved, its layout and glyphs.
pub(crate) fn text_json(id: EntityId, text: &SketchText, solved: Option<&SketchResult>) -> Value {
    let mut out = json!({
        "template": text.text.to_string(),
        "font": text.font,
        "size": text.size.source(),
        "sizeOf": match text.size_of { TextSizeOf::Cap => "cap", TextSizeOf::Em => "em" },
        "at": text.at.to_string(),
        "anchor": match text.anchor {
            TextAnchor::Baseline => "baseline",
            TextAnchor::Top => "top",
            TextAnchor::Middle => "middle",
            TextAnchor::Bottom => "bottom",
        },
        "align": match text.align {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        },
        "rotation": text.rotation.source(),
        "lineSpacing": text.line_spacing.source(),
    });
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    if let Some(w) = &text.width {
        map.insert("width".into(), w.source().into());
    }
    if let Some(path) = &text.path {
        map.insert(
            "path".into(),
            json!({
                "curve": path.curve.to_string(),
                "start": path.start.source(),
                "offset": path.offset.source(),
                "side": match path.side { TextSide::Left => "left", TextSide::Right => "right" },
                "reversed": path.reversed,
            }),
        );
    }
    if let Some(laid) = solved.and_then(|s| s.text(id)) {
        map.insert("layout".into(), laid_out(solved, id));
        // Per glyph: its place in the text, character, contours, and where
        // it stands (its origin on the baseline and the baseline's
        // direction).
        let glyphs: Vec<Value> = laid
            .glyphs
            .iter()
            .take(500)
            .map(|g| {
                json!({
                    "glyph": g.index,
                    "char": g.ch.to_string(),
                    "contours": g.contours,
                    "origin": g.origin,
                    "direction": g.direction,
                    "advance": g.advance,
                })
            })
            .collect();
        map.insert("glyphs".into(), glyphs.into());
    }
    out
}

/// The measure `textExtent`: `{"sketch": id or name, "text": "e7", "what":
/// "width|height|capHeight|lines|inkWidth|inkHeight" = "width"}`, a length
/// (or the number of lines) of a text as laid out.
pub(crate) fn take(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    for key in p.keys() {
        if !matches!(key.as_str(), "sketch" | "text" | "what") {
            return Err(format!(
                "textExtent takes sketch, text and what, not `{}`",
                key.chars().take(40).collect::<String>()
            ));
        }
    }
    let sketch = match p.get("sketch") {
        Some(Value::String(s)) => s.as_str(),
        _ => return Err("`sketch` is required: the sketch's name or id".into()),
    };
    let feature = ctx
        .doc
        .feature_by_name(sketch)
        .or_else(|| {
            let n: u64 = sketch.strip_prefix('F')?.parse().ok()?;
            ctx.doc.feature(FeatureId(n))
        })
        .ok_or_else(|| {
            format!(
                "there is no sketch `{}`",
                sketch.chars().take(60).collect::<String>()
            )
        })?;
    let entity = match p.get("text") {
        Some(Value::String(t)) => t
            .strip_prefix('e')
            .unwrap_or(t)
            .parse::<u64>()
            .map(EntityId)
            .map_err(|_| "`text` is a text's id, such as \"e7\"".to_string())?,
        Some(Value::Number(n)) => EntityId(n.as_u64().ok_or("`text` is a text's id")?),
        _ => return Err("`text` is required: the text's id, such as \"e7\"".into()),
    };
    let what = match p.get("what") {
        None => "width",
        Some(Value::String(w)) => w.as_str(),
        Some(_) => {
            return Err("`what` is width, height, capHeight, lines, inkWidth or inkHeight".into());
        }
    };
    let solved = ctx
        .regen
        .feature(feature.id)
        .and_then(|f| f.sketch.as_deref())
        .ok_or_else(|| format!("sketch `{}` has no solution", feature.name))?;
    let laid = solved
        .text(entity)
        .ok_or_else(|| format!("sketch `{}` has no text {entity} laid out", feature.name))?;
    let ink = laid.ink.unwrap_or([0.0; 4]);
    let (dim, value) = match what {
        "width" => (Dim::Length(1), laid.width),
        "height" => (Dim::Length(1), laid.height),
        "capHeight" => (Dim::Length(1), laid.cap_height),
        "lines" => (Dim::Count, laid.lines as f64),
        "inkWidth" => (Dim::Length(1), ink[2] - ink[0]),
        "inkHeight" => (Dim::Length(1), ink[3] - ink[1]),
        other => {
            return Err(format!(
                "`what` is width, height, capHeight, lines, inkWidth or inkHeight, got {:?}",
                other.chars().take(20).collect::<String>()
            ));
        }
    };
    Ok(Outcome::number(dim, value))
}
