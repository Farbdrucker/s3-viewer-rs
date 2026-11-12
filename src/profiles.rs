//! Named connections in `~/.s3-viewer/connections.json`.
//!
//! The file format is the one the Python version wrote, so saved
//! connections carry over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::s3::{ConnectionSettings, SignatureVersion};
use crate::tr;

#[derive(Serialize, Deserialize)]
struct Stored {
    endpoint_url: String,
    access_key: String,
    secret_key: String,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    direct_bucket: Option<String>,
    #[serde(default = "yes")]
    path_style: bool,
    #[serde(default = "yes")]
    verify_ssl: bool,
    #[serde(default)]
    signature_version: Option<String>,
}

fn yes() -> bool {
    true
}

fn store_path(root: &Path) -> PathBuf {
    root.join("connections.json")
}

fn read(root: &Path) -> Result<BTreeMap<String, Value>, String> {
    let path = store_path(root);
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|_| tr!("profile_unreadable"))?;
    let payload: Value = serde_json::from_str(&text).map_err(|_| tr!("profile_unreadable"))?;
    let profiles = payload.get("profiles").and_then(Value::as_object).ok_or_else(|| tr!("profile_unreadable"))?;
    Ok(profiles
        .iter()
        .filter(|(_, value)| value.is_object())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect())
}

fn write(root: &Path, profiles: &BTreeMap<String, Value>) -> Result<(), String> {
    let fail = |error: std::io::Error| error.to_string();
    std::fs::create_dir_all(root).map_err(fail)?;
    restrict(root, 0o700);
    let payload = serde_json::json!({ "version": 1, "profiles": profiles });
    let mut text = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    text.push('\n');
    let temporary = root.join("connections.json.tmp");
    let result = (|| {
        write_private(&temporary, text.as_bytes())?;
        std::fs::rename(&temporary, store_path(root))
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    restrict(&store_path(root), 0o600);
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(data)
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

pub fn list_profile_names(root: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = read(root)?.into_keys().collect();
    names.sort_by_key(|name| name.to_lowercase());
    Ok(names)
}

pub fn load_profile(root: &Path, name: &str) -> Result<ConnectionSettings, String> {
    let profiles = read(root)?;
    let value = profiles.get(name).ok_or_else(|| tr!("profile_missing", name = name))?;
    let stored: Stored = serde_json::from_value(value.clone()).map_err(|_| tr!("profile_incomplete", name = name))?;
    Ok(ConnectionSettings {
        endpoint_url: stored.endpoint_url,
        access_key: stored.access_key,
        secret_key: stored.secret_key,
        region: stored.region.filter(|r| !r.is_empty()).unwrap_or_else(|| "us-east-1".into()),
        direct_bucket: stored.direct_bucket.unwrap_or_default(),
        path_style: stored.path_style,
        verify_ssl: stored.verify_ssl,
        signature_version: stored
            .signature_version
            .as_deref()
            .and_then(SignatureVersion::from_code)
            .unwrap_or_default(),
    })
}

pub fn save_profile(root: &Path, name: &str, settings: &ConnectionSettings) -> Result<String, String> {
    let cleaned = normalize_profile_name(name)?;
    let mut profiles = read(root)?;
    let stored = Stored {
        endpoint_url: settings.endpoint_url.clone(),
        access_key: settings.access_key.clone(),
        secret_key: settings.secret_key.clone(),
        region: Some(settings.region.clone()),
        direct_bucket: Some(settings.direct_bucket.clone()),
        path_style: settings.path_style,
        verify_ssl: settings.verify_ssl,
        signature_version: Some(settings.signature_version.code().to_owned()),
    };
    profiles.insert(cleaned.clone(), serde_json::to_value(stored).map_err(|error| error.to_string())?);
    write(root, &profiles)?;
    Ok(cleaned)
}

pub fn delete_profile(root: &Path, name: &str) -> Result<(), String> {
    let mut profiles = read(root)?;
    if profiles.remove(name).is_none() {
        return Err(tr!("profile_missing", name = name));
    }
    write(root, &profiles)
}

pub fn normalize_profile_name(name: &str) -> Result<String, String> {
    let cleaned = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return Err(tr!("profile_needs_name"));
    }
    if cleaned.contains('/') || cleaned.contains('\\') || cleaned.chars().count() > 80 {
        return Err(tr!("profile_bad_name"));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ConnectionSettings {
        ConnectionSettings {
            endpoint_url: "https://s3.example.com".into(),
            access_key: "AK".into(),
            secret_key: "SK".into(),
            region: "RegionOne".into(),
            direct_bucket: "b".into(),
            path_style: true,
            verify_ssl: false,
            signature_version: SignatureVersion::V2,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("s3viewer-profiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trip_and_delete() {
        let root = scratch("roundtrip");
        assert!(list_profile_names(&root).unwrap().is_empty());
        let name = save_profile(&root, "  Prod   cloud ", &settings()).unwrap();
        assert_eq!(name, "Prod cloud");
        save_profile(&root, "alpha", &settings()).unwrap();
        assert_eq!(list_profile_names(&root).unwrap(), vec!["alpha", "Prod cloud"]);
        assert_eq!(load_profile(&root, "Prod cloud").unwrap(), settings());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(root.join("connections.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        delete_profile(&root, "alpha").unwrap();
        assert!(delete_profile(&root, "alpha").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_python_files() {
        let root = scratch("python");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("connections.json"),
            r#"{"version": 1, "profiles": {"old": {"endpoint_url": "https://e", "access_key": "a", "secret_key": "s", "region": "", "signature_version": "s3"}}}"#,
        )
        .unwrap();
        let loaded = load_profile(&root, "old").unwrap();
        assert_eq!(loaded.region, "us-east-1");
        assert_eq!(loaded.signature_version, SignatureVersion::V2);
        assert!(loaded.path_style && loaded.verify_ssl);
        std::fs::write(root.join("connections.json"), "not json").unwrap();
        assert!(list_profile_names(&root).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn validates_names() {
        assert!(normalize_profile_name("  ").is_err());
        assert!(normalize_profile_name("a/b").is_err());
        assert!(normalize_profile_name(&"x".repeat(81)).is_err());
    }
}
