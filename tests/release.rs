//! What a release has to keep in step with the version in `Cargo.toml`: the
//! changelog's newest release heading and the README's status line. A
//! release edits them by hand, so these tests catch one being forgotten.
//! (`Cargo.lock` is covered by the `--locked` builds in CI, and the
//! version `slowrx-cli --version` prints by `tests/cli.rs`.)
//!
//! They read files from the repository, so they run from a checkout only.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The first `## [x.y.z] - date` heading in the changelog, after
/// `## [Unreleased]`, is the version being built, and it has a date.
#[test]
fn the_changelogs_newest_release_is_the_package_version() {
    let changelog = read("CHANGELOG.md");
    let (version, rest) = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## [")?.split_once(']'))
        .find(|(version, _)| *version != "Unreleased")
        .expect("CHANGELOG.md has a release heading");
    assert_eq!(version, VERSION, "newest release in CHANGELOG.md");
    let date = rest.strip_prefix(" - ").unwrap_or_default();
    assert!(
        date.len() == 10 && date.split('-').all(|part| part.parse::<u32>().is_ok()),
        "`## [{version}]` is followed by ` - YYYY-MM-DD`, not {rest:?}"
    );
}

/// The README's status line leads with the current version.
#[test]
fn the_readme_status_names_the_package_version() {
    let readme = read("README.md");
    assert!(
        readme.contains(&format!("**{VERSION}**")),
        "README.md names **{VERSION}** in its status line"
    );
}
