//! `cargo xtask package --formats web`: the browser build of 3DCraft as a
//! static site in `3dcraft-web-<version>.zip` (docs/development.md, Web
//! build), packed the way the other Craft apps pack theirs:
//!
//! ```text
//! 3dcraft-web-<version>/
//!   index.html                       relative URLs only: any path works
//!   craft3d-web-<hash>.js            wasm-bindgen's glue (an ES module)
//!   craft3d-web-<hash>_bg.wasm       the app
//!   craft3d-worker-<hash>.js         the checking worker (a module worker
//!                                    that loads the glue), named by the page
//!   manifest.webmanifest, icon-256.png, icon-512.png   installable app
//!   HOSTING.md, _headers, .htaccess  server settings
//!   environments/*.hdr, LICENSE.md   the studio look's environments (CC0),
//!                                    fetched when chosen
//!   LICENSE, THIRD-PARTY-NOTICES.txt
//! ```
//!
//! `<hash>` is 16 lower-case hex digits (the first of the SHA-256 of the
//! module and its glue), so a server can cache the two forever by their
//! names (`-[0-9a-f]{16}(_bg)?\.(wasm|js)$`) and revalidate the rest.
//! `SHA256SUMS.txt` beside the zip holds its checksum, which is what an
//! updater checks before it unpacks it.
//!
//! The steps, without Trunk: `cargo build --profile web` (release
//! optimised for size, except the crates whose speed the user waits for)
//! for `wasm32-unknown-unknown`,
//! `wasm-bindgen --target web` (the CLI of exactly the version in
//! `Cargo.lock`), `wasm-opt` when it is there (it only makes the module
//! smaller), the files of `packaging/web` with the names filled in, and
//! `zip`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The package's name in file names.
const NAME: &str = "craft3d-web";
/// The cargo profile of the browser build (the root `Cargo.toml`): release
/// optimised for size, except the modelling and meshing crates.
const PROFILE: &str = "web";
/// The theme colour of the installed app and its address bar: the
/// accent of `craft3d_ui_egui::Tokens` (`accent`), and the page behind the
/// canvas while it loads: its viewport background.
const THEME_COLOR: &str = "#e0a93e";
const BACKGROUND: &str = "#ebe9e5";

/// The tools, as the command line or the environment names them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub wasm_bindgen: Option<PathBuf>,
    pub wasm_opt: Option<PathBuf>,
}

/// The version of `package` in `Cargo.lock`.
pub fn locked_version(lock: &str, package: &str) -> Option<String> {
    let wanted = format!("name = \"{package}\"");
    let mut lines = lock.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line == wanted {
            let version = lines.next()?;
            return version
                .strip_prefix("version = \"")?
                .strip_suffix('"')
                .map(str::to_string);
        }
    }
    None
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The name part shared by the module and its glue: the first 16 hex
/// digits of the SHA-256 of both.
pub fn content_hash(wasm: &[u8], js: &[u8]) -> String {
    let mut both = Vec::with_capacity(wasm.len() + js.len());
    both.extend_from_slice(wasm);
    both.extend_from_slice(js);
    hex(&craft3d_doc::sha256(&both)).chars().take(16).collect()
}

/// A line of `SHA256SUMS.txt` (as `sha256sum` writes it).
pub fn sums_line(bytes: &[u8], name: &str) -> String {
    format!("{}  {name}\n", hex(&craft3d_doc::sha256(bytes)))
}

/// `SHA256SUMS.txt` with the line of `name` put in (replacing an older
/// one), the lines in order of their names.
pub fn with_sum(existing: &str, line: &str, name: &str) -> String {
    let mut lines: Vec<&str> = existing
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            l.split_whitespace()
                .nth(1)
                .map(|n| n.trim_start_matches('*'))
                != Some(name)
        })
        .collect();
    lines.push(line.trim_end());
    lines.sort_by_key(|l| l.split_whitespace().nth(1).unwrap_or_default().to_string());
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The checking worker's file name: the first 16 hex digits of the
/// SHA-256 of the worker as filled in (which names the glue, whose name
/// carries the module's hash), so it is cached forever like them.
pub fn worker_name(worker: &[u8]) -> String {
    let hash: String = hex(&craft3d_doc::sha256(worker)).chars().take(16).collect();
    format!("craft3d-worker-{hash}.js")
}

/// A template of `packaging/web` with its names filled in.
pub fn fill(template: &str, js: &str, wasm: &str, worker: &str, version: &str) -> String {
    template
        .replace("@JS@", js)
        .replace("@WASM@", wasm)
        .replace("@WORKER@", worker)
        .replace("@VERSION@", version)
        .replace("@THEME_COLOR@", THEME_COLOR)
        .replace("@BACKGROUND@", BACKGROUND)
}

/// Builds the module the site carries: `cargo build` (when `build` is
/// given, with those extra arguments), `wasm-bindgen`, then `wasm-opt`.
/// Returns the module and its glue. `target` is cargo's target directory,
/// `stage` a directory of its own to work in. Without `require_opt` a
/// `wasm-opt` the caller did not name may be missing (the module is then
/// left as wasm-bindgen made it); with it, that is an error, since
/// `cargo xtask web-size` measures the module that is shipped.
pub fn module(
    repo: &Path,
    target: &Path,
    stage: &Path,
    tools: &Tools,
    build: Option<&[&str]>,
    require_opt: bool,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    if let Some(extra) = build {
        let mut cargo = vec![
            "build",
            "--profile",
            PROFILE,
            "--locked",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            NAME,
        ];
        cargo.extend(extra);
        crate::cargo(&cargo)?;
    }
    let built = target
        .join("wasm32-unknown-unknown")
        .join(PROFILE)
        .join("craft3d_web.wasm");
    if !built.is_file() {
        return Err(format!(
            "there is no {} (build it, or leave out --no-build)",
            built.display()
        ));
    }
    let lock = std::fs::read_to_string(repo.join("Cargo.lock"))
        .map_err(|e| format!("cannot read Cargo.lock: {e}"))?;
    let wanted = locked_version(&lock, "wasm-bindgen")
        .ok_or("Cargo.lock has no wasm-bindgen; is craft3d-web in the workspace?")?;
    let bindgen = tools
        .wasm_bindgen
        .clone()
        .or_else(|| std::env::var_os("WASM_BINDGEN").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("wasm-bindgen"));
    let install = format!(
        "the wasm-bindgen CLI {wanted} (`cargo install wasm-bindgen-cli --version {wanted} --locked`, \
         or a release from github.com/wasm-bindgen/wasm-bindgen; pass --wasm-bindgen PATH or set WASM_BINDGEN)"
    );
    let said = Command::new(&bindgen)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run {} ({e}); it needs {install}", bindgen.display()))?;
    let said = String::from_utf8_lossy(&said.stdout).to_string();
    if !said.split_whitespace().any(|word| word == wanted) {
        return Err(format!(
            "{} is `{}`; the module needs {install}",
            bindgen.display(),
            said.trim()
        ));
    }
    let bound = stage.join("bindgen");
    if bound.exists() {
        std::fs::remove_dir_all(&bound)
            .map_err(|e| format!("cannot clear {}: {e}", bound.display()))?;
    }
    crate::package::tool(
        Command::new(&bindgen)
            .args(["--target", "web", "--no-typescript", "--out-name", NAME])
            .arg("--out-dir")
            .arg(&bound)
            .arg(&built),
        "wasm-bindgen",
    )?;
    let mut wasm_file = bound.join(format!("{NAME}_bg.wasm"));
    let js_file = bound.join(format!("{NAME}.js"));
    // wasm-opt is optional: without it the module is a fifth larger.
    let given = tools
        .wasm_opt
        .clone()
        .or_else(|| std::env::var_os("WASM_OPT").map(PathBuf::from));
    let optimiser = given.clone().unwrap_or_else(|| PathBuf::from("wasm-opt"));
    let optimised = bound.join(format!("{NAME}_opt.wasm"));
    let mut command = Command::new(&optimiser);
    command
        .args([
            "-O2",
            "--enable-bulk-memory",
            "--enable-mutable-globals",
            "--enable-nontrapping-float-to-int",
            "--enable-sign-ext",
            "--enable-reference-types",
            "--enable-multivalue",
        ])
        .arg(&wasm_file)
        .arg("-o")
        .arg(&optimised);
    eprintln!("$ {command:?}");
    match command.output() {
        Ok(output) if output.status.success() => wasm_file = optimised,
        Ok(output) => {
            return Err(crate::package::failure(
                "wasm-opt",
                &output.status.to_string(),
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Err(e) if !require_opt && given.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "package: no wasm-opt on PATH; the module is left as wasm-bindgen made it \
                 (binaryen's wasm-opt makes it smaller: --wasm-opt PATH or WASM_OPT)"
            );
        }
        Err(e) => return Err(format!("cannot run {} ({e})", optimiser.display())),
    }
    let wasm = crate::package::read(&wasm_file)?;
    let js = crate::package::read(&js_file)?;
    Ok((wasm, js))
}

/// Makes `out/3dcraft-web-<version>.zip` and puts its line into
/// `out/SHA256SUMS.txt`. `target` is cargo's target directory, `stage` a
/// directory of its own to work in, `notices` the written
/// `THIRD-PARTY-NOTICES.txt`.
/// What a release's site is made of besides the module: its version and
/// the third-party notices written for it.
pub struct Release<'a> {
    pub version: &'a str,
    pub notices: &'a Path,
}

pub fn package(
    repo: &Path,
    target: &Path,
    stage: &Path,
    out: &Path,
    release: &Release,
    tools: &Tools,
    build: Option<&[&str]>,
) -> Result<PathBuf, String> {
    let (version, notices) = (release.version, release.notices);
    let (wasm, js) = module(repo, target, stage, tools, build, false)?;
    let hash = content_hash(&wasm, &js);
    let (js_name, wasm_name) = (
        format!("{NAME}-{hash}.js"),
        format!("{NAME}-{hash}_bg.wasm"),
    );

    let folder = format!("3dcraft-web-{version}");
    let site = stage.join(&folder);
    if site.exists() {
        std::fs::remove_dir_all(&site)
            .map_err(|e| format!("cannot clear {}: {e}", site.display()))?;
    }
    let web = repo.join("packaging").join("web");
    crate::package::write(&site.join(&js_name), &js)?;
    crate::package::write(&site.join(&wasm_name), &wasm)?;
    let worker_template = String::from_utf8(crate::package::read(&web.join("worker.js"))?)
        .map_err(|_| "packaging/web/worker.js is not UTF-8".to_string())?;
    let worker = fill(&worker_template, &js_name, &wasm_name, "", version);
    let worker_file = worker_name(worker.as_bytes());
    crate::package::write(&site.join(&worker_file), worker.as_bytes())?;
    for (from, to) in [
        ("index.html", "index.html"),
        ("manifest.webmanifest", "manifest.webmanifest"),
        ("HOSTING.md", "HOSTING.md"),
        ("_headers", "_headers"),
        (".htaccess", ".htaccess"),
    ] {
        let template = String::from_utf8(crate::package::read(&web.join(from))?)
            .map_err(|_| format!("packaging/web/{from} is not UTF-8"))?;
        let filled = fill(&template, &js_name, &wasm_name, &worker_file, version);
        crate::package::write(&site.join(to), filled.as_bytes())?;
    }
    for size in [256, 512] {
        let icon = crate::package::read(
            &repo
                .join("packaging")
                .join("icons")
                .join(format!("craft3d-{size}.png")),
        )?;
        crate::package::write(&site.join(format!("icon-{size}.png")), &icon)?;
    }
    let licence = crate::package::read(&repo.join("LICENSE"))?;
    crate::package::write(&site.join("LICENSE"), &licence)?;
    let notices = crate::package::read(notices)?;
    crate::package::write(&site.join(crate::notices::FILE_NAME), &notices)?;
    // The studio look's environments, fetched by the page when chosen
    // (not in the module: they would weigh on every load).
    // The built-in fonts, fetched by the page when a text first uses one
    // (not in the module: the web budget, docs/design/text.md).
    crate::package::copy_fonts(&site.join("fonts"))?;
    let environments = repo.join("assets").join("environments");
    for name in ["studio.hdr", "outdoor.hdr", "workshop.hdr", "LICENSE.md"] {
        let bytes = crate::package::read(&environments.join(name))?;
        crate::package::write(&site.join("environments").join(name), &bytes)?;
    }

    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let zip_name = format!("{folder}.zip");
    let zip = out.join(&zip_name);
    if zip.exists() {
        std::fs::remove_file(&zip).map_err(|e| format!("cannot replace {}: {e}", zip.display()))?;
    }
    crate::package::tool(
        Command::new("zip")
            .current_dir(stage)
            .args(["-r", "-X", "-9", "-q"])
            .arg(&zip)
            .arg(&folder),
        "zip (Info-ZIP; `apt install zip`)",
    )?;
    let bytes = crate::package::read(&zip)?;
    let sums = out.join("SHA256SUMS.txt");
    let existing = std::fs::read_to_string(&sums).unwrap_or_default();
    let updated = with_sum(&existing, &sums_line(&bytes, &zip_name), &zip_name);
    crate::package::write(&sums, updated.as_bytes())?;
    eprintln!(
        "package: {zip_name}: {} bytes; the module {} bytes ({wasm_name})",
        bytes.len(),
        wasm.len()
    );
    Ok(zip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_locked_version_is_read() {
        let lock = "[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.129\"\nsource = \"x\"\n\n[[package]]\nname = \"wasm-bindgen-futures\"\nversion = \"0.4.79\"\n";
        assert_eq!(
            locked_version(lock, "wasm-bindgen").as_deref(),
            Some("0.2.129")
        );
        assert_eq!(
            locked_version(lock, "wasm-bindgen-futures").as_deref(),
            Some("0.4.79")
        );
        assert_eq!(locked_version(lock, "web-sys"), None);
        // The workspace's own lock file has it, as the web build needs.
        let ours =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.lock"))
                .unwrap();
        assert!(locked_version(&ours, "wasm-bindgen").is_some());
    }

    #[test]
    fn hashed_names_are_what_a_server_caches_forever() {
        let hash = content_hash(b"\0asm", b"export default 1;");
        assert_eq!(hash.len(), 16);
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        // The same files give the same name; another file another one.
        assert_eq!(hash, content_hash(b"\0asm", b"export default 1;"));
        assert_ne!(hash, content_hash(b"\0asm", b"export default 2;"));
    }

    #[test]
    fn sums_are_lines_sha256sum_checks() {
        // SHA-256 of "abc" (FIPS 180-2, appendix B.1).
        let line = sums_line(b"abc", "a.zip");
        assert_eq!(
            line,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  a.zip\n"
        );
        let sums = with_sum("ffff  b.zip\n0000  a.zip\n", &line, "a.zip");
        assert_eq!(sums, format!("{}ffff  b.zip\n", line));
        assert_eq!(with_sum("", &line, "a.zip"), line);
    }

    /// The templates name what the package fills in, and the site's
    /// files are linked from the page.
    #[test]
    fn the_templates_fit_together() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/web");
        let read = |name: &str| std::fs::read_to_string(web.join(name)).unwrap();
        let index = fill(
            &read("index.html"),
            "a-0123456789abcdef.js",
            "a-0123456789abcdef_bg.wasm",
            "craft3d-worker-0123456789abcdef.js",
            "1.2.3",
        );
        assert!(!index.contains('@'), "{index}");
        assert!(index.contains("import init from './a-0123456789abcdef.js'"));
        assert!(index.contains("module_or_path: './a-0123456789abcdef_bg.wasm'"));
        // The page names its checking worker, which loads the glue.
        assert!(index.contains(
            "<meta name=\"craft3d-worker\" content=\"./craft3d-worker-0123456789abcdef.js\">"
        ));
        let worker = fill(&read("worker.js"), "a-0123456789abcdef.js", "", "", "");
        assert!(!worker.contains('@'), "{worker}");
        assert!(worker.contains("from './a-0123456789abcdef.js'"));
        assert!(worker.contains("craft3d_check"));
        assert!(worker.contains("craft3d_scene_begin") && worker.contains("craft3d_scene_next"));
        let name = worker_name(worker.as_bytes());
        assert!(
            name.starts_with("craft3d-worker-") && name.ends_with(".js") && name.len() == 34,
            "{name}"
        );
        assert_ne!(name, worker_name(b"another worker"));
        // An updater installs it as an app only without a manifest of its
        // own; ours is there, with relative addresses.
        assert!(index.contains("rel=\"manifest\""));
        assert!(
            index.contains("id=\"craft3d_canvas\"") && index.contains("id=\"craft3d_loading\"")
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&fill(&read("manifest.webmanifest"), "", "", "", "")).unwrap();
        assert_eq!(manifest["start_url"], "./");
        assert_eq!(manifest["scope"], "./");
        assert_eq!(manifest["theme_color"], THEME_COLOR);
        // The hosting notes say how to force WebGL2 (updaters look for it).
        let hosting = read("HOSTING.md");
        assert!(hosting.contains("?webgl"));
        // Every absolute URL in the page would break under a prefix.
        for attribute in ["href=\"/", "src=\"/"] {
            assert!(!index.contains(attribute), "{attribute}");
        }
    }
}
