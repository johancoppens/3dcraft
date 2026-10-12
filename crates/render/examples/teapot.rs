//! Renders the Utah teapot with the CPU reference renderer.
//!
//! ```sh
//! cargo run --release -p craft3d-render --example teapot -- --out target/evidence/teapot.png
//! ```
//!
//! Options: `--out <png>`, `--ids <png>` (false-colour face ids),
//! `--size <W>x<H>`, `--tolerance <model units>`, `--ssaa <1..4>`.

use craft3d_math::Vec3;
use craft3d_render::{Camera, RenderParams, encode_png, render};
use craft3d_tess::{TessParams, tessellate_patches};
use craft3d_testkit::teapot::teapot;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()).into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut out = PathBuf::from("target/evidence/teapot.png");
    let mut ids_out: Option<PathBuf> = None;
    let mut params = RenderParams {
        width: 1280,
        height: 800,
        ..RenderParams::default()
    };
    let mut tess = TessParams {
        chordal_tolerance: 0.002,
        ..TessParams::default()
    };

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--out" => out = PathBuf::from(value),
            "--ids" => ids_out = Some(PathBuf::from(value)),
            "--size" => {
                let (w, h) = value
                    .split_once('x')
                    .ok_or_else(|| format!("--size expects WxH, got `{value}`"))?;
                params.width = w.parse().map_err(|e| format!("bad width `{w}`: {e}"))?;
                params.height = h.parse().map_err(|e| format!("bad height `{h}`: {e}"))?;
            }
            "--tolerance" => {
                tess.chordal_tolerance = value
                    .parse()
                    .map_err(|e| format!("bad tolerance `{value}`: {e}"))?;
            }
            "--ssaa" => {
                params.ssaa = value
                    .parse()
                    .map_err(|e| format!("bad ssaa `{value}`: {e}"))?
            }
            other => return Err(format!("unknown option `{other}`").into()),
        }
    }

    let patches = teapot()?;
    let started = Instant::now();
    let mesh = tessellate_patches(&patches, &tess)?;
    let tess_ms = started.elapsed().as_secs_f64() * 1e3;

    let aspect = f64::from(params.width) / f64::from(params.height.max(1));
    let camera = Camera::framing(
        &mesh.bbox(),
        Vec3::new(0.35, -1.0, 0.45),
        Vec3::Z,
        30.0,
        aspect,
    )
    .ok_or("cannot frame an empty model")?;
    let started = Instant::now();
    let frame = render(&mesh, &camera, &params)?;
    let render_ms = started.elapsed().as_secs_f64() * 1e3;

    write(&out, &frame.to_png()?)?;
    if let Some(path) = &ids_out {
        write(
            path,
            &encode_png(frame.width, frame.height, &frame.false_color())?,
        )?;
    }

    let (lo, hi) = mesh.bbox().corners().ok_or("empty mesh")?;
    println!(
        "{{\"out\":\"{}\",\"patches\":{},\"triangles\":{},\"vertices\":{},\"tolerance\":{},\
         \"bbox\":[[{:.4},{:.4},{:.4}],[{:.4},{:.4},{:.4}]],\"meshArea\":{:.4},\
         \"size\":[{},{}],\"ssaa\":{},\"coverage\":{:.4},\"visiblePatches\":{},\
         \"tessMs\":{:.1},\"renderMs\":{:.1}}}",
        out.display(),
        patches.len(),
        mesh.triangles.len(),
        mesh.positions.len(),
        tess.chordal_tolerance,
        lo.x,
        lo.y,
        lo.z,
        hi.x,
        hi.y,
        hi.z,
        mesh.area(),
        frame.width,
        frame.height,
        params.ssaa,
        frame.coverage(),
        frame.visible_faces().len(),
        tess_ms,
        render_ms,
    );
    Ok(())
}
