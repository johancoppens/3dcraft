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

/// A path as the file system spells it (`workspace::canonical`, after
/// `workspace::plain`), written as [`crate::asm::join`] writes paths, for
/// comparing two folders, where it is a plain path: from `/` (Unix) or
/// from a drive (`C:\`). `None` for a share (`\\server\share`, what a
/// mapped network drive resolves to), a verbatim path (`\\?\…`, kept for
/// one longer than 260 characters or with a part Windows reads as a
/// device) and anything else: `join` would write those as another path,
/// so such folders are compared as the texts given.
pub(crate) fn plain_spelling(spelled: &str) -> Option<String> {
    let mut chars = spelled.chars();
    let drive = matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':'), Some('\\' | '/')) if letter.is_ascii_alphabetic()
    );
    let unix = spelled.starts_with('/') && !spelled.starts_with("//");
    if !(drive || unix) {
        return None;
    }
    crate::asm::join(spelled, "").ok()
}

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

/// The start of `path` as [`crate::asm::join`] reads it ([`path_start`]):
/// whether there is one (the path is absolute), and how many of its
/// parts, between `separators`, are that start, which no `..` takes back:
/// a drive (`C:`) one, a share (`\\server\share`) two, a verbatim drive
/// (`\\?\C:`) two and a verbatim share (`\\?\UNC\server\share`) four; `/`
/// none. `None` where [`path_start`] finds no start it can name (`C:x`,
/// `\\server` alone, `\\?\` of another kind).
pub(crate) fn start_parts(path: &str, separators: &[char]) -> Option<(bool, usize)> {
    let (start, rest) = path_start(path).ok()?;
    // `rest` is the end of `path`.
    let head = path.get(..path.len().saturating_sub(rest.len()))?;
    let parts = head
        .split(separators)
        .filter(|part| !part.is_empty() && *part != ".")
        .count();
    Some((start != Start::Relative, parts))
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
/// both relative to one folder (`asm_cmds::Relinker` gives the file on
/// disk and the folder written into); the path is
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

/// The reviewer's tests (review-asm-save, 2026-11-18): every path
/// [`relink`] returns, over generated folders and sources of every kind
/// (Unix, drives in either case, shares, verbatim paths, mixed
/// separators, `..` beyond the start, NUL, lossy names), leads from the
/// new folder to the file the source led to from the old one, read by an
/// independent reading of the two folders; it refuses only where no source
/// path leads there; nothing panics.
#[cfg(test)]
mod review {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use super::*;

    /// A path's start, as a key, and the rest after it; `None` where no
    /// path can be written against it.
    fn start(path: &str) -> Option<(String, String)> {
        fn share(rest: &str) -> Option<(String, String)> {
            let mut it = rest.splitn(3, ['/', '\\']);
            let server = it.next()?;
            let name = it.next()?;
            if server.is_empty() || name.is_empty() {
                return None;
            }
            Some((
                format!("share:{}/{}", server.to_lowercase(), name.to_lowercase()),
                it.next().unwrap_or("").to_string(),
            ))
        }
        // `None`: no drive; `Some(None)`: a drive no path is written
        // against (`C:x`).
        fn drive(text: &str) -> Option<Option<(String, String)>> {
            let mut chars = text.chars();
            let letter = chars.next()?;
            if !letter.is_ascii_alphabetic() || chars.next() != Some(':') {
                return None;
            }
            let rest = &text[2..];
            if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
                return Some(None);
            }
            Some(Some((
                format!("drive:{}", letter.to_ascii_uppercase()),
                rest.to_string(),
            )))
        }
        if let Some(rest) = path.strip_prefix("\\\\?\\") {
            if let Some(unc) = rest.strip_prefix("UNC\\") {
                return share(unc);
            }
            return drive(rest).flatten();
        }
        if let Some(unc) = path.strip_prefix("\\\\") {
            return share(unc);
        }
        if let Some(found) = drive(path) {
            return found;
        }
        if path.starts_with('/') || path.starts_with('\\') {
            return Some(("root".to_string(), path.to_string()));
        }
        Some(("relative".to_string(), path.to_string()))
    }

    /// `parts` with those of `text` added; `None` above an absolute start.
    fn walk(relative: bool, parts: &mut Vec<String>, text: &str) -> Option<()> {
        for part in text.split(['/', '\\']) {
            match part {
                "" | "." => {}
                ".." => {
                    let back = parts.last().is_some_and(|last| last.as_str() != "..");
                    if back {
                        parts.pop();
                    } else if relative {
                        parts.push("..".to_string());
                    } else {
                        return None;
                    }
                }
                other => parts.push(other.to_string()),
            }
        }
        Some(())
    }

    /// The file `source` names from the folder `base`: start and parts.
    fn file_of(base: &str, source: &str) -> Option<(String, Vec<String>)> {
        let (key, rest) = start(base)?;
        let relative = key == "relative";
        let mut parts = Vec::new();
        walk(relative, &mut parts, &rest)?;
        walk(relative, &mut parts, source)?;
        Some((key, parts))
    }

    fn pick(seed: &mut u64, n: usize) -> usize {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(*seed >> 33).unwrap_or(0) % n
    }

    const STARTS: [&str; 27] = [
        "",
        ".",
        "/",
        "//",
        "\\",
        "\\\\",
        "C:",
        "c:",
        "C:\\",
        "c:/",
        "D:\\",
        "C:x",
        "1:/",
        "é:",
        "\\\\srv\\share",
        "\\\\SRV\\Share\\",
        "\\\\srv",
        "\\\\srv\\",
        "\\\\?\\",
        "\\\\?\\C:\\",
        "\\\\?\\c:",
        "\\\\?\\UNC\\srv\\share\\",
        "\\\\?\\UNC\\srv",
        "\\\\?\\X\\",
        "//srv/share/",
        "\\\\.\\C:\\",
        "C:/w",
    ];
    const PARTS: [&str; 14] = [
        "a", "b", "A", "..", ".", "", "é", "x y", "\u{FFFD}", "a:b", "nul", "a\u{0}b", "a\\b", "w",
    ];
    const SEPARATORS: [&str; 4] = ["/", "\\", "//", "/./"];
    const SOURCES: [&str; 20] = [
        "p.3dcraft",
        "a/p.3dcraft",
        "../p.3dcraft",
        "../../p.3dcraft",
        "../../../p.3dcraft",
        "a/../p.3dcraft",
        "../a/../b/p.3dcraft",
        "é/\u{FFFD}.3dcraft",
        "",
        ".",
        "..",
        "/",
        "/p.3dcraft",
        "a\\p.3dcraft",
        "C:/p.3dcraft",
        "p\u{0}.3dcraft",
        "a//p.3dcraft",
        "./p.3dcraft",
        "p.3dcraft/",
        "x y/p.3dcraft",
    ];

    fn folder(seed: &mut u64) -> String {
        let mut out = STARTS[pick(seed, STARTS.len())].to_string();
        let count = pick(seed, 4);
        for i in 0..count {
            if i > 0 || !(out.is_empty() || out.ends_with('/') || out.ends_with('\\')) {
                out.push_str(SEPARATORS[pick(seed, SEPARATORS.len())]);
            }
            out.push_str(PARTS[pick(seed, PARTS.len())]);
        }
        out
    }

    #[test]
    fn review_every_link_leads_to_the_file_and_nothing_panics() {
        let mut seed = 0x5eed_u64;
        let (mut written, mut refused) = (0_usize, 0_usize);
        let mut wrong: Vec<String> = Vec::new();
        for _ in 0..60_000 {
            let old = folder(&mut seed);
            let new = if pick(&mut seed, 4) == 0 {
                old.clone()
            } else {
                folder(&mut seed)
            };
            let source = SOURCES[pick(&mut seed, SOURCES.len())];
            let Ok(got) = std::panic::catch_unwind(|| relink(&old, source, &new)) else {
                wrong.push(format!("panic: relink({old:?}, {source:?}, {new:?})"));
                continue;
            };
            let want = file_of(&old, source);
            let new_start = start(&new);
            match got {
                Ok(link) => {
                    written += 1;
                    let back = new_start.as_ref().and_then(|_| file_of(&new, &link));
                    let fine = craft3d_doc::assembly::check_source_path(&link).is_ok()
                        && want.is_some()
                        && back == want;
                    if !fine {
                        wrong.push(format!(
                            "relink({old:?}, {source:?}, {new:?}) = {link:?}, which leads to {back:?}, not {want:?}"
                        ));
                    }
                }
                Err(why) => {
                    refused += 1;
                    if why.is_empty() {
                        wrong.push(format!("relink({old:?}, {source:?}, {new:?}): no reason"));
                    }
                    // Refused although a link exists: the two folders of
                    // one start, the file named, the new folder below the
                    // part it shares without `..`.
                    if let (Some((key, target)), Some((new_key, new_rest))) = (&want, &new_start) {
                        let mut fold = Vec::new();
                        if key == new_key
                            && walk(new_key == "relative", &mut fold, new_rest).is_some()
                        {
                            let common =
                                fold.iter().zip(target).take_while(|(a, b)| a == b).count();
                            let up = &fold[common..];
                            if !up.iter().any(|p| p == "..") {
                                let mut link: Vec<String> = vec!["..".to_string(); up.len()];
                                link.extend(target[common..].iter().cloned());
                                let link = link.join("/");
                                if link.len() <= 4096
                                    && craft3d_doc::assembly::check_source_path(&link).is_ok()
                                {
                                    wrong.push(format!(
                                        "relink({old:?}, {source:?}, {new:?}) refused ({why}), though `{link}` leads there"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        println!(
            "review relink: {written} written, {refused} refused, {} wrong",
            wrong.len()
        );
        for w in wrong.iter().take(40) {
            println!("WRONG {w}");
        }
        assert!(written > 1000 && refused > 1000, "{written} {refused}");
        assert!(
            wrong.is_empty(),
            "{} wrong; first: {:?}",
            wrong.len(),
            wrong.first()
        );
    }

    #[test]
    fn review_is_absolute_agrees_with_the_start() {
        let mut seed = 0xab5_u64;
        for _ in 0..20_000 {
            let path = folder(&mut seed);
            match path_start(&path) {
                Ok((Start::Relative, _)) => assert!(!is_absolute(&path), "{path:?}"),
                Ok(_) => assert!(is_absolute(&path), "{path:?}"),
                Err(why) => assert!(!why.is_empty(), "{path:?}"),
            }
        }
    }

    /// Long and odd inputs, each way: an answer, never a panic; and what
    /// relink makes of a `source` that is no source path (absolute, a
    /// drive, a backslash: a stored source never is one, so this is the
    /// contract at its edge), printed for the review.
    #[test]
    fn review_long_and_odd_inputs() {
        let ups = "../".repeat(10_000);
        let deep = "/d".repeat(10_000);
        let lossy = String::from_utf8_lossy(b"caf\xe9/\xff\xfe.3dcraft").to_string();
        for (old, source, new) in [
            (ups.as_str(), "p.3dcraft", ""),
            ("", "p.3dcraft", ups.as_str()),
            (deep.as_str(), ups.as_str(), deep.as_str()),
            ("/w", lossy.as_str(), "/x"),
            ("/w\u{0}", "p.3dcraft", "/x"),
            ("\u{0}", "\u{0}", "\u{0}"),
        ] {
            let got = std::panic::catch_unwind(|| relink(old, source, new));
            assert!(
                got.is_ok(),
                "panic on {} {} {}",
                old.len(),
                source.len(),
                new.len()
            );
        }
        let want = format!("../w/{lossy}");
        assert_eq!(
            relink("/w", lossy.as_str(), "/x").as_deref(),
            Ok(want.as_str())
        );
        for (old, source, new) in [
            ("/w", "/etc/p.3dcraft", "/x"),
            ("/w", "C:/p.3dcraft", "/x"),
            ("/w", "a\\p.3dcraft", "/x"),
            (r"C:\w", r"D:\p.3dcraft", r"C:\x"),
            ("//srv/share/a", "p.3dcraft", "//srv/other/b"),
        ] {
            println!(
                "review relink({old:?}, {source:?}, {new:?}) = {:?}",
                relink(old, source, new)
            );
        }
    }
}

/// The reviewer's third round (review-asm-save, 2026-11-18): what the
/// engine makes of the spellings `workspace::canonical` hands back on
/// Windows (after `workspace::plain`), as text: a mapped network drive
/// (`Z:\dir`, which the system resolves to its share), a folder longer
/// than 260 characters and a part that is a device name (both kept
/// verbatim by `plain`). Round three found them written through
/// `asm::join` as `/server/share/dir` and `/?/C:/…` and kept as an
/// assembly's path; since asm-save-path-2's second hand-in no resolved
/// spelling is kept or written, and [`super::plain_spelling`], which the
/// save's comparison of two folders reads them by, gives none for them
/// (they are compared as the texts given): never a path that is not a
/// Windows one. A plain drive path is compared as one.
#[cfg(test)]
mod review3 {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    #[test]
    fn review3_what_a_canonical_windows_folder_becomes() {
        let long = format!(r"\\?\C:\{}", "d".repeat(300));
        let mut wrong = Vec::new();
        for (canonical, given, what) in [
            (
                r"\\?\UNC\server\share\dir".to_string(),
                r"Z:\dir".to_string(),
                "a mapped network drive",
            ),
            (
                long.clone(),
                long.replace(r"\\?\", ""),
                "a folder longer than 260 characters",
            ),
            (
                r"\\?\C:\a\con".to_string(),
                r"C:\a\con".to_string(),
                "a part that is a device name",
            ),
        ] {
            let plain = crate::workspace::plain(std::path::PathBuf::from(&canonical));
            let spelled = plain.to_str().unwrap().to_string();
            let compared = super::plain_spelling(&spelled);
            let as_given = crate::asm::join(&given, "").unwrap();
            println!(
                "review3 {what}: canonical {spelled:.40} -> compared as {compared:?} (as given: {as_given:.40})"
            );
            if let Some(joined) = &compared {
                let mut chars = joined.chars();
                let drive = matches!((chars.next(), chars.next()), (Some(l), Some(':')) if l.is_ascii_alphabetic());
                if !drive && !joined.starts_with(r"\\") {
                    wrong.push(format!("{what}: {joined:.40}"));
                }
            }
        }
        assert!(wrong.is_empty(), "not a Windows path: {wrong:?}");
        let plain = crate::workspace::plain(std::path::PathBuf::from(r"\\?\C:\a\b"));
        assert_eq!(
            super::plain_spelling(plain.to_str().unwrap()).as_deref(),
            Some("C:/a/b")
        );
        assert_eq!(
            super::plain_spelling("/home/w/x").as_deref(),
            Some("/home/w/x")
        );
        for other in [
            r"\\server\share\dir",
            r"\\?\C:\a\con",
            "//srv/share",
            "x/y",
            "",
        ] {
            assert_eq!(super::plain_spelling(other), None, "{other}");
        }
    }
}
