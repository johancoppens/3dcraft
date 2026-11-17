//! The grid a grid surface was made through, drawn like a sketch
//! (docs/design/surfacing.md › *Seeing the grid*): its rows and columns
//! as the sheet's iso-curves through its points, and its points, in the
//! scene; hidden with its feature (`feature.visible`), drawn also once a
//! later feature took the sheet, for a grid from a table and one given
//! point by point; thinned by a stated rule on a large grid, within its
//! caps; not drawn for a grid that failed. `view.render` with `sketches`
//! draws it over the picture, its points on the rendered sheet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use craft3d_engine::{
    MAX_GRID_LINE_POINTS, MAX_GRID_LINES, SURFACE_LIFT, Scene, SceneGrid, SceneQuality,
    SceneSketch, Session, Workspace, grid_step,
};
use craft3d_math::Vec3;
use serde_json::{Value, json};
use std::sync::Arc;

fn session() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

/// A hill of 6 columns of 4 rows of heights in a table, 10 mm apart in x
/// and 8 mm in y, heights in mm: the grid surface `Hill` through them.
fn hill(s: &mut Session) -> Value {
    let columns = json!(["row", "a", "b", "c", "d", "e", "f"]);
    run(
        s,
        "table.set",
        json!({"name": "heights", "columns": columns, "rows": [
            [1, 1.0, 2.0, 3.0, 3.5, 2.0, 1.0],
            [2, 2.0, 4.0, 6.5, 6.0, 3.0, 1.5],
            [3, 1.5, 3.5, 7.0, 5.0, 2.5, 1.0],
            [4, 1.0, 2.0, 3.0, 2.5, 1.5, 0.5]
        ]}),
    );
    run(
        s,
        "feature.surface.grid",
        json!({"table": "heights", "step": ["10 mm", "8 mm"], "factor": "1 mm", "name": "Hill"}),
    )
}

/// The height of the hill's table at row `r`, column `c`, in mm.
fn height(r: usize, c: usize) -> f64 {
    [
        [1.0, 2.0, 3.0, 3.5, 2.0, 1.0],
        [2.0, 4.0, 6.5, 6.0, 3.0, 1.5],
        [1.5, 3.5, 7.0, 5.0, 2.5, 1.0],
        [1.0, 2.0, 3.0, 2.5, 1.5, 0.5],
    ][r][c]
}

fn scene(s: &mut Session) -> Arc<Scene> {
    s.scene_with(SceneQuality::display()).unwrap().unwrap()
}

fn grid_of<'a>(scene: &'a Scene, name: &str) -> Option<&'a SceneSketch> {
    scene
        .sketches
        .iter()
        .find(|sketch| sketch.name == name && sketch.grid.is_some())
}

/// The distance from `p` to the polyline `line`.
fn distance_to(line: &[Vec3], p: Vec3) -> f64 {
    line.windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let ab = b - a;
            let t = if ab.dot(ab) > 0.0 {
                ((p - a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            p.distance(a + ab * t)
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_tables_grid_is_drawn_through_its_points_and_switched_with_its_feature() {
    let mut s = session();
    let made = hill(&mut s);
    let deviation = made["grid"]["deviation"].as_f64().unwrap();
    let shown = scene(&mut s);
    let grid = grid_of(&shown, "Hill").expect("the hill's grid is drawn");
    assert_eq!(
        grid.grid,
        Some(SceneGrid {
            rows: 4,
            columns: 6,
            row_step: 1,
            column_step: 1
        })
    );
    // Every row and every column a line, every point a point.
    let entities: Vec<&str> = grid.polylines.iter().map(|p| p.entity.as_str()).collect();
    assert_eq!(
        entities,
        [
            "row 1", "row 2", "row 3", "row 4", "column 1", "column 2", "column 3", "column 4",
            "column 5", "column 6"
        ]
    );
    assert_eq!(grid.points.len(), 24);
    // Its lines are a guide, not a profile: drawn as construction
    // geometry is.
    assert!(grid.polylines.iter().all(|line| line.construction));
    // The points are the table's, within what the feature reports.
    for r in 0..4 {
        for c in 0..6 {
            let given = Vec3::new(10.0 * c as f64, 8.0 * r as f64, height(r, c));
            let drawn = grid.points[r * 6 + c];
            assert!(
                drawn.distance(given) <= deviation.max(1e-12),
                "point ({r}, {c}): {drawn:?} against {given:?}"
            );
            // Its row's line and its column's line pass through it, within
            // the chords' tolerance.
            for line in [&grid.polylines[r], &grid.polylines[4 + c]] {
                let off = distance_to(&line.points, drawn);
                assert!(
                    off <= shown.tolerance,
                    "{} misses point ({r}, {c}) by {off}",
                    line.entity
                );
            }
        }
    }
    // Each line runs from one border of the grid to the other.
    let row = &grid.polylines[1].points;
    assert!(row.first().unwrap().distance(grid.points[6]) < 1e-9);
    assert!(row.last().unwrap().distance(grid.points[11]) < 1e-9);
    // Lifted against the depth: a grid lies on its sheet, a sketch on its
    // plane.
    assert_eq!(grid.lift(0.5), SURFACE_LIFT * 0.5);

    // Hidden with its feature, shown again with it.
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Hill", "visible": false}),
    );
    assert!(grid_of(&scene(&mut s), "Hill").is_none());
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Hill", "visible": true}),
    );
    assert_eq!(grid_of(&scene(&mut s), "Hill").unwrap(), grid);

    // Cut into a block, the sheet is no body of its own any more; its
    // grid is drawn all the same, and the row of the tree says it draws.
    run(
        &mut s,
        "feature.block",
        json!({"name": "Base", "size": [50, 24, 20], "position": [0, 0, -10]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "Base", "tool": "Hill", "name": "Terrain"}),
    );
    let cut = scene(&mut s);
    assert!(cut.bodies.iter().all(|b| b.name != "Hill"));
    assert_eq!(grid_of(&cut, "Hill").unwrap().points, grid.points);
    let row = cut.features.iter().find(|f| f.name == "Hill").unwrap();
    assert!(row.draws && !row.hidden, "{row:?}");

    // Suppressed: not drawn, as a suppressed sketch is not.
    run(&mut s, "feature.suppress", json!({"feature": "Hill"}));
    assert!(grid_of(&scene(&mut s), "Hill").is_none());
    run(&mut s, "edit.undo", json!({}));
    assert!(grid_of(&scene(&mut s), "Hill").is_some());

    // Its table deleted: the feature fails, and nothing is drawn of it.
    run(
        &mut s,
        "table.delete",
        json!({"name": "heights", "force": true}),
    );
    let failed = scene(&mut s);
    assert!(grid_of(&failed, "Hill").is_none());
    assert!(
        failed
            .features
            .iter()
            .any(|f| f.name == "Hill" && f.error.is_some())
    );
}

#[test]
fn a_grid_given_point_by_point_of_two_by_two_is_drawn_too() {
    let mut s = session();
    run(
        &mut s,
        "feature.surface.grid",
        json!({"points": [[[0, 0, 0], [10, 0, 1]], [[0, 10, 2], [10, 10, 4]]], "name": "Patch"}),
    );
    let shown = scene(&mut s);
    let grid = grid_of(&shown, "Patch").unwrap();
    assert_eq!(
        grid.grid,
        Some(SceneGrid {
            rows: 2,
            columns: 2,
            row_step: 1,
            column_step: 1
        })
    );
    assert_eq!(grid.polylines.len(), 4);
    let corners = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(10.0, 0.0, 1.0),
        Vec3::new(0.0, 10.0, 2.0),
        Vec3::new(10.0, 10.0, 4.0),
    ];
    for (drawn, given) in grid.points.iter().zip(corners) {
        assert!(drawn.distance(given) < 1e-12, "{drawn:?} against {given:?}");
    }
    // A bilinear patch: its borders are straight, a chord each.
    for line in &grid.polylines {
        assert_eq!(line.points.len(), 2, "{}", line.entity);
    }
}

#[test]
fn a_large_grid_is_drawn_every_kth_line_within_its_caps() {
    // The rule: at most 65 lines a side, the first, every k-th and the last.
    assert_eq!(MAX_GRID_LINES, 65);
    for (count, step) in [
        (2, 1),
        (65, 1),
        (66, 2),
        (129, 2),
        (130, 3),
        (300, 5),
        (400, 7),
        (512, 8),
        (1024, 16),
    ] {
        assert_eq!(grid_step(count), step, "{count} points");
        let grid = SceneGrid {
            rows: count,
            columns: 2,
            row_step: step,
            column_step: 1,
        };
        let drawn = grid.drawn_rows();
        assert!(drawn.len() <= MAX_GRID_LINES, "{count}: {}", drawn.len());
        assert_eq!(drawn.first(), Some(&0));
        assert_eq!(drawn.last(), Some(&(count - 1)));
        assert!(drawn.windows(2).all(|w| w[1] > w[0] && w[1] - w[0] <= step));
    }
    // A grid of 200 × 70 points given point by point: a gentle wave.
    let (rows, columns) = (70, 200);
    let points: Vec<Value> = (0..rows)
        .map(|r| {
            Value::Array(
                (0..columns)
                    .map(|c| {
                        let (x, y) = (c as f64, r as f64);
                        json!([x, y, (x / 9.0).sin() + (y / 7.0).cos()])
                    })
                    .collect(),
            )
        })
        .collect();
    let mut s = session();
    let made = run(
        &mut s,
        "feature.surface.grid",
        json!({"points": points, "name": "Wave"}),
    );
    assert_eq!(made["grid"]["rows"], rows, "{made}");
    let shown = scene(&mut s);
    let wave = grid_of(&shown, "Wave").unwrap();
    let grid = wave.grid.unwrap();
    assert!(grid.thinned());
    assert_eq!((grid.row_step, grid.column_step), (2, 4));
    let (drawn_rows, drawn_columns) = (grid.drawn_rows(), grid.drawn_columns());
    assert_eq!((drawn_rows.len(), drawn_columns.len()), (36, 51));
    assert_eq!(wave.polylines.len(), 36 + 51);
    assert_eq!(wave.points.len(), 36 * 51);
    let total: usize = wave.polylines.iter().map(|p| p.points.len()).sum();
    assert!(total <= MAX_GRID_LINE_POINTS, "{total}");
    // The thinned points are the grid's own at the rows and columns drawn.
    let at = |r: usize, c: usize| {
        wave.points[drawn_rows.iter().position(|&k| k == r).unwrap() * 51
            + drawn_columns.iter().position(|&k| k == c).unwrap()]
    };
    for (r, c) in [(0, 0), (68, 196), (69, 199), (2, 4)] {
        let (x, y) = (c as f64, r as f64);
        let given = Vec3::new(x, y, (x / 9.0).sin() + (y / 7.0).cos());
        assert!(at(r, c).distance(given) < 1e-9, "({r}, {c})");
    }
    // ui.get and view.render say so.
    let said = craft3d_engine::grids_json(&shown);
    assert_eq!(
        said,
        json!([{"feature": wave.feature, "name": "Wave", "rows": 70, "columns": 200, "every": [2, 4], "thinned": true}])
    );
}

/// A PNG of `view.render` as its pixels.
fn pixels(result: &Value) -> (usize, Vec<u8>) {
    let text = result["base64"].as_str().unwrap();
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let mut png = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let bits = chunk
            .iter()
            .filter(|&&c| c != b'=')
            .fold(0_u32, |acc, &c| (acc << 6) | value(c))
            << (6 * pad);
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        png.extend_from_slice(&bytes[..3 - pad]);
    }
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgb).unwrap();
    rgb.truncate(info.buffer_size());
    (info.width as usize, rgb)
}

#[test]
fn view_render_draws_the_grid_with_its_points_on_the_rendered_sheet() {
    let mut s = session();
    hill(&mut s);
    let points = grid_of(&scene(&mut s), "Hill").unwrap().points.clone();
    // Seen from above at a slant, every point in sight (lower, the
    // hill's top hides the middle of the last row), on the CPU (the
    // oracle).
    let (eye, target) = ([25.0, -20.0, 80.0], [25.0, 12.0, 3.0]);
    let camera = json!({"eye": eye, "target": target, "up": [0, 0, 1], "fovY": 40});
    let (width, height) = (640_u32, 480_u32);
    let picture = |s: &mut Session, more: Value| {
        let mut params = json!({"width": width, "height": height, "camera": camera, "renderer": "cpu", "background": "dark"});
        params
            .as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        run(s, "view.render", params)
    };
    let plain = picture(&mut s, json!({}));
    let drawn = picture(&mut s, json!({"sketches": true}));
    assert!(plain.get("grids").is_none());
    assert_eq!(drawn["sketches"], 1, "{drawn}");
    assert_eq!(
        drawn["grids"],
        json!([{"feature": "F1", "name": "Hill", "rows": 4, "columns": 6, "every": [1, 1], "thinned": false}])
    );
    let tolerance = drawn["tolerance"].as_f64().unwrap();
    let (w, rgb) = pixels(&drawn);
    let (_, before) = pixels(&plain);
    let view = craft3d_render::Viewpoint::new(
        &craft3d_render::Camera {
            eye: Vec3::new(eye[0], eye[1], eye[2]),
            target: Vec3::new(target[0], target[1], target[2]),
            up: Vec3::Z,
            projection: craft3d_render::Projection::Perspective { fov_y_deg: 40.0 },
        },
        width,
        height,
    )
    .unwrap();
    for (k, point) in points.iter().enumerate() {
        let [x, y, depth] = view.project(*point).unwrap();
        // The point is drawn, in the sketches' colour, where the plain
        // picture shows the sheet: in sight, not behind it.
        let i = 3 * (y as usize * w + x as usize);
        assert_eq!(
            rgb[i..i + 3],
            craft3d_engine::SKETCH_COLOR,
            "point {k} at ({x:.1}, {y:.1})"
        );
        assert_ne!(before[i..i + 3], rgb[i..i + 3]);
        // And the sheet as rendered passes through it: the mesh under the
        // point, along the ray through it, is within the lift of it (a
        // point inside the grid; a ray through a point of the border
        // grazes the sheet's edge).
        let (r, c) = (k / 6, k % 6);
        if !(1..3).contains(&r) || !(1..5).contains(&c) {
            continue;
        }
        let hit = run(
            &mut s,
            "view.pick",
            json!({"x": x, "y": y, "width": width, "height": height, "camera": camera, "radius": 0}),
        );
        assert_eq!(hit["feature"], "F1", "{hit}");
        assert_eq!(hit["face"], "F1:face(0)", "{hit}");
        let on = hit["depth"].as_f64().unwrap();
        assert!(
            (on - depth).abs() <= SURFACE_LIFT * tolerance,
            "point {k}: {depth} against the sheet's {on}, tolerance {tolerance}"
        );
    }
    // Not over a picture of face ids, which is read by machine.
    let ids = picture(&mut s, json!({"sketches": true, "mode": "ids"}));
    let plain_ids = picture(&mut s, json!({"mode": "ids"}));
    assert_eq!(ids["base64"], plain_ids["base64"]);
    assert!(ids.get("grids").is_none());
    // A wrong type is refused.
    let refused = s
        .execute("view.render", &json!({"sketches": "yes"}))
        .unwrap_err();
    assert!(refused.message().contains("sketches"), "{refused:?}");
}
