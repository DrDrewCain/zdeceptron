#![forbid(unsafe_code)]

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser as ClapParser, Subcommand};
use zdc_diagnostics::{render, Diagnostic};

mod new;

#[derive(ClapParser)]
#[command(name = "zdc", version, about = "The ZDeceptron compiler")]
struct Cli {
    /// Print diagnostics without colour.
    ///
    /// `NO_COLOR` in the environment does the same thing and needs no
    /// flag; this is for the case where the environment says nothing and
    /// the output is going somewhere that cannot render escapes anyway
    /// (#153). Global rather than per-subcommand: every command that can
    /// print a diagnostic should honour it, and a reader should not have
    /// to remember which ones do.
    #[arg(long, global = true)]
    no_color: bool,

    /// Report every warning as an error.
    ///
    /// The compiler's warnings are the findings a reader is entitled to
    /// disagree with; this is how a project says it does not intend to.
    /// There is deliberately no flag going the other way: an error cannot
    /// be demoted, because a compiler whose rejections are optional has
    /// an exit code that means nothing.
    #[arg(long, global = true)]
    deny_warnings: bool,

    /// Silence one warning, by code. Repeatable.
    ///
    /// Takes a code rather than a message, because the message is the
    /// part that gets reworded.
    #[arg(long, global = true, value_name = "CODE")]
    allow: Vec<String>,

    /// Report one warning as a warning, whatever `--deny-warnings` says.
    /// Repeatable.
    #[arg(long, global = true, value_name = "CODE")]
    warn: Vec<String>,

    /// How diagnostics are printed.
    ///
    /// `human` draws the source line and a caret. `json` writes one JSON
    /// object per diagnostic, one per line — line-delimited rather than a
    /// single array, so a run that is killed part way through has still
    /// emitted complete records and a consumer can act on each as it
    /// arrives. The shape is documented in `zdc-diagnostics`'s `json`
    /// module; every key is present on every line, `null` where the
    /// compiler has nothing.
    #[arg(long, global = true, value_enum, default_value_t = Format::Human)]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

/// What the reader asked for, as the diagnostics crate understands it.
///
/// The per-code settings are applied after `--deny-warnings` so that the
/// more specific statement wins, which is what a reader writing both of
/// them means.
fn policy_from(cli: &Cli) -> zdc_diagnostics::Policy {
    let mut policy = zdc_diagnostics::Policy::new();
    if cli.deny_warnings {
        policy = policy.deny_warnings();
    }
    for code in &cli.allow {
        policy = policy.set(code.to_ascii_uppercase(), zdc_diagnostics::Setting::Silence);
    }
    for code in &cli.warn {
        policy = policy.set(code.to_ascii_uppercase(), zdc_diagnostics::Setting::Warn);
    }
    policy
}

/// The command-line spelling of `zdc_diagnostics::Format`.
///
/// A separate enum because `clap`'s derive needs to own the value type it
/// parses, and because the compiler's own type should not gain a
/// dependency on an argument parser to be printable.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Format {
    Human,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Start a project: a program that runs, a stylesheet, and the command
    /// to run them with.
    ///
    /// First in this list because it is first in the order a reader meets
    /// the compiler, and `--help` is printed in declaration order.
    ///
    /// The generated program is small and deliberately not static — one
    /// signal, one derived from it, one event handler — so the first edit
    /// is a change rather than a deletion. A directory that already holds
    /// anything is refused and nothing is written.
    New {
        /// Directory to create. Its last part names the project.
        path: PathBuf,
    },
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
    /// Rewrite source files in the one canonical layout.
    ///
    /// §4.1's bargain is that the grammar admits exactly one phrasing per
    /// construct. Indentation being the block structure makes laying that
    /// out both harder and more worth doing, because a mis-indented file
    /// is not ugly, it is a different program.
    ///
    /// Several files rather than one, because the thing CI runs is
    /// `zdc fmt --check examples/*.zd` and a command that took a single
    /// path would be wrapped in a shell loop that loses the exit code of
    /// every file but the last.
    Fmt {
        /// Paths to `.zd` files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Report which files are not in canonical layout and write
        /// nothing. Exits non-zero if any would change.
        #[arg(long)]
        check: bool,
    },
    /// Print the rule behind a diagnostic code.
    ///
    /// The other half of the diagnostic format. A rejection prints the
    /// claim and the spans; this prints why the rule exists and a worked
    /// repair, for the reader who wants it and at no cost to the reader
    /// who does not.
    Explain {
        /// A diagnostic code, such as `E-IFC-05`. Case-insensitive.
        code: String,
    },
    /// Compile a source file into a runnable bundle.
    Build {
        /// Path to a `.zd` file.
        file: PathBuf,
        /// Where to write the bundle.
        #[arg(long, short, default_value = "dist")]
        out: PathBuf,
    },
    /// Write a program's own declarations out as Markdown.
    ///
    /// One page per source file, plus an overview whose first table is the
    /// program's placement split: every signal, where it lives, and what a
    /// read of it from the browser costs. The endpoints the compiler
    /// derived are listed with the files they are emitted to, because
    /// nobody wrote them and there is nowhere else to read them.
    ///
    /// The program is checked first. Documentation for a file that does
    /// not compile would state placements that have not been settled and
    /// types that were never inferred, so a refusal is printed and no
    /// pages are written.
    ///
    /// With `--prelude` and no file, the standard library is documented
    /// instead: it is eight files of ZDeceptron shipped inside the
    /// compiler, and this is the only way to read their surface without
    /// opening them.
    // A group rather than a pair of `conflicts_with`, so that `zdc doc`
    // with nothing at all names both things it would have taken. Told only
    // that `<FILE>` is required, a reader has no way to discover that the
    // library is documentable at all.
    #[command(group(clap::ArgGroup::new("subject").required(true).args(["file", "prelude"])))]
    Doc {
        /// Path to a `.zd` file.
        file: Option<PathBuf>,
        /// Document the standard library instead of a program.
        ///
        /// Not a path, because pointing this command at
        /// `prelude/list.zd` compiles that file *against* the prelude and
        /// every name in it collides with itself.
        #[arg(long)]
        prelude: bool,
        /// Where to write the pages.
        #[arg(long, short, default_value = "doc")]
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
    /// Check every claim a program makes about itself — issue #169.
    ///
    /// A `test "…" expect …` declaration is a claim and its evidence. This
    /// compiles the program, evaluates each expectation on the build host,
    /// and reports which claims held. Nothing is written and no server is
    /// started: a claim is a question about the program, not a run of it.
    Test {
        /// Path to a `.zd` file.
        file: PathBuf,
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

    // Before anything can print. `NO_COLOR` is consulted on every render;
    // this is the flag half.
    if cli.no_color {
        zdc_diagnostics::disable_colour();
    }
    // Likewise: which findings are reported, at what level, and in what
    // form are properties of the invocation, fixed before any pass runs.
    zdc_diagnostics::set_policy(policy_from(&cli));
    zdc_diagnostics::set_format(match cli.format {
        Format::Human => zdc_diagnostics::Format::Human,
        Format::Json => zdc_diagnostics::Format::Json,
    });

    match &cli.command {
        Command::New { path } => new(path),
        Command::Parse { file } => parse(file),
        Command::Check { file } => check(file),
        Command::Fmt { files, check } => fmt(files, *check),
        Command::Explain { code } => explain(code),
        Command::Build { file, out } => build(file, out),
        Command::Doc { file, prelude, out } => match file {
            Some(file) => doc(file, out),
            None if *prelude => doc_prelude(out),
            // Unreachable: the `subject` argument group is `required`, so
            // clap has already refused both neither and both. Written out
            // rather than unwrapped, because a panic in argument
            // dispatch is a worse answer than an exit code.
            None => ExitCode::FAILURE,
        },
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
        Command::Test { file } => test(file),
        Command::Lsp => lsp(),
        Command::Dev { file, port, host } => dev(file, *host, *port),
    }
}

/// Write a new project, and say what to run next.
///
/// The report goes to stdout, because it is the command's *output* rather
/// than a note about it: a reader is meant to copy the `zdc dev` line out
/// of it. A refusal goes through the same diagnostic renderer every other
/// failure uses, so "I will not overwrite your directory" reads like a
/// compile error, which is what it is — a claim and a repair.
fn new(path: &Path) -> ExitCode {
    match new::scaffold(path) {
        Ok(scaffold) => {
            print!("{}", scaffold.report());
            ExitCode::SUCCESS
        }
        Err(message) => command_failure("zdc new", &message),
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

/// Print the rule behind one diagnostic code.
///
/// An unknown code lists the ones that exist rather than saying only that
/// this one does not: a reader who mistyped a code is one line from the
/// right one, and a reader who guessed learns what the compiler can say.
fn explain(code: &str) -> ExitCode {
    let wanted = code.to_ascii_uppercase();
    match zdc_diagnostics::explain(&wanted) {
        Some(explanation) => {
            print!("{}", explanation.render());
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("There is no diagnostic code `{code}`.");
            eprintln!();
            eprintln!("The codes this compiler can produce are:");
            for known in zdc_diagnostics::explain::codes() {
                eprintln!("  {known}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Lay out every named file, or report the ones that are not laid out.
///
/// **Every file is visited even after one fails.** A run that stopped at
/// the first unformatted file would make `--check` a bisect: fix one,
/// re-run, learn about the next. The exit code is the disjunction, which
/// is what CI reads.
///
/// A file that is already canonical is not rewritten and says nothing.
/// Rewriting it would touch its mtime and rebuild everything downstream
/// of it for no change, and printing about it would bury the files that
/// did change in a list of the ones that did not.
fn fmt(files: &[PathBuf], check: bool) -> ExitCode {
    let mut failed = false;

    for file in files {
        let path = file.display().to_string();
        let Some(src) = read(file, &path) else {
            failed = true;
            continue;
        };

        let laid_out = match zdc_fmt::format(&src) {
            Ok(laid_out) => laid_out,
            Err(error) => {
                // A file the compiler will not read is a file this must
                // not rewrite, so the compiler's own diagnostic is what is
                // printed: the repair is the same repair `zdc check` would
                // have asked for.
                //
                // The entangled case gets a caret too, built here rather
                // than taken from `file_error`. `file_error` is for a
                // diagnostic about a *file* — unreadable, missing, not
                // UTF-8 — and this one is about a line. Printed without a
                // span it was the only refusal `zdc` can make that does not
                // say where, which in a file with three block literals in
                // it is the difference between a repair and a search.
                let diagnostic = match &error {
                    zdc_fmt::FmtError::Unreadable(reported) => Diagnostic::from(reported.clone()),
                    zdc_fmt::FmtError::Entangled(span) => Diagnostic {
                        // A refusal: `zdc fmt` leaves the file alone.
                        level: zdc_diagnostics::Level::Error,
                        message: error.message(),
                        span: Some(*span),
                        label: Some(
                            "this begins on a line that is still inside a block text literal"
                                .to_string(),
                        ),
                        notes: Vec::new(),
                        help: Some(
                            "Move it to a line of its own. `zdc fmt` leaves the file alone until \
                             then; nothing else about it is wrong."
                                .to_string(),
                        ),
                        suggestion: None,
                        code: None,
                    },
                };
                eprint!("{}", render(&src, &path, &diagnostic));
                failed = true;
                continue;
            }
        };

        if laid_out == src {
            continue;
        }
        if check {
            eprintln!("{path} is not in the canonical layout. Run `zdc fmt {path}`.");
            failed = true;
            continue;
        }
        if let Err(e) = std::fs::write(file, &laid_out) {
            eprint!(
                "{}",
                render(
                    "",
                    &path,
                    &Diagnostic::file_error(format!("Could not write {path}: {e}"))
                )
            );
            failed = true;
        }
    }

    if failed {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
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
///
/// **Code generation runs too, and its output is thrown away.** §17.1.2
/// puts codegen last because it reads all four of the earlier products, and
/// that ordering is not in question — but "runs last" was silently read as
/// "runs only in `zdc build`", which split the diagnostic set in two along
/// a line no rule justifies. A program whose only fault is a codegen
/// refusal exited 0 here and failed to build, and the editor, which runs
/// this same pipeline, showed a clean file. `zdc_codegen::check` is
/// `zdc_codegen::compile` with the bundle dropped, so the two sets cannot
/// differ.
fn check(file: &Path) -> ExitCode {
    let Ok(compiled) = front_end(file) else {
        return ExitCode::FAILURE;
    };
    // `front_end` returns `Err` when the flow pass reported anything, so a
    // program that reaches here is cleared. Read rather than assumed, and
    // by the same call the two build paths make.
    let Some(cleared) = compiled.verdict.clearance() else {
        return ExitCode::FAILURE;
    };
    let refusals = zdc_codegen::check(&zdc_codegen::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        verdict: &compiled.verdict,
        table: &compiled.table,
        cleared,
    });
    if refusals.is_empty() {
        return ExitCode::SUCCESS;
    }
    report(&compiled.linked, refusals);
    ExitCode::FAILURE
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
        Err(failure) => {
            // Against the file each span belongs to, not the entry's text
            // (#4). Every error used to be rendered against the entry, so
            // a parse error in an imported module fell outside it and
            // printed with no file name and no caret: the reader was told
            // what was wrong and not which of their files it was in.
            for error in &failure.errors {
                let message = error.message.clone();
                let mut diagnostic = Diagnostic::from(error.clone());
                let located = diagnostic.span.and_then(|span| {
                    failure.locate(span).map(|(path, source, local)| {
                        (path.display().to_string(), source.to_string(), local)
                    })
                });
                match located {
                    Some((path, source, local)) => {
                        diagnostic.span = Some(local);
                        eprint!("{}", render(&source, &path, &diagnostic));
                    }
                    // Nothing was read at all — the entry file itself could
                    // not be opened. There is no text to point into, and
                    // pointing at text the reader does not have would be
                    // worse than saying so.
                    None => eprint!(
                        "{}",
                        render(
                            "",
                            &file.display().to_string(),
                            &Diagnostic::file_error(message)
                        )
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
    //
    // Every diagnostic the split produced is reported, not only the
    // errors. The split is where `W0330` and `W0331` are raised, and
    // filtering here is how those two spent their existence unprintable:
    // the compiler computed them, the CLI dropped them, and the only
    // evidence a reader had that they existed was a `zdc explain` entry.
    // `report` decides what stops the build, because the level is now on
    // the diagnostic and a `--deny-warnings` run has to stop here.
    let split = zdc_graph::split(&hir);
    if report(&linked, split.diagnostics.clone()) {
        return Err(());
    }

    let verdict = zdc_graph::ifc(&hir, &split);
    let checked = zdc_types::check(&hir, &split);

    let mut failed = false;
    if let Err(errors) = &checked {
        failed |= report(&linked, errors.clone());
    }
    failed |= report(&linked, verdict.diagnostics.clone());
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
///
/// Returns whether anything was reported **as an error**, which is not the
/// same question as whether anything was reported: a warning prints and
/// the build continues, and the same warning under `--deny-warnings`
/// prints and stops it. Every caller decides its exit code from this
/// rather than from the length of the list it passed in.
fn report<E>(linked: &zdc_resolve::Linked, errors: Vec<E>) -> bool
where
    Diagnostic: From<E>,
{
    let policy = zdc_diagnostics::policy();
    let mut fatal = false;
    for error in errors {
        let mut diagnostic = Diagnostic::from(error);
        // A silenced diagnostic is not printed and does not count.
        if !policy.apply(&mut diagnostic) {
            continue;
        }
        fatal |= diagnostic.level.is_error();
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
    fatal
}

/// Compile a file into `out`, reporting **every** diagnostic.
///
/// The bundle is written only once the whole program has compiled. A
/// half-written `dist/` that a browser would happily load is worse than no
/// `dist/` at all: the failure would show up as a blank page rather than as
/// the diagnostic that explains it.
///
/// # When `--report` is added, `dist/report.json` must not carry `attacker_reachable`
///
/// §19.5 as amended by §21.7.7 specifies that field, and §21.8.3 and
/// §21.8.7 withdraw it. Two independent reasons, both fatal:
///
/// 1. **It cannot be computed for the grants that matter.** The flag is
///    set by walking a grant's arguments back to a crossing. A purity
///    grant — `is anywhere`, `gives trusted T` — has no argument to walk,
///    so the grants §21.7's soundness leans on are exactly the ones the
///    flag cannot describe (residual risk R6).
/// 2. **It reads as a verdict and would be a false one.** §21.7.10 tells a
///    user that if nothing is marked `attacker_reachable` then no visitor
///    can steer any declassification. For §21.8.1's `launder3.zd` that
///    list is empty and a visitor steers the declassification with a query
///    string.
///
/// A report that enumerates the grants is still worth emitting — the
/// enumeration is complete by grammar (§19.5), which no configured taint
/// tool can manage. What must not ship is the claim laid over it. Emit the
/// grants and their spans; do not emit a field that answers "is this
/// program safe", because nothing here answers that.
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
    // Everything under `assets/` beside the entry file ships unchanged,
    // and the `.css` among it is linked after the generated stylesheet.
    let assets = zdc_codegen::assets::discover(file);
    // A refused asset fails the build rather than shipping a bundle that
    // is quietly missing a file. The build is the last place that can tell
    // the difference between "no stylesheet" and "a stylesheet that was
    // refused" (#188).
    if !assets.refused.is_empty() {
        for name in &assets.refused {
            eprintln!(
                "error: `{name}` resolves outside the project directory, so it is not copied \
                 into the bundle. An asset is a file in the project; a link out of it is not."
            );
        }
        return ExitCode::FAILURE;
    }
    let options = zdc_codegen::Options::new(&path, name)
        .with_stylesheets(assets.stylesheets.clone())
        .with_icon(assets.icon.clone());

    // The flow pass's own permission to emit. `front_end` has already
    // reported and refused on a leak, so this always succeeds — but an
    // `Inputs` cannot be built without asking, which is what makes
    // §16.3.12's invariant 3 a property of the type system.
    let Some(cleared) = compiled.verdict.clearance() else {
        return ExitCode::FAILURE;
    };

    let inputs = zdc_codegen::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        verdict: &compiled.verdict,
        table: &compiled.table,
        cleared,
    };

    // §17.4.8: the build root runs first, on the build host, and what it
    // computes is inlined into the bundle the next call prints. A program
    // with no `static` state never reaches the evaluator at all.
    let Ok(evaluated) = evaluate_build_root(file, &compiled, &inputs, &options) else {
        return ExitCode::FAILURE;
    };
    let options = options.with_statics(evaluated.values);

    // One document per URL (spec §14G.2). An unrouted program has one,
    // at `/`, which is what it has always had — so this is one code path
    // rather than a routed one and an unrouted one that could disagree.
    let site = match zdc_codegen::compile_site(&inputs, &options) {
        Ok(site) => site,
        Err(errors) => {
            report(&compiled.linked, errors);
            return ExitCode::FAILURE;
        }
    };
    // `zdc build` writes what a reader downloads, so the comments and the
    // indentation come out (#135). `zdc dev` does not do this: the person
    // reading a development build is the person writing the program.
    let site = site.minified();

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
            if let Some(boot_js) = &page.boot_js {
                files.push((
                    out.join(format!("pages/{}.boot.js", page.slug)),
                    boot_js.as_str(),
                ));
            }
            files.push((
                out.join(format!("pages/{}.js", page.slug)),
                page.client_js.as_str(),
            ));
            // Not `pages/<slug>.css`: the stylesheet's name carries a
            // content hash and the document links that name, so the
            // bundle's own answer is the only one that can be right
            // (#137).
            files.push((out.join(&page.styles_path), page.styles_css.as_str()));
        } else {
            // A module with no `view` has no page, and the page is the one
            // artifact that would be wrong rather than merely unused: it
            // imports a `main` the module does not export (§16.3.1).
            if let Some(document_html) = &page.document_html {
                files.push((out.join("index.html"), document_html.as_str()));
            }
            if let Some(boot_js) = &page.boot_js {
                files.push((out.join("boot.js"), boot_js.as_str()));
            }
            files.push((out.join("client.js"), page.client_js.as_str()));
            files.push((out.join(&page.styles_path), page.styles_css.as_str()));
        }
    }
    files.push((out.join("manifest.json"), site.manifest_json.as_str()));
    if routed {
        files.push((out.join("routes.json"), site.routes_json.as_str()));
    }
    // The cache configuration, over both halves of what was hashed: the
    // stylesheets the emitter produced and the ones the asset directory
    // contributed (#137). `zdc build` writes a directory someone uploads
    // to a static host, and `_headers` is what the hosts that read
    // anything read. A program with nothing hashed gets no file.
    let mut immutable = site.immutable.clone();
    immutable.extend(assets.immutable.iter().cloned());
    let headers = zdc_codegen::cache::headers(&immutable);
    if let Some(headers) = &headers {
        files.push((out.join("_headers"), headers.as_str()));
    }
    // One file per emitted server root. The split decided which exist,
    // what they are called, and what they take.
    for function in &site.functions {
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
    let runtime = zdc_codegen::runtime_files(&site.runtime, zdc_codegen::Mode::Release);
    for (relative, source) in &runtime {
        files.push((out.join(relative), source.as_str()));
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

    // `/favicon.ico`, at the root, whatever the icon is actually called.
    //
    // A browser asks for that exact path on its own, before and regardless
    // of anything the document says, so a site that only names its icon in
    // the head still answers a request every visitor makes with a 404. The
    // head link is what lets the icon be an SVG or live under `assets/`;
    // this is what answers the request nobody wrote.
    if let Some(icon) = &assets.icon {
        let source = out.join(icon.trim_start_matches('/'));
        let target = out.join("favicon.ico");
        if source.exists() && !target.exists() {
            if let Err(e) = std::fs::copy(&source, &target) {
                return write_failure(&target, e);
            }
        }
    }

    // The `foreign` modules the emitted imports point at (#223). Unlike an
    // asset, the path here came out of the *source text* — a `from "./io.js"`
    // clause — so it is sandboxed exactly as a `build read` path is: a
    // climbing path and a symlink resolving outside the project are both
    // refused, on the canonical path so a link cannot launder one.
    if let Err(code) = ship_linked_modules(file, out, &site.linked_modules) {
        return code;
    }

    ExitCode::SUCCESS
}

/// Write a checked program's declarations out as Markdown.
///
/// Runs the same front end `zdc check` and `zdc build` run, and for the
/// same reason the editor does: a second, laxer path to the same facts is
/// a path that can report different ones. A program that does not compile
/// has no settled placements — the split is what decides them — so there
/// is nothing here to write down about it except the diagnostic, which is
/// what it gets.
fn doc(file: &Path, out: &Path) -> ExitCode {
    let Ok(compiled) = front_end(file) else {
        return ExitCode::FAILURE;
    };

    let pages = zdc_doc::render(&zdc_doc::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        table: &compiled.table,
        linked: &compiled.linked,
        subject: zdc_doc::Subject::Program(file),
    });
    write_pages(&pages, out)
}

/// Document the standard library.
///
/// The library cannot be documented by pointing `zdc doc` at one of its
/// files, because every entry point compiles *against* the prelude
/// (§17.4.1) and `prelude/list.zd` compiled that way collides with itself
/// at its first declaration. So it is compiled as the program instead —
/// [`zdc_doc::library::linked`] links its sources into one — and the same
/// resolver, split and type checker run over it.
///
/// The prelude is loaded from the compiler binary rather than from disk,
/// so unlike every other path here there is no file to fail to read and
/// nothing a caller could point at that does not exist.
fn doc_prelude(out: &Path) -> ExitCode {
    let linked = zdc_doc::library::linked();

    // Reported rather than unwrapped, for the reason the library is a
    // compilation unit at all: it is checked by the compiler that compiles
    // it, and a failure here is a real diagnostic about a real file that
    // happens to ship inside this binary.
    let hir = match zdc_doc::library::resolve(&linked) {
        Ok(hir) => hir,
        Err(errors) => {
            report(&linked, errors);
            return ExitCode::FAILURE;
        }
    };
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        let errors: Vec<zdc_graph::GraphError> = split
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        report(&linked, errors);
        return ExitCode::FAILURE;
    }
    let table = match zdc_types::check(&hir, &split) {
        Ok(table) => table,
        Err(errors) => {
            report(&linked, errors);
            return ExitCode::FAILURE;
        }
    };

    let pages = zdc_doc::render(&zdc_doc::Inputs {
        hir: &hir,
        split: &split,
        table: &table,
        linked: &linked,
        subject: zdc_doc::Subject::Prelude,
    });
    write_pages(&pages, out)
}

/// Write a rendered page set, or refuse at the first file that will not
/// take it.
///
/// The pages are written only once every one of them has been rendered.
/// `zdc_doc::render` cannot fail, so this is not defending against a
/// half-rendered set; it is keeping the same shape `build` has, where the
/// directory a reader is about to open is never half a program.
fn write_pages(pages: &[zdc_doc::DocFile], out: &Path) -> ExitCode {
    for page in pages {
        let target = out.join(&page.path);
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return write_failure(parent, e);
            }
        }
        if let Err(e) = std::fs::write(&target, &page.text) {
            return write_failure(&target, e);
        }
    }

    // Named on stdout because a generator that writes files silently
    // leaves a reader guessing where they went, and the overview is the
    // one a person opens first.
    println!(
        "Wrote {} page{} to {}. Start at {}.",
        pages.len(),
        if pages.len() == 1 { "" } else { "s" },
        out.display(),
        out.join("index.md").display()
    );
    ExitCode::SUCCESS
}

/// Copy each linked `foreign` module into the output tree, or refuse.
///
/// An emitted import naming a file the bundle does not contain is worse
/// than the `ReferenceError` it replaced: it fails at deploy rather than at
/// build, and further from its cause.
///
/// Shared by `zdc build` and `zdc deploy` rather than written twice (#225).
/// The two trees differ, and only in the destinations — which the caller
/// brings, already resolved for its own layout. Everything else is
/// identical, and the part that must not diverge is the sandbox rule: a
/// second copy of it is a second chance to weaken one of them.
fn ship_linked_modules(
    entry: &Path,
    out: &Path,
    modules: &std::collections::BTreeSet<zdc_codegen::LinkedModule>,
) -> Result<(), ExitCode> {
    if modules.is_empty() {
        return Ok(());
    }
    let root = entry.parent().unwrap_or(Path::new("."));
    let Ok(canonical_root) = root.canonicalize() else {
        eprintln!(
            "error: the project directory `{}` could not be resolved",
            root.display()
        );
        return Err(ExitCode::FAILURE);
    };

    for module in modules {
        let relative = module.specifier.trim_start_matches("./");
        let source = canonical_root.join(relative);
        if let Some(reason) = zdc_hir::sandbox::refuse(&canonical_root, &module.specifier, &source)
        {
            eprintln!(
                "error: `foreign … from \"{}\"` names a file that {}",
                module.specifier,
                reason.reason()
            );
            return Err(ExitCode::FAILURE);
        }
        let target = out.join(&module.destination);
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(write_failure(parent, e));
            }
        }
        if let Err(e) = std::fs::copy(&source, &target) {
            eprintln!(
                "error: `foreign … from \"{}\"` names {}, which could not be read: {e}",
                module.specifier,
                source.display()
            );
            return Err(write_failure(&target, e));
        }
    }
    Ok(())
}

/// Check every claim a program makes about itself — issue #169.
///
/// # What this command is, in one sentence
///
/// It compiles the program exactly as `zdc build` does, and then calls the
/// expectations the compiler printed into the build root.
///
/// That sentence is the whole design. The compiler already had to answer
/// *how do you run this program's own code with no browser and no Node* —
/// §17.4.8, for `static` state — and the answer it settled on is a printed
/// JavaScript module evaluated in an engine the compiler carries. A test
/// runner needs precisely that, so it reuses precisely that. Nothing here
/// interprets ZDeceptron; **what a claim is checked against is the code
/// that ships**, which is the only reason a passing claim means anything.
///
/// # What it cannot reach, stated plainly
///
/// A claim is evaluated at `static` placement, so it may read what a build
/// may read: pure functions, other `static` state, and the prelude. It may
/// **not** read `client`, `server` or `durable` state, and it may not
/// render or inspect a `view`. Those refusals are not special-cased here —
/// they are the ordinary placement diagnostics, arriving for a claim for
/// the same reason they arrive for a `static` signal.
///
/// So: pure computation is testable today and interaction is not. That is
/// the honest boundary, and it is where the value is: `sorting.zd`,
/// `poker.zd`, `knapsack.zd`, `edit-distance.zd` and the prelude are all
/// pure, and none of them had any way to state what they should compute.
/// Reaching a `view` needs the DOM shim in `zdc-runtime` driven from a
/// second kind of root, and reaching a signal needs the reactive core
/// stepped by something that can write to it; both are further work and
/// neither is started.
fn test(file: &Path) -> ExitCode {
    let path = file.display().to_string();
    let Ok(compiled) = front_end(file) else {
        return ExitCode::FAILURE;
    };
    let Some(cleared) = compiled.verdict.clearance() else {
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
        cleared,
    };

    // The same printer `zdc build` uses, and deliberately so: a claim
    // checked against a differently-printed module would be a claim about
    // a program nobody ships.
    let module = match zdc_codegen::build_module(&inputs, &options) {
        Ok(Some(module)) => module,
        // A program with no `static` state and no claims prints no build
        // root at all, which is the same observation as "there is nothing
        // to check" — so it is reported the same way as an empty one.
        Ok(None) => return no_claims(&path),
        Err(errors) => {
            report(&compiled.linked, errors);
            return ExitCode::FAILURE;
        }
    };
    if module.tests.is_empty() {
        return no_claims(&path);
    }

    let directory = file.parent().unwrap_or(Path::new("."));
    let outcomes = match zdc_codegen::run_tests(&module, directory) {
        Ok(outcomes) => outcomes,
        // The module would not even load, so no claim was reached. One
        // failure, not one per claim: they all have the same cause.
        Err(error) => {
            let diagnostic = Diagnostic::file_error(error.report());
            eprint!("{}", render("", &path, &diagnostic));
            return ExitCode::FAILURE;
        }
    };

    println!("zdc test · {path}");
    println!();

    let mut held = 0usize;
    let mut broken: Vec<zdc_codegen::Broken> = Vec::new();
    let mut undecided = 0usize;
    // Two passes over the outcomes, because the two audiences are
    // different: the list on stdout is *what was checked*, in the order it
    // was written, and it is worth reading even when everything passed.
    // The diagnostics on stderr are *what to fix*, and they come after, so
    // that a terminal showing both ends with the actionable half.
    for outcome in &outcomes {
        match &outcome.verdict {
            zdc_codegen::ClaimVerdict::Held => {
                held += 1;
                println!("  held       {}", outcome.claim);
            }
            zdc_codegen::ClaimVerdict::Broken(_) => {
                println!("  broken     {}", outcome.claim);
            }
            zdc_codegen::ClaimVerdict::Unevaluable(_) => {
                undecided += 1;
                println!("  undecided  {}", outcome.claim);
            }
        }
    }
    let mut unevaluable: Vec<zdc_codegen::EvaluationError> = Vec::new();
    for outcome in outcomes {
        match outcome.verdict {
            zdc_codegen::ClaimVerdict::Held => {}
            zdc_codegen::ClaimVerdict::Broken(failure) => broken.push(failure),
            zdc_codegen::ClaimVerdict::Unevaluable(error) => unevaluable.push(error),
        }
    }

    println!();
    println!("{}", tally(held, broken.len(), undecided));

    if broken.is_empty() && undecided == 0 {
        return ExitCode::SUCCESS;
    }
    // Collected above and rendered here rather than in that loop, so that
    // both kinds of diagnostic land on the same side of the tally.
    // Printing them as they were found put the undecided ones *before* it
    // and the broken ones after, which in a terminal showing both streams
    // reads as though the run had reported twice.
    for error in unevaluable {
        let diagnostic = Diagnostic::file_error(error.report());
        eprint!("{}", render("", &path, &diagnostic));
    }
    // Rendered against the file each span belongs to, by the same path
    // every other diagnostic in this binary takes: a claim may be written
    // in one file about a function in another, and the caret has to land
    // on the `expect` line the reader wrote.
    report(&compiled.linked, broken);
    ExitCode::FAILURE
}

/// The one-line summary, in the vocabulary the per-claim lines used.
///
/// Zero counts are omitted rather than printed as `0 broken`: a suite that
/// says `6 held` has said everything, and a reader who has to subtract to
/// find out whether anything failed is being made to do arithmetic on a
/// summary line.
fn tally(held: usize, broken: usize, undecided: usize) -> String {
    let mut parts = Vec::new();
    if held > 0 {
        parts.push(format!("{held} held"));
    }
    if broken > 0 {
        parts.push(format!("{broken} broken"));
    }
    if undecided > 0 {
        parts.push(format!("{undecided} undecided"));
    }
    parts.join(", ")
}

/// A program that states nothing is not a program that passed.
///
/// The distinction is the whole reason this is not silence-plus-exit-0: a
/// runner that printed a green tally for a file with no claims in it would
/// be reporting on a suite that does not exist, and that is the failure
/// mode a test runner can least afford — it is indistinguishable from
/// working.
fn no_claims(path: &str) -> ExitCode {
    println!("zdc test · {path}");
    println!();
    println!("This program makes no claims, so nothing was checked.");
    println!("A claim is a `test \"…\"` declaration with one indented `expect` line.");
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

    // The flow pass's own permission to emit, asked for here as it is in
    // `build`: `front_end` has already reported and refused on a leak, so
    // this always succeeds — but an `Inputs` cannot be built without
    // asking, which is what makes §16.3.12's invariant 3 a property of the
    // type system rather than a convention.
    let Some(cleared) = compiled.verdict.clearance() else {
        return ExitCode::FAILURE;
    };

    let inputs = zdc_codegen::Inputs {
        hir: &compiled.hir,
        split: &compiled.split,
        verdict: &compiled.verdict,
        table: &compiled.table,
        cleared,
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
    // A deployment is a release build for the same reason a `zdc build`
    // is, and it must be the *same* one: a deployed page that differed
    // from the built page in any way would make "test it locally" a
    // weaker statement than it is (#135).
    let bundle = bundle.minified();

    let settings = match deploy_options(args, name) {
        Ok(settings) => settings,
        Err(message) => return command_failure("zdc deploy", &message),
    };
    let program = zdc_deploy::Program {
        functions: &bundle.functions,
        linked: &bundle.linked_modules,
        durable: &bundle.durable,
        environment: &bundle.environment,
        // What the target may cache for a year, from the bundle rather
        // than from an adapter's guess at what the emitter named (#137).
        immutable: &bundle.immutable,
    };
    let deployment = match zdc_deploy::generate(&program, &settings) {
        Ok(deployment) => deployment,
        Err(refusal) => return command_failure("zdc deploy", &refusal.message),
    };

    print!("{}", deployment.capabilities.report());
    if args.report_only {
        return ExitCode::SUCCESS;
    }

    // The browser half goes under `public/`, which is where every target's
    // static handling looks: Cloudflare's `[assets]`, Vercel's
    // `outputDirectory`, and the Deno entry's own file read. The directory
    // is asked for rather than written out, because the adapter has to
    // place a browser-imported `foreign` module in the same place (#225)
    // and two spellings of one directory is how those come apart.
    let browser = args.out.join(settings.target.browser_root());
    let mut files: Vec<(PathBuf, &str)> = vec![
        (browser.join("client.js"), bundle.client_js.as_str()),
        // The stylesheet's name carries a content hash and the document
        // links that name, so the bundle says where it goes (#137).
        (
            browser.join(&bundle.styles_path),
            bundle.styles_css.as_str(),
        ),
        (browser.join("manifest.json"), bundle.manifest_json.as_str()),
    ];
    // A module with no `view` has no page: writing one would ship a
    // document whose only script imports a `main` the module does not
    // export (§16.3.1).
    if let Some(index_html) = &bundle.index_html {
        files.push((browser.join("index.html"), index_html.as_str()));
    }
    // The page loads this and no inline script, so the policy it carries
    // needs no exception for one (#146).
    if let Some(boot_js) = &bundle.boot_js {
        files.push((browser.join("boot.js"), boot_js.as_str()));
    }
    // `zdc build` writes a release build: no `// $dev` assertions (#140).
    let runtime = zdc_codegen::runtime_files(&bundle.runtime, zdc_codegen::Mode::Release);
    for (relative, source) in &runtime {
        files.push((browser.join(relative), source.as_str()));
    }
    // §14C.3b's generated files. They are part of the site, so they go
    // beside the page rather than being dropped on the way to a platform.
    for (relative, contents) in &evaluated.files {
        files.push((browser.join(relative), contents.as_str()));
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

    // The `foreign` modules the emitted imports point at (#225), copied by
    // the same routine and under the same sandbox rule `zdc build` uses —
    // the destinations are the deployment's, which is the only part that
    // differs, and the adapter has already worked those out.
    //
    // #225 proposed skipping the sandbox check here, on the grounds that
    // the path was already validated at build time. It was not: `zdc
    // deploy` is its own command and never runs the build path, so a
    // project that is only ever deployed would meet no sandbox at all.
    // Checked here, on the canonical path, exactly as #188 requires — an
    // escaping module is refused by name rather than quietly left out.
    if let Err(code) = ship_linked_modules(file, args.out, &deployment.linked_modules) {
        return code;
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

/// A refusal from a command rather than from a program: an unusable flag,
/// a target that cannot do what was asked, a directory that will not be
/// overwritten.
///
/// Rendered through the same diagnostic path as everything else so these
/// read like compile errors, which is what they are. The command's own
/// name stands where a file name would, because there is no file to point
/// at and a blank there reads as a bug.
fn command_failure(command: &str, message: &str) -> ExitCode {
    let diagnostic = Diagnostic::file_error(message.to_string());
    eprint!("{}", render("", command, &diagnostic));
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
