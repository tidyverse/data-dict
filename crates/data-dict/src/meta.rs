//! Metadata-level validation: the data's column names and types match the
//! dictionary.
//!
//! This is the middle of the three validation levels. It reads only the data's
//! schema (e.g. a parquet footer), never its values, so it stays cheap. The
//! checks it owns are:
//!
//! - `M01` type mismatch — a column's declared type is incompatible with the data.
//! - `M02` missing column — a column the dictionary describes is absent from the data.
//! - `M03` undocumented column (warning) — a column in the data the dictionary omits.
//!
//! [`validate_meta`] is the entry point; [`meta_issues`] is the reusable core
//! that the data level ([`crate::data`]) runs before its own value checks.

use std::path::Path;

use crate::model::Column;
use crate::{ColumnIssue, CompareError, CompareReport, Diagnostics, IssueKind, Level};

/// Validate a parquet file's column names and types against a data dictionary.
///
/// Validates the dictionary first (schema + lint), then — when it is free of
/// errors — compares the parquet file's column schema against the selected
/// table, reporting type mismatches, columns described but absent from the data,
/// and columns in the data the dictionary does not describe. Values are never
/// read; see [`crate::data::validate_data`] for the level that does.
pub fn validate_meta(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
) -> (Diagnostics, Result<CompareReport, CompareError>) {
    crate::compare(dict_path, parquet_path, table, Level::Meta)
}

/// Compare the dictionary's `table` against the actual column types read from
/// the data, returning the metadata-level issues. Reused by the data level,
/// which appends its value-level issues to the same list.
pub(crate) fn meta_issues(
    table: &crate::model::Table,
    actual: &[(String, String)],
) -> Vec<ColumnIssue> {
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
