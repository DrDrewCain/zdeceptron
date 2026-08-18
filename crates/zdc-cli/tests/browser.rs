//! The emitted program, in a real browser.
//!
//! Every other JavaScript suite in this workspace runs under the embedded
//! engine, which is what lets `cargo test` be the only command anyone
//! needs. That engine is not a browser, and `runtime/dom-shim.js` is not a
//! DOM: it models no HTML parser insertion modes at all. #205 is what that
//! costs. A `Paragraph` holding a block element emitted a walk that the
//! shim and the emitter both agreed on, both parity tests passed, and the
//! page threw a `TypeError` on load in every browser — because the parser
//! closes a `p` before a block and the walk descended into a paragraph the
//! browser had left empty. Two halves agreeing on a tree no browser builds
//! is a failure no amount of shim testing can reach (#162).
//!
//! So this asks the only authority that can answer: it builds a program,
//! serves the built output over HTTP, loads it in a real browser, and
//! checks the page rendered. A program whose walk is wrong renders
//! nothing, because the exception stops module evaluation before the view
//! is attached.
//!
//! **Served rather than opened.** `zdc build` emits ES modules, and a
//! browser refuses those over `file://` — the README says as much. The
//! server here is static and deliberately small: `zdc dev` is a watcher,
//! and what a user deploys is the built directory, so the built directory
//! is what gets loaded.
//!
//! **Nothing here waits for the browser to exit.** It writes the DOM and
//! then, reliably, does not exit; see [`rendered`]. Waiting for it is what
//! made this job's first CI run sit for forty minutes and end in a manual
//! cancellation.
//!
//! **What this harness cannot reach: `zdc dev`.** Every test here builds
//! with `zdc build` and serves the output statically, and #281 asks for the
//! other half — the dev server is what the tutorial and `zdc new` both tell
//! a reader to run, and nothing has ever loaded it in a browser.
//!
//! It cannot be loaded *this way*, and the reason is worth writing down
//! because it costs an afternoon to rediscover. `--dump-dom` writes the
//! serialisation when the page is done, and `--virtual-time-budget` decides
//! "done" by the page running out of pending work. A `zdc dev` page holds a
//! live-reload `EventSource` open, which is a request that by design never
//! completes — so the budget is never spent, the dump never comes, and the
//! test fails at the deadline looking exactly like a page that never
//! rendered. Dropping the budget for a real clock does not help: the dump
//! still waits on a document that never reports itself finished.
//!
//! So covering `zdc dev` needs a driver that can observe a live page rather
//! than a finished one — CDP over a socket, which is a different harness
//! from this one rather than another test in it. Driven that way, the page
//! loads in about 900 ms and renders its view, repeatedly, with the stream
//! open and answering fetches alongside it; that is how #281 was established
//! as not reproducing. What is still missing is a *test* that says so.

//! **`#[ignore]`d, and CI runs it anyway.** A browser is the one dependency
//! this workspace does not otherwise need, so `cargo test` stays honest
//! for somebody who has not installed one. The `browser` job runs
//! `--ignored`, which is what makes this a gate rather than a suggestion.
//! Set `ZDC_BROWSER` to use a specific binary; otherwise the usual names
//! are tried.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Where a headless browser might be, in the order worth trying.
///
/// `ZDC_BROWSER` wins, so a runner with a browser somewhere unusual — or a
/// developer who wants a different engine — needs no change here.
const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/google-chrome",
];

fn browser() -> Option<String> {
    if let Ok(named) = std::env::var("ZDC_BROWSER") {
        // Set and wrong is a mistake worth failing on rather than
        // skipping past: somebody meant to run this.
        assert!(
            Path::new(&named).exists() || which(&named).is_some(),
            "ZDC_BROWSER is set to `{named}`, which is not a program that exists"
        );
        return Some(named);
    }
    BROWSERS.iter().find_map(|name| which(name))
}

fn which(name: &str) -> Option<String> {
    if name.contains('/') {
        return Path::new(name).exists().then(|| name.to_string());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.exists())
        .map(|candidate| candidate.display().to_string())
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Under the build directory rather than under `/tmp`.
    ///
    /// A snap-packaged Chromium — which is what `chromium` is on several
    /// Linux distributions, Ubuntu included — is confined and cannot read
    /// a `--user-data-dir` outside the user's home. It does not say so; it
    /// exits having written nothing, which is indistinguishable from a
    /// page that failed to render. The build directory is inside the
    /// checkout, and the checkout is under `$HOME` on the runners and on
    /// any normal developer machine.
    fn new(name: &str) -> TempDir {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let path = base.join(format!("browser-tests/zdc-{}-{name}", std::process::id()));
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

fn build(source: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zdc"))
        .args([
            "build",
            source.to_str().expect("utf-8 path"),
            "--out",
            out.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("failed to run the zdc binary")
}

/// Serve `root` until something asks for `/__stop`.
///
/// Static and just enough: the browser fetches the document, one module
/// graph and a stylesheet. The content types are load-bearing rather than
/// decorative — a browser refuses a module served as anything but
/// JavaScript, and the failure looks exactly like the bug this file is
/// hunting.
///
/// # Why it is not the serial loop it started as
///
/// **This was not what hung the job** — that was the browser not exiting,
/// and [`rendered`] carries that story. The serial loop was found while
/// looking for it, and is fixed here because it is a real defect that
/// would have hung something later.
///
/// The first version accepted a connection, **read the request on the
/// accept thread**, answered it, and went back to `accept`. Against the
/// embedded engine that is indistinguishable from a real server, because
/// that engine asks for one file at a time and always sends a request on
/// every socket it opens.
///
/// A browser does not. It **opens sockets it has nothing to say on yet** —
/// preconnecting is a latency optimisation, and such a socket may carry a
/// request later or never. Reading on the accept thread parks the server
/// in `read` on one of those while every later request queues in a backlog
/// nobody is emptying.
///
/// So: **the accept thread only accepts.** Reading happens on a thread per
/// connection under a read timeout, where a socket that never speaks costs
/// one parked thread for [`IDLE`] and nothing else. `Connection: close` is
/// there too, so the client is never left holding a socket this server has
/// finished with.
fn serve(root: PathBuf) -> (SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    listener
        .set_nonblocking(true)
        .expect("a listener that can be polled");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        loop {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(_) => continue,
            };
            let root = root.clone();
            let flag = std::sync::Arc::clone(&flag);
            std::thread::spawn(move || {
                let _ = stream.set_nonblocking(false);
                // A socket that never carries a request is a normal thing
                // for a browser to open; without this it is a thread
                // parked for the lifetime of the test.
                let _ = stream.set_read_timeout(Some(IDLE));
                let mut stream = stream;
                let mut buffer = [0u8; 2048];
                let Ok(read) = stream.read(&mut buffer) else {
                    return;
                };
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let Some(target) = request.split_whitespace().nth(1) else {
                    return;
                };
                if target == "/__stop" {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                respond(&root, target, &mut stream);
            });
        }
    });
    (address, handle)
}

/// How long an accepted socket gets to produce a request before its thread
/// gives up on it.
///
/// A speculatively opened connection never produces one, so this is the
/// normal path and not the error path.
const IDLE: std::time::Duration = std::time::Duration::from_secs(5);

/// Write one response for one request, then let the socket close.
fn respond(root: &Path, target: &str, stream: &mut std::net::TcpStream) {
    let relative = target
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or("");
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    // A served path may not climb: this is a test server, but a test
    // server that reads outside its root is a habit worth not forming.
    let path = root.join(relative);
    let inside = path
        .canonicalize()
        .ok()
        .zip(root.canonicalize().ok())
        .is_some_and(|(file, base)| file.starts_with(base));
    let body = if inside {
        std::fs::read(&path).ok()
    } else {
        None
    };
    // `Connection: close` on every response, including the 404. Without
    // it the client keeps the socket for its next request and this server
    // is not there any more; see `serve`'s comment for what that costs.
    let response = match body {
        Some(bytes) => {
            let kind = match path.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("json") => "application/json",
                _ => "application/octet-stream",
            };
            let mut head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                bytes.len()
            )
            .into_bytes();
            head.extend_from_slice(&bytes);
            head
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    let _ = std::io::Write::write_all(stream, &response);
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// How long a browser gets before the test decides it is stuck.
///
/// The dump is a few hundred milliseconds of virtual time; a cold start on
/// a loaded CI runner is a few seconds. A minute is far outside both, so
/// reaching it means something is wrong rather than slow.
const BROWSER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// The DOM a real browser built, after scripts ran.
///
/// # Why this waits for the dump and not for the browser
///
/// `--dump-dom` writes the serialised document and is documented to exit.
/// It does not reliably exit. On macOS it writes a complete, correct DOM
/// and then sits indefinitely while Chrome's updater and three crashpad
/// handlers run; on the `ubuntu-latest` runner the first CI run of this
/// job left `chrome` and `chrome_crashpad_handler` orphans behind after
/// forty minutes and a manual cancellation. The dump was never the
/// problem. Waiting on `wait()` was.
///
/// So the artefact is the signal: poll the file until it holds a whole
/// document, then kill the browser, which by then has already done
/// everything that was asked of it. Only if [`BROWSER_DEADLINE`] passes
/// with no complete document is anything actually wrong — and that is a
/// failure with a message rather than a job that runs until somebody
/// notices.
///
/// This is why the flags include `--no-first-run` and friends: they cut
/// down the background work that makes exiting unreliable. They reduce it;
/// they do not fix it, and nothing here depends on them doing so.
fn rendered(browser: &str, url: &str, profile: &Path) -> String {
    rendered_under(browser, url, profile, &[])
}

/// [`rendered`], with the browser told something about its reader.
///
/// One caller: the animation test, which loads the same page twice and
/// needs the second load to come from somebody who has asked their system
/// for less motion. That preference is not something a page can set — it
/// is a property of the reader — so the only way to test the behaviour
/// this compiler promises is to ask the browser to report it, which is
/// `--force-prefers-reduced-motion`. The flag is a request rather than a
/// guarantee, and the caller checks what the page actually reports rather
/// than assuming the flag took.
fn rendered_under(browser: &str, url: &str, profile: &Path, extra: &[&str]) -> String {
    // Piped to a file rather than to a pipe: `wait_timeout` is not in std,
    // so this polls `try_wait`, and a child writing into a pipe nobody is
    // draining fills the buffer and blocks — which would be a second
    // deadlock wearing the first one's clothes.
    let dump = profile.join("dump.html");
    let sink = std::fs::File::create(&dump).expect("a file for the dumped DOM");
    // Kept rather than discarded. When this test fails it fails on a
    // machine nobody is sitting at, and a browser that refuses to start
    // explains itself here and nowhere else — the first Linux failure of
    // this job reported an empty DOM and no reason, because stderr went to
    // `/dev/null`.
    let complaints = profile.join("browser.log");
    let log = std::fs::File::create(&complaints).expect("a file for the browser's complaints");
    let mut child = Command::new(browser)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            // Modules load asynchronously, so a dump taken the instant the
            // document is ready would race the render this test exists to
            // observe. Virtual time lets the page reach quiescence without
            // spending real seconds on it.
            "--virtual-time-budget=6000",
            // Background work that has nothing to do with the page and
            // everything to do with why the process does not exit.
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-crash-reporter",
            "--dump-dom",
        ])
        .args(extra)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .stdout(Stdio::from(sink))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("failed to launch the browser");

    let started = std::time::Instant::now();
    let dom = loop {
        // A complete document is the whole condition. `--dump-dom` writes
        // the serialisation in one go, so a file ending in `</html>` is
        // finished rather than partially flushed.
        let so_far = std::fs::read_to_string(&dump).unwrap_or_default();
        if so_far.trim_end().ends_with("</html>") {
            break so_far;
        }
        // Exited without ever producing one. This is a different failure
        // from "the page rendered nothing", and conflating them is what
        // made the first Linux run unreadable: a browser that never
        // started looks exactly like a program that threw, and only one of
        // those is this test's subject.
        if matches!(child.try_wait(), Ok(Some(_))) {
            assert!(
                !so_far.trim().is_empty(),
                "`{browser}` exited without writing a DOM at all, so nothing \
                 was learned about the page. This is the browser failing to \
                 run, not the program failing to render.\n\
                 --- what it said on stderr ---\n{}",
                complained(&complaints),
            );
            break so_far;
        }
        if started.elapsed() >= BROWSER_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "`{browser}` produced no complete DOM within {}s loading \
                 {url}.\n--- what it had written, if anything ---\n{so_far}\n\
                 --- what it said on stderr ---\n{}",
                BROWSER_DEADLINE.as_secs(),
                complained(&complaints),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };

    // It has done everything it was asked for. Whether it would ever have
    // exited on its own is not this test's question.
    let _ = child.kill();
    let _ = child.wait();
    dom
}

/// Whatever the browser wrote to stderr, trimmed to the end.
///
/// Chrome is voluble about things nobody asked about, and the useful line
/// is the last one. An empty result is worth saying out loud rather than
/// printing as blank space, because "it said nothing" is itself a clue.
fn complained(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let text = text.trim();
    if text.is_empty() {
        return "(nothing)".to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines.len().saturating_sub(20);
    lines[tail..].join("\n")
}

/// **A built program renders in a browser, not only in the shim.**
///
/// `counter.zd` is the smallest program with a view, a signal and a
/// handler, so a page that renders it has parsed the template, walked to
/// the nodes the emitter named, and attached the runtime. #205's failure
/// mode — a walk that names a node the parser did not build — stops module
/// evaluation, and the body stays empty.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_built_program_renders_in_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-counter");
    let built = build(&example("counter.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let profile = TempDir::new("browser-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    // The page is a shell plus a module. Rendering is the only thing that
    // distinguishes "the runtime attached" from "the script threw", and
    // the exception a bad walk raises does the latter silently.
    assert!(
        dom.contains("<button"),
        "the view did not render — the module threw before attaching. \
         This is the shape #205 had, and the shim cannot see it.\n\
         --- dumped DOM ---\n{dom}"
    );
    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
}

/// **An `each` of `<path>` really is in the SVG namespace.**
///
/// This is the one claim in the vector vocabulary that no shim can
/// settle, and getting it wrong is invisible in every other way. The
/// HTML parser decides an element's namespace from where it appears, so
/// a fragment that is nothing but `<path d="…">` — which is exactly what
/// `each` clones per ring — parses into an *HTML* element called `path`.
/// It has no geometry, it paints nothing, and it serialises identically
/// to the real thing: a DOM dump cannot tell the two apart, `outerHTML`
/// cannot, and no diagnostic anywhere fires. The drawing is simply
/// missing.
///
/// So the page asks the browser directly and writes the answer into the
/// document, which is the only form a dumped DOM can carry. `bare` is
/// asserted false as well as `flagged` true: a test that only checked the
/// fix would keep passing if `template` started namespacing everything,
/// and that would break every ordinary program on the same line.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_row_of_svg_is_namespaced_by_the_only_thing_that_knows() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-svg-namespace");
    // Built rather than hand-copied so the module under test is the one a
    // release actually ships, `// $dev` blocks and all.
    let built = build(&example("counter.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before its runtime can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // `counter.zd` draws nothing, so the build has no reason to write
    // `vector.js` — which is the point of the module and is also why the
    // probe has to be handed a copy.
    std::fs::write(out.path.join("runtime/vector.js"), zdc_runtime::VECTOR_JS)
        .expect("the vector module");

    std::fs::write(
        out.path.join("index.html"),
        r#"<!doctype html><html><body><div id="out"></div>
<script type="module">
import { template } from './runtime/dom.js';
import { templateSvg } from './runtime/vector.js';
const NS = 'http://www.w3.org/2000/svg';
const bare = template('<path d="M0 0L1 1"></path>')();
const flagged = templateSvg('<path d="M0 0L1 1"></path>')();
const whole = templateSvg('<svg viewBox="0 0 1 1"><circle cx="1" cy="1" r="1"></circle></svg>')();
document.getElementById('out').textContent = JSON.stringify({
  bare: bare.firstChild.namespaceURI === NS,
  flagged: flagged.firstChild.namespaceURI === NS,
  isPath: flagged.firstChild instanceof SVGPathElement,
  wholeChild: whole.firstChild.firstChild.namespaceURI === NS,
});
</script></body></html>"#,
    )
    .expect("the probe page");

    let profile = TempDir::new("browser-svg-namespace-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    let expected = r#"{"bare":false,"flagged":true,"isPath":true,"wholeChild":true}"#;
    assert!(
        dom.contains(expected),
        "the namespace answers were not {expected}.\n--- dumped DOM ---\n{dom}"
    );
}

/// **A program with a list ships its reconciler and renders it.**
///
/// `runtime/list.js` is linked only by a program that emits an `eachInto`
/// (#207), and the whole of that decision is a build writing one more file
/// and a module importing it by name. Either half being wrong is invisible
/// until a browser resolves the import graph: an unresolved specifier is
/// not a diagnostic, it is a body that stays empty. No suite against the
/// embedded engine can see it either, because that engine has no module
/// loader — every harness in this workspace flattens the runtime files
/// into one scope, so a bundle that failed to *link* one still runs there.
///
/// `todo.zd` is the subject because it is the example whose view is a
/// list: two rows from a list literal, so a page that renders them has
/// fetched `list.js`, resolved the import and run the reconciler.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_program_with_a_list_links_its_reconciler_and_renders_it() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-list");
    let built = build(&example("todo.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // Checked before the browser runs, so that "the page is empty" and
    // "the file was never written" are different failures with different
    // messages rather than one confusing one.
    assert!(
        out.path.join("runtime/list.js").exists(),
        "a program with an `each` must ship `runtime/list.js`; the build wrote {:?}",
        std::fs::read_dir(out.path.join("runtime"))
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );

    let profile = TempDir::new("browser-list-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    for row in ["write the parser", "write the checker"] {
        assert!(
            dom.contains(row),
            "the list did not render `{row}` — the module threw before attaching, \
             which is what an unresolved `./runtime/list.js` looks like.\n\
             --- dumped DOM ---\n{dom}"
        );
    }
    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
}

/// **A clock signal actually ticks, in a real browser.**
///
/// `runtime/clock.test.js` proves disposal against a scheduler it
/// controls, which is the only way to prove it deterministically — but
/// that scheduler is one this repository wrote, and the whole point of
/// this file is that agreeing with our own shim is not evidence. Three
/// things here need a browser and are unreachable anywhere else:
///
/// * `runtime/clock.js` is linked only by a program with an `every` or an
///   `after`, and an unresolved module specifier is not a diagnostic — it
///   is a body that stays empty. The embedded engine has no module loader
///   and flattens every runtime file into one scope, so a bundle that
///   failed to *link* one still runs there. This is the hole
///   `a_program_with_a_list_links_its_reconciler_and_renders_it` exists to
///   close, for a second optional module.
/// * `requestAnimationFrame` is not in the shim at all. Whether a frame
///   loop starts and reschedules itself is a question only something that
///   paints can answer.
/// * The virtual-time budget advances timers, so the dumped DOM is taken
///   *after* several seconds of scheduled work rather than at load. A
///   stopwatch reading zero in that dump is a timer that never fired.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_clock_signal_ticks_in_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-timers");
    let built = build(&example("timers.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // Checked before the browser runs, so "the page is empty" and "the
    // file was never written" stay two failures with two messages.
    assert!(
        out.path.join("runtime/clock.js").exists(),
        "a program with an `every` must ship `runtime/clock.js`; the build wrote {:?}",
        std::fs::read_dir(out.path.join("runtime"))
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );

    let profile = TempDir::new("browser-timers-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
    // The `after "2s"` arm. The virtual-time budget is six seconds, so a
    // two-second delay has fired by the time the DOM is dumped, and this
    // string is in the document only if it did.
    assert!(
        dom.contains("hello, two seconds late"),
        "the `after \"2s\"` signal never became true, so `setTimeout` never \
         reached the cell.\n--- dumped DOM ---\n{dom}"
    );
    // The frame loop. `requestAnimationFrame` writes `value` on the
    // `progress` element, which is emitted with no `value` attribute at
    // all — so its presence is the loop having run at least once.
    assert!(
        dom.contains("<progress") && dom.contains("value="),
        "the frame loop never wrote the progress bar, so \
         `requestAnimationFrame` never reached the cell.\n--- dumped DOM ---\n{dom}"
    );
    // The interval, read through a `from`: `elapsed` is milliseconds and
    // `wholeSeconds` divides it, so a non-zero reading means the 100 ms
    // timer fired at least ten times *and* the derivation ran.
    let ticked = dom
        .split("elapsed: </span><span>")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .and_then(|digits| digits.trim().parse::<u32>().ok());
    assert!(
        ticked.is_some_and(|seconds| seconds >= 1),
        "the stopwatch read {ticked:?} seconds, so the interval did not \
         run.\n--- dumped DOM ---\n{dom}"
    );
}

/// **The offset walk survives a real HTML parse, for every shape the
/// vocabulary can express.**
///
/// `elements.rs` keeps a list of tags a parser was seen to leave alone,
/// and the unit test beside it holds new elements to that list. What the
/// list cannot do is notice that a *browser* changed its mind, because it
/// records an observation rather than making one. This makes the
/// observation again, in CI, against whatever browser is installed.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_paragraph_holding_a_block_is_still_the_shape_the_refusal_is_written_for() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-parse-probe");
    std::fs::create_dir_all(&out.path).expect("the probe directory");
    // Built by hand rather than by `zdc build`, because the compiler now
    // refuses the program that produces this markup (#205) — which is the
    // point. The claim under test is about the parser, and it has to stay
    // checkable after the emitter stopped being able to emit it.
    std::fs::write(
        out.path.join("index.html"),
        r#"<!doctype html><meta charset="utf-8"><body><pre id="out"></pre><script>
const el = document.createElement('template');
el.innerHTML = '<p><div class="zd-col"><span>x</span></div></p>';
const names = [...el.content.childNodes].map(n => n.nodeName).join(',');
const walked = (() => {
  try {
    const n0 = el.content.firstChild;
    return 'reached ' + n0.firstChild.firstChild.nodeName;
  } catch (e) { return 'threw ' + e.constructor.name; }
})();
document.getElementById('out').textContent = names + ' | ' + walked;
</script></body>"#,
    )
    .expect("the probe page");

    let profile = TempDir::new("browser-parse-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    assert!(
        dom.contains("P,DIV,P"),
        "a browser no longer splits a paragraph around a block child. \
         `Paragraph`'s content model in `elements.rs` was written for that \
         behaviour and should be revisited:\n{dom}"
    );
    assert!(
        dom.contains("threw TypeError"),
        "the walk this refusal exists to prevent no longer throws; \
         the refusal's justification has changed:\n{dom}"
    );
}

/// The two typed fields, rendered by a real browser (#45, #48).
///
/// `element_parity.rs` compares the compiled template against the tree
/// `elements.js` builds, and `vocabulary.rs` drives both controls in the
/// shim. Neither is a browser, and both of these elements are `input`
/// elements whose *type attribute* changes what the browser does with the
/// value — which is the one thing a shim cannot inherit.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn the_typed_fields_render_in_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-booking");
    let built = build(&example("booking.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let profile = TempDir::new("browser-booking-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    for expected in ["type=\"number\"", "type=\"date\""] {
        assert!(
            dom.contains(expected),
            "`{expected}` did not reach the page — the module threw before \
             attaching, or the element lowered to something else.\n\
             --- dumped DOM ---\n{dom}"
        );
    }
    // Both signals start `None`, and `None` is an empty box rather than a
    // zero or the epoch.
    assert!(
        dom.contains("Say how many are coming.") && dom.contains("Pick a day."),
        "the empty arms did not render, so the starting `None` did not \
         reach the view:\n{dom}"
    );
    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
}

/// The browser behaviour `Slot::OptionalLevel` is designed around, asked
/// of the only authority that can answer it.
///
/// Two claims hold the design up, and neither is checkable in the shim,
/// which stores whatever text it is given:
///
///  1. A `number` field runs HTML's **value sanitisation**, so `value` is
///     the empty string while a reader is part way through `1.` or `-`.
///     That is why the binding compares `valueAsNumber` and not `value`:
///     comparing the text would rewrite the box on every keystroke and a
///     decimal point could never be typed at all.
///  2. A `date` field's `valueAsNumber` **is** a moment — milliseconds to
///     midnight UTC on the chosen day — in both directions. That is what
///     lets `DateInput` bind the type `prelude/time.zd` already has
///     instead of a `Date` type the language does not have, and it is why
///     nothing in this compiler formats a date.
///
/// Written as a probe page rather than as a driven program because the
/// harness loads a page and dumps its DOM; there is no keyboard here. The
/// claims are about the control, so the control is what is asked.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_numeric_field_reports_the_value_as_a_number_the_way_the_binding_assumes() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-numeric-probe");
    std::fs::create_dir_all(&out.path).expect("the probe directory");
    std::fs::write(
        out.path.join("index.html"),
        r#"<!doctype html><meta charset="utf-8"><body><pre id="out"></pre><script>
const n = document.createElement('input');
n.type = 'number';
const d = document.createElement('input');
d.type = 'date';
const said = [];
// 1. Value sanitisation: a part-typed number has no `value` at all.
n.value = '1.';
said.push('partial-value=' + JSON.stringify(n.value));
said.push('partial-number=' + (Number.isNaN(n.valueAsNumber) ? 'NaN' : n.valueAsNumber));
n.value = '-';
said.push('sign-value=' + JSON.stringify(n.value));
// The number goes back in through the same property it came out of.
n.valueAsNumber = 1.5;
said.push('written=' + JSON.stringify(n.value));
n.valueAsNumber = NaN;
said.push('cleared=' + JSON.stringify(n.value));
// 2. A date field's number is the moment, both ways. 1709164800000 is
// 2024-02-29T00:00:00Z.
d.valueAsNumber = 1709164800000;
said.push('day=' + JSON.stringify(d.value));
d.value = '2024-02-29';
said.push('moment=' + d.valueAsNumber);
d.valueAsNumber = NaN;
said.push('day-cleared=' + JSON.stringify(d.value));
document.getElementById('out').textContent = said.join(' | ');
</script></body>"#,
    )
    .expect("the probe page");

    let profile = TempDir::new("browser-numeric-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    for (claim, expected) in [
        // If `value` ever starts holding the partial text, binding it
        // would still be wrong — but for a different reason, and this
        // element's design note would need rewriting.
        ("a part-typed number has no `value`", "partial-value=\"\""),
        ("a part-typed number has no number", "partial-number=NaN"),
        ("a lone sign has no `value`", "sign-value=\"\""),
        ("a number written back reaches the box", "written=\"1.5\""),
        ("`NaN` empties the box", "cleared=\"\""),
        ("a moment renders as its UTC day", "day=\"2024-02-29\""),
        ("a UTC day reads back as its moment", "moment=1709164800000"),
        ("`NaN` empties a date box", "day-cleared=\"\""),
    ] {
        assert!(
            dom.contains(expected),
            "{claim}: expected `{expected}` in the probe output. The \
             browser contract `Slot::OptionalLevel` is written against has \
             changed, and the two numeric fields must be revisited.\n{dom}"
        );
    }
}

/// **A `remembered` value survives a reload, in a real browser.**
///
/// The whole of the `remembered` placement is a claim about what happens
/// *between two page loads*, and no unit test can make that claim: the
/// embedded engine has no `localStorage`, no origin and no second load, so
/// every harness in this workspace would pass on a runtime that wrote
/// nothing at all. `remembered.js` even degrades to a plain `signal` where
/// there is no store, which is right for the shim and means the shim can
/// never tell the difference.
///
/// So this drives one browser through a write and a reload in a single
/// session, which is what makes them the same browser and the same origin:
///
/// 1. The first load runs a driver page that imports the emitted
///    `client.js`, mounts it, and clicks the button — a write through the
///    compiler's own emitted setter, not through a hand-written
///    `setItem`. Nothing is asserted about it beyond that it happened.
/// 2. The second load is `index.html`, untouched, exactly as a visitor
///    would return to it.
///
/// The second load showing `1` is the feature. A `client` signal shows `0`
/// there, and so does a `remembered` one whose runtime failed to write, to
/// read, or to encode — which is why the assertion is on the rendered
/// value rather than on the presence of a key.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_remembered_value_survives_a_reload_in_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-remembered");
    let built = build(&example("preferences.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    // The write half. It mounts the real module and clicks the real
    // button, so the value goes into the store through `remembered.js`'s
    // setter and `wire.js`'s encoder — the two things this test is here to
    // exercise. A hand-written `localStorage.setItem` here would be a test
    // of the reader alone, and would pass against a writer that never ran.
    std::fs::write(
        out.path.join("write-once.html"),
        "<!doctype html><meta charset=\"utf-8\"><div id=\"app\"></div>\
         <script type=\"module\">\
         import { main } from './client.js';\
         main(document.getElementById('app'));\
         document.querySelector('button').click();\
         location.replace('./');\
         </script>\n",
    )
    .expect("a driver page beside the bundle");

    // **One browser, and the reload happens inside it.** The driver page
    // clicks and then navigates to the program's own document, so the
    // value is written and read back in a single session.
    //
    // Two separate browser runs against one profile is the more obvious
    // shape and it is not reliable: `localStorage` is flushed to disk
    // asynchronously, and `rendered` kills the browser as soon as the DOM
    // is dumped rather than waiting for an exit that may never come — so
    // the first run's write can be lost before the second run reads. That
    // passed on macOS and failed on the Linux runner, which is the signature
    // of a race rather than a rule.
    //
    // A reload within one session is what a visitor does when they refresh,
    // and it exercises the same setter, encoder and initialiser. What it no
    // longer proves on its own is that the browser persists across a full
    // restart — that is the browser's guarantee, not this language's.
    let profile = TempDir::new("browser-remembered-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");

    let (address, server) = serve(out.path.clone());
    let returned = rendered(
        &browser,
        &format!("http://{address}/write-once.html"),
        &profile.path,
    );
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    // The first load has to have rendered, or the click went nowhere and
    // the second load is being asked about a write that never happened.
    assert!(
        returned.contains("<button"),
        "the driver page did not mount the program, so nothing was written \
         and the reload proves nothing.\n--- dumped DOM ---\n{returned}"
    );
    assert!(
        returned.contains("<span>1</span>"),
        "`visits` came back as something other than 1 after a reload in the \
         same browser profile. The value did not survive: either the setter \
         did not write the store, the initialiser did not read it, or the \
         wire format did not round-trip it.\n--- dumped DOM ---\n{returned}"
    );
}

const ARIA_PROBE: &str = "state chosen is client Whole starting 0\n\
                          \n\
                          view\n\
                          \x20   Row role is \"tablist\"\n\
                          \x20       Button \"Issues\", role is \"tab\", selected is chosen is 0\n\
                          \x20       Button \"Activity\", role is \"tab\", selected is chosen is 1\n\
                          \x20       Button \"Previous\", disabled is yes, disabledColor is \"grey\"\n";

/// **A bound ARIA state reaches a real browser as a word, and the rule
/// that selects an unavailable control is a selector a real browser
/// parses.**
///
/// Both halves fail silently and neither is reachable from the shim.
///
/// The state is the `rotate` lesson again. `dom.js`'s `setAttribute`
/// implements HTML's boolean attributes, so a bound `false` would *remove*
/// `aria-selected` rather than write the word — and a tablist in which no
/// tab says `false` is announced as one with nothing chosen while
/// rendering identically. `runtime/dom-shim.js` stores whatever it is
/// handed, so it would agree with either behaviour; only an accessibility
/// tree can tell them apart, and the attribute is the closest thing to one
/// a dumped DOM has.
///
/// The selector is the sharper of the two. `disabledColor` folds into
/// `.zd-s0:is(:disabled,[aria-disabled="true"])`, and a browser that
/// cannot parse a selector **drops the whole rule** — the declaration
/// applies to nothing, nothing is logged, and the control is simply not
/// grey. The shim parses no CSS at all and has no computed style to ask.
/// So this asks: the announced-unavailable button must be grey and the tab
/// beside it must not be.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_bound_aria_state_and_the_rule_that_selects_it_survive_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let project = TempDir::new("browser-aria-src");
    std::fs::create_dir_all(&project.path).expect("the project directory");
    let source = project.path.join("aria.zd");
    std::fs::write(&source, ARIA_PROBE).expect("the probe program");

    let out = TempDir::new("browser-aria");
    let built = build(&source, &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the probe must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    // Appended rather than injected into the emitted document: module
    // scripts run in order, so this one sees the tree the program's own
    // module attached. It writes its verdict into the DOM because the
    // harness reads a dumped document and has no console.
    // A file beside the bundle, not an inline `<script>`. The emitted
    // document carries a Content Security Policy with `script-src 'self'`
    // (#146), so an inline probe is blocked and writes nothing — the
    // verdict comes back empty and the assertion below reports a missing
    // attribute that is in fact present. The policy is right; injecting
    // inline script was the part that had to change.
    std::fs::write(
        out.path.join("probe.js"),
        r#"const said = [];
const tabs = [...document.querySelectorAll('[role="tab"]')];
said.push('selected=' + tabs.map((t) => t.getAttribute('aria-selected')).join(','));
const off = document.querySelector('[aria-disabled="true"]');
said.push('disabled=' + (off === null ? 'missing' : getComputedStyle(off).color));
said.push('tab=' + (tabs.length === 0 ? 'missing' : getComputedStyle(tabs[0]).color));
document.getElementById('verdict').textContent = said.join(' | ');
"#,
    )
    .expect("the probe module");
    let page = out.path.join("index.html");
    let document = std::fs::read_to_string(&page).expect("the emitted document");
    std::fs::write(
        &page,
        document.replace(
            "</body>",
            r#"<pre id="verdict"></pre><script type="module" src="./probe.js"></script></body>"#,
        ),
    )
    .expect("the probe page");

    let profile = TempDir::new("browser-aria-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    // The word, in both positions. `selected=true,` alone would pass for a
    // runtime that removed the attribute on `false`, which is exactly the
    // failure this is here to catch.
    assert!(
        dom.contains("selected=true,false"),
        "a bound ARIA state must reach the browser as the word `true` and the word `false`. \
         An unselected tab carrying no `aria-selected` announces a tablist with nothing \
         chosen.\n--- dumped DOM ---\n{dom}"
    );
    // `grey` is `rgb(128, 128, 128)`. A dropped rule leaves the browser's
    // own button colour, which is not that on any engine.
    assert!(
        dom.contains("disabled=rgb(128, 128, 128)"),
        "the folded `disabled` rule did not apply. A selector a browser cannot parse takes \
         the whole rule with it, silently.\n--- dumped DOM ---\n{dom}"
    );
    assert!(
        !dom.contains("tab=rgb(128, 128, 128)"),
        "the rule applied to a control that is not disabled, so it is selecting more than \
         it names.\n--- dumped DOM ---\n{dom}"
    );
}

/// **`keys.zd` loads, and the module a shortcut needs resolves.**
///
/// `runtime/keys.js` is linked only by a program that writes `on key`, and
/// a module specifier that does not resolve is not a compile error — it is
/// a page that renders nothing, because the exception stops module
/// evaluation before the view is attached. Every other suite in this
/// workspace evaluates the runtime flattened into one scope, so none of
/// them can see that. This can.
///
/// What it deliberately does not do is press a key: `rendered` is a
/// one-shot `--dump-dom` load with no way to drive input, and inventing
/// one here would be a second browser harness. The keystroke path is
/// covered three other ways — the shim suite in `zdc-runtime`, the emitted
/// module in `zdc-codegen`'s `document_keys.rs`, and by hand in a real
/// browser, written up in the pull request.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_program_with_a_document_key_links_its_module_and_renders() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let out = TempDir::new("browser-keys");
    let built = build(&example("keys.zd"), &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the example must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // Before the browser runs, so "the page is empty" and "the file was
    // never written" stay two failures with two messages.
    assert!(
        out.path.join("runtime/keys.js").exists(),
        "a program with an `on key` must ship `runtime/keys.js`; the build wrote {:?}",
        std::fs::read_dir(out.path.join("runtime"))
            .map(|entries| entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );

    let profile = TempDir::new("browser-keys-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    assert!(
        dom.contains("Document keys"),
        "the view did not attach — the module threw before mounting, which is \
         what an unresolved `./runtime/keys.js` looks like.\n\
         --- dumped DOM ---\n{dom}"
    );
    assert!(
        dom.contains("cursor:"),
        "the bindings a key handler writes into are missing:\n{dom}"
    );
    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
}

const DIALOG_PROBE: &str = "state welcoming is client Truth starting yes\n\
                            state confirming is client Truth starting no\n\
                            \n\
                            view\n\
                            \x20   Column\n\
                            \x20       Button \"Delete\", id is \"opener\"\n\
                            \x20           on click\n\
                            \x20               set confirming to yes\n\
                            \x20       if confirming\n\
                            \x20           Text \"the program says open\"\n\
                            \x20       Dialog welcoming, label is \"Welcome\", id is \"welcome\"\n\
                            \x20           Text \"Hello\"\n\
                            \x20           Button \"Close\", id is \"dismiss\"\n\
                            \x20               on click\n\
                            \x20                   set welcoming to no\n\
                            \x20       Dialog confirming, label is \"Confirm deletion\", id is \
                            \"confirm\"\n\
                            \x20           Text \"Delete it?\"\n";

/// **A `Dialog` is a modal in a real browser: focus moves in, is trapped,
/// and comes back out to whatever opened it (#53).**
///
/// Every claim `Slot::Open` rests on belongs to `showModal()`, and not one
/// of them is checkable anywhere else in this workspace. `dom-shim.js`
/// models the open/closed state machine — that much is driven in
/// `zdc-codegen/tests/vocabulary.rs` — and models no focus, no top layer,
/// no `inert` and no close request, so a binding that opened the dialog
/// with an `open` attribute instead would pass every other suite while
/// rendering a box that traps nobody. Five things are asked here:
///
///  1. **The deferred opening really opens.** `welcoming` starts `yes`,
///     and every binding this compiler emits runs while the tree is still
///     a clone of a `<template>`, where `showModal()` throws. Only the
///     microtask deferral makes this dialog open at all, and only a
///     browser has the distinction it turns on.
///  2. **It is modal, not merely shown.** `:modal` matches a dialog opened
///     with `showModal()` and nothing else; a hand-written `open`
///     attribute renders the same box and matches nothing.
///  3. **Focus is inside it, and the page behind it is inert.** Focusing
///     the button underneath is attempted and must fail.
///  4. **Focus returns to whatever opened it.** This is the half a
///     hand-rolled modal forgets. HTML's "close the dialog" steps do it
///     from the element's own record of the previously focused element,
///     which is why the probe focuses the opener before clicking it — a
///     scripted `click()` moves no focus, and a person's does.
///  5. **Closing writes back, so the opener still works.** Escape's close
///     request is a user-agent action no script can forge; `close()` is
///     the step it performs and the same step Escape reaches, so the
///     write-back path is the one under test either way.
///
/// The parser is asked at the same time, because `elements.rs`'s
/// `PARSER_SAFE_TAGS` records an observation rather than making one, and
/// `dialog` is new to that list.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn a_dialog_is_a_modal_with_a_focus_trap_in_a_real_browser() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let project = TempDir::new("browser-dialog-src");
    std::fs::create_dir_all(&project.path).expect("the project directory");
    let source = project.path.join("dialog.zd");
    std::fs::write(&source, DIALOG_PROBE).expect("the probe program");

    let out = TempDir::new("browser-dialog");
    let built = build(&source, &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the probe must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    // A file beside the bundle rather than an inline `<script>`: the
    // emitted document carries `script-src 'self'` (#146), so an inline
    // probe is blocked and its verdict comes back empty.
    //
    // The first step is deferred once, because the opening of `welcome` is
    // queued as a *microtask* by the module that mounted the view and a
    // module script may share a task with this one. The virtual-time
    // budget advances the timer.
    //
    // Everything after that is driven by the dialog's own `close` event
    // and not by a second timer, and that is a correction rather than a
    // preference. HTML queues `close` on a task source; a `setTimeout(…,
    // 0)` is another task with no ordering against it, and the first
    // version of this probe read the write-back from one — which passed
    // once and then failed, having asked before the event was delivered.
    // The listener is registered after the program's own, so listener
    // order puts it after the write-back and after the effects that write
    // flushed. Nothing about the element races; the probe did.
    std::fs::write(
        out.path.join("probe.js"),
        r#"const said = [];
const at = (id) => document.getElementById(id);

setTimeout(() => {
  // The parser measurement `PARSER_SAFE_TAGS` records: a `<dialog>` must
  // not have a block child moved, or the emitted offset walk names a node
  // the browser did not put there (#205).
  const shape = document.createElement('template');
  shape.innerHTML = '<dialog><div><span>x</span></div></dialog>';
  said.push('nesting=' + ((shape.content.firstChild.firstChild || {}).nodeName || 'none'));

  const welcome = at('welcome');
  const modal = at('confirm');
  const opener = at('opener');

  // 1 and 2: a dialog whose signal starts `yes` is open, and it is modal.
  said.push('welcome=' + welcome.open + ',' + welcome.matches(':modal'));
  // 3: focus is inside it, and the page behind it cannot take focus back.
  said.push('welcome-focus=' + welcome.contains(document.activeElement));
  opener.focus();
  said.push(
    'behind-inert=' +
      (document.activeElement !== opener && welcome.contains(document.activeElement))
  );

  welcome.addEventListener('close', () => {
    said.push('welcome-closed=' + welcome.open);

    // A person who activates a button has focused it; a scripted click
    // does not, and what focus returns to is what had it when the dialog
    // opened.
    opener.focus();
    opener.click();
    said.push('confirm=' + modal.open + ',' + modal.matches(':modal'));
    said.push('confirm-focus=' + modal.contains(document.activeElement));

    // 5: the program learned about the dismissal, and the opener opens it
    // again. Read here rather than after a timer, because this listener
    // runs after the program's own and therefore after the write-back.
    modal.addEventListener('close', () => {
      said.push('wrote-back=' + !document.body.textContent.includes('the program says open'));
      opener.click();
      said.push('reopened=' + modal.open);
      at('verdict').textContent = said.join(' | ');
    }, { once: true });

    // 4: what Escape ends in, and focus comes back out synchronously —
    // only the event is queued.
    modal.close();
    said.push('returned=' + (document.activeElement === opener));
  }, { once: true });

  at('dismiss').click();
}, 0);
"#,
    )
    .expect("the probe module");
    let page = out.path.join("index.html");
    let document = std::fs::read_to_string(&page).expect("the emitted document");
    std::fs::write(
        &page,
        document.replace(
            "</body>",
            r#"<pre id="verdict"></pre><script type="module" src="./probe.js"></script></body>"#,
        ),
    )
    .expect("the probe page");

    let profile = TempDir::new("browser-dialog-profile");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");
    let (address, server) = serve(out.path.clone());
    let dom = rendered(&browser, &format!("http://{address}/"), &profile.path);
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    assert!(
        !dom.contains("zd-error"),
        "the runtime reported an error into the page:\n{dom}"
    );
    for (claim, expected) in [
        (
            "a `<dialog>` keeps its block children, so the offset walk survives the parse; \
             `PARSER_SAFE_TAGS` in `elements.rs` records this measurement",
            "nesting=DIV",
        ),
        (
            "a dialog whose signal starts `yes` opens once the tree is in the document. \
             `showModal()` throws on a detached node, and every binding runs before `mount`, \
             so this is the deferral in `$modal` working — or not",
            "welcome=true,true",
        ),
        (
            "focus moves into the dialog when it opens",
            "welcome-focus=true",
        ),
        (
            "the page behind a modal is inert, so focus cannot be taken back out of it",
            "behind-inert=true",
        ),
        (
            "writing the signal `no` closes the dialog",
            "welcome-closed=false",
        ),
        (
            "writing the signal `yes` opens a modal, not a merely-shown dialog",
            "confirm=true,true",
        ),
        (
            "focus moves in on every opening, not only the first",
            "confirm-focus=true",
        ),
        (
            "focus returns to whatever opened the dialog. This is the half everyone forgets, \
             and it is `showModal()`'s own record of the previously focused element",
            "returned=true",
        ),
        (
            "closing writes the signal back, so the program and the DOM agree about whether \
             the modal is showing",
            "wrote-back=true",
        ),
        (
            "and the button that opened it opens it again. Without the write-back this is \
             where a modal dies: the signal is still `yes`, the effect sees no change, and \
             nothing happens",
            "reopened=true",
        ),
    ] {
        assert!(
            dom.contains(expected),
            "{claim}: expected `{expected}` in the probe output.\n--- dumped DOM ---\n{dom}"
        );
    }
}

const ANIMATION_PROBE: &str = "view\n\
                               \x20   Column\n\
                               \x20       Text \"turning\", id is \"mover\", animation is \"20s\", \
                               repeat is \"forever\", fromRotate is 0, toRotate is 360\n";

/// **An animation runs in a real browser, and stops existing for a reader
/// who asked for less motion.**
///
/// Three things fail silently here and not one of them is reachable from
/// the shim, which parses no CSS and has no computed style to ask.
///
/// * **The `@keyframes` block is a top-level at-rule inside an `@media`
///   block.** If an engine did not accept that nesting it would drop the
///   block, `animation-name` would name nothing, and the element would sit
///   perfectly still with nothing logged anywhere.
/// * **The name has to resolve.** `animation-name: zd-k0` computes to
///   `zd-k0` whether or not a block by that name exists, so a computed
///   style alone cannot tell a working animation from a dangling
///   reference. `document.getAnimations()` can: an animation whose
///   keyframes do not resolve produces no `Animation` object at all.
/// * **`prefers-reduced-motion` is a property of the reader**, so the only
///   authority on whether the query works is a browser told to report the
///   preference. That is the second load.
///
/// The animation repeats forever on purpose. `--virtual-time-budget`
/// races through the page's clock, and a finite animation would have
/// finished before the DOM was dumped; a repeating one is running
/// whenever the probe asks.
#[test]
#[ignore = "needs a real browser; the `browser` CI job runs it with --ignored"]
fn an_animation_runs_in_a_real_browser_and_not_for_a_reader_who_asked_it_not_to() {
    let Some(browser) = browser() else {
        panic!(
            "no browser found. Set `ZDC_BROWSER`, or install one of: {}",
            BROWSERS.join(", ")
        )
    };

    let project = TempDir::new("browser-animation-src");
    std::fs::create_dir_all(&project.path).expect("the project directory");
    let source = project.path.join("animation.zd");
    std::fs::write(&source, ANIMATION_PROBE).expect("the probe program");

    let out = TempDir::new("browser-animation");
    let built = build(&source, &out.path);
    assert_eq!(
        built.status.code(),
        Some(0),
        "the probe must build before it can be loaded: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    // A file beside the bundle rather than an inline `<script>`: the
    // emitted document carries `script-src 'self'` (#146), which blocks
    // an inline probe and would leave the verdict empty.
    std::fs::write(
        out.path.join("probe.js"),
        r#"const said = [];
const asked = matchMedia('(prefers-reduced-motion: reduce)').matches;
said.push('motion=' + (asked ? 'reduce' : 'no-preference'));
const el = document.getElementById('mover');
said.push('name=' + (el === null ? 'missing' : getComputedStyle(el).animationName));
said.push('running=' + document.getAnimations().length);
document.getElementById('verdict').textContent = said.join(' | ');
"#,
    )
    .expect("the probe module");
    let page = out.path.join("index.html");
    let document = std::fs::read_to_string(&page).expect("the emitted document");
    std::fs::write(
        &page,
        document.replace(
            "</body>",
            r#"<pre id="verdict"></pre><script type="module" src="./probe.js"></script></body>"#,
        ),
    )
    .expect("the probe page");

    let (address, server) = serve(out.path.clone());
    let moving = TempDir::new("browser-animation-profile");
    std::fs::create_dir_all(&moving.path).expect("a profile directory");
    let played = rendered(&browser, &format!("http://{address}/"), &moving.path);
    let still = TempDir::new("browser-animation-still-profile");
    std::fs::create_dir_all(&still.path).expect("a second profile directory");
    let refused = rendered_under(
        &browser,
        &format!("http://{address}/"),
        &still.path,
        &["--force-prefers-reduced-motion"],
    );
    let _ = std::net::TcpStream::connect(address).map(|mut stop| {
        use std::io::Write;
        let _ = stop.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    });
    let _ = server.join();

    assert!(
        !played.contains("name=missing"),
        "the probe never found the element, so nothing was learned about the \
         page.\n--- dumped DOM ---\n{played}"
    );
    // The first load only says anything if this browser reports the
    // default. A headless browser that reports `reduce` unasked is not a
    // failure of the compiler, and reading it as one would make this test
    // fail for the one reason it is meant to prove.
    if played.contains("motion=no-preference") {
        assert!(
            played.contains("name=zd-k0"),
            "the folded rule did not apply, so the element is not animated at \
             all.\n--- dumped DOM ---\n{played}"
        );
        assert!(
            !played.contains("running=0"),
            "the rule applied but no animation is running, which is what a \
             `@keyframes` block the engine dropped looks like: the name \
             resolves to nothing.\n--- dumped DOM ---\n{played}"
        );
    }
    // And the half that is the point. If the flag did not take, this
    // browser cannot answer the question and says so rather than passing
    // quietly.
    assert!(
        refused.contains("motion=reduce"),
        "`--force-prefers-reduced-motion` did not take on this browser, so \
         nothing was learned about the preference this compiler promises to \
         respect. Run the suite on a Chromium that honours it.\n\
         --- dumped DOM ---\n{refused}"
    );
    assert!(
        refused.contains("name=none"),
        "an animation reached a reader who asked their system for less \
         motion. Every declaration and the block itself must be inside \
         `prefers-reduced-motion: no-preference`.\n--- dumped DOM ---\n{refused}"
    );
    assert!(
        refused.contains("running=0"),
        "the page is still animating for a reader who asked it not \
         to.\n--- dumped DOM ---\n{refused}"
    );
}

/// **`zdc dev` can be driven from a browser**, which #281 doubted and no
/// test spoke for.
///
/// # Why this test observes the server and not the page
///
/// Every other test in this file reads `--dump-dom`, and that cannot work
/// here. `--dump-dom` serialises when the page is *done*, and a `zdc dev`
/// page holds an `EventSource` open — a request that by design never
/// completes — so the budget is never spent and the dump never arrives.
/// The test then fails at its deadline looking exactly like a page that
/// rendered nothing, which is the symptom #281 reported and the reason it
/// was believed.
///
/// So this asks the server what it saw instead. A browser appears in
/// `Handle::subscribers` only after it has fetched the document, parsed
/// it, fetched `/__zdc/live.js` as a module, and run it far enough to open
/// the stream. A page that hangs before `load` — the reported failure —
/// never gets there, so the count is a proxy for the whole chain and one
/// that needs no driver.
///
/// The server runs in this process rather than as a `zdc dev` subprocess
/// precisely so that `subscribers` is reachable. What that gives up is the
/// CLI's own argument handling, which `zdc-dev`'s suite already covers.
#[test]
#[ignore = "needs a browser; the `browser` CI job runs it"]
fn a_browser_can_load_and_live_reload_a_page_zdc_dev_is_serving() {
    let Some(browser) = browser() else {
        return;
    };
    let profile = TempDir::new("dev-server");
    std::fs::create_dir_all(&profile.path).expect("a profile directory");

    let site = zdc_dev::build_once(&example("counter.zd"), &zdc_dev::Settings::default());
    assert!(site.is_ready(), "the fixture must compile");
    let server = std::sync::Arc::new(
        zdc_dev::DevServer::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), site)
            .expect("could not bind an ephemeral port"),
    );
    let url = format!("http://{}/", server.local_addr());
    let handle = server.handle();
    let serving = std::sync::Arc::clone(&server);
    std::thread::spawn(move || serving.serve());

    // No `--dump-dom` and no virtual-time budget: this browser is meant to
    // stay on the page rather than serialise it and leave, and virtual
    // time would run the reconnect backoff out in an instant.
    let child = Command::new(&browser)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-crash-reporter",
        ])
        .arg(format!("--user-data-dir={}", profile.path.display()))
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            std::fs::File::create(profile.path.join("browser.log"))
                .expect("a file for the browser's complaints"),
        ))
        .spawn()
        .expect("failed to launch the browser");

    // Killed on every path out, including a panicking assertion: an
    // orphaned headless Chrome is what made this job's first run cost
    // forty minutes.
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _guard = Kill(child);

    let until = |deadline: std::time::Duration, want: usize| {
        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            if handle.subscribers() >= want {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    };

    assert!(
        until(BROWSER_DEADLINE, 1),
        "no browser reached the reload stream within {BROWSER_DEADLINE:?}. The page \
         served by `zdc dev` did not get as far as running `/__zdc/live.js`, which is \
         issue #281's report. The browser's own complaints are in {}",
        profile.path.join("browser.log").display()
    );

    // Loading is weaker than the feature working. Publishing tells every
    // subscriber to reload, and a reload tears the stream down and opens a
    // new one — so the count returning is the browser having acted on what
    // it was sent, rather than merely having been sent it.
    let before = handle.generation();
    let generation = handle.publish(zdc_dev::build_once(
        &example("counter.zd"),
        &zdc_dev::Settings::default(),
    ));
    assert_eq!(
        generation,
        before + 1,
        "publishing advances the generation the reload frame carries"
    );
    assert!(
        until(BROWSER_DEADLINE, 1),
        "the browser never came back after a reload was published, so live reload \
         reaches a real browser only in principle"
    );
}
