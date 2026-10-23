//! Source text → tokens.
//!
//! The lexer works on bytes and only ever steps over ASCII, so every
//! position it produces is a char boundary. Anything non-ASCII is reported
//! as an unexpected character and never sliced through.

use crate::error::{ErrorKind, ExprError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TokenKind {
    Number(f64),
    /// A name: variable, unit, function or `pi`. Its text is
    /// `source[pos..end]`.
    Ident,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
    /// `<`, `<=`, `>`, `>=`, `==`, `!=`.
    Less,
    LessEq,
    Greater,
    GreaterEq,
    EqEq,
    NotEq,
    /// `&&`, `||`, `!`.
    AndAnd,
    OrOr,
    Bang,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Token {
    pub kind: TokenKind,
    /// Byte range `pos..end` in the source.
    pub pos: usize,
    pub end: usize,
}

/// A lexing failure plus the offset at which scanning can resume, so that
/// renaming can still work on the rest of a broken expression.
#[derive(Debug)]
pub(crate) struct ScanError {
    pub error: ExprError,
    pub resume: usize,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// First offset at or after `from` whose byte does not satisfy `keep`.
fn skip_while(bytes: &[u8], from: usize, keep: impl Fn(u8) -> bool) -> usize {
    let mut at = from;
    while bytes.get(at).is_some_and(|&b| keep(b)) {
        at += 1;
    }
    at
}

/// The token starting at or after `from` (whitespace is skipped). At the
/// end of the text this returns `End` with `pos == end == source.len()`.
/// `from` must be a char boundary; the returned `end` is one too.
pub(crate) fn scan(source: &str, from: usize) -> Result<Token, ScanError> {
    let bytes = source.as_bytes();
    let pos = skip_while(bytes, from, |b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    let Some(&first) = bytes.get(pos) else {
        return Ok(Token {
            kind: TokenKind::End,
            pos: bytes.len(),
            end: bytes.len(),
        });
    };
    let simple = |kind| {
        Ok(Token {
            kind,
            pos,
            end: pos + 1,
        })
    };
    match first {
        b'+' => simple(TokenKind::Plus),
        b'-' => simple(TokenKind::Minus),
        b'*' => simple(TokenKind::Star),
        b'/' => simple(TokenKind::Slash),
        b'^' => simple(TokenKind::Caret),
        b'(' => simple(TokenKind::LParen),
        b')' => simple(TokenKind::RParen),
        b',' => simple(TokenKind::Comma),
        b'<' | b'>' | b'=' | b'!' | b'&' | b'|' => operator(bytes, pos, first),
        b if is_ident_start(b) => Ok(Token {
            kind: TokenKind::Ident,
            pos,
            end: skip_while(bytes, pos, is_ident_continue),
        }),
        b if b.is_ascii_digit() => number(source, pos),
        // `.5`, but a lone `.` is not a number.
        b'.' if bytes.get(pos + 1).is_some_and(u8::is_ascii_digit) => number(source, pos),
        _ => {
            // `pos` is a char boundary (see the module comment), so this
            // decodes the whole offending character.
            let ch = source
                .get(pos..)
                .and_then(|rest| rest.chars().next())
                .unwrap_or(char::REPLACEMENT_CHARACTER);
            Err(ScanError {
                error: ExprError::new(pos, ErrorKind::UnexpectedChar { ch }),
                resume: pos + ch.len_utf8(),
            })
        }
    }
}

/// A comparison or logical operator starting with `first` at `pos`. A
/// lone `=`, `&` or `|` is an error that names the operator meant.
fn operator(bytes: &[u8], pos: usize, first: u8) -> Result<Token, ScanError> {
    let second = bytes.get(pos + 1).copied();
    let (kind, len) = match (first, second) {
        (b'<', Some(b'=')) => (TokenKind::LessEq, 2),
        (b'<', _) => (TokenKind::Less, 1),
        (b'>', Some(b'=')) => (TokenKind::GreaterEq, 2),
        (b'>', _) => (TokenKind::Greater, 1),
        (b'=', Some(b'=')) => (TokenKind::EqEq, 2),
        (b'!', Some(b'=')) => (TokenKind::NotEq, 2),
        (b'!', _) => (TokenKind::Bang, 1),
        (b'&', Some(b'&')) => (TokenKind::AndAnd, 2),
        (b'|', Some(b'|')) => (TokenKind::OrOr, 2),
        _ => {
            let (found, meant) = match first {
                b'=' => ('=', "=="),
                b'&' => ('&', "&&"),
                _ => ('|', "||"),
            };
            return Err(ScanError {
                error: ExprError::new(pos, ErrorKind::LoneOperator { found, meant }),
                resume: pos + 1,
            });
        }
    };
    Ok(Token {
        kind,
        pos,
        end: pos + len,
    })
}

/// `digits [. digits] [e [+-] digits]` or `. digits [e …]`, starting at
/// `pos`. The `e` only counts as an exponent when digits follow, so that
/// `2 em` stays "number, name" and gets a sensible parse error.
fn number(source: &str, pos: usize) -> Result<Token, ScanError> {
    let bytes = source.as_bytes();
    let digit = |b: u8| b.is_ascii_digit();
    let mut end = skip_while(bytes, pos, digit);
    if bytes.get(end) == Some(&b'.') {
        end = skip_while(bytes, end + 1, digit);
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut digits_from = end + 1;
        if matches!(bytes.get(digits_from), Some(b'+' | b'-')) {
            digits_from += 1;
        }
        let exponent_end = skip_while(bytes, digits_from, digit);
        if exponent_end > digits_from {
            end = exponent_end;
        }
    }
    let value = source
        .get(pos..end)
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite());
    match value {
        Some(value) => Ok(Token {
            kind: TokenKind::Number(value),
            pos,
            end,
        }),
        None => Err(ScanError {
            error: ExprError::new(pos, ErrorKind::NumberOutOfRange),
            resume: end.max(pos + 1),
        }),
    }
}

/// All tokens of `source`, ending with `End`. The caller has already
/// capped the length of `source`, which bounds the token count.
pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, ExprError> {
    let mut tokens = Vec::new();
    let mut at = 0;
    loop {
        let token = scan(source, at).map_err(|e| e.error)?;
        tokens.push(token);
        if token.kind == TokenKind::End {
            return Ok(tokens);
        }
        at = token.end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source).unwrap().iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers() {
        use TokenKind::{End, Number};
        assert_eq!(kinds("25"), [Number(25.0), End]);
        assert_eq!(kinds("2.5"), [Number(2.5), End]);
        assert_eq!(kinds(".5"), [Number(0.5), End]);
        assert_eq!(kinds("5."), [Number(5.0), End]);
        assert_eq!(kinds("1.5e-3"), [Number(1.5e-3), End]);
        assert_eq!(kinds("2E+2"), [Number(200.0), End]);
        assert_eq!(kinds("1e3"), [Number(1000.0), End]);
    }

    #[test]
    fn exponent_needs_digits() {
        use TokenKind::{End, Ident, Minus, Number};
        // `e` without digits is a name, not an exponent.
        assert_eq!(kinds("2e"), [Number(2.0), Ident, End]);
        assert_eq!(kinds("2e-x"), [Number(2.0), Ident, Minus, Ident, End]);
        assert_eq!(kinds("25mm"), [Number(25.0), Ident, End]);
    }

    #[test]
    fn operators_and_positions() {
        let tokens = tokenize(" a_1+( b ) ").unwrap();
        let spans: Vec<(usize, usize)> = tokens.iter().map(|t| (t.pos, t.end)).collect();
        assert_eq!(spans, [(1, 4), (4, 5), (5, 6), (7, 8), (9, 10), (11, 11)]);
        use TokenKind::*;
        assert_eq!(
            kinds("+-*/^(),"),
            [Plus, Minus, Star, Slash, Caret, LParen, RParen, Comma, End]
        );
    }

    #[test]
    fn comparison_and_logical_operators() {
        use TokenKind::*;
        assert_eq!(
            kinds("< <= > >= == != && || !"),
            [
                Less, LessEq, Greater, GreaterEq, EqEq, NotEq, AndAnd, OrOr, Bang, End
            ]
        );
        // Longest match, without spaces.
        assert_eq!(
            kinds("a<=b!=!c"),
            [Ident, LessEq, Ident, NotEq, Bang, Ident, End]
        );
        assert_eq!(kinds("!!a"), [Bang, Bang, Ident, End]);
        assert_eq!(kinds("<<"), [Less, Less, End]);
        let spans: Vec<(usize, usize)> = tokenize("a>=1")
            .unwrap()
            .iter()
            .map(|t| (t.pos, t.end))
            .collect();
        assert_eq!(spans, [(0, 1), (1, 3), (3, 4), (4, 4)]);
    }

    #[test]
    fn a_lone_equals_ampersand_or_bar_names_the_operator_meant() {
        for (source, pos, found, meant) in [
            ("a = 2", 2, '=', "=="),
            ("a & b", 2, '&', "&&"),
            ("a | b", 2, '|', "||"),
            ("a =", 2, '=', "=="),
            ("|", 0, '|', "||"),
        ] {
            assert_eq!(
                tokenize(source).unwrap_err(),
                ExprError::new(pos, ErrorKind::LoneOperator { found, meant }),
                "{source}"
            );
        }
        assert!(tokenize("a = 2").unwrap_err().to_string().contains("`==`"));
    }

    #[test]
    fn unexpected_characters_carry_their_position() {
        let err = tokenize("2 $ 3").unwrap_err();
        assert_eq!(
            err,
            ExprError::new(2, ErrorKind::UnexpectedChar { ch: '$' })
        );
        let err = tokenize("1 . 2").unwrap_err();
        assert_eq!(
            err,
            ExprError::new(2, ErrorKind::UnexpectedChar { ch: '.' })
        );
    }

    #[test]
    fn non_ascii_is_an_error_not_a_panic() {
        let err = tokenize("2 × 3").unwrap_err();
        assert_eq!(
            err,
            ExprError::new(2, ErrorKind::UnexpectedChar { ch: '×' })
        );
        // A multi-byte character right after a name and inside a number.
        let err = tokenize("largeur_é").unwrap_err();
        assert_eq!(
            err,
            ExprError::new(8, ErrorKind::UnexpectedChar { ch: 'é' })
        );
        let err = tokenize("1.5µm").unwrap_err();
        assert_eq!(err.pos, 3);
        let err = tokenize("\u{1F600}").unwrap_err();
        assert_eq!(err.pos, 0);
    }

    #[test]
    fn scan_error_resumes_after_the_character() {
        let Err(e) = scan("é+1", 0) else {
            panic!("expected an error");
        };
        assert_eq!(e.resume, 2);
        assert_eq!(scan("é+1", e.resume).unwrap().kind, TokenKind::Plus);
    }

    #[test]
    fn huge_literal_is_out_of_range() {
        let err = tokenize("1 + 1e999").unwrap_err();
        assert_eq!(err, ExprError::new(4, ErrorKind::NumberOutOfRange));
    }
}
