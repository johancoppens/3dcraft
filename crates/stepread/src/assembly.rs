//! Assemblies: where the file places each solid, as often as it places it
//! (ISO 10303-42 `mapped_item`, -43 `representation_relationship_with_
//! transformation` and `item_defined_transformation`, -44
//! `next_assembly_usage_occurrence` and `context_dependent_shape_
//! representation`, the product structure AP203, AP214 and AP242 share).
//!
//! **The graph.** Every shape representation is a node. A relationship
//! without a transformation (`SHAPE_REPRESENTATION_RELATIONSHIP` between
//! a part's shape and its B-rep) says two representations are in one
//! place: they are one node (a union, in file order). A relationship with
//! a transformation, or a `MAPPED_ITEM` in a representation, places a
//! child node in a parent: an edge, with the frame of the child that goes
//! onto the frame of the parent. A solid is in the node of the first
//! representation that lists it.
//!
//! **Which is the child.** Where a `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION`
//! ties the relationship to a `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, the
//! occurrence says (its related product definition is the child, whose
//! shape the `SHAPE_DEFINITION_REPRESENTATION` names); without one,
//! `rep_1` is the child, as ISO 10303-43 relates `rep_1` to `rep_2`. Of
//! the two frames of an `ITEM_DEFINED_TRANSFORMATION` the child's is the
//! one its node lists (writers differ in which comes first); when the
//! file lists neither or both, `transform_item_1` is in `rep_1`.
//!
//! **Placements.** A solid is placed once for every way up from its node
//! to a node nothing places (a root), the transformations composed from
//! the solid up; a solid in a root, or in a file without assemblies, is
//! placed once where it is. A transformation is `F_parent ∘ F_child⁻¹`,
//! each frame read in the units of its own representation, so a part in
//! inches placed in an assembly in millimetres lands where it should. A
//! frame is an `AXIS2_PLACEMENT_3D` or a `CARTESIAN_TRANSFORMATION_
//! OPERATOR_3D` without scale or mirror (a placement that scales or
//! mirrors a solid is refused, naming it).
//!
//! **Names.** Each placement has a path: the root's product name, then
//! per step down the child product's name and the occurrence's id
//! (`bracket/bolt#2`), or the mapped item's name, or the instance id of a
//! relationship that says nothing else. The order of placements comes
//! from the file alone (items in instance order, each item's ways up in
//! the order of the relationships' instances), so the same bytes give the
//! same placements, paths and numbers.
//!
//! **Bounded.** The ways up are followed with a set of the nodes on the
//! way: a file whose assembly contains itself is refused naming the
//! relationship that closes the circle; deeper than [`MAX_DEPTH`] levels
//! or more than [`MAX_PLACEMENTS`] placed solids in all is refused before
//! anything is built.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::{ImportError, Parsed, err, real};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_step::Param;
use std::collections::{BTreeMap, BTreeSet};

/// Deepest assembly read: levels of placement from a solid to its root.
pub const MAX_DEPTH: usize = 32;
/// Most placed solids a file may hold, in all its assemblies.
pub const MAX_PLACEMENTS: usize = 10_000;

/// A frame of a placement, read when the body is built (its lengths need
/// the caller's unit): an `AXIS2_PLACEMENT_3D` or a
/// `CARTESIAN_TRANSFORMATION_OPERATOR_3D`, in the units of `context`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameRef {
    pub id: u64,
    pub context: Option<u64>,
}

/// One step up: the child's frame (none: the child's origin) goes onto
/// the parent's frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub child: Option<FrameRef>,
    pub parent: FrameRef,
    /// The relationship or mapped item, for messages.
    pub by: u64,
}

/// One placed solid: which item, its path for people, and the steps up
/// from the solid to its root.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The item's index among [`Parsed::items`].
    pub item: usize,
    /// The root's product, then each step down (`bracket/bolt#2`); empty
    /// for a solid that no assembly places.
    pub path: String,
    pub(crate) steps: Vec<Step>,
}

/// An edge of the graph: `child` node placed in `parent` node.
struct Edge {
    child: u64,
    parent: u64,
    step: Step,
    label: String,
}

/// Union-find over representation ids, in file order (the smaller id
/// stays the root, so the result does not depend on the order of unions).
#[derive(Default)]
struct Groups {
    parent: BTreeMap<u64, u64>,
}

impl Groups {
    fn find(&self, mut id: u64) -> u64 {
        // Bounded: every step goes to a smaller id.
        for _ in 0..self.parent.len().saturating_add(1) {
            match self.parent.get(&id) {
                Some(&up) if up != id => id = up,
                _ => return id,
            }
        }
        id
    }

    fn union(&mut self, a: u64, b: u64) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (low, high) = (ra.min(rb), ra.max(rb));
            self.parent.insert(high, low);
        }
    }
}

fn text(param: Option<&Param>) -> Option<&str> {
    param
        .and_then(Param::as_str)
        .filter(|s| !s.trim().is_empty())
}

impl Parsed {
    /// `(rep_1, rep_2)` of a representation relationship, simple or in a
    /// complex instance.
    fn related(&self, id: u64) -> Option<(u64, u64)> {
        let entity = &self.instance(id).ok()?.entity;
        let record = entity
            .record("REPRESENTATION_RELATIONSHIP")
            .or_else(|| entity.record("SHAPE_REPRESENTATION_RELATIONSHIP"))?;
        Some((
            record.params.get(2)?.as_ref()?,
            record.params.get(3)?.as_ref()?,
        ))
    }

    /// The name of the product a product definition is of.
    fn product_name(&self, definition: u64) -> Option<String> {
        let pd = self.record(definition, "PRODUCT_DEFINITION").ok()?;
        let formation = pd.params.get(2)?.as_ref()?;
        let formation = self.instance(formation).ok()?;
        let of = formation.entity.records().iter().find_map(|r| {
            r.keyword
                .starts_with("PRODUCT_DEFINITION_FORMATION")
                .then(|| r.params.get(2).and_then(Param::as_ref))
                .flatten()
        })?;
        let product = self.record(of, "PRODUCT").ok()?;
        text(product.params.get(1))
            .or_else(|| text(product.params.first()))
            .map(str::to_string)
    }

    /// Where the file places each of its items (module documentation),
    /// in the order of the items' first placements, then the further
    /// placements: see [`Parsed::placed`].
    pub(crate) fn find_placements(&self) -> Result<Vec<Placed>, ImportError> {
        // Representations: their items and contexts.
        let mut reps: BTreeMap<u64, (Vec<u64>, Option<u64>)> = BTreeMap::new();
        for instance in &self.file.data {
            for record in instance.entity.records() {
                if !record.keyword.ends_with("REPRESENTATION")
                    || record.keyword.ends_with("PRESENTATION_REPRESENTATION")
                {
                    continue;
                }
                let Some(list) = record.params.get(1).and_then(Param::as_list) else {
                    continue;
                };
                let items: Vec<u64> = list.iter().filter_map(Param::as_ref).collect();
                let context = record.params.get(2).and_then(Param::as_ref);
                reps.entry(instance.id).or_insert((items, context));
            }
        }
        // One node per set of representations in one place.
        let mut groups = Groups::default();
        let mut transformed: Vec<u64> = Vec::new();
        for instance in &self.file.data {
            let entity = &instance.entity;
            if entity.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
                transformed.push(instance.id);
            } else if (entity.is("REPRESENTATION_RELATIONSHIP")
                || entity.is("SHAPE_REPRESENTATION_RELATIONSHIP"))
                && let Some((a, b)) = self.related(instance.id)
                && reps.contains_key(&a)
                && reps.contains_key(&b)
            {
                groups.union(a, b);
            }
        }
        // The items each node lists, and a context with units per node.
        let mut listed: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        let mut context_of: BTreeMap<u64, u64> = BTreeMap::new();
        // The context of the representation that lists each item: a frame
        // is read in its own representation's units, which may differ
        // from another representation's in the same place.
        let mut context_of_item: BTreeMap<u64, u64> = BTreeMap::new();
        for (&rep, (items, context)) in &reps {
            let node = groups.find(rep);
            listed
                .entry(node)
                .or_default()
                .extend(items.iter().copied());
            if let Some(context) = *context
                && self
                    .instance(context)
                    .is_ok_and(|i| i.entity.is("GLOBAL_UNIT_ASSIGNED_CONTEXT"))
            {
                context_of.entry(node).or_insert(context);
                for &item in items {
                    context_of_item.entry(item).or_insert(context);
                }
            }
        }
        // Product definitions and their nodes; occurrences by relationship.
        let mut node_of_definition: BTreeMap<u64, u64> = BTreeMap::new();
        let mut occurrence_of: BTreeMap<u64, u64> = BTreeMap::new();
        let definition_of_shape = |pds: u64| -> Option<u64> {
            self.record(pds, "PRODUCT_DEFINITION_SHAPE")
                .ok()?
                .params
                .get(2)?
                .as_ref()
        };
        for instance in &self.file.data {
            if let Some(sdr) = instance.entity.record("SHAPE_DEFINITION_REPRESENTATION")
                && let (Some(pds), Some(rep)) = (
                    sdr.params.first().and_then(Param::as_ref),
                    sdr.params.get(1).and_then(Param::as_ref),
                )
                && reps.contains_key(&rep)
                && let Some(definition) = definition_of_shape(pds)
            {
                node_of_definition
                    .entry(definition)
                    .or_insert_with(|| groups.find(rep));
            }
            if let Some(cdsr) = instance
                .entity
                .record("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION")
                && let (Some(relation), Some(pds)) = (
                    cdsr.params.first().and_then(Param::as_ref),
                    cdsr.params.get(1).and_then(Param::as_ref),
                )
                && let Some(occurrence) = definition_of_shape(pds)
                && self
                    .instance(occurrence)
                    .is_ok_and(|i| i.entity.is("NEXT_ASSEMBLY_USAGE_OCCURRENCE"))
            {
                occurrence_of.entry(relation).or_insert(occurrence);
            }
        }
        let frame_ref = |id: u64, node: u64| FrameRef {
            id,
            context: context_of_item
                .get(&id)
                .or_else(|| context_of.get(&node))
                .copied(),
        };
        let lists = |node: u64, item: u64| listed.get(&node).is_some_and(|s| s.contains(&item));
        let mut edges: Vec<Edge> = Vec::new();
        // Relationships with a transformation.
        for &id in &transformed {
            let Some((rep_1, rep_2)) = self.related(id) else {
                return err(format!(
                    "{}: a placement without the two representations it relates",
                    self.describe(id)
                ));
            };
            if !reps.contains_key(&rep_1) || !reps.contains_key(&rep_2) {
                return err(format!(
                    "{}: relates something that is not a representation",
                    self.describe(id)
                ));
            }
            let (n1, n2) = (groups.find(rep_1), groups.find(rep_2));
            let occurrence = occurrence_of.get(&id).copied();
            let nauo =
                occurrence.and_then(|o| self.record(o, "NEXT_ASSEMBLY_USAGE_OCCURRENCE").ok());
            let child_definition = nauo.and_then(|r| r.params.get(4).and_then(Param::as_ref));
            // rep_1 is the child unless the occurrence says rep_2 is.
            let swap = child_definition
                .and_then(|d| node_of_definition.get(&d))
                .is_some_and(|&n| n == n2 && n != n1);
            let (child, parent) = if swap { (n2, n1) } else { (n1, n2) };
            let operator = self
                .record(id, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")?
                .params
                .first()
                .and_then(Param::as_ref)
                .ok_or_else(|| ImportError(format!("{}: no transformation", self.describe(id))))?;
            let step = if let Ok(idt) = self.record(operator, "ITEM_DEFINED_TRANSFORMATION") {
                let (Some(i1), Some(i2)) = (
                    idt.params.get(2).and_then(Param::as_ref),
                    idt.params.get(3).and_then(Param::as_ref),
                ) else {
                    return err(format!(
                        "{}: a transformation without its two frames",
                        self.describe(operator)
                    ));
                };
                // Which frame is the child's: the one its node lists.
                let first_is_child = lists(child, i1) as u8 + lists(parent, i2) as u8;
                let second_is_child = lists(child, i2) as u8 + lists(parent, i1) as u8;
                let child_first = match first_is_child.cmp(&second_is_child) {
                    std::cmp::Ordering::Greater => true,
                    std::cmp::Ordering::Less => false,
                    // transform_item_1 is in rep_1.
                    std::cmp::Ordering::Equal => !swap,
                };
                let (c, p) = if child_first { (i1, i2) } else { (i2, i1) };
                Step {
                    child: Some(frame_ref(c, child)),
                    parent: frame_ref(p, parent),
                    by: id,
                }
            } else if self
                .instance(operator)?
                .entity
                .is("CARTESIAN_TRANSFORMATION_OPERATOR_3D")
            {
                Step {
                    child: None,
                    parent: frame_ref(operator, parent),
                    by: id,
                }
            } else {
                return err(format!(
                    "{}: a placement by this kind of transformation is not read",
                    self.describe(operator)
                ));
            };
            let label = match (nauo, occurrence) {
                (Some(r), Some(o)) => {
                    let product = child_definition
                        .and_then(|d| self.product_name(d))
                        .unwrap_or_else(|| format!("#{o}"));
                    let tag = text(r.params.first())
                        .or_else(|| text(r.params.get(1)))
                        .map_or_else(|| format!("#{o}"), str::to_string);
                    format!("{product}#{tag}")
                }
                _ => format!("#{id}"),
            };
            edges.push(Edge {
                child,
                parent,
                step,
                label,
            });
        }
        // Mapped items: the item's representation is the parent.
        for (&rep, (items, _)) in &reps {
            for &item in items {
                let Ok(instance) = self.instance(item) else {
                    continue;
                };
                let Some(mapped) = instance.entity.record("MAPPED_ITEM") else {
                    continue;
                };
                let source = mapped.params.get(1).and_then(Param::as_ref);
                let target = mapped.params.get(2).and_then(Param::as_ref);
                let (Some(source), Some(target)) = (source, target) else {
                    return err(format!(
                        "{}: a mapped item without its source or target",
                        self.describe(item)
                    ));
                };
                let map = self.record(source, "REPRESENTATION_MAP")?;
                let (Some(origin), Some(of)) = (
                    map.params.first().and_then(Param::as_ref),
                    map.params.get(1).and_then(Param::as_ref),
                ) else {
                    return err(format!(
                        "{}: a representation map without its origin or representation",
                        self.describe(source)
                    ));
                };
                if !reps.contains_key(&of) {
                    return err(format!(
                        "{}: maps something that is not a representation",
                        self.describe(source)
                    ));
                }
                let (child, parent) = (groups.find(of), groups.find(rep));
                let label =
                    text(mapped.params.first()).map_or_else(|| format!("#{item}"), str::to_string);
                edges.push(Edge {
                    child,
                    parent,
                    step: Step {
                        child: Some(frame_ref(origin, child)),
                        parent: frame_ref(target, parent),
                        by: item,
                    },
                    label,
                });
            }
        }
        // The node of each item, and the names of the roots.
        let mut node_of_item: BTreeMap<u64, u64> = BTreeMap::new();
        for (&rep, (items, _)) in &reps {
            for &item in items {
                node_of_item.entry(item).or_insert_with(|| groups.find(rep));
            }
        }
        let mut root_name: BTreeMap<u64, String> = BTreeMap::new();
        for (&definition, &node) in &node_of_definition {
            if let Some(name) = self.product_name(definition) {
                root_name.entry(node).or_insert(name);
            }
        }
        let mut up: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (k, edge) in edges.iter().enumerate() {
            up.entry(edge.child).or_default().push(k);
        }
        // Every way up from each item's node.
        let mut first: Vec<Placed> = Vec::with_capacity(self.items.len());
        let mut more: Vec<Placed> = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            let Some(&node) = node_of_item.get(&item.id) else {
                first.push(Placed {
                    item: index,
                    path: String::new(),
                    steps: Vec::new(),
                });
                continue;
            };
            let mut found: Vec<Placed> = Vec::new();
            // (node, edges taken from the item up, nodes on the way).
            let mut stack: Vec<(u64, Vec<usize>)> = vec![(node, Vec::new())];
            while let Some((at, taken)) = stack.pop() {
                let parents = up.get(&at).map_or(&[][..], Vec::as_slice);
                if parents.is_empty() {
                    let mut labels: Vec<&str> = taken
                        .iter()
                        .rev()
                        .filter_map(|&k| edges.get(k).map(|e| e.label.as_str()))
                        .collect();
                    if !taken.is_empty()
                        && let Some(name) = root_name.get(&at)
                    {
                        labels.insert(0, name);
                    }
                    found.push(Placed {
                        item: index,
                        path: labels.join("/"),
                        steps: taken
                            .iter()
                            .filter_map(|&k| edges.get(k).map(|e| e.step.clone()))
                            .collect(),
                    });
                    if first.len() + more.len() + found.len() > MAX_PLACEMENTS {
                        return err(format!(
                            "the file's assemblies place more than {MAX_PLACEMENTS} solids; it is not read"
                        ));
                    }
                    continue;
                }
                if taken.len() >= MAX_DEPTH {
                    return err(format!(
                        "the file's assemblies are nested more than {MAX_DEPTH} deep; it is not read"
                    ));
                }
                // In reverse, so the first relationship is followed first.
                for &k in parents.iter().rev() {
                    let Some(edge) = edges.get(k) else { continue };
                    // The nodes on the way: the item's, and the parent of
                    // every edge taken (the last of which is `at`).
                    let on_the_way = edge.parent == node
                        || taken
                            .iter()
                            .any(|&t| edges.get(t).is_some_and(|e| e.parent == edge.parent));
                    if on_the_way {
                        return err(format!(
                            "{}: the assembly contains itself (a part placed, through others, in itself); it is not read",
                            self.describe(edge.step.by)
                        ));
                    }
                    let mut next = taken.clone();
                    next.push(k);
                    stack.push((edge.parent, next));
                }
            }
            let mut found = found.into_iter();
            if let Some(placed) = found.next() {
                first.push(placed);
            }
            more.extend(found);
        }
        first.extend(more);
        Ok(first)
    }

    /// The frame `frame` refers to, in the caller's unit (`metres_per_unit`
    /// metres each).
    fn placement_frame(
        &self,
        frame: &FrameRef,
        metres_per_unit: Option<f64>,
    ) -> Result<Frame, ImportError> {
        let context = match frame.context {
            Some(context) => context,
            None => self.only_context().ok_or_else(|| {
                ImportError(format!(
                    "{} is in no representation with units",
                    self.describe(frame.id)
                ))
            })?,
        };
        let units = crate::units::read(self, context)?;
        // None: in the file's own units.
        let scale = metres_per_unit.map_or(1.0, |m| units.metres_per_length / m);
        if !(scale.is_finite() && scale > 0.0) {
            return err(format!(
                "{}: its unit cannot be converted to the document's",
                self.describe(frame.id)
            ));
        }
        let reader = crate::geometry::Reader::new(self, scale, units.radians_per_angle);
        let entity = &self.instance(frame.id)?.entity;
        if entity.is("AXIS2_PLACEMENT_3D") {
            return reader.placement(frame.id);
        }
        if let Some(record) = entity.record("CARTESIAN_TRANSFORMATION_OPERATOR_3D") {
            // (…, axis1, axis2, local_origin, scale, axis3): read from the
            // end, as writers differ in the names before.
            let p = &record.params;
            let at = |back: usize| p.len().checked_sub(back).and_then(|k| p.get(k));
            let direction = |param: Option<&Param>| -> Result<Option<Vec3>, ImportError> {
                match param.and_then(Param::as_ref) {
                    Some(id) => Ok(Some(reader.direction(id)?)),
                    None => Ok(None),
                }
            };
            let axis3 = direction(at(1))?;
            let scale = at(2).and_then(real);
            let origin = at(3).and_then(Param::as_ref).ok_or_else(|| {
                ImportError(format!("{}: no local origin", self.describe(frame.id)))
            })?;
            let axis2 = direction(at(4))?;
            let axis1 = direction(at(5))?;
            if let Some(s) = scale
                && (s - 1.0).abs() > 1e-12
            {
                return err(format!(
                    "{}: a placement that scales by {s} is not read",
                    self.describe(frame.id)
                ));
            }
            let z = axis3.unwrap_or(Vec3::Z);
            let x = axis1.unwrap_or(if z.cross(Vec3::X).length() > 1e-6 {
                Vec3::X
            } else {
                Vec3::Y
            });
            let made = Frame::new(reader.point(origin)?, z, x).ok_or_else(|| {
                ImportError(format!(
                    "{}: its axes are parallel",
                    self.describe(frame.id)
                ))
            })?;
            if let Some(y) = axis2
                && y.dot(made.y()) < 0.0
            {
                return err(format!(
                    "{}: a placement that mirrors is not read",
                    self.describe(frame.id)
                ));
            }
            return Ok(made);
        }
        err(format!(
            "{}: a placement frame of this kind is not read",
            self.describe(frame.id)
        ))
    }

    /// Whether every step of the placement puts the child's frame onto a
    /// frame that is the same within 1e-12 in the file's own units (the
    /// test slice 1 accepted a placement by): such a placement leaves the
    /// body as read, bit for bit.
    pub(crate) fn placement_is_identity(&self, placed: &Placed) -> Result<bool, ImportError> {
        let same = |p: Vec3, q: Vec3| (p - q).length() <= 1e-12;
        for step in &placed.steps {
            let parent = self.placement_frame(&step.parent, None)?;
            let child = match &step.child {
                Some(frame) => self.placement_frame(frame, None)?,
                None => Frame::WORLD,
            };
            if !(same(parent.origin(), child.origin())
                && same(parent.x(), child.x())
                && same(parent.z(), child.z()))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The transformation of a placed solid, in the caller's unit: the
    /// steps composed from the solid up.
    ///
    /// A placement so far away that rounding there is more than the
    /// tolerance (`|t| ε` past an eighth of it) is refused: no body there
    /// could be checked.
    pub(crate) fn placement_transform(
        &self,
        placed: &Placed,
        options: &crate::Options,
    ) -> Result<Transform, ImportError> {
        let metres_per_unit = options.metres_per_unit;
        let mut total = Transform::IDENTITY;
        for step in &placed.steps {
            let parent =
                Transform::from_frame(&self.placement_frame(&step.parent, Some(metres_per_unit))?);
            let child = match &step.child {
                Some(frame) => {
                    Transform::from_frame(&self.placement_frame(frame, Some(metres_per_unit))?)
                }
                None => Transform::IDENTITY,
            };
            let inverse = child.inverse().ok_or_else(|| {
                ImportError(format!(
                    "{}: a frame without volume",
                    self.describe(step.by)
                ))
            })?;
            total = total.then(&inverse).then(&parent);
        }
        if !total.is_isometry(1e-9) {
            return err(format!(
                "the placement {} is not a rigid motion within rounding",
                placed.path
            ));
        }
        let reach = total.point(Vec3::ZERO).length();
        if !(reach * f64::EPSILON * 8.0 <= options.tolerance.linear) {
            return err(format!(
                "the placement {} puts the part {reach:e} away, where rounding is more than the tolerance {:e}",
                placed.path, options.tolerance.linear
            ));
        }
        Ok(total)
    }
}
