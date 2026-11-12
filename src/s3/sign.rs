//! AWS Signature Version 4 and the older Version 2 (HMAC-SHA1).

use base64::Engine as _;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::{Digest, Sha256};

pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Everything except the RFC 3986 unreserved characters.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');
/// Same as [`QUERY`] but keeps the path separator.
const PATH: &AsciiSet = &QUERY.remove(b'/');

pub fn encode_path(path: &str) -> String {
    utf8_percent_encode(path, PATH).to_string()
}

pub fn encode_query_part(part: &str) -> String {
    utf8_percent_encode(part, QUERY).to_string()
}

/// Sorted, encoded `k=v&k=v`, used both in the URL and in the signature.
pub fn canonical_query(params: &[(String, String)]) -> String {
    let mut encoded: Vec<(String, String)> =
        params.iter().map(|(k, v)| (encode_query_part(k), encode_query_part(v))).collect();
    encoded.sort();
    encoded.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

pub struct V4<'a> {
    pub method: &'a str,
    /// Already encoded path, for example `/bucket/my%20file.txt`.
    pub canonical_uri: &'a str,
    pub canonical_query: &'a str,
    /// Headers to sign, including `host`. Names in any case.
    pub headers: &'a [(String, String)],
    pub payload_hash: &'a str,
    pub region: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub now: DateTime<Utc>,
}

impl V4<'_> {
    /// Returns the value for the `Authorization` header. The caller must also
    /// send `x-amz-date` and `x-amz-content-sha256`, which have to be part of `headers`.
    pub fn authorization(&self) -> String {
        let date = self.now.format("%Y%m%d").to_string();
        let timestamp = self.now.format("%Y%m%dT%H%M%SZ").to_string();
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.split_whitespace().collect::<Vec<_>>().join(" ")))
            .collect();
        headers.sort();
        let canonical_headers: String = headers.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
        let signed_headers = headers.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(";");
        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            self.method, self.canonical_uri, self.canonical_query, canonical_headers, signed_headers, self.payload_hash
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign =
            format!("AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}", sha256_hex(canonical_request.as_bytes()));
        let key = hmac_sha256(format!("AWS4{}", self.secret_key).as_bytes(), date.as_bytes());
        let key = hmac_sha256(&key, self.region.as_bytes());
        let key = hmac_sha256(&key, b"s3");
        let key = hmac_sha256(&key, b"aws4_request");
        let signature = hex(&hmac_sha256(&key, string_to_sign.as_bytes()));
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key
        )
    }
}

/// Query parameters that are part of the resource in Signature Version 2.
const V2_SUBRESOURCES: &[&str] = &[
    "acl",
    "cors",
    "delete",
    "lifecycle",
    "location",
    "logging",
    "notification",
    "partNumber",
    "policy",
    "requestPayment",
    "restore",
    "tagging",
    "torrent",
    "uploadId",
    "uploads",
    "versionId",
    "versioning",
    "versions",
    "website",
];

pub struct V2<'a> {
    pub method: &'a str,
    /// `/bucket/key` with the key encoded as in the request path.
    pub resource_path: &'a str,
    pub query: &'a [(String, String)],
    pub content_md5: &'a str,
    pub content_type: &'a str,
    pub date: &'a str,
    /// All `x-amz-*` headers of the request.
    pub amz_headers: &'a [(String, String)],
    pub access_key: &'a str,
    pub secret_key: &'a str,
}

impl V2<'_> {
    pub fn string_to_sign(&self) -> String {
        let mut amz: Vec<(String, String)> = self
            .amz_headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
            .filter(|(name, _)| name.starts_with("x-amz-"))
            .collect();
        amz.sort();
        let amz: String = amz.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
        let mut sub: Vec<(String, String)> =
            self.query.iter().filter(|(name, _)| V2_SUBRESOURCES.contains(&name.as_str())).cloned().collect();
        sub.sort();
        let mut resource = self.resource_path.to_owned();
        if !sub.is_empty() {
            resource.push('?');
            let parts: Vec<String> = sub
                .iter()
                .map(|(name, value)| if value.is_empty() { name.clone() } else { format!("{name}={value}") })
                .collect();
            resource.push_str(&parts.join("&"));
        }
        format!("{}\n{}\n{}\n{}\n{amz}{resource}", self.method, self.content_md5, self.content_type, self.date)
    }

    pub fn authorization(&self) -> String {
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(self.secret_key.as_bytes()).expect("any key length");
        mac.update(self.string_to_sign().as_bytes());
        let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        format!("AWS {}:{signature}", self.access_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // Examples from the AWS documentation "Signature Calculations for the
    // Authorization Header: Transferring Payload in a Single Chunk".
    #[test]
    fn v4_get_object_with_range() {
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let headers = headers(&[
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ]);
        let auth = V4 {
            method: "GET",
            canonical_uri: "/test.txt",
            canonical_query: "",
            headers: &headers,
            payload_hash: EMPTY_SHA256,
            region: "us-east-1",
            access_key: ACCESS,
            secret_key: SECRET,
            now,
        }
        .authorization();
        assert!(auth.ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"), "{auth}");
        assert!(auth.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn v4_list_objects() {
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let headers = headers(&[
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ]);
        let query = canonical_query(&[("prefix".into(), "J".into()), ("max-keys".into(), "2".into())]);
        assert_eq!(query, "max-keys=2&prefix=J");
        let auth = V4 {
            method: "GET",
            canonical_uri: "/",
            canonical_query: &query,
            headers: &headers,
            payload_hash: EMPTY_SHA256,
            region: "us-east-1",
            access_key: ACCESS,
            secret_key: SECRET,
            now,
        }
        .authorization();
        assert!(auth.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"), "{auth}");
    }

    // Example from the AWS documentation "Signing and authenticating REST requests" (V2).
    #[test]
    fn v2_get_object() {
        let signer = V2 {
            method: "GET",
            resource_path: "/awsexamplebucket1/photos/puppy.jpg",
            query: &[],
            content_md5: "",
            content_type: "",
            date: "Tue, 27 Mar 2007 19:36:42 +0000",
            amz_headers: &[],
            access_key: ACCESS,
            secret_key: SECRET,
        };
        assert_eq!(signer.authorization(), "AWS AKIAIOSFODNN7EXAMPLE:qgk2+6Sv9/oM7G3qLEjTH1a1l1g=");
    }

    #[test]
    fn v2_keeps_subresources_only() {
        let query = vec![
            ("uploadId".to_string(), "abc".to_string()),
            ("prefix".to_string(), "x".to_string()),
            ("partNumber".to_string(), "2".to_string()),
        ];
        let signer = V2 {
            method: "PUT",
            resource_path: "/b/k",
            query: &query,
            content_md5: "",
            content_type: "text/plain",
            date: "D",
            amz_headers: &[("X-Amz-Meta-A".into(), " 1 ".into())],
            access_key: "a",
            secret_key: "s",
        };
        assert_eq!(signer.string_to_sign(), "PUT\n\ntext/plain\nD\nx-amz-meta-a:1\n/b/k?partNumber=2&uploadId=abc");
    }

    #[test]
    fn encodes_keys_like_s3() {
        assert_eq!(encode_path("/b/a b/ä+c~.txt"), "/b/a%20b/%C3%A4%2Bc~.txt");
        assert_eq!(encode_query_part("a/b c"), "a%2Fb%20c");
    }
}
