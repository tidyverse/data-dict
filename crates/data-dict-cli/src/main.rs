use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use data_dict::data::{ColumnIssue, DataError};

#[derive(Parser)]
#[command(name = "data-dict", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
    match cli.command {
        Command::ValidateSchema { path } => {
            let path = match resolve_dict_path(path) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            };
            match data_dict::validate(&path) {
                Ok(()) => {
                    println!("{}: ok", path.display());
                    ExitCode::SUCCESS
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
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "status": "error",
                                "kind": "schema",
                                "message": err,
                            })
                        );
                    } else {
                        eprintln!("{err}");
                    }
                    return ExitCode::FAILURE;
                }
            };
            let result = data_dict::data::validate_parquet(&dict, &parquet, table.as_deref());
            if json {
                println!("{}", validate_result_to_json(&result));
            }
            match result {
                Ok(()) => {
                    if !json {
                        println!("{}: ok", parquet.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    if !json {
                        eprintln!("{err}");
                    }
                    ExitCode::FAILURE
                }
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
}

fn validate_result_to_json(result: &Result<(), DataError>) -> serde_json::Value {
    match result {
        Ok(()) => serde_json::json!({"status": "ok"}),
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
        Err(DataError::Mismatch { table, issues }) => serde_json::json!({
            "status": "error",
            "kind": "mismatch",
            "table": table,
            "issues": issues.iter().map(issue_to_json).collect::<Vec<_>>(),
        }),
    }
}

fn issue_to_json(issue: &ColumnIssue) -> serde_json::Value {
    match issue {
        ColumnIssue::TypeMismatch { column, declared, actual } => serde_json::json!({
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
    }
}

fn print_types_table(cols: &[data_dict_parquet::ColumnTypeInfo]) {
    let headers = ["#", "column", "dict type", "logical type", "physical type"];
    let num_width = cols.len().to_string().len().max(headers[0].len());
    let widths = [
        num_width,
        cols.iter().map(|c| c.name.len()).max().unwrap_or(0).max(headers[1].len()),
        cols.iter().map(|c| c.dict_type.len()).max().unwrap_or(0).max(headers[2].len()),
        cols.iter()
            .map(|c| c.logical_type.as_deref().unwrap_or("").len())
            .max()
            .unwrap_or(0)
            .max(headers[3].len()),
        cols.iter().map(|c| c.physical_type.len()).max().unwrap_or(0).max(headers[4].len()),
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
