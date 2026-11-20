//! `cargo xtask package --formats mcpb`: the MCP bundle of a release
//! (docs/development.md, Packaging › *The MCP bundle*; docs/decisions.md,
//! 2026-11-13). One `.mcpb` file, the format of the MCP Bundle
//! specification (github.com/modelcontextprotocol/mcpb, `MANIFEST.md`,
//! manifest version 0.3): a ZIP archive with `manifest.json` at its root
//! and `craft3d-cli` for Linux, macOS and Windows, which an MCP client such
//! as Claude for macOS and Windows installs with one click and starts
//! headless (`craft3d-cli mcp`) with a work folder the person chooses. And
//! beside it `server.json`, the entry of the official MCP registry
//! (`registry.modelcontextprotocol.io`, `io.github.johancoppens/3dcraft`)
//! that points at the bundle as a release asset, with its SHA-256: made by
//! the build, published by the developer.
//!
//! The programs are given, not built: the Release workflow's job
//! `package (mcpb)` passes the `craft3d-cli` of each of its three package
//! jobs (`--cli-linux`, `--cli-macos`, `--cli-windows`), and each is checked
//! to be a program for its platform before it is packed. The archive is
//! written by `craft3d-zipstore` (deflated by flate2, a Unix mode on every
//! entry so the programs unpack executable) and read back before it is
//! kept.
//!
//! What the bundle holds:
//!
//! ```text
//! manifest.json                     packaging/mcpb/manifest.json, the version filled in
//! icon.png                          packaging/icons/craft3d-512.png
//! LICENSE, THIRD-PARTY-NOTICES.txt, EXAMPLES-LICENSE.md
//! server/linux/craft3d-cli          x86-64
//! server/darwin/craft3d-cli         Apple Silicon (arm64)
//! server/win32/craft3d-cli.exe      x86-64
//! server/share/craft3d/fonts/       the shipped fonts with their licences
//! ```
//!
//! The fonts are `../share/craft3d/fonts` from each program's folder, one
//! of the places the programs look (`crates/engine/src/fonts.rs`), so one
//! copy serves the three.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The manifest, with `@VERSION@` (the workspace version) and `@SERIES@`
/// (its major and minor) for the build to fill in.
pub(crate) const MANIFEST_TEMPLATE: &str = "packaging/mcpb/manifest.json";

/// The registry's entry, with `@VERSION@`, `@URL@` (the bundle's address in
/// the release) and `@SHA256@` (its SHA-256) for the build to fill in.
pub(crate) const SERVER_TEMPLATE: &str = "packaging/mcpb/server.json";

/// The file the build writes the registry's entry to, beside the bundle:
/// the name `mcp-publisher publish` reads.
pub(crate) const SERVER_JSON: &str = "server.json";

/// The bundle's icon: 512 pixels, the size MCPB's validator recommends.
const ICON: &str = "packaging/icons/craft3d-512.png";

/// The name of the server in the official MCP registry (the developer,
/// 2026-11-13): a GitHub namespace, proven by signing in with GitHub.
pub(crate) const REGISTRY_NAME: &str = "io.github.johancoppens/3dcraft";

/// The manifest version the bundle is written for, and the fields its
/// schema knows (MCPB's `src/schemas/0.3.ts`, a strict object: a client
/// refuses a manifest with any other).
const MANIFEST_VERSION: &str = "0.3";
const MANIFEST_FIELDS: [&str; 28] = [
    "$schema",
    "dxt_version",
    "manifest_version",
    "name",
    "display_name",
    "version",
    "description",
    "long_description",
    "author",
    "repository",
    "homepage",
    "documentation",
    "support",
    "icon",
    "icons",
    "screenshots",
    "localization",
    "server",
    "tools",
    "tools_generated",
    "prompts",
    "prompts_generated",
    "keywords",
    "license",
    "privacy_policies",
    "compatibility",
    "user_config",
    "_meta",
];

/// One program of the bundle.
pub(crate) struct Platform {
    /// The platform as MCPB names it (Node's `process.platform`): a key of
    /// `platform_overrides` and an entry of `compatibility.platforms`.
    pub(crate) name: &'static str,
    /// The option of `cargo xtask package` that gives its program.
    pub(crate) option: &'static str,
    /// The program's path in the bundle.
    pub(crate) path: &'static str,
}

/// The three programs, in the order the bundle holds them.
pub(crate) const PLATFORMS: [Platform; 3] = [
    Platform {
        name: "linux",
        option: "--cli-linux",
        path: "server/linux/craft3d-cli",
    },
    Platform {
        name: "darwin",
        option: "--cli-macos",
        path: "server/darwin/craft3d-cli",
    },
    Platform {
        name: "win32",
        option: "--cli-windows",
        path: "server/win32/craft3d-cli.exe",
    },
];

/// Where the fonts are in the bundle: `../share/craft3d/fonts` from every
/// program's folder.
const FONTS: &str = "server/share/craft3d/fonts";

/// Where a program looks for its fonts, from its own folder, that the
/// bundle uses (one of the places `crates/engine/src/fonts.rs` names).
pub(crate) const FONTS_FROM_PROGRAM: &str = "../share/craft3d/fonts";

/// The variables MCPB's validator accepts in `mcp_config` besides
/// `${user_config.<key>}` (`src/node/validate.ts`).
const VARIABLES: [&str; 3] = ["__dirname", "pathSeparator", "/"];

/// What `${__dirname}` stands for in a command: the folder the client
/// unpacked the bundle into.
const DIRNAME: &str = "${__dirname}/";

/// The bundle's file name for `version`.
pub(crate) fn file_name(version: &str) -> String {
    format!("3dcraft-{version}.mcpb")
}

/// Where the release of `version` publishes the bundle: an asset of the
/// public repository's release `v<version>`, the kind of address the
/// registry accepts for an MCPB package.
pub(crate) fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/{}/releases/download/v{version}/{}",
        crate::package::RELEASES_OWNER,
        crate::package::RELEASES_REPOSITORY,
        file_name(version)
    )
}

/// The major and minor of `version` (`0.2` of `0.2.1`), which the long
/// description names so that it need not change at every patch release.
fn series(version: &str) -> Result<String, String> {
    let mut parts = version.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) if !major.is_empty() && !minor.is_empty() => {
            Ok(format!("{major}.{minor}"))
        }
        _ => Err(format!("the version `{version}` has no major and minor")),
    }
}

/// The first `@NAME@` placeholder left in `text`, if any.
fn placeholder(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let after = rest.get(at + 1..)?;
        let len = after
            .find(|c: char| !(c.is_ascii_uppercase() || c == '_'))
            .unwrap_or(after.len());
        if len > 0 && after.get(len..)?.starts_with('@') {
            return after.get(..len).map(|name| format!("@{name}@"));
        }
        rest = after;
    }
    None
}

/// `rel` from the folder `dir` of a bundle path, with `..` taken off
/// (`server/linux` and `../share/craft3d/fonts` give
/// `server/share/craft3d/fonts`); `None` when it leaves the bundle.
pub(crate) fn resolve(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// The folder of a bundle path.
fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    field(value, path).and_then(Value::as_str)
}

/// The manifest of `version`: the template filled in and checked
/// ([`check_manifest`]).
pub(crate) fn manifest(template: &str, version: &str) -> Result<String, String> {
    let filled = template
        .replace("@VERSION@", version)
        .replace("@SERIES@", &series(version)?);
    if let Some(name) = placeholder(&filled) {
        return Err(format!(
            "{MANIFEST_TEMPLATE} has a placeholder the build does not fill in: {name}"
        ));
    }
    let value: Value = serde_json::from_str(&filled)
        .map_err(|e| format!("{MANIFEST_TEMPLATE} is not JSON: {e}"))?;
    check_manifest(&value, version)?;
    Ok(filled)
}

/// Refuses a manifest MCPB's validator or a client would refuse, or that
/// does not start this bundle's programs as decided: a field the schema of
/// its version does not know, another version than the workspace's, a
/// platform whose command is not its program in the bundle (or one not
/// listed under `compatibility.platforms`), an entry point that is none of
/// the programs, a variable the validator does not know or a work folder
/// not asked for, and a server that is not the headless one (`mcp`, not
/// `--bridge`). Every problem is named.
pub(crate) fn check_manifest(manifest: &Value, version: &str) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    let Some(fields) = manifest.as_object() else {
        return Err("the manifest is not a JSON object".to_string());
    };
    for key in fields.keys() {
        if !MANIFEST_FIELDS.contains(&key.as_str()) {
            problems.push(format!(
                "`{key}` is no field of an MCPB {MANIFEST_VERSION} manifest"
            ));
        }
    }
    if text(manifest, &["manifest_version"]) != Some(MANIFEST_VERSION) {
        problems.push(format!("manifest_version is not \"{MANIFEST_VERSION}\""));
    }
    if text(manifest, &["version"]) != Some(version) {
        problems.push(format!("version is not the workspace's, {version}"));
    }
    for required in ["name", "description"] {
        if text(manifest, &[required]).is_none_or(str::is_empty) {
            problems.push(format!("{required} is missing"));
        }
    }
    if text(manifest, &["author", "name"]).is_none() {
        problems.push("author.name is missing".to_string());
    }
    if text(manifest, &["server", "type"]) != Some("binary") {
        problems.push("server.type is not \"binary\"".to_string());
    }
    let entry = text(manifest, &["server", "entry_point"]).unwrap_or_default();
    if !PLATFORMS.iter().any(|p| p.path == entry) {
        problems.push(format!(
            "server.entry_point `{entry}` is none of the programs"
        ));
    }
    let config = field(manifest, &["server", "mcp_config"]);
    let overrides = config
        .and_then(|c| c.get("platform_overrides"))
        .and_then(Value::as_object);
    for name in overrides.into_iter().flat_map(|o| o.keys()) {
        if !PLATFORMS.iter().any(|p| p.name == name.as_str()) {
            problems.push(format!(
                "platform_overrides names `{name}`, no platform of the bundle"
            ));
        }
    }
    for platform in &PLATFORMS {
        let command = overrides
            .and_then(|o| o.get(platform.name))
            .and_then(|o| o.get("command"))
            .or_else(|| config.and_then(|c| c.get("command")))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let wanted = format!("{DIRNAME}{}", platform.path);
        if command != wanted {
            problems.push(format!(
                "on {} the client starts `{command}`, not the bundle's program `{wanted}`",
                platform.name
            ));
        }
    }
    let listed: Vec<&str> = field(manifest, &["compatibility", "platforms"])
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for platform in &PLATFORMS {
        if !listed.contains(&platform.name) {
            problems.push(format!(
                "compatibility.platforms does not list {}",
                platform.name
            ));
        }
    }
    if listed.len() != PLATFORMS.len() {
        problems.push(format!(
            "compatibility.platforms lists {listed:?}, not the bundle's three"
        ));
    }
    let args: Vec<&str> = config
        .and_then(|c| c.get("args"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if args.first() != Some(&"mcp") || args.contains(&"--bridge") {
        problems.push(format!(
            "the server is started with {args:?}: the bundle starts `craft3d-cli mcp` without \
             a window, never --bridge (the developer, 2026-11-13)"
        ));
    }
    for root in ["--automation-read-root", "--automation-write-root"] {
        let value = args
            .iter()
            .position(|a| *a == root)
            .and_then(|at| args.get(at + 1));
        if value != Some(&"${user_config.work_folder}") {
            problems.push(format!("{root} is not the work folder the person chooses"));
        }
    }
    let user_config = field(manifest, &["user_config"]).and_then(Value::as_object);
    let mut strings: Vec<&str> = Vec::new();
    for each in config
        .into_iter()
        .chain(overrides.into_iter().flat_map(|o| o.values()))
    {
        strings.extend(each.get("command").and_then(Value::as_str));
        if let Some(args) = each.get("args").and_then(Value::as_array) {
            strings.extend(args.iter().filter_map(Value::as_str));
        }
        if let Some(env) = each.get("env").and_then(Value::as_object) {
            strings.extend(env.values().filter_map(Value::as_str));
        }
    }
    for string in strings {
        let mut rest = string;
        while let Some((_, after)) = rest.split_once("${") {
            let Some((name, tail)) = after.split_once('}') else {
                problems.push(format!("`{string}` has a `${{` that is not closed"));
                break;
            };
            rest = tail;
            if VARIABLES.contains(&name) {
                continue;
            }
            let key = name.strip_prefix("user_config.");
            let option = key.and_then(|k| user_config.and_then(|u| u.get(k)));
            match option {
                Some(option) if option.get("required").and_then(Value::as_bool) == Some(true) => {}
                Some(_) => problems.push(format!(
                    "`${{{name}}}` is not required: the server cannot start without it"
                )),
                None => problems.push(format!(
                    "`${{{name}}}` is no variable MCPB knows nor a user_config of the manifest"
                )),
            }
        }
    }
    if text(manifest, &["user_config", "work_folder", "type"]) != Some("directory") {
        problems.push("user_config.work_folder is not a directory".to_string());
    }
    if text(manifest, &["icon"]) != Some("icon.png") {
        problems.push("icon is not icon.png".to_string());
    }
    let tools = field(manifest, &["tools"]).and_then(Value::as_array);
    for tool in tools.into_iter().flatten() {
        if text(tool, &["name"]).is_none() || text(tool, &["description"]).is_none() {
            problems.push(format!("a tool without a name or a description: {tool}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{MANIFEST_TEMPLATE}: {}", problems.join("; ")))
    }
}

/// The registry's entry for the bundle of `version` whose SHA-256 is
/// `sha256`: the template filled in and checked ([`check_server`]).
pub(crate) fn server(template: &str, version: &str, sha256: &str) -> Result<String, String> {
    let filled = template
        .replace("@VERSION@", version)
        .replace("@URL@", &asset_url(version))
        .replace("@SHA256@", sha256);
    if let Some(name) = placeholder(&filled) {
        return Err(format!(
            "{SERVER_TEMPLATE} has a placeholder the build does not fill in: {name}"
        ));
    }
    let value: Value =
        serde_json::from_str(&filled).map_err(|e| format!("{SERVER_TEMPLATE} is not JSON: {e}"))?;
    check_server(&value, version, sha256)?;
    Ok(filled)
}

/// Refuses an entry the official registry would refuse for an MCPB
/// package (its `server.json` schema of 2025-12-11 and
/// `internal/validators/registries/mcpb.go`), or that does not name this
/// bundle: the name, a description over 100 characters, a package that is
/// not one MCPB file at this release's address (HTTPS, a GitHub release
/// asset, "mcp" in the address), or a SHA-256 or version that is not this
/// bundle's.
pub(crate) fn check_server(server: &Value, version: &str, sha256: &str) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    if text(server, &["name"]) != Some(REGISTRY_NAME) {
        problems.push(format!("name is not {REGISTRY_NAME}"));
    }
    for key in ["description", "title"] {
        let length = text(server, &[key]).map_or(0, |t| t.chars().count());
        if !(1..=100).contains(&length) {
            problems.push(format!("{key} has {length} characters, not 1 to 100"));
        }
    }
    if text(server, &["version"]) != Some(version) {
        problems.push(format!("version is not {version}"));
    }
    let packages = field(server, &["packages"]).and_then(Value::as_array);
    match packages.map(Vec::as_slice) {
        Some([package]) => {
            if text(package, &["registryType"]) != Some("mcpb") {
                problems.push("the package is not of registryType mcpb".to_string());
            }
            let url = text(package, &["identifier"]).unwrap_or_default();
            if url != asset_url(version) {
                problems.push(format!(
                    "the package is `{url}`, not {}",
                    asset_url(version)
                ));
            }
            if let Err(why) = registry_accepts(url) {
                problems.push(why);
            }
            if text(package, &["fileSha256"]) != Some(sha256)
                || sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                problems.push(format!("fileSha256 is not the bundle's, {sha256}"));
            }
            if text(package, &["version"]) != Some(version) {
                problems.push(format!("the package's version is not {version}"));
            }
            if package.get("registryBaseUrl").is_some() {
                problems.push("an MCPB package has no registryBaseUrl".to_string());
            }
            if text(package, &["transport", "type"]) != Some("stdio") {
                problems.push("the package's transport is not stdio".to_string());
            }
        }
        _ => problems.push("packages is not one package".to_string()),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{SERVER_TEMPLATE}: {}", problems.join("; ")))
    }
}

/// What the registry asks of an MCPB package's address
/// (`internal/validators/registries/mcpb.go`, read on 2026-11-20): HTTPS,
/// on github.com, the path of a release asset
/// (`/owner/repo/releases/download/tag/file`), and "mcp" somewhere in it.
fn registry_accepts(url: &str) -> Result<(), String> {
    let path = url
        .strip_prefix("https://github.com/")
        .ok_or_else(|| format!("`{url}` is not an HTTPS address on github.com"))?;
    let parts: Vec<&str> = path.split('/').collect();
    let release_asset = matches!(
        parts.as_slice(),
        [owner, repo, "releases", "download", tag, file]
            if !owner.is_empty() && !repo.is_empty() && !tag.is_empty() && !file.is_empty()
    );
    if !release_asset {
        return Err(format!(
            "`{url}` is not a GitHub release asset (/owner/repo/releases/download/tag/file)"
        ));
    }
    if !url.to_ascii_lowercase().contains("mcp") {
        return Err(format!("`{url}` does not contain \"mcp\""));
    }
    Ok(())
}

/// What the bundle holds besides its manifest and its programs.
pub(crate) struct Contents<'a> {
    /// The manifest, filled in ([`manifest`]).
    pub(crate) manifest: &'a str,
    pub(crate) icon: &'a [u8],
    pub(crate) licence: &'a [u8],
    pub(crate) notices: &'a [u8],
    pub(crate) examples_licence: &'a [u8],
    /// The programs, in the order of [`PLATFORMS`].
    pub(crate) programs: [&'a [u8]; 3],
    /// The shipped fonts and their licences: file name and bytes.
    pub(crate) fonts: Vec<(&'a str, &'a [u8])>,
}

/// The entries of the bundle, in order: path, bytes and Unix mode.
fn entries<'a>(contents: &'a Contents<'a>) -> Vec<(String, &'a [u8], u16)> {
    let mut entries: Vec<(String, &[u8], u16)> = vec![
        (
            "manifest.json".to_string(),
            contents.manifest.as_bytes(),
            0o644,
        ),
        ("icon.png".to_string(), contents.icon, 0o644),
        ("LICENSE".to_string(), contents.licence, 0o644),
        (
            crate::notices::FILE_NAME.to_string(),
            contents.notices,
            0o644,
        ),
        (
            crate::package::EXAMPLES_LICENCE.to_string(),
            contents.examples_licence,
            0o644,
        ),
    ];
    for (platform, program) in PLATFORMS.iter().zip(contents.programs) {
        entries.push((platform.path.to_string(), program, 0o755));
    }
    for (file, bytes) in &contents.fonts {
        entries.push((format!("{FONTS}/{file}"), *bytes, 0o644));
    }
    entries
}

/// A raw deflate stream of `data` at the best compression, when that is
/// smaller than `data`; flate2's pure-Rust backend, so the bundle is the
/// same bytes on every machine.
fn deflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(data).ok()?;
    let stream = encoder.finish().ok()?;
    (stream.len() < data.len()).then_some(stream)
}

/// The bundle's bytes: every entry written (the programs with mode 0o755,
/// the rest 0o644), then read back with `craft3d-zipstore`'s reader and
/// compared with what went in, so the file kept is one that unpacks to
/// exactly that.
pub(crate) fn archive(contents: &Contents<'_>) -> Result<Vec<u8>, String> {
    let entries = entries(contents);
    for platform in &PLATFORMS {
        let fonts = resolve(folder(platform.path), FONTS_FROM_PROGRAM);
        if fonts.as_deref() != Some(FONTS) {
            return Err(format!(
                "{} would look for its fonts in {fonts:?}, not in {FONTS}",
                platform.path
            ));
        }
    }
    let new: Vec<craft3d_zipstore::NewEntry<'_>> = entries
        .iter()
        .map(|(name, data, mode)| craft3d_zipstore::NewEntry {
            name: name.as_str(),
            data: *data,
            unix_mode: Some(*mode),
        })
        .collect();
    let bytes =
        craft3d_zipstore::write(&new, Some(&deflate)).map_err(|e| format!("the bundle: {e}"))?;
    let back = craft3d_zipstore::read(&bytes).map_err(|e| format!("the bundle reads back: {e}"))?;
    if back.len() != entries.len() {
        return Err(format!(
            "the bundle reads back with {} entries, not {}",
            back.len(),
            entries.len()
        ));
    }
    for (got, (name, data, mode)) in back.iter().zip(&entries) {
        let read = got
            .data()
            .map_err(|e| format!("the bundle reads back: {e}"))?;
        if got.name != name.as_str() || got.unix_mode != Some(*mode) || *read != **data {
            return Err(format!("the bundle's {name} does not read back as written"));
        }
    }
    Ok(bytes)
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(raw))
}

/// What kind of program `bytes` look like, for an error that says what
/// was given.
fn kind(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x7fELF") {
        "an ELF program (Linux)"
    } else if matches!(u32_le(bytes, 0), Some(0xFEED_FACF | 0xFEED_FACE))
        || u32_be(bytes, 0) == Some(0xCAFE_BABE)
    {
        "a Mach-O program (macOS)"
    } else if bytes.starts_with(b"MZ") {
        "a Windows program"
    } else {
        "no program"
    }
}

/// Mach-O's CPU type of arm64 (`CPU_TYPE_ARM64`).
const MACHO_ARM64: u32 = 0x0100_000C;

/// Whether `bytes` are a Mach-O program that runs on Apple Silicon: a
/// 64-bit one for arm64, or a universal one with an arm64 slice.
fn macho_arm64(bytes: &[u8]) -> bool {
    if u32_le(bytes, 0) == Some(0xFEED_FACF) {
        return u32_le(bytes, 4) == Some(MACHO_ARM64);
    }
    if u32_be(bytes, 0) != Some(0xCAFE_BABE) {
        return false;
    }
    // The slices: a count, then 20 bytes each, the CPU type first.
    let count = u32_be(bytes, 4).unwrap_or(0).min(16);
    (0..count).any(|i| {
        usize::try_from(i)
            .ok()
            .and_then(|i| i.checked_mul(20))
            .and_then(|at| at.checked_add(8))
            .and_then(|at| u32_be(bytes, at))
            == Some(MACHO_ARM64)
    })
}

/// Refuses a program given for `platform` that is not one for it: an
/// ELF program for x86-64 on Linux; a Mach-O program for arm64 on macOS;
/// on Windows an x86-64 console program that needs no runtime a fresh
/// Windows lacks (the checks of the Windows packages). Says what it is,
/// for the log.
pub(crate) fn check_program(platform: &Platform, bytes: &[u8]) -> Result<String, String> {
    let wrong = |wanted: &str| {
        format!(
            "{} {}: the program given is {}, not {wanted}",
            platform.option,
            platform.path,
            kind(bytes)
        )
    };
    match platform.name {
        "linux" => {
            // ELFCLASS64, little-endian, e_machine EM_X86_64.
            let x86_64 = bytes.get(..6) == Some(b"\x7fELF\x02\x01".as_slice())
                && bytes.get(0x12..0x14) == Some([0x3E_u8, 0].as_slice());
            if x86_64 {
                Ok("an ELF program for x86-64".to_string())
            } else {
                Err(wrong("an ELF program for x86-64"))
            }
        }
        "darwin" => {
            if macho_arm64(bytes) {
                Ok("a Mach-O program for arm64 (Apple Silicon)".to_string())
            } else {
                Err(wrong("a Mach-O program for arm64 (Apple Silicon)"))
            }
        }
        "win32" => {
            let image = crate::pe::read(bytes).map_err(|_| wrong("a Windows program"))?;
            if image.machine != 0x8664 {
                return Err(format!(
                    "{} {}: the program given is for {}, not x86-64",
                    platform.option,
                    platform.path,
                    crate::pe::machine_name(image.machine)
                ));
            }
            crate::package::check_windows_program(platform.path, bytes)?;
            crate::package::check_windows_subsystem(platform.path, bytes, crate::pe::CONSOLE)?;
            Ok("a Windows console program for x86-64".to_string())
        }
        other => Err(format!("no platform `{other}`")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What `cargo xtask package --formats mcpb` was given and where it
/// writes.
pub(crate) struct Job<'a> {
    pub(crate) repo: &'a Path,
    /// `THIRD-PARTY-NOTICES.txt`, made for the packages.
    pub(crate) notices: &'a Path,
    pub(crate) out: &'a Path,
    pub(crate) version: &'a str,
    /// The `craft3d-cli` of each platform, in the order of [`PLATFORMS`].
    pub(crate) programs: [&'a Path; 3],
}

/// Makes the bundle and the registry's entry into `job.out`; returns both
/// paths.
pub(crate) fn package(job: &Job<'_>) -> Result<Vec<PathBuf>, String> {
    let read = crate::package::read;
    let mut programs: Vec<Vec<u8>> = Vec::new();
    for (platform, path) in PLATFORMS.iter().zip(job.programs) {
        let bytes = read(path)?;
        let what = check_program(platform, &bytes)?;
        eprintln!(
            "package: mcpb: {} is {what}, {} bytes, from {}",
            platform.path,
            bytes.len(),
            path.display()
        );
        programs.push(bytes);
    }
    let [linux, darwin, win32] = programs.as_slice() else {
        return Err("the bundle needs three programs".to_string());
    };
    crate::fonts::run()?;
    let font_dir = crate::fonts::dir();
    let font_bytes: Vec<(&str, Vec<u8>)> = crate::package::font_files()
        .map(|file| read(&font_dir.join(file)).map(|bytes| (file, bytes)))
        .collect::<Result<_, _>>()?;
    let template = String::from_utf8(read(&job.repo.join(MANIFEST_TEMPLATE))?)
        .map_err(|_| format!("{MANIFEST_TEMPLATE} is not UTF-8"))?;
    let manifest = manifest(&template, job.version)?;
    let icon = read(&job.repo.join(ICON))?;
    let licence = read(&job.repo.join("LICENSE"))?;
    let notices = read(job.notices)?;
    let examples_licence = read(&job.repo.join(crate::bundle::LICENCE))?;
    let contents = Contents {
        manifest: &manifest,
        icon: &icon,
        licence: &licence,
        notices: &notices,
        examples_licence: &examples_licence,
        programs: [linux.as_slice(), darwin.as_slice(), win32.as_slice()],
        fonts: font_bytes
            .iter()
            .map(|(file, bytes)| (*file, bytes.as_slice()))
            .collect(),
    };
    let bytes = archive(&contents)?;
    let sha256 = hex(&craft3d_doc::sha256(&bytes));
    let bundle = job.out.join(file_name(job.version));
    crate::package::write(&bundle, &bytes)?;
    let template = String::from_utf8(read(&job.repo.join(SERVER_TEMPLATE))?)
        .map_err(|_| format!("{SERVER_TEMPLATE} is not UTF-8"))?;
    let entry = job.out.join(SERVER_JSON);
    crate::package::write(&entry, server(&template, job.version, &sha256)?.as_bytes())?;
    let unpacked: usize = entries(&contents)
        .iter()
        .map(|(_, data, _)| data.len())
        .sum();
    eprintln!(
        "package: mcpb: {} holds {unpacked} bytes in {} bytes, SHA-256 {sha256}; {} names it \
         for the registry as {REGISTRY_NAME} {}",
        bundle.display(),
        bytes.len(),
        entry.display(),
        job.version
    );
    Ok(vec![bundle, entry])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::repo_root;

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    fn template(rel: &str) -> String {
        std::fs::read_to_string(repo_root().join(rel)).unwrap()
    }

    fn filled() -> Value {
        serde_json::from_str(&manifest(&template(MANIFEST_TEMPLATE), VERSION).unwrap()).unwrap()
    }

    /// A program of `len` bytes that compresses (as programs do).
    fn program(tag: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| if i % 7 == 0 { tag } else { (i % 13) as u8 })
            .collect()
    }

    /// The bundle of synthetic programs and fonts, with the real manifest.
    fn bundle(manifest: &str) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        let programs = [program(1, 50_000), program(2, 40_000), program(3, 60_000)];
        let fonts: Vec<(String, Vec<u8>)> = crate::package::font_files()
            .map(|file| (file.to_string(), file.as_bytes().repeat(100)))
            .collect();
        let contents = Contents {
            manifest,
            icon: b"\x89PNG\r\n\x1a\n",
            licence: b"MIT License",
            notices: b"notices",
            examples_licence: b"examples",
            programs: [
                programs[0].as_slice(),
                programs[1].as_slice(),
                programs[2].as_slice(),
            ],
            fonts: fonts
                .iter()
                .map(|(f, b)| (f.as_str(), b.as_slice()))
                .collect(),
        };
        (archive(&contents).unwrap(), fonts)
    }

    /// The manifest the build writes carries the workspace version, and
    /// its long description the version's series (`0.2` for 0.2.1); no
    /// placeholder is left.
    #[test]
    fn the_manifest_names_this_version() {
        let manifest = filled();
        assert_eq!(manifest["version"], VERSION);
        let named = series(VERSION).unwrap();
        let long = manifest["long_description"].as_str().unwrap();
        assert!(
            long.contains(&format!("3DCraft is at version {named}. ")),
            "{long}"
        );
        assert_eq!(
            named,
            VERSION.split('.').take(2).collect::<Vec<_>>().join(".")
        );
        assert!(placeholder(&manifest.to_string()).is_none());
        assert_eq!(placeholder("a @VERSION@ b"), Some("@VERSION@".to_string()));
        assert_eq!(placeholder("x@y.z and @ alone, @lower@"), None);
        assert!(series("7").is_err());
    }

    /// Every platform's command is a program the bundle holds, executable
    /// (mode 0o755, which an unpacker on macOS and Linux restores), the
    /// entry point is one of them, and from each program's folder the
    /// fonts are where the programs look: `../share/craft3d/fonts`, which
    /// `crates/engine/src/fonts.rs` names.
    #[test]
    fn each_platform_starts_a_program_of_the_bundle_that_finds_its_fonts() {
        let text = manifest(&template(MANIFEST_TEMPLATE), VERSION).unwrap();
        let (bytes, fonts) = bundle(&text);
        let entries = craft3d_zipstore::read(&bytes).unwrap();
        let get = |name: &str| entries.iter().find(|e| e.name == name);
        let manifest: Value =
            serde_json::from_slice(&get("manifest.json").unwrap().data().unwrap()).unwrap();
        assert_eq!(manifest["version"], VERSION);
        let config = &manifest["server"]["mcp_config"];
        for platform in &PLATFORMS {
            let command = config["platform_overrides"][platform.name]["command"]
                .as_str()
                .or(config["command"].as_str())
                .unwrap();
            let path = command.strip_prefix("${__dirname}/").unwrap();
            let program = get(path).unwrap_or_else(|| panic!("{}: no {path}", platform.name));
            assert_eq!(program.unix_mode, Some(0o755), "{path}");
            assert_eq!(program.method, 8, "{path} is deflated");
            let dir = folder(path);
            for (file, data) in &fonts {
                let at = resolve(dir, &format!("../share/craft3d/fonts/{file}")).unwrap();
                let font = get(&at).unwrap_or_else(|| panic!("{path}: no font at {at}"));
                assert_eq!(&font.data().unwrap()[..], &data[..]);
                assert_eq!(font.unix_mode, Some(0o644));
            }
        }
        let entry = manifest["server"]["entry_point"].as_str().unwrap();
        assert_eq!(get(entry).unwrap().unix_mode, Some(0o755));
        assert!(get(manifest["icon"].as_str().unwrap()).is_some());
        for licence in [
            "LICENSE",
            crate::notices::FILE_NAME,
            crate::package::EXAMPLES_LICENCE,
        ] {
            assert!(get(licence).is_some(), "{licence}");
        }
        assert_eq!(
            fonts.len(),
            crate::package::font_files().count(),
            "every shipped font with its licence"
        );
        let source =
            std::fs::read_to_string(repo_root().join("crates/engine/src/fonts.rs")).unwrap();
        assert!(
            source.contains(&format!("exe.join(\"{FONTS_FROM_PROGRAM}\")")),
            "crates/engine/src/fonts.rs no longer looks in {FONTS_FROM_PROGRAM}"
        );
        assert_eq!(resolve("server/linux", "../../.."), None);
    }

    /// The same programs give the same bundle, byte for byte.
    #[test]
    fn the_bundle_is_the_same_bytes_every_time() {
        let text = manifest(&template(MANIFEST_TEMPLATE), VERSION).unwrap();
        assert_eq!(bundle(&text).0, bundle(&text).0);
    }

    /// The manifest is one MCPB's validator and a client accept, and the
    /// check refuses what they would refuse, or what was not decided.
    #[test]
    fn a_manifest_that_does_not_start_the_bundle_as_decided_is_refused() {
        let good = filled();
        check_manifest(&good, VERSION).unwrap();
        let broken = |change: &dyn Fn(&mut Value), words: &str| {
            let mut manifest = good.clone();
            change(&mut manifest);
            let error = check_manifest(&manifest, VERSION).unwrap_err();
            assert!(error.contains(words), "{words}: {error}");
        };
        broken(
            &|m| m["permissions"] = Value::Bool(true),
            "`permissions` is no field",
        );
        broken(
            &|m| m["version"] = "0.0.1".into(),
            "version is not the workspace's",
        );
        broken(
            &|m| {
                m["server"]["mcp_config"]["platform_overrides"]["win32"]["command"] =
                    "${__dirname}/server/win32/craft3d.exe".into()
            },
            "on win32 the client starts",
        );
        broken(
            &|m| {
                m["server"]["mcp_config"]["platform_overrides"]["freebsd"] =
                    serde_json::json!({"command": "x"})
            },
            "names `freebsd`",
        );
        broken(
            &|m| m["compatibility"]["platforms"] = serde_json::json!(["darwin", "win32"]),
            "does not list linux",
        );
        broken(
            &|m| m["server"]["mcp_config"]["args"] = serde_json::json!(["mcp", "--bridge"]),
            "never --bridge",
        );
        broken(
            &|m| m["server"]["mcp_config"]["env"] = serde_json::json!({"A": "${HOME}/x"}),
            "`${HOME}` is no variable",
        );
        broken(
            &|m| m["user_config"]["work_folder"]["required"] = false.into(),
            "is not required",
        );
        broken(
            &|m| m["server"]["entry_point"] = "server/craft3d-cli".into(),
            "entry_point",
        );
        broken(&|m| m["icon"] = "icon.svg".into(), "icon is not");
    }

    /// The registry's entry names the bundle of this release at its
    /// address, with its SHA-256, in the form the registry accepts; its
    /// description is the manifest's.
    #[test]
    fn the_registry_entry_points_at_this_releases_bundle() {
        let text = manifest(&template(MANIFEST_TEMPLATE), VERSION).unwrap();
        let bytes = bundle(&text).0;
        let sha = hex(&craft3d_doc::sha256(&bytes));
        let entry: Value =
            serde_json::from_str(&server(&template(SERVER_TEMPLATE), VERSION, &sha).unwrap())
                .unwrap();
        let package = &entry["packages"][0];
        assert_eq!(
            package["identifier"],
            format!(
                "https://github.com/johancoppens/3dcraft/releases/download/v{VERSION}/3dcraft-{VERSION}.mcpb"
            )
        );
        assert_eq!(package["fileSha256"], sha.as_str());
        assert_eq!(package["version"], VERSION);
        assert_eq!(entry["version"], VERSION);
        assert_eq!(entry["name"], REGISTRY_NAME);
        let manifest: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(entry["description"], manifest["description"]);
        assert_eq!(entry["title"], manifest["display_name"]);
        // The registry's own rules for the address.
        assert!(registry_accepts(&asset_url(VERSION)).is_ok());
        assert!(registry_accepts("http://github.com/a/b/releases/download/v1/x.mcpb").is_err());
        assert!(registry_accepts("https://github.com/a/b/raw/main/x.mcpb").is_err());
        assert!(registry_accepts("https://github.com/a/b/releases/download/v1/x.zip").is_err());
        // A wrong sum, or one that is no SHA-256, is refused.
        assert!(server(&template(SERVER_TEMPLATE), VERSION, &"0".repeat(63)).is_err());
        assert!(server(&template(SERVER_TEMPLATE), VERSION, &"A".repeat(64)).is_err());
        let mut other = entry.clone();
        other["packages"][0]["fileSha256"] = "f".repeat(64).into();
        assert!(check_server(&other, VERSION, &sha).is_err());
    }

    /// Each platform takes its own program only: one built for another
    /// platform or processor is refused, saying what it is.
    #[test]
    fn a_program_for_another_platform_is_refused() {
        use crate::pe::{CONSOLE, GUI, synthetic::image};
        let [linux, darwin, win32] = &PLATFORMS;
        let mut elf = b"\x7fELF\x02\x01\x01".to_vec();
        elf.resize(64, 0);
        elf[0x12] = 0x3E;
        let mut thin = 0xFEED_FACF_u32.to_le_bytes().to_vec();
        thin.extend(MACHO_ARM64.to_le_bytes());
        thin.resize(64, 0);
        let mut fat = 0xCAFE_BABE_u32.to_be_bytes().to_vec();
        fat.extend(2_u32.to_be_bytes());
        for cpu in [0x0100_0007_u32, MACHO_ARM64] {
            fat.extend(cpu.to_be_bytes());
            fat.extend([0; 16]);
        }
        let mut intel = 0xFEED_FACF_u32.to_le_bytes().to_vec();
        intel.extend(0x0100_0007_u32.to_le_bytes());
        let exe = image(true, CONSOLE, &["KERNEL32.dll"], &[]);
        check_program(linux, &elf).unwrap();
        check_program(darwin, &thin).unwrap();
        check_program(darwin, &fat).unwrap();
        check_program(win32, &exe).unwrap();
        for (platform, bytes, words) in [
            (
                darwin,
                &elf,
                "--cli-macos server/darwin/craft3d-cli: the program given is an ELF program (Linux)",
            ),
            (
                linux,
                &thin,
                "is a Mach-O program (macOS), not an ELF program for x86-64",
            ),
            (darwin, &intel, "not a Mach-O program for arm64"),
            (linux, &exe, "is a Windows program"),
            (
                win32,
                &elf,
                "is an ELF program (Linux), not a Windows program",
            ),
        ] {
            let error = check_program(platform, bytes).unwrap_err();
            assert!(error.contains(words), "{words}: {error}");
        }
        let gui = image(true, GUI, &["KERNEL32.dll"], &[]);
        assert!(check_program(win32, &gui).unwrap_err().contains("console"));
        let x86 = image(false, CONSOLE, &["KERNEL32.dll"], &[]);
        assert!(
            check_program(win32, &x86)
                .unwrap_err()
                .contains("not x86-64")
        );
        let runtime = image(true, CONSOLE, &["VCRUNTIME140.dll"], &[]);
        assert!(
            check_program(win32, &runtime)
                .unwrap_err()
                .contains("VCRUNTIME140.dll")
        );
    }

    /// The Release workflow passes each package job's program by its
    /// option, and its draft takes the bundle as one of the release's
    /// packages, by the name the build gives it.
    #[test]
    fn the_release_workflow_publishes_the_bundle() {
        let workflow = template(".github/workflows/release.yml");
        assert!(workflow.contains("--formats mcpb"));
        for platform in &PLATFORMS {
            assert!(workflow.contains(platform.option), "{}", platform.option);
        }
        assert!(
            workflow.contains(&format!("\"{}\"", file_name("$version"))),
            "the draft's list of packages does not name {}",
            file_name("$version")
        );
        assert!(workflow.contains(&format!("dist/{SERVER_JSON}")));
    }
}
