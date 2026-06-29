//! Metadata-level validation, the `M##` checks (see `site/validation.md`).
//!
//! [`validate_meta`] is the entry point; [`meta_issues`] is the reusable core
//! that the data level ([`crate::validate_data`]) runs before its own value checks.

use std::path::Path;

use crate::model::Table;
use crate::problem::{Problem, ProblemKind, ProblemSet, Severity};

/// Validate a parquet file's column names and types against a data dictionary.
///
/// Validates the spec first, then — when it is free of errors — compares the
/// parquet file's column schema against the selected table, reporting type
/// mismatches, columns described but absent from the data, and columns in the
/// data the dictionary does not describe. Values are never read; see
/// [`crate::validate_data::validate_data`] for the level that does.
pub fn validate_meta(dict_path: &Path, parquet_path: &Path, table: Option<&str>) -> ProblemSet {
    crate::compare_dataset(dict_path, parquet_path, table, |table, actual, problems| {
        meta_issues(table, actual, problems);
    })
}

/// Compare the dictionary's `table` against the actual column types read from
/// the data, pushing the metadata-level problems into `out`. Reused by the data
/// level, which appends its value-level problems to the same set.
pub(crate) fn meta_issues(table: &Table, actual: &[(String, String)], out: &mut ProblemSet) {
    validate_m04_source(table, out);
    validate_m01_column_types(table, actual, out);
    validate_m02_missing_columns(table, actual, out);
    validate_m03_extra_columns(table, actual, out);
}

fn validate_m04_source(table: &Table, out: &mut ProblemSet) {
    if table.source.is_none() {
        out.push_located(
            ProblemKind::MissingSource,
            Severity::Error,
            "A table validated against data must declare a `source`.",
            "has no `source`",
            [table.name.span.clone()],
        );
    }
}

fn validate_m01_column_types(table: &Table, actual: &[(String, String)], out: &mut ProblemSet) {
    for col in &table.columns {
        // Only described columns present in the data are type-checked; an absent
        // column is M02's concern, and a column with no `type` makes no claims.
        let Some((_, actual_type)) = actual.iter().find(|(n, _)| n == &col.name.value) else {
            continue;
        };
        if let Some(declared) = &col.col_type
            && !types_compatible(&declared.value, actual_type)
        {
            out.push_located(
                ProblemKind::TypeMismatch {
                    declared: declared.value.clone(),
                    actual: actual_type.to_string(),
                },
                Severity::Error,
                "A column's data must match its declared type.",
                format!("the data is `{actual_type}`"),
                [
                    table.name.span.clone(),
                    col.name.span.clone(),
                    declared.span.clone(),
                ],
            );
        }
    }
}

fn validate_m02_missing_columns(table: &Table, actual: &[(String, String)], out: &mut ProblemSet) {
    for col in &table.columns {
        if !actual.iter().any(|(n, _)| n == &col.name.value) {
            out.push_located(
                ProblemKind::MissingInData,
                Severity::Error,
                "Every column in the dictionary must be present in the data.",
                "is missing from the data",
                [table.name.span.clone(), col.name.span.clone()],
            );
        }
    }
}

fn validate_m03_extra_columns(table: &Table, actual: &[(String, String)], out: &mut ProblemSet) {
    for (name, actual_type) in actual {
        if table.column(name).is_none() {
            // The column exists only in the data, so there is no dictionary node
            // to point at; it is named in the message instead.
            out.push(Problem::undocumented_column(name, actual_type.clone()));
        }
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
