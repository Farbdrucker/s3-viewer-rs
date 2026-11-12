//! A small XML reader. It covers what S3 responses and the preview need:
//! elements, attributes, text, comments, CDATA and declarations.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Qualified name as written, for example `s3:Key`.
    pub name: String,
    /// Attribute names and values, values still escaped as in the source.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    /// Name without a namespace prefix.
    pub fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.local_name() == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |element| element.local_name() == name)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            _ => None,
        })
    }

    /// Unescaped text of all direct text and CDATA children.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.children {
            match node {
                Node::Text(text) => out.push_str(&unescape(text)),
                Node::CData(text) => out.push_str(text),
                _ => {}
            }
        }
        out
    }

    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name).map(Element::text)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub declaration: Option<String>,
    pub doctype: Option<String>,
    /// Comments before the root element.
    pub prolog_comments: Vec<String>,
    pub root: Element,
}

#[derive(Debug)]
pub struct XmlError(pub String);

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn parse(text: &str) -> Result<Document, XmlError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = Parser { src: text, pos: 0 };
    let mut declaration = None;
    let mut doctype = None;
    let mut prolog_comments = Vec::new();
    let root = loop {
        parser.skip_whitespace();
        if parser.eof() {
            return Err(XmlError("no root element".into()));
        }
        if parser.starts_with("<?") {
            let raw = parser.take_until("?>", "<?")?;
            if raw.starts_with("<?xml") && declaration.is_none() {
                declaration = Some(raw.to_owned());
            }
        } else if parser.starts_with("<!--") {
            let raw = parser.take_until("-->", "<!--")?;
            prolog_comments.push(raw[4..raw.len() - 3].to_owned());
        } else if parser.starts_with("<!DOCTYPE") || parser.starts_with("<!doctype") {
            doctype = Some(parser.take_doctype()?.to_owned());
        } else if parser.starts_with("<") {
            break parser.element()?;
        } else {
            return Err(XmlError("text before the root element".into()));
        }
    };
    loop {
        parser.skip_whitespace();
        if parser.eof() {
            break;
        }
        if parser.starts_with("<!--") {
            parser.take_until("-->", "<!--")?;
        } else if parser.starts_with("<?") {
            parser.take_until("?>", "<?")?;
        } else {
            return Err(XmlError("content after the root element".into()));
        }
    }
    Ok(Document { declaration, doctype, prolog_comments, root })
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.rest().starts_with(prefix)
    }

    fn skip_whitespace(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    fn take_until(&mut self, end: &str, what: &str) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        match rest.find(end) {
            Some(index) => {
                let raw = &rest[..index + end.len()];
                self.pos += raw.len();
                Ok(raw)
            }
            None => Err(XmlError(format!("unterminated {what}"))),
        }
    }

    fn take_doctype(&mut self) -> Result<&'a str, XmlError> {
        // A doctype may contain an internal subset in square brackets.
        let rest = self.rest();
        let mut depth = 0usize;
        for (index, ch) in rest.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                '>' if depth == 0 => {
                    self.pos += index + 1;
                    return Ok(&rest[..=index]);
                }
                _ => {}
            }
        }
        Err(XmlError("unterminated doctype".into()))
    }

    fn name(&mut self) -> Result<&'a str, XmlError> {
        let rest = self.rest();
        let end = rest.find(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '/' | '=' | '<')).unwrap_or(rest.len());
        if end == 0 {
            return Err(XmlError("expected a name".into()));
        }
        self.pos += end;
        Ok(&rest[..end])
    }

    fn element(&mut self) -> Result<Element, XmlError> {
        self.pos += 1; // '<'
        let name = self.name()?.to_owned();
        let mut attrs = Vec::new();
        loop {
            self.skip_whitespace();
            if self.starts_with("/>") {
                self.pos += 2;
                return Ok(Element { name, attrs, children: Vec::new() });
            }
            if self.starts_with(">") {
                self.pos += 1;
                break;
            }
            if self.eof() {
                return Err(XmlError(format!("unterminated tag <{name}>")));
            }
            let attr = self.name()?.to_owned();
            self.skip_whitespace();
            if !self.starts_with("=") {
                return Err(XmlError(format!("attribute {attr} without value")));
            }
            self.pos += 1;
            self.skip_whitespace();
            let quote = self.rest().chars().next().ok_or_else(|| XmlError("unexpected end".into()))?;
            if quote != '"' && quote != '\'' {
                return Err(XmlError(format!("attribute {attr} needs quotes")));
            }
            self.pos += 1;
            let rest = self.rest();
            let end = rest.find(quote).ok_or_else(|| XmlError("unterminated attribute".into()))?;
            attrs.push((attr, rest[..end].to_owned()));
            self.pos += end + 1;
        }
        let mut children = Vec::new();
        loop {
            if self.eof() {
                return Err(XmlError(format!("missing </{name}>")));
            }
            if self.starts_with("</") {
                self.pos += 2;
                let end_name = self.name()?;
                if end_name != name {
                    return Err(XmlError(format!("</{end_name}> does not close <{name}>")));
                }
                self.skip_whitespace();
                if !self.starts_with(">") {
                    return Err(XmlError(format!("unterminated </{name}>")));
                }
                self.pos += 1;
                return Ok(Element { name, attrs, children });
            }
            if self.starts_with("<!--") {
                let raw = self.take_until("-->", "<!--")?;
                children.push(Node::Comment(raw[4..raw.len() - 3].to_owned()));
            } else if self.starts_with("<![CDATA[") {
                let raw = self.take_until("]]>", "<![CDATA[")?;
                children.push(Node::CData(raw[9..raw.len() - 3].to_owned()));
            } else if self.starts_with("<?") {
                self.take_until("?>", "<?")?;
            } else if self.starts_with("<") {
                children.push(Node::Element(self.element()?));
            } else {
                let rest = self.rest();
                let end = rest.find('<').unwrap_or(rest.len());
                children.push(Node::Text(rest[..end].to_owned()));
                self.pos += end;
            }
        }
    }
}

pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';').filter(|&end| end <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..end];
        let resolved = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match resolved {
            Some(ch) => {
                out.push(ch);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Re-indents a document with two spaces per level. Elements that only hold
/// text stay on one line, whitespace-only text disappears.
pub fn pretty_print(text: &str) -> Result<String, XmlError> {
    let document = parse(text)?;
    let mut out = String::new();
    if let Some(declaration) = &document.declaration {
        out.push_str(declaration);
        out.push('\n');
    }
    for comment in &document.prolog_comments {
        let _ = writeln!(out, "<!--{comment}-->");
    }
    if let Some(doctype) = &document.doctype {
        out.push_str(doctype);
        out.push('\n');
    }
    write_element(&mut out, &document.root, 0);
    Ok(out)
}

fn write_element(out: &mut String, element: &Element, depth: usize) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&element.name);
    for (name, value) in &element.attrs {
        let quote = if value.contains('"') { '\'' } else { '"' };
        let _ = write!(out, " {name}={quote}{value}{quote}");
    }
    let meaningful: Vec<&Node> =
        element.children.iter().filter(|node| !matches!(node, Node::Text(text) if text.trim().is_empty())).collect();
    if meaningful.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push('>');
    let text_only = meaningful.iter().all(|node| matches!(node, Node::Text(_) | Node::CData(_)));
    if text_only {
        for node in &meaningful {
            match node {
                Node::Text(text) => out.push_str(text.trim()),
                Node::CData(text) => {
                    let _ = write!(out, "<![CDATA[{text}]]>");
                }
                _ => {}
            }
        }
    } else {
        out.push('\n');
        let inner = "  ".repeat(depth + 1);
        for node in meaningful {
            match node {
                Node::Element(child) => write_element(out, child, depth + 1),
                Node::Text(text) => {
                    let _ = writeln!(out, "{inner}{}", text.trim());
                }
                Node::CData(text) => {
                    let _ = writeln!(out, "{inner}<![CDATA[{text}]]>");
                }
                Node::Comment(text) => {
                    let _ = writeln!(out, "{inner}<!--{text}-->");
                }
            }
        }
        out.push_str(&indent);
    }
    let _ = writeln!(out, "</{}>", element.name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_s3_listing() {
        let doc = parse(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>b</Name><IsTruncated>false</IsTruncated>
  <Contents><Key>a &amp; b.txt</Key><Size>12</Size></Contents>
  <CommonPrefixes><Prefix>dir/</Prefix></CommonPrefixes>
</ListBucketResult>"#,
        )
        .unwrap();
        let contents = doc.root.child("Contents").unwrap();
        assert_eq!(contents.child_text("Key").unwrap(), "a & b.txt");
        assert_eq!(doc.root.child("CommonPrefixes").unwrap().child_text("Prefix").unwrap(), "dir/");
    }

    #[test]
    fn unescapes_numeric_entities() {
        assert_eq!(unescape("&#65;&#x42;&lt;&unknown; & x"), "AB<&unknown; & x");
    }

    #[test]
    fn pretty_prints() {
        let out = pretty_print("<a x='1'><b>text</b><c/><!-- note --><d><e>1</e></d></a>").unwrap();
        assert_eq!(out, "<a x=\"1\">\n  <b>text</b>\n  <c/>\n  <!-- note -->\n  <d>\n    <e>1</e>\n  </d>\n</a>\n");
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(pretty_print("<a><b></a>").is_err());
        assert!(pretty_print("<a>").is_err());
        assert!(pretty_print("plain").is_err());
    }
}
