//! TEMPORARY: renders a bonnet and a wing panel built from curve networks,
//! the wing held in curvature (G2) to the bonnet along their seam, and
//! prints the PNG (hex) and the seam's continuity for the evidence.
//! Removed once the picture is looked at.

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
fn render_g2_panels() {
    let dir = workdir("render", "panels");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let xs = [0.0, 60.0, 120.0];
    let crown = [0.0, 4.0, 2.0];
    for k in 0..3 {
        let c = crown[k];
        let name = format!("Section{k}");
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "yz", "offset": format!("{} mm", xs[k]), "name": name}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": name, "through": [[-40.0, 0.0], [-20.0, 8.0 + c], [0.0, 10.0 + c], [20.0, 8.0 + c], [40.0, 0.0]]}),
        );
        let wing = format!("Wing{k}");
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "yz", "offset": format!("{} mm", xs[k]), "name": wing}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": wing, "through": [[40.0, 0.0], [48.0, -8.0], [53.0, -16.0], [55.0, -25.0]]}),
        );
    }
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Left"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Left", "from": [0.0, -40.0], "to": [120.0, -40.0]}),
    );
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Right"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Right", "from": [0.0, 40.0], "to": [120.0, 40.0]}),
    );
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Top"}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Top", "through": [[10.0, 0.0], [14.0, 60.0], [12.0, 120.0]], "degree": 2}),
    );
    built(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": "-25 mm", "name": "Low"}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Low", "from": [0.0, 55.0], "to": [120.0, 55.0]}),
    );
    let bonnet = built(
        &mut s,
        "feature.surface.network",
        json!({"u": ["Section0", "Section1", "Section2"], "v": ["Left", "Top", "Right"], "name": "Bonnet"}),
    );
    println!("BONNET {bonnet}");
    let face = body(&mut s, "Bonnet")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string();
    let wing = built(
        &mut s,
        "feature.surface.network",
        json!({"u": ["Right", "Low"], "v": ["Wing0", "Wing1", "Wing2"], "edges": {"uFirst": {"kind": "curvature", "face": face}}, "name": "Wing"}),
    );
    println!("WING {wing}");
    for (view, file) in [("iso", "panels-iso.png"), ("front", "panels-front.png")] {
        run(
            &mut s,
            "view.render",
            json!({"path": file, "width": 640, "height": 420, "view": view}),
        );
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        for (k, chunk) in hex.as_bytes().chunks(3000).enumerate() {
            println!("PNG-HEX {file} {k} {}", String::from_utf8_lossy(chunk));
        }
    }
}
