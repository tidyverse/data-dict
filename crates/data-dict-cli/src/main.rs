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
    /// Validate a data-dict.yaml file
    Validate { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path } => match data_dict::validate(&path) {
            Ok(()) => {
                println!("{}: ok", path.display());
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
    }
}
