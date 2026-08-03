#![forbid(unsafe_code)]

use std::net::IpAddr;
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
    /// Compile a source file into a runnable bundle.
    Build {
        /// Path to a `.zd` file.
        file: PathBuf,
        /// Where to write the bundle.
        #[arg(long, short, default_value = "dist")]
        out: PathBuf,
        /// Emit constructs whose correctness depends on the type checker
        /// that does not exist yet (spec §16.7).
        #[arg(long)]
        unchecked: bool,
    },
    /// Serve a source file, rebuilding and reloading as it is edited.
    Dev {
        /// Path to a `.zd` file.
        file: PathBuf,
        /// Port to listen on.
        #[arg(long, short, default_value_t = zdc_dev::DEFAULT_PORT)]
        port: u16,
        /// Address to listen on. Defaults to loopback; set `0.0.0.0` to
        /// reach the server from another device on the network.
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        /// Emit constructs whose correctness depends on the type checker
        /// that does not exist yet (spec §16.7).
        #[arg(long)]
        unchecked: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Command::Parse { file } => parse(file),
        Command::Check { file } => check(file),
        Command::Build {
            file,
            out,
            unchecked,
        } => build(file, out, *unchecked),
        Command::Dev {
            file,
            port,
            host,
            unchecked,
        } => dev(file, *host, *port, *unchecked),
    }
}

/// Serve a file and rebuild it as it changes.
///
/// Returns only if the server could not start. A program that does not
/// compile is not such a case: the diagnostic is printed and shown on the
/// page, and the watcher keeps running, because the fix is a keystroke
/// away (spec §9).
fn dev(file: &Path, host: IpAddr, port: u16, unchecked: bool) -> ExitCode {
    let mut options = zdc_dev::Options::new(file);
    options.host = host;
    options.port = port;
    options.settings.unchecked = unchecked;

    match zdc_dev::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprint!("{}", error.report());
            ExitCode::FAILURE
        }
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

/// Compile a file into `out`, reporting **every** diagnostic.
///
/// The bundle is written only once the whole program has compiled. A
/// half-written `dist/` that a browser would happily load is worse than no
/// `dist/` at all: the failure would show up as a blank page rather than as
/// the diagnostic that explains it.
fn build(file: &Path, out: &Path, unchecked: bool) -> ExitCode {
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

    let hir = match zdc_resolve::Resolver::new(&program).resolve() {
        Ok(hir) => hir,
        Err(errors) => {
            for error in errors {
                eprint!("{}", render(&src, &path, &Diagnostic::from(error)));
            }
            return ExitCode::FAILURE;
        }
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let mut options = zdc_codegen::Options::new(&path, name);
    options.unchecked = unchecked;

    let bundle = match zdc_codegen::compile(&hir, &options) {
        Ok(bundle) => bundle,
        Err(errors) => {
            for error in errors {
                eprint!("{}", render(&src, &path, &Diagnostic::from(error)));
            }
            return ExitCode::FAILURE;
        }
    };

    let mut files: Vec<(PathBuf, &str)> = vec![
        (out.join("client.js"), bundle.client_js.as_str()),
        (out.join("styles.css"), bundle.styles_css.as_str()),
        (out.join("index.html"), bundle.index_html.as_str()),
        (out.join("manifest.json"), bundle.manifest_json.as_str()),
    ];
    // `elements.js` is deliberately not among these: generated code never
    // imports it (spec §16.3.1).
    for (relative, source) in zdc_codegen::runtime_files() {
        files.push((out.join(relative), source));
    }

    for (target, contents) in files {
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return write_failure(parent, e);
            }
        }
        if let Err(e) = std::fs::write(&target, contents) {
            return write_failure(&target, e);
        }
    }

    ExitCode::SUCCESS
}

fn write_failure(target: &Path, error: std::io::Error) -> ExitCode {
    let target = target.display().to_string();
    let diagnostic = Diagnostic::file_error(format!("Could not write {target}: {error}"));
    eprint!("{}", render("", &target, &diagnostic));
    ExitCode::FAILURE
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
