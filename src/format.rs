//! Sizes and timestamps as shown in the interface.

use chrono::{DateTime, Local, Utc};

use crate::i18n::{Language, language};

pub fn format_size(size: Option<u64>) -> String {
    let Some(size) = size else {
        return "—".to_owned();
    };
    let mut value = size as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if value < 1024.0 || unit == "TB" {
            if unit == "B" {
                return format!("{} B", value as u64);
            }
            let text = format!("{value:.1} {unit}");
            return match language() {
                Language::De => text.replace('.', ","),
                Language::En => text,
            };
        }
        value /= 1024.0;
    }
    format!("{size} B")
}

pub fn format_dt(value: Option<DateTime<Utc>>) -> String {
    match value {
        Some(value) => value.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string(),
        None => String::new(),
    }
}

/// Parses the ISO 8601 timestamps S3 puts into listings.
pub fn parse_iso8601(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim()).ok().map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{TEST_LOCK, set_language};

    #[test]
    fn sizes() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language(Language::De);
        assert_eq!(format_size(None), "—");
        assert_eq!(format_size(Some(512)), "512 B");
        assert_eq!(format_size(Some(1536)), "1,5 KB");
        assert_eq!(format_size(Some(5 * 1024 * 1024 * 1024)), "5,0 GB");
        set_language(Language::En);
        assert_eq!(format_size(Some(1536)), "1.5 KB");
        set_language(Language::De);
    }

    #[test]
    fn parses_listing_dates() {
        let value = parse_iso8601("2024-03-05T10:20:30.000Z").unwrap();
        assert_eq!(value.timestamp(), 1_709_634_030);
        assert!(parse_iso8601("garbage").is_none());
    }
}
