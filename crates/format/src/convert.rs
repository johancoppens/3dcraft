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
        bodies,
        faces,
        assertions,
        sets,
        hidden,
        info,
        tables,
        outputs,
        inserted,
        assembly,
    } = document;
    let doc::Units { length, angle } = units;
    let doc::Tolerance {
        linear,
        angular,
        loosest,
    } = tolerance;
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
            loosest: *loosest,
        },
        variables: variables
            .iter()
            .map(|variable| {
                let doc::Variable { name, expr, public } = variable;
                manifest::Variable {
                    name: name.clone(),
                    expr: expr_to_manifest(expr),
                    public: public.as_ref().map(public_to_manifest),
                }
            })
            .collect(),
        features: features.iter().map(feature_to_manifest).collect(),
        rollback: rollback.map(|doc::FeatureId(id)| id),
        next_feature_id: *next_feature_id,
        bodies: bodies.iter().map(body_to_manifest).collect(),
        faces: faces.iter().map(face_to_manifest).collect(),
        sets: sets
            .iter()
            .map(|doc::NamedSet { name, query }| manifest::NamedSet {
                name: name.clone(),
                query: query.clone(),
            })
            .collect(),
        assertions: assertions.iter().map(assertion_to_manifest).collect(),
        hidden: hidden.iter().map(|doc::FeatureId(id)| *id).collect(),
        info: (!info.is_empty()).then(|| info_to_manifest(info)),
        tables: tables.iter().map(table_to_manifest).collect(),
        outputs: outputs.iter().map(output_to_manifest).collect(),
        inserted: inserted.iter().map(inserted_to_manifest).collect(),
        assembly: assembly.as_ref().map(assembly_to_manifest),
    }
}

fn assembly_to_manifest(assembly: &doc::Assembly) -> manifest::Assembly {
    let doc::Assembly {
        instances,
        mates,
        couplings,
        packed,
    } = assembly;
    manifest::Assembly {
        instances: instances
            .iter()
            .map(|instance| {
                let doc::Instance {
                    name,
                    source,
                    body,
                    params,
                    pinned,
                    mode,
                    visible,
                    colour,
                    placement: doc::Placement { position, rotation },
                } = instance;
                manifest::Instance {
                    name: name.clone(),
                    source: match source {
                        doc::Source::File { path, hash } => manifest::Source::File {
                            path: path.clone(),
                            hash: hash.clone(),
                        },
                        doc::Source::Packed { hash } => {
                            manifest::Source::Packed { hash: hash.clone() }
                        }
                    },
                    body: body.clone(),
                    params: params
                        .iter()
                        .map(
                            |doc::ParamOverride { name, expr }| manifest::ParamOverride {
                                name: name.clone(),
                                expr: expr_to_manifest(expr),
                            },
                        )
                        .collect(),
                    pinned: pinned.clone(),
                    mode: match mode {
                        doc::InstanceMode::Rigid => manifest::InstanceMode::Rigid,
                        doc::InstanceMode::Flexible => manifest::InstanceMode::Flexible,
                    },
                    visible: *visible,
                    colour: *colour,
                    placement: manifest::Placement {
                        position: *position,
                        rotation: *rotation,
                    },
                }
            })
            .collect(),
        mates: mates
            .iter()
            .map(|mate| {
                let doc::Mate {
                    name,
                    kind,
                    a,
                    b,
                    offset,
                    angle,
                    limits,
                    flip,
                } = mate;
                manifest::Mate {
                    name: name.clone(),
                    kind: match kind {
                        doc::MateKind::Fixed => manifest::MateKind::Fixed,
                        doc::MateKind::Revolute => manifest::MateKind::Revolute,
                        doc::MateKind::Slider => manifest::MateKind::Slider,
                        doc::MateKind::Cylindrical => manifest::MateKind::Cylindrical,
                        doc::MateKind::Planar => manifest::MateKind::Planar,
                        doc::MateKind::Ball => manifest::MateKind::Ball,
                    },
                    a: a.to_string(),
                    b: b.to_string(),
                    offset: offset.as_ref().map(expr_to_manifest),
                    angle: angle.as_ref().map(expr_to_manifest),
                    limits: limits
                        .as_ref()
                        .map(|(lo, hi)| [expr_to_manifest(lo), expr_to_manifest(hi)]),
                    flip: *flip,
                }
            })
            .collect(),
        couplings: couplings
            .iter()
            .map(|coupling| {
                let doc::Coupling {
                    name,
                    kind,
                    mates: (first, second),
                    value,
                } = coupling;
                manifest::Coupling {
                    name: name.clone(),
                    kind: match kind {
                        doc::CouplingKind::Gear => manifest::CouplingKind::Gear,
                        doc::CouplingKind::Rack => manifest::CouplingKind::Rack,
                        doc::CouplingKind::Screw => manifest::CouplingKind::Screw,
                    },
                    mates: [first.clone(), second.clone()],
                    value: expr_to_manifest(value),
                }
            })
            .collect(),
        packed: packed.iter().map(doc::Blob::sha256).collect(),
    }
}

/// A mate's reference as written; one that does not read as
/// `instance/name` is kept whole as the instance part, for validation to
/// refuse by its text.
fn mate_ref_to_doc(text: String) -> doc::MateRef {
    match text.split_once('/') {
        Some((instance, name)) => doc::MateRef {
            instance: instance.to_string(),
            name: name.to_string(),
        },
        None => doc::MateRef {
            instance: text,
            name: String::new(),
        },
    }
}

fn assembly_to_doc(assembly: manifest::Assembly, blobs: &Blobs) -> doc::Assembly {
    let manifest::Assembly {
        instances,
        mates,
        couplings,
        packed,
    } = assembly;
    doc::Assembly {
        instances: instances
            .into_iter()
            .map(|instance| {
                let manifest::Instance {
                    name,
                    source,
                    body,
                    params,
                    pinned,
                    mode,
                    visible,
                    colour,
                    placement: manifest::Placement { position, rotation },
                } = instance;
                doc::Instance {
                    name,
                    source: match source {
                        manifest::Source::File { path, hash } => doc::Source::File { path, hash },
                        manifest::Source::Packed { hash } => doc::Source::Packed { hash },
                    },
                    body,
                    params: params
                        .into_iter()
                        .map(
                            |manifest::ParamOverride { name, expr }| doc::ParamOverride {
                                name,
                                expr: doc::Expr(expr),
                            },
                        )
                        .collect(),
                    pinned,
                    mode: match mode {
                        manifest::InstanceMode::Rigid => doc::InstanceMode::Rigid,
                        manifest::InstanceMode::Flexible => doc::InstanceMode::Flexible,
                    },
                    visible,
                    colour,
                    placement: doc::Placement { position, rotation },
                }
            })
            .collect(),
        mates: mates
            .into_iter()
            .map(|mate| {
                let manifest::Mate {
                    name,
                    kind,
                    a,
                    b,
                    offset,
                    angle,
                    limits,
                    flip,
                } = mate;
                doc::Mate {
                    name,
                    kind: match kind {
                        manifest::MateKind::Fixed => doc::MateKind::Fixed,
                        manifest::MateKind::Revolute => doc::MateKind::Revolute,
                        manifest::MateKind::Slider => doc::MateKind::Slider,
                        manifest::MateKind::Cylindrical => doc::MateKind::Cylindrical,
                        manifest::MateKind::Planar => doc::MateKind::Planar,
                        manifest::MateKind::Ball => doc::MateKind::Ball,
                    },
                    a: mate_ref_to_doc(a),
                    b: mate_ref_to_doc(b),
                    offset: offset.map(doc::Expr),
                    angle: angle.map(doc::Expr),
                    limits: limits.map(|[lo, hi]| (doc::Expr(lo), doc::Expr(hi))),
                    flip,
                }
            })
            .collect(),
        couplings: couplings
            .into_iter()
            .map(|coupling| {
                let manifest::Coupling {
                    name,
                    kind,
                    mates: [first, second],
                    value,
                } = coupling;
                doc::Coupling {
                    name,
                    kind: match kind {
                        manifest::CouplingKind::Gear => doc::CouplingKind::Gear,
                        manifest::CouplingKind::Rack => doc::CouplingKind::Rack,
                        manifest::CouplingKind::Screw => doc::CouplingKind::Screw,
                    },
                    mates: (first, second),
                    value: doc::Expr(value),
                }
            })
            .collect(),
        // Every one is in `blobs`: `read_manifest` refuses a file without
        // one first.
        packed: packed
            .iter()
            .filter_map(|hash| blobs.get(hash).cloned())
            .collect(),
    }
}

fn inserted_to_manifest(entry: &doc::Inserted) -> manifest::Inserted {
    let doc::Inserted {
        feature: doc::FeatureId(feature),
        repointed,
    } = entry;
    manifest::Inserted {
        feature: *feature,
        repointed: repointed
            .iter()
            .map(|r| {
                let doc::Repointed {
                    feature: doc::FeatureId(feature),
                    param,
                    was: doc::FeatureId(was),
                } = r;
                manifest::Repointed {
                    feature: *feature,
                    param: param.clone(),
                    was: *was,
                }
            })
            .collect(),
    }
}

fn inserted_to_doc(entry: manifest::Inserted) -> doc::Inserted {
    let manifest::Inserted { feature, repointed } = entry;
    doc::Inserted {
        feature: doc::FeatureId(feature),
        repointed: repointed
            .into_iter()
            .map(
                |manifest::Repointed {
                     feature,
                     param,
                     was,
                 }| doc::Repointed {
                    feature: doc::FeatureId(feature),
                    param,
                    was: doc::FeatureId(was),
                },
            )
            .collect(),
    }
}

fn output_to_manifest(output: &doc::Output) -> manifest::Output {
    let doc::Output { name, def } = output;
    let (measure, expr) = match def {
        // Written as a JSON object, as an assertion's measure is; anything
        // else is kept as the text it is.
        doc::OutputDef::Measure(text) => (
            Some(
                serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .filter(serde_json::Value::is_object)
                    .unwrap_or_else(|| serde_json::Value::String(text.clone())),
            ),
            None,
        ),
        doc::OutputDef::Expr(e) => (None, Some(expr_to_manifest(e))),
    };
    manifest::Output {
        name: name.clone(),
        measure,
        expr,
    }
}

fn output_to_doc(output: manifest::Output) -> doc::Output {
    let manifest::Output {
        name,
        measure,
        expr,
    } = output;
    // One with both or neither is refused before this, by
    // `from_manifest_json_noted`.
    let def = match (measure, expr) {
        (Some(serde_json::Value::String(text)), _) => doc::OutputDef::Measure(text),
        (Some(value), _) => {
            doc::OutputDef::Measure(serde_json::to_string(&value).unwrap_or_default())
        }
        (None, expr) => doc::OutputDef::Expr(doc::Expr(expr.unwrap_or_default())),
    };
    doc::Output { name, def }
}

fn table_to_manifest(table: &doc::Table) -> manifest::Table {
    let doc::Table {
        name,
        key,
        columns,
        rows,
    } = table;
    manifest::Table {
        name: name.clone(),
        key: key.clone(),
        columns: columns
            .iter()
            .map(|doc::TableColumn { name, unit }| manifest::TableColumn {
                name: name.clone(),
                unit: unit.clone(),
            })
            .collect(),
        rows: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        doc::TableCell::Number(v) => manifest::TableCell::Number(*v),
                        doc::TableCell::Text(t) => manifest::TableCell::Text(t.clone()),
                    })
                    .collect()
            })
            .collect(),
    }
}

fn table_to_doc(table: manifest::Table) -> doc::Table {
    let manifest::Table {
        name,
        key,
        columns,
        rows,
    } = table;
    doc::Table {
        name,
        key,
        columns: columns
            .into_iter()
            .map(|manifest::TableColumn { name, unit }| doc::TableColumn { name, unit })
            .collect(),
        rows: std::sync::Arc::new(
            rows.into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| match cell {
                            manifest::TableCell::Number(v) => doc::TableCell::Number(v),
                            manifest::TableCell::Text(t) => doc::TableCell::Text(t),
                        })
                        .collect()
                })
                .collect(),
        ),
    }
}

fn public_to_manifest(public: &doc::PublicParam) -> manifest::PublicParam {
    let doc::PublicParam {
        label,
        description,
        group,
        order,
        kind,
        choices,
        choices_from,
        min,
        max,
        step,
        unit,
        default,
    } = public;
    manifest::PublicParam {
        label: label.clone(),
        description: description.clone(),
        group: group.clone(),
        order: *order,
        kind: match kind {
            doc::ParamKind::Number => manifest::ParamKind::Number,
            doc::ParamKind::Integer => manifest::ParamKind::Integer,
            doc::ParamKind::Boolean => manifest::ParamKind::Boolean,
            doc::ParamKind::Choice => manifest::ParamKind::Choice,
        },
        choices: choices.clone(),
        choices_from: choices_from.clone(),
        min: min.as_ref().map(expr_to_manifest),
        max: max.as_ref().map(expr_to_manifest),
        step: step.as_ref().map(expr_to_manifest),
        unit: unit.clone(),
        default: expr_to_manifest(default),
    }
}

fn public_to_doc(public: manifest::PublicParam) -> doc::PublicParam {
    let manifest::PublicParam {
        label,
        description,
        group,
        order,
        kind,
        choices,
        choices_from,
        min,
        max,
        step,
        unit,
        default,
    } = public;
    doc::PublicParam {
        label,
        description,
        group,
        order,
        kind: match kind {
            manifest::ParamKind::Number => doc::ParamKind::Number,
            manifest::ParamKind::Integer => doc::ParamKind::Integer,
            manifest::ParamKind::Boolean => doc::ParamKind::Boolean,
            manifest::ParamKind::Choice => doc::ParamKind::Choice,
        },
        choices,
        choices_from,
        min: min.map(doc::Expr),
        max: max.map(doc::Expr),
        step: step.map(doc::Expr),
        unit,
        default: doc::Expr(default),
    }
}

fn info_to_manifest(info: &doc::DocInfo) -> manifest::DocInfo {
    let doc::DocInfo {
        title,
        description,
        author,
        licence,
        url,
        tags,
    } = info;
    manifest::DocInfo {
        title: title.clone(),
        description: description.clone(),
        author: author.clone(),
        licence: licence.clone(),
        url: url.clone(),
        tags: tags.clone(),
    }
}

fn info_to_doc(info: manifest::DocInfo) -> doc::DocInfo {
    let manifest::DocInfo {
        title,
        description,
        author,
        licence,
        url,
        tags,
    } = info;
    doc::DocInfo {
        title,
        description,
        author,
        licence,
        url,
        tags,
    }
}

fn assertion_to_manifest(assertion: &doc::Assertion) -> manifest::Assertion {
    let doc::Assertion {
        name,
        measure,
        expect,
        when,
    } = assertion;
    // The engine writes the measure as a JSON object; anything else is
    // kept as the text it is.
    let measure = serde_json::from_str::<serde_json::Value>(measure)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::Value::String(measure.clone()));
    let text = |expr: &doc::Expr| expr.source().to_string();
    let expect = match expect {
        doc::Expect::Is(is) => manifest::Expect {
            is: Some(*is),
            ..manifest::Expect::default()
        },
        doc::Expect::Equal { value, tol } => manifest::Expect {
            eq: Some(text(value)),
            tol: tol.as_ref().map(text),
            ..manifest::Expect::default()
        },
        doc::Expect::Within { min, max } => manifest::Expect {
            min: min.as_ref().map(text),
            max: max.as_ref().map(text),
            ..manifest::Expect::default()
        },
    };
    manifest::Assertion {
        name: name.clone(),
        measure,
        expect,
        when: when.as_ref().map(text),
    }
}

fn assertion_to_doc(assertion: manifest::Assertion) -> doc::Assertion {
    let manifest::Assertion {
        name,
        measure,
        expect,
        when,
    } = assertion;
    let measure = match measure {
        serde_json::Value::String(text) => text,
        value => serde_json::to_string(&value).unwrap_or_default(),
    };
    let manifest::Expect {
        is,
        eq,
        tol,
        min,
        max,
    } = expect;
    // A combination that means nothing (no bound at all) is refused by the
    // document's own check after loading.
    let expect = match (is, eq) {
        (Some(is), _) => doc::Expect::Is(is),
        (None, Some(value)) => doc::Expect::Equal {
            value: doc::Expr(value),
            tol: tol.map(doc::Expr),
        },
        (None, None) => doc::Expect::Within {
            min: min.map(doc::Expr),
            max: max.map(doc::Expr),
        },
    };
    doc::Assertion {
        name,
        measure,
        expect,
        when: when.map(doc::Expr),
    }
}

fn appearance_to_manifest(appearance: doc::Appearance) -> manifest::Appearance {
    let doc::Appearance {
        color,
        metallic,
        roughness,
        optics,
    } = appearance;
    manifest::Appearance {
        color,
        metallic,
        roughness,
        optics: optics_to_manifest(optics),
    }
}

fn appearance_to_doc(appearance: manifest::Appearance) -> doc::Appearance {
    let manifest::Appearance {
        color,
        metallic,
        roughness,
        optics,
    } = appearance;
    doc::Appearance {
        color,
        metallic,
        roughness,
        optics: optics_to_doc(optics),
    }
}

/// The layers as the file keeps them: `None` for none, and each field at
/// its default left out.
fn optics_to_manifest(optics: doc::Optics) -> Option<manifest::Optics> {
    if optics.is_none() {
        return None;
    }
    let doc::Optics {
        anisotropy,
        direction,
        clearcoat,
        clearcoat_roughness,
        transmission,
        ior,
        thin_walled,
        absorption,
        absorption_distance,
        subsurface,
        subsurface_color,
    } = optics;
    Some(manifest::Optics {
        anisotropy,
        direction: match direction {
            doc::AnisotropyDirection::Parametric => manifest::AnisotropyDirection::Parametric,
            doc::AnisotropyDirection::Axis(axis) => manifest::AnisotropyDirection::Axis(axis),
        },
        clearcoat,
        clearcoat_roughness,
        transmission,
        ior: (ior != doc::Optics::default_ior()).then_some(ior),
        thin_walled,
        absorption,
        absorption_distance: (absorption_distance != doc::Optics::default_absorption_distance())
            .then_some(absorption_distance),
        subsurface,
        subsurface_color,
    })
}

fn optics_to_doc(optics: Option<manifest::Optics>) -> doc::Optics {
    let Some(optics) = optics else {
        return doc::Optics::default();
    };
    let manifest::Optics {
        anisotropy,
        direction,
        clearcoat,
        clearcoat_roughness,
        transmission,
        ior,
        thin_walled,
        absorption,
        absorption_distance,
        subsurface,
        subsurface_color,
    } = optics;
    doc::Optics {
        anisotropy,
        direction: match direction {
            manifest::AnisotropyDirection::Parametric => doc::AnisotropyDirection::Parametric,
            manifest::AnisotropyDirection::Axis(axis) => doc::AnisotropyDirection::Axis(axis),
        },
        clearcoat,
        clearcoat_roughness,
        transmission,
        ior: ior.unwrap_or_else(doc::Optics::default_ior),
        thin_walled,
        absorption,
        absorption_distance: absorption_distance
            .unwrap_or_else(doc::Optics::default_absorption_distance),
        subsurface,
        subsurface_color,
    }
}

fn face_to_manifest(face: &doc::FaceProperties) -> manifest::FaceProperties {
    let doc::FaceProperties {
        feature: doc::FeatureId(feature),
        face,
        appearance,
    } = face;
    manifest::FaceProperties {
        feature: *feature,
        face: name_to_manifest(face),
        appearance: appearance_to_manifest(*appearance),
    }
}

fn face_to_doc(face: manifest::FaceProperties) -> doc::FaceProperties {
    let manifest::FaceProperties {
        feature,
        face,
        appearance,
    } = face;
    doc::FaceProperties {
        feature: doc::FeatureId(feature),
        face: name_to_doc(face),
        appearance: appearance_to_doc(appearance),
    }
}

fn body_to_manifest(body: &doc::BodyProperties) -> manifest::BodyProperties {
    let doc::BodyProperties {
        feature: doc::FeatureId(feature),
        appearance,
        material,
    } = body;
    manifest::BodyProperties {
        feature: *feature,
        appearance: appearance.as_ref().map(|appearance| {
            let doc::BodyAppearance {
                color,
                metallic,
                roughness,
                optics,
            } = appearance;
            manifest::BodyAppearance {
                color: match color {
                    doc::BodyColor::Fixed(color) => manifest::BodyColor::Fixed(*color),
                    doc::BodyColor::Exprs(exprs) => {
                        manifest::BodyColor::Exprs(exprs.clone().map(|e| e.0))
                    }
                },
                metallic: *metallic,
                roughness: *roughness,
                optics: optics_to_manifest(*optics),
            }
        }),
        material: material.as_ref().map(|material| {
            let doc::Material {
                name,
                density,
                density_expr,
            } = material;
            manifest::Material {
                name: name.clone(),
                density: *density,
                density_expr: density_expr.as_ref().map(|e| e.0.clone()),
            }
        }),
    }
}

fn body_to_doc(body: manifest::BodyProperties) -> doc::BodyProperties {
    let manifest::BodyProperties {
        feature,
        appearance,
        material,
    } = body;
    doc::BodyProperties {
        feature: doc::FeatureId(feature),
        appearance: appearance.map(|appearance| {
            let manifest::BodyAppearance {
                color,
                metallic,
                roughness,
                optics,
            } = appearance;
            doc::BodyAppearance {
                color: match color {
                    manifest::BodyColor::Fixed(color) => doc::BodyColor::Fixed(color),
                    manifest::BodyColor::Exprs(exprs) => {
                        doc::BodyColor::Exprs(Box::new(exprs.map(doc::Expr)))
                    }
                },
                metallic,
                roughness,
                optics: optics_to_doc(optics),
            }
        }),
        material: material.map(|material| {
            let manifest::Material {
                name,
                density,
                density_expr,
            } = material;
            doc::Material {
                name,
                density,
                density_expr: density_expr.map(doc::Expr),
            }
        }),
    }
}

fn feature_to_manifest(feature: &doc::Feature) -> manifest::Feature {
    let doc::Feature {
        id: doc::FeatureId(id),
        name,
        suppressed,
        when,
        kind,
    } = feature;
    manifest::Feature {
        id: *id,
        name: name.clone(),
        suppressed: *suppressed,
        when: when.as_ref().map(|when| when.source().to_string()),
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
            doc::FeatureKind::Sketch(doc::Sketch {
                plane,
                entities,
                constraints,
                next_entity_id,
                next_constraint_id,
                patterns,
                next_pattern_id,
                image_places,
            }) => manifest::FeatureKind::Sketch {
                plane: match plane {
                    doc::SketchPlane::World { plane, offset } => manifest::SketchPlane::World {
                        plane: match plane {
                            doc::WorldPlane::Xy => manifest::WorldPlane::Xy,
                            doc::WorldPlane::Yz => manifest::WorldPlane::Yz,
                            doc::WorldPlane::Zx => manifest::WorldPlane::Zx,
                        },
                        offset: expr_to_manifest(offset),
                    },
                    doc::SketchPlane::Face {
                        face,
                        offset,
                        x_axis,
                    } => manifest::SketchPlane::Face {
                        face: name_to_manifest(face),
                        offset: expr_to_manifest(offset),
                        x_axis: x_axis.as_ref().map(|axis| axis.clone().map(|e| e.0)),
                    },
                    doc::SketchPlane::Datum {
                        plane: doc::FeatureId(plane),
                        offset,
                    } => manifest::SketchPlane::Datum {
                        plane: *plane,
                        offset: expr_to_manifest(offset),
                    },
                },
                entities: entities.iter().map(entity_to_manifest).collect(),
                constraints: constraints.iter().map(constraint_to_manifest).collect(),
                next_entity_id: *next_entity_id,
                next_constraint_id: *next_constraint_id,
                patterns: patterns.iter().map(pattern_to_manifest).collect(),
                next_pattern_id: *next_pattern_id,
                image_places: image_places
                    .iter()
                    .map(|p| manifest::ImagePlace {
                        entity: p.entity.0,
                        at: p.at,
                        map: p.map.map(|m| match m {
                            doc::PlaceMap::Shift { offset } => manifest::PlaceMap::Shift { offset },
                            doc::PlaceMap::Chain { k } => manifest::PlaceMap::Chain { k },
                            doc::PlaceMap::Turn { angle, at } => {
                                manifest::PlaceMap::Turn { angle, at }
                            }
                        }),
                    })
                    .collect(),
            },
            doc::FeatureKind::Extrude(doc::Extrude {
                sketch: doc::FeatureId(sketch),
                distance,
                symmetric,
                end,
                reversed,
            }) => manifest::FeatureKind::Extrude {
                sketch: *sketch,
                distance: expr_to_manifest(distance),
                symmetric: *symmetric,
                end: match end {
                    doc::ExtrudeEnd::Distance => manifest::ExtrudeEnd::Distance {},
                    doc::ExtrudeEnd::ThroughAll => manifest::ExtrudeEnd::ThroughAll {},
                    doc::ExtrudeEnd::UpToFace { face } => manifest::ExtrudeEnd::UpToFace {
                        face: name_to_manifest(face),
                    },
                },
                reversed: *reversed,
            },
            doc::FeatureKind::Loft(doc::Loft {
                sections,
                ruled,
                closed,
                guides,
            }) => manifest::FeatureKind::Loft {
                guides: guides.iter().map(rail_to_manifest).collect(),
                sections: sections
                    .iter()
                    .map(
                        |doc::LoftSection {
                             sketch: doc::FeatureId(sketch),
                             start,
                         }| manifest::LoftSection {
                            sketch: *sketch,
                            start: start.map(|doc::EntityId(id)| id),
                        },
                    )
                    .collect(),
                ruled: *ruled,
                closed: *closed,
            },
            doc::FeatureKind::Fill(doc::Fill {
                sketch: doc::FeatureId(sketch),
            }) => manifest::FeatureKind::Fill { sketch: *sketch },
            doc::FeatureKind::SheetMetal(doc::SheetMetal {
                sketch: doc::FeatureId(sketch),
                thickness,
                radius,
                k_factor,
                flanges,
                relief,
                cutouts,
                corner,
                corners,
                next_flange_id,
                bend_relief,
            }) => manifest::FeatureKind::SheetMetal {
                bend_relief: bend_relief
                    .as_ref()
                    .map(|doc::CornerRelief { shape, size }| manifest::SheetRelief {
                        shape: pick_to_manifest(shape, relief_shape_to_manifest),
                        size: expr_to_manifest(size),
                    }),
                // Written only when it is not past the flanges' ids, as a
                // flange's id only when it is not its place plus 1.
                next_flange_id: (*next_flange_id != next_past(flanges)).then_some(*next_flange_id),
                corner: corner.as_ref().map(|doc::CornerClose { kind, gap }| {
                    manifest::SheetCornerClose {
                        kind: pick_to_manifest(kind, corner_kind_to_manifest),
                        gap: gap.as_ref().map(expr_to_manifest),
                    }
                }),
                corners: corners
                    .iter()
                    .map(
                        |doc::SheetCorner {
                             flanges,
                             kind,
                             gap,
                             over,
                         }| manifest::SheetCorner {
                            flanges: *flanges,
                            kind: pick_to_manifest(kind, corner_kind_to_manifest),
                            gap: gap.as_ref().map(expr_to_manifest),
                            over: *over,
                        },
                    )
                    .collect(),
                cutouts: cutouts.iter().map(|c| c.0).collect(),
                sketch: *sketch,
                thickness: expr_to_manifest(thickness),
                radius: expr_to_manifest(radius),
                k_factor: expr_to_manifest(k_factor),
                flanges: flanges
                    .iter()
                    .enumerate()
                    .map(|(i, f)| flange_to_manifest(i, f))
                    .collect(),
                relief: relief.as_ref().map(|doc::CornerRelief { shape, size }| {
                    manifest::SheetRelief {
                        shape: pick_to_manifest(shape, relief_shape_to_manifest),
                        size: expr_to_manifest(size),
                    }
                }),
            },
            doc::FeatureKind::Fold(doc::Fold {
                sketch: doc::FeatureId(sketch),
                creases,
                fixed,
                panels,
            }) => manifest::FeatureKind::Fold {
                panels: panels.as_ref().map(
                    |doc::FoldPanels {
                         thickness,
                         clearance,
                         side,
                     }| manifest::FoldPanels {
                        thickness: expr_to_manifest(thickness),
                        clearance: expr_to_manifest(clearance),
                        side: match side {
                            doc::PanelSide::Back => manifest::PanelSide::Back,
                            doc::PanelSide::Front => manifest::PanelSide::Front,
                        },
                    },
                ),
                sketch: *sketch,
                creases: creases
                    .iter()
                    .map(
                        |doc::Crease {
                             line: doc::EntityId(line),
                             kind,
                             angle,
                             index,
                             alternate,
                         }| manifest::FoldCrease {
                            instance: match index {
                                doc::CurveIndex::Seed => None,
                                doc::CurveIndex::At(k) => Some(manifest::FoldInstance::At(*k)),
                                doc::CurveIndex::Grid(i, j) => {
                                    Some(manifest::FoldInstance::Grid([*i, *j]))
                                }
                                doc::CurveIndex::All => Some(manifest::FoldInstance::All),
                            },
                            alternate: *alternate,
                            line: *line,
                            kind: match kind {
                                doc::CreaseKind::Mountain => manifest::CreaseKind::Mountain,
                                doc::CreaseKind::Valley => manifest::CreaseKind::Valley,
                                doc::CreaseKind::Cut => manifest::CreaseKind::Cut,
                            },
                            angle: angle.as_ref().map(expr_to_manifest),
                        },
                    )
                    .collect(),
                fixed: fixed
                    .as_ref()
                    .map(|[x, y]| [expr_to_manifest(x), expr_to_manifest(y)]),
            },
            doc::FeatureKind::Split(doc::Split {
                source: doc::FeatureId(source),
                face,
                number,
            }) => manifest::FeatureKind::Split {
                source: *source,
                face: name_to_manifest(face),
                number: *number,
            },
            doc::FeatureKind::Developable(doc::Developable { a, b }) => {
                manifest::FeatureKind::Developable {
                    a: rail_to_manifest(a),
                    b: rail_to_manifest(b),
                }
            }
            doc::FeatureKind::SurfaceNetwork(doc::SurfaceNetwork { u, v, edges }) => {
                let doc::NetworkEdges {
                    u_first,
                    u_last,
                    v_first,
                    v_last,
                } = edges;
                manifest::FeatureKind::SurfaceNetwork {
                    u: u.iter().map(rail_to_manifest).collect(),
                    v: v.iter().map(rail_to_manifest).collect(),
                    edges: manifest::NetworkEdges {
                        u_first: hold_to_manifest(u_first),
                        u_last: hold_to_manifest(u_last),
                        v_first: hold_to_manifest(v_first),
                        v_last: hold_to_manifest(v_last),
                    },
                }
            }
            doc::FeatureKind::Unfold(doc::Unfold { faces, at, angle }) => {
                manifest::FeatureKind::Unfold {
                    faces: faces.iter().map(name_to_manifest).collect(),
                    at: at.clone().map(|e| expr_to_manifest(&e)),
                    angle: expr_to_manifest(angle),
                }
            }
            doc::FeatureKind::Import(doc::Import {
                file_name,
                solid,
                data,
            }) => manifest::FeatureKind::Import {
                file_name: file_name.clone(),
                solid: *solid,
                sha256: data.sha256(),
            },
            doc::FeatureKind::Sweep(doc::Sweep {
                sketch: doc::FeatureId(sketch),
                path: doc::FeatureId(path),
            }) => manifest::FeatureKind::Sweep {
                sketch: *sketch,
                path: *path,
            },
            doc::FeatureKind::OffsetProfile(doc::OffsetProfile {
                sketch: doc::FeatureId(sketch),
                distance,
                corners,
            }) => manifest::FeatureKind::OffsetProfile {
                sketch: *sketch,
                distance: expr_to_manifest(distance),
                corners: match corners {
                    doc::OffsetCorners::Round => manifest::OffsetCorners::Round,
                    doc::OffsetCorners::Sharp => manifest::OffsetCorners::Sharp,
                },
            },
            doc::FeatureKind::Revolve(doc::Revolve {
                sketch: doc::FeatureId(sketch),
                axis,
                angle,
                symmetric,
            }) => manifest::FeatureKind::Revolve {
                sketch: *sketch,
                axis: match axis {
                    doc::RevolveAxis::X => manifest::RevolveAxis::X,
                    doc::RevolveAxis::Y => manifest::RevolveAxis::Y,
                    doc::RevolveAxis::Line {
                        entity: doc::EntityId(entity),
                    } => manifest::RevolveAxis::Line { entity: *entity },
                    doc::RevolveAxis::Datum {
                        axis: doc::FeatureId(axis),
                    } => manifest::RevolveAxis::Datum { axis: *axis },
                },
                angle: expr_to_manifest(angle),
                symmetric: *symmetric,
            },
            doc::FeatureKind::Transform(doc::Transform {
                source: doc::FeatureId(source),
                translate,
                pivot,
                axis,
                angle,
                keep,
            }) => manifest::FeatureKind::Transform {
                source: *source,
                translate: exprs_to_manifest(translate),
                pivot: exprs_to_manifest(pivot),
                axis: exprs_to_manifest(axis),
                angle: expr_to_manifest(angle),
                keep: *keep,
            },
            doc::FeatureKind::Mirror(doc::Mirror {
                source: doc::FeatureId(source),
                point,
                normal,
                plane,
                keep,
            }) => manifest::FeatureKind::Mirror {
                source: *source,
                point: exprs_to_manifest(point),
                normal: exprs_to_manifest(normal),
                plane: plane.map(|doc::FeatureId(id)| id),
                keep: *keep,
            },
            doc::FeatureKind::DatumPoint(doc::DatumPoint { at }) => {
                manifest::FeatureKind::DatumPoint {
                    at: point_ref_to_manifest(at),
                }
            }
            doc::FeatureKind::DatumHelix(doc::DatumHelix {
                axis,
                start,
                pitch,
                extent,
                left_handed,
            }) => manifest::FeatureKind::DatumHelix {
                axis: axis_ref_to_manifest(axis),
                start: match start {
                    doc::HelixStart::Radius { radius } => manifest::HelixStart::Radius {
                        radius: expr_to_manifest(radius),
                    },
                    doc::HelixStart::Point { point } => manifest::HelixStart::Point {
                        point: point_ref_to_manifest(point),
                    },
                },
                pitch: expr_to_manifest(pitch),
                extent: match extent {
                    doc::HelixExtent::Turns { turns } => manifest::HelixExtent::Turns {
                        turns: expr_to_manifest(turns),
                    },
                    doc::HelixExtent::Height { height } => manifest::HelixExtent::Height {
                        height: expr_to_manifest(height),
                    },
                },
                left_handed: *left_handed,
            },
            doc::FeatureKind::OffsetFace(doc::OffsetFace { faces, distance }) => {
                manifest::FeatureKind::OffsetFace {
                    faces: faces.iter().map(name_to_manifest).collect(),
                    distance: expr_to_manifest(distance),
                }
            }
            doc::FeatureKind::MoveFace(doc::MoveFace {
                faces,
                translate,
                pivot,
                axis,
                angle,
            }) => manifest::FeatureKind::MoveFace {
                faces: faces.iter().map(name_to_manifest).collect(),
                translate: exprs_to_manifest(translate),
                pivot: exprs_to_manifest(pivot),
                axis: exprs_to_manifest(axis),
                angle: expr_to_manifest(angle),
            },
            doc::FeatureKind::Draft(doc::Draft {
                faces,
                neutral,
                angle,
                reversed,
            }) => manifest::FeatureKind::Draft {
                faces: faces.iter().map(name_to_manifest).collect(),
                neutral: plane_ref_to_manifest(neutral),
                angle: expr_to_manifest(angle),
                reversed: *reversed,
            },
            doc::FeatureKind::Plate(doc::Plate {
                faces,
                thickness,
                side,
                joints,
            }) => manifest::FeatureKind::Plate {
                faces: faces.iter().map(name_to_manifest).collect(),
                thickness: expr_to_manifest(thickness),
                side: plate_side_to_manifest(*side),
                joints: joints
                    .iter()
                    .map(
                        |doc::PlateJoint {
                             edge,
                             kind,
                             with,
                             thickness,
                             side,
                             angle,
                             onto,
                         }| manifest::PlateJoint {
                            edge: name_to_manifest(edge),
                            kind: match kind {
                                doc::JointKind::Square => manifest::JointKind::Square,
                                doc::JointKind::Bevel => manifest::JointKind::Bevel,
                                doc::JointKind::Mitre => manifest::JointKind::Mitre,
                                doc::JointKind::Touch => manifest::JointKind::Touch,
                                doc::JointKind::Over => manifest::JointKind::Over,
                                doc::JointKind::Under => manifest::JointKind::Under,
                                doc::JointKind::Onto => manifest::JointKind::Onto,
                            },
                            with: with.as_ref().map(name_to_manifest),
                            thickness: thickness.as_ref().map(expr_to_manifest),
                            side: side.map(plate_side_to_manifest),
                            angle: angle.as_ref().map(expr_to_manifest),
                            onto: onto.as_ref().map(plane_ref_to_manifest),
                        },
                    )
                    .collect(),
            },
            doc::FeatureKind::Thicken(doc::Thicken {
                faces,
                thickness,
                side,
            }) => manifest::FeatureKind::Thicken {
                faces: faces.iter().map(name_to_manifest).collect(),
                thickness: expr_to_manifest(thickness),
                side: match side {
                    doc::ThickenSide::Outside => manifest::ThickenSide::Outside,
                    doc::ThickenSide::Inside => manifest::ThickenSide::Inside,
                    doc::ThickenSide::Both => manifest::ThickenSide::Both,
                },
            },
            doc::FeatureKind::DeleteFace(doc::DeleteFace { faces }) => {
                manifest::FeatureKind::DeleteFace {
                    faces: faces.iter().map(name_to_manifest).collect(),
                }
            }
            doc::FeatureKind::Thread(doc::Thread {
                face,
                start,
                size,
                pitch,
                length,
                left_handed,
                modeled,
            }) => manifest::FeatureKind::Thread {
                face: name_to_manifest(face),
                start: name_to_manifest(start),
                size: size.as_ref().map(|size| match size {
                    doc::ThreadSize::Designation(text) => {
                        manifest::ThreadSize::Designation(text.clone())
                    }
                    doc::ThreadSize::Diameter { diameter } => {
                        manifest::ThreadSize::Diameter(manifest::ThreadDiameter {
                            diameter: expr_to_manifest(diameter),
                        })
                    }
                }),
                pitch: pitch.as_ref().map(expr_to_manifest),
                length: expr_to_manifest(length),
                left_handed: *left_handed,
                modeled: *modeled,
            },
            doc::FeatureKind::Shell(doc::Shell {
                source: doc::FeatureId(source),
                thickness,
                open,
            }) => manifest::FeatureKind::Shell {
                source: *source,
                thickness: expr_to_manifest(thickness),
                open: open.iter().map(name_to_manifest).collect(),
            },
            doc::FeatureKind::Fillet(doc::Fillet {
                edges,
                radius,
                chain,
            }) => manifest::FeatureKind::Fillet {
                edges: edges
                    .iter()
                    .map(|[first, second]| [name_to_manifest(first), name_to_manifest(second)])
                    .collect(),
                radius: expr_to_manifest(radius),
                chain: *chain,
            },
            doc::FeatureKind::Chamfer(doc::Chamfer { edges, size, chain }) => {
                manifest::FeatureKind::Chamfer {
                    edges: edges
                        .iter()
                        .map(|[first, second]| [name_to_manifest(first), name_to_manifest(second)])
                        .collect(),
                    size: match size {
                        doc::ChamferSize::Equal { distance } => manifest::ChamferSize::Equal {
                            distance: expr_to_manifest(distance),
                        },
                        doc::ChamferSize::TwoDistances { distance, second } => {
                            manifest::ChamferSize::TwoDistances {
                                distance: expr_to_manifest(distance),
                                second: expr_to_manifest(second),
                            }
                        }
                        doc::ChamferSize::DistanceAngle { distance, angle } => {
                            manifest::ChamferSize::DistanceAngle {
                                distance: expr_to_manifest(distance),
                                angle: expr_to_manifest(angle),
                            }
                        }
                    },
                    chain: *chain,
                }
            }
            doc::FeatureKind::DatumAxis(doc::DatumAxis { by }) => {
                manifest::FeatureKind::DatumAxis {
                    by: match by {
                        doc::AxisBy::TwoPoints { points } => {
                            let [a, b] = points;
                            manifest::AxisBy::TwoPoints {
                                points: [point_ref_to_manifest(a), point_ref_to_manifest(b)],
                            }
                        }
                        doc::AxisBy::Edge { between } => {
                            let [a, b] = between;
                            manifest::AxisBy::Edge {
                                between: [name_to_manifest(a), name_to_manifest(b)],
                            }
                        }
                        doc::AxisBy::Face { face } => manifest::AxisBy::Face {
                            face: name_to_manifest(face),
                        },
                        doc::AxisBy::TwoPlanes { planes } => {
                            let [a, b] = planes;
                            manifest::AxisBy::TwoPlanes {
                                planes: [plane_ref_to_manifest(a), plane_ref_to_manifest(b)],
                            }
                        }
                    },
                }
            }
            doc::FeatureKind::DatumPlane(doc::DatumPlane { by }) => {
                manifest::FeatureKind::DatumPlane {
                    by: match by {
                        doc::PlaneBy::Offset { from, distance } => manifest::PlaneBy::Offset {
                            from: plane_ref_to_manifest(from),
                            distance: expr_to_manifest(distance),
                        },
                        doc::PlaneBy::Angle { from, about, angle } => manifest::PlaneBy::Angle {
                            from: plane_ref_to_manifest(from),
                            about: axis_ref_to_manifest(about),
                            angle: expr_to_manifest(angle),
                        },
                        doc::PlaneBy::ThreePoints { points } => {
                            let [a, b, c] = points;
                            manifest::PlaneBy::ThreePoints {
                                points: [
                                    point_ref_to_manifest(a),
                                    point_ref_to_manifest(b),
                                    point_ref_to_manifest(c),
                                ],
                            }
                        }
                    },
                }
            }
            doc::FeatureKind::Boolean(doc::Boolean {
                op,
                target: doc::FeatureId(target),
                tool: doc::FeatureId(tool),
                keep_tool,
            }) => manifest::FeatureKind::Boolean {
                op: match op {
                    doc::BooleanOp::Union => manifest::BooleanOp::Union,
                    doc::BooleanOp::Subtract => manifest::BooleanOp::Subtract,
                    doc::BooleanOp::Intersect => manifest::BooleanOp::Intersect,
                },
                target: *target,
                tool: *tool,
                keep_tool: *keep_tool,
            },
            doc::FeatureKind::Pattern(doc::Pattern {
                source: doc::FeatureId(source),
                count,
                layout,
            }) => manifest::FeatureKind::Pattern {
                source: *source,
                count: expr_to_manifest(count),
                layout: match layout {
                    doc::PatternLayout::Linear { step } => manifest::PatternLayout::Linear {
                        step: exprs_to_manifest(step),
                    },
                    doc::PatternLayout::Circular {
                        point,
                        axis,
                        angle,
                        about,
                        step,
                    } => manifest::PatternLayout::Circular {
                        point: exprs_to_manifest(point),
                        axis: exprs_to_manifest(axis),
                        angle: expr_to_manifest(angle),
                        about: about.map(|doc::FeatureId(id)| id),
                        step: step.as_ref().map(expr_to_manifest),
                    },
                },
            },
        },
    }
}

fn world_plane_to_manifest(plane: doc::WorldPlane) -> manifest::WorldPlane {
    match plane {
        doc::WorldPlane::Xy => manifest::WorldPlane::Xy,
        doc::WorldPlane::Yz => manifest::WorldPlane::Yz,
        doc::WorldPlane::Zx => manifest::WorldPlane::Zx,
    }
}

fn world_plane_to_doc(plane: manifest::WorldPlane) -> doc::WorldPlane {
    match plane {
        manifest::WorldPlane::Xy => doc::WorldPlane::Xy,
        manifest::WorldPlane::Yz => doc::WorldPlane::Yz,
        manifest::WorldPlane::Zx => doc::WorldPlane::Zx,
    }
}

fn plane_ref_to_manifest(plane: &doc::PlaneRef) -> manifest::PlaneRef {
    match plane {
        doc::PlaneRef::World { plane } => manifest::PlaneRef::World {
            plane: world_plane_to_manifest(*plane),
        },
        doc::PlaneRef::Face { face } => manifest::PlaneRef::Face {
            face: name_to_manifest(face),
        },
        doc::PlaneRef::Datum {
            feature: doc::FeatureId(feature),
        } => manifest::PlaneRef::Datum { feature: *feature },
    }
}

fn plane_ref_to_doc(plane: manifest::PlaneRef) -> doc::PlaneRef {
    match plane {
        manifest::PlaneRef::World { plane } => doc::PlaneRef::World {
            plane: world_plane_to_doc(plane),
        },
        manifest::PlaneRef::Face { face } => doc::PlaneRef::Face {
            face: name_to_doc(face),
        },
        manifest::PlaneRef::Datum { feature } => doc::PlaneRef::Datum {
            feature: doc::FeatureId(feature),
        },
    }
}

fn axis_ref_to_manifest(axis: &doc::AxisRef) -> manifest::AxisRef {
    match axis {
        doc::AxisRef::World { axis } => manifest::AxisRef::World {
            axis: match axis {
                doc::WorldAxis::X => manifest::WorldAxis::X,
                doc::WorldAxis::Y => manifest::WorldAxis::Y,
                doc::WorldAxis::Z => manifest::WorldAxis::Z,
            },
        },
        doc::AxisRef::Edge { between } => {
            let [a, b] = between;
            manifest::AxisRef::Edge {
                between: [name_to_manifest(a), name_to_manifest(b)],
            }
        }
        doc::AxisRef::Datum {
            feature: doc::FeatureId(feature),
        } => manifest::AxisRef::Datum { feature: *feature },
    }
}

fn axis_ref_to_doc(axis: manifest::AxisRef) -> doc::AxisRef {
    match axis {
        manifest::AxisRef::World { axis } => doc::AxisRef::World {
            axis: match axis {
                manifest::WorldAxis::X => doc::WorldAxis::X,
                manifest::WorldAxis::Y => doc::WorldAxis::Y,
                manifest::WorldAxis::Z => doc::WorldAxis::Z,
            },
        },
        manifest::AxisRef::Edge { between } => doc::AxisRef::Edge {
            between: between.map(name_to_doc),
        },
        manifest::AxisRef::Datum { feature } => doc::AxisRef::Datum {
            feature: doc::FeatureId(feature),
        },
    }
}

fn point_ref_to_manifest(point: &doc::PointRef) -> manifest::PointRef {
    match point {
        doc::PointRef::At { position } => manifest::PointRef::At {
            position: exprs_to_manifest(position),
        },
        doc::PointRef::Vertex { vertex } => manifest::PointRef::Vertex {
            vertex: name_to_manifest(vertex),
        },
        doc::PointRef::Center { between } => {
            let [a, b] = between;
            manifest::PointRef::Center {
                between: [name_to_manifest(a), name_to_manifest(b)],
            }
        }
        doc::PointRef::Datum {
            feature: doc::FeatureId(feature),
        } => manifest::PointRef::Datum { feature: *feature },
    }
}

fn point_ref_to_doc(point: manifest::PointRef) -> doc::PointRef {
    match point {
        manifest::PointRef::At { position } => doc::PointRef::At {
            position: exprs_to_doc(position),
        },
        manifest::PointRef::Vertex { vertex } => doc::PointRef::Vertex {
            vertex: name_to_doc(vertex),
        },
        manifest::PointRef::Center { between } => doc::PointRef::Center {
            between: between.map(name_to_doc),
        },
        manifest::PointRef::Datum { feature } => doc::PointRef::Datum {
            feature: doc::FeatureId(feature),
        },
    }
}

fn role_to_manifest(role: doc::NameRole) -> manifest::NameRole {
    use doc::NameRole as D;
    use manifest::NameRole as M;
    match role {
        D::StartCap => M::StartCap,
        D::EndCap => M::EndCap,
        D::Side => M::Side,
        D::StartEdge => M::StartEdge,
        D::EndEdge => M::EndEdge,
        D::LateralEdge => M::LateralEdge,
        D::StartVertex => M::StartVertex,
        D::EndVertex => M::EndVertex,
        D::Copy => M::Copy,
        D::Piece => M::Piece,
        D::Face => M::Face,
        D::Edge => M::Edge,
        D::Vertex => M::Vertex,
        D::Between => M::Between,
        D::Corner => M::Corner,
        D::Chamfer => M::Chamfer,
        D::ChamferCorner => M::ChamferCorner,
        D::Fillet => M::Fillet,
        D::FilletCorner => M::FilletCorner,
        D::Thread => M::Thread,
        D::Rim => M::Rim,
    }
}

fn role_to_doc(role: manifest::NameRole) -> doc::NameRole {
    use doc::NameRole as D;
    use manifest::NameRole as M;
    match role {
        M::StartCap => D::StartCap,
        M::EndCap => D::EndCap,
        M::Side => D::Side,
        M::StartEdge => D::StartEdge,
        M::EndEdge => D::EndEdge,
        M::LateralEdge => D::LateralEdge,
        M::StartVertex => D::StartVertex,
        M::EndVertex => D::EndVertex,
        M::Copy => D::Copy,
        M::Piece => D::Piece,
        M::Face => D::Face,
        M::Edge => D::Edge,
        M::Vertex => D::Vertex,
        M::Between => D::Between,
        M::Corner => D::Corner,
        M::Chamfer => D::Chamfer,
        M::ChamferCorner => D::ChamferCorner,
        M::Fillet => D::Fillet,
        M::FilletCorner => D::FilletCorner,
        M::Thread => D::Thread,
        M::Rim => D::Rim,
    }
}

fn pick_to_manifest<T: Copy, U>(pick: &doc::Pick<T>, named: fn(T) -> U) -> manifest::Pick<U> {
    match pick {
        doc::Pick::Named(t) => manifest::Pick::Named(named(*t)),
        doc::Pick::Expr(e) => manifest::Pick::Expr(manifest::PickExpr {
            expr: expr_to_manifest(e),
        }),
    }
}

fn pick_to_doc<T, U>(pick: manifest::Pick<U>, named: fn(U) -> T) -> doc::Pick<T> {
    match pick {
        manifest::Pick::Named(u) => doc::Pick::Named(named(u)),
        manifest::Pick::Expr(manifest::PickExpr { expr }) => doc::Pick::Expr(doc::Expr(expr)),
    }
}

fn corner_kind_to_manifest(kind: doc::CornerKind) -> manifest::CornerKind {
    match kind {
        doc::CornerKind::Open => manifest::CornerKind::Open,
        doc::CornerKind::Butt => manifest::CornerKind::Butt,
        doc::CornerKind::Overlap => manifest::CornerKind::Overlap,
        doc::CornerKind::Miter => manifest::CornerKind::Miter,
    }
}

fn corner_kind_to_doc(kind: manifest::CornerKind) -> doc::CornerKind {
    match kind {
        manifest::CornerKind::Open => doc::CornerKind::Open,
        manifest::CornerKind::Butt => doc::CornerKind::Butt,
        manifest::CornerKind::Overlap => doc::CornerKind::Overlap,
        manifest::CornerKind::Miter => doc::CornerKind::Miter,
    }
}

fn relief_shape_to_manifest(shape: doc::ReliefShape) -> manifest::ReliefShape {
    match shape {
        doc::ReliefShape::Rect => manifest::ReliefShape::Rect,
        doc::ReliefShape::Round => manifest::ReliefShape::Round,
        doc::ReliefShape::Tear => manifest::ReliefShape::Tear,
    }
}

fn relief_shape_to_doc(shape: manifest::ReliefShape) -> doc::ReliefShape {
    match shape {
        manifest::ReliefShape::Rect => doc::ReliefShape::Rect,
        manifest::ReliefShape::Round => doc::ReliefShape::Round,
        manifest::ReliefShape::Tear => doc::ReliefShape::Tear,
    }
}

/// The id a flange without one has: its place in the list plus 1.
fn place_id(i: usize) -> u32 {
    u32::try_from(i).map_or(u32::MAX, |i| i.saturating_add(1))
}

/// The next flange id past every flange's.
fn next_past(flanges: &[doc::Flange]) -> u32 {
    flanges
        .iter()
        .map(|f| f.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn flange_to_manifest(place: usize, flange: &doc::Flange) -> manifest::SheetFlange {
    let doc::Flange {
        on,
        length,
        angle,
        direction,
        radius,
        k_factor,
        from,
        inset_start,
        inset_end,
        outline,
        id,
    } = flange;
    let (base, tip, side, line) = match on {
        doc::FlangeOn::Base(doc::EntityId(id)) => (Some(*id), None, None, None),
        doc::FlangeOn::Tip(i) => (None, Some(*i), None, None),
        doc::FlangeOn::Side(i, end) => (
            None,
            None,
            Some(manifest::SheetFlangeSide {
                flange: *i,
                end: match end {
                    doc::FlangeSide::Start => manifest::FlangeSideEnd::Start,
                    doc::FlangeSide::End => manifest::FlangeSideEnd::End,
                },
            }),
            None,
        ),
        doc::FlangeOn::Line(i, doc::EntityId(l)) => (
            None,
            None,
            None,
            Some(manifest::SheetFlangeLine {
                flange: *i,
                line: *l,
            }),
        ),
    };
    manifest::SheetFlange {
        line,
        id: (*id != place_id(place)).then_some(*id),
        base,
        tip,
        side,
        inset_start: inset_start.as_ref().map(expr_to_manifest),
        inset_end: inset_end.as_ref().map(expr_to_manifest),
        outline: outline.map(|o| o.0),
        length: expr_to_manifest(length),
        angle: expr_to_manifest(angle),
        direction: match direction {
            doc::BendDirection::Up => manifest::BendDirection::Up,
            doc::BendDirection::Down => manifest::BendDirection::Down,
        },
        radius: radius.as_ref().map(expr_to_manifest),
        k_factor: k_factor.as_ref().map(expr_to_manifest),
        from: match from {
            doc::FlangeFrom::Tangent => manifest::FlangeFrom::Tangent,
            doc::FlangeFrom::Mould => manifest::FlangeFrom::Mould,
        },
    }
}

// A flange on neither or both is read as on the tip of flange u32::MAX,
// which the part refuses by its place, rather than dropped.
/// A flange without an id is its place plus 1 (a part written before
/// ids, or one written compactly); one written with an id keeps it, also
/// a 0, which validation refuses.
fn flange_to_doc(place: usize, flange: manifest::SheetFlange) -> doc::Flange {
    let manifest::SheetFlange {
        base,
        tip,
        length,
        angle,
        direction,
        radius,
        k_factor,
        from,
        side,
        inset_start,
        inset_end,
        outline,
        id,
        line,
    } = flange;
    doc::Flange {
        id: id.unwrap_or_else(|| place_id(place)),
        on: match (base, tip, side, line) {
            (Some(id), None, None, None) => doc::FlangeOn::Base(doc::EntityId(id)),
            (None, Some(i), None, None) => doc::FlangeOn::Tip(i),
            (None, None, None, Some(l)) => doc::FlangeOn::Line(l.flange, doc::EntityId(l.line)),
            (None, None, Some(side), None) => doc::FlangeOn::Side(
                side.flange,
                match side.end {
                    manifest::FlangeSideEnd::Start => doc::FlangeSide::Start,
                    manifest::FlangeSideEnd::End => doc::FlangeSide::End,
                },
            ),
            _ => doc::FlangeOn::Tip(u32::MAX),
        },
        inset_start: inset_start.map(doc::Expr),
        inset_end: inset_end.map(doc::Expr),
        outline: outline.map(doc::FeatureId),
        length: doc::Expr(length),
        angle: doc::Expr(angle),
        direction: match direction {
            manifest::BendDirection::Up => doc::BendDirection::Up,
            manifest::BendDirection::Down => doc::BendDirection::Down,
        },
        radius: radius.map(doc::Expr),
        k_factor: k_factor.map(doc::Expr),
        from: match from {
            manifest::FlangeFrom::Tangent => doc::FlangeFrom::Tangent,
            manifest::FlangeFrom::Mould => doc::FlangeFrom::Mould,
        },
    }
}

fn rail_to_manifest(rail: &doc::Rail) -> manifest::Rail {
    match rail {
        doc::Rail::Sketch(doc::FeatureId(id)) => manifest::Rail::Sketch(*id),
        doc::Rail::Edges(edges) => {
            manifest::Rail::Edges(edges.iter().map(name_to_manifest).collect())
        }
    }
}

fn hold_to_manifest(hold: &doc::EdgeHold) -> manifest::EdgeHold {
    match hold {
        doc::EdgeHold::Free => manifest::EdgeHold::Free,
        doc::EdgeHold::Position { face } => manifest::EdgeHold::Position {
            face: name_to_manifest(face),
        },
        doc::EdgeHold::Tangent { face, weight } => manifest::EdgeHold::Tangent {
            face: name_to_manifest(face),
            weight: expr_to_manifest(weight),
        },
        doc::EdgeHold::Curvature { face, weight } => manifest::EdgeHold::Curvature {
            face: name_to_manifest(face),
            weight: expr_to_manifest(weight),
        },
    }
}

fn hold_to_doc(hold: manifest::EdgeHold) -> doc::EdgeHold {
    match hold {
        manifest::EdgeHold::Free => doc::EdgeHold::Free,
        manifest::EdgeHold::Position { face } => doc::EdgeHold::Position {
            face: name_to_doc(face),
        },
        manifest::EdgeHold::Tangent { face, weight } => doc::EdgeHold::Tangent {
            face: name_to_doc(face),
            weight: doc::Expr(weight),
        },
        manifest::EdgeHold::Curvature { face, weight } => doc::EdgeHold::Curvature {
            face: name_to_doc(face),
            weight: doc::Expr(weight),
        },
    }
}

fn rail_to_doc(rail: manifest::Rail) -> doc::Rail {
    match rail {
        manifest::Rail::Sketch(id) => doc::Rail::Sketch(doc::FeatureId(id)),
        manifest::Rail::Edges(edges) => {
            doc::Rail::Edges(edges.into_iter().map(name_to_doc).collect())
        }
    }
}

fn plate_side_to_manifest(side: doc::PlateSide) -> manifest::PlateSide {
    match side {
        doc::PlateSide::In => manifest::PlateSide::In,
        doc::PlateSide::Out => manifest::PlateSide::Out,
        doc::PlateSide::Mid => manifest::PlateSide::Mid,
    }
}

fn plate_side_to_doc(side: manifest::PlateSide) -> doc::PlateSide {
    match side {
        manifest::PlateSide::In => doc::PlateSide::In,
        manifest::PlateSide::Out => doc::PlateSide::Out,
        manifest::PlateSide::Mid => doc::PlateSide::Mid,
    }
}

// A name is a chain (a copy of a copy of a face), walked in a loop so a
// hostile file cannot overflow the stack; `Document::validate` refuses a
// chain longer than `doc::MAX_NAME_DEPTH`. An edge or a vertex lists the
// faces it lies on, which list none themselves: with `lists` those are
// converted, each as a chain without, and never deeper. A list in a place
// where there is none to be is kept as one empty name per entry on the way
// in, so that validation refuses the name instead of the list being
// dropped.
fn name_to_manifest(name: &doc::PersistentName) -> manifest::Name {
    name_to_manifest_with(name, true)
}

fn name_to_manifest_with(name: &doc::PersistentName, lists: bool) -> manifest::Name {
    let listed = |names: &[doc::PersistentName]| -> Vec<manifest::Name> {
        if lists {
            names
                .iter()
                .map(|face| name_to_manifest_with(face, false))
                .collect()
        } else {
            Vec::new()
        }
    };
    let mut links = Vec::new();
    let mut next = Some(name);
    while let Some(link) = next {
        let doc::PersistentName {
            feature: doc::FeatureId(feature),
            role,
            sources,
            of,
            by,
        } = link;
        let faces = by.as_deref().map(|doc::FaceLists { on, near }| {
            Box::new(manifest::NameFaces {
                on: listed(on),
                near: listed(near),
            })
        });
        links.push(manifest::Name {
            feature: *feature,
            role: role_to_manifest(*role),
            sources: sources.clone(),
            of: None,
            faces,
        });
        next = of.as_deref();
    }
    let mut built: Option<manifest::Name> = None;
    while let Some(mut link) = links.pop() {
        link.of = built.take().map(Box::new);
        built = Some(link);
    }
    built.unwrap_or(manifest::Name {
        feature: name.feature.0,
        role: role_to_manifest(name.role),
        sources: name.sources.clone(),
        of: None,
        faces: None,
    })
}

pub(crate) fn name_to_doc(name: manifest::Name) -> doc::PersistentName {
    // A file written before format version 15 has a copy of a copy as two
    // links (`F9:copy(0)/F7:copy(1)/…`); it means the same entity as the
    // one link regeneration names it by now (`F9:copy(0,1)/…`).
    name_to_doc_with(name, true).canonical()
}

fn name_to_doc_with(name: manifest::Name, lists: bool) -> doc::PersistentName {
    // A face that lists faces is not a name: it is read as a face that
    // lists as many names that are none, which validation refuses.
    let not_a_name =
        || doc::PersistentName::new(doc::FeatureId(0), doc::NameRole::Copy, Vec::new());
    let listed = |names: Vec<manifest::Name>| -> Vec<doc::PersistentName> {
        names
            .into_iter()
            .map(|face| {
                if lists {
                    name_to_doc_with(face, false)
                } else {
                    not_a_name()
                }
            })
            .collect()
    };
    let mut links = Vec::new();
    let mut next = Some(name);
    while let Some(link) = next {
        let manifest::Name {
            feature,
            role,
            sources,
            of,
            faces,
        } = link;
        let (on, near) = match faces.map(|boxed| *boxed) {
            Some(manifest::NameFaces { on, near }) => (listed(on), listed(near)),
            None => (Vec::new(), Vec::new()),
        };
        // In the order names are compared in: a file with the faces of
        // an edge the other way round names the same edge.
        links.push(
            doc::PersistentName::new(doc::FeatureId(feature), role_to_doc(role), sources)
                .with_faces(on, near),
        );
        next = of.map(|boxed| *boxed);
    }
    let mut built: Option<doc::PersistentName> = None;
    while let Some(mut link) = links.pop() {
        link.of = built.take().map(std::sync::Arc::new);
        built = Some(link);
    }
    // There is always at least one link; the fallback is never used.
    built.unwrap_or_else(|| {
        doc::PersistentName::new(doc::FeatureId(0), doc::NameRole::Side, Vec::new())
    })
}

fn entity_to_manifest(entity: &doc::Entity) -> manifest::SketchEntity {
    let doc::Entity {
        id: doc::EntityId(id),
        construction,
        geometry,
    } = entity;
    let raw = |doc::EntityId(id): &doc::EntityId| *id;
    manifest::SketchEntity {
        id: *id,
        construction: *construction,
        geometry: match geometry {
            doc::Geometry::Point { x, y } => manifest::SketchGeometry::Point { x: *x, y: *y },
            doc::Geometry::Line { start, end } => manifest::SketchGeometry::Line {
                start: raw(start),
                end: raw(end),
            },
            doc::Geometry::Circle { center, radius } => manifest::SketchGeometry::Circle {
                center: raw(center),
                radius: *radius,
            },
            doc::Geometry::Arc { center, start, end } => manifest::SketchGeometry::Arc {
                center: raw(center),
                start: raw(start),
                end: raw(end),
            },
            doc::Geometry::Spline { degree, control } => manifest::SketchGeometry::Spline {
                degree: *degree,
                control: control.iter().map(raw).collect(),
            },
            doc::Geometry::Formula {
                x,
                y,
                from,
                to,
                start,
                end,
            } => manifest::SketchGeometry::Formula {
                x: x.source().to_string(),
                y: y.source().to_string(),
                from: from.source().to_string(),
                to: to.source().to_string(),
                start: raw(start),
                end: raw(end),
            },
            doc::Geometry::Interpolated { degree, through } => {
                manifest::SketchGeometry::Interpolated {
                    degree: *degree,
                    through: through.iter().map(raw).collect(),
                }
            }
            doc::Geometry::Image {
                of,
                pattern: doc::PatternId(pattern),
                index,
            } => manifest::SketchGeometry::Image {
                of: raw(of),
                pattern: *pattern,
                index: match index {
                    doc::InstanceIndex::At(k) => manifest::InstanceIndex::At(*k),
                    doc::InstanceIndex::Grid(ij) => manifest::InstanceIndex::Grid(*ij),
                    doc::InstanceIndex::Last => manifest::InstanceIndex::Last,
                },
            },
        },
    }
}

fn index_to_manifest(index: &doc::InstanceIndex) -> manifest::InstanceIndex {
    match index {
        doc::InstanceIndex::At(k) => manifest::InstanceIndex::At(*k),
        doc::InstanceIndex::Grid(ij) => manifest::InstanceIndex::Grid(*ij),
        doc::InstanceIndex::Last => manifest::InstanceIndex::Last,
    }
}

fn index_to_doc(index: manifest::InstanceIndex) -> doc::InstanceIndex {
    match index {
        manifest::InstanceIndex::At(k) => doc::InstanceIndex::At(k),
        manifest::InstanceIndex::Grid(ij) => doc::InstanceIndex::Grid(ij),
        manifest::InstanceIndex::Last => doc::InstanceIndex::Last,
    }
}

fn pattern_to_manifest(pattern: &doc::SketchPattern) -> manifest::SketchPattern {
    let doc::SketchPattern {
        id: doc::PatternId(id),
        seed,
        count,
        layout,
        patterns,
        skip,
    } = pattern;
    let raw = |doc::EntityId(id): &doc::EntityId| *id;
    let pair = |[a, b]: &[doc::Expr; 2]| [expr_to_manifest(a), expr_to_manifest(b)];
    manifest::SketchPattern {
        id: *id,
        seed: seed.iter().map(raw).collect(),
        patterns: patterns.iter().map(|doc::PatternId(id)| *id).collect(),
        skip: skip.iter().map(index_to_manifest).collect(),
        count: expr_to_manifest(count),
        layout: match layout {
            doc::SketchPatternLayout::Linear { step, second } => {
                manifest::SketchPatternLayout::Linear {
                    step: pair(step),
                    second: second.as_ref().map(|doc::SecondDirection { step, count }| {
                        manifest::SecondDirection {
                            step: pair(step),
                            count: expr_to_manifest(count),
                        }
                    }),
                }
            }
            doc::SketchPatternLayout::Chain { from, to } => manifest::SketchPatternLayout::Chain {
                from: raw(from),
                to: raw(to),
            },
            doc::SketchPatternLayout::Circular {
                center,
                angle,
                chain,
            } => manifest::SketchPatternLayout::Circular {
                center: match center {
                    doc::CenterRef::Point { point } => {
                        manifest::CenterRef::Point { point: raw(point) }
                    }
                    doc::CenterRef::At { at } => manifest::CenterRef::At { at: pair(at) },
                },
                angle: expr_to_manifest(angle),
                chain: chain.as_ref().map(|[a, b]| [raw(a), raw(b)]),
            },
        },
    }
}

fn pattern_to_doc(pattern: manifest::SketchPattern) -> doc::SketchPattern {
    use doc::EntityId as Id;
    let manifest::SketchPattern {
        id,
        seed,
        count,
        layout,
        patterns,
        skip,
    } = pattern;
    let pair = |[a, b]: [String; 2]| [doc::Expr(a), doc::Expr(b)];
    doc::SketchPattern {
        id: doc::PatternId(id),
        seed: seed.into_iter().map(Id).collect(),
        patterns: patterns.into_iter().map(doc::PatternId).collect(),
        skip: skip.into_iter().map(index_to_doc).collect(),
        count: doc::Expr(count),
        layout: match layout {
            manifest::SketchPatternLayout::Linear { step, second } => {
                doc::SketchPatternLayout::Linear {
                    step: pair(step),
                    second: second.map(|manifest::SecondDirection { step, count }| {
                        doc::SecondDirection {
                            step: pair(step),
                            count: doc::Expr(count),
                        }
                    }),
                }
            }
            manifest::SketchPatternLayout::Chain { from, to } => doc::SketchPatternLayout::Chain {
                from: Id(from),
                to: Id(to),
            },
            manifest::SketchPatternLayout::Circular {
                center,
                angle,
                chain,
            } => doc::SketchPatternLayout::Circular {
                center: match center {
                    manifest::CenterRef::Point { point } => {
                        doc::CenterRef::Point { point: Id(point) }
                    }
                    manifest::CenterRef::At { at } => doc::CenterRef::At { at: pair(at) },
                },
                angle: doc::Expr(angle),
                chain: chain.map(|[a, b]| [Id(a), Id(b)]),
            },
        },
    }
}

fn constraint_to_manifest(constraint: &doc::Constraint) -> manifest::SketchConstraint {
    use doc::ConstraintKind as D;
    use manifest::SketchConstraintKind as M;
    let doc::Constraint {
        id: doc::ConstraintId(id),
        kind,
    } = constraint;
    let raw = |doc::EntityId(id): &doc::EntityId| *id;
    let opt = |id: &Option<doc::EntityId>| id.as_ref().map(raw);
    let (a, b) = (raw, raw);
    manifest::SketchConstraint {
        id: *id,
        kind: match kind {
            D::Coincident { a: p, b: q } => M::Coincident { a: a(p), b: b(q) },
            D::PointOn { point, curve } => M::PointOn {
                point: raw(point),
                curve: raw(curve),
            },
            D::Horizontal { a: p, b: q } => M::Horizontal { a: a(p), b: opt(q) },
            D::Vertical { a: p, b: q } => M::Vertical { a: a(p), b: opt(q) },
            D::Parallel { a: p, b: q } => M::Parallel { a: a(p), b: b(q) },
            D::Perpendicular { a: p, b: q } => M::Perpendicular { a: a(p), b: b(q) },
            D::Tangent { a: p, b: q } => M::Tangent { a: a(p), b: b(q) },
            D::Equal { a: p, b: q } => M::Equal { a: a(p), b: b(q) },
            D::Concentric { a: p, b: q } => M::Concentric { a: a(p), b: b(q) },
            D::Symmetric { a: p, b: q, about } => M::Symmetric {
                a: a(p),
                b: b(q),
                about: raw(about),
            },
            D::Midpoint { point, line } => M::Midpoint {
                point: raw(point),
                line: raw(line),
            },
            D::Fixed { point } => M::Fixed { point: raw(point) },
            D::X { point, value } => M::X {
                point: raw(point),
                value: expr_to_manifest(value),
            },
            D::Y { point, value } => M::Y {
                point: raw(point),
                value: expr_to_manifest(value),
            },
            D::Distance {
                a: p,
                b: q,
                direction,
                value,
            } => M::Distance {
                a: a(p),
                b: opt(q),
                direction: match direction {
                    doc::DistanceDirection::Direct => manifest::DistanceDirection::Direct,
                    doc::DistanceDirection::Horizontal => manifest::DistanceDirection::Horizontal,
                    doc::DistanceDirection::Vertical => manifest::DistanceDirection::Vertical,
                },
                value: expr_to_manifest(value),
            },
            D::Angle { a: p, b: q, value } => M::Angle {
                a: a(p),
                b: b(q),
                value: expr_to_manifest(value),
            },
            D::Radius { curve, value } => M::Radius {
                curve: raw(curve),
                value: expr_to_manifest(value),
            },
            D::Diameter { curve, value } => M::Diameter {
                curve: raw(curve),
                value: expr_to_manifest(value),
            },
        },
    }
}

fn entity_to_doc(entity: manifest::SketchEntity) -> doc::Entity {
    use doc::EntityId as Id;
    let manifest::SketchEntity {
        id,
        construction,
        geometry,
    } = entity;
    doc::Entity {
        id: Id(id),
        construction,
        geometry: match geometry {
            manifest::SketchGeometry::Point { x, y } => doc::Geometry::Point { x, y },
            manifest::SketchGeometry::Line { start, end } => doc::Geometry::Line {
                start: Id(start),
                end: Id(end),
            },
            manifest::SketchGeometry::Circle { center, radius } => doc::Geometry::Circle {
                center: Id(center),
                radius,
            },
            manifest::SketchGeometry::Arc { center, start, end } => doc::Geometry::Arc {
                center: Id(center),
                start: Id(start),
                end: Id(end),
            },
            manifest::SketchGeometry::Spline { degree, control } => doc::Geometry::Spline {
                degree,
                control: control.into_iter().map(Id).collect(),
            },
            manifest::SketchGeometry::Formula {
                x,
                y,
                from,
                to,
                start,
                end,
            } => doc::Geometry::Formula {
                x: doc::Expr::new(x),
                y: doc::Expr::new(y),
                from: doc::Expr::new(from),
                to: doc::Expr::new(to),
                start: Id(start),
                end: Id(end),
            },
            manifest::SketchGeometry::Interpolated { degree, through } => {
                doc::Geometry::Interpolated {
                    degree,
                    through: through.into_iter().map(Id).collect(),
                }
            }
            manifest::SketchGeometry::Image { of, pattern, index } => doc::Geometry::Image {
                of: Id(of),
                pattern: doc::PatternId(pattern),
                index: match index {
                    manifest::InstanceIndex::At(k) => doc::InstanceIndex::At(k),
                    manifest::InstanceIndex::Grid(ij) => doc::InstanceIndex::Grid(ij),
                    manifest::InstanceIndex::Last => doc::InstanceIndex::Last,
                },
            },
        },
    }
}

fn constraint_to_doc(constraint: manifest::SketchConstraint) -> doc::Constraint {
    use doc::ConstraintKind as D;
    use doc::EntityId as Id;
    use manifest::SketchConstraintKind as M;
    let manifest::SketchConstraint { id, kind } = constraint;
    doc::Constraint {
        id: doc::ConstraintId(id),
        kind: match kind {
            M::Coincident { a, b } => D::Coincident { a: Id(a), b: Id(b) },
            M::PointOn { point, curve } => D::PointOn {
                point: Id(point),
                curve: Id(curve),
            },
            M::Horizontal { a, b } => D::Horizontal {
                a: Id(a),
                b: b.map(Id),
            },
            M::Vertical { a, b } => D::Vertical {
                a: Id(a),
                b: b.map(Id),
            },
            M::Parallel { a, b } => D::Parallel { a: Id(a), b: Id(b) },
            M::Perpendicular { a, b } => D::Perpendicular { a: Id(a), b: Id(b) },
            M::Tangent { a, b } => D::Tangent { a: Id(a), b: Id(b) },
            M::Equal { a, b } => D::Equal { a: Id(a), b: Id(b) },
            M::Concentric { a, b } => D::Concentric { a: Id(a), b: Id(b) },
            M::Symmetric { a, b, about } => D::Symmetric {
                a: Id(a),
                b: Id(b),
                about: Id(about),
            },
            M::Midpoint { point, line } => D::Midpoint {
                point: Id(point),
                line: Id(line),
            },
            M::Fixed { point } => D::Fixed { point: Id(point) },
            M::X { point, value } => D::X {
                point: Id(point),
                value: doc::Expr(value),
            },
            M::Y { point, value } => D::Y {
                point: Id(point),
                value: doc::Expr(value),
            },
            M::Distance {
                a,
                b,
                direction,
                value,
            } => D::Distance {
                a: Id(a),
                b: b.map(Id),
                direction: match direction {
                    manifest::DistanceDirection::Direct => doc::DistanceDirection::Direct,
                    manifest::DistanceDirection::Horizontal => doc::DistanceDirection::Horizontal,
                    manifest::DistanceDirection::Vertical => doc::DistanceDirection::Vertical,
                },
                value: doc::Expr(value),
            },
            M::Angle { a, b, value } => D::Angle {
                a: Id(a),
                b: Id(b),
                value: doc::Expr(value),
            },
            M::Radius { curve, value } => D::Radius {
                curve: Id(curve),
                value: doc::Expr(value),
            },
            M::Diameter { curve, value } => D::Diameter {
                curve: Id(curve),
                value: doc::Expr(value),
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

pub(crate) fn to_doc(document: manifest::Document, blobs: &Blobs) -> doc::Document {
    let manifest::Document {
        units: manifest::Units { length, angle },
        tolerance:
            manifest::Tolerance {
                linear,
                angular,
                loosest,
            },
        variables,
        features,
        rollback,
        next_feature_id,
        bodies,
        faces,
        assertions,
        sets,
        hidden,
        info,
        tables,
        outputs,
        inserted,
        assembly,
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
        // As written; one outside the range is brought into it, and said
        // so, by `from_manifest_json_noted`.
        tolerance: doc::Tolerance {
            linear,
            angular,
            loosest,
        },
        variables: variables
            .into_iter()
            .map(|variable| {
                let manifest::Variable { name, expr, public } = variable;
                doc::Variable {
                    name,
                    expr: doc::Expr(expr),
                    public: public.map(public_to_doc),
                }
            })
            .collect(),
        features: features
            .into_iter()
            .map(|f| feature_to_doc(f, blobs))
            .collect(),
        rollback: rollback.map(doc::FeatureId),
        next_feature_id,
        bodies: bodies.into_iter().map(body_to_doc).collect(),
        faces: faces.into_iter().map(face_to_doc).collect(),
        assertions: assertions.into_iter().map(assertion_to_doc).collect(),
        sets: sets
            .into_iter()
            .map(|manifest::NamedSet { name, query }| doc::NamedSet { name, query })
            .collect(),
        hidden: hidden.into_iter().map(doc::FeatureId).collect(),
        info: info.map(info_to_doc).unwrap_or_default(),
        tables: tables.into_iter().map(table_to_doc).collect(),
        outputs: outputs.into_iter().map(output_to_doc).collect(),
        inserted: inserted.into_iter().map(inserted_to_doc).collect(),
        assembly: assembly.map(|a| assembly_to_doc(a, blobs)),
    }
}

/// The bytes of the blobs of a file, by their SHA-256.
pub(crate) type Blobs = std::collections::BTreeMap<String, doc::Blob>;

fn feature_to_doc(feature: manifest::Feature, blobs: &Blobs) -> doc::Feature {
    let manifest::Feature {
        id,
        name,
        suppressed,
        when,
        kind,
    } = feature;
    doc::Feature {
        id: doc::FeatureId(id),
        name,
        suppressed,
        when: when.map(doc::Expr),
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
            manifest::FeatureKind::Sketch {
                plane,
                entities,
                constraints,
                next_entity_id,
                next_constraint_id,
                patterns,
                next_pattern_id,
                image_places,
            } => doc::FeatureKind::Sketch(doc::Sketch {
                plane: match plane {
                    manifest::SketchPlane::World { plane, offset } => doc::SketchPlane::World {
                        plane: match plane {
                            manifest::WorldPlane::Xy => doc::WorldPlane::Xy,
                            manifest::WorldPlane::Yz => doc::WorldPlane::Yz,
                            manifest::WorldPlane::Zx => doc::WorldPlane::Zx,
                        },
                        offset: doc::Expr(offset),
                    },
                    manifest::SketchPlane::Face {
                        face,
                        offset,
                        x_axis,
                    } => doc::SketchPlane::Face {
                        face: name_to_doc(face),
                        offset: doc::Expr(offset),
                        x_axis: x_axis.map(|axis| axis.map(doc::Expr)),
                    },
                    manifest::SketchPlane::Datum { plane, offset } => doc::SketchPlane::Datum {
                        plane: doc::FeatureId(plane),
                        offset: doc::Expr(offset),
                    },
                },
                entities: entities.into_iter().map(entity_to_doc).collect(),
                constraints: constraints.into_iter().map(constraint_to_doc).collect(),
                next_entity_id,
                next_constraint_id,
                patterns: patterns.into_iter().map(pattern_to_doc).collect(),
                next_pattern_id,
                image_places: image_places
                    .into_iter()
                    .map(|p| doc::ImagePlace {
                        entity: doc::EntityId(p.entity),
                        at: p.at,
                        map: p.map.map(|m| match m {
                            manifest::PlaceMap::Shift { offset } => doc::PlaceMap::Shift { offset },
                            manifest::PlaceMap::Chain { k } => doc::PlaceMap::Chain { k },
                            manifest::PlaceMap::Turn { angle, at } => {
                                doc::PlaceMap::Turn { angle, at }
                            }
                        }),
                    })
                    .collect(),
            }),
            manifest::FeatureKind::Extrude {
                sketch,
                distance,
                symmetric,
                end,
                reversed,
            } => doc::FeatureKind::Extrude(doc::Extrude {
                sketch: doc::FeatureId(sketch),
                distance: doc::Expr(distance),
                symmetric,
                end: match end {
                    manifest::ExtrudeEnd::Distance {} => doc::ExtrudeEnd::Distance,
                    manifest::ExtrudeEnd::ThroughAll {} => doc::ExtrudeEnd::ThroughAll,
                    manifest::ExtrudeEnd::UpToFace { face } => doc::ExtrudeEnd::UpToFace {
                        face: name_to_doc(face),
                    },
                },
                reversed,
            }),
            manifest::FeatureKind::Loft {
                sections,
                ruled,
                closed,
                guides,
            } => doc::FeatureKind::Loft(doc::Loft {
                guides: guides.into_iter().map(rail_to_doc).collect(),
                sections: sections
                    .into_iter()
                    .map(|manifest::LoftSection { sketch, start }| doc::LoftSection {
                        sketch: doc::FeatureId(sketch),
                        start: start.map(doc::EntityId),
                    })
                    .collect(),
                ruled,
                closed,
            }),
            manifest::FeatureKind::Fill { sketch } => doc::FeatureKind::Fill(doc::Fill {
                sketch: doc::FeatureId(sketch),
            }),
            manifest::FeatureKind::SheetMetal {
                sketch,
                thickness,
                radius,
                k_factor,
                flanges,
                relief,
                cutouts,
                corner,
                corners,
                next_flange_id,
                bend_relief,
            } => doc::FeatureKind::SheetMetal({
                let mut part = doc::SheetMetal {
                    next_flange_id: 0,
                    bend_relief: bend_relief.map(|manifest::SheetRelief { shape, size }| {
                        doc::CornerRelief {
                            shape: pick_to_doc(shape, relief_shape_to_doc),
                            size: doc::Expr(size),
                        }
                    }),
                    corner: corner.map(|manifest::SheetCornerClose { kind, gap }| {
                        doc::CornerClose {
                            kind: pick_to_doc(kind, corner_kind_to_doc),
                            gap: gap.map(doc::Expr),
                        }
                    }),
                    corners: corners
                        .into_iter()
                        .map(
                            |manifest::SheetCorner {
                                 flanges,
                                 kind,
                                 gap,
                                 over,
                             }| doc::SheetCorner {
                                flanges,
                                kind: pick_to_doc(kind, corner_kind_to_doc),
                                gap: gap.map(doc::Expr),
                                over,
                            },
                        )
                        .collect(),
                    cutouts: cutouts.into_iter().map(doc::FeatureId).collect(),
                    sketch: doc::FeatureId(sketch),
                    thickness: doc::Expr(thickness),
                    radius: doc::Expr(radius),
                    k_factor: doc::Expr(k_factor),
                    flanges: flanges
                        .into_iter()
                        .enumerate()
                        .map(|(i, f)| flange_to_doc(i, f))
                        .collect(),
                    relief: relief.map(|manifest::SheetRelief { shape, size }| doc::CornerRelief {
                        shape: pick_to_doc(shape, relief_shape_to_doc),
                        size: doc::Expr(size),
                    }),
                };
                // The next id, without one, is past the flanges' (one
                // written keeps its value, also a 0, which validation
                // refuses).
                part.next_flange_id = next_flange_id.unwrap_or_else(|| next_past(&part.flanges));
                part
            }),
            manifest::FeatureKind::Fold {
                sketch,
                creases,
                fixed,
                panels,
            } => doc::FeatureKind::Fold(doc::Fold {
                panels: panels.map(
                    |manifest::FoldPanels {
                         thickness,
                         clearance,
                         side,
                     }| doc::FoldPanels {
                        thickness: doc::Expr(thickness),
                        clearance: doc::Expr(clearance),
                        side: match side {
                            manifest::PanelSide::Back => doc::PanelSide::Back,
                            manifest::PanelSide::Front => doc::PanelSide::Front,
                        },
                    },
                ),
                sketch: doc::FeatureId(sketch),
                creases: creases
                    .into_iter()
                    .map(
                        |manifest::FoldCrease {
                             line,
                             kind,
                             angle,
                             instance,
                             alternate,
                         }| doc::Crease {
                            index: match instance {
                                None => doc::CurveIndex::Seed,
                                Some(manifest::FoldInstance::At(k)) => doc::CurveIndex::At(k),
                                Some(manifest::FoldInstance::Grid([i, j])) => {
                                    doc::CurveIndex::Grid(i, j)
                                }
                                Some(manifest::FoldInstance::All) => doc::CurveIndex::All,
                            },
                            alternate,
                            line: doc::EntityId(line),
                            kind: match kind {
                                manifest::CreaseKind::Mountain => doc::CreaseKind::Mountain,
                                manifest::CreaseKind::Valley => doc::CreaseKind::Valley,
                                manifest::CreaseKind::Cut => doc::CreaseKind::Cut,
                            },
                            angle: angle.map(doc::Expr),
                        },
                    )
                    .collect(),
                fixed: fixed.map(|[x, y]| [doc::Expr(x), doc::Expr(y)]),
            }),
            manifest::FeatureKind::Split {
                source,
                face,
                number,
            } => doc::FeatureKind::Split(doc::Split {
                source: doc::FeatureId(source),
                face: name_to_doc(face),
                number,
            }),
            manifest::FeatureKind::Developable { a, b } => {
                doc::FeatureKind::Developable(doc::Developable {
                    a: rail_to_doc(a),
                    b: rail_to_doc(b),
                })
            }
            manifest::FeatureKind::SurfaceNetwork { u, v, edges } => {
                let manifest::NetworkEdges {
                    u_first,
                    u_last,
                    v_first,
                    v_last,
                } = edges;
                doc::FeatureKind::SurfaceNetwork(doc::SurfaceNetwork {
                    u: u.into_iter().map(rail_to_doc).collect(),
                    v: v.into_iter().map(rail_to_doc).collect(),
                    edges: doc::NetworkEdges {
                        u_first: hold_to_doc(u_first),
                        u_last: hold_to_doc(u_last),
                        v_first: hold_to_doc(v_first),
                        v_last: hold_to_doc(v_last),
                    },
                })
            }
            manifest::FeatureKind::Unfold { faces, at, angle } => {
                doc::FeatureKind::Unfold(doc::Unfold {
                    faces: faces.into_iter().map(name_to_doc).collect(),
                    at: exprs_to_doc(at),
                    angle: doc::Expr(angle),
                })
            }
            manifest::FeatureKind::Import {
                file_name,
                solid,
                sha256,
            } => doc::FeatureKind::Import(doc::Import {
                file_name,
                solid,
                // Every blob the manifest names is there: checked before
                // conversion (`missing_blob`).
                data: blobs
                    .get(&sha256)
                    .cloned()
                    .unwrap_or_else(|| doc::Blob::new(Vec::new())),
            }),
            manifest::FeatureKind::Sweep { sketch, path } => doc::FeatureKind::Sweep(doc::Sweep {
                sketch: doc::FeatureId(sketch),
                path: doc::FeatureId(path),
            }),
            manifest::FeatureKind::OffsetProfile {
                sketch,
                distance,
                corners,
            } => doc::FeatureKind::OffsetProfile(doc::OffsetProfile {
                sketch: doc::FeatureId(sketch),
                distance: doc::Expr(distance),
                corners: match corners {
                    manifest::OffsetCorners::Round => doc::OffsetCorners::Round,
                    manifest::OffsetCorners::Sharp => doc::OffsetCorners::Sharp,
                },
            }),
            manifest::FeatureKind::Revolve {
                sketch,
                axis,
                angle,
                symmetric,
            } => doc::FeatureKind::Revolve(doc::Revolve {
                sketch: doc::FeatureId(sketch),
                axis: match axis {
                    manifest::RevolveAxis::X => doc::RevolveAxis::X,
                    manifest::RevolveAxis::Y => doc::RevolveAxis::Y,
                    manifest::RevolveAxis::Line { entity } => doc::RevolveAxis::Line {
                        entity: doc::EntityId(entity),
                    },
                    manifest::RevolveAxis::Datum { axis } => doc::RevolveAxis::Datum {
                        axis: doc::FeatureId(axis),
                    },
                },
                angle: doc::Expr(angle),
                symmetric,
            }),
            manifest::FeatureKind::Transform {
                source,
                translate,
                pivot,
                axis,
                angle,
                keep,
            } => doc::FeatureKind::Transform(doc::Transform {
                source: doc::FeatureId(source),
                translate: exprs_to_doc(translate),
                pivot: exprs_to_doc(pivot),
                axis: exprs_to_doc(axis),
                angle: doc::Expr(angle),
                keep,
            }),
            manifest::FeatureKind::Mirror {
                source,
                point,
                normal,
                plane,
                keep,
            } => doc::FeatureKind::Mirror(doc::Mirror {
                source: doc::FeatureId(source),
                point: exprs_to_doc(point),
                normal: exprs_to_doc(normal),
                plane: plane.map(doc::FeatureId),
                keep,
            }),
            manifest::FeatureKind::DatumPoint { at } => {
                doc::FeatureKind::DatumPoint(doc::DatumPoint {
                    at: point_ref_to_doc(at),
                })
            }
            manifest::FeatureKind::DatumHelix {
                axis,
                start,
                pitch,
                extent,
                left_handed,
            } => doc::FeatureKind::DatumHelix(doc::DatumHelix {
                axis: axis_ref_to_doc(axis),
                start: match start {
                    manifest::HelixStart::Radius { radius } => doc::HelixStart::Radius {
                        radius: doc::Expr(radius),
                    },
                    manifest::HelixStart::Point { point } => doc::HelixStart::Point {
                        point: point_ref_to_doc(point),
                    },
                },
                pitch: doc::Expr(pitch),
                extent: match extent {
                    manifest::HelixExtent::Turns { turns } => doc::HelixExtent::Turns {
                        turns: doc::Expr(turns),
                    },
                    manifest::HelixExtent::Height { height } => doc::HelixExtent::Height {
                        height: doc::Expr(height),
                    },
                },
                left_handed,
            }),
            manifest::FeatureKind::OffsetFace { faces, distance } => {
                doc::FeatureKind::OffsetFace(doc::OffsetFace {
                    faces: faces.into_iter().map(name_to_doc).collect(),
                    distance: doc::Expr(distance),
                })
            }
            manifest::FeatureKind::MoveFace {
                faces,
                translate,
                pivot,
                axis,
                angle,
            } => doc::FeatureKind::MoveFace(doc::MoveFace {
                faces: faces.into_iter().map(name_to_doc).collect(),
                translate: exprs_to_doc(translate),
                pivot: exprs_to_doc(pivot),
                axis: exprs_to_doc(axis),
                angle: doc::Expr(angle),
            }),
            manifest::FeatureKind::Draft {
                faces,
                neutral,
                angle,
                reversed,
            } => doc::FeatureKind::Draft(doc::Draft {
                faces: faces.into_iter().map(name_to_doc).collect(),
                neutral: plane_ref_to_doc(neutral),
                angle: doc::Expr(angle),
                reversed,
            }),
            manifest::FeatureKind::Plate {
                faces,
                thickness,
                side,
                joints,
            } => doc::FeatureKind::Plate(doc::Plate {
                faces: faces.into_iter().map(name_to_doc).collect(),
                thickness: doc::Expr(thickness),
                side: plate_side_to_doc(side),
                joints: joints
                    .into_iter()
                    .map(|j| doc::PlateJoint {
                        edge: name_to_doc(j.edge),
                        kind: match j.kind {
                            manifest::JointKind::Square => doc::JointKind::Square,
                            manifest::JointKind::Bevel => doc::JointKind::Bevel,
                            manifest::JointKind::Mitre => doc::JointKind::Mitre,
                            manifest::JointKind::Touch => doc::JointKind::Touch,
                            manifest::JointKind::Over => doc::JointKind::Over,
                            manifest::JointKind::Under => doc::JointKind::Under,
                            manifest::JointKind::Onto => doc::JointKind::Onto,
                        },
                        with: j.with.map(name_to_doc),
                        thickness: j.thickness.map(doc::Expr),
                        side: j.side.map(plate_side_to_doc),
                        angle: j.angle.map(doc::Expr),
                        onto: j.onto.map(plane_ref_to_doc),
                    })
                    .collect(),
            }),
            manifest::FeatureKind::Thicken {
                faces,
                thickness,
                side,
            } => doc::FeatureKind::Thicken(doc::Thicken {
                faces: faces.into_iter().map(name_to_doc).collect(),
                thickness: doc::Expr(thickness),
                side: match side {
                    manifest::ThickenSide::Outside => doc::ThickenSide::Outside,
                    manifest::ThickenSide::Inside => doc::ThickenSide::Inside,
                    manifest::ThickenSide::Both => doc::ThickenSide::Both,
                },
            }),
            manifest::FeatureKind::DeleteFace { faces } => {
                doc::FeatureKind::DeleteFace(doc::DeleteFace {
                    faces: faces.into_iter().map(name_to_doc).collect(),
                })
            }
            manifest::FeatureKind::Thread {
                face,
                start,
                size,
                pitch,
                length,
                left_handed,
                modeled,
            } => doc::FeatureKind::Thread(doc::Thread {
                face: name_to_doc(face),
                start: name_to_doc(start),
                size: size.map(|size| match size {
                    manifest::ThreadSize::Designation(text) => doc::ThreadSize::Designation(text),
                    manifest::ThreadSize::Diameter(manifest::ThreadDiameter { diameter }) => {
                        doc::ThreadSize::Diameter {
                            diameter: doc::Expr(diameter),
                        }
                    }
                }),
                pitch: pitch.map(doc::Expr),
                length: doc::Expr(length),
                left_handed,
                modeled,
            }),
            manifest::FeatureKind::Shell {
                source,
                thickness,
                open,
            } => doc::FeatureKind::Shell(doc::Shell {
                source: doc::FeatureId(source),
                thickness: doc::Expr(thickness),
                open: open.into_iter().map(name_to_doc).collect(),
            }),
            manifest::FeatureKind::Fillet {
                edges,
                radius,
                chain,
            } => doc::FeatureKind::Fillet(doc::Fillet {
                edges: edges
                    .into_iter()
                    .map(|pair| pair.map(name_to_doc))
                    .collect(),
                radius: doc::Expr(radius),
                chain,
            }),
            manifest::FeatureKind::Chamfer { edges, size, chain } => {
                doc::FeatureKind::Chamfer(doc::Chamfer {
                    edges: edges
                        .into_iter()
                        .map(|pair| pair.map(name_to_doc))
                        .collect(),
                    size: match size {
                        manifest::ChamferSize::Equal { distance } => doc::ChamferSize::Equal {
                            distance: doc::Expr(distance),
                        },
                        manifest::ChamferSize::TwoDistances { distance, second } => {
                            doc::ChamferSize::TwoDistances {
                                distance: doc::Expr(distance),
                                second: doc::Expr(second),
                            }
                        }
                        manifest::ChamferSize::DistanceAngle { distance, angle } => {
                            doc::ChamferSize::DistanceAngle {
                                distance: doc::Expr(distance),
                                angle: doc::Expr(angle),
                            }
                        }
                    },
                    chain,
                })
            }
            manifest::FeatureKind::DatumAxis { by } => {
                doc::FeatureKind::DatumAxis(doc::DatumAxis {
                    by: match by {
                        manifest::AxisBy::TwoPoints { points } => doc::AxisBy::TwoPoints {
                            points: points.map(point_ref_to_doc),
                        },
                        manifest::AxisBy::Edge { between } => doc::AxisBy::Edge {
                            between: between.map(name_to_doc),
                        },
                        manifest::AxisBy::Face { face } => doc::AxisBy::Face {
                            face: name_to_doc(face),
                        },
                        manifest::AxisBy::TwoPlanes { planes } => doc::AxisBy::TwoPlanes {
                            planes: planes.map(plane_ref_to_doc),
                        },
                    },
                })
            }
            manifest::FeatureKind::DatumPlane { by } => {
                doc::FeatureKind::DatumPlane(doc::DatumPlane {
                    by: match by {
                        manifest::PlaneBy::Offset { from, distance } => doc::PlaneBy::Offset {
                            from: plane_ref_to_doc(from),
                            distance: doc::Expr(distance),
                        },
                        manifest::PlaneBy::Angle { from, about, angle } => doc::PlaneBy::Angle {
                            from: plane_ref_to_doc(from),
                            about: axis_ref_to_doc(about),
                            angle: doc::Expr(angle),
                        },
                        manifest::PlaneBy::ThreePoints { points } => doc::PlaneBy::ThreePoints {
                            points: points.map(point_ref_to_doc),
                        },
                    },
                })
            }
            manifest::FeatureKind::Boolean {
                op,
                target,
                tool,
                keep_tool,
            } => doc::FeatureKind::Boolean(doc::Boolean {
                op: match op {
                    manifest::BooleanOp::Union => doc::BooleanOp::Union,
                    manifest::BooleanOp::Subtract => doc::BooleanOp::Subtract,
                    manifest::BooleanOp::Intersect => doc::BooleanOp::Intersect,
                },
                target: doc::FeatureId(target),
                tool: doc::FeatureId(tool),
                keep_tool,
            }),
            manifest::FeatureKind::Pattern {
                source,
                count,
                layout,
            } => doc::FeatureKind::Pattern(doc::Pattern {
                source: doc::FeatureId(source),
                count: doc::Expr(count),
                layout: match layout {
                    manifest::PatternLayout::Linear { step } => doc::PatternLayout::Linear {
                        step: exprs_to_doc(step),
                    },
                    manifest::PatternLayout::Circular {
                        point,
                        axis,
                        angle,
                        about,
                        step,
                    } => doc::PatternLayout::Circular {
                        point: exprs_to_doc(point),
                        axis: exprs_to_doc(axis),
                        angle: doc::Expr(angle),
                        about: about.map(doc::FeatureId),
                        step: step.map(doc::Expr),
                    },
                },
            }),
        },
    }
}
