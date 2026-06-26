//! Metadata-level validation: the data's column names and types match the
//! dictionary.
//!
//! This is the middle of the three validation levels (the `M##` checks; see
//! `site/validation.md` for what each code means). It reads only the data's
//! schema (e.g. a parquet footer), never its values, so it stays cheap.
//!
//! [`validate_meta`] is the entry point; [`meta_issues`] is the reusable core
//! that the data level ([`crate::validate_data`]) runs before its own value checks.

use std::path::Path;

use crate::model::{Column, DataDict, Table};
use crate::{ColumnIssue, Diagnostics, IssueKind, Level, ValidationError, ValidationReport};

/// Validate a parquet file's column names and types against a data dictionary.
///
/// Validates the spec first, then — when it is free of errors — compares the
/// parquet file's column schema against the selected table, reporting type
/// mismatches, columns described but absent from the data, and columns in the
/// data the dictionary does not describe. Values are never read; see
/// [`crate::validate_data::validate_data`] for the level that does.
pub fn validate_meta(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
) -> (Diagnostics, Result<ValidationReport, ValidationError>) {
    let (diagnostics, dict) = crate::validated_dict(dict_path);
    let result = match dict {
        Err(err) => Err(err),
        Ok(None) => Ok(ValidationReport::skipped(Level::Meta)),
        Ok(Some(dict)) => report(&dict, parquet_path, table),
    };
    (diagnostics, result)
}

/// Build the metadata-level report for one dataset: select the table, read its
/// column schema, and run the metadata checks.
fn report(
    dict: &DataDict,
    parquet_path: &Path,
    table: Option<&str>,
) -> Result<ValidationReport, ValidationError> {
    let table = crate::select_table(dict, table)?;
    let actual = data_dict_parquet::column_types(parquet_path)?;
    Ok(ValidationReport {
        table: table.name.value.clone(),
        level: Level::Meta,
        issues: meta_issues(table, &actual),
    })
}

/// Compare the dictionary's `table` against the actual column types read from
/// the data, returning the metadata-level issues. Reused by the data level,
/// which appends its value-level issues to the same list.
pub(crate) fn meta_issues(table: &Table, actual: &[(String, String)]) -> Vec<ColumnIssue> {
    let mut issues = Vec::new();

    // Columns the dictionary describes: each must exist in the data, and its
    // declared type (if any) must be compatible.
    for col in &table.columns {
        match actual.iter().find(|(n, _)| n == &col.name.value) {
            None => issues.push(ColumnIssue::error(
                col.name.value.clone(),
                IssueKind::MissingInData,
            )),
            // A column with no `type` makes no claims about its contents, so
            // `check_type` is naturally a no-op; only its existence is required.
            Some((_, actual_type)) => check_type(col, actual_type, &mut issues),
        }
    }

    // Columns present in the data that the dictionary does not describe.
    for (name, actual_type) in actual {
        if table.column(name).is_none() {
            issues.push(ColumnIssue::warning(
                name.clone(),
                IssueKind::ExtraInData {
                    actual: actual_type.clone(),
                },
            ));
        }
    }

    issues
}

/// A column's declared type must be compatible with the type read from the data.
fn check_type(col: &Column, actual_type: &str, issues: &mut Vec<ColumnIssue>) {
    if let Some(declared) = &col.col_type
        && !types_compatible(&declared.value, actual_type)
    {
        issues.push(ColumnIssue::error(
            col.name.value.clone(),
            IssueKind::TypeMismatch {
                declared: declared.value.clone(),
                actual: actual_type.to_string(),
            },
        ));
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
