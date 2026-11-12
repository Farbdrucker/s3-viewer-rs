//! S3 client for OpenStack and other compatible endpoints.
//!
//! Requests are signed here (SigV4 or SigV2) and sent with a blocking HTTP
//! agent, so every call is meant to run on a worker thread.

pub mod error;
pub mod listing;
pub mod sign;

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ureq::http;

pub use error::{S3Error, S3Result};
pub use listing::{BucketInfo, ObjectEntry};

use crate::tr;
use listing::{
    ListPage, dedupe_entries, entries_from_listing, file_records, parse_buckets, parse_list_page, sort_entries,
};

pub const MAX_LISTED: usize = 5000;
const PAGE_SIZE: usize = 1000;
/// Objects up to this size go up in one PUT, larger ones as multipart upload.
const MULTIPART_THRESHOLD: u64 = 16 * 1024 * 1024;
const MAX_PARTS: u64 = 9000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SignatureVersion {
    #[default]
    #[serde(rename = "s3v4")]
    V4,
    #[serde(rename = "s3")]
    V2,
}

impl SignatureVersion {
    pub fn code(self) -> &'static str {
        match self {
            SignatureVersion::V4 => "s3v4",
            SignatureVersion::V2 => "s3",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "s3v4" => Some(SignatureVersion::V4),
            "s3" => Some(SignatureVersion::V2),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SignatureVersion::V4 => "s3v4",
            SignatureVersion::V2 => "s3 (SigV2)",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub endpoint_url: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub direct_bucket: String,
    pub path_style: bool,
    pub verify_ssl: bool,
    pub signature_version: SignatureVersion,
}

impl std::fmt::Debug for ConnectionSettings {
    /// Never prints the secret key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionSettings")
            .field("endpoint_url", &self.endpoint_url)
            .field("access_key", &self.access_key)
            .field("region", &self.region)
            .field("direct_bucket", &self.direct_bucket)
            .field("path_style", &self.path_style)
            .field("verify_ssl", &self.verify_ssl)
            .field("signature_version", &self.signature_version)
            .finish_non_exhaustive()
    }
}

struct Endpoint {
    scheme: String,
    /// Host plus port when the port is not the scheme default.
    authority: String,
    /// Path below the host without a trailing slash, usually empty.
    base_path: String,
}

fn split_endpoint(url: &str) -> Option<Endpoint> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    let default_port = if scheme == "https" { ":443" } else { ":80" };
    let authority = authority.strip_suffix(default_port).unwrap_or(authority).to_ascii_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or("").trim_end_matches('/').to_owned();
    Some(Endpoint { scheme, authority, base_path: path })
}

pub fn host_of(url: &str) -> String {
    split_endpoint(url).map(|endpoint| endpoint.authority).unwrap_or_default()
}

pub fn normalize_endpoint(url: &str) -> S3Result<String> {
    let mut url = url.trim().to_owned();
    if url.is_empty() {
        return Err(S3Error::Invalid(tr!("err_endpoint_missing")));
    }
    if !url.contains("://") {
        url = format!("https://{url}");
    }
    if split_endpoint(&url).is_none() {
        return Err(S3Error::Invalid(tr!("err_endpoint_url")));
    }
    Ok(url.trim_end_matches('/').to_owned())
}

#[allow(clippy::too_many_arguments)]
pub fn validate_settings(
    endpoint_url: &str,
    access_key: &str,
    secret_key: &str,
    region: &str,
    direct_bucket: &str,
    path_style: bool,
    verify_ssl: bool,
    signature_version: SignatureVersion,
) -> S3Result<ConnectionSettings> {
    let endpoint_url = normalize_endpoint(endpoint_url)?;
    let access_key = access_key.trim();
    let secret_key = secret_key.trim();
    let region = match region.trim() {
        "" => "us-east-1",
        region => region,
    };
    let direct_bucket = direct_bucket.trim();
    if access_key.is_empty() || secret_key.is_empty() {
        return Err(S3Error::Invalid(tr!("err_keys_required")));
    }
    if direct_bucket.contains('/') || direct_bucket.chars().any(char::is_whitespace) {
        return Err(S3Error::Invalid(tr!("err_bucket_name")));
    }
    Ok(ConnectionSettings {
        endpoint_url,
        access_key: access_key.to_owned(),
        secret_key: secret_key.to_owned(),
        region: region.to_owned(),
        direct_bucket: direct_bucket.to_owned(),
        path_style,
        verify_ssl,
        signature_version,
    })
}

/// Tells a running transfer to stop. Cheap to clone and share with workers.
#[derive(Clone, Default, Debug)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> S3Result<()> {
        if self.is_cancelled() { Err(S3Error::Cancelled) } else { Ok(()) }
    }
}

#[derive(Debug, Clone)]
pub struct Connected {
    pub buckets: Vec<BucketInfo>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FetchedObject {
    pub data: Vec<u8>,
    /// Size of the whole object.
    pub size: u64,
    pub content_type: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub size: Option<u64>,
    pub content_type: String,
}

enum Payload<'a> {
    Empty,
    Bytes(&'a [u8]),
    Stream { reader: &'a mut dyn Read, len: u64, sha256: String },
}

struct Request<'a> {
    method: &'static str,
    bucket: Option<&'a str>,
    key: Option<&'a str>,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

impl<'a> Request<'a> {
    fn new(method: &'static str, bucket: Option<&'a str>, key: Option<&'a str>) -> Self {
        Request { method, bucket, key, query: Vec::new(), headers: Vec::new() }
    }

    fn query(mut self, name: &str, value: impl Into<String>) -> Self {
        self.query.push((name.to_owned(), value.into()));
        self
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }
}

pub struct Session {
    pub settings: ConnectionSettings,
    endpoint: Endpoint,
    agent: ureq::Agent,
    list_v1: AtomicBool,
}

impl Session {
    pub fn new(settings: ConnectionSettings) -> S3Result<Self> {
        let endpoint =
            split_endpoint(&settings.endpoint_url).ok_or_else(|| S3Error::Invalid(tr!("err_endpoint_url")))?;
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .disable_verification(!settings.verify_ssl)
            .build();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .accept_encoding("identity")
            .user_agent(concat!("s3-viewer/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(8)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .timeout_per_write(Some(Duration::from_secs(60)))
            .tls_config(tls)
            .build()
            .new_agent();
        Ok(Session { settings, endpoint, agent, list_v1: AtomicBool::new(false) })
    }

    pub fn host(&self) -> &str {
        &self.endpoint.authority
    }

    // ---------------------------------------------------------------- requests

    fn virtual_host(&self, bucket: &str) -> bool {
        !self.settings.path_style
            && !bucket.is_empty()
            && bucket.len() <= 63
            && !bucket.contains('.')
            && bucket.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
            && !bucket.starts_with('-')
            && !bucket.ends_with('-')
    }

    fn send(&self, request: &Request<'_>, payload: Payload<'_>) -> S3Result<http::Response<ureq::Body>> {
        let base = &self.endpoint.base_path;
        let key_path = request.key.map(|key| format!("/{key}")).unwrap_or_default();
        let (host, raw_path, v2_resource) = match request.bucket {
            Some(bucket) if self.virtual_host(bucket) => {
                let path = if key_path.is_empty() { format!("{base}/") } else { format!("{base}{key_path}") };
                (
                    format!("{bucket}.{}", self.endpoint.authority),
                    path,
                    format!("/{bucket}/{}", request.key.unwrap_or("")),
                )
            }
            Some(bucket) => {
                let path = format!("{base}/{bucket}{key_path}");
                (self.endpoint.authority.clone(), path.clone(), path)
            }
            None => (self.endpoint.authority.clone(), format!("{base}/"), "/".to_owned()),
        };
        let path = sign::encode_path(&raw_path);
        let query = sign::canonical_query(&request.query);
        let url = if query.is_empty() {
            format!("{}://{host}{path}", self.endpoint.scheme)
        } else {
            format!("{}://{host}{path}?{query}", self.endpoint.scheme)
        };

        let mut headers = request.headers.clone();
        headers.push(("host".into(), host.clone()));
        let now = Utc::now();
        match self.settings.signature_version {
            SignatureVersion::V4 => {
                let payload_hash = match &payload {
                    Payload::Empty => sign::EMPTY_SHA256.to_owned(),
                    Payload::Bytes(bytes) => sign::sha256_hex(bytes),
                    Payload::Stream { sha256, .. } => sha256.clone(),
                };
                headers.push(("x-amz-date".into(), now.format("%Y%m%dT%H%M%SZ").to_string()));
                headers.push(("x-amz-content-sha256".into(), payload_hash.clone()));
                let authorization = sign::V4 {
                    method: request.method,
                    canonical_uri: &path,
                    canonical_query: &query,
                    headers: &headers,
                    payload_hash: &payload_hash,
                    region: &self.settings.region,
                    access_key: &self.settings.access_key,
                    secret_key: &self.settings.secret_key,
                    now,
                }
                .authorization();
                headers.push(("authorization".into(), authorization));
            }
            SignatureVersion::V2 => {
                let date = httpdate::fmt_http_date(now.into());
                let content_type = header_value(&headers, "content-type").unwrap_or_default();
                let authorization = sign::V2 {
                    method: request.method,
                    resource_path: &sign::encode_path(&v2_resource),
                    query: &request.query,
                    content_md5: "",
                    content_type: &content_type,
                    date: &date,
                    amz_headers: &headers,
                    access_key: &self.settings.access_key,
                    secret_key: &self.settings.secret_key,
                }
                .authorization();
                headers.push(("date".into(), date));
                headers.push(("authorization".into(), authorization));
            }
        }

        let mut builder = http::Request::builder().method(request.method).uri(&url);
        for (name, value) in &headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let result = match payload {
            Payload::Empty => {
                let builder = if request.method == "PUT" || request.method == "POST" {
                    builder.header("content-length", "0")
                } else {
                    builder
                };
                self.agent.run(builder.body(()).map_err(http_error)?)
            }
            Payload::Bytes(bytes) => self.agent.run(builder.body(bytes).map_err(http_error)?),
            Payload::Stream { reader, len, .. } => {
                let builder = builder.header("content-length", len.to_string());
                self.agent.run(builder.body(ureq::SendBody::from_reader(reader)).map_err(http_error)?)
            }
        };
        let mut response = result?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let mut body = Vec::new();
            if request.method != "HEAD" {
                let _ = response.body_mut().as_reader().take(256 * 1024).read_to_end(&mut body);
            }
            return Err(error::service_error(status, &body));
        }
        Ok(response)
    }

    /// Sends a request without body, retrying once on transient failures.
    fn call(&self, request: &Request<'_>) -> S3Result<http::Response<ureq::Body>> {
        match self.send(request, Payload::Empty) {
            Err(error) if error.is_transient() => {
                std::thread::sleep(Duration::from_millis(300));
                self.send(request, Payload::Empty)
            }
            other => other,
        }
    }

    fn call_xml(&self, request: &Request<'_>) -> S3Result<crate::xml::Element> {
        let mut response = self.call(request)?;
        let mut body = Vec::new();
        response.body_mut().as_reader().take(64 * 1024 * 1024).read_to_end(&mut body)?;
        parse_xml(&body)
    }

    // ------------------------------------------------------------- operations

    pub fn establish(&self) -> S3Result<Connected> {
        let listed = self.list_buckets();
        let direct = self.settings.direct_bucket.clone();
        if direct.is_empty() {
            return Ok(Connected { buckets: listed?, notice: None });
        }
        self.ensure_bucket(&direct)?;
        let (mut buckets, notice) = match listed {
            Ok(buckets) => (buckets, None),
            Err(_) => (Vec::new(), Some(tr!("direct_bucket_notice"))),
        };
        if buckets.iter().all(|bucket| bucket.name != direct) {
            buckets.push(BucketInfo { name: direct, created: None });
        }
        listing::sort_buckets(&mut buckets);
        Ok(Connected { buckets, notice })
    }

    pub fn list_buckets(&self) -> S3Result<Vec<BucketInfo>> {
        let root = self.call_xml(&Request::new("GET", None, None))?;
        Ok(parse_buckets(&root))
    }

    pub fn ensure_bucket(&self, bucket: &str) -> S3Result<()> {
        self.page(bucket, "", Some("/"), None, 1).map(|_| ())
    }

    fn page(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: Option<&str>,
        token: Option<&str>,
        max_keys: usize,
    ) -> S3Result<(ListPage, Option<String>)> {
        if !self.list_v1.load(Ordering::Relaxed) {
            let mut request = Request::new("GET", Some(bucket), None)
                .query("list-type", "2")
                .query("prefix", prefix)
                .query("max-keys", max_keys.to_string());
            if let Some(delimiter) = delimiter {
                request = request.query("delimiter", delimiter);
            }
            if let Some(token) = token {
                request = request.query("continuation-token", token);
            }
            match self.call_xml(&request) {
                Ok(root) => {
                    let page = parse_list_page(&root);
                    let next = if page.truncated { page.next_token.clone() } else { None };
                    return Ok((page, next));
                }
                Err(error) if token.is_none() && error.unsupported() => {
                    self.list_v1.store(true, Ordering::Relaxed);
                }
                Err(error) => return Err(error),
            }
        }
        let mut request =
            Request::new("GET", Some(bucket), None).query("prefix", prefix).query("max-keys", max_keys.to_string());
        if let Some(delimiter) = delimiter {
            request = request.query("delimiter", delimiter);
        }
        if let Some(marker) = token {
            request = request.query("marker", marker);
        }
        let root = self.call_xml(&request)?;
        let page = parse_list_page(&root);
        let next = if page.truncated {
            page.next_marker
                .clone()
                .or_else(|| page.objects.last().map(|(key, _, _)| key.clone()))
                .or_else(|| page.prefixes.last().cloned())
        } else {
            None
        };
        Ok((page, next))
    }

    /// One folder level, sorted folders first. The flag says whether the list was cut at [`MAX_LISTED`].
    pub fn list_folder(&self, bucket: &str, prefix: &str) -> S3Result<(Vec<ObjectEntry>, bool)> {
        let prefix = folder_prefix(prefix);
        let mut collected = Vec::new();
        let mut token: Option<String> = None;
        let mut capped = false;
        loop {
            let (page, next) = self.page(bucket, &prefix, Some("/"), token.as_deref(), PAGE_SIZE)?;
            collected.extend(entries_from_listing(&prefix, &page));
            if collected.len() >= MAX_LISTED {
                collected.truncate(MAX_LISTED);
                capped = true;
                break;
            }
            match next {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        let mut entries = dedupe_entries(collected);
        sort_entries(&mut entries);
        Ok((entries, capped))
    }

    /// All file keys and sizes below a prefix, without folder placeholders.
    pub fn list_files(&self, bucket: &str, prefix: &str) -> S3Result<(Vec<(String, u64)>, bool)> {
        let prefix = folder_prefix(prefix);
        let mut records = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let (page, next) = self.page(bucket, &prefix, None, token.as_deref(), PAGE_SIZE)?;
            records.extend(file_records(&prefix, &page));
            if records.len() >= MAX_LISTED {
                records.truncate(MAX_LISTED);
                return Ok((records, true));
            }
            match next {
                Some(next) => token = Some(next),
                None => return Ok((records, false)),
            }
        }
    }

    pub fn head(&self, bucket: &str, key: &str) -> S3Result<ObjectMeta> {
        let response = self.call(&Request::new("HEAD", Some(bucket), Some(key)))?;
        Ok(ObjectMeta {
            size: response_header(&response, "content-length").and_then(|value| value.parse().ok()),
            content_type: response_header(&response, "content-type").unwrap_or_default(),
        })
    }

    /// Reads at most `limit` bytes from the start of an object.
    pub fn fetch(&self, bucket: &str, key: &str, limit: u64) -> S3Result<FetchedObject> {
        let meta = match self.head(bucket, key) {
            Ok(meta) => Some(meta),
            Err(error) if error.is_not_found() => return Err(error),
            Err(_) => None,
        };
        if let Some(ObjectMeta { size: Some(size), content_type }) = meta {
            let truncated = size > limit;
            let ranged = if truncated { Some((0, limit)) } else { None };
            let (data, response_type, content_range) = match self.get_bytes(bucket, key, limit, ranged) {
                Err(S3Error::Service { .. }) if truncated => self.get_bytes(bucket, key, limit, None)?,
                other => other?,
            };
            let total = total_size(content_range.as_deref(), size);
            let truncated = truncated || total > data.len() as u64;
            let content_type = if content_type.is_empty() { response_type } else { content_type };
            return Ok(FetchedObject { data, size: total, content_type, truncated });
        }
        let (mut data, content_type, content_range) = self.get_bytes(bucket, key, limit + 1, None)?;
        let truncated = data.len() as u64 > limit;
        data.truncate(limit as usize);
        let mut size = total_size(content_range.as_deref(), data.len() as u64);
        if truncated {
            size = size.max(limit + 1);
        }
        Ok(FetchedObject { data, size, content_type, truncated })
    }

    fn get_bytes(
        &self,
        bucket: &str,
        key: &str,
        limit: u64,
        range: Option<(u64, u64)>,
    ) -> S3Result<(Vec<u8>, String, Option<String>)> {
        let mut request = Request::new("GET", Some(bucket), Some(key));
        if let Some((start, len)) = range {
            request = request.header("range", format!("bytes={start}-{}", start + len - 1));
        }
        let mut response = self.call(&request)?;
        let content_type = response_header(&response, "content-type").unwrap_or_default();
        let content_range = response_header(&response, "content-range");
        let mut data = Vec::new();
        response.body_mut().as_reader().take(limit).read_to_end(&mut data)?;
        Ok((data, content_type, content_range))
    }

    /// Reads `len` bytes at `offset`. Copes with servers that ignore `Range`.
    pub fn read_range(&self, bucket: &str, key: &str, offset: u64, len: u64) -> S3Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let request = Request::new("GET", Some(bucket), Some(key))
            .header("range", format!("bytes={offset}-{}", offset + len - 1));
        let mut response = self.call(&request)?;
        let partial = response.status().as_u16() == 206;
        let mut reader = response.body_mut().as_reader();
        if !partial && offset > 0 {
            io::copy(&mut (&mut reader).take(offset), &mut io::sink())?;
        }
        let mut data = Vec::with_capacity(len as usize);
        reader.take(len).read_to_end(&mut data)?;
        Ok(data)
    }

    pub fn download(
        &self,
        bucket: &str,
        key: &str,
        destination: &Path,
        progress: &dyn Fn(u64),
        cancel: &CancelFlag,
    ) -> S3Result<()> {
        cancel.check()?;
        let name = destination.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let temporary = destination.with_file_name(format!(".{name}.part"));
        let result = (|| {
            let mut response = self.call(&Request::new("GET", Some(bucket), Some(key)))?;
            let mut reader = response.body_mut().as_reader();
            let mut file = File::create(&temporary)?;
            let mut buffer = vec![0u8; 256 * 1024];
            loop {
                cancel.check()?;
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                file.write_all(&buffer[..read])?;
                progress(read as u64);
            }
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, destination)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    pub fn upload(
        &self,
        bucket: &str,
        key: &str,
        source: &Path,
        progress: &dyn Fn(u64),
        cancel: &CancelFlag,
    ) -> S3Result<()> {
        cancel.check()?;
        let mut file = File::open(source)?;
        let size = file.metadata()?.len();
        if size <= MULTIPART_THRESHOLD {
            let mut data = Vec::with_capacity(size as usize);
            file.read_to_end(&mut data)?;
            let request = Request::new("PUT", Some(bucket), Some(key));
            self.put_part(&request, &data, progress, cancel)?;
            return Ok(());
        }
        let part_size = multipart_part_size(size);
        let root = self.call_xml(&Request::new("POST", Some(bucket), Some(key)).query("uploads", ""))?;
        let upload_id = root
            .child_text("UploadId")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| S3Error::Other("CreateMultipartUpload returned no UploadId".into()))?;
        let result = (|| {
            let mut parts = Vec::new();
            let mut buffer = vec![0u8; part_size as usize];
            let mut number = 1u32;
            loop {
                cancel.check()?;
                let filled = read_full(&mut file, &mut buffer)?;
                if filled == 0 {
                    break;
                }
                let request = Request::new("PUT", Some(bucket), Some(key))
                    .query("partNumber", number.to_string())
                    .query("uploadId", upload_id.clone());
                let etag = self.put_part(&request, &buffer[..filled], progress, cancel)?;
                parts.push((number, etag));
                number += 1;
                if filled < buffer.len() {
                    break;
                }
            }
            let mut body = String::from("<CompleteMultipartUpload>");
            for (number, etag) in &parts {
                body.push_str(&format!(
                    "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
                    crate::xml::escape(etag)
                ));
            }
            body.push_str("</CompleteMultipartUpload>");
            let request = Request::new("POST", Some(bucket), Some(key))
                .query("uploadId", upload_id.clone())
                .header("content-type", "application/xml");
            let mut response = self.send(&request, Payload::Bytes(body.as_bytes()))?;
            // CompleteMultipartUpload may report an error with status 200.
            let mut answer = Vec::new();
            response.body_mut().as_reader().take(1024 * 1024).read_to_end(&mut answer)?;
            if let Ok(root) = parse_xml(&answer)
                && root.local_name() == "Error"
            {
                return Err(error::service_error(500, &answer));
            }
            Ok(())
        })();
        if result.is_err() {
            let abort = Request::new("DELETE", Some(bucket), Some(key)).query("uploadId", upload_id);
            let _ = self.send(&abort, Payload::Empty);
        }
        result
    }

    /// Sends one PUT with a progress-reporting body and returns the ETag.
    fn put_part(
        &self,
        request: &Request<'_>,
        data: &[u8],
        progress: &dyn Fn(u64),
        cancel: &CancelFlag,
    ) -> S3Result<String> {
        let mut reader = ProgressReader { inner: data, progress, cancel };
        let payload = Payload::Stream { reader: &mut reader, len: data.len() as u64, sha256: sign::sha256_hex(data) };
        let response = self
            .send(request, payload)
            .map_err(|error| if cancel.is_cancelled() { S3Error::Cancelled } else { error })?;
        Ok(response_header(&response, "etag").unwrap_or_default())
    }

    pub fn put_bytes(&self, bucket: &str, key: &str, data: &[u8], content_type: &str) -> S3Result<()> {
        let request = Request::new("PUT", Some(bucket), Some(key)).header("content-type", content_type);
        self.send(&request, Payload::Bytes(data)).map(|_| ())
    }
}

struct ProgressReader<'a> {
    inner: &'a [u8],
    progress: &'a dyn Fn(u64),
    cancel: &'a CancelFlag,
}

impl Read for ProgressReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let read = self.inner.read(buf)?;
        if read > 0 {
            (self.progress)(read as u64);
        }
        Ok(read)
    }
}

fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

/// Part size for multipart uploads: at least 16 MiB and few enough parts for the 10 000 limit.
pub fn multipart_part_size(size: u64) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let needed = size.div_ceil(MAX_PARTS);
    needed.div_ceil(MIB).max(16) * MIB
}

fn folder_prefix(prefix: &str) -> String {
    if prefix.is_empty() || prefix.ends_with('/') { prefix.to_owned() } else { format!("{prefix}/") }
}

fn parse_xml(body: &[u8]) -> S3Result<crate::xml::Element> {
    let text = String::from_utf8_lossy(body);
    crate::xml::parse(&text)
        .map(|document| document.root)
        .map_err(|error| S3Error::Other(format!("Unreadable answer from the endpoint: {error}")))
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.clone())
}

fn response_header(response: &http::Response<ureq::Body>, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
}

fn http_error(error: http::Error) -> S3Error {
    S3Error::Other(error.to_string())
}

fn total_size(content_range: Option<&str>, fallback: u64) -> u64 {
    content_range
        .and_then(|range| range.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse().ok())
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_endpoints() {
        assert_eq!(normalize_endpoint(" s3.example.com/ ").unwrap(), "https://s3.example.com");
        assert!(normalize_endpoint("ftp://x").is_err());
        assert!(normalize_endpoint("").is_err());
        let endpoint = split_endpoint("https://S3.Example.com:443/base/").unwrap();
        assert_eq!(endpoint.authority, "s3.example.com");
        assert_eq!(endpoint.base_path, "/base");
        assert_eq!(split_endpoint("http://[::1]:9000").unwrap().authority, "[::1]:9000");
    }

    #[test]
    fn validates_settings() {
        let ok = validate_settings("h", "a", "s", "", "", true, true, SignatureVersion::V4).unwrap();
        assert_eq!(ok.region, "us-east-1");
        assert!(validate_settings("h", "", "s", "", "", true, true, SignatureVersion::V4).is_err());
        assert!(validate_settings("h", "a", "s", "", "a b", true, true, SignatureVersion::V4).is_err());
        assert!(!format!("{ok:?}").contains("\"s\""));
    }

    #[test]
    fn part_sizes_respect_limits() {
        assert_eq!(multipart_part_size(100 * 1024 * 1024), 16 * 1024 * 1024);
        let huge = 1024u64 * 1024 * 1024 * 1024; // 1 TiB
        let part = multipart_part_size(huge);
        assert!(huge.div_ceil(part) <= 10_000);
    }

    #[test]
    fn reads_total_from_content_range() {
        assert_eq!(total_size(Some("bytes 0-9/1234"), 5), 1234);
        assert_eq!(total_size(Some("bytes 0-9/*"), 5), 5);
        assert_eq!(total_size(None, 5), 5);
    }
}
