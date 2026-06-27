//! Core library for the `data-dict.yaml` specification.
//!
//! Validation happens at three levels, each a strict superset of the last (see
//! `site/validation.md`):
//!
//! 1. [`validate_spec`] (`S##`) — the dictionary itself conforms to the
//!    data-dict spec: well-formed and internally consistent. Never looks at the
//!    data.
//! 2. [`validate_meta`] (`M##`) — the data's column names and types match the
//!    dictionary. Reads only the data's schema (e.g. a parquet footer).
//! 3. [`validate_data`] (`D##`) — the data's values match the dictionary. Reads
//!    the data.
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

/// The full text of the `data-dict.yaml` specification (`site/spec.md`),
/// embedded at compile time so the CLI can print it without a network or
/// filesystem dependency.
pub const SPEC_MD: &str = include_str!("../../../site/spec.md");

/// Which of the three validation levels a check belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Spec,
    Meta,
    Data,
}

/// Drive a metadata- or data-level comparison: initialise the run from
/// `dict_path`, validate the spec, select the table, read its column schema, then
/// run `checks` — the level-specific work — against the table and that schema.
///
/// The shared prologue (which every step bails out of on failure, reporting only
/// what it has) lives here so `validate_meta` and `validate_data` differ only in
/// the `checks` they pass.
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

/// Resolve which table to validate against: `table` if given, otherwise the sole
/// table when the dictionary describes exactly one. On failure pushes a
/// pre-flight [`Problem`] into `out` and returns `None`.
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
