//! Tests for the pinned Clang/LLVM 22.1.0 toolchain selector and the
//! per-target archive filter in `prepare_dependencies` (plan 045 Task 1).
//!
//! No network I/O: HTTP fixtures use a local `tiny_http` server, and the
//! `validate_llvm_layout` tests build a synthetic tree of empty files under
//! `tempfile::TempDir` with a stubbed `version_reader`.

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
use infrastructure::library_adapter::cpp_toolchain::{
    CPP_TOOLCHAIN_LINUX_ARM64_NAME, CPP_TOOLCHAIN_LINUX_X64_NAME, CPP_TOOLCHAIN_MACOS_ARM64_NAME,
    CppToolchainError, LLVM_EXPECTED_VERSION, all_cpp_toolchain_specs, select_cpp_toolchain,
    validate_llvm_layout,
};
use infrastructure::library_adapter::download::DownloadPolicy;
use infrastructure::library_adapter::prepare::{
    PrepareRequest, PrepareRunError, prepare_dependencies, prepared_dir,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

// ─── select_cpp_toolchain ───────────────────────────────────────────────────

fn platform(os: &str, arch: &str) -> TargetPlatform {
    TargetPlatform {
        os: os.into(),
        arch: arch.into(),
    }
}

#[test]
fn selects_linux_x86_64_spec() {
    let spec = select_cpp_toolchain(&platform("linux", "x86_64")).unwrap();
    assert_eq!(spec.archive_name, CPP_TOOLCHAIN_LINUX_X64_NAME);
    assert_eq!(spec.target_os, "linux");
    assert_eq!(spec.target_arch, "x86_64");
    assert_eq!(spec.format, ArchiveFormat::TarXz);
    assert_eq!(
        spec.url,
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.0/LLVM-22.1.0-Linux-X64.tar.xz"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "8d662e425e46c48b45f5f970770b5e37f323607c8c2cbc371593fc9c4ba1e7b3"
    );
    assert_eq!(spec.expected_version, LLVM_EXPECTED_VERSION);
}

#[test]
fn selects_linux_aarch64_spec() {
    let spec = select_cpp_toolchain(&platform("linux", "aarch64")).unwrap();
    assert_eq!(spec.archive_name, CPP_TOOLCHAIN_LINUX_ARM64_NAME);
    assert_eq!(spec.target_os, "linux");
    assert_eq!(spec.target_arch, "aarch64");
    assert_eq!(spec.format, ArchiveFormat::TarXz);
    assert_eq!(
        spec.url,
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.0/LLVM-22.1.0-Linux-ARM64.tar.xz"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "e3b4205fe45d5561dec9d46465873a79c26b25b028b310515b38c34f668c6aec"
    );
}

#[test]
fn selects_macos_aarch64_spec() {
    let spec = select_cpp_toolchain(&platform("macos", "aarch64")).unwrap();
    assert_eq!(spec.archive_name, CPP_TOOLCHAIN_MACOS_ARM64_NAME);
    assert_eq!(spec.target_os, "macos");
    assert_eq!(spec.target_arch, "aarch64");
    assert_eq!(spec.format, ArchiveFormat::TarXz);
    assert_eq!(
        spec.url,
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.0/LLVM-22.1.0-macOS-ARM64.tar.xz"
    );
    assert_eq!(
        spec.sha256.as_str(),
        "cd5e615f4dab23d0239359cd343202c5f6ceeaf072c245a3c685d73afac09646"
    );
}

#[test]
fn rejects_unsupported_windows_x86_64() {
    let err = select_cpp_toolchain(&platform("windows", "x86_64")).unwrap_err();
    assert!(
        matches!(err, CppToolchainError::UnsupportedTarget { .. }),
        "{err:?}"
    );
}

#[test]
fn rejects_unsupported_linux_riscv64() {
    let err = select_cpp_toolchain(&platform("linux", "riscv64")).unwrap_err();
    assert!(
        matches!(err, CppToolchainError::UnsupportedTarget { .. }),
        "{err:?}"
    );
}

// ─── validate_llvm_layout ───────────────────────────────────────────────────

fn write_llvm_layout(root: &Path) {
    let bin = root.join("bin");
    let lib = root.join("lib");
    let include = root.join("include/clang");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir_all(&include).unwrap();
    fs::write(bin.join("clang"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::write(bin.join("llvm-config"), b"#!/bin/sh\nexit 0\n").unwrap();
}

fn ok_version_reader(_: &Path) -> Result<String, io::Error> {
    Ok("22.1.0\n".to_string())
}

#[test]
fn validate_layout_accepts_synthetic_llvm() {
    let dir = TempDir::new().unwrap();
    write_llvm_layout(dir.path());
    let paths = validate_llvm_layout(dir.path(), ok_version_reader).unwrap();
    assert_eq!(paths.root, dir.path());
    assert_eq!(paths.clang, dir.path().join("bin/clang"));
    assert_eq!(paths.llvm_config, dir.path().join("bin/llvm-config"));
    assert_eq!(paths.lib_dir, dir.path().join("lib"));
    assert_eq!(paths.include_dir, dir.path().join("include/clang"));
    assert_eq!(paths.version, "22.1.0");
}

#[test]
fn validate_layout_rejects_missing_clang() {
    let dir = TempDir::new().unwrap();
    write_llvm_layout(dir.path());
    fs::remove_file(dir.path().join("bin/clang")).unwrap();
    let err = validate_llvm_layout(dir.path(), ok_version_reader).unwrap_err();
    match err {
        CppToolchainError::MissingBinary { path } => {
            assert!(path.ends_with("bin/clang"), "path = {path}");
        }
        other => panic!("expected MissingBinary, got {other:?}"),
    }
}

#[test]
fn validate_layout_rejects_missing_llvm_config() {
    let dir = TempDir::new().unwrap();
    write_llvm_layout(dir.path());
    fs::remove_file(dir.path().join("bin/llvm-config")).unwrap();
    let err = validate_llvm_layout(dir.path(), ok_version_reader).unwrap_err();
    match err {
        CppToolchainError::MissingBinary { path } => {
            assert!(path.ends_with("bin/llvm-config"), "path = {path}");
        }
        other => panic!("expected MissingBinary, got {other:?}"),
    }
}

#[test]
fn validate_layout_rejects_wrong_version() {
    let dir = TempDir::new().unwrap();
    write_llvm_layout(dir.path());
    let reader = |_: &Path| Ok("22.0.0".to_string());
    let err = validate_llvm_layout(dir.path(), reader).unwrap_err();
    match err {
        CppToolchainError::VersionMismatch { expected, actual } => {
            assert_eq!(expected, "22.1.0");
            assert_eq!(actual, "22.0.0");
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

// ─── prepare_dependencies per-target filtering ──────────────────────────────
//
// The three helpers below (fixture server, tar builder, http policy, sha
// helper) mirror the ones used in `adapter_prepare.rs`. They are copied rather
// than extracted into a shared crate because integration tests cannot share
// private modules across files without extra plumbing.

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

fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut gz);
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
    gz.finish().expect("finish gz")
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

/// Return a manifest that lists all three C++ toolchain archives against the
/// same fixture URL/checksum. In production these would each point at a
/// different LLVM release; here we only need the per-target gate to pick one.
fn manifest_from_specs(url: String, sha: ContentDigest) -> DependencyManifest {
    let mut archives = Vec::new();
    for spec in all_cpp_toolchain_specs() {
        archives.push(ArchiveDependency {
            name: spec.archive_name,
            url: url.clone(),
            sha256: sha.clone(),
            // Serve a tar.gz fixture, matching the shared handler below.
            format: ArchiveFormat::TarGz,
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
fn prepare_dependencies_downloads_only_matching_llvm_archive() {
    let archive_bytes = build_tar_gz(&[("hello.txt", b"hello\n")]);
    let sha = sha256_hex(&archive_bytes);
    let server_bytes = archive_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(server_bytes.clone()).with_header(
            Header::from_bytes(b"Content-Type".as_ref(), b"application/gzip".as_ref()).unwrap(),
        ));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = manifest_from_specs(
        server.url("/llvm.tar.gz"),
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
            .join(format!("archives/{CPP_TOOLCHAIN_LINUX_X64_NAME}"))
            .is_dir()
    );
    assert!(
        !id_dir
            .join(format!("archives/{CPP_TOOLCHAIN_LINUX_ARM64_NAME}"))
            .exists()
    );
    assert!(
        !id_dir
            .join(format!("archives/{CPP_TOOLCHAIN_MACOS_ARM64_NAME}"))
            .exists()
    );

    // And only the linux/x86_64 archive is byte-hashed under downloads/.
    assert!(
        id_dir
            .join(format!("downloads/{CPP_TOOLCHAIN_LINUX_X64_NAME}.tar.gz"))
            .is_file()
    );
    assert!(
        !id_dir
            .join(format!("downloads/{CPP_TOOLCHAIN_LINUX_ARM64_NAME}.tar.gz"))
            .exists()
    );
    assert!(
        !id_dir
            .join(format!("downloads/{CPP_TOOLCHAIN_MACOS_ARM64_NAME}.tar.gz"))
            .exists()
    );

    // Manifest should list exactly the one prepared artifact.
    assert_eq!(set.manifest.artifacts.len(), 1);
    assert_eq!(set.manifest.artifacts[0].name, CPP_TOOLCHAIN_LINUX_X64_NAME);
}

#[test]
fn prepare_dependencies_rejects_wrong_llvm_digest() {
    let real_bytes = build_tar_gz(&[("hello.txt", b"actual\n")]);
    let advertised_bytes = build_tar_gz(&[("hello.txt", b"expected\n")]);
    let advertised_sha = sha256_hex(&advertised_bytes);

    let server_bytes = real_bytes.clone();
    let server = FixtureServer::start(move |request| {
        let _ = request.respond(Response::from_data(server_bytes.clone()));
    });

    let repo = TempDir::new().unwrap();
    let prepared_root = TempDir::new().unwrap();
    let manifest = manifest_from_specs(
        server.url("/llvm.tar.gz"),
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
