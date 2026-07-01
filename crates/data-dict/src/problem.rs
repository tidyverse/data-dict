//! A single problem vocabulary for every validation level.
//!
//! A [`Problem`] is anything that can be wrong while validating — a failed spec
//! check, a column that disagrees with the data, an unreadable file. They vary
//! only on where the problem points (a YAML span / a named column / nowhere) and
//! what structured payload they carry; a code, a severity, and a message are
//! common to all. The structured payload is the flattened
//! [`ProblemKind`] tag, so a single `#[derive(Serialize)]` produces the JSON for
//! every kind uniformly.
//!
//! There is no `fatal` field by design: a problem that must stop the run is the
//! last thing a level pushes before returning, and a problem that blocks the
//! next level is caught by the driver checking [`ProblemSet::status`] before
//! descending. Fatality is control flow, not data.

use quarto_error_reporting::DiagnosticMessageBuilder;
use quarto_source_map::{SourceContext, SourceInfo};

use crate::Level;

/// Whether a problem blocks validation (`Error`) or is purely advisory
/// (`Warning`). Errors fail validation; warnings are reported alongside a
/// successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// The overall verdict for a [`ProblemSet`]: the worst severity present, or
/// `Ok` when there is nothing to report. Only `Error` fails validation; `Ok`
/// and `Warning` both pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warning,
    Error,
}

impl Status {
    /// Whether this verdict fails validation (i.e. is an error).
    pub fn failed(self) -> bool {
        self == Status::Error
    }
}

/// One problem found while validating, at any level. `code` and `column` are
/// present only when meaningful (spec problems have a code but no column;
/// pre-flight failures have neither); `span`/`context` drive source-highlighted
/// rendering and are never serialized; `kind` is the structured payload.
#[derive(Debug, serde::Serialize)]
pub struct Problem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// What the spec expects, stated independently of this occurrence. When
    /// present it leads the rendering (the title line) and `message` reports
    /// what was found instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// The YAML span this problem points at (spec problems only). Display-only.
    #[serde(skip)]
    pub span: Option<SourceInfo>,
    /// Enclosing source lines shown faded (unlabelled) around the primary
    /// label, locating it within the document (e.g. the table and column a bad
    /// value sits in). Display-only; ordered outermost-first.
    #[serde(skip)]
    pub context: Vec<SourceInfo>,
    #[serde(flatten)]
    pub kind: ProblemKind,
}

/// A resolved source span as 0-based line/column bounds, for JSON consumers.
/// Lines and columns count from 0, following the LSP convention; the
/// human-rendered diagnostics show the same positions 1-based.
#[derive(Debug, serde::Serialize)]
pub struct SpanLocation {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
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
    /// A semantic spec check (`S##`) failed; the specific code is in
    /// [`Problem::code`] and the detail in [`Problem::message`].
    Spec,
    /// `M01` — declared type is not compatible with the type read from the data.
    TypeMismatch { declared: String, actual: String },
    /// `M02` — column described by the dictionary but absent from the data.
    MissingInData,
    /// `M03` — column present in the data but not described by the dictionary.
    ExtraInData { actual: String },
    /// `M04` — a table validated against data declares no `source`.
    MissingSource,
    /// `M05` — a table's `source` is declared but its data can't be read (the
    /// `parquet` file is absent or unreadable).
    UnreadableSource,
    /// `D01` — a `required` (or `primary_key`) column contains nulls. `rows`
    /// lists the first few offending row numbers (1-based); `count` is the total.
    NullsInRequired { count: usize, rows: Vec<usize> },
    /// `D02` — a unique column or composite primary key contains duplicates.
    DuplicateValues {
        columns: Vec<String>,
        count: usize,
        rows: Vec<usize>,
    },
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
            ProblemKind::MissingSource => "M04",
            ProblemKind::UnreadableSource => "M05",
            ProblemKind::NullsInRequired { .. } => "D01",
            ProblemKind::DuplicateValues { .. } => "D02",
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
            | ProblemKind::ExtraInData { .. }
            | ProblemKind::MissingSource
            | ProblemKind::UnreadableSource => Level::Meta,
            ProblemKind::NullsInRequired { .. } | ProblemKind::DuplicateValues { .. } => {
                Level::Data
            }
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
            expected: None,
            span: Some(span),
            context: Vec::new(),
            kind: ProblemKind::Spec,
        }
    }

    /// `M03` — a column present in the data but not described by the
    /// dictionary. It exists only in the data, so it has no dictionary location
    /// and is named in the message rather than highlighted in source.
    pub(crate) fn undocumented_column(name: &str, actual_type: impl Into<String>) -> Self {
        let actual = actual_type.into();
        Problem {
            code: Some("M03"),
            severity: Severity::Warning,
            message: format!("`{name}` is in the data (`{actual}`) but not the dictionary"),
            column: Some(name.to_string()),
            expected: Some(
                "Every column in the data should be described in the dictionary.".into(),
            ),
            span: None,
            context: Vec::new(),
            kind: ProblemKind::ExtraInData { actual },
        }
    }

    /// A pre-flight failure: it carries no location, only a kind and a message.
    pub(crate) fn preflight(kind: ProblemKind, message: impl Into<String>) -> Self {
        Problem {
            code: None,
            severity: Severity::Error,
            message: message.into(),
            column: None,
            expected: None,
            span: None,
            context: Vec::new(),
            kind,
        }
    }

    /// Resolve the primary span to 0-based line/column bounds (LSP convention)
    /// for JSON consumers, e.g. an editor placing the diagnostic in the file.
    /// `None` for problems with no span (column-located and pre-flight failures).
    pub fn location(&self, ctx: &SourceContext) -> Option<SpanLocation> {
        let span = self.span.as_ref()?;
        let start = span.map_offset(0, ctx)?.location;
        let end = span.map_offset(span.length(), ctx)?.location;
        Some(SpanLocation {
            start_line: start.row,
            start_column: start.column,
            end_line: end.row,
            end_column: end.column,
        })
    }

    /// Render to display text. Span-located problems get full source
    /// highlighting; the rest render as a `severity [code]: message` line (or
    /// just the message when there is no code). When `expected` is set it leads
    /// the output and `message` follows on its own line as the "found" detail.
    pub fn to_text(&self, ctx: &SourceContext) -> String {
        match &self.span {
            Some(span) => self.render_with_source(span, ctx),
            None => self.render_plain(),
        }
    }

    fn render_with_source(&self, span: &SourceInfo, ctx: &SourceContext) -> String {
        let header = match (self.code, &self.expected) {
            (Some(c), Some(e)) => format!("[{c}] {e}"),
            (Some(c), None) => format!("[{c}]"),
            (None, Some(e)) => e.clone(),
            (None, None) => String::new(),
        };
        let mut builder = match self.severity {
            Severity::Error => DiagnosticMessageBuilder::error(header),
            Severity::Warning => DiagnosticMessageBuilder::warning(header),
        };
        for ctx_span in &self.context {
            builder = builder.add_faded_at("", ctx_span.clone());
        }
        builder = builder
            .problem(self.message.clone())
            .with_location(span.clone());
        builder.build().to_text(Some(ctx))
    }

    fn render_plain(&self) -> String {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let headline = self.expected.as_deref().unwrap_or(&self.message);
        let mut line = match self.code {
            Some(code) => format!("{severity} [{code}]: {headline}"),
            None => headline.to_string(),
        };
        if self.expected.is_some() {
            line.push_str(&format!("\n  {}", self.message));
        }
        line
    }
}

/// Every problem found while validating a document, with the [`SourceContext`]
/// needed to render the span-located ones. Levels push into a `ProblemSet` as
/// they run; the driver descends to the next level only while [`status`] is not
/// [`Status::Error`].
///
/// [`status`]: ProblemSet::status
#[derive(Debug)]
pub struct ProblemSet {
    pub items: Vec<Problem>,
    pub source: SourceContext,
}

impl ProblemSet {
    /// An empty set tied to a source context, ready for checks to push into.
    pub fn new(source: SourceContext) -> Self {
        ProblemSet {
            items: Vec::new(),
            source,
        }
    }

    /// A set holding a single pre-flight failure, with no source. Used when
    /// validation could not begin (unreadable or unparseable document).
    pub fn from_preflight(kind: ProblemKind, message: impl Into<String>) -> Self {
        let mut set = ProblemSet::new(SourceContext::new());
        set.push(Problem::preflight(kind, message));
        set
    }

    pub fn push(&mut self, problem: Problem) {
        self.items.push(problem);
    }

    /// Push a problem located in the document: `expected` states the rule,
    /// `actual` reports what was found, and `spans` locates it — the **last**
    /// span is the primary highlight carrying `actual`, and any preceding spans
    /// are shown faded as enclosing context (outermost-first, e.g. the table
    /// then the column). `spans` must be non-empty.
    fn push_located_problem(
        &mut self,
        code: &'static str,
        kind: ProblemKind,
        severity: Severity,
        expected: impl Into<String>,
        actual: impl Into<String>,
        spans: impl IntoIterator<Item = SourceInfo>,
    ) {
        let mut spans: Vec<SourceInfo> = spans.into_iter().collect();
        let primary = spans
            .pop()
            .expect("a located problem needs at least the primary span");
        self.push(Problem {
            code: Some(code),
            severity,
            message: actual.into(),
            column: None,
            expected: Some(expected.into()),
            span: Some(primary),
            context: spans,
            kind,
        });
    }

    /// Push a spec problem (`S##`) at error severity; see
    /// [`push_located_problem`](Self::push_located_problem) for the `spans`
    /// convention. A spec check with no rule statement (a bare parse failure)
    /// builds its [`Problem`] directly.
    pub(crate) fn push_spec_error(
        &mut self,
        code: &'static str,
        expected: impl Into<String>,
        actual: impl Into<String>,
        spans: impl IntoIterator<Item = SourceInfo>,
    ) {
        self.push_located_problem(
            code,
            ProblemKind::Spec,
            Severity::Error,
            expected,
            actual,
            spans,
        );
    }

    /// Push a spec problem at warning severity; see [`push_spec_error`](Self::push_spec_error).
    pub(crate) fn push_spec_warning(
        &mut self,
        code: &'static str,
        expected: impl Into<String>,
        actual: impl Into<String>,
        spans: impl IntoIterator<Item = SourceInfo>,
    ) {
        self.push_located_problem(
            code,
            ProblemKind::Spec,
            Severity::Warning,
            expected,
            actual,
            spans,
        );
    }

    /// Push a metadata/data problem located at the dictionary node it concerns;
    /// `code` and any structured payload come from `kind`. See
    /// [`push_located_problem`](Self::push_located_problem) for the `spans`
    /// convention.
    pub(crate) fn push_located(
        &mut self,
        kind: ProblemKind,
        severity: Severity,
        expected: impl Into<String>,
        actual: impl Into<String>,
        spans: impl IntoIterator<Item = SourceInfo>,
    ) {
        let code = kind
            .code()
            .expect("a located metadata/data kind has a code");
        self.push_located_problem(code, kind, severity, expected, actual, spans);
    }

    /// Order span-located problems by their position in the document. Checks
    /// emit in their own order, but readers expect source order; the sort is
    /// stable, and problems without a resolvable span (column/pre-flight) sort
    /// last.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|p| {
            p.span
                .as_ref()
                .and_then(|s| s.resolve_byte_range())
                .map(|(file, start, _)| (file, start))
                .unwrap_or((usize::MAX, usize::MAX))
        });
    }

    /// The overall verdict: [`Status::Error`] if anything is an error,
    /// [`Status::Warning`] if there are only warnings, [`Status::Ok`] if there
    /// is nothing to report.
    pub fn status(&self) -> Status {
        let mut status = Status::Ok;
        for p in &self.items {
            match p.severity {
                Severity::Error => return Status::Error,
                Severity::Warning => status = Status::Warning,
            }
        }
        status
    }

    /// Render every problem to display text, in their current order.
    pub fn render(&self) -> Vec<String> {
        self.items.iter().map(|p| p.to_text(&self.source)).collect()
    }
}

/// Build a sub-span pointing at `[start, end)` byte offsets within `parent`.
/// Returns `None` if `parent` does not resolve to a single contiguous byte
/// range (Concat / FilterProvenance variants, which YAML scalar literals
/// shouldn't produce in practice).
pub(crate) fn subspan(parent: &SourceInfo, start: usize, end: usize) -> Option<SourceInfo> {
    parent.resolve_byte_range()?;
    Some(SourceInfo::substring(parent.clone(), start, end))
}

/// Format offending row numbers for display: `rows: 3, 7, 12`, with a trailing
/// `, …` when there were more offenders than the recorded sample. The label is
/// singular (`row: 3`) for a lone offender.
pub(crate) fn format_rows(rows: &[usize], count: usize) -> String {
    let label = if count == 1 { "row" } else { "rows" };
    let listed = rows
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if count > rows.len() {
        format!("{label}: {listed}, …")
    } else {
        format!("{label}: {listed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undocumented_column_json_flattens_kind_with_code_and_column() {
        let p = Problem::undocumented_column("notes", "string");
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({
                "code": "M03",
                "severity": "warning",
                "message": "`notes` is in the data (`string`) but not the dictionary",
                "expected": "Every column in the data should be described in the dictionary.",
                "column": "notes",
                "kind": "extra_in_data",
                "actual": "string",
            })
        );
    }

    #[test]
    fn spec_problem_json_carries_code_and_message_no_column() {
        let p = Problem::spec("S07", Severity::Error, "bad column", SourceInfo::for_test());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["code"], "S07");
        assert_eq!(v["kind"], "spec");
        assert_eq!(v["message"], "bad column");
        assert!(v.get("column").is_none());
    }

    #[test]
    fn plain_problem_renders_expected_then_found() {
        let p = Problem::undocumented_column("notes", "string");
        assert_eq!(
            p.to_text(&SourceContext::new()),
            "warning [M03]: Every column in the data should be described in the dictionary.\n  \
             `notes` is in the data (`string`) but not the dictionary",
        );
    }

    #[test]
    fn preflight_problem_json_has_kind_but_no_code() {
        let p = Problem::preflight(
            ProblemKind::TableNotFound {
                available: vec!["a".into(), "b".into()],
            },
            "table \"x\" is not in the data dictionary",
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "table_not_found");
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
        assert_eq!(
            ProblemKind::DuplicateValues {
                columns: vec!["id".into()],
                count: 1,
                rows: vec![2],
            }
            .code(),
            Some("D02")
        );
        assert_eq!(ProblemKind::Io.code(), None);
        assert_eq!(ProblemKind::Io.level(), None);
    }

    #[test]
    fn row_formatting() {
        assert_eq!(format_rows(&[2], 1), "row: 2");
        assert_eq!(format_rows(&[2, 5, 9], 3), "rows: 2, 5, 9");
        assert_eq!(format_rows(&[1, 2, 3, 4, 5], 8), "rows: 1, 2, 3, 4, 5, …");
    }
}
