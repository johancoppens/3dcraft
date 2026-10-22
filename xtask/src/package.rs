//! `cargo xtask package`: the installers of 3DCraft (docs/development.md,
//! Packaging). Builds `craft3d` and `craft3d-cli` in release and packs them
//! for the platform it runs on:
//!
//! - Linux: a tarball (with `install.sh`) and an AppImage;
//! - macOS: `3DCraft.app` in a `.dmg`;
//! - Windows: a zip and an `.msi`.
//!
//! The files they carry (desktop entry, MIME type, Info.plist, WiX source,
//! icons) live in `packaging/`. The external tools are the platform's own
//! (`tar`, `hdiutil`, `codesign`) plus `appimagetool` on Linux and the WiX
//! Toolset (`wix`) on Windows; a format whose tool is missing fails with a
//! message that names it, and `--formats` leaves it out.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The reverse-DNS identifier of 3DCraft: desktop file, AppStream id, macOS
/// bundle id (docs/decisions.md, 2026-10-22).
pub const APP_ID: &str = "io.github.johancoppens.craft3d";

/// The sizes of the PNG icons in `packaging/icons` (`craft3d-<size>.png`).
const ICON_SIZES: [u32; 8] = [16, 32, 48, 64, 128, 256, 512, 1024];

/// The PNG sizes in the macOS `.icns` and the Windows `.ico`.
const ICNS_SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];
const ICO_SIZES: [u32; 5] = [16, 32, 48, 64, 256];

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    target: Option<String>,
    out: Option<PathBuf>,
    formats: Option<Vec<String>>,
    jobs: Option<String>,
    build: bool,
    appimagetool: Option<PathBuf>,
    appimage_runtime: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        build: true,
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--target" => o.target = Some(value()?),
            "--out" => o.out = Some(PathBuf::from(value()?)),
            "--formats" => {
                o.formats = Some(
                    value()?
                        .split(',')
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty())
                        .collect(),
                );
            }
            "-j" | "--jobs" => o.jobs = Some(value()?),
            "--no-build" => o.build = false,
            "--appimagetool" => o.appimagetool = Some(PathBuf::from(value()?)),
            "--appimage-runtime" => o.appimage_runtime = Some(PathBuf::from(value()?)),
            other => {
                return Err(format!(
                    "package: unknown argument `{other}`; usage: cargo xtask package \
                     [--target TRIPLE] [--out DIR] [--formats LIST] [-j N] [--no-build] \
                     [--appimagetool PATH] [--appimage-runtime PATH]"
                ));
            }
        }
    }
    Ok(o)
}

/// The formats made on this platform when `--formats` is not given.
fn default_formats() -> Vec<String> {
    let names: &[&str] = if cfg!(target_os = "macos") {
        &["app", "dmg"]
    } else if cfg!(windows) {
        &["zip", "msi"]
    } else {
        &["tar", "appimage"]
    };
    names.iter().map(|s| (*s).to_string()).collect()
}

fn known_formats() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["app", "dmg"]
    } else if cfg!(windows) {
        &["zip", "msi"]
    } else {
        &["tar", "appimage"]
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn target_dir(repo: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => std::env::current_dir().unwrap_or_default().join(dir),
        None => repo.join("target"),
    }
}

/// The CPU name in artifact names: the first part of the target triple.
fn arch_of(target: Option<&str>) -> String {
    target
        .and_then(|t| t.split('-').next())
        .filter(|a| !a.is_empty())
        .unwrap_or(std::env::consts::ARCH)
        .to_string()
}

/// An MSI version is three numbers: `0.1.0-beta.2` becomes `0.1.0`.
fn msi_version(version: &str) -> Result<String, String> {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.parse::<u32>().is_ok_and(|n| n <= 65535));
    if numeric {
        Ok(core.to_string())
    } else {
        Err(format!(
            "version `{version}` is not x.y.z, which an MSI needs"
        ))
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let formats = options.formats.clone().unwrap_or_else(default_formats);
    if formats.is_empty() {
        return Err("package: --formats names no format".to_string());
    }
    for f in &formats {
        if !known_formats().contains(&f.as_str()) {
            return Err(format!(
                "package: `{f}` is not made on this platform; here: {}",
                known_formats().join(", ")
            ));
        }
    }
    let repo = repo_root();
    if options.build {
        let mut cargo = vec!["build", "--release", "--locked"];
        if let Some(t) = &options.target {
            cargo.extend(["--target", t.as_str()]);
        }
        if let Some(j) = &options.jobs {
            cargo.extend(["-j", j.as_str()]);
        }
        cargo.extend(["-p", "craft3d", "-p", "craft3d-cli"]);
        crate::cargo(&cargo)?;
    }
    let target = target_dir(&repo);
    let mut bin_dir = target.clone();
    if let Some(t) = &options.target {
        bin_dir.push(t);
    }
    bin_dir.push("release");
    // Absolute, since the tools run with other working directories.
    let out = match options.out.clone() {
        Some(dir) if dir.is_relative() => std::env::current_dir()
            .map_err(|e| format!("no current directory: {e}"))?
            .join(dir),
        Some(dir) => dir,
        None => target.join("package"),
    };
    let p = Packer {
        repo,
        bin_dir,
        stage: out.join("stage"),
        out,
        version: env!("CARGO_PKG_VERSION").to_string(),
        arch: arch_of(options.target.as_deref()),
    };
    if p.stage.exists() {
        std::fs::remove_dir_all(&p.stage)
            .map_err(|e| format!("cannot clear {}: {e}", p.stage.display()))?;
    }
    mkdir(&p.stage)?;
    let mut made = Vec::new();
    for f in &formats {
        let file = match f.as_str() {
            "tar" => p.tarball()?,
            "appimage" => p.appimage(
                options.appimagetool.as_deref(),
                options.appimage_runtime.as_deref(),
            )?,
            "app" => p.app_bundle()?,
            "dmg" => p.dmg()?,
            "zip" => p.zip()?,
            "msi" => p.msi()?,
            other => return Err(format!("package: unknown format `{other}`")),
        };
        made.push(file);
    }
    for file in &made {
        eprintln!("package: made {}", file.display());
    }
    Ok(())
}

struct Packer {
    repo: PathBuf,
    bin_dir: PathBuf,
    out: PathBuf,
    stage: PathBuf,
    version: String,
    arch: String,
}

fn mkdir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))
}

/// Copies `from` to `to`, creating `to`'s directory, and sets the mode
/// (on Unix): 0o755 for programs and scripts, 0o644 for data.
fn copy(from: &Path, to: &Path, executable: bool) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        mkdir(dir)?;
    }
    std::fs::copy(from, to)
        .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
    set_mode(to, executable)
}

#[cfg(unix)]
fn set_mode(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("cannot set the mode of {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

fn write(to: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        mkdir(dir)?;
    }
    std::fs::write(to, bytes).map_err(|e| format!("cannot write {}: {e}", to.display()))
}

fn read(from: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(from).map_err(|e| format!("cannot read {}: {e}", from.display()))
}

/// How many lines of a failed tool's output its error repeats.
const FAILURE_LINES: usize = 20;

/// Runs an external tool and fails with its name when it cannot be started
/// or does not succeed. Its output is captured and passed on, and a failure
/// repeats the end of it, so the error says why even when the output
/// scrolled away (a CI log shows the error last).
fn tool(command: &mut Command, what: &str) -> Result<(), String> {
    eprintln!("$ {command:?}");
    let output = command
        .output()
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(what, &output.status.to_string(), &stdout, &stderr))
    }
}

/// The error of a failed tool: its name, its exit status and the last
/// lines it wrote (standard output, then standard error).
fn failure(what: &str, status: &str, stdout: &str, stderr: &str) -> String {
    let lines: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = lines
        .get(lines.len().saturating_sub(FAILURE_LINES)..)
        .unwrap_or_default();
    if tail.is_empty() {
        format!("{what} failed ({status}) and wrote nothing")
    } else {
        let mut message = format!("{what} failed ({status}); it wrote:");
        for line in tail {
            message.push_str("\n  ");
            message.push_str(line);
        }
        message
    }
}

impl Packer {
    fn packaging(&self, rel: &str) -> PathBuf {
        self.repo.join("packaging").join(rel)
    }

    fn icon(&self, size: u32) -> PathBuf {
        self.packaging(&format!("icons/craft3d-{size}.png"))
    }

    fn binary(&self, name: &str) -> Result<PathBuf, String> {
        let file = self
            .bin_dir
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if file.is_file() {
            Ok(file)
        } else {
            Err(format!(
                "there is no {} (build it, or leave out --no-build)",
                file.display()
            ))
        }
    }

    /// The freedesktop tree below a prefix (`bin/` and `share/`), shared
    /// by the tarball and the AppImage.
    fn linux_tree(&self, prefix: &Path) -> Result<(), String> {
        for name in ["craft3d", "craft3d-cli"] {
            copy(&self.binary(name)?, &prefix.join("bin").join(name), true)?;
        }
        let share = prefix.join("share");
        let linux = |f: &str| self.packaging(&format!("linux/{f}"));
        copy(
            &linux(&format!("{APP_ID}.desktop")),
            &share.join(format!("applications/{APP_ID}.desktop")),
            false,
        )?;
        copy(
            &linux(&format!("{APP_ID}.xml")),
            &share.join(format!("mime/packages/{APP_ID}.xml")),
            false,
        )?;
        copy(
            &linux(&format!("{APP_ID}.metainfo.xml")),
            &share.join(format!("metainfo/{APP_ID}.metainfo.xml")),
            false,
        )?;
        for size in ICON_SIZES.iter().filter(|s| **s <= 512) {
            copy(
                &self.icon(*size),
                &share.join(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png")),
                false,
            )?;
        }
        copy(
            &self.packaging("icons/craft3d.svg"),
            &share.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")),
            false,
        )
    }

    fn tarball(&self) -> Result<PathBuf, String> {
        let name = format!("3dcraft-{}-{}-linux", self.version, self.arch);
        let root = self.stage.join(&name);
        self.linux_tree(&root)?;
        for script in ["install.sh", "uninstall.sh"] {
            copy(
                &self.packaging(&format!("linux/{script}")),
                &root.join(script),
                true,
            )?;
        }
        copy(&self.repo.join("LICENSE"), &root.join("LICENSE"), false)?;
        mkdir(&self.out)?;
        let file = self.out.join(format!("{name}.tar.gz"));
        tool(
            Command::new("tar")
                .arg("-C")
                .arg(&self.stage)
                .arg("-czf")
                .arg(&file)
                .arg(&name),
            "tar",
        )?;
        Ok(file)
    }

    fn appimage(&self, given: Option<&Path>, runtime: Option<&Path>) -> Result<PathBuf, String> {
        let tool_path = given
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("APPIMAGETOOL").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("appimagetool"));
        // The runtime the AppImage starts with. Without one, appimagetool
        // downloads the newest continuous build of the type2 runtime.
        let runtime = runtime
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("APPIMAGE_RUNTIME").map(PathBuf::from));
        if let Some(r) = runtime.as_ref().filter(|r| !r.is_file()) {
            return Err(format!("there is no AppImage runtime {}", r.display()));
        }
        let app_dir = self.stage.join("AppDir");
        self.linux_tree(&app_dir.join("usr"))?;
        copy(
            &self.packaging("linux/AppRun"),
            &app_dir.join("AppRun"),
            true,
        )?;
        copy(
            &self.packaging(&format!("linux/{APP_ID}.desktop")),
            &app_dir.join(format!("{APP_ID}.desktop")),
            false,
        )?;
        copy(
            &self.icon(256),
            &app_dir.join(format!("{APP_ID}.png")),
            false,
        )?;
        copy(&self.icon(256), &app_dir.join(".DirIcon"), false)?;
        copy(&self.repo.join("LICENSE"), &app_dir.join("LICENSE"), false)?;
        mkdir(&self.out)?;
        let file = self
            .out
            .join(format!("3DCraft-{}-{}.AppImage", self.version, self.arch));
        let mut command = Command::new(&tool_path);
        command
            .env("ARCH", &self.arch)
            // Runs without FUSE (containers, CI runners).
            .env("APPIMAGE_EXTRACT_AND_RUN", "1")
            .arg("--no-appstream");
        if let Some(r) = &runtime {
            command.arg("--runtime-file").arg(r);
        }
        tool(
            command.arg(&app_dir).arg(&file),
            &format!(
                "appimagetool ({}; pass --appimagetool PATH or APPIMAGETOOL, or --formats tar)",
                tool_path.display()
            ),
        )?;
        Ok(file)
    }

    fn app_path(&self) -> PathBuf {
        self.stage.join("dmg").join("3DCraft.app")
    }

    fn app_bundle(&self) -> Result<PathBuf, String> {
        let app = self.app_path();
        if app.exists() {
            return Ok(app);
        }
        let contents = app.join("Contents");
        for name in ["craft3d", "craft3d-cli"] {
            copy(
                &self.binary(name)?,
                &contents.join("MacOS").join(name),
                true,
            )?;
        }
        let plist = String::from_utf8(read(&self.packaging("macos/Info.plist"))?)
            .map_err(|_| "packaging/macos/Info.plist is not UTF-8".to_string())?;
        write(
            &contents.join("Info.plist"),
            plist.replace("@VERSION@", &self.version).as_bytes(),
        )?;
        write(&contents.join("PkgInfo"), b"APPL????")?;
        let icns = icns(&self.icon_pngs(&ICNS_SIZES)?)?;
        write(&contents.join("Resources").join("craft3d.icns"), &icns)?;
        copy(
            &self.repo.join("LICENSE"),
            &contents.join("Resources").join("LICENSE"),
            false,
        )?;
        // An ad-hoc signature over the whole bundle: not a Developer ID
        // (the app is not notarised, docs/development.md), but Apple
        // Silicon runs only signed code and a bundle edited after linking
        // must be signed again.
        if cfg!(target_os = "macos") {
            tool(
                Command::new("codesign")
                    .args(["--force", "--deep", "--sign", "-"])
                    .arg(&app),
                "codesign",
            )?;
        }
        Ok(app)
    }

    fn dmg(&self) -> Result<PathBuf, String> {
        let app = self.app_bundle()?;
        let folder = app
            .parent()
            .map(Path::to_path_buf)
            .ok_or("the app bundle has no folder")?;
        // A link to /Applications beside the app, to drag it onto.
        #[cfg(unix)]
        {
            let link = folder.join("Applications");
            if !link.exists() {
                std::os::unix::fs::symlink("/Applications", &link)
                    .map_err(|e| format!("cannot link {}: {e}", link.display()))?;
            }
        }
        mkdir(&self.out)?;
        let file = self
            .out
            .join(format!("3DCraft-{}-{}.dmg", self.version, self.arch));
        tool(
            Command::new("hdiutil")
                .args([
                    "create", "-volname", "3DCraft", "-fs", "HFS+", "-format", "UDZO", "-ov",
                ])
                .arg("-srcfolder")
                .arg(&folder)
                .arg(&file),
            "hdiutil",
        )?;
        Ok(file)
    }

    fn windows_dir(&self) -> Result<PathBuf, String> {
        let dir = self
            .stage
            .join(format!("3dcraft-{}-{}-windows", self.version, self.arch));
        if dir.exists() {
            return Ok(dir);
        }
        for name in ["craft3d", "craft3d-cli"] {
            let exe = self.binary(name)?;
            let file_name = exe.file_name().ok_or("a binary without a name")?;
            copy(&exe, &dir.join(file_name), true)?;
        }
        copy(&self.repo.join("LICENSE"), &dir.join("LICENSE.txt"), false)?;
        let ico = ico(&self.icon_pngs(&ICO_SIZES)?)?;
        write(&dir.join("craft3d.ico"), &ico)?;
        Ok(dir)
    }

    fn zip(&self) -> Result<PathBuf, String> {
        let dir = self.windows_dir()?;
        let name = dir
            .file_name()
            .ok_or("the staging folder has no name")?
            .to_os_string();
        mkdir(&self.out)?;
        let mut file = self.out.join(&name);
        file.set_extension("zip");
        // bsdtar (Windows 10 and later, macOS) writes a zip for `-a` and a
        // `.zip` name.
        tool(
            Command::new("tar")
                .arg("-a")
                .arg("-c")
                .arg("-f")
                .arg(&file)
                .arg("-C")
                .arg(&self.stage)
                .arg(&name),
            "tar (a zip)",
        )?;
        Ok(file)
    }

    fn msi(&self) -> Result<PathBuf, String> {
        let dir = self.windows_dir()?;
        mkdir(&self.out)?;
        let file = self
            .out
            .join(format!("3DCraft-{}-{}.msi", self.version, self.arch));
        let wix_arch = match self.arch.as_str() {
            "aarch64" => "arm64",
            "x86" | "i686" => "x86",
            _ => "x64",
        };
        tool(
            Command::new("wix")
                .arg("build")
                .arg(self.packaging("windows/craft3d.wxs"))
                .args(["-arch", wix_arch])
                .arg("-d")
                .arg(format!("Version={}", msi_version(&self.version)?))
                .arg("-d")
                .arg(format!("BinDir={}", dir.display()))
                .arg("-d")
                .arg(format!("Icon={}", dir.join("craft3d.ico").display()))
                .arg("-d")
                .arg(format!("Licence={}", dir.join("LICENSE.txt").display()))
                .arg("-o")
                .arg(&file),
            "wix (the WiX Toolset v5: `dotnet tool install --global wix --version 5.0.2`, \
             or --formats zip)",
        )?;
        Ok(file)
    }

    fn icon_pngs(&self, sizes: &[u32]) -> Result<Vec<Vec<u8>>, String> {
        sizes.iter().map(|s| read(&self.icon(*s))).collect()
    }
}

/// The width and height of a PNG, from its IHDR chunk.
fn png_size(png: &[u8]) -> Result<(u32, u32), String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let be = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = png.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    };
    if png.get(..8) != Some(SIGNATURE) || png.get(12..16) != Some(b"IHDR".as_slice()) {
        return Err("not a PNG file".to_string());
    }
    match (be(16), be(20)) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err("a PNG without a size".to_string()),
    }
}

fn len_u32(n: usize) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| "an icon too large for its container".to_string())
}

/// A Windows icon (`.ico`) holding square PNG images (Windows Vista and
/// later read PNG entries): the ICONDIR header, one 16-byte entry per
/// image, then the images.
fn ico(pngs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let count = u16::try_from(pngs.len()).map_err(|_| "too many icon images".to_string())?;
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    let mut offset = 6 + 16 * pngs.len();
    for png in pngs {
        let (w, h) = png_size(png)?;
        if w != h || w > 256 {
            return Err(format!(
                "an .ico image must be square and at most 256: {w}x{h}"
            ));
        }
        // 256 is written as 0.
        let side = u8::try_from(w).unwrap_or(0);
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend(len_u32(png.len())?.to_le_bytes());
        out.extend(len_u32(offset)?.to_le_bytes());
        offset = offset.saturating_add(png.len());
    }
    for png in pngs {
        out.extend(png);
    }
    Ok(out)
}

/// The icns type that holds a PNG of this many pixels a side. Sizes with
/// two types (32: icp5 and ic11, the @2x of 16) get the 1x one; `icns`
/// adds the @2x entries itself.
fn icns_types(side: u32) -> &'static [&'static [u8; 4]] {
    match side {
        16 => &[b"icp4"],
        32 => &[b"icp5", b"ic11"],
        64 => &[b"icp6", b"ic12"],
        128 => &[b"ic07"],
        256 => &[b"ic08", b"ic13"],
        512 => &[b"ic09", b"ic14"],
        1024 => &[b"ic10"],
        _ => &[],
    }
}

/// A macOS icon (`.icns`) holding PNG images: the `icns` header with the
/// file's length, then per image its type, its length with the 8-byte
/// entry header, and the PNG.
fn icns(pngs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    for png in pngs {
        let (w, h) = png_size(png)?;
        let types = icns_types(w);
        if w != h || types.is_empty() {
            return Err(format!("no icns type for a {w}x{h} image"));
        }
        for kind in types {
            body.extend(kind.as_slice());
            body.extend(len_u32(png.len().saturating_add(8))?.to_be_bytes());
            body.extend(png);
        }
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend(b"icns");
    out.extend(len_u32(body.len().saturating_add(8))?.to_be_bytes());
    out.extend(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_png(side: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend(side.to_be_bytes());
        png.extend(side.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        png
    }

    #[test]
    fn arguments_are_read() {
        let args: Vec<String> = ["--formats", "tar", "-j", "2", "--no-build"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = parse(&args).unwrap();
        assert_eq!(o.formats, Some(vec!["tar".to_string()]));
        assert_eq!(o.jobs.as_deref(), Some("2"));
        assert!(!o.build);
        assert!(parse(&["--bogus".to_string()]).is_err());
        assert!(parse(&["--target".to_string()]).is_err());
    }

    #[test]
    fn a_failed_tool_says_what_it_wrote() {
        let m = failure(
            "appimagetool",
            "exit status: 1",
            "appimagetool, build 172\n",
            "desktop-file-validate command is missing, please install it\n",
        );
        assert!(m.starts_with("appimagetool failed (exit status: 1); it wrote:"));
        assert!(m.contains("\n  appimagetool, build 172"));
        assert!(m.ends_with("\n  desktop-file-validate command is missing, please install it"));
        let quiet = failure("tar", "exit status: 2", "", "\n");
        assert_eq!(quiet, "tar failed (exit status: 2) and wrote nothing");
        let long: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let m = failure("wix", "exit status: 1", &long, "");
        assert_eq!(m.lines().count(), 1 + FAILURE_LINES);
        assert!(m.contains("line 80") && !m.contains("line 79"));
    }

    #[test]
    fn the_appimage_runtime_is_read() {
        let args: Vec<String> = ["--appimage-runtime", "target/tools/runtime"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = parse(&args).unwrap();
        assert_eq!(
            o.appimage_runtime,
            Some(PathBuf::from("target/tools/runtime"))
        );
        assert!(parse(&["--appimage-runtime".to_string()]).is_err());
    }

    #[test]
    fn architecture_comes_from_the_triple() {
        assert_eq!(arch_of(Some("aarch64-apple-darwin")), "aarch64");
        assert_eq!(arch_of(None), std::env::consts::ARCH);
    }

    #[test]
    fn msi_versions_are_three_numbers() {
        assert_eq!(msi_version("0.0.1").unwrap(), "0.0.1");
        assert_eq!(msi_version("1.2.3-beta.1").unwrap(), "1.2.3");
        assert!(msi_version("1.2").is_err());
        assert!(msi_version("1.x.3").is_err());
        assert!(msi_version("1.2.70000").is_err());
    }

    #[test]
    fn png_sizes_are_read_and_junk_is_refused() {
        assert_eq!(png_size(&fake_png(48)).unwrap(), (48, 48));
        assert!(png_size(b"GIF89a").is_err());
        assert!(png_size(&fake_png(48)[..18]).is_err());
    }

    #[test]
    fn ico_has_a_directory_and_offsets() {
        let pngs = vec![fake_png(16), fake_png(256)];
        let ico = ico(&pngs).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        // The 256 image writes 0 as its side.
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0);
        let first = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(first, 6 + 32);
        assert_eq!(&ico[first..first + 8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(ico.len(), 6 + 32 + pngs[0].len() + pngs[1].len());
        assert!(super::ico(&[fake_png(512)]).is_err());
    }

    #[test]
    fn icns_entries_add_up_to_the_file_length() {
        let pngs = vec![fake_png(16), fake_png(32), fake_png(1024)];
        let icns = icns(&pngs).unwrap();
        assert_eq!(&icns[..4], b"icns");
        let total = u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize;
        assert_eq!(total, icns.len());
        // Walk the entries: icp4, icp5, ic11, ic10.
        let mut at = 8;
        let mut kinds = Vec::new();
        while at < icns.len() {
            kinds.push(String::from_utf8(icns[at..at + 4].to_vec()).unwrap());
            at += u32::from_be_bytes(icns[at + 4..at + 8].try_into().unwrap()) as usize;
        }
        assert_eq!(at, icns.len());
        assert_eq!(kinds, ["icp4", "icp5", "ic11", "ic10"]);
        assert!(super::icns(&[fake_png(20)]).is_err());
    }

    /// The icons in the repository make the `.ico` and `.icns` the
    /// packages carry: every PNG they take is there, square and in size.
    #[test]
    fn the_repository_icons_make_ico_and_icns() {
        let root = repo_root().join("packaging/icons");
        let pngs = |sizes: &[u32]| -> Vec<Vec<u8>> {
            sizes
                .iter()
                .map(|n| std::fs::read(root.join(format!("craft3d-{n}.png"))).unwrap())
                .collect()
        };
        let ico = ico(&pngs(&ICO_SIZES)).unwrap();
        assert_eq!(usize::from(ico[4]), ICO_SIZES.len());
        let icns = icns(&pngs(&ICNS_SIZES)).unwrap();
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize,
            icns.len()
        );
    }

    /// The files the packages carry are in the repository and agree with
    /// each other on the identifier.
    #[test]
    fn packaging_files_agree_on_the_identifier() {
        let root = repo_root().join("packaging");
        for size in ICON_SIZES {
            let png = std::fs::read(root.join(format!("icons/craft3d-{size}.png"))).unwrap();
            assert_eq!(png_size(&png).unwrap(), (size, size), "icon {size}");
        }
        let desktop =
            std::fs::read_to_string(root.join(format!("linux/{APP_ID}.desktop"))).unwrap();
        assert!(desktop.contains(&format!("Icon={APP_ID}\n")));
        assert!(desktop.contains("MimeType=application/x-craft3d;"));
        let mime = std::fs::read_to_string(root.join(format!("linux/{APP_ID}.xml"))).unwrap();
        assert!(mime.contains("type=\"application/x-craft3d\""));
        assert!(mime.contains("*.3dcraft"));
        let plist = std::fs::read_to_string(root.join("macos/Info.plist")).unwrap();
        assert!(plist.contains(&format!("<string>{APP_ID}</string>")));
        assert!(plist.contains("<string>3dcraft</string>"));
        assert!(plist.contains("@VERSION@"));
        let wxs = std::fs::read_to_string(root.join("windows/craft3d.wxs")).unwrap();
        assert!(wxs.contains("Extension Id=\"3dcraft\""));
        // The window's app id is the desktop file's name, so a Wayland
        // compositor finds the icon and the entry.
        let app = std::fs::read_to_string(repo_root().join("apps/craft3d/src/app.rs")).unwrap();
        assert!(app.contains(&format!("pub const APP_ID: &str = \"{APP_ID}\";")));
        assert!(app.contains(".with_app_id(APP_ID)"));
    }
}
