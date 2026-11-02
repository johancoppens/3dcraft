//! An assembly as a viewer lists it (docs/design/assemblies.md › *The
//! tree*): its instances nested by subassembly, then its mates with their
//! status and value, and the degrees of freedom left. Read from the
//! placement the view shows (the moment of a motion when one is shown,
//! else the assembly at rest); runs no command.

use crate::Session;
use crate::asm::{Built, InstanceState, Placed};
use craft3d_doc::{Document, MateKind};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Most rows an outline lists: a viewer's list, not a report.
const MAX_ROWS: usize = 2_000;

/// An assembly's outline ([`Session::assembly_outline`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AssemblyOutline {
    /// Its instances, each followed by those of its subassembly, nested.
    pub instances: Vec<OutlineInstance>,
    pub mates: Vec<OutlineMate>,
    /// The degrees of freedom left when it is solved.
    pub dof: Option<usize>,
    /// Each free motion left, in words.
    pub free: Vec<String>,
    /// Why the solve failed, when it did.
    pub solve_error: Option<String>,
    /// The document's length unit (`mm`): what a distance is set in.
    pub unit: String,
}

/// An instance row.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineInstance {
    /// Its name in the assembly the view shows (`Sub/Inner` for one of a
    /// flexible subassembly); empty-free.
    pub name: String,
    /// The name it is listed under: the last part of `name`.
    pub label: String,
    /// How deep in subassemblies: 0 for the assembly's own.
    pub depth: usize,
    /// Its source file (or `packed`).
    pub source: String,
    /// It is a subassembly.
    pub subassembly: bool,
    /// Why it is not placed, when it is not.
    pub error: Option<String>,
    /// Shown (`instance.edit` with `visible` hides it); only for the
    /// assembly's own instances.
    pub visible: bool,
    /// It can be edited from this assembly (its own instance, not one
    /// inside a subassembly).
    pub own: bool,
    /// The features of the shown part that hold its bodies (what a click
    /// selects and a double click flies to); none for an instance inside
    /// a rigid subassembly, whose bodies are the subassembly's.
    pub features: Vec<String>,
}

/// A mate row.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineMate {
    pub name: String,
    pub kind: &'static str,
    /// `ok`, `failed` (its own error) or `unsolved` (the solve failed).
    pub status: &'static str,
    pub error: Option<String>,
    /// Its main free motion's value after the solve (radians or model
    /// units), and whether that is an angle.
    pub value: Option<(f64, bool)>,
    /// Driven by a variable.
    pub driven: bool,
    /// The variables that drive its free motions, by free motion.
    pub variables: Vec<String>,
    /// Its main free motion can be set by hand (`joint.set`) and the
    /// assembly stays solved: it has one, no formula of other variables (a
    /// driver of `t`) drives it, and either a value drives it already or
    /// the solve leaves it free (driving it takes a degree of freedom and
    /// contradicts nothing: in a closed loop of one degree of freedom
    /// that one driven joint, or while none is, every joint of the loop).
    pub settable: bool,
    /// The limits of its main free motion, evaluated.
    pub limits: Option<(f64, f64)>,
}

/// A part of an assembly that the view can drag along a free motion
/// ([`Session::assembly_joint_of`]): the mate whose main free motion
/// moves it, that motion's axis in the world, and its value now.
#[derive(Clone, Debug, PartialEq)]
pub struct JointHandle {
    pub mate: String,
    /// An angle about `axis` (a revolute or cylindrical mate), else a
    /// distance along it.
    pub angle: bool,
    pub origin: [f64; 3],
    pub axis: [f64; 3],
    /// Radians or model units.
    pub value: f64,
    /// Which way the value goes for the dragged instance turning (right
    /// handed about `axis`) or moving along `axis`: 1 on the mate's side
    /// b, −1 on its side a (the value measures b against a).
    pub sign: f64,
    pub limits: Option<(f64, f64)>,
    /// The document's length unit.
    pub unit: String,
    /// The size of the assembly (the diagonal of its placed bodies' box),
    /// what "near the axis" is measured against.
    pub size: f64,
}

impl Session {
    /// The active document's outline when it is an assembly, as the view
    /// shows it: placed at the moment of the motion shown, else at rest.
    pub fn assembly_outline(&mut self) -> Option<AssemblyOutline> {
        // Placed elsewhere (the page's worker): as it was listed there,
        // the last listing of the same state while a moment is made.
        if let Some(facts) = self.remote_facts() {
            return facts?.outline.clone();
        }
        let open = self.documents.get_mut(self.active)?;
        let doc = Arc::clone(open.history.current());
        let assembly = doc.assembly.as_ref()?;
        let placed = open.shown_placed()?;
        let mut out = AssemblyOutline {
            dof: placed.dof.as_ref().map(|(dof, _)| *dof),
            free: placed
                .dof
                .as_ref()
                .map(|(_, free)| free.clone())
                .unwrap_or_default(),
            solve_error: placed.solve_error.clone(),
            unit: crate::commands::length_unit(&doc).to_string(),
            ..AssemblyOutline::default()
        };
        for st in placed.instances.iter().filter(|i| i.flex_of.is_none()) {
            let visible = assembly.instance(&st.name).is_none_or(|i| i.visible);
            push_instance(&mut out.instances, &placed, st, 0, visible, true);
        }
        for mate in &assembly.mates {
            let st = placed.mates.iter().find(|m| m.name == mate.name);
            let error = st.and_then(|s| s.error.clone());
            let status = match (&error, &placed.solve_error) {
                (Some(_), _) => "failed",
                (None, Some(_)) => "unsolved",
                (None, None) => "ok",
            };
            let angle = matches!(mate.kind, MateKind::Revolute | MateKind::Cylindrical);
            let settable = settable(&doc, mate, st);
            out.mates.push(OutlineMate {
                name: mate.name.clone(),
                kind: mate.kind.name(),
                status,
                error,
                value: st.and_then(|s| s.free).map(|v| (v, angle)),
                driven: st.is_some_and(|s| s.driven),
                variables: mate
                    .kind
                    .free()
                    .iter()
                    .map(|w| crate::asm::joint_variable(&mate.name, w))
                    .collect(),
                settable,
                limits: crate::asm::mate_limits(&doc, mate),
            });
        }
        Some(out)
    }

    /// The free motion a drag on the shown body of `feature` (an instance's
    /// placed body, `F3`) moves along: the first mate in the assembly's
    /// order that has the instance on a side and can be set by hand
    /// ([`OutlineMate::settable`]); `sign` says which way the value goes
    /// for the instance turning or moving forward about the axis (the
    /// value measures side b against side a, so −1 on side a). `None` for
    /// a part, an instance no such mate moves, and an instance held to the
    /// world (by fixed mates, or held as the first of a floating
    /// assembly): it does not move.
    pub fn assembly_joint_of(&mut self, feature: &str) -> Option<JointHandle> {
        if let Some(facts) = self.remote_facts() {
            return facts?.joints.get(feature).cloned();
        }
        let open = self.documents.get_mut(self.active)?;
        let doc = Arc::clone(open.history.current());
        let assembly = doc.assembly.as_ref()?;
        let placed = open.shown_placed()?;
        let instance = placed
            .instances
            .iter()
            .find(|i| i.bodies.iter().any(|b| b.to_string() == feature))?;
        // An instance of a flexible subassembly is named `Sub/Inner` in the
        // solve and its mates are the subassembly's: only the assembly's own.
        let name = instance.name.as_str();
        if grounded(assembly, &placed).contains(name) {
            return None;
        }
        let mate = assembly.mates.iter().find(|m| {
            let on = |r: &craft3d_doc::MateRef| r.instance == name;
            (on(&m.a) || on(&m.b))
                && settable(&doc, m, placed.mates.iter().find(|s| s.name == m.name))
        })?;
        let state = placed.mates.iter().find(|m| m.name == mate.name)?;
        let sign = if mate.b.instance == name { 1.0 } else { -1.0 };
        let frame = crate::asm::frame_of_ref(&placed, &mate.a.to_string()).ok()?;
        let angle = matches!(mate.kind, MateKind::Revolute | MateKind::Cylindrical);
        Some(JointHandle {
            mate: mate.name.clone(),
            angle,
            origin: [frame.origin.x, frame.origin.y, frame.origin.z],
            axis: [frame.z.x, frame.z.y, frame.z.z],
            value: state.free?,
            sign,
            limits: crate::asm::mate_limits(&doc, mate),
            unit: crate::commands::length_unit(&doc).to_string(),
            size: crate::asm::model_size(&placed),
        })
    }
}

/// True when `mate`'s main free motion can be set by hand and the
/// assembly stays solved ([`OutlineMate::settable`]): it has one, and its
/// variable either holds a value (what `joint.set` wrote: setting another
/// value keeps the degrees of freedom as they are) or does not exist and
/// the solve (`state`) leaves the motion free. A variable that refers to
/// another (a driver of `t`) is the timeline's.
fn settable(
    doc: &Document,
    mate: &craft3d_doc::Mate,
    state: Option<&crate::asm::MateState>,
) -> bool {
    let Some(which) = mate.kind.free().first() else {
        return false;
    };
    let var = crate::asm::joint_variable(&mate.name, which);
    match doc.variable(&var) {
        Some(v) => !doc
            .variables
            .iter()
            .any(|other| craft3d_expr::refers_to_variable(v.expr.source(), &other.name)),
        None => state.is_some_and(|s| s.main_free),
    }
}

/// The instances held to the world: those a chain of mates without a free
/// motion (fixed mates) joins to it, and the first instance of a floating
/// assembly, which the solve holds where it is placed. A drag cannot move
/// them.
fn grounded(assembly: &craft3d_doc::Assembly, placed: &Placed) -> BTreeSet<String> {
    let mut held: BTreeSet<String> = BTreeSet::new();
    if placed.floating
        && let Some(first) = placed
            .instances
            .iter()
            .find(|i| i.error.is_none() && !i.flexed)
    {
        held.insert(first.name.clone());
    }
    let rigid: Vec<(&str, &str)> = assembly
        .mates
        .iter()
        .filter(|m| m.kind.free().is_empty())
        .map(|m| (m.a.instance.as_str(), m.b.instance.as_str()))
        .collect();
    let world = |n: &str| n == craft3d_doc::WORLD;
    // Each pass adds at least one instance or stops: at most one pass per
    // mate.
    for _ in 0..=rigid.len() {
        let mut grew = false;
        for &(a, b) in &rigid {
            let a_held = world(a) || held.contains(a);
            let b_held = world(b) || held.contains(b);
            if a_held && !b_held {
                held.insert(b.to_string());
                grew = true;
            } else if b_held && !a_held {
                held.insert(a.to_string());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    held
}

/// `st` and, nested under it, its subassembly's instances: all in
/// `rows`, which holds at most [`MAX_ROWS`] at any depth.
fn push_instance(
    rows: &mut Vec<OutlineInstance>,
    placed: &Placed,
    st: &InstanceState,
    depth: usize,
    visible: bool,
    own: bool,
) {
    if rows.len() >= MAX_ROWS || depth > crate::asm::MAX_DEPTH {
        return;
    }
    let source = st.source.clone();
    let sub = match st.built.as_deref() {
        Some(Built::Assembly(sub)) => Some(Arc::clone(sub)),
        _ => None,
    };
    rows.push(OutlineInstance {
        name: st.name.clone(),
        label: st.name.rsplit('/').next().unwrap_or(&st.name).to_string(),
        depth,
        source,
        subassembly: sub.is_some(),
        error: st.error.clone(),
        visible,
        own,
        // A flexible subassembly's bodies are its instances'.
        features: if st.flexed {
            Vec::new()
        } else {
            st.bodies.iter().map(ToString::to_string).collect()
        },
    });
    if st.flexed {
        // Its instances take part in this assembly's solve, placed here.
        for child in placed
            .instances
            .iter()
            .filter(|i| i.flex_of.as_deref() == Some(st.name.as_str()))
        {
            push_instance(rows, placed, child, depth + 1, visible, false);
        }
    } else if let Some(sub) = sub {
        let first = rows.len();
        for child in sub.instances.iter().filter(|i| i.flex_of.is_none()) {
            push_instance(rows, &sub, child, depth + 1, visible, false);
        }
        // Their bodies are drawn as the subassembly's.
        for row in rows.iter_mut().skip(first) {
            row.features.clear();
        }
    }
}
