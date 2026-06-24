//! Core library for the `data-dict.yaml` specification.
//!
//! [`validate`] runs two passes on a document:
//!
//! 1. Structural validation against the embedded `schema.yaml` for spec
//!    version 0.1.0, via the `quarto-yaml-validation` crate.
//! 2. Cross-table semantic linting (see [`lint`]) — foreign-key targets,
//!    `join` expression parsing, `conflicts` column resolution, cardinality
//!    consistency.
//!
//! The second pass only runs if the first succeeds: there is no point
//! chasing FK references in a document whose `tables` block is malformed.
//!
//! Linting can also surface *warnings* (e.g. a missing `$learn_more` key).
//! Warnings do not fail validation. Errors and warnings are returned together
//! in a single vector, sorted by their position in the document, so they read
//! in source order when rendered; the caller decides validity by checking for
//! any [`Severity::Error`].

use std::path::Path;
use std::sync::OnceLock;

use quarto_yaml_validation::{Schema, SchemaRegistry, ValidationDiagnostic};

pub mod data;
pub mod join_expr;
pub mod lint;
pub mod lower;
pub mod model;

pub use lint::{Diagnostic, Diagnostics, Severity};
pub use quarto_source_map::SourceContext;

use model::DataDict;

const SCHEMA_YAML: &str = include_str!("../../../schema.yaml");

/// The full text of the `data-dict.yaml` specification (`site/spec.md`),
/// embedded at compile time so the CLI can print it without a network or
/// filesystem dependency.
pub const SPEC_MD: &str = include_str!("../../../site/spec.md");

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let yaml =
            quarto_yaml::parse(SCHEMA_YAML).expect("embedded schema.yaml must be parseable YAML");
        Schema::from_yaml(&yaml).expect("embedded schema.yaml must compile to a valid schema")
    })
}

/// Errors returned by [`validate`].
#[derive(Debug)]
pub enum Error {
    /// I/O failure reading the document.
    Io(std::io::Error),
    /// The document is not parseable as YAML. Boxed because `quarto_yaml::Error`
    /// is large and would otherwise bloat every `Result` in this module.
    Parse(Box<quarto_yaml::Error>),
    /// The document failed structural schema validation. Carries the structured
    /// diagnostic (code, message, source location) so programmatic consumers
    /// such as an LSP can act on it; its `Display` renders the same
    /// human-readable, source-highlighted report as before. Boxed to keep
    /// `Error` small, like [`Error::Parse`].
    Invalid(Box<SchemaError>),
}

/// A structural schema-validation failure, retained in structured form.
///
/// Schema validation stops at the first failure — linting cannot run on a
/// document whose shape is wrong — so this represents a single diagnostic.
/// [`Display`](std::fmt::Display) reproduces the full source-highlighted report;
/// the fields expose the same information for programmatic consumers (e.g. an
/// LSP mapping it to an editor squiggle).
#[derive(Debug, Clone)]
pub struct SchemaError {
    /// Machine-readable validation code (e.g. `Q-1-10`).
    pub code: String,
    /// Human-readable description of the failure.
    pub message: String,
    /// Source location of the failure, when one is known.
    pub location: Option<SchemaErrorLocation>,
    /// Pre-rendered, source-highlighted report (what `Display` emits).
    rendered: String,
}

/// Resolved source location of a [`SchemaError`]: byte offsets together with
/// 1-indexed line/column positions, matching the convention used in the
/// rendered report.
#[derive(Debug, Clone)]
pub struct SchemaErrorLocation {
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl SchemaError {
    fn from_diagnostic(diagnostic: &ValidationDiagnostic, source: &SourceContext) -> Self {
        let location = diagnostic
            .source_range
            .as_ref()
            .map(|r| SchemaErrorLocation {
                start_offset: r.start_offset,
                end_offset: r.end_offset,
                start_line: r.start_line,
                start_column: r.start_column,
                end_line: r.end_line,
                end_column: r.end_column,
            });
        SchemaError {
            code: diagnostic.code.clone(),
            message: diagnostic.message(),
            location,
            rendered: diagnostic.to_text(source),
        }
    }
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.rendered)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Parse(e) => Some(e),
            Error::Invalid(_) => None,
        }
    }
}

/// Validate a `data-dict.yaml` file at `path`: structural schema check
/// followed by cross-table semantic linting. Returns the [`Diagnostics`] —
/// every lint diagnostic (errors and warnings, in emission order) bundled with
/// the source context needed to render them. [`Diagnostics::is_ok`] reports
/// whether the document is valid.
///
/// [`Error`] is reserved for failures that prevent linting altogether: I/O,
/// unparseable YAML, or a structurally invalid document.
pub fn validate(path: &Path) -> Result<Diagnostics, Error> {
    validate_and_lower(path).map(|(_, diagnostics)| diagnostics)
}

/// Validate in-memory `data-dict.yaml` `content`, attributing diagnostics to
/// `filename`. The content twin of [`validate`]: it performs no I/O, so an
/// editor or LSP can validate an unsaved buffer. See [`validate`] for the
/// meaning of the result.
pub fn validate_str(content: &str, filename: &str) -> Result<Diagnostics, Error> {
    validate_and_lower_str(content, filename).map(|(_, diagnostics)| diagnostics)
}

/// Validate a `data-dict.yaml` file at `path` and return the lowered
/// [`DataDict`] model alongside its [`Diagnostics`]. Runs the same two passes
/// as [`validate`] — structural schema check then cross-table semantic linting.
/// The model is returned even when the diagnostics contain errors, since
/// lowering succeeds whenever the document is structurally sound.
pub fn validate_and_lower(path: &Path) -> Result<(DataDict, Diagnostics), Error> {
    let content = std::fs::read_to_string(path).map_err(Error::Io)?;
    let filename = path.display().to_string();
    validate_and_lower_str(&content, &filename)
}

/// Validate and lower in-memory `data-dict.yaml` `content`, attributing
/// diagnostics to `filename`. The content twin of [`validate_and_lower`]: it
/// performs no I/O, so an editor or LSP can validate an unsaved buffer.
pub fn validate_and_lower_str(
    content: &str,
    filename: &str,
) -> Result<(DataDict, Diagnostics), Error> {
    let doc = quarto_yaml::parse_file(content, filename).map_err(|e| Error::Parse(Box::new(e)))?;

    let mut source = SourceContext::new();
    let file_id = quarto_yaml::file_id_for_filename(filename);
    source.add_file_with_id(file_id, filename.to_string(), Some(content.to_string()));

    let registry = SchemaRegistry::new();
    if let Err(err) = quarto_yaml_validation::validate(&doc, schema(), &registry, &source) {
        let diagnostic = ValidationDiagnostic::from_validation_error(&err, &source);
        return Err(Error::Invalid(Box::new(SchemaError::from_diagnostic(
            &diagnostic,
            &source,
        ))));
    }

    let mut diagnostics = Diagnostics::new(source);
    let dict = lower::lower(&doc, &mut diagnostics);
    lint::lint(&dict, &mut diagnostics);
    lint::check_learn_more(&doc, &mut diagnostics);
    diagnostics.sort();

    Ok((dict, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_compiles() {
        let _ = schema();
    }
}
