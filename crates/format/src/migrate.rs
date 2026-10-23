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
    v26_to_v27, v27_to_v28, v28_to_v29, v29_to_v30, v30_to_v31, v31_to_v32,
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
}
