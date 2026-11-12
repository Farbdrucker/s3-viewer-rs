//! The file name filter: plain text or a regular expression.

use regex::{Regex, RegexBuilder};

/// A compiled filter. Built once when the text or the mode changes.
#[derive(Clone, Debug, Default)]
pub enum NameFilter {
    /// Empty filter: everything matches.
    #[default]
    All,
    /// Case-insensitive substring, stored lower-case.
    Text(String),
    /// Case-insensitive regular expression, matching anywhere in the name
    /// unless anchored with `^` / `$`.
    Pattern(Regex),
    /// The expression does not compile; nothing is filtered out.
    Invalid(String),
}

/// Keeps compiled expressions small, so a pasted monster pattern cannot stall the UI.
const SIZE_LIMIT: usize = 1 << 20;

impl NameFilter {
    pub fn new(query: &str, regex: bool) -> Self {
        if query.trim().is_empty() {
            return NameFilter::All;
        }
        if !regex {
            return NameFilter::Text(query.trim().to_lowercase());
        }
        match RegexBuilder::new(query).case_insensitive(true).size_limit(SIZE_LIMIT).build() {
            Ok(pattern) => NameFilter::Pattern(pattern),
            Err(error) => NameFilter::Invalid(short_error(&error)),
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            NameFilter::All | NameFilter::Invalid(_) => true,
            NameFilter::Text(text) => name.to_lowercase().contains(text),
            NameFilter::Pattern(pattern) => pattern.is_match(name),
        }
    }

    /// Whether the filter narrows the list (used for the "x / y" counter).
    pub fn is_active(&self) -> bool {
        matches!(self, NameFilter::Text(_) | NameFilter::Pattern(_))
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            NameFilter::Invalid(error) => Some(error),
            _ => None,
        }
    }
}

/// The regex crate prints the pattern with a caret line; the last line says what is wrong.
fn short_error(error: &regex::Error) -> String {
    let text = error.to_string();
    let last = text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or(&text);
    last.trim().trim_start_matches("error: ").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_case_insensitive_substring() {
        let filter = NameFilter::new("  DEM ", false);
        assert!(filter.matches("big_dem_no_overviews.tif"));
        assert!(!filter.matches("cog.tif"));
        assert!(filter.is_active());
        // Regex characters have no meaning in text mode.
        assert!(NameFilter::new("a.b", false).matches("a.b.csv"));
        assert!(!NameFilter::new("a.b", false).matches("axb.csv"));
    }

    #[test]
    fn regular_expressions() {
        let tif = NameFilter::new(r"\.tiff?$", true);
        assert!(tif.matches("orthophoto.TIF"));
        assert!(tif.matches("a.tiff"));
        assert!(!tif.matches("a.tif.aux.xml"));
        let tiles = NameFilter::new(r"^tile_\d{3}_\d{3}\.", true);
        assert!(tiles.matches("tile_012_345.png"));
        assert!(!tiles.matches("tile_12_345.png"));
        assert!(NameFilter::new("2024|2025", true).matches("report-2025.csv"));
    }

    #[test]
    fn invalid_expressions_filter_nothing() {
        let filter = NameFilter::new("tile_(", true);
        assert!(filter.error().is_some(), "{filter:?}");
        assert!(!filter.error().unwrap().contains('\n'));
        assert!(filter.matches("anything"));
        assert!(!filter.is_active());
    }

    #[test]
    fn empty_filter_matches_everything() {
        for regex in [false, true] {
            let filter = NameFilter::new("   ", regex);
            assert!(filter.matches("x") && !filter.is_active());
        }
    }
}
