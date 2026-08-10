//! Integration tests for `prepare_dependencies` (plan 041 Task 2).
//!
//! Spins up a local `tiny_http` fixture server per test so we can exercise
//! checksum failure, truncated downloads, redirect policy, unsafe tar/zip
//! entries, concurrent locking, and the successful atomic publication path
//! without ever touching the public Internet.

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{ArchiveDependency, ArchiveFormat, DependencyManifest};
use infrastructure::library_adapter::download::DownloadPolicy;
use infrastructure::library_adapter::prepare::{
    PREPARED_SUBDIR, PrepareLock, PrepareRequest, PrepareRunError, prepare_dependencies,
    prepared_dir,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

// ─── shared helpers ─────────────────────────────────────────────────────────

fn linux() -> TargetPlatform {
    TargetPlatform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn http_policy() -> DownloadPolicy {
    DownloadPolicy {
        allowed_scheme: "http",
        max_bytes: 8 * 1024 * 1024,
        timeout: Duration::from_secs(5),
        https_proxy: None,
        ca_bundle_pem: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out: [u8; 32] = hasher.finalize().into();
    let mut hex = String::with_capacity(64);
    for b in out {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

fn empty_manifest() -> DependencyManifest {
    DependencyManifest::default()
}

fn make_request(repo: &Path, prepared_root: &Path, policy: DownloadPolicy) -> PrepareRequest {
    PrepareRequest {
        repository_root: repo.to_path_buf(),
        prepared_root: prepared_root.to_path_buf(),
        target_platform: linux(),
        download_policy: policy,
    }
}

/// Wrap a simple response route with a shutdown flag so the server thread
/// exits when the test drops the handle.
struct FixtureServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start<F>(handler: F) -> Self
    where
        F: FnMut(tiny_http::Request) + Send + 'static,
    {
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let addr = server.server_addr().to_ip().expect("ip addr");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let mut handler = handler;
        let handle = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => handler(request),
                    Ok(None) => continue,
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── Tar helpers ────────────────────────────────────────────────────────────

fn build_tar_gz(entries: &[TarEntry]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut gz);
        for entry in entries {
            match entry {
                TarEntry::File { name, contents } => {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(contents.len() as u64);
                    header.set_mode(0o644);
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_cksum();
                    builder
                        .append_data(&mut header, name, &contents[..])
                        .expect("append tar file");
                }
                TarEntry::Symlink { name, target } => {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(0);
                    header.set_mode(0o777);
                    header.set_entry_type(tar::EntryType::Symlink);
                    header
                        .set_link_name(target)
                        .expect("symlink target fits in tar header");
                    header.set_cksum();
                    builder
                        .append_data(&mut header, name, std::io::empty())
                        .expect("append tar symlink");
                }
            }
        }
        builder.finish().expect("finish tar");
    }
    gz.finish().expect("finish gz")
}

enum TarEntry<'a> {
    File { name: &'a str, contents: &'a [u8] },
    Symlink { name: &'a str, target: &'a str },
}

fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options: SimpleFileOptions = SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).expect("start zip file");
            writer.write_all(content).expect("write zip file");
        }
        writer.finish().expect("finish zip");
    }
    cursor.into_inner()
}

// ─── Successful atomic publication ──────────────────────────────────────────

#[test]
fn prepare_dependencies_publishes_prepared_set_atomically() {
    let archive_bytes = build_tar_gz(&[TarEntry::File {
        name: "hello.txt",
        contents: b"hello world\n",
    }]);
    let sha = sha256_hex(&archive_bytes);
    let server_bytes = archive_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let response = Response::from_data(server_bytes.clone()).with_header(
            Header::from_bytes(b"Content-Type".as_ref(), b"application/gzip".as_ref()).unwrap(),
        );
        let _ = request.respond(response);
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "sample".into(),
            url: server.url("/sample.tar.gz"),
            sha256: ContentDigest::from_hex(sha).unwrap(),
            format: ArchiveFormat::TarGz,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let set = prepare_dependencies(&request, &manifest).expect("prepare");
    let id_dir = prepared_dir(prepared_root.path(), &set.id);
    assert!(id_dir.join("manifest.json").is_file());
    assert!(id_dir.join("archives/sample").is_dir());
    assert!(id_dir.join("downloads/sample.tar.gz").is_file());
    // Staging directory should not remain after atomic rename.
    let staging_root = prepared_root.path().join(PREPARED_SUBDIR);
    let staging_leftover: Vec<_> = fs::read_dir(&staging_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("staging-"))
        .collect();
    assert!(
        staging_leftover.is_empty(),
        "staging leftover: {staging_leftover:?}"
    );
}

// ─── Checksum mismatch ──────────────────────────────────────────────────────

#[test]
fn prepare_dependencies_fails_on_checksum_mismatch() {
    let real_bytes = build_tar_gz(&[TarEntry::File {
        name: "hello.txt",
        contents: b"real\n",
    }]);
    let advertised_bytes = build_tar_gz(&[TarEntry::File {
        name: "hello.txt",
        contents: b"expected\n",
    }]);
    let advertised_sha = sha256_hex(&advertised_bytes);

    let server_bytes = real_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(server_bytes.clone()));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "mismatch".into(),
            url: server.url("/mismatch.tar.gz"),
            sha256: ContentDigest::from_hex(advertised_sha).unwrap(),
            format: ArchiveFormat::TarGz,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Download(_)), "{err:?}");
    // No cache hit on failure: staging is torn down and no `<id>` directory
    // is published.
    let entries: Vec<_> = fs::read_dir(prepared_root.path().join(PREPARED_SUBDIR))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with("staging-"))
        .collect();
    assert!(entries.is_empty(), "unexpected cache hit: {entries:?}");
}

// ─── Truncated download ─────────────────────────────────────────────────────

#[test]
fn prepare_dependencies_fails_on_truncated_download() {
    let full_bytes = build_tar_gz(&[TarEntry::File {
        name: "hello.txt",
        contents: b"complete\n",
    }]);
    let advertised_sha = sha256_hex(&full_bytes);
    let full_len = full_bytes.len();

    let truncated = full_bytes[..full_bytes.len() / 2].to_vec();
    let server = FixtureServer::start(move |request| {
        let mut response = Response::from_data(truncated.clone());
        // Advertise the FULL length so the client detects truncation.
        response = response.with_header(
            Header::from_bytes(b"Content-Length".as_ref(), full_len.to_string().as_bytes())
                .unwrap(),
        );
        let _ = request.respond(response);
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "trunc".into(),
            url: server.url("/trunc.tar.gz"),
            sha256: ContentDigest::from_hex(advertised_sha).unwrap(),
            format: ArchiveFormat::TarGz,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Download(_)), "{err:?}");
}

// ─── Unsafe tar entry ───────────────────────────────────────────────────────

#[test]
fn prepare_dependencies_rejects_tar_symlink_entry() {
    let archive_bytes = build_tar_gz(&[TarEntry::Symlink {
        name: "link",
        target: "../etc/passwd",
    }]);
    let sha = sha256_hex(&archive_bytes);

    let bytes = archive_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(bytes.clone()));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "evil".into(),
            url: server.url("/evil.tar.gz"),
            sha256: ContentDigest::from_hex(sha).unwrap(),
            format: ArchiveFormat::TarGz,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Archive(_)), "{err:?}");
}

// ─── Unsafe zip entry ───────────────────────────────────────────────────────

#[test]
fn prepare_dependencies_rejects_zip_parent_traversal() {
    let archive_bytes = build_zip(&[("../evil.txt", b"pwned\n")]);
    let sha = sha256_hex(&archive_bytes);

    let bytes = archive_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(bytes.clone()));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "evilzip".into(),
            url: server.url("/evil.zip"),
            sha256: ContentDigest::from_hex(sha).unwrap(),
            format: ArchiveFormat::Zip,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Archive(_)), "{err:?}");
}

// ─── Redirect to non-allowed scheme ─────────────────────────────────────────

#[test]
fn prepare_dependencies_rejects_scheme_changing_redirect() {
    let server = FixtureServer::start(move |request| {
        // Redirect to a scheme not in the allowlist.
        let response = Response::empty(302).with_header(
            Header::from_bytes(
                b"Location".as_ref(),
                b"https://example.invalid/target".as_ref(),
            )
            .unwrap(),
        );
        let _ = request.respond(response);
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = DependencyManifest {
        archives: vec![ArchiveDependency {
            name: "redir".into(),
            url: server.url("/start"),
            sha256: ContentDigest::from_hex(
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap(),
            format: ArchiveFormat::TarGz,
        }],
        ..empty_manifest()
    };
    let request = make_request(repo.path(), prepared_root.path(), http_policy());
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Download(_)), "{err:?}");
}

// ─── Concurrent lock fails fast ─────────────────────────────────────────────

#[test]
fn prepare_lock_is_exclusive() {
    let prepared_root = TempDir::new().unwrap();
    fs::create_dir_all(prepared_root.path()).unwrap();
    let _held = PrepareLock::acquire(prepared_root.path()).unwrap();
    let err = PrepareLock::acquire(prepared_root.path()).unwrap_err();
    assert!(
        matches!(err, PrepareRunError::LockContended { .. }),
        "{err:?}"
    );
}
