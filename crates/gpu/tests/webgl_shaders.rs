//! The shaders stay within what WebGL2 can take, so the browser build
//! (`apps/craft3d-web`) draws with the same shaders as the desktop: wgpu
//! translates WGSL to GLSL ES 3.0 there, which has no arrays of arrays
//! (naga refuses `array<array<…>>` for it, and the device then reports a
//! validation error for every pipeline that uses it).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

#[test]
fn no_shader_declares_an_array_of_arrays() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("wgsl") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            !compact.contains("array<array<"),
            "{} declares an array of arrays, which WebGL2 (GLSL ES 3.0) cannot take; use one array indexed by row * columns + column",
            path.display()
        );
        checked += 1;
    }
    assert!(checked > 10, "only {checked} shaders found in {}", dir.display());
}
