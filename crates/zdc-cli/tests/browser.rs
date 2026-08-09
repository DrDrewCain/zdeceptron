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
//! server here is twenty lines and static, deliberately: `zdc dev` is a
//! watcher, and what a user deploys is the built directory, so the built
//! directory is what gets loaded.
//!
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

/// Serve `root` until the returned handle is dropped.
///
/// Static, single-threaded and just enough: the browser fetches the
/// document, one module graph and a stylesheet. The content types are
/// load-bearing rather than decorative — a browser refuses a module served
/// as anything but JavaScript, and the failure looks exactly like the bug
/// this file is hunting.
fn serve(root: PathBuf) -> (SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let Ok(read) = stream.read(&mut buffer) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let Some(target) = request.split_whitespace().nth(1) else {
                continue;
            };
            if target == "/__stop" {
                return;
            }
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
            // A served path may not climb: this is a test server, but a
            // test server that reads outside its root is a habit worth not
            // forming.
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
                        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n",
                        bytes.len()
                    )
                    .into_bytes();
                    head.extend_from_slice(&bytes);
                    head
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = std::io::Write::write_all(&mut stream, &response);
        }
    });
    (address, handle)
}

/// The DOM a real browser built, after scripts ran.
fn rendered(browser: &str, url: &str, profile: &Path) -> String {
    let output = Command::new(browser)
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
            "--dump-dom",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .stderr(Stdio::null())
        .output()
        .expect("failed to launch the browser");
    String::from_utf8_lossy(&output.stdout).into_owned()
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
