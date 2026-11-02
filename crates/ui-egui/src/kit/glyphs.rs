//! The kit's line icons, painted from a few points each (no image assets,
//! no icon font): drawn on a 16-unit square, scaled to the place they are
//! given, with strokes `icon_stroke` wide at 16 points (thicker in
//! proportion when larger). One colour each, the caller's: `text_dim` at
//! rest, `text` when active, the chrome's inks on the chrome; a sketch's
//! icon is `icon_sketch`, a datum's `icon_datum` ([`Glyph::feature_ink`]).
//!
//! The documents' icons are the simplified ones of the plan's *middle way*
//! (2026-11-02): a part is a block, an assembly two blocks joined by a pin,
//! a Jig a block above a slider. The app icon, where 3DCraft shows itself,
//! is [`super::brand`].

use crate::shell::icons::{self, Icon};
use crate::theme::{Rgb, Tokens};
use craft3d_math::Portable;
use egui::{Color32, Pos2, Rect, Shape, Stroke, vec2};

/// What an icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    // Documents.
    Part,
    Assembly,
    Jig,
    // Features.
    Sketch,
    Extrude,
    Revolve,
    Loft,
    Sweep,
    Union,
    Subtract,
    Intersect,
    Fillet,
    Chamfer,
    DatumPlane,
    DatumAxis,
    DatumPoint,
    /// Datums folded into a group in place.
    Datums,
    Helix,
    Pattern,
    Mirror,
    Shell,
    Block,
    Cylinder,
    Sphere,
    Cone,
    Torus,
    Thread,
    Draft,
    Transform,
    OffsetFace,
    Thicken,
    Split,
    DeleteFace,
    Import,
    SheetMetal,
    Unfold,
    Developable,
    Surface,
    OffsetProfile,
    /// A feature of a kind without its own icon.
    Feature,
    // Controls.
    Search,
    ChevronDown,
    ChevronRight,
    ChevronUp,
    Close,
    Copy,
    Undo,
    Eye,
    Warning,
    Info,
    Check,
    Plus,
    More,
    Square,
    Menu,
    Folder,
    /// One of the icons of the controls over the viewport
    /// (`shell/icons.rs`), so the kit and the bars share them.
    Nav(Icon),
}

impl Glyph {
    /// Every feature kind's icon, as `FeatureKind::type_name` names it;
    /// a boolean by its operation (`union`, `subtract`, `intersect`).
    pub fn for_feature(kind: &str, op: Option<&str>) -> Glyph {
        match kind {
            "sketch" => Glyph::Sketch,
            "extrude" => Glyph::Extrude,
            "revolve" => Glyph::Revolve,
            "loft" => Glyph::Loft,
            "sweep" => Glyph::Sweep,
            "boolean" => match op {
                Some("subtract" | "difference" | "cut") => Glyph::Subtract,
                Some("intersect" | "intersection" | "common") => Glyph::Intersect,
                _ => Glyph::Union,
            },
            "fillet" => Glyph::Fillet,
            "chamfer" => Glyph::Chamfer,
            "datumPlane" => Glyph::DatumPlane,
            "datumAxis" => Glyph::DatumAxis,
            "datumPoint" => Glyph::DatumPoint,
            "datumHelix" => Glyph::Helix,
            "pattern" => Glyph::Pattern,
            "mirror" => Glyph::Mirror,
            "shell" => Glyph::Shell,
            "block" => Glyph::Block,
            "cylinder" => Glyph::Cylinder,
            "sphere" => Glyph::Sphere,
            "cone" => Glyph::Cone,
            "torus" => Glyph::Torus,
            "thread" => Glyph::Thread,
            "draft" => Glyph::Draft,
            "transform" => Glyph::Transform,
            "offsetFace" | "moveFace" => Glyph::OffsetFace,
            "thicken" => Glyph::Thicken,
            "split" => Glyph::Split,
            "deleteFace" => Glyph::DeleteFace,
            "import" => Glyph::Import,
            "sheetMetal" | "fold" => Glyph::SheetMetal,
            "unfold" => Glyph::Unfold,
            "developable" => Glyph::Developable,
            "surfaceNetwork" | "surfaceTrim" | "surfaceNsided" | "fill" | "plate" => Glyph::Surface,
            "offsetProfile" => Glyph::OffsetProfile,
            _ => Glyph::Feature,
        }
    }

    /// The colour of a feature's icon in a list: a sketch's and a datum's
    /// as the viewport draws them (tinted to read on the ground), the rest
    /// `rest` (the ground's dim ink).
    pub fn feature_ink(self, sketch: Rgb, datum: Rgb, rest: Rgb) -> Rgb {
        match self {
            Glyph::Sketch => sketch,
            Glyph::DatumPlane
            | Glyph::DatumAxis
            | Glyph::DatumPoint
            | Glyph::Datums
            | Glyph::Helix => datum,
            _ => rest,
        }
    }

    /// The feature icons, for the gallery and the tests.
    pub const FEATURES: [Glyph; 37] = [
        Glyph::Sketch,
        Glyph::Extrude,
        Glyph::Revolve,
        Glyph::Loft,
        Glyph::Sweep,
        Glyph::Union,
        Glyph::Subtract,
        Glyph::Intersect,
        Glyph::Fillet,
        Glyph::Chamfer,
        Glyph::DatumPlane,
        Glyph::DatumAxis,
        Glyph::DatumPoint,
        Glyph::Datums,
        Glyph::Helix,
        Glyph::Pattern,
        Glyph::Mirror,
        Glyph::Shell,
        Glyph::Block,
        Glyph::Cylinder,
        Glyph::Sphere,
        Glyph::Cone,
        Glyph::Torus,
        Glyph::Thread,
        Glyph::Draft,
        Glyph::Transform,
        Glyph::OffsetFace,
        Glyph::Thicken,
        Glyph::Split,
        Glyph::DeleteFace,
        Glyph::Import,
        Glyph::SheetMetal,
        Glyph::Unfold,
        Glyph::Developable,
        Glyph::Surface,
        Glyph::OffsetProfile,
        Glyph::Feature,
    ];

    /// The documents' icons.
    pub const DOCUMENTS: [Glyph; 3] = [Glyph::Part, Glyph::Assembly, Glyph::Jig];

    /// The controls' icons.
    pub const CONTROLS: [Glyph; 17] = [
        Glyph::Search,
        Glyph::ChevronDown,
        Glyph::ChevronRight,
        Glyph::ChevronUp,
        Glyph::Close,
        Glyph::Copy,
        Glyph::Undo,
        Glyph::Eye,
        Glyph::Warning,
        Glyph::Info,
        Glyph::Check,
        Glyph::Plus,
        Glyph::More,
        Glyph::Square,
        Glyph::Menu,
        Glyph::Folder,
        Glyph::Nav(Icon::Studio),
    ];
}

/// Paints `glyph` centred in `place` (its shorter side is the icon's), in
/// `ink`.
pub fn paint(painter: &egui::Painter, glyph: Glyph, place: Rect, ink: Color32, tokens: &Tokens) {
    let side = place.width().min(place.height());
    if !(side.is_finite() && side > 0.0) {
        return;
    }
    if let Glyph::Nav(icon) = glyph {
        icons::paint(painter, icon, place, ink, tokens);
        return;
    }
    let scale = side / 16.0;
    let origin = place.center() - vec2(8.0, 8.0) * scale;
    let p = |x: f32, y: f32| -> Pos2 { origin + vec2(x, y) * scale };
    let width = if tokens.icon_stroke.is_finite() {
        tokens.icon_stroke.max(0.5)
    } else {
        1.4
    } * scale.max(1.0);
    let stroke = Stroke::new(width, ink);
    let mut shapes: Vec<Shape> = Vec::new();
    let line = |shapes: &mut Vec<Shape>, points: &[(f32, f32)]| {
        shapes.push(Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let closed = |shapes: &mut Vec<Shape>, points: &[(f32, f32)]| {
        shapes.push(Shape::closed_line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let dot = |shapes: &mut Vec<Shape>, x: f32, y: f32, r: f32| {
        shapes.push(Shape::circle_filled(p(x, y), r * scale, ink));
    };
    let ring = |shapes: &mut Vec<Shape>, x: f32, y: f32, r: f32| {
        shapes.push(Shape::circle_stroke(p(x, y), r * scale, stroke));
    };
    // An elliptic arc about (cx, cy) from angle a0 to a1 (radians, y down).
    let arc = |shapes: &mut Vec<Shape>, c: (f32, f32), r: (f32, f32), a0: f32, a1: f32| {
        let n = 24;
        let points = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                p(c.0 + r.0 * a.pcos(), c.1 + r.1 * a.psin())
            })
            .collect();
        shapes.push(Shape::line(points, stroke));
    };
    let dashed = |shapes: &mut Vec<Shape>, a: (f32, f32), b: (f32, f32)| {
        let (a, b) = (p(a.0, a.1), p(b.0, b.1));
        shapes.extend(Shape::dashed_line(
            &[a, b],
            stroke,
            1.6 * scale,
            1.6 * scale,
        ));
    };
    // A cubic Bézier, sampled.
    let curve = |shapes: &mut Vec<Shape>, q: [(f32, f32); 4]| {
        let n = 24;
        let points = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let u = 1.0 - t;
                let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                p(
                    b0 * q[0].0 + b1 * q[1].0 + b2 * q[2].0 + b3 * q[3].0,
                    b0 * q[0].1 + b1 * q[1].1 + b2 * q[2].1 + b3 * q[3].1,
                )
            })
            .collect();
        shapes.push(Shape::line(points, stroke));
    };
    use std::f32::consts::PI;
    let s = &mut shapes;
    match glyph {
        Glyph::Part => slab(s, &closed, &line, 8.0, 3.5, 6.0, 3.0, 3.0),
        Glyph::Block => slab(s, &closed, &line, 8.0, 2.5, 6.0, 3.0, 6.0),
        Glyph::Assembly => {
            closed(s, &[(2.0, 2.0), (9.0, 2.0), (9.0, 9.0), (2.0, 9.0)]);
            closed(s, &[(7.0, 8.0), (14.0, 8.0), (14.0, 14.5), (7.0, 14.5)]);
            dot(s, 8.0, 8.5, 1.6);
        }
        Glyph::Jig => {
            slab(s, &closed, &line, 8.0, 1.5, 5.0, 2.5, 2.5);
            line(s, &[(2.0, 13.0), (7.2, 13.0)]);
            line(s, &[(10.8, 13.0), (14.0, 13.0)]);
            ring(s, 9.0, 13.0, 1.8);
        }
        Glyph::Sketch => {
            curve(s, [(2.5, 13.5), (4.5, 1.5), (11.5, 14.5), (13.5, 2.5)]);
            dot(s, 2.5, 13.5, 1.5);
            dot(s, 13.5, 2.5, 1.5);
        }
        Glyph::Extrude => {
            line(s, &[(2.5, 13.5), (13.5, 13.5)]);
            line(s, &[(8.0, 11.0), (8.0, 2.5)]);
            line(s, &[(4.8, 5.7), (8.0, 2.5), (11.2, 5.7)]);
        }
        Glyph::Revolve => {
            dashed(s, (8.0, 1.5), (8.0, 14.5));
            arc(s, (8.0, 8.5), (5.5, 2.6), 0.15 * PI, 1.75 * PI);
            let (tx, ty) = (
                8.0 + 5.5 * (1.75 * PI).pcos(),
                8.5 + 2.6 * (1.75 * PI).psin(),
            );
            line(s, &[(tx - 2.6, ty - 1.2), (tx, ty), (tx - 0.6, ty + 2.4)]);
        }
        Glyph::Loft => {
            arc(s, (8.0, 12.5), (5.5, 1.8), 0.0, 2.0 * PI);
            closed(s, &[(5.5, 2.5), (10.5, 2.5), (10.5, 5.0), (5.5, 5.0)]);
            line(s, &[(2.5, 12.5), (5.5, 5.0)]);
            line(s, &[(13.5, 12.5), (10.5, 5.0)]);
        }
        Glyph::Sweep => {
            curve(s, [(4.0, 12.0), (5.0, 4.0), (10.0, 13.0), (14.0, 3.0)]);
            arc(s, (4.0, 12.0), (2.4, 2.4), 0.0, 2.0 * PI);
        }
        Glyph::Union => closed(
            s,
            &[
                (2.0, 2.0),
                (10.0, 2.0),
                (10.0, 6.0),
                (14.0, 6.0),
                (14.0, 14.0),
                (6.0, 14.0),
                (6.0, 10.0),
                (2.0, 10.0),
            ],
        ),
        Glyph::Subtract => {
            line(
                s,
                &[
                    (14.0, 8.0),
                    (14.0, 2.0),
                    (2.0, 2.0),
                    (2.0, 14.0),
                    (8.0, 14.0),
                    (8.0, 8.0),
                    (14.0, 8.0),
                ],
            );
            dashed(s, (14.0, 8.0), (14.0, 14.0));
            dashed(s, (14.0, 14.0), (8.0, 14.0));
        }
        Glyph::Intersect => {
            dashed(s, (2.0, 2.0), (10.0, 2.0));
            dashed(s, (2.0, 2.0), (2.0, 10.0));
            dashed(s, (14.0, 6.0), (14.0, 14.0));
            dashed(s, (6.0, 14.0), (14.0, 14.0));
            closed(s, &[(6.0, 6.0), (10.0, 6.0), (10.0, 10.0), (6.0, 10.0)]);
        }
        Glyph::Fillet => {
            line(s, &[(3.0, 2.0), (3.0, 7.0)]);
            arc(s, (10.0, 7.0), (7.0, 7.0), PI, 0.5 * PI);
            line(s, &[(10.0, 14.0), (14.0, 14.0)]);
        }
        Glyph::Chamfer => line(s, &[(3.0, 2.0), (3.0, 8.5), (8.5, 14.0), (14.0, 14.0)]),
        Glyph::DatumPlane => closed(s, &[(5.5, 3.5), (15.0, 3.5), (10.5, 12.5), (1.0, 12.5)]),
        Glyph::DatumAxis => {
            line(s, &[(2.0, 14.0), (6.5, 9.5)]);
            dot(s, 8.0, 8.0, 0.9);
            line(s, &[(9.5, 6.5), (14.0, 2.0)]);
            line(s, &[(11.0, 2.0), (14.0, 2.0), (14.0, 5.0)]);
        }
        Glyph::DatumPoint => {
            ring(s, 8.0, 8.0, 3.0);
            line(s, &[(8.0, 1.5), (8.0, 4.5)]);
            line(s, &[(8.0, 11.5), (8.0, 14.5)]);
            line(s, &[(1.5, 8.0), (4.5, 8.0)]);
            line(s, &[(11.5, 8.0), (14.5, 8.0)]);
        }
        Glyph::Datums => {
            closed(s, &[(5.5, 4.5), (15.0, 4.5), (10.5, 12.5), (1.0, 12.5)]);
            line(s, &[(8.0, 1.0), (8.0, 15.0)]);
        }
        Glyph::Helix | Glyph::Thread => {
            let top = if glyph == Glyph::Helix { 2.0 } else { 3.0 };
            line(
                s,
                &[
                    (4.0, top),
                    (12.0, top + 2.0),
                    (4.0, top + 4.0),
                    (12.0, top + 6.0),
                    (4.0, top + 8.0),
                    (12.0, top + 10.0),
                ],
            );
            if glyph == Glyph::Thread {
                line(s, &[(2.5, 2.0), (2.5, 14.0)]);
                line(s, &[(13.5, 2.0), (13.5, 14.0)]);
            }
        }
        Glyph::Pattern => {
            for x in [1.0, 6.0, 11.0] {
                closed(s, &[(x, 6.0), (x + 4.0, 6.0), (x + 4.0, 10.0), (x, 10.0)]);
            }
        }
        Glyph::Mirror => {
            closed(s, &[(6.5, 3.0), (6.5, 13.0), (1.5, 13.0)]);
            closed(s, &[(9.5, 3.0), (9.5, 13.0), (14.5, 13.0)]);
            dashed(s, (8.0, 1.0), (8.0, 15.0));
        }
        Glyph::Shell => {
            line(
                s,
                &[
                    (2.0, 3.0),
                    (2.0, 14.0),
                    (14.0, 14.0),
                    (14.0, 3.0),
                    (11.0, 3.0),
                ],
            );
            line(
                s,
                &[
                    (11.0, 3.0),
                    (11.0, 11.0),
                    (5.0, 11.0),
                    (5.0, 3.0),
                    (2.0, 3.0),
                ],
            );
        }
        Glyph::Cylinder => {
            arc(s, (8.0, 4.0), (5.5, 2.0), 0.0, 2.0 * PI);
            line(s, &[(2.5, 4.0), (2.5, 12.0)]);
            line(s, &[(13.5, 4.0), (13.5, 12.0)]);
            arc(s, (8.0, 12.0), (5.5, 2.0), 0.0, PI);
        }
        Glyph::Sphere => {
            ring(s, 8.0, 8.0, 6.0);
            arc(s, (8.0, 8.0), (6.0, 2.0), 0.0, PI);
        }
        Glyph::Cone => {
            line(s, &[(2.5, 12.0), (8.0, 2.0), (13.5, 12.0)]);
            arc(s, (8.0, 12.0), (5.5, 2.0), 0.0, PI);
            dashed(s, (2.5, 12.0), (13.5, 12.0));
        }
        Glyph::Torus => {
            arc(s, (8.0, 8.0), (6.5, 4.0), 0.0, 2.0 * PI);
            arc(s, (8.0, 7.5), (2.8, 1.0), 0.0, PI);
            arc(s, (8.0, 8.5), (2.2, 0.8), PI, 2.0 * PI);
        }
        Glyph::Draft => {
            closed(s, &[(4.5, 3.0), (11.5, 3.0), (14.0, 13.0), (2.0, 13.0)]);
        }
        Glyph::Transform => {
            line(s, &[(8.0, 1.5), (8.0, 14.5)]);
            line(s, &[(1.5, 8.0), (14.5, 8.0)]);
            line(s, &[(6.0, 3.5), (8.0, 1.5), (10.0, 3.5)]);
            line(s, &[(6.0, 12.5), (8.0, 14.5), (10.0, 12.5)]);
            line(s, &[(3.5, 6.0), (1.5, 8.0), (3.5, 10.0)]);
            line(s, &[(12.5, 6.0), (14.5, 8.0), (12.5, 10.0)]);
        }
        Glyph::OffsetFace => {
            line(s, &[(2.0, 13.5), (14.0, 13.5)]);
            dashed(s, (2.0, 4.0), (14.0, 4.0));
            line(s, &[(8.0, 12.0), (8.0, 5.5)]);
            line(s, &[(5.8, 7.7), (8.0, 5.5), (10.2, 7.7)]);
        }
        Glyph::Thicken => {
            curve(s, [(2.0, 11.0), (5.0, 5.0), (11.0, 5.0), (14.0, 11.0)]);
            curve(s, [(2.0, 14.0), (5.0, 8.0), (11.0, 8.0), (14.0, 14.0)]);
            line(s, &[(2.0, 11.0), (2.0, 14.0)]);
            line(s, &[(14.0, 11.0), (14.0, 14.0)]);
        }
        Glyph::Split => {
            closed(s, &[(2.0, 2.0), (14.0, 2.0), (14.0, 14.0), (2.0, 14.0)]);
            line(s, &[(1.0, 11.5), (15.0, 4.5)]);
        }
        Glyph::DeleteFace => {
            closed(s, &[(2.0, 2.0), (14.0, 2.0), (14.0, 14.0), (2.0, 14.0)]);
            line(s, &[(5.5, 5.5), (10.5, 10.5)]);
            line(s, &[(10.5, 5.5), (5.5, 10.5)]);
        }
        Glyph::Import => {
            line(s, &[(2.0, 9.5), (2.0, 14.0), (14.0, 14.0), (14.0, 9.5)]);
            line(s, &[(8.0, 1.5), (8.0, 10.5)]);
            line(s, &[(5.0, 7.5), (8.0, 10.5), (11.0, 7.5)]);
        }
        Glyph::SheetMetal => {
            line(s, &[(2.5, 2.0), (2.5, 11.0)]);
            arc(s, (5.5, 11.0), (3.0, 3.0), PI, 0.5 * PI);
            line(s, &[(5.5, 14.0), (14.5, 14.0)]);
            line(s, &[(5.0, 2.0), (5.0, 11.0), (14.5, 11.0)]);
        }
        Glyph::Unfold => {
            closed(s, &[(1.5, 5.0), (14.5, 5.0), (14.5, 11.0), (1.5, 11.0)]);
            dashed(s, (8.0, 5.0), (8.0, 11.0));
        }
        Glyph::Developable => {
            curve(s, [(2.0, 4.0), (6.0, 1.0), (10.0, 7.0), (14.0, 4.0)]);
            curve(s, [(2.0, 12.0), (6.0, 9.0), (10.0, 15.0), (14.0, 12.0)]);
            line(s, &[(2.0, 4.0), (2.0, 12.0)]);
            line(s, &[(14.0, 4.0), (14.0, 12.0)]);
        }
        Glyph::Surface => {
            curve(s, [(2.0, 5.0), (6.0, 1.5), (10.0, 6.5), (14.0, 3.0)]);
            curve(s, [(2.0, 13.0), (6.0, 9.5), (10.0, 14.5), (14.0, 11.0)]);
            line(s, &[(2.0, 5.0), (2.0, 13.0)]);
            line(s, &[(14.0, 3.0), (14.0, 11.0)]);
            curve(s, [(8.0, 4.0), (8.6, 6.5), (7.4, 9.5), (8.0, 12.2)]);
        }
        Glyph::OffsetProfile => {
            closed(s, &[(1.5, 1.5), (14.5, 1.5), (14.5, 14.5), (1.5, 14.5)]);
            dashed(s, (5.0, 5.0), (11.0, 5.0));
            dashed(s, (11.0, 5.0), (11.0, 11.0));
            dashed(s, (11.0, 11.0), (5.0, 11.0));
            dashed(s, (5.0, 11.0), (5.0, 5.0));
        }
        Glyph::Feature | Glyph::Square => {
            closed(s, &[(3.0, 3.0), (13.0, 3.0), (13.0, 13.0), (3.0, 13.0)]);
        }
        Glyph::Search => {
            ring(s, 6.8, 6.8, 4.6);
            line(s, &[(10.2, 10.2), (14.0, 14.0)]);
        }
        Glyph::ChevronDown => line(s, &[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Glyph::ChevronUp => line(s, &[(4.0, 10.0), (8.0, 6.0), (12.0, 10.0)]),
        Glyph::ChevronRight => line(s, &[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Glyph::Close => {
            line(s, &[(4.0, 4.0), (12.0, 12.0)]);
            line(s, &[(12.0, 4.0), (4.0, 12.0)]);
        }
        Glyph::Copy => {
            closed(s, &[(3.0, 5.5), (10.5, 5.5), (10.5, 14.0), (3.0, 14.0)]);
            line(
                s,
                &[
                    (5.5, 5.5),
                    (5.5, 2.0),
                    (13.0, 2.0),
                    (13.0, 10.5),
                    (10.5, 10.5),
                ],
            );
        }
        Glyph::Undo => {
            arc(s, (8.5, 9.0), (5.0, 4.5), 1.0 * PI, 2.6 * PI);
            line(s, &[(3.5, 5.5), (3.5, 9.0), (7.0, 9.0)]);
        }
        Glyph::Eye => {
            curve(s, [(1.5, 8.0), (4.5, 3.0), (11.5, 3.0), (14.5, 8.0)]);
            curve(s, [(1.5, 8.0), (4.5, 13.0), (11.5, 13.0), (14.5, 8.0)]);
            ring(s, 8.0, 8.0, 2.2);
        }
        Glyph::Warning => {
            closed(s, &[(8.0, 1.8), (14.8, 14.0), (1.2, 14.0)]);
            line(s, &[(8.0, 6.0), (8.0, 9.8)]);
            dot(s, 8.0, 11.9, 0.9);
        }
        Glyph::Info => {
            ring(s, 8.0, 8.0, 6.3);
            line(s, &[(8.0, 7.2), (8.0, 11.5)]);
            dot(s, 8.0, 4.9, 0.9);
        }
        Glyph::Check => line(s, &[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)]),
        Glyph::Plus => {
            line(s, &[(8.0, 3.0), (8.0, 13.0)]);
            line(s, &[(3.0, 8.0), (13.0, 8.0)]);
        }
        Glyph::More => {
            for x in [3.5, 8.0, 12.5] {
                dot(s, x, 8.0, 1.2);
            }
        }
        Glyph::Menu => {
            for y in [4.0, 8.0, 12.0] {
                line(s, &[(2.5, y), (13.5, y)]);
            }
        }
        Glyph::Folder => closed(
            s,
            &[
                (1.5, 3.0),
                (6.0, 3.0),
                (7.5, 5.0),
                (14.5, 5.0),
                (14.5, 13.5),
                (1.5, 13.5),
            ],
        ),
        Glyph::Nav(_) => {}
    }
    painter.extend(shapes);
}

/// Draws a polyline of unit-square points into the shapes.
type Draw<'a> = dyn Fn(&mut Vec<Shape>, &[(f32, f32)]) + 'a;

/// A block seen from a corner: its top (a rhombus about `cx`, its top at
/// `top`, `half` wide each way and `rise` deep) and its sides `height`
/// high.
#[allow(clippy::too_many_arguments)]
fn slab(
    s: &mut Vec<Shape>,
    closed: &Draw<'_>,
    line: &Draw<'_>,
    cx: f32,
    top: f32,
    half: f32,
    rise: f32,
    height: f32,
) {
    let (l, r, mid, low) = (cx - half, cx + half, top + rise, top + 2.0 * rise);
    closed(s, &[(cx, top), (r, mid), (cx, low), (l, mid)]);
    line(
        s,
        &[
            (l, mid),
            (l, mid + height),
            (cx, low + height),
            (r, mid + height),
            (r, mid),
        ],
    );
    line(s, &[(cx, low), (cx, low + height)]);
}

/// A place of `side` points for an icon at `at` (its top-left corner).
pub fn place(at: Pos2, side: f32) -> Rect {
    Rect::from_min_size(at, vec2(side, side))
}

/// Whether every shape stays within `place` (and a stroke's width more):
/// a glyph never draws outside the place it is given.
pub fn bounds_ok(shapes: &[Shape], place: Rect) -> bool {
    shapes.iter().all(|shape| {
        let b = shape.visual_bounding_rect();
        !b.is_positive() || place.expand(2.0).contains_rect(b)
    })
}
