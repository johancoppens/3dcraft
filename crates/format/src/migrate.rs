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
//!    literal, and never edit the fixtures of earlier versions.
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
const STEPS: &[Step] = &[v1_to_v2, v2_to_v3, v3_to_v4];

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
