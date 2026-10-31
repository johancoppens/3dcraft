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
    /// Any path: for a trusted local user at the command line.
    Unrestricted,
    /// The session's own files in memory ([`crate::Session::put_file`],
    /// [`crate::Session::take_file`]), where there is no file system to
    /// reach (the browser). A path is a relative name with the rules of a
    /// path beneath a root: no `..`, nothing absolute.
    Memory,
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
/// system, never the requested path, and the path returned is always the
/// root as given joined with the relative part checked here. (A symbolic
/// link inside the root that points outside it is followed: roots are
/// meant to be directories the user chose to share.)
fn resolve(access: &Access, path: &str, direction: &str) -> Result<PathBuf, CommandError> {
    if path.is_empty() || path.contains('\0') {
        return Err(CommandError::new("a file path must not be empty"));
    }
    // Names in memory resolve beneath nothing: the name itself.
    let in_memory = PathBuf::new();
    let root = match access {
        Access::Unrestricted => return Ok(PathBuf::from(path)),
        Access::None => {
            return Err(CommandError::new(format!(
                "this session has no {direction} access to files; start it with a {direction} root to allow `{path}`"
            )));
        }
        Access::Root(root) => root,
        Access::Memory => {
            if Path::new(path).has_root() {
                return Err(CommandError::new(format!(
                    "`{path}` is an absolute path; a file in memory has a relative name"
                )));
            }
            &in_memory
        }
    };
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
    let mut resolved = root.clone();
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
        let mut file =
            std::fs::File::create(&temp).map_err(|e| fail("create a temporary file for", e))?;
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
}
