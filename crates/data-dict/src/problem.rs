//! A single problem vocabulary for every validation level.
//!
//! A [`Problem`] is anything that can be wrong while validating — a failed spec
//! check, a column that disagrees with the data, an unreadable file. They vary
//! only on where the problem points (a YAML span / a named column / nowhere) and
//! what structured payload they carry; a code, a severity, a message, and an
//! optional hint are common to all. The structured payload is the flattened
//! [`ProblemKind`] tag, so a single `#[derive(Serialize)]` produces the JSON for
//! every kind uniformly.
//!
//! There is no `fatal` field by design: a problem that must stop the run is the
//! last thing a level pushes before returning, and a problem that blocks the
//! next level is caught by the driver checking [`ProblemSet::has_errors`] before
//! descending. Fatality is control flow, not data.

use quarto_error_reporting::DiagnosticMessageBuilder;
use quarto_source_map::{SourceContext, SourceInfo};

use crate::Level;
use crate::diagnostic::Severity;

/// One problem found while validating, at any level. `code` and `column` are
/// present only when meaningful (spec problems have a code but no column;
/// pre-flight failures have neither); `span`/`related` drive source-highlighted
/// rendering and are never serialized; `kind` is the structured payload.
#[derive(Debug, serde::Serialize)]
pub struct Problem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The YAML span this problem points at (spec problems only). Display-only.
    #[serde(skip)]
    pub span: Option<SourceInfo>,
    #[serde(flatten)]
    pub kind: ProblemKind,
}

/// The structured payload behind a [`Problem`]. The serde tag (`"kind"`) is the
/// machine-readable discriminator; variants with fields flatten those fields
/// alongside it. Variants whose whole story is in [`Problem::message`] (the
/// pre-flight failures and the spec checks) carry no fields.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProblemKind {
    /// I/O failure reading the document.
    Io,
    /// The document is not parseable as YAML.
    Parse,
    /// The document failed structural validation against `schema.yaml`.
    Schema,
    /// The parquet file could not be read.
    Parquet,
    /// A table name was given but no such table exists.
    TableNotFound { available: Vec<String> },
    /// No table name was given and the dictionary describes more than one.
    AmbiguousTable { available: Vec<String> },
    /// A semantic spec check (`S##`) failed; the specific code is in
    /// [`Problem::code`] and the detail in [`Problem::message`].
    Spec,
    /// `M01` — declared type is not compatible with the type read from the data.
    TypeMismatch { declared: String, actual: String },
    /// `M02` — column described by the dictionary but absent from the data.
    MissingInData,
    /// `M03` — column present in the data but not described by the dictionary.
    ExtraInData { actual: String },
    /// `D01` — a `required` (or `primary_key`) column contains nulls. `rows`
    /// lists the first few offending row numbers (1-based); `count` is the total.
    NullsInRequired { count: usize, rows: Vec<usize> },
}

impl ProblemKind {
    /// The fixed rule code for kinds that have one. Spec checks return `None`
    /// because the code (`S01`…`S09`) varies per check and is set explicitly;
    /// pre-flight failures have no rule code.
    pub fn code(&self) -> Option<&'static str> {
        Some(match self {
            ProblemKind::TypeMismatch { .. } => "M01",
            ProblemKind::MissingInData => "M02",
            ProblemKind::ExtraInData { .. } => "M03",
            ProblemKind::NullsInRequired { .. } => "D01",
            _ => return None,
        })
    }

    /// The validation level this kind belongs to, when it maps to one. Pre-flight
    /// failures (`Io`/`Parse`/`Parquet`/…) return `None`.
    pub fn level(&self) -> Option<Level> {
        Some(match self {
            ProblemKind::Spec => Level::Spec,
            ProblemKind::TypeMismatch { .. }
            | ProblemKind::MissingInData
            | ProblemKind::ExtraInData { .. } => Level::Meta,
            ProblemKind::NullsInRequired { .. } => Level::Data,
            _ => return None,
        })
    }
}

impl Problem {
    /// A spec-level problem (`S##`), located by a YAML span.
    pub(crate) fn spec(
        code: &'static str,
        severity: Severity,
        message: impl Into<String>,
        span: SourceInfo,
    ) -> Self {
        Problem {
            code: Some(code),
            severity,
            message: message.into(),
            column: None,
            hint: None,
            span: Some(span),
            kind: ProblemKind::Spec,
        }
    }

    /// A column-level problem (`M##`/`D##`), located by a column name. The code
    /// is derived from `kind`, and the human message is rendered from the two.
    pub(crate) fn column(severity: Severity, column: impl Into<String>, kind: ProblemKind) -> Self {
        let column = column.into();
        Problem {
            code: kind.code(),
            severity,
            message: column_message(&column, &kind),
            column: Some(column),
            hint: None,
            span: None,
            kind,
        }
    }

    /// A pre-flight failure: it carries no location, only a kind and a message.
    pub(crate) fn preflight(kind: ProblemKind, message: impl Into<String>) -> Self {
        Problem {
            code: None,
            severity: Severity::Error,
            message: message.into(),
            column: None,
            hint: None,
            span: None,
            kind,
        }
    }

    /// Attach an advisory hint (rendered as an info bullet).
    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Render to display text. Span-located problems get full source
    /// highlighting; the rest render as a single `severity [code]: message` line
    /// (or just the message when there is no code).
    pub fn to_text(&self, ctx: &SourceContext) -> String {
        match &self.span {
            Some(span) => self.render_with_source(span, ctx),
            None => self.render_plain(),
        }
    }

    fn render_with_source(&self, span: &SourceInfo, ctx: &SourceContext) -> String {
        let header = self.code.map_or_else(String::new, |c| format!("[{c}]"));
        let mut builder = match self.severity {
            Severity::Error => DiagnosticMessageBuilder::error(header),
            Severity::Warning => DiagnosticMessageBuilder::warning(header),
        };
        builder = builder
            .problem(self.message.clone())
            .with_location(span.clone());
        if let Some(hint) = &self.hint {
            builder = builder.add_info(hint.clone());
        }
        builder.build().to_text(Some(ctx))
    }

    fn render_plain(&self) -> String {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut line = match self.code {
            Some(code) => format!("{severity} [{code}]: {}", self.message),
            None => self.message.clone(),
        };
        if let Some(hint) = &self.hint {
            line.push_str(&format!("\n  {hint}"));
        }
        line
    }
}

/// The human-readable message for a column-level problem.
fn column_message(column: &str, kind: &ProblemKind) -> String {
    match kind {
        ProblemKind::TypeMismatch { declared, actual } => {
            format!("column \"{column}\": declared {declared}, data is {actual}")
        }
        ProblemKind::MissingInData => {
            format!("column \"{column}\": described in dictionary but missing from data")
        }
        ProblemKind::ExtraInData { actual } => {
            format!("column \"{column}\": present in data ({actual}) but not in dictionary")
        }
        ProblemKind::NullsInRequired { count, rows } => format!(
            "column \"{column}\": required but has {count} null value{} ({})",
            if *count == 1 { "" } else { "s" },
            format_rows(rows, *count),
        ),
        // The remaining kinds are never column-located.
        _ => String::new(),
    }
}

/// Format offending row numbers for display: `rows: 3, 7, 12`, with a trailing
/// `, …` when there were more offenders than the recorded sample.
fn format_rows(rows: &[usize], count: usize) -> String {
    let listed = rows
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if count > rows.len() {
        format!("rows: {listed}, …")
    } else {
        format!("rows: {listed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_problem_json_flattens_kind_with_code_and_column() {
        let p = Problem::column(
            Severity::Error,
            "weight",
            ProblemKind::TypeMismatch {
                declared: "string".into(),
                actual: "number".into(),
            },
        );
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({
                "code": "M01",
                "severity": "error",
                "message": "column \"weight\": declared string, data is number",
                "column": "weight",
                "kind": "type_mismatch",
                "declared": "string",
                "actual": "number",
            })
        );
    }

    #[test]
    fn spec_problem_json_carries_code_and_message_no_column() {
        let p = Problem::spec("S07", Severity::Error, "bad column", SourceInfo::default())
            .with_hint("fix it");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["code"], "S07");
        assert_eq!(v["kind"], "spec");
        assert_eq!(v["message"], "bad column");
        assert_eq!(v["hint"], "fix it");
        assert!(v.get("column").is_none());
    }

    #[test]
    fn preflight_problem_json_has_kind_but_no_code() {
        let p = Problem::preflight(
            ProblemKind::AmbiguousTable {
                available: vec!["a".into(), "b".into()],
            },
            "the dictionary describes multiple tables",
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "ambiguous_table");
        assert_eq!(v["available"], serde_json::json!(["a", "b"]));
        assert_eq!(v["severity"], "error");
        assert!(v.get("code").is_none());
    }

    #[test]
    fn codes_and_levels() {
        assert_eq!(ProblemKind::MissingInData.code(), Some("M02"));
        assert_eq!(ProblemKind::MissingInData.level(), Some(Level::Meta));
        assert_eq!(
            ProblemKind::NullsInRequired {
                count: 1,
                rows: vec![2]
            }
            .level(),
            Some(Level::Data)
        );
        assert_eq!(ProblemKind::Io.code(), None);
        assert_eq!(ProblemKind::Io.level(), None);
    }

    #[test]
    fn row_formatting() {
        assert_eq!(format_rows(&[2], 1), "rows: 2");
        assert_eq!(format_rows(&[2, 5, 9], 3), "rows: 2, 5, 9");
        assert_eq!(format_rows(&[1, 2, 3, 4, 5], 8), "rows: 1, 2, 3, 4, 5, …");
    }
}
