//! What a body looks like and what it is made of: a colour and a material
//! per body, and a colour per face, kept beside the history and not in it.
//!
//! A body is known by the feature that makes it ([`FeatureId`], never
//! reused), so an entry survives every regeneration and every edit of
//! the feature's parameters. Entries are not features: they do not change
//! what regeneration builds, only how the bodies are drawn, exported and
//! weighed. How an entry reaches the bodies and faces made from that
//! feature's body by later features (a boolean, a fillet, a pattern) is
//! decided where the bodies are known (`craft3d-engine`, `appearance`),
//! by the rule in docs/design/document.md.
//!
//! A face is known by the feature whose body has it and its persistent
//! name there ([`FaceProperties`]), so its colour survives every edit that
//! leaves the face's name in that body, and reaches the faces later
//! features make of it by the same rule.

use crate::{DocError, Document, FeatureId, MAX_NAME_LEN, PersistentName};
use serde::{Deserialize, Serialize};

/// How a body is drawn. `color` is sRGB; `metallic` and `roughness` are
/// the two numbers of a metallic-roughness material (0 to 1 each), which
/// both renderers shade with and OBJ's material library carries.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub color: [u8; 3],
    #[serde(default)]
    pub metallic: Option<f64>,
    #[serde(default)]
    pub roughness: Option<f64>,
}

impl Appearance {
    /// A plain colour.
    pub fn color(color: [u8; 3]) -> Self {
        Self {
            color,
            metallic: None,
            roughness: None,
        }
    }

    /// Why this is not an appearance: a factor outside 0 to 1.
    pub fn check(&self) -> Result<(), String> {
        for (name, value) in [("metallic", self.metallic), ("roughness", self.roughness)] {
            if let Some(value) = value
                && !(0.0..=1.0).contains(&value)
            {
                return Err(format!("{name} must be between 0 and 1, but it is {value}"));
            }
        }
        Ok(())
    }

    /// The colour as `#rrggbb`.
    pub fn hex(&self) -> String {
        let [r, g, b] = self.color;
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

/// What a body is made of: a name for people and a density, in kg/m³
/// whatever the document's length unit, so a body's mass is its volume
/// (converted to m³) times this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// kg/m³, positive and finite.
    pub density: f64,
}

impl Material {
    /// Why this is not a material: an empty or overlong name, or a density
    /// that is not a positive number.
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.trim() != self.name {
            return Err(
                "the material's name must not be empty or start or end with a space".into(),
            );
        }
        if self.name.len() > MAX_NAME_LEN {
            return Err(format!(
                "the material's name is longer than {MAX_NAME_LEN} bytes"
            ));
        }
        if self.name.chars().any(char::is_control) {
            return Err("the material's name must not contain control characters".into());
        }
        if !(self.density.is_finite() && self.density > 0.0) {
            return Err(format!(
                "the density must be a positive number (kg/m³), but it is {}",
                self.density
            ));
        }
        Ok(())
    }
}

/// The appearance and the material given to the body of one feature.
/// An entry has at least one of the two.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyProperties {
    pub feature: FeatureId,
    #[serde(default)]
    pub appearance: Option<Appearance>,
    #[serde(default)]
    pub material: Option<Material>,
}

/// Most face appearances a document holds.
pub const MAX_FACE_APPEARANCES: usize = 100_000;

/// The appearance given to one face: the face named `face` in the body of
/// `feature`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceProperties {
    pub feature: FeatureId,
    pub face: PersistentName,
    pub appearance: Appearance,
}

impl Document {
    /// The face appearances given in the body of `feature`, in the order
    /// of their names.
    pub fn face_properties(&self, feature: FeatureId) -> &[FaceProperties] {
        let start = self.faces.partition_point(|p| p.feature < feature);
        let end = self.faces.partition_point(|p| p.feature <= feature);
        self.faces.get(start..end).unwrap_or_default()
    }

    /// The appearance given to the face named `face` in the body of
    /// `feature` itself, if any (not what it takes over from elsewhere).
    pub fn face_appearance(&self, feature: FeatureId, face: &PersistentName) -> Option<Appearance> {
        let entries = self.face_properties(feature);
        entries
            .binary_search_by(|p| p.face.cmp(face))
            .ok()
            .and_then(|at| entries.get(at))
            .map(|p| p.appearance)
    }

    /// Gives the face named `face` in the body of `feature` an appearance,
    /// or takes it away (`None`). Whether the body has such a face is the
    /// caller's to check: an entry of a face that is gone is kept (it
    /// does nothing) until it is taken away.
    pub fn set_face_appearance(
        &mut self,
        feature: FeatureId,
        face: PersistentName,
        appearance: Option<Appearance>,
    ) -> Result<(), DocError> {
        if self.feature(feature).is_none() {
            return Err(DocError::UnknownFeature(feature));
        }
        let face = face.canonical();
        check_face_name(feature, &face).map_err(DocError::Invalid)?;
        let at = self
            .faces
            .binary_search_by(|p| (p.feature, &p.face).cmp(&(feature, &face)));
        match (at, appearance) {
            (Ok(at), Some(appearance)) => {
                appearance.check().map_err(DocError::Invalid)?;
                if let Some(entry) = self.faces.get_mut(at) {
                    entry.appearance = appearance;
                }
            }
            (Err(at), Some(appearance)) => {
                appearance.check().map_err(DocError::Invalid)?;
                if self.faces.len() >= MAX_FACE_APPEARANCES {
                    return Err(DocError::Invalid(format!(
                        "a document holds at most {MAX_FACE_APPEARANCES} face colours"
                    )));
                }
                self.faces.insert(
                    at,
                    FaceProperties {
                        feature,
                        face,
                        appearance,
                    },
                );
            }
            (Ok(at), None) => {
                self.faces.remove(at);
            }
            (Err(_), None) => {}
        }
        Ok(())
    }

    /// What the body of `feature` was given itself, if anything (not what
    /// it takes over from the body it was made from).
    pub fn body_properties(&self, feature: FeatureId) -> Option<&BodyProperties> {
        self.bodies
            .binary_search_by_key(&feature, |p| p.feature)
            .ok()
            .and_then(|at| self.bodies.get(at))
    }

    /// Changes the entry of `feature` with `change`, keeping the list in
    /// the order of feature ids and without empty entries.
    fn edit_body(
        &mut self,
        feature: FeatureId,
        change: impl FnOnce(&mut BodyProperties),
    ) -> Result<(), DocError> {
        if self.feature(feature).is_none() {
            return Err(DocError::UnknownFeature(feature));
        }
        let at = match self.bodies.binary_search_by_key(&feature, |p| p.feature) {
            Ok(at) => at,
            Err(at) => {
                self.bodies.insert(
                    at,
                    BodyProperties {
                        feature,
                        appearance: None,
                        material: None,
                    },
                );
                at
            }
        };
        let empty = match self.bodies.get_mut(at) {
            Some(entry) => {
                change(entry);
                entry.appearance.is_none() && entry.material.is_none()
            }
            None => false,
        };
        if empty {
            self.bodies.remove(at);
        }
        Ok(())
    }

    /// Gives the body of `feature` an appearance, or takes it away
    /// (`None`). The feature need not have a body (yet): the entry is kept
    /// for when it has one.
    pub fn set_appearance(
        &mut self,
        feature: FeatureId,
        appearance: Option<Appearance>,
    ) -> Result<(), DocError> {
        if let Some(appearance) = &appearance {
            appearance.check().map_err(DocError::Invalid)?;
        }
        self.edit_body(feature, |entry| entry.appearance = appearance)
    }

    /// Gives the body of `feature` a material, or takes it away (`None`).
    pub fn set_material(
        &mut self,
        feature: FeatureId,
        material: Option<Material>,
    ) -> Result<(), DocError> {
        if let Some(material) = &material {
            material.check().map_err(DocError::Invalid)?;
        }
        self.edit_body(feature, |entry| entry.material = material)
    }

    /// The rules of [`Document::validate`] for the body entries: in
    /// ascending order of feature id without repeats, of ids that were
    /// given out, each with something in it and its values in range. An
    /// entry of a feature that was deleted is allowed (it does nothing).
    pub(crate) fn validate_bodies(&self) -> Result<(), DocError> {
        let mut last: Option<FeatureId> = None;
        for entry in &self.bodies {
            if last.is_some_and(|last| last >= entry.feature) {
                return Err(DocError::Invalid(
                    "the body appearances and materials are not in the order of their feature ids, or list one twice".to_string(),
                ));
            }
            last = Some(entry.feature);
            if entry.feature.0 == 0 || entry.feature.0 >= self.next_feature_id {
                return Err(DocError::Invalid(format!(
                    "a body appearance or material is of feature {}, an id that was never given out",
                    entry.feature
                )));
            }
            if entry.appearance.is_none() && entry.material.is_none() {
                return Err(DocError::Invalid(format!(
                    "the body entry of {} has neither an appearance nor a material",
                    entry.feature
                )));
            }
            let bad = |reason: String| {
                DocError::Invalid(format!("the body of {}: {reason}", entry.feature))
            };
            if let Some(appearance) = &entry.appearance {
                appearance.check().map_err(bad)?;
            }
            if let Some(material) = &entry.material {
                material.check().map_err(bad)?;
            }
        }
        Ok(())
    }

    /// The rules of [`Document::validate`] for the face entries: in
    /// ascending order of feature id and name without repeats, of ids that
    /// were given out, each name a face's name of a feature no later than
    /// the body's, and the values in range.
    pub(crate) fn validate_faces(&self) -> Result<(), DocError> {
        if self.faces.len() > MAX_FACE_APPEARANCES {
            return Err(DocError::Invalid(format!(
                "the document has {} face colours; at most {MAX_FACE_APPEARANCES} are read",
                self.faces.len()
            )));
        }
        let mut last: Option<(FeatureId, &PersistentName)> = None;
        for entry in &self.faces {
            if last.is_some_and(|last| last >= (entry.feature, &entry.face)) {
                return Err(DocError::Invalid(
                    "the face colours are not in the order of their feature ids and names, or list one twice".to_string(),
                ));
            }
            last = Some((entry.feature, &entry.face));
            if entry.feature.0 == 0 || entry.feature.0 >= self.next_feature_id {
                return Err(DocError::Invalid(format!(
                    "a face colour is of feature {}, an id that was never given out",
                    entry.feature
                )));
            }
            let bad = |reason: String| {
                DocError::Invalid(format!(
                    "the colour of face {} of {}: {reason}",
                    entry.face, entry.feature
                ))
            };
            check_face_name(entry.feature, &entry.face).map_err(bad)?;
            entry.appearance.check().map_err(bad)?;
        }
        Ok(())
    }
}

/// Why `face` cannot be the name of a face of the body of `feature`.
fn check_face_name(feature: FeatureId, face: &PersistentName) -> Result<(), String> {
    face.check()
        .map_err(|reason| format!("`{face}` is not a name: {reason}"))?;
    if !face.is_face() {
        return Err(format!("`{face}` is not the name of a face"));
    }
    if face.feature > feature {
        return Err(format!(
            "`{face}` is made by {}, which comes after {feature}",
            face.feature
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, Expr, FeatureKind, Orientation, Units};

    fn doc_with_blocks(n: usize) -> (Document, Vec<FeatureId>) {
        let mut doc = Document::new(Units::default());
        let ids = (0..n)
            .map(|_| {
                doc.add_feature(
                    None,
                    FeatureKind::Block(Block {
                        position: [Expr::new("0"), Expr::new("0"), Expr::new("0")],
                        size: [Expr::new("1"), Expr::new("1"), Expr::new("1")],
                        orientation: Orientation::default(),
                    }),
                )
                .unwrap()
            })
            .collect();
        (doc, ids)
    }

    #[test]
    fn entries_stay_in_id_order_and_empty_ones_go() {
        let (mut doc, ids) = doc_with_blocks(3);
        doc.set_appearance(ids[2], Some(Appearance::color([1, 2, 3])))
            .unwrap();
        doc.set_material(
            ids[0],
            Some(Material {
                name: "Steel".into(),
                density: 7850.0,
            }),
        )
        .unwrap();
        doc.set_appearance(ids[0], Some(Appearance::color([9, 9, 9])))
            .unwrap();
        let order: Vec<FeatureId> = doc.bodies.iter().map(|b| b.feature).collect();
        assert_eq!(order, vec![ids[0], ids[2]]);
        doc.validate().unwrap();
        doc.set_appearance(ids[2], None).unwrap();
        assert_eq!(doc.bodies.len(), 1);
        doc.set_material(ids[0], None).unwrap();
        doc.set_appearance(ids[0], None).unwrap();
        assert!(doc.bodies.is_empty());
    }

    #[test]
    fn bad_values_and_unknown_features_are_refused() {
        let (mut doc, ids) = doc_with_blocks(1);
        assert!(
            doc.set_appearance(FeatureId(99), Some(Appearance::color([0; 3])))
                .is_err()
        );
        let shiny = Appearance {
            metallic: Some(1.5),
            ..Appearance::color([0; 3])
        };
        assert!(doc.set_appearance(ids[0], Some(shiny)).is_err());
        for density in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let material = Material {
                name: "X".into(),
                density,
            };
            assert!(doc.set_material(ids[0], Some(material)).is_err());
        }
        for name in ["", " x", "a\nb"] {
            let material = Material {
                name: name.into(),
                density: 1.0,
            };
            assert!(doc.set_material(ids[0], Some(material)).is_err());
        }
        assert!(doc.bodies.is_empty());
    }

    #[test]
    fn face_entries_stay_in_order_and_bad_ones_are_refused() {
        let (mut doc, ids) = doc_with_blocks(2);
        let name = |text: &str| text.parse::<PersistentName>().unwrap();
        let blue = Some(Appearance::color([0, 0, 255]));
        doc.set_face_appearance(ids[1], name("F2:face(3)"), blue)
            .unwrap();
        doc.set_face_appearance(ids[0], name("F1:face(1)"), blue)
            .unwrap();
        doc.set_face_appearance(ids[1], name("F2:face(0)"), blue)
            .unwrap();
        let order: Vec<String> = doc.faces.iter().map(|f| f.face.to_string()).collect();
        assert_eq!(order, ["F1:face(1)", "F2:face(0)", "F2:face(3)"]);
        assert_eq!(doc.face_properties(ids[1]).len(), 2);
        assert_eq!(doc.face_appearance(ids[1], &name("F2:face(3)")), blue);
        assert_eq!(doc.face_appearance(ids[1], &name("F2:face(1)")), None);
        doc.validate().unwrap();
        // A long name is kept in its short form.
        doc.set_face_appearance(ids[1], name("F2:copy(0)/F2:copy(0)/F1:face(4)"), blue)
            .unwrap();
        assert!(
            doc.faces
                .iter()
                .any(|f| f.face.to_string() == "F2:copy(0)/F1:face(4)")
        );
        // Refused: an unknown feature, an edge's name, a face of a later
        // feature, a factor out of range.
        assert!(
            doc.set_face_appearance(FeatureId(9), name("F1:face(0)"), blue)
                .is_err()
        );
        assert!(
            doc.set_face_appearance(ids[0], name("F1:edge(0)"), blue)
                .is_err()
        );
        assert!(
            doc.set_face_appearance(ids[0], name("F2:face(0)"), blue)
                .is_err()
        );
        let shiny = Appearance {
            metallic: Some(-1.0),
            ..Appearance::color([0; 3])
        };
        assert!(
            doc.set_face_appearance(ids[0], name("F1:face(0)"), Some(shiny))
                .is_err()
        );
        // Taken away; taking away what is not there does nothing.
        doc.set_face_appearance(ids[1], name("F2:face(3)"), None)
            .unwrap();
        doc.set_face_appearance(ids[1], name("F2:face(3)"), None)
            .unwrap();
        assert_eq!(doc.faces.len(), 3);
        // Validation catches a file out of order.
        doc.faces.swap(0, 1);
        assert!(doc.validate().is_err());
        doc.faces.swap(0, 1);
        doc.validate().unwrap();
        // Deleting the feature drops its face entries.
        doc.remove_feature(ids[1]).unwrap();
        assert_eq!(doc.faces.len(), 1);
        doc.validate().unwrap();
    }

    #[test]
    fn deleting_the_feature_drops_its_entry() {
        let (mut doc, ids) = doc_with_blocks(2);
        doc.set_appearance(ids[1], Some(Appearance::color([5; 3])))
            .unwrap();
        doc.remove_feature(ids[1]).unwrap();
        assert!(doc.bodies.is_empty());
        doc.validate().unwrap();
    }

    #[test]
    fn validate_catches_hand_edited_entries() {
        let (mut doc, ids) = doc_with_blocks(2);
        let entry = |feature| BodyProperties {
            feature,
            appearance: Some(Appearance::color([0; 3])),
            material: None,
        };
        doc.bodies = vec![entry(ids[1]), entry(ids[0])];
        assert!(doc.validate().is_err());
        doc.bodies = vec![entry(FeatureId(77))];
        assert!(doc.validate().is_err());
        doc.bodies = vec![BodyProperties {
            feature: ids[0],
            appearance: None,
            material: None,
        }];
        assert!(doc.validate().is_err());
        doc.bodies = vec![entry(ids[0]), entry(ids[1])];
        doc.validate().unwrap();
    }
}
