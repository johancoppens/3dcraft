//! `cargo xtask example-thumbs`: the pictures of the example scripts
//! (docs/development.md, Example pictures): the guide's lessons in
//! `docs/examples` and the models kept as tests in
//! `crates/engine/tests/example_models`, each shown beside its entry in its
//! folder's README.
//!
//! Each `<folder>/<id>.json` script is run in a new document by
//! `craft3d-cli run` and rendered by `view.render` with the CPU renderer
//! ([`PICTURE`]); the picture is cut to at most [`PICTURE_PALETTE`] colours
//! (the most common ones, each pixel to the nearest) and written as an
//! indexed PNG to `<folder>/<id>.png`. [`STAMPS`] holds, per script, a hash
//! of the script and of these settings: the test
//! [`tests::the_pictures_are_up_to_date`] fails when a script or the
//! settings changed and the pictures were not made again. The pixels
//! themselves are not compared (they may differ in the last bit between
//! platforms); the stamp says what they were made from.
//!
//! The start screen's thumbnails are not made here: they are the bundled
//! examples' own pictures, made smaller by `cargo xtask bundle-examples`
//! (`bundle.rs`), which uses [`quantize`] and [`write_indexed`] too.

use std::path::{Path, PathBuf};

/// The folders of example scripts (relative to the repository) and the
/// scripts in each, in their README's order: the guide's lessons, then the
/// models that were its examples until 2026-11-10 and stay as tests.
pub const FOLDERS: [(&str, &[&str]); 2] = [
    (
        "docs/examples",
        &[
            "bracket", "flange", "case", "bushing", "spring", "hopper", "gear",
        ],
    ),
    (
        "crates/engine/tests/example_models",
        &[
            "pipe",
            "panel",
            "wedge",
            "nameplate",
            "clamp",
            "channel",
            "chassis",
            "fin",
            "shade",
            "tank",
            "mouse",
        ],
    ),
];

/// What the pictures were made from, relative to the repository.
pub const STAMPS: &str = "xtask/example-pictures.txt";

/// The params of `view.render` the pictures are made with (without the
/// path), part of the stamp: the studio look in the Workshop environment
/// with the ACES tone curve (the developer's choice, 2026-10-31: more
/// colourful) and half a stop brighter (also his, the same day: metals
/// read dark in ACES), on a plain background, so a script added later
/// gets the same look from here. On the CPU renderer (`craft3d-cli`'s
/// sessions draw on the GPU when there is one): the same pictures on every
/// machine.
pub const PICTURE: &str = r#"{"width": 640, "height": 480, "view": "iso", "projection": "perspective", "zoom": 1, "renderer": "cpu", "look": {"mode": "studio", "environment": "workshop", "tonemap": "aces", "exposure": 0.5, "background": "plain", "accumulate": 8}}"#;

/// Most colours of a picture.
pub const PICTURE_PALETTE: usize = 256;
/// Largest picture accepted, in bytes (`guide_examples` holds them under
/// 200 kB).
pub const MAX_PICTURE_BYTES: u64 = 190 * 1024;
/// What else the stamp covers: bump it when the way the pictures are
/// made changes in a way the settings above do not show. (v7: the
/// start screen's thumbnails of the scripts are no longer made; the
/// pictures are made as in v6.)
const METHOD: &str = "pictures v7: units mm, studio look (workshop, ACES, +0.5 EV) on a plain background, popularity palette on 5-bit buckets, nearest colour; the README pictures of the scripts";

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

/// The stamp of a script: what its picture was made from.
pub fn stamp(script: &[u8]) -> String {
    let script = lf(script);
    let script = script.as_slice();
    let mut all = Vec::new();
    for part in [
        METHOD.as_bytes(),
        PICTURE.as_bytes(),
        &PICTURE_PALETTE.to_le_bytes(),
        script,
    ] {
        all.extend_from_slice(&(part.len() as u64).to_le_bytes());
        all.extend_from_slice(part);
    }
    hex(&craft3d_doc::sha256(&all)).chars().take(16).collect()
}

/// Every script: its folder and its id.
fn scripts() -> impl Iterator<Item = (&'static str, &'static str)> {
    FOLDERS
        .into_iter()
        .flat_map(|(folder, ids)| ids.iter().map(move |id| (folder, *id)))
}

/// The lines of the stamps file for the scripts as they are now.
pub fn stamps() -> Result<String, String> {
    let mut out = String::from(
        "# What the pictures of the example scripts (<folder>/<id>.png) were made from: cargo xtask example-thumbs\n",
    );
    for (folder, id) in scripts() {
        let path = root().join(folder).join(format!("{id}.json"));
        let script =
            std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        out.push_str(&format!("{id} {}\n", stamp(&script)));
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
pub(crate) fn read_rgb(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
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
pub(crate) fn write_indexed(
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

/// Makes every picture again, and the stamps: all of them or none (each
/// is checked against [`MAX_PICTURE_BYTES`] before any is written).
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
    let mut pictures = Vec::new();
    // Every script is tried, so one run names every one that fails.
    let mut failed = Vec::new();
    for (folder, id) in scripts() {
        let picture_png = work.join(format!("{id}-picture.png"));
        let mut params: serde_json::Value =
            serde_json::from_str(PICTURE).map_err(|e| e.to_string())?;
        if let Some(map) = params.as_object_mut() {
            map.insert(
                "path".into(),
                serde_json::Value::String(picture_png.to_string_lossy().into_owned()),
            );
        }
        let script = root().join(folder).join(format!("{id}.json"));
        let status = std::process::Command::new(&cli)
            .args(["run", "--new", r#"{"units":"mm"}"#, "--script"])
            .arg(&script)
            .args(["--cmd", "view.render", "--params"])
            .arg(params.to_string())
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("cannot run {}: {e}", cli.display()))?;
        if !status.success() {
            eprintln!("{id}: craft3d-cli failed ({status})");
            failed.push(id);
            continue;
        }
        let bytes = std::fs::read(&picture_png).map_err(|e| e.to_string())?;
        let (width, height, rgb) = read_rgb(&bytes)?;
        let (palette, indices) = quantize(&rgb, PICTURE_PALETTE);
        let png = write_indexed(width, height, &palette, &indices)?;
        let size = png.len() as u64;
        println!("{id}: {size} bytes");
        if size > MAX_PICTURE_BYTES {
            return Err(format!(
                "{id}: the picture is {size} bytes, more than {MAX_PICTURE_BYTES}; nothing was written"
            ));
        }
        pictures.push((folder, id, png));
    }
    if !failed.is_empty() {
        return Err(format!(
            "craft3d-cli failed for {}; nothing was written",
            failed.join(", ")
        ));
    }
    for (folder, id, png) in pictures {
        std::fs::write(root().join(folder).join(format!("{id}.png")), &png)
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(root().join(STAMPS), stamps()?).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_are_up_to_date() {
        let kept = std::fs::read_to_string(root().join(STAMPS)).unwrap_or_default();
        // Line by line: a checkout with CRLF line ends reads the same.
        let kept: Vec<&str> = kept.lines().map(str::trim_end).collect();
        let now = stamps().unwrap();
        let now: Vec<&str> = now.lines().collect();
        assert_eq!(
            kept, now,
            "an example script or the picture settings changed: run `cargo xtask example-thumbs` and commit the pictures and {STAMPS}"
        );
        for (folder, id) in scripts() {
            let size = std::fs::metadata(root().join(folder).join(format!("{id}.png")))
                .unwrap()
                .len();
            assert!(size <= MAX_PICTURE_BYTES, "{id}: {size} bytes");
        }
    }

    #[test]
    fn every_script_of_a_folder_is_listed() {
        for (folder, ids) in FOLDERS {
            let mut scripts: Vec<String> = std::fs::read_dir(root().join(folder))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter_map(|name| name.strip_suffix(".json").map(str::to_string))
                .collect();
            scripts.sort();
            let mut listed: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
            listed.sort();
            assert_eq!(scripts, listed, "{folder}");
        }
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
