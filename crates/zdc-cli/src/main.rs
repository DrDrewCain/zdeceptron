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
    /// Compile a source file and generate everything one platform needs to
    /// run it — and report what that platform cannot do.
    ///
    /// Nothing is deployed. This writes files and prints a capability
    /// report; running the platform's own deploy command is a separate,
    /// deliberate act.
    Deploy {
        /// Path to a `.zd` file.
        file: PathBuf,
        /// Which platform. Azure Functions is deliberately absent; `zdc
        /// deploy --target azure` says why.
        #[arg(long, short)]
        target: String,
        /// Where to write the deployment.
        #[arg(long, short, default_value = "deploy")]
        out: PathBuf,
        /// The deployment's name. Defaults to the source file's stem.
        #[arg(long)]
        app: Option<String>,
        /// How requests reach an AWS Lambda function. Decides whether a
        /// stream is possible at all.
        #[arg(long, default_value = "function-url")]
        front: String,
        /// Which Vercel runtime to target.
        #[arg(long, default_value = "fluid")]
        runtime: String,
        /// Whether the account is on the vendor's paid tier. Changes only
        /// which numbers the report is allowed to promise.
        #[arg(long, default_value = "free")]
        plan: String,
        /// Close a live-sync stream that has had nothing to say for this
        /// long. On AWS Lambda this is the only defence against being
        /// billed for a browser tab that closed.
        #[arg(long, default_value_t = 60)]
        idle_seconds: u32,
        /// How often a store with no push channel is re-read.
        #[arg(long, default_value_t = 2)]
        poll_seconds: u32,
        /// Print the capability report and write nothing.
        #[arg(long)]
        report_only: bool,
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
        Command::Deploy {
            file,
            target,
            out,
            app,
            front,
            runtime,
            plan,
            idle_seconds,
            poll_seconds,
            report_only,
        } => deploy(
            file,
            &DeployArgs {
                target,
                out,
                app: app.as_deref(),
                front,
                runtime,
                plan,
                idle_seconds: *idle_seconds,
                poll_seconds: *poll_seconds,
                report_only: *report_only,
            },
        ),
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

/// Everything the front end produces, once every pass has agreed.
struct Compiled {
    /// Kept so a later pass's diagnostics can still be pointed at the file
    /// they came from, which need not be the entry file.
    linked: zdc_resolve::Linked,
    hir: zdc_hir::Hir,
    split: zdc_graph::TierSplit,
    verdict: zdc_graph::Verdict,
    table: zdc_types::TypeTable,
}

/// Parse, resolve, split, typecheck and check information flow.
///
/// The entry file is a module and so is everything it imports (§14D.2), so
/// this loads the whole reachable set before resolving any of it: a
/// `durable` signal may be declared in one file and read in another, and
/// the placement pass needs both ends (§14D.3).
///
/// The order is spec §17.1.2's: **the split runs before the type
/// checker**, because the type of a cross-placement read depends on the
/// crossing, so types depend on placement and never the reverse.
///
/// Typechecking and the flow pass both run when the split succeeded, and
/// **both** report. A program that renders a secret and has a type error
/// should be told about the leak, not only about the type — the leak is
/// the more interesting of the two.
fn front_end(file: &Path) -> Result<Compiled, ()> {
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

    // Every entry point compiles against the prelude (§17.4.1), and the
    // linked program on top of it.
    let prelude = zdc_lib::load();
    let hir = match zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked).resolve()
    {
        Ok(hir) => hir,
        Err(errors) => {
            report(&linked, errors);
            return Err(());
        }
    };

    // The type checker refuses to run if the split found an error: a
    // program whose placements do not resolve has no settled read table,
    // so every cross-placement type after the first would be invented
    // (§17.1.3).
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        let errors: Vec<zdc_graph::GraphError> = split
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        report(&linked, errors);
        return Err(());
    }

    let verdict = zdc_graph::ifc(&hir, &split);
    let checked = zdc_types::check(&hir, &split);

    let mut failed = false;
    if let Err(errors) = &checked {
        report(&linked, errors.clone());
        failed = true;
    }
    let leaks: Vec<zdc_graph::GraphError> = verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    if !leaks.is_empty() {
        report(&linked, leaks);
        failed = true;
    }
    if failed {
        return Err(());
    }

    Ok(Compiled {
        linked,
        hir,
        split,
        verdict,
        table: checked.expect("checked is Ok when nothing failed"),
    })
}

/// §17.4.8's build root: printed, then run on the build host, so that what
/// it computed can be inlined into the bundle.
///
/// Shared by `build` and `deploy` because a deployment whose `static`
/// values came from somewhere else — or from nowhere — is not the program
/// `zdc build` produced. A program with no `static` state never reaches the
/// evaluator at all.
fn evaluate_build_root(
    file: &Path,
    compiled: &Compiled,
    inputs: &zdc_codegen::Inputs<'_>,
    options: &zdc_codegen::Options,
) -> Result<zdc_codegen::Evaluated, ()> {
    match zdc_codegen::build_module(inputs, options) {
        Ok(None) => Ok(zdc_codegen::Evaluated::default()),
        Ok(Some(module)) => {
            let directory = file.parent().unwrap_or(Path::new("."));
            match zdc_codegen::evaluate(&module, directory) {
                Ok(evaluated) => Ok(evaluated),
                Err(error) => {
                    let diagnostic = Diagnostic::file_error(error.report());
                    eprint!("{}", render("", "", &diagnostic));
                    Err(())
                }
            }
        }
        Err(errors) => {
            report(&compiled.linked, errors);
            Err(())
        }
    }
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
    let Ok(compiled) = front_end(file) else {
        return ExitCode::FAILURE;
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let options = zdc_codegen::Options::new(&path, name);

    let inputs = zdc_codegen::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        verdict: &compiled.verdict,
        table: &compiled.table,
    };

    // §17.4.8: the build root runs first, on the build host, and what it
    // computes is inlined into the bundle the next call prints. A program
    // with no `static` state never reaches the evaluator at all.
    let Ok(evaluated) = evaluate_build_root(file, &compiled, &inputs, &options) else {
        return ExitCode::FAILURE;
    };
    let options = options.with_statics(evaluated.values);

    let bundle = match zdc_codegen::compile(&inputs, &options) {
        Ok(bundle) => bundle,
        Err(errors) => {
            report(&compiled.linked, errors);
            return ExitCode::FAILURE;
        }
    };

    let mut files: Vec<(PathBuf, &str)> = vec![
        (out.join("client.js"), bundle.client_js.as_str()),
        (out.join("styles.css"), bundle.styles_css.as_str()),
        (out.join("index.html"), bundle.index_html.as_str()),
        (out.join("manifest.json"), bundle.manifest_json.as_str()),
    ];
    // One file per emitted server root. The split decided which exist,
    // what they are called, and what they take.
    for function in &bundle.functions {
        files.push((out.join(&function.path), function.source.as_str()));
    }
    // §14C.3b's generated files. They are part of the bundle, so they are
    // written like any other part of it: `rss.xml` and `llms.txt` are files
    // beside `index.html`, not endpoints beside `functions/`.
    for (path, contents) in &evaluated.files {
        files.push((out.join(path), contents.as_str()));
    }
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

/// Everything `zdc deploy` was asked for except the source file.
struct DeployArgs<'a> {
    target: &'a str,
    out: &'a Path,
    app: Option<&'a str>,
    front: &'a str,
    runtime: &'a str,
    plan: &'a str,
    idle_seconds: u32,
    poll_seconds: u32,
    report_only: bool,
}

/// Compile a file and generate one platform's deployment.
///
/// The capability report is printed whether or not files are written, and
/// before they are: a user who finds out at 900 seconds that their stream
/// dies, or after the bill arrives that Lambda kept charging for a closed
/// tab, has been failed by this command.
///
/// Nothing is deployed here, and nothing here can deploy. The platform's
/// own command is a separate act, run by someone who has read the report.
fn deploy(file: &Path, args: &DeployArgs<'_>) -> ExitCode {
    let path = file.display().to_string();
    let Ok(compiled) = front_end(file) else {
        return ExitCode::FAILURE;
    };

    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("app");
    let options = zdc_codegen::Options::new(&path, name);
    let inputs = zdc_codegen::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        verdict: &compiled.verdict,
        table: &compiled.table,
    };

    // The same two steps `zdc build` runs, in the same order (§17.4.8). A
    // deployment built without the build root would refuse every `static`
    // read, so the deployed program and the built one have to agree about
    // what the build host computed.
    let evaluated = match evaluate_build_root(file, &compiled, &inputs, &options) {
        Ok(evaluated) => evaluated,
        Err(()) => return ExitCode::FAILURE,
    };
    let options = options.with_statics(evaluated.values);

    let bundle = match zdc_codegen::compile(&inputs, &options) {
        Ok(bundle) => bundle,
        Err(errors) => {
            report(&compiled.linked, errors);
            return ExitCode::FAILURE;
        }
    };

    let settings = match deploy_options(args, name) {
        Ok(settings) => settings,
        Err(message) => return setting_failure(&message),
    };
    let program = zdc_deploy::Program {
        functions: &bundle.functions,
        durable: &bundle.durable,
        environment: &bundle.environment,
    };
    let deployment = match zdc_deploy::generate(&program, &settings) {
        Ok(deployment) => deployment,
        Err(refusal) => return setting_failure(&refusal.message),
    };

    print!("{}", deployment.capabilities.report());
    if args.report_only {
        return ExitCode::SUCCESS;
    }

    // The browser half goes under `public/`, which is where every target's
    // static handling looks: Cloudflare's `[assets]`, Vercel's
    // `outputDirectory`, and the Deno entry's own file read.
    let mut files: Vec<(PathBuf, &str)> = vec![
        (args.out.join("public/client.js"), bundle.client_js.as_str()),
        (
            args.out.join("public/styles.css"),
            bundle.styles_css.as_str(),
        ),
        (
            args.out.join("public/index.html"),
            bundle.index_html.as_str(),
        ),
        (
            args.out.join("public/manifest.json"),
            bundle.manifest_json.as_str(),
        ),
    ];
    for (relative, source) in zdc_codegen::runtime_files() {
        files.push((args.out.join("public").join(relative), source));
    }
    // §14C.3b's generated files. They are part of the site, so they go
    // beside the page rather than being dropped on the way to a platform.
    for (relative, contents) in &evaluated.files {
        files.push((args.out.join("public").join(relative), contents.as_str()));
    }
    for generated in &deployment.files {
        files.push((args.out.join(&generated.path), generated.contents.as_str()));
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

    eprintln!(
        "\nzdc deploy · {} · {} · {}\nNothing has been deployed. Run the platform's own deploy \
         command when the report above is acceptable.",
        settings.target.title(),
        args.out.display(),
        deployment.capabilities.shim.report(),
    );
    ExitCode::SUCCESS
}

fn deploy_options(args: &DeployArgs<'_>, name: &str) -> Result<zdc_deploy::Options, String> {
    let target = zdc_deploy::Target::parse(args.target)?;
    let mut options = zdc_deploy::Options::new(target, args.app.unwrap_or(name));
    options.front = zdc_deploy::LambdaFront::parse(args.front)?;
    options.runtime = zdc_deploy::VercelRuntime::parse(args.runtime)?;
    options.plan = zdc_deploy::Plan::parse(args.plan)?;
    options.idle_seconds = args.idle_seconds;
    options.poll_seconds = args.poll_seconds;
    Ok(options)
}

/// A refusal, or an unusable flag. Rendered through the same diagnostic
/// path as everything else so a deploy error reads like a compile error,
/// which is what it is.
fn setting_failure(message: &str) -> ExitCode {
    let diagnostic = Diagnostic::file_error(message.to_string());
    eprint!("{}", render("", "zdc deploy", &diagnostic));
    ExitCode::FAILURE
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
