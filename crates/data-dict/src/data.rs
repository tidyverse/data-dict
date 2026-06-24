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

use std::path::Path;

use data_dict_parquet::ParquetError;

use crate::model::Table;
use crate::{DataDict, Diagnostics, Error};

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
/// mismatches, columns described but absent from the data, and columns in the
/// data the dictionary does not describe.
///
/// Returns the dictionary's [`Diagnostics`] (errors and warnings, in emission
/// order, with the source context to render them) and the data-validation
/// result. The data comparison only runs when the dictionary itself is free of
/// errors; either an error diagnostic or an `Err` result means validation
/// failed.
pub fn validate_parquet(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
) -> (Diagnostics, Result<(), DataError>) {
    let (dict, diagnostics) = match crate::validate_and_lower(dict_path) {
        Ok(parsed) => parsed,
        Err(err) => return (Diagnostics::empty(), Err(err.into())),
    };
    // Comparing data against a dictionary that fails its own validation isn't
    // meaningful, so skip it; the error diagnostics already signal the failure.
    let result = if diagnostics.has_errors() {
        Ok(())
    } else {
        compare_parquet_to_dict(&dict, parquet_path, table)
    };
    (diagnostics, result)
}

fn compare_parquet_to_dict(
    dict: &DataDict,
    parquet_path: &Path,
    table: Option<&str>,
) -> Result<(), DataError> {
    let available = || dict.tables.keys().cloned().collect::<Vec<_>>();
    let table = match table {
        Some(name) => dict
            .tables
            .get(name)
            .ok_or_else(|| DataError::TableNotFound {
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
    let issues = compare_columns(table, &actual);

    if issues.is_empty() {
        Ok(())
    } else {
        Err(DataError::Mismatch {
            table: table.name.value.clone(),
            issues,
        })
    }
}

/// Compare a table's declared columns against the `(name, type)` pairs read from
/// the data, returning every discrepancy. An empty result means they agree.
fn compare_columns(table: &Table, actual: &[(String, String)]) -> Vec<ColumnIssue> {
    let mut issues = Vec::new();

    for (name, actual_type) in actual {
        match table.column(name) {
            None => issues.push(ColumnIssue::ExtraInData {
                column: name.clone(),
                actual: actual_type.clone(),
            }),
            Some(col) => {
                if let Some(declared) = &col.col_type
                    && !types_compatible(&declared.value, actual_type)
                {
                    issues.push(ColumnIssue::TypeMismatch {
                        column: name.clone(),
                        declared: declared.value.clone(),
                        actual: actual_type.clone(),
                    });
                }
            }
        }
    }

    for col in &table.columns {
        if !actual.iter().any(|(n, _)| n == &col.name.value) {
            issues.push(ColumnIssue::MissingInData {
                column: col.name.value.clone(),
            });
        }
    }

    issues
}

/// The outcome of validating one table against the data at its `source`.
#[derive(Debug)]
pub enum TableDataResult {
    /// The data was read and compared; the vector holds any discrepancies
    /// (empty means the data matches the dictionary).
    Compared(Vec<ColumnIssue>),
    /// The table declares no Parquet source, so there is nothing to read.
    NoParquetSource,
    /// The source path could not be read as a Parquet file.
    Unreadable { path: String, error: String },
}

/// Validate a single table against the Parquet file named in its `source`.
///
/// A relative `source.parquet` path is resolved against `base_dir` (the
/// directory containing the `data-dict.yaml` file). This does not re-validate
/// the dictionary; callers should do that first.
pub fn validate_table_source(table: &Table, base_dir: &Path) -> TableDataResult {
    let Some(raw) = table.source.as_ref().and_then(|s| s.parquet.as_ref()) else {
        return TableDataResult::NoParquetSource;
    };
    let candidate = Path::new(&raw.value);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base_dir.join(candidate)
    };
    match data_dict_parquet::column_types(&path) {
        Ok(actual) => TableDataResult::Compared(compare_columns(table, &actual)),
        Err(error) => TableDataResult::Unreadable {
            path: raw.value.clone(),
            error: error.to_string(),
        },
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
