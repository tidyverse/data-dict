//! Integration tests that run the `data-dict` binary end to end.

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
