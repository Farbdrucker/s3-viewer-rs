//! Runs the S3 client against a real endpoint. Skipped unless configured:
//!
//! ```sh
//! S3VIEWER_TEST_ENDPOINT=http://localhost:8333 S3VIEWER_TEST_ACCESS=testkey \
//! S3VIEWER_TEST_SECRET=testsecret123 S3VIEWER_TEST_BUCKET=viewer-test cargo test --test s3_integration
//! ```
//!
//! The bucket must exist and may be filled with test objects.

use std::sync::atomic::{AtomicU64, Ordering};

use s3_viewer::geotiff::{self, FnSource};
use s3_viewer::s3::{CancelFlag, S3Error, Session, SignatureVersion, validate_settings};

struct Target {
    endpoint: String,
    access: String,
    secret: String,
    bucket: String,
}

fn target() -> Option<Target> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    Some(Target {
        endpoint: var("S3VIEWER_TEST_ENDPOINT")?,
        access: var("S3VIEWER_TEST_ACCESS")?,
        secret: var("S3VIEWER_TEST_SECRET")?,
        bucket: var("S3VIEWER_TEST_BUCKET")?,
    })
}

fn session(target: &Target, signature: SignatureVersion, secret: &str) -> Session {
    let settings =
        validate_settings(&target.endpoint, &target.access, secret, "us-east-1", "", true, true, signature).unwrap();
    Session::new(settings).unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("s3viewer-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn round_trip_against_endpoint() {
    let Some(target) = target() else {
        eprintln!("S3VIEWER_TEST_* not set; skipping");
        return;
    };
    for signature in [SignatureVersion::V4, SignatureVersion::V2] {
        let s3 = session(&target, signature, &target.secret);
        let bucket = target.bucket.as_str();
        // A fresh prefix per run keeps the test repeatable against the same bucket.
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let base = format!("it-{stamp}-{}/", signature.code());

        let connected = s3.establish().unwrap();
        assert!(connected.buckets.iter().any(|b| b.name == bucket), "{signature:?}: bucket listed");

        // Keys with characters that need careful encoding.
        let odd = format!("{base}dir with space/ä ö+&=x;(1).txt");
        s3.put_bytes(bucket, &odd, "hallo welt\n".as_bytes(), "text/plain; charset=utf-8").unwrap();
        s3.put_bytes(bucket, &format!("{base}top.json"), br#"{"a": 1}"#, "application/json").unwrap();
        s3.put_bytes(bucket, &format!("{base}deep/a/b/c.csv"), b"x,y\n1,2\n", "text/csv").unwrap();

        let (entries, capped) = s3.list_folder(bucket, &base).unwrap();
        assert!(!capped);
        let names: Vec<(&str, bool)> = entries.iter().map(|e| (e.name.as_str(), e.is_folder)).collect();
        assert_eq!(names, vec![("deep", true), ("dir with space", true), ("top.json", false)], "{signature:?}");

        let (inner, _) = s3.list_folder(bucket, &format!("{base}dir with space")).unwrap();
        assert_eq!(inner[0].key, odd);

        let fetched = s3.fetch(bucket, &odd, 1_000_000).unwrap();
        assert_eq!(fetched.data, b"hallo welt\n");
        assert!(!fetched.truncated);
        assert!(fetched.content_type.starts_with("text/plain"));

        let partial = s3.fetch(bucket, &odd, 5).unwrap();
        assert_eq!(partial.data, b"hallo");
        assert!(partial.truncated);
        assert_eq!(partial.size, 11);

        assert_eq!(s3.read_range(bucket, &odd, 6, 4).unwrap(), b"welt");

        let (files, _) = s3.list_files(bucket, &base).unwrap();
        assert_eq!(files.len(), 3);

        // Multipart upload (above the 16 MiB threshold), then download it again.
        let dir = scratch(signature.code());
        let source = dir.join("big.bin");
        let data: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &data).unwrap();
        let sent = AtomicU64::new(0);
        let key = format!("{base}big.bin");
        s3.upload(
            bucket,
            &key,
            &source,
            &|n| {
                sent.fetch_add(n, Ordering::Relaxed);
            },
            &CancelFlag::new(),
        )
        .unwrap();
        assert_eq!(sent.load(Ordering::Relaxed), data.len() as u64);
        assert_eq!(s3.head(bucket, &key).unwrap().size, Some(data.len() as u64));

        let target_file = dir.join("back.bin");
        let received = AtomicU64::new(0);
        s3.download(
            bucket,
            &key,
            &target_file,
            &|n| {
                received.fetch_add(n, Ordering::Relaxed);
            },
            &CancelFlag::new(),
        )
        .unwrap();
        assert_eq!(std::fs::read(&target_file).unwrap(), data);
        assert_eq!(received.load(Ordering::Relaxed), data.len() as u64);

        // Cancelling leaves no partial file behind.
        let cancel = CancelFlag::new();
        let cancelled_file = dir.join("cancelled.bin");
        let result = s3.download(bucket, &key, &cancelled_file, &|_| cancel.cancel(), &cancel);
        assert!(matches!(result, Err(S3Error::Cancelled)));
        assert!(!cancelled_file.exists());
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));

        // A GeoTIFF read through range requests gives the same result as from memory.
        let tiff = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/dem_f32_deflate.tif")).unwrap();
        let tiff_key = format!("{base}dem.tif");
        s3.put_bytes(bucket, &tiff_key, &tiff, "image/tiff").unwrap();
        let source = FnSource {
            len: tiff.len() as u64,
            read: |offset, len| s3.read_range(bucket, &tiff_key, offset, len).map_err(|e| e.humanize()),
        };
        let options = geotiff::Options { edge: 128, ..Default::default() };
        let remote = geotiff::load(&source, &options, &|| false).unwrap();
        let local = geotiff::load(&geotiff::MemorySource(tiff.clone()), &options, &|| false).unwrap();
        assert_eq!(remote.raster.width, local.raster.width);
        assert_eq!(remote.raster.bands[0].len(), local.raster.bands[0].len());
        assert!(
            remote.raster.bands[0]
                .iter()
                .zip(&local.raster.bands[0])
                .all(|(a, b)| a == b || (a.is_nan() && b.is_nan()))
        );
        assert!(remote.info.fetched < tiff.len() as u64, "only part of the file is read");

        // New folder, then delete everything below the prefix (DeleteObjects with Content-MD5).
        s3.create_folder(bucket, &format!("{base}made")).unwrap();
        let (entries, _) = s3.list_folder(bucket, &base).unwrap();
        assert!(entries.iter().any(|e| e.name == "made" && e.is_folder), "{signature:?}: new folder listed");
        let (keys, capped) = s3.list_object_keys(bucket, &base).unwrap();
        assert!(!capped);
        assert!(keys.contains(&format!("{base}made/")), "placeholders are included");
        let done = AtomicU64::new(0);
        let deleted = s3
            .delete_keys(
                bucket,
                &keys,
                &|n| {
                    done.fetch_add(n, Ordering::Relaxed);
                },
                &CancelFlag::new(),
            )
            .unwrap();
        assert_eq!(deleted, keys.len());
        assert_eq!(done.load(Ordering::Relaxed), keys.len() as u64);
        assert!(s3.list_object_keys(bucket, &base).unwrap().0.is_empty(), "{signature:?}: everything deleted");
        // Deleting again is harmless.
        s3.delete_keys(bucket, &keys, &|_| {}, &CancelFlag::new()).unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn wrong_secret_is_reported() {
    let Some(target) = target() else { return };
    for signature in [SignatureVersion::V4, SignatureVersion::V2] {
        let error = session(&target, signature, "definitely-wrong").list_buckets().unwrap_err();
        assert_eq!(error.status(), Some(403), "{signature:?}: {error:?}");
        assert!(!error.humanize().is_empty());
    }
}

#[test]
fn unreachable_endpoint() {
    let settings = validate_settings("http://127.0.0.1:9", "a", "b", "", "", true, true, SignatureVersion::V4).unwrap();
    let error = Session::new(settings).unwrap().list_buckets().unwrap_err();
    assert!(matches!(error, S3Error::Unreachable(_)), "{error:?}");
}
