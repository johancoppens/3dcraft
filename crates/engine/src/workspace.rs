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
}

/// Maps a path from a request to a path on disk, or refuses it. Beneath a
/// root, a path is relative, or absolute and textually under the root; it
/// has no `..`, and nothing is resolved on the file system, so the check
/// cannot be raced. (A symbolic link inside the root that points outside it
/// is followed: roots are meant to be directories the user chose to share.)
fn resolve(access: &Access, path: &str, direction: &str) -> Result<PathBuf, CommandError> {
    if path.is_empty() || path.contains('\0') {
        return Err(CommandError::new("a file path must not be empty"));
    }
    let root = match access {
        Access::Unrestricted => return Ok(PathBuf::from(path)),
        Access::None => {
            return Err(CommandError::new(format!(
                "this session has no {direction} access to files; start it with a {direction} root to allow `{path}`"
            )));
        }
        Access::Root(root) => root,
    };
    let requested = Path::new(path);
    let relative = if requested.is_absolute() {
        requested.strip_prefix(root).map_err(|_| {
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
