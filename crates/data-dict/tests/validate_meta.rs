//! Integration tests for the metadata level (`data_dict::validate_meta`):
//! the data's column names and types against the dictionary, no value scan.
//!
//! Each test writes a small parquet file (a `string` column `name` and a
//! `number` column `weight`) and a dictionary YAML to a temp dir, then validates
//! one against the other.

mod common;
use common::{temp_dir, write_parquet, write_yaml};

use data_dict::{Problem, ProblemKind, Severity, Status, validate_meta};
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem { column: Some(column), code: Some(code), kind: ProblemKind::TypeMismatch { declared, actual }, .. }]
            if column == "weight" && *code == "M01" && declared == "string" && actual == "number"
    ));
    insta::assert_snapshot!(problems.render().join("\n"));
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
            $learn_more: http://data-dict.tidyverse.org/
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
    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(
        problems.status(),
        Status::Warning,
        "got {:?}",
        problems.items
    );
    assert!(matches!(
        problems.items.as_slice(),
        [Problem { column: Some(column), code: Some(code), severity, kind: ProblemKind::ExtraInData { actual }, .. }]
            if column == "weight" && *code == "M03" && actual == "number" && *severity == Severity::Warning
    ));
    insta::assert_snapshot!(problems.render().join("\n"));
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem { column: Some(column), kind: ProblemKind::MissingInData, .. }] if column == "height"
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem { column: Some(column), kind: ProblemKind::MissingInData, .. }] if column == "height"
    ));
    insta::assert_snapshot!(problems.render().join("\n"));
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, None);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem {
                kind: ProblemKind::AmbiguousTable { .. },
                ..
            }]
        ),
        "got {:?}",
        problems.items
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
            $learn_more: http://data-dict.tidyverse.org/
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

    let problems = validate_meta(&yaml, &parquet, Some("nope"));
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem {
                kind: ProblemKind::TableNotFound { .. },
                ..
            }]
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn missing_source_reported() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // The table declares no `source`; valid at the spec level, but M04 at meta.
    let yaml = write_yaml(
        &dir,
        indoc! {"
            $version: 0.1.0
            $learn_more: http://data-dict.tidyverse.org/
            tables:
              animals:
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let problems = validate_meta(&yaml, &parquet, None);
    assert!(
        problems
            .items
            .iter()
            .any(|p| p.code == Some("M04") && p.severity == Severity::Error),
        "expected an M04 missing-source error, got {:?}",
        problems.items
    );
}
