//! The control token: 256 random bits as 64 hexadecimal characters, kept in
//! a file only its owner can read (docs/control-protocol.md,
//! "Authentication"). It is never on a command line or in a log.

use std::io::{ErrorKind, Write};
use std::path::Path;

/// Characters in a token.
pub const TOKEN_CHARS: usize = 64;

fn is_token(text: &str) -> bool {
    text.len() == TOKEN_CHARS && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A fresh token from the operating system's random source.
pub fn generate() -> Result<String, String> {
    let mut bytes = [0_u8; TOKEN_CHARS / 2];
    getrandom::fill(&mut bytes)
        .map_err(|e| format!("cannot get random bytes for a control token: {e}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn create(path: &Path, token: &str) -> std::io::Result<()> {
    // The folder it goes in, made when it is missing: a configuration
    // folder (`%APPDATA%\3DCraft`, `~/.config`) that nothing made yet.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    // Never over an existing file, and never through a link someone put
    // there in the meantime.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
}

/// Reads the token from `path`, or creates the file with a fresh token when
/// it does not exist (mode 0600 on Unix), and the folders it goes in. A file that holds anything but a
/// token is an error and is left as it is.
pub fn load_or_create(path: &Path) -> Result<String, String> {
    let shown = path.display();
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let token = text.trim();
            if is_token(token) {
                Ok(token.to_ascii_lowercase())
            } else {
                Err(format!(
                    "`{shown}` does not hold a control token ({TOKEN_CHARS} hexadecimal characters); delete it to have a new one made"
                ))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let token = generate()?;
            create(path, &token)
                .map_err(|e| format!("cannot create the control token file `{shown}`: {e}"))?;
            Ok(token)
        }
        Err(e) => Err(format!("cannot read the control token file `{shown}`: {e}")),
    }
}

/// Whether `given` is the token, compared without stopping at the first
/// difference, so the time taken does not say how much of it was right.
pub fn matches(expected: &str, given: &str) -> bool {
    let (expected, given) = (expected.as_bytes(), given.as_bytes());
    let mut difference = expected.len() ^ given.len();
    for (index, byte) in expected.iter().enumerate() {
        let other = given.get(index).copied().unwrap_or(0);
        difference |= usize::from(byte ^ other.to_ascii_lowercase());
    }
    difference == 0 && !expected.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-token-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_generated_token_is_64_hexadecimal_characters_and_never_the_same() {
        let (a, b) = (generate().unwrap(), generate().unwrap());
        assert!(is_token(&a) && is_token(&b));
        assert_ne!(a, b);
        assert_eq!(a, a.to_ascii_lowercase());
    }

    #[test]
    fn a_missing_file_is_created_private_and_then_reused() {
        let dir = dir("create");
        let path = dir.join("control.token");
        let token = load_or_create(&path).unwrap();
        assert!(is_token(&token));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{token}\n")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the owner may read it");
        }
        // The second start reads the same token and leaves the file alone.
        assert_eq!(load_or_create(&path).unwrap(), token);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{token}\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_token_written_by_hand_is_accepted_in_any_case_and_with_line_endings() {
        let dir = dir("by-hand");
        let path = dir.join("t");
        let token = "0123456789ABCDEF".repeat(4);
        std::fs::write(&path, format!("  {token}\r\n")).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), token.to_ascii_lowercase());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_token_is_an_error_and_is_not_overwritten() {
        let dir = dir("invalid");
        for (index, content) in [
            "",
            "secret",
            &"g".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
            &format!("{}\n{}", "a".repeat(64), "b".repeat(64)),
        ]
        .iter()
        .enumerate()
        {
            let path = dir.join(format!("t{index}"));
            std::fs::write(&path, content).unwrap();
            let error = load_or_create(&path).unwrap_err();
            assert!(error.contains("does not hold a control token"), "{error}");
            assert!(!error.contains("secret"), "the content is not echoed");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), **content);
        }
        // Not UTF-8, a directory, a parent that cannot be a folder: errors,
        // not panics.
        let binary = dir.join("binary");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(load_or_create(&binary).is_err());
        assert!(load_or_create(&dir).is_err());
        // (Reading beneath a file is "not a directory", not "not found".)
        let error = load_or_create(&binary.join("under/a/file/t")).unwrap_err();
        assert!(error.contains("control token file"), "{error}");
        // Missing folders are made (2026-10-30: the suggested token file's
        // configuration folder may not exist yet).
        let deep = dir.join("no/such/dir/t");
        let token = load_or_create(&deep).unwrap();
        assert_eq!(load_or_create(&deep).unwrap(), token);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The file is read first and created only when it is missing; should
    /// one appear in between (another instance starting, or a link put
    /// there on purpose), it is not written over or through.
    #[test]
    fn creating_never_replaces_a_file_that_is_there() {
        let dir = dir("race");
        let path = dir.join("control.token");
        std::fs::write(&path, "someone else's file").unwrap();
        let error = create(&path, &"a".repeat(64)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "someone else's file"
        );
        #[cfg(unix)]
        {
            let target = dir.join("elsewhere");
            let link = dir.join("link.token");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(create(&link, &"a".repeat(64)).is_err());
            assert!(!target.exists(), "nothing was written through the link");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_whole_token_matches() {
        let token = "0123456789abcdef".repeat(4);
        assert!(matches(&token, &token));
        assert!(matches(&token, &token.to_ascii_uppercase()));
        let mut last_wrong = token.clone();
        last_wrong.pop();
        last_wrong.push('0');
        for wrong in [
            "",
            "0",
            &token[..63],
            &format!("{token}0"),
            &format!("{token}{token}"),
            &last_wrong,
            &"f".repeat(64),
            &format!(" {}", &token[1..]),
        ] {
            assert!(!matches(&token, wrong), "{wrong}");
        }
        assert!(!matches("", ""), "no token, no entry");
    }
}
