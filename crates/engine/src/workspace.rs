//! File access for commands: explicit read and write roots, so automation
//! never gets ambient access to the file system (docs/control-protocol.md).

use crate::CommandError;
use std::path::{Component, Path, PathBuf};

/// What a session may do in one direction (reading or writing).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Access {
    /// Nothing.
    #[default]
    None,
    /// Paths beneath this directory only.
    Root(PathBuf),
    /// Paths beneath any of these directories (2026-11-02: the desktop
    /// app gives an agent its chosen folder and the open document's). A
    /// relative path is beneath the first; an absolute one beneath the
    /// first that holds it. Made with [`Access::from_roots`].
    Roots(Vec<PathBuf>),
    /// `base`, and this one file as well (2026-11-02: the person allowed it
    /// once, for one request, `Workspace::with_file`). Only that absolute
    /// path, spelled as given, with no link in its folder or as itself.
    Grant { base: Box<Access>, file: PathBuf },
    /// Any path: for a trusted local user at the command line.
    Unrestricted,
    /// The session's own files in memory ([`crate::Session::put_file`],
    /// [`crate::Session::take_file`]), where there is no file system to
    /// reach (the browser). A path is a relative name with the rules of a
    /// path beneath a root: no `..`, nothing absolute.
    Memory,
}

impl Access {
    /// The directories it reaches, in order (the first takes relative
    /// paths); none for no access, unrestricted access or memory.
    pub fn roots(&self) -> Vec<&Path> {
        match self {
            Access::Root(root) => vec![root.as_path()],
            Access::Roots(roots) => roots.iter().map(PathBuf::as_path).collect(),
            Access::Grant { base, .. } => base.roots(),
            Access::None | Access::Unrestricted | Access::Memory => Vec::new(),
        }
    }

    /// Access beneath `roots`, in order: none for no root, [`Access::Root`]
    /// for one, [`Access::Roots`] for more. A root that is the same
    /// directory as an earlier one (in its canonical spelling) is left out.
    pub fn from_roots(roots: Vec<PathBuf>) -> Access {
        let mut kept: Vec<PathBuf> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for root in roots {
            let spelled = spelled_root(&root);
            if !seen.contains(&spelled) {
                seen.push(spelled);
                kept.push(root);
            }
        }
        match kept.len() {
            0 => Access::None,
            1 => kept.pop().map_or(Access::None, Access::Root),
            _ => Access::Roots(kept),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Workspace {
    pub read: Access,
    pub write: Access,
}

impl Workspace {
    /// No file access at all.
    pub fn none() -> Self {
        Self::default()
    }

    /// Full access, for a trusted local user.
    pub fn unrestricted() -> Self {
        Self {
            read: Access::Unrestricted,
            write: Access::Unrestricted,
        }
    }

    /// This workspace with one more file, in one direction: what "Allow
    /// this file once" gives the one request it was asked for.
    pub fn with_file(&self, file: &Path, write: bool) -> Workspace {
        let grant = |base: &Access| Access::Grant {
            base: Box::new(base.clone()),
            file: file.to_path_buf(),
        };
        if write {
            Workspace {
                read: self.read.clone(),
                write: grant(&self.write),
            }
        } else {
            Workspace {
                read: grant(&self.read),
                write: self.write.clone(),
            }
        }
    }

    pub fn resolve_read(&self, path: &str) -> Result<PathBuf, CommandError> {
        resolve(&self.read, path, "read")
    }

    pub fn resolve_write(&self, path: &str) -> Result<PathBuf, CommandError> {
        resolve(&self.write, path, "write")
    }

    /// Both directions in the session's own memory: the browser's
    /// workspace.
    pub fn memory() -> Self {
        Self {
            read: Access::Memory,
            write: Access::Memory,
        }
    }
}

/// The path a document opened from or saved to `path` (found at
/// `resolved`) keeps, for a later `file.save` without a path and for
/// `file.list`. With unrestricted access a relative path is relative to
/// the process's working directory, which the document must not depend
/// on: the desktop app opens the file named on its command line that way
/// and then hands requests a write root, beneath which the same relative
/// path would name another file. So it is kept absolute, with its links
/// resolved (a root is spelled that way too, and paths are compared with
/// it as text). Beneath a root the path is kept as given: it is relative
/// to the root, which does not move.
pub(crate) fn kept_path(access: &Access, path: &str, resolved: &Path) -> String {
    if !matches!(access, Access::Unrestricted) || Path::new(path).is_absolute() {
        return path.to_string();
    }
    canonical(resolved)
        .or_else(|_| std::path::absolute(resolved))
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| path.to_string())
}

/// `path` absolute with its links resolved (`std::fs::canonicalize`), in
/// the spelling people and the other paths use: on Windows the system
/// hands back a verbatim path (`\\?\C:\a`, `\\?\UNC\server\share`),
/// which no path a user or an agent writes ever starts with, so it is
/// written as the plain path (`C:\a`, `\\server\share`). Every
/// canonical spelling the engine and its hosts compare goes through here.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(plain)
}

/// A verbatim Windows path written plainly ([`canonical`]); any other
/// path as it is. It stays verbatim where the plain spelling would mean
/// another file to Windows (the rules of the `dunce` crate): a part that
/// is a device name (`NUL`, `COM1`, `CON.txt`) or ends in a dot or a
/// space (which Windows strips), or a path longer than 260 characters.
pub fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let (rest, unc) = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        (rest, true)
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        // Only a drive path: `\\?\C:\...`.
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => (rest, false),
            _ => return path,
        }
    } else {
        return path;
    };
    let plain = if unc {
        format!(r"\\{rest}")
    } else {
        rest.to_string()
    };
    // The drive (`C:`) or the server and share are not parts to check.
    let skip = if unc { 2 } else { 1 };
    let unsafe_part = rest
        .split('\\')
        .skip(skip)
        .any(|part| !part.is_empty() && windows_unsafe_part(part));
    if plain.chars().count() > 260 || unsafe_part {
        return path;
    }
    PathBuf::from(plain)
}

/// Whether Windows reads a path part as something else than the file it
/// names: a device (`CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`, `LPT0`–`LPT9`,
/// `CONIN$`, `CONOUT$`, in any case, with any extension and trailing
/// spaces: `con .txt`), a name ending in a dot or a space (stripped, so
/// `a.` is `a`), or one with a `:` (an alternate data stream). Pure text,
/// so it is tested on every platform.
pub fn windows_unsafe_part(part: &str) -> bool {
    if part.ends_with('.') || part.ends_with(' ') || part.contains(':') {
        return part != "." && part != "..";
    }
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let digit: String = chars.collect();
    (prefix == "COM" || prefix == "LPT")
        && (digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit())
            || matches!(digit.as_str(), "\u{b9}" | "\u{b2}" | "\u{b3}"))
}

/// `root` written absolute with its links resolved, as the hosts spell
/// the roots they make (`std::fs::canonicalize`); as given where it
/// cannot be (it does not exist, or there is no file system).
pub(crate) fn spelled_root(root: &Path) -> PathBuf {
    canonical(root)
        .or_else(|_| std::path::absolute(root))
        .unwrap_or_else(|_| root.to_path_buf())
}

/// Maps a path from a request to a path on disk, or refuses it. Beneath a
/// root, a path is relative, or absolute and textually under the root (as
/// the root is given, or in its canonical spelling, [`spelled_root`]); it
/// has no `..`. Only the root's own spelling is resolved on the file
/// system for the comparison, and the path returned is always the root as
/// given joined with the relative part checked here. Since 2026-11-02 a
/// symbolic link beneath the root that leads outside it is refused too
/// ([`leaves_root`]): before, it was followed. With several roots a
/// relative path is beneath the first, an absolute one beneath the first
/// that holds it.
fn resolve(access: &Access, path: &str, direction: &str) -> Result<PathBuf, CommandError> {
    if path.is_empty() || path.contains('\0') {
        return Err(CommandError::new("a file path must not be empty"));
    }
    match access {
        Access::Unrestricted => Ok(PathBuf::from(path)),
        Access::None => Err(CommandError::new(format!(
            "this session has no {direction} access to files; start it with a {direction} root to allow `{path}`"
        ))),
        Access::Root(root) => beneath(root, path, direction),
        Access::Grant { base, file } => {
            if Path::new(path) == file.as_path() && granted_file(file) {
                return Ok(file.clone());
            }
            resolve(base, path, direction)
        }
        Access::Roots(roots) => {
            let Some(first) = roots.first() else {
                return Err(CommandError::new(format!(
                    "this session has no {direction} access to files"
                )));
            };
            if !Path::new(path).is_absolute() {
                return beneath(first, path, direction);
            }
            let mut last = None;
            for root in roots {
                match beneath(root, path, direction) {
                    Ok(resolved) => return Ok(resolved),
                    Err(e) => last = Some(e),
                }
            }
            let names: Vec<String> = roots
                .iter()
                .map(|root| format!("`{}`", root.display()))
                .collect();
            // One root's own reason (a link out, `..`) says more than
            // "outside" when the path was under it.
            match last {
                Some(e) if roots.len() == 1 => Err(e),
                _ => Err(CommandError::new(format!(
                    "`{path}` is outside every {direction} root ({})",
                    names.join(", ")
                ))),
            }
        }
        Access::Memory => {
            if Path::new(path).has_root() {
                return Err(CommandError::new(format!(
                    "`{path}` is an absolute path; a file in memory has a relative name"
                )));
            }
            checked_join(&PathBuf::new(), Path::new(path), path, direction)
        }
    }
}

/// Whether a file granted once is still the file the person was shown:
/// absolute, without `..`, its folder spelled as its resolved spelling (no
/// link on the way) and not itself a link.
fn granted_file(file: &Path) -> bool {
    let plain = file.is_absolute()
        && !file
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir));
    let Some(folder) = file.parent() else {
        return false;
    };
    let folder_real = canonical(folder).is_ok_and(|real| real == folder);
    let not_link = std::fs::symlink_metadata(file).map_or(true, |m| !m.file_type().is_symlink());
    plain && folder_real && not_link
}

/// `path` beneath the one directory `root` ([`resolve`]).
fn beneath(root: &Path, path: &str, direction: &str) -> Result<PathBuf, CommandError> {
    let requested = Path::new(path);
    let relative = if requested.is_absolute() {
        // A root given relative (or through a link) is compared in its
        // absolute spelling too: an absolute path inside it is inside it.
        requested
            .strip_prefix(root)
            .or_else(|_| requested.strip_prefix(spelled_root(root)))
            .map_err(|_| {
                CommandError::new(format!(
                    "`{path}` is outside the {direction} root `{}`",
                    root.display()
                ))
            })?
    } else {
        requested
    };
    let resolved = checked_join(root, relative, path, direction)?;
    if leaves_root(root, &resolved) {
        return Err(CommandError::new(format!(
            "`{path}` leads outside the {direction} root `{}` through a symbolic link",
            root.display()
        )));
    }
    Ok(resolved)
}

/// `root` joined with `relative`, part by part: no `..`, no absolute
/// part, no part Windows reads as another name, and not the root itself.
fn checked_join(
    root: &Path,
    relative: &Path,
    path: &str,
    direction: &str,
) -> Result<PathBuf, CommandError> {
    let mut resolved = root.to_path_buf();
    let mut depth = 0_usize;
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                // Beneath a root, a part Windows would read as a device or
                // as another name is refused there: the root is written
                // plainly ([`plain`]), so Windows normalises what is
                // joined to it.
                if cfg!(windows) && part.to_str().is_none_or(windows_unsafe_part) {
                    return Err(CommandError::new(format!(
                        "`{path}`: `{}` is a device name or a name Windows changes (a trailing dot or space, a `:`); not allowed beneath the {direction} root",
                        part.to_string_lossy()
                    )));
                }
                resolved.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CommandError::new(format!(
                    "`{path}` must stay beneath the {direction} root: `..` and absolute parts are not allowed"
                )));
            }
        }
    }
    if depth == 0 {
        return Err(CommandError::new(format!(
            "`{path}` names the {direction} root itself, not a file"
        )));
    }
    Ok(resolved)
}

/// Whether `resolved` (the root joined with a checked relative path)
/// leaves `root` on disk: the deepest part of it that exists, with its
/// links resolved, is not beneath the root's own resolved spelling. A link
/// that leads nowhere is taken to leave it (what it would create is not
/// known to be inside). A root that does not exist has nothing on disk to
/// leave it by.
fn leaves_root(root: &Path, resolved: &Path) -> bool {
    let Ok(real_root) = canonical(root) else {
        return false;
    };
    let mut probe = resolved.to_path_buf();
    while std::fs::symlink_metadata(&probe).is_err() {
        if !probe.pop() {
            return false;
        }
    }
    match canonical(&probe) {
        Ok(real) => !real.starts_with(&real_root),
        Err(_) => true,
    }
}

/// Writes a file so that a crash or a full disk leaves either the old file
/// or the new one, never half of it: write a temporary file next to the
/// target, sync it, rename it over the target.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    use std::io::Write;
    let fail = |what: &str, e: std::io::Error| {
        CommandError::new(format!("cannot {what} `{}`: {e}", path.display()))
    };
    let name = path
        .file_name()
        .ok_or_else(|| CommandError::new(format!("`{}` is not a file path", path.display())))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let result = (|| {
        // A new file, never through a link someone put there; a file
        // left by an earlier run of this process id is taken away first
        // (removing a link removes the link, not what it points to).
        let open = || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
        };
        let mut file = match open() {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&temp).map_err(|e| fail("create a temporary file for", e))?;
                open()
            }
            other => other,
        }
        .map_err(|e| fail("create a temporary file for", e))?;
        file.write_all(bytes).map_err(|e| fail("write", e))?;
        file.sync_all().map_err(|e| fail("write", e))?;
        std::fs::rename(&temp, path).map_err(|e| fail("replace", e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory `name` with a file `a.3dcraft` in it, its
    /// canonical spelling, and the same directory written relative to the
    /// working directory.
    fn real_root(name: &str) -> (PathBuf, PathBuf) {
        // Under the target directory, on the working directory's drive
        // (Windows' temporary directory may be on another, which no
        // relative path reaches).
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/tmp/craft3d-ws-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.3dcraft"), b"x").unwrap();
        let canonical = super::canonical(&dir).unwrap();
        let cwd = super::canonical(&std::env::current_dir().unwrap()).unwrap();
        let a: Vec<_> = cwd.components().collect();
        let b: Vec<_> = canonical.components().collect();
        let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let mut relative = PathBuf::new();
        for _ in common..a.len() {
            relative.push("..");
        }
        for part in b.iter().skip(common) {
            relative.push(part);
        }
        (canonical, relative)
    }

    /// The four pins of a root given in another spelling than its
    /// canonical one: an absolute path inside the canonical spelling is
    /// the root as given joined with the rest; `..` past it, a sibling
    /// that only shares its prefix and the root itself are refused.
    fn pins(given: &Path, canonical: &Path) {
        let access = Access::Root(given.to_path_buf());
        let inside = canonical.join("a.3dcraft");
        assert_eq!(
            resolve(&access, inside.to_str().unwrap(), "read").unwrap(),
            given.join("a.3dcraft")
        );
        let up = format!("{}/../x", canonical.display());
        assert!(resolve(&access, &up, "read").is_err(), "{up}");
        let sibling = format!("{}evil/x", canonical.display());
        assert!(resolve(&access, &sibling, "read").is_err(), "{sibling}");
        assert!(resolve(&access, canonical.to_str().unwrap(), "read").is_err());
        assert!(resolve(&access, &format!("{}/", canonical.display()), "read").is_err());
    }

    #[test]
    fn verbatim_windows_paths_are_written_plainly() {
        let p = |s: &str| plain(PathBuf::from(s));
        assert_eq!(p(r"\\?\C:\a\b"), PathBuf::from(r"C:\a\b"));
        assert_eq!(p(r"\\?\UNC\srv\share\x"), PathBuf::from(r"\\srv\share\x"));
        // Not a drive: as it is.
        assert_eq!(p(r"\\?\Volume{1}\x"), PathBuf::from(r"\\?\Volume{1}\x"));
        // Verbatim where the plain spelling means another file.
        for keep in [
            r"\\?\C:\a\NUL",
            r"\\?\C:\a\con.txt",
            r"\\?\C:\a\COM1\b",
            r"\\?\C:\a\b.",
            r"\\?\C:\a\b ",
            r"\\?\UNC\srv\share\lpt9",
        ] {
            assert_eq!(p(keep), PathBuf::from(keep), "{keep}");
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(p(&long), PathBuf::from(&long));
        assert_eq!(p("/home/a"), PathBuf::from("/home/a"));
        assert_eq!(p(r"D:\a"), PathBuf::from(r"D:\a"));
    }

    #[test]
    fn parts_windows_reads_as_other_files_are_told() {
        for bad in [
            "NUL",
            "nul",
            "Con",
            "CON.txt",
            "con .txt",
            "aux.tar.gz",
            "COM1",
            "lpt9.x",
            "COM\u{b9}",
            "CONIN$",
            "a.",
            "a ",
            "a.txt:stream",
        ] {
            assert!(windows_unsafe_part(bad), "{bad}");
        }
        for good in [
            "a",
            "console",
            "COM",
            "COM10",
            "LPT",
            "nul2",
            "a.b",
            ".",
            "..",
            "x.3dcraft",
        ] {
            assert!(!windows_unsafe_part(good), "{good}");
        }
    }

    #[test]
    fn a_relative_root_takes_absolute_paths_in_its_canonical_spelling() {
        let (canonical, relative) = real_root("relative");
        assert!(relative.is_relative());
        pins(&relative, &canonical);
        let _ = std::fs::remove_dir_all(&canonical);
    }

    #[cfg(unix)]
    #[test]
    fn a_root_through_a_link_takes_absolute_paths_in_its_canonical_spelling() {
        let (canonical, _) = real_root("linked");
        let link = canonical.with_file_name(format!(
            "{}-link",
            canonical.file_name().unwrap().to_str().unwrap()
        ));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&canonical, &link).unwrap();
        pins(&link, &canonical);
        // The link's own spelling is still inside.
        let access = Access::Root(link.clone());
        let through = link.join("a.3dcraft");
        assert_eq!(
            resolve(&access, through.to_str().unwrap(), "read").unwrap(),
            through
        );
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&canonical);
    }

    /// Two absolute roots that are valid on every platform (`/work/in` is
    /// not an absolute path on Windows, where it lacks a drive).
    fn roots() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join("craft3d-workspace-test");
        (base.join("in"), base.join("out"))
    }

    fn rooted() -> Workspace {
        let (read, write) = roots();
        Workspace {
            read: Access::Root(read),
            write: Access::Root(write),
        }
    }

    #[test]
    fn paths_resolve_beneath_their_own_root() {
        let w = rooted();
        let (read, write) = roots();
        assert_eq!(w.resolve_read("a.3dcraft").unwrap(), read.join("a.3dcraft"));
        assert_eq!(
            w.resolve_read("./parts/a.step").unwrap(),
            read.join("parts").join("a.step")
        );
        assert_eq!(w.resolve_write("b.stl").unwrap(), write.join("b.stl"));
        // An absolute path is accepted when it lies under the root.
        let inside_read = read.join("a.stl");
        let inside_write = write.join("b.stl");
        assert_eq!(
            w.resolve_read(inside_read.to_str().unwrap()).unwrap(),
            inside_read
        );
        // Read and write authority are independent.
        assert!(w.resolve_write(inside_read.to_str().unwrap()).is_err());
        assert!(w.resolve_read(inside_write.to_str().unwrap()).is_err());
    }

    #[test]
    fn escapes_are_refused() {
        let w = rooted();
        let (read, _) = roots();
        let root = read.to_str().unwrap();
        let separator = std::path::MAIN_SEPARATOR;
        let computed = [
            // A sibling directory whose name merely starts like the root.
            format!("{root}side{separator}x"),
            // The root itself is not a file.
            root.to_string(),
            // Leaving through `..` after an absolute prefix under the root.
            format!("{root}{separator}..{separator}out{separator}x"),
        ];
        let fixed = ["../x", "a/../../x", "/etc/passwd", "", ".", "a\0b"].map(String::from);
        for bad in fixed.iter().chain(&computed) {
            let error = w.resolve_read(bad).unwrap_err();
            assert!(!error.to_string().is_empty(), "`{bad}`");
        }
    }

    #[test]
    fn memory_names_are_relative_and_stay_inside() {
        let w = Workspace::memory();
        assert_eq!(
            w.resolve_read("a.3dcraft").unwrap(),
            PathBuf::from("a.3dcraft")
        );
        assert_eq!(
            w.resolve_write("./parts/b.stl").unwrap(),
            Path::new("parts").join("b.stl")
        );
        for bad in ["../x", "a/../../x", "/etc/passwd", "", ".", "a\0b"] {
            assert!(w.resolve_read(bad).is_err(), "`{bad}`");
            assert!(w.resolve_write(bad).is_err(), "`{bad}`");
        }
    }

    #[test]
    fn no_root_means_no_access_and_says_how_to_get_it() {
        let error = Workspace::none()
            .resolve_write("out.stl")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no write access") && error.contains("write root"),
            "{error}"
        );
        assert!(Workspace::none().resolve_read("in.3dcraft").is_err());
        assert_eq!(
            Workspace::unrestricted()
                .resolve_read("/any/where")
                .unwrap(),
            PathBuf::from("/any/where")
        );
    }

    /// Several roots (2026-11-02): relative paths beneath the first, an
    /// absolute path beneath whichever holds it, nothing outside all.
    #[test]
    fn several_roots_take_paths_beneath_any_and_nothing_else() {
        let (first, _) = real_root("many-a");
        let (second, _) = real_root("many-b");
        let access = Access::from_roots(vec![first.clone(), second.clone(), first.clone()]);
        assert_eq!(access, Access::Roots(vec![first.clone(), second.clone()]));
        assert_eq!(access.roots(), [first.as_path(), second.as_path()]);
        assert_eq!(
            resolve(&access, "x.stl", "write").unwrap(),
            first.join("x.stl")
        );
        let in_second = second.join("a.3dcraft");
        assert_eq!(
            resolve(&access, in_second.to_str().unwrap(), "read").unwrap(),
            in_second
        );
        let outside = first.with_file_name("elsewhere").join("x");
        let error = resolve(&access, outside.to_str().unwrap(), "read")
            .unwrap_err()
            .message()
            .to_string();
        assert!(error.contains("outside every read root"), "{error}");
        for bad in ["../x", "", "."] {
            assert!(resolve(&access, bad, "read").is_err(), "{bad}");
        }
        let up = format!("{}/../x", second.display());
        assert!(resolve(&access, &up, "read").is_err());
        assert_eq!(Access::from_roots(Vec::new()), Access::None);
        assert_eq!(
            Access::from_roots(vec![first.clone()]),
            Access::Root(first.clone())
        );
        assert!(Access::Unrestricted.roots().is_empty());
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    /// A symbolic link beneath a root that leads outside it is refused,
    /// to read through, to write through, and as a folder to go into; a
    /// link that stays inside is fine (2026-11-02).
    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_root_is_refused() {
        let (root, _) = real_root("link-out");
        let (outside, _) = real_root("link-target");
        std::os::unix::fs::symlink(outside.join("a.3dcraft"), root.join("out.3dcraft")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("away")).unwrap();
        std::os::unix::fs::symlink(outside.join("none"), root.join("dangling")).unwrap();
        std::os::unix::fs::symlink(root.join("a.3dcraft"), root.join("in.3dcraft")).unwrap();
        for access in [
            Access::Root(root.clone()),
            Access::from_roots(vec![root.clone(), outside.with_file_name("other")]),
        ] {
            for bad in ["out.3dcraft", "away/a.3dcraft", "away/new.stl", "dangling"] {
                let error = resolve(&access, bad, "write").unwrap_err();
                assert!(
                    error.message().contains("symbolic link"),
                    "{bad}: {error:?}"
                );
                let absolute = root.join(bad);
                assert!(resolve(&access, absolute.to_str().unwrap(), "read").is_err());
            }
            assert_eq!(
                resolve(&access, "in.3dcraft", "read").unwrap(),
                root.join("in.3dcraft")
            );
            assert_eq!(
                resolve(&access, "new/deeper.stl", "write").unwrap(),
                root.join("new").join("deeper.stl")
            );
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// A file allowed once: that file only, in that direction only.
    #[test]
    fn a_file_allowed_once_is_that_file_only() {
        let (root, _) = real_root("once-root");
        let (other, _) = real_root("once-other");
        let file = other.join("part.stl");
        let workspace = Workspace {
            read: Access::Root(root.clone()),
            write: Access::Root(root.clone()),
        }
        .with_file(&file, true);
        assert_eq!(
            workspace.resolve_write(file.to_str().unwrap()).unwrap(),
            file
        );
        assert!(
            workspace.resolve_read(file.to_str().unwrap()).is_err(),
            "write only"
        );
        assert!(
            workspace
                .resolve_write(other.join("else.stl").to_str().unwrap())
                .is_err()
        );
        assert!(
            workspace
                .resolve_write(&format!(
                    "{}/../{}/part.stl",
                    other.display(),
                    other.file_name().unwrap().to_str().unwrap()
                ))
                .is_err()
        );
        assert_eq!(
            workspace.resolve_write("x.stl").unwrap(),
            root.join("x.stl"),
            "the roots stay"
        );
        assert_eq!(workspace.write.roots(), [root.as_path()]);
        #[cfg(unix)]
        {
            let link = other.join("link.stl");
            std::os::unix::fs::symlink(root.join("a.3dcraft"), &link).unwrap();
            let through = Workspace::none().with_file(&link, false);
            assert!(
                through.resolve_read(link.to_str().unwrap()).is_err(),
                "a link is not the file"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&other);
    }
}
