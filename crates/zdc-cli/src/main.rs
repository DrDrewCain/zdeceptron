#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser as ClapParser, Subcommand};
use zdc_diagnostics::{render, Diagnostic};

#[derive(ClapParser)]
#[command(name = "zdc", version, about = "The ZDeceptron compiler")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a source file and print its syntax tree.
    Parse {
        /// Path to a `.zd` file.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Parse { file } => {
            let path = file.display().to_string();
            let src = match std::fs::read_to_string(&file) {
                Ok(src) => src,
                Err(e) => {
                    eprintln!("Could not read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            };

            match zdc_parser::parse(&src) {
                Ok(program) => {
                    println!("{program:#?}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprint!("{}", render(&src, &path, &Diagnostic::from(error)));
                    ExitCode::FAILURE
                }
            }
        }
    }
}
