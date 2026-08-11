//! Tests for the pinned Lean 4.30.0 toolchain selector, layout validation,
//! lake-manifest validation, and the per-target archive filter in
//! `prepare_dependencies` (plan 048 Task 1).
//!
//! No network I/O: HTTP fixtures use a local `tiny_http` server, and the
//! `validate_lean_layout` tests build a synthetic tree of empty files under
//! `tempfile::TempDir` with stubbed version readers.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use domain::adapter_build::{ContentDigest, TargetPlatform};
use domain::adapter_prepare::{ArchiveDependency, ArchiveFormat, DependencyManifest};
use infrastructure::library_adapter::download::DownloadPolicy;
use infrastructure::library_adapter::lean_toolchain::{
    LEAN_EXPECTED_VERSION, LEAN_TOOLCHAIN_LINUX_ARM64_NAME, LEAN_TOOLCHAIN_LINUX_X64_NAME,
    LEAN_TOOLCHAIN_MACOS_ARM64_NAME, LeanToolchainError, all_lean_toolchain_specs,
    select_lean_toolchain, validate_lake_manifest, validate_lean_layout,
};
use infrastructure::library_adapter::prepare::{
    PrepareRequest, PrepareRunError, prepare_dependencies, prepared_dir,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

// ─── select_lean_toolchain ──────────────────────────────────────────────────

fn platform(os: &str, arch: &str) -> TargetPlatform {
    TargetPlatform {
        os: os.into(),
        arch: arch.into(),
    }
}

#[test]
fn selects_linux_x86_64_spec() {
    let spec = select_lean_toolchain(&platform("linux", "x86_64")).unwrap();
    assert_eq!(spec.archive_name, LEAN_TOOLCHAIN_LINUX_X64_NAME);
    assert_eq!(spec.target_os, "linux");
    assert_eq!(spec.target_arch, "x86_64");
    assert_eq!(spec.format, ArchiveFormat::TarZst);
    assert_eq!(
        spec.url,
        "https://github.com/leanprover/lean4/releases/download/v4.30.0/lean-4.30.0-linux.tar.zst"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "4dad74141c2c119ca1aa626656be83b8e14238afba97271fd7bf1eb3f081b319"
    );
    assert_eq!(spec.expected_version, LEAN_EXPECTED_VERSION);
}

#[test]
fn selects_linux_aarch64_spec() {
    let spec = select_lean_toolchain(&platform("linux", "aarch64")).unwrap();
    assert_eq!(spec.archive_name, LEAN_TOOLCHAIN_LINUX_ARM64_NAME);
    assert_eq!(spec.target_os, "linux");
    assert_eq!(spec.target_arch, "aarch64");
    assert_eq!(spec.format, ArchiveFormat::TarZst);
    assert_eq!(
        spec.url,
        "https://github.com/leanprover/lean4/releases/download/v4.30.0/lean-4.30.0-linux_aarch64.tar.zst"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "c99c6f0edd446956d4758c59d4383e8e6411ff6cc71a01f9caabe5eba454121d"
    );
}

#[test]
fn selects_macos_aarch64_spec() {
    let spec = select_lean_toolchain(&platform("macos", "aarch64")).unwrap();
    assert_eq!(spec.archive_name, LEAN_TOOLCHAIN_MACOS_ARM64_NAME);
    assert_eq!(spec.target_os, "macos");
    assert_eq!(spec.target_arch, "aarch64");
    assert_eq!(spec.format, ArchiveFormat::TarZst);
    assert_eq!(
        spec.url,
        "https://github.com/leanprover/lean4/releases/download/v4.30.0/lean-4.30.0-darwin_aarch64.tar.zst"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "072dca4a38fbc0d3cedb96fea886cc243b424f2bd16247596200b9a9ab93f0f5"
    );
}

#[test]
fn rejects_unsupported_windows_x86_64() {
    let err = select_lean_toolchain(&platform("windows", "x86_64")).unwrap_err();
    assert!(
        matches!(err, LeanToolchainError::UnsupportedTarget { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_unsupported_linux_riscv64() {
    let err = select_lean_toolchain(&platform("linux", "riscv64")).unwrap_err();
    assert!(
        matches!(err, LeanToolchainError::UnsupportedTarget { .. }),
        "{err:?}"
    );
}

// ─── validate_lean_layout ───────────────────────────────────────────────────

fn write_lean_layout(root: &Path) {
    let bin = root.join("bin");
    let lib = root.join("lib/lean");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&lib).unwrap();
    fs::write(bin.join("lean"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::write(bin.join("lake"), b"#!/bin/sh\nexit 0\n").unwrap();
}

fn ok_lean_reader(_: &Path) -> Result<String, io::Error> {
    Ok("4.30.0".to_string())
}

fn ok_lake_reader(_: &Path) -> Result<String, io::Error> {
    Ok("4.30.0".to_string())
}

#[test]
fn validate_layout_accepts_synthetic_lean() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    let paths = validate_lean_layout(dir.path(), ok_lean_reader, ok_lake_reader).unwrap();
    assert_eq!(paths.root, dir.path());
    assert_eq!(paths.lean, dir.path().join("bin/lean"));
    assert_eq!(paths.lake, dir.path().join("bin/lake"));
    assert_eq!(paths.lib_dir, dir.path().join("lib/lean"));
    assert_eq!(paths.lean_version, "4.30.0");
    assert_eq!(paths.lake_version, "4.30.0");
}

#[test]
fn validate_layout_rejects_missing_lean() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    fs::remove_file(dir.path().join("bin/lean")).unwrap();
    let err = validate_lean_layout(dir.path(), ok_lean_reader, ok_lake_reader).unwrap_err();
    match err {
        LeanToolchainError::MissingBinary { path } => {
            assert!(path.ends_with("bin/lean"), "path = {path}");
        }
        other => panic!("expected MissingBinary, got {other:?}"),
    }
}

#[test]
fn validate_layout_rejects_missing_lake() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    fs::remove_file(dir.path().join("bin/lake")).unwrap();
    let err = validate_lean_layout(dir.path(), ok_lean_reader, ok_lake_reader).unwrap_err();
    match err {
        LeanToolchainError::MissingBinary { path } => {
            assert!(path.ends_with("bin/lake"), "path = {path}");
        }
        other => panic!("expected MissingBinary, got {other:?}"),
    }
}

#[test]
fn validate_layout_rejects_missing_lib() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    fs::remove_dir_all(dir.path().join("lib")).unwrap();
    let err = validate_lean_layout(dir.path(), ok_lean_reader, ok_lake_reader).unwrap_err();
    assert!(
        matches!(err, LeanToolchainError::MissingLib { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_layout_rejects_wrong_lean_version() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    let bad_reader = |_: &Path| Ok("4.29.0".to_string());
    let err = validate_lean_layout(dir.path(), bad_reader, ok_lake_reader).unwrap_err();
    match err {
        LeanToolchainError::VersionMismatch {
            tool,
            expected,
            actual,
        } => {
            assert_eq!(tool, "lean");
            assert_eq!(expected, "4.30.0");
            assert_eq!(actual, "4.29.0");
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

#[test]
fn validate_layout_rejects_wrong_lake_version() {
    let dir = TempDir::new().unwrap();
    write_lean_layout(dir.path());
    let bad_lake = |_: &Path| Ok("4.29.0".to_string());
    let err = validate_lean_layout(dir.path(), ok_lean_reader, bad_lake).unwrap_err();
    match err {
        LeanToolchainError::VersionMismatch {
            tool,
            expected,
            actual,
        } => {
            assert_eq!(tool, "lake");
            assert_eq!(expected, "4.30.0");
            assert_eq!(actual, "4.29.0");
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

// ─── validate_lake_manifest ─────────────────────────────────────────────────

#[test]
fn validate_lake_manifest_accepts_committed_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lake-manifest.json");
    fs::write(
        &path,
        r#"{
  "version": "1.1.0",
  "packagesDir": ".lake/packages",
  "packages": [],
  "name": "ce-lean-analyzer",
  "lakeDir": ".lake"
}
"#,
    )
    .unwrap();
    validate_lake_manifest(&path).unwrap();
}

#[test]
fn validate_lake_manifest_rejects_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lake-manifest.json");
    let err = validate_lake_manifest(&path).unwrap_err();
    assert!(
        matches!(err, LeanToolchainError::LakeManifestMissing { .. }),
        "{err:?}"
    );
}

#[test]
fn validate_lake_manifest_rejects_missing_packages() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lake-manifest.json");
    fs::write(
        &path,
        r#"{
  "version": "1.1.0",
  "packagesDir": ".lake/packages",
  "name": "ce-lean-analyzer",
  "lakeDir": ".lake"
}
"#,
    )
    .unwrap();
    let err = validate_lake_manifest(&path).unwrap_err();
    match err {
        LeanToolchainError::LakeManifestField { field, .. } => assert_eq!(field, "packages"),
        other => panic!("expected LakeManifestField, got {other:?}"),
    }
}

#[test]
fn validate_lake_manifest_rejects_missing_name() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lake-manifest.json");
    fs::write(
        &path,
        r#"{
  "version": "1.1.0",
  "packagesDir": ".lake/packages",
  "packages": [],
  "lakeDir": ".lake"
}
"#,
    )
    .unwrap();
    let err = validate_lake_manifest(&path).unwrap_err();
    match err {
        LeanToolchainError::LakeManifestField { field, .. } => assert_eq!(field, "name"),
        other => panic!("expected LakeManifestField, got {other:?}"),
    }
}

#[test]
fn validate_lake_manifest_rejects_invalid_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lake-manifest.json");
    fs::write(&path, "{ not json ").unwrap();
    let err = validate_lake_manifest(&path).unwrap_err();
    assert!(
        matches!(err, LeanToolchainError::LakeManifestParse { .. }),
        "{err:?}"
    );
}

// ─── prepare_dependencies per-target filtering ──────────────────────────────
//
// Same shape as `cpp_toolchain.rs` — a local `tiny_http` server serves a
// synthetic tar.zst archive and the manifest gates the three declared
// per-target Lean archives to the current host.

fn linux_x86_64() -> TargetPlatform {
    platform("linux", "x86_64")
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

fn build_tar_zst(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use zstd::stream::write::Encoder as ZstdEncoder;
    let mut zst = ZstdEncoder::new(Vec::new(), 3).expect("start zstd encoder");
    {
        let mut builder = tar::Builder::new(&mut zst);
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, name, &contents[..])
                .expect("append tar file");
        }
        builder.finish().expect("finish tar");
    }
    zst.finish().expect("finish zst")
}

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

fn manifest_from_specs(url: String, sha: ContentDigest) -> DependencyManifest {
    let mut archives = Vec::new();
    for spec in all_lean_toolchain_specs() {
        archives.push(ArchiveDependency {
            name: spec.archive_name,
            url: url.clone(),
            sha256: sha.clone(),
            format: ArchiveFormat::TarZst,
            target_os: Some(spec.target_os),
            target_arch: Some(spec.target_arch),
        });
    }
    DependencyManifest {
        archives,
        ..Default::default()
    }
}

#[test]
fn prepare_dependencies_downloads_only_matching_lean_archive() {
    let archive_bytes = build_tar_zst(&[("hello.txt", b"hello\n")]);
    let sha = sha256_hex(&archive_bytes);
    let server_bytes = archive_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(server_bytes.clone()).with_header(
            Header::from_bytes(b"Content-Type".as_ref(), b"application/zstd".as_ref()).unwrap(),
        ));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = manifest_from_specs(
        server.url("/lean.tar.zst"),
        ContentDigest::from_hex(sha).unwrap(),
    );

    let request = PrepareRequest {
        repository_root: repo.path().to_path_buf(),
        prepared_root: prepared_root.path().to_path_buf(),
        target_platform: linux_x86_64(),
        download_policy: http_policy(),
    };
    let set = prepare_dependencies(&request, &manifest).expect("prepare");
    let id_dir = prepared_dir(prepared_root.path(), &set.id);

    // Only the linux/x86_64 archive should be extracted.
    assert!(
        id_dir
            .join(format!("archives/{LEAN_TOOLCHAIN_LINUX_X64_NAME}"))
            .is_dir()
    );
    assert!(
        !id_dir
            .join(format!("archives/{LEAN_TOOLCHAIN_LINUX_ARM64_NAME}"))
            .exists()
    );
    assert!(
        !id_dir
            .join(format!("archives/{LEAN_TOOLCHAIN_MACOS_ARM64_NAME}"))
            .exists()
    );

    // And only the linux/x86_64 archive is byte-hashed under downloads/.
    assert!(
        id_dir
            .join(format!("downloads/{LEAN_TOOLCHAIN_LINUX_X64_NAME}.tar.zst"))
            .is_file()
    );
    assert!(
        !id_dir
            .join(format!(
                "downloads/{LEAN_TOOLCHAIN_LINUX_ARM64_NAME}.tar.zst"
            ))
            .exists()
    );
    assert!(
        !id_dir
            .join(format!(
                "downloads/{LEAN_TOOLCHAIN_MACOS_ARM64_NAME}.tar.zst"
            ))
            .exists()
    );

    // Manifest should list exactly the one prepared artifact.
    assert_eq!(set.manifest.artifacts.len(), 1);
    assert_eq!(
        set.manifest.artifacts[0].name,
        LEAN_TOOLCHAIN_LINUX_X64_NAME
    );
}

#[test]
fn prepare_dependencies_rejects_wrong_lean_digest() {
    let real_bytes = build_tar_zst(&[("hello.txt", b"actual\n")]);
    let advertised_bytes = build_tar_zst(&[("hello.txt", b"expected\n")]);
    let advertised_sha = sha256_hex(&advertised_bytes);

    let server_bytes = real_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(server_bytes.clone()));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = manifest_from_specs(
        server.url("/lean.tar.zst"),
        ContentDigest::from_hex(advertised_sha).unwrap(),
    );

    let request = PrepareRequest {
        repository_root: repo.path().to_path_buf(),
        prepared_root: prepared_root.path().to_path_buf(),
        target_platform: linux_x86_64(),
        download_policy: http_policy(),
    };
    let err = prepare_dependencies(&request, &manifest).unwrap_err();
    assert!(matches!(err, PrepareRunError::Download(_)), "{err:?}");
}
