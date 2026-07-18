//! Typed in-memory model of a data dictionary.
//!
//! Lowered from the source YAML by `lower::lower` once the structural schema
//! has accepted the document, so the lowering code can assume well-formed
//! input. Each significant node carries a `SourceInfo` so schema-check diagnostics
//! can point back at the source.

use quarto_source_map::SourceInfo;

use crate::join_expr::JoinExpr;

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
    pub tables: Vec<Table>,
    pub relationships: Vec<Relationship>,
}

impl DataDict {
    /// The first table with the given name, or `None`. Duplicate names are an
    /// error (S10); lookups resolve to the first so downstream checks still run.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name.value == name)
    }

    /// The `(table, column)` a single-column foreign key points at: the
    /// `primary_key` on the other side of a relationship whose join names `col`.
    /// `None` if `col` is not a foreign key, or no relationship resolves it (the
    /// S01 case). Shared by the S01 spec check and the D05/D06 data checks.
    pub fn resolve_foreign_key(&self, table: &Table, col: &Column) -> Option<(&Table, &Column)> {
        if !col.has(Constraint::ForeignKey) {
            return None;
        }
        let table_name = table.name.value.as_str();
        for rel in &self.relationships {
            let Some(join) = &rel.join else { continue };
            for conj in &join.conjuncts {
                for (fk_side, pk_side) in [(&conj.lhs, &conj.rhs), (&conj.rhs, &conj.lhs)] {
                    if fk_side.table != table_name || fk_side.column != col.name.value {
                        continue;
                    }
                    let Some(other_tbl) = self.table(&pk_side.table) else {
                        continue;
                    };
                    let Some(other_col) = other_tbl.column(&pk_side.column) else {
                        continue;
                    };
                    if other_col.has(Constraint::PrimaryKey) {
                        return Some((other_tbl, other_col));
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: Spanned<String>,
    pub columns: Vec<Column>,
    /// Where the table's data lives, when it declares a `source`. Optional
    /// for spec validation; required for metadata validation (M04).
    pub source: Option<Source>,
    /// Spans of the `label`/`description`/`details` keys, when present. Held so
    /// S16 can point at a single-table dictionary's misplaced table-level
    /// descriptions.
    pub label: Option<SourceInfo>,
    pub description: Option<SourceInfo>,
    pub details: Option<SourceInfo>,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub span: SourceInfo,
    /// path relative to dictionary
    pub parquet: Spanned<String>,
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
    /// The allowed values of an `enum` column: the list items, or the keys of
    /// the map form (whose labels are dropped — only the values are constrained).
    pub values: Option<Representation>,
    pub range: Option<Representation>,
    pub examples: Option<Representation>,
    pub units: Option<Spanned<String>>,
    pub time_zone: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub struct Representation {
    pub span: SourceInfo,
    pub items: Vec<Spanned<Scalar>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// An integer, kept distinct from `Float` so its exact value survives for
    /// value-equality (D04); routing every number through `f64` would lose
    /// precision past 2^53.
    Int(i64),
    Float(f64),
    String(String), // includes date/times
    Bool(bool),
    Null,
    /// A list or map — never valid in a representation list.
    Compound,
}

impl Scalar {
    /// English noun phrase naming the scalar's kind, for diagnostics.
    pub fn noun(&self) -> &'static str {
        match self {
            Scalar::Int(_) | Scalar::Float(_) => "a number",
            Scalar::String(_) => "a string",
            Scalar::Bool(_) => "a boolean",
            Scalar::Null => "null",
            Scalar::Compound => "a list or map",
        }
    }

    /// The numeric value as `f64` for ordering comparisons (S13 range order),
    /// or `None` if not a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Scalar::Int(n) => Some(*n as f64),
            Scalar::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// The canonical string forms this value can take in data, for value-equality
    /// comparison (D04). Empty for kinds that can't appear as a data value
    /// (`null`, compound). Must agree with the data side's canonicalization in
    /// `data-dict-parquet`.
    ///
    /// A float yields two forms — its own (`f64`, matching a `DOUBLE` column) and
    /// its narrowing to `f32` (matching a `FLOAT` column) — because a value like
    /// `3.14159265358979` prints differently at each width, and the data side
    /// formats at the column's physical width.
    pub fn value_keys(&self) -> Vec<String> {
        match self {
            Scalar::Int(n) => vec![n.to_string()],
            Scalar::Float(n) => {
                let wide = n.to_string();
                let narrow = (*n as f32).to_string();
                if narrow == wide {
                    vec![wide]
                } else {
                    vec![wide, narrow]
                }
            }
            Scalar::String(s) => vec![s.clone()],
            Scalar::Bool(b) => vec![b.to_string()],
            Scalar::Null | Scalar::Compound => vec![],
        }
    }
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

    /// True if the column may not contain nulls: explicitly `required` or
    /// `primary_key` (which the spec defines as implying `required`).
    pub fn is_required_implied(&self) -> bool {
        self.has(Constraint::Required) || self.has(Constraint::PrimaryKey)
    }

    pub fn is_enum(&self) -> bool {
        self.col_type.as_ref().is_some_and(|t| t.value == "enum")
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
    /// `None` if the join string failed to parse — S04 is emitted in that
    /// case and downstream rules that need the parsed form (S01, S05,
    /// S06) skip the relationship.
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
