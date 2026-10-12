//! Exports the Utah teapot as STL, 3MF and OBJ.
//!
//! ```sh
//! cargo run --release -p craft3d-io --example export_teapot -- target/evidence
//! ```

use craft3d_io::{MeshFormat, export_mesh};
use craft3d_meshio::Unit;
use craft3d_tess::{TessParams, tessellate_patches};
use craft3d_testkit::teapot::teapot;
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/evidence".to_string()),
    );
    std::fs::create_dir_all(&dir)?;
    let params = TessParams {
        chordal_tolerance: 0.002,
        ..TessParams::default()
    };
    let mesh = tessellate_patches(&teapot()?, &params)?;
    for (file, format) in [
        ("teapot.stl", MeshFormat::StlBinary),
        ("teapot.3mf", MeshFormat::ThreeMf),
        ("teapot.obj", MeshFormat::Obj),
    ] {
        let bytes = export_mesh(&mesh, "teapot", Unit::Millimeter, format)?;
        let path = dir.join(file);
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!(
            "{}  {} bytes  {} triangles",
            path.display(),
            bytes.len(),
            mesh.triangles.len()
        );
    }
    Ok(())
}
