//! `zdc deploy`, through the binary a developer actually runs.
//!
//! Nothing here deploys anything, and the subcommand cannot: it writes
//! files and prints a report. That is the point of the last test in this
//! file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(args)
        .output()
        .expect("failed to run the zdc binary")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("zdc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn deploy(out: &TempDir, extra: &[&str]) -> Output {
    let guestbook = example("guestbook.zd");
    let mut args: Vec<&str> = vec![
        "deploy",
        guestbook.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ];
    args.extend_from_slice(extra);
    run(&args)
}

#[test]
fn deploying_writes_the_browser_half_the_handlers_and_the_platform_config() {
    let out = TempDir::new("deploy-cloudflare");
    let output = deploy(&out, &["--target", "cloudflare"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    for expected in [
        "public/index.html",
        "public/client.js",
        "public/runtime/signal.js",
        "functions/greeting.js",
        "functions/visits.incr.js",
        "_zd/router.js",
        "_zd/endpoints.js",
        "_zd/store.js",
        "worker.js",
        "wrangler.toml",
        "CAPABILITIES.md",
    ] {
        assert!(
            out.path.join(expected).is_file(),
            "the deployment is missing {expected}"
        );
    }
    // The browser half is under `public/`, which is what `wrangler.toml`'s
    // `[assets]` directory points at.
    assert!(!out.path.join("client.js").exists());

    // The cache rules go inside that directory too, because `_headers` is
    // read by the static-assets handling and not by `wrangler.toml`
    // (#137). Every path it names must be a file the deployment contains,
    // or the rule is for a URL nobody can request.
    let headers = std::fs::read_to_string(out.path.join("public/_headers")).expect("_headers");
    let mut ruled = 0;
    for line in headers.lines() {
        let Some(path) = line.strip_prefix('/') else {
            continue;
        };
        assert!(
            out.path.join("public").join(path).is_file(),
            "`_headers` names {path}, which the deployment does not contain:\n{headers}"
        );
        ruled += 1;
    }
    assert!(ruled > 0, "no rule at all:\n{headers}");
}

/// The report is printed, not merely written — a user who has to open a
/// file to find out that their stream dies at 900 seconds will not open it.
#[test]
fn the_capability_report_is_printed_before_anything_is_written() {
    let out = TempDir::new("deploy-report");
    let output = deploy(&out, &["--target", "lambda", "--report-only"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# AWS Lambda — what you are getting"));
    assert!(stdout.contains("Max stream duration | 900 s"));
    assert!(stdout.contains("Live sync |"));
    assert!(stdout.contains("Atomic writes |"));
    // falsifiable: the two arms are the same warning in two wordings —
    // the report's own summary line and the sentence quoted from AWS's
    // documentation — and the adapter prints one or the other depending
    // on how much of the quotation fits. Neither is a substring of the
    // rest of the report, so a report that dropped the billing warning
    // satisfies neither arm.
    assert!(
        stdout.contains("does not stop when the client disconnects")
            || stdout.contains("not interrupted when the invoking client's connection is broken"),
        "the billing warning is the whole reason this report exists:\n{stdout}"
    );
    assert!(!out.path.exists(), "`--report-only` wrote files");
}

#[test]
fn an_impossible_combination_fails_the_build_and_names_the_limitation() {
    let out = TempDir::new("deploy-alb");
    let output = deploy(&out, &["--target", "lambda", "--front", "alb"]);
    assert_eq!(output.status.code(), Some(1), "the ALB must be refused");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Application Load Balancer"), "{stderr}");
    assert!(stderr.contains("visits"), "{stderr}");
    assert!(!out.path.exists(), "a refused deployment wrote files");
}

#[test]
fn an_unknown_target_says_why_azure_is_absent() {
    let out = TempDir::new("deploy-azure");
    let output = deploy(&out, &["--target", "azure"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("230 seconds"), "{stderr}");
    assert!(stderr.contains("atomic increment"), "{stderr}");
}

/// A program that does not compile does not deploy, and says so in the
/// compiler's own words rather than the deployer's.
#[test]
fn a_program_that_does_not_compile_does_not_deploy() {
    let out = TempDir::new("deploy-broken");
    let broken = std::env::temp_dir().join(format!("zdc-{}-deploy-broken.zd", std::process::id()));
    std::fs::write(&broken, "view Text\n").expect("write");
    let output = run(&[
        "deploy",
        broken.to_str().expect("utf-8 path"),
        "--target",
        "deno",
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);
    let _ = std::fs::remove_file(&broken);
    assert_eq!(output.status.code(), Some(1));
    assert!(!out.path.exists());
}

/// A program whose two halves each call a `foreign` of their own, and the
/// two JavaScript modules they name. Written to a directory of its own
/// because the sandbox's root is the entry file's parent (§14C.3b), so
/// where the fixture lives is part of what is being tested.
const FOREIGN_APP_ZD: &str = concat!(
    "foreign draw is client\n",
    "    from \"./draw.js\" as \"mount\"\n",
    "    takes level is Whole\n",
    "    gives Text\n",
    "foreign readAt is server\n",
    "    from \"./io.js\" as \"readAt\"\n",
    "    takes path is Text\n",
    "    gives Text\n",
    "state n is client Whole starting 1\n",
    "state out is client Text from draw with level is n\n",
    "state contents is server Text from readAt with path is \"in.txt\"\n",
    "view\n",
    "    Column\n",
    "        Text out\n",
    "        when contents\n",
    "            Loading           show Text \"…\"\n",
    "            Failed with error show Text error.message\n",
    "            Ready with body   show Text body\n",
);
const FOREIGN_DRAW_JS: &str = "export function mount() {\n  return {};\n}\n";
const FOREIGN_IO_JS: &str = "export function readAt(path) {\n  return path;\n}\n";

/// **A deployed program takes its `foreign` modules with it (#225).**
///
/// `zdc build` ships them and `zdc deploy` did not, so a deployment carried
/// an `import` naming a file it did not contain. That is the same defect as
/// #223's client half, one step further from its cause: the build-time
/// version failed on the machine that produced it, and this one fails on a
/// platform, in whatever words that platform has for an unresolvable
/// import.
///
/// Two targets rather than one, because the destination is not a constant.
/// The browser half lands under `public/` — Cloudflare's `[assets]`
/// directory, Vercel's `outputDirectory` — while an endpoint lands in
/// `functions/` beside the others, and the emitted import is the author's
/// specifier verbatim, so each module has to be beside *its own* importer.
/// A single target would pass with either half shipped to the wrong place.
#[test]
fn a_deployment_ships_the_foreign_modules_its_imports_name() {
    let project = TempDir::new("deploy-foreign-project");
    std::fs::create_dir_all(&project.path).expect("the project directory");
    std::fs::write(project.path.join("draw.js"), FOREIGN_DRAW_JS).expect("the client's module");
    std::fs::write(project.path.join("io.js"), FOREIGN_IO_JS).expect("the endpoint's module");
    let source = project.path.join("app.zd");
    std::fs::write(&source, FOREIGN_APP_ZD).expect("the program");

    for target in ["cloudflare", "vercel"] {
        let out = TempDir::new(&format!("deploy-foreign-{target}"));
        let output = run(&[
            "deploy",
            source.to_str().expect("utf-8 path"),
            "--target",
            target,
            "--out",
            out.path.to_str().expect("utf-8 path"),
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{target} did not deploy:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        // The import is asserted as well as the file, in both halves. The
        // pair is the whole claim: a module shipped somewhere the import
        // does not name is as broken as one not shipped at all, and either
        // assertion alone would accept that.
        let client = std::fs::read_to_string(out.path.join("public/client.js"))
            .expect("the browser half is written");
        assert!(
            client.contains("from './draw.js'"),
            "{target}: the client imports its foreign by relative path:\n{client}"
        );
        assert_eq!(
            std::fs::read_to_string(out.path.join("public/draw.js")).ok(),
            Some(FOREIGN_DRAW_JS.to_string()),
            "{target}: `client.js` sits in `public/`, so `./draw.js` is `public/draw.js`"
        );

        let endpoint = std::fs::read_to_string(out.path.join("functions/contents.js"))
            .expect("the server half is written");
        assert!(
            endpoint.contains("from './io.js'"),
            "{target}: the endpoint imports its foreign by relative path:\n{endpoint}"
        );
        assert_eq!(
            std::fs::read_to_string(out.path.join("functions/io.js")).ok(),
            Some(FOREIGN_IO_JS.to_string()),
            "{target}: an endpoint sits in `functions/`, so `./io.js` is `functions/io.js`"
        );
    }
}

/// A `foreign` naming a file outside the project is refused by name, on the
/// deploy path as on the build path (#188, #223).
///
/// The rule cannot be inherited from `zdc build`: `zdc deploy` is its own
/// command and never runs the build path, so a project that is only ever
/// deployed would meet no sandbox at all. The specifier here contains no
/// `..` and no leading `/` — it is a symbolic link planted inside the
/// project — because that is the escape only the canonical path catches,
/// and a check that ran on the written specifier would pass it.
#[test]
fn a_deployed_foreign_that_resolves_outside_the_project_is_refused_by_name() {
    let outside = TempDir::new("deploy-foreign-outside");
    std::fs::create_dir_all(&outside.path).expect("the directory outside the project");
    std::fs::write(
        outside.path.join("stolen.js"),
        "export function mount() {}\n",
    )
    .expect("a module to steal");

    let project = TempDir::new("deploy-foreign-escape");
    std::fs::create_dir_all(&project.path).expect("the project directory");
    symlink(
        &outside.path.join("stolen.js"),
        &project.path.join("draw.js"),
    );
    let source = project.path.join("app.zd");
    std::fs::write(
        &source,
        concat!(
            "foreign draw is client\n",
            "    from \"./draw.js\" as \"mount\"\n",
            "    takes level is Whole\n",
            "    gives Text\n",
            "state n is client Whole starting 1\n",
            "state out is client Text from draw with level is n\n",
            "view\n",
            "    Column\n",
            "        Text out\n",
        ),
    )
    .expect("the escaping program");

    let out = TempDir::new("deploy-foreign-escape-out");
    let output = run(&[
        "deploy",
        source.to_str().expect("utf-8 path"),
        "--target",
        "cloudflare",
        "--out",
        out.path.to_str().expect("utf-8 path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a module resolving outside the project must not deploy"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("./draw.js") && stderr.contains("points outside the project"),
        "the refusal has to name the module and the fault:\n{stderr}"
    );
    assert!(
        !out.path.join("public/draw.js").exists(),
        "a refused module was copied anyway"
    );
}

/// The subcommand generates and reports. It does not deploy, and the
/// message says so, because "deploy" is a word that invites the assumption
/// that something reached the internet.
#[test]
fn deploying_says_plainly_that_nothing_was_deployed() {
    let out = TempDir::new("deploy-nothing");
    let output = deploy(&out, &["--target", "deno"]);
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Nothing has been deployed"), "{stderr}");
    assert!(
        stderr.contains("shim:"),
        "the shim size is part of the honesty"
    );
}

/// A symbolic link, on whichever platform the tests are running.
///
/// `std::os::unix` does not exist on Windows, so naming it directly in the
/// test above did not fail there at runtime — it failed to *compile*,
/// taking every other test in this binary down with it. The same two lines
/// are in `cli.rs`, and stay written out in both: each integration test is
/// its own crate, and a test that proves a symlink cannot escape the
/// project has to create a real one.
#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("the symbolic link");
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("the symbolic link");
}

/// **A deployment carries the asset directory, like a build does.**
///
/// It did not, and the shape of the failure is why this test exists rather
/// than a comment. `deploy` compiled, wrote every file it knew about and
/// reported success — while the document it wrote linked nothing from
/// `assets/` and no file from `assets/` was beside it. A site deployed that
/// way renders unstyled, and the deployment that produced it exits 0.
///
/// `build` reads the directory at `main.rs`'s asset step and `deploy` ran
/// two of those three steps while a comment in it claimed both ran "the
/// same two steps `zdc build` runs, in the same order".
///
/// `tree.zd` is the subject because it is an example with an `assets/`
/// directory holding a stylesheet — `guestbook.zd`, which the other tests
/// here use, has none, which is precisely why they never noticed.
#[test]
fn a_deployment_carries_the_asset_directory() {
    let out = TempDir::new("deploy-assets");
    let tree = example("tree/tree.zd");
    let output = run(&[
        "deploy",
        tree.to_str().expect("utf-8 path"),
        "--out",
        out.path.to_str().expect("utf-8 path"),
        "--target",
        "cloudflare",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");

    let stylesheet = out.path.join("public/assets/tree.css");
    assert!(
        stylesheet.exists(),
        "the asset directory's stylesheet is not in the deployment. The document \
         links it, so a site deployed like this renders unstyled and the deploy \
         exits 0 saying nothing:\n{}",
        std::fs::read_dir(out.path.join("public"))
            .map(|entries| entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );

    // Shipped *and* linked: either one alone is a page that is still wrong.
    let document = std::fs::read_to_string(out.path.join("public/index.html"))
        .expect("the deployment writes a document");
    assert!(
        document.contains("assets/tree.css"),
        "the stylesheet is in the deployment but the document does not link it:\n{document}"
}

/// **A dry run writes nothing at all** — issue #131.
///
/// The strongest form of the claim: not "writes no files" but that the
/// output directory does not come into existence. A command that created
/// the tree and then declined to fill it would pass a file-count
/// assertion and still have changed the disk.
#[test]
fn a_dry_run_does_not_even_create_the_output_directory() {
    let out = TempDir::new("deploy-dry-run-writes-nothing");
    let output = deploy(&out, &["--target", "cloudflare", "--dry-run"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        !out.path.exists(),
        "a dry run created {}",
        out.path.display()
    );
}

/// The endpoint table, which is the half of #131 a reader cannot get any
/// other way before the deployment is live.
///
/// `guestbook.zd` derives three: a value with an input, a value without,
/// and a command. Nobody wrote any of them — they are what the tier split
/// made of where the program put its state — so the table is the first
/// place their wire names and argument order appear.
#[test]
fn a_dry_run_names_every_endpoint_the_split_derived() {
    let out = TempDir::new("deploy-dry-run-endpoints");
    let output = deploy(&out, &["--target", "cloudflare", "--dry-run"]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("endpoints"), "{text}");
    // Name, shape and the file it lands in, for each of the three.
    for expected in [
        "greeting",
        "value",
        "functions/greeting.js",
        "visits.incr",
        "command",
        "functions/visits.incr.js",
    ] {
        assert!(
            text.contains(expected),
            "the endpoint table does not mention `{expected}`:\n{text}"
        );
    }
    // The wire order of the inputs, which is what a hand-written client
    // has to agree with.
    assert!(text.contains("(name)"), "{text}");
}

/// **The difference, which is the whole question #131 asks.**
///
/// Three states, in the order a reader meets them: an empty directory,
/// the same directory after a real deploy, and one file changed
/// underneath. "Would write 21 files" is the same sentence in all three;
/// only a report that separates them answers "what would change".
#[test]
fn a_dry_run_says_what_would_change_against_what_is_already_there() {
    let out = TempDir::new("deploy-dry-run-difference");

    let fresh = deploy(&out, &["--target", "cloudflare", "--dry-run"]);
    let fresh = String::from_utf8_lossy(&fresh.stdout).into_owned();
    assert!(fresh.contains("0 to replace, 0 already correct"), "{fresh}");
    assert!(fresh.contains("add      "), "{fresh}");

    let written = deploy(&out, &["--target", "cloudflare"]);
    assert_eq!(written.status.code(), Some(0), "{written:?}");

    let again = deploy(&out, &["--target", "cloudflare", "--dry-run"]);
    let again = String::from_utf8_lossy(&again.stdout).into_owned();
    assert!(
        again.contains("0 to add, 0 to replace"),
        "a deploy that changes nothing must say so:\n{again}"
    );
    assert!(
        again.contains("nothing — every file is already exactly this"),
        "{again}"
    );

    // One file edited underneath, which is the case the report exists for:
    // it is named, and the other twenty are not.
    let drifted = out.path.join("worker.js");
    let original = std::fs::read_to_string(&drifted).expect("the worker the deploy wrote");
    std::fs::write(&drifted, format!("{original}\n// edited by hand\n"))
        .expect("could not edit the worker");

    let third = deploy(&out, &["--target", "cloudflare", "--dry-run"]);
    let third = String::from_utf8_lossy(&third.stdout).into_owned();
    assert!(
        third.contains("1 to replace"),
        "the edited file was not reported:\n{third}"
    );
    assert!(third.contains("replace  "), "{third}");
    assert!(third.contains("worker.js"), "{third}");
    // And the deploy still did not run: the hand edit is still there.
    assert_eq!(
        std::fs::read_to_string(&drifted).expect("the worker"),
        format!("{original}\n// edited by hand\n"),
        "a dry run overwrote the file it was reporting on"
    );
}
