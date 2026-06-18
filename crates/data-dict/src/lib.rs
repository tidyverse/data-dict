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
    /// The document is not parseable as YAML.
    Parse(quarto_yaml::Error),
    /// The document failed structural and/or semantic validation. The string
    /// is a rendered, human-readable report covering every diagnostic, with
    /// source-location highlighting.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::Invalid(s) => f.write_str(s),
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

/// Validate a `data-dict.yaml` file at `path` and return the lowered
/// [`DataDict`] model alongside its [`Diagnostics`]. Runs the same two passes
/// as [`validate`] — structural schema check then cross-table semantic linting.
/// The model is returned even when the diagnostics contain errors, since
/// lowering succeeds whenever the document is structurally sound.
pub fn validate_and_lower(path: &Path) -> Result<(DataDict, Diagnostics), Error> {
    let content = std::fs::read_to_string(path).map_err(Error::Io)?;
    let filename = path.display().to_string();

    let doc = quarto_yaml::parse_file(&content, &filename).map_err(Error::Parse)?;

    let mut source = SourceContext::new();
    let file_id = quarto_yaml::file_id_for_filename(&filename);
    source.add_file_with_id(file_id, filename, Some(content));

    let registry = SchemaRegistry::new();
    if let Err(err) = quarto_yaml_validation::validate(&doc, schema(), &registry, &source) {
        let diagnostic = ValidationDiagnostic::from_validation_error(&err, &source);
        return Err(Error::Invalid(diagnostic.to_text(&source)));
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
