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
    /// Parse a source file, resolve every name in it, and typecheck it.
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
    },
    /// Serve the Language Server Protocol over stdin and stdout.
    ///
    /// Started by an editor rather than by hand, which is why it takes no
    /// file: the editor sends the documents it has open.
    Lsp,
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
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Command::Parse { file } => parse(file),
        Command::Check { file } => check(file),
        Command::Build { file, out } => build(file, out),
        Command::Lsp => lsp(),
        Command::Dev { file, port, host } => dev(file, *host, *port),
    }
}

/// Serve a file and rebuild it as it changes.
///
/// Returns only if the server could not start. A program that does not
/// compile is not such a case: the diagnostic is printed and shown on the
/// page, and the watcher keeps running, because the fix is a keystroke
/// away (spec §9).
fn dev(file: &Path, host: IpAddr, port: u16) -> ExitCode {
    let mut options = zdc_dev::Options::new(file);
    options.host = host;
    options.port = port;

    match zdc_dev::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprint!("{}", error.report());
            ExitCode::FAILURE
        }
    }
}

/// Serve an editor over stdin and stdout.
///
/// Nothing is printed on the success path: stdout is the protocol's
/// transport, and a stray line on it desynchronises the client's framing.
/// A failure to start goes to stderr, which is where the editor's log is.
fn lsp() -> ExitCode {
    match zdc_lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("The language server stopped: {error}");
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

/// Resolve every name in a file and check every type in it, reporting
/// **every** problem either pass finds.
///
/// A programmer with three undefined names should see three diagnostics
/// from one run, not one per run, so the whole list is rendered rather
/// than only its first element. The same holds for type errors.
///
/// Typechecking is folded into `check` rather than given a subcommand of
/// its own. Two commands where one is a strict prefix of the other is the
/// CLI's version of the phrasing ambiguity §4.1 forbids in the language:
/// asked to check a file, the compiler should say everything it knows.
/// Resolution runs first because a name that points nowhere has no type
/// to check, so its errors would only be repeated.
fn check(file: &Path) -> ExitCode {
    match front_end(file) {
        Ok(_) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

/// Parse, resolve and typecheck, rendering every diagnostic from the
/// first pass that produced any.
///
/// The entry file is a module and so is everything it imports (§14D.2), so
/// this loads the whole reachable set before resolving any of it: a
/// `durable` signal may be declared in one file and read in another, and
/// the placement pass needs both ends (§14D.3).
///
/// The type table comes back with the HIR because code generation needs
/// it: §16.7's list is a contract, not a suggestion.
fn front_end(file: &Path) -> Result<(zdc_resolve::Linked, zdc_hir::Hir, zdc_types::TypeTable), ()> {
    let linked = match zdc_resolve::load(file) {
        Ok(linked) => linked,
        Err(errors) => {
            let path = file.display().to_string();
            for error in errors {
                match std::fs::read_to_string(file) {
                    Ok(src) => eprint!("{}", render(&src, &path, &Diagnostic::from(error))),
                    // The entry file itself could not be read, so there is
                    // no text to point into.
                    Err(_) => eprint!(
                        "{}",
                        render("", &path, &Diagnostic::file_error(error.message))
                    ),
                }
            }
            return Err(());
        }
    };

    let hir = match zdc_resolve::Resolver::linked(&linked).resolve() {
        Ok(hir) => hir,
        Err(errors) => {
            report(&linked, errors);
            return Err(());
        }
    };

    let types = match zdc_types::check(&hir) {
        Ok(types) => types,
        Err(errors) => {
            report(&linked, errors);
            return Err(());
        }
    };

    Ok((linked, hir, types))
}

/// Render every diagnostic against the file its span belongs to.
///
/// A span is a byte range with no file in it, so the linker's combined
/// buffer is what turns one back into a place a reader can look at. Without
/// this, an error in an imported file would be reported at whatever text
/// happened to sit at that offset in the entry file.
fn report<E>(linked: &zdc_resolve::Linked, errors: Vec<E>)
where
    Diagnostic: From<E>,
{
    for error in errors {
        let mut diagnostic = Diagnostic::from(error);
        let Some(span) = diagnostic.span else {
            eprint!("{}", render("", "", &diagnostic));
            continue;
        };
        let (path, source, local) = linked.locate(span);
        diagnostic.span = Some(local);
        eprint!(
            "{}",
            render(source, &path.display().to_string(), &diagnostic)
        );
    }
}

/// Compile a file into `out`, reporting **every** diagnostic.
///
/// The bundle is written only once the whole program has compiled. A
/// half-written `dist/` that a browser would happily load is worse than no
/// `dist/` at all: the failure would show up as a blank page rather than as
/// the diagnostic that explains it.
fn build(file: &Path, out: &Path) -> ExitCode {
    let path = file.display().to_string();

    // A bundle is only emitted from a program that resolves *and*
    // typechecks: §16.7 lists what codegen is silently wrong without, and
    // building past a type error is exactly the case it names.
    let Ok((linked, hir, types)) = front_end(file) else {
        return ExitCode::FAILURE;
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    // Everything under `assets/` beside the entry file ships unchanged,
    // and the `.css` among it is linked after the generated stylesheet.
    let assets = zdc_codegen::assets::discover(file);
    let options =
        zdc_codegen::Options::new(&path, name).with_stylesheets(assets.stylesheets.clone());

    // One document per URL (spec §14G.2). An unrouted program has one,
    // at `/`, which is what it has always had — so this is one code path
    // rather than a routed one and an unrouted one that could disagree.
    let site = match zdc_codegen::compile_site(&hir, &types, &options) {
        Ok(site) => site,
        Err(errors) => {
            report(&linked, errors);
            return ExitCode::FAILURE;
        }
    };

    let routed = site.pages.len() > 1;
    let mut files: Vec<(PathBuf, &str)> = Vec::new();
    for page in &site.pages {
        if routed {
            // A module with no `view` is never routed, so a routed page
            // always has a document.
            if let Some(document_html) = &page.document_html {
                files.push((
                    out.join(zdc_codegen::document_path(&page.url)),
                    document_html.as_str(),
                ));
            }
            files.push((
                out.join(format!("pages/{}.js", page.slug)),
                page.client_js.as_str(),
            ));
            files.push((
                out.join(format!("pages/{}.css", page.slug)),
                page.styles_css.as_str(),
            ));
        } else {
            // A module with no `view` has no page, and the page is the one
            // artifact that would be wrong rather than merely unused: it
            // imports a `main` the module does not export (§16.3.1).
            if let Some(document_html) = &page.document_html {
                files.push((out.join("index.html"), document_html.as_str()));
            }
            files.push((out.join("client.js"), page.client_js.as_str()));
            files.push((out.join("styles.css"), page.styles_css.as_str()));
        }
    }
    let manifest = if routed {
        site.routes_json.clone()
    } else {
        match zdc_codegen::compile(&hir, &types, &options) {
            Ok(bundle) => bundle.manifest_json,
            Err(errors) => {
                report(&linked, errors);
                return ExitCode::FAILURE;
            }
        }
    };
    files.push((
        out.join(if routed {
            "routes.json"
        } else {
            "manifest.json"
        }),
        manifest.as_str(),
    ));
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

    // Assets are copied byte for byte rather than read into a string: an
    // asset directory holds fonts and images as well as stylesheets.
    for asset in &assets.files {
        let target = out.join(&asset.relative);
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return write_failure(parent, e);
            }
        }
        if let Err(e) = std::fs::copy(&asset.source, &target) {
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
