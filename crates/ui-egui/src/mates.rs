//! An assembly's mate markers in the viewport (docs/design/assemblies.md ›
//! *Mate markers*): per mate a small glyph at its frame, its axis an arrow
//! along the frame's z, and a sign of its kind around it (an arc about the
//! axis for a revolute, a double arrow along it for a slider, both for a
//! cylindrical, a square in its plane for a planar, a crossed box for a
//! fixed mate, three circles for a ball).
//!
//! The glyphs are segments in the world, sized for a camera so that they
//! keep their size on screen at the camera's target (as the world axes
//! do): both paths of the viewport draw them as the lines of
//! [`crate::viewport::Marks`], tested against the bodies like a datum, so
//! the GPU's picture is compared with the CPU oracle's like any line.

use crate::camera::Projector;
use crate::state::CameraState;
use craft3d_engine::OutlineMate;
use craft3d_math::{Portable, Vec3};

/// Pieces of a whole circle.
const CIRCLE: usize = 24;

/// Most mates drawn: a viewer's markers, not a report.
pub const MAX_MARKERS: usize = 2_000;

/// One mate's marker: its name and its segments in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub name: String,
    pub lines: Vec<[Vec3; 2]>,
}

/// The length of a marker's axis in the world for `camera` on an image
/// `height` pixels high, drawn `scale` pixels per point: `size` points at
/// the camera's target. `None` for a camera that looks nowhere.
pub fn world_size(
    camera: &CameraState,
    width: u32,
    height: u32,
    scale: f32,
    size: f32,
) -> Option<f64> {
    let projector = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))?;
    let length = f64::from(size) * f64::from(scale) * projector.footprint(camera.distance);
    (length.is_finite() && length > 0.0).then_some(length)
}

/// The markers of `mates` placed (those with a frame), `length` long.
pub fn markers<'a>(mates: impl IntoIterator<Item = &'a OutlineMate>, length: f64) -> Vec<Marker> {
    mates
        .into_iter()
        .take(MAX_MARKERS)
        .filter_map(|mate| {
            let frame = mate.frame?;
            let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
            let lines = glyph(mate.kind, v(frame.origin), v(frame.x), v(frame.z), length);
            (!lines.is_empty()).then(|| Marker {
                name: mate.name.clone(),
                lines,
            })
        })
        .collect()
}

/// The segments of the glyph of a mate of `kind` at `origin`, its axis
/// `z` and its x axis `x`, `length` long. Nothing when the axes are not a
/// square pair of directions or a number is not finite: a marker is never
/// guessed.
pub fn glyph(kind: &str, origin: Vec3, x: Vec3, z: Vec3, length: f64) -> Vec<[Vec3; 2]> {
    let (Some(z), Some(x)) = (z.normalized(), x.normalized()) else {
        return Vec::new();
    };
    let Some(y) = z.cross(x).normalized() else {
        return Vec::new();
    };
    let usable = origin.is_finite()
        && x.dot(z).abs() < 1e-6
        && length.is_finite()
        && length > 0.0
        && (origin + z * length).is_finite();
    if !usable {
        return Vec::new();
    }
    let l = length;
    let mut out = Vec::new();
    // A small cross at the origin, in the frame's plane.
    let tick = 0.12 * l;
    out.push([origin - x * tick, origin + x * tick]);
    out.push([origin - y * tick, origin + y * tick]);
    // An arrow's head at `tip` pointing along `along`, opening along `side`.
    let head = |out: &mut Vec<[Vec3; 2]>, tip: Vec3, along: Vec3, side: Vec3, size: f64| {
        out.push([tip, tip - along * size + side * (size * 0.45)]);
        out.push([tip, tip - along * size - side * (size * 0.45)]);
    };
    // A circle (or an arc) about `centre` in the plane of `u` and `v`, from
    // `from` to `to` radians, as pieces of the whole circle's size.
    let arc =
        |out: &mut Vec<[Vec3; 2]>, centre: Vec3, u: Vec3, v: Vec3, r: f64, from: f64, to: f64| {
            let sweep = to - from;
            let pieces = ((sweep.abs() / std::f64::consts::TAU) * CIRCLE as f64)
                .ceil()
                .max(1.0) as usize;
            let at = |t: f64| {
                let (s, c) = t.psin_cos();
                centre + u * (r * c) + v * (r * s)
            };
            for k in 0..pieces {
                let t0 = from + sweep * k as f64 / pieces as f64;
                let t1 = from + sweep * (k + 1) as f64 / pieces as f64;
                out.push([at(t0), at(t1)]);
            }
        };
    let axis = |out: &mut Vec<[Vec3; 2]>, from: f64| {
        out.push([origin + z * from, origin + z * l]);
        head(out, origin + z * l, z, x, 0.2 * l);
    };
    let turn = |out: &mut Vec<[Vec3; 2]>| {
        // Three quarters of a turn about the axis, with its head.
        let (r, h) = (0.32 * l, 0.45 * l);
        let centre = origin + z * h;
        let (from, to) = (0.0, 1.5 * std::f64::consts::PI);
        arc(out, centre, x, y, r, from, to);
        let (s, c) = to.psin_cos();
        let end = centre + x * (r * c) + y * (r * s);
        let tangent = y * c - x * s;
        head(out, end, tangent, z, 0.14 * l);
    };
    let both_ways = |out: &mut Vec<[Vec3; 2]>| {
        // The axis goes on below the origin, with a head there too.
        axis(out, -0.55 * l);
        head(out, origin - z * (0.55 * l), -z, x, 0.2 * l);
    };
    match kind {
        "revolute" => {
            axis(&mut out, 0.0);
            turn(&mut out);
        }
        "slider" => both_ways(&mut out),
        "cylindrical" => {
            both_ways(&mut out);
            turn(&mut out);
        }
        "planar" => {
            axis(&mut out, 0.0);
            let h = 0.38 * l;
            let corners = [
                origin + x * h + y * h,
                origin - x * h + y * h,
                origin - x * h - y * h,
                origin + x * h - y * h,
            ];
            for k in 0..4 {
                if let (Some(&a), Some(&b)) = (corners.get(k), corners.get((k + 1) % 4)) {
                    out.push([a, b]);
                }
            }
        }
        "fixed" => {
            axis(&mut out, 0.0);
            let h = 0.22 * l;
            let corners = [
                origin + x * h + y * h,
                origin - x * h + y * h,
                origin - x * h - y * h,
                origin + x * h - y * h,
            ];
            for k in 0..4 {
                if let (Some(&a), Some(&b)) = (corners.get(k), corners.get((k + 1) % 4)) {
                    out.push([a, b]);
                }
            }
            let [a, b, c, d] = corners;
            out.push([a, c]);
            out.push([b, d]);
        }
        "ball" => {
            let r = 0.3 * l;
            let tau = std::f64::consts::TAU;
            arc(&mut out, origin, x, y, r, 0.0, tau);
            arc(&mut out, origin, y, z, r, 0.0, tau);
            arc(&mut out, origin, z, x, r, 0.0, tau);
        }
        _ => axis(&mut out, 0.0),
    }
    out
}

/// The distance in pixels from `(x, y)` to the nearest segment of
/// `marker` as `projector` shows it; `None` when none of it is in front of
/// the camera.
pub fn distance_on_screen(marker: &Marker, projector: &Projector, x: f64, y: f64) -> Option<f64> {
    marker
        .lines
        .iter()
        .filter_map(|&[a, b]| {
            let (a, b) = projector.segment(a, b)?;
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (((x - a.x) * dx + (y - a.y) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (a.x + dx * t - x, a.y + dy * t - y);
            let d = px.phypot(py);
            d.is_finite().then_some(d)
        })
        .min_by(f64::total_cmp)
}

/// The marker nearest `(x, y)` (pixels of the image `projector` makes)
/// within `reach` pixels, by its name.
pub fn marker_at<'a>(
    markers: &'a [Marker],
    projector: &Projector,
    x: f64,
    y: f64,
    reach: f64,
) -> Option<&'a str> {
    markers
        .iter()
        .filter_map(|m| Some((m, distance_on_screen(m, projector, x, y)?)))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(m, _)| m.name.as_str())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// Every kind's glyph lies within its length of the origin, along the
    /// axis given; a revolute's arc is about the axis.
    #[test]
    fn a_glyph_stays_by_its_frame() {
        let origin = Vec3::new(10.0, -4.0, 2.0);
        let (x, z) = (Vec3::Y, Vec3::X);
        for kind in [
            "revolute",
            "slider",
            "cylindrical",
            "planar",
            "fixed",
            "ball",
            "other",
        ] {
            let lines = glyph(kind, origin, x, z, 5.0);
            assert!(!lines.is_empty(), "{kind}");
            for [a, b] in &lines {
                for p in [a, b] {
                    assert!((*p - origin).length() <= 5.0 + 1e-9, "{kind}: {p:?}");
                }
            }
        }
        // The revolute's axis runs from the origin along z.
        let lines = glyph("revolute", origin, x, z, 5.0);
        assert!(
            lines
                .iter()
                .any(|[a, b]| *a == origin && (*b - (origin + z * 5.0)).length() < 1e-12)
        );
        // A slider's runs both ways.
        let lines = glyph("slider", origin, x, z, 5.0);
        let lowest = lines
            .iter()
            .flat_map(|[a, b]| [*a, *b])
            .map(|p| (p - origin).dot(z))
            .fold(f64::INFINITY, f64::min);
        assert!(lowest < -2.0, "{lowest}");
    }

    /// Hostile frames give no marker: a zero or not-finite axis, axes not
    /// square, a length that is none.
    #[test]
    fn a_hostile_frame_gives_no_marker() {
        let o = Vec3::ZERO;
        assert!(glyph("revolute", o, Vec3::X, Vec3::ZERO, 1.0).is_empty());
        assert!(glyph("revolute", o, Vec3::ZERO, Vec3::Z, 1.0).is_empty());
        assert!(glyph("revolute", o, Vec3::X, Vec3::X, 1.0).is_empty());
        assert!(glyph("revolute", o, Vec3::X, Vec3::new(f64::NAN, 0.0, 1.0), 1.0).is_empty());
        assert!(
            glyph(
                "revolute",
                Vec3::new(f64::INFINITY, 0.0, 0.0),
                Vec3::X,
                Vec3::Z,
                1.0
            )
            .is_empty()
        );
        for l in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(glyph("ball", o, Vec3::X, Vec3::Z, l).is_empty(), "{l}");
        }
        assert!(
            glyph(
                "slider",
                Vec3::new(f64::MAX, 0.0, 0.0),
                Vec3::Y,
                Vec3::X,
                f64::MAX
            )
            .is_empty()
        );
    }
}
