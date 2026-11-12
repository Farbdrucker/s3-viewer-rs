//! Errors from S3 calls and their wording for the user.

use crate::tr;

#[derive(Debug, Clone)]
pub enum S3Error {
    /// Input that failed validation; the text is already meant for the user.
    Invalid(String),
    /// The service answered with an error status.
    Service {
        status: u16,
        code: String,
        message: String,
    },
    Unreachable(String),
    Tls(String),
    Timeout,
    Cancelled,
    Io(String),
    Other(String),
}

pub type S3Result<T> = Result<T, S3Error>;

impl S3Error {
    pub fn code(&self) -> &str {
        match self {
            S3Error::Service { code, .. } => code,
            _ => "",
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            S3Error::Service { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether the endpoint said it does not implement the request
    /// (used to fall back from ListObjectsV2 to ListObjects).
    pub fn unsupported(&self) -> bool {
        match self {
            S3Error::Service { status, code, message } => {
                *status == 501
                    || code == "NotImplemented"
                    || code == "XNotImplemented"
                    || message.to_lowercase().contains("not implemented")
            }
            _ => false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self.code(), "NoSuchKey" | "404" | "NotFound" | "NoSuchBucket")
    }

    /// Worth one more attempt for idempotent requests.
    pub fn is_transient(&self) -> bool {
        match self {
            S3Error::Service { status, .. } => matches!(status, 500 | 502 | 503 | 504),
            S3Error::Unreachable(_) | S3Error::Timeout => true,
            _ => false,
        }
    }

    pub fn humanize(&self) -> String {
        match self {
            S3Error::Invalid(text) => text.clone(),
            S3Error::Service { status, code, message } => humanize_service(*status, code, message),
            S3Error::Unreachable(detail) => {
                if looks_like_tls(detail) {
                    tr!("err_tls")
                } else {
                    tr!("err_unreachable")
                }
            }
            S3Error::Tls(_) => tr!("err_tls"),
            S3Error::Timeout => tr!("err_timeout"),
            S3Error::Cancelled => tr!("download_cancelled"),
            S3Error::Io(detail) | S3Error::Other(detail) => detail.clone(),
        }
    }
}

impl std::fmt::Display for S3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.humanize())
    }
}

impl From<std::io::Error> for S3Error {
    fn from(error: std::io::Error) -> Self {
        S3Error::Io(error.to_string())
    }
}

impl From<ureq::Error> for S3Error {
    fn from(error: ureq::Error) -> Self {
        let detail = error.to_string();
        match error {
            ureq::Error::Timeout(_) => S3Error::Timeout,
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) => S3Error::Tls(detail),
            ureq::Error::Io(ref io) => match io.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => S3Error::Timeout,
                _ if looks_like_tls(&detail) => S3Error::Tls(detail),
                _ => S3Error::Unreachable(detail),
            },
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::ConnectProxyFailed(_) => {
                S3Error::Unreachable(detail)
            }
            _ if looks_like_tls(&detail) => S3Error::Tls(detail),
            _ => S3Error::Other(detail),
        }
    }
}

fn looks_like_tls(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("ssl") || text.contains("certificate") || text.contains("tls")
}

fn humanize_service(status: u16, code: &str, message: &str) -> String {
    let known = match code {
        "InvalidAccessKeyId" => Some("err_access_key"),
        "SignatureDoesNotMatch" => Some("err_sig_mismatch"),
        "AccessDenied" => Some("err_access_denied"),
        "AllAccessDisabled" => Some("err_access_disabled"),
        "NoSuchBucket" => Some("err_no_bucket"),
        "NoSuchKey" => Some("err_no_key"),
        "InvalidBucketName" => Some("err_bad_bucket"),
        "AuthorizationHeaderMalformed" => Some("err_auth_header"),
        "PermanentRedirect" => Some("err_redirect"),
        "RequestTimeTooSkewed" => Some("err_clock"),
        "InvalidArgument" => Some("err_argument"),
        _ => None,
    };
    if let Some(id) = known {
        return tr!("err_with_code", message = tr!(id), code = code);
    }
    if status == 501 || code == "NotImplemented" || code == "XNotImplemented" {
        return tr!("err_not_implemented", code = code);
    }
    if status == 403 {
        return tr!("err_forbidden", code = code);
    }
    if !message.is_empty() {
        return tr!("err_with_code", message = message, code = code);
    }
    tr!("err_failed", code = code)
}

/// Builds the error for a non-success response from its status and XML body.
pub fn service_error(status: u16, body: &[u8]) -> S3Error {
    let text = String::from_utf8_lossy(body);
    let (mut code, mut message) = (String::new(), String::new());
    if let Ok(document) = crate::xml::parse(&text) {
        code = document.root.child_text("Code").unwrap_or_default();
        message = document.root.child_text("Message").unwrap_or_default();
    }
    if code.is_empty() {
        // HEAD answers and some proxies send no body. Same convention as botocore.
        code = match status {
            301 => "PermanentRedirect".to_owned(),
            404 => "NotFound".to_owned(),
            _ => status.to_string(),
        };
    }
    S3Error::Service { status, code, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, TEST_LOCK, set_language};

    #[test]
    fn humanizes_known_codes() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language(Language::En);
        let error = service_error(403, b"<Error><Code>SignatureDoesNotMatch</Code><Message>x</Message></Error>");
        assert!(error.humanize().starts_with("The signature does not match"));
        assert!(error.humanize().ends_with("(SignatureDoesNotMatch)"));
        let error = service_error(403, b"");
        assert_eq!(error.humanize(), "Access denied (403).");
        let error = service_error(501, b"<Error><Code>NotImplemented</Code></Error>");
        assert!(error.unsupported());
        assert!(service_error(404, b"").is_not_found());
        set_language(Language::De);
    }
}
