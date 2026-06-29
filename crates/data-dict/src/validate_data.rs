//! Data-level validation, the `D##` checks (see `site/validation.md`).
//!
//! [`validate_data`] is the entry point; `value_issues` is the value-checking
//! core it runs after the metadata checks ([`crate::validate_meta`]).

use std::collections::HashMap;
use std::path::Path;

use data_dict_parquet::{ColumnNeeds, ColumnStats};

use crate::model::{Column, Table};
use crate::problem::{Problem, ProblemKind, ProblemSet, Severity};

/// How many example values (e.g. offending rows) to record per validation
/// issue. Issues count every offender but only list this many.
const SAMPLE_LIMIT: usize = 5;

/// Validate a parquet file's values against a data dictionary.
///
/// Validates the spec first, then — when it is free of errors — runs every
/// metadata-level check ([`crate::validate_meta`]) plus the value-level checks
/// below: reading the columns and pages the checks imply and reporting, for
/// example, nulls in a required column.
pub fn validate_data(dict_path: &Path, parquet_path: &Path, table: Option<&str>) -> ProblemSet {
    crate::compare_dataset(dict_path, parquet_path, table, |table, actual, problems| {
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

    // Phase 3 — check. Per column with gathered stats, run the value-level checks.
    for col in &table.columns {
        if let Some(stat) = stats.get(&col.name.value) {
            for check in VALUE_CHECKS {
                check.check(col, stat, out);
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
    /// What this check needs read from the column's data. Returning the default
    /// (nothing requested) opts the column out of this check.
    fn needs(&self, col: &Column) -> ColumnNeeds;

    /// Draw a verdict from the gathered stats. Only ever called with stats whose
    /// requested fields this check (or another) asked for.
    fn check(&self, col: &Column, stats: &ColumnStats, out: &mut ProblemSet);
}

/// Every value-level check, run against each present column. Add a check here
/// and the plan/scan/check pipeline picks it up automatically.
const VALUE_CHECKS: &[&dyn ColumnCheck] = &[&RequiredNotNull];

/// D01 — a `required` (or `primary_key`) column must contain no nulls.
struct RequiredNotNull;

impl ColumnCheck for RequiredNotNull {
    fn needs(&self, col: &Column) -> ColumnNeeds {
        ColumnNeeds {
            nulls: col.is_required_implied(),
        }
    }

    fn check(&self, col: &Column, stats: &ColumnStats, out: &mut ProblemSet) {
        // Nulls are only counted when this check requested them (i.e. the column
        // is required), so a positive count is exactly a violation.
        if stats.null_count > 0 {
            out.push(Problem::column(
                Severity::Error,
                col.name.value.clone(),
                ProblemKind::NullsInRequired {
                    count: stats.null_count,
                    rows: stats.null_rows.clone(),
                },
            ));
        }
    }
}
