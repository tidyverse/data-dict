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
//! holds the shared [`Level`] and the [`select_table`] helper the meta and data
//! levels build on.

pub mod join_expr;
pub mod lower;
pub mod model;
pub mod problem;
pub mod validate_data;
pub mod validate_meta;
pub mod validate_spec;

pub use problem::{Problem, ProblemKind, ProblemSet, Severity};
pub use quarto_source_map::SourceContext;
pub use validate_data::validate_data;
pub use validate_meta::validate_meta;
pub use validate_spec::{validate_and_lower, validate_spec};

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
