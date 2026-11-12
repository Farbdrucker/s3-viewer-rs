//! Remembered form values and view options in `~/.s3-viewer/settings.json`.
//! The secret key is never written here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `~/.s3-viewer`, or `$S3VIEWER_HOME` when set.
pub fn config_root() -> PathBuf {
    if let Some(home) = std::env::var_os("S3VIEWER_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".s3-viewer"))
        .unwrap_or_else(|| PathBuf::from(".s3-viewer"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PreviewSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub endpoint: String,
    pub access_key: String,
    pub region: String,
    pub bucket: String,
    pub signature: String,
    pub path_style: bool,
    pub verify_ssl: bool,
    pub connection_collapsed: bool,
    pub language: String,
    pub theme: String,
    pub preview_side: PreviewSide,
    pub preview_detached: bool,
    /// Longest side of GeoTIFF previews in pixels.
    pub geotiff_edge: u32,
    /// Most tile data fetched for one GeoTIFF preview, in MiB.
    pub geotiff_budget_mb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            endpoint: String::new(),
            access_key: String::new(),
            region: "us-east-1".into(),
            bucket: String::new(),
            signature: "s3v4".into(),
            path_style: true,
            verify_ssl: true,
            connection_collapsed: false,
            language: "de".into(),
            theme: "light".into(),
            preview_side: PreviewSide::Right,
            preview_detached: false,
            geotiff_edge: 1024,
            geotiff_budget_mb: 64,
        }
    }
}

impl Settings {
    pub fn path(root: &Path) -> PathBuf {
        root.join("settings.json")
    }

    /// Reads the settings, falling back to the Python version's file once.
    pub fn load(root: &Path) -> Settings {
        if let Ok(text) = std::fs::read_to_string(Self::path(root)) {
            return serde_json::from_str(&text).unwrap_or_default();
        }
        legacy_qt_settings().unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let path = Self::path(root);
        let temporary = root.join("settings.json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(temporary, path)
    }
}

/// `~/.config/s3viewer/S3Viewer.conf`, written by the PySide version.
fn legacy_qt_settings() -> Option<Settings> {
    let path = directories::BaseDirs::new()?.config_dir().join("s3viewer").join("S3Viewer.conf");
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_qt_ini(&text))
}

fn parse_qt_ini(text: &str) -> Settings {
    let mut settings = Settings::default();
    let flag = |value: &str| matches!(value.trim().to_lowercase().as_str(), "1" | "true" | "yes");
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "endpoint" => settings.endpoint = value,
            "access_key" => settings.access_key = value,
            "region" if !value.is_empty() => settings.region = value,
            "bucket" => settings.bucket = value,
            "signature" if value == "s3" || value == "s3v4" => settings.signature = value,
            "path_style" => settings.path_style = flag(&value),
            "verify_ssl" => settings.verify_ssl = flag(&value),
            "connection_collapsed" => settings.connection_collapsed = flag(&value),
            "language" => settings.language = if value == "en" { "en".into() } else { "de".into() },
            "dark_mode" => settings.theme = if flag(&value) { "dark".into() } else { "light".into() },
            _ => {}
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let root = std::env::temp_dir().join(format!("s3viewer-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let settings = Settings { endpoint: "https://e".into(), theme: "nord".into(), ..Settings::default() };
        settings.save(&root).unwrap();
        assert_eq!(Settings::load(&root), settings);
        std::fs::write(Settings::path(&root), r#"{"endpoint": "x"}"#).unwrap();
        let partial = Settings::load(&root);
        assert_eq!(partial.endpoint, "x");
        assert_eq!(partial.geotiff_edge, 1024);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_python_ini() {
        let settings = parse_qt_ini(
            "[General]\nendpoint=https://s3.example.com\naccess_key=AK\nregion=RegionOne\nsignature=s3\npath_style=0\nverify_ssl=1\nlanguage=en\ndark_mode=1\n",
        );
        assert_eq!(settings.endpoint, "https://s3.example.com");
        assert_eq!(settings.region, "RegionOne");
        assert_eq!(settings.signature, "s3");
        assert!(!settings.path_style && settings.verify_ssl);
        assert_eq!(settings.language, "en");
        assert_eq!(settings.theme, "dark");
    }
}
