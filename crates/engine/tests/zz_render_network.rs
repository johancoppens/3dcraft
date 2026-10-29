//! TEMPORARY: renders a mouse-like body built from curve networks (a dome
//! through three cross sections and three lengthwise curves, and a tail
//! held tangent to it) and prints the PNG (hex) for the evidence. Removed
//! once the picture is looked at.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, new_document, run, session_in, workdir};
use serde_json::json;

#[test]
fn render_network_mouse() {
    let dir = workdir("render", "mouse");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let xs = [0.0, 45.0, 90.0, 120.0];
    let ws = [18.0, 26.0, 16.0, 9.0];
    let hs = [10.0, 22.0, 12.0, 6.0];
    for k in 0..4 {
        let name = format!("Section{k}");
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "yz", "offset": format!("{} mm", xs[k]), "name": name}),
        );
        let (w, h) = (ws[k], hs[k]);
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": name, "through": [[-w, 0.0], [-0.7 * w, 0.75 * h], [0.0, h], [0.7 * w, 0.75 * h], [w, 0.0]]}),
        );
    }
    // Lengthwise: the left and right edges on the ground, the top line.
    for (name, plane, points) in [
        (
            "Left",
            "xy",
            (0..3).map(|k| [xs[k], -ws[k]]).collect::<Vec<_>>(),
        ),
        ("Top", "zx", (0..3).map(|k| [hs[k], xs[k]]).collect()),
        ("Right", "xy", (0..3).map(|k| [xs[k], ws[k]]).collect()),
        ("LeftTail", "xy", (2..4).map(|k| [xs[k], -ws[k]]).collect()),
        ("TopTail", "zx", (2..4).map(|k| [hs[k], xs[k]]).collect()),
        ("RightTail", "xy", (2..4).map(|k| [xs[k], ws[k]]).collect()),
    ] {
        built(
            &mut s,
            "sketch.create",
            json!({"plane": plane, "name": name}),
        );
        if points.len() > 2 {
            run(
                &mut s,
                "sketch.spline",
                json!({"sketch": name, "through": points, "degree": 2}),
            );
        } else {
            run(
                &mut s,
                "sketch.line",
                json!({"sketch": name, "from": points[0], "to": points[1]}),
            );
        }
    }
    let dome = built(
        &mut s,
        "feature.surface.network",
        json!({"u": ["Section0", "Section1", "Section2"], "v": ["Left", "Top", "Right"], "name": "Dome"}),
    );
    println!("DOME {dome}");
    let shell = body(&mut s, "Dome");
    println!("DOME-BODY {shell}");
    let face = shell["faceNames"][0].as_str().unwrap().to_string();
    let seam = shell["edgeNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_string())
        .find(|e| e.contains("endEdge(e0,") || e.contains("endEdge(0,"))
        .unwrap();
    println!("SEAM {seam}");
    let tail = run(
        &mut s,
        "feature.surface.network",
        json!({"u": [{"edges": [seam]}, "Section3"], "v": ["LeftTail", "TopTail", "RightTail"],
               "edges": {"uFirst": {"kind": "tangent", "face": face}}, "name": "Tail"}),
    );
    println!("TAIL {tail}");
    for (view, file) in [("iso", "mouse-iso.png"), ("left", "mouse-left.png")] {
        run(
            &mut s,
            "view.render",
            json!({"path": file, "width": 640, "height": 420, "view": view}),
        );
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        println!("PNG-HEX {file} {hex}");
    }
}
