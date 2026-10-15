//! What a body looks like and what it is made of: a colour and a material
//! per body, kept beside the history and not in it.
//!
//! A body is known by the feature that makes it ([`FeatureId`], never
//! reused), so an entry survives every regeneration and every edit of
//! the feature's parameters. Entries are not features: they do not change
//! what regeneration builds, only how the bodies are drawn, exported and
//! weighed. How an entry reaches the bodies and faces made from that
//! feature's body by later features (a boolean, a fillet, a pattern) is
//! decided where the bodies are known (`craft3d-engine`, `appearance`),
//! by the rule in docs/design/document.md.

use crate::{DocError, Document, FeatureId, MAX_NAME_LEN};
use serde::{Deserialize, Serialize};

/// How a body is drawn. `color` is sRGB; `metallic` and `roughness` are
/// the two numbers of a metallic-roughness material (0 to 1 each), kept
/// for exporters and renderers that use them: the viewport shades from
/// the colour alone so far.
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

impl Document {
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
