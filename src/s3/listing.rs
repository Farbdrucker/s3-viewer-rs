//! Turns ListObjects answers into one folder level.

use std::collections::{HashSet, hash_map::Entry};

use chrono::{DateTime, Utc};

use crate::format::parse_iso8601;
use crate::xml::Element;

#[derive(Debug, Clone, PartialEq)]
pub struct BucketInfo {
    pub name: String,
    pub created: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectEntry {
    pub name: String,
    pub key: String,
    pub is_folder: bool,
    pub size: Option<u64>,
    pub last_modified: Option<DateTime<Utc>>,
}

/// One page of a ListObjects or ListObjectsV2 answer.
#[derive(Debug, Default)]
pub struct ListPage {
    pub prefixes: Vec<String>,
    pub objects: Vec<(String, u64, Option<DateTime<Utc>>)>,
    pub truncated: bool,
    pub next_token: Option<String>,
    pub next_marker: Option<String>,
}

pub fn parse_list_page(root: &Element) -> ListPage {
    let mut page = ListPage {
        truncated: root.child_text("IsTruncated").is_some_and(|value| value.trim() == "true"),
        next_token: root.child_text("NextContinuationToken").filter(|value| !value.is_empty()),
        next_marker: root.child_text("NextMarker").filter(|value| !value.is_empty()),
        ..Default::default()
    };
    for common in root.children_named("CommonPrefixes") {
        if let Some(prefix) = common.child_text("Prefix") {
            page.prefixes.push(prefix);
        }
    }
    for contents in root.children_named("Contents") {
        let Some(key) = contents.child_text("Key") else { continue };
        let size = contents.child_text("Size").and_then(|value| value.trim().parse().ok()).unwrap_or(0);
        let modified = contents.child_text("LastModified").and_then(|value| parse_iso8601(&value));
        page.objects.push((key, size, modified));
    }
    page
}

pub fn parse_buckets(root: &Element) -> Vec<BucketInfo> {
    let mut buckets: Vec<BucketInfo> = root
        .child("Buckets")
        .map(|list| {
            list.children_named("Bucket")
                .filter_map(|bucket| {
                    let name = bucket.child_text("Name")?;
                    if name.is_empty() {
                        return None;
                    }
                    let created = bucket.child_text("CreationDate").and_then(|value| parse_iso8601(&value));
                    Some(BucketInfo { name, created })
                })
                .collect()
        })
        .unwrap_or_default();
    sort_buckets(&mut buckets);
    buckets
}

pub fn sort_buckets(buckets: &mut [BucketInfo]) {
    buckets.sort_by_key(|bucket| bucket.name.to_lowercase());
}

fn relative<'a>(prefix: &str, key: &'a str) -> &'a str {
    if prefix.is_empty() {
        return key;
    }
    key.strip_prefix(prefix).unwrap_or("")
}

/// Reads a page as one folder level below `prefix`.
pub fn entries_from_listing(prefix: &str, page: &ListPage) -> Vec<ObjectEntry> {
    let mut prefix = prefix.to_owned();
    if !prefix.is_empty() && !prefix.ends_with('/') {
        prefix.push('/');
    }
    let mut folders: Vec<ObjectEntry> = Vec::new();
    let mut folder_keys = std::collections::HashMap::new();
    let mut add_folder = |relative: &str, folders: &mut Vec<ObjectEntry>| {
        let name = relative.trim_matches('/').split('/').next().unwrap_or("");
        if name.is_empty() {
            return;
        }
        let key = format!("{prefix}{name}/");
        if let Entry::Vacant(slot) = folder_keys.entry(key.clone()) {
            slot.insert(folders.len());
            folders.push(ObjectEntry { name: name.to_owned(), key, is_folder: true, size: None, last_modified: None });
        }
    };
    for common in &page.prefixes {
        add_folder(relative(&prefix, common), &mut folders);
    }
    let mut files: Vec<ObjectEntry> = Vec::new();
    let mut seen_files = HashSet::new();
    for (key, size, modified) in &page.objects {
        if key.is_empty() || *key == prefix {
            continue;
        }
        let rel = relative(&prefix, key);
        if rel.is_empty() {
            continue;
        }
        match rel.split_once('/') {
            Some((name, _)) => add_folder(name, &mut folders),
            None => {
                if seen_files.insert(key.clone()) {
                    files.push(ObjectEntry {
                        name: rel.to_owned(),
                        key: key.clone(),
                        is_folder: false,
                        size: Some(*size),
                        last_modified: *modified,
                    });
                }
            }
        }
    }
    folders.extend(files);
    folders
}

/// Files below a prefix with their sizes, without folder placeholders.
pub fn file_records(prefix: &str, page: &ListPage) -> Vec<(String, u64)> {
    page.objects
        .iter()
        .filter(|(key, _, _)| {
            !key.is_empty() && key != prefix && !key.ends_with('/') && (prefix.is_empty() || key.starts_with(prefix))
        })
        .map(|(key, size, _)| (key.clone(), *size))
        .collect()
}

/// Keys below a prefix including folder placeholders, but not the prefix itself.
pub fn object_keys(prefix: &str, page: &ListPage) -> Vec<String> {
    page.objects
        .iter()
        .map(|(key, _, _)| key)
        .filter(|key| !key.is_empty() && *key != prefix && (prefix.is_empty() || key.starts_with(prefix)))
        .cloned()
        .collect()
}

pub fn sort_entries(entries: &mut [ObjectEntry]) {
    entries.sort_by_cached_key(|entry| (!entry.is_folder, entry.name.to_lowercase()));
}

pub fn dedupe_entries(entries: Vec<ObjectEntry>) -> Vec<ObjectEntry> {
    let mut seen = HashSet::new();
    entries.into_iter().filter(|entry| seen.insert(entry.key.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(prefixes: &[&str], objects: &[(&str, u64)]) -> ListPage {
        ListPage {
            prefixes: prefixes.iter().map(|p| p.to_string()).collect(),
            objects: objects.iter().map(|(k, s)| (k.to_string(), *s, None)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn folders_and_files_at_one_level() {
        let listing =
            page(&["data/sub/"], &[("data/", 0), ("data/a.txt", 3), ("data/deep/x.bin", 9), ("data/b.csv", 1)]);
        let mut entries = entries_from_listing("data", &listing);
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.is_folder)).collect();
        assert_eq!(names, vec![("deep", true), ("sub", true), ("a.txt", false), ("b.csv", false)]);
        assert_eq!(entries[0].key, "data/deep/");
        assert_eq!(entries[2].size, Some(3));
    }

    #[test]
    fn ignores_keys_outside_prefix() {
        let listing = page(&[], &[("other/a", 1)]);
        assert!(entries_from_listing("data/", &listing).is_empty());
    }

    #[test]
    fn file_records_skip_placeholders() {
        let listing = page(&[], &[("p/", 0), ("p/a", 1), ("p/d/", 0), ("p/d/b", 2)]);
        assert_eq!(file_records("p/", &listing), vec![("p/a".to_string(), 1), ("p/d/b".to_string(), 2)]);
    }

    #[test]
    fn object_keys_keep_placeholders() {
        let listing = page(&[], &[("p/", 0), ("p/a", 1), ("p/d/", 0), ("p/d/b", 2), ("q/x", 3)]);
        assert_eq!(object_keys("p/", &listing), vec!["p/a", "p/d/", "p/d/b"]);
    }

    #[test]
    fn parses_bucket_list() {
        let doc = crate::xml::parse(
            "<ListAllMyBucketsResult><Buckets><Bucket><Name>zeta</Name></Bucket>\
             <Bucket><Name>Alpha</Name><CreationDate>2020-01-01T00:00:00.000Z</CreationDate></Bucket></Buckets></ListAllMyBucketsResult>",
        )
        .unwrap();
        let buckets = parse_buckets(&doc.root);
        assert_eq!(buckets[0].name, "Alpha");
        assert!(buckets[0].created.is_some());
        assert_eq!(buckets[1].name, "zeta");
    }
}
