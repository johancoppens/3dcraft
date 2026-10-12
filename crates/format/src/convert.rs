//! Document model <-> manifest, in both directions.
//!
//! COMPLETENESS IS ENFORCED BY THE COMPILER (AGENTS.md §8). Every
//! `craft3d-doc` struct is destructured with all of its fields named and no
//! `..` rest pattern, every enum is matched with no wildcard arm, and every
//! value going back is built with a full struct literal (no
//! `..Default::default()`). So when a field is added to a document struct,
//! or a variant to `FeatureKind`, this file stops compiling until the new
//! data is saved *and* loaded. Do not "fix" such an error with `..` or `_`:
//! add the field to `manifest.rs` (with `#[serde(default)]` so older files
//! still load) and convert it here, in both functions.

use crate::manifest;
use craft3d_doc as doc;

fn exprs_to_manifest(exprs: &[doc::Expr; 3]) -> [String; 3] {
    let [x, y, z] = exprs;
    [
        expr_to_manifest(x),
        expr_to_manifest(y),
        expr_to_manifest(z),
    ]
}

fn expr_to_manifest(expr: &doc::Expr) -> String {
    let doc::Expr(source) = expr;
    source.clone()
}

fn exprs_to_doc(sources: [String; 3]) -> [doc::Expr; 3] {
    sources.map(doc::Expr)
}

pub(crate) fn to_manifest(document: &doc::Document) -> manifest::Document {
    let doc::Document {
        units,
        tolerance,
        variables,
        features,
        rollback,
        next_feature_id,
    } = document;
    let doc::Units { length, angle } = units;
    let doc::Tolerance { linear, angular } = tolerance;
    manifest::Document {
        units: manifest::Units {
            length: match length {
                doc::LengthUnit::Micrometre => manifest::LengthUnit::Micrometre,
                doc::LengthUnit::Millimetre => manifest::LengthUnit::Millimetre,
                doc::LengthUnit::Centimetre => manifest::LengthUnit::Centimetre,
                doc::LengthUnit::Metre => manifest::LengthUnit::Metre,
                doc::LengthUnit::Inch => manifest::LengthUnit::Inch,
                doc::LengthUnit::Foot => manifest::LengthUnit::Foot,
            },
            angle: match angle {
                doc::AngleUnit::Degree => manifest::AngleUnit::Degree,
                doc::AngleUnit::Radian => manifest::AngleUnit::Radian,
            },
        },
        tolerance: manifest::Tolerance {
            linear: *linear,
            angular: *angular,
        },
        variables: variables
            .iter()
            .map(|variable| {
                let doc::Variable { name, expr } = variable;
                manifest::Variable {
                    name: name.clone(),
                    expr: expr_to_manifest(expr),
                }
            })
            .collect(),
        features: features.iter().map(feature_to_manifest).collect(),
        rollback: rollback.map(|doc::FeatureId(id)| id),
        next_feature_id: *next_feature_id,
    }
}

fn feature_to_manifest(feature: &doc::Feature) -> manifest::Feature {
    let doc::Feature {
        id: doc::FeatureId(id),
        name,
        suppressed,
        kind,
    } = feature;
    manifest::Feature {
        id: *id,
        name: name.clone(),
        suppressed: *suppressed,
        kind: match kind {
            doc::FeatureKind::Block(doc::Block {
                position,
                size,
                orientation,
            }) => manifest::FeatureKind::Block {
                position: exprs_to_manifest(position),
                size: exprs_to_manifest(size),
                orientation: orientation_to_manifest(orientation),
            },
            doc::FeatureKind::Cylinder(doc::Cylinder {
                position,
                radius,
                height,
                orientation,
            }) => manifest::FeatureKind::Cylinder {
                position: exprs_to_manifest(position),
                radius: expr_to_manifest(radius),
                height: expr_to_manifest(height),
                orientation: orientation_to_manifest(orientation),
            },
            doc::FeatureKind::Sphere(doc::Sphere { position, radius }) => {
                manifest::FeatureKind::Sphere {
                    position: exprs_to_manifest(position),
                    radius: expr_to_manifest(radius),
                }
            }
            doc::FeatureKind::Cone(doc::Cone {
                position,
                radius,
                height,
                orientation,
            }) => manifest::FeatureKind::Cone {
                position: exprs_to_manifest(position),
                radius: expr_to_manifest(radius),
                height: expr_to_manifest(height),
                orientation: orientation_to_manifest(orientation),
            },
            doc::FeatureKind::Torus(doc::Torus {
                position,
                major,
                minor,
                orientation,
            }) => manifest::FeatureKind::Torus {
                position: exprs_to_manifest(position),
                major: expr_to_manifest(major),
                minor: expr_to_manifest(minor),
                orientation: orientation_to_manifest(orientation),
            },
        },
    }
}

fn orientation_to_manifest(orientation: &doc::Orientation) -> manifest::Orientation {
    let doc::Orientation { axis, angle } = orientation;
    manifest::Orientation {
        axis: exprs_to_manifest(axis),
        angle: expr_to_manifest(angle),
    }
}

fn orientation_to_doc(orientation: manifest::Orientation) -> doc::Orientation {
    let manifest::Orientation { axis, angle } = orientation;
    doc::Orientation {
        axis: exprs_to_doc(axis),
        angle: doc::Expr(angle),
    }
}

pub(crate) fn to_doc(document: manifest::Document) -> doc::Document {
    let manifest::Document {
        units: manifest::Units { length, angle },
        tolerance: manifest::Tolerance { linear, angular },
        variables,
        features,
        rollback,
        next_feature_id,
    } = document;
    doc::Document {
        units: doc::Units {
            length: match length {
                manifest::LengthUnit::Micrometre => doc::LengthUnit::Micrometre,
                manifest::LengthUnit::Millimetre => doc::LengthUnit::Millimetre,
                manifest::LengthUnit::Centimetre => doc::LengthUnit::Centimetre,
                manifest::LengthUnit::Metre => doc::LengthUnit::Metre,
                manifest::LengthUnit::Inch => doc::LengthUnit::Inch,
                manifest::LengthUnit::Foot => doc::LengthUnit::Foot,
            },
            angle: match angle {
                manifest::AngleUnit::Degree => doc::AngleUnit::Degree,
                manifest::AngleUnit::Radian => doc::AngleUnit::Radian,
            },
        },
        tolerance: doc::Tolerance { linear, angular },
        variables: variables
            .into_iter()
            .map(|variable| {
                let manifest::Variable { name, expr } = variable;
                doc::Variable {
                    name,
                    expr: doc::Expr(expr),
                }
            })
            .collect(),
        features: features.into_iter().map(feature_to_doc).collect(),
        rollback: rollback.map(doc::FeatureId),
        next_feature_id,
    }
}

fn feature_to_doc(feature: manifest::Feature) -> doc::Feature {
    let manifest::Feature {
        id,
        name,
        suppressed,
        kind,
    } = feature;
    doc::Feature {
        id: doc::FeatureId(id),
        name,
        suppressed,
        kind: match kind {
            manifest::FeatureKind::Block {
                position,
                size,
                orientation,
            } => doc::FeatureKind::Block(doc::Block {
                position: exprs_to_doc(position),
                size: exprs_to_doc(size),
                orientation: orientation_to_doc(orientation),
            }),
            manifest::FeatureKind::Cylinder {
                position,
                radius,
                height,
                orientation,
            } => doc::FeatureKind::Cylinder(doc::Cylinder {
                position: exprs_to_doc(position),
                radius: doc::Expr(radius),
                height: doc::Expr(height),
                orientation: orientation_to_doc(orientation),
            }),
            manifest::FeatureKind::Sphere { position, radius } => {
                doc::FeatureKind::Sphere(doc::Sphere {
                    position: exprs_to_doc(position),
                    radius: doc::Expr(radius),
                })
            }
            manifest::FeatureKind::Cone {
                position,
                radius,
                height,
                orientation,
            } => doc::FeatureKind::Cone(doc::Cone {
                position: exprs_to_doc(position),
                radius: doc::Expr(radius),
                height: doc::Expr(height),
                orientation: orientation_to_doc(orientation),
            }),
            manifest::FeatureKind::Torus {
                position,
                major,
                minor,
                orientation,
            } => doc::FeatureKind::Torus(doc::Torus {
                position: exprs_to_doc(position),
                major: doc::Expr(major),
                minor: doc::Expr(minor),
                orientation: orientation_to_doc(orientation),
            }),
        },
    }
}
