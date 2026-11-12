//! Remembered form values and view options in `settings.json` in the
//! configuration folder. The secret key is never written here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The configuration folder, shared with the Python version:
/// `~/.config/s3-viewer` (or `$XDG_CONFIG_HOME/s3-viewer`) on Linux,
/// `~/Library/Application Support/s3-viewer` on macOS, `%APPDATA%\s3-viewer` on
/// Windows. `$S3VIEWER_HOME` overrides it.
pub fn config_root() -> PathBuf {
    if let Some(home) = std::env::var_os("S3VIEWER_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("s3-viewer"))
        .unwrap_or_else(|| PathBuf::from(".s3-viewer"))
}

/// The configuration folder as shown in tooltips, with `~` for the home folder.
pub fn display_path(path: &Path) -> String {
    if cfg!(windows)
        && let Some(appdata) = std::env::var_os("APPDATA")
        && let Ok(rest) = path.strip_prefix(appdata)
    {
        return format!("%AppData%\\{}", rest.display());
    }
    match directories::BaseDirs::new().and_then(|dirs| path.strip_prefix(dirs.home_dir()).ok().map(Path::to_path_buf)) {
        Some(rest) if !cfg!(windows) => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// Copies connections, settings and styles from the old `~/.s3-viewer` folder
/// into `root` once, for files that do not exist there yet. The old folder stays.
pub fn import_legacy(root: &Path) {
    let Some(old) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".s3-viewer")) else { return };
    import_from(&old, root);
}

fn import_from(old: &Path, root: &Path) {
    if !old.is_dir() || old == root {
        return;
    }
    let copy = |from: PathBuf, to: PathBuf| {
        if from.is_file() && !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(&from, &to);
        }
    };
    copy(old.join("connections.json"), root.join("connections.json"));
    crate::profiles::restrict_private(root);
    copy(old.join("settings.json"), root.join("settings.json"));
    if let Ok(entries) = std::fs::read_dir(old.join("themes")) {
        for entry in entries.flatten() {
            copy(entry.path(), root.join("themes").join(entry.file_name()));
        }
    }
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
        legacy_qt_settings(root).unwrap_or_default()
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

/// Settings of the PySide version: `settings.ini` in the shared folder, or its
/// older `~/.config/s3viewer/S3Viewer.conf`.
fn legacy_qt_settings(root: &Path) -> Option<Settings> {
    let old = directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("s3viewer").join("S3Viewer.conf"));
    let text =
        std::iter::once(root.join("settings.ini")).chain(old).find_map(|path| std::fs::read_to_string(path).ok())?;
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

    #[cfg(unix)]
    #[test]
    fn shows_home_as_tilde() {
        let home = directories::BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(display_path(&home.join(".config").join("s3-viewer")), "~/.config/s3-viewer");
        assert_eq!(display_path(Path::new("/srv/x")), "/srv/x");
    }

    #[test]
    fn imports_old_folder_without_overwriting() {
        let base = std::env::temp_dir().join(format!("s3viewer-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (old, root) = (base.join("old"), base.join("new"));
        std::fs::create_dir_all(old.join("themes")).unwrap();
        std::fs::write(old.join("connections.json"), "old").unwrap();
        std::fs::write(old.join("settings.json"), "old").unwrap();
        std::fs::write(old.join("themes").join("mine.toml"), "old").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("settings.json"), "new").unwrap();
        import_from(&old, &root);
        assert_eq!(std::fs::read_to_string(root.join("connections.json")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(root.join("settings.json")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(root.join("themes").join("mine.toml")).unwrap(), "old");
        assert!(old.join("connections.json").exists(), "the old folder stays");
        let _ = std::fs::remove_dir_all(base);
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
