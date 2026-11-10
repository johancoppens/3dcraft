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
    let measure_two = second.as_ref().map(|other| {
        (
            "measure.distance",
            json!({"from": first.name, "to": other.name}),
        )
    });
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
            // Measured where the bodies are, or not at all.
            measure: measure_two.filter(|_| bodies_here),
            shape: None,
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
    let measure = measure_two
        .or_else(|| {
            (first.kind == SelectionKind::Face)
                .then(|| ("measure.thickness", json!({"of": first.name})))
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
        t.nav_gap
            + 1.0
            + kit.hit(t.control_height)
            + (props.rows.len() as f32 + measured) * t.label_row_height
            + 4.0 * t.nav_spacing
            + kit.hit(t.control_height)
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
            let title = kit.galley(ui, props.kind, kit.medium_font(ui), sk.text);
            let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
            ui.painter().galley(rect.min, title, kit.c(sk.text));
            let mut subtitle = props.subtitle.clone();
            if props.more > 0 {
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
                    subtitle,
                    kit.font(),
                    kit.c(sk.text_dim),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let g = ui.painter().layout_job(job);
                let (rect, _) = ui.allocate_exact_size(vec2(room, g.size().y), Sense::hover());
                ui.painter().galley(
                    pos2(rect.left(), rect.center().y - g.size().y * 0.5),
                    g,
                    kit.c(sk.text_dim),
                );
            });
        });
        let label_width = (ui.available_width() * 0.32).clamp(56.0, 110.0);
        let mut follow: Option<String> = None;
        for row in &props.rows {
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
            if row.style == PropStyle::Link {
                let link = ui.interact(
                    r.rect,
                    egui::Id::new(("craft3d-side-made-by", &row.value)),
                    Sense::click(),
                );
                self.side.hits.targets.push(("made by".to_string(), r.rect));
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
        ui.horizontal(|ui| {
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
}
