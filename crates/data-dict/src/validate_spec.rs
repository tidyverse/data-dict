//! Spec-level validation: the dictionary itself conforms to the data-dict spec.
//!
//! The first of the three validation levels (the `S##` checks; see
//! `site/validation.md` for what each code means). [`validate_spec`] runs two
//! internal passes on a `data-dict.yaml` document — a distinction not surfaced
//! in the CLI:
//!
//! 1. **schema**: structural validation against the embedded `schema.yaml` via
//!    the `quarto-yaml-validation` crate — everything a JSON Schema can express.
//! 2. **spec**: the cross-table semantic checks below that the schema can't
//!    express (foreign-key targets, `join` parsing, cardinality, …).
//!
//! The second pass only runs if the first succeeds: there is no point chasing
//! FK references in a document whose `tables` block is malformed. The checks can
//! also surface *warnings* (e.g. a missing `$learn_more` key), which do not fail
//! validation.
//!
//! This level never looks at the data. The [`crate::validate_meta`] and
//! [`crate::validate_data`] levels build on it: both validate the spec first and
//! only compare against a dataset when the spec is free of errors.

use std::path::Path;
use std::sync::OnceLock;

use quarto_yaml::YamlWithSourceInfo;
use quarto_yaml_validation::{Schema, SchemaRegistry, ValidationDiagnostic};

use crate::join_expr::{JoinExpr, QCol};
use crate::model::{Cardinality, DataDict};
use crate::problem::{Problem, ProblemKind, ProblemSet, Severity, subspan};
use crate::{SourceContext, lower};

/// The canonical documentation URL suggested for `$learn_more`.
pub const LEARN_MORE_URL: &str = "http://data-dict.tidyverse.org/";

const SCHEMA_YAML: &str = include_str!("../../../schema.yaml");

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let yaml =
            quarto_yaml::parse(SCHEMA_YAML).expect("embedded schema.yaml must be parseable YAML");
        Schema::from_yaml(&yaml).expect("embedded schema.yaml must compile to a valid schema")
    })
}

/// Validate a `data-dict.yaml` file at `path`: structural schema check followed
/// by the cross-table semantic checks. Returns a [`ProblemSet`] — every problem
/// (errors and warnings, in source order) bundled with the source context needed
/// to render them. [`ProblemSet::status`] reports whether the document is valid.
///
/// Failures that prevent checking altogether — I/O, unparseable YAML, a
/// structurally invalid document — are themselves reported as pre-flight
/// [`Problem`]s in the set.
pub fn validate_spec(path: &Path) -> ProblemSet {
    // The dictionary is only of interest to the meta/data levels; here we want
    // the problems whether or not the spec turned out to be usable.
    match validate_and_lower(path) {
        Ok((_, problems)) | Err(problems) => problems,
    }
}

/// Validate a `data-dict.yaml` file at `path` and return the lowered
/// [`DataDict`] alongside its [`ProblemSet`]. Initialises the run with [`load`],
/// then steps through the spec-level checks.
///
/// `Ok((dict, problems))` when the spec is usable — the document lowered and the
/// semantic checks found no errors (`problems` may still hold warnings). `Err`
/// when it is not: a pre-flight failure (I/O, unparseable YAML, a document the
/// schema rejected) for which no dictionary was ever built, or semantic errors
/// that make comparing data against the dictionary meaningless. Either way the
/// `Err` set carries the problems that explain the outcome.
pub fn validate_and_lower(path: &Path) -> Result<(DataDict, ProblemSet), ProblemSet> {
    let (mut problems, doc) = load(path);
    let Some(doc) = doc else {
        return Err(problems);
    };
    let dict = lower::lower(&doc, &mut problems);
    check_spec(&dict, &mut problems);
    check_learn_more(&doc, &mut problems);
    problems.sort();

    if problems.status().failed() {
        Err(problems)
    } else {
        Ok((dict, problems))
    }
}

/// Read, parse, and schema-check the document at `path`, creating the run's
/// [`ProblemSet`] with the document's source — this is where every level starts.
/// Returns the parsed AST when the document is structurally sound; on a
/// pre-flight failure (I/O, unparseable YAML, or a document the schema rejects)
/// the failure is recorded in the set and `None` is returned.
fn load(path: &Path) -> (ProblemSet, Option<YamlWithSourceInfo>) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => {
            return (
                ProblemSet::from_preflight(ProblemKind::Io, e.to_string()),
                None,
            );
        }
    };
    let filename = path.display().to_string();

    let doc = match quarto_yaml::parse_file(&content, &filename) {
        Ok(doc) => doc,
        Err(e) => {
            return (
                ProblemSet::from_preflight(ProblemKind::Parse, e.to_string()),
                None,
            );
        }
    };

    let mut source = SourceContext::new();
    let file_id = quarto_yaml::file_id_for_filename(&filename);
    source.add_file_with_id(file_id, filename, Some(content));

    let registry = SchemaRegistry::new();
    if let Err(err) = quarto_yaml_validation::validate(&doc, schema(), &registry, &source) {
        let diagnostic = ValidationDiagnostic::from_validation_error(&err, &source);
        // The structural diagnostic is already rendered (with source
        // highlighting) into its message; the pre-flight problem carries it as-is.
        let message = diagnostic.to_text(&source);
        let mut problems = ProblemSet::new(source);
        problems.push(Problem::preflight(ProblemKind::Schema, message));
        return (problems, None);
    }

    (ProblemSet::new(source), Some(doc))
}

/// Run every rule, pushing any findings into `out`. Rules run in code order;
/// call [`ProblemSet::sort`] afterwards to put the findings in source order.
fn check_spec(dict: &DataDict, out: &mut ProblemSet) {
    check_relationship_table_refs(dict, out); // S02
    check_relationship_column_refs(dict, out); // S03
    check_join_table_count(dict, out); // S04
    check_foreign_keys_resolve(dict, out); // S01
    check_conflicts_present_on_both_sides(dict, out); // S05
    check_cardinality_consistency(dict, out); // S06
    check_column_data_representation(dict, out); // S07
    check_units_only_on_quantity(dict, out); // S08
}

// --- S02 --------------------------------------------------------------

fn check_relationship_table_refs(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        for q in join.qcols() {
            if !dict.tables.contains_key(&q.table) {
                let span = subspan(&rel.join_text.span, q.start, q.end)
                    .unwrap_or_else(|| rel.join_text.span.clone());
                out.push(Problem::spec(
                    "S02",
                    Severity::Error,
                    format!(
                        "relationship references table `{}`, which is not defined in `tables`",
                        q.table
                    ),
                    span,
                ));
            }
        }
    }
}

// --- S03 --------------------------------------------------------------

fn check_relationship_column_refs(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        if let Some(join) = &rel.join {
            for q in join.qcols() {
                // Skip if the table doesn't exist — S02 handles that case
                // and a column report would be noise.
                let Some(table) = dict.tables.get(&q.table) else {
                    continue;
                };
                if table.column(&q.column).is_none() {
                    let span = subspan(&rel.join_text.span, q.start, q.end)
                        .unwrap_or_else(|| rel.join_text.span.clone());
                    out.push(Problem::spec(
                        "S03",
                        Severity::Error,
                        format!(
                            "column `{}` is not defined in table `{}`",
                            q.column, q.table
                        ),
                        span,
                    ));
                }
            }
        }
        // `conflicts` column references are checked by S05 alongside the
        // "appears on both sides" check, so a missing column there reports
        // the more specific message.
    }
}

// --- S04 --------------------------------------------------------------

fn check_join_table_count(dict: &DataDict, out: &mut ProblemSet) {
    // Parse failures are emitted during lowering. Here we only check the
    // table-count invariant on successfully parsed joins.
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        if tables.is_empty() || tables.len() > 2 {
            out.push(Problem::spec(
                "S04",
                Severity::Error,
                format!(
                    "`join` must reference exactly one (self-join) or two tables; found {}",
                    tables.len()
                ),
                rel.join_text.span.clone(),
            ));
        }
    }
}

// --- S01 --------------------------------------------------------------

fn check_foreign_keys_resolve(dict: &DataDict, out: &mut ProblemSet) {
    use crate::model::Constraint::*;

    for (table_name, table) in &dict.tables {
        for col in &table.columns {
            if !col.has(ForeignKey) {
                continue;
            }
            let satisfied = dict.relationships.iter().any(|rel| {
                let Some(join) = &rel.join else { return false };
                // The FK column must appear on one side of some conjunct,
                // and the corresponding other side must carry PrimaryKey.
                join.conjuncts.iter().any(|conj| {
                    let sides = [(&conj.lhs, &conj.rhs), (&conj.rhs, &conj.lhs)];
                    sides.iter().any(|(fk_side, pk_side)| {
                        if fk_side.table != *table_name || fk_side.column != col.name.value {
                            return false;
                        }
                        let Some(other_tbl) = dict.tables.get(&pk_side.table) else {
                            return false;
                        };
                        let Some(other_col) = other_tbl.column(&pk_side.column) else {
                            return false;
                        };
                        other_col.has(PrimaryKey)
                    })
                })
            });
            if !satisfied {
                out.push(
                    Problem::spec(
                        "S01",
                        Severity::Error,
                        format!(
                        "column `{}.{}` is marked `foreign_key` but no `relationships` entry points it at a `primary_key` column",
                        table_name, col.name.value
                    ),
                        col.name.span.clone(),
                    ),
                );
            }
        }
    }
}

// --- S05 --------------------------------------------------------------

fn check_conflicts_present_on_both_sides(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        if rel.conflicts.is_empty() {
            continue;
        }
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        // For a self-join, the "both sides" reduces to the single table; for
        // a normal join, both tables must contain the column.
        for c in &rel.conflicts {
            let mut missing_from: Vec<&str> = Vec::new();
            for t_name in &tables {
                let Some(table) = dict.tables.get(*t_name) else {
                    // S02 already flagged the missing table; skip to avoid
                    // a cascade of confusing reports.
                    continue;
                };
                if table.column(&c.value).is_none() {
                    missing_from.push(*t_name);
                }
            }
            if !missing_from.is_empty() {
                out.push(Problem::spec(
                    "S05",
                    Severity::Error,
                    format!(
                        "`conflicts` entry `{}` is not a column of {}",
                        c.value,
                        join_with_commas(&missing_from)
                    ),
                    c.span.clone(),
                ));
            }
        }
    }
}

fn join_with_commas(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("`{s}`")).collect();
    match quoted.len() {
        0 => String::new(),
        1 => quoted[0].clone(),
        _ => {
            let (last, init) = quoted.split_last().unwrap();
            format!("{} and {}", init.join(", "), last)
        }
    }
}

// --- S06 --------------------------------------------------------------

fn check_cardinality_consistency(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };

        // Skip if any join column references a missing table or column. The
        // missing reference is already reported (S02 / S03), and checking
        // cardinality against a column that doesn't exist would just produce a
        // redundant, confusing S06.
        let all_cols_resolve = join.qcols().all(|q| {
            dict.tables
                .get(&q.table)
                .is_some_and(|t| t.column(&q.column).is_some())
        });
        if !all_cols_resolve {
            continue;
        }

        // The cardinality rule is defined in terms of the LHS and RHS tables
        // of the join. With multi-conjunct joins (date-range overlap), the
        // LHS and RHS tables are the same across all conjuncts, so we can
        // use the first conjunct as the canonical orientation.
        let Some(first) = join.conjuncts.first() else {
            continue;
        };
        let lhs_table = first.lhs.table.clone();
        let rhs_table = first.rhs.table.clone();

        // Which columns are "the join side" for each table?  For the
        // single-conjunct equality case this is straightforward. For
        // multi-conjunct (range) joins we require ALL conjunct columns on the
        // "one" side to be jointly unique-implied — in practice users
        // typically mark just one of them as PK/unique. We err on the
        // permissive side and check whether *any* column on the "one" side
        // is unique-implied; that matches the loose intuition behind range
        // joins without producing noise for legitimate overlap joins.

        let lhs_cols_unique =
            side_has_unique_implied(dict, &lhs_table, join, /* use_lhs = */ true);
        let rhs_cols_unique =
            side_has_unique_implied(dict, &rhs_table, join, /* use_lhs = */ false);

        let card_span = rel.cardinality.span.clone();
        match rel.cardinality.value {
            Cardinality::OneToOne => {
                if !lhs_cols_unique || !rhs_cols_unique {
                    out.push(
                        Problem::spec(
                            "S06",
                            Severity::Error,
                            format!(
                            "cardinality is `one-to-one` but the join columns on `{}` or `{}` are not marked `primary_key` or `unique`",
                            lhs_table, rhs_table
                        ),
                            card_span,
                        ),
                    );
                }
            }
            Cardinality::OneToMany => {
                // Spec: "from left to right" — one row on the left maps to
                // many on the right, so the left side is the "one" side.
                if !lhs_cols_unique {
                    out.push(
                        Problem::spec(
                            "S06",
                            Severity::Error,
                            format!(
                            "cardinality is `one-to-many` but the left-side join column on `{}` is not marked `primary_key` or `unique`",
                            lhs_table
                        ),
                            card_span,
                        ),
                    );
                }
            }
            Cardinality::ManyToOne => {
                if !rhs_cols_unique {
                    out.push(
                        Problem::spec(
                            "S06",
                            Severity::Error,
                            format!(
                            "cardinality is `many-to-one` but the right-side join column on `{}` is not marked `primary_key` or `unique`",
                            rhs_table
                        ),
                            card_span,
                        ),
                    );
                }
            }
        }
    }
}

fn side_has_unique_implied(
    dict: &DataDict,
    table_name: &str,
    join: &JoinExpr,
    use_lhs: bool,
) -> bool {
    let Some(table) = dict.tables.get(table_name) else {
        return false;
    };
    join.conjuncts.iter().any(|conj| {
        let q: &QCol = if use_lhs { &conj.lhs } else { &conj.rhs };
        if q.table != table_name {
            return false;
        }
        table
            .column(&q.column)
            .is_some_and(|c| c.is_unique_implied())
    })
}

// --- S07 --------------------------------------------------------------

const RANGE_TYPES: &[&str] = &["number(ordinal)", "number(quantity)", "date", "datetime"];

fn check_column_data_representation(dict: &DataDict, out: &mut ProblemSet) {
    for (table_name, table) in &dict.tables {
        for col in &table.columns {
            let Some(col_type) = &col.col_type else {
                continue;
            };
            let type_name = col_type.value.as_str();
            let span = col.name.span.clone();

            if type_name == "enum" {
                if !col.has_values {
                    out.push(
                        Problem::spec(
                            "S07",
                            Severity::Error,
                            format!(
                            "column `{}.{}` has type `enum` but is missing the required `values` property",
                            table_name, col.name.value
                        ),
                            span,
                        ),
                    );
                }
                if col.has_range {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `enum` but uses `range`; \
                             enum columns represent their data with `values`",
                            table_name, col.name.value
                        ),
                        col.name.span.clone(),
                    ));
                }
                if col.has_examples {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `enum` but uses `examples`; \
                             enum columns represent their data with `values`",
                            table_name, col.name.value
                        ),
                        col.name.span.clone(),
                    ));
                }
            } else if RANGE_TYPES.contains(&type_name) {
                if !col.has_range {
                    out.push(
                        Problem::spec(
                            "S07",
                            Severity::Error,
                            format!(
                            "column `{}.{}` has type `{}` but is missing the expected `range` property",
                            table_name, col.name.value, type_name
                        ),
                            span,
                        ),
                    );
                }
                if col.has_values {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `{}` but uses `values`; \
                             use `range` for ordered numeric and date columns",
                            table_name, col.name.value, type_name
                        ),
                        col.name.span.clone(),
                    ));
                }
                if col.has_examples {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `{}` but uses `examples`; \
                             use `range` for ordered numeric and date columns",
                            table_name, col.name.value, type_name
                        ),
                        col.name.span.clone(),
                    ));
                }
            } else {
                if !col.has_examples && type_name != "boolean" {
                    out.push(
                        Problem::spec(
                            "S07",
                            Severity::Error,
                            format!(
                            "column `{}.{}` has type `{}` but is missing the expected `examples` property",
                            table_name, col.name.value, type_name
                        ),
                            span,
                        ),
                    );
                }
                if col.has_values {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `{}` but uses `values`; \
                             only `enum` columns should use `values`",
                            table_name, col.name.value, type_name
                        ),
                        col.name.span.clone(),
                    ));
                }
                if col.has_range {
                    out.push(Problem::spec(
                        "S07",
                        Severity::Error,
                        format!(
                            "column `{}.{}` has type `{}` but uses `range`; \
                             `range` is only valid for `number(ordinal)`, `number(quantity)`, \
                             `date`, and `datetime`",
                            table_name, col.name.value, type_name
                        ),
                        col.name.span.clone(),
                    ));
                }
            }
        }
    }
}

// --- S08 --------------------------------------------------------------

fn check_units_only_on_quantity(dict: &DataDict, out: &mut ProblemSet) {
    for (table_name, table) in &dict.tables {
        for col in &table.columns {
            let Some(units) = &col.units else { continue };
            let is_quantity = col
                .col_type
                .as_ref()
                .is_some_and(|t| t.value == "number(quantity)");
            if !is_quantity {
                let type_desc = col
                    .col_type
                    .as_ref()
                    .map_or_else(|| "no type".to_string(), |t| format!("type `{}`", t.value));
                out.push(
                    Problem::spec(
                        "S08",
                        Severity::Error,
                        format!(
                        "column `{}.{}` has `units` but {}; `units` is only valid on `number(quantity)` columns",
                        table_name, col.name.value, type_desc
                    ),
                        units.span.clone(),
                    ),
                );
            }
        }
    }
}

// --- S09 --------------------------------------------------------------

/// Warn when the document omits the recommended `$learn_more` key. Unlike the
/// other rules this inspects the raw AST, because `$learn_more` is top-level
/// metadata that the lowered [`DataDict`] does not carry. The warning is
/// anchored at the `$version` key, which the schema guarantees is present.
fn check_learn_more(root: &YamlWithSourceInfo, out: &mut ProblemSet) {
    let Some(entries) = root.as_hash() else {
        return;
    };
    let has = |key: &str| entries.iter().find(|e| e.key.yaml.as_str() == Some(key));
    if has("$learn_more").is_some() {
        return;
    }
    let span = has("$version")
        .map(|e| e.key_span.clone())
        .unwrap_or_else(|| root.source_info.clone());
    out.push(
        Problem::spec(
            "S09",
            Severity::Warning,
            "document is missing the recommended `$learn_more` key".to_string(),
            span,
        )
        .with_hint(format!(
            "Add `$learn_more: {LEARN_MORE_URL}` so readers unfamiliar with the format can find it"
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_compiles() {
        let _ = schema();
    }
}
