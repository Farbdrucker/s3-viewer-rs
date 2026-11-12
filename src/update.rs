//! Optional check for a newer release on GitHub.
//!
//! The check only reads the public "latest release" API, at most once a day,
//! with short timeouts. It never downloads or installs anything; failures are
//! silent so the viewer works the same without access to GitHub.

use std::io::Read;
use std::time::Duration;

pub const RELEASES_API: &str = "https://api.github.com/repos/Farbdrucker/s3-viewer-rs/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/Farbdrucker/s3-viewer-rs/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Seconds between two automatic checks.
pub const INTERVAL: i64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Without the leading "v", for example "2.3.0".
    pub version: String,
    pub url: String,
}

/// `S3VIEWER_UPDATE_CHECK=0` (or `false`, `off`, `no`) turns the check off for
/// everyone on the machine, regardless of the setting in the app.
pub fn disabled_by_environment() -> bool {
    std::env::var("S3VIEWER_UPDATE_CHECK")
        .map(|value| matches!(value.trim().to_lowercase().as_str(), "0" | "false" | "off" | "no"))
        .unwrap_or(false)
}

/// Asks GitHub for the latest release. Meant for a worker thread.
pub fn fetch_latest() -> Result<Release, String> {
    fetch_from(RELEASES_API)
}

fn fetch_from(url: &str) -> Result<Release, String> {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(concat!("s3-viewer/", env!("CARGO_PKG_VERSION")))
        .timeout_global(Some(Duration::from_secs(8)))
        .timeout_connect(Some(Duration::from_secs(4)))
        .tls_config(tls)
        .build()
        .new_agent();
    let mut response =
        agent.get(url).header("accept", "application/vnd.github+json").call().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered {}", response.status()));
    }
    let mut body = Vec::new();
    response.body_mut().as_reader().take(1024 * 1024).read_to_end(&mut body).map_err(|error| error.to_string())?;
    parse_release(&body).ok_or_else(|| "unexpected answer from GitHub".to_owned())
}

pub fn parse_release(body: &[u8]) -> Option<Release> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    if value.get("draft").and_then(|v| v.as_bool()) == Some(true)
        || value.get("prerelease").and_then(|v| v.as_bool()) == Some(true)
    {
        return None;
    }
    let tag = value.get("tag_name")?.as_str()?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_owned();
    parse_version(&version)?;
    let url = value.get("html_url").and_then(|v| v.as_str()).unwrap_or(RELEASES_PAGE).to_owned();
    Some(Release { version, url })
}

/// Major, minor and patch of "2.1.0" or "v2.1.0-beta"; anything after `-` or `+` is ignored.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// Whether `candidate` is a newer version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// Whether the last check is old enough for another one.
pub fn due(last_check: Option<i64>, now: i64) -> bool {
    match last_check {
        Some(last) => now - last >= INTERVAL || now < last,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("2.2.0", "2.1.9"));
        assert!(is_newer("v3.0.0", "2.9.9"));
        assert!(is_newer("2.10.0", "2.9.0"));
        assert!(!is_newer("2.1.0", "2.1.0"));
        assert!(!is_newer("2.0.9", "2.1.0"));
        assert!(!is_newer("garbage", "2.1.0"));
        assert!(is_newer("2.2", "2.1.5"));
    }

    #[test]
    fn reads_github_answer() {
        let body = br#"{"tag_name": "v2.3.1", "html_url": "https://github.com/x/y/releases/tag/v2.3.1", "draft": false, "prerelease": false}"#;
        let release = parse_release(body).unwrap();
        assert_eq!(release.version, "2.3.1");
        assert!(release.url.ends_with("v2.3.1"));
        assert!(parse_release(br#"{"tag_name": "v9.0.0", "prerelease": true}"#).is_none());
        assert!(parse_release(br#"{"message": "Not Found"}"#).is_none());
        assert!(parse_release(b"<html>proxy login</html>").is_none());
    }

    #[test]
    fn checks_once_a_day() {
        assert!(due(None, 1000));
        assert!(!due(Some(1000), 1000 + INTERVAL - 1));
        assert!(due(Some(1000), 1000 + INTERVAL));
        assert!(due(Some(5000), 1000), "a clock that went backwards does not block the check");
    }

    /// Talks to the real GitHub API: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn reads_the_real_latest_release() {
        let release = fetch_latest().unwrap();
        assert!(parse_version(&release.version).is_some(), "{release:?}");
        assert!(release.url.starts_with("https://github.com/"));
    }

    #[test]
    fn unreachable_host_fails_quickly() {
        let started = std::time::Instant::now();
        assert!(fetch_from("http://127.0.0.1:9/releases/latest").is_err());
        assert!(started.elapsed() < Duration::from_secs(9));
    }
}
