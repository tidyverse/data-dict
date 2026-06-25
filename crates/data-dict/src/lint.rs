//! Cross-table semantic linting for data-dict.yaml documents.
//!
//! Most rules run over a lowered [`DataDict`] and emit zero or more
//! [`Diagnostic`]s. Diagnostics carry a [`Severity`]: rules DD001–DD008 are
//! errors that fail validation, while DD009 is a warning that is reported but
//! does not. DD009 inspects the raw document rather than the lowered model,
//! since it is about a top-level metadata key.
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
//! - `DD007`: a column's data representation key (`values`, `range`, or
//!   `examples`) is absent or wrong for its type. Each type expects exactly
//!   one: `enum` → `values`; `number(ordinal)`, `number(quantity)`, `date`,
//!   `datetime` → `range`; all others → `examples` (except `boolean` and
//!   `ignore`, which need no data representation key).
//! - `DD008`: a column carries `units` but its type is not `number(quantity)`.
//!   Units are only meaningful for quantities.
//! - `DD009` (warning): the document omits the recommended `$learn_more`
//!   top-level key.

use quarto_error_reporting::DiagnosticMessageBuilder;
use quarto_source_map::{SourceContext, SourceInfo};
use quarto_yaml::YamlWithSourceInfo;

use crate::join_expr::{JoinExpr, ParseError, QCol};
use crate::model::{Cardinality, DataDict, Spanned};

/// The canonical documentation URL suggested for `$learn_more`.
pub const LEARN_MORE_URL: &str = "http://data-dict.tidyverse.org/";

/// Whether a diagnostic blocks validation (`Error`) or is purely advisory
/// (`Warning`). Errors fail validation; warnings are reported alongside a
/// successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A document's lint diagnostics together with the [`SourceContext`] needed to
/// render them. Checks push into a `Diagnostics` as they run; calling [`sort`]
/// then orders them by their position in the document.
///
/// [`sort`]: Diagnostics::sort
#[derive(Debug)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
    pub source: SourceContext,
}

impl Diagnostics {
    /// An empty set of diagnostics tied to a source context, ready for checks
    /// to push into.
    pub fn new(source: SourceContext) -> Self {
        Diagnostics {
            items: Vec::new(),
            source,
        }
    }

    /// An empty set of diagnostics with no source. Used when validation could
    /// not even begin (e.g. the document failed to parse).
    pub fn empty() -> Self {
        Diagnostics::new(SourceContext::new())
    }

    /// Record a diagnostic found by a check.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Order diagnostics by their position in the document. Rules emit in their
    /// own order (e.g. `$learn_more` is checked last), but readers expect
    /// diagnostics in source order. The sort is stable, so diagnostics sharing
    /// a position keep their emission order; spans that don't resolve to a byte
    /// range sort last.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|d| {
            d.span
                .resolve_byte_range()
                .map(|(file, start, _)| (file, start))
                .unwrap_or((usize::MAX, usize::MAX))
        });
    }

    /// Whether any diagnostic is an error. Errors fail validation; warnings do
    /// not.
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    /// Whether the document is valid: no error-severity diagnostics. It may
    /// still carry warnings.
    pub fn is_ok(&self) -> bool {
        !self.has_errors()
    }

    /// Render every diagnostic to display text, in their current order.
    pub fn render(&self) -> Vec<String> {
        self.items.iter().map(|d| d.to_text(&self.source)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub severity: Severity,
    pub span: SourceInfo,
    /// Secondary spans with explanatory labels. Rendered as bulleted details
    /// below the primary location.
    pub related: Vec<(SourceInfo, String)>,
    /// Advisory follow-up rendered as an info bullet, e.g. how to resolve a
    /// warning.
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn to_text(&self, ctx: &SourceContext) -> String {
        // The header is just the rule code (e.g. "Error: [DD007]"); the message
        // is shown once, against the source span. We drop the generic title that
        // added nothing. The code goes in the title rather than via `with_code`,
        // which would append it after an empty title and leave a trailing space.
        let header = format!("[{}]", self.code);
        let mut builder = match self.severity {
            Severity::Error => DiagnosticMessageBuilder::error(header),
            Severity::Warning => DiagnosticMessageBuilder::warning(header),
        };
        builder = builder
            .problem(self.message.clone())
            .with_location(self.span.clone());
        for (span, label) in &self.related {
            builder = builder.add_detail_at(label.clone(), span.clone());
        }
        if let Some(hint) = &self.hint {
            builder = builder.add_info(hint.clone());
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
            severity: Severity::Error,
            hint: None,
        }
    }
}

/// Run every rule, pushing any findings into `out`. Rules run in code order;
/// call [`Diagnostics::sort`] afterwards to put the findings in source order.
pub fn lint(dict: &DataDict, out: &mut Diagnostics) {
    check_relationship_table_refs(dict, out); // DD002
    check_relationship_column_refs(dict, out); // DD003
    check_join_table_count(dict, out); // DD004
    check_foreign_keys_resolve(dict, out); // DD001
    check_conflicts_present_on_both_sides(dict, out); // DD005
    check_cardinality_consistency(dict, out); // DD006
    check_column_data_representation(dict, out); // DD007
    check_units_only_on_quantity(dict, out); // DD008
}

// --- DD002 --------------------------------------------------------------

fn check_relationship_table_refs(dict: &DataDict, out: &mut Diagnostics) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        for q in join.qcols() {
            if !dict.tables.contains_key(&q.table) {
                let span = subspan(&rel.join_text.span, q.start, q.end)
                    .unwrap_or_else(|| rel.join_text.span.clone());
                out.push(Diagnostic {
                    code: "DD002",
                    message: format!(
                        "relationship references table `{}`, which is not defined in `tables`",
                        q.table
                    ),
                    span,
                    related: Vec::new(),
                    severity: Severity::Error,
                    hint: None,
                });
            }
        }
    }
}

// --- DD003 --------------------------------------------------------------

fn check_relationship_column_refs(dict: &DataDict, out: &mut Diagnostics) {
    for rel in &dict.relationships {
        if let Some(join) = &rel.join {
            for q in join.qcols() {
                // Skip if the table doesn't exist — DD002 handles that case
                // and a column report would be noise.
                let Some(table) = dict.tables.get(&q.table) else {
                    continue;
                };
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
                        severity: Severity::Error,
                        hint: None,
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

fn check_join_table_count(dict: &DataDict, out: &mut Diagnostics) {
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
                severity: Severity::Error,
                hint: None,
            });
        }
    }
}

// --- DD001 --------------------------------------------------------------

fn check_foreign_keys_resolve(dict: &DataDict, out: &mut Diagnostics) {
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
                out.push(Diagnostic {
                    code: "DD001",
                    message: format!(
                        "column `{}.{}` is marked `foreign_key` but no `relationships` entry points it at a `primary_key` column",
                        table_name, col.name.value
                    ),
                    span: col.name.span.clone(),
                    related: Vec::new(),
                    severity: Severity::Error,
                    hint: None,
                });
            }
        }
    }
}

// --- DD005 --------------------------------------------------------------

fn check_conflicts_present_on_both_sides(dict: &DataDict, out: &mut Diagnostics) {
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
                    severity: Severity::Error,
                    hint: None,
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

fn check_cardinality_consistency(dict: &DataDict, out: &mut Diagnostics) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };

        // Skip if any join column references a missing table or column. The
        // missing reference is already reported (DD002 / DD003), and checking
        // cardinality against a column that doesn't exist would just produce a
        // redundant, confusing DD006.
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
                    out.push(Diagnostic {
                        code: "DD006",
                        message: format!(
                            "cardinality is `one-to-one` but the join columns on `{}` or `{}` are not marked `primary_key` or `unique`",
                            lhs_table, rhs_table
                        ),
                        span: card_span,
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
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
                        severity: Severity::Error,
                        hint: None,
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
                        severity: Severity::Error,
                        hint: None,
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
            .is_some_and(|c| c.is_unique_implied())
    })
}

// --- DD007 --------------------------------------------------------------

const RANGE_TYPES: &[&str] = &["number(ordinal)", "number(quantity)", "date", "datetime"];

fn check_column_data_representation(dict: &DataDict, out: &mut Diagnostics) {
    for (table_name, table) in &dict.tables {
        for col in &table.columns {
            let Some(col_type) = &col.col_type else {
                continue;
            };
            let type_name = col_type.value.as_str();
            let span = col.name.span.clone();

            // An `ignore` column is intentionally undocumented, so it carries no
            // data representation key.
            if type_name == "ignore" {
                continue;
            }

            if type_name == "enum" {
                if !col.has_values {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `enum` but is missing the required `values` property",
                            table_name, col.name.value
                        ),
                        span,
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_range {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `enum` but uses `range`; \
                             enum columns represent their data with `values`",
                            table_name, col.name.value
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_examples {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `enum` but uses `examples`; \
                             enum columns represent their data with `values`",
                            table_name, col.name.value
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
            } else if RANGE_TYPES.contains(&type_name) {
                if !col.has_range {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but is missing the expected `range` property",
                            table_name, col.name.value, type_name
                        ),
                        span,
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_values {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but uses `values`; \
                             use `range` for ordered numeric and date columns",
                            table_name, col.name.value, type_name
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_examples {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but uses `examples`; \
                             use `range` for ordered numeric and date columns",
                            table_name, col.name.value, type_name
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
            } else {
                if !col.has_examples && type_name != "boolean" {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but is missing the expected `examples` property",
                            table_name, col.name.value, type_name
                        ),
                        span,
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_values {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but uses `values`; \
                             only `enum` columns should use `values`",
                            table_name, col.name.value, type_name
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
                if col.has_range {
                    out.push(Diagnostic {
                        code: "DD007",
                        message: format!(
                            "column `{}.{}` has type `{}` but uses `range`; \
                             `range` is only valid for `number(ordinal)`, `number(quantity)`, \
                             `date`, and `datetime`",
                            table_name, col.name.value, type_name
                        ),
                        span: col.name.span.clone(),
                        related: Vec::new(),
                        severity: Severity::Error,
                        hint: None,
                    });
                }
            }
        }
    }
}

// --- DD008 --------------------------------------------------------------

fn check_units_only_on_quantity(dict: &DataDict, out: &mut Diagnostics) {
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
                out.push(Diagnostic {
                    code: "DD008",
                    message: format!(
                        "column `{}.{}` has `units` but {}; `units` is only valid on `number(quantity)` columns",
                        table_name, col.name.value, type_desc
                    ),
                    span: units.span.clone(),
                    related: Vec::new(),
                    severity: Severity::Error,
                    hint: None,
                });
            }
        }
    }
}

// --- DD009 --------------------------------------------------------------

/// Warn when the document omits the recommended `$learn_more` key. Unlike the
/// other rules this inspects the raw AST, because `$learn_more` is top-level
/// metadata that the lowered [`DataDict`] does not carry. The warning is
/// anchored at the `$version` key, which the schema guarantees is present.
pub fn check_learn_more(root: &YamlWithSourceInfo, out: &mut Diagnostics) {
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
    out.push(Diagnostic {
        code: "DD009",
        message: "document is missing the recommended `$learn_more` key".to_string(),
        severity: Severity::Warning,
        span,
        related: Vec::new(),
        hint: Some(format!(
            "Add `$learn_more: {LEARN_MORE_URL}` so readers unfamiliar with the format can find it"
        )),
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
