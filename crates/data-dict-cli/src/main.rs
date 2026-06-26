use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use data_dict::ProblemSet;

#[derive(Parser)]
#[command(name = "data-dict", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a data-dict.yaml file or directory against the spec [default: .]
    ValidateSpec { path: Option<PathBuf> },
    /// Validate a dataset's column names and types against a data dictionary
    ValidateMeta(ValidateArgs),
    /// Validate a dataset's values against a data dictionary
    ValidateData(ValidateArgs),
    /// Print the data-dict.yaml specification
    Spec,
    /// Inspect data types of a data source
    Types {
        #[command(subcommand)]
        command: TypesCommand,
    },
    /// Agents: read these skills to learn how to work with data-dict files
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

/// Shared arguments for `validate-meta` and `validate-data`.
#[derive(clap::Args)]
struct ValidateArgs {
    dict: PathBuf,
    parquet: PathBuf,
    #[arg(long)]
    table: Option<String>,
    /// Emit results as JSON
    #[arg(long)]
    json: bool,
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
enum TypesCommand {
    /// Print column types for a parquet file
    Parquet { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        print_all_subcommands();
        return ExitCode::SUCCESS;
    };
    match command {
        Command::ValidateSpec { path } => {
            let path = match resolve_dict_path(path) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            };
            let problems = data_dict::validate_spec(&path);
            for line in problems.render() {
                eprintln!("{line}");
            }
            if problems.has_errors() {
                ExitCode::FAILURE
            } else {
                println!("{}: ok", path.display());
                ExitCode::SUCCESS
            }
        }
        Command::ValidateMeta(args) => run_validate(args, data_dict::validate_meta),
        Command::ValidateData(args) => run_validate(args, data_dict::validate_data),
        Command::Spec => {
            print!("{}", data_dict::SPEC_MD);
            ExitCode::SUCCESS
        }
        Command::Types {
            command: TypesCommand::Parquet { path },
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

/// A validation entry point: `validate_meta` or `validate_data`. Both share the
/// signature, so `run_validate` is generic over which one it drives.
type ValidateFn = fn(&Path, &Path, Option<&str>) -> ProblemSet;

/// Run a meta or data validation (`validate-meta` / `validate-data`) and turn
/// its outcome into rendered output and an exit code. Both commands share the
/// same plumbing; they differ only in the `validate` entry point passed in.
fn run_validate(args: ValidateArgs, validate: ValidateFn) -> ExitCode {
    let dict = match resolve_dict_path(Some(args.dict)) {
        Ok(dict) => dict,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let problems = validate(&dict, &args.parquet, args.table.as_deref());
    if args.json {
        println!("{}", problems_to_json(&problems));
    } else {
        for line in problems.render() {
            eprintln!("{line}");
        }
        if problems.is_ok() {
            println!("{}: ok", args.parquet.display());
        }
    }
    if problems.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn problems_to_json(problems: &ProblemSet) -> serde_json::Value {
    let status = if problems.has_errors() { "error" } else { "ok" };
    serde_json::json!({
        "status": status,
        "problems": problems.items,
    })
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

    /// Validate a dictionary that is clean apart from a S09 ($learn_more)
    /// warning, returning its problems.
    fn warning_problems(name: &str) -> ProblemSet {
        let dir = temp_dir(name);
        let dict = dir.join("data-dict.yaml");
        fs::write(&dict, "$version: 0.1.0\n").unwrap();
        data_dict::validate_spec(&dict)
    }

    #[test]
    fn json_carries_problems_on_success() {
        // A warning-only set still passes (status ok) but reports the warning.
        let json = problems_to_json(&warning_problems("json-ok"));
        assert_eq!(json["status"], "ok");
        assert_eq!(json["problems"][0]["code"], "S09");
        assert_eq!(json["problems"][0]["severity"], "warning");
        assert_eq!(json["problems"][0]["kind"], "spec");
        assert!(
            json["problems"][0]["hint"]
                .as_str()
                .is_some_and(|h| h.contains("$learn_more")),
            "S09 hint should be carried in the JSON output"
        );
    }

    #[test]
    fn json_reports_error_status() {
        let problems = ProblemSet::from_preflight(
            data_dict::ProblemKind::AmbiguousTable {
                available: vec!["a".to_string(), "b".to_string()],
            },
            "the data dictionary describes multiple tables",
        );
        let json = problems_to_json(&problems);
        assert_eq!(json["status"], "error");
        assert_eq!(json["problems"][0]["kind"], "ambiguous_table");
        assert_eq!(json["problems"][0]["available"][1], "b");
    }
}
