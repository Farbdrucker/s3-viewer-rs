//! Target paths for downloads and target keys for uploads.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::tr;

// ------------------------------------------------------------------ downloads

/// The part of `key` below `prefix`, without leading slashes.
pub fn relative_name(prefix: &str, key: &str) -> String {
    let name = if !prefix.is_empty() { key.strip_prefix(prefix).unwrap_or(key) } else { key };
    name.trim_start_matches('/').to_owned()
}

/// Joins an object path below `root` and refuses anything that would leave it.
pub fn destination_for(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut parts = Vec::new();
    let normalized = relative.replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(tr!("bad_filename", name = relative)),
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(tr!("empty_filename"));
    }
    let mut target = root.to_path_buf();
    for part in parts {
        // Drive letters or roots inside a key must not escape the folder on Windows.
        let component_ok = Path::new(part).components().all(|component| matches!(component, Component::Normal(_)));
        if !component_ok || part.contains(':') && cfg!(windows) {
            return Err(tr!("bad_filename", name = relative));
        }
        target.push(part);
    }
    Ok(target)
}

/// `name.txt`, then `name (2).txt`, `name (3).txt`, … until the path is free.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|number| path.with_file_name(format!("{stem} ({number}){suffix}")))
        .find(|candidate| !candidate.exists())
        .expect("an unused name exists")
}

pub fn saved_message(count: usize, directory: &str, capped: bool, limit: usize) -> String {
    let mut text = if count == 1 {
        tr!("saved_one", directory = directory)
    } else {
        tr!("saved_many", count = count, directory = directory)
    };
    if capped {
        text += &tr!("saved_capped", limit = limit);
    }
    text
}

// -------------------------------------------------------------------- uploads

fn folder_key(prefix: &str) -> String {
    if prefix.is_empty() || prefix.ends_with('/') { prefix.to_owned() } else { format!("{prefix}/") }
}

/// Key of the placeholder object for a new folder `name` below `prefix`.
pub fn folder_marker_key(prefix: &str, name: &str) -> Result<String, String> {
    let cleaned = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || cleaned.contains('/') || cleaned.contains('\\') {
        return Err(tr!("new_folder_invalid"));
    }
    Ok(format!("{}{cleaned}/", folder_key(prefix)))
}

pub fn upload_key(prefix: &str, local_path: &Path) -> Result<String, String> {
    let name = local_path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(tr!("upload_bad_path", path = local_path.display()));
    }
    Ok(format!("{}{name}", folder_key(prefix)))
}

pub type UploadPlan = Vec<(PathBuf, String)>;

pub fn upload_plan(prefix: &str, local_paths: &[PathBuf]) -> Result<UploadPlan, String> {
    let mut plan = Vec::new();
    let mut seen = HashSet::new();
    for path in local_paths {
        let key = upload_key(prefix, path)?;
        let name = key.rsplit('/').next().unwrap_or(&key).to_owned();
        if !seen.insert(name.clone()) {
            return Err(tr!("upload_duplicate", name = name));
        }
        plan.push((path.clone(), key));
    }
    if plan.is_empty() {
        return Err(tr!("upload_none"));
    }
    Ok(plan)
}

pub fn folder_upload_plan(prefix: &str, directory: &Path) -> Result<UploadPlan, String> {
    if !directory.is_dir() {
        return Err(tr!("not_a_folder"));
    }
    let folder_name = directory.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    if folder_name.is_empty() || folder_name == "." || folder_name == ".." {
        return Err(tr!("bad_folder"));
    }
    let base = format!("{}{folder_name}/", folder_key(prefix));
    let mut files = Vec::new();
    collect_files(directory, &mut files);
    files.sort();
    let plan: UploadPlan = files
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(directory).ok()?;
            let parts: Vec<String> = relative
                .components()
                .map(|component| match component {
                    Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if parts.is_empty() {
                return None;
            }
            let key = format!("{base}{}", parts.join("/"));
            Some((path, key))
        })
        .collect();
    if plan.is_empty() {
        return Err(tr!("folder_empty_local"));
    }
    Ok(plan)
}

fn collect_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            collect_files(&entry.path(), out);
        } else if kind.is_file() || (kind.is_symlink() && entry.path().is_file()) {
            out.push(entry.path());
        }
    }
}

pub fn colliding_keys(plan: &UploadPlan, existing: &HashSet<String>) -> Vec<String> {
    plan.iter().filter(|(_, key)| existing.contains(key)).map(|(_, key)| key.clone()).collect()
}

pub fn upload_destination_label(bucket: &str, prefix: &str) -> String {
    let folder = prefix.trim_matches('/');
    if folder.is_empty() { format!("{bucket}/") } else { format!("{bucket}/{folder}/") }
}

pub fn uploaded_message(count: usize, destination: &str) -> String {
    if count == 1 {
        tr!("uploaded_one", destination = destination)
    } else {
        tr!("uploaded_many", count = count, destination = destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("s3viewer-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_names() {
        assert_eq!(relative_name("a/", "a/b/c.txt"), "b/c.txt");
        assert_eq!(relative_name("", "/x"), "x");
        assert_eq!(relative_name("z/", "a/b"), "a/b");
    }

    #[test]
    fn destinations_stay_inside() {
        let root = Path::new("/tmp/root");
        assert_eq!(destination_for(root, "a/./b.txt").unwrap(), root.join("a").join("b.txt"));
        assert!(destination_for(root, "../etc/passwd").is_err());
        assert!(destination_for(root, "a/../../x").is_err());
        assert!(destination_for(root, "//").is_err());
    }

    #[test]
    fn unique_paths_count_up() {
        let dir = scratch("unique");
        let path = dir.join("f.txt");
        assert_eq!(unique_path(&path), path);
        std::fs::write(&path, b"x").unwrap();
        assert_eq!(unique_path(&path), dir.join("f (2).txt"));
        std::fs::write(dir.join("f (2).txt"), b"x").unwrap();
        assert_eq!(unique_path(&path), dir.join("f (3).txt"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn folder_markers() {
        assert_eq!(folder_marker_key("data", "  new   maps ").unwrap(), "data/new maps/");
        assert_eq!(folder_marker_key("", "x").unwrap(), "x/");
        for bad in ["", "  ", "..", "a/b", "a\\b"] {
            assert!(folder_marker_key("", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn upload_plans() {
        let plan = upload_plan("data", &[PathBuf::from("/x/a.txt"), PathBuf::from("/y/b.txt")]).unwrap();
        assert_eq!(plan[0].1, "data/a.txt");
        assert!(upload_plan("", &[PathBuf::from("/x/a.txt"), PathBuf::from("/y/a.txt")]).is_err());
        assert!(upload_plan("", &[]).is_err());
    }

    #[test]
    fn folder_plans_keep_structure() {
        let dir = scratch("folder");
        let root = dir.join("photos");
        std::fs::create_dir_all(root.join("2024")).unwrap();
        std::fs::write(root.join("a.jpg"), b"1").unwrap();
        std::fs::write(root.join("2024").join("b.jpg"), b"2").unwrap();
        let plan = folder_upload_plan("in/", &root).unwrap();
        let keys: Vec<_> = plan.iter().map(|(_, key)| key.as_str()).collect();
        assert_eq!(keys, vec!["in/photos/2024/b.jpg", "in/photos/a.jpg"]);
        let existing: HashSet<String> = ["in/photos/a.jpg".to_string()].into();
        assert_eq!(colliding_keys(&plan, &existing), vec!["in/photos/a.jpg"]);
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(folder_upload_plan("", &empty).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn labels() {
        assert_eq!(upload_destination_label("b", ""), "b/");
        assert_eq!(upload_destination_label("b", "x/y/"), "b/x/y/");
    }
}
