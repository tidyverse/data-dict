//! Integration tests for the `validate` entry point.
//!
//! Each test points at a YAML fixture under `tests/fixtures/{valid,invalid}/`.
//! The fixtures double as runnable inputs for the CLI:
//!
//!     cargo run -p data-dict-cli -- validate \
//!         crates/data-dict/tests/fixtures/invalid/enum-without-values.yaml
//!
//! When adding a new rule, prefer adding a fixture file (with a one-line
//! `# expected: ...` header) and a one-line test here over inline YAML.

use std::path::PathBuf;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn assert_valid(path: PathBuf) {
    if let Err(e) = data_dict::validate(&path) {
        panic!("expected {} to validate, but:\n{e}", path.display());
    }
}

fn assert_invalid(path: PathBuf) {
    assert!(
        data_dict::validate(&path).is_err(),
        "expected {} to fail validation, but it passed",
        path.display(),
    );
}

// --- valid fixtures ------------------------------------------------------

#[test]
fn minimal() {
    assert_valid(fixture("valid/minimal.yaml"));
}

#[test]
fn example_foodbank() {
    assert_valid(workspace_root().join("examples/foodbank.yaml"));
}

#[test]
fn example_otters() {
    assert_valid(workspace_root().join("examples/otters.yaml"));
}

#[test]
fn example_elevators() {
    assert_valid(workspace_root().join("examples/elevators.yaml"));
}

// --- invalid fixtures ----------------------------------------------------

#[test]
fn missing_version() {
    assert_invalid(fixture("invalid/missing-version.yaml"));
}

#[test]
fn unknown_top_level_key() {
    assert_invalid(fixture("invalid/unknown-top-level-key.yaml"));
}

#[test]
fn enum_without_values() {
    assert_invalid(fixture("invalid/enum-without-values.yaml"));
}

#[test]
fn range_on_string_type() {
    assert_invalid(fixture("invalid/range-on-string-type.yaml"));
}

#[test]
fn bad_cardinality() {
    assert_invalid(fixture("invalid/bad-cardinality.yaml"));
}

#[test]
fn non_string_glossary_value() {
    assert_invalid(fixture("invalid/non-string-glossary-value.yaml"));
}
