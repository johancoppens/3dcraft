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
            doc::FeatureKind::Sketch(doc::Sketch {
                plane,
                entities,
                constraints,
                next_entity_id,
                next_constraint_id,
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
                    doc::SketchPlane::Face { face, offset } => manifest::SketchPlane::Face {
                        face: name_to_manifest(face),
                        offset: expr_to_manifest(offset),
                    },
                },
                entities: entities.iter().map(entity_to_manifest).collect(),
                constraints: constraints.iter().map(constraint_to_manifest).collect(),
                next_entity_id: *next_entity_id,
                next_constraint_id: *next_constraint_id,
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
                keep,
            }) => manifest::FeatureKind::Mirror {
                source: *source,
                point: exprs_to_manifest(point),
                normal: exprs_to_manifest(normal),
                keep: *keep,
            },
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
                    doc::PatternLayout::Circular { point, axis, angle } => {
                        manifest::PatternLayout::Circular {
                            point: exprs_to_manifest(point),
                            axis: exprs_to_manifest(axis),
                            angle: expr_to_manifest(angle),
                        }
                    }
                },
            },
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
        D::Face => M::Face,
        D::Edge => M::Edge,
        D::Vertex => M::Vertex,
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
        M::Face => D::Face,
        M::Edge => D::Edge,
        M::Vertex => D::Vertex,
    }
}

// A name is a chain (a copy of a copy of a face), walked in a loop so a
// hostile file cannot overflow the stack; `Document::validate` refuses a
// chain longer than `doc::MAX_NAME_DEPTH`.
fn name_to_manifest(name: &doc::PersistentName) -> manifest::Name {
    let mut links = Vec::new();
    let mut next = Some(name);
    while let Some(link) = next {
        let doc::PersistentName {
            feature: doc::FeatureId(feature),
            role,
            sources,
            of,
        } = link;
        links.push(manifest::Name {
            feature: *feature,
            role: role_to_manifest(*role),
            sources: sources.clone(),
            of: None,
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
    })
}

fn name_to_doc(name: manifest::Name) -> doc::PersistentName {
    let mut links = Vec::new();
    let mut next = Some(name);
    while let Some(link) = next {
        let manifest::Name {
            feature,
            role,
            sources,
            of,
        } = link;
        links.push(doc::PersistentName::new(
            doc::FeatureId(feature),
            role_to_doc(role),
            sources,
        ));
        next = of.map(|boxed| *boxed);
    }
    let mut built: Option<doc::PersistentName> = None;
    while let Some(mut link) = links.pop() {
        link.of = built.take().map(Box::new);
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
            manifest::FeatureKind::Sketch {
                plane,
                entities,
                constraints,
                next_entity_id,
                next_constraint_id,
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
                    manifest::SketchPlane::Face { face, offset } => doc::SketchPlane::Face {
                        face: name_to_doc(face),
                        offset: doc::Expr(offset),
                    },
                },
                entities: entities.into_iter().map(entity_to_doc).collect(),
                constraints: constraints.into_iter().map(constraint_to_doc).collect(),
                next_entity_id,
                next_constraint_id,
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
                keep,
            } => doc::FeatureKind::Mirror(doc::Mirror {
                source: doc::FeatureId(source),
                point: exprs_to_doc(point),
                normal: exprs_to_doc(normal),
                keep,
            }),
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
                    manifest::PatternLayout::Circular { point, axis, angle } => {
                        doc::PatternLayout::Circular {
                            point: exprs_to_doc(point),
                            axis: exprs_to_doc(axis),
                            angle: doc::Expr(angle),
                        }
                    }
                },
            }),
        },
    }
}
