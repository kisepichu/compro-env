//! Build every declared adapter offline, handshake each one with the shared
//! protocol runner, and atomically publish the resulting build set
//! (spec §6.9, plan 042 Task 2).
//!
//! The driver never accepts fallbacks. It runs each language plan in a stable
//! order under a sanitized environment, refuses to continue if any handshake
//! disagrees with the declared adapter identity, and only after every language
//! succeeds does it rename the fully-populated `builds/<build-id>/` directory
//! into place and atomically swap the `bin` symlink. Failures leave the
//! `build-in-progress` marker so `ce site-data` / verify refuse to reuse the
//! old build set.
//!
//! Concurrency is enforced by a fail-fast OS advisory lock on
//! `<analyzer_root>/build.lock`. A crashed build leaves the marker but frees
//! the lock, so `build_state::inspect_build_state` can distinguish
//! `BuildRunning` from `PreviousBuildFailed`.
//!
//! Handshake requests are the ordinary empty `AnalysisRequest`; there is no
//! second handshake schema.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs as unix_fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use domain::adapter_build::{
    AdapterExecutableRecord, BuildManifest, ContentDigest, ExpectedBuild, TargetPlatform,
    ToolchainRecord, UnsignedBuildManifest,
};
use domain::adapter_prepare::PreparedSet;
use fs2::FileExt;
use library_adapter_protocol::{AdapterIdentity, AnalysisRequest, SCHEMA_VERSION};
use sha2::{Digest, Sha256};
use thiserror::Error;
use usecases::library_adapter::{AdapterRunError, LibraryAdapterRunner};

use super::build_state::{
    BUILD_BIN_SUBDIR, BUILD_IN_PROGRESS_MARKER, BUILD_LOCK_FILE, BUILD_MANIFEST_FILE,
    BUILDS_SUBDIR, BuildStateError, CURRENT_BIN_LINK, UsableBuildSet, derive_build_id,
    inspect_build_state, write_build_manifest_json,
};

/// Staging directory prefix under `<analyzer_root>/builds/`.
pub const STAGING_PREFIX: &str = "staging-";

/// Empty AnalysisRequest used for the handshake smoke test.
pub fn empty_handshake_request(language: &str) -> AnalysisRequest {
    AnalysisRequest {
        schema_version: SCHEMA_VERSION,
        repository_root: ".".into(),
        language: language.into(),
        libraries: vec![],
        solutions: vec![],
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum BuildError {
    #[error(transparent)]
    State(#[from] BuildStateError),
    #[error("failed to acquire build lock at {path}: another build is running")]
    LockContended { path: String },
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove {path}: {source}")]
    Cleanup {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("prepared set is missing at {path}")]
    PreparedSetMissing { path: String },
    #[error("build plan is missing for language {language:?}")]
    LanguagePlanMissing { language: String },
    #[error("duplicate language plan for {language:?}")]
    DuplicateLanguagePlan { language: String },
    #[error(
        "duplicate output file name {file_name:?} in build plans for \
         languages {first:?} and {second:?}"
    )]
    DuplicateFileName {
        file_name: String,
        first: String,
        second: String,
    },
    #[error("build command for {language:?} exited with {status}\nstderr tail:\n{stderr_tail}")]
    BuildCommandFailed {
        language: String,
        status: String,
        stderr_tail: String,
    },
    #[error("failed to spawn build command for {language:?}: {source}")]
    BuildCommandSpawn {
        language: String,
        #[source]
        source: std::io::Error,
    },
    #[error("build for {language:?} did not produce the executable at {path}")]
    OutputMissing { language: String, path: String },
    #[error("handshake for {language:?} failed: {source}")]
    HandshakeRun {
        language: String,
        #[source]
        source: AdapterRunError,
    },
    #[error(
        "handshake for {language:?} reported language {actual:?}; \
         expected {expected:?}"
    )]
    HandshakeLanguageMismatch {
        expected: String,
        actual: String,
        language: String,
    },
    #[error(
        "handshake for {language:?} reported adapter identity {actual_name:?} \
         @ {actual_version:?}; expected {expected_name:?} @ {expected_version:?}"
    )]
    HandshakeIdentityMismatch {
        language: String,
        expected_name: String,
        expected_version: String,
        actual_name: String,
        actual_version: String,
    },
    #[error("output executable {path} is not a regular file")]
    OutputNotRegularFile { path: String },
    #[error("output executable {path} is a symlink; symlinks are not permitted")]
    OutputSymlink { path: String },
}

// ─── BuildRequest ───────────────────────────────────────────────────────────

/// Instructions the driver needs to construct one build set.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub repository_root: PathBuf,
    /// Usually `<repo>/target/library-analyzers/`.
    pub analyzer_root: PathBuf,
    pub target_platform: TargetPlatform,
    pub build_profile: String,
    pub protocol_version: u32,
    pub input_digest: ContentDigest,
    pub git_commit_sha: String,
    /// Already-validated prepared set (from `prepare_dependencies`).
    pub prepared_set: PreparedSet,
    /// One plan per language, deduped by language and by output file name.
    /// The build driver sorts these by language ID before execution so the
    /// resulting `build-id` does not depend on caller iteration order.
    pub language_plans: Vec<LanguageBuildPlan>,
    /// Timeout per handshake.
    pub handshake_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct LanguageBuildPlan {
    /// Language ID this plan builds. Must be unique within `BuildRequest`.
    pub language: String,
    /// File name inside the build set's `bin/` directory. Must be unique.
    pub file_name: String,
    /// Declared adapter identity, cross-checked against the handshake response.
    pub expected_adapter_name: String,
    pub expected_adapter_version: String,
    /// Command line the driver runs to produce the executable. `argv[0]` is
    /// the program; subsequent entries are arguments. No shell is invoked.
    pub argv: Vec<String>,
    /// Sanitized environment for the build command.
    pub environment: BTreeMap<String, String>,
    /// Working directory for the build command. Defaults to the fresh staging
    /// directory when unset.
    pub working_directory: Option<PathBuf>,
    /// Where the build command is expected to place the executable, relative
    /// to `working_directory` (or the staging directory).
    pub output_relative_path: String,
    /// Sanitized environment forwarded to the handshake run.
    pub handshake_environment: BTreeMap<String, String>,
}

// ─── build_adapters ─────────────────────────────────────────────────────────

/// Build, handshake, and atomically publish one build set.
///
/// The caller supplies an already-validated `PreparedSet` and one
/// `LanguageBuildPlan` per language. The driver acquires the build lock,
/// creates the marker, stages each executable, verifies the handshake, moves
/// the finished bin/ into `<analyzer_root>/builds/<build-id>/`, writes the
/// manifest, and swaps the `bin` symlink under an atomic `rename`. Any error
/// leaves the marker in place so the analysis path refuses to run.
pub fn build_adapters(
    request: &BuildRequest,
    runner: &dyn LibraryAdapterRunner,
) -> Result<UsableBuildSet, BuildError> {
    // 1. Basic invariants: prepared set is present, no duplicate plans.
    if !request.prepared_set.root.is_dir() {
        return Err(BuildError::PreparedSetMissing {
            path: request.prepared_set.root.display().to_string(),
        });
    }
    ensure_plans_unique(&request.language_plans)?;

    // 2. Preconditions on the analyzer root.
    fs::create_dir_all(&request.analyzer_root).map_err(|source| BuildError::CreateDir {
        path: request.analyzer_root.display().to_string(),
        source,
    })?;
    let builds_dir = request.analyzer_root.join(BUILDS_SUBDIR);
    fs::create_dir_all(&builds_dir).map_err(|source| BuildError::CreateDir {
        path: builds_dir.display().to_string(),
        source,
    })?;

    // 3. Acquire the fail-fast build lock. Failure returns immediately.
    let lock_path = request.analyzer_root.join(BUILD_LOCK_FILE);
    let _lock = BuildLock::acquire(&lock_path)?;

    // 4. Create (or refresh) the in-progress marker BEFORE any build step so
    //    a crash between here and the atomic switch leaves the marker.
    let marker_path = request.analyzer_root.join(BUILD_IN_PROGRESS_MARKER);
    write_and_fsync(&marker_path, marker_body().as_bytes())?;

    // 5. Sort plans by language so the manifest and derived build-id do not
    //    depend on iteration order. `derive_build_id` also sorts, but sorting
    //    here means the build commands themselves run in a stable order.
    let mut plans = request.language_plans.clone();
    plans.sort_by(|a, b| a.language.cmp(&b.language));

    // 6. Unique staging directory holding the `bin/` for this attempt.
    let staging_dir = builds_dir.join(format!("{STAGING_PREFIX}{}", nonce()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|source| BuildError::Cleanup {
            path: staging_dir.display().to_string(),
            source,
        })?;
    }
    let staging_bin = staging_dir.join(BUILD_BIN_SUBDIR);
    fs::create_dir_all(&staging_bin).map_err(|source| BuildError::CreateDir {
        path: staging_bin.display().to_string(),
        source,
    })?;

    // 7. Execute each build plan, collect executable records.
    let mut records: Vec<AdapterExecutableRecord> = Vec::with_capacity(plans.len());
    for plan in &plans {
        let record = run_one_plan(plan, &staging_dir, &staging_bin, runner, request)?;
        records.push(record);
    }

    // 8. Build the unsigned manifest, derive the build-id, and move staging
    //    into `<builds>/<build-id>/`.
    let unsigned = UnsignedBuildManifest {
        input_digest: request.input_digest.clone(),
        target_platform: request.target_platform.clone(),
        build_profile: request.build_profile.clone(),
        protocol_version: request.protocol_version,
        executables: records.clone(),
    };
    let build_id = derive_build_id(&unsigned)?;

    let final_dir = builds_dir.join(build_id.as_str());
    if final_dir.exists() {
        // Spec §6.9: never delete an existing build set; the caller already
        // asked to (re)publish this exact id. Two possibilities here: the old
        // set is intact and safe to reuse, or it is inconsistent and we
        // should not clobber it. Fall through to validation via
        // `inspect_build_state`.
        drop_staging(&staging_dir);
        return Err(BuildError::Cleanup {
            path: final_dir.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "build directory already exists; run --check or clean it manually",
            ),
        });
    }

    let manifest = BuildManifest {
        input_digest: request.input_digest.clone(),
        target_platform: request.target_platform.clone(),
        build_profile: request.build_profile.clone(),
        protocol_version: request.protocol_version,
        git_commit_sha: request.git_commit_sha.clone(),
        executables: records,
    };
    let manifest_path = staging_dir.join(BUILD_MANIFEST_FILE);
    write_and_fsync(
        &manifest_path,
        write_build_manifest_json(&manifest).as_bytes(),
    )?;
    fsync_directory(&staging_dir)?;

    fs::rename(&staging_dir, &final_dir).map_err(|source| BuildError::Write {
        path: final_dir.display().to_string(),
        source,
    })?;
    fsync_directory(&builds_dir)?;

    // 9. Swap the `bin` symlink atomically via a `rename` on a temporary
    //    symlink under the analyzer root.
    swap_current_bin(&request.analyzer_root, build_id.as_str())?;

    // 10. Success. Remove the marker LAST so a crash between the switch and
    //     here also leaves an in-progress marker that a subsequent build
    //     inspect can recover.
    let _ = fs::remove_file(&marker_path);
    fsync_directory(&request.analyzer_root)?;

    // 11. Re-validate the published set with the same code path analysis will
    //     run before invoking adapters. This guarantees the driver never
    //     succeeds unless the resulting state is `UsableBuildSet`-valid.
    let expected = ExpectedBuild {
        input_digest: request.input_digest.clone(),
        target_platform: request.target_platform.clone(),
        build_profile: request.build_profile.clone(),
        protocol_version: request.protocol_version,
    };
    let set = inspect_build_state(&request.analyzer_root, &expected)?;
    Ok(set)
}

fn drop_staging(staging_dir: &Path) {
    let _ = fs::remove_dir_all(staging_dir);
}

fn ensure_plans_unique(plans: &[LanguageBuildPlan]) -> Result<(), BuildError> {
    let mut seen_langs: std::collections::BTreeSet<String> = Default::default();
    let mut seen_files: std::collections::BTreeMap<String, String> = Default::default();
    for p in plans {
        if !seen_langs.insert(p.language.clone()) {
            return Err(BuildError::DuplicateLanguagePlan {
                language: p.language.clone(),
            });
        }
        if let Some(first) = seen_files.insert(p.file_name.clone(), p.language.clone()) {
            return Err(BuildError::DuplicateFileName {
                file_name: p.file_name.clone(),
                first,
                second: p.language.clone(),
            });
        }
    }
    Ok(())
}

fn run_one_plan(
    plan: &LanguageBuildPlan,
    staging_dir: &Path,
    staging_bin: &Path,
    runner: &dyn LibraryAdapterRunner,
    request: &BuildRequest,
) -> Result<AdapterExecutableRecord, BuildError> {
    let program = plan
        .argv
        .first()
        .ok_or_else(|| BuildError::BuildCommandSpawn {
            language: plan.language.clone(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty argv"),
        })?;
    let cwd = plan
        .working_directory
        .clone()
        .unwrap_or_else(|| staging_dir.to_path_buf());
    fs::create_dir_all(&cwd).map_err(|source| BuildError::CreateDir {
        path: cwd.display().to_string(),
        source,
    })?;
    let mut cmd = Command::new(program);
    cmd.args(plan.argv.iter().skip(1));
    cmd.current_dir(&cwd);
    cmd.env_clear();
    for (k, v) in &plan.environment {
        cmd.env(k, v);
    }
    // Expose the staging bin path so plans can `cp` their output into it
    // without hard-coding the analyzer root layout.
    cmd.env("CE_ADAPTER_STAGE_BIN", staging_bin);
    cmd.env("CE_ADAPTER_STAGE_DIR", staging_dir);
    cmd.env("CE_ADAPTER_PREPARED_ROOT", &request.prepared_set.root);
    cmd.env("CE_ADAPTER_REPOSITORY_ROOT", &request.repository_root);

    let output = cmd
        .output()
        .map_err(|source| BuildError::BuildCommandSpawn {
            language: plan.language.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(BuildError::BuildCommandFailed {
            language: plan.language.clone(),
            status: format_exit_status(&output.status),
            stderr_tail: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    // Locate the built executable relative to the plan's working directory.
    let output_path = cwd.join(&plan.output_relative_path);
    let meta = match fs::symlink_metadata(&output_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(BuildError::OutputMissing {
                language: plan.language.clone(),
                path: output_path.display().to_string(),
            });
        }
        Err(source) => {
            return Err(BuildError::Write {
                path: output_path.display().to_string(),
                source,
            });
        }
    };
    if meta.file_type().is_symlink() {
        return Err(BuildError::OutputSymlink {
            path: output_path.display().to_string(),
        });
    }
    if !meta.file_type().is_file() {
        return Err(BuildError::OutputNotRegularFile {
            path: output_path.display().to_string(),
        });
    }

    // Copy into `<staging>/bin/<file_name>` and ensure it is executable.
    let staged = staging_bin.join(&plan.file_name);
    if output_path != staged {
        fs::copy(&output_path, &staged).map_err(|source| BuildError::Write {
            path: staged.display().to_string(),
            source,
        })?;
    }
    let mut perms = fs::metadata(&staged)
        .map_err(|source| BuildError::Write {
            path: staged.display().to_string(),
            source,
        })?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&staged, perms).map_err(|source| BuildError::Write {
        path: staged.display().to_string(),
        source,
    })?;

    let sha256 = stream_sha256(&staged).map_err(|source| BuildError::Write {
        path: staged.display().to_string(),
        source,
    })?;

    let identity = handshake_adapter(runner, &staged, &plan.language, request.handshake_timeout)?;
    if identity.name != plan.expected_adapter_name
        || identity.version != plan.expected_adapter_version
    {
        return Err(BuildError::HandshakeIdentityMismatch {
            language: plan.language.clone(),
            expected_name: plan.expected_adapter_name.clone(),
            expected_version: plan.expected_adapter_version.clone(),
            actual_name: identity.name,
            actual_version: identity.version,
        });
    }

    let record = AdapterExecutableRecord {
        language: plan.language.clone(),
        file_name: plan.file_name.clone(),
        sha256,
        adapter_name: identity.name,
        adapter_version: identity.version,
        toolchains: identity
            .toolchains
            .into_iter()
            .map(|t| ToolchainRecord {
                name: t.name,
                version: t.version,
                target: t.target,
            })
            .collect(),
    };
    Ok(record)
}

// ─── handshake_adapter ──────────────────────────────────────────────────────

/// Run one adapter with the empty analysis request and return its identity.
///
/// This is the same code path as ordinary analysis; there is no separate
/// handshake protocol. The caller is responsible for supplying a `runner`
/// configured with a sanitized environment.
pub fn handshake_adapter(
    runner: &dyn LibraryAdapterRunner,
    executable: &Path,
    language: &str,
    timeout: Duration,
) -> Result<AdapterIdentity, BuildError> {
    let request = empty_handshake_request(language);
    let response = runner
        .analyze(executable, &request, timeout)
        .map_err(|source| BuildError::HandshakeRun {
            language: language.to_string(),
            source,
        })?;
    // Response schema/version is enforced by the runner; we only assert that
    // the handshake did not emit libraries or solutions and that the reported
    // language matches. (The response type does not carry `language` — that
    // is validated by the runner's own schema check.)
    if !response.libraries.is_empty() || !response.solutions.is_empty() {
        return Err(BuildError::HandshakeLanguageMismatch {
            language: language.to_string(),
            expected: "empty analysis".into(),
            actual: format!(
                "{} libraries, {} solutions",
                response.libraries.len(),
                response.solutions.len()
            ),
        });
    }
    Ok(response.adapter)
}

// ─── lock ───────────────────────────────────────────────────────────────────

/// RAII guard around the analyzer-root advisory build lock.
#[derive(Debug)]
pub struct BuildLock {
    file: File,
}

impl BuildLock {
    pub fn acquire(path: &Path) -> Result<Self, BuildError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BuildError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let file = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|source| BuildError::CreateDir {
                path: path.display().to_string(),
                source,
            })?;
        file.try_lock_exclusive()
            .map_err(|_| BuildError::LockContended {
                path: path.display().to_string(),
            })?;
        Ok(Self { file })
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn nonce() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_else(|_| Instant::now().elapsed().as_nanos());
    format!("{}-{}", std::process::id(), ns)
}

fn marker_body() -> String {
    format!(
        "pid={} started_ns={}\n",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

fn write_and_fsync(path: &Path, bytes: &[u8]) -> Result<(), BuildError> {
    let mut file = File::create(path).map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.flush().map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_all().map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn fsync_directory(path: &Path) -> Result<(), BuildError> {
    let file = File::open(path).map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_all().map_err(|source| BuildError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn stream_sha256(path: &Path) -> Result<ContentDigest, std::io::Error> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let out: [u8; 32] = hasher.finalize().into();
    Ok(ContentDigest::from_sha256_bytes(out))
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        format!("code {code}")
    } else {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
        "unknown".into()
    }
}

/// Atomically replace `<analyzer_root>/bin` with a symlink to
/// `builds/<build_id>/bin` via `rename` on a sibling temporary symlink.
fn swap_current_bin(analyzer_root: &Path, build_id: &str) -> Result<(), BuildError> {
    let bin_link = analyzer_root.join(CURRENT_BIN_LINK);
    let temp_link = analyzer_root.join(format!("{CURRENT_BIN_LINK}.new-{}", nonce()));
    let target = PathBuf::from(BUILDS_SUBDIR)
        .join(build_id)
        .join(BUILD_BIN_SUBDIR);
    if let Ok(_meta) = fs::symlink_metadata(&temp_link) {
        fs::remove_file(&temp_link).map_err(|source| BuildError::Cleanup {
            path: temp_link.display().to_string(),
            source,
        })?;
    }
    unix_fs::symlink(&target, &temp_link).map_err(|source| BuildError::Write {
        path: temp_link.display().to_string(),
        source,
    })?;
    fs::rename(&temp_link, &bin_link).map_err(|source| BuildError::Write {
        path: bin_link.display().to_string(),
        source,
    })?;
    Ok(())
}
