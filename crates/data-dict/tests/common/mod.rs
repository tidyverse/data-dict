//! Shared fixtures for the metadata and data comparison integration tests.
//!
//! Each helper writes a small parquet file and/or dictionary YAML to a unique
//! temp dir. Both `validate_meta` and `validate_data` test binaries include this
//! module, so not every helper is used by each — hence the blanket allow.
#![allow(dead_code)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use indoc::indoc;
use parquet::data_type::{ByteArray, ByteArrayType, DoubleType};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp directory for one test's fixtures.
pub fn temp_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "data-dict-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a parquet file with a string `name` column and a double `weight`
/// column.
pub fn write_parquet(path: &Path) {
    let message = indoc! {"
        message schema {
            REQUIRED BYTE_ARRAY name (UTF8);
            REQUIRED DOUBLE weight;
        }
    "};
    let schema = Arc::new(parse_message_type(message).unwrap());
    let props = Arc::new(WriterProperties::builder().build());
    let file = File::create(path).unwrap();
    let mut writer = SerializedFileWriter::new(file, schema, props).unwrap();
    let mut rg = writer.next_row_group().unwrap();

    let mut col = rg.next_column().unwrap().unwrap();
    col.typed::<ByteArrayType>()
        .write_batch(
            &[ByteArray::from("otter"), ByteArray::from("seal")],
            None,
            None,
        )
        .unwrap();
    col.close().unwrap();

    let mut col = rg.next_column().unwrap().unwrap();
    col.typed::<DoubleType>()
        .write_batch(&[1.0_f64, 2.0], None, None)
        .unwrap();
    col.close().unwrap();

    rg.close().unwrap();
    writer.close().unwrap();
}

/// Write `yaml` to `<dir>/dict.yaml` and return the path.
pub fn write_yaml(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("dict.yaml");
    std::fs::write(&path, yaml).unwrap();
    path
}

/// The boilerplate top-level header — the required `$version` and recommended
/// `$learn_more` — that nearly every dictionary needs. Exactly two lines, so a
/// `body` written beneath it starts at line 3.
pub const HEADER: &str = "$version: \"0.1.0\"\n$learn_more: http://data-dict.tidyverse.org/\n";

/// Write `body` to `<dir>/dict.yaml` with [`HEADER`] prepended, returning the
/// path, so a test shows only the part under test. Use [`write_yaml`] for the
/// few cases that must omit or alter the header keys themselves.
pub fn write_dict(dir: &Path, body: &str) -> PathBuf {
    write_yaml(dir, &format!("{HEADER}{body}"))
}

/// Make a source-highlighted diagnostic snapshottable: strip terminal styling
/// (ANSI escapes and OSC-8 hyperlinks) and rewrite the absolute `dir` prefix off
/// its paths, both of which vary per run. `dir` is the temp dir for inline
/// documents (leaving the bare `dict.yaml`) or the fixtures root for fixture
/// documents (leaving e.g. `spec/s01-….yaml`). Backslashes are normalized to
/// `/` so Windows paths match the snapshots.
pub fn sanitize(rendered: &str, dir: &Path) -> String {
    let dir_prefix = format!("{}/", dir.display()).replace('\\', "/");
    strip_terminal_escapes(rendered)
        .replace('\\', "/")
        .replace(&dir_prefix, "")
}

/// A validation outcome captured for snapshotting: the YAML `source` that was
/// validated and the `rendered` diagnostic it produced. Snapshot it with
/// [`assert_snapshot!`], which shows the source and the diagnostic together.
pub struct Diagnostic {
    pub source: String,
    pub rendered: String,
}

/// Separates the validated YAML source from the rendered diagnostic in a
/// combined snapshot body.
const RULE: &str = "────────────────────────────────────────";

impl Diagnostic {
    /// The snapshot body: the YAML source, a rule, then the rendered diagnostic.
    pub fn body(&self) -> String {
        format!("{}{RULE}\n{}", self.source, self.rendered)
    }
}

/// Capture a validation result for snapshotting: read the validated YAML from
/// `path` as the source and [`sanitize`] `rendered` against the file's own
/// directory (turning its absolute paths into the bare `dict.yaml`).
pub fn diagnostic(path: &Path, rendered: &str) -> Diagnostic {
    Diagnostic {
        source: std::fs::read_to_string(path).unwrap(),
        rendered: sanitize(rendered, path.parent().unwrap()),
    }
}

/// Snapshot a [`Diagnostic`] as its YAML source followed by the rendered
/// diagnostic (see [`Diagnostic::body`]), so the `.snap` shows input and output
/// together. The source lives in the snapshot body rather than metadata, so it
/// is readable and self-maintaining: editing the YAML shows up as an ordinary
/// snapshot diff. The redundant `expression:` header is omitted.
macro_rules! assert_snapshot {
    ($diagnostic:expr) => {{
        let body = $diagnostic.body();
        let mut settings = insta::Settings::clone_current();
        settings.set_omit_expression(true);
        let _guard = settings.bind_to_scope();
        insta::assert_snapshot!(body);
    }};
}
pub(crate) use assert_snapshot;

/// Remove ANSI SGR sequences (`ESC [ ... m`) and OSC-8 hyperlink wrappers
/// (`ESC ] 8 ; ; ... BEL|ST`) while leaving the visible text intact.
fn strip_terminal_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                b']' => {
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 2,
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("stripping ASCII escapes preserves UTF-8")
}
