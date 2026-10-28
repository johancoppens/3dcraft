//! Bringing the manifest of an older file to the current format version.
//!
//! A migration works on the parsed JSON, before it is read into the types
//! of `manifest.rs`: those types describe the *current* version only, and
//! an old file need not fit them.
//!
//! To change the meaning of existing data (docs/file-formats.md,
//! AGENTS.md §8):
//! 1. write a step `fn vN_to_vM(manifest: Value) -> Result<Value, String>`
//!    here and append it to [`STEPS`];
//! 2. raise [`FORMAT_VERSION`](crate::FORMAT_VERSION) (the crate does not
//!    compile while the two disagree);
//! 3. add a test with a fixture of the old version, written as a string
//!    literal, and never edit the fixtures of earlier versions;
//! 4. write the new version's file with the new build and freeze it
//!    (`tests/versions.rs` fails until then: the format promise,
//!    docs/file-formats.md).
//!
//! A new optional field on its own needs no step, only `#[serde(default)]`;
//! raise the version anyway when older builds could not read what the
//! field or a new kind writes, so they report the version rather than an
//! unknown field (as version 2 did).

use crate::{FORMAT_VERSION, FormatError};
use serde_json::Value;

/// One step takes the manifest of one version to the next. The error is
/// the reason, in words; [`migrate`] adds which step failed.
type Step = fn(Value) -> Result<Value, String>;

/// `STEPS[n]` brings a manifest from version `n + 1` to version `n + 2`.
const STEPS: &[Step] = &[
    v1_to_v2, v2_to_v3, v3_to_v4, v4_to_v5, v5_to_v6, v6_to_v7, v7_to_v8, v8_to_v9, v9_to_v10,
    v10_to_v11, v11_to_v12, v12_to_v13, v13_to_v14, v14_to_v15, v15_to_v16, v16_to_v17, v17_to_v18,
    v18_to_v19, v19_to_v20, v20_to_v21, v21_to_v22, v22_to_v23, v23_to_v24, v24_to_v25, v25_to_v26,
    v26_to_v27, v27_to_v28, v28_to_v29, v29_to_v30, v30_to_v31, v31_to_v32, v32_to_v33, v33_to_v34,
    v34_to_v35, v35_to_v36, v36_to_v37, v37_to_v38, v38_to_v39, v39_to_v40, v40_to_v41, v41_to_v42,
    v42_to_v43, v43_to_v44, v44_to_v45, v45_to_v46, v46_to_v47, v47_to_v48, v48_to_v49, v49_to_v50,
];

/// Version 2 adds the `cone` and `torus` feature kinds and an optional
/// `orientation` on block, cylinder, cone and torus. Nothing a version-1
/// file contains changes meaning: its features are upright, which is what
/// a missing `orientation` reads as. The version was raised all the same,
/// so that a build which only knows version 1 says "a newer 3DCraft is
/// needed" instead of stumbling over an unknown field or kind.
fn v1_to_v2(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 3 adds the `sketch` feature kind. Nothing a version-2 file
/// contains changes meaning; the version was raised for the same reason as
/// version 2, so an older build reports the version and not an unknown kind.
fn v2_to_v3(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 4 adds the `extrude` feature kind and sketches on a face of a
/// body (plane type `face`). Nothing a version-3 file contains changes
/// meaning.
fn v3_to_v4(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 5 adds the `revolve` and `boolean` feature kinds, the feature kinds that make
/// a body out of another body (`transform`, `mirror`, `pattern`), the name
/// role `copy` with its `of`, and the roles `face`, `edge` and `vertex` for
/// the entities of primitives. Nothing a version-4 file contains changes meaning.
fn v4_to_v5(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 6 adds the `spline` kind of sketch entity. Nothing a version-5
/// file contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v5_to_v6(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 7 adds the `chamfer` feature kind and the name roles `chamfer`
/// and `chamferCorner` for the faces it makes. Nothing a version-6 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v6_to_v7(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 8 adds the `loft` feature kind. Nothing a version-7 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v7_to_v8(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 9 adds the `fillet` feature kind and the name roles `fillet`
/// and `filletCorner` for the faces it makes. Nothing a version-8 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v8_to_v9(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 10 adds the `sweep` feature kind. Nothing a version-9 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v9_to_v10(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 11 adds the feature kinds of the face operations:
/// `offsetFace` and `shell`. They make no new name roles: their faces are
/// copies of the faces they are made from. Nothing a version-10 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v10_to_v11(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 12 adds the feature kind `deleteFace`. It makes no new name
/// roles: its faces are copies of the faces of the body it is made from,
/// and its edges and vertices are named by their faces. Nothing a
/// version-11 file contains changes meaning in the file; the version was
/// raised so that an older build reports the version and not an unknown
/// kind. (The same build fixed which way a sketch on a face of a
/// subtraction's tool faces, a pocket's floor or a hole's wall: out of
/// the material, as on every other face. That is a correction of how
/// regeneration reads the face, not of what the file says, and is not
/// migrated: docs/decisions.md, 14 October 2026.)
fn v11_to_v12(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 13 adds the feature kinds `datumHelix` and `thread`, and a
/// sweep whose `path` is a datum helix. Nothing a version-12 file
/// contains changes meaning; the version was raised so that an older
/// build reports the version and not an unknown kind.
fn v12_to_v13(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// The coarse pitches of ISO 261 as the version-13 build knew them
/// (nominal diameter, pitch, in millimetres): what [`v13_to_v14`] has to
/// recognise. Frozen with the version, never edited: a later change of the
/// thread table does not change what a version-13 file wrote.
const V13_COARSE: [(f64, f64); 26] = [
    (1.0, 0.25),
    (1.2, 0.25),
    (1.4, 0.3),
    (1.6, 0.35),
    (1.8, 0.35),
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (3.5, 0.6),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (20.0, 2.5),
    (24.0, 3.0),
    (30.0, 3.5),
    (36.0, 4.0),
    (42.0, 4.5),
    (48.0, 5.0),
    (56.0, 5.5),
    (64.0, 6.0),
    (68.0, 6.0),
];

/// The pitch text the version-13 `feature.thread` wrote when it was given
/// none: the coarse pitch of the designation, `"1.5 mm"` for `M10`.
fn v13_default_pitch(size: &str) -> Option<String> {
    let digits = size.trim().strip_prefix(['M', 'm'])?;
    if digits.is_empty()
        || digits.len() > 8
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return None;
    }
    let nominal: f64 = digits.parse().ok()?;
    V13_COARSE
        .iter()
        .find(|(d, _)| (d - nominal).abs() < 1e-9)
        .map(|(_, pitch)| format!("{pitch} mm"))
}

/// Version 14 lets a thread's `size` be the nominal diameter as an
/// expression (`{"diameter": "d"}`) or be absent (the size the face has),
/// and its `pitch` be absent: the coarse pitch of the size, looked up at
/// every regeneration so that it follows the size. Version 13 always
/// wrote the pitch, and when the command was given none it wrote the
/// coarse pitch of the size as text (`"1.5 mm"` for `M10`), fixed from
/// then on: a later change of the size kept it. That text is taken out
/// here, so the pitch of such a thread follows its size. A pitch that was
/// given as exactly that text cannot be told from it and follows the size
/// too; it is the same pitch until the size changes, and then the coarse
/// pitch of the new size is what that pitch meant (docs/decisions.md, 14
/// October 2026). Any other pitch stays.
///
/// Version 14 also makes a thread cosmetic by default, as other modellers
/// do: the flag is `modeled`, written only when set. Version 13 wrote
/// `cosmetic` only when set, and a thread without it was cut; so
/// `cosmetic: true` is taken out, and a thread without it gets
/// `modeled: true`. Every version-13 thread keeps what it was.
fn v13_to_v14(mut manifest: Value) -> Result<Value, String> {
    let Some(features) = manifest
        .get_mut("document")
        .and_then(|d| d.get_mut("features"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(manifest);
    };
    for feature in features {
        let Some(kind) = feature.get_mut("kind").and_then(Value::as_object_mut) else {
            continue;
        };
        if kind.get("type").and_then(Value::as_str) != Some("thread") {
            continue;
        }
        let default = kind
            .get("size")
            .and_then(Value::as_str)
            .and_then(v13_default_pitch);
        if default.is_some() && kind.get("pitch").and_then(Value::as_str) == default.as_deref() {
            kind.remove("pitch");
        }
        // Anything but a boolean is left for the reader to refuse.
        match kind.get("cosmetic") {
            Some(Value::Bool(true)) => {
                kind.remove("cosmetic");
            }
            None | Some(Value::Bool(false)) => {
                kind.remove("cosmetic");
                kind.insert("modeled".to_string(), Value::Bool(true));
            }
            Some(_) => {}
        }
    }
    Ok(manifest)
}

/// Version 15 writes a copy of a copy as one link whose numbers are the
/// instances along the way (`F9:copy(0,1)/F2:side(e7)` for what version 14
/// wrote `F9:copy(0)/F7:copy(1)/F2:side(e7)`), so that a name does not
/// grow a link per feature (docs/design/names.md). The long form
/// means the same entity and is read into the short one by every version
/// (`PersistentName::canonical`, in `convert`), so nothing changes here;
/// the version was raised so that an older build, which takes a copy to
/// have one number, says a newer 3DCraft is needed instead of refusing
/// the names.
fn v14_to_v15(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 16 adds `bodies` to the document: an appearance (a colour) and
/// a material (a density) per body, by the feature that makes it. A file
/// of version 15 has none, which is what a missing `bodies` reads as, so
/// nothing changes; the version was raised so that an older build says a
/// newer 3DCraft is needed instead of refusing an unknown field.
fn v15_to_v16(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 17 adds the functions `floor`, `ceil`, `round`, `trunc` and
/// `mod` to expressions, which makes those names reserved. A variable an
/// older file gave one of them is renamed `floor_1` (the first free
/// name), and every expression that refers to it with it, after the
/// manifest is read into a document (`Document::rename_reserved_variables`,
/// called by `from_manifest_json_noted` for a file older than 17, with a
/// warning per rename): which strings are expressions is known to the
/// document's types, not to the JSON. So nothing changes here; the version
/// was raised so that an older build says a newer 3DCraft is needed
/// instead of failing the features whose expressions use the functions.
fn v16_to_v17(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 18 adds patterns inside a sketch: `patterns` and
/// `next_pattern_id` on a sketch, image points (geometry `image`), and the
/// name forms `e7[3]`, `e7[2,5]` and `e7[*]` for the curves a pattern
/// makes. Nothing a version-17 file contains changes meaning: it has no
/// patterns, and its names read as they did. The version was raised so
/// that an older build says a newer 3DCraft is needed instead of failing
/// on an unknown field.
fn v17_to_v18(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 19 adds the `offsetProfile` feature kind (the closed profiles
/// of a sketch offset by a distance) and the name forms
/// `F9:offset(e7)`, `F9:offsetCorner(e7,e8)` and
/// `F9:piece(1)/F9:offset(e7)` for the curves it makes, inside the name
/// of what a sweep made from them. Nothing a version-18 file contains
/// changes meaning; the version was raised so that an older build says a
/// newer 3DCraft is needed instead of failing on an unknown kind.
fn v18_to_v19(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 20 adds `image_places` on a sketch: where its image points lay
/// in the stored solution, so that the solver carries the sketch along
/// when an edit moves an image. Nothing a version-19 file contains changes
/// meaning: without it a sketch solves from its coordinates as before. The
/// version was raised so that an older build says a newer 3DCraft is
/// needed instead of failing on an unknown field.
fn v19_to_v20(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 21 adds nested sketch patterns and skipped instances:
/// `patterns` (the patterns repeated whole as part of a pattern's seed)
/// and `skip` on a sketch pattern, image points whose `of` is an image
/// point, and the name forms `e3[2][1]` and `e3[*][1]` (an index per
/// level, the innermost first; in a name's numbers a continuation word per
/// outer level). Nothing a version-20 file contains changes meaning: it
/// has no nested patterns and no skips, and its names read as they did.
/// The version was raised so that an older build says a newer 3DCraft is
/// needed instead of failing on an unknown field.
fn v20_to_v21(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 22 adds the `fill` feature kind (a sheet: the flat faces a
/// sketch's closed profiles bound). Nothing a version-21 file contains
/// changes meaning. The version was raised so that an older build says a
/// newer 3DCraft is needed instead of failing on an unknown kind.
fn v21_to_v22(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 23 adds the optional `step` of a circular pattern (a screw:
/// the length each instance climbs along the axis beyond its turn),
/// written only when set. Nothing a version-22 file contains changes
/// meaning: a circular pattern without it is the plain turn it was. The
/// version was raised so that an older build says a newer 3DCraft is
/// needed instead of failing on an unknown field.
fn v22_to_v23(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 24 adds the `split` feature kind (one lump of a body of
/// several as a body of its own, `body.split`). Nothing a version-23
/// file contains changes meaning; the version was raised so that an
/// older build says a newer 3DCraft is needed instead of failing on an
/// unknown kind.
fn v23_to_v24(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 25 added the `import` feature kind and the `blobs/` entries
/// that hold imported files; nothing older changes meaning.
fn v24_to_v25(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 26 adds `faces` to the document: a colour per face, by the
/// feature whose body has the face and its persistent name there
/// (`face.appearance`). A file of version 25 has none, which is what a
/// missing `faces` reads as, so nothing changes; the version was raised so
/// that an older build says a newer 3DCraft is needed instead of refusing
/// an unknown field.
fn v25_to_v26(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 27 adds `sets` to the document: named sets, a query of
/// `select.query` kept under a name and computed again at every
/// regeneration (`set.define`). A file of version 26 has none, which is
/// what a missing `sets` reads as, so nothing changes; the version was
/// raised so that an older build says a newer 3DCraft is needed instead of
/// refusing an unknown field.
fn v26_to_v27(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 28 adds `assertions` to the document: the design intent
/// written down as checks (`assert.add`). A file of version 27 has none,
/// which is what a missing `assertions` reads as, so nothing changes; the
/// version was raised so that an older build says a newer 3DCraft is
/// needed instead of refusing an unknown field.
fn v27_to_v28(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 29 adds the sketch constraint kinds `x` and `y`: a point's
/// signed coordinate in its sketch, an expression (`sketch.point` with
/// coordinates written as expressions). Nothing a version-28 file contains
/// changes meaning; the version was raised so that an older build says a
/// newer 3DCraft is needed instead of refusing an unknown constraint kind.
fn v28_to_v29(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 30 adds `hidden` to the document: the ids of the features
/// hidden in the view (`feature.visible`), view data that regeneration
/// does not read. A file of version 29 has none, which is what a missing
/// `hidden` reads as, so nothing changes; the version was raised so that
/// an older build says a newer 3DCraft is needed instead of refusing an
/// unknown field.
fn v29_to_v30(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 31 adds conditions to expressions: the comparisons
/// `< <= > >= == !=`, the logical operators `&& || !`, the functions `if`,
/// `clamp` and `choose`, and `exp`, `ln`, `log10`, `pow`, `sinh`, `cosh`
/// and `tanh`. Those function names become reserved, so a variable an
/// older file gave one of them is renamed `ln_1` (the first free name),
/// and every expression that refers to it with it, after the manifest is
/// read into a document (`Document::rename_reserved_variables`, called by
/// `from_manifest_json_noted` for a file older than 31, with a warning per
/// rename), as version 17 did for `floor`. So nothing changes here; the
/// version was raised so that an older build says a newer 3DCraft is
/// needed instead of failing the expressions that use the new syntax.
fn v30_to_v31(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 32 adds public parameters and document info (Jigs): `public`
/// on a variable and `info` on the document, each written only when set,
/// and the archive entry `thumb.png`, a picture that loading ignores. A
/// file of version 31 has none of them, which is what their absence reads
/// as, so nothing changes; the version was raised so that an older build
/// says a newer 3DCraft is needed instead of refusing an unknown field.
fn v31_to_v32(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 33 adds tables: `tables` on the document (named columns with
/// units, rows of numbers and texts), looked up in expressions as
/// `belt[2].pitch` or `belt["GT2"].pitch`, and `choices_from` on a choice
/// parameter (the table whose key column gives its labels). Each is
/// written only when set; a file of version 32 has none, which is what
/// their absence reads as, so nothing changes. The version was raised so
/// that an older build says a newer 3DCraft is needed instead of refusing
/// an unknown field or failing a lookup.
fn v32_to_v33(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 34 adds conditions: `when` on a feature, an expression that
/// switches it on (not 0) and off (0, inactive), written only when set; a
/// file of version 33 has none, which is what its absence reads as.
///
/// It also changes what `suppressed` means (docs/design/regeneration.md ›
/// *Switched off*): a suppressed feature that changes a body (a boolean's
/// target, a fillet, a chamfer, a face operation, a shell, a thread, a
/// pattern, a transform or mirror without `keep`) now passes that body on
/// and takes it, where version 33 left it free. A later feature that named
/// that body as a body parameter (a boolean's `target` or `tool`, the
/// `source` of a transform, mirror, pattern, shell or split) would then
/// find it gone into the suppressed feature. Such a parameter is pointed
/// at the suppressed feature instead: the same body, unchanged, so the
/// document regenerates to the same bodies. Names of faces and edges need
/// nothing: they are followed through the feature that took the body. A
/// body hidden in the view stays hidden: the suppressed feature that holds
/// it now is hidden too.
fn v33_to_v34(mut manifest: Value) -> Result<Value, String> {
    let Some(document) = manifest.get_mut("document").and_then(Value::as_object_mut) else {
        return Ok(manifest);
    };
    let mut hidden: Vec<u64> = document
        .get("hidden")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let Some(features) = document.get_mut("features").and_then(Value::as_array_mut) else {
        return Ok(manifest);
    };
    // Which feature took the body of which, as version 34 builds the
    // history: what a name of an earlier body is followed to.
    let mut taker: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let steps = features.len();
    let newest = |taker: &std::collections::BTreeMap<u64, u64>, id: u64| {
        let mut at = id;
        // Each step goes to a later feature: no more steps than features.
        for _ in 0..=steps {
            match taker.get(&at) {
                Some(next) if *next != at => at = *next,
                _ => break,
            }
        }
        at
    };
    let mut hide = Vec::new();
    for index in 0..steps {
        let Some(feature) = features.get(index) else {
            break;
        };
        let Some(id) = feature.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let suppressed = feature.get("suppressed").and_then(Value::as_bool) == Some(true);
        let kind = feature.get("kind").cloned().unwrap_or(Value::Null);
        if suppressed {
            let Some((source, by_name)) = v34_passed_source(&kind) else {
                continue;
            };
            // A body parameter names the body itself; a name is followed
            // to the newest body of its chain.
            let from = if by_name {
                newest(&taker, source)
            } else {
                source
            };
            if taker.contains_key(&from) {
                // Gone into another feature already: version 34 cannot
                // pass it on either, and says why.
                continue;
            }
            for later in features.iter_mut().skip(index.saturating_add(1)) {
                if let Some(kind) = later.get_mut("kind") {
                    v34_point_body_params(kind, from, id);
                }
            }
            taker.insert(from, id);
            if (hidden.contains(&from) || hide.contains(&from)) && !hidden.contains(&id) {
                hide.push(id);
            }
        } else {
            for (source, by_name) in v34_taken_sources(&kind) {
                let from = if by_name {
                    newest(&taker, source)
                } else {
                    source
                };
                taker.entry(from).or_insert(id);
            }
        }
    }
    if !hide.is_empty() {
        hidden.extend(hide);
        hidden.sort_unstable();
        hidden.dedup();
        document.insert(
            "hidden".to_string(),
            Value::Array(hidden.into_iter().map(Value::from).collect()),
        );
    }
    Ok(manifest)
}

/// The feature named by a name (`{"feature": 3, ...}`) in a version-33
/// manifest.
fn v34_name_feature(name: Option<&Value>) -> Option<u64> {
    name?.get("feature")?.as_u64()
}

/// The feature a name-based kind is made from: the one its first name is
/// of (the first face of a fillet's or a chamfer's first edge, the first
/// face of a face operation, the face of a thread).
fn v34_named_source(kind: &Value) -> Option<u64> {
    match kind.get("type")?.as_str()? {
        "chamfer" | "fillet" => {
            v34_name_feature(kind.get("edges")?.as_array()?.first()?.as_array()?.first())
        }
        "offsetFace" | "moveFace" | "draft" | "deleteFace" | "thicken" => {
            v34_name_feature(kind.get("faces")?.as_array()?.first())
        }
        "thread" => v34_name_feature(kind.get("face")),
        _ => None,
    }
}

/// The body a suppressed feature of this kind passes on in version 34,
/// with whether it is given by a name (followed) rather than by a body
/// parameter; `None` for a kind that passes nothing on.
fn v34_passed_source(kind: &Value) -> Option<(u64, bool)> {
    let keep = kind.get("keep").and_then(Value::as_bool) == Some(true);
    match kind.get("type")?.as_str()? {
        "boolean" => Some((kind.get("target")?.as_u64()?, false)),
        "transform" | "mirror" if !keep => Some((kind.get("source")?.as_u64()?, false)),
        "pattern" | "shell" => Some((kind.get("source")?.as_u64()?, false)),
        "chamfer" | "fillet" | "offsetFace" | "moveFace" | "draft" | "deleteFace" | "thread" => {
            Some((v34_named_source(kind)?, true))
        }
        _ => None,
    }
}

/// The bodies a feature that is on takes (as version 33 and 34 alike
/// build it), each with whether it is given by a name.
fn v34_taken_sources(kind: &Value) -> Vec<(u64, bool)> {
    let flag = |key: &str| kind.get(key).and_then(Value::as_bool) == Some(true);
    let id = |key: &str| kind.get(key).and_then(Value::as_u64);
    let Some(kind_type) = kind.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };
    match kind_type {
        "boolean" => {
            let mut out: Vec<(u64, bool)> = id("target").map(|t| (t, false)).into_iter().collect();
            if !flag("keep_tool") {
                out.extend(id("tool").map(|t| (t, false)));
            }
            out
        }
        "transform" | "mirror" if flag("keep") => Vec::new(),
        "transform" | "mirror" | "pattern" | "shell" | "split" => {
            id("source").map(|s| (s, false)).into_iter().collect()
        }
        "chamfer" | "fillet" | "offsetFace" | "moveFace" | "draft" | "deleteFace" | "thread" => {
            v34_named_source(kind)
                .map(|s| (s, true))
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Points the body parameters of `kind` that name `from` at `to`.
fn v34_point_body_params(kind: &mut Value, from: u64, to: u64) {
    let keys: &[&str] = match kind.get("type").and_then(Value::as_str) {
        Some("boolean") => &["target", "tool"],
        Some("transform" | "mirror" | "pattern" | "shell" | "split") => &["source"],
        _ => &[],
    };
    for key in keys {
        if let Some(value) = kind.get_mut(*key)
            && value.as_u64() == Some(from)
        {
            *value = Value::from(to);
        }
    }
}

/// Version 35 adds two kinds of sketch entity: a curve from formulas
/// (`formula`: `x`, `y`, `from`, `to`, `start`, `end`) and a live spline
/// (`interpolated`: `degree`, `through`), both fitted by regeneration and
/// stored without their fits. A file of version 34 has none, so nothing
/// changes; the version was raised so that an older build says a newer
/// 3DCraft is needed instead of refusing an unknown kind of entity.
fn v34_to_v35(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 36 renames the assertion measure `interference` to `overlap`
/// and turns its yes or no round: `interference` was true when none of
/// the bodies overlap, which read backwards (`is: true` asserted that
/// nothing interferes); `overlap` is true when a pair overlaps. An
/// assertion of version 35 with `{"measure": "interference", ...}` and
/// `{"is": b}` becomes `{"measure": "overlap", ...}` with `{"is": !b}`,
/// its subjects as they were: it asserts the same thing. One without an
/// `is` (which version 35 never wrote) meant the default, true. The
/// version also adds the measures `centerOfMass`, `boxMin` and `boxMax`,
/// which an older build would report as unknown.
fn v35_to_v36(mut manifest: Value) -> Result<Value, String> {
    let Some(assertions) = manifest
        .get_mut("document")
        .and_then(|d| d.get_mut("assertions"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(manifest);
    };
    for assertion in assertions {
        let name: String = assertion
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .chars()
            .take(80)
            .collect();
        let Some(measure) = assertion.get_mut("measure").and_then(Value::as_object_mut) else {
            continue;
        };
        if measure.get("measure").and_then(Value::as_str) != Some("interference") {
            continue;
        }
        measure.insert("measure".into(), Value::from("overlap"));
        let Some(expect) = assertion.get_mut("expect").and_then(Value::as_object_mut) else {
            return Err(format!(
                "assertion `{name}` measures interference but has no `expect`"
            ));
        };
        let none_overlap = match expect.get("is") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(is)) => *is,
            Some(_) => {
                return Err(format!(
                    "assertion `{name}` measures interference but its `is` is not true or false"
                ));
            }
        };
        expect.insert("is".into(), Value::from(!none_overlap));
    }
    Ok(manifest)
}

/// Version 37 adds the functions `asinh`, `acosh`, `atanh`, `log2`,
/// `hypot` and `sign` to expressions, and lets a body's colour and
/// density follow the variables: a body appearance's `color` may be three
/// expressions (`["woods[wood].red", "120", "40"]`) and a material may
/// have a `density_expr`. A file of version 36 has neither, which is what
/// their absence reads as, so nothing changes here. The new function
/// names become reserved, so a variable an older file gave one of them is
/// renamed `sign_1` (the first free name), and every expression that
/// refers to it with it, after the manifest is read into a document
/// (`Document::rename_reserved_variables`, called by
/// `from_manifest_json_noted` for a file older than 37, with a warning
/// per rename), as version 31 did for `ln`. The version was raised so
/// that an older build says a newer 3DCraft is needed instead of failing
/// an expression or refusing a colour it cannot read.
fn v36_to_v37(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 38 adds the `fold` feature kind (a crease pattern folded
/// rigidly); an older file has none, so nothing changes.
fn v37_to_v38(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 39 adds the `developable` and `unfold` feature kinds
/// (docs/design/developable.md). A file of version 38 has neither, so
/// nothing changes; the version was raised so that an older build says a
/// newer 3DCraft is needed instead of failing on a kind it does not know.
fn v38_to_v39(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 40 adds the `plate` feature kind and the name role `rim`
/// (docs/design/plates.md). A file of version 39 has neither, so nothing
/// changes; the version was raised so that an older build says a newer
/// 3DCraft is needed instead of failing on a kind it does not know.
fn v39_to_v40(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 41 adds the `sheetMetal` feature kind (docs/design/sheetmetal.md).
/// A file of version 40 has none, so nothing changes; the version was raised
/// so that an older build says a newer 3DCraft is needed instead of failing
/// on a kind it does not know.
fn v40_to_v41(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 42 adds `cutouts` to the `sheetMetal` kind (sketches whose
/// closed profiles are holes of the flats) and lets the base sketch hold
/// closed profiles beside the base, which version 41 refused (so no file
/// of 41 has them). A file of version 41 has no `cutouts`, which reads as
/// none, so nothing changes; the version was raised so that an older
/// build says a newer 3DCraft is needed instead of refusing the field.
fn v41_to_v42(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 43 adds outputs (read-only variables computed after
/// regeneration from measures and expressions) and the assertion measure
/// `value` (docs/design/verification.md › *Outputs*). A file of version 42
/// has neither, so nothing changes; the version was raised so that an
/// older build says a newer 3DCraft is needed instead of refusing a field
/// it does not know.
fn v42_to_v43(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 44 adds `corner` and `corners` to the `sheetMetal` kind (closed
/// corners) and lets a relief's `shape` be `{"expr": "…"}`, an expression
/// that picks a shape (docs/design/sheetmetal.md › *Closed corners*). A
/// file of version 43 has neither, and a shape written by name reads as
/// before, so nothing changes; the version was raised so that an older
/// build says a newer 3DCraft is needed instead of refusing the fields.
fn v43_to_v44(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 45 lets a fold's crease be a copy a sketch pattern made of its
/// line: `instance` (`{"at": 2}`, `{"grid": [1, 3]}` or `"all"`) and
/// `alternate` on a crease, written only when set
/// (docs/design/origami.md › *Creases from a sketch pattern*). A file of
/// version 44 has neither, so nothing in it changes; the version was
/// raised so that an older build says a newer 3DCraft is needed instead
/// of refusing the field. One thing regenerates differently, not from
/// the file but from the rule that came with it: a fold of version 38 to
/// 44 whose crease line has copies made by a sketch pattern, which those
/// versions dropped without a word, now fails, naming the copies left out
/// (name them as creases, or the line with `[*]`).
fn v44_to_v45(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 46 adds `when` to an assertion, a condition that switches it
/// on and off (inactive while it is 0), written only when set
/// (docs/design/verification.md › *Conditions*). A file of version 45 has
/// none, so its assertions are always on, as they were; the version was
/// raised so that an older build says a newer 3DCraft is needed instead
/// of refusing the field. Nothing it holds changes meaning; what reads
/// differently comes with the rule, not the file: an output or an
/// assertion that uses an unknown output only in a branch of `if` that is
/// not taken is now known or decided, where it was unknown or notProven.
fn v45_to_v46(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 47 adds `x_axis` to a sketch plane of type `face`: a world
/// direction the sketch's x axis is turned to, written only when set
/// (docs/design/document.md › *A sketch on a face with turned axes*). A
/// file of version 46 has none, so its sketches on faces keep the face's
/// own axes, as they did; the version was raised so that an older build
/// says a newer 3DCraft is needed instead of refusing the field.
fn v46_to_v47(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 48: a fold may have thick `panels` (thickness, clearance,
/// side), written only when set (docs/design/origami.md › *Thick
/// panels*). A file without them reads as before; the version is raised so
/// that an older build says a newer 3DCraft is needed instead of refusing
/// the field. Nothing it holds changes meaning.
fn v47_to_v48(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// Version 49 adds to a `sheetMetal` flange `side` (on a side of an
/// earlier flange), `inset_start` and `inset_end` (a bend narrower than
/// its edge), `outline` (the sketch of the flange's own flat) and `id`
/// (with the part's `next_flange_id`) (docs/design/sheetmetal.md ›
/// *Inset flanges*, *Names*). A file of an older version has none, which
/// reads as before (on its edge or tip, the whole edge, the rectangle,
/// each flange's id its place in the list plus 1).
///
/// It also **changes the names of a sheet-metal part's edges and rims**:
/// they are named by what the edge is (a piece of a base curve, a side of
/// a flange, a bend's tangent line), no longer by the faces round it
/// (docs/design/sheetmetal.md › *Names*). The old names cannot be mapped
/// to the new ones without the old naming, which is gone, so nothing is
/// rewritten here: every stored name that refers to such an edge or rim
/// (a fillet's or chamfer's edge, a datum, a face colour, a set, a sketch
/// on a face; [`sheet_metal_edge_names`]) is reported as a load warning
/// by `from_manifest_json_noted`, naming it, so it can be picked again;
/// left as it is, it names nothing (the feature fails with the name it
/// lost, a colour colours nothing).
///
/// Every other name of the part is numbered by the flanges' ids since 49
/// (a flange's faces `face(2 · id − 1)` and `face(2 · id)`, a bend by `id −
/// 1`, a flat by its flange's id), no longer by their places in the list,
/// so deleting or inserting a flange retargets none. In an older file
/// every id is the flange's place plus 1, so those numbers are the places
/// they were: the part's faces, and every other name that is not an edge
/// or a rim, name what they named, with no warning.
fn v48_to_v49(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

/// The stored names of a manifest (of a version before 49) that refer to
/// the edge or rim of a `sheetMetal` part, whose names changed in format
/// 49 (see [`v48_to_v49`]): every persistent name anywhere in the
/// document that is a `rim` or an `edge` of a sheet-metal feature, or
/// lists one among its faces (a `between` edge or a `corner`), or is the
/// image (`of`) of one. Each with where it is stored: the feature's name,
/// or the document's field. In document order; the outermost name only.
pub(crate) fn sheet_metal_edge_names(manifest: &Value) -> Vec<(String, Value)> {
    let document = manifest.get("document");
    let features = document
        .and_then(|d| d.get("features"))
        .and_then(Value::as_array);
    let parts: std::collections::BTreeSet<u64> = features
        .into_iter()
        .flatten()
        .filter(|f| f.pointer("/kind/type").and_then(Value::as_str) == Some("sheetMetal"))
        .filter_map(|f| f.get("id").and_then(Value::as_u64))
        .collect();
    let mut found = Vec::new();
    if parts.is_empty() {
        return found;
    }
    let Some(Value::Object(fields)) = document else {
        return found;
    };
    for (key, value) in fields {
        if key == "features" {
            for feature in features.into_iter().flatten() {
                let at = feature
                    .get("name")
                    .and_then(Value::as_str)
                    .map_or_else(|| "a feature".to_string(), |n| format!("feature `{n}`"));
                if let Some(kind) = feature.get("kind") {
                    sheet_metal_names_in(kind, &parts, &at, &mut found, 0);
                }
            }
        } else {
            sheet_metal_names_in(value, &parts, &format!("`{key}`"), &mut found, 0);
        }
    }
    found
}

/// Deepest a stored name or the JSON round it is walked (serde_json's own
/// limit is 128).
const MAX_NAME_DEPTH: usize = 256;

/// Whether `value` is a persistent name, as `manifest::Name` writes it.
fn is_name(value: &Value) -> bool {
    value.get("feature").is_some_and(Value::is_u64)
        && value.get("role").is_some_and(Value::is_string)
        && value.get("sources").is_some_and(Value::is_array)
}

/// Whether the name `value` refers to a sheet-metal part's edge or rim.
fn names_sheet_metal_edge(
    value: &Value,
    parts: &std::collections::BTreeSet<u64>,
    depth: usize,
) -> bool {
    if depth > MAX_NAME_DEPTH || !is_name(value) {
        return false;
    }
    let feature = value.get("feature").and_then(Value::as_u64);
    let role = value.get("role").and_then(Value::as_str);
    if feature.is_some_and(|f| parts.contains(&f)) && matches!(role, Some("rim" | "edge")) {
        return true;
    }
    let nested = |v: Option<&Value>| {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|n| names_sheet_metal_edge(n, parts, depth + 1))
    };
    value
        .get("of")
        .is_some_and(|of| names_sheet_metal_edge(of, parts, depth + 1))
        || value
            .get("faces")
            .is_some_and(|f| nested(f.get("on")) || nested(f.get("near")))
}

fn sheet_metal_names_in(
    value: &Value,
    parts: &std::collections::BTreeSet<u64>,
    at: &str,
    found: &mut Vec<(String, Value)>,
    depth: usize,
) {
    if depth > MAX_NAME_DEPTH {
        return;
    }
    if is_name(value) {
        if names_sheet_metal_edge(value, parts, depth) {
            found.push((at.to_string(), value.clone()));
        }
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                sheet_metal_names_in(item, parts, at, found, depth + 1);
            }
        }
        Value::Object(fields) => {
            for item in fields.values() {
                sheet_metal_names_in(item, parts, at, found, depth + 1);
            }
        }
        _ => {}
    }
}

/// Version 50 adds `inserted`: the features inserted
/// at the rollback marker that take a body, with the parameters of later
/// features each pointed at itself and what they named before
/// (docs/design/names.md › *A feature inserted at the marker*), so that
/// deleting or editing one gives them back. A file of an older version
/// has none: a re-point it holds was made once and is kept as written, as
/// before; the version was raised so that an older build says a newer
/// 3DCraft is needed instead of refusing the field.
fn v49_to_v50(manifest: Value) -> Result<Value, String> {
    Ok(manifest)
}

// One step per version after the first: forgetting either half of a
// version bump does not compile.
const _: () = assert!(STEPS.len() + 1 == FORMAT_VERSION as usize);

/// Brings `manifest`, written by format version `version`, to
/// [`FORMAT_VERSION`](crate::FORMAT_VERSION) by running every step in
/// between, in order. The result carries the current version in its
/// `format_version` field.
///
/// Fails for version 0 (versions start at 1), for a version newer than
/// this build knows, and when a step cannot make sense of the data.
pub fn migrate(manifest: Value, version: u32) -> Result<Value, FormatError> {
    migrate_with(manifest, version, STEPS)
}

/// [`migrate`] over an explicit chain of steps, so the mechanism is tested
/// before the first real migration exists.
fn migrate_with(mut manifest: Value, version: u32, steps: &[Step]) -> Result<Value, FormatError> {
    let current = u32::try_from(steps.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .unwrap_or(u32::MAX);
    if version == 0 {
        return Err(FormatError::BadVersion(
            "it is 0, and versions start at 1".to_string(),
        ));
    }
    if version > current {
        return Err(FormatError::NewerVersion {
            file: u64::from(version),
            supported: current,
        });
    }
    for (from, step) in (1..current).zip(steps).filter(|(from, _)| *from >= version) {
        let failed = |reason: String| FormatError::Migration { from, reason };
        manifest = step(manifest).map_err(failed)?;
        // The version is stamped here, not by the step, so a step that
        // forgets it cannot leave the manifest claiming to be old.
        let to = from.saturating_add(1);
        manifest
            .as_object_mut()
            .ok_or_else(|| failed("the result is not a JSON object".to_string()))?
            .insert("format_version".to_string(), Value::from(to));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A file as version 1 wrote it. Never edit this: it stands for the
    /// files people have on disk.
    const V1_FIXTURE: &str = r#"{
  "format": "3dcraft",
  "format_version": 1,
  "document": {
    "units": { "length": "in", "angle": "rad" },
    "tolerance": { "linear": 1e-6, "angular": 1e-9 },
    "variables": [
      { "name": "width", "expr": "2 in" }
    ],
    "features": [
      {
        "id": 1,
        "name": "Plate",
        "suppressed": false,
        "kind": { "type": "block", "position": ["0", "0", "0"], "size": ["width", "width / 2", "0.25 in"] }
      },
      {
        "id": 3,
        "name": "Boss",
        "suppressed": true,
        "kind": { "type": "cylinder", "position": ["1 in", "0.5 in", "0.25 in"], "radius": "width / 8", "height": "1 in" }
      },
      {
        "id": 4,
        "name": "Ball",
        "kind": { "type": "sphere", "position": ["0", "0", "2 in"], "radius": "10 mm" }
      }
    ],
    "rollback": 3,
    "next_feature_id": 5
  }
}"#;

    #[test]
    fn a_version_1_file_loads() {
        use craft3d_doc::{AngleUnit, Expr, FeatureId, FeatureKind, LengthUnit};
        let doc = crate::from_manifest_json(V1_FIXTURE.as_bytes()).unwrap();
        assert_eq!(doc.units.length, LengthUnit::Inch);
        assert_eq!(doc.units.angle, AngleUnit::Radian);
        assert_eq!((doc.tolerance.linear, doc.tolerance.angular), (1e-6, 1e-9));
        assert_eq!(doc.variables.len(), 1);
        assert_eq!(doc.variables[0].name, "width");
        assert_eq!(doc.variables[0].expr.source(), "2 in");
        let features: Vec<_> = doc
            .features
            .iter()
            .map(|f| (f.id.0, f.name.as_str(), f.suppressed, f.kind.type_name()))
            .collect();
        assert_eq!(
            features,
            [
                (1, "Plate", false, "block"),
                (3, "Boss", true, "cylinder"),
                (4, "Ball", false, "sphere"),
            ]
        );
        let FeatureKind::Cylinder(boss) = &doc.features[1].kind else {
            panic!("not a cylinder")
        };
        assert_eq!(boss.position[2], Expr::new("0.25 in"));
        assert_eq!(boss.radius.source(), "width / 8");
        assert_eq!(boss.height.source(), "1 in");
        let FeatureKind::Block(plate) = &doc.features[0].kind else {
            panic!("not a block")
        };
        assert_eq!(plate.size[1].source(), "width / 2");
        assert_eq!(doc.rollback, Some(FeatureId(3)));
        assert_eq!(doc.next_feature_id, 5);
    }

    #[test]
    fn the_current_version_passes_through_unchanged() {
        let manifest: Value = serde_json::from_str(V1_FIXTURE).unwrap();
        assert_eq!(migrate(manifest.clone(), FORMAT_VERSION).unwrap(), manifest);
    }

    #[test]
    fn versions_outside_the_known_range_are_errors() {
        assert!(matches!(
            migrate(json!({}), 0),
            Err(FormatError::BadVersion(_))
        ));
        assert!(matches!(
            migrate(json!({}), FORMAT_VERSION + 1),
            Err(FormatError::NewerVersion { file, supported })
                if file == u64::from(FORMAT_VERSION) + 1 && supported == FORMAT_VERSION
        ));
    }

    /// A pretend history of three versions: 1 -> 2 renames `size` to
    /// `length`, 2 -> 3 turns it from a number into text with a unit.
    fn rename(mut manifest: Value) -> Result<Value, String> {
        let object = manifest.as_object_mut().ok_or("not an object")?;
        let size = object.remove("size").ok_or("`size` is missing")?;
        object.insert("length".to_string(), size);
        Ok(manifest)
    }

    fn add_unit(mut manifest: Value) -> Result<Value, String> {
        let length = manifest["length"]
            .as_f64()
            .ok_or("`length` is not a number")?;
        manifest["length"] = json!(format!("{length} mm"));
        Ok(manifest)
    }

    const CHAIN: &[Step] = &[rename, add_unit];

    #[test]
    fn steps_run_in_order_from_the_files_version() {
        let current = json!({"format_version": 3, "length": "5 mm"});
        assert_eq!(
            migrate_with(json!({"format_version": 1, "size": 5.0}), 1, CHAIN).unwrap(),
            current
        );
        // From the middle: only the later step runs.
        assert_eq!(
            migrate_with(json!({"format_version": 2, "length": 5.0}), 2, CHAIN).unwrap(),
            current
        );
        // Already current: no step runs (`add_unit` would fail on text).
        assert_eq!(migrate_with(current.clone(), 3, CHAIN).unwrap(), current);
        assert!(matches!(
            migrate_with(current, 4, CHAIN),
            Err(FormatError::NewerVersion {
                file: 4,
                supported: 3
            })
        ));
    }

    #[test]
    fn a_step_that_fails_says_which_one_and_why() {
        let error = migrate_with(json!({"format_version": 1}), 1, CHAIN).unwrap_err();
        assert!(matches!(
            &error,
            FormatError::Migration { from: 1, reason } if reason == "`size` is missing"
        ));
        let text = error.to_string();
        assert!(
            text.contains("version 1 to 2") && text.contains("`size` is missing"),
            "{text}"
        );
        let error = migrate_with(json!({"size": "wide"}), 1, CHAIN).unwrap_err();
        assert!(matches!(error, FormatError::Migration { from: 2, .. }));
        // A step that returns something that is not a manifest.
        let broken: &[Step] = &[|_| Ok(json!(7))];
        assert!(matches!(
            migrate_with(json!({}), 1, broken),
            Err(FormatError::Migration { from: 1, .. })
        ));
    }

    /// A face name of feature `f` in a version-33 manifest.
    fn face(f: u64, k: u64) -> Value {
        json!({"feature": f, "role": "face", "sources": [k]})
    }

    fn feature(id: u64, suppressed: bool, kind: Value) -> Value {
        json!({"id": id, "name": format!("F{id}"), "suppressed": suppressed, "kind": kind})
    }

    /// Version 34: a later body parameter that named the body a suppressed
    /// feature now passes on names that feature, through a chain of
    /// suppressed features, a name followed through an active one, a kept
    /// copy, and a hidden body; what no suppressed feature passes on, and
    /// a suppressed copy that keeps its source, are left alone.
    #[test]
    fn version_34_points_later_body_parameters_at_the_suppressed_feature() {
        let block = json!({"type": "block"});
        let manifest = json!({"document": {"hidden": [1], "features": [
            feature(1, false, block.clone()),
            // A suppressed fillet and chamfer of the plate, one after the
            // other: the plate goes into 2, then 3.
            feature(2, true, json!({"type": "fillet", "edges": [[face(1, 0), face(1, 2)]]})),
            feature(3, true, json!({"type": "chamfer", "edges": [[face(1, 1), face(1, 3)]]})),
            feature(4, false, block.clone()),
            // Was the plate: now the chamfer that holds it.
            feature(5, false, json!({"type": "boolean", "target": 1, "tool": 4})),
            // A suppressed boolean on the cut: what names the cut after it
            // names the boolean; a kept copy too.
            feature(6, true, json!({"type": "boolean", "target": 5, "tool": 4})),
            feature(7, false, json!({"type": "transform", "source": 5, "keep": true})),
            feature(8, false, json!({"type": "pattern", "source": 5})),
            // A suppressed copy that keeps its source passes nothing on.
            feature(9, true, json!({"type": "mirror", "source": 7, "keep": true})),
            feature(10, false, json!({"type": "shell", "source": 7})),
            // A suppressed block passes nothing on either.
            feature(11, true, block.clone()),
            feature(12, false, json!({"type": "boolean", "target": 11, "tool": 10})),
        ]}});
        let migrated = v33_to_v34(manifest).unwrap();
        let features = migrated["document"]["features"].as_array().unwrap();
        let kind = |i: usize| &features[i]["kind"];
        assert_eq!(
            (kind(4)["target"].as_u64(), kind(4)["tool"].as_u64()),
            (Some(3), Some(4))
        );
        assert_eq!(kind(5)["target"], 5);
        assert_eq!(kind(6)["source"], 6);
        assert_eq!(kind(7)["source"], 6);
        assert_eq!(kind(9)["source"], 7);
        assert_eq!(
            (kind(11)["target"].as_u64(), kind(11)["tool"].as_u64()),
            (Some(11), Some(10))
        );
        // The plate was hidden: so are the features that hold its body now.
        assert_eq!(migrated["document"]["hidden"], json!([1, 2, 3]));
        // Nothing to do without suppressed features.
        let plain = json!({"document": {"features": [
            feature(1, false, block.clone()),
            feature(2, false, json!({"type": "fillet", "edges": [[face(1, 0), face(1, 2)]]})),
        ]}});
        assert_eq!(v33_to_v34(plain.clone()).unwrap(), plain);
    }

    /// Assertions as version 35 wrote them. Never edit this: it stands for
    /// the files people have on disk.
    const V35_ASSERTIONS: &str = r#"{
  "format": "3dcraft",
  "format_version": 35,
  "document": {
    "units": { "length": "mm", "angle": "deg" },
    "tolerance": { "linear": 1e-6, "angular": 1e-9 },
    "variables": [],
    "features": [
      { "id": 1, "name": "A", "kind": { "type": "block", "position": ["0", "0", "0"], "size": ["10", "10", "10"] } },
      { "id": 2, "name": "B", "kind": { "type": "block", "position": ["20", "0", "0"], "size": ["10", "10", "10"] } }
    ],
    "next_feature_id": 3,
    "assertions": [
      { "name": "apart", "measure": { "measure": "interference" }, "expect": { "is": true } },
      { "name": "fit", "measure": { "measure": "interference", "of": "F1", "to": ["F2"] }, "expect": { "is": false } },
      { "name": "wide", "measure": { "axis": "x", "measure": "extent" }, "expect": { "max": "30 mm" } }
    ]
  }
}"#;

    #[test]
    fn version_36_writes_interference_as_overlap_with_the_opposite_is() {
        use craft3d_doc::Expect;
        let doc = crate::from_manifest_json(V35_ASSERTIONS.as_bytes()).unwrap();
        let read: Vec<_> = doc
            .assertions
            .iter()
            .map(|a| {
                (
                    a.name.as_str(),
                    serde_json::from_str::<Value>(&a.measure).unwrap(),
                    a.expect.clone(),
                )
            })
            .collect();
        assert_eq!(read[0].1, json!({"measure": "overlap"}));
        assert_eq!(read[0].2, Expect::Is(false));
        assert_eq!(
            read[1].1,
            json!({"measure": "overlap", "of": "F1", "to": ["F2"]})
        );
        assert_eq!(read[1].2, Expect::Is(true));
        // Other measures are left alone.
        assert_eq!(read[2].1, json!({"axis": "x", "measure": "extent"}));
        // No `is` meant true; an `is` that is no boolean is refused.
        let bare = json!({"document": {"assertions": [
            {"name": "a", "measure": {"measure": "interference"}, "expect": {}}
        ]}});
        let migrated = v35_to_v36(bare).unwrap();
        assert_eq!(
            migrated["document"]["assertions"][0]["expect"],
            json!({"is": false})
        );
        let wrong = json!({"document": {"assertions": [
            {"name": "a", "measure": {"measure": "interference"}, "expect": {"is": 1}}
        ]}});
        assert!(v35_to_v36(wrong).is_err());
        // Nothing to do without assertions.
        let plain = json!({"document": {"features": []}});
        assert_eq!(v35_to_v36(plain.clone()).unwrap(), plain);
    }
}
