use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "data-dict", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a data-dict.yaml file against the schema
    ValidateSchema { path: PathBuf },
    /// Work with parquet files
    Parquet {
        #[command(subcommand)]
        command: ParquetCommand,
    },
}

#[derive(Subcommand)]
enum ParquetCommand {
    /// Print column types for a parquet file
    Types { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::ValidateSchema { path } => match data_dict::validate(&path) {
            Ok(()) => {
                println!("{}: ok", path.display());
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
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
