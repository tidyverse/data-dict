//! Integration tests for the data level (`data_dict::validate_data`): the
//! data's *values* against the dictionary, which requires scanning the data.
//!
//! These tests focus on the value-level checks the data level adds on top of the
//! metadata checks (today, nulls in a required column), and confirm the metadata
//! and data levels are genuinely distinct.

mod common;
use common::{assert_snapshot, temp_dir, write_dict};

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use data_dict::{Problem, ProblemKind, ProblemSet, Status, validate_data, validate_meta};
use indoc::{formatdoc, indoc};
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType};
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::writer::{SerializedColumnWriter, SerializedFileWriter};
use parquet::schema::parser::parse_message_type;

/// Validate a single column's values in isolation, via [`build_column`].
fn check_column(
    schema_col: &str,
    write: impl FnOnce(&mut SerializedColumnWriter),
    column: &str,
) -> ProblemSet {
    let yaml = build_column(schema_col, write, column);
    validate_data(&yaml, None)
}

/// Write a one-column parquet file (`schema_col` is that column's line in a
/// parquet message-type schema, e.g. `OPTIONAL DOUBLE weight`; `write` fills in
/// its data) and wrap `column` — the YAML for one `columns:` entry — in an
/// otherwise-minimal one-table dictionary whose `source` points at that file.
/// Returns the dictionary path.
fn build_column(
    schema_col: &str,
    write: impl FnOnce(&mut SerializedColumnWriter),
    column: &str,
) -> PathBuf {
    build_column_with_properties(
        schema_col,
        write,
        column,
        WriterProperties::builder().build(),
    )
}

fn build_column_with_properties(
    schema_col: &str,
    write: impl FnOnce(&mut SerializedColumnWriter),
    column: &str,
    properties: WriterProperties,
) -> PathBuf {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");

    let message = format!("message schema {{ {schema_col}; }}");
    let schema = Arc::new(parse_message_type(&message).unwrap());
    let props = Arc::new(properties);
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
    write_dict(
        &dir,
        &formatdoc! {"
            tables:
              - name: t
                source:
                  parquet: data.parquet
                columns:
            {column}
        "},
    )
}

/// Write an optional double column whose second row (1-based) is null.
fn write_double_with_null(col: &mut SerializedColumnWriter) {
    // Definition levels: 1 = present, 0 = null. Row 2 is null, so the values
    // slice holds only the two non-null doubles.
    col.typed::<DoubleType>()
        .write_batch(&[1.0_f64, 2.0], Some(&[1, 0, 1]), None)
        .unwrap();
}

fn build_composite_key(first: &[f64], second: &[f64]) -> PathBuf {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    let schema = Arc::new(
        parse_message_type("message schema { REQUIRED DOUBLE a; REQUIRED DOUBLE b; }").unwrap(),
    );
    let file = File::create(&parquet).unwrap();
    let mut writer =
        SerializedFileWriter::new(file, schema, Arc::new(WriterProperties::builder().build()))
            .unwrap();
    let mut row_group = writer.next_row_group().unwrap();
    let mut a = row_group.next_column().unwrap().unwrap();
    a.typed::<DoubleType>()
        .write_batch(first, None, None)
        .unwrap();
    a.close().unwrap();
    let mut b = row_group.next_column().unwrap().unwrap();
    b.typed::<DoubleType>()
        .write_batch(second, None, None)
        .unwrap();
    b.close().unwrap();
    row_group.close().unwrap();
    writer.close().unwrap();

    write_dict(
        &dir,
        indoc! {"
            tables:
              - name: t
                source:
                  parquet: data.parquet
                columns:
                  - name: a
                    type: number(id)
                    constraints: [primary_key]
                    examples: [1, 2]
                  - name: b
                    type: number(id)
                    constraints: [primary_key]
                    examples: [1, 2]
        "},
    )
}

/// The defining difference between the two levels: a `required` column with
/// nulls is a *value* problem, so it is invisible to `validate-meta` (which
/// reads only names and types) but caught by `validate-data` (which scans).
#[test]
fn meta_ignores_null_values_that_data_catches() {
    let yaml = build_column(
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
    let meta = validate_meta(&yaml, None);
    assert_eq!(meta.status(), Status::Ok, "meta got {:?}", meta.items);

    // Data level: the null in a required column is an error.
    let data = validate_data(&yaml, None);
    assert_eq!(data.status(), Status::Error);
    assert!(
        matches!(
            data.items.as_slice(),
            [Problem {
                code: Some(code),
                kind: ProblemKind::NullsInRequired { .. },
                ..
            }] if *code == "D01"
        ),
        "data got {:?}",
        data.items
    );
}

#[test]
fn nulls_in_required_column_reported() {
    let yaml = build_column(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(quantity)
              constraints: [required]
              range: [0, 100]
        "},
    );
    let result = validate_data(&yaml, None);

    assert_eq!(result.status(), Status::Error);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem { kind: ProblemKind::NullsInRequired { count, rows }, .. }]
                if *count == 1 && rows.is_empty()
        ),
        "got {:?}",
        result.items
    );
    #[cfg(unix)]
    assert_snapshot!(common::diagnostic(
        &yaml,
        &result.render(common::SNAPSHOT_STYLE).join("\n")
    ));
}

#[test]
fn missing_null_statistics_falls_back_to_data_scan() {
    let yaml = build_column_with_properties(
        "OPTIONAL DOUBLE weight",
        write_double_with_null,
        indoc! {"
            - name: weight
              type: number(quantity)
              constraints: [required]
              range: [0, 100]
        "},
        WriterProperties::builder()
            .set_statistics_enabled(EnabledStatistics::None)
            .build(),
    );

    let result = validate_data(&yaml, None);
    assert!(matches!(
        result.items.as_slice(),
        [Problem { kind: ProblemKind::NullsInRequired { count: 1, rows }, .. }]
            if rows == &[2]
    ));
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

    assert_eq!(result.status(), Status::Ok);
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

    assert_eq!(result.status(), Status::Ok);
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

    assert_eq!(result.status(), Status::Error);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem {
                kind: ProblemKind::NullsInRequired { .. },
                ..
            }]
        ),
        "got {:?}",
        result.items
    );
}

#[test]
fn duplicate_values_in_unique_column_reported() {
    let result = check_column(
        "REQUIRED DOUBLE id",
        |col| {
            col.typed::<DoubleType>()
                .write_batch(&[1.0, 1.0, 2.0], None, None)
                .unwrap();
        },
        indoc! {"
            - name: id
              type: number(id)
              constraints: [unique]
              examples: [1, 2]
        "},
    );

    assert!(matches!(
        result.items.as_slice(),
        [Problem {
            code: Some("D02"),
            kind: ProblemKind::DuplicateValues { columns, count: 1, rows },
            ..
        }] if columns == &["id"] && rows == &[2]
    ));
}

/// Write a single required string column whose values are split across the
/// given row groups, so the scan accumulates row offsets across group
/// boundaries and exercises the variable-length byte-key path.
fn build_string_groups(groups: &[&[&str]]) -> PathBuf {
    let dir = temp_dir();
    let parquet = dir.join("data.parquet");
    let schema = Arc::new(
        parse_message_type("message schema { REQUIRED BYTE_ARRAY code (UTF8); }").unwrap(),
    );
    let file = File::create(&parquet).unwrap();
    let mut writer =
        SerializedFileWriter::new(file, schema, Arc::new(WriterProperties::builder().build()))
            .unwrap();
    for group in groups {
        let values = group
            .iter()
            .map(|s| ByteArray::from(*s))
            .collect::<Vec<_>>();
        let mut row_group = writer.next_row_group().unwrap();
        let mut col = row_group.next_column().unwrap().unwrap();
        col.typed::<ByteArrayType>()
            .write_batch(&values, None, None)
            .unwrap();
        col.close().unwrap();
        row_group.close().unwrap();
    }
    writer.close().unwrap();

    write_dict(
        &dir,
        indoc! {"
            tables:
              - name: t
                source:
                  parquet: data.parquet
                columns:
                  - name: code
                    type: string
                    constraints: [unique]
                    examples: [a, b]
        "},
    )
}

#[test]
fn duplicate_string_values_across_row_groups_reported() {
    // No duplicates across two groups.
    let unique = build_string_groups(&[&["a", "b"], &["c", "d"]]);
    assert_eq!(validate_data(&unique, None).status(), Status::Ok);

    // "a" recurs in the second group, so the duplicate sits at row 4 — proving
    // row numbers carry across the row-group boundary.
    let duplicate = build_string_groups(&[&["a", "b"], &["c", "a"]]);
    let result = validate_data(&duplicate, None);
    assert!(matches!(
        result.items.as_slice(),
        [Problem {
            code: Some("D02"),
            kind: ProblemKind::DuplicateValues { columns, count: 1, rows },
            ..
        }] if columns == &["code"] && rows == &[4]
    ));
}

#[test]
fn composite_primary_key_is_checked_collectively() {
    let unique = build_composite_key(&[1.0, 1.0, 2.0], &[1.0, 2.0, 1.0]);
    assert_eq!(validate_data(&unique, None).status(), Status::Ok);

    let duplicate = build_composite_key(&[1.0, 1.0, 2.0], &[1.0, 1.0, 2.0]);
    let result = validate_data(&duplicate, None);
    assert!(matches!(
        result.items.as_slice(),
        [Problem {
            code: Some("D02"),
            kind: ProblemKind::DuplicateValues { columns, count: 1, rows },
            ..
        }] if columns == &["a", "b"] && rows == &[2]
    ));
}
