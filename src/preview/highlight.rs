//! Colours JSON, YAML and XML in the preview. The scanners are forgiving,
//! because truncated previews often end in the middle of a token.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    Key,
    String,
    Number,
    Keyword,
    Comment,
    Tag,
    Attribute,
    Punct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    None,
    Json,
    Yaml,
    Xml,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

pub fn highlight(language: Language, text: &str) -> Vec<Span> {
    match language {
        Language::None => Vec::new(),
        Language::Json => json(text),
        Language::Yaml => yaml(text),
        Language::Xml => xml(text),
    }
}

/// Spans that overlap `start..end`, clipped to it. `spans` must be sorted.
pub fn spans_in(spans: &[Span], start: usize, end: usize) -> impl Iterator<Item = Span> + '_ {
    let first = spans.partition_point(|span| span.end <= start);
    spans[first..].iter().take_while(move |span| span.start < end).map(move |span| Span {
        start: span.start.max(start),
        end: span.end.min(end),
        token: span.token,
    })
}

struct Out(Vec<Span>);

impl Out {
    fn push(&mut self, start: usize, end: usize, token: Token) {
        if end > start {
            self.0.push(Span { start, end, token });
        }
    }
}

fn json(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut out = Out(Vec::new());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                let mut look = i;
                while look < bytes.len() && matches!(bytes[look], b' ' | b'\t' | b'\r' | b'\n') {
                    look += 1;
                }
                let token = if look < bytes.len() && bytes[look] == b':' { Token::Key } else { Token::String };
                out.push(start, i, token);
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    i += 1;
                }
                out.push(start, i, Token::Number);
            }
            b'a'..=b'z' | b'A'..=b'Z' => {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                if matches!(&text[start..i], "true" | "false" | "null") {
                    out.push(start, i, Token::Keyword);
                }
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                out.push(i, i + 1, Token::Punct);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out.0
}

const YAML_WORDS: &[&str] =
    &["true", "false", "yes", "no", "on", "off", "null", "~", "True", "False", "Null", "TRUE", "FALSE", "NULL"];

fn yaml(text: &str) -> Vec<Span> {
    let mut out = Out(Vec::new());
    let mut block_indent: Option<usize> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let base = offset;
        offset += line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let indent = content.len() - content.trim_start().len();
        let trimmed = content.trim_start();
        if let Some(block) = block_indent {
            if trimmed.is_empty() || indent > block {
                out.push(base, base + content.len(), Token::String);
                continue;
            }
            block_indent = None;
        }
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            out.push(base + indent, base + content.len(), Token::Comment);
            continue;
        }
        if trimmed == "---" || trimmed == "..." {
            out.push(base + indent, base + content.len(), Token::Punct);
            continue;
        }
        let bytes = content.as_bytes();
        let mut i = indent;
        while i < bytes.len() && bytes[i] == b'-' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            out.push(base + i, base + i + 1, Token::Punct);
            i += 1;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
        }
        if let Some((key_end, colon)) = yaml_key(content, i) {
            out.push(base + i, base + key_end, Token::Key);
            out.push(base + colon, base + colon + 1, Token::Punct);
            i = colon + 1;
        }
        if yaml_value(content, i, base, &mut out) {
            block_indent = Some(indent);
        }
    }
    out.0
}

/// Finds `key:` starting at `start`; returns the end of the key and the colon position.
fn yaml_key(line: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let mut i = start;
    if bytes[i] == b'"' || bytes[i] == b'\'' {
        let quote = bytes[i];
        i += 1;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let end = (i + 1).min(bytes.len());
        return (end < bytes.len() && bytes[end] == b':').then_some((end, end));
    }
    if matches!(bytes[i], b'[' | b'{' | b'#' | b'&' | b'*' | b'|' | b'>') {
        return None;
    }
    while i < bytes.len() {
        if bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
            return Some((i, i));
        }
        if bytes[i] == b'#' && i > start && bytes[i - 1] == b' ' {
            return None;
        }
        i += 1;
    }
    None
}

/// Colours a value; returns true when it opens a block scalar (`|` or `>`).
fn yaml_value(line: &str, start: usize, base: usize, out: &mut Out) -> bool {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    if i >= bytes.len() {
        return false;
    }
    match bytes[i] {
        b'|' | b'>' => {
            out.push(base + i, base + line.len(), Token::Punct);
            return true;
        }
        b'#' => {
            out.push(base + i, base + line.len(), Token::Comment);
            return false;
        }
        _ => {}
    }
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'#' if i == start || bytes[i - 1] == b' ' => {
                out.push(base + i, base + line.len(), Token::Comment);
                return false;
            }
            b'"' | b'\'' => {
                let quote = bytes[i];
                let begin = i;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' && quote == b'"' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                out.push(base + begin, base + i, Token::String);
            }
            b'[' | b']' | b'{' | b'}' | b',' => {
                out.push(base + i, base + i + 1, Token::Punct);
                i += 1;
            }
            b'&' | b'*' | b'!' => {
                let begin = i;
                while i < bytes.len() && !matches!(bytes[i], b' ' | b',' | b']' | b'}') {
                    i += 1;
                }
                out.push(base + begin, base + i, Token::Keyword);
            }
            _ => {
                let begin = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b',' | b']' | b'}')
                    && !(bytes[i] == b'#' && bytes[i - 1] == b' ')
                {
                    i += 1;
                }
                let word = line[begin..i].trim_end();
                let end = begin + word.len();
                if YAML_WORDS.contains(&word) {
                    out.push(base + begin, base + end, Token::Keyword);
                } else if word.parse::<f64>().is_ok() || word.starts_with("0x") {
                    out.push(base + begin, base + end, Token::Number);
                }
            }
        }
    }
    false
}

fn xml(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut out = Out(Vec::new());
    let find =
        |from: usize, pattern: &str| text[from..].find(pattern).map(|p| from + p + pattern.len()).unwrap_or(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if text[i..].starts_with("<!--") {
            let end = find(i + 4, "-->");
            out.push(i, end, Token::Comment);
            i = end;
        } else if text[i..].starts_with("<![CDATA[") {
            let end = find(i + 9, "]]>");
            out.push(i, end, Token::String);
            i = end;
        } else if text[i..].starts_with("<?") || text[i..].starts_with("<!") {
            let end = find(i + 2, ">");
            out.push(i, end, Token::Keyword);
            i = end;
        } else if bytes[i] == b'<' {
            let start = i;
            i += 1;
            if i < bytes.len() && bytes[i] == b'/' {
                i += 1;
            }
            out.push(start, i, Token::Punct);
            let name_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'/' | b'<') {
                i += 1;
            }
            out.push(name_start, i, Token::Tag);
            // attributes until the tag closes
            while i < bytes.len() && bytes[i] != b'>' && bytes[i] != b'<' {
                match bytes[i] {
                    b'"' | b'\'' => {
                        let quote = bytes[i];
                        let begin = i;
                        i += 1;
                        while i < bytes.len() && bytes[i] != quote {
                            i += 1;
                        }
                        i = (i + 1).min(bytes.len());
                        out.push(begin, i, Token::String);
                    }
                    b'=' | b'/' => {
                        out.push(i, i + 1, Token::Punct);
                        i += 1;
                    }
                    b if b.is_ascii_whitespace() => i += 1,
                    _ => {
                        let begin = i;
                        while i < bytes.len()
                            && !bytes[i].is_ascii_whitespace()
                            && !matches!(bytes[i], b'=' | b'>' | b'/' | b'<')
                        {
                            i += 1;
                        }
                        out.push(begin, i, Token::Attribute);
                    }
                }
            }
            if i < bytes.len() && bytes[i] == b'>' {
                out.push(i, i + 1, Token::Punct);
                i += 1;
            }
        } else if bytes[i] == b'&' {
            let end = text[i..].find(';').filter(|&p| p <= 10).map(|p| i + p + 1);
            match end {
                Some(end) => {
                    out.push(i, end, Token::Keyword);
                    i = end;
                }
                None => i += 1,
            }
        } else {
            i += text[i..].find(['<', '&']).unwrap_or(text.len() - i).max(1);
        }
    }
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(language: Language, text: &str) -> Vec<(&str, Token)> {
        highlight(language, text).into_iter().map(|span| (&text[span.start..span.end], span.token)).collect()
    }

    #[test]
    fn json_keys_values() {
        let found = tokens(Language::Json, r#"{"a": "b", "n": -1.5e3, "t": true}"#);
        assert!(found.contains(&("\"a\"", Token::Key)));
        assert!(found.contains(&("\"b\"", Token::String)));
        assert!(found.contains(&("-1.5e3", Token::Number)));
        assert!(found.contains(&("true", Token::Keyword)));
    }

    #[test]
    fn yaml_blocks_and_comments() {
        let text = "# top\nkey: value # note\nlist:\n  - 12\n  - yes\ntext: |\n  line one\n  line two\nnext: 'q'\n";
        let found = tokens(Language::Yaml, text);
        assert!(found.contains(&("# top", Token::Comment)));
        assert!(found.contains(&("key", Token::Key)));
        assert!(found.contains(&("# note", Token::Comment)));
        assert!(found.contains(&("12", Token::Number)));
        assert!(found.contains(&("yes", Token::Keyword)));
        assert!(found.contains(&("  line two", Token::String)));
        assert!(found.contains(&("next", Token::Key)));
        assert!(found.contains(&("'q'", Token::String)));
    }

    #[test]
    fn xml_tags_and_attributes() {
        let found = tokens(Language::Xml, "<?xml version=\"1.0\"?><a href=\"x\">t&amp;<!-- c --></a>");
        assert!(found.contains(&("a", Token::Tag)));
        assert!(found.contains(&("href", Token::Attribute)));
        assert!(found.contains(&("\"x\"", Token::String)));
        assert!(found.contains(&("&amp;", Token::Keyword)));
        assert!(found.contains(&("<!-- c -->", Token::Comment)));
    }

    #[test]
    fn unterminated_input_is_fine() {
        for language in [Language::Json, Language::Yaml, Language::Xml] {
            for text in ["\"abc", "<a b=\"", "<!-- x", "key: \"x", "ä€\"😀"] {
                let _ = highlight(language, text);
            }
        }
    }

    #[test]
    fn spans_clip_to_lines() {
        let spans =
            vec![Span { start: 0, end: 10, token: Token::String }, Span { start: 12, end: 14, token: Token::Key }];
        let clipped: Vec<_> = spans_in(&spans, 5, 13).collect();
        assert_eq!(
            clipped,
            vec![Span { start: 5, end: 10, token: Token::String }, Span { start: 12, end: 13, token: Token::Key }]
        );
    }
}
