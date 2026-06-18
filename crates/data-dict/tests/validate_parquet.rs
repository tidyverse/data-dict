//! Integration tests for `data_dict::data::validate_parquet`.
//!
//! Each test writes a small parquet file (a `string` column `name` and a
//! `number` column `weight`) and a dictionary YAML to a temp dir, then checks
//! the outcome of validating one against the other.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use data_dict::data::{ColumnIssue, DataError, validate_parquet};
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
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
    let message = "
        message schema {
            REQUIRED BYTE_ARRAY name (UTF8);
            REQUIRED DOUBLE weight;
        }
    ";
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
        "
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
",
    );

    assert!(validate_parquet(&yaml, &parquet, None).is_ok());
}

#[test]
fn type_mismatch_reported() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // `weight` is a double in the data but declared as a string here.
    let yaml = write_yaml(
        &dir,
        "
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
",
    );

    let err = validate_parquet(&yaml, &parquet, None).unwrap_err();
    let DataError::Mismatch { issues, .. } = err else {
        panic!("expected Mismatch, got {err:?}");
    };
    assert!(matches!(
        issues.as_slice(),
        [ColumnIssue::TypeMismatch { column, declared, actual }]
            if column == "weight" && declared == "string" && actual == "number"
    ));
}

#[test]
fn extra_column_in_data_reported() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    // Dictionary omits `weight`, which is present in the parquet file.
    let yaml = write_yaml(
        &dir,
        "
$version: 0.1.0
tables:
  animals:
    source:
      parquet: data.parquet
    columns:
      - name: name
        type: string
        examples: [otter, seal]
",
    );

    let err = validate_parquet(&yaml, &parquet, None).unwrap_err();
    let DataError::Mismatch { issues, .. } = err else {
        panic!("expected Mismatch, got {err:?}");
    };
    assert!(matches!(
        issues.as_slice(),
        [ColumnIssue::ExtraInData { column, actual }]
            if column == "weight" && actual == "number"
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
        "
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
",
    );

    let err = validate_parquet(&yaml, &parquet, None).unwrap_err();
    let DataError::Mismatch { issues, .. } = err else {
        panic!("expected Mismatch, got {err:?}");
    };
    assert!(matches!(
        issues.as_slice(),
        [ColumnIssue::MissingInData { column }] if column == "height"
    ));
}

#[test]
fn ambiguous_table_without_name() {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    write_parquet(&parquet);
    let yaml = write_yaml(
        &dir,
        "
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
",
    );

    let err = validate_parquet(&yaml, &parquet, None).unwrap_err();
    assert!(
        matches!(err, DataError::AmbiguousTable { .. }),
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
        "
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
",
    );

    let err = validate_parquet(&yaml, &parquet, Some("nope")).unwrap_err();
    assert!(
        matches!(err, DataError::TableNotFound { .. }),
        "got {err:?}"
    );
}
