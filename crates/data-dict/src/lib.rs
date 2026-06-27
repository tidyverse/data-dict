//! Core library for the `data-dict.yaml` specification.
//!
//! Validation happens at three levels — [`validate_spec`], [`validate_meta`],
//! [`validate_data`] — each a strict superset of the last; `site/validation.md`
//! defines them and their `S##`/`M##`/`D##` checks.
//!
//! Every level reports its findings as a single [`ProblemSet`]: one vector of
//! [`Problem`]s, whatever their origin (I/O, the schema, a spec check, a
//! metadata or data mismatch). A level pushes its problems and stops the run
//! short by returning early; the meta and data levels validate the spec first
//! and compare against a dataset only when it is free of errors. This module
//! holds the shared [`Level`], the [`select_table`] helper, and the
//! `compare_dataset` driver the meta and data levels build on.

use std::path::Path;

pub mod join_expr;
pub mod lower;
pub mod model;
pub mod problem;
pub mod validate_data;
pub mod validate_meta;
pub mod validate_spec;

pub use problem::{Problem, ProblemKind, ProblemSet, Severity, Status};
pub use quarto_source_map::SourceContext;
pub use validate_data::validate_data;
pub use validate_meta::validate_meta;
pub use validate_spec::validate_spec;
pub(crate) use validate_spec::{load, validate_and_lower};

use model::{DataDict, Table};

pub const SPEC_MD: &str = include_str!("../../../site/spec.md");

/// Which of the three validation levels a check belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Spec,
    Meta,
    Data,
}

/// The shared prologue for `validate_meta` and `validate_data`, so they differ
/// only in the `checks` they pass. Each step bails out on failure, reporting only
/// what it has by then.
pub(crate) fn compare_dataset(
    dict_path: &Path,
    parquet_path: &Path,
    table: Option<&str>,
    checks: impl FnOnce(&Table, &[(String, String)], &mut ProblemSet),
) -> ProblemSet {
    let (mut problems, doc) = match load(dict_path) {
        Ok(loaded) => loaded,
        Err(problems) => return problems,
    };
    let Some(dict) = validate_and_lower(&doc, &mut problems) else {
        return problems;
    };
    let Some(table) = select_table(&dict, table, &mut problems) else {
        return problems;
    };
    let actual = match data_dict_parquet::column_types(parquet_path) {
        Ok(actual) => actual,
        Err(e) => {
            problems.push(Problem::preflight(ProblemKind::Parquet, e.to_string()));
            return problems;
        }
    };
    checks(table, &actual, &mut problems);
    problems
}

/// Falls back to the sole table when `table` is `None` and the dictionary
/// describes exactly one; otherwise records why in `out` and returns `None`.
pub(crate) fn select_table<'a>(
    dict: &'a DataDict,
    table: Option<&str>,
    out: &mut ProblemSet,
) -> Option<&'a Table> {
    let available = || dict.tables.keys().cloned().collect::<Vec<_>>();
    match table {
        Some(name) => match dict.tables.get(name) {
            Some(table) => Some(table),
            None => {
                let available = available();
                out.push(Problem::preflight(
                    ProblemKind::TableNotFound {
                        available: available.clone(),
                    },
                    format!(
                        "table \"{name}\" is not in the data dictionary (available: {})",
                        available.join(", ")
                    ),
                ));
                None
            }
        },
        None => {
            if dict.tables.len() == 1 {
                Some(dict.tables.values().next().expect("len == 1"))
            } else {
                let available = available();
                out.push(Problem::preflight(
                    ProblemKind::AmbiguousTable {
                        available: available.clone(),
                    },
                    format!(
                        "the data dictionary describes multiple tables ({}); specify which one to validate against",
                        available.join(", ")
                    ),
                ));
                None
            }
        }
    }
}
