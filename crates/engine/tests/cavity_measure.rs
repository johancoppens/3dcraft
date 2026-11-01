//! The measure `cavity`: what a body's cavities hold (the shells inside it,
//! which enclose a negative volume). A block made hollow by `feature.shell`
//! with no face left open holds the block shrunk by the wall on every
//! side, in closed form, at three scales; open at its top it holds
//! nothing; the volume and the cavity together are the block's outside.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use serde_json::json;

#[test]
fn a_shelled_block_holds_the_block_less_its_walls() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = common::session();
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        let (a, b, c, t) = (40.0 * k, 30.0 * k, 20.0 * k, 2.0 * k);
        run(
            &mut s,
            "feature.block",
            json!({"size": [format!("{a} mm"), format!("{b} mm"), format!("{c} mm")], "name": "B"}),
        );
        run(
            &mut s,
            "feature.shell",
            json!({"source": "B", "thickness": format!("{t} mm"), "name": "S"}),
        );
        let held = (a - 2.0 * t) * (b - 2.0 * t) * (c - 2.0 * t);
        let measure = |s: &mut craft3d_engine::Session, m: &str| {
            let out = run(
                s,
                "variable.measure",
                json!({"name": format!("m_{m}"), "measure": m, "of": "S"}),
            );
            out["value"].as_f64().unwrap_or_else(|| panic!("{out}"))
        };
        let volume = measure(&mut s, "volume");
        let cavity = measure(&mut s, "cavity");
        assert!(
            (cavity - held).abs() <= 1e-9 * held,
            "at {k}: {cavity} for {held}"
        );
        let outside = a * b * c;
        assert!(
            (volume + cavity - outside).abs() <= 1e-9 * outside,
            "at {k}"
        );
        // Open at a face: no cavity.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "S", "open": ["F1:face(0)"]}),
        );
        let open = measure(&mut s, "cavity");
        assert!(open.abs() <= 1e-9 * outside, "at {k}: {open}");
    }
}
