//! Core library for the `data-dict.yaml` specification.
//!
//! Currently exposes [`validate`], which checks a file against the embedded
//! schema for spec version 0.1 (see `schema.yaml` at the repo root).

use std::path::Path;
use std::sync::OnceLock;

use quarto_source_map::SourceContext;
use quarto_yaml_validation::{Schema, SchemaRegistry, ValidationDiagnostic};

const SCHEMA_YAML: &str = include_str!("../../../schema.yaml");

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let yaml = quarto_yaml::parse(SCHEMA_YAML)
            .expect("embedded schema.yaml must be parseable YAML");
        Schema::from_yaml(&yaml)
            .expect("embedded schema.yaml must compile to a valid schema")
    })
}

/// Errors returned by [`validate`].
#[derive(Debug)]
pub enum Error {
    /// I/O failure reading the document.
    Io(std::io::Error),
    /// The document is not parseable as YAML.
    Parse(quarto_yaml::Error),
    /// The document parses but does not conform to the schema. The string is a
    /// rendered, human-readable diagnostic with source location highlighting.
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

/// Validate a `data-dict.yaml` file at `path` against the embedded schema.
pub fn validate(path: &Path) -> Result<(), Error> {
    let content = std::fs::read_to_string(path).map_err(Error::Io)?;
    let filename = path.display().to_string();

    let doc = quarto_yaml::parse_file(&content, &filename).map_err(Error::Parse)?;

    let mut source_ctx = SourceContext::new();
    let file_id = quarto_yaml::file_id_for_filename(&filename);
    source_ctx.add_file_with_id(file_id, filename, Some(content));

    let registry = SchemaRegistry::new();
    match quarto_yaml_validation::validate(&doc, schema(), &registry, &source_ctx) {
        Ok(()) => Ok(()),
        Err(err) => {
            let diagnostic = ValidationDiagnostic::from_validation_error(&err, &source_ctx);
            Err(Error::Invalid(diagnostic.to_text(&source_ctx)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_compiles() {
        let _ = schema();
    }
}
