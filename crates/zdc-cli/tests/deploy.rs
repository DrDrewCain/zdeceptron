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
