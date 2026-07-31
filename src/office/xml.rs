//! Reading XML, one tag at a time.
//!
//! Both office formats are XML inside a zip, so something has to read it. This
//! is a scanner rather than a parser: it hands back tags and text in the order
//! they appear and keeps no tree, because the two readers that use it both walk
//! the document once and neither wants a copy of it in memory.
//!
//! # What it does not do
//!
//! It does not validate, resolve external entities, or follow a DTD. That is
//! deliberate. A validating parser on an untrusted file is a way to be handed a
//! document that asks to read `/etc/passwd`, or one that expands to a terabyte
//! from four kilobytes of entity definitions — both are old, real attacks on
//! exactly this kind of code. Nothing here fetches anything, and the only
//! entities understood are the five XML defines plus numeric ones.
//!
//! Names come back without their prefix. `<w:p>` and `<p>` are both `p`. The
//! prefix in an office document is bound to a namespace by the file itself and
//! could in principle be anything, so matching on the local name is both
//! simpler and more forgiving than matching on `w:p` — and within one
//! `document.xml` or `content.xml` there is no ambiguity about what `p` means.

/// What the scanner hands back.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `<name …>`. A self-closing tag gives this and then [`Event::End`].
    Start(Tag),
    End(String),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// The name without its prefix: `w:tblPr` is `tblPr`.
    pub name: String,
    /// The prefix, kept for the rare case where it matters.
    pub prefix: String,
    pub attrs: Vec<(String, String)>,
    /// True when the tag closed itself, so it has no children.
    pub empty: bool,
}

impl Tag {
    /// An attribute by local name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| local(key) == name)
            .map(|(_, value)| value.as_str())
    }

    /// An attribute read as a number, when it is one.
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name)?.trim().parse().ok()
    }

    /// Whether a flag attribute is on. Word writes `w:val="0"` or `"false"` to
    /// turn something off and leaves it out — or writes nothing at all — to
    /// turn it on, so a missing `val` means yes.
    pub fn on(&self) -> bool {
        match self.get("val") {
            None => true,
            Some(value) => !matches!(value.trim(), "0" | "false" | "off" | "none"),
        }
    }
}

/// A name without its namespace prefix.
pub fn local(name: &str) -> &str {
    match name.rfind(':') {
        Some(at) => &name[at + 1..],
        None => name,
    }
}

pub struct Reader<'a> {
    text: &'a str,
    at: usize,
    /// Set when a self-closing tag has handed back its start and owes its end.
    owed: Option<String>,
}

impl<'a> Reader<'a> {
    pub fn new(text: &'a str) -> Reader<'a> {
        Reader {
            text: text.strip_prefix('\u{feff}').unwrap_or(text),
            at: 0,
            owed: None,
        }
    }

    /// Everything after this point, for skipping a subtree by hand.
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    /// Read to the end of the element that has just started.
    ///
    /// The depth count is what makes this safe: a `w:p` inside a `w:p` — which
    /// happens in every text box in every Word document — would otherwise end
    /// the outer one at the inner one's closing tag and put the rest of the
    /// document in the wrong place.
    pub fn skip_element(&mut self, name: &str) {
        let mut depth = 1usize;
        // Not a `for` loop: it would borrow the scanner for the whole body,
        // and every caller of this is itself in the middle of walking one.
        #[allow(clippy::while_let_on_iterator)]
        while let Some(event) = self.next() {
            match event {
                Event::Start(tag) if tag.name == name && !tag.empty => depth += 1,
                Event::End(ended) if ended == name => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

impl Iterator for Reader<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        if let Some(name) = self.owed.take() {
            return Some(Event::End(name));
        }

        // A loop rather than a recursive call, because a file may hold any
        // number of comments in a row and each one would be a stack frame.
        let rest = loop {
            if self.at >= self.text.len() {
                return None;
            }
            let rest = self.rest();
            if !rest.starts_with('<') {
                // Text, up to the next tag.
                let end = rest.find('<').unwrap_or(rest.len());
                self.at += end;
                return Some(Event::Text(unescape(&rest[..end])));
            }

            // Everything that begins `<!` or `<?` is not an element: a comment,
            // a CDATA section, a doctype, or the declaration at the top.
            if let Some(after) = rest.strip_prefix("<!--") {
                let end = after.find("-->").map(|at| at + 3).unwrap_or(after.len());
                self.at += 4 + end;
                continue;
            }
            if let Some(after) = rest.strip_prefix("<![CDATA[") {
                let end = after.find("]]>").unwrap_or(after.len());
                let text = after[..end].to_string();
                self.at += 9 + end + 3.min(after.len() - end);
                return Some(Event::Text(text));
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>').map(|at| at + 1).unwrap_or(rest.len());
                self.at += end;
                continue;
            }
            break rest;
        };

        // A closing tag.
        if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>').unwrap_or(after.len());
            let name = local(after[..end].trim()).to_string();
            self.at += 2 + end + 1.min(after.len() - end);
            return Some(Event::End(name));
        }

        // An opening tag. Finding its end means skipping any `>` that is inside
        // a quoted attribute value — `w:val="a>b"` is legal and does appear.
        let mut end = None;
        let mut quote: Option<char> = None;
        for (index, ch) in rest.char_indices().skip(1) {
            match (quote, ch) {
                (Some(open), c) if c == open => quote = None,
                (Some(_), _) => {}
                (None, '"') | (None, '\'') => quote = Some(ch),
                (None, '>') => {
                    end = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            // An unterminated tag at the end of a truncated file.
            self.at = self.text.len();
            return None;
        };

        let inside = &rest[1..end];
        let empty = inside.trim_end().ends_with('/');
        let inside = inside.trim_end().trim_end_matches('/');
        self.at += end + 1;

        let mut parts = inside.splitn(2, |c: char| c.is_ascii_whitespace());
        let qualified = parts.next().unwrap_or("").trim();
        let attrs = match parts.next() {
            Some(tail) => attributes(tail),
            None => Vec::new(),
        };
        let name = local(qualified).to_string();
        let prefix = match qualified.rfind(':') {
            Some(at) => qualified[..at].to_string(),
            None => String::new(),
        };
        if empty {
            self.owed = Some(name.clone());
        }
        Some(Event::Start(Tag {
            name,
            prefix,
            attrs,
            empty,
        }))
    }
}

/// The attributes of a tag, from the text after its name.
fn attributes(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0usize;

    while at < bytes.len() {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let name_from = at;
        while at < bytes.len() && bytes[at] != b'=' && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == name_from {
            break;
        }
        let name = text[name_from..at].to_string();

        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at >= bytes.len() || bytes[at] != b'=' {
            // A bare attribute with no value. Not legal XML, but a file that
            // has one is better read than refused.
            out.push((name, String::new()));
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let Some(&quote) = bytes.get(at) else { break };
        if quote != b'"' && quote != b'\'' {
            break;
        }
        at += 1;
        let value_from = at;
        while at < bytes.len() && bytes[at] != quote {
            at += 1;
        }
        out.push((name, unescape(&text[value_from..at])));
        at += 1;
    }
    out
}

/// Turn `&amp;` and its friends back into characters.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // Twelve characters, not twelve bytes. `rest[..12]` panics outright if
        // byte 12 lands inside a character — `&` followed by anything
        // accented, which in a Word file means any European name — and the
        // longest entity there is (`&#x10FFFF;`) is ten characters, so
        // counting characters loses nothing.
        let far_enough = rest
            .char_indices()
            .nth(12)
            .map(|(offset, _)| offset)
            .unwrap_or(rest.len());
        let Some(end) = rest[..far_enough].find(';') else {
            // A bare ampersand, which is not legal and is also what a
            // hand-edited file is full of.
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let resolved = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => numeric(name),
        };
        match resolved {
            Some(ch) => out.push(ch),
            // An entity nobody defined here stays as it was written, which at
            // least shows what the file said.
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `&#233;` and `&#xE9;`.
fn numeric(name: &str) -> Option<char> {
    let digits = name.strip_prefix('#')?;
    let value = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(value)
}

/// The text of a document, whatever it was encoded in.
///
/// UTF-8 is what every producer of these files writes. UTF-16 turns up in
/// hand-made and converted files often enough to be worth the twenty lines,
/// and is unmistakable from its byte-order mark.
pub fn decode(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let big_endian = bytes[0] == 0xFE;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests;
