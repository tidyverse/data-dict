//! Integration tests for the metadata and data comparison levels
//! (`data_dict::meta::validate_meta` and `data_dict::data::validate_data`).
//!
//! Each test writes a small parquet file (a `string` column `name` and a
//! `number` column `weight`) and a dictionary YAML to a temp dir, then checks
//! the outcome of validating one against the other.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use data_dict::data::validate_data;
use data_dict::meta::validate_meta;
use data_dict::{ColumnIssue, CompareError, CompareReport, IssueKind, Severity};
use indoc::{formatdoc, indoc};
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::{SerializedColumnWriter, SerializedFileWriter};
use parquet::schema::parser::parse_message_type;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp directory for one test's fixtures.
fn temp_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "data-dict-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a parquet file with a string `name` column and a double `weight`
/// column.
fn write_parquet(path: &Path) {
    let message = indoc! {"
        message schema {
            REQUIRED BYTE_ARRAY name (UTF8);
            REQUIRED DOUBLE weight;
        }
    "};
    let schema = Arc::new(parse_message_type(message).unwrap());
    let props = Arc::new(WriterProperties::builder().build());
    let file = File::create(path).unwrap();
    let mut writer = SerializedFileWriter::new(file, schema, props).unwrap();
    let mut rg = writer.next_row_group().unwrap();

    let mut col = rg.next_column().unwrap().unwrap();
    col.typed::<ByteArrayType>()
        .write_batch(
            &[ByteArray::from("otter"), ByteArray::from("seal")],
            None,
            None,
        )
        .unwrap();
    col.close().unwrap();

    let mut col = rg.next_column().unwrap().unwrap();
    col.typed::<DoubleType>()
        .write_batch(&[1.0_f64, 2.0], None, None)
        .unwrap();
    col.close().unwrap();

    rg.close().unwrap();
    writer.close().unwrap();
}

/// Write `yaml` to `<dir>/dict.yaml` and return the path.
fn write_yaml(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("dict.yaml");
    std::fs::write(&path, yaml).unwrap();
    path
}

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

    let report = validate_data(&yaml, &parquet, None).1.unwrap();
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

    let report = validate_data(&yaml, &parquet, None).1.unwrap();
    assert!(report.has_errors());
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, kind: IssueKind::TypeMismatch { declared, actual }, .. }]
            if column == "weight" && declared == "string" && actual == "number"
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
    let report = validate_data(&yaml, &parquet, None).1.unwrap();
    assert!(!report.has_errors(), "got {:?}", report.issues);
    assert!(matches!(
        report.issues.as_slice(),
        [ColumnIssue { column, severity, kind: IssueKind::ExtraInData { actual }, .. }]
            if column == "weight" && actual == "number" && *severity == Severity::Warning
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

    let report = validate_data(&yaml, &parquet, None).1.unwrap();
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

    let report = validate_data(&yaml, &parquet, None).1.unwrap();
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

    let report = validate_data(&yaml, &parquet, None).1.unwrap();
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

    let err = validate_data(&yaml, &parquet, None).1.unwrap_err();
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

    let err = validate_data(&yaml, &parquet, Some("nope")).1.unwrap_err();
    assert!(
        matches!(err, CompareError::TableNotFound { .. }),
        "got {err:?}"
    );
}

/// Validate a single column in isolation. Writes a one-column parquet file
/// (`schema_col` is that column's line in a parquet message-type schema, e.g.
/// `OPTIONAL DOUBLE weight`; `write` fills in its data) and wraps `column` —
/// the YAML for one `columns:` entry — in an otherwise-minimal one-table
/// dictionary, then validates the two against each other.
///
/// This keeps per-column tests focused: only the column under test is generated
/// in both the data and the spec.
fn check_column(
    schema_col: &str,
    write: impl FnOnce(&mut SerializedColumnWriter),
    column: &str,
) -> Result<CompareReport, CompareError> {
    let (yaml, parquet) = build_column(schema_col, write, column);
    validate_data(&yaml, &parquet, None).1
}

/// Write the one-column parquet file and dictionary used by [`check_column`],
/// returning their paths so a test can run more than one validation level
/// against the same pair.
fn build_column(
    schema_col: &str,
    write: impl FnOnce(&mut SerializedColumnWriter),
    column: &str,
) -> (PathBuf, PathBuf) {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");

    let message = format!("message schema {{ {schema_col}; }}");
    let schema = Arc::new(parse_message_type(&message).unwrap());
    let props = Arc::new(WriterProperties::builder().build());
    let file = File::create(&parquet).unwrap();
    let mut writer = SerializedFileWriter::new(file, schema, props).unwrap();
    let mut rg = writer.next_row_group().unwrap();
    let mut col = rg.next_column().unwrap().unwrap();
    write(&mut col);
    col.close().unwrap();
    rg.close().unwrap();
    writer.close().unwrap();

    // Indent the caller's column entry to sit under `columns:`.
    let column = column
        .trim_end()
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let yaml = write_yaml(
        &dir,
        &formatdoc! {"
            $version: 0.1.0
            tables:
              t:
                source:
                  parquet: data.parquet
                columns:
            {column}
        "},
    );

    (yaml, parquet)
}

// --- metadata level vs data level ----------------------------------------

/// The defining difference between the two levels: a `required` column with
/// nulls is a *value* problem, so it is invisible to `validate-meta` (which
/// reads only names and types) but caught by `validate-data` (which scans).
#[test]
fn meta_ignores_null_values_that_data_catches() {
    let (yaml, parquet) = build_column(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(quantity)
              constraints: [required]
              range: [0, 100]
        "},
    );

    // Metadata level: the column exists with a compatible type, so it's clean.
    let meta = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(meta.is_clean(), "meta got {:?}", meta.issues);

    // Data level: the null in a required column is an error.
    let data = validate_data(&yaml, &parquet, None).1.unwrap();
    assert!(data.has_errors());
    assert!(
        matches!(
            data.issues.as_slice(),
            [ColumnIssue {
                kind: IssueKind::NullsInRequired { .. },
                ..
            }]
        ),
        "data got {:?}",
        data.issues
    );
}

/// Type and presence problems are metadata-level, so `validate-meta` reports
/// them on its own, without reading any values.
#[test]
fn meta_reports_type_mismatch() {
    let (yaml, parquet) = build_column(
        "REQUIRED DOUBLE weight",
        |col| {
            col.typed::<DoubleType>()
                .write_batch(&[1.0_f64, 2.0], None, None)
                .unwrap();
        },
        indoc! {"
            - name: weight
              type: string
              examples: ['1', '2']
        "},
    );

    let report = validate_meta(&yaml, &parquet, None).1.unwrap();
    assert!(report.has_errors());
    assert!(
        matches!(
            report.issues.as_slice(),
            [ColumnIssue { column, code, kind: IssueKind::TypeMismatch { .. }, .. }]
                if column == "weight" && *code == "M01"
        ),
        "got {:?}",
        report.issues
    );
}

/// Write an optional double column whose second row (1-based) is null. Used by
/// the nullability tests via [`check_column`].
fn write_double_with_null(col: &mut SerializedColumnWriter) {
    // Definition levels: 1 = present, 0 = null. Row 2 is null, so the values
    // slice holds only the two non-null doubles.
    col.typed::<DoubleType>()
        .write_batch(&[1.0_f64, 2.0], Some(&[1, 0, 1]), None)
        .unwrap();
}

#[test]
fn nulls_in_required_column_reported() {
    let result = check_column(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(quantity)
              constraints: [required]
              range: [0, 100]
        "},
    );

    let report = result.unwrap();
    assert!(report.has_errors());
    assert!(
        matches!(
            report.issues.as_slice(),
            [ColumnIssue { column, kind: IssueKind::NullsInRequired { count, rows }, .. }]
                if column == "weight" && *count == 1 && rows == &[2]
        ),
        "got {:?}",
        report.issues
    );
}

#[test]
fn required_column_without_nulls_ok() {
    // No nulls present, so the statistics fast-path should resolve this without
    // scanning the data pages.
    let result = check_column(
        "REQUIRED DOUBLE weight",
        |col| {
            col.typed::<DoubleType>()
                .write_batch(&[1.0_f64, 2.0, 3.0], None, None)
                .unwrap();
        },
        indoc! {"
            - name: weight
              type: number(quantity)
              constraints: [required]
              range: [0, 100]
        "},
    );

    assert!(result.unwrap().is_clean());
}

#[test]
fn nulls_in_optional_column_ok() {
    // `weight` has a null but is not declared required, so it's fine.
    let result = check_column(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(quantity)
              range: [0, 100]
        "},
    );

    assert!(result.unwrap().is_clean());
}

#[test]
fn primary_key_implies_required_for_nulls() {
    // `primary_key` implies `required`, so the null is reported even without an
    // explicit `required` constraint.
    let result = check_column(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(id)
              constraints: [primary_key]
              examples: [1, 2]
        "},
    );

    let report = result.unwrap();
    assert!(report.has_errors());
    assert!(
        matches!(
            report.issues.as_slice(),
            [ColumnIssue { column, kind: IssueKind::NullsInRequired { .. }, .. }] if column == "weight"
        ),
        "got {:?}",
        report.issues
    );
}
