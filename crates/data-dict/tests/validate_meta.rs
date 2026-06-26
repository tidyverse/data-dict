//! Integration tests for the metadata level (`data_dict::validate_meta`):
//! the data's column names and types against the dictionary, no value scan.
//!
//! Each test writes a small parquet file (a `string` column `name` and a
//! `number` column `weight`) and a dictionary YAML to a temp dir, then validates
//! one against the other.

mod common;
use common::{temp_dir, write_parquet, write_yaml};

use data_dict::{ColumnIssue, CompareError, IssueKind, Severity, validate_meta};
use indoc::indoc;

#[test]
fn matching_dict_and_parquet() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.is_clean(), "got {:?}", report.issues);
}

#[test]
fn type_mismatch_reported() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // `weight` is a double in the data but declared as a string here.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: string
                    examples: ['1', '2']
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.has_errors());
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, code, kind: IssueKind::TypeMismatch { declared, actual }, .. }]
            if column == "weight" && *code == "M01" && declared == "string" && actual == "number"
    ));
}

#[test]
fn extra_column_in_data_is_warning() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // Dictionary omits `weight`, which is present in the parquet file.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
        "},
    );

    // An undocumented column is a warning, not an error: it is reported but does
    // not fail validation.
    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(!report.has_errors(), "got {:?}", report.issues);
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, code, severity, kind: IssueKind::ExtraInData { actual } }]
            if column == "weight" && *code == "M03" && actual == "number" && *severity == Severity::Warning
    ));
}

#[test]
fn typeless_column_skips_type_check_for_present_column() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // `weight` is a double in the data but listed without a `type`, so its type
    // is not checked; and because it is listed it is not flagged as undocumented.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.is_clean(), "got {:?}", report.issues);
}

#[test]
fn typeless_column_still_must_exist_in_data() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // `height` is listed (without a `type`) but absent from the data. Listing a
    // column that doesn't exist is an error, even when it isn't described.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
                  - name: height
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.has_errors());
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, kind: IssueKind::MissingInData, .. }] if column == "height"
    ));
}

#[test]
fn missing_column_in_data_reported() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // Dictionary describes `height`, which is absent from the parquet file.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
                  - name: height
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.has_errors());
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, kind: IssueKind::MissingInData, .. }] if column == "height"
    ));
}

#[test]
fn ambiguous_table_without_name() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
              other:
                source:
                  parquet: other.parquet
                columns:
                  - name: id
                    type: number(id)
                    examples: [1, 2]
        "},
    );

    let err = validate_meta(&yaml, &parquet, None).1.unwrap_err();
    assert!(
        matches!(err, CompareError::AmbiguousTable { .. }),
        "got {err:?}"
    );
}

#[test]
fn unknown_table_name() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            tables:
              animals:
                source:
                  parquet: data.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let err = validate_meta(&yaml, &parquet, Some("nope")).1.unwrap_err();
    assert!(
        matches!(err, CompareError::TableNotFound { .. }),
        "got {err:?}"
    );
}
