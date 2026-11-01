//! A small XML reader (XML 1.0, enough for FreeCAD's `Document.xml` and
//! `GuiDocument.xml`): elements with their attributes, nested; text
//! content, comments, processing instructions, CDATA and the document
//! type declaration are read past. The five predefined entities and
//! character references are decoded in attribute values.
//!
//! Hostile input: nesting is at most [`MAX_DEPTH`] deep and a file holds
//! at most [`MAX_ELEMENTS`] elements; anything malformed is an error,
//! never a panic.

/// The deepest elements may nest.
pub const MAX_DEPTH: usize = 128;
/// The most elements a file may hold.
pub const MAX_ELEMENTS: usize = 4_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The children named `name`.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every element below this one, depth first (bounded by the depth
    /// of the tree, which [`parse`] bounds).
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut stack: Vec<&Element> = self.children.iter().rev().collect();
        while let Some(e) = stack.pop() {
            out.push(e);
            stack.extend(e.children.iter().rev());
        }
        out
    }
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.')
}

/// Decodes `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&#n;`, `&#xh;`.
fn decode(raw: &str) -> Result<String, String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|&e| e <= 12) else {
            return Err("an `&` that starts no entity".to_string());
        };
        let entity = after.get(..end).unwrap_or("");
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            other => {
                let code = if let Some(hex) = other.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = other.strip_prefix('#') {
                    dec.parse::<u32>().ok()
                } else {
                    None
                };
                code.and_then(char::from_u32)
                    .ok_or_else(|| format!("the entity `&{}`; is not known", clip(other)))?
            }
        };
        out.push(c);
        rest = after.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(out)
}

fn clip(s: &str) -> String {
    s.chars().take(24).collect()
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
    elements: usize,
}

impl<'a> Reader<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or("")
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        let line = self
            .text
            .get(..self.at)
            .map_or(0, |t| t.bytes().filter(|&b| b == b'\n').count())
            + 1;
        Err(format!("line {line}: {what}"))
    }

    /// Moves past `marker`, or fails.
    fn past(&mut self, marker: &str, what: &str) -> Result<(), String> {
        match self.rest().find(marker) {
            Some(i) => {
                self.at += i + marker.len();
                Ok(())
            }
            None => self.fail(&format!("{what} that does not end")),
        }
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    fn name(&mut self) -> Result<&'a str, String> {
        let rest = self.rest();
        let len = rest
            .char_indices()
            .find(|&(_, c)| !is_name_char(c))
            .map_or(rest.len(), |(i, _)| i);
        if len == 0 {
            return self.fail("a name is missing");
        }
        self.at += len;
        Ok(rest.get(..len).unwrap_or(""))
    }

    /// Reads past text, comments, processing instructions, CDATA and
    /// declarations up to the next `<` of an element or end tag; returns
    /// whether one is there.
    fn seek_tag(&mut self) -> Result<bool, String> {
        loop {
            let rest = self.rest();
            let Some(i) = rest.find('<') else {
                self.at = self.text.len();
                return Ok(false);
            };
            self.at += i;
            let rest = self.rest();
            if rest.starts_with("<!--") {
                self.past("-->", "a comment")?;
            } else if rest.starts_with("<?") {
                self.past("?>", "a processing instruction")?;
            } else if rest.starts_with("<![CDATA[") {
                self.past("]]>", "a CDATA section")?;
            } else if rest.starts_with("<!") {
                self.past(">", "a declaration")?;
            } else {
                return Ok(true);
            }
        }
    }

    /// The element whose `<` is at the cursor, and everything inside it.
    fn element(&mut self) -> Result<Element, String> {
        // An explicit stack instead of recursion.
        let mut stack: Vec<Element> = Vec::new();
        loop {
            // At a `<`.
            if self.rest().starts_with("</") {
                self.at += 2;
                let name = self.name()?;
                self.skip_space();
                if !self.rest().starts_with('>') {
                    return self.fail("an end tag without `>`");
                }
                self.at += 1;
                let Some(done) = stack.pop() else {
                    return self.fail("an end tag without a start tag");
                };
                if done.name != name {
                    return self.fail(&format!(
                        "`</{}>` closes `<{}>`",
                        clip(name),
                        clip(&done.name)
                    ));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => return Ok(done),
                }
            } else {
                self.at += 1;
                self.elements += 1;
                if self.elements > MAX_ELEMENTS {
                    return self.fail("too many elements");
                }
                let mut element = Element {
                    name: self.name()?.to_string(),
                    ..Element::default()
                };
                let closed = loop {
                    self.skip_space();
                    let rest = self.rest();
                    if rest.starts_with("/>") {
                        self.at += 2;
                        break true;
                    }
                    if rest.starts_with('>') {
                        self.at += 1;
                        break false;
                    }
                    let name = self.name()?.to_string();
                    self.skip_space();
                    if !self.rest().starts_with('=') {
                        return self.fail("an attribute without `=`");
                    }
                    self.at += 1;
                    self.skip_space();
                    let quote = match self.rest().chars().next() {
                        Some(q @ ('"' | '\'')) => q,
                        _ => return self.fail("an attribute value without quotes"),
                    };
                    self.at += 1;
                    let rest = self.rest();
                    let Some(end) = rest.find(quote) else {
                        return self.fail("an attribute value that does not end");
                    };
                    let value = decode(rest.get(..end).unwrap_or("")).or_else(|e| self.fail(&e))?;
                    self.at += end + 1;
                    element.attributes.push((name, value));
                };
                if closed {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(element),
                        None => return Ok(element),
                    }
                } else {
                    if stack.len() >= MAX_DEPTH {
                        return self.fail("elements nest too deep");
                    }
                    stack.push(element);
                }
            }
            if !self.seek_tag()? {
                return self.fail("the file ends inside an element");
            }
        }
    }
}

/// The root element of a document.
pub fn parse(text: &str) -> Result<Element, String> {
    let mut reader = Reader {
        text,
        at: 0,
        elements: 0,
    };
    if !reader.seek_tag()? {
        return Err("no element".to_string());
    }
    if reader.rest().starts_with("</") {
        return reader.fail("an end tag before any element");
    }
    reader.element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_entities() {
        let doc = parse(
            "<?xml version='1.0'?>\n<!-- c -->\n<A x=\"1 &amp; 2\" y='&#65;&#x42;'><B/><!-- <C/> -->text<C z=\"&lt;\"></C><![CDATA[<D/>]]></A>",
        )
        .unwrap();
        assert_eq!(doc.name, "A");
        assert_eq!(doc.attribute("x"), Some("1 & 2"));
        assert_eq!(doc.attribute("y"), Some("AB"));
        let names: Vec<&str> = doc.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["B", "C"]);
        assert_eq!(doc.child("C").and_then(|c| c.attribute("z")), Some("<"));
    }

    #[test]
    fn malformed_files_are_errors() {
        for bad in [
            "",
            "<A>",
            "<A></B>",
            "<A x=1/>",
            "<A x=\"1/>",
            "</A>",
            "<A x=\"&bogus;\"/>",
            "<A><!-- </A>",
            "<>",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let deep = "<a>".repeat(MAX_DEPTH + 2);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn every_prefix_and_byte_flip_is_safe() {
        let doc = "<?xml version='1.0'?><Doc a=\"1\"><O n='x &amp; y'><P/></O><Q></Q></Doc>";
        for i in 0..=doc.len() {
            if let Some(prefix) = doc.get(..i) {
                let _ = parse(prefix);
            }
        }
        let bytes = doc.as_bytes();
        for i in 0..bytes.len() {
            let mut b = bytes.to_vec();
            b[i] ^= 0x15;
            if let Ok(t) = std::str::from_utf8(&b) {
                let _ = parse(t);
            }
        }
    }
}
