//! Lower a `quarto_yaml` AST into the typed [`DataDict`] model.
//!
//! Invariant: lowering only runs after the schema has accepted the document,
//! so we may assume the shape conforms (required keys present, enums valid,
//! arrays where arrays are expected). Unexpected shapes are silently dropped
//! rather than panicking — they should be unreachable.

use quarto_source_map::SourceInfo;
use quarto_yaml::YamlWithSourceInfo;

use crate::join_expr::JoinExpr;
use crate::lint::Diagnostic;
use crate::model::{Cardinality, Column, Constraint, DataDict, Relationship, Spanned, Table};

/// Lower an AST, collecting any lowering diagnostics (currently only DD004
/// for unparseable join expressions).
pub fn lower(root: &YamlWithSourceInfo) -> (DataDict, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut tables = indexmap::IndexMap::new();
    if let Some(t_node) = root.get_hash_value("tables") {
        if let Some(entries) = t_node.as_hash() {
            for entry in entries {
                let Some(name) = entry.key.yaml.as_str() else {
                    continue;
                };
                let table = lower_table(name, &entry.key_span, &entry.value);
                tables.insert(name.to_string(), table);
            }
        }
    }

    let mut relationships = Vec::new();
    if let Some(r_node) = root.get_hash_value("relationships") {
        if let Some(items) = r_node.as_array() {
            for item in items {
                relationships.push(lower_relationship(item, &mut diagnostics));
            }
        }
    }

    (
        DataDict {
            tables,
            relationships,
        },
        diagnostics,
    )
}

fn lower_table(name: &str, name_span: &SourceInfo, value: &YamlWithSourceInfo) -> Table {
    let mut columns = Vec::new();
    if let Some(c_node) = value.get_hash_value("columns") {
        if let Some(items) = c_node.as_array() {
            for col in items {
                if let Some(c) = lower_column(col) {
                    columns.push(c);
                }
            }
        }
    }
    Table {
        name: Spanned::new(name.to_string(), name_span.clone()),
        columns,
    }
}

fn lower_column(node: &YamlWithSourceInfo) -> Option<Column> {
    let entries = node.as_hash()?;
    let mut name: Option<Spanned<String>> = None;
    let mut constraints: Vec<Spanned<Constraint>> = Vec::new();
    let mut col_type: Option<Spanned<String>> = None;
    let mut has_values = false;
    let mut has_range = false;
    let mut has_examples = false;
    let mut units: Option<Spanned<String>> = None;
    for entry in entries {
        let Some(key) = entry.key.yaml.as_str() else {
            continue;
        };
        match key {
            "name" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    name = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "type" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    col_type = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "values" => has_values = true,
            "range" => has_range = true,
            "examples" => has_examples = true,
            "units" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    units = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "constraints" => {
                if let Some(items) = entry.value.as_array() {
                    for c in items {
                        if let Some(s) = c.yaml.as_str() {
                            if let Some(parsed) = Constraint::parse(s) {
                                constraints.push(Spanned::new(parsed, c.source_info.clone()));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(Column {
        name: name?,
        constraints,
        col_type,
        has_values,
        has_range,
        has_examples,
        units,
    })
}

fn lower_relationship(
    node: &YamlWithSourceInfo,
    diagnostics: &mut Vec<Diagnostic>,
) -> Relationship {
    let entries = node.as_hash().expect("schema guarantees mapping");
    let mut cardinality: Option<Spanned<Cardinality>> = None;
    let mut join_text: Option<Spanned<String>> = None;
    let mut conflicts: Vec<Spanned<String>> = Vec::new();

    for entry in entries {
        let Some(key) = entry.key.yaml.as_str() else {
            continue;
        };
        match key {
            "cardinality" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    if let Some(c) = Cardinality::parse(s) {
                        cardinality = Some(Spanned::new(c, entry.value_span.clone()));
                    }
                }
            }
            "join" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    join_text = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "conflicts" => {
                if let Some(items) = entry.value.as_array() {
                    for c in items {
                        if let Some(s) = c.yaml.as_str() {
                            conflicts.push(Spanned::new(s.to_string(), c.source_info.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let cardinality = cardinality.expect("schema guarantees cardinality is a valid enum");
    let join_text = join_text.expect("schema guarantees join is present and a string");

    let join = match JoinExpr::parse(&join_text.value) {
        Ok(expr) => Some(expr),
        Err(err) => {
            diagnostics.push(Diagnostic::join_parse_error(&join_text, &err));
            None
        }
    };

    Relationship {
        cardinality,
        join_text,
        join,
        conflicts,
    }
}
