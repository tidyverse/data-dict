//! Integration tests for the metadata level (`data_dict::validate_meta`):
//! the data's column names and types against the dictionary, no value scan.
//!
//! Each test writes a small parquet file (a `string` column `name` and a
//! `number` column `weight`) and a dictionary YAML to a temp dir, then validates
//! one against the other.

mod common;
use common::{assert_snapshot, temp_dir, write_dict, write_parquet};

use std::path::{Path, PathBuf};

use data_dict::{Problem, ProblemKind, Severity, Status, validate_meta};
use indoc::{formatdoc, indoc};

/// A fresh temp dir with the standard two-column parquet (`name`, `weight`)
/// written to `data.parquet`, ready for a dictionary that sources it.
fn dir_with_parquet() -> PathBuf {
    let dir = temp_dir();
    write_parquet(&dir.join("data.parquet"));
    dir
}

/// The boilerplate `name` column — a `string` matching the standard parquet
/// fixture's first column. Present in nearly every dictionary here and never
/// itself under test.
const NAME: &str = indoc! {"
    - name: name
      type: string
      examples: [otter, seal]
"};

/// The canonical `weight` column matching the fixture's second column.
const WEIGHT: &str = indoc! {"
    - name: weight
      type: number(quantity)
      range: [0, 100]
"};

/// Build a one-`animals`-table dictionary that sources `parquet` and lists
/// `columns` (one or more `columns:` entries, e.g. [`NAME`]/[`WEIGHT`],
/// re-indented to fit), written beneath the standard header into `dir`.
fn animals_dict(dir: &Path, parquet: &str, columns: &str) -> PathBuf {
    let columns = columns
        .trim_end()
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    write_dict(
        dir,
        &formatdoc! {"
            tables:
              - name: animals
                source:
                  parquet: {parquet}
                columns:
            {columns}
        "},
    )
}

#[test]
fn matching_dict_and_parquet() {
    let dir = dir_with_parquet();
    let yaml = animals_dict(&dir, "data.parquet", &format!("{NAME}{WEIGHT}"));

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn type_mismatch_reported() {
    let dir = dir_with_parquet();
    // `weight` is a double in the data but declared as a string here.
    let yaml = animals_dict(
        &dir,
        "data.parquet",
        &format!(
            "{NAME}{}",
            indoc! {"
                - name: weight
                  type: string
                  examples: ['1', '2']
            "}
        ),
    );

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem { code: Some(code), kind: ProblemKind::TypeMismatch { declared, actual }, .. }]
            if *code == "M01" && declared == "string" && actual == "number"
    ));
    #[cfg(unix)]
    assert_snapshot!(common::diagnostic(&yaml, &problems.render().join("\n")));
}

#[test]
fn extra_column_in_data_is_warning() {
    let dir = dir_with_parquet();
    // Dictionary omits `weight`, which is present in the parquet file.
    let yaml = animals_dict(&dir, "data.parquet", NAME);

    // An undocumented column is a warning, not an error: it is reported but does
    // not fail validation.
    let problems = validate_meta(&yaml, None);
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
    assert_snapshot!(common::diagnostic(&yaml, &problems.render().join("\n")));
}

#[test]
fn typeless_column_skips_type_check_for_present_column() {
    let dir = dir_with_parquet();
    // `weight` is a double in the data but listed without a `type`, so its type
    // is not checked; and because it is listed it is not flagged as undocumented.
    let yaml = animals_dict(&dir, "data.parquet", &format!("{NAME}- name: weight\n"));

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn typeless_column_still_must_exist_in_data() {
    let dir = dir_with_parquet();
    // `height` is listed (without a `type`) but absent from the data. Listing a
    // column that doesn't exist is an error, even when it isn't described.
    let yaml = animals_dict(
        &dir,
        "data.parquet",
        &format!("{NAME}{WEIGHT}- name: height\n"),
    );

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem {
            kind: ProblemKind::MissingInData,
            ..
        }]
    ));
}

#[test]
fn missing_column_in_data_reported() {
    let dir = dir_with_parquet();
    // Dictionary describes `height`, which is absent from the parquet file.
    let yaml = animals_dict(
        &dir,
        "data.parquet",
        &format!(
            "{NAME}{WEIGHT}{}",
            indoc! {"
                - name: height
                  type: number(quantity)
                  range: [0, 100]
            "}
        ),
    );

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(matches!(
        problems.items.as_slice(),
        [Problem {
            kind: ProblemKind::MissingInData,
            ..
        }]
    ));
    #[cfg(unix)]
    assert_snapshot!(common::diagnostic(&yaml, &problems.render().join("\n")));
}

#[test]
fn validates_every_table() {
    let dir = temp_dir();
    write_parquet(&dir.join("animals.parquet"));
    write_parquet(&dir.join("plants.parquet"));
    // Two tables, each with its own source. `animals` matches its data; `plants`
    // declares a `height` column its data lacks (M02). One run checks both.
    let yaml = write_dict(
        &dir,
        indoc! {"
            tables:
              - name: animals
                source:
                  parquet: animals.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
              - name: plants
                source:
                  parquet: plants.parquet
                columns:
                  - name: name
                    type: string
                    examples: [moss, fern]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
                  - name: height
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems
            .items
            .iter()
            .any(|p| p.code == Some("M02") && p.severity == Severity::Error),
        "expected plants/height to be reported as M02, got {:?}",
        problems.items
    );
}

#[test]
fn unreadable_source_reported() {
    let dir = temp_dir();
    // The table declares a `source`, but the parquet file it names does not exist.
    let yaml = animals_dict(&dir, "missing.parquet", NAME);

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem {
                code: Some(code),
                kind: ProblemKind::UnreadableSource,
                severity: Severity::Error,
                ..
            }] if *code == "M05"
        ),
        "got {:?}",
        problems.items
    );
    #[cfg(unix)]
    assert_snapshot!(common::diagnostic(&yaml, &problems.render().join("\n")));
}

#[test]
fn unreadable_source_does_not_stop_other_tables() {
    let dir = temp_dir();
    // `plants` has real data; `animals` points at a file that doesn't exist. The
    // missing source (M05) is reported, but `plants` is still checked, where its
    // declared `weight` type disagrees with the data (M01).
    write_parquet(&dir.join("plants.parquet"));
    let yaml = write_dict(
        &dir,
        indoc! {"
            tables:
              - name: animals
                source:
                  parquet: missing.parquet
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
              - name: plants
                source:
                  parquet: plants.parquet
                columns:
                  - name: name
                    type: string
                    examples: [moss, fern]
                  - name: weight
                    type: string
                    examples: ['1', '2']
        "},
    );

    let problems = validate_meta(&yaml, None);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems.items.iter().any(|p| p.code == Some("M05")),
        "expected an M05 unreadable-source error, got {:?}",
        problems.items
    );
    assert!(
        problems.items.iter().any(|p| p.code == Some("M01")),
        "expected an M01 type-mismatch error from the readable table, got {:?}",
        problems.items
    );
    #[cfg(unix)]
    assert_snapshot!(common::diagnostic(&yaml, &problems.render().join("\n")));
}

#[test]
fn unknown_table_name() {
    let dir = dir_with_parquet();
    let yaml = animals_dict(&dir, "data.parquet", &format!("{NAME}{WEIGHT}"));

    let problems = validate_meta(&yaml, Some("nope"));
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
    // The table declares no `source`; valid at the spec level, but M04 at meta.
    let yaml = write_dict(
        &dir,
        indoc! {"
            tables:
              - name: animals
                columns:
                  - name: name
                    type: string
                    examples: [otter, seal]
                  - name: weight
                    type: number(quantity)
                    range: [0, 100]
        "},
    );

    let problems = validate_meta(&yaml, None);
    assert!(
        problems
            .items
            .iter()
            .any(|p| p.code == Some("M04") && p.severity == Severity::Error),
        "expected an M04 missing-source error, got {:?}",
        problems.items
    );
}
