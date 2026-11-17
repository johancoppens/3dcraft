//! What is selected, at the foot of the Features tab (plan of 2026-11-02,
//! boards 13 and 14): its kind and where it came from, then its
//! properties, all measured by commands:
//!
//! - a face: its surface (plane, cylinder R…, cone, sphere, torus,
//!   spline), its area, its normal or axis (`select.get` with context and
//!   measures);
//! - an edge: its curve and length, the faces it lies between (from its
//!   name), the angle there (`measure.continuity`);
//! - a body: volume, mass and area (`document.measure`), its box (the
//!   scene's);
//!
//! then the feature that made it (a link that selects it) and its
//! persistent name (the name an agent uses), with *Measure* (a face's
//! thickness, or the distance between the first two items) and *Copy
//! name*.

use super::{Shell, grouped};
use crate::kit::{Button, Glyph, Kit, ValueStyle};
use crate::shell::views::FlyTo;
use craft3d_engine::history_view::number;
use craft3d_engine::{Scene, SelectionKind, Session};
use craft3d_math::{Portable, Vec3};
use egui::{Sense, pos2, vec2};
use serde_json::{Value, json};

/// A property of what is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropRow {
    pub label: &'static str,
    pub value: String,
    pub style: PropStyle,
}

/// How a property's value is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropStyle {
    Plain,
    /// The feature that made it: a link that selects it.
    Link,
    /// A persistent name: monospace, in the info ink.
    Name,
}

/// What is selected, described.
#[derive(Clone, Debug, PartialEq)]
pub struct Props {
    /// `Face`, `Edge`, `Vertex`, `Body`.
    pub kind: &'static str,
    /// Where it came from, short: `Sleeve · face(0)`.
    pub subtitle: String,
    pub rows: Vec<PropRow>,
    /// Its persistent name (a body's: its feature's name).
    pub name: String,
    /// The feature that made it, by id.
    pub made_by: Option<String>,
    /// How many more are selected.
    pub more: usize,
    /// What Measure measures, when it can: the command and its params.
    pub measure: Option<(&'static str, Value)>,
    /// What it is: a face's surface (`plane`, `cylinder`, `cone`,
    /// `sphere`, `torus`, `freeform`) or an edge's curve (`line`,
    /// `circle`, …); `None` when it was not measured (the bodies are
    /// built elsewhere).
    pub shape: Option<String>,
    /// With two things selected, their relation (`measure.between`),
    /// shown in place of the first one's properties.
    pub measurement: Option<Measurement>,
    /// With one thing selected, the measure the view shows beside it.
    pub single: Option<Single>,
}

/// The measure of the one thing selected that the view shows beside it
/// (the developer, 2026-11-17: "ik zou maten in de view willen zien"):
/// an edge's length (a whole circle's diameter, an arc's radius), a
/// cylindrical face's diameter. A flat face, a vertex and a body have
/// none in the view (their facts are the card's).
#[derive(Clone, Debug, PartialEq)]
pub struct Single {
    /// `length`, `diameter` or `radius`.
    pub measure: &'static str,
    /// In the document's length unit.
    pub value: f64,
    /// As shown and copied: `48 mm`, `Ø 8 mm`, `R 4.5 mm`.
    pub text: String,
    /// The edge's points in space (a straight edge's two ends), along
    /// which a length is drawn; empty for a face.
    pub points: Vec<[f64; 3]>,
    /// A circle's or a cylinder's centre (on its axis, by the middle of
    /// the face), its unit axis and its radius: what a diameter or a
    /// radius is drawn across.
    pub round: Option<([f64; 3], [f64; 3], f64)>,
}

impl Single {
    /// What `ui.get` says under `measurement` for one thing.
    pub fn json(&self) -> Value {
        json!({
            "relation": null,
            "measure": self.measure,
            "value": self.value,
            "text": self.text,
        })
    }
}

/// The three numbers of `v`, an array `[x, y, z]`.
fn triple(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    let t = [
        a.first()?.as_f64()?,
        a.get(1)?.as_f64()?,
        a.get(2)?.as_f64()?,
    ];
    t.iter().all(|c| c.is_finite()).then_some(t)
}

/// What the one thing `first` shows in the view, from its description
/// (`item`, as `select.get` gives it with measures) and the scene's edge
/// lines and mesh; in the document's `unit`.
fn single_of(
    item: &Value,
    first: &craft3d_engine::SelectionItem,
    scene: Option<&Scene>,
    unit: &str,
    tolerance: f64,
) -> Option<Single> {
    let with_unit = |v: f64| length_text(v, tolerance, unit);
    let positive = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
    match first.kind {
        SelectionKind::Edge => {
            let curve = item.get("curve")?;
            // The edge's line as the view draws it.
            let points: Vec<[f64; 3]> = scene
                .and_then(|scene| {
                    let found = scene
                        .edges_named(Some(first.feature.as_str()), Some(first.name.as_str()), &[])
                        .ok()?;
                    let body = scene.bodies.get(found.body_index)?;
                    let line = body.edges.get(*found.edges.first()?)?;
                    Some(line.iter().map(|p| [p.x, p.y, p.z]).collect())
                })
                .unwrap_or_default();
            let length = positive(curve.get("length").and_then(Value::as_f64));
            match curve.get("type").and_then(Value::as_str) {
                Some("circle") => {
                    let r = positive(curve.get("radius").and_then(Value::as_f64))?;
                    let center = triple(curve.get("center")?)?;
                    let axis = triple(curve.get("axis")?)?;
                    // Whole: the edge closed, its ends one point (the review
                    // of 2026-11-17: an arc of 359.9999 degrees is an arc);
                    // without the scene's line, as long as its circle.
                    let whole = match (points.first(), points.last()) {
                        (Some(a), Some(b)) if points.len() > 2 => {
                            let ([ax, ay, az], [bx, by, bz]) = (*a, *b);
                            Vec3::new(ax, ay, az).distance(Vec3::new(bx, by, bz)) <= 1e-9 * r
                        }
                        _ => length
                            .is_some_and(|l| (l - std::f64::consts::TAU * r).abs() <= 1e-12 * l),
                    };
                    let (measure, value, text) = if whole {
                        ("diameter", 2.0 * r, format!("Ø {}", with_unit(2.0 * r)))
                    } else {
                        ("radius", r, format!("R {}", with_unit(r)))
                    };
                    Some(Single {
                        measure,
                        value,
                        text,
                        points,
                        round: Some((center, axis, r)),
                    })
                }
                Some("line") => {
                    let length = length?;
                    let ends = match (points.first(), points.last()) {
                        (Some(a), Some(b)) => vec![*a, *b],
                        _ => Vec::new(),
                    };
                    Some(Single {
                        measure: "length",
                        value: length,
                        text: with_unit(length),
                        points: ends,
                        round: None,
                    })
                }
                _ => {
                    let length = length?;
                    Some(Single {
                        measure: "length",
                        value: length,
                        text: with_unit(length),
                        points,
                        round: None,
                    })
                }
            }
        }
        SelectionKind::Face => {
            let surface = item.get("surface")?;
            if surface.get("type").and_then(Value::as_str) != Some("cylinder") {
                return None;
            }
            let r = positive(surface.get("radius").and_then(Value::as_f64))?;
            let center = triple(surface.get("center")?)?;
            let axis = triple(surface.get("axis")?)?;
            let ([cx, cy, cz], [ax, ay, az]) = (center, axis);
            let (c, a) = (Vec3::new(cx, cy, cz), Vec3::new(ax, ay, az).normalized()?);
            // On the axis by the middle of the face, not at the surface's
            // own origin, which may lie far along it.
            let at = scene
                .and_then(|scene| face_middle(scene, &first.feature, &first.name))
                .map_or(c, |m| c + a * (m - c).dot(a));
            if !at.is_finite() {
                return None;
            }
            // A part of a cylinder (a fillet's round): its radius, drawn
            // to the middle of its arc (the review of 2026-11-17).
            if let Some(middle) =
                scene.and_then(|scene| partial_arc(scene, &first.feature, &first.name, at, a))
            {
                let rim = at + middle * r;
                return Some(Single {
                    measure: "radius",
                    value: r,
                    text: format!("R {}", with_unit(r)),
                    points: vec![[rim.x, rim.y, rim.z]],
                    round: Some(([at.x, at.y, at.z], [a.x, a.y, a.z], r)),
                });
            }
            Some(Single {
                measure: "diameter",
                value: 2.0 * r,
                text: format!("Ø {}", with_unit(2.0 * r)),
                points: Vec::new(),
                round: Some(([at.x, at.y, at.z], [a.x, a.y, a.z], r)),
            })
        }
        _ => None,
    }
}

/// Where the face `name` of the body of `feature` in `scene`, round the
/// unit `axis` through `at`, leaves a gap of more than a quarter turn (a
/// part of a cylinder): the unit direction from the axis to the middle of
/// the arc it covers. `None` for a whole cylinder.
fn partial_arc(scene: &Scene, feature: &str, name: &str, at: Vec3, axis: Vec3) -> Option<Vec3> {
    let found = scene.faces_named(Some(feature), name).ok()?;
    let side = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(side).normalized()?;
    let w = axis.cross(u);
    let mut angles: Vec<f64> = Vec::new();
    for (t, id) in scene.mesh.face_ids.iter().enumerate() {
        if found.ids.binary_search(id).is_err() {
            continue;
        }
        if let Some(corners) = scene.mesh.corners(t) {
            for q in corners {
                let v = q - at;
                let angle = v.dot(w).patan2(v.dot(u));
                if angle.is_finite() {
                    angles.push(angle);
                }
            }
        }
        if angles.len() > 100_000 {
            break;
        }
    }
    angles.sort_by(f64::total_cmp);
    let (first, last) = (*angles.first()?, *angles.last()?);
    // The widest gap between angles in order, the one round the back
    // included.
    let mut gap = (first + std::f64::consts::TAU - last, last);
    for pair in angles.windows(2) {
        if let [a, b] = *pair
            && b - a > gap.0
        {
            gap = (b - a, a);
        }
    }
    let (width, starts) = gap;
    if width <= std::f64::consts::FRAC_PI_2 {
        return None;
    }
    // The arc runs from the gap's far end round to its near end.
    let middle = starts + width + (std::f64::consts::TAU - width) * 0.5;
    let direction = u * middle.pcos() + w * middle.psin();
    direction.is_finite().then_some(direction)
}

/// The middle of the face `name` of the body of `feature` in `scene`: the
/// mean of its triangles' corners.
fn face_middle(scene: &Scene, feature: &str, name: &str) -> Option<Vec3> {
    let found = scene.faces_named(Some(feature), name).ok()?;
    let (mut sum, mut n) = (Vec3::ZERO, 0_u32);
    for (t, id) in scene.mesh.face_ids.iter().enumerate() {
        if found.ids.binary_search(id).is_err() {
            continue;
        }
        if let Some([a, b, c]) = scene.mesh.corners(t) {
            sum += a + b + c;
            n = n.saturating_add(3);
        }
    }
    let middle = sum * (1.0 / f64::from(n.max(1)));
    (n > 0 && middle.is_finite()).then_some(middle)
}

/// The relation of the two things selected, as `measure.between` gives
/// it and as the window shows it: one number, what it is in a few words,
/// its parts along the axes, and the dimension line's ends in space.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// `distance`, `angle`, `centerDistance`, `leastDistance`,
    /// `clearance`; empty when the engine refused (then `refused` says
    /// why).
    pub relation: String,
    /// In the document's length unit, an angle in degrees.
    pub value: Option<f64>,
    /// The number as shown and copied: `48.00 mm`, `135.00°`.
    pub text: String,
    /// What it is: `Centre distance · axes parallel`.
    pub words: String,
    /// Its parts along X, Y and Z: `X 48.00 · Y 0.00 · Z 0.00 mm`.
    pub parts: Option<String>,
    /// The dimension line's ends; for an angle its corner too.
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub at: Option<[f64; 3]>,
    pub refused: Option<String>,
    /// The two things, as the engine took them, and the document's
    /// tolerance in the value's unit (radians for an angle): what "Keep
    /// as a check" writes.
    pub items: [Value; 2],
    pub tolerance: f64,
    /// The engine's answer as it came: what `ui.get` shows.
    pub answer: Value,
    /// What is selected, in a few words (`2 faces`), and the name a check
    /// kept of it takes (`Centre distance Left face – Right face`).
    pub title: String,
    pub check_name: String,
    /// No less than this, proven, where the relation is a least searched
    /// for (a least distance, a clearance).
    pub at_least: Option<f64>,
    /// The search rests on samples (a freeform curve or face): not proven,
    /// whatever the bound (the review of 2026-11-17, round three).
    pub sampled: bool,
    /// Being measured, on a thread of its own (`Session::between_later`):
    /// the card says so and nothing is drawn or kept yet.
    pub measuring: bool,
    /// The document's length unit, and the two things in a few words.
    pub unit: String,
    pub things: [String; 2],
}

impl Measurement {
    /// The two things `items` being measured.
    fn measuring(items: [Value; 2], unit: &str, things: [String; 2]) -> Self {
        Measurement {
            relation: String::new(),
            value: None,
            text: "…".to_string(),
            words: "Measuring…".to_string(),
            parts: None,
            from: [0.0; 3],
            to: [0.0; 3],
            at: None,
            refused: None,
            items,
            tolerance: 0.0,
            answer: Value::Null,
            title: String::new(),
            check_name: String::new(),
            at_least: None,
            sampled: false,
            measuring: true,
            unit: unit.to_string(),
            things,
        }
    }

    /// This, measured: the engine's answer, or why it gave none.
    pub fn settled(&self, answer: Result<Value, String>) -> Self {
        match answer {
            Ok(answer) => {
                let mut m = measurement_of(&answer, self.items.clone(), &self.unit);
                m.title = self.title.clone();
                m.things = self.things.clone();
                // Each thing in at most 60 characters: a check's name is
                // at most 256 bytes, and a thing's can be far longer (the
                // review of 2026-11-17, round three).
                let short = |s: &str| {
                    if s.chars().count() > 60 {
                        s.chars().take(59).collect::<String>() + "…"
                    } else {
                        s.to_string()
                    }
                };
                m.check_name = format!(
                    "{} {} – {}",
                    relation_words(&m.relation),
                    short(&self.things[0]),
                    short(&self.things[1])
                );
                m
            }
            Err(why) => {
                let mut m = self.clone();
                m.measuring = false;
                m.text = "—".to_string();
                m.words = "Not measured".to_string();
                m.refused = Some(
                    why.strip_prefix("measure.between: ")
                        .unwrap_or(&why)
                        .to_string(),
                );
                m
            }
        }
    }

    /// What `ui.get` says under `measurement`.
    pub fn json(&self) -> Value {
        json!({
            "measuring": self.measuring,
            "atLeast": self.at_least,
            "sampled": self.sampled,
            "proven": self.proven(),
            "relation": if self.relation.is_empty() { Value::Null } else { self.relation.clone().into() },
            "value": self.value,
            "text": self.text,
            "words": self.words,
            "title": self.title,
            "parts": self.parts,
            "from": self.from,
            "to": self.to,
            "at": self.at,
            "refused": self.refused,
            "answer": self.answer,
        })
    }

    /// Whether the value is what it is within the tolerance: a least
    /// searched for (a least distance, a clearance) only with a proven
    /// bound that close below it, or that prints the same, and not resting
    /// on samples (the review of 2026-11-17: "a number shown as a least
    /// distance is either proven or says what it is").
    pub fn proven(&self) -> bool {
        least_proven(
            &self.relation,
            self.value,
            self.at_least,
            self.sampled,
            self.tolerance,
        )
    }

    /// The parameters of `assert.add` that keep this as a check named
    /// `name`: the relation of the two things, the value now, the
    /// document's tolerance; where the value is not proven, what is (no
    /// more than the value found between real places, no less than the
    /// bound). `None` for what has no value (two bodies that overlap, a
    /// refusal).
    pub fn check(&self, name: &str) -> Option<Value> {
        let value = self.value.filter(|v| v.is_finite())?;
        if !self.proven() && !self.relation.is_empty() {
            let mut kept = json!({
                "name": name,
                "measure": "between",
                "from": self.items[0],
                "to": self.items[1],
                "relation": self.relation,
                "max": value,
            });
            if let (Some(least), Some(map)) = (self.at_least, kept.as_object_mut()) {
                map.insert("min".into(), least.into());
            }
            return Some(kept);
        }
        if self.relation == "angle" {
            Some(json!({
                "name": name,
                "measure": "angleBetween",
                "from": self.items[0],
                "to": self.items[1],
                "eq": format!("{value} deg"),
                "tol": format!("{} rad", self.tolerance),
            }))
        } else if self.relation.is_empty() {
            None
        } else {
            Some(json!({
                "name": name,
                "measure": "between",
                "from": self.items[0],
                "to": self.items[1],
                "relation": self.relation,
                "eq": value,
                "tol": self.tolerance,
            }))
        }
    }
}

/// Whether a least searched for (`relation` a least distance or a
/// clearance) is proven to `value` within `tolerance`: by `at_least` that
/// close below it or printing the same at the tolerance (a range narrower
/// than its digits reads as noise: the review of 2026-11-17, round
/// three), and not `sampled`. Any other relation is what it is.
fn least_proven(
    relation: &str,
    value: Option<f64>,
    at_least: Option<f64>,
    sampled: bool,
    tolerance: f64,
) -> bool {
    let searched = matches!(relation, "leastDistance" | "clearance");
    !searched
        || (!sampled
            && value.zip(at_least).is_some_and(|(v, least)| {
                least >= v - tolerance || number_to(least, tolerance) == number_to(v, tolerance)
            }))
}

/// An item of the selection as `measure.between` takes it.
fn item_json(item: &craft3d_engine::SelectionItem) -> Value {
    json!({"kind": item.kind.name(), "name": item.name, "feature": item.feature})
}

/// The relation of `a` and `b` (`measure.between`), in the document's
/// `unit`, being measured: the parameters to measure it with go to
/// `wanted`, for the panel to measure on a thread of its own, one at a
/// time (the review of 2026-11-17: on the window's thread it held a frame
/// up); `None` for kinds it does not take (a feature, a sketch, a datum).
pub(crate) fn relation(
    a: &craft3d_engine::SelectionItem,
    b: &craft3d_engine::SelectionItem,
    unit: &str,
    things: [String; 2],
    wanted: &mut Option<Value>,
) -> Option<Measurement> {
    let measured = |kind: SelectionKind| {
        matches!(
            kind,
            SelectionKind::Face | SelectionKind::Edge | SelectionKind::Vertex | SelectionKind::Body
        )
    };
    if !(measured(a.kind) && measured(b.kind)) {
        return None;
    }
    let items = [item_json(a), item_json(b)];
    *wanted = Some(json!({"from": items[0], "to": items[1]}));
    Some(Measurement::measuring(items, unit, things))
}

/// `v` as the window prints a measure: to the decimals `tolerance` holds
/// (a length's own, an angle's in degrees), no trailing zeros (the review
/// of 2026-11-17: two fixed decimals showed a 0.006 diameter as 0.01).
/// What is copied is what is shown.
pub fn number_to(v: f64, tolerance: f64) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    let decimals = if tolerance.is_finite() && tolerance > 0.0 {
        (-tolerance.plog10()).ceil().clamp(0.0, 12.0) as usize
    } else {
        6
    };
    let text = format!("{v:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" { "0".to_string() } else { text }
}

/// A length to `tolerance`, with the document's `unit`.
pub fn length_text(v: f64, tolerance: f64, unit: &str) -> String {
    let n = number_to(v, tolerance);
    if unit.is_empty() {
        n
    } else {
        format!("{n} {unit}")
    }
}

/// The engine's answer as the window shows it.
pub fn measurement_of(answer: &Value, items: [Value; 2], unit: &str) -> Measurement {
    let point = |key: &str| -> Option<[f64; 3]> {
        let v = answer.get(key)?.as_array()?;
        Some([
            v.first()?.as_f64()?,
            v.get(1)?.as_f64()?,
            v.get(2)?.as_f64()?,
        ])
    };
    let relation = answer
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let angle = relation == "angle";
    let value = if angle {
        answer.get("degrees").and_then(Value::as_f64)
    } else {
        answer.get("value").and_then(Value::as_f64)
    };
    let tolerance = answer
        .get("tolerance")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let at_least = answer.get("atLeast").and_then(Value::as_f64);
    let sampled = answer.get("sampled").and_then(Value::as_bool) == Some(true);
    let unproven = !least_proven(&relation, value, at_least, sampled, tolerance);
    let text = match value {
        Some(v) if angle => format!("{}°", number_to(v, tolerance.to_degrees())),
        // Not proven within the tolerance: "about" where it rests on
        // samples or has no bound, else the range it is proven in.
        Some(v) if unproven => match at_least.filter(|_| !sampled) {
            Some(l) => length_text(v, tolerance, unit).replacen(
                &number_to(v, tolerance),
                &format!(
                    "{} – {}",
                    number_to(l.max(0.0), tolerance),
                    number_to(v, tolerance)
                ),
                1,
            ),
            None => format!("about {}", length_text(v, tolerance, unit)),
        },
        Some(v) => length_text(v, tolerance, unit),
        None if answer.get("overlap").and_then(Value::as_bool) == Some(true) => {
            "Overlap".to_string()
        }
        None => "—".to_string(),
    };
    let mut words = answer
        .get("words")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if words == "Centre distance" && answer.get("parallel").and_then(Value::as_bool) == Some(true) {
        words.push_str(" · axes parallel");
    }
    let parts = matches!(
        relation.as_str(),
        "centerDistance" | "distance" | "leastDistance"
    )
    .then(|| point("parts"))
    .flatten()
    .map(|[x, y, z]| {
        let [x, y, z] = [x, y, z].map(|c| number_to(c, tolerance));
        format!("X {x} · Y {y} · Z {z} {unit}")
            .trim_end()
            .to_string()
    });
    Measurement {
        relation,
        value,
        text,
        words,
        parts,
        from: point("from").unwrap_or([0.0; 3]),
        to: point("to").unwrap_or([0.0; 3]),
        at: point("at"),
        refused: None,
        items,
        tolerance,
        answer: answer.clone(),
        title: String::new(),
        check_name: String::new(),
        at_least,
        sampled,
        measuring: false,
        unit: unit.to_string(),
        things: [String::new(), String::new()],
    }
}

/// A relation's name in words, short: what a check kept of it is called
/// after.
fn relation_words(relation: &str) -> &'static str {
    match relation {
        "distance" => "Distance",
        "angle" => "Angle",
        "centerDistance" => "Centre distance",
        "leastDistance" => "Least distance",
        "clearance" => "Clearance",
        _ => "Measure",
    }
}

/// A thing of the selection in a few words: a face as the feature that
/// made it and its own part (`Left face`), an edge or a vertex by its
/// body, a body by its feature's name.
fn thing_words(item: &craft3d_engine::SelectionItem, scene: Option<&Scene>) -> String {
    let feature = scene
        .and_then(|s| s.features.iter().find(|f| f.id == item.feature))
        .map_or_else(|| item.feature.clone(), |f| f.name.clone());
    match item.kind {
        SelectionKind::Face => face_words(&item.name, scene),
        SelectionKind::Edge => format!("{feature} edge"),
        SelectionKind::Vertex => format!("{feature} vertex"),
        _ => feature,
    }
}

/// The icon of what is selected: a face by its surface, an edge a line
/// or a curve, a face not measured a neutral one.
pub fn props_glyph(kind: &str, shape: Option<&str>) -> Glyph {
    match (kind, shape) {
        ("Face", Some("plane")) => Glyph::DatumPlane,
        ("Face", Some("cylinder")) => Glyph::Cylinder,
        ("Face", Some("cone")) => Glyph::Cone,
        ("Face", Some("sphere")) => Glyph::Sphere,
        ("Face", Some("torus")) => Glyph::Torus,
        ("Face", _) => Glyph::Surface,
        ("Edge", Some("line")) => Glyph::DatumAxis,
        ("Edge", _) => Glyph::Sketch,
        ("Vertex", _) => Glyph::DatumPoint,
        _ => Glyph::Block,
    }
}

/// The selection's first item described, from the session; `None` for
/// a selection of features (their rows show them). Without the bodies
/// here (`bodies_here` false) only what the names say.
pub(crate) fn read(
    session: &mut Session,
    scene: Option<&Scene>,
    bodies_here: bool,
    wanted: &mut Option<Value>,
) -> Option<Props> {
    let view = session.selection();
    let first = view.items.first()?.clone();
    let more = view.items.len().saturating_sub(1);
    let second = view.items.get(1).cloned();
    let unit = session
        .document_status()
        .map_or("", |s| s.length_unit)
        .to_string();
    let name_of = |id: &str| -> String {
        scene
            .and_then(|s| s.features.iter().find(|f| f.id == id))
            .map_or_else(|| id.to_string(), |f| f.name.clone())
    };
    let kind = match first.kind {
        SelectionKind::Face => "Face",
        SelectionKind::Edge => "Edge",
        SelectionKind::Vertex => "Vertex",
        SelectionKind::Body => "Body",
        _ => return None,
    };
    let mut rows = Vec::new();
    let mut made_by = None;
    let mut subtitle = String::new();
    // Two things: their relation, by itself (the Measure button stays
    // for one face's wall thickness).
    let measurement = match (&second, view.items.len()) {
        (Some(other), 2) if bodies_here => {
            let things = [thing_words(&first, scene), thing_words(other, scene)];
            relation(&first, other, &unit, things, wanted).map(|mut m| {
                m.title = craft3d_engine::selection_summary(&view.items);
                m
            })
        }
        _ => None,
    };
    if first.kind == SelectionKind::Body {
        let id = first.feature.clone();
        subtitle = name_of(&id);
        if bodies_here
            && let Ok(m) = session.execute(
                "document.measure",
                &json!({"feature": id, "inertia": false}),
            )
        {
            if let Some(v) = m.get("volume").and_then(Value::as_f64) {
                rows.push(plain("Volume", volume_text(v, &unit)));
            }
            if let Some(kg) = m.get("mass").and_then(Value::as_f64) {
                rows.push(plain("Mass", mass_text(kg)));
            }
            if let Some(a) = m.get("area").and_then(Value::as_f64) {
                rows.push(plain("Area", format!("{} {unit}²", grouped(a, 0))));
            }
        }
        if let Some((lo, hi)) = scene.and_then(|s| s.feature_box(&id).corners()) {
            let d = hi - lo;
            rows.push(plain(
                "Box",
                format!("{} × {} × {} {unit}", number(d.x), number(d.y), number(d.z)),
            ));
        }
        made_by = Some(id.clone());
        return Some(Props {
            kind,
            subtitle,
            rows,
            name: first.name.clone(),
            made_by,
            more,
            measure: None,
            shape: None,
            measurement,
            single: None,
        });
    }
    let described = if bodies_here {
        session
            .execute("select.get", &json!({"context": true, "measures": true}))
            .ok()
            .and_then(|v| v.get("items").and_then(Value::as_array)?.first().cloned())
    } else {
        None
    };
    // One thing: the measure the view shows beside it.
    let single = match (&described, view.items.len()) {
        (Some(item), 1) => single_of(
            item,
            &first,
            scene,
            &unit,
            session.linear_tolerance().unwrap_or(1e-6),
        ),
        _ => None,
    };
    let local = local_name(&first.name);
    if let Some(item) = &described {
        if let Some(origin) = item.get("origin") {
            let id = origin.get("feature").and_then(Value::as_str).unwrap_or("");
            let name = origin.get("name").and_then(Value::as_str).unwrap_or(id);
            if !id.is_empty() {
                made_by = Some(id.to_string());
                subtitle = format!("{name} · {local}");
            }
        }
        if let Some(surface) = item.get("surface") {
            rows.push(plain("Type", surface_text(surface, &unit)));
        }
        if let Some(area) = item.get("area").and_then(Value::as_f64) {
            rows.push(plain("Area", format!("{} {unit}²", grouped(area, 0))));
        }
        if let Some(surface) = item.get("surface") {
            if let Some(n) = surface.get("normal") {
                rows.push(plain("Normal", direction_text(n)));
            } else if let Some(axis) = surface.get("axis") {
                let through = surface.get("center").map(point_text).unwrap_or_default();
                rows.push(plain(
                    "Axis",
                    format!("{}  through {through}", direction_text(axis)),
                ));
            }
        }
        if let Some(curve) = item.get("curve") {
            rows.push(plain("Type", curve_text(curve, &unit)));
            if let Some(length) = curve.get("length").and_then(Value::as_f64) {
                rows.push(plain("Length", format!("{} {unit}", decimals(length, 2))));
            }
        }
        if let Some(point) = item.get("point") {
            rows.push(plain("Point", point_text(point)));
        }
    }
    if subtitle.is_empty() {
        subtitle = format!("{} · {local}", name_of(&first.feature));
    }
    if first.kind == SelectionKind::Edge {
        let faces = between(&first.name, scene, &first.feature);
        if let [a, b] = faces.as_slice() {
            rows.push(plain(
                "Between",
                format!("{} · {}", face_words(a, scene), face_words(b, scene)),
            ));
        }
        if bodies_here && let Some(angle) = edge_angle(session, &first.name, &first.feature, &faces)
        {
            rows.push(plain("Angle", format!("{}°", number(angle.to_degrees()))));
        }
    }
    if let Some(id) = &made_by {
        rows.push(PropRow {
            label: "Made by",
            value: format!("{} ({id})", name_of(id)),
            style: PropStyle::Link,
        });
    }
    rows.push(PropRow {
        label: "Name",
        value: first.name.clone(),
        style: PropStyle::Name,
    });
    // Measure runs here: not while the bodies are built elsewhere (a
    // command there would answer later, deferred).
    let measure = (first.kind == SelectionKind::Face && second.is_none())
        .then(|| {
            (
                "measure.thickness",
                json!({"of": {"kind": "face", "where": {"in": first.feature, "name": first.name}}}),
            )
        })
        .filter(|_| bodies_here);
    let shape = described.as_ref().and_then(|item| {
        item.get("surface")
            .or_else(|| item.get("curve"))
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Some(Props {
        kind,
        subtitle,
        rows,
        name: first.name.clone(),
        made_by,
        more,
        measure,
        shape,
        measurement,
        single,
    })
}

/// The angle between the faces an edge lies between, in radians: what
/// `measure.continuity` proves along it (the angle between their tangent
/// planes); where it does not compare the two faces yet (two analytic
/// faces), from their surfaces as `select.query` gives them: the normals
/// of two planes, or a plane square to a cylinder's or a cone's axis
/// (a rim: a right angle). `None` when neither says.
fn edge_angle(session: &mut Session, edge: &str, feature: &str, faces: &[String]) -> Option<f64> {
    if let Ok(c) = session.execute("measure.continuity", &json!({"edge": edge}))
        && let Some(angle) = c.get("angle").and_then(Value::as_f64)
    {
        return Some(angle);
    }
    let surface = |session: &mut Session, name: &str| -> Option<Value> {
        let found = session
            .execute(
                "select.query",
                &json!({"kind": "face", "where": {"in": feature, "name": name}, "context": true, "limit": 1}),
            )
            .ok()?;
        found
            .get("items")?
            .as_array()?
            .first()?
            .get("surface")
            .cloned()
    };
    let [a, b] = faces else {
        return None;
    };
    let sa = surface(session, a)?;
    let sb = surface(session, b)?;
    let vector = |v: Option<&Value>| -> Option<[f64; 3]> {
        let a = v?.as_array()?;
        Some([
            a.first()?.as_f64()?,
            a.get(1)?.as_f64()?,
            a.get(2)?.as_f64()?,
        ])
    };
    let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let kind = |s: &Value| {
        s.get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let (ka, kb) = (kind(&sa), kind(&sb));
    if ka == "plane" && kb == "plane" {
        let (na, nb) = (vector(sa.get("normal"))?, vector(sb.get("normal"))?);
        let cos = dot(na, nb).clamp(-1.0, 1.0);
        return Some(craft3d_math::Portable::pacos(cos));
    }
    let (plane, round) = match (ka.as_str(), kb.as_str()) {
        ("plane", "cylinder" | "cone") => (&sa, &sb),
        ("cylinder" | "cone", "plane") => (&sb, &sa),
        _ => return None,
    };
    let n = vector(plane.get("normal"))?;
    let axis = vector(round.get("axis"))?;
    // A rim: the plane square to the axis meets the side at a right angle.
    (dot(n, axis).abs() > 1.0 - 1e-9).then_some(std::f64::consts::FRAC_PI_2)
}

fn plain(label: &'static str, value: String) -> PropRow {
    PropRow {
        label,
        value,
        style: PropStyle::Plain,
    }
}

/// `v` with `places` decimals, trailing zeros kept (a length read off a
/// curve: `21.99`).
fn decimals(v: f64, places: usize) -> String {
    if v.is_finite() {
        format!("{v:.places$}")
    } else {
        "?".to_string()
    }
}

/// A volume: cm³ in a millimetre document, else the unit cubed.
pub fn volume_text(v: f64, unit: &str) -> String {
    if unit == "mm" {
        format!("{} cm³", grouped(v / 1000.0, 2))
    } else {
        format!("{} {unit}³", grouped(v, 2))
    }
}

/// A mass in kilograms, in grams below one.
pub fn mass_text(kg: f64) -> String {
    if kg.abs() < 1.0 {
        format!("{} g", grouped(kg * 1000.0, 1))
    } else {
        format!("{} kg", grouped(kg, 3))
    }
}

/// The last part of a persistent name, the face's own: `face(0)` of
/// `F23:copy(0)/F10:copy(1)/F6:face(0)`.
fn local_name(name: &str) -> String {
    // An edge named by the faces it lies between.
    if name.contains("between[") {
        return "edge".to_string();
    }
    let last = name.rsplit('/').next().unwrap_or(name);
    let local = last.split_once(':').map_or(last, |(_, l)| l);
    let local: String = local.chars().take(32).collect();
    local
}

/// The faces an edge lies between, by their names: from a name made of
/// them (`F23:between[A,B]`), else from the body's facts in the scene.
fn between(edge: &str, scene: Option<&Scene>, feature: &str) -> Vec<String> {
    if let Some(inner) = edge
        .split_once("between[")
        .and_then(|(_, rest)| rest.strip_suffix(']'))
    {
        // The two names, split at the comma outside parentheses.
        let mut depth = 0i32;
        for (i, c) in inner.char_indices() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    let (a, b) = inner.split_at(i);
                    return vec![a.to_string(), b.get(1..).unwrap_or("").to_string()];
                }
                _ => {}
            }
        }
    }
    let Some(body) = scene.and_then(|s| s.bodies.iter().find(|b| b.feature == feature)) else {
        return Vec::new();
    };
    let Some(k) = body.edge_names.iter().position(|n| n == edge) else {
        return Vec::new();
    };
    body.edge_faces
        .get(k)
        .map(|faces| {
            faces
                .iter()
                .filter_map(|&f| body.face_names.get(f).cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// A face's name in words: the feature that made it and its own part
/// (`Hole face(0)`).
fn face_words(name: &str, scene: Option<&Scene>) -> String {
    let last = name.rsplit('/').next().unwrap_or(name);
    let Some((id, local)) = last.split_once(':') else {
        return last.to_string();
    };
    let feature = scene
        .and_then(|s| s.features.iter().find(|f| f.id == id))
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    let local = local.split_once('(').map_or(local, |(head, _)| head);
    format!("{feature} {local}")
}

fn surface_text(surface: &Value, unit: &str) -> String {
    let r = |key: &str| surface.get(key).and_then(Value::as_f64).map(number);
    // A measure that is missing leaves the surface's word alone, not
    // `R  mm`.
    let with = |word: &str, text: Option<String>| {
        text.map_or_else(|| word.to_string(), |t| format!("{word}  {t}"))
    };
    match surface.get("type").and_then(Value::as_str).unwrap_or("") {
        "plane" => "Plane".to_string(),
        "cylinder" => with("Cylinder", r("radius").map(|v| format!("R {v} {unit}"))),
        "cone" => with(
            "Cone",
            r("radius")
                .zip(r("halfAngle"))
                .map(|(rad, a)| format!("R {rad} {unit} · {a}°")),
        ),
        "sphere" => with("Sphere", r("radius").map(|v| format!("R {v} {unit}"))),
        "torus" => with(
            "Torus",
            r("major")
                .zip(r("minor"))
                .map(|(a, b)| format!("R {a} · r {b} {unit}")),
        ),
        "freeform" | "nurbs" | "bezier" => "Spline".to_string(),
        other => {
            let mut word = other.to_string();
            if let Some(first) = word.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            word
        }
    }
}

fn curve_text(curve: &Value, unit: &str) -> String {
    match curve.get("type").and_then(Value::as_str).unwrap_or("") {
        "line" => "Line".to_string(),
        "circle" => match curve.get("radius").and_then(Value::as_f64) {
            Some(r) => format!("Circle  R {} {unit}", number(r)),
            None => "Circle".to_string(),
        },
        "ellipse" => "Ellipse".to_string(),
        _ => "Spline".to_string(),
    }
}

/// A direction: `+Z` along an axis, else its three numbers.
fn direction_text(d: &Value) -> String {
    let v: Vec<f64> = d
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let [x, y, z] = v.as_slice() else {
        return "?".to_string();
    };
    for (axis, c, others) in [("X", x, [y, z]), ("Y", y, [x, z]), ("Z", z, [x, y])] {
        if (c.abs() - 1.0).abs() < 1e-9 && others.iter().all(|o| o.abs() < 1e-9) {
            return if *c > 0.0 {
                axis.to_string()
            } else {
                format!("−{axis}")
            };
        }
    }
    format!("({}, {}, {})", number(*x), number(*y), number(*z))
}

fn point_text(p: &Value) -> String {
    let v: Vec<String> = p
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).map(number).collect())
        .unwrap_or_default();
    format!("({})", v.join(", "))
}

/// The answer of a measure in one line: `Thickness 7 mm`.
pub(crate) fn measured_line(command: &str, result: &Value) -> String {
    let unit = result
        .get("lengthUnit")
        .and_then(Value::as_str)
        .unwrap_or("");
    for (key, word) in [
        ("distance", "Distance"),
        ("thickness", "Thickness"),
        ("clearance", "Clearance"),
    ] {
        if let Some(v) = result.get(key).and_then(Value::as_f64) {
            return format!("{word} {} {unit}", number(v))
                .trim_end()
                .to_string();
        }
    }
    format!("{command}: no single value")
}

impl Shell {
    /// What is selected, as described at the foot of the Features tab.
    pub fn side_props(&self) -> Option<&Props> {
        self.side.props.as_ref()
    }

    /// How high the card is drawn: the rule, its heading, a row per
    /// property, the measure's answer, the buttons.
    pub(super) fn props_height(&self) -> f32 {
        let Some(props) = &self.side.props else {
            return 0.0;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let measured = if self.side.measured.is_some() {
            1.0
        } else {
            0.0
        };
        // Two things: one large number and a line or two of what it is
        // in place of the rows.
        let body = match &props.measurement {
            Some(m) => {
                let lines = 1.0
                    + f32::from(u8::from(m.parts.is_some()))
                    + f32::from(u8::from(m.refused.is_some()));
                t.measure_font * 1.5 + lines * t.label_row_height
            }
            None => props.rows.len() as f32 * t.label_row_height,
        };
        t.nav_gap
            + 1.0
            + kit.hit(t.control_height)
            + body
            + measured * t.label_row_height
            + 4.0 * t.nav_spacing
            + kit.hit(t.control_height)
    }

    /// The relation of two things at the foot of the panel: one large
    /// number (a click copies it with its unit), what it is in a few
    /// words, its parts along the axes, why it was not measured; and
    /// "Keep as a check".
    fn measurement_ui(&mut self, ui: &mut egui::Ui, m: &Measurement) -> bool {
        if m.measuring {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let sk = kit.skin;
        let big = kit.galley(
            ui,
            m.text.clone(),
            egui::FontId::proportional(t.measure_font),
            sk.text,
        );
        let (rect, number) = ui.allocate_exact_size(
            vec2(ui.available_width(), t.measure_font * 1.5),
            Sense::click(),
        );
        let at = pos2(rect.left(), rect.center().y - big.size().y * 0.5);
        let ink = if number.hovered() && m.value.is_some() {
            sk.info_ink
        } else {
            sk.text
        };
        let shown = egui::Rect::from_min_size(at, big.size());
        ui.painter().galley(at, big, kit.c(ink));
        self.side
            .hits
            .targets
            .push(("copy value".to_string(), shown));
        if m.value.is_some() {
            if number.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if number.clone().on_hover_text("Copy").clicked() {
                ui.ctx().copy_text(m.text.clone());
                self.side.measured = Some(format!("Copied {}", m.text));
            }
        }
        let small = |ui: &mut egui::Ui, text: &str| {
            let room = ui.available_width();
            let mut job = egui::text::LayoutJob::simple_singleline(
                text.to_string(),
                kit.font(),
                kit.c(sk.text_dim),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = ui.painter().layout_job(job);
            let cut = g.elided;
            let (rect, line) =
                ui.allocate_exact_size(vec2(room, t.label_row_height), Sense::hover());
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - g.size().y * 0.5),
                g,
                kit.c(sk.text_dim),
            );
            if cut {
                line.on_hover_text(text);
            }
        };
        small(ui, &m.words);
        if let Some(parts) = &m.parts {
            small(ui, parts);
        }
        if let Some(why) = &m.refused {
            small(ui, why);
        }
        m.check("").is_some()
    }

    /// Whether the relation of the two things selected is kept as a check
    /// already: for this document and selection (the check changes the
    /// revision, not that), while the check is there (undone, it can be
    /// kept again: the review of 2026-11-17).
    fn kept_now(&self) -> bool {
        self.side.kept.as_ref().is_some_and(|(key, name)| {
            Some(*key) == self.side.kept_key()
                && self.assertion_rows().iter().any(|a| a.name == *name)
        })
    }

    /// "Keep as a check" (or that it is kept); true when it was clicked.
    fn keep_button(&mut self, ui: &mut egui::Ui, keepable: bool) -> bool {
        let kit = Kit::panel(&self.tokens);
        let kept = self.kept_now();
        let k = kit.button(
            ui,
            Button::primary(if kept {
                "Kept as a check"
            } else {
                "Keep as a check"
            })
            .enabled(keepable && !kept),
        );
        self.side.hits.targets.push(("keep".to_string(), k.rect));
        k.clicked()
    }

    /// Keeps the relation `m` as a check of the document, under a name of
    /// its own.
    fn keep(&mut self, m: &Measurement) {
        let name = self.free_check_name(&m.check_name);
        if let (Some(params), Some(key)) = (m.check(&name), self.side.kept_key()) {
            self.side_queue("assert.add", params, None);
            self.side.kept = Some((key, name));
        }
    }

    /// The card in one line, where the Features tab has no room for it (a
    /// short window: the review of 2026-11-17 found the relation and Keep
    /// out of reach there): what is selected and its number, and Keep as a
    /// check for two things.
    pub(super) fn props_compact_ui(&mut self, ui: &mut egui::Ui) {
        let Some(props) = self.side.props.clone() else {
            return;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let (text, keepable) = match (&props.measurement, &props.single) {
            (Some(m), _) => (format!("{} {}", m.words, m.text), m.check("").is_some()),
            (None, Some(single)) => (format!("{} {}", props.kind, single.text), false),
            (None, None) => (format!("{} · {}", props.kind, props.subtitle), false),
        };
        if props.measurement.as_ref().is_some_and(|m| m.measuring) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let mut keep = false;
        ui.horizontal(|ui| {
            let button = if props.measurement.is_some() {
                120.0
            } else {
                0.0
            };
            let room = (ui.available_width() - button).max(40.0);
            let shown = clip(ui, &kit, &text, ValueStyle::Plain, room);
            let (rect, line) =
                ui.allocate_exact_size(vec2(room, kit.hit(t.control_height)), Sense::click());
            let galley = kit.galley(ui, shown, kit.font(), kit.skin.text);
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                galley,
                kit.c(kit.skin.text),
            );
            self.side.hits.targets.push(("compact".to_string(), rect));
            line.on_hover_text(text.clone());
            if props.measurement.is_some() {
                keep = self.keep_button(ui, keepable);
            }
        });
        if keep && let Some(m) = &props.measurement {
            self.keep(m);
        }
    }

    /// How high the one-line card is.
    pub(super) fn props_compact_height(&self) -> f32 {
        let kit = Kit::panel(&self.tokens);
        kit.hit(kit.t.control_height) + 2.0 * kit.t.nav_spacing
    }

    /// The card of what is selected.
    pub(super) fn props_ui(&mut self, ui: &mut egui::Ui) {
        let Some(props) = self.side.props.clone() else {
            return;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let sk = kit.skin;
        kit.divider(ui);
        // The heading: the kind in the selection's copper, where it came
        // from, and the target that flies to it.
        let mut fly = false;
        ui.horizontal(|ui| {
            let side = t.nav_icon_size;
            let (icon, _) = ui.allocate_exact_size(vec2(side, t.row_height), Sense::hover());
            let glyph = props_glyph(props.kind, props.shape.as_deref());
            crate::kit::glyphs::paint(
                ui.painter(),
                glyph,
                egui::Rect::from_center_size(icon.center(), vec2(side, side)),
                kit.c(t.face_highlight),
                &t,
            );
            let title = props
                .measurement
                .as_ref()
                .map_or_else(|| props.kind.to_string(), |m| m.title.clone());
            let title = kit.galley(ui, title, kit.medium_font(ui), sk.text);
            let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
            ui.painter().galley(rect.min, title, kit.c(sk.text));
            let mut subtitle = props.subtitle.clone();
            if props.measurement.is_some() {
                subtitle.clear();
            } else if props.more > 0 {
                subtitle = format!("{subtitle} · +{}", props.more);
            }
            // The target against the right edge first, the subtitle cut
            // to what is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let target = kit.icon_button(
                    ui,
                    Glyph::Nav(crate::shell::icons::Icon::ZoomTo),
                    false,
                    "Fly to it",
                );
                self.side
                    .hits
                    .targets
                    .push(("fly".to_string(), target.rect));
                fly = target.clicked();
                let room = ui.available_width().max(0.0);
                let mut job = egui::text::LayoutJob::simple_singleline(
                    subtitle.clone(),
                    kit.font(),
                    kit.c(sk.text_dim),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let g = ui.painter().layout_job(job);
                let cut = g.elided;
                let (rect, line) = ui.allocate_exact_size(vec2(room, g.size().y), Sense::hover());
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - g.size().y * 0.5),
                    g,
                    kit.c(sk.text_dim),
                );
                if cut {
                    line.on_hover_text(subtitle);
                }
            });
        });
        let label_width = (ui.available_width() * 0.32).clamp(56.0, 110.0);
        let mut follow: Option<String> = None;
        let keepable = match &props.measurement {
            Some(m) => self.measurement_ui(ui, m),
            None => false,
        };
        let rows: &[PropRow] = if props.measurement.is_some() {
            &[]
        } else {
            &props.rows
        };
        for row in rows {
            let style = match row.style {
                PropStyle::Plain => ValueStyle::Plain,
                PropStyle::Link => ValueStyle::Link,
                PropStyle::Name => ValueStyle::Name,
            };
            // A long name is cut to the panel (the whole is copied).
            let value = clip(
                ui,
                &kit,
                &row.value,
                style,
                ui.available_width() - label_width,
            );
            let r = kit.prop_row(ui, row.label, &value, style, label_width);
            self.side.hits.drawn.push(r.rect);
            // A link's row takes the click: its response is the one under
            // the pointer.
            let r = if row.style == PropStyle::Link {
                ui.interact(
                    r.rect,
                    egui::Id::new(("craft3d-side-made-by", &row.value)),
                    Sense::click(),
                )
            } else {
                r
            };
            // Cut to the panel: the whole on hover.
            if value != row.value {
                r.clone().on_hover_text(&row.value);
            }
            if row.style == PropStyle::Link {
                let link = r;
                self.side
                    .hits
                    .targets
                    .push(("made by".to_string(), link.rect));
                if link.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if link.clicked() {
                    follow = props.made_by.clone();
                }
            }
        }
        if let Some(line) = &self.side.measured {
            let g = kit.galley(ui, line.clone(), kit.font(), sk.info_ink);
            let (rect, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), t.label_row_height),
                Sense::hover(),
            );
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - g.size().y * 0.5),
                g,
                kit.c(sk.info_ink),
            );
        }
        ui.add_space(t.nav_spacing * 2.0);
        let mut measure = false;
        let mut copy = false;
        let mut keep = false;
        ui.horizontal(|ui| {
            if props.measurement.is_some() {
                keep = self.keep_button(ui, keepable);
                return;
            }
            let m = kit.button(
                ui,
                Button::primary("Measure").enabled(props.measure.is_some()),
            );
            self.side.hits.targets.push(("measure".to_string(), m.rect));
            measure = m.clicked();
            let c = kit.button(ui, Button::secondary("Copy name"));
            self.side
                .hits
                .targets
                .push(("copy name".to_string(), c.rect));
            copy = c.clicked();
        });
        if keep && let Some(m) = &props.measurement {
            self.keep(m);
        }
        ui.add_space(t.nav_spacing * 2.0);
        if measure && let Some((command, params)) = props.measure.clone() {
            self.side_queue(command, params, None);
        }
        if copy {
            ui.ctx().copy_text(props.name.clone());
        }
        if fly {
            self.request_fly(FlyTo::Selection);
        }
        if let Some(id) = follow {
            self.state.selected_feature = Some(id.clone());
            self.queue_select(json!({"items": [{"kind": "feature", "feature": id}]}));
        }
    }
}

impl Shell {
    /// `name`, or with a number after it when a check of the document has
    /// that name already.
    fn free_check_name(&self, name: &str) -> String {
        let taken = |n: &str| self.assertion_rows().iter().any(|a| a.name == n);
        if !taken(name) {
            return name.to_string();
        }
        (2..1000)
            .map(|k| format!("{name} {k}"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| format!("{name} {}", self.assertion_rows().len() + 1))
    }

    /// What `ui.get` says under `measurement`: the relation of the two
    /// things selected, as the foot of the side panel and the view show
    /// it, or the measure the view shows of the one selected; `null`
    /// without either.
    pub fn measurement_json(&self) -> Value {
        match (self.measurement(), self.single_measure()) {
            (Some(m), _) => m.json(),
            (None, Some(single)) => single.json(),
            (None, None) => Value::Null,
        }
    }

    /// The relation of the two things selected, when there is one.
    pub fn measurement(&self) -> Option<&Measurement> {
        self.side.props.as_ref()?.measurement.as_ref()
    }

    /// Whether the relation of the two things selected is being measured
    /// still, on its own thread.
    pub fn measuring(&self) -> bool {
        self.measurement().is_some_and(|m| m.measuring)
    }

    /// The measure of the one thing selected the view shows, when it has
    /// one.
    pub fn single_measure(&self) -> Option<&Single> {
        self.side.props.as_ref()?.single.as_ref()
    }
}

/// `text` cut to `room` with an ellipsis, in the font `style` draws it in.
pub(super) fn clip(ui: &egui::Ui, kit: &Kit, text: &str, style: ValueStyle, room: f32) -> String {
    let font = match style {
        ValueStyle::Name => kit.mono_font(),
        _ => kit.font(),
    };
    let fits = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), font.clone(), egui::Color32::BLACK)
            .size()
            .x
            <= room
    };
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // Halving the length kept: few layouts for a long name.
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars.iter().take(mid).collect::<String>() + "…";
        if fits(&candidate) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars.iter().take(lo).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_directions_in_words() {
        assert_eq!(local_name("F23:copy(0)/F10:copy(1)/F6:face(0)"), "face(0)");
        assert_eq!(direction_text(&json!([0.0, 1.0, 0.0])), "Y");
        assert_eq!(direction_text(&json!([0.0, 0.0, -1.0])), "−Z");
        assert_eq!(direction_text(&json!([0.6, 0.8, 0.0])), "(0.6, 0.8, 0)");
        assert_eq!(
            between(
                "F23:between[F23:copy(0)/F3:startCap(e5,e6),F23:copy(0)/F19:face(0)]",
                None,
                "F23"
            ),
            vec![
                "F23:copy(0)/F3:startCap(e5,e6)".to_string(),
                "F23:copy(0)/F19:face(0)".to_string()
            ]
        );
        assert_eq!(volume_text(112_123.08, "mm"), "112.12 cm³");
        assert_eq!(mass_text(0.3027), "302.7 g");
        assert_eq!(
            measured_line(
                "measure.thickness",
                &json!({"thickness": 7.0, "lengthUnit": "mm"})
            ),
            "Thickness 7 mm"
        );
    }

    /// The review of 2026-11-17, round two: a least not proven within the
    /// tolerance says what it is (the range it is proven in, or "about"
    /// without a bound) and is kept as what is proven; proven, the value.
    #[test]
    fn a_least_not_proven_says_so_and_keeps_what_is_proven() {
        let items = [json!("A"), json!("B")];
        let answer = |at_least: Value| {
            json!({"relation": "leastDistance", "value": 0.1, "atLeast": at_least,
                "tolerance": 1e-6, "words": "Least distance", "from": [0, 0, 0], "to": [0.1, 0, 0]})
        };
        let loose = measurement_of(&answer(json!(0.05)), items.clone(), "mm");
        assert_eq!(loose.text, "0.05 – 0.1 mm");
        assert!(!loose.proven());
        let kept = loose.check("Least").unwrap();
        assert_eq!(
            (kept["min"].as_f64(), kept["max"].as_f64()),
            (Some(0.05), Some(0.1))
        );
        assert!(kept.get("eq").is_none(), "{kept}");
        let none = measurement_of(&answer(Value::Null), items.clone(), "mm");
        assert_eq!(none.text, "about 0.1 mm");
        let kept = none.check("Least").unwrap();
        assert_eq!((kept.get("min"), kept["max"].as_f64()), (None, Some(0.1)));
        let tight = measurement_of(&answer(json!(0.1 - 1e-9)), items.clone(), "mm");
        assert_eq!(tight.text, "0.1 mm");
        assert!(tight.proven());
        assert_eq!(tight.check("Least").unwrap()["eq"].as_f64(), Some(0.1));
        // Round three: what rests on samples is "about", whatever its
        // bound, and is kept as that bound and the value found, so the
        // check can fail either way.
        let mut on_samples = answer(json!(0.1 - 1e-9));
        on_samples["sampled"] = json!(true);
        let sampled = measurement_of(&on_samples, items, "mm");
        assert_eq!(sampled.text, "about 0.1 mm");
        assert!(!sampled.proven());
        let kept = sampled.check("Least").unwrap();
        assert_eq!(
            (kept["min"].as_f64(), kept["max"].as_f64()),
            (Some(0.1 - 1e-9), Some(0.1))
        );
    }
}
