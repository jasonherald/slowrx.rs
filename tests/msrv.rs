//! The minimum supported Rust version and the install command are written
//! down in more than one place. `Cargo.toml` is the source for the first,
//! and these tests keep the rest agreeing with it: the CI job that checks
//! the MSRV, the README, and every documented `cargo install` (which must
//! use `--locked`, so users build with the dependency versions a release
//! was tested and audited with).
//!
//! They read files from the repository, so they run from a checkout only.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// `rust-version` from `Cargo.toml`, for example `1.88`.
fn declared_msrv() -> String {
    read(&root().join("Cargo.toml"))
        .lines()
        .find_map(|line| line.strip_prefix("rust-version = \"")?.strip_suffix('"'))
        .expect("Cargo.toml declares rust-version")
        .to_owned()
}

/// Whether `line` starts the next job in a workflow: it is indented by
/// exactly two spaces.
fn is_job_key(line: &str) -> bool {
    line.starts_with("  ") && !line.starts_with("   ")
}

/// The lines of the `msrv` job in the CI workflow: from its key to the next
/// job's.
fn ci_msrv_job() -> String {
    let ci = read(&root().join(".github/workflows/ci.yml"));
    let job: Vec<&str> = ci
        .lines()
        .skip_while(|line| *line != "  msrv:")
        .skip(1)
        .take_while(|line| !is_job_key(line))
        .collect();
    assert!(!job.is_empty(), "ci.yml has an `msrv` job");
    job.join("\n")
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("list {}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The CI job that guards the MSRV has to check the version `Cargo.toml`
/// declares, against the committed lockfile.
#[test]
fn the_ci_msrv_job_checks_the_declared_rust_version_with_the_lockfile() {
    let msrv = declared_msrv();
    let job = ci_msrv_job();
    assert!(
        job.contains(&format!("toolchain: \"{msrv}\"")),
        "the msrv job installs Rust {msrv}:\n{job}"
    );
    assert!(
        job.contains(&format!("name: MSRV check ({msrv})")),
        "the msrv job is named after Rust {msrv}:\n{job}"
    );
    assert!(
        job.lines()
            .any(|line| line.contains("cargo check") && line.contains("--locked")),
        "the msrv job runs `cargo check --locked`:\n{job}"
    );
}

/// The README tells users which Rust they need.
#[test]
fn the_readme_states_the_declared_rust_version() {
    let msrv = declared_msrv();
    let readme = read(&root().join("README.md"));
    assert!(
        readme.contains(&format!("Requires Rust {msrv} or newer")),
        "README.md says `Requires Rust {msrv} or newer`"
    );
}

/// Every install command in the README, the sources and the examples uses
/// the lockfile.
#[test]
fn every_documented_cargo_install_uses_the_lockfile() {
    let mut files = vec![root().join("README.md")];
    rust_files(&root().join("src"), &mut files);
    rust_files(&root().join("examples"), &mut files);
    let mut commands = 0;
    for file in &files {
        for (number, line) in read(file).lines().enumerate() {
            if line.contains("cargo install") {
                commands += 1;
                assert!(
                    line.contains("--locked"),
                    "{}:{}: {line}",
                    file.display(),
                    number + 1
                );
            }
        }
    }
    assert!(commands > 0, "no `cargo install` command found to check");
}
