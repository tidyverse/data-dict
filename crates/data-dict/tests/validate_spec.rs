//! Integration tests for the `validate` entry point.
//!
//! Each test points at a YAML fixture under `tests/fixtures/{valid,invalid}/`.
//! The fixtures double as runnable inputs for the CLI:
//!
//!     cargo run -p data-dict-cli -- validate-spec \
//!         crates/data-dict/tests/fixtures/spec/s07-enum-without-values.yaml
//!
//! When adding a new rule, prefer adding a fixture file (with a one-line
//! `# expected: ...` header) and a one-line test here over inline YAML.

use std::path::{Path, PathBuf};

mod common;

use data_dict::Severity;
use indoc::indoc;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

/// Render the problems of the given `severity` for a fixture, in source order.
/// Pre-flight failures (I/O, unparseable YAML, structural schema errors) are
/// error-severity problems like any other, so they surface here when collecting
/// errors and are skipped when collecting warnings.
fn diagnostics(path: &Path, severity: Severity) -> Vec<String> {
    let problems = data_dict::validate_spec(path);
    problems
        .items
        .iter()
        .filter(|p| p.severity == severity)
        .map(|p| p.to_text(&problems.source))
        .collect()
}

fn assert_valid(path: PathBuf) {
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors.is_empty(),
        "expected {} to validate, but:\n{}",
        path.display(),
        errors.join("\n"),
    );
}

fn assert_invalid(path: PathBuf, expected: &[&str]) {
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        !errors.is_empty(),
        "expected {} to fail validation, but it passed",
        path.display()
    );
    let text = errors.join("\n");
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
/// schema-`invalid/` and `schema/` fixtures — any document expected to error.
///
/// The diagnostic carries two unstable bits: terminal styling (ANSI color
/// escapes and OSC-8 hyperlinks, the latter embedding an absolute `file://`
/// URL) and the absolute on-disk path of the fixture. We strip the escapes and
/// rewrite the path to its `tests/fixtures/`-relative form.
fn failing_diagnostic(rel: &str) -> String {
    let errors = diagnostics(&fixture(rel), Severity::Error);
    if errors.is_empty() {
        panic!("expected {rel} to fail validation, but it passed");
    }
    sanitize(&errors.join("\n"))
}

/// Validate a fixture expected to pass *with* warnings, returning the rendered
/// warnings (sanitized like [`failing_diagnostic`]) for snapshotting.
fn warning_diagnostic(rel: &str) -> String {
    let path = fixture(rel);
    assert!(
        diagnostics(&path, Severity::Error).is_empty(),
        "expected {rel} to validate, but it failed",
    );
    let warnings = diagnostics(&path, Severity::Warning);
    if warnings.is_empty() {
        panic!("expected {rel} to emit a warning, but it was clean");
    }
    sanitize(&warnings.join("\n"))
}

/// Strip the two unstable bits from a rendered diagnostic so it can be
/// snapshotted: terminal styling (ANSI color escapes and OSC-8 hyperlinks, the
/// latter embedding an absolute `file://` URL) and the absolute on-disk path of
/// the fixture, which is rewritten to its `tests/fixtures/`-relative form.
fn sanitize(diagnostic: &str) -> String {
    let fixtures_root = format!(
        "{}/",
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .display()
    )
    .replace('\\', "/");
    strip_terminal_escapes(diagnostic)
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
    let path = fixture("valid/minimal.yaml");
    assert!(
        diagnostics(&path, Severity::Error).is_empty(),
        "minimal must validate"
    );
    let warnings = diagnostics(&path, Severity::Warning);
    assert!(
        warnings.is_empty(),
        "minimal carries `$learn_more`, so it must validate without warnings, got: {warnings:?}"
    );
}

// A column with only a `name` and no `type` is acknowledged but not described,
// so it is exempt from the S07 data-representation requirement.
#[test]
fn typeless_column_needs_no_representation() {
    let path = fixture("valid/typeless-column.yaml");
    assert!(
        diagnostics(&path, Severity::Error).is_empty(),
        "a column with no `type` must not trigger S07"
    );
}

// --- warnings ------------------------------------------------------------

// A document missing the recommended `$learn_more` key validates (it is not an
// error) but surfaces a S09 warning.

#[test]
#[cfg(unix)]
fn warn_missing_learn_more() {
    insta::assert_snapshot!(warning_diagnostic("valid/no-learn-more.yaml"));
}

#[test]
fn warn_missing_learn_more_text() {
    let warnings = diagnostics(&fixture("valid/no-learn-more.yaml"), Severity::Warning);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("S09") && w.contains("$learn_more")),
        "expected a S09 `$learn_more` warning, got: {warnings:?}"
    );
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
        &["Missing required property '$version'"],
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
    assert_invalid(fixture("invalid/bad-cardinality.yaml"), &["many-to-many"]);
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

// --- schema-check fixtures -------------------------------------------------------

#[test]
fn clean_two_tables() {
    assert_valid(fixture("spec/clean-two-tables.yaml"));
}

// Each local schema-check fixture snapshots its full rendered diagnostic. Snapshotting
// the whole output (rather than asserting a single code is present) guards the
// exact set of findings — e.g. that `s03-missing-column` reports the missing
// column without *also* checking cardinality against it and emitting a
// redundant S06.

#[test]
fn s01_fk_no_relationship() {
    insta::assert_snapshot!(failing_diagnostic("spec/s01-fk-no-relationship.yaml"));
}

#[test]
fn s02_missing_table() {
    insta::assert_snapshot!(failing_diagnostic("spec/s02-missing-table.yaml"));
}

#[test]
fn s03_missing_column() {
    insta::assert_snapshot!(failing_diagnostic("spec/s03-missing-column.yaml"));
}

#[test]
fn s04_bad_join() {
    insta::assert_snapshot!(failing_diagnostic("spec/s04-bad-join.yaml"));
}

#[test]
fn s05_conflicts_not_on_both_sides() {
    insta::assert_snapshot!(failing_diagnostic(
        "spec/s05-conflicts-not-on-both-sides.yaml"
    ));
}

// The opposite of the above: `amount` is genuinely a column on both tables (a
// real conflict) but is not declared in `conflicts`. S05 only checks declared
// entries, so this must validate cleanly rather than demanding the conflict be named.
#[test]
fn s05_undeclared_conflict_ok() {
    assert_valid(fixture("spec/s05-undeclared-conflict-ok.yaml"));
}

#[test]
fn s06_cardinality_mismatch() {
    insta::assert_snapshot!(failing_diagnostic("spec/s06-cardinality-mismatch.yaml"));
}

// Recreated from the bundled `otters` example: a one-to-many self-join whose
// "one" side is not unique. Exercises the self-join orientation of S06.
#[test]
fn s06_self_join_one_to_many() {
    insta::assert_snapshot!(failing_diagnostic("spec/s06-self-join-one-to-many.yaml"));
}

#[test]
fn s07_enum_without_values() {
    insta::assert_snapshot!(failing_diagnostic("spec/s07-enum-without-values.yaml"));
}

#[test]
fn s07_range_type_missing_range() {
    insta::assert_snapshot!(failing_diagnostic("spec/s07-range-type-missing-range.yaml"));
}

#[test]
fn s07_other_type_missing_examples() {
    insta::assert_snapshot!(failing_diagnostic(
        "spec/s07-other-type-missing-examples.yaml"
    ));
}

// A `boolean` column carries no data representation key, so it must validate cleanly
// without `examples` — the one non-enum/range type exempt from S07's
// missing-`examples` check.
#[test]
fn s07_boolean_no_examples_ok() {
    assert_valid(fixture("spec/s07-boolean-no-examples-ok.yaml"));
}

#[test]
fn s07_wrong_rep_on_enum() {
    insta::assert_snapshot!(failing_diagnostic("spec/s07-wrong-rep-on-enum.yaml"));
}

#[test]
fn s07_range_on_string_type() {
    insta::assert_snapshot!(failing_diagnostic("spec/s07-range-on-string-type.yaml"));
}

// `units` is valid only on `number(quantity)`. A quantity column with units
// validates cleanly; units on any other type is S08.
#[test]
fn s08_units_ok_on_quantity() {
    assert_valid(fixture("spec/s08-units-on-quantity-ok.yaml"));
}

#[test]
fn s08_units_on_non_quantity() {
    insta::assert_snapshot!(failing_diagnostic("spec/s08-units-on-non-quantity.yaml"));
}

#[test]
#[cfg(unix)]
fn s10_duplicate_column_name() {
    insta::assert_snapshot!(failing_diagnostic("spec/s10-duplicate-column-name.yaml"));
}

#[test]
fn s10_duplicate_column_name_errors() {
    assert_invalid(
        fixture("spec/s10-duplicate-column-name.yaml"),
        &[
            "S10",
            "Column names must be unique",
            "appears more than once",
        ],
    );
}

#[test]
#[cfg(unix)]
fn s11_empty_table_name() {
    insta::assert_snapshot!(failing_diagnostic("spec/s11-empty-table-name.yaml"));
}

#[test]
fn s11_empty_table_name_errors() {
    assert_invalid(
        fixture("spec/s11-empty-table-name.yaml"),
        &["S11", "table name is empty"],
    );
}

#[test]
#[cfg(unix)]
fn s11_empty_column_name() {
    insta::assert_snapshot!(failing_diagnostic("spec/s11-empty-column-name.yaml"));
}

#[test]
fn s11_empty_column_name_errors() {
    assert_invalid(
        fixture("spec/s11-empty-column-name.yaml"),
        &["S11", "the `name` is empty"],
    );
}

#[test]
#[cfg(unix)]
fn s07_examples_on_boolean() {
    insta::assert_snapshot!(failing_diagnostic("spec/s07-examples-on-boolean.yaml"));
}

#[test]
fn s07_examples_on_boolean_errors() {
    assert_invalid(
        fixture("spec/s07-examples-on-boolean.yaml"),
        &["S07", "type `boolean`", "examples"],
    );
}

#[test]
#[cfg(unix)]
fn s12_wrong_value_type() {
    insta::assert_snapshot!(failing_diagnostic("spec/s12-wrong-value-type.yaml"));
}

#[test]
fn s12_wrong_value_type_errors() {
    assert_invalid(
        fixture("spec/s12-wrong-value-type.yaml"),
        &["S12", "must be a number"],
    );
}

#[test]
#[cfg(unix)]
fn s12_date_not_iso() {
    insta::assert_snapshot!(failing_diagnostic("spec/s12-date-not-iso.yaml"));
}

#[test]
fn s12_date_not_iso_errors() {
    assert_invalid(
        fixture("spec/s12-date-not-iso.yaml"),
        &["S12", "ISO 8601 date"],
    );
}

#[test]
fn s12_datetime_requires_timezone_errors() {
    assert_invalid(
        fixture("spec/s12-datetime-no-timezone.yaml"),
        &["S12", "timezone"],
    );
}

#[test]
#[cfg(unix)]
fn s13_descending_range() {
    insta::assert_snapshot!(failing_diagnostic("spec/s13-descending-range.yaml"));
}

#[test]
fn s13_descending_range_errors() {
    assert_invalid(
        fixture("spec/s13-descending-range.yaml"),
        &["S13", "is greater than the maximum"],
    );
}

// Guards that valid representation values and ascending ranges across every
// type — including quoted numeric-looking strings and a boolean with no
// representation key — produce no S07/S12/S13 noise.
#[test]
fn s12_s13_valid_ok() {
    assert_valid(fixture("spec/s12-s13-valid-ok.yaml"));
}

// --- version (S14) -------------------------------------------------------
//
// The optional top-level `version` has a small, self-contained grammar, so its
// tests use inline YAML (written to a temp file) rather than fixture files.

/// Write inline `yaml` to a temp file and return its path, for the path-based
/// `assert_valid` / `assert_invalid` helpers.
fn inline(yaml: &str) -> PathBuf {
    common::write_yaml(&common::temp_dir(), yaml)
}

/// Validate inline `yaml` expected to fail, returning its rendered errors with
/// the temp path rewritten to the bare `dict.yaml` so they can be snapshotted.
fn failing_inline(yaml: &str) -> String {
    let path = inline(yaml);
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        !errors.is_empty(),
        "expected inline document to fail validation, but it passed"
    );
    common::sanitize(&errors.join("\n"), path.parent().unwrap())
}

// The three valid forms of the optional top-level `version`: a date, a
// (quoted) version number, and an opaque hash.
#[test]
fn version_date_ok() {
    assert_valid(inline(indoc! {"
        $version: 0.1.0
        $learn_more: http://data-dict.tidyverse.org/
        version:
          date: 2024-01-31
    "}));
}

#[test]
fn version_number_ok() {
    // Quoted so its exact text (1.10, not 1.1) survives YAML parsing.
    assert_valid(inline(indoc! {r#"
        $version: 0.1.0
        $learn_more: http://data-dict.tidyverse.org/
        version:
          number: "1.10.0"
    "#}));
}

#[test]
fn version_hash_ok() {
    assert_valid(inline(indoc! {"
        $version: 0.1.0
        $learn_more: http://data-dict.tidyverse.org/
        version:
          hash: a1b2c3d
    "}));
}

#[test]
#[cfg(unix)]
fn s14_multiple_keys() {
    let rendered = failing_inline(indoc! {"
        $version: 0.1.0
        $learn_more: http://data-dict.tidyverse.org/
        version:
          date: 2024-01-31
          hash: a1b2c3d
    "});
    insta::assert_snapshot!(rendered);
}

#[test]
fn s14_multiple_keys_errors() {
    assert_invalid(
        inline(indoc! {"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            version:
              date: 2024-01-31
              hash: a1b2c3d
        "}),
        &["S14", "exactly one", "`date` has already been supplied"],
    );
}

#[test]
fn s14_empty_errors() {
    assert_invalid(
        inline(indoc! {"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            version: {}
        "}),
        &["S14", "exactly one", "names none"],
    );
}

#[test]
#[cfg(unix)]
fn s14_date_not_iso() {
    let rendered = failing_inline(indoc! {r#"
        $version: 0.1.0
        $learn_more: http://data-dict.tidyverse.org/
        version:
          date: "31/01/2024"
    "#});
    insta::assert_snapshot!(rendered);
}

#[test]
fn s14_date_not_iso_errors() {
    assert_invalid(
        inline(indoc! {r#"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            version:
              date: "31/01/2024"
        "#}),
        &["S14", "ISO 8601 date", "31/01/2024"],
    );
}

// The schema fixes `version`'s shape, so an unknown kind or a non-map value
// fails structurally (pre-flight) rather than at S14.
#[test]
fn version_unknown_key_errors() {
    assert_invalid(
        inline(indoc! {"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            version:
              tag: release-7
        "}),
        &["Unknown property 'tag'"],
    );
}

#[test]
fn version_not_a_map_errors() {
    assert_invalid(
        inline(indoc! {"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            version: 2024-01-31
        "}),
        &["object"],
    );
}

#[test]
#[cfg(unix)]
fn enum_non_string_label() {
    insta::assert_snapshot!(failing_diagnostic("invalid/enum-non-string-label.yaml"));
}

#[test]
fn enum_non_string_label_errors() {
    assert_invalid(
        fixture("invalid/enum-non-string-label.yaml"),
        &["YAML Validation Failed"],
    );
}
