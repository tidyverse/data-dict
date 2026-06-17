//! Validate actual data against a data dictionary.
//!
//! Where [`crate::validate`] checks that a `data-dict.yaml` document is
//! internally well-formed, this module checks that a *dataset* is consistent
//! with the dictionary that describes it. Parquet is the first supported
//! backend; more (SQL, R, …) are expected later.
//!
//! [`validate_parquet`] validates the dictionary, reads the parquet file's
//! column types via [`data_dict_parquet::column_types`], and compares the two
//! column-by-column.

use std::collections::HashMap;
use std::path::Path;

use data_dict_parquet::{NullReport, ParquetError};

use crate::model::Column;
use crate::Error;

/// How many example values (e.g. offending rows) to record per validation
/// issue. Issues count every offender but only list this many.
const SAMPLE_LIMIT: usize = 5;

/// A single way in which a dataset disagrees with its data dictionary.
#[derive(Debug)]
pub enum ColumnIssue {
    /// A column present in both, but whose declared type is not compatible
    /// with the type read from the data.
    TypeMismatch {
        column: String,
        declared: String,
        actual: String,
    },
    /// A column described by the dictionary that is absent from the data.
    MissingInData { column: String },
    /// A column in the data that the dictionary does not describe.
    ExtraInData { column: String, actual: String },
    /// A column the dictionary marks `required` (or `primary_key`) that
    /// nonetheless contains null values. `rows` lists the first few offending
    /// row numbers (1-based); `count` is the true total.
    NullsInRequired {
        column: String,
        count: usize,
        rows: Vec<usize>,
    },
}

/// Errors returned by [`validate_parquet`].
#[derive(Debug)]
pub enum DataError {
    /// The dictionary itself failed schema/semantic validation.
    Schema(Error),
    /// The parquet file could not be read.
    Parquet(ParquetError),
    /// A table name was given but no such table exists in the dictionary.
    TableNotFound {
        name: String,
        available: Vec<String>,
    },
    /// No table name was given and the dictionary describes more than one, so
    /// the target is ambiguous.
    AmbiguousTable { available: Vec<String> },
    /// The dataset disagrees with the dictionary in one or more columns.
    Mismatch {
        table: String,
        issues: Vec<ColumnIssue>,
    },
}

impl From<Error> for DataError {
    fn from(e: Error) -> Self {
        DataError::Schema(e)
    }
}

impl From<ParquetError> for DataError {
    fn from(e: ParquetError) -> Self {
        DataError::Parquet(e)
    }
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Schema(e) => write!(f, "{e}"),
            DataError::Parquet(e) => write!(f, "{e}"),
            DataError::TableNotFound { name, available } => {
                write!(
                    f,
                    "table \"{name}\" is not in the data dictionary (available: {})",
                    available.join(", ")
                )
            }
            DataError::AmbiguousTable { available } => {
                write!(
                    f,
                    "the data dictionary describes multiple tables ({}); specify which one to validate against",
                    available.join(", ")
                )
            }
            DataError::Mismatch { table, issues } => {
                writeln!(
                    f,
                    "data does not match table \"{table}\" in the data dictionary:"
                )?;
                for issue in issues {
                    match issue {
                        ColumnIssue::TypeMismatch {
                            column,
                            declared,
                            actual,
                        } => writeln!(
                            f,
                            "  column \"{column}\": declared {declared}, data is {actual}"
                        )?,
                        ColumnIssue::MissingInData { column } => writeln!(
                            f,
                            "  column \"{column}\": described in dictionary but missing from data"
                        )?,
                        ColumnIssue::ExtraInData { column, actual } => writeln!(
                            f,
                            "  column \"{column}\": present in data ({actual}) but not in dictionary"
                        )?,
                        ColumnIssue::NullsInRequired { column, count, rows } => writeln!(
                            f,
                            "  column \"{column}\": required but has {count} null value{} ({})",
                            if *count == 1 { "" } else { "s" },
                            format_rows(rows, *count),
                        )?,
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DataError::Schema(e) => Some(e),
            DataError::Parquet(e) => Some(e),
            _ => None,
        }
    }
}

/// Validate a parquet file's columns against a data dictionary.
///
/// First validates the dictionary at `dict_path` (schema + lint). Then selects
/// the table to compare against: `table` if given, otherwise the sole table
/// when the dictionary describes exactly one. Finally reads the column types of
/// the parquet file at `parquet_path` and checks every column — reporting type
/// mismatches, nulls in required columns, columns described but absent from the
/// data, and columns in the data the dictionary does not describe.
pub fn validate_parquet(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
) -> Result<(), DataError> {
    let dict = crate::validate_and_lower(dict_path)?;

    let available = || dict.tables.keys().cloned().collect::<Vec<_>>();
    let table = match table {
        Some(name) => dict.tables.get(name).ok_or_else(|| DataError::TableNotFound {
            name: name.to_string(),
            available: available(),
        })?,
        None => {
            if dict.tables.len() == 1 {
                dict.tables.values().next().expect("len == 1")
            } else {
                return Err(DataError::AmbiguousTable {
                    available: available(),
                });
            }
        }
    };

    let actual = data_dict_parquet::column_types(parquet_path)?;

    // Value-level checks read the data pages, so scan once up front for the
    // columns those checks apply to (currently: required columns present in the
    // data) and let the per-column checks look up what they need.
    let scan_cols: Vec<String> = table
        .columns
        .iter()
        .filter(|c| c.is_required_implied())
        .map(|c| c.name.value.clone())
        .filter(|n| actual.iter().any(|(an, _)| an == n))
        .collect();
    let nulls = if scan_cols.is_empty() {
        HashMap::new()
    } else {
        data_dict_parquet::null_report(parquet_path, &scan_cols, SAMPLE_LIMIT)?
    };

    let mut issues = Vec::new();

    // One pass per dictionary column: this is where per-column validations live.
    // A column absent from the data can't be value-checked, so we report it and
    // skip; otherwise it runs the full battery of checks.
    for col in &table.columns {
        let Some((_, actual_type)) = actual.iter().find(|(n, _)| n == &col.name.value) else {
            issues.push(ColumnIssue::MissingInData {
                column: col.name.value.clone(),
            });
            continue;
        };
        check_type(col, actual_type, &mut issues);
        check_required_not_null(col, &nulls, &mut issues);
    }

    // Columns present in the data that the dictionary does not describe.
    for (name, actual_type) in &actual {
        if table.column(name).is_none() {
            issues.push(ColumnIssue::ExtraInData {
                column: name.clone(),
                actual: actual_type.clone(),
            });
        }
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(DataError::Mismatch {
            table: table.name.value.clone(),
            issues,
        })
    }
}

/// A column's declared type must be compatible with the type read from the data.
fn check_type(col: &Column, actual_type: &str, issues: &mut Vec<ColumnIssue>) {
    if let Some(declared) = &col.col_type
        && !types_compatible(&declared.value, actual_type)
    {
        issues.push(ColumnIssue::TypeMismatch {
            column: col.name.value.clone(),
            declared: declared.value.clone(),
            actual: actual_type.to_string(),
        });
    }
}

/// A `required` (or `primary_key`) column must contain no nulls. Relies on
/// `nulls` having been populated for `col` by the up-front scan.
fn check_required_not_null(
    col: &Column,
    nulls: &HashMap<String, NullReport>,
    issues: &mut Vec<ColumnIssue>,
) {
    if col.is_required_implied()
        && let Some(report) = nulls.get(&col.name.value)
        && report.count > 0
    {
        issues.push(ColumnIssue::NullsInRequired {
            column: col.name.value.clone(),
            count: report.count,
            rows: report.rows.clone(),
        });
    }
}

/// Format offending row numbers for display: `rows: 3, 7, 12`, with a trailing
/// `, …` when there were more nulls than the recorded sample.
fn format_rows(rows: &[usize], count: usize) -> String {
    let listed = rows
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if count > rows.len() {
        format!("rows: {listed}, …")
    } else {
        format!("rows: {listed}")
    }
}

/// Collapse a declared dictionary type to its base form for comparison by
/// dropping any trailing `(...)` qualifier.
fn normalize_dict_type(dict_type: &str) -> &str {
    match dict_type.find('(') {
        Some(i) => &dict_type[..i],
        None => dict_type,
    }
}

/// Whether a declared dictionary type is compatible with a type read from the
/// data (one of `boolean`, `string`, `enum`, `date`, `datetime`, `number`).
///
/// Dictionary types are coarser/richer than physical types, so the match is by
/// category rather than exact string. An `enum` is backed by either a string
/// or a number in the data (or a true parquet enum), so all three are accepted.
fn types_compatible(dict_type: &str, actual: &str) -> bool {
    match normalize_dict_type(dict_type) {
        "number" => actual == "number",
        "string" => actual == "string",
        "boolean" => actual == "boolean",
        "date" => actual == "date",
        "datetime" => actual == "datetime",
        "enum" => matches!(actual, "string" | "number" | "enum"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_measures_normalize() {
        assert_eq!(normalize_dict_type("number(quantity)"), "number");
        assert_eq!(normalize_dict_type("number(id)"), "number");
        assert_eq!(normalize_dict_type("number"), "number");
        assert_eq!(normalize_dict_type("string"), "string");
    }

    #[test]
    fn row_formatting() {
        assert_eq!(format_rows(&[2], 1), "rows: 2");
        assert_eq!(format_rows(&[2, 5, 9], 3), "rows: 2, 5, 9");
        // More nulls than the recorded sample gets an ellipsis.
        assert_eq!(format_rows(&[1, 2, 3, 4, 5], 8), "rows: 1, 2, 3, 4, 5, …");
    }

    #[test]
    fn compatibility() {
        assert!(types_compatible("number(quantity)", "number"));
        assert!(types_compatible("string", "string"));
        assert!(types_compatible("enum", "string"));
        assert!(types_compatible("enum", "number"));
        assert!(types_compatible("enum", "enum"));
        assert!(!types_compatible("number", "string"));
        assert!(!types_compatible("date", "datetime"));
        assert!(!types_compatible("boolean", "number"));
    }
}
