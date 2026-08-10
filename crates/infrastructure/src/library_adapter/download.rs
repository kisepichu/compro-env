//! Bounded HTTPS downloads with checksum verification and strict redirect
//! policy (spec §6.9, plan 041 Task 2).
//!
//! `download_artifact` streams a URL into a destination file, enforcing a
//! byte-count ceiling and computing the SHA-256 of every byte written. The
//! reqwest client is built with `no_proxy()` and (optionally) a caller-supplied
//! HTTPS proxy or CA bundle so no environment secret leaks in. Redirects that
//! change the URL scheme — the canonical HTTPS-to-HTTP downgrade — are
//! rejected before the response body is read.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain::adapter_build::ContentDigest;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Default maximum artifact size (1 GiB). Callers can tighten it per-request.
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

/// Default HTTP timeout for both connect and read halves.
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(120);

/// Immutable download policy. Prod builds always set `allowed_scheme` to
/// `"https"`; the fixture-server tests widen it to `"http"`.
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    pub allowed_scheme: &'static str,
    pub max_bytes: u64,
    pub timeout: Duration,
    pub https_proxy: Option<String>,
    pub ca_bundle_pem: Option<Vec<u8>>,
}

impl DownloadPolicy {
    pub fn https() -> Self {
        Self {
            allowed_scheme: "https",
            max_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            timeout: DEFAULT_HTTP_TIMEOUT,
            https_proxy: None,
            ca_bundle_pem: None,
        }
    }
}

/// Result of a successful download.
#[derive(Debug, Clone)]
pub struct DownloadedArtifact {
    pub path: PathBuf,
    pub sha256: ContentDigest,
    pub bytes: u64,
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("failed to build HTTP client: {source}")]
    ClientBuild {
        #[source]
        source: reqwest::Error,
    },
    #[error("URL scheme is not allowed ({expected:?} required): {url:?}")]
    InvalidScheme { url: String, expected: &'static str },
    #[error("redirect target scheme changed to {actual:?}, expected {expected:?}: {url:?}")]
    InvalidRedirectScheme {
        url: String,
        expected: &'static str,
        actual: String,
    },
    #[error("HTTP request failed: {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("HTTP body read failed: {source}")]
    ReadBody {
        #[source]
        source: std::io::Error,
    },
    #[error("HTTP {status} while downloading {url}")]
    HttpStatus { url: String, status: u16 },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("downloaded artifact exceeded {limit} bytes: {url}")]
    TooLarge { url: String, limit: u64 },
    #[error("downloaded artifact was truncated at {actual} bytes (expected more)")]
    Truncated { url: String, actual: u64 },
    #[error("checksum mismatch for {url}: expected {expected}, actual {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("PEM certificate is invalid: {source}")]
    InvalidCaBundle {
        #[source]
        source: reqwest::Error,
    },
    #[error("proxy URL is invalid: {source}")]
    InvalidProxy {
        #[source]
        source: reqwest::Error,
    },
}

fn build_client(policy: &DownloadPolicy) -> Result<Client, DownloadError> {
    let expected_scheme = policy.allowed_scheme;
    let redirect = Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        if attempt.url().scheme() != expected_scheme {
            // The invalid-redirect-scheme branch is surfaced as `Request`
            // by reqwest; the caller checks the response URL below to
            // produce a `InvalidRedirectScheme` diagnostic when possible.
            return attempt.stop();
        }
        attempt.follow()
    });
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(redirect)
        .connect_timeout(policy.timeout)
        .timeout(policy.timeout);
    if let Some(proxy) = &policy.https_proxy {
        let proxy = reqwest::Proxy::https(proxy)
            .map_err(|source| DownloadError::InvalidProxy { source })?;
        builder = builder.proxy(proxy);
    }
    if let Some(pem) = &policy.ca_bundle_pem {
        let cert = reqwest::Certificate::from_pem(pem)
            .map_err(|source| DownloadError::InvalidCaBundle { source })?;
        builder = builder.add_root_certificate(cert);
    }
    builder
        .build()
        .map_err(|source| DownloadError::ClientBuild { source })
}

/// Download `url` into `dest`, verify `expected_sha256`, and enforce
/// `policy.max_bytes`.
pub fn download_artifact(
    url: &str,
    dest: &Path,
    expected_sha256: &ContentDigest,
    policy: &DownloadPolicy,
) -> Result<DownloadedArtifact, DownloadError> {
    validate_scheme(url, policy.allowed_scheme)?;

    let client = build_client(policy)?;
    let response = client
        .get(url)
        .send()
        .map_err(|source| DownloadError::Request { source })?;
    let final_url = response.url().clone();
    if final_url.scheme() != policy.allowed_scheme {
        return Err(DownloadError::InvalidRedirectScheme {
            url: final_url.to_string(),
            expected: policy.allowed_scheme,
            actual: final_url.scheme().to_string(),
        });
    }
    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::HttpStatus {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    let content_length = response.content_length();

    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|source| DownloadError::Write {
            path: dir.display().to_string(),
            source,
        })?;
    }
    let mut file = File::create(dest).map_err(|source| DownloadError::Write {
        path: dest.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes_written: u64 = 0;
    let mut reader = response;
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(source) => {
                return Err(DownloadError::ReadBody { source });
            }
        };
        bytes_written = bytes_written.saturating_add(n as u64);
        if bytes_written > policy.max_bytes {
            return Err(DownloadError::TooLarge {
                url: url.to_string(),
                limit: policy.max_bytes,
            });
        }
        file.write_all(&buffer[..n])
            .map_err(|source| DownloadError::Write {
                path: dest.display().to_string(),
                source,
            })?;
        hasher.update(&buffer[..n]);
    }
    file.flush().map_err(|source| DownloadError::Write {
        path: dest.display().to_string(),
        source,
    })?;
    file.sync_all().map_err(|source| DownloadError::Write {
        path: dest.display().to_string(),
        source,
    })?;
    drop(file);

    if let Some(expected_len) = content_length
        && bytes_written < expected_len
    {
        return Err(DownloadError::Truncated {
            url: url.to_string(),
            actual: bytes_written,
        });
    }

    let bytes: [u8; 32] = hasher.finalize().into();
    let actual = ContentDigest::from_sha256_bytes(bytes);
    if actual != *expected_sha256 {
        // Delete the tainted file so callers cannot silently reuse it.
        let _ = std::fs::remove_file(dest);
        return Err(DownloadError::ChecksumMismatch {
            url: url.to_string(),
            expected: expected_sha256.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(DownloadedArtifact {
        path: dest.to_path_buf(),
        sha256: actual,
        bytes: bytes_written,
    })
}

fn validate_scheme(url: &str, expected: &'static str) -> Result<(), DownloadError> {
    let scheme = url.split(':').next().unwrap_or("");
    if scheme != expected {
        return Err(DownloadError::InvalidScheme {
            url: url.to_string(),
            expected,
        });
    }
    Ok(())
}
