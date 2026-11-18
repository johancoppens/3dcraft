//! The source paths an assembly writes when it is saved into another
//! folder (docs/design/assemblies.md, *Pack and unpack*): pure text, so
//! the paths of every platform are tested on every platform.
//!
//! 2026-11-18: an assembly whose sources were relative to the read root,
//! saved under an absolute path, had them written as if the root were
//! the file system's (`../../../../model/rod.3dcraft` from
//! `/home/w/work/abs`), and the save answered ok. [`relink`] compares
//! the two folders only when they start at the same place, and says why
//! when no source path leads from the new folder to the file.

/// The longest source path, in bytes
/// ([`craft3d_doc::assembly::check_source_path`]).
const MOST_SOURCE_BYTES: usize = 4096;

/// Whether a path is absolute as the engine reads its folders: from `/`
/// or `\` (a Windows share, `\\server\share`, too) or from a Windows
/// drive (`C:`).
pub(crate) fn is_absolute(path: &str) -> bool {
    let mut chars = path.chars();
    path.starts_with(['/', '\\'])
        || matches!(
            (chars.next(), chars.next()),
            (Some(drive), Some(':')) if drive.is_ascii_alphabetic()
        )
}

/// Where a path starts: no relative path leads from one start to
/// another.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Start {
    /// Not absolute: relative to a folder both paths share.
    Relative,
    /// `/`, or a single `\`.
    Root,
    /// A Windows drive (`C:\`), its letter in upper case: Windows does
    /// not tell `c:` from `C:`.
    Drive(char),
    /// A Windows share (`\\server\share`), in lower case for the same
    /// reason.
    Share(String, String),
}

/// `path`'s start and the rest of it after that start.
fn path_start(path: &str) -> Result<(Start, &str), String> {
    // A verbatim Windows path, as the system hands back a long one:
    // `\\?\C:\…` or `\\?\UNC\server\share\…`.
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        if let Some(share) = rest.strip_prefix(r"UNC\") {
            return share_start(path, share);
        }
        return match drive_start(path, rest)? {
            Some(found) => Ok(found),
            None => Err(format!(
                "`{path}` is a Windows path of a kind a source path cannot be written against"
            )),
        };
    }
    if let Some(share) = path.strip_prefix(r"\\") {
        return share_start(path, share);
    }
    if let Some(found) = drive_start(path, path)? {
        return Ok(found);
    }
    if path.starts_with(['/', '\\']) {
        return Ok((Start::Root, path));
    }
    Ok((Start::Relative, path))
}

/// `\\server\share\rest` (given without its `\\`).
fn share_start<'a>(path: &str, share: &'a str) -> Result<(Start, &'a str), String> {
    let mut parts = share.splitn(3, ['\\', '/']);
    match (parts.next(), parts.next()) {
        (Some(server), Some(name)) if !server.is_empty() && !name.is_empty() => Ok((
            Start::Share(server.to_lowercase(), name.to_lowercase()),
            parts.next().unwrap_or_default(),
        )),
        _ => Err(format!("`{path}` names no share (`\\\\server\\share`)")),
    }
}

/// `C:\rest` or `C:` alone; `None` when `text` does not start with a
/// drive. `C:x` is refused: it is relative to the current folder of that
/// drive, which a path written into a file cannot follow.
fn drive_start<'a>(path: &str, text: &'a str) -> Result<Option<(Start, &'a str)>, String> {
    let mut chars = text.chars();
    let (Some(letter), Some(':')) = (chars.next(), chars.next()) else {
        return Ok(None);
    };
    if !letter.is_ascii_alphabetic() {
        return Ok(None);
    }
    let rest = text.get(2..).unwrap_or_default();
    if !(rest.is_empty() || rest.starts_with(['/', '\\'])) {
        return Err(format!(
            "`{path}` is relative to the current folder of drive {letter}:, which a source path cannot be written against"
        ));
    }
    Ok(Some((Start::Drive(letter.to_ascii_uppercase()), rest)))
}

/// Adds the parts of `text` (between `/` or `\`) to `parts`: `.` and
/// empty parts left out, `..` taking back the part before it. A relative
/// path keeps the `..` it begins with; an absolute one cannot climb above
/// its start (an error).
fn push_parts<'a>(start: &Start, parts: &mut Vec<&'a str>, text: &'a str) -> Result<(), ()> {
    for part in text.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if *start == Start::Relative {
                    parts.push("..");
                } else {
                    return Err(());
                }
            }
            other => parts.push(other),
        }
    }
    Ok(())
}

/// The source path that leads from the folder `new_base` to the file the
/// source path `source` leads to from the folder `old_base`: what a save
/// into `new_base` writes for it. The two folders are both absolute, or
/// both relative to one folder (`Session::save_bases`); the path is
/// `..` to climb, then the parts down, between `/`. An error, never
/// another path, when no source path leads there: the folders are of two
/// kinds, `source` climbs above the start of `old_base`, the folders are
/// on two Windows drives or shares, `new_base` climbs out of the folder
/// they share by parts it does not name, or the path is not one a source
/// may have (longer than a source path may be). The error is the reason,
/// to follow "cannot be written relative to `new_base`: ".
pub(crate) fn relink(old_base: &str, source: &str, new_base: &str) -> Result<String, String> {
    let (old_start, old_rest) = path_start(old_base)?;
    let (new_start, new_rest) = path_start(new_base)?;
    if (old_start == Start::Relative) != (new_start == Start::Relative) {
        return Err(format!(
            "the folder it is relative to now, `{old_base}`, and that one are not both absolute or both relative"
        ));
    }
    if old_start != new_start {
        return Err(format!(
            "it is on another drive or share than that folder (from `{old_base}`), and no relative path leads from one to the other"
        ));
    }
    let mut target: Vec<&str> = Vec::new();
    push_parts(&old_start, &mut target, old_rest)
        .map_err(|()| format!("the folder `{old_base}` climbs above its own start"))?;
    push_parts(&old_start, &mut target, source).map_err(|()| {
        format!(
            "from `{old_base}` it climbs above the start of the file system, so it leads to no file"
        )
    })?;
    let mut folder: Vec<&str> = Vec::new();
    push_parts(&new_start, &mut folder, new_rest)
        .map_err(|()| "that folder climbs above its own start".to_string())?;
    let common = folder
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    let up = folder.get(common..).unwrap_or_default();
    if up.contains(&"..") {
        return Err(format!(
            "that folder lies outside the folder it shares with the source (from `{old_base}`) by parts it does not name"
        ));
    }
    let mut link: Vec<&str> = vec![".."; up.len()];
    link.extend(target.get(common..).unwrap_or_default());
    let link = link.join("/");
    // Said here, not with the path: it can be very long.
    if link.len() > MOST_SOURCE_BYTES {
        return Err(format!(
            "the path that leads there would be {} bytes long, and a source path has at most {MOST_SOURCE_BYTES}",
            link.len()
        ));
    }
    craft3d_doc::assembly::check_source_path(&link)
        .map_err(|e| format!("the path that leads there is not one a source may have: {e}"))?;
    Ok(link)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn absolute_paths_are_told_apart_from_relative_ones() {
        for absolute in ["/", "/a", r"\a", r"\\srv\share", "C:", r"c:\a", "Z:/a"] {
            assert!(is_absolute(absolute), "{absolute}");
        }
        for relative in ["", ".", "a", "a/b", "1:/a", ":a", "é:"] {
            assert!(!is_absolute(relative), "{relative}");
        }
    }

    /// Unix folders: the same as the text of the paths says.
    #[test]
    fn a_source_is_relinked_between_folders_of_one_kind() {
        let cases = [
            (
                "work/rel",
                "../../model/v2/rod.3dcraft",
                "work/abs",
                "../../model/v2/rod.3dcraft",
            ),
            (
                "",
                "model/rod.3dcraft",
                "work/rel",
                "../../model/rod.3dcraft",
            ),
            (
                "work/rel",
                "../../model/rod.3dcraft",
                "",
                "model/rod.3dcraft",
            ),
            (
                "/w/work/rel",
                "../../model/rod.3dcraft",
                "/w/work/abs",
                "../../model/rod.3dcraft",
            ),
            ("/w/a", "p.3dcraft", "/w", "a/p.3dcraft"),
            ("/w", "a/p.3dcraft", "/w/a/b/c", "../../p.3dcraft"),
            ("/", "p.3dcraft", "/w", "../p.3dcraft"),
            ("/w", "p.3dcraft", "/", "w/p.3dcraft"),
            // Spellings of one folder: trailing and doubled slashes, `.`,
            // `..` inside the folder.
            ("/w/a/", "p.3dcraft", "/w//b/./", "../a/p.3dcraft"),
            ("/w/x/../a", "p.3dcraft", "/w/b", "../a/p.3dcraft"),
            ("work//rel/.", "p.3dcraft", "./work/rel", "p.3dcraft"),
            // A source above the root, from a relative folder, stays above
            // it.
            ("", "../p.3dcraft", "a/b", "../../../p.3dcraft"),
            ("a", "../../p.3dcraft", "", "../p.3dcraft"),
        ];
        for (old, source, new, want) in cases {
            assert_eq!(
                relink(old, source, new).as_deref(),
                Ok(want),
                "{old} + {source} from {new}"
            );
        }
    }

    /// The bug of 2026-11-18: a folder relative to the root and one from
    /// the file system's root are not compared as if both started at one
    /// place.
    #[test]
    fn folders_of_two_kinds_are_refused() {
        for (old, new) in [
            ("work/rel", "/home/w/work/abs"),
            ("/home/w/work/abs", "work/rel"),
            ("", "/w"),
            ("work", r"C:\w"),
            (r"\\srv\share\a", "a"),
        ] {
            let why = relink(old, "../../model/rod.3dcraft", new).unwrap_err();
            assert!(why.contains("not both absolute"), "{old} {new}: {why}");
        }
    }

    #[test]
    fn windows_drives_and_shares() {
        let ok = [
            (
                r"C:\w\work\rel",
                "../../model/rod.3dcraft",
                r"C:\w\work\abs",
                "../../model/rod.3dcraft",
            ),
            // Either separator, either case of the drive.
            ("c:/w/a", "p.3dcraft", r"C:\w\b", "../a/p.3dcraft"),
            (r"C:\", "p.3dcraft", r"C:\w", "../p.3dcraft"),
            ("C:", "w/p.3dcraft", r"C:\w", "p.3dcraft"),
            // One share, the server and share in any case.
            (
                r"\\srv\share\a",
                "p.3dcraft",
                r"\\SRV\Share\b",
                "../a/p.3dcraft",
            ),
            (
                r"\\srv\share",
                "a/p.3dcraft",
                r"\\srv\share\a\b",
                "../p.3dcraft",
            ),
            // Verbatim paths, as the system hands back long ones.
            (r"\\?\C:\w\a", "p.3dcraft", r"C:\w\b", "../a/p.3dcraft"),
            (
                r"\\?\UNC\srv\share\a",
                "p.3dcraft",
                r"\\srv\share",
                "a/p.3dcraft",
            ),
        ];
        for (old, source, new, want) in ok {
            assert_eq!(
                relink(old, source, new).as_deref(),
                Ok(want),
                "{old} + {source} from {new}"
            );
        }
        // No relative path leads to another drive or share: refused, not
        // written as `../../D:/…`.
        for (old, new) in [
            (r"D:\parts", r"C:\w"),
            (r"\\srv\share\a", r"\\srv\other\a"),
            (r"\\srv\share\a", r"\\other\share\a"),
            (r"\\srv\share\a", r"C:\a"),
            (r"C:\a", "/a"),
            (r"\\?\D:\a", r"C:\a"),
        ] {
            let why = relink(old, "p.3dcraft", new).unwrap_err();
            assert!(why.contains("another drive or share"), "{old} {new}: {why}");
        }
        // A drive's current folder, a share without its name.
        assert!(relink("C:w", "p.3dcraft", r"C:\w").is_err());
        assert!(relink(r"C:\w", "p.3dcraft", "C:w").is_err());
        assert!(relink(r"\\srv", "p.3dcraft", r"\\srv\share").is_err());
        assert!(relink(r"\\", "p.3dcraft", r"\\srv\share").is_err());
        assert!(relink(r"\\?\", "p.3dcraft", r"C:\").is_err());
    }

    #[test]
    fn hostile_folders_and_sources_are_refused_or_relinked_but_never_wrong() {
        // `..` above an absolute start.
        assert!(relink("/w", "../../p.3dcraft", "/x").is_err());
        assert!(relink("/..", "p.3dcraft", "/x").is_err());
        assert!(relink("/x", "p.3dcraft", "/../x").is_err());
        assert!(relink(r"C:\w", "../../p.3dcraft", r"C:\x").is_err());
        // A relative folder above the folder both share: its name above
        // is not known.
        assert!(relink("a", "p.3dcraft", "../b").is_err());
        // The folder itself, not a file: no source path names it.
        assert!(relink("/w", ".", "/w").is_err());
        // Ten thousand parts each way: too long for a source path, said
        // so in a few words, not with the path.
        let deep = "/d".repeat(10_000);
        let why = relink("/w", "p.3dcraft", &deep).unwrap_err();
        assert!(why.contains("at most 4096"), "{why}");
        assert!(why.len() < 200, "{}", why.len());
        let source = vec!["d"; 10_000].join("/");
        assert!(relink("/w", &source, "/x").is_err());
        let ups = vec![".."; 10_000].join("/");
        assert!(relink("/w", &ups, "/x").is_err());
        assert!(relink("", &format!("{ups}/p"), "").is_err());
        let ups = vec![".."; 1000].join("/");
        assert_eq!(
            relink("", &format!("{ups}/p"), "").unwrap().len(),
            1000 * 3 + 1
        );
        // A path of `..` alone that the root's start cannot take.
        assert!(relink("/", "..", "/").is_err());
        // Empty and odd folders.
        assert_eq!(relink("", "p.3dcraft", "").as_deref(), Ok("p.3dcraft"));
        assert_eq!(relink(".", "p.3dcraft", "./.").as_deref(), Ok("p.3dcraft"));
        assert_eq!(relink("/", "p.3dcraft", "///").as_deref(), Ok("p.3dcraft"));
    }
}
