//! Language server for `data-dict.yaml` authoring.
//!
//! Wraps the core [`data_dict`] validator in an LSP server so editors can show
//! live diagnostics while a dictionary is being written. The server validates
//! the in-memory buffer (via [`data_dict::validate_str`]) on open, change, and
//! save, then publishes the findings as editor squiggles.
//!
//! Two diagnostic channels feed in: cross-table lint findings arrive as a
//! [`data_dict::Diagnostics`] (each carrying a `SourceInfo` span), while a
//! structural schema failure arrives as [`data_dict::Error::Invalid`] with a
//! resolved line/column [`location`](data_dict::SchemaError::location). Both are
//! converted to LSP ranges here; the core stays free of LSP types.

use std::collections::HashMap;
use std::path::Path;

use data_dict::data::{ColumnIssue, TableDataResult, validate_table_source};
use data_dict::model::Table;
use data_dict::{Error, SchemaError, Severity, SourceContext, SourceInfo};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// The source label attached to every diagnostic this server emits.
const SOURCE: &str = "data-dict";

/// The `workspace/executeCommand` that validates a dictionary against the data
/// at each table's `source`.
const VALIDATE_DATA: &str = "data-dict.validateData";

struct Backend {
    client: Client,
    /// Last-seen text of each open document, so a save with no inlined text can
    /// still be re-validated.
    documents: Mutex<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: SOURCE.to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![VALIDATE_DATA.to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        if params.command != VALIDATE_DATA {
            return Ok(None);
        }
        let Some(uri) = params
            .arguments
            .first()
            .and_then(Value::as_str)
            .and_then(|s| Url::parse(s).ok())
        else {
            return Ok(None);
        };
        let buffer = self.documents.lock().await.get(&uri).cloned();
        let result = tokio::task::spawn_blocking(move || validate_data_command(&uri, buffer))
            .await
            .unwrap_or_else(|_| json!({ "summary": "internal error", "diagnostics": [] }));
        Ok(Some(result))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.documents
            .lock()
            .await
            .insert(doc.uri.clone(), doc.text.clone());
        self.validate_and_publish(doc.uri, doc.text, Some(doc.version))
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full-sync: the final change event carries the entire document.
        let Some(change) = params.content_changes.into_iter().next_back() else {
            return;
        };
        let uri = params.text_document.uri;
        self.documents
            .lock()
            .await
            .insert(uri.clone(), change.text.clone());
        self.validate_and_publish(uri, change.text, Some(params.text_document.version))
            .await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = match params.text {
            Some(text) => text,
            None => self
                .documents
                .lock()
                .await
                .get(&uri)
                .cloned()
                .unwrap_or_default(),
        };
        self.validate_and_publish(uri, text, None).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().await.remove(&uri);
        // Clear any squiggles we published for the now-closed document.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

impl Backend {
    async fn validate_and_publish(&self, uri: Url, text: String, version: Option<i32>) {
        let target = uri.clone();
        // Validation is synchronous and CPU-bound; keep it off the LSP event loop.
        let diagnostics = tokio::task::spawn_blocking(move || document_diagnostics(&text, &target))
            .await
            .unwrap_or_default();
        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }
}

/// Validate `content` as a `data-dict.yaml` document and return the findings as
/// LSP diagnostics. `uri` identifies the document (used for diagnostic
/// attribution and related-information locations). An empty result means the
/// document is valid.
pub fn document_diagnostics(content: &str, uri: &Url) -> Vec<Diagnostic> {
    match data_dict::validate_str(content, uri.as_str()) {
        Ok(diagnostics) => diagnostics
            .items
            .iter()
            .map(|d| lint_to_lsp(d, &diagnostics.source, uri))
            .collect(),
        Err(Error::Invalid(schema_error)) => vec![schema_to_lsp(&schema_error)],
        // An unparseable buffer has no usable span; flag it at the top of the
        // document so the editor still shows the failure.
        Err(err @ Error::Parse(_)) => vec![whole_document(err.to_string())],
        // `validate_str` does no I/O, so `Io` cannot occur here.
        Err(Error::Io(_)) => Vec::new(),
    }
}

fn lint_to_lsp(
    diagnostic: &data_dict::Diagnostic,
    source: &SourceContext,
    uri: &Url,
) -> Diagnostic {
    let severity = match diagnostic.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    };
    let related: Vec<DiagnosticRelatedInformation> = diagnostic
        .related
        .iter()
        .map(|(span, label)| DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range: span_to_range(span, source),
            },
            message: label.clone(),
        })
        .collect();
    Diagnostic {
        range: span_to_range(&diagnostic.span, source),
        severity: Some(severity),
        code: Some(NumberOrString::String(diagnostic.code.to_string())),
        source: Some(SOURCE.to_string()),
        message: message_with_hint(&diagnostic.message, diagnostic.hint.as_deref()),
        related_information: (!related.is_empty()).then_some(related),
        ..Default::default()
    }
}

fn schema_to_lsp(error: &SchemaError) -> Diagnostic {
    let range = error
        .location
        .as_ref()
        .map(|l| Range {
            // `SchemaErrorLocation` is 1-indexed; LSP positions are 0-indexed.
            start: Position {
                line: l.start_line.saturating_sub(1) as u32,
                character: l.start_column.saturating_sub(1) as u32,
            },
            end: Position {
                line: l.end_line.saturating_sub(1) as u32,
                character: l.end_column.saturating_sub(1) as u32,
            },
        })
        .unwrap_or_default();
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(error.code.clone())),
        source: Some(SOURCE.to_string()),
        message: error.message.clone(),
        ..Default::default()
    }
}

/// A diagnostic spanning the very start of the document, for failures with no
/// resolvable source location.
fn whole_document(message: String) -> Diagnostic {
    Diagnostic {
        range: Range::default(),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(SOURCE.to_string()),
        message,
        ..Default::default()
    }
}

/// Convert a `SourceInfo` span to an LSP range. Spans that don't resolve to a
/// source location (e.g. generated nodes) collapse to the top of the document
/// rather than being dropped.
fn span_to_range(span: &SourceInfo, source: &SourceContext) -> Range {
    let start = span.map_offset(0, source);
    let end = span.map_offset(span.length(), source);
    match (start, end) {
        (Some(start), Some(end)) => Range {
            start: Position {
                line: start.location.row as u32,
                character: start.location.column as u32,
            },
            end: Position {
                line: end.location.row as u32,
                character: end.location.column as u32,
            },
        },
        _ => Range::default(),
    }
}

fn message_with_hint(message: &str, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("{message}\n{hint}"),
        None => message.to_string(),
    }
}

/// Validate the dictionary at `uri` against the data at each table's `source`,
/// returning `{ summary, diagnostics }` for the client to display. `buffer` is
/// the editor's current (possibly unsaved) text; when absent the file is read
/// from disk. Diagnostics are anchored to spans in the dictionary file.
pub fn validate_data_command(uri: &Url, buffer: Option<String>) -> Value {
    let Ok(dict_path) = uri.to_file_path() else {
        return json!({ "summary": "Data validation needs a file on disk.", "diagnostics": [] });
    };
    let base_dir = dict_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let content = match buffer {
        Some(text) => text,
        None => match std::fs::read_to_string(&dict_path) {
            Ok(text) => text,
            Err(err) => {
                return json!({ "summary": format!("Cannot read the dictionary: {err}"), "diagnostics": [] });
            }
        },
    };

    let (dict, diagnostics) = match data_dict::validate_and_lower_str(&content, uri.as_str()) {
        Ok(parsed) if parsed.1.is_ok() => parsed,
        _ => {
            return json!({
                "summary": "Fix the dictionary's errors before validating data.",
                "diagnostics": [],
            });
        }
    };
    let source = &diagnostics.source;

    let mut out = Vec::new();
    let mut tables_checked = 0;
    let mut tables_with_issues = 0;
    for table in dict.tables.values() {
        match validate_table_source(table, &base_dir) {
            TableDataResult::NoParquetSource => {}
            TableDataResult::Unreadable { path, error } => {
                tables_checked += 1;
                tables_with_issues += 1;
                let span = table
                    .source
                    .as_ref()
                    .and_then(|s| s.parquet.as_ref())
                    .map(|p| &p.span)
                    .unwrap_or(&table.name.span);
                out.push(data_diagnostic(
                    span_to_range(span, source),
                    &format!("cannot read source `{path}`: {error}"),
                    "source-unreadable",
                ));
            }
            TableDataResult::Compared(issues) => {
                tables_checked += 1;
                if !issues.is_empty() {
                    tables_with_issues += 1;
                }
                for issue in &issues {
                    out.push(issue_diagnostic(table, issue, source));
                }
            }
        }
    }

    let summary = if out.is_empty() {
        format!("Data matches the dictionary ({tables_checked} table(s) checked).")
    } else {
        format!(
            "{} issue(s) in {tables_with_issues} of {tables_checked} table(s).",
            out.len()
        )
    };
    json!({ "summary": summary, "diagnostics": out })
}

fn issue_diagnostic(table: &Table, issue: &ColumnIssue, source: &SourceContext) -> Value {
    match issue {
        ColumnIssue::TypeMismatch {
            column,
            declared,
            actual,
        } => {
            let span = table
                .column(column)
                .and_then(|c| c.col_type.as_ref().map(|t| &t.span))
                .or_else(|| table.column(column).map(|c| &c.name.span))
                .unwrap_or(&table.name.span);
            data_diagnostic(
                span_to_range(span, source),
                &format!(
                    "column `{column}`: dictionary declares `{declared}`, data has `{actual}`"
                ),
                "type-mismatch",
            )
        }
        ColumnIssue::MissingInData { column } => {
            let span = table
                .column(column)
                .map(|c| &c.name.span)
                .unwrap_or(&table.name.span);
            data_diagnostic(
                span_to_range(span, source),
                &format!(
                    "column `{column}` is described in the dictionary but missing from the data"
                ),
                "missing-in-data",
            )
        }
        ColumnIssue::ExtraInData { column, actual } => data_diagnostic(
            span_to_range(&table.name.span, source),
            &format!(
                "column `{column}` (`{actual}`) is in the data but not described in the dictionary"
            ),
            "extra-in-data",
        ),
    }
}

/// Build a diagnostic payload for the client. All data mismatches are errors
/// (LSP severity `1`).
fn data_diagnostic(range: Range, message: &str, code: &str) -> Value {
    json!({
        "range": {
            "start": { "line": range.start.line, "character": range.start.character },
            "end": { "line": range.end.line, "character": range.end.character },
        },
        "severity": 1,
        "code": code,
        "message": message,
    })
}

/// Run the language server over stdio until the client disconnects. Builds its
/// own Tokio runtime so callers (e.g. the CLI) can stay synchronous.
pub fn run_stdio() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| Backend {
            client,
            documents: Mutex::new(HashMap::new()),
        });
        Server::new(stdin, stdout, socket).serve(service).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnose(content: &str) -> Vec<Diagnostic> {
        let uri = Url::parse("file:///buffer.yaml").unwrap();
        document_diagnostics(content, &uri)
    }

    #[test]
    fn valid_document_has_no_diagnostics() {
        let content = "$version: 0.1.0\n$learn_more: http://data-dict.tidyverse.org/\n";
        assert!(diagnose(content).is_empty());
    }

    #[test]
    fn schema_failure_becomes_error_diagnostic() {
        // Missing the required `$version` key.
        let diagnostics = diagnose("tables: {}\n");
        assert_eq!(diagnostics.len(), 1);
        let d = &diagnostics[0];
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert!(matches!(&d.code, Some(NumberOrString::String(c)) if c.starts_with("Q-")));
        assert_eq!(d.source.as_deref(), Some(SOURCE));
    }

    #[test]
    fn lint_failure_carries_code_and_range() {
        // An `enum` column must declare `values`; omitting it is DD007.
        let content = "\
$version: 0.1.0
$learn_more: http://data-dict.tidyverse.org/
tables:
  t:
    description: A table.
    source:
      parquet: t.parquet
    columns:
      - name: c
        type: enum
        description: A column.
";
        let diagnostics = diagnose(content);
        let dd007 = diagnostics
            .iter()
            .find(|d| matches!(&d.code, Some(NumberOrString::String(c)) if c == "DD007"))
            .expect("expected a DD007 diagnostic");
        assert_eq!(dd007.severity, Some(DiagnosticSeverity::ERROR));
        // The span should point past the document header, not collapse to (0,0).
        assert!(dd007.range.start.line > 0);
    }

    #[test]
    fn unparseable_buffer_is_flagged() {
        // A tab in indentation is invalid YAML.
        let diagnostics = diagnose("foo:\n\t- bar\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "data-dict-lsp-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_data_reports_unreadable_source() {
        let dir = temp_dir();
        let dict = dir.join("data-dict.yaml");
        std::fs::write(
            &dict,
            "\
$version: 0.1.0
$learn_more: http://data-dict.tidyverse.org/
tables:
  t:
    description: A table.
    source:
      parquet: missing.parquet
    columns:
      - name: c
        type: string
        examples: [a, b]
        description: A column.
",
        )
        .unwrap();

        let uri = Url::from_file_path(&dict).unwrap();
        let result = validate_data_command(&uri, None);
        let diags = result["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0]["code"], "source-unreadable");
        assert_eq!(diags[0]["severity"], 1);
    }

    #[test]
    fn validate_data_refuses_invalid_dictionary() {
        // Missing `$version`: the dictionary itself is invalid, so data
        // validation should decline rather than report data diagnostics.
        let uri = Url::from_file_path("/tmp/data-dict.yaml").unwrap();
        let result = validate_data_command(&uri, Some("tables: {}\n".to_string()));
        assert!(result["diagnostics"].as_array().unwrap().is_empty());
        assert!(
            result["summary"]
                .as_str()
                .unwrap()
                .contains("Fix the dictionary"),
        );
    }
}
