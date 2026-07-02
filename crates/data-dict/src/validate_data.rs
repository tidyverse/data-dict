//! Data-level validation, the `D##` checks (see `site/validation.md`).
//!
//! [`validate_data`] is the entry point; `value_issues` is the value-checking
//! core it runs after the metadata checks ([`crate::validate_meta`]).

use std::collections::HashMap;
use std::path::Path;

use data_dict_parquet::{ColumnMeta, ColumnNeeds, ColumnStats};

use crate::model::{Column, Constraint, Table};
use crate::problem::{Problem, ProblemKind, ProblemSet, Severity};
use crate::validate_meta::CheckResult;

/// How many example values (e.g. offending rows) to record per validation
/// issue. Issues count every offender but only list this many.
const SAMPLE_LIMIT: usize = 5;

/// Validate a parquet file's values against a data dictionary.
///
/// Validates the spec first, then — when it is free of errors — runs every
/// metadata-level check ([`crate::validate_meta`]) plus the value-level checks
/// below: reading the columns and pages the checks imply and reporting, for
/// example, nulls in a required column.
pub fn validate_data(dict_path: &Path, table: Option<&str>) -> ProblemSet {
    crate::compare_dataset(dict_path, table, |table, parquet_path, actual, problems| {
        crate::validate_meta::meta_issues(table, actual, problems);
        if let Err(e) = value_issues(table, parquet_path, actual, problems) {
            problems.push(Problem::preflight(ProblemKind::Parquet, e.to_string()));
        }
    })
}

/// Run the value-level checks for the dictionary's `table` against the data,
/// pushing any problems found into `out`. `actual` is the column schema already
/// read for the metadata checks, used here only to tell which columns are
/// present.
fn value_issues(
    table: &Table,
    parquet_path: &Path,
    actual: &[(String, String)],
    out: &mut ProblemSet,
) -> Result<(), data_dict_parquet::ParquetError> {
    let present = |name: &str| actual.iter().any(|(an, _)| an == name);
    let metadata = data_dict_parquet::column_meta(parquet_path)?;

    // Phase 1 — check the footer. A data-level rule remains D## even when
    // Parquet metadata is sufficient to prove its result. Only inconclusive
    // checks are allowed to request a value scan.
    let mut needs: HashMap<String, ColumnNeeds> = HashMap::new();
    let mut pending: HashMap<String, Vec<&dyn ColumnCheck>> = HashMap::new();
    for col in &table.columns {
        if !present(&col.name.value) {
            continue;
        }
        let Some(meta) = metadata.get(&col.name.value) else {
            continue;
        };
        let mut merged = ColumnNeeds::default();
        for check in VALUE_CHECKS {
            match check.check_meta(table, col, meta) {
                CheckResult::Pass => {}
                CheckResult::Inconclusive => {
                    merged = merged.merge(check.needs(col));
                    pending
                        .entry(col.name.value.clone())
                        .or_default()
                        .push(*check);
                }
                CheckResult::Fail(problem) => out.push(*problem),
            }
        }
        if merged.any() {
            needs.insert(col.name.value.clone(), merged);
        }
    }

    // Phase 2 — scan. Gather exactly those statistics, in one pass, reading only
    // the columns and pages the plan implies.
    let stats = data_dict_parquet::column_stats(parquet_path, &needs, SAMPLE_LIMIT)?;

    // Phase 3 — check. Per column with gathered stats, run the value-level checks.
    for col in &table.columns {
        if let Some(stat) = stats.get(&col.name.value) {
            for check in pending.get(&col.name.value).into_iter().flatten() {
                if let Some(problem) = check.check_data(table, col, stat) {
                    out.push(problem);
                }
            }
        }
    }

    Ok(())
}

/// A value-level column check, split into the data it needs and the verdict it
/// draws from that data. Keeping the two together (rather than in the
/// orchestrator) lets the scanner compute the union of all checks' needs in a
/// single pass, and lets a new check be added without touching the pipeline.
trait ColumnCheck {
    /// Attempt the check from footer metadata alone.
    fn check_meta(&self, table: &Table, col: &Column, meta: &ColumnMeta) -> CheckResult;

    /// What this check needs read from the column's data. Returning the default
    /// (nothing requested) opts the column out of this check.
    fn needs(&self, col: &Column) -> ColumnNeeds;

    /// Draw a verdict from the gathered stats. Only ever called with stats whose
    /// requested fields this check (or another) asked for. `table` is passed for
    /// locating the finding at the column's node in the dictionary.
    /// Complete an inconclusive metadata check from scanned values. `None` is
    /// pass and `Some` is fail; data checks cannot remain inconclusive.
    fn check_data(&self, table: &Table, col: &Column, stats: &ColumnStats) -> Option<Problem>;
}

/// Every value-level check, run against each present column. Add a check here
/// and the plan/scan/check pipeline picks it up automatically.
const VALUE_CHECKS: &[&dyn ColumnCheck] = &[&RequiredNotNull];

/// D01 — a `required` (or `primary_key`) column must contain no nulls.
struct RequiredNotNull;

impl ColumnCheck for RequiredNotNull {
    fn check_meta(&self, table: &Table, col: &Column, meta: &ColumnMeta) -> CheckResult {
        crate::validate_meta::validate_d01_required_not_null(table, col, meta)
    }

    fn needs(&self, col: &Column) -> ColumnNeeds {
        ColumnNeeds {
            nulls: col.is_required_implied(),
        }
    }

    fn check_data(&self, table: &Table, col: &Column, stats: &ColumnStats) -> Option<Problem> {
        // Nulls are only counted when this check requested them (i.e. the column
        // is required), so a positive count is exactly a violation.
        if stats.null_count == 0 {
            return None;
        }
        Some(nulls_in_required_data(
            table,
            col,
            stats.null_count,
            stats.null_rows.clone(),
        ))
    }
}

fn nulls_in_required_data(table: &Table, col: &Column, count: usize, rows: Vec<usize>) -> Problem {
    let detail = crate::problem::format_rows(&rows, count);
    let plural = if count == 1 { "" } else { "s" };
    let constraint_span = col
        .constraints
        .iter()
        .find(|constraint| {
            matches!(
                constraint.value,
                Constraint::Required | Constraint::PrimaryKey
            )
        })
        .map_or_else(
            || col.name.span.clone(),
            |constraint| constraint.span.clone(),
        );
    Problem {
        code: Some("D01"),
        severity: Severity::Error,
        message: format!("has {count} null value{plural} ({detail})"),
        column: None,
        expected: Some("A required column must not contain nulls.".into()),
        hint: None,
        suggestion: None,
        context: vec![
            table.name.span.clone(),
            col.name.span.clone(),
            constraint_span,
        ],
        kind: ProblemKind::NullsInRequired { count, rows },
    }
}
