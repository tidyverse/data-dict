use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use data_dict::data::{ColumnIssue, DataError, DataReport};
use data_dict::{Diagnostic, Diagnostics, Severity};

#[derive(Parser)]
#[command(name = "data-dict", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a data-dict.yaml file or directory against the schema [default: .]
    ValidateSchema { path: Option<PathBuf> },
    /// Print the data-dict.yaml specification
    Spec,
    /// Work with parquet files
    Parquet {
        #[command(subcommand)]
        command: ParquetCommand,
    },
    /// Agents: read these skills to learn how to work with data-dict files
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Skill for reading and understanding a data dictionary
    Read,
    /// Skill for creating or updating a data dictionary
    Write,
}

const READ_SKILL: &str = include_str!("../skills/read-data-dict.md");
const WRITE_SKILL: &str = include_str!("../skills/write-data-dict.md");

#[derive(Subcommand)]
enum ParquetCommand {
    /// Print column types for a parquet file
    Types { path: PathBuf },
    /// Validate a parquet file's columns against a data dictionary
    Validate {
        dict: PathBuf,
        parquet: PathBuf,
        #[arg(long)]
        table: Option<String>,
        /// Emit results as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        print_all_subcommands();
        return ExitCode::SUCCESS;
    };
    match command {
        Command::ValidateSchema { path } => {
            let path = match resolve_dict_path(path) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            };
            match data_dict::validate(&path) {
                Ok(diagnostics) => {
                    for line in diagnostics.render() {
                        eprintln!("{line}");
                    }
                    if diagnostics.has_errors() {
                        ExitCode::FAILURE
                    } else {
                        println!("{}: ok", path.display());
                        ExitCode::SUCCESS
                    }
                }
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Spec => {
            print!("{}", data_dict::SPEC_MD);
            ExitCode::SUCCESS
        }
        Command::Parquet {
            command: ParquetCommand::Types { path },
        } => match data_dict_parquet::column_type_info(&path) {
            Ok(cols) => {
                print_types_table(&cols);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Command::Parquet {
            command:
                ParquetCommand::Validate {
                    dict,
                    parquet,
                    table,
                    json,
                },
        } => {
            let dict = match resolve_dict_path(Some(dict)) {
                Ok(dict) => dict,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            };
            let (diagnostics, result) =
                data_dict::data::validate_parquet(&dict, &parquet, table.as_deref());
            if json {
                println!("{}", validate_result_to_json(&diagnostics, &result));
            } else {
                for line in diagnostics.render() {
                    eprintln!("{line}");
                }
                match &result {
                    Ok(report) if report.is_clean() && diagnostics.is_ok() => {
                        println!("{}: ok", parquet.display())
                    }
                    Ok(report) => eprint!("{report}"),
                    Err(err) => eprintln!("{err}"),
                }
            }
            let data_failed = match &result {
                Ok(report) => report.has_errors(),
                Err(_) => true,
            };
            if diagnostics.has_errors() || data_failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Command::Skill { command } => {
            let skill = match command {
                SkillCommand::Read => READ_SKILL,
                SkillCommand::Write => WRITE_SKILL,
            };
            print!("{skill}");
            ExitCode::SUCCESS
        }
    }
}

/// Print every (leaf) subcommand, including nested ones like `skill read`.
fn print_all_subcommands() {
    print!("{}", subcommands_listing());
}

/// Build the listing of all leaf subcommands, including nested ones like
/// `skill read`. The top-level `help` command is kept, but the auto-generated
/// `help` entries on each subcommand group are dropped as noise.
fn subcommands_listing() -> String {
    // `build()` injects clap's auto-generated `help` subcommand into the tree.
    let mut cmd = Cli::command();
    cmd.build();
    let mut rows = Vec::new();
    collect_subcommands(&cmd, "", &mut rows);
    let width = rows.iter().map(|(path, _)| path.len()).max().unwrap_or(0);
    let mut out = String::from("Usage: data-dict <COMMAND>\n\nCommands:\n");
    for (path, about) in rows {
        out.push_str(&format!("  {path:<width$}  {about}\n"));
    }
    out
}

fn collect_subcommands(cmd: &clap::Command, prefix: &str, rows: &mut Vec<(String, String)>) {
    for sub in cmd.get_subcommands() {
        let is_help = sub.get_name() == "help";
        // Keep only the top-level `help`; nested `help` entries are noise.
        if is_help && !prefix.is_empty() {
            continue;
        }
        let path = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        // `help` carries a mirror of the whole command tree; treat it as a leaf.
        if !is_help && sub.get_subcommands().any(|s| s.get_name() != "help") {
            collect_subcommands(sub, &path, rows);
        } else {
            let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            rows.push((path, about));
        }
    }
}

fn resolve_dict_path(path: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    if path.is_dir() {
        let candidate = path.join("data-dict.yaml");
        if candidate.is_file() {
            Ok(candidate)
        } else {
            Err(format!("no data-dict.yaml found in {}", path.display()))
        }
    } else {
        Ok(path)
    }
}

fn validate_result_to_json(
    diagnostics: &Diagnostics,
    result: &Result<DataReport, DataError>,
) -> serde_json::Value {
    let mut value = match result {
        Ok(report) => {
            let failed = diagnostics.has_errors() || report.has_errors();
            let status = if failed { "error" } else { "ok" };
            if diagnostics.has_errors() && report.is_clean() {
                // The data comparison was skipped because the dictionary itself
                // is invalid; the diagnostics array carries the details.
                serde_json::json!({"status": status, "kind": "dictionary"})
            } else if report.is_clean() {
                serde_json::json!({"status": status})
            } else {
                serde_json::json!({
                    "status": status,
                    "kind": "mismatch",
                    "table": report.table,
                    "issues": report.issues.iter().map(issue_to_json).collect::<Vec<_>>(),
                })
            }
        }
        Err(DataError::Schema(e)) => serde_json::json!({
            "status": "error",
            "kind": "schema",
            "message": e.to_string(),
        }),
        Err(DataError::Parquet(e)) => serde_json::json!({
            "status": "error",
            "kind": "parquet",
            "message": e.to_string(),
        }),
        Err(DataError::TableNotFound { name, available }) => serde_json::json!({
            "status": "error",
            "kind": "table_not_found",
            "name": name,
            "available": available,
        }),
        Err(DataError::AmbiguousTable { available }) => serde_json::json!({
            "status": "error",
            "kind": "ambiguous_table",
            "available": available,
        }),
    };
    value["diagnostics"] = serde_json::json!(
        diagnostics
            .items
            .iter()
            .map(diagnostic_to_json)
            .collect::<Vec<_>>()
    );
    value
}

fn diagnostic_to_json(diagnostic: &Diagnostic) -> serde_json::Value {
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let mut value = serde_json::json!({
        "severity": severity,
        "code": diagnostic.code,
        "message": diagnostic.message,
    });
    if let Some(hint) = &diagnostic.hint {
        value["hint"] = serde_json::json!(hint);
    }
    value
}

fn issue_to_json(issue: &ColumnIssue) -> serde_json::Value {
    let severity = match issue.severity() {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let mut value = match issue {
        ColumnIssue::TypeMismatch {
            column,
            declared,
            actual,
        } => serde_json::json!({
            "kind": "type_mismatch",
            "column": column,
            "declared": declared,
            "actual": actual,
        }),
        ColumnIssue::MissingInData { column } => serde_json::json!({
            "kind": "missing_in_data",
            "column": column,
        }),
        ColumnIssue::ExtraInData { column, actual } => serde_json::json!({
            "kind": "extra_in_data",
            "column": column,
            "actual": actual,
        }),
        ColumnIssue::NullsInRequired {
            column,
            count,
            rows,
        } => serde_json::json!({
            "kind": "nulls_in_required",
            "column": column,
            "count": count,
            "rows": rows,
        }),
    };
    value["severity"] = serde_json::json!(severity);
    value
}

fn print_types_table(cols: &[data_dict_parquet::ColumnTypeInfo]) {
    let headers = ["#", "column", "dict type", "logical type", "physical type"];
    let num_width = cols.len().to_string().len().max(headers[0].len());
    let widths = [
        num_width,
        cols.iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(0)
            .max(headers[1].len()),
        cols.iter()
            .map(|c| c.dict_type.len())
            .max()
            .unwrap_or(0)
            .max(headers[2].len()),
        cols.iter()
            .map(|c| c.logical_type.as_deref().unwrap_or("").len())
            .max()
            .unwrap_or(0)
            .max(headers[3].len()),
        cols.iter()
            .map(|c| c.physical_type.len())
            .max()
            .unwrap_or(0)
            .max(headers[4].len()),
    ];

    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
    );
    println!("{}", "─".repeat(widths.iter().sum::<usize>() + 8));

    for (i, col) in cols.iter().enumerate() {
        println!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
            i + 1,
            col.name,
            col.dict_type,
            col.logical_type.as_deref().unwrap_or(""),
            col.physical_type,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "data-dict-cli-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn explicit_file_is_returned_as_is() {
        let dir = temp_dir("file");
        let file = dir.join("custom.yaml");
        fs::write(&file, "tables: {}\n").unwrap();
        assert_eq!(resolve_dict_path(Some(file.clone())).unwrap(), file);
    }

    #[test]
    fn directory_resolves_to_data_dict_yaml() {
        let dir = temp_dir("dir");
        let dict = dir.join("data-dict.yaml");
        fs::write(&dict, "tables: {}\n").unwrap();
        assert_eq!(resolve_dict_path(Some(dir)).unwrap(), dict);
    }

    #[test]
    fn directory_without_data_dict_yaml_errors() {
        let dir = temp_dir("empty");
        let err = resolve_dict_path(Some(dir.clone())).unwrap_err();
        assert!(err.contains("no data-dict.yaml found"));
        assert!(err.contains(&dir.display().to_string()));
    }

    #[test]
    fn none_defaults_to_current_directory() {
        assert_eq!(resolve_dict_path(None), resolve_dict_path(Some(".".into())));
    }

    #[test]
    fn nonexistent_file_is_returned_as_is() {
        // A path that is neither a dir nor an existing file is passed through
        // so the caller surfaces the real read error.
        let path = PathBuf::from("does-not-exist.yaml");
        assert_eq!(resolve_dict_path(Some(path.clone())).unwrap(), path);
    }

    /// Validate a dictionary that is clean apart from a DD009 ($learn_more)
    /// warning, returning its diagnostics.
    fn warning_diagnostics(name: &str) -> Diagnostics {
        let dir = temp_dir(name);
        let dict = dir.join("data-dict.yaml");
        fs::write(&dict, "$version: 0.1.0\n").unwrap();
        data_dict::validate(&dict).expect("must validate")
    }

    #[test]
    fn json_includes_diagnostics_on_success() {
        let diagnostics = warning_diagnostics("json-ok");
        let report = DataReport {
            table: String::new(),
            issues: Vec::new(),
        };
        let json = validate_result_to_json(&diagnostics, &Ok(report));
        assert_eq!(json["status"], "ok");
        assert_eq!(json["diagnostics"][0]["code"], "DD009");
        assert_eq!(json["diagnostics"][0]["severity"], "warning");
        assert!(
            json["diagnostics"][0]["hint"]
                .as_str()
                .is_some_and(|h| h.contains("$learn_more")),
            "DD009 hint should be carried in the JSON output"
        );
    }

    #[test]
    fn json_includes_diagnostics_on_error() {
        let diagnostics = warning_diagnostics("json-err");
        let err = DataError::AmbiguousTable {
            available: vec!["a".to_string(), "b".to_string()],
        };
        let json = validate_result_to_json(&diagnostics, &Err(err));
        assert_eq!(json["status"], "error");
        assert_eq!(json["diagnostics"][0]["code"], "DD009");
    }
}
