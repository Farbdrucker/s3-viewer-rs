//! Detects file formats and prepares a readable preview.

pub mod highlight;

use highlight::{Language, Span};

use crate::tr;

pub const PREVIEW_BYTE_LIMIT: u64 = 1_000_000;
pub const IMAGE_BYTE_LIMIT: u64 = 32 * 1024 * 1024;
const CSV_ROW_LIMIT: usize = 2000;
const CSV_COLUMN_LIMIT: usize = 40;
const IMAGE_PREVIEW_EDGE: u32 = 2048;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "text",
    "log",
    "md",
    "rst",
    "markdown",
    "csv",
    "tsv",
    "json",
    "geojson",
    "ndjson",
    "xml",
    "html",
    "htm",
    "xhtml",
    "css",
    "js",
    "mjs",
    "ts",
    "tsx",
    "jsx",
    "py",
    "rb",
    "go",
    "rs",
    "java",
    "kt",
    "c",
    "h",
    "cpp",
    "hpp",
    "cs",
    "php",
    "sql",
    "sh",
    "bash",
    "zsh",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "config",
    "properties",
    "env",
    "svg",
    "tex",
    "r",
    "dockerfile",
    "prj",
    "aux",
    "vrt",
    "qmd",
    "wld",
    "tfw",
    "jgw",
    "pgw",
];

const BINARY_EXTENSIONS: &[&str] = &[
    "ico", "heic", "zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar", "pdf", "parquet", "orc", "avro", "feather",
    "pkl", "pickle", "npy", "npz", "exe", "so", "dylib", "bin", "dat", "mp3", "mp4", "mov", "avi", "mkv", "webm",
    "wav", "flac", "ogg", "iso", "img", "qcow2", "sqlite", "db", "ttf", "otf", "woff", "woff2", "eot", "class", "jar",
    "war", "ear", "xls", "xlsx", "doc", "docx", "ppt", "pptx", "ods", "odt", "odp", "jp2", "nc", "h5", "hdf", "grib",
    "grb2", "zarr", "laz", "las", "gpkg", "shp", "shx", "dbf",
];

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

const TEXT_CONTENT_TYPES: &[&str] = &[
    "application/json",
    "application/geo+json",
    "application/xml",
    "application/yaml",
    "application/x-yaml",
    "application/toml",
    "application/javascript",
    "application/sql",
    "application/xhtml+xml",
    "application/csv",
    "application/x-ndjson",
    "image/svg+xml",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Json,
    Ndjson,
    Yaml,
    Html,
    Svg,
    Xml,
    Csv,
    Tsv,
    Tiff,
    Image,
    Binary,
    Empty,
}

impl Kind {
    pub fn badge(self) -> String {
        match self {
            Kind::Text => "Text".into(),
            Kind::Json => "JSON".into(),
            Kind::Ndjson => "NDJSON".into(),
            Kind::Yaml => "YAML".into(),
            Kind::Html => "HTML".into(),
            Kind::Svg => "SVG".into(),
            Kind::Xml => "XML".into(),
            Kind::Csv => "CSV".into(),
            Kind::Tsv => "TSV".into(),
            Kind::Tiff => "TIFF".into(),
            Kind::Image => tr!("badge_image"),
            Kind::Binary => tr!("badge_binary"),
            Kind::Empty => tr!("badge_empty"),
        }
    }

    pub fn editable(self) -> bool {
        matches!(self, Kind::Text | Kind::Json | Kind::Ndjson | Kind::Xml | Kind::Html | Kind::Svg | Kind::Yaml)
    }

    pub fn language(self) -> Language {
        match self {
            Kind::Json | Kind::Ndjson => Language::Json,
            Kind::Yaml => Language::Yaml,
            Kind::Xml | Kind::Html | Kind::Svg => Language::Xml,
            _ => Language::None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_numbers: Vec<usize>,
}

/// A decoded raster image ready for display.
#[derive(Clone)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}×{})", self.width, self.height)
    }
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub kind: Kind,
    pub filename: String,
    pub text: String,
    /// Byte ranges of the lines in `text`, without line breaks.
    pub lines: Vec<(usize, usize)>,
    pub spans: Vec<Span>,
    pub table: Option<Table>,
    pub image: Option<RgbaImage>,
    pub notice: String,
    pub truncated: bool,
    pub size: u64,
    pub content_type: String,
}

impl Preview {
    fn new(kind: Kind, filename: &str, size: u64, content_type: &str, truncated: bool) -> Self {
        Preview {
            kind,
            filename: filename.to_owned(),
            text: String::new(),
            lines: Vec::new(),
            spans: Vec::new(),
            table: None,
            image: None,
            notice: String::new(),
            truncated,
            size,
            content_type: content_type_base(content_type),
        }
    }

    pub fn editable(&self) -> bool {
        self.kind.editable() && !self.truncated
    }

    /// Fills `text` and the derived line index and highlighting.
    pub fn set_text(&mut self, text: String) {
        self.lines = line_ranges(&text);
        self.spans = highlight::highlight(self.kind.language(), &text);
        self.text = text;
    }
}

pub fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            let end = if index > start && text.as_bytes()[index - 1] == b'\r' { index - 1 } else { index };
            lines.push((start, end));
            start = index + 1;
        }
    }
    if start < text.len() || lines.is_empty() {
        lines.push((start, text.len()));
    }
    lines
}

pub fn filename_of(key: &str) -> &str {
    match key.rsplit('/').next() {
        Some(name) if !name.is_empty() => name,
        _ => key,
    }
}

pub fn extension_of(key: &str) -> String {
    let name = key.rsplit('/').next().unwrap_or(key);
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_lowercase(),
        None => String::new(),
    }
}

pub fn content_type_base(content_type: &str) -> String {
    content_type.split(';').next().unwrap_or("").trim().to_lowercase()
}

pub fn is_tiff(key: &str, content_type: &str) -> bool {
    matches!(extension_of(key).as_str(), "tif" | "tiff") || content_type_base(content_type) == "image/tiff"
}

pub fn is_image(key: &str, content_type: &str) -> bool {
    let ctype = content_type_base(content_type);
    IMAGE_EXTENSIONS.contains(&extension_of(key).as_str())
        || matches!(ctype.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp")
}

/// Decodes UTF-16 (with BOM), UTF-8 or Latin-1. `truncated` allows dropping
/// a multi-byte character that the preview limit cut in half.
pub fn decode_text(data: &[u8], truncated: bool) -> String {
    if let Some(rest) = data.strip_prefix(b"\xff\xfe") {
        let units: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = data.strip_prefix(b"\xfe\xff") {
        let units: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    match std::str::from_utf8(data) {
        Ok(text) => text.to_owned(),
        Err(error) if truncated && error.error_len().is_none() => {
            String::from_utf8_lossy(&data[..error.valid_up_to()]).into_owned()
        }
        Err(_) => data.iter().map(|&byte| byte as char).collect(),
    }
}

fn has_binary_null(data: &[u8]) -> bool {
    if data.starts_with(b"\xff\xfe") || data.starts_with(b"\xfe\xff") {
        return false;
    }
    data[..data.len().min(8192)].contains(&0)
}

pub fn looks_textual(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    let sample = &data[..data.len().min(8192)];
    if sample.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(error) if error.error_len().is_none() => true,
        Err(_) => {
            let printable =
                sample.iter().filter(|&&b| matches!(b, b'\n' | b'\r' | b'\t' | 0x20..=0x7e | 0xa0..=0xff)).count();
            printable as f64 / sample.len() as f64 > 0.90
        }
    }
}

pub fn content_type_for_save(kind: Kind, existing: &str) -> String {
    let mut base = content_type_base(existing);
    if base.is_empty() || base == "application/octet-stream" || base == "binary/octet-stream" {
        base = match kind {
            Kind::Json => "application/json",
            Kind::Ndjson => "application/x-ndjson",
            Kind::Xml => "application/xml",
            Kind::Html => "text/html",
            Kind::Svg => "image/svg+xml",
            Kind::Yaml => "application/yaml",
            _ => "text/plain",
        }
        .to_owned();
    }
    if existing.to_lowercase().contains("charset=") {
        return existing.to_owned();
    }
    format!("{base}; charset=utf-8")
}

pub fn detect_kind(key: &str, content_type: &str, data: &[u8]) -> Kind {
    let ext = extension_of(key);
    let ext = ext.as_str();
    let ctype = content_type_base(content_type);
    let ctype = ctype.as_str();
    let text_type = ctype.starts_with("text/") || TEXT_CONTENT_TYPES.contains(&ctype);
    if is_tiff(key, content_type) {
        return Kind::Tiff;
    }
    if is_image(key, content_type) && !text_type {
        return Kind::Image;
    }
    if BINARY_EXTENSIONS.contains(&ext) && !text_type {
        return Kind::Binary;
    }
    if matches!(ext, "json" | "geojson") || matches!(ctype, "application/json" | "application/geo+json") {
        return Kind::Json;
    }
    if ext == "ndjson" || matches!(ctype, "application/x-ndjson" | "application/ndjson") {
        return Kind::Ndjson;
    }
    if matches!(ext, "yaml" | "yml")
        || matches!(ctype, "application/yaml" | "application/x-yaml" | "text/yaml" | "text/x-yaml")
    {
        return Kind::Yaml;
    }
    if matches!(ext, "html" | "htm" | "xhtml") || matches!(ctype, "text/html" | "application/xhtml+xml") {
        return Kind::Html;
    }
    if ext == "svg" || ctype == "image/svg+xml" {
        return Kind::Svg;
    }
    if matches!(ext, "xml" | "vrt" | "qmd" | "aux") || matches!(ctype, "application/xml" | "text/xml") {
        return Kind::Xml;
    }
    if ext == "csv" || matches!(ctype, "text/csv" | "application/csv") {
        return Kind::Csv;
    }
    if ext == "tsv" || ctype == "text/tab-separated-values" {
        return Kind::Tsv;
    }
    let textual = TEXT_EXTENSIONS.contains(&ext) || text_type;
    if textual || looks_textual(data) {
        if has_binary_null(data) && !TEXT_EXTENSIONS.contains(&ext) && !ctype.starts_with("text/") {
            return Kind::Binary;
        }
        return Kind::Text;
    }
    Kind::Binary
}

/// Builds the preview for text-like and binary files. TIFFs and images are
/// rendered elsewhere, see [`crate::geotiff`] and [`build_image_preview`].
pub fn build_preview(key: &str, data: &[u8], content_type: &str, size: Option<u64>, truncated: bool) -> Preview {
    let filename = filename_of(key);
    let size = size.unwrap_or(data.len() as u64);
    let kind = detect_kind(key, content_type, data);
    let mut preview = Preview::new(kind, filename, size, content_type, truncated);
    if matches!(kind, Kind::Binary | Kind::Tiff | Kind::Image) {
        preview.kind = Kind::Binary;
        preview.notice = tr!("no_text_preview");
        return preview;
    }
    if data.is_empty() {
        preview.kind = Kind::Empty;
        preview.notice = tr!("empty_file");
        return preview;
    }
    let mut text = decode_text(data, truncated);
    let mut notices = Vec::new();
    if truncated {
        if let Some(index) = text.rfind('\n') {
            text.truncate(index + 1);
        }
        notices.push(tr!("preview_truncated"));
    }
    match kind {
        Kind::Json => match pretty_json(&text) {
            Some(pretty) => text = pretty,
            None => notices.push(tr!("json_raw")),
        },
        Kind::Xml => match crate::xml::pretty_print(&text) {
            Ok(pretty) => text = pretty,
            Err(_) => notices.push(tr!("xml_raw")),
        },
        Kind::Csv | Kind::Tsv => {
            let delimiter = if kind == Kind::Tsv { Some(b'\t') } else { None };
            match parse_table(&text, delimiter) {
                TableResult::Table(table, limited) => {
                    if limited {
                        notices.push(tr!("table_limited"));
                    }
                    preview.table = Some(table);
                }
                TableResult::Empty => {}
                TableResult::TooWide => {
                    preview.kind = Kind::Text;
                    notices.push(tr!("table_too_wide"));
                }
            }
        }
        _ => {}
    }
    preview.notice = notices.join(" ");
    preview.set_text(text);
    preview
}

pub fn build_image_preview(key: &str, data: &[u8], content_type: &str, size: u64, truncated: bool) -> Preview {
    let mut preview = Preview::new(Kind::Image, filename_of(key), size, content_type, truncated);
    if data.is_empty() {
        preview.kind = Kind::Empty;
        preview.notice = tr!("empty_file");
        return preview;
    }
    if truncated {
        preview.notice = tr!("image_too_big", limit = IMAGE_BYTE_LIMIT / (1024 * 1024));
        return preview;
    }
    match image::load_from_memory(data) {
        Ok(decoded) => {
            let (width, height) = (decoded.width(), decoded.height());
            let shown = if width.max(height) > IMAGE_PREVIEW_EDGE {
                decoded.thumbnail(IMAGE_PREVIEW_EDGE, IMAGE_PREVIEW_EDGE)
            } else {
                decoded
            };
            let rgba = shown.to_rgba8();
            preview.image = Some(RgbaImage {
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                pixels: rgba.into_raw(),
            });
            preview.notice = tr!("image_info", width = width, height = height);
        }
        Err(_) => preview.notice = tr!("image_failed"),
    }
    preview
}

fn pretty_json(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut out = serde_json::to_string_pretty(&value).ok()?;
    out.push('\n');
    Some(out)
}

enum TableResult {
    Table(Table, bool),
    Empty,
    TooWide,
}

/// Picks the delimiter that splits the first lines most consistently.
fn sniff_delimiter(sample: &str) -> u8 {
    let lines: Vec<&str> = sample.lines().filter(|line| !line.trim().is_empty()).take(20).collect();
    let mut best = (b',', 0.0f64);
    for candidate in *b",;\t|" {
        let counts: Vec<usize> = lines.iter().map(|line| count_outside_quotes(line, candidate)).collect();
        let Some(&first) = counts.first() else { continue };
        if first == 0 {
            continue;
        }
        let consistent = counts.iter().filter(|&&count| count == first).count() as f64 / counts.len() as f64;
        let score = consistent * 1000.0 + first as f64;
        if score > best.1 {
            best = (candidate, score);
        }
    }
    best.0
}

fn count_outside_quotes(line: &str, delimiter: u8) -> usize {
    let mut quoted = false;
    line.bytes()
        .filter(|&byte| {
            if byte == b'"' {
                quoted = !quoted;
            }
            !quoted && byte == delimiter
        })
        .count()
}

fn parse_table(text: &str, delimiter: Option<u8>) -> TableResult {
    let delimiter = delimiter.unwrap_or_else(|| {
        let mut end = text.len().min(8192);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        sniff_delimiter(&text[..end])
    });
    let mut reader =
        csv::ReaderBuilder::new().has_headers(false).flexible(true).delimiter(delimiter).from_reader(text.as_bytes());
    let mut numbered: Vec<(usize, Vec<String>)> = Vec::new();
    let mut limited = false;
    for (index, record) in reader.records().enumerate() {
        if numbered.len() >= CSV_ROW_LIMIT {
            limited = true;
            break;
        }
        let Ok(record) = record else { continue };
        let row: Vec<String> = record.iter().map(str::to_owned).collect();
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        numbered.push((index + 1, row));
    }
    if numbered.is_empty() {
        return TableResult::Empty;
    }
    let mut width = numbered.iter().map(|(_, row)| row.len()).max().unwrap_or(0);
    if width > CSV_COLUMN_LIMIT {
        return TableResult::TooWide;
    }
    let (mut header, body) = if looks_like_header(&numbered[0].1) {
        let header = numbered[0].1.clone();
        (header, numbered.split_off(1))
    } else {
        ((1..=width).map(|index| tr!("column", index = index)).collect(), numbered)
    };
    width = width.max(header.len());
    header.resize(width, String::new());
    let row_numbers = body.iter().map(|(number, _)| *number).collect();
    let rows = body
        .into_iter()
        .map(|(_, mut row)| {
            row.resize(width, String::new());
            row
        })
        .collect();
    TableResult::Table(Table { headers: header, rows, row_numbers }, limited)
}

fn looks_like_header(row: &[String]) -> bool {
    if row.is_empty() {
        return false;
    }
    let numeric = row.iter().filter(|cell| cell.trim().replace(',', ".").parse::<f64>().is_ok()).count();
    (numeric as f64) < row.len() as f64 / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kinds() {
        assert_eq!(detect_kind("a/b.json", "", b"{}"), Kind::Json);
        assert_eq!(detect_kind("x", "application/json; charset=utf-8", b"{}"), Kind::Json);
        assert_eq!(detect_kind("a.TIF", "", b""), Kind::Tiff);
        assert_eq!(detect_kind("a.png", "", b""), Kind::Image);
        assert_eq!(detect_kind("a.zip", "", b"PK"), Kind::Binary);
        assert_eq!(detect_kind("README", "", b"hello\nworld"), Kind::Text);
        assert_eq!(detect_kind("blob", "", b"\x00\x01\x02"), Kind::Binary);
        assert_eq!(detect_kind("x.yml", "", b"a: 1"), Kind::Yaml);
        assert_eq!(detect_kind("x.svg", "", b"<svg/>"), Kind::Svg);
        assert_eq!(detect_kind("x.tsv", "", b"a\tb"), Kind::Tsv);
    }

    #[test]
    fn pretty_json_keeps_order_and_numbers() {
        let preview = build_preview("a.json", r#"{"z":1.50,"a":[1,2],"u":"ä"}"#.as_bytes(), "", None, false);
        assert_eq!(preview.text, "{\n  \"z\": 1.50,\n  \"a\": [\n    1,\n    2\n  ],\n  \"u\": \"ä\"\n}\n");
        assert!(preview.notice.is_empty());
        assert!(!preview.spans.is_empty());
    }

    #[test]
    fn broken_json_shown_raw() {
        let preview = build_preview("a.json", b"{oops", "", None, false);
        assert_eq!(preview.text, "{oops");
        assert!(!preview.notice.is_empty());
    }

    #[test]
    fn truncated_text_drops_partial_line() {
        let preview = build_preview("a.txt", b"one\ntwo\nthr", "", Some(100), true);
        assert_eq!(preview.text, "one\ntwo\n");
        assert!(preview.truncated);
        assert!(!preview.editable());
    }

    #[test]
    fn csv_with_header_and_sniffing() {
        let preview = build_preview("a.csv", b"name;value\nx;1\n\ny;2\n", "", None, false);
        let table = preview.table.unwrap();
        assert_eq!(table.headers, vec!["name", "value"]);
        assert_eq!(table.rows, vec![vec!["x", "1"], vec!["y", "2"]]);
        assert_eq!(table.row_numbers, vec![2, 3]);
    }

    #[test]
    fn csv_without_header() {
        let _guard = crate::i18n::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language(crate::i18n::Language::En);
        let preview = build_preview("a.csv", b"1,2\n3,4,5\n", "", None, false);
        let table = preview.table.unwrap();
        assert_eq!(table.headers, vec!["Column 1", "Column 2", "Column 3"]);
        assert_eq!(table.rows[0], vec!["1", "2", ""]);
        crate::i18n::set_language(crate::i18n::Language::De);
    }

    #[test]
    fn wide_csv_falls_back_to_text() {
        let line = vec!["a"; 50].join(",");
        let preview = build_preview("a.csv", line.as_bytes(), "", None, false);
        assert_eq!(preview.kind, Kind::Text);
        assert!(preview.table.is_none());
    }

    #[test]
    fn decodes_utf16_and_latin1() {
        assert_eq!(decode_text(b"\xff\xfeh\x00i\x00", false), "hi");
        assert_eq!(decode_text(b"caf\xe9", false), "café");
        assert_eq!(decode_text("ä".as_bytes(), false), "ä");
        assert_eq!(decode_text(&"aä".as_bytes()[..2], true), "a");
    }

    #[test]
    fn save_content_types() {
        assert_eq!(content_type_for_save(Kind::Json, ""), "application/json; charset=utf-8");
        assert_eq!(content_type_for_save(Kind::Text, "text/plain; charset=latin-1"), "text/plain; charset=latin-1");
        assert_eq!(content_type_for_save(Kind::Yaml, "binary/octet-stream"), "application/yaml; charset=utf-8");
    }

    #[test]
    fn lines_index() {
        assert_eq!(line_ranges("a\r\nbc\n"), vec![(0, 1), (3, 5)]);
        assert_eq!(line_ranges(""), vec![(0, 0)]);
        assert_eq!(line_ranges("x"), vec![(0, 1)]);
    }

    #[test]
    fn decodes_png() {
        let mut png = Vec::new();
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let preview = build_image_preview("a.png", &png, "image/png", png.len() as u64, false);
        let decoded = preview.image.unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(&decoded.pixels[..4], &[1, 2, 3, 255]);
    }
}
