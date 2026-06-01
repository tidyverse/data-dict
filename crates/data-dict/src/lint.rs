//! Cross-table semantic linting for data-dict.yaml documents.
//!
//! Each rule runs over a lowered [`DataDict`] and emits zero or more
//! [`Diagnostic`]s. All diagnostics are errors — there is no warning level.
//!
//! Rule codes:
//!
//! - `DD001`: `foreign_key` column has no matching `relationships` entry
//!   whose other side is a `primary_key` column.
//! - `DD002`: relationship references a table that does not exist.
//! - `DD003`: relationship references a column that does not exist on its
//!   table.
//! - `DD004`: `join` string fails to parse, or references neither one nor two
//!   tables.
//! - `DD005`: a name in `conflicts` does not appear as a column on both sides
//!   of the join.
//! - `DD006`: cardinality is inconsistent with the constraints on the joined
//!   columns (e.g. `one-to-many` whose "one" side lacks `primary_key` /
//!   `unique`).

use quarto_error_reporting::DiagnosticMessageBuilder;
use quarto_source_map::{SourceContext, SourceInfo};

use crate::join_expr::{JoinExpr, ParseError, QCol};
use crate::model::{Cardinality, DataDict, Spanned};

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: SourceInfo,
    /// Secondary spans with explanatory labels. Rendered as bulleted details
    /// below the primary location.
    pub related: Vec<(SourceInfo, String)>,
}

impl Diagnostic {
    pub fn to_text(&self, ctx: &SourceContext) -> String {
        let mut builder = DiagnosticMessageBuilder::error("data-dict.yaml lint")
            .with_code(self.code)
            .problem(self.message.clone())
            .with_location(self.span.clone());
        for (span, label) in &self.related {
            builder = builder.add_detail_at(label.clone(), span.clone());
        }
        builder.build().to_text(Some(ctx))
    }

    pub(crate) fn join_parse_error(join_text: &Spanned<String>, err: &ParseError) -> Self {
        let span = subspan(&join_text.span, err.at, err.at.min(join_text.value.len()));
        Diagnostic {
            code: "DD004",
            message: format!("`join` expression does not parse: {}", err.message),
            span: span.unwrap_or_else(|| join_text.span.clone()),
            related: Vec::new(),
        }
    }
}

/// Run every rule and return all diagnostics. Diagnostics retain emission
/// order: rules earlier in the list run first, and within a rule the order
/// follows source order.
pub fn lint(dict: &DataDict) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    check_relationship_table_refs(dict, &mut out); // DD002
    check_relationship_column_refs(dict, &mut out); // DD003
    check_join_table_count(dict, &mut out); // DD004
    check_foreign_keys_resolve(dict, &mut out); // DD001
    check_conflicts_present_on_both_sides(dict, &mut out); // DD005
    check_cardinality_consistency(dict, &mut out); // DD006
    out
}

// --- DD002 --------------------------------------------------------------

fn check_relationship_table_refs(dict: &DataDict, out: &mut Vec<Diagnostic>) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        for q in join.qcols() {
            if !dict.tables.contains_key(&q.table) {
                let span =
                    subspan(&rel.join_text.span, q.start, q.end).unwrap_or_else(|| rel.join_text.span.clone());
                out.push(Diagnostic {
                    code: "DD002",
                    message: format!(
                        "relationship references table `{}`, which is not defined in `tables`",
                        q.table
                    ),
                    span,
                    related: Vec::new(),
                });
            }
        }
    }
}

// --- DD003 --------------------------------------------------------------

fn check_relationship_column_refs(dict: &DataDict, out: &mut Vec<Diagnostic>) {
    for rel in &dict.relationships {
        if let Some(join) = &rel.join {
            for q in join.qcols() {
                // Skip if the table doesn't exist — DD002 handles that case
                // and a column report would be noise.
                let Some(table) = dict.tables.get(&q.table) else { continue };
                if table.column(&q.column).is_none() {
                    let span = subspan(&rel.join_text.span, q.start, q.end)
                        .unwrap_or_else(|| rel.join_text.span.clone());
                    out.push(Diagnostic {
                        code: "DD003",
                        message: format!(
                            "column `{}` is not defined in table `{}`",
                            q.column, q.table
                        ),
                        span,
                        related: Vec::new(),
                    });
                }
            }
        }
        // `conflicts` column references are checked by DD005 alongside the
        // "appears on both sides" check, so a missing column there reports
        // the more specific message.
    }
}

// --- DD004 --------------------------------------------------------------

fn check_join_table_count(dict: &DataDict, out: &mut Vec<Diagnostic>) {
    // Parse failures are emitted during lowering. Here we only check the
    // table-count invariant on successfully parsed joins.
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        if tables.is_empty() || tables.len() > 2 {
            out.push(Diagnostic {
                code: "DD004",
                message: format!(
                    "`join` must reference exactly one (self-join) or two tables; found {}",
                    tables.len()
                ),
                span: rel.join_text.span.clone(),
                related: Vec::new(),
            });
        }
    }
}

// --- DD001 --------------------------------------------------------------

fn check_foreign_keys_resolve(dict: &DataDict, out: &mut Vec<Diagnostic>) {
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
                        let Some(other_tbl) = dict.tables.get(&pk_side.table) else { return false };
                        let Some(other_col) = other_tbl.column(&pk_side.column) else { return false };
                        other_col.has(PrimaryKey)
                    })
                })
            });
            if !satisfied {
                out.push(Diagnostic {
                    code: "DD001",
                    message: format!(
                        "column `{}.{}` is marked `foreign_key` but no `relationships` entry points it at a `primary_key` column",
                        table_name, col.name.value
                    ),
                    span: col.name.span.clone(),
                    related: Vec::new(),
                });
            }
        }
    }
}

// --- DD005 --------------------------------------------------------------

fn check_conflicts_present_on_both_sides(dict: &DataDict, out: &mut Vec<Diagnostic>) {
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
                    // DD002 already flagged the missing table; skip to avoid
                    // a cascade of confusing reports.
                    continue;
                };
                if table.column(&c.value).is_none() {
                    missing_from.push(*t_name);
                }
            }
            if !missing_from.is_empty() {
                out.push(Diagnostic {
                    code: "DD005",
                    message: format!(
                        "`conflicts` entry `{}` is not a column of {}",
                        c.value,
                        join_with_commas(&missing_from)
                    ),
                    span: c.span.clone(),
                    related: Vec::new(),
                });
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

// --- DD006 --------------------------------------------------------------

fn check_cardinality_consistency(dict: &DataDict, out: &mut Vec<Diagnostic>) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };

        // Skip if any join column references a missing table or column. The
        // missing reference is already reported (DD002 / DD003), and checking
        // cardinality against a column that doesn't exist would just produce a
        // redundant, confusing DD006.
        let all_cols_resolve = join.qcols().all(|q| {
            dict.tables
                .get(&q.table)
                .map_or(false, |t| t.column(&q.column).is_some())
        });
        if !all_cols_resolve {
            continue;
        }

        // The cardinality rule is defined in terms of the LHS and RHS tables
        // of the join. With multi-conjunct joins (date-range overlap), the
        // LHS and RHS tables are the same across all conjuncts, so we can
        // use the first conjunct as the canonical orientation.
        let Some(first) = join.conjuncts.first() else { continue };
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

        let lhs_cols_unique = side_has_unique_implied(dict, &lhs_table, join, /* use_lhs = */ true);
        let rhs_cols_unique = side_has_unique_implied(dict, &rhs_table, join, /* use_lhs = */ false);

        let card_span = rel.cardinality.span.clone();
        match rel.cardinality.value {
            Cardinality::OneToOne => {
                if !lhs_cols_unique || !rhs_cols_unique {
                    out.push(Diagnostic {
                        code: "DD006",
                        message: format!(
                            "cardinality is `one-to-one` but the join columns on `{}` or `{}` are not marked `primary_key` or `unique`",
                            lhs_table, rhs_table
                        ),
                        span: card_span,
                        related: Vec::new(),
                    });
                }
            }
            Cardinality::OneToMany => {
                // Spec: "from left to right" — one row on the left maps to
                // many on the right, so the left side is the "one" side.
                if !lhs_cols_unique {
                    out.push(Diagnostic {
                        code: "DD006",
                        message: format!(
                            "cardinality is `one-to-many` but the left-side join column on `{}` is not marked `primary_key` or `unique`",
                            lhs_table
                        ),
                        span: card_span,
                        related: Vec::new(),
                    });
                }
            }
            Cardinality::ManyToOne => {
                if !rhs_cols_unique {
                    out.push(Diagnostic {
                        code: "DD006",
                        message: format!(
                            "cardinality is `many-to-one` but the right-side join column on `{}` is not marked `primary_key` or `unique`",
                            rhs_table
                        ),
                        span: card_span,
                        related: Vec::new(),
                    });
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
            .map_or(false, |c| c.is_unique_implied())
    })
}

// --- Helpers ------------------------------------------------------------

/// Build a sub-span pointing at `[start, end)` byte offsets within `parent`.
/// Returns `None` if `parent` does not resolve to a single contiguous byte
/// range (Concat / FilterProvenance variants, which YAML scalar literals
/// shouldn't produce in practice).
fn subspan(parent: &SourceInfo, start: usize, end: usize) -> Option<SourceInfo> {
    parent.resolve_byte_range()?;
    Some(SourceInfo::substring(parent.clone(), start, end))
}
