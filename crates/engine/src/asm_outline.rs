//! An assembly as a viewer lists it (docs/design/assemblies.md › *The
//! tree*): its instances nested by subassembly, then its mates with their
//! status and value, and the degrees of freedom left. Read from the
//! placement the view shows (the moment of a motion when one is shown,
//! else the assembly at rest); runs no command.

use crate::Session;
use crate::asm::{Built, InstanceState, Placed};
use craft3d_doc::{Document, MateKind, Source};
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
}

impl Session {
    /// The active document's outline when it is an assembly, as the view
    /// shows it: placed at the moment of the motion shown, else at rest.
    pub fn assembly_outline(&mut self) -> Option<AssemblyOutline> {
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
            ..AssemblyOutline::default()
        };
        for st in placed.instances.iter().filter(|i| i.flex_of.is_none()) {
            let visible = assembly.instance(&st.name).is_none_or(|i| i.visible);
            push_instance(&mut out.instances, &doc, &placed, st, 0, visible, true);
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
            });
        }
        Some(out)
    }
}

/// `st` and, nested under it, its subassembly's instances.
fn push_instance(
    rows: &mut Vec<OutlineInstance>,
    doc: &Document,
    placed: &Placed,
    st: &InstanceState,
    depth: usize,
    visible: bool,
    own: bool,
) {
    if rows.len() >= MAX_ROWS || depth > crate::asm::MAX_DEPTH {
        return;
    }
    let source = doc
        .assembly
        .as_ref()
        .and_then(|a| a.instance(&st.name))
        .map_or_else(String::new, |i| match &i.source {
            Source::File { path, .. } => path.clone(),
            Source::Packed { .. } => "packed".to_string(),
        });
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
            push_instance(rows, doc, placed, child, depth + 1, visible, false);
        }
    } else if let Some(sub) = sub {
        let mut inner = Vec::new();
        let sub_doc = Arc::clone(&sub.doc);
        for child in sub.instances.iter().filter(|i| i.flex_of.is_none()) {
            push_instance(&mut inner, &sub_doc, &sub, child, depth + 1, visible, false);
        }
        // Their bodies are drawn as the subassembly's.
        for row in &mut inner {
            row.features.clear();
        }
        rows.extend(inner);
    }
}
