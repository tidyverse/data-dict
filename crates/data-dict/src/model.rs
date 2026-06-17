//! Typed in-memory model of a data dictionary.
//!
//! Lowered from the source YAML by `lower::lower` once the structural schema
//! has accepted the document, so the lowering code can assume well-formed
//! input. Each significant node carries a `SourceInfo` so lint diagnostics
//! can point back at the source.

use indexmap::IndexMap;
use quarto_source_map::SourceInfo;

use crate::join_expr::JoinExpr;

/// A value paired with its source location.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub span: SourceInfo,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: SourceInfo) -> Self {
        Self { value, span }
    }
}

#[derive(Debug, Clone)]
pub struct DataDict {
    pub tables: IndexMap<String, Table>,
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: Spanned<String>,
    pub columns: Vec<Column>,
}

impl Table {
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name.value == name)
    }
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: Spanned<String>,
    pub constraints: Vec<Spanned<Constraint>>,
    pub col_type: Option<Spanned<String>>,
    pub has_values: bool,
    pub has_range: bool,
    pub has_examples: bool,
    pub units: Option<Spanned<String>>,
    pub time_zone: Option<Spanned<String>>,
}

impl Column {
    pub fn has(&self, c: Constraint) -> bool {
        self.constraints.iter().any(|x| x.value == c)
    }

    /// True if the column is unique-by-row: explicitly `unique` or
    /// `primary_key` (which the spec defines as implying `unique`).
    pub fn is_unique_implied(&self) -> bool {
        self.has(Constraint::Unique) || self.has(Constraint::PrimaryKey)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    PrimaryKey,
    ForeignKey,
    Required,
    Unique,
}

impl Constraint {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "primary_key" => Self::PrimaryKey,
            "foreign_key" => Self::ForeignKey,
            "required" => Self::Required,
            "unique" => Self::Unique,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub cardinality: Spanned<Cardinality>,
    /// The original join string with its source span. Kept alongside the
    /// parsed `JoinExpr` so diagnostics about parse failure can refer back to
    /// it.
    pub join_text: Spanned<String>,
    /// `None` if the join string failed to parse — DD004 is emitted in that
    /// case and downstream rules that need the parsed form (DD001, DD005,
    /// DD006) skip the relationship.
    pub join: Option<JoinExpr>,
    pub conflicts: Vec<Spanned<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
}

impl Cardinality {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "one-to-one" => Self::OneToOne,
            "one-to-many" => Self::OneToMany,
            "many-to-one" => Self::ManyToOne,
            _ => return None,
        })
    }
}
