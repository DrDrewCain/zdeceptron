#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
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
    /// Parse a source file and resolve every name in it.
    Check {
        /// Path to a `.zd` file.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Command::Parse { file } => parse(file),
        Command::Check { file } => check(file),
    }
}

fn parse(file: &Path) -> ExitCode {
    let path = file.display().to_string();
    let Some(src) = read(file, &path) else {
        return ExitCode::FAILURE;
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

/// Resolve every name in a file, reporting **every** one that could not
/// be found.
///
/// A programmer with three undefined names should see three diagnostics
/// from one run, not one per run, so the whole list is rendered rather
/// than only its first element.
fn check(file: &Path) -> ExitCode {
    let path = file.display().to_string();
    let Some(src) = read(file, &path) else {
        return ExitCode::FAILURE;
    };

    let program = match zdc_parser::parse(&src) {
        Ok(program) => program,
        Err(error) => {
            eprint!("{}", render(&src, &path, &Diagnostic::from(error)));
            return ExitCode::FAILURE;
        }
    };

    match zdc_resolve::Resolver::new(&program).resolve() {
        Ok(_) => ExitCode::SUCCESS,
        Err(errors) => {
            for error in errors {
                eprint!("{}", render(&src, &path, &Diagnostic::from(error)));
            }
            ExitCode::FAILURE
        }
    }
}

/// Read a source file, rendering a file-level diagnostic if it cannot be
/// read. Shared by every subcommand so the message never drifts.
fn read(file: &Path, path: &str) -> Option<String> {
    match std::fs::read_to_string(file) {
        Ok(src) => Some(src),
        Err(e) => {
            let diagnostic = Diagnostic::file_error(format!("Could not read {path}: {e}"));
            eprint!("{}", render("", path, &diagnostic));
            None
        }
    }
}
