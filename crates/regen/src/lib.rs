//! Regeneration: evaluates a document's variables and feature history into
//! bodies. The document is the recipe; what this returns is derived and can
//! be recomputed at any time (AGENTS.md rule 2).
//!
//! A failure is data, not an exception: a variable or feature that cannot
//! be evaluated gets a status that says why, and everything that does not
//! depend on it is still built.
//!
//! Not here yet: caching per feature (every call rebuilds everything),
//! features that consume other features' bodies, persistent names.

use craft3d_doc::{Document, Expr, Feature, FeatureId, FeatureKind};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::Body;
use std::sync::Arc;

/// The value of a variable, in the units the document shows.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableResult {
    pub name: String,
    /// The expression as written.
    pub expr: String,
    pub value: Result<Value, String>,
}

/// A number with its dimension. Lengths are in the document's length unit
/// (areas in its square, and so on); angles in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub value: f64,
    pub dimension: Dimension,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ok,
    /// Switched off by the user.
    Suppressed,
    /// After the rollback marker.
    RolledBack,
    /// Could not be built; the text names the parameter and the reason.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureResult {
    pub id: FeatureId,
    pub status: Status,
    /// The body this feature made, when it is `Ok`.
    pub body: Option<Arc<Body>>,
}

/// Everything derived from one document.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Regen {
    /// In evaluation order (dependencies first).
    pub variables: Vec<VariableResult>,
    /// In history order, one per feature.
    pub features: Vec<FeatureResult>,
}

impl Regen {
    /// The bodies that exist, with the feature that made each, in history
    /// order.
    pub fn bodies(&self) -> impl Iterator<Item = (FeatureId, &Arc<Body>)> {
        self.features
            .iter()
            .filter_map(|f| Some((f.id, f.body.as_ref()?)))
    }

    /// True when no variable and no feature failed.
    pub fn is_ok(&self) -> bool {
        self.variables.iter().all(|v| v.value.is_ok())
            && self
                .features
                .iter()
                .all(|f| !matches!(f.status, Status::Failed(_)))
    }

    pub fn feature(&self, id: FeatureId) -> Option<&FeatureResult> {
        self.features.iter().find(|f| f.id == id)
    }
}

/// The factor from the expression engine's base length (millimetres) to
/// the document's length unit, per power of length.
fn to_document(quantity: Quantity, doc: &Document) -> Value {
    let per_mm = Quantity::new(1.0, Dimension::LENGTH)
        .to_unit(doc.units.length.as_expr_unit())
        .unwrap_or(1.0);
    Value {
        value: quantity.value * per_mm.powi(quantity.dimension.length),
        dimension: quantity.dimension,
    }
}

/// Evaluates one length parameter, in the document's unit. A bare number
/// is taken to be in that unit, as when typing a dimension in a CAD
/// system.
fn length(doc: &Document, variables: &Evaluation, name: &str, expr: &Expr) -> Result<f64, String> {
    let quantity = craft3d_expr::evaluate(expr.source(), |variable| variables.value(variable))
        .map_err(|e| format!("{name}: `{}`: {e}", expr.source()))?;
    if quantity.dimension == Dimension::LENGTH {
        Ok(to_document(quantity, doc).value)
    } else if quantity.dimension.is_dimensionless() {
        Ok(quantity.value)
    } else {
        Err(format!(
            "{name}: `{}` is a {}, but a length is needed",
            expr.source(),
            quantity.dimension
        ))
    }
}

fn build(doc: &Document, variables: &Evaluation, feature: &Feature) -> Result<Body, String> {
    let get = |name: &str, expr: &Expr| length(doc, variables, name, expr);
    let point = |exprs: &[Expr; 3]| -> Result<Vec3, String> {
        let [x, y, z] = exprs;
        Ok(Vec3::new(
            get("position.x", x)?,
            get("position.y", y)?,
            get("position.z", z)?,
        ))
    };
    let at = |origin: Vec3| {
        Frame::new(origin, Vec3::Z, Vec3::X).ok_or("position is not finite".to_string())
    };
    // Sizes are checked here, where the parameter has the name the user
    // knows it by; the kernel would report its own argument name.
    let size = |name: &str, expr: &Expr| -> Result<f64, String> {
        let value = get(name, expr)?;
        if value > 0.0 {
            Ok(value)
        } else {
            Err(format!(
                "{name} must be a positive length, but `{}` is {value}",
                expr.source()
            ))
        }
    };
    let body = match &feature.kind {
        FeatureKind::Block(b) => {
            let [dx, dy, dz] = &b.size;
            let frame = at(point(&b.position)?)?;
            primitives::block(
                &frame,
                size("size.x", dx)?,
                size("size.y", dy)?,
                size("size.z", dz)?,
            )
        }
        FeatureKind::Cylinder(c) => {
            let frame = at(point(&c.position)?)?;
            primitives::cylinder(
                &frame,
                size("radius", &c.radius)?,
                size("height", &c.height)?,
            )
        }
        FeatureKind::Sphere(s) => {
            let frame = at(point(&s.position)?)?;
            primitives::sphere(&frame, size("radius", &s.radius)?)
        }
    }
    .map_err(|e| e.to_string())?;

    // Never hand on an invalid body (docs/architecture.md §5.3).
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    match craft3d_topo::check(&body, &tolerance).first() {
        None => Ok(body),
        Some(defect) => Err(format!("the result is not a valid body: {defect}")),
    }
}

/// Regenerates the whole document.
pub fn regenerate(doc: &Document) -> Regen {
    let evaluation = match doc.variable_table() {
        Ok(table) => table.evaluate(),
        // An invalid table (a hand-edited file): every variable fails the
        // same way, and so does every feature that uses one.
        Err(error) => {
            let message = error.to_string();
            let variables = doc
                .variables
                .iter()
                .map(|v| VariableResult {
                    name: v.name.clone(),
                    expr: v.expr.source().to_string(),
                    value: Err(message.clone()),
                })
                .collect();
            return Regen {
                variables,
                features: features(doc, &craft3d_expr::VariableTable::new().evaluate()),
            };
        }
    };
    let variables = evaluation
        .entries()
        .iter()
        .map(|entry| VariableResult {
            name: entry.name.clone(),
            expr: doc
                .variable(&entry.name)
                .map(|v| v.expr.source().to_string())
                .unwrap_or_default(),
            value: match &entry.result {
                Ok(quantity) => Ok(to_document(*quantity, doc)),
                Err(error) => Err(error.to_string()),
            },
        })
        .collect();
    Regen {
        variables,
        features: features(doc, &evaluation),
    }
}

fn features(doc: &Document, variables: &Evaluation) -> Vec<FeatureResult> {
    let mut results = Vec::with_capacity(doc.features.len());
    let mut rolled_back = false;
    for feature in &doc.features {
        let (status, body) = if rolled_back {
            (Status::RolledBack, None)
        } else if feature.suppressed {
            (Status::Suppressed, None)
        } else {
            match build(doc, variables, feature) {
                Ok(body) => (Status::Ok, Some(Arc::new(body))),
                Err(reason) => (Status::Failed(reason), None),
            }
        };
        results.push(FeatureResult {
            id: feature.id,
            status,
            body,
        });
        if doc.rollback == Some(feature.id) {
            rolled_back = true;
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_doc::{Block, Cylinder, LengthUnit, Sphere, Units};
    use craft3d_topo::mass_properties;
    use std::f64::consts::PI;

    fn exprs(values: [&str; 3]) -> [Expr; 3] {
        values.map(Expr::new)
    }

    fn sample(units: Units) -> Document {
        let mut doc = Document::new(units);
        doc.set_variable("width", "40 mm").unwrap();
        doc.set_variable("half", "width / 2").unwrap();
        doc.add_feature(
            None,
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["width", "half", "10 mm"]),
            }),
        )
        .unwrap();
        doc.add_feature(
            Some("Pin"),
            FeatureKind::Cylinder(Cylinder {
                position: exprs(["half", "10 mm", "10 mm"]),
                radius: Expr::new("width / 8"),
                height: Expr::new("2 cm"),
            }),
        )
        .unwrap();
        doc.add_feature(
            Some("Ball"),
            FeatureKind::Sphere(Sphere {
                position: exprs(["0 mm", "0 mm", "50 mm"]),
                radius: Expr::new("half / 2"),
            }),
        )
        .unwrap();
        doc
    }

    fn volumes(regen: &Regen) -> Vec<f64> {
        regen
            .bodies()
            .map(|(_, body)| mass_properties(body).unwrap().volume)
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs()
    }

    #[test]
    fn features_become_bodies_of_the_expected_size_and_place() {
        let regen = regenerate(&sample(Units::default()));
        assert!(regen.is_ok());
        let v = volumes(&regen);
        assert!(close(v[0], 40.0 * 20.0 * 10.0));
        assert!(close(v[1], PI * 25.0 * 20.0));
        assert!(close(v[2], 4.0 / 3.0 * PI * 1000.0));
        let centroid = |i: usize| {
            mass_properties(regen.features[i].body.as_ref().unwrap())
                .unwrap()
                .centroid
                .unwrap()
        };
        assert!(centroid(0).distance(Vec3::new(20.0, 10.0, 5.0)) < 1e-10);
        assert!(centroid(1).distance(Vec3::new(20.0, 10.0, 20.0)) < 1e-10);
        assert!(centroid(2).distance(Vec3::new(0.0, 0.0, 50.0)) < 1e-10);
        // Variables come back in the document's unit, dependencies first.
        let half = regen.variables.iter().find(|v| v.name == "half").unwrap();
        assert_eq!(
            (half.expr.as_str(), half.value.clone().unwrap().value),
            ("width / 2", 20.0)
        );
    }

    #[test]
    fn the_same_recipe_in_inches_gives_the_same_part_in_inches() {
        let units = Units {
            length: LengthUnit::Inch,
            ..Units::default()
        };
        let regen = regenerate(&sample(units));
        assert!(regen.is_ok());
        let per_inch = 25.4_f64;
        assert!(close(
            volumes(&regen)[0],
            40.0 * 20.0 * 10.0 / per_inch.powi(3)
        ));
        let width = regen.variables.iter().find(|v| v.name == "width").unwrap();
        assert!(close(width.value.clone().unwrap().value, 40.0 / per_inch));
        // A bare number is in the document's unit: 1 means one inch here.
        let mut doc = Document::new(units);
        doc.add_feature(
            None,
            FeatureKind::Block(Block {
                position: exprs(["0", "0", "0"]),
                size: exprs(["1", "25.4 mm", "1 in"]),
            }),
        )
        .unwrap();
        assert!(close(volumes(&regenerate(&doc))[0], 1.0));
    }

    #[test]
    fn changing_a_variable_changes_the_parts_that_use_it() {
        let mut doc = sample(Units::default());
        let before = volumes(&regenerate(&doc));
        doc.set_variable("width", "80 mm").unwrap();
        let after = volumes(&regenerate(&doc));
        assert!(
            close(after[0], before[0] * 4.0),
            "block: width and half both doubled"
        );
        assert!(close(after[1], before[1] * 4.0), "pin: radius doubled");
        assert!(close(after[2], before[2] * 8.0), "ball: radius doubled");
    }

    #[test]
    fn a_failure_stays_local_and_says_what_is_wrong() {
        let mut doc = sample(Units::default());
        doc.set_variable("half", "width / 0").unwrap();
        let regen = regenerate(&doc);
        assert!(!regen.is_ok());
        let half = regen.variables.iter().find(|v| v.name == "half").unwrap();
        assert!(
            half.value.as_ref().unwrap_err().contains("zero"),
            "{:?}",
            half.value
        );
        assert!(
            regen
                .variables
                .iter()
                .find(|v| v.name == "width")
                .unwrap()
                .value
                .is_ok()
        );
        let Status::Failed(reason) = &regen.features[0].status else {
            panic!("the block uses `half`")
        };
        assert!(reason.starts_with("size.y: `half`"), "{reason}");
        assert!(regen.features[0].body.is_none());
        // Only the features that use `half` fail.
        doc.features[2].kind = FeatureKind::Sphere(Sphere {
            position: exprs(["0", "0", "0"]),
            radius: Expr::new("width"),
        });
        let regen = regenerate(&doc);
        assert_eq!(regen.features[2].status, Status::Ok);
        assert_eq!(regen.bodies().count(), 1);
    }

    #[test]
    fn bad_parameters_are_failures_with_the_parameter_named() {
        let cases = [
            ("0 mm", "size.x must be"),
            ("-5 mm", "size.x must be"),
            ("30 deg", "but a length is needed"),
            ("1 mm * 1 mm", "but a length is needed"),
            ("nope", "unknown"),
            ("1 +", "size.x"),
            ("1e300 * 1e300", "size.x"),
        ];
        for (source, expected) in cases {
            let mut doc = Document::new(Units::default());
            doc.add_feature(
                None,
                FeatureKind::Block(Block {
                    position: exprs(["0", "0", "0"]),
                    size: exprs([source, "1", "1"]),
                }),
            )
            .unwrap();
            let regen = regenerate(&doc);
            let Status::Failed(reason) = &regen.features[0].status else {
                panic!("`{source}` should fail")
            };
            assert!(reason.contains(expected), "`{source}`: {reason}");
        }
    }

    #[test]
    fn suppressed_and_rolled_back_features_make_no_bodies() {
        let mut doc = sample(Units::default());
        doc.feature_mut(FeatureId(2)).unwrap().suppressed = true;
        let regen = regenerate(&doc);
        assert_eq!(regen.features[1].status, Status::Suppressed);
        assert_eq!(
            regen.bodies().map(|(id, _)| id.0).collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(regen.is_ok());

        doc.set_rollback(Some(FeatureId(1))).unwrap();
        let regen = regenerate(&doc);
        let statuses: Vec<_> = regen.features.iter().map(|f| f.status.clone()).collect();
        assert_eq!(
            statuses,
            [Status::Ok, Status::RolledBack, Status::RolledBack]
        );
        assert_eq!(regen.feature(FeatureId(3)).unwrap().body, None);
    }

    #[test]
    fn regeneration_is_deterministic() {
        let doc = sample(Units::default());
        assert_eq!(regenerate(&doc), regenerate(&doc));
        assert_eq!(
            regenerate(&Document::new(Units::default())),
            Regen::default()
        );
    }
}
