//! The sketches over a picture made without a window (`view.render` with
//! `sketches: true`): the lines and points of every sketch the scene
//! draws, a helix and the grid of a grid surface among them, as the
//! window's Sketches toggle shows them (`ui.set {"show": {"sketches":
//! true}}`). The window draws them itself, on the CPU or the GPU
//! (`craft3d_ui_egui::viewport`, `direct`); here they are inked on the
//! finished picture by the CPU renderer's own line and dot drawers, over
//! a picture of either renderer, as curvature combs are
//! (`craft3d_render::analysis::draw_lines`): one pixel wide, left out (not
//! dimmed) where a body is in front.

use crate::scene::{Scene, SceneSketch};
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::{Camera, Cut, Frame, RenderError};
use serde_json::{Value, json};

/// The colour of a sketch's lines and points in pictures without a
/// window: the window theme's (`Tokens::sketch`).
pub const SKETCH_COLOR: [u8; 3] = [76, 90, 166];

/// The colour of construction geometry there (`Tokens::sketch_construction`).
pub const CONSTRUCTION_COLOR: [u8; 3] = [134, 146, 214];

/// The radius of a sketch's points there, in pixels: the window theme's
/// (`Tokens::point_radius`) at one pixel a point.
pub const POINT_RADIUS: f64 = 2.25;

/// How much further than the surface under it a line or a point may lie
/// and still be drawn, as a share of the diagonal of the model's box: the
/// depth under a pixel is the surface's at the pixel's sample, not at the
/// line's own point, so a line on a sloping face needs room (curvature
/// combs are tested with the same). A grid's lines and points get their
/// lift ([`SceneSketch::lift`]) on top.
const SLACK: f64 = 1e-3;

/// Draws the sketches of `scene` over `frame`, seen with `camera`, framed
/// on `model`; what it drew, for the result: how many sketches, and the
/// grids among them ([`grids_json`]). With a section (`cut`, the plane the
/// bodies were cut with) a grid is drawn on the kept side only, its lines
/// ending at the plane ([`SceneSketch::cut_by_section`]); a sketch and a
/// helix whole.
pub(crate) fn draw(
    frame: &mut Frame,
    camera: &Camera,
    scene: &Scene,
    model: &Bbox3,
    cut: Option<&Cut>,
) -> Result<Value, RenderError> {
    let base = model.diagonal() * SLACK;
    let base = if base.is_finite() && base > 0.0 {
        base
    } else {
        0.0
    };
    for sketch in &scene.sketches {
        let slack = base + sketch.lift(scene.tolerance);
        let cut = cut.filter(|_| sketch.cut_by_section());
        for polyline in &sketch.polylines {
            let color = if polyline.construction {
                CONSTRUCTION_COLOR
            } else {
                SKETCH_COLOR
            };
            match cut {
                None => craft3d_render::analysis::draw_lines(
                    frame,
                    camera,
                    std::slice::from_ref(&polyline.points),
                    color,
                    slack,
                )?,
                Some(cut) => craft3d_render::analysis::draw_lines(
                    frame,
                    camera,
                    &kept_pieces(&polyline.points, cut),
                    color,
                    slack,
                )?,
            }
        }
        match cut {
            None => craft3d_render::analysis::draw_dots(
                frame,
                camera,
                &sketch.points,
                SKETCH_COLOR,
                POINT_RADIUS,
                slack,
            )?,
            Some(cut) => {
                let kept: Vec<Vec3> = sketch
                    .points
                    .iter()
                    .copied()
                    .filter(|p| cut.clip_segment(*p, *p).is_some())
                    .collect();
                craft3d_render::analysis::draw_dots(
                    frame,
                    camera,
                    &kept,
                    SKETCH_COLOR,
                    POINT_RADIUS,
                    slack,
                )?;
            }
        }
    }
    Ok(json!({ "sketches": scene.sketches.len(), "grids": grids_json(scene) }))
}

/// The parts of the polyline `points` on the kept side of `cut`, each a
/// polyline of its own: every segment clipped at the plane
/// (`Cut::clip_segment`, as the window's overlay clips it), the pieces
/// that follow on from each other joined.
fn kept_pieces(points: &[Vec3], cut: &Cut) -> Vec<Vec<Vec3>> {
    let mut out: Vec<Vec<Vec3>> = Vec::new();
    let mut joined = false;
    for pair in points.windows(2) {
        let [a, b] = [pair.first(), pair.get(1)];
        let (Some(&a), Some(&b)) = (a, b) else {
            continue;
        };
        match cut.clip_segment(a, b) {
            Some([from, to]) => {
                match out.last_mut() {
                    Some(piece) if joined && from == a => piece.push(to),
                    _ => out.push(vec![from, to]),
                }
                // The next segment goes on from this one where this one
                // was kept to its end.
                joined = to == b;
            }
            None => joined = false,
        }
    }
    out
}

/// The grids of grid surfaces among the sketches of `scene`, in history
/// order: each `{feature, name, rows, columns, every: [rows, columns],
/// thinned}`, `every` the step at which rows and columns are drawn (1 for
/// all; [`crate::MAX_GRID_LINES`] lines a side at most, the last line
/// drawn either way).
pub fn grids_json(scene: &Scene) -> Value {
    Value::Array(scene.sketches.iter().filter_map(grid_json).collect())
}

fn grid_json(sketch: &SceneSketch) -> Option<Value> {
    let grid = sketch.grid?;
    Some(json!({
        "feature": sketch.feature,
        "name": sketch.name,
        "rows": grid.rows,
        "columns": grid.columns,
        "every": [grid.row_step, grid.column_step],
        "thinned": grid.thinned(),
    }))
}
