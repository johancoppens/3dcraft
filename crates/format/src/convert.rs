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
        bodies: bodies.iter().map(body_to_manifest).collect(),
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
        appearance: appearance.map(|appearance| {
            let doc::Appearance {
                color,
                metallic,
                roughness,
            } = appearance;
            manifest::Appearance {
                color,
                metallic,
                roughness,
            }
        }),
        material: material.as_ref().map(|material| {
            let doc::Material { name, density } = material;
            manifest::Material {
                name: name.clone(),
                density: *density,
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
            let manifest::Appearance {
                color,
                metallic,
                roughness,
            } = appearance;
            doc::Appearance {
                color,
                metallic,
                roughness,
            }
        }),
        material: material.map(|material| {
            let manifest::Material { name, density } = material;
            doc::Material { name, density }
        }),
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
                    doc::SketchPlane::Face { face, offset } => manifest::SketchPlane::Face {
                        face: name_to_manifest(face),
                        offset: expr_to_manifest(offset),
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
            }) => manifest::FeatureKind::Loft {
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
                    } => manifest::PatternLayout::Circular {
                        point: exprs_to_manifest(point),
                        axis: exprs_to_manifest(axis),
                        angle: expr_to_manifest(angle),
                        about: about.map(|doc::FeatureId(id)| id),
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

fn name_to_doc(name: manifest::Name) -> doc::PersistentName {
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
        bodies,
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
        bodies: bodies.into_iter().map(body_to_doc).collect(),
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
                    manifest::SketchPlane::Face { face, offset } => doc::SketchPlane::Face {
                        face: name_to_doc(face),
                        offset: doc::Expr(offset),
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
            } => doc::FeatureKind::Loft(doc::Loft {
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
                    } => doc::PatternLayout::Circular {
                        point: exprs_to_doc(point),
                        axis: exprs_to_doc(axis),
                        angle: doc::Expr(angle),
                        about: about.map(doc::FeatureId),
                    },
                },
            }),
        },
    }
}
