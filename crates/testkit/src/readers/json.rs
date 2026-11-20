//! A JSON reader of its own, from RFC 8259 alone, for the readers of
//! formats that hold JSON (glTF): independent of the writers and of
//! `serde_json`, and strict where the RFC is: UTF-8, one value with white
//! space around it, strings with their escapes (surrogate pairs joined, a
//! lone one refused) and no raw control character, numbers by the RFC's
//! grammar (read as the `f64` nearest), and objects whose keys are unique
//! (the RFC says they should be; a writer of ours that repeats one is at
//! fault). Nesting is limited to 128, so no input can exhaust the stack.

use std::fmt::Write as _;

/// A JSON value. An object keeps its members in the order of the text.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// The deepest nesting read.
const MAX_DEPTH: usize = 128;

impl Json {
    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The element `index` of an array.
    pub fn item(&self, index: usize) -> Option<&Json> {
        self.as_array()?.get(index)
    }

    /// The value at a path of object keys.
    pub fn at(&self, path: &[&str]) -> Option<&Json> {
        path.iter().try_fold(self, |value, key| value.get(key))
    }

    /// An object's keys, in order.
    pub fn keys(&self) -> Vec<&str> {
        match self {
            Json::Object(members) => members.iter().map(|(k, _)| k.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// A number that is a whole number from 0 to 2^53, which an `f64` holds
    /// exactly.
    pub fn as_u64(&self) -> Option<u64> {
        let n = self.as_f64()?;
        // Whole and within 0..=2^53 before the cast.
        (n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_992.0).then_some(n as u64)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// Reads `bytes` as one JSON text (RFC 8259).
pub fn parse_json(bytes: &[u8]) -> Result<Json, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("JSON: not UTF-8: {e}"))?;
    let mut parser = Parser { text, at: 0 };
    let value = parser.value(0)?;
    parser.space();
    if parser.at != text.len() {
        return Err(format!("JSON: more after the value, at byte {}", parser.at));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at byte {}", self.at))
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            self.fail(&format!("`{}` expected", char::from(byte)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.fail("nested too deep");
        }
        self.space();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.fail("a value expected"),
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        let end = self.at.saturating_add(word.len());
        if self.text.as_bytes().get(self.at..end) == Some(word.as_bytes()) {
            self.at = end;
            Ok(value)
        } else {
            self.fail(&format!("`{word}` expected"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.space();
            let key = self.string()?;
            self.space();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            if members.iter().any(|(k, _)| *k == key) {
                return self.fail(&format!("the key `{key}` a second time in one object"));
            }
            members.push((key, value));
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(members));
                }
                _ => return self.fail("`,` or `}` expected"),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.space();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return self.fail("`,` or `]` expected"),
            }
        }
    }

    /// Four hexadecimal digits of a `\u` escape.
    fn hex4(&mut self) -> Result<u32, String> {
        let end = self.at.saturating_add(4);
        let digits = self
            .text
            .get(self.at..end)
            .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| format!("JSON: four hexadecimal digits expected at byte {}", self.at))?;
        let code = u32::from_str_radix(digits, 16).map_err(|e| e.to_string())?;
        self.at = end;
        Ok(code)
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return self.fail("a string without its end"),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(0..=0x1f) => return self.fail("a control character in a string"),
                Some(b'\\') => {
                    self.at += 1;
                    let escape = self.peek();
                    self.at += 1;
                    match escape {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let high = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&high) {
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return self.fail("a high surrogate without its low one");
                                }
                                0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&high) {
                                return self.fail("a low surrogate on its own");
                            } else {
                                high
                            };
                            match char::from_u32(code) {
                                Some(c) => out.push(c),
                                None => return self.fail("an escape that is no character"),
                            }
                        }
                        _ => return self.fail("an unknown escape"),
                    }
                }
                Some(_) => {
                    let Some(c) = self
                        .text
                        .get(self.at..)
                        .and_then(|rest| rest.chars().next())
                    else {
                        return self.fail("a broken character");
                    };
                    out.push(c);
                    self.at += c.len_utf8();
                }
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.fail("a digit expected"),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return self.fail("a digit expected after `.`");
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return self.fail("a digit expected in the exponent");
            }
        }
        let token = self
            .text
            .get(start..self.at)
            .ok_or_else(|| format!("JSON: a number at byte {start}"))?;
        let value: f64 = token.parse().map_err(|e| format!("JSON: `{token}`: {e}"))?;
        if !value.is_finite() {
            return self.fail("a number too large for an f64");
        }
        Ok(Json::Number(value))
    }
}

/// `value` as compact JSON text, for messages.
pub fn json_text(value: &Json) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => {
            let _ = write!(out, "{n}");
        }
        Json::String(s) => {
            let _ = write!(out, "{s:?}");
        }
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Json::Object(members) => {
            out.push('{');
            for (i, (key, item)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{key:?}:");
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_strings_and_numbers_by_the_rfc() {
        let value = parse_json(
            br#" {"a": [1, -0.5, 2e3, 1E-2, 0], "b": {"c": null, "d": true, "e": false},
                "s": "q\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00\u0001 x"} "#,
        )
        .unwrap();
        let a: Vec<f64> = value
            .get("a")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .map(|n| n.as_f64().unwrap())
            .collect();
        assert_eq!(a, [1.0, -0.5, 2000.0, 0.01, 0.0]);
        assert_eq!(value.at(&["b", "c"]), Some(&Json::Null));
        assert_eq!(value.at(&["b", "d"]).and_then(Json::as_bool), Some(true));
        assert_eq!(value.keys(), ["a", "b", "s"]);
        assert_eq!(
            value.get("s").and_then(Json::as_str),
            Some("q\"\\/\u{8}\u{c}\n\r\t\u{e9}\u{1f600}\u{1} x")
        );
        assert_eq!(
            value.at(&["a"]).and_then(|a| a.item(1)),
            Some(&Json::Number(-0.5))
        );
        assert_eq!(Json::Number(3.0).as_u64(), Some(3));
        assert_eq!(Json::Number(3.5).as_u64(), None);
        assert_eq!(Json::Number(-1.0).as_u64(), None);
        assert_eq!(
            json_text(&parse_json(br#"{"k":[1,"x"]}"#).unwrap()),
            r#"{"k":[1,"x"]}"#
        );
    }

    #[test]
    fn what_the_rfc_does_not_allow_is_refused() {
        for bad in [
            &b""[..],
            b"{",
            b"[1,]",
            b"{\"a\":1,}",
            b"{\"a\":1,\"a\":2}",
            b"01",
            b"1.",
            b".5",
            b"1e",
            b"+1",
            b"1e400",
            b"\"\x01\"",
            b"\"\\x\"",
            b"\"\\ud800\"",
            b"\"\\udc00\"",
            b"\"\xff\"",
            b"nul",
            b"[1] [2]",
            b"NaN",
        ] {
            assert!(
                parse_json(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // Deep nesting is an error, not a stack overflow.
        let deep = "[".repeat(10_000);
        assert!(parse_json(deep.as_bytes()).is_err());
    }
}
