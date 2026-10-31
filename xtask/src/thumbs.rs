//! `cargo xtask example-thumbs`: the pictures of the guide's example
//! models on the window's start screen (docs/development.md, Example
//! thumbnails).
//!
//! Each `docs/examples/<id>.json` script is run in a new document by
//! `craft3d-cli run` and rendered by `view.render` with the CPU renderer
//! ([`RENDER`]: iso, 160 × 120); the picture is cut to at most
//! [`PALETTE`] colours (the most common ones, each pixel to the nearest)
//! and written as an indexed PNG to `docs/examples/thumbs/<id>.png`, so
//! each stays a few kilobytes in the program. `STAMPS.txt` beside them
//! holds, per example, a hash of its script and of these settings: the
//! test [`tests::the_thumbnails_are_up_to_date`] fails when a script or
//! the settings changed and the thumbnails were not made again. The
//! pixels themselves are not compared (they may differ in the last bit
//! between platforms); the stamp says what they were made from.

use std::path::{Path, PathBuf};

/// The examples, as `docs/examples` names them (the guide's order).
pub const EXAMPLES: [&str; 18] = [
    "bracket",
    "flange",
    "case",
    "bushing",
    "pipe",
    "spring",
    "hopper",
    "panel",
    "wedge",
    "gear",
    "nameplate",
    "clamp",
    "channel",
    "fin",
    "shade",
    "tank",
    "mouse",
    "chassis",
];

/// The published Jigs and assemblies of the start screen
/// (docs/decisions.md, 2026-10-30): an id for the thumbnail and the file
/// in `docs/examples/published`, opened as it is.
pub const PUBLISHED: [(&str, &str); 6] = [
    ("jig-planetary", "planetary-gearbox.3dcraftjig"),
    ("jig-reducer", "gear-reducer.3dcraftjig"),
    ("jig-strandbeest", "strandbeest-leg.3dcraftjig"),
    ("asm-planetary", "planetary-gearbox.3dcraftasm"),
    ("asm-reducer", "gear-reducer.3dcraftasm"),
    ("asm-walker", "strandbeest-walker.3dcraftasm"),
];

/// The params of `view.render` the thumbnails are made with (without the
/// path), part of the stamp: the studio look in the Workshop environment
/// with the ACES tone curve (the developer's choice, 2026-10-31: more
/// colourful), on a plain background, for every thumbnail, so an example
/// added later gets the same look from here. On the CPU renderer
/// (`craft3d-cli`'s sessions draw on the GPU when there is one): the same
/// pictures on every machine.
pub const RENDER: &str = r#"{"width": 160, "height": 120, "view": "iso", "projection": "perspective", "zoom": 1.2, "renderer": "cpu", "look": {"mode": "studio", "environment": "workshop", "tonemap": "aces", "background": "plain", "accumulate": 4}}"#;
/// The params of `view.render` for the picture of a script example in
/// the examples' README (`docs/examples/<id>.png`), part of the stamp: the
/// thumbnails' look ([`RENDER`]), larger and smoother.
pub const PICTURE: &str = r#"{"width": 640, "height": 480, "view": "iso", "projection": "perspective", "zoom": 1, "renderer": "cpu", "look": {"mode": "studio", "environment": "workshop", "tonemap": "aces", "background": "plain", "accumulate": 8}}"#;
/// Thumbnails drawn at another zoom than [`RENDER`]'s, as (id, zoom): a
/// long, flat part that the closer framing would cut at its ends. Part of
/// every stamp.
pub const THUMB_ZOOM: [(&str, f64); 1] = [("nameplate", 1.0)];

/// The thumbnail's `view.render` params of example `id` (without the
/// path): [`RENDER`], at its [`THUMB_ZOOM`] when it has one.
fn thumb_render(id: &str) -> Result<String, String> {
    let mut params: serde_json::Value = serde_json::from_str(RENDER).map_err(|e| e.to_string())?;
    if let Some((_, zoom)) = THUMB_ZOOM.iter().find(|(i, _)| *i == id)
        && let Some(map) = params.as_object_mut()
    {
        map.insert("zoom".into(), serde_json::json!(zoom));
    }
    Ok(params.to_string())
}

/// Most colours of a thumbnail.
pub const PALETTE: usize = 128;
/// Most colours of a README picture.
pub const PICTURE_PALETTE: usize = 256;
/// Largest README picture accepted, in bytes (`guide_examples` holds
/// them under 200 kB).
pub const MAX_PICTURE_BYTES: u64 = 190 * 1024;
/// What else the stamp covers: bump it when the way the pictures are
/// made changes in a way the settings above do not show.
const METHOD: &str = "thumbs v5: units mm, studio look (workshop, ACES) on a plain background, popularity palette on 5-bit buckets, nearest colour; README pictures of the scripts; published files opened as they are";
/// Largest thumbnail file accepted, in bytes.
pub const MAX_BYTES: u64 = 24 * 1024;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `text` with every `\r\n` made `\n`: a checkout that turned the
/// script's line ends into CRLF (git's autocrlf on Windows) stamps the
/// same.
fn lf(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.iter().peekable();
    while let Some(&b) = bytes.next() {
        if b == b'\r' && bytes.peek() == Some(&&b'\n') {
            continue;
        }
        out.push(b);
    }
    out
}

/// The stamp of an example: what its thumbnail was made from.
pub fn stamp(script: &[u8]) -> String {
    let script = lf(script);
    let script = script.as_slice();
    let mut all = Vec::new();
    for part in [
        METHOD.as_bytes(),
        RENDER.as_bytes(),
        format!("{THUMB_ZOOM:?}").as_bytes(),
        PICTURE.as_bytes(),
        &PALETTE.to_le_bytes(),
        &PICTURE_PALETTE.to_le_bytes(),
        script,
    ] {
        all.extend_from_slice(&(part.len() as u64).to_le_bytes());
        all.extend_from_slice(part);
    }
    hex(&craft3d_doc::sha256(&all)).chars().take(16).collect()
}

/// The lines of `STAMPS.txt` for the scripts as they are now.
pub fn stamps() -> Result<String, String> {
    let mut out = String::from(
        "# What docs/examples/thumbs/<id>.png were made from: cargo xtask example-thumbs\n",
    );
    for id in EXAMPLES {
        let path = root().join("docs/examples").join(format!("{id}.json"));
        let script =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        out.push_str(&format!("{id} {}\n", stamp(&script)));
    }
    for (id, file) in PUBLISHED {
        let path = root().join("docs/examples/published").join(file);
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        out.push_str(&format!("{id} {}\n", stamp(&bytes)));
    }
    Ok(out)
}

/// `pixels` (RGB, 8 bits each) as at most `colours` colours: a palette
/// (RGB triples) and an index per pixel. The palette is the average of
/// the most common 5-bit buckets, most common first (ties by bucket), and
/// every pixel takes the nearest entry; deterministic.
pub fn quantize(pixels: &[u8], colours: usize) -> (Vec<u8>, Vec<u8>) {
    let bucket = |p: &[u8]| -> usize {
        match p {
            [r, g, b] => {
                (usize::from(r >> 3) << 10) | (usize::from(g >> 3) << 5) | usize::from(b >> 3)
            }
            _ => 0,
        }
    };
    let mut sums = vec![[0_u64; 4]; 1 << 15];
    for p in pixels.as_chunks::<3>().0 {
        if let (Some(sum), [r, g, b]) = (sums.get_mut(bucket(p)), p) {
            sum[0] += u64::from(*r);
            sum[1] += u64::from(*g);
            sum[2] += u64::from(*b);
            sum[3] += 1;
        }
    }
    let mut used: Vec<(usize, [u64; 4])> = sums
        .into_iter()
        .enumerate()
        .filter(|(_, s)| s[3] > 0)
        .collect();
    used.sort_by(|a, b| b.1[3].cmp(&a.1[3]).then(a.0.cmp(&b.0)));
    let palette: Vec<[u8; 3]> = used
        .iter()
        .take(colours.clamp(1, 256))
        .map(|(_, s)| {
            let [r, g, b, n] = *s;
            let mean = |v: u64| u8::try_from(v / n.max(1)).unwrap_or(u8::MAX);
            [mean(r), mean(g), mean(b)]
        })
        .collect();
    let mut cache: std::collections::BTreeMap<[u8; 3], u8> = std::collections::BTreeMap::new();
    let indices = pixels
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&key| {
            *cache.entry(key).or_insert_with(|| {
                let distance = |c: &[u8; 3]| -> i32 {
                    c.iter()
                        .zip(key)
                        .map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2))
                        .sum()
                };
                let best = palette
                    .iter()
                    .enumerate()
                    .min_by_key(|(i, c)| (distance(c), *i))
                    .map_or(0, |(i, _)| i);
                u8::try_from(best).unwrap_or(0)
            })
        })
        .collect();
    (palette.concat(), indices)
}

/// Reads an 8-bit RGB or RGBA PNG as RGB.
fn read_rgb(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    buffer.truncate(info.buffer_size());
    let rgb = match info.color_type {
        png::ColorType::Rgb => buffer,
        png::ColorType::Rgba => buffer
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|[r, g, b, _]| [*r, *g, *b])
            .collect(),
        other => return Err(format!("a {other:?} picture; RGB or RGBA expected")),
    };
    Ok((info.width, info.height, rgb))
}

/// An indexed PNG of `palette` and `indices`.
fn write_indexed(
    width: u32,
    height: u32,
    palette: &[u8],
    indices: &[u8],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette.to_vec());
    encoder.set_compression(png::Compression::Best);
    encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(indices)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Builds `craft3d-cli` and says where cargo put it (its artifact
/// message), as `ci.rs` finds the test binaries.
fn build_cli() -> Result<PathBuf, String> {
    eprintln!("$ cargo build -p craft3d-cli");
    let output = std::process::Command::new(crate::cargo_path())
        .args([
            "build",
            "-p",
            "craft3d-cli",
            "--message-format=json-render-diagnostics",
        ])
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo build -p craft3d-cli` failed ({})",
            output.status
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact"))
        .filter(|m| {
            m.get("target")
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                == Some("craft3d-cli")
        })
        .find_map(|m| {
            m.get("executable")
                .and_then(|e| e.as_str())
                .map(PathBuf::from)
        })
        .ok_or_else(|| "cargo did not say where craft3d-cli is".to_string())
}

/// Makes every thumbnail again, and `STAMPS.txt`: all of them or none
/// (each is checked against [`MAX_BYTES`] before any is written).
pub fn run(_args: &[String]) -> Result<(), String> {
    let cli = build_cli()?;
    let target = cli
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root().join("target"));
    let work = target.join("tmp").join("example-thumbs");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let out_dir = root().join("docs/examples/thumbs");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut total = 0_u64;
    let mut made = Vec::new();
    let scripts = EXAMPLES.iter().map(|id| (*id, None));
    let published = PUBLISHED.iter().map(|(id, file)| (*id, Some(*file)));
    let mut pictures = Vec::new();
    // Every example is tried, so one run names every one that fails.
    let mut failed = Vec::new();
    for (id, file) in scripts.chain(published) {
        let thumb_png = work.join(format!("{id}.png"));
        let picture_png = work.join(format!("{id}-picture.png"));
        let with_path = |settings: &str, path: &Path| -> Result<String, String> {
            let mut params: serde_json::Value =
                serde_json::from_str(settings).map_err(|e| e.to_string())?;
            if let Some(map) = params.as_object_mut() {
                map.insert(
                    "path".into(),
                    serde_json::Value::String(path.to_string_lossy().into_owned()),
                );
            }
            Ok(params.to_string())
        };
        let mut command = std::process::Command::new(&cli);
        match file {
            None => {
                let script = root().join("docs/examples").join(format!("{id}.json"));
                command
                    .args(["run", "--new", r#"{"units":"mm"}"#, "--script"])
                    .arg(&script)
                    .args(["--cmd", "view.render", "--params"])
                    .arg(with_path(PICTURE, &picture_png)?);
            }
            Some(file) => {
                command
                    .arg("run")
                    .arg(root().join("docs/examples/published").join(file));
            }
        }
        let status = command
            .args(["--cmd", "view.render", "--params"])
            .arg(with_path(&thumb_render(id)?, &thumb_png)?)
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("cannot run {}: {e}", cli.display()))?;
        if !status.success() {
            eprintln!("{id}: craft3d-cli failed ({status})");
            failed.push(id);
            continue;
        }
        let bytes = std::fs::read(&thumb_png).map_err(|e| e.to_string())?;
        let (width, height, rgb) = read_rgb(&bytes)?;
        let (palette, indices) = quantize(&rgb, PALETTE);
        let png = write_indexed(width, height, &palette, &indices)?;
        let size = png.len() as u64;
        total += size;
        println!("{id}: {size} bytes");
        if size > MAX_BYTES {
            return Err(format!(
                "{id}: {size} bytes, more than {MAX_BYTES}; nothing was written"
            ));
        }
        made.push((id, png));
        if file.is_none() {
            let bytes = std::fs::read(&picture_png).map_err(|e| e.to_string())?;
            let (width, height, rgb) = read_rgb(&bytes)?;
            let (palette, indices) = quantize(&rgb, PICTURE_PALETTE);
            let png = write_indexed(width, height, &palette, &indices)?;
            let size = png.len() as u64;
            println!("{id} picture: {size} bytes");
            if size > MAX_PICTURE_BYTES {
                return Err(format!(
                    "{id}: the picture is {size} bytes, more than {MAX_PICTURE_BYTES}; nothing was written"
                ));
            }
            pictures.push((id, png));
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "craft3d-cli failed for {}; nothing was written",
            failed.join(", ")
        ));
    }
    for (id, png) in made {
        std::fs::write(out_dir.join(format!("{id}.png")), &png).map_err(|e| e.to_string())?;
    }
    for (id, png) in pictures {
        std::fs::write(root().join("docs/examples").join(format!("{id}.png")), &png)
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(out_dir.join("STAMPS.txt"), stamps()?).map_err(|e| e.to_string())?;
    println!("all: {total} bytes");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumbnails_are_up_to_date() {
        let dir = root().join("docs/examples/thumbs");
        let kept = std::fs::read_to_string(dir.join("STAMPS.txt")).unwrap_or_default();
        // Line by line: a checkout with CRLF line ends reads the same.
        let kept: Vec<&str> = kept.lines().map(str::trim_end).collect();
        let now = stamps().unwrap();
        let now: Vec<&str> = now.lines().collect();
        assert_eq!(
            kept, now,
            "an example script or the thumbnail settings changed: run `cargo xtask example-thumbs` and commit docs/examples/thumbs"
        );
        for id in EXAMPLES.into_iter().chain(PUBLISHED.map(|(id, _)| id)) {
            let size = std::fs::metadata(dir.join(format!("{id}.png")))
                .unwrap()
                .len();
            assert!(size <= MAX_BYTES, "{id}: {size} bytes");
        }
    }

    #[test]
    fn every_example_of_the_guide_has_a_thumbnail() {
        let mut scripts: Vec<String> = std::fs::read_dir(root().join("docs/examples"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|name| name.strip_suffix(".json").map(str::to_string))
            .collect();
        scripts.sort();
        let mut listed: Vec<String> = EXAMPLES.iter().map(|s| s.to_string()).collect();
        listed.sort();
        assert_eq!(scripts, listed);
    }

    #[test]
    fn a_stamp_follows_the_script() {
        assert_eq!(stamp(b"[]"), stamp(b"[]"));
        assert_eq!(
            stamp(b"[\n1\n]\n"),
            stamp(b"[\r\n1\r\n]\r\n"),
            "line ends do not count"
        );
        assert_ne!(stamp(b"[\n1]"), stamp(b"[\r1]"));
        assert_ne!(stamp(b"[]"), stamp(b"[ ]"));
        assert_eq!(stamp(b"[]").len(), 16);
    }

    #[test]
    fn quantizing_keeps_few_colours_exact_and_caps_many() {
        let few = [10, 20, 30, 10, 20, 30, 200, 100, 0];
        let (palette, indices) = quantize(&few, 128);
        assert_eq!(palette, [10, 20, 30, 200, 100, 0]);
        assert_eq!(indices, [0, 0, 1]);
        let many: Vec<u8> = (0..=255_u8).flat_map(|v| [v, 255 - v, v / 2]).collect();
        let (palette, indices) = quantize(&many, 16);
        assert_eq!(palette.len(), 48);
        assert!(indices.iter().all(|&i| i < 16));
        let png = write_indexed(16, 16, &palette, &indices).unwrap();
        let (w, h, rgb) = read_rgb(&png).unwrap();
        assert_eq!((w, h, rgb.len()), (16, 16, 768));
    }
}
