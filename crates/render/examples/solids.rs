//! Renders every reference solid of the test kit into one contact sheet,
//! so a person can look at what the tessellator makes of them.
//!
//! ```sh
//! cargo run --release -p craft3d-render --example solids -- target/evidence/solids.png
//! ```

use craft3d_math::{Frame, Vec3};
use craft3d_render::{Camera, RenderParams, encode_png, render};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_testkit::{solids, trimmed};
use std::error::Error;

const CELL: (u32, u32) = (360, 270);
const COLUMNS: u32 = 5;

fn main() -> Result<(), Box<dyn Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/evidence/solids.png".to_string());
    let mut all = solids::all(&Frame::WORLD, 1.0)?;
    all.extend(trimmed::all(&Frame::WORLD, 1.0)?);
    all.push(trimmed::pipe_elbow(&Frame::WORLD, 3.0, 1.0, 1.9)?);
    all.push(trimmed::sphere_octant(&Frame::WORLD, 2.0)?);

    let rows = (all.len() as u32).div_ceil(COLUMNS);
    let (width, height) = (CELL.0 * COLUMNS, CELL.1 * rows);
    let mut sheet = vec![0_u8; (width * height * 3) as usize];
    let params = RenderParams {
        width: CELL.0,
        height: CELL.1,
        ..RenderParams::default()
    };
    for (index, solid) in all.iter().enumerate() {
        let mesh = tessellate_body(&solid.body, &TessParams::default())?;
        let aspect = f64::from(CELL.0) / f64::from(CELL.1);
        let camera = Camera::framing(
            &mesh.bbox(),
            Vec3::new(1.0, -1.3, 0.9),
            Vec3::Z,
            30.0,
            aspect,
        )
        .ok_or("cannot frame an empty model")?;
        let frame = render(&mesh, &camera, &params)?;
        let (column, row) = (index as u32 % COLUMNS, index as u32 / COLUMNS);
        for (y, line) in frame.rgb.chunks((CELL.0 * 3) as usize).enumerate() {
            let start = (((row * CELL.1 + y as u32) * width + column * CELL.0) * 3) as usize;
            if let Some(target) = sheet.get_mut(start..start + line.len()) {
                target.copy_from_slice(line);
            }
        }
        println!(
            "{:2} {:28} {:7} triangles",
            index,
            solid.name,
            mesh.triangles.len()
        );
    }
    if let Some(dir) = std::path::Path::new(&out)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
    {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, encode_png(width, height, &sheet)?)?;
    println!("{out}");
    Ok(())
}
