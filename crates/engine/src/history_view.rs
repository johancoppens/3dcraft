//! The design history as a side panel lists it: every feature with its
//! key value ("Extrude 8 mm", "R18 × 60 mm", "− Bore"), what it is made
//! from, and the one value a click edits in place; every variable with
//! its formula, its value in the document's units and its group. Read
//! from the document alone, without regenerating it, so a window that has
//! its bodies built elsewhere (the browser's worker) can show it at once
//! (docs/design/rendering.md › *The side panel*).

use crate::Session;
use craft3d_doc::{
    AxisBy, BooleanOp, ChamferSize, Document, Expr, ExtrudeEnd, Feature, FeatureKind,
    PatternLayout, PlaneBy, PlaneRef, WorldPlane,
};
use craft3d_expr::{Dimension, Evaluation};

/// Most features and variables listed; past it the rest is left out (a
/// document that long is listed by its first ones).
pub const MAX_LISTED: usize = 100_000;
/// Longest key value, in characters.
const MAX_KEY_CHARS: usize = 64;

/// The history of the active document, as listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryView {
    /// In history order.
    pub features: Vec<HistoryEntry>,
    /// In the document's order.
    pub variables: Vec<VariableEntry>,
    /// `mm`, `deg`.
    pub length_unit: &'static str,
    pub angle_unit: &'static str,
    /// The document's linear tolerance, in its length unit.
    pub tolerance: f64,
}

/// One feature.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    /// `F3`.
    pub id: String,
    pub name: String,
    /// `extrude`, as `FeatureKind::type_name` names it.
    pub kind: &'static str,
    /// A boolean's operation: `union`, `subtract`, `intersect`.
    pub op: Option<&'static str>,
    pub is_sketch: bool,
    pub is_datum: bool,
    /// What it shows at the right of its row: `Extrude 8 mm`.
    pub key: String,
    /// The value a click on [`Self::key`] edits, when it has one.
    pub edit: Option<KeyEdit>,
    /// The features it is made from, directly, by id, in a fixed order.
    pub uses: Vec<String>,
    /// For a sketch: how many of each curve it has, in words
    /// (`2 splines`, `4 lines · 1 arc`); empty for others.
    pub curves: String,
}

/// The one value of a feature its row edits in place: the parameter of
/// `feature.edit`, the expression as written, and the words around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEdit {
    /// The parameter of `feature.edit` (`distance`, `radius`, `count`).
    pub param: &'static str,
    /// The expression as written (`8 mm`, `base_t`).
    pub expr: String,
    /// What the field is called (`Distance`).
    pub label: &'static str,
}

/// One variable.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableEntry {
    pub name: String,
    /// As written.
    pub expr: String,
    /// Its value in the document's units (`8 mm`, `30°`, `3`); `None`
    /// when it does not evaluate.
    pub value: Option<String>,
    /// Why it does not evaluate.
    pub error: Option<String>,
    /// Computed from other variables (its expression names some).
    pub formula: bool,
    /// The group it is listed under: the one a public parameter was given
    /// (`variable.publish`), else the part of its name before the first
    /// `_` when another variable shares it, else empty.
    pub group: String,
    /// A public parameter (a Jig's form shows it).
    pub public: bool,
}

impl Session {
    /// The active document's history as a side panel lists it, from the
    /// document alone (nothing is regenerated). `None` without a document.
    pub fn history_view(&self) -> Option<HistoryView> {
        let open = self.document().ok()?;
        Some(view_of(open.history.current()))
    }
}

/// The view of `doc`.
pub fn view_of(doc: &Document) -> HistoryView {
    let values = doc.variable_table().ok().map(|table| table.evaluate());
    let shown = Shown {
        doc,
        values: values.as_ref(),
    };
    let features = doc
        .features
        .iter()
        .take(MAX_LISTED)
        .map(|feature| entry(&shown, feature))
        .collect();
    let variables = variables(&shown);
    HistoryView {
        features,
        variables,
        length_unit: doc.units.length.symbol(),
        angle_unit: match doc.units.angle {
            craft3d_doc::AngleUnit::Degree => "deg",
            craft3d_doc::AngleUnit::Radian => "rad",
        },
        tolerance: doc.tolerance.linear,
    }
}

/// The document and its variables' values, for writing values.
struct Shown<'d> {
    doc: &'d Document,
    values: Option<&'d Evaluation>,
}

impl Shown<'_> {
    /// The number of `expr` and its unit, in the document's units.
    fn quantity(&self, expr: &Expr) -> Option<(String, &'static str)> {
        let q = match self.values {
            Some(values) => values.evaluate(expr.source()).ok()?,
            None => craft3d_expr::Expr::parse(expr.source())
                .ok()?
                .eval(|_| None)
                .ok()?,
        };
        if !q.value.is_finite() {
            return None;
        }
        let d = q.dimension;
        if d == Dimension::ANGLE {
            return Some(match self.doc.units.angle {
                craft3d_doc::AngleUnit::Degree => (number(q.value.to_degrees()), "°"),
                craft3d_doc::AngleUnit::Radian => (number(q.value), "rad"),
            });
        }
        if d.is_dimensionless() {
            return Some((number(q.value), ""));
        }
        let unit = d.default_unit(self.doc.units.length.as_expr_unit());
        let factor = if unit.factor.is_finite() && unit.factor > 0.0 {
            unit.factor
        } else {
            1.0
        };
        if d == Dimension::LENGTH {
            Some((number(q.value / factor), self.doc.units.length.symbol()))
        } else {
            // An area, a force: its number in that unit, the unit after.
            Some((number(q.value / factor), ""))
        }
    }

    /// A length: `expr` evaluated and written in the document's units
    /// (`8 mm`; as written when it does not evaluate), a plain number
    /// in the document's length unit (a length given without one is in
    /// it: `8` is `8 mm` in a millimetre document).
    fn length(&self, expr: &Expr) -> String {
        match self.quantity(expr) {
            Some((v, "")) => join(&v, self.doc.units.length.symbol()),
            Some((v, unit)) => join(&v, unit),
            None => expr.source().to_string(),
        }
    }

    /// An angle: a plain number in the document's angle unit (`30` is
    /// `30°` in a document in degrees).
    fn angle(&self, expr: &Expr) -> String {
        match self.quantity(expr) {
            Some((v, "")) => match self.doc.units.angle {
                craft3d_doc::AngleUnit::Degree => join(&v, "°"),
                craft3d_doc::AngleUnit::Radian => join(&v, "rad"),
            },
            Some((v, unit)) => join(&v, unit),
            None => expr.source().to_string(),
        }
    }

    /// Just the number of `expr` (`8`), for `R8 × 60 mm`; as written when
    /// it does not evaluate.
    fn bare(&self, expr: &Expr) -> String {
        self.quantity(expr)
            .map_or_else(|| expr.source().to_string(), |(v, _)| v)
    }

    fn name(&self, id: craft3d_doc::FeatureId) -> String {
        self.doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    }
}

/// `v unit`, or `v°` for degrees, or just `v`.
fn join(v: &str, unit: &str) -> String {
    match unit {
        "" => v.to_string(),
        "°" => format!("{v}°"),
        unit => format!("{v} {unit}"),
    }
}

/// A number as a person writes it: at most four decimals, no trailing
/// zeros (`8`, `3.5`, `0.25`); `?` for one that is not finite.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    // Past 1e15 four decimals mean nothing (and `v * 1e4` would overflow
    // near the top of the range): the exponent form.
    if v.abs() >= 1e15 {
        return format!("{v:e}");
    }
    let rounded = (v * 1e4).round() / 1e4;
    // −0 is 0.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

fn entry(shown: &Shown<'_>, feature: &Feature) -> HistoryEntry {
    let kind = &feature.kind;
    let (key, edit) = key_value(shown, kind);
    let key: String = key.chars().take(MAX_KEY_CHARS).collect();
    HistoryEntry {
        id: feature.id.to_string(),
        name: feature.name.clone(),
        kind: kind.type_name(),
        op: match kind {
            FeatureKind::Boolean(b) => Some(match b.op {
                BooleanOp::Union => "union",
                BooleanOp::Subtract => "subtract",
                BooleanOp::Intersect => "intersect",
                BooleanOp::Sew => "sew",
            }),
            _ => None,
        },
        is_sketch: matches!(kind, FeatureKind::Sketch(_)),
        is_datum: kind.is_datum(),
        key,
        edit,
        uses: kind.inputs().iter().map(ToString::to_string).collect(),
        curves: match kind {
            FeatureKind::Sketch(sketch) => curves(sketch),
            _ => String::new(),
        },
    }
}

/// A sketch's curves in words: `2 splines`, `4 lines · 1 arc`.
fn curves(sketch: &craft3d_doc::Sketch) -> String {
    use craft3d_doc::Geometry as G;
    let mut counts = [0usize; 4];
    for entity in sketch.entities.iter().filter(|e| !e.construction) {
        let at = match entity.geometry {
            G::Line { .. } => 0,
            G::Arc { .. } => 1,
            G::Circle { .. } => 2,
            G::Spline { .. } => 3,
            _ => continue,
        };
        if let Some(n) = counts.get_mut(at) {
            *n = n.saturating_add(1);
        }
    }
    // Splines say the most of a shape: they go first.
    let names = [
        (3, "spline", "splines"),
        (0, "line", "lines"),
        (1, "arc", "arcs"),
        (2, "circle", "circles"),
    ];
    let parts: Vec<String> = names
        .iter()
        .filter_map(|&(at, one, many)| {
            let n = *counts.get(at)?;
            (n > 0).then(|| format!("{n} {}", if n == 1 { one } else { many }))
        })
        .take(2)
        .collect();
    parts.join(" · ")
}

fn world(plane: WorldPlane) -> &'static str {
    match plane {
        WorldPlane::Xy => "XY",
        WorldPlane::Yz => "YZ",
        WorldPlane::Zx => "ZX",
    }
}

fn plane_ref(shown: &Shown<'_>, plane: &PlaneRef) -> String {
    match plane {
        PlaneRef::World { plane } => world(*plane).to_string(),
        PlaneRef::Face { face } => shown.name(face.feature),
        PlaneRef::Datum { feature } => shown.name(*feature),
    }
}

/// What a feature's row shows at the right, and the value a click there
/// edits. Where nothing short says more than the icon, the kind's word.
fn key_value(shown: &Shown<'_>, kind: &FeatureKind) -> (String, Option<KeyEdit>) {
    let edit = |param: &'static str, label: &'static str, expr: &Expr| {
        Some(KeyEdit {
            param,
            expr: expr.source().to_string(),
            label,
        })
    };
    match kind {
        FeatureKind::Extrude(e) => match &e.end {
            ExtrudeEnd::Distance => (
                format!("Extrude {}", shown.length(&e.distance)),
                edit("distance", "Distance", &e.distance),
            ),
            ExtrudeEnd::ThroughAll => ("Through all".to_string(), None),
            ExtrudeEnd::UpToFace { face } => (format!("Up to {}", shown.name(face.feature)), None),
            ExtrudeEnd::Wrap { face } => (format!("Wrap on {}", shown.name(face.feature)), None),
        },
        FeatureKind::Revolve(r) => (
            format!("Revolve {}", shown.angle(&r.angle)),
            edit("angle", "Angle", &r.angle),
        ),
        FeatureKind::Cylinder(c) => (
            format!("R{} × {}", shown.bare(&c.radius), shown.length(&c.height)),
            edit("radius", "Radius", &c.radius),
        ),
        FeatureKind::Cone(c) => (
            format!("R{} × {}", shown.bare(&c.radius), shown.length(&c.height)),
            edit("radius", "Radius", &c.radius),
        ),
        FeatureKind::Sphere(s) => (
            format!("R{}", shown.length(&s.radius)),
            edit("radius", "Radius", &s.radius),
        ),
        FeatureKind::Torus(t) => (
            format!("R{} × r{}", shown.bare(&t.major), shown.length(&t.minor)),
            edit("major", "Major radius", &t.major),
        ),
        FeatureKind::Block(b) => {
            let [x, y, z] = &b.size;
            (
                format!(
                    "{} × {} × {}",
                    shown.bare(x),
                    shown.bare(y),
                    shown.length(z)
                ),
                None,
            )
        }
        FeatureKind::Boolean(b) => {
            let sign = match b.op {
                BooleanOp::Union => "+",
                BooleanOp::Subtract => "−",
                BooleanOp::Intersect => "∩",
                BooleanOp::Sew => "⊕",
            };
            (format!("{sign} {}", shown.name(b.tool)), None)
        }
        FeatureKind::Mirror(m) => match m.plane {
            Some(plane) => (format!("Mirror · {}", shown.name(plane)), None),
            None => ("Mirror".to_string(), None),
        },
        FeatureKind::Pattern(p) => {
            let count = shown.bare(&p.count);
            let step = match &p.layout {
                PatternLayout::Linear { step } => {
                    // The length of the step when it lies along an axis
                    // (the usual case); else just the count.
                    let parts: Vec<&Expr> = step
                        .iter()
                        .filter(|e| shown.quantity(e).is_none_or(|(v, _)| v != "0"))
                        .collect();
                    match parts.as_slice() {
                        [one] => format!(" × {}", shown.length(one)),
                        _ => String::new(),
                    }
                }
                PatternLayout::Circular { angle, .. } => format!(" × {}", shown.angle(angle)),
            };
            (format!("{count}{step}"), edit("count", "Count", &p.count))
        }
        FeatureKind::Fillet(f) => (
            format!("R{} · {} edges", shown.length(&f.radius), f.edges.len()),
            edit("radius", "Radius", &f.radius),
        ),
        FeatureKind::Chamfer(c) => match &c.size {
            ChamferSize::Equal { distance } => (
                shown.length(distance),
                edit("distance", "Distance", distance),
            ),
            ChamferSize::TwoDistances { distance, second } => (
                format!("{} × {}", shown.length(distance), shown.length(second)),
                edit("distance", "Distance", distance),
            ),
            ChamferSize::DistanceAngle { distance, angle } => (
                format!("{} · {}", shown.length(distance), shown.angle(angle)),
                edit("distance", "Distance", distance),
            ),
        },
        FeatureKind::Shell(s) => (
            format!("Shell {}", shown.length(&s.thickness)),
            edit("thickness", "Thickness", &s.thickness),
        ),
        FeatureKind::Draft(d) => (
            format!("Draft {}", shown.angle(&d.angle)),
            edit("angle", "Angle", &d.angle),
        ),
        FeatureKind::Thread(t) => (
            format!("Thread {}", shown.length(&t.length)),
            edit("length", "Length", &t.length),
        ),
        FeatureKind::Loft(l) => (format!("{} sections", l.sections.len()), None),
        FeatureKind::Sweep(s) => match s.rail {
            Some(rail) => (
                format!("Along {} and {}", shown.name(s.path), shown.name(rail)),
                None,
            ),
            None => (format!("Along {}", shown.name(s.path)), None),
        },
        FeatureKind::Transform(t) => (format!("Move {}", shown.name(t.source)), None),
        FeatureKind::DatumPlane(datum) => match &datum.by {
            PlaneBy::Offset { from, distance } => (
                format!("{} · {}", plane_ref(shown, from), shown.length(distance)),
                edit("distance", "Offset", distance),
            ),
            PlaneBy::Angle { angle, .. } => (shown.angle(angle), edit("angle", "Angle", angle)),
            PlaneBy::ThreePoints { .. } => ("3 points".to_string(), None),
        },
        FeatureKind::DatumAxis(datum) => (
            match &datum.by {
                AxisBy::TwoPoints { .. } => "axis · 2 points",
                AxisBy::Edge { .. } => "axis · edge",
                AxisBy::Face { .. } => "axis",
                AxisBy::TwoPlanes { .. } => "axis · 2 planes",
            }
            .to_string(),
            None,
        ),
        FeatureKind::DatumPoint(_) => ("point".to_string(), None),
        FeatureKind::DatumHelix(h) => (
            format!("pitch {}", shown.length(&h.pitch)),
            edit("pitch", "Pitch", &h.pitch),
        ),
        // The sketch's row adds its degrees of freedom, which take solving.
        FeatureKind::Sketch(_) => (String::new(), None),
        other => (other.default_name(), None),
    }
}

/// The variables, with their values and groups.
fn variables(shown: &Shown<'_>) -> Vec<VariableEntry> {
    let doc = shown.doc;
    let prefix = |name: &str| -> Option<String> {
        let (head, _) = name.split_once('_')?;
        (!head.is_empty()).then(|| head.to_string())
    };
    // A prefix names a group only when two variables or more share it.
    let mut shared = std::collections::BTreeMap::<String, usize>::new();
    for v in doc.variables.iter().take(MAX_LISTED) {
        if let Some(p) = prefix(&v.name) {
            let n = shared.entry(p).or_insert(0);
            *n = n.saturating_add(1);
        }
    }
    doc.variables
        .iter()
        .take(MAX_LISTED)
        .map(|v| {
            let (value, error) = match shown.values.map(|values| values.get(&v.name)) {
                Some(Some(Ok(_))) => (shown.quantity(&v.expr).map(|(n, u)| join(&n, u)), None),
                Some(Some(Err(e))) => (None, Some(e.to_string())),
                // A table that does not hold together (a hand-edited
                // file): every variable says so.
                _ => (
                    None,
                    Some("the variables do not evaluate together".to_string()),
                ),
            };
            let formula = craft3d_expr::Expr::parse(v.expr.source())
                .is_ok_and(|e| !e.variables().is_empty() || !e.tables().is_empty());
            let group = match &v.public {
                Some(public) if !public.group.is_empty() => public.group.clone(),
                _ => prefix(&v.name)
                    .filter(|p| shared.get(p).is_some_and(|n| *n > 1))
                    .map(|p| {
                        let mut g = p;
                        if let Some(first) = g.get_mut(0..1) {
                            first.make_ascii_uppercase();
                        }
                        g
                    })
                    .unwrap_or_default(),
            };
            VariableEntry {
                name: v.name.clone(),
                expr: v.expr.source().to_string(),
                value,
                error,
                formula,
                group,
                public: v.public.is_some(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use super::*;
    use crate::Workspace;
    use serde_json::{Value, json};

    fn run(s: &mut Session, id: &str, params: Value) {
        s.execute(id, &params)
            .unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }

    fn key(view: &HistoryView, name: &str) -> String {
        view.features
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.key.clone())
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn numbers_are_written_short() {
        assert_eq!(number(8.0), "8");
        assert_eq!(number(3.5), "3.5");
        assert_eq!(number(0.1 + 0.2), "0.3");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(-1e-9), "0");
        assert_eq!(number(f64::NAN), "?");
        assert_eq!(number(22.123_456), "22.1235");
        assert_eq!(number(1e306), "1e306");
        assert_eq!(number(-f64::MAX).chars().next(), Some('-'));
    }

    /// The key value of each common kind, from its parameters, in the
    /// document's units, and what uses what.
    #[test]
    fn key_values_of_the_common_kinds() {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "r", "expr": "18 mm"}),
        );
        run(
            &mut s,
            "datum.plane",
            json!({"name": "Mid", "from": "yz", "distance": 0}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Sleeve", "radius": "r", "height": "60 mm"}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [120, 70, 8]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Solid", "op": "union", "target": "Base", "tool": "Sleeve"}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Hole", "radius": 3.5, "height": 20}),
        );
        run(
            &mut s,
            "feature.pattern",
            json!({"name": "Holes", "source": "Hole", "count": 3, "step": [0, 22, 0]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Drilled", "op": "subtract", "target": "Solid", "tool": "Holes"}),
        );
        run(
            &mut s,
            "feature.mirror",
            json!({"name": "Other", "source": "Drilled", "plane": "Mid"}),
        );
        let view = s.history_view().unwrap();
        assert_eq!(key(&view, "Mid"), "YZ · 0 mm");
        assert_eq!(key(&view, "Sleeve"), "R18 × 60 mm");
        assert_eq!(key(&view, "Base"), "120 × 70 × 8 mm");
        assert_eq!(key(&view, "Solid"), "+ Sleeve");
        assert_eq!(key(&view, "Holes"), "3 × 22 mm");
        assert_eq!(key(&view, "Drilled"), "− Holes");
        assert_eq!(key(&view, "Other"), "Mirror · Mid");
        let sleeve = view.features.iter().find(|f| f.name == "Sleeve").unwrap();
        assert_eq!(
            sleeve.edit,
            Some(KeyEdit {
                param: "radius",
                expr: "r".to_string(),
                label: "Radius"
            })
        );
        let drilled = view.features.iter().find(|f| f.name == "Drilled").unwrap();
        assert_eq!(drilled.op, Some("subtract"));
        // Made from the target and the tool, in that order.
        let id = |name: &str| {
            view.features
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .id
                .clone()
        };
        assert_eq!(drilled.uses, vec![id("Solid"), id("Holes")]);
        assert_eq!(view.variables[0].value.as_deref(), Some("18 mm"));
        assert!(!view.variables[0].formula);
    }

    /// An extrusion, its sketch's curves, a fillet; formulas and groups.
    #[test]
    fn extrusions_sketches_fillets_and_variables() {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "base_t", "expr": "8 mm"}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "base_r", "expr": "3 mm"}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "wall", "expr": "base_t - base_r"}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "oil", "expr": "30 deg"}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Outline", "plane": "xy"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Outline", "from": [0, 0], "to": [40, 30]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"name": "Plate", "sketch": "Outline", "distance": "base_t"}),
        );
        let view = s.history_view().unwrap();
        assert_eq!(key(&view, "Plate"), "Extrude 8 mm");
        let outline = view.features.iter().find(|f| f.name == "Outline").unwrap();
        assert!(outline.is_sketch);
        assert_eq!(outline.curves, "4 lines");
        let plate = view.features.iter().find(|f| f.name == "Plate").unwrap();
        assert_eq!(plate.uses, vec![outline.id.clone()]);
        let by_name = |n: &str| view.variables.iter().find(|v| v.name == n).unwrap();
        assert_eq!(by_name("base_t").group, "Base");
        assert_eq!(by_name("wall").group, "");
        assert!(by_name("wall").formula);
        assert_eq!(by_name("wall").value.as_deref(), Some("5 mm"));
        assert_eq!(by_name("oil").value.as_deref(), Some("30°"));
    }

    /// A variable that does not evaluate says why, and the features keep
    /// their expressions as written.
    #[test]
    fn a_broken_variable_says_why() {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "mm"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "a", "expr": "nope + 1"}),
        );
        let view = s.history_view().unwrap();
        assert!(view.variables[0].value.is_none());
        assert!(view.variables[0].error.is_some());
        assert!(Session::new(Workspace::none()).history_view().is_none());
    }
}
