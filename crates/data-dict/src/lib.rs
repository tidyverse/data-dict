//! Core library for the `data-dict.yaml` specification.
//!
//! Validation happens at three levels, each a strict superset of the last (see
//! `site/validation.md`):
//!
//! 1. [`schema`] (`S##`) — the dictionary itself: well-formed and internally
//!    consistent. Never looks at the data.
//! 2. [`meta`] (`M##`) — the data's column names and types match the dictionary.
//!    Reads only the data's schema (e.g. a parquet footer).
//! 3. [`data`] (`D##`) — the data's values match the dictionary. Reads the data.
//!
//! Each level validates the schema first, then (for meta/data) compares the
//! dictionary against a dataset. This module holds the shared comparison
//! vocabulary ([`Level`], [`ColumnIssue`], [`CompareReport`], [`CompareError`])
//! and the [`compare`] orchestration the meta and data levels delegate to.

use std::path::Path;

use data_dict_parquet::ParquetError;

pub mod data;
pub mod diagnostic;
pub mod join_expr;
pub mod lower;
pub mod meta;
pub mod model;
pub mod schema;

pub use diagnostic::{Diagnostic, Diagnostics, Severity};
pub use quarto_source_map::SourceContext;
pub use schema::{validate_and_lower, validate_schema};

use model::{DataDict, Table};

/// The full text of the `data-dict.yaml` specification (`site/spec.md`),
/// embedded at compile time so the CLI can print it without a network or
/// filesystem dependency.
pub const SPEC_MD: &str = include_str!("../../../site/spec.md");

/// Errors returned by [`schema::validate_schema`].
#[derive(Debug)]
pub enum Error {
    /// I/O failure reading the document.
    Io(std::io::Error),
    /// The document is not parseable as YAML. Boxed because `quarto_yaml::Error`
    /// is large and would otherwise bloat every `Result` in this module.
    Parse(Box<quarto_yaml::Error>),
    /// The document failed structural and/or semantic validation. The string
    /// is a rendered, human-readable report covering every diagnostic, with
    /// source-location highlighting.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Invalid(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Parse(e) => Some(e),
            Error::Invalid(_) => None,
        }
    }
}

/// Which of the three validation levels a check or report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Schema,
    Meta,
    Data,
}

/// A single way in which a dataset disagrees with its data dictionary. Every
/// issue concerns one `column`, carries its rule `code` (e.g. `M01`, `D01`) and
/// a [`Severity`]; `kind` says what specifically is wrong.
///
/// The `serde` representation is the tool's JSON wire format: `column`, `code`,
/// `severity`, and the `kind`'s snake_case tag with its fields flattened
/// alongside (e.g. `{"column": "x", "code": "M01", "severity": "error", "kind":
/// "type_mismatch", "declared": ..., "actual": ...}`).
#[derive(Debug, serde::Serialize)]
pub struct ColumnIssue {
    pub column: String,
    pub code: &'static str,
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

impl IssueKind {
    /// The rule code for this kind of issue, prefixed by its validation level.
    pub fn code(&self) -> &'static str {
        match self {
            IssueKind::TypeMismatch { .. } => "M01",
            IssueKind::MissingInData => "M02",
            IssueKind::ExtraInData { .. } => "M03",
            IssueKind::NullsInRequired { .. } => "D01",
        }
    }

    /// The validation level this issue belongs to. Type and presence checks are
    /// metadata-level; value checks (e.g. nulls) are data-level.
    pub fn level(&self) -> Level {
        match self {
            IssueKind::NullsInRequired { .. } => Level::Data,
            _ => Level::Meta,
        }
    }
}

impl ColumnIssue {
    /// An error-severity issue: a hard mismatch that fails validation.
    pub(crate) fn error(column: impl Into<String>, kind: IssueKind) -> Self {
        ColumnIssue {
            column: column.into(),
            code: kind.code(),
            severity: Severity::Error,
            kind,
        }
    }

    /// A warning-severity issue: advisory drift that is reported but does not
    /// fail validation.
    pub(crate) fn warning(column: impl Into<String>, kind: IssueKind) -> Self {
        ColumnIssue {
            column: column.into(),
            code: kind.code(),
            severity: Severity::Warning,
            kind,
        }
    }

    /// The human-readable description of the issue, without code or severity.
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

/// The outcome of comparing a dataset against one table of a data dictionary at
/// a given [`Level`]: the table compared and every way the two disagree. Issues
/// carry their own [`Severity`]; the report fails validation only if some issue
/// is an error.
#[derive(Debug)]
pub struct CompareReport {
    pub table: String,
    pub level: Level,
    pub issues: Vec<ColumnIssue>,
}

impl CompareReport {
    /// A report for a comparison that did not run (e.g. the dictionary itself
    /// failed validation, so comparing data against it is not meaningful).
    fn skipped(level: Level) -> Self {
        CompareReport {
            table: String::new(),
            level,
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

impl std::fmt::Display for CompareReport {
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
            writeln!(f, "  {severity} [{}]: {}", issue.code, issue.body())?;
        }
        Ok(())
    }
}

/// Errors returned by the meta/data comparison entry points.
#[derive(Debug)]
pub enum CompareError {
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

impl From<Error> for CompareError {
    fn from(e: Error) -> Self {
        CompareError::Schema(e)
    }
}

impl From<ParquetError> for CompareError {
    fn from(e: ParquetError) -> Self {
        CompareError::Parquet(e)
    }
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareError::Schema(e) => write!(f, "{e}"),
            CompareError::Parquet(e) => write!(f, "{e}"),
            CompareError::TableNotFound { name, available } => {
                write!(
                    f,
                    "table \"{name}\" is not in the data dictionary (available: {})",
                    available.join(", ")
                )
            }
            CompareError::AmbiguousTable { available } => {
                write!(
                    f,
                    "the data dictionary describes multiple tables ({}); specify which one to validate against",
                    available.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for CompareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CompareError::Schema(e) => Some(e),
            CompareError::Parquet(e) => Some(e),
            _ => None,
        }
    }
}

/// Validate the schema, then compare a parquet file against the dictionary at
/// `level`. Shared by [`meta::validate_meta`] and [`data::validate_data`].
///
/// First validates the dictionary at `dict_path` (schema check). The data
/// comparison only runs when the dictionary itself is free of errors; otherwise
/// the error diagnostics already signal the failure and a skipped report is
/// returned. Returns the dictionary's [`Diagnostics`] and the comparison result.
pub(crate) fn compare(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
    level: Level,
) -> (Diagnostics, Result<CompareReport, CompareError>) {
    let (dict, diagnostics) = match schema::validate_and_lower(dict_path) {
        Ok(parsed) => parsed,
        Err(err) => return (Diagnostics::empty(), Err(err.into())),
    };
    let result = if diagnostics.has_errors() {
        Ok(CompareReport::skipped(level))
    } else {
        compare_dataset(&dict, parquet_path, table, level)
    };
    (diagnostics, result)
}

fn compare_dataset(
    dict: &DataDict,
    parquet_path: &Path,
    table: Option<&str>,
    level: Level,
) -> Result<CompareReport, CompareError> {
    let table = select_table(dict, table)?;

    // Metadata level: column names and types, from the parquet schema only.
    let actual = data_dict_parquet::column_types(parquet_path)?;
    let mut issues = meta::meta_issues(table, &actual);

    // Data level adds value checks, which scan the data.
    if level == Level::Data {
        data::value_issues(table, parquet_path, &actual, &mut issues)?;
    }

    Ok(CompareReport {
        table: table.name.value.clone(),
        level,
        issues,
    })
}

/// Resolve which table to compare against: `table` if given, otherwise the sole
/// table when the dictionary describes exactly one.
fn select_table<'a>(dict: &'a DataDict, table: Option<&str>) -> Result<&'a Table, CompareError> {
    let available = || dict.tables.keys().cloned().collect::<Vec<_>>();
    match table {
        Some(name) => dict
            .tables
            .get(name)
            .ok_or_else(|| CompareError::TableNotFound {
                name: name.to_string(),
                available: available(),
            }),
        None => {
            if dict.tables.len() == 1 {
                Ok(dict.tables.values().next().expect("len == 1"))
            } else {
                Err(CompareError::AmbiguousTable {
                    available: available(),
                })
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_json_flattens_kind_with_column_code_and_severity() {
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
                "code": "M01",
                "severity": "error",
                "kind": "type_mismatch",
                "declared": "string",
                "actual": "number",
            })
        );

        // A unit kind carries no extra fields beyond column/code/severity/kind.
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
                "code": "M03",
                "severity": "warning",
                "kind": "extra_in_data",
                "actual": "string",
            })
        );
    }

    #[test]
    fn issue_codes_and_levels() {
        assert_eq!(IssueKind::MissingInData.code(), "M02");
        assert_eq!(IssueKind::MissingInData.level(), Level::Meta);
        assert_eq!(
            IssueKind::NullsInRequired {
                count: 1,
                rows: vec![2]
            }
            .level(),
            Level::Data
        );
    }

    #[test]
    fn row_formatting() {
        assert_eq!(format_rows(&[2], 1), "rows: 2");
        assert_eq!(format_rows(&[2, 5, 9], 3), "rows: 2, 5, 9");
        // More nulls than the recorded sample gets an ellipsis.
        assert_eq!(format_rows(&[1, 2, 3, 4, 5], 8), "rows: 1, 2, 3, 4, 5, …");
    }
}
