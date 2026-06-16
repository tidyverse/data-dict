//! Integration tests for the `validate` entry point.
//!
//! Each test points at a YAML fixture under `tests/fixtures/{valid,invalid}/`.
//! The fixtures double as runnable inputs for the CLI:
//!
//!     cargo run -p data-dict-cli -- validate-schema \
//!         crates/data-dict/tests/fixtures/lint/dd007-enum-without-values.yaml
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

fn assert_invalid(path: PathBuf, expected: &[&str]) {
    let err = data_dict::validate(&path)
        .err()
        .unwrap_or_else(|| panic!("expected {} to fail validation, but it passed", path.display()));
    let text = err.to_string();
    for s in expected {
        assert!(
            text.contains(s),
            "expected {:?} in diagnostic for {}, got:\n{text}",
            s,
            path.display(),
        );
    }
}

/// Validate a fixture that must fail, returning the rendered diagnostic with
/// machine-specific noise stripped so it can be snapshotted. Used for both
/// schema-`invalid/` and `lint/` fixtures — any document expected to error.
///
/// The diagnostic carries two unstable bits: terminal styling (ANSI color
/// escapes and OSC-8 hyperlinks, the latter embedding an absolute `file://`
/// URL) and the absolute on-disk path of the fixture. We strip the escapes and
/// rewrite the path to its `tests/fixtures/`-relative form.
fn failing_diagnostic(rel: &str) -> String {
    let path = fixture(rel);
    let diagnostic = match data_dict::validate(&path) {
        Ok(()) => panic!("expected {rel} to fail validation, but it passed"),
        Err(e) => e.to_string(),
    };
    let fixtures_root = format!(
        "{}/",
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .display()
    )
    .replace('\\', "/");
    strip_terminal_escapes(&diagnostic)
        .replace('\\', "/")
        .replace(&fixtures_root, "")
}

/// Remove ANSI SGR sequences (`ESC [ ... m`) and OSC-8 hyperlink wrappers
/// (`ESC ] 8 ; ; ... BEL|ST`) while leaving the visible text intact.
fn strip_terminal_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    // CSI: run until a final byte in 0x40..=0x7e.
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1; // consume the final byte
                }
                b']' => {
                    // OSC: run until BEL or ST (ESC \).
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 2,
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("stripping ASCII escapes preserves UTF-8")
}

// --- valid fixtures ------------------------------------------------------

#[test]
fn minimal() {
    assert_valid(fixture("valid/minimal.yaml"));
}

// --- bundled examples ----------------------------------------------------
//
// The bundled examples under `site/examples/` are downloaded and refreshed by
// the `update-examples` workflow, which only commits files that pass
// validation. They must therefore validate cleanly here too.

#[test]
fn examples_validate() {
    let dir = workspace_root().join("site/examples");
    for entry in std::fs::read_dir(&dir).expect("read site/examples") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            assert_valid(path);
        }
    }
}

// --- invalid fixtures ----------------------------------------------------

// Each invalid fixture is tested at two levels:
//
// 1. A snapshot test (Unix only) that guards the exact rendered diagnostic,
//    including formatting. Gated to Unix because the upstream renderer measures
//    Unicode box-drawing characters differently on Windows, shifting pointer
//    arrows by one column.  Regenerate after intentional message changes with:
//
//        INSTA_UPDATE=always cargo test -p data-dict
//
// 2. A cross-platform test that verifies the right error is reported on all
//    platforms by checking for key phrases in the diagnostic text.

#[test]
#[cfg(unix)]
fn missing_version() {
    insta::assert_snapshot!(failing_diagnostic("invalid/missing-version.yaml"));
}

#[test]
fn missing_version_errors() {
    assert_invalid(
        fixture("invalid/missing-version.yaml"),
        &["Missing required property 'version'"],
    );
}

#[test]
#[cfg(unix)]
fn unknown_top_level_key() {
    insta::assert_snapshot!(failing_diagnostic("invalid/unknown-top-level-key.yaml"));
}

#[test]
fn unknown_top_level_key_errors() {
    assert_invalid(
        fixture("invalid/unknown-top-level-key.yaml"),
        &["Unknown property 'bogus'"],
    );
}

#[test]
#[cfg(unix)]
fn bad_cardinality() {
    insta::assert_snapshot!(failing_diagnostic("invalid/bad-cardinality.yaml"));
}

#[test]
fn bad_cardinality_errors() {
    assert_invalid(
        fixture("invalid/bad-cardinality.yaml"),
        &["many-to-many"],
    );
}

#[test]
#[cfg(unix)]
fn non_string_glossary_value() {
    insta::assert_snapshot!(failing_diagnostic("invalid/non-string-glossary-value.yaml"));
}

#[test]
fn non_string_glossary_value_errors() {
    assert_invalid(
        fixture("invalid/non-string-glossary-value.yaml"),
        &["Expected string"],
    );
}

// --- lint fixtures -------------------------------------------------------

#[test]
fn lint_clean_two_tables() {
    assert_valid(fixture("lint/clean-two-tables.yaml"));
}

// Each local lint fixture snapshots its full rendered diagnostic. Snapshotting
// the whole output (rather than asserting a single code is present) guards the
// exact set of findings — e.g. that `dd003-missing-column` reports the missing
// column without *also* checking cardinality against it and emitting a
// redundant DD006.

#[test]
fn lint_dd001_fk_no_relationship() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd001-fk-no-relationship.yaml"));
}

#[test]
fn lint_dd002_missing_table() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd002-missing-table.yaml"));
}

#[test]
fn lint_dd003_missing_column() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd003-missing-column.yaml"));
}

#[test]
fn lint_dd004_bad_join() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd004-bad-join.yaml"));
}

#[test]
fn lint_dd005_conflicts_not_on_both_sides() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd005-conflicts-not-on-both-sides.yaml"));
}

// The opposite of the above: `amount` is genuinely a column on both tables (a
// real conflict) but is not declared in `conflicts`. DD005 only checks declared
// entries, so this must lint clean rather than demanding the conflict be named.
#[test]
fn lint_dd005_undeclared_conflict_ok() {
    assert_valid(fixture("lint/dd005-undeclared-conflict-ok.yaml"));
}

#[test]
fn lint_dd006_cardinality_mismatch() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd006-cardinality-mismatch.yaml"));
}

// Recreated from the bundled `otters` example: a one-to-many self-join whose
// "one" side is not unique (DD006), alongside a string column missing
// `examples` (DD007). Guards that both findings surface together.
#[test]
fn lint_dd006_self_join_one_to_many() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd006-self-join-one-to-many.yaml"));
}

#[test]
fn lint_dd007_enum_without_values() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd007-enum-without-values.yaml"));
}

#[test]
fn lint_dd007_range_type_missing_range() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd007-range-type-missing-range.yaml"));
}

#[test]
fn lint_dd007_other_type_missing_examples() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd007-other-type-missing-examples.yaml"));
}

// A `boolean` column carries no data representation key, so it must lint clean
// without `examples` — the one non-enum/range type exempt from DD007's
// missing-`examples` check.
#[test]
fn lint_dd007_boolean_no_examples_ok() {
    assert_valid(fixture("lint/dd007-boolean-no-examples-ok.yaml"));
}

#[test]
fn lint_dd007_wrong_rep_on_enum() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd007-wrong-rep-on-enum.yaml"));
}

#[test]
fn lint_dd008_range_on_string_type() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd008-range-on-string-type.yaml"));
}

// `units` is valid only on `number(quantity)`. A quantity column with units
// lints clean; units on any other type is DD008.
#[test]
fn lint_dd008_units_ok_on_quantity() {
    assert_valid(fixture("lint/dd008-units-on-quantity-ok.yaml"));
}

#[test]
fn lint_dd008_units_on_non_quantity() {
    insta::assert_snapshot!(failing_diagnostic("lint/dd008-units-on-non-quantity.yaml"));
}
