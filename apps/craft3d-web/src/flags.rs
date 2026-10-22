//! The flags of the page's address (`?webgl&open=part.3dcraft`): which
//! renderer, and a document to open at the start. They work on an
//! iframe's `src` too (packaging/web/HOSTING.md).

/// What the address asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `?webgl`: WebGL2, also where WebGPU is there (a WebGPU driver that
    /// misbehaves).
    pub webgl: bool,
    /// `?cpu`: the viewport rendered by the CPU reference renderer, on
    /// the page's own thread.
    pub cpu: bool,
    /// `?open=<url>`: a `.3dcraft` or STEP file of the same site, opened
    /// at the start.
    pub open: Option<String>,
}

/// The longest `open` address taken.
const MAX_OPEN: usize = 2048;

/// Reads the query part of the address (`location.search`, with or
/// without its `?`). Unknown flags are left alone; an `open` that could
/// lead to another site (a scheme, or `//`) is dropped.
pub fn parse(search: &str) -> Flags {
    let mut flags = Flags::default();
    let query = search.strip_prefix('?').unwrap_or(search);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "webgl" => flags.webgl = true,
            "cpu" => flags.cpu = true,
            "open" => flags.open = decode(value).filter(|url| same_site(url)),
            _ => {}
        }
    }
    flags
}

/// True for an address on the page's own site: relative, or a path from
/// its root; no scheme, no other host.
pub fn same_site(url: &str) -> bool {
    !url.is_empty()
        && url.len() <= MAX_OPEN
        && !url.contains(':')
        && !url.starts_with("//")
        && !url.starts_with("\\\\")
        && !url.chars().any(char::is_control)
}

/// Percent-decoding of a query value (`+` is a space); `None` when it is
/// not UTF-8 or a `%` is not followed by two hex digits.
pub fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let text = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(text, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The file name at the end of an address, for the document's name.
pub fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("download")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_read_from_the_query() {
        assert_eq!(parse(""), Flags::default());
        assert!(parse("?webgl").webgl);
        let flags = parse("?cpu&x=1&open=models%2Fa%20b.3dcraft");
        assert!(flags.cpu && !flags.webgl);
        assert_eq!(flags.open.as_deref(), Some("models/a b.3dcraft"));
        assert!(parse("webgl=1").webgl);
    }

    #[test]
    fn only_the_own_site_is_opened() {
        for bad in [
            "?open=https://evil.example/x.3dcraft",
            "?open=//evil.example/x.3dcraft",
            "?open=javascript:alert(1)",
            "?open=%",
            "?open=%zz",
            "?open=%ff",
            "?open=",
        ] {
            assert_eq!(parse(bad).open, None, "{bad}");
        }
        assert_eq!(parse("?open=/m/a.step").open.as_deref(), Some("/m/a.step"));
    }

    #[test]
    fn the_name_is_the_last_part() {
        assert_eq!(file_name("models/a.3dcraft?v=2"), "a.3dcraft");
        assert_eq!(file_name("a.step"), "a.step");
        assert_eq!(file_name("models/"), "download");
    }
}
