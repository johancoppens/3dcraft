//! STEP export of an assembly with its structure (ISO 10303-242 / AP214
//! assembly structure, as `craft3d-stepread` reads it): each distinct part
//! once, as a product with its B-rep (as [`step_model`] writes a body),
//! and one product for the assembly that places them, each placement a
//! `NEXT_ASSEMBLY_USAGE_OCCURRENCE` with a `CONTEXT_DEPENDENT_SHAPE_
//! REPRESENTATION` whose `ITEM_DEFINED_TRANSFORMATION` maps the part's
//! origin frame onto the frame it is placed at.

use crate::IoError;
use crate::step::{StepBody, StepOptions, StepSummary, step_model};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_step::{Entity, Instance, Param, Record, StepFile};

/// One placement of a part in the assembly.
#[derive(Clone, Debug)]
pub struct StepOccurrence {
    /// The part, by its index among the parts.
    pub part: usize,
    /// From the part's coordinates to the assembly's: a rigid motion.
    pub pose: Transform,
    /// The occurrence's name (the instance's).
    pub name: String,
}

struct Adder {
    file: StepFile,
    next: u64,
}

fn unsupported(what: String) -> IoError {
    IoError::Unsupported {
        body: String::new(),
        what,
    }
}

impl Adder {
    fn add(&mut self, keyword: &str, params: Vec<Param>) -> u64 {
        self.push(Entity::Simple(Record::new(keyword, params)))
    }

    fn push(&mut self, entity: Entity) -> u64 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        self.file.data.push(Instance { id, entity });
        id
    }

    /// The instances with `keyword`, in the file's order.
    fn all(&self, keyword: &str) -> Vec<u64> {
        self.file
            .data
            .iter()
            .filter(|i| i.entity.is(keyword))
            .map(|i| i.id)
            .collect()
    }

    fn first(&self, keyword: &str) -> Result<u64, IoError> {
        self.all(keyword)
            .first()
            .copied()
            .ok_or_else(|| unsupported(format!("the part file has no {keyword}")))
    }

    fn placement(&mut self, f: &Frame) -> u64 {
        let p = f.origin();
        let origin = self.add(
            "CARTESIAN_POINT",
            vec![Param::string(""), Param::reals([p.x, p.y, p.z])],
        );
        let z = f.z();
        let z = self.add(
            "DIRECTION",
            vec![Param::string(""), Param::reals([z.x, z.y, z.z])],
        );
        let x = f.x();
        let x = self.add(
            "DIRECTION",
            vec![Param::string(""), Param::reals([x.x, x.y, x.z])],
        );
        self.add(
            "AXIS2_PLACEMENT_3D",
            vec![
                Param::string(""),
                Param::Ref(origin),
                Param::Ref(z),
                Param::Ref(x),
            ],
        )
    }
}

/// The frame a rigid pose carries the world frame to.
fn frame_of(pose: &Transform) -> Option<Frame> {
    Frame::new(
        pose.point(Vec3::ZERO),
        pose.vector(Vec3::Z),
        pose.vector(Vec3::X),
    )
}

/// The parts and their placements as one STEP file with an assembly
/// structure: the product `name` places every occurrence. Every part is
/// checked as [`step_model`] checks a body; a pose that is not a rigid
/// motion, or an occurrence of a part that is not there, is an error.
pub fn step_assembly(
    name: &str,
    parts: &[StepBody<'_>],
    occurrences: &[StepOccurrence],
    options: &StepOptions,
) -> Result<(Vec<u8>, StepSummary), IoError> {
    if occurrences.is_empty() {
        return Err(IoError::NoBodies);
    }
    let (file, summary) = step_model(parts, options)?;
    let next = file
        .data
        .iter()
        .map(|i| i.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut a = Adder { file, next };
    let definitions = a.all("PRODUCT_DEFINITION");
    let reps: Vec<u64> = a
        .file
        .data
        .iter()
        .filter(|i| {
            i.entity.is("ADVANCED_BREP_SHAPE_REPRESENTATION")
                || i.entity.is("MANIFOLD_SURFACE_SHAPE_REPRESENTATION")
        })
        .map(|i| i.id)
        .collect();
    if definitions.len() != parts.len() || reps.len() != parts.len() {
        return Err(unsupported(
            "the parts did not come out as one product each".to_string(),
        ));
    }
    let context = a.first("GLOBAL_UNIT_ASSIGNED_CONTEXT")?;
    let product_context = a.first("PRODUCT_CONTEXT")?;
    let definition_context = a.first("PRODUCT_DEFINITION_CONTEXT")?;
    // Each part as a child: a shape representation with an origin frame,
    // related to its B-rep's.
    let mut children = Vec::with_capacity(parts.len());
    for (definition, brep) in definitions.iter().zip(&reps) {
        let origin = a.placement(&Frame::WORLD);
        let rep = a.add(
            "SHAPE_REPRESENTATION",
            vec![
                Param::string(""),
                Param::refs([origin]),
                Param::Ref(context),
            ],
        );
        a.add(
            "SHAPE_REPRESENTATION_RELATIONSHIP",
            vec![
                Param::string(""),
                Param::string(""),
                Param::Ref(rep),
                Param::Ref(*brep),
            ],
        );
        children.push((*definition, rep, origin));
    }
    // The assembly's product.
    let product = a.add(
        "PRODUCT",
        vec![
            Param::string(name),
            Param::string(name),
            Param::string(""),
            Param::refs([product_context]),
        ],
    );
    let formation = a.add(
        "PRODUCT_DEFINITION_FORMATION",
        vec![Param::string(""), Param::string(""), Param::Ref(product)],
    );
    let root = a.add(
        "PRODUCT_DEFINITION",
        vec![
            Param::string("design"),
            Param::string(""),
            Param::Ref(formation),
            Param::Ref(definition_context),
        ],
    );
    let shape = a.add(
        "PRODUCT_DEFINITION_SHAPE",
        vec![Param::string(""), Param::string(""), Param::Ref(root)],
    );
    let root_origin = a.placement(&Frame::WORLD);
    let mut items = vec![root_origin];
    let mut placements = Vec::with_capacity(occurrences.len());
    for occurrence in occurrences {
        if !occurrence.pose.is_isometry(1e-9) {
            return Err(unsupported(format!(
                "the placement of `{}` is not a rigid motion",
                occurrence.name
            )));
        }
        let frame = frame_of(&occurrence.pose).ok_or_else(|| {
            unsupported(format!(
                "the placement of `{}` has no frame",
                occurrence.name
            ))
        })?;
        let at = a.placement(&frame);
        items.push(at);
        placements.push(at);
    }
    let root_rep = a.add(
        "SHAPE_REPRESENTATION",
        vec![Param::string(name), Param::refs(items), Param::Ref(context)],
    );
    a.add(
        "SHAPE_DEFINITION_REPRESENTATION",
        vec![Param::Ref(shape), Param::Ref(root_rep)],
    );
    for (occurrence, at) in occurrences.iter().zip(placements) {
        let &(definition, rep, origin) = children.get(occurrence.part).ok_or_else(|| {
            unsupported(format!(
                "`{}` places a part that is not there",
                occurrence.name
            ))
        })?;
        let transformation = a.add(
            "ITEM_DEFINED_TRANSFORMATION",
            vec![
                Param::string(""),
                Param::string(""),
                Param::Ref(origin),
                Param::Ref(at),
            ],
        );
        let relationship = a.push(Entity::Complex(vec![
            Record::new(
                "REPRESENTATION_RELATIONSHIP",
                vec![
                    Param::string(""),
                    Param::string(""),
                    Param::Ref(rep),
                    Param::Ref(root_rep),
                ],
            ),
            Record::new(
                "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION",
                vec![Param::Ref(transformation)],
            ),
            Record::new("SHAPE_REPRESENTATION_RELATIONSHIP", vec![]),
        ]));
        let usage = a.add(
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE",
            vec![
                Param::string(&occurrence.name),
                Param::string(&occurrence.name),
                Param::string(""),
                Param::Ref(root),
                Param::Ref(definition),
                Param::Omitted,
            ],
        );
        let occurrence_shape = a.add(
            "PRODUCT_DEFINITION_SHAPE",
            vec![Param::string(""), Param::string(""), Param::Ref(usage)],
        );
        a.add(
            "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION",
            vec![Param::Ref(relationship), Param::Ref(occurrence_shape)],
        );
    }
    let mut summary = summary;
    summary.instances = a.file.data.len();
    let text = craft3d_step::write(&a.file).map_err(|e| unsupported(e.to_string()))?;
    Ok((text.into_bytes(), summary))
}
