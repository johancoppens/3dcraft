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
//! persistent name (the name an agent uses), with *Wall thickness* (the
//! thinnest wall behind a face, or of a body) and *Copy name*.
//!
//! The card opens with the number the view shows (the UX round of
//! 2026-11-18, V4): for one thing its length, diameter, radius, area,
//! position or volume ([`Head`]), for two their relation; every number
//! written by `shell/shown.rs`, so the view and the card say one text.
//!
//! What of this needs the bodies ([`Facts`], the relation, the wall) is
//! read where they are: here in the window; beside a host that builds
//! them elsewhere (the browser's worker), by the card's own job there
//! ([`CARD_JOB`], [`card_job`]), the card saying *Measuring…* until it
//! comes, and *Not measured* with why where nothing came (0.2.1).

use super::{Shell, grouped};
use crate::kit::{Button, Glyph, Kit, ValueStyle};
use crate::shell::hints::Timed;
use crate::shell::shown;
use crate::shell::views::FlyTo;
use craft3d_engine::history_view::number;
use craft3d_engine::{Scene, SelectionKind, Session};
use craft3d_math::{Bbox3, Portable, Vec3};
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
    /// The card's heading: `Face of BoltHole`, `Edge of Bar`, `Body
    /// Base[2]`.
    pub heading: String,
    /// The number at the head of the card for one thing, and the words
    /// under it; for three things or more only the words.
    pub head: Option<Head>,
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

/// The number that heads the card of one thing (V4): what the view shows
/// beside it where it shows one (a length, a diameter, a radius), else
/// its area, its position, its volume or its box; and the words under
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// `length`, `diameter`, `radius`, `area`, `position`, `volume`,
    /// `box`; `none` where three things or more give no number;
    /// `measuring` while its measures are read where the bodies are (the
    /// browser's worker), `notMeasured` where none came from there.
    pub measure: &'static str,
    /// In the document's unit (squared, cubed), where it is one number.
    pub value: Option<f64>,
    /// As shown and copied (`Ø 8 mm`); empty for none.
    pub text: String,
    /// The value whole, for the hover where `text` shortened it.
    pub whole: String,
    /// What it is: `Diameter`, `Area`, `Volume · 108 g`.
    pub words: String,
    /// Written smaller than a number (a position, a box: three numbers).
    pub small: bool,
    /// Why it was not measured (`notMeasured`): the hover of its words.
    pub why: Option<String>,
}

impl Head {
    fn of(measure: &'static str, value: Option<f64>, text: String, words: &str) -> Self {
        Head {
            measure,
            value,
            whole: text.clone(),
            text,
            words: words.to_string(),
            small: false,
            why: None,
        }
    }

    /// Its measures being read where the bodies are.
    fn measuring() -> Self {
        Head::of("measuring", None, "…".to_string(), "Measuring…")
    }

    /// No measures came from where the bodies are: `why`, for the hover.
    fn not_measured(why: String) -> Self {
        Head {
            why: Some(why),
            ..Head::of("notMeasured", None, "—".to_string(), "Not measured")
        }
    }

    /// A number a click copies: not the marks of one being measured or
    /// not measured.
    pub fn copies(&self) -> bool {
        !matches!(self.measure, "measuring" | "notMeasured")
    }

    /// What `ui.get` says under `measurement` for one thing.
    pub fn json(&self) -> Value {
        json!({
            "relation": null,
            "measure": self.measure,
            "value": self.value,
            "text": if self.text.is_empty() { Value::Null } else { self.text.clone().into() },
            // No number (three things or more), no whole value.
            "whole": if self.text.is_empty() { Value::Null } else { self.whole.clone().into() },
            "words": self.words,
        })
    }
}

/// The thinnest wall behind the face selected, or of the body
/// (`measure.thickness`, *Wall thickness*): it takes the card's head and
/// the view draws it, until the selection or the model changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub measuring: bool,
    pub value: Option<f64>,
    /// `2 mm`, `…` while it is measured, `—` where none was found.
    pub text: String,
    pub whole: String,
    pub words: String,
    /// Why none was found: the command's own words, for the hover.
    pub refused: Option<String>,
    /// Where the wall is measured from and to.
    pub from: Option<[f64; 3]>,
    pub to: Option<[f64; 3]>,
}

impl Wall {
    /// Being measured.
    pub fn measuring() -> Self {
        Wall {
            measuring: true,
            value: None,
            text: "…".to_string(),
            whole: "…".to_string(),
            words: "Measuring the wall…".to_string(),
            refused: None,
            from: None,
            to: None,
        }
    }

    /// The answer of `measure.thickness`, for a face (`body` false) or a
    /// body, in the document's `tolerance`.
    pub fn answered(result: Result<&Value, String>, body: bool, tolerance: f64) -> Self {
        let place = |v: &Value, key: &str| triple(v.get(key)?.get("point")?);
        match result {
            Ok(result) => {
                let Some(value) = result
                    .get("thickness")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
                else {
                    return Self::answered(Err("no single value".to_string()), body, tolerance);
                };
                let unit = result
                    .get("lengthUnit")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let shown = shown::length(value, tolerance, unit);
                Wall {
                    measuring: false,
                    value: Some(value),
                    text: shown.text,
                    whole: shown.whole,
                    words: if body {
                        "Thinnest wall of this part"
                    } else {
                        "Thinnest wall behind this face"
                    }
                    .to_string(),
                    refused: None,
                    from: place(result, "at"),
                    to: place(result, "to"),
                }
            }
            Err(why) => Wall {
                measuring: false,
                value: None,
                text: "—".to_string(),
                whole: "—".to_string(),
                words: if body {
                    "No wall found in this part"
                } else {
                    "No wall found behind this face"
                }
                .to_string(),
                refused: Some(
                    why.strip_prefix("measure.thickness: ")
                        .unwrap_or(&why)
                        .to_string(),
                ),
                from: None,
                to: None,
            },
        }
    }

    /// Asked where the bodies are, and no answer came from there: `why`,
    /// for the hover.
    pub fn not_measured(why: String) -> Self {
        Wall {
            measuring: false,
            value: None,
            text: "—".to_string(),
            whole: "—".to_string(),
            words: "Not measured".to_string(),
            refused: Some(why),
            from: None,
            to: None,
        }
    }

    /// What `ui.get` says under `measurement` while it heads the card.
    pub fn json(&self) -> Value {
        json!({
            "relation": null,
            "measure": "thickness",
            "measuring": self.measuring,
            "value": self.value,
            "text": self.text,
            "whole": self.whole,
            "words": self.words,
            "refused": self.refused,
            "from": self.from,
            "to": self.to,
        })
    }
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
    /// The value whole in the same words (`Ø 8 mm`, `R 4.513274 mm`): the
    /// same as `text` where nothing was shortened.
    pub whole: String,
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
            "whole": self.whole,
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
    // A length's text and its whole value, each with the same sign before
    // it (`Ø`, `R`): the hover says it in the card's own words (the review
    // of the UX round, 2026-11-18).
    let marked = |mark: &str, v: f64| {
        let said = shown::length(v, tolerance, unit);
        (
            format!("{mark}{}", said.text),
            format!("{mark}{}", said.whole),
        )
    };
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
                    let (measure, value, (text, said_whole)) = if whole {
                        ("diameter", 2.0 * r, marked("Ø ", 2.0 * r))
                    } else {
                        ("radius", r, marked("R ", r))
                    };
                    Some(Single {
                        measure,
                        value,
                        text,
                        whole: said_whole,
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
                    let (text, whole) = marked("", length);
                    Some(Single {
                        measure: "length",
                        value: length,
                        text,
                        whole,
                        points: ends,
                        round: None,
                    })
                }
                _ => {
                    let length = length?;
                    let (text, whole) = marked("", length);
                    Some(Single {
                        measure: "length",
                        value: length,
                        text,
                        whole,
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
                let (text, whole) = marked("R ", r);
                return Some(Single {
                    measure: "radius",
                    value: r,
                    text,
                    whole,
                    points: vec![[rim.x, rim.y, rim.z]],
                    round: Some(([at.x, at.y, at.z], [a.x, a.y, a.z], r)),
                });
            }
            // A whole hole or boss: across its rim in view, where it opens
            // (`opening`, as two holes' line is drawn; the review of
            // 2026-11-17: at mid-depth it floated on the top face below
            // the opening); by the middle where it opens nowhere in view.
            let rim = item
                .get("opening")
                .and_then(triple)
                .map(|[x, y, z]| Vec3::new(x, y, z))
                .filter(|p| p.is_finite())
                .unwrap_or(at);
            let (text, whole) = marked("Ø ", 2.0 * r);
            Some(Single {
                measure: "diameter",
                value: 2.0 * r,
                text,
                whole,
                points: Vec::new(),
                round: Some(([rim.x, rim.y, rim.z], [a.x, a.y, a.z], r)),
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
    /// The number as shown and copied: `48 mm`, `135°`, `25.981 mm`.
    pub text: String,
    /// The number whole, to the tolerance: what its hover says where
    /// `text` shortened it (`25.980762 mm`).
    pub whole: String,
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
            whole: "…".to_string(),
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
            // For people; the command's own sentence is the hover
            // (`refused`).
            Err(why) => self.without_answer(
                "These two cannot be measured against each other",
                why.strip_prefix("measure.between: ")
                    .unwrap_or(&why)
                    .to_string(),
            ),
        }
    }

    /// This, asked where the bodies are, and no answer came from there:
    /// `why`, for the hover.
    pub fn not_measured(&self, why: String) -> Self {
        self.without_answer("Not measured", why)
    }

    /// This settled without a value: `words`, and `why` for their hover.
    fn without_answer(&self, words: &str, why: String) -> Self {
        let mut m = self.clone();
        m.measuring = false;
        // Nothing of an answer shown before (the same two things
        // measured again) stays.
        m.relation = String::new();
        m.value = None;
        m.parts = None;
        m.at_least = None;
        m.answer = Value::Null;
        m.text = "—".to_string();
        m.whole = "—".to_string();
        m.words = words.to_string();
        m.refused = Some(why);
        m
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
            "whole": self.whole,
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
    // Written as every number is (`shell/shown.rs`), the whole value kept
    // for the hover.
    let plain = |text: &str| shown::Shown {
        text: text.to_string(),
        whole: text.to_string(),
    };
    let about = |s: shown::Shown| shown::Shown {
        text: format!("about {}", s.text),
        whole: format!("about {}", s.whole),
    };
    let number = match value {
        Some(v) if angle => shown::angle(v, tolerance.to_degrees()),
        // Not proven within the tolerance: "about" where it rests on
        // samples or has no bound, else the range it is proven in.
        Some(v) if unproven => match at_least.filter(|_| !sampled) {
            Some(l) => shown::length_range(l.max(0.0), v, tolerance, unit),
            None => about(shown::length(v, tolerance, unit)),
        },
        Some(v) => shown::length(v, tolerance, unit),
        None if answer.get("overlap").and_then(Value::as_bool) == Some(true) => plain("Overlap"),
        None => plain("—"),
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
    .map(|parts| shown::length_parts(parts, tolerance, unit).text);
    Measurement {
        relation,
        value,
        text: number.text,
        whole: number.whole,
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

/// The side panel's own job where the bodies are: a host that builds them
/// elsewhere (the browser's worker) hands it over (`Session::hand_over`)
/// and has it run there by [`card_job`]. Not a command: no script or
/// agent can ask for it.
pub const CARD_JOB: &str = "host.card";

/// What the card of what is selected needs of the bodies, read where they
/// are: the session there has the document state and the selection the
/// job was handed over with (`craft3d_engine::run_on_state`). `params` is
/// one of
///
/// - `{facts: {first, faces}}`: [`Facts`] of the selection's first thing
///   (`first` as the host has it, `faces` the edge's two faces by the
///   host's scene), answered `{facts}`;
/// - `{between: params}`: `measure.between`, the relation of two things;
/// - `{wall: params}`: `measure.thickness`, *Wall thickness*;
///
/// each of the last two answered `{ok: result}` or `{error: why}`, the
/// command's own refusal; an error of the job itself means that none was
/// read.
pub fn card_job(
    session: &mut Session,
    params: &Value,
) -> Result<Value, craft3d_engine::CommandError> {
    let answered = |result: Result<Value, craft3d_engine::CommandError>| match result {
        Ok(value) => json!({ "ok": value }),
        Err(e) => json!({ "error": e.message() }),
    };
    if let Some(ask) = params.get("facts") {
        let first = ask.get("first").and_then(item_of).ok_or_else(|| {
            craft3d_engine::CommandError::new("the card's job names no thing selected")
        })?;
        let faces: Vec<String> = ask
            .get("faces")
            .and_then(Value::as_array)
            .map(|faces| {
                faces
                    .iter()
                    .filter_map(Value::as_str)
                    .take(2)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        return Ok(json!({ "facts": Facts::read(session, &first, &faces).json() }));
    }
    if let Some(between) = params.get("between") {
        return Ok(answered(session.execute("measure.between", between)));
    }
    if let Some(wall) = params.get("wall") {
        return Ok(answered(session.execute("measure.thickness", wall)));
    }
    Err(craft3d_engine::CommandError::new(
        "the card's job asks for nothing it knows",
    ))
}

/// An item `{kind, name, feature}` as [`item_json`] writes it.
fn item_of(value: &Value) -> Option<craft3d_engine::SelectionItem> {
    Some(craft3d_engine::SelectionItem {
        kind: SelectionKind::parse(value.get("kind")?.as_str()?)?,
        name: value.get("name")?.as_str()?.to_string(),
        feature: value.get("feature")?.as_str()?.to_string(),
    })
}

/// What the card of the selection's first thing needs of the bodies: the
/// session's answers, read where the bodies are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// A body's `document.measure` (without the inertia).
    pub measure: Option<Value>,
    /// A face's, an edge's or a vertex's `select.get` with context and
    /// measures: the selection's first item.
    pub described: Option<Value>,
    /// The angle between the faces an edge lies between, in radians.
    pub angle: Option<f64>,
}

impl Facts {
    /// Read from `session`, which has the bodies (or builds them), for
    /// the selection's first item `first` (an edge lying between `faces`):
    /// what the window reads.
    fn read(
        session: &mut Session,
        first: &craft3d_engine::SelectionItem,
        faces: &[String],
    ) -> Self {
        if first.kind == SelectionKind::Body {
            return Facts {
                measure: session
                    .execute(
                        "document.measure",
                        &json!({"feature": first.feature, "inertia": false}),
                    )
                    .ok(),
                ..Facts::default()
            };
        }
        let described = session
            .execute("select.get", &json!({"context": true, "measures": true}))
            .ok()
            .and_then(|v| v.get("items").and_then(Value::as_array)?.first().cloned());
        let angle = (first.kind == SelectionKind::Edge)
            .then(|| edge_angle(session, &first.name, &first.feature, faces))
            .flatten();
        Facts {
            measure: None,
            described,
            angle,
        }
    }

    /// As it crosses to the host (every number read back the same `f64`:
    /// `serde_json`'s `float_roundtrip`).
    fn json(&self) -> Value {
        json!({"measure": self.measure, "described": self.described, "angle": self.angle})
    }

    /// The card job's answer `{facts}`, read back.
    fn of(answer: &Value) -> Option<Self> {
        let facts = answer.get("facts")?;
        let given = |key: &str| facts.get(key).filter(|v| !v.is_null()).cloned();
        Some(Facts {
            measure: given("measure"),
            described: given("described"),
            angle: facts.get("angle").and_then(Value::as_f64),
        })
    }

    /// Whether these are the facts of `first`: what was described there
    /// is the thing selected here (a body's measure is asked by its
    /// feature).
    fn of_thing(&self, first: &craft3d_engine::SelectionItem) -> bool {
        first.kind == SelectionKind::Body
            || self
                .described
                .as_ref()
                .and_then(item_of)
                .is_some_and(|d| d.same(first))
    }
}

/// What answered a job of the card handed over ([`CARD_JOB`]): the facts,
/// or why none came from where the bodies are.
pub(crate) fn facts_answered(result: Result<Value, String>) -> Result<Facts, String> {
    let read = result.map_err(|why| plain_why(&why))?;
    Facts::of(&read).ok_or_else(|| "its answer cannot be read".to_string())
}

/// What answered a relation's or a wall's job of the card: the command's
/// own answer, or why none came from where the bodies are.
pub(crate) enum Away {
    Answer(Result<Value, String>),
    Missed(String),
}

impl Away {
    pub(crate) fn of(result: Result<Value, String>) -> Self {
        match result {
            Ok(read) => match (read.get("ok"), read.get("error").and_then(Value::as_str)) {
                (Some(ok), None) => Away::Answer(Ok(ok.clone())),
                (None, Some(error)) => Away::Answer(Err(error.to_string())),
                _ => Away::Missed("its answer cannot be read".to_string()),
            },
            Err(why) => Away::Missed(plain_why(&why)),
        }
    }
}

/// Why a job handed over brought nothing, without the job's own name.
fn plain_why(why: &str) -> String {
    why.strip_prefix(CARD_JOB)
        .and_then(|rest| rest.strip_prefix(": "))
        .unwrap_or(why)
        .to_string()
}

/// What the card says where the facts of the thing it shows came from
/// where the bodies are, but of another thing (the selection there,
/// checked against the document, no longer begins with it).
const ANOTHER_THING: &str =
    "what was measured beside the page is no longer what is selected here; select it again";

/// Where the card's facts that need the bodies come from.
pub(crate) enum Source {
    /// Read here, from the session (which has the bodies, or builds them):
    /// the window.
    Here,
    /// Asked where the bodies are: not come yet (`None`), or come, or why
    /// none came.
    Elsewhere(Option<Result<Facts, String>>),
    /// The bodies are not here and nothing can be asked of where they are
    /// (a host that builds them elsewhere but runs no commands there): not
    /// measured, saying so, and nothing offered that needs them; never
    /// read here, where it would build them all in one frame (the review of
    /// 0.2.1).
    Nowhere,
}

/// Why the card of a host that builds the bodies elsewhere and asks
/// nothing there is not measured ([`Source::Nowhere`]).
pub(crate) const NOWHERE: &str =
    "the model is built elsewhere, and its measures cannot be asked for there";

/// The job that asks where the bodies are for what the card of the
/// selection needs of them (`{facts: {first, faces}}`, [`card_job`]);
/// `None` where it shows nothing that needs them (nothing selected, a
/// feature, a sketch, a datum).
pub(crate) fn facts_job(session: &mut Session, scene: Option<&Scene>) -> Option<Value> {
    let view = session.selection();
    let first = view.items.first()?;
    if !matches!(
        first.kind,
        SelectionKind::Face | SelectionKind::Edge | SelectionKind::Vertex | SelectionKind::Body
    ) {
        return None;
    }
    let faces = if first.kind == SelectionKind::Edge {
        between(&first.name, scene, &first.feature)
    } else {
        Vec::new()
    };
    Some(json!({"facts": {"first": item_json(first), "faces": faces}}))
}

/// The selection's first item described, from the session; `None` for
/// a selection of features (their rows show them). What needs the bodies
/// comes from `source`: read here, or asked where they are; while it has
/// not come, the card says it is being measured, and where none came, why.
/// The relation of two things to be measured goes to `wanted`.
pub(crate) fn read(
    session: &mut Session,
    scene: Option<&Scene>,
    source: Source,
    wanted: &mut Option<Value>,
) -> Option<Props> {
    let view = session.selection();
    let first = view.items.first()?.clone();
    let count = view.items.len();
    let more = count.saturating_sub(1);
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
    // The faces an edge lies between, by its name or the scene.
    let faces = if first.kind == SelectionKind::Edge {
        between(&first.name, scene, &first.feature)
    } else {
        Vec::new()
    };
    // What needs the bodies, and the head the card has while it is asked
    // for where they are, or where none came from there.
    let askable = !matches!(source, Source::Nowhere);
    let (facts, waiting) = match source {
        Source::Nowhere => (
            Facts::default(),
            Some(Head::not_measured(NOWHERE.to_string())),
        ),
        Source::Here => (Facts::read(session, &first, &faces), None),
        Source::Elsewhere(None) => (Facts::default(), Some(Head::measuring())),
        Source::Elsewhere(Some(Ok(facts))) if facts.of_thing(&first) => (facts, None),
        Source::Elsewhere(Some(Ok(_))) => (
            Facts::default(),
            Some(Head::not_measured(ANOTHER_THING.to_string())),
        ),
        Source::Elsewhere(Some(Err(why))) => (Facts::default(), Some(Head::not_measured(why))),
    };
    // Lengths in the facts as everywhere else (`shell/shown.rs`), to the
    // document's tolerance at most.
    let tolerance = session.linear_tolerance().unwrap_or(1e-6);
    let angular = session.angular_tolerance().map_or(0.0, f64::to_degrees);
    // Three things or more give no number: one sentence says why, true
    // for any count (the UX verdict of 2026-11-18); `whole` holds what is
    // selected in a few words, what the short card shows.
    let many = (count > 2).then(|| Head {
        measure: "none",
        value: None,
        text: String::new(),
        whole: format!(
            "{} selected",
            craft3d_engine::selection_summary(&view.items)
        ),
        words: MANY_WORDS.to_string(),
        small: false,
        why: None,
    });
    let mut rows = Vec::new();
    let mut made_by = None;
    // Two things: their relation, by itself (Wall thickness stays for one
    // face's or one body's wall), measured where the bodies are.
    let measurement = match (&second, count) {
        (Some(other), 2) => {
            let things = [thing_words(&first, scene), thing_words(other, scene)];
            // Asked only where it can be: else not measured, saying so.
            let mut asked = None;
            let m = relation(&first, other, &unit, things, &mut asked).map(|mut m| {
                m.title = craft3d_engine::selection_summary(&view.items);
                if !askable {
                    m = m.not_measured(NOWHERE.to_string());
                }
                m
            });
            if askable {
                *wanted = asked;
            }
            m
        }
        _ => None,
    };
    if first.kind == SelectionKind::Body {
        let id = first.feature.clone();
        let name = name_of(&id);
        // Being measured where the bodies are, or not measured there: its
        // box a row below.
        let mut head = waiting;
        if let Some(m) = &facts.measure {
            let mass = m.get("mass").and_then(Value::as_f64).map(mass_text);
            if let Some(v) = m.get("volume").and_then(Value::as_f64) {
                let text = shown::volume(v, &unit);
                // The volume heads the card of one body: no row again.
                if count != 1 {
                    rows.push(plain("Volume", text.clone()));
                }
                let words = match &mass {
                    Some(mass) => format!("Volume · {mass}"),
                    None => "Volume".to_string(),
                };
                head = Some(Head::of("volume", Some(v), text, &words));
            }
            if let Some(mass) = mass {
                rows.push(plain("Mass", mass));
            }
            if let Some(a) = m.get("area").and_then(Value::as_f64) {
                rows.push(plain("Area", shown::area(a, &unit)));
            }
        }
        // The box of this body's own mesh, as the scene places it (an
        // instance of an assembly in the assembly's space): not the box of
        // everything its feature made, which in an assembly is every
        // instance of the part (the UX verdict of 2026-11-18: Base[2] read
        // the assembly's width and height).
        let mut bbox = Bbox3::EMPTY;
        if let Some(scene) = scene {
            scene.items_points(std::slice::from_ref(&first), &mut |p| bbox.add(p));
        }
        if let Some((lo, hi)) = bbox.corners() {
            let d = hi - lo;
            let [x, y, z] = [d.x, d.y, d.z].map(|v| shown::length_number(v, tolerance, &unit));
            let text = if unit.is_empty() {
                format!("{x} × {y} × {z}")
            } else {
                format!("{x} × {y} × {z} {unit}")
            };
            // Its volume not known here (an assembly's part, the bodies
            // built elsewhere): its box heads the card, and no row says it
            // again.
            if head.is_none() && count == 1 {
                head = Some(Head {
                    small: true,
                    ..Head::of("box", None, text, "Size of its box")
                });
            } else {
                rows.push(plain("Box", text));
            }
        }
        made_by = Some(id.clone());
        // Wall thickness of the part, measured where the bodies are.
        let measure = (count == 1 && askable).then(|| ("measure.thickness", json!({"of": id})));
        return Some(Props {
            kind,
            heading: format!("Body {name}"),
            head: many.or(head.filter(|_| count == 1)),
            subtitle: name,
            rows,
            name: first.name.clone(),
            made_by,
            more,
            measure,
            shape: None,
            measurement,
            single: None,
        });
    }
    let described = facts.described;
    // The first thing's measure: the view shows it beside it for one thing
    // alone; its Type row says it in the same words whatever the count (a
    // whole cylinder Ø, part of one R: the UX verdict of 2026-11-18).
    let first_single = described
        .as_ref()
        .and_then(|item| single_of(item, &first, scene, &unit, tolerance));
    let single = first_single.clone().filter(|_| count == 1);
    let local = local_name(&first.name);
    let mut of = name_of(&first.feature);
    let mut head = single.as_ref().map(|single| {
        let words = match (single.measure, single.round.is_some(), single.points.len()) {
            ("diameter", ..) => "Diameter",
            ("radius", ..) => "Radius",
            (_, false, 2) => "Length",
            _ => "Length along the curve",
        };
        Head {
            whole: single.whole.clone(),
            ..Head::of(
                single.measure,
                Some(single.value),
                single.text.clone(),
                words,
            )
        }
    });
    // Being measured where the bodies are, or not measured there (then
    // nothing was described).
    if head.is_none() {
        head = waiting;
    }
    if let Some(item) = &described {
        if let Some(origin) = item.get("origin") {
            let id = origin.get("feature").and_then(Value::as_str).unwrap_or("");
            let name = origin.get("name").and_then(Value::as_str).unwrap_or(id);
            if !id.is_empty() {
                made_by = Some(id.to_string());
                name.clone_into(&mut of);
            }
        }
        if let Some(surface) = item.get("surface") {
            rows.push(plain(
                "Type",
                surface_text(surface, &unit, tolerance, first_single.as_ref()),
            ));
        }
        if let Some(area) = item.get("area").and_then(Value::as_f64) {
            let text = shown::area(area, &unit);
            // The area heads the card of one face that has no other number:
            // then no row says it again (the pictures of the merged round,
            // 2026-11-18).
            if head.is_none() && first.kind == SelectionKind::Face && count == 1 {
                head = Some(Head::of("area", Some(area), text, "Area"));
            } else {
                rows.push(plain("Area", text));
            }
        }
        if let Some(surface) = item.get("surface") {
            if let Some(n) = surface.get("normal") {
                rows.push(plain("Normal", direction_text(n)));
            } else if let Some(axis) = surface.get("axis") {
                let through = surface
                    .get("center")
                    .map(|c| point_text(c, tolerance, &unit))
                    .unwrap_or_default();
                rows.push(plain(
                    "Axis",
                    format!("{}  through {through}", direction_text(axis)),
                ));
            }
        }
        if let Some(curve) = item.get("curve") {
            rows.push(plain(
                "Type",
                curve_text(curve, &unit, tolerance, first_single.as_ref()),
            ));
            if let Some(length) = curve.get("length").and_then(Value::as_f64) {
                rows.push(plain(
                    "Length",
                    shown::length_text(length, tolerance, &unit),
                ));
            }
        }
        if let Some(point) = item.get("point") {
            rows.push(plain("Point", point_text(point, tolerance, &unit)));
            if head.is_none()
                && first.kind == SelectionKind::Vertex
                && let Some(p) = triple(point)
            {
                let parts = shown::length_parts(p, tolerance, &unit);
                head = Some(Head {
                    whole: parts.whole,
                    small: true,
                    ..Head::of("position", None, parts.text, "Position")
                });
            }
        }
    }
    if first.kind == SelectionKind::Edge {
        if let [a, b] = faces.as_slice() {
            rows.push(plain(
                "Between",
                format!("{} · {}", face_words(a, scene), face_words(b, scene)),
            ));
        }
        if let Some(angle) = facts.angle {
            rows.push(plain(
                "Angle",
                shown::angle(angle.to_degrees(), angular).text,
            ));
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
    // Wall thickness, for one face: measured where the bodies are (beside
    // a host that builds them elsewhere, `Shell::apply_side_requests`).
    let measure = (first.kind == SelectionKind::Face && count == 1 && askable).then(|| {
        (
            "measure.thickness",
            json!({"of": {"kind": "face", "where": {"in": first.feature, "name": first.name}}}),
        )
    });
    let shape = described.as_ref().and_then(|item| {
        item.get("surface")
            .or_else(|| item.get("curve"))
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Some(Props {
        kind,
        heading: format!("{kind} of {of}"),
        head: many.or(head.filter(|_| count == 1)),
        subtitle: format!("{of} · {local}"),
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

/// A face's surface in words, its measure as the card's number says it
/// (`single`, the one the view shows): `Cylinder  Ø 8 mm` for a whole
/// cylinder, `Cylinder  R 6 mm` for part of one.
fn surface_text(surface: &Value, unit: &str, tolerance: f64, single: Option<&Single>) -> String {
    // Radii are lengths; a cone's half angle is an angle.
    let r = |key: &str| {
        let v = surface.get(key).and_then(Value::as_f64)?;
        Some(if key == "halfAngle" {
            shown::angle(v, 0.0).text
        } else {
            shown::length_number(v, tolerance, unit)
        })
    };
    // A measure that is missing leaves the surface's word alone, not
    // `R  mm`.
    let with = |word: &str, text: Option<String>| {
        text.map_or_else(|| word.to_string(), |t| format!("{word}  {t}"))
    };
    let length = |v: f64| shown::length_text(v, tolerance, unit);
    match surface.get("type").and_then(Value::as_str).unwrap_or("") {
        "plane" => "Plane".to_string(),
        "cylinder" => match single {
            Some(s) if s.measure == "diameter" => format!("Cylinder  Ø {}", length(s.value)),
            _ => with(
                "Cylinder",
                surface
                    .get("radius")
                    .and_then(Value::as_f64)
                    .map(|v| format!("R {}", length(v))),
            ),
        },
        "cone" => with(
            "Cone",
            r("radius")
                .zip(r("halfAngle"))
                .map(|(rad, a)| format!("R {rad} {unit} · {a}")),
        ),
        "sphere" => with(
            "Sphere",
            surface
                .get("radius")
                .and_then(Value::as_f64)
                .map(|v| format!("R {}", length(v))),
        ),
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

/// An edge's curve in words, its measure as the card's number says it:
/// `Circle  Ø 8 mm` for a whole circle, `Arc  R 6 mm` for part of one.
fn curve_text(curve: &Value, unit: &str, tolerance: f64, single: Option<&Single>) -> String {
    let length = |v: f64| shown::length_text(v, tolerance, unit);
    match curve.get("type").and_then(Value::as_str).unwrap_or("") {
        "line" => "Line".to_string(),
        "circle" => match (single, curve.get("radius").and_then(Value::as_f64)) {
            (Some(s), _) if s.measure == "diameter" => format!("Circle  Ø {}", length(s.value)),
            (Some(s), _) if s.measure == "radius" => format!("Arc  R {}", length(s.value)),
            (_, Some(r)) => format!("Circle  R {}", length(r)),
            (_, None) => "Circle".to_string(),
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

fn point_text(p: &Value, tolerance: f64, unit: &str) -> String {
    let v: Vec<String> = p
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_f64)
                .map(|x| shown::length_number(x, tolerance, unit))
                .collect()
        })
        .unwrap_or_default();
    format!("({})", v.join(", "))
}

/// What the card of three things or more says, and the status bar after
/// what is selected: one sentence, true for any count.
pub const MANY_WORDS: &str = "A measure takes one or two things";

/// How long a click on a number says *Copied* in its hover, in seconds.
pub const COPIED_SECONDS: f32 = 1.5;
/// How long the check just kept is tinted in the Checks, in seconds.
pub const KEPT_TINT_SECONDS: f32 = 2.0;

/// What heads the card now: the relation of two things, else a wall
/// measured, else the one thing's own number.
enum Headline<'a> {
    Relation(&'a Measurement),
    Wall(&'a Wall),
    Thing(&'a Head),
    None,
}

impl Shell {
    /// What is selected, as described at the foot of the Features tab.
    pub fn side_props(&self) -> Option<&Props> {
        self.side.props.as_ref()
    }

    /// The wall measured for the card, while it heads it.
    pub fn side_wall(&self) -> Option<&Wall> {
        self.side.wall.as_ref()
    }

    /// The headline of `props`, with the wall measured for it.
    fn headline<'a>(&'a self, props: &'a Props) -> Headline<'a> {
        match (&props.measurement, &self.side.wall, &props.head) {
            (Some(m), _, _) => Headline::Relation(m),
            (None, Some(wall), _) => Headline::Wall(wall),
            (None, None, Some(head)) => Headline::Thing(head),
            (None, None, None) => Headline::None,
        }
    }

    /// How high the card is drawn: the rule, its heading, its number and
    /// the words under it, a row per property, the buttons.
    pub(super) fn props_height(&self) -> f32 {
        let Some(props) = &self.side.props else {
            return 0.0;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let number = |size: f32| size * 1.5 + t.label_row_height;
        let head = match self.headline(props) {
            // Two things: one large number and a line or two of what it
            // is, in place of the rows.
            Headline::Relation(m) => {
                return t.nav_gap
                    + 1.0
                    + kit.hit(t.control_height)
                    + number(t.measure_font)
                    + f32::from(u8::from(m.parts.is_some())) * t.label_row_height
                    + 4.0 * t.nav_spacing
                    + kit.hit(t.control_height);
            }
            Headline::Wall(_) => number(t.measure_font),
            // The many-things sentence wraps: two lines at most.
            Headline::Thing(head) if head.text.is_empty() => 2.0 * t.label_row_height,
            Headline::Thing(head) if head.small => number(t.measure_font * 2.0 / 3.0),
            Headline::Thing(_) => number(t.measure_font),
            Headline::None => 0.0,
        };
        t.nav_gap
            + 1.0
            + kit.hit(t.control_height)
            + head
            + props.rows.len() as f32 * t.label_row_height
            + 4.0 * t.nav_spacing
            + kit.hit(t.control_height)
    }

    /// A number at the head of the card, `size` points high, fitted to
    /// the room (down to half the size; a range broken after its dash):
    /// a click copies it as shown, and its hover says it whole where it
    /// was shortened, then *Copied …* for a moment (the UX round of
    /// 2026-11-18: one grammar for every number).
    fn number_ui(&mut self, ui: &mut egui::Ui, text: &str, whole: &str, size: f32, copies: bool) {
        let kit = Kit::panel(&self.tokens);
        let sk = kit.skin;
        let room = ui.available_width().max(1.0);
        let lay = |text: &str, size: f32| {
            kit.galley(
                ui,
                text.to_string(),
                egui::FontId::proportional(size),
                sk.text,
            )
        };
        let mut big = lay(text, size);
        if big.size().x > room {
            let smaller = (size * room / big.size().x).max(size * 0.5);
            big = lay(text, smaller);
            if big.size().x > room && text.contains(" – ") {
                big = lay(&text.replacen(" – ", " –\n", 1), smaller);
            }
        }
        let (rect, number) = ui.allocate_exact_size(vec2(room, size * 1.5), Sense::click());
        let at = pos2(rect.left(), rect.center().y - big.size().y * 0.5);
        let ink = if number.hovered() && copies {
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
        if !copies {
            return;
        }
        if number.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let now = ui.input(|i| i.time);
        let copied = match self.side.copied.as_mut().map(|c| (c.live(now, None), c)) {
            Some(((true, left), c)) => {
                if let Some(left) = left {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(left));
                }
                Some(c.text.clone())
            }
            _ => None,
        };
        if copied.is_none() {
            self.side.copied = None;
        }
        let hover = copied.unwrap_or_else(|| {
            if whole == text {
                "Copy".to_string()
            } else {
                format!("{whole} · a click copies {text}")
            }
        });
        if number.on_hover_text(hover).clicked() {
            ui.ctx().copy_text(text.to_string());
            self.side.copied = Some(Timed::seconds(format!("Copied {text}"), COPIED_SECONDS));
        }
    }

    /// Small dim words that wrap to the panel, never cut: the many-things
    /// sentence.
    fn words_wrapped_ui(&self, ui: &mut egui::Ui, text: &str) {
        let kit = Kit::panel(&self.tokens);
        let sk = kit.skin;
        let room = ui.available_width().max(1.0);
        let g = ui
            .painter()
            .layout(text.to_string(), kit.font(), kit.c(sk.text_dim), room);
        let (rect, _) = ui.allocate_exact_size(vec2(room, g.size().y.max(1.0)), Sense::hover());
        ui.painter().galley(rect.min, g, kit.c(sk.text_dim));
    }

    /// A line of small dim words under the number, cut to the panel; its
    /// hover says `hover`, or the whole line where it was cut.
    fn words_ui(&self, ui: &mut egui::Ui, text: &str, hover: Option<&str>) {
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let sk = kit.skin;
        let room = ui.available_width();
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.to_string(),
            kit.font(),
            kit.c(sk.text_dim),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = ui.painter().layout_job(job);
        let cut = g.elided;
        let (rect, line) = ui.allocate_exact_size(vec2(room, t.label_row_height), Sense::hover());
        ui.painter().galley(
            pos2(rect.left(), rect.center().y - g.size().y * 0.5),
            g,
            kit.c(sk.text_dim),
        );
        match hover {
            Some(hover) => {
                line.on_hover_text(hover);
            }
            None if cut => {
                line.on_hover_text(text);
            }
            None => {}
        }
    }

    /// The relation of two things at the foot of the panel: one large
    /// number (a click copies it with its unit), what it is in a few
    /// words (the command's own sentence as their hover where it was
    /// refused), its parts along the axes.
    fn measurement_ui(&mut self, ui: &mut egui::Ui, m: &Measurement) {
        if m.measuring {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let size = self.tokens.measure_font;
        self.number_ui(ui, &m.text, &m.whole, size, m.value.is_some());
        self.words_ui(ui, &m.words, m.refused.as_deref());
        if let Some(parts) = &m.parts {
            self.words_ui(ui, parts, None);
        }
    }

    /// Whether the relation of the two things selected is kept as a check
    /// already: for this document and selection (the check changes the
    /// revision, not that), while the check is there (undone, it can be
    /// kept again: the review of 2026-11-17).
    fn kept_now(&self) -> bool {
        self.keeping_now()
            || self.side.kept.as_ref().is_some_and(|(key, name)| {
                Some(*key) == self.side.kept_key()
                    && self.assertion_rows().iter().any(|a| a.name == *name)
            })
    }

    /// Whether the relation of the two things selected is being kept: its
    /// `assert.add` asked for and not answered yet (waiting behind an edit
    /// made where the bodies are, or made there itself).
    fn keeping_now(&self) -> bool {
        self.side
            .keeping
            .as_ref()
            .is_some_and(|k| Some(k.key) == self.side.kept_key())
    }

    /// Why the document shown takes no new check, in plain words: a Jig in
    /// use, which changes only through its parameters (`assert.add` is
    /// refused there, as every other edit), or a document that holds as
    /// many checks as one may. `None` where it takes one; what else can
    /// refuse a check (its measure, all the checks' text together) is said
    /// in the status bar when it is refused (`Shell::keep_failed`).
    pub fn new_check_refused(&self) -> Option<String> {
        if self.shown_status.as_ref().is_some_and(|s| s.jig_in_use) {
            return Some(
                "A Jig in use takes no new checks: only its parameters change. Open it for editing to keep one"
                    .to_string(),
            );
        }
        (self.assertion_rows().len() >= craft3d_engine::MAX_ASSERTIONS).then(|| {
            format!(
                "The document holds {} checks, as many as one may",
                craft3d_engine::MAX_ASSERTIONS
            )
        })
    }

    /// "Keep as a check" (or that it is kept), quiet: keeping is the rare
    /// act; while it cannot keep, its hover says why. True when it was
    /// clicked.
    fn keep_button(&mut self, ui: &mut egui::Ui, m: &Measurement) -> bool {
        let kit = Kit::panel(&self.tokens);
        let kept = self.kept_now();
        let keepable = m.check("").is_some();
        let refused = self.new_check_refused();
        let k = kit.button(
            ui,
            Button::secondary(if kept {
                "Kept as a check"
            } else {
                "Keep as a check"
            })
            .enabled(keepable && !kept && refused.is_none()),
        );
        self.side.hits.targets.push(("keep".to_string(), k.rect));
        let why = if m.measuring {
            Some("Still measuring".to_string())
        } else if self.keeping_now() {
            // Asked for, not answered yet (made where the bodies are, or
            // waiting behind an edit made there).
            Some("Being kept as a check…".to_string())
        } else if kept {
            Some("Kept: Ctrl+Z takes it back".to_string())
        } else if let Some(refused) = refused {
            Some(refused)
        } else if m.answer.get("overlap").and_then(Value::as_bool) == Some(true) {
            Some("Nothing to keep: the two overlap".to_string())
        } else if !keepable {
            Some("Nothing to keep: not measured".to_string())
        } else {
            None
        };
        let k = match why {
            Some(why) => k.on_hover_text(why),
            None => k.on_hover_text("Keep this measure as a check of the document"),
        };
        k.clicked()
    }

    /// Keeps the relation `m` as a check of the document, under a name of
    /// its own: `assert.add` asked for; once it is made
    /// ([`Shell::kept_done`]) the Checks unfold with its row tinted, and
    /// the status bar says it until the next click, key or command; where
    /// it is refused, the status bar says why ([`Shell::keep_failed`]).
    fn keep(&mut self, m: &Measurement) {
        if self.new_check_refused().is_some() || self.keeping_now() {
            return;
        }
        let name = self.free_check_name(&m.check_name);
        if let (Some(params), Some(key)) = (m.check(&name), self.side.kept_key()) {
            self.side_queue("assert.add", params, None);
            // Queued (the panel takes so many requests at most).
            if self
                .side
                .pending
                .last()
                .is_some_and(|r| r.command == "assert.add")
            {
                self.side.keeping = Some(super::Keeping {
                    key,
                    name,
                    job: None,
                });
            }
        }
    }

    /// The check `name` was kept for the selection of `key`: the Checks
    /// unfold with its row tinted, and the status bar says so until the
    /// next click, key or command.
    pub(super) fn kept_done(&mut self, key: (u64, u64), name: String) {
        self.side.kept = Some((key, name.clone()));
        self.state.side_panel.checks_open = true;
        self.side.checks_asked = true;
        self.side.kept_tint = Some((name.clone(), Timed::seconds("", KEPT_TINT_SECONDS), false));
        self.hint_until_next(format!("Kept as the check {name} · Ctrl+Z takes it back"));
    }

    /// The check asked for was not made: the status bar says why for a
    /// while (it said nothing was kept before it was).
    pub(super) fn keep_failed(&mut self, why: &str) {
        let why = why.strip_prefix("assert.add: ").unwrap_or(why);
        self.hint(
            format!("Not kept as a check: {why}"),
            crate::shell::hints::HINT_SECONDS,
        );
    }

    /// *Wall thickness* (quiet), where it can run, and *Copy name*; by
    /// their short words *Wall* and *Copy* on the short card (`short`),
    /// their hovers the same.
    fn thing_buttons(&mut self, ui: &mut egui::Ui, props: &Props, short: bool) -> (bool, bool) {
        let kit = Kit::panel(&self.tokens);
        let mut measure = false;
        if props.measure.is_some() {
            let m = kit
                .button(
                    ui,
                    Button::secondary(if short { "Wall" } else { "Wall thickness" }),
                )
                .on_hover_text(if props.kind == "Body" {
                    "The thinnest wall of this part"
                } else {
                    "The thinnest wall behind this face"
                });
            self.side.hits.targets.push(("measure".to_string(), m.rect));
            measure = m.clicked();
        }
        let copied = self.side.name_copied.as_mut().is_some_and(|c| {
            let now = ui.input(|i| i.time);
            let (live, left) = c.live(now, None);
            if let Some(left) = left {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(left));
            }
            live
        });
        if !copied {
            self.side.name_copied = None;
        }
        let c = kit.button(
            ui,
            Button::secondary(match (copied, short) {
                (true, _) => "Copied",
                (false, true) => "Copy",
                (false, false) => "Copy name",
            }),
        );
        self.side
            .hits
            .targets
            .push(("copy name".to_string(), c.rect));
        let copy = c.on_hover_text("The lasting name an agent uses").clicked();
        (measure, copy)
    }

    /// What a click on *Wall thickness* or *Copy name* does.
    fn thing_actions(&mut self, ui: &egui::Ui, props: &Props, measure: bool, copy: bool) {
        if measure && let Some((command, params)) = props.measure.clone() {
            self.side.wall = Some(Wall::measuring());
            self.side_queue(command, params, None);
        }
        if copy {
            ui.ctx().copy_text(props.name.clone());
            self.side.name_copied = Some(Timed::seconds("Copied", COPIED_SECONDS));
        }
    }

    /// The card short, where the Features tab has no room for it (a short
    /// window: the review of 2026-11-17 found the relation and Keep out of
    /// reach there): one line of what is selected and its number, the
    /// target that flies to it, and Keep as a check for two things or
    /// Wall (thickness) and Copy for one.
    pub(super) fn props_compact_ui(&mut self, ui: &mut egui::Ui) {
        let Some(props) = self.side.props.clone() else {
            return;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let text = match self.headline(&props) {
            Headline::Relation(m) => format!("{} {}", m.words, m.text),
            Headline::Wall(wall) => format!("{} · {} {}", props.heading, wall.words, wall.text),
            // Being measured where the bodies are, or not measured: said.
            Headline::Thing(head) if !head.copies() => {
                format!("{} · {}", props.heading, head.words)
            }
            Headline::Thing(head) if !head.text.is_empty() => {
                format!("{} · {}", props.heading, head.text)
            }
            // Many things: what is selected; the sentence is the hover's,
            // never cut on this one line.
            Headline::Thing(head) => head.whole.clone(),
            Headline::None => props.heading.clone(),
        };
        if props.measurement.as_ref().is_some_and(|m| m.measuring) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let (mut keep, mut fly, mut measure, mut copy) = (false, false, false, false);
        // One line, as high as it was before its buttons came (a taller
        // foot moved the list under a selected row: views.rs): against its
        // right edge Keep for two things, or Copy and Wall (thickness, where
        // it can run) for one, by their short words (the notes' review of
        // 2026-11-18: they were not drawn at all), and the target that
        // flies to it, laid from the right so that the line is cut to what
        // is left and nothing passes the panel's edge (the review of
        // 2026-11-18: Keep reached 8 points past it).
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(width, kit.hit(t.control_height)),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if let Some(m) = &props.measurement {
                    keep = self.keep_button(ui, m);
                } else {
                    (measure, copy) = self.thing_buttons(ui, &props, true);
                }
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
            },
        );
        if keep && let Some(m) = &props.measurement {
            self.keep(m);
        }
        self.thing_actions(ui, &props, measure, copy);
        if fly {
            self.request_fly(FlyTo::Selection);
        }
    }

    /// How high the short card is: one line.
    pub(super) fn props_compact_height(&self) -> f32 {
        let kit = Kit::panel(&self.tokens);
        kit.hit(kit.t.control_height) + 2.0 * kit.t.nav_spacing
    }

    /// The card of what is selected: its heading, the number the view
    /// shows and what it is, its facts, its buttons.
    pub(super) fn props_ui(&mut self, ui: &mut egui::Ui) {
        let Some(props) = self.side.props.clone() else {
            return;
        };
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let sk = kit.skin;
        kit.divider(ui);
        // The heading: the kind in the selection's copper, of what it is,
        // and the target that flies to it.
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
                .map_or_else(|| props.heading.clone(), |m| m.title.clone());
            // The target against the right edge first, the heading cut
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
                if props.measurement.is_none() && props.more > 0 {
                    let more = kit.galley(ui, format!("+{}", props.more), kit.font(), sk.text_dim);
                    let (rect, _) = ui.allocate_exact_size(more.size(), Sense::hover());
                    ui.painter().galley(rect.min, more, kit.c(sk.text_dim));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let room = ui.available_width().max(0.0);
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        title.clone(),
                        kit.medium_font(ui),
                        kit.c(sk.text),
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                    let g = ui.painter().layout_job(job);
                    let cut = g.elided;
                    let (rect, line) =
                        ui.allocate_exact_size(vec2(room, g.size().y), Sense::hover());
                    ui.painter().galley(
                        pos2(rect.left(), rect.center().y - g.size().y * 0.5),
                        g,
                        kit.c(sk.text),
                    );
                    if cut {
                        line.on_hover_text(title);
                    }
                });
            });
        });
        let label_width = (ui.available_width() * 0.32).clamp(56.0, 110.0);
        let mut follow: Option<String> = None;
        // The number the view shows, and what it is.
        match self.headline(&props) {
            Headline::Relation(m) => {
                let m = m.clone();
                self.measurement_ui(ui, &m);
            }
            Headline::Wall(wall) => {
                let wall = wall.clone();
                if wall.measuring {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(30));
                }
                self.number_ui(
                    ui,
                    &wall.text,
                    &wall.whole,
                    t.measure_font,
                    wall.value.is_some(),
                );
                self.words_ui(ui, &wall.words, wall.refused.as_deref());
            }
            Headline::Thing(head) => {
                let head = head.clone();
                if !head.text.is_empty() {
                    let size = if head.small {
                        t.measure_font * 2.0 / 3.0
                    } else {
                        t.measure_font
                    };
                    self.number_ui(ui, &head.text, &head.whole, size, head.copies());
                }
                if head.text.is_empty() {
                    self.words_wrapped_ui(ui, &head.words);
                } else {
                    self.words_ui(ui, &head.words, head.why.as_deref());
                }
            }
            Headline::None => {}
        }
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
        ui.add_space(t.nav_spacing * 2.0);
        let (mut measure, mut copy, mut keep) = (false, false, false);
        ui.horizontal(|ui| {
            if let Some(m) = &props.measurement {
                keep = self.keep_button(ui, m);
                return;
            }
            (measure, copy) = self.thing_buttons(ui, &props, false);
        });
        if keep && let Some(m) = &props.measurement {
            self.keep(m);
        }
        ui.add_space(t.nav_spacing * 2.0);
        self.thing_actions(ui, &props, measure, copy);
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
        match self.side.props.as_ref().map(|props| self.headline(props)) {
            Some(Headline::Relation(m)) => m.json(),
            Some(Headline::Wall(wall)) => wall.json(),
            Some(Headline::Thing(head)) => head.json(),
            Some(Headline::None) | None => Value::Null,
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
        // The review of 2026-11-18: a tiny face's area is not 0. Areas
        // to four significant digits (`shell/shown.rs`, V5).
        assert_eq!(shown::area(1200.0, "mm"), "1200 mm²");
        assert_eq!(shown::area(314.159_265, "mm"), "314.2 mm²");
        assert_eq!(shown::area(0.000_4, "mm"), "0.0004 mm²");
        assert_eq!(shown::area(0.012_34, "mm"), "0.01234 mm²");
        assert_eq!(shown::area(2.5e-9, "mm"), "0.0000000025 mm²");
        assert_eq!(shown::area(0.0, "mm"), "0 mm²");
        assert_eq!(mass_text(0.3027), "302.7 g");
        // Wall thickness: its answer in the card's head, its line's ends.
        let wall = Wall::answered(
            Ok(&json!({"thickness": 7.0, "lengthUnit": "mm",
                "at": {"point": [0, 0, 0]}, "to": {"point": [7, 0, 0]}})),
            false,
            1e-6,
        );
        assert_eq!(wall.text, "7 mm");
        assert_eq!(wall.words, "Thinnest wall behind this face");
        assert_eq!(wall.to, Some([7.0, 0.0, 0.0]));
        let none = Wall::answered(Err("measure.thickness: no point".to_string()), false, 1e-6);
        assert_eq!(
            (none.text.as_str(), none.refused.as_deref()),
            ("—", Some("no point"))
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
