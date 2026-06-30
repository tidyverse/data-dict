//! Integration tests that run the `data-dict` binary end to end.

use std::path::PathBuf;
use std::process::Command;

/// Running `data-dict` with no arguments lists every subcommand, including
/// nested ones like `skill read`.
///
/// When this snapshot changes (i.e. the set of commands changes), update the
/// command listing under "### Usage" in the repo-root README.md to match.
#[test]
fn no_args_lists_all_subcommands() {
    let output = Command::new(env!("CARGO_BIN_EXE_data-dict"))
        .output()
        .expect("failed to run data-dict");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    insta::assert_snapshot!(stdout);
}

/// A fixture that fails schema validation with two errors (S07, S08) and a warning (S09),
/// in that emission order. Validating its data skips the data comparison (the
/// dictionary has errors), so no source is ever read.
fn multi_error_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi-error-with-warning.yaml")
}

/// The default (text) output renders every diagnostic — both errors and the
/// warning — to stderr, in emission order.
#[test]
fn multiple_diagnostics_text_output() {
    let fixture = multi_error_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_data-dict"))
        .args(["validate-data"])
        .arg(&fixture)
        .output()
        .expect("failed to run data-dict");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is not valid UTF-8");
    insta::assert_snapshot!(sanitize(&stderr, &fixture.display().to_string()));
}

/// The `--json` output carries the same diagnostics as a structured array,
/// preserving severity, code, and emission order.
#[test]
fn multiple_diagnostics_json_output() {
    let fixture = multi_error_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_data-dict"))
        .args(["validate-data"])
        .arg(&fixture)
        .arg("--json")
        .output()
        .expect("failed to run data-dict");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    // Re-serialize so the snapshot is pretty-printed and key order is stable.
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is valid JSON");
    insta::assert_snapshot!(serde_json::to_string_pretty(&value).unwrap());
}

/// Strip terminal styling (ANSI SGR escapes and OSC-8 hyperlinks) and rewrite
/// the fixture's absolute path to a stable placeholder, so the rendered
/// diagnostic can be snapshotted.
fn sanitize(s: &str, fixture_path: &str) -> String {
    strip_terminal_escapes(s).replace(fixture_path, "<fixture>")
}

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
                    // CSI: run until a final byte in 0x40..=0x7e.
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1; // consume the final byte
                }
                b']' => {
                    // OSC: run until BEL or ST (ESC \).
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
