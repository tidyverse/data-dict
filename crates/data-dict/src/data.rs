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

use data_dict_parquet::{ColumnNeeds, ColumnStats, ParquetError};

use crate::model::Column;
use crate::{DataDict, Diagnostics, Error, Severity};

/// How many example values (e.g. offending rows) to record per validation
/// issue. Issues count every offender but only list this many.
const SAMPLE_LIMIT: usize = 5;

/// A single way in which a dataset disagrees with its data dictionary. Every
/// issue concerns one `column` and carries its own [`Severity`]; `kind` says
/// what specifically is wrong.
///
/// The `serde` representation is the tool's JSON wire format: the `kind`'s
/// snake_case tag and its fields are flattened alongside `column` and
/// `severity` (e.g. `{"column": "x", "severity": "error", "kind":
/// "type_mismatch", "declared": ..., "actual": ...}`).
#[derive(Debug, serde::Serialize)]
pub struct ColumnIssue {
    pub column: String,
    pub severity: Severity,
    #[serde(flatten)]
    pub kind: IssueKind,
}

/// What is wrong with a column — the payload behind a [`ColumnIssue`].
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueKind {
    /// The column is present in both, but its declared type is not compatible
    /// with the type read from the data.
    TypeMismatch { declared: String, actual: String },
    /// The column is described by the dictionary but absent from the data.
    MissingInData,
    /// The column is present in the data but the dictionary does not describe it.
    ExtraInData { actual: String },
    /// The column is marked `required` (or `primary_key`) but contains null
    /// values. `rows` lists the first few offending row numbers (1-based);
    /// `count` is the true total.
    NullsInRequired { count: usize, rows: Vec<usize> },
}

impl ColumnIssue {
    /// An error-severity issue: a hard mismatch that fails validation.
    fn error(column: impl Into<String>, kind: IssueKind) -> Self {
        ColumnIssue {
            column: column.into(),
            severity: Severity::Error,
            kind,
        }
    }

    /// A warning-severity issue: advisory drift that is reported but does not
    /// fail validation.
    fn warning(column: impl Into<String>, kind: IssueKind) -> Self {
        ColumnIssue {
            column: column.into(),
            severity: Severity::Warning,
            kind,
        }
    }

    /// The human-readable description of the issue, without any severity prefix.
    fn body(&self) -> String {
        let column = &self.column;
        match &self.kind {
            IssueKind::TypeMismatch { declared, actual } => {
                format!("column \"{column}\": declared {declared}, data is {actual}")
            }
            IssueKind::MissingInData => {
                format!("column \"{column}\": described in dictionary but missing from data")
            }
            IssueKind::ExtraInData { actual } => {
                format!("column \"{column}\": present in data ({actual}) but not in dictionary")
            }
            IssueKind::NullsInRequired { count, rows } => format!(
                "column \"{column}\": required but has {count} null value{} ({})",
                if *count == 1 { "" } else { "s" },
                format_rows(rows, *count),
            ),
        }
    }
}

/// The outcome of comparing a dataset against one table of a data dictionary:
/// the table compared and every way the two disagree. Issues carry their own
/// [`Severity`]; the report fails validation only if some issue is an error.
#[derive(Debug)]
pub struct DataReport {
    pub table: String,
    pub issues: Vec<ColumnIssue>,
}

impl DataReport {
    /// A report for a comparison that did not run (e.g. the dictionary itself
    /// failed validation, so comparing data against it is not meaningful).
    fn skipped() -> Self {
        DataReport {
            table: String::new(),
            issues: Vec::new(),
        }
    }

    /// Whether any issue is an error. Warning-only reports (e.g. an undocumented
    /// column) do not fail validation.
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Whether the dataset matched the dictionary with nothing to report.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

impl std::fmt::Display for DataReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.issues.is_empty() {
            return Ok(());
        }
        writeln!(f, "data does not match table \"{}\":", self.table)?;
        for issue in &self.issues {
            let severity = match issue.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            writeln!(f, "  {severity}: {}", issue.body())?;
        }
        Ok(())
    }
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
///
/// Returns the dictionary's [`Diagnostics`] (errors and warnings, in emission
/// order, with the source context to render them) and the data-validation
/// result. The data comparison only runs when the dictionary itself is free of
/// errors. Validation has failed if either an error diagnostic, an `Err`
/// result, or a [`DataReport`] with error-severity issues is present.
pub fn validate_parquet(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
) -> (Diagnostics, Result<DataReport, DataError>) {
    let (dict, diagnostics) = match crate::validate_and_lower(dict_path) {
        Ok(parsed) => parsed,
        Err(err) => return (Diagnostics::empty(), Err(err.into())),
    };
    // Comparing data against a dictionary that fails its own validation isn't
    // meaningful, so skip it; the error diagnostics already signal the failure.
    let result = if diagnostics.has_errors() {
        Ok(DataReport::skipped())
    } else {
        compare_parquet_to_dict(&dict, parquet_path, table)
    };
    (diagnostics, result)
}

fn compare_parquet_to_dict(
    dict: &DataDict,
    parquet_path: &Path,
    table: Option<&str>,
) -> Result<DataReport, DataError> {
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

    let present = |name: &str| actual.iter().any(|(an, _)| an == name);

    // Phase 1 — plan. Ask every value-level check what it needs of each present
    // column, and union those needs. This is the only place check-specific data
    // requirements are expressed.
    let mut needs: HashMap<String, ColumnNeeds> = HashMap::new();
    for col in &table.columns {
        if !present(&col.name.value) {
            continue;
        }
        let merged = VALUE_CHECKS
            .iter()
            .fold(ColumnNeeds::default(), |acc, check| {
                acc.merge(check.needs(col))
            });
        if merged.any() {
            needs.insert(col.name.value.clone(), merged);
        }
    }

    // Phase 2 — scan. Gather exactly those statistics, in one pass, reading only
    // the columns and pages the plan implies.
    let stats = data_dict_parquet::column_stats(parquet_path, &needs, SAMPLE_LIMIT)?;

    // Phase 3 — check. Per column: report absence/type structurally, then run the
    // value-level checks against the gathered stats.
    let mut issues = Vec::new();
    for col in &table.columns {
        let Some((_, actual_type)) = actual.iter().find(|(n, _)| n == &col.name.value) else {
            issues.push(ColumnIssue::error(
                col.name.value.clone(),
                IssueKind::MissingInData,
            ));
            continue;
        };
        // A column with no `type` makes no claims about its contents, so
        // `check_type` and the value checks below are naturally no-ops for it;
        // only its existence (checked above) is required.
        check_type(col, actual_type, &mut issues);
        if let Some(stat) = stats.get(&col.name.value) {
            for check in VALUE_CHECKS {
                check.check(col, stat, &mut issues);
            }
        }
    }

    // Columns present in the data that the dictionary does not describe.
    for (name, actual_type) in &actual {
        if table.column(name).is_none() {
            issues.push(ColumnIssue::warning(
                name.clone(),
                IssueKind::ExtraInData {
                    actual: actual_type.clone(),
                },
            ));
        }
    }

    Ok(DataReport {
        table: table.name.value.clone(),
        issues,
    })
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

/// A value-level column check, split into the data it needs and the verdict it
/// draws from that data. Keeping the two together (rather than in the
/// orchestrator) lets the scanner compute the union of all checks' needs in a
/// single pass, and lets a new check be added without touching the pipeline.
trait ColumnCheck {
    /// What this check needs read from the column's data. Returning the default
    /// (nothing requested) opts the column out of this check.
    fn needs(&self, col: &Column) -> ColumnNeeds;

    /// Draw a verdict from the gathered stats. Only ever called with stats whose
    /// requested fields this check (or another) asked for.
    fn check(&self, col: &Column, stats: &ColumnStats, issues: &mut Vec<ColumnIssue>);
}

/// Every value-level check, run against each present column. Add a check here
/// and the plan/scan/check pipeline picks it up automatically.
const VALUE_CHECKS: &[&dyn ColumnCheck] = &[&RequiredNotNull];

/// A `required` (or `primary_key`) column must contain no nulls.
struct RequiredNotNull;

impl ColumnCheck for RequiredNotNull {
    fn needs(&self, col: &Column) -> ColumnNeeds {
        ColumnNeeds {
            nulls: col.is_required_implied(),
        }
    }

    fn check(&self, col: &Column, stats: &ColumnStats, issues: &mut Vec<ColumnIssue>) {
        // Nulls are only counted when this check requested them (i.e. the column
        // is required), so a positive count is exactly a violation.
        if stats.null_count > 0 {
            issues.push(ColumnIssue::error(
                col.name.value.clone(),
                IssueKind::NullsInRequired {
                    count: stats.null_count,
                    rows: stats.null_rows.clone(),
                },
            ));
        }
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
    fn issue_json_flattens_kind_with_column_and_severity() {
        let issue = ColumnIssue::error(
            "weight",
            IssueKind::TypeMismatch {
                declared: "string".into(),
                actual: "number".into(),
            },
        );
        assert_eq!(
            serde_json::to_value(&issue).unwrap(),
            serde_json::json!({
                "column": "weight",
                "severity": "error",
                "kind": "type_mismatch",
                "declared": "string",
                "actual": "number",
            })
        );

        // A unit kind carries no extra fields beyond `column`/`severity`/`kind`.
        let extra = ColumnIssue::warning(
            "notes",
            IssueKind::ExtraInData {
                actual: "string".into(),
            },
        );
        assert_eq!(
            serde_json::to_value(&extra).unwrap(),
            serde_json::json!({
                "column": "notes",
                "severity": "warning",
                "kind": "extra_in_data",
                "actual": "string",
            })
        );
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
