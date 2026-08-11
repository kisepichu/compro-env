//! Integration tests for the atomic site-data write repository (spec §12, §14).

use std::fs;
use std::path::Path;

use infrastructure::repository_impl::site_data_repository_impl::SiteDataRepositoryImpl;
use site_schema::{BuildMetadata, BuildMode, SITE_SCHEMA_VERSION, SiteData, SiteMetadata};
use tempfile::tempdir;
use usecases::repository::site_data_repository::SiteDataRepository;

fn sample_site_data(revision: &str) -> SiteData {
    SiteData {
        schema_version: SITE_SCHEMA_VERSION,
        build: BuildMetadata {
            schema_version: SITE_SCHEMA_VERSION,
            generated_at: "2026-08-11T12:00:00+00:00".into(),
            mode: BuildMode::Production,
            source_commit_sha: revision.into(),
            source_commit_short_sha: revision[..7.min(revision.len())].into(),
            source_committed_at: "2026-08-11T11:59:00+00:00".into(),
            uncommitted_changes: false,
            observed_toolchains: vec![],
            adapters: vec![],
        },
        site: SiteMetadata {
            title: "compro-env".into(),
            description: "d".into(),
            language: "en".into(),
            repository_url: None,
        },
        languages: vec![],
        libraries: vec![],
        solutions: vec![],
    }
}

fn read_site_data(dir: &Path) -> SiteData {
    let json = fs::read(dir.join("site-data.json")).expect("site-data.json missing");
    serde_json::from_slice(&json).expect("valid JSON")
}

#[test]
fn writes_site_data_json_atomically() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("ce-site-data");

    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("deadbeef00000000000000000000000000000000");
    repo.write_atomically(&output, &data).unwrap();

    assert!(output.is_dir(), "output directory created");
    assert!(output.join("site-data.json").is_file());
    let round_trip = read_site_data(&output);
    assert_eq!(
        round_trip.build.source_commit_sha,
        data.build.source_commit_sha
    );
}

#[test]
fn replaces_existing_output_directory() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("ce-site-data");

    let repo = SiteDataRepositoryImpl::new();
    let old = sample_site_data("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    repo.write_atomically(&output, &old).unwrap();

    // Add a stray file to prove the whole directory is replaced.
    fs::write(output.join("stray.txt"), b"gone").unwrap();

    let new = sample_site_data("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    repo.write_atomically(&output, &new).unwrap();

    let round_trip = read_site_data(&output);
    assert_eq!(
        round_trip.build.source_commit_sha,
        new.build.source_commit_sha
    );
    assert!(
        !output.join("stray.txt").exists(),
        "old stray file must not survive replacement"
    );
}

#[test]
fn creates_missing_parent_directories() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("nested").join("deeper").join("out");

    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("cccccccccccccccccccccccccccccccccccccccc");
    repo.write_atomically(&output, &data).unwrap();

    assert!(output.join("site-data.json").is_file());
}

#[test]
fn output_json_ends_with_newline_for_readability() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("out");
    let repo = SiteDataRepositoryImpl::new();
    let data = sample_site_data("dddddddddddddddddddddddddddddddddddddddd");
    repo.write_atomically(&output, &data).unwrap();

    let raw = fs::read(output.join("site-data.json")).unwrap();
    assert_eq!(*raw.last().unwrap(), b'\n');
}
