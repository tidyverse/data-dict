//! Diagnostics: the shared vocabulary for reporting problems found during
//! spec validation.
//!
//! A [`Diagnostic`] is one problem found in a `data-dict.yaml` document, with a
//! rule code, a [`Severity`], and the source span it concerns. A [`Diagnostics`]
//! collects them together with the [`SourceContext`] needed to render them. The
//! spec checks in [`crate::validate_spec`] push into a `Diagnostics` as they run; the
//! meta and data levels report their findings as [`crate::ColumnIssue`]s instead.

use quarto_error_reporting::DiagnosticMessageBuilder;
use quarto_source_map::{SourceContext, SourceInfo};

use crate::join_expr::ParseError;
use crate::model::Spanned;

/// Whether a diagnostic blocks validation (`Error`) or is purely advisory
/// (`Warning`). Errors fail validation; warnings are reported alongside a
/// successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A document's validation diagnostics together with the [`SourceContext`] needed to
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

/// The `serde` representation is the tool's JSON wire format: `code`,
/// `message`, `severity` (lowercase), and `hint` (omitted when absent). The
/// source spans are display-only and are not serialized.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub severity: Severity,
    #[serde(skip)]
    pub span: SourceInfo,
    /// Secondary spans with explanatory labels. Rendered as bulleted details
    /// below the primary location.
    #[serde(skip)]
    pub related: Vec<(SourceInfo, String)>,
    /// Advisory follow-up rendered as an info bullet, e.g. how to resolve a
    /// warning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn to_text(&self, ctx: &SourceContext) -> String {
        // The header is just the rule code (e.g. "Error: [S07]"); the message
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
            code: "S04",
            message: format!("`join` expression does not parse: {}", err.message),
            span: span.unwrap_or_else(|| join_text.span.clone()),
            related: Vec::new(),
            severity: Severity::Error,
            hint: None,
        }
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
