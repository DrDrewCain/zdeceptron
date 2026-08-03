//! `zdc dev`, driven as a developer drives it: start the binary, edit the
//! file, watch the served bundle change.
//!
//! The server's own behaviour is covered in `zdc-dev`. What is checked
//! here is the part only the real process can show — that the arguments
//! are wired up, that a compile error at startup does **not** end the
//! process, and that saving the file really does produce a new bundle on
//! the port.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Generous, because this waits on a compiler and a filesystem poll on a
/// machine that may be running the rest of the suite at the same time. It
/// exists to turn a hang into a failure, not to measure anything.
const TIMEOUT: Duration = Duration::from_secs(30);

const VALID: &str = "state count is client Whole starting 0\n\nview\n    Text count\n";
const ALSO_VALID: &str =
    "state greeting is client Text starting \"hi\"\n\nview\n    Text greeting\n";
const BROKEN: &str = "view Text\n";

/// A scratch directory plus a running `zdc dev`, both cleaned up when the
/// test ends whether it passed or not.
struct Dev {
    child: Child,
    dir: PathBuf,
    source: PathBuf,
    log: PathBuf,
    errors: PathBuf,
}

impl Dev {
    fn start(name: &str, contents: &str) -> Dev {
        let dir = std::env::temp_dir().join(format!("zdc-dev-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("could not create the scratch directory");

        let source = dir.join("app.zd");
        std::fs::write(&source, contents).expect("could not write the source");

        let log = dir.join("stdout");
        let errors = dir.join("stderr");
        let child = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .args([
                "dev",
                source.to_str().expect("utf-8 path"),
                // Port 0 so the suite never collides with a developer's own
                // `zdc dev`, or with itself.
                "--port",
                "0",
            ])
            .stdout(Stdio::from(
                std::fs::File::create(&log).expect("could not create the stdout log"),
            ))
            .stderr(Stdio::from(
                std::fs::File::create(&errors).expect("could not create the stderr log"),
            ))
            .spawn()
            .expect("could not start zdc dev");

        Dev {
            child,
            dir,
            source,
            log,
            errors,
        }
    }

    fn stdout(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.errors).unwrap_or_default()
    }

    /// Wait until `needle` appears in the process's output, or fail.
    fn wait_for_output(&self, needle: &str) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let out = self.stdout();
            if out.contains(needle) {
                return out;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}\nstdout:\n{out}\nstderr:\n{}",
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// The address from the startup banner.
    fn addr(&self) -> SocketAddr {
        let out = self.wait_for_output("http://");
        let line = out
            .lines()
            .find(|line| line.contains("http://"))
            .expect("the banner was matched but then vanished");
        let after = line
            .split("http://")
            .nth(1)
            .expect("no address in the banner");
        let addr = after.split_whitespace().next().expect("empty address");
        addr.parse()
            .unwrap_or_else(|e| panic!("bad address {addr:?}: {e}"))
    }

    fn save(&self, contents: &str) {
        // The way editors save: write a temporary, rename it over the
        // target. If the watcher only survives in-place writes, this is
        // where it stops noticing.
        let temp = self.dir.join("app.zd.tmp");
        std::fs::write(&temp, contents).expect("could not write the replacement");
        std::fs::rename(&temp, &self.source).expect("could not rename over the source");
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A GET written by hand, so the tests need no HTTP client dependency.
fn get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("could not connect to the dev server");
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("could not send");
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("could not read the reply");
    String::from_utf8_lossy(&raw).into_owned()
}

/// Fetch `path` until the reply satisfies `done`, or fail.
fn get_until(addr: SocketAddr, path: &str, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let reply = get(addr, path);
        if done(&reply) {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {path} to change; last reply was:\n{reply}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn dev_serves_the_program_on_the_port_it_reports() {
    let mut dev = Dev::start("serves", VALID);
    let addr = dev.addr();

    let page = get(addr, "/");
    assert!(page.contains("200 OK"), "the page was not served:\n{page}");
    assert!(page.contains("<div id=\"app\">"), "no mount point:\n{page}");
    assert!(page.contains("EventSource"), "no live reload:\n{page}");

    let client = get(addr, "/client.js");
    assert!(
        client.contains("export function main"),
        "no bundle:\n{client}"
    );
    assert!(dev.is_running(), "the server stopped while serving");
}

#[test]
fn dev_names_the_file_it_is_watching() {
    let dev = Dev::start("banner", VALID);
    let banner = dev.wait_for_output("http://");
    assert!(
        banner.contains("app.zd"),
        "the banner must name the file:\n{banner}"
    );
}

#[test]
fn saving_the_file_produces_a_new_bundle_on_the_same_port() {
    let mut dev = Dev::start("rebuild", VALID);
    let addr = dev.addr();
    assert!(
        get(addr, "/client.js").contains("count"),
        "wrong first build"
    );

    dev.save(ALSO_VALID);

    let client = get_until(addr, "/client.js", |reply| reply.contains("greeting"));
    assert!(
        !client.contains("count"),
        "the old build is still served:\n{client}"
    );
    assert!(dev.is_running(), "the server stopped after a rebuild");
}

/// Spec §9 and the reason `zdc dev` exists: the fix is a keystroke away,
/// so a program that does not compile must not take the server down with
/// it. Exiting here would mean re-running the command after every typo.
#[test]
fn a_compile_error_at_startup_reports_and_keeps_watching() {
    let mut dev = Dev::start("startup-error", BROKEN);
    let addr = dev.addr();

    assert!(dev.is_running(), "a compile error must not end the process");

    let stderr = dev.stderr();
    assert!(
        stderr.contains("line break"),
        "the diagnostic must reach the terminal:\n{stderr}"
    );

    let page = get(addr, "/");
    assert!(
        page.contains("line break"),
        "the diagnostic must reach the browser:\n{page}"
    );
    assert!(
        !page.contains('\u{1b}'),
        "raw terminal escapes reached the browser:\n{page}"
    );

    // And the fix is picked up without restarting anything.
    dev.save(VALID);
    let page = get_until(addr, "/", |reply| reply.contains("<div id=\"app\">"));
    assert!(
        !page.contains("line break"),
        "the error page persisted:\n{page}"
    );
    assert!(dev.is_running(), "the server stopped after the fix");
}

#[test]
fn breaking_a_working_program_replaces_the_app_with_the_diagnostic() {
    let mut dev = Dev::start("break", VALID);
    let addr = dev.addr();
    assert!(
        get(addr, "/").contains("<div id=\"app\">"),
        "wrong first build"
    );

    dev.save(BROKEN);

    let page = get_until(addr, "/", |reply| reply.contains("line break"));
    assert!(
        page.contains("EventSource"),
        "no live reload on the error page:\n{page}"
    );
    assert!(dev.is_running(), "a compile error must not end the process");
}

/// A program the compiler refuses is refused by `zdc dev` in exactly the
/// words `zdc build` refuses it — the dev server does not paper over a
/// verdict and does not restate one in its own wording.
///
/// The program is `guestbook.zd` with the secret rendered, because that is
/// the refusal it matters most that both agree about.
#[test]
fn dev_refuses_what_build_refuses_and_says_the_same_thing() {
    let dir = std::env::temp_dir().join(format!("zdc-dev-refusal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("could not create the scratch directory");
    let example = dir.join("leak.zd");
    std::fs::write(&example, LEAK).expect("could not write the source");

    let built = Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args([
            "build",
            example.to_str().expect("utf-8 path"),
            "--out",
            dir.join("out").to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("could not run zdc build");
    assert_eq!(
        built.status.code(),
        Some(1),
        "the program is supposed to be refused"
    );
    let expected = String::from_utf8_lossy(&built.stderr).into_owned();

    let dev = Dev::start("refusal", LEAK);
    dev.wait_for_output("http://");

    // Same bytes, modulo the path each command was given.
    let normalize = |report: &str, path: &str| report.replace(path, "<source>");
    assert_eq!(
        normalize(&dev.stderr(), dev.source.to_str().expect("utf-8 path")),
        normalize(&expected, example.to_str().expect("utf-8 path")),
        "`zdc dev` and `zdc build` disagree about the same program"
    );
}

/// `guestbook.zd`, with the one line its own comment says is a compile
/// error: the secret, rendered.
const LEAK: &str = concat!(
    "secret state apiKey is server Text from environment \"GREETING_API_KEY\"\n",
    "state name is client Text starting \"\"\n",
    "state greeting is server Text from politeGreeting with name, apiKey\n",
    "\n",
    "function politeGreeting with who, key\n",
    "    if who is \"\"\n",
    "        give \"Hello, stranger.\"\n",
    "    give \"Hello, \" + who + \".\"\n",
    "\n",
    "view\n",
    "    Column\n",
    "        Input name, hint is \"your name\"\n",
    "        Text apiKey\n",
);

#[test]
fn a_file_that_does_not_exist_exits_rather_than_watching_nothing() {
    // A typo in the argument is not a compile error: there is nothing to
    // watch, so staying up would only hide the mistake.
    let missing = std::env::temp_dir().join(format!("zdc-dev-{}-absent.zd", std::process::id()));
    let _ = std::fs::remove_file(&missing);

    let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args(["dev", missing.to_str().expect("utf-8 path"), "--port", "0"])
        .output()
        .expect("could not run zdc dev");

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not read"),
        "unclear message:\n{stderr}"
    );
    assert!(
        stderr.contains("absent.zd"),
        "the path must be named:\n{stderr}"
    );
}

#[test]
fn a_port_already_in_use_is_reported_and_names_the_flag_to_change_it() {
    let dev = Dev::start("port-clash", VALID);
    let addr = dev.addr();

    let source = dev.dir.join("second.zd");
    std::fs::write(&source, VALID).expect("could not write the second source");
    let output = Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args([
            "dev",
            source.to_str().expect("utf-8 path"),
            "--port",
            &addr.port().to_string(),
        ])
        .output()
        .expect("could not run zdc dev");

    assert_eq!(output.status.code(), Some(1), "expected exit code 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--port"), "no way out offered:\n{stderr}");
}
