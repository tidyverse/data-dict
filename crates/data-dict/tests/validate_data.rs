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
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType, Int32Type};
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

/// Write the given strings as a required UTF-8 byte-array column.
fn write_strings<'a>(values: &'a [&'a str]) -> impl FnOnce(&mut SerializedColumnWriter) + 'a {
    move |col| {
        let bytes = values
            .iter()
            .map(|s| ByteArray::from(*s))
            .collect::<Vec<_>>();
        col.typed::<ByteArrayType>()
            .write_batch(&bytes, None, None)
            .unwrap();
    }
}

#[test]
fn values_outside_enum_reported() {
    let yaml = build_column(
        "REQUIRED BYTE_ARRAY status (UTF8)",
        write_strings(&["active", "banned", "active", "sleepy"]),
        indoc! {"
            - name: status
              type: enum
              values: [active, banned]
        "},
    );
    let result = validate_data(&yaml, None);

    assert_eq!(result.status(), Status::Error);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem {
                code: Some("D03"),
                kind: ProblemKind::ValuesOutsideEnum { count: 1, rows, values },
                ..
            }] if rows == &[4] && values == &["sleepy"]
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
fn enum_values_within_set_ok() {
    let result = check_column(
        "REQUIRED BYTE_ARRAY status (UTF8)",
        write_strings(&["active", "banned", "active"]),
        indoc! {"
            - name: status
              type: enum
              values: [active, banned]
        "},
    );

    assert_eq!(result.status(), Status::Ok, "got {:?}", result.items);
}

#[test]
fn enum_map_form_values_are_the_keys() {
    // The map form's keys are the allowed values; the labels are ignored.
    let result = check_column(
        "REQUIRED BYTE_ARRAY status (UTF8)",
        write_strings(&["A", "Active"]),
        indoc! {"
            - name: status
              type: enum
              values:
                A: Active
                B: Banned
        "},
    );

    assert_eq!(result.status(), Status::Error);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem {
                kind: ProblemKind::ValuesOutsideEnum { count: 1, rows, values },
                ..
            }] if rows == &[2] && values == &["Active"]
        ),
        "got {:?}",
        result.items
    );
}

#[test]
fn nulls_in_optional_enum_are_not_outside_values() {
    // A null is the concern of D01 (and only when required); it is never an
    // "outside the set" value.
    let result = check_column(
        "OPTIONAL BYTE_ARRAY status (UTF8)",
        |col| {
            let bytes = [ByteArray::from("active"), ByteArray::from("banned")];
            col.typed::<ByteArrayType>()
                .write_batch(&bytes, Some(&[1, 0, 1]), None)
                .unwrap();
        },
        indoc! {"
            - name: status
              type: enum
              values: [active, banned]
        "},
    );

    assert_eq!(result.status(), Status::Ok, "got {:?}", result.items);
}

#[test]
fn numeric_enum_values_are_checked() {
    let result = check_column(
        "REQUIRED INT32 grade",
        |col| {
            col.typed::<Int32Type>()
                .write_batch(&[1, 2, 3], None, None)
                .unwrap();
        },
        indoc! {"
            - name: grade
              type: enum
              values: [1, 2]
        "},
    );

    assert_eq!(result.status(), Status::Error);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem {
                kind: ProblemKind::ValuesOutsideEnum { count: 1, rows, values },
                ..
            }] if rows == &[3] && values == &["3"]
        ),
        "got {:?}",
        result.items
    );
}

/// With dictionary encoding disabled, the D03 dictionary fast-path can't prove
/// conformance and must fall back to the value scan — which still finds the
/// violation and its exact row.
#[test]
fn enum_without_dictionary_encoding_falls_back_to_scan() {
    let no_dict = || {
        WriterProperties::builder()
            .set_dictionary_enabled(false)
            .build()
    };

    let clean = build_column_with_properties(
        "REQUIRED BYTE_ARRAY status (UTF8)",
        write_strings(&["active", "banned", "active"]),
        indoc! {"
            - name: status
              type: enum
              values: [active, banned]
        "},
        no_dict(),
    );
    assert_eq!(validate_data(&clean, None).status(), Status::Ok);

    let bad = build_column_with_properties(
        "REQUIRED BYTE_ARRAY status (UTF8)",
        write_strings(&["active", "banned", "sleepy"]),
        indoc! {"
            - name: status
              type: enum
              values: [active, banned]
        "},
        no_dict(),
    );
    let result = validate_data(&bad, None);
    assert!(
        matches!(
            result.items.as_slice(),
            [Problem {
                code: Some("D03"),
                kind: ProblemKind::ValuesOutsideEnum { count: 1, rows, values },
                ..
            }] if rows == &[3] && values == &["sleepy"]
        ),
        "got {:?}",
        result.items
    );
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
