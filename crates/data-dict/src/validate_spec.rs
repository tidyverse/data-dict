//! Spec-level validation, the `S##` checks (see `site/validation.md`).
//!
//! [`validate_spec`] runs two internal passes — a split not surfaced in the CLI:
//!
//! 1. **schema**: structural validation against the embedded `schema.yaml` via
//!    the `quarto-yaml-validation` crate — everything a JSON Schema can express.
//! 2. **spec**: the cross-table semantic checks below that the schema can't
//!    express (foreign-key targets, `join` parsing, cardinality, …).
//!
//! The second pass only runs if the first succeeds: there is no point chasing
//! FK references in a document whose `tables` block is malformed.

use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, NaiveDate};
use quarto_source_map::SourceInfo;
use quarto_yaml::YamlWithSourceInfo;
use quarto_yaml_validation::{Schema, SchemaRegistry, ValidationDiagnostic};

use crate::join_expr::{JoinExpr, QCol};
use crate::model::{Cardinality, Column, DataDict, Scalar, Spanned};
use crate::problem::{Problem, ProblemKind, ProblemSet, subspan};
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

/// Validate the `data-dict.yaml` file at `path`. The returned [`ProblemSet`]
/// bundles every problem (errors and warnings, in source order) with the source
/// context needed to render them; [`ProblemSet::status`] reports whether the
/// document is valid. Failures that prevent checking altogether — I/O,
/// unparseable YAML, a structurally invalid document — surface as pre-flight
/// [`Problem`]s in the set.
pub fn validate_spec(path: &Path) -> ProblemSet {
    let (mut problems, doc) = match load(path) {
        Ok(loaded) => loaded,
        Err(problems) => return problems,
    };
    // We only want the problems here, not the lowered dictionary.
    validate_and_lower(&doc, &mut problems);
    problems
}

/// Read, parse, and schema-check the document at `path`, creating the run's
/// [`ProblemSet`] with the document's source — this is where every level starts.
/// `Ok((problems, doc))` hands back the fresh set and the parsed AST to validate;
/// `Err(problems)` carries a pre-flight failure (I/O, unparseable YAML, or a
/// document the schema rejects) for which no document could be produced.
pub(crate) fn load(path: &Path) -> Result<(ProblemSet, YamlWithSourceInfo), ProblemSet> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => return Err(ProblemSet::from_preflight(ProblemKind::Io, e.to_string())),
    };
    let filename = path.display().to_string();

    let doc = match quarto_yaml::parse_file(&content, &filename) {
        Ok(doc) => doc,
        Err(e) => {
            return Err(ProblemSet::from_preflight(
                ProblemKind::Parse,
                e.to_string(),
            ));
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
        return Err(problems);
    }

    Ok((ProblemSet::new(source), doc))
}

/// Lower the parsed document `doc` and run the S## semantic checks, pushing any
/// findings into `out`. Returns the lowered dictionary when the spec validates,
/// or `None` when it has errors (which `out` then carries).
pub(crate) fn validate_and_lower(
    doc: &YamlWithSourceInfo,
    out: &mut ProblemSet,
) -> Option<DataDict> {
    let dict = lower::lower(doc, out);
    check_spec(&dict, out);
    validate_s09_learn_more(doc, out);
    out.sort();

    if out.status().failed() {
        None
    } else {
        Some(dict)
    }
}

/// Run every rule, pushing any findings into `out`. Rules run in code order;
/// call [`ProblemSet::sort`] afterwards to put the findings in source order.
fn check_spec(dict: &DataDict, out: &mut ProblemSet) {
    validate_s02_relationship_table_refs(dict, out);
    validate_s03_relationship_column_refs(dict, out);
    validate_s04_join_table_count(dict, out);
    validate_s01_foreign_keys_resolve(dict, out);
    validate_s05_conflicts_present_on_both_sides(dict, out);
    validate_s06_cardinality_consistency(dict, out);
    validate_s07_column_data_representation(dict, out);
    validate_s08_units_only_on_quantity(dict, out);
    validate_s10_unique_column_names(dict, out);
    validate_s11_non_empty_names(dict, out);
    validate_s12_value_types(dict, out);
    validate_s13_range_order(dict, out);
}

// --- S02 --------------------------------------------------------------

fn validate_s02_relationship_table_refs(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        for q in join.qcols() {
            if !dict.tables.contains_key(&q.table) {
                let span = subspan(&rel.join_text.span, q.start, q.end)
                    .unwrap_or_else(|| rel.join_text.span.clone());
                out.push_spec_error(
                    "S02",
                    "A `join` must refer to known tables.",
                    format!("table `{}` is not defined", q.table),
                    [span],
                );
            }
        }
    }
}

// --- S03 --------------------------------------------------------------

fn validate_s03_relationship_column_refs(dict: &DataDict, out: &mut ProblemSet) {
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
                    out.push_spec_error(
                        "S03",
                        "A `join` must refer to known columns.",
                        format!("table `{}` has no column `{}`", q.table, q.column),
                        [span],
                    );
                }
            }
        }
        // `conflicts` column references are checked by S05 alongside the
        // "appears on both sides" check, so a missing column there reports
        // the more specific message.
    }
}

// --- S04 --------------------------------------------------------------

fn validate_s04_join_table_count(dict: &DataDict, out: &mut ProblemSet) {
    // Parse failures are emitted during lowering. Here we only check the
    // table-count invariant on successfully parsed joins.
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        if tables.is_empty() || tables.len() > 2 {
            out.push_spec_error(
                "S04",
                "A `join` must reference one (self-join) or two tables.",
                format!("this `join` references {} tables", tables.len()),
                [rel.join_text.span.clone()],
            );
        }
    }
}

// --- S01 --------------------------------------------------------------

fn validate_s01_foreign_keys_resolve(dict: &DataDict, out: &mut ProblemSet) {
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
                let fk_span = col
                    .constraints
                    .iter()
                    .find(|c| c.value == ForeignKey)
                    .map_or_else(|| col.name.span.clone(), |c| c.span.clone());
                out.push_spec_error(
                    "S01",
                    "Every `foreign_key` column must have a matching relationship to a `primary_key`.",
                    "is `foreign_key` but no relationship points it at a `primary_key`",
                    [table.name.span.clone(), col.name.span.clone(), fk_span],
                );
            }
        }
    }
}

// --- S05 --------------------------------------------------------------

fn validate_s05_conflicts_present_on_both_sides(dict: &DataDict, out: &mut ProblemSet) {
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
                out.push_spec_error(
                    "S05",
                    "A `conflicts` entry must name a column on both sides of the join.",
                    format!(
                        "`{}` is not a column of {}",
                        c.value,
                        join_with_commas(&missing_from)
                    ),
                    [rel.join_text.span.clone(), c.span.clone()],
                );
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

fn validate_s06_cardinality_consistency(dict: &DataDict, out: &mut ProblemSet) {
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
                    out.push_spec_error(
                        "S06",
                        "A `one-to-one` join must have `primary_key` or `unique` columns on both sides.",
                        format!(
                            "the join columns on `{}` or `{}` are not marked `primary_key` or `unique`",
                            lhs_table, rhs_table
                        ),
                        [card_span, rel.join_text.span.clone()],
                    );
                }
            }
            Cardinality::OneToMany => {
                // Spec: "from left to right" — one row on the left maps to
                // many on the right, so the left side is the "one" side.
                if !lhs_cols_unique {
                    out.push_spec_error(
                        "S06",
                        "A `one-to-many` join must have a `primary_key` or `unique` column on its left (\"one\") side.",
                        format!(
                            "the left-side join column on `{}` is not marked `primary_key` or `unique`",
                            lhs_table
                        ),
                        [card_span, rel.join_text.span.clone()],
                    );
                }
            }
            Cardinality::ManyToOne => {
                if !rhs_cols_unique {
                    out.push_spec_error(
                        "S06",
                        "A `many-to-one` join must have a `primary_key` or `unique` column on its right (\"one\") side.",
                        format!(
                            "the right-side join column on `{}` is not marked `primary_key` or `unique`",
                            rhs_table
                        ),
                        [card_span, rel.join_text.span.clone()],
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

fn validate_s07_column_data_representation(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
        for col in &table.columns {
            let Some(col_type) = &col.col_type else {
                continue;
            };
            let type_name = col_type.value.as_str();

            let found = |key: &str| format!("has type `{type_name}` but uses `{key}`");
            let missing = |key: &str| format!("has type `{type_name}` but is missing `{key}`");

            // A finding is reported on a specific line — the `type` line for a
            // missing representation, the offending key's line for a present
            // one — with the table and column shown as faded context above it.
            let at =
                |span: &SourceInfo| [table.name.span.clone(), col.name.span.clone(), span.clone()];

            if type_name == "enum" {
                if col.values.is_none() {
                    out.push_spec_error(
                        "S07",
                        "An `enum` column must list its categories with `values`.",
                        missing("values"),
                        at(&col_type.span),
                    );
                }
                if let Some(range) = &col.range {
                    out.push_spec_error(
                        "S07",
                        "An `enum` column must use `values`, not `range`.",
                        found("range"),
                        at(&range.span),
                    );
                }
                if let Some(examples) = &col.examples {
                    out.push_spec_error(
                        "S07",
                        "An `enum` column must use `values`, not `examples`.",
                        found("examples"),
                        at(&examples.span),
                    );
                }
            } else if RANGE_TYPES.contains(&type_name) {
                if col.range.is_none() {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must describe its bounds with `range`."),
                        missing("range"),
                        at(&col_type.span),
                    );
                }
                if let Some(values) = &col.values {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must use `range`, not `values`."),
                        found("values"),
                        at(values),
                    );
                }
                if let Some(examples) = &col.examples {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must use `range`, not `examples`."),
                        found("examples"),
                        at(&examples.span),
                    );
                }
            } else if type_name == "boolean" {
                for (span, key) in [
                    (col.values.as_ref(), "values"),
                    (col.range.as_ref().map(|r| &r.span), "range"),
                    (col.examples.as_ref().map(|e| &e.span), "examples"),
                ] {
                    if let Some(span) = span {
                        out.push_spec_error(
                            "S07",
                            "A `boolean` column must not have `values`, `range`, or `examples`.",
                            found(key),
                            at(span),
                        );
                    }
                }
            } else {
                if col.examples.is_none() {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must describe its data with `examples`."),
                        missing("examples"),
                        at(&col_type.span),
                    );
                }
                if let Some(values) = &col.values {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must not use `values`."),
                        found("values"),
                        at(values),
                    );
                }
                if let Some(range) = &col.range {
                    out.push_spec_error(
                        "S07",
                        format!("A `{type_name}` column must not use `range`."),
                        found("range"),
                        at(&range.span),
                    );
                }
            }
        }
    }
}

// --- S08 --------------------------------------------------------------

fn validate_s08_units_only_on_quantity(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
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
                out.push_spec_error(
                    "S08",
                    "A column with `units` must have type `number(quantity)`.",
                    format!("has `units` but has {type_desc}"),
                    [
                        table.name.span.clone(),
                        col.name.span.clone(),
                        units.span.clone(),
                    ],
                );
            }
        }
    }
}

// --- S10 --------------------------------------------------------------

fn validate_s10_unique_column_names(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for col in &table.columns {
            // Empty names are S11's concern; reporting them as duplicates too
            // would just be noise.
            if col.name.value.is_empty() {
                continue;
            }
            if !seen.insert(col.name.value.as_str()) {
                out.push_spec_error(
                    "S10",
                    "Column names must be unique within a table.",
                    "appears more than once",
                    [table.name.span.clone(), col.name.span.clone()],
                );
            }
        }
    }
}

// --- S11 --------------------------------------------------------------

fn validate_s11_non_empty_names(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
        if table.name.value.is_empty() {
            out.push_spec_error(
                "S11",
                "A table must have a non-empty name.",
                "table name is empty",
                [table.name.span.clone()],
            );
        }
        for col in &table.columns {
            if col.name.value.is_empty() {
                out.push_spec_error(
                    "S11",
                    "Every column must have a non-empty `name`.",
                    "the `name` is empty",
                    [table.name.span.clone(), col.name.span.clone()],
                );
            }
        }
    }
}

// --- S12 --------------------------------------------------------------

/// The representation list whose values are type-checked for a given column
/// type, or `None` for types that carry no typed representation (`enum`,
/// `boolean`, and any unrecognized type). Mirrors S07: each type owns exactly
/// one representation key, and we only check the one it owns so that a
/// misplaced key reports as S07 rather than cascading into S12.
fn typed_representation(col: &Column) -> Option<(&'static str, &[Spanned<Scalar>])> {
    match col.col_type.as_ref()?.value.as_str() {
        "number(ordinal)" | "number(quantity)" | "date" | "datetime" => {
            Some(("range", &col.range.as_ref()?.items))
        }
        "string" | "number" | "number(id)" => Some(("examples", &col.examples.as_ref()?.items)),
        _ => None,
    }
}

fn validate_s12_value_types(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
        for col in &table.columns {
            let type_name = match &col.col_type {
                Some(t) => t.value.as_str(),
                None => continue,
            };
            let Some((key, values)) = typed_representation(col) else {
                continue;
            };
            for v in values {
                if value_matches_type(type_name, &v.value) {
                    continue;
                }
                out.push_spec_error(
                    "S12",
                    format!(
                        "Each `{}` value of a `{}` column must be {}.",
                        key,
                        type_name,
                        expected_noun(type_name),
                    ),
                    format!("is {}", v.value.noun()),
                    [
                        table.name.span.clone(),
                        col.name.span.clone(),
                        v.span.clone(),
                    ],
                );
            }
        }
    }
}

fn value_matches_type(type_name: &str, value: &Scalar) -> bool {
    match type_name {
        "number" | "number(id)" | "number(ordinal)" | "number(quantity)" => {
            matches!(value, Scalar::Number(_))
        }
        // The YAML parser discards quote style, so a quoted `'1'` arrives as a
        // number and a quoted `'null'` as null; we can't tell those from a real
        // string. So `string` accepts any scalar and only rejects a list/map.
        "string" => !matches!(value, Scalar::Compound),
        "date" => matches!(value, Scalar::String(s) if parse_date(s).is_some()),
        "datetime" => matches!(value, Scalar::String(s) if parse_datetime(s).is_some()),
        _ => true,
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    s.parse().ok()
}

fn parse_datetime(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

fn expected_noun(type_name: &str) -> &'static str {
    match type_name {
        "string" => "a string",
        "date" => "an ISO 8601 date (YYYY-MM-DD)",
        "datetime" => "an ISO 8601 datetime with a timezone (e.g. 2024-01-31T09:30:00Z)",
        _ => "a number",
    }
}

// --- S13 --------------------------------------------------------------

fn validate_s13_range_order(dict: &DataDict, out: &mut ProblemSet) {
    for table in dict.tables.values() {
        for col in &table.columns {
            let type_name = match &col.col_type {
                Some(t) => t.value.as_str(),
                None => continue,
            };
            let Some(range) = &col.range else { continue };
            if !RANGE_TYPES.contains(&type_name) || range.items.len() != 2 {
                continue;
            }
            let (lo, hi) = (&range.items[0], &range.items[1]);
            // A mistyped bound is S12's to report; comparing it here would be
            // meaningless, so `range_descending` only fires when both bounds
            // parse for the column's type.
            if range_descending(type_name, &lo.value, &hi.value) {
                out.push_spec_error(
                    "S13",
                    "A range's minimum must be less than or equal to its maximum.",
                    "is greater than the maximum",
                    [
                        table.name.span.clone(),
                        col.name.span.clone(),
                        lo.span.clone(),
                    ],
                );
            }
        }
    }
}

/// Whether `lo`..`hi` runs backwards for the column's type. Returns `false`
/// unless both bounds parse as the type's value (a mistyped bound is S12's to
/// report). Numbers compare numerically; dates and datetimes compare as parsed
/// instants, so mixed timezone offsets are handled correctly.
fn range_descending(type_name: &str, lo: &Scalar, hi: &Scalar) -> bool {
    match (type_name, lo, hi) {
        ("date", Scalar::String(a), Scalar::String(b)) => match (parse_date(a), parse_date(b)) {
            (Some(a), Some(b)) => a > b,
            _ => false,
        },
        ("datetime", Scalar::String(a), Scalar::String(b)) => {
            match (parse_datetime(a), parse_datetime(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        (_, Scalar::Number(a), Scalar::Number(b)) => a > b,
        _ => false,
    }
}

// --- S09 --------------------------------------------------------------

/// Warn when the document omits the recommended `$learn_more` key. Unlike the
/// other rules this inspects the raw AST, because `$learn_more` is top-level
/// metadata that the lowered [`DataDict`] does not carry. The warning is
/// anchored at the `$version` key, which the schema guarantees is present.
fn validate_s09_learn_more(root: &YamlWithSourceInfo, out: &mut ProblemSet) {
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
    out.push_spec_warning(
        "S09",
        format!(
            "A document should point readers to the spec with `$learn_more: {LEARN_MORE_URL}`."
        ),
        "`$learn_more` is not set",
        [span],
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
