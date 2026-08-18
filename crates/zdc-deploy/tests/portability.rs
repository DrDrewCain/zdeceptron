//! The portability claim, asserted rather than asserted-to.
//!
//! Spec §8 says one artifact runs everywhere. The research this crate was
//! built from found that half true: ECMA-429 standardises the interior of a
//! handler and no entrypoint at all. These tests pin down which half is
//! which, so that if the portable half ever stops being portable the build
//! fails instead of the claim quietly becoming false.

mod support;

use boa_engine::{Context, Source};
use support::{compile_example, compile_source, deploy, file, program};
use zdc_deploy::{Options, Program, Target};

/// **The claim.** Compile one program, generate it for all four targets,
/// and diff the handler bodies.
///
/// If this ever fails, the portable core is not portable and that is the
/// most important thing anyone working on this crate could know.
#[test]
fn the_handler_bodies_are_byte_identical_on_every_target() {
    let bundle = compile_example("examples/guestbook.zd");
    assert!(
        !bundle.functions.is_empty(),
        "a program with no server functions cannot demonstrate anything"
    );

    let deployments: Vec<(Target, zdc_deploy::Deployment)> = Target::ALL
        .into_iter()
        .map(|target| {
            let program = program(&bundle);
            let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
                .unwrap_or_else(|refusal| panic!("{}: {}", target.slug(), refusal.message));
            (target, deployment)
        })
        .collect();

    for function in &bundle.functions {
        for (target, deployment) in &deployments {
            let emitted = file(deployment, &function.path);
            assert_eq!(
                emitted.contents,
                function.source,
                "{} rewrote {} instead of copying it. The handler body is the whole \
                 portability claim; a target that edits one has broken it.",
                target.slug(),
                function.path
            );
        }
    }

    // And every target ships the same set of them.
    let expected: Vec<&str> = bundle
        .functions
        .iter()
        .map(|function| function.path.as_str())
        .collect();
    for (target, deployment) in &deployments {
        let mut shipped: Vec<&str> = deployment
            .files
            .iter()
            .map(|file| file.path.as_str())
            .filter(|path| path.starts_with("functions/"))
            .collect();
        shipped.sort();
        let mut wanted = expected.clone();
        wanted.sort();
        assert_eq!(
            shipped,
            wanted,
            "{} ships a different endpoint set",
            target.slug()
        );
    }
}

/// The adapter layered on top of the handlers is portable too. Only the
/// entry and the store binding are allowed to differ.
#[test]
fn the_router_and_the_cell_helpers_are_byte_identical_on_every_target() {
    let bundle = compile_example("examples/guestbook.zd");
    assert_eq!(Target::ALL.len(), 4, "{:?}", Target::ALL);
    let mut seen: Vec<(Target, String, String)> = Vec::new();
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
            .expect("every target accepts guestbook.zd");
        seen.push((
            target,
            file(&deployment, "_zd/router.js").contents.clone(),
            file(&deployment, "_zd/cells.js").contents.clone(),
        ));
    }
    let (first, router, cells) = seen[0].clone();
    for (target, other_router, other_cells) in &seen[1..] {
        assert_eq!(
            *other_router,
            router,
            "{} and {} disagree about the router",
            first.slug(),
            target.slug()
        );
        assert_eq!(
            *other_cells,
            cells,
            "{} and {} disagree about the cell helpers",
            first.slug(),
            target.slug()
        );
    }
}

/// The router answers the paths the emitted client actually asks for.
///
/// There were two spellings once — this router routed `~watch`, and
/// `runtime/store.js` subscribed to `live` and polled `poll`. Both sides
/// worked in isolation and no test failed, because nothing compared them:
/// the client is Rust-emitted JavaScript and the router is a checked-in
/// asset, so the disagreement could only show up as a 404 in a browser.
/// `~watch` was retired; this is what keeps it retired.
#[test]
fn the_router_routes_the_transport_paths_the_client_runtime_requests() {
    let bundle = compile_example("examples/guestbook.zd");
    let program = program(&bundle);
    let deployment = zdc_deploy::generate(&program, &Options::new(Target::Cloudflare, "test-app"))
        .expect("cloudflare accepts guestbook.zd");
    let router = &file(&deployment, "_zd/router.js").contents;

    // `guestbook.zd` has a `durable` signal, so its bundle links the live-
    // sync half. If that ever stops being true the fixture is wrong, not
    // the router.
    let store_js = zdc_codegen::runtime_files(&bundle.runtime, zdc_codegen::Mode::Release)
        .into_iter()
        .find(|(path, _)| *path == "runtime/store.js")
        .expect("a durable program links store.js")
        .1;

    for path in ["/_zd/live?", "/_zd/poll?"] {
        assert!(
            store_js.contains(path),
            "the client no longer builds `{path}` — this test is now checking the wrong pair"
        );
    }
    for name in ["const LIVE = 'live'", "const POLL = 'poll'"] {
        assert!(
            router.contains(name),
            "the router does not declare `{name}`"
        );
    }
    assert!(
        !router.contains("~watch"),
        "`~watch` is the retired spelling; the client never asks for it"
    );
    // The event name is half the contract. `receive` in `store.js`
    // dispatches on it, so a router emitting `change` would deliver frames
    // that advance the cursor and update nothing.
    assert!(
        router.contains("event: update"),
        "the stream must send the `update` events `store.js` listens for"
    );
}

/// The portable half uses only the minimum common web API.
///
/// This is a grep, and it is worth saying what it can and cannot do: it
/// catches a Node built-in or a platform global creeping into the shared
/// file, which is the mistake that actually happens. It cannot prove
/// conformance, because there is no WinterTC conformance suite to run —
/// wintertc.org lists no test-suite work item and the GitHub organisation
/// has no such repository.
#[test]
fn the_portable_half_names_no_platform_api() {
    let bundle = compile_example("examples/guestbook.zd");
    let program = program(&bundle);
    let deployment = zdc_deploy::generate(&program, &Options::new(Target::Cloudflare, "test-app"))
        .expect("cloudflare accepts guestbook.zd");

    let forbidden = [
        "awslambda",
        "process.env",
        "Deno.",
        "require(",
        "node:",
        "Buffer",
        "cloudflare:",
        "__dirname",
    ];
    for path in [
        "_zd/router.js",
        "_zd/cells.js",
        "_zd/endpoints.js",
        "_zd/config.js",
    ] {
        let source = &file(&deployment, path).contents;
        for name in forbidden {
            assert!(
                !source.contains(name),
                "{path} names `{name}`, which is not in ECMA-429. The portable half has to \
                 stay portable; put it in the entry shim."
            );
        }
    }
}

/// The emitted handlers themselves name no platform API either — which is
/// what makes copying them rather than rewriting them possible.
#[test]
fn the_emitted_handlers_name_no_platform_api() {
    // `voting-board.zd` would be the interesting third case — it is the one
    // with a durable `Map` — but it does not compile yet: §16.3.6's leading
    // text slot for `Row` and `Column` is unratified and the emitter refuses
    // rather than inventing the semantics.
    let mut scanned = 0;
    for example in ["examples/guestbook.zd", "examples/todo.zd"] {
        let bundle = compile_example(example);
        for function in &bundle.functions {
            scanned += 1;
            for name in [
                "awslambda",
                "process.env",
                "Deno.",
                "require(",
                "node:",
                "import ",
            ] {
                assert!(
                    !function.source.contains(name),
                    "{example}: {} names `{name}`",
                    function.path
                );
            }
        }
    }
    // `guestbook.zd` alone emits a value endpoint and a command; a run
    // that read no handler at all would satisfy every loop above without
    // looking at a byte.
    assert!(scanned >= 2, "only {scanned} handlers were read");
}

/// Every generated JavaScript file parses as an ES module.
///
/// The shims are hand-written and copied verbatim, so nothing else would
/// notice a syntax error in them until a platform did. Parsing is all this
/// can do — the engine has no `Response`, no `ReadableStream` and no
/// `Deno`, so evaluation would prove only that the shim is unrunnable
/// here, which is already known.
#[test]
fn every_generated_module_parses() {
    let bundle = compile_example("examples/guestbook.zd");
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
            .expect("every target accepts guestbook.zd");
        let mut checked = 0;
        for generated in &deployment.files {
            if !generated.path.ends_with(".js") && !generated.path.ends_with(".mjs") {
                continue;
            }
            checked += 1;
            let mut context = Context::default();
            let source = Source::from_bytes(generated.contents.as_bytes());
            boa_engine::Module::parse(source, None, &mut context).unwrap_or_else(|error| {
                panic!(
                    "{}: {} does not parse: {error}",
                    target.slug(),
                    generated.path
                )
            });
        }
        assert!(
            checked >= 8,
            "{} generated only {checked} modules; the scan is not covering the adapter",
            target.slug()
        );
    }
}

/// The compiler's path argument and the adapters' decoder are one contract.
///
/// `cells.js` is the portable half of all four stores and the only place
/// the emitted call is taken apart, so a change to the emitted shape that
/// missed it would give every target the defect `zdc-host` had: an index
/// dropped, and the whole key overwritten. Both halves are asserted here so
/// neither can move alone.
#[test]
fn the_emitted_place_decodes_into_a_cell_and_a_field_path() {
    let bundle = compile_example("examples/voting-board.zd");
    let incr = bundle
        .functions
        .iter()
        .find(|function| function.name == "votes.incr.at")
        .expect("voting-board.zd writes through a path");
    assert!(
        incr.source.contains("[['at', $args[1]]], new Map()"),
        "the emitted place is not the one `cells.js` decodes:\n{}",
        incr.source
    );

    let (_, deployment) = deploy("examples/voting-board.zd", Target::Deno);
    let cells = file(&deployment, "_zd/cells.js")
        .contents
        .lines()
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");

    let mut context = Context::default();
    context
        .eval(Source::from_bytes(cells.as_bytes()))
        .expect("cells.js evaluates as a script");
    let decoded = context
        .eval(Source::from_bytes(
            // The argument list `$store.incr('votes', 1, [['at','ada']],
            // new Map())` hands `address`, which is everything after the
            // key. An index becomes the cell's subkey — that is what gives
            // DynamoDB and Deno KV a native atomic add on one candidate —
            // and a record field stays a path inside the cell.
            b"const $r = address([1, [['at', 'ada'], ['field', 'count']], new Map()]);\
              $r.value + '|' + $r.sub + '|' + $r.path.join(',')",
        ))
        .expect("address decodes the emitted place");
    assert_eq!(
        decoded
            .as_string()
            .expect("a string")
            .to_std_string_escaped(),
        "1|ada|count"
    );
}

/// A program with no crossing generates an empty endpoint table rather than
/// a broken one — the same principle as `hello.zd` shipping no `rpc.js`.
#[test]
fn a_program_with_no_server_work_still_generates_a_deployment() {
    let (bundle, deployment) = deploy("examples/hello.zd", Target::Cloudflare);
    assert!(bundle.functions.is_empty(), "hello.zd has no crossing");
    assert!(file(&deployment, "_zd/endpoints.js")
        .contents
        .contains("export const endpoints = {};"));
    assert!(bundle.durable.is_empty());
    assert!(bundle.environment.is_empty());
}

// --- the wire format's compatibility rule (#144) --------------------------

/// The minimum of the web platform `route` touches, as a script.
///
/// Boa has no Web APIs, so `URL`, `Response` and the request object are
/// stubbed here. They are deliberately thin: the point is to run the
/// router's *own* control flow, not to reimplement fetch. Anything the
/// router needs that is missing shows up as a `TypeError` from the eval
/// rather than as a passing test.
const WEB_SHIM: &str = r#"
// Constructor functions rather than `class`: this engine miscompiles a
// class declaration sitting in the same script as the router and aborts
// the process inside its own `define` opcode. `new` works the same way on
// either, and a shim is not the place to find out.
function URL(href) {
  const text = String(href);
  // The origin has to come off, or `pathname` is the whole URL and
  // `route`'s `startsWith('/_zd/')` is false for every request — which
  // looks exactly like a router that answers nothing.
  const scheme = text.indexOf('://');
  const after = scheme === -1 ? text : text.slice(scheme + 3);
  const slash = after.indexOf('/');
  const rest = slash === -1 ? '/' : after.slice(slash);
  const cut = rest.indexOf('?');
  this.pathname = cut === -1 ? rest : rest.slice(0, cut);
  const query = cut === -1 ? '' : rest.slice(cut + 1);
  const found = new Map();
  for (const pair of query.split('&')) {
    if (pair === '') continue;
    const at = pair.indexOf('=');
    const name = at === -1 ? pair : pair.slice(0, at);
    const value = at === -1 ? '' : pair.slice(at + 1);
    found.set(decodeURIComponent(name), decodeURIComponent(value));
  }
  this.searchParams = { get: (name) => (found.has(name) ? found.get(name) : null) };
}
function Response(body, init) {
  this.body = body;
  this.status = (init && init.status) || 200;
  // Named `headerBag` and not `headers`: a test asserting on `headers`
  // would pass against a shim that simply stored whatever it was given,
  // which is the same thing this shim does — the distinct name is a
  // reminder that this is the stub's field and not the platform's.
  this.headerBag = (init && init.headers) || {};
}
// A request that names `wire` as its format, or names none when it is null.
function requestFor(url, method, body, wire) {
  return {
    url,
    method,
    headers: { get: (name) => (name === 'zd-wire' && wire !== null ? wire : null) },
    json: () => Promise.resolve(body),
  };
}
const ENDPOINTS = {
  greeting: { handler: ({ name }) => 'Hello, ' + name + '.', inputs: ['name'], command: false },
};
globalThis.__ran = 0;
const COUNTING = {
  greeting: {
    handler: ({ name }) => { globalThis.__ran += 1; return 'Hello, ' + name + '.'; },
    inputs: ['name'],
    command: false,
  },
};
const STORE = { get: () => Promise.resolve(null) };
const CONFIG = { heartbeatSeconds: 1, idleSeconds: 1, maxStreamSeconds: 0, pollSeconds: 1 };
"#;

/// Evaluate `router.js` with the shim, run `script`, and report `answer`.
fn drive_router(script: &str, answer: &str) -> String {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Cloudflare);
    // `wire.js` first, because the router imports `stringify` from it
    // (#144). A module's `import` line is not something a script may
    // contain, so the file it names is inlined ahead of it and the line
    // itself is dropped — the same trick the shim already needed, applied
    // to the dependency the router grew.
    let script_of = |path: &str| {
        file(&deployment, path)
            .contents
            .lines()
            .filter(|line| !line.starts_with("import "))
            .map(|line| line.strip_prefix("export ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let wire = script_of("_zd/wire.js");
    let router = script_of("_zd/router.js");

    // One script, not four. The shim's `class` declarations and the
    // router's `const`s are lexical, so they are scoped to the script that
    // declares them — evaluating the case separately would not see either.
    let source = format!("{WEB_SHIM}\n{wire}\n{router}\n{script}");

    let mut context = Context::default();
    context
        .eval(Source::from_bytes(source.as_bytes()))
        .unwrap_or_else(|error| panic!("the router case failed to evaluate: {error}"));
    context.run_jobs().expect("the router settles");
    context
        .eval(Source::from_bytes(answer.as_bytes()))
        .expect("reading the answer")
        .to_string(&mut context)
        .expect("the answer is a string")
        .to_std_string_escaped()
}

/// **The deliberate mismatch, on the server every deployment runs.**
///
/// `zdc-dev` has the same test over a socket, and this is the one that
/// matters more: `zdc dev` faces one developer reloading a page, and this
/// file faces a browser that loaded before the deploy and is still open
/// after it. That is not an edge case — during any rolling deploy it is
/// what most of the traffic *is*.
///
/// Three cases, and the third is what stops the first two passing
/// vacuously: agreeing on the version must still run the handler.
#[test]
fn the_router_refuses_a_call_that_names_another_wire_format() {
    let reported = drive_router(
        r#"
globalThis.__out = [];
const cases = [
  ['agreed', '1'],
  ['ahead', '2'],
  ['unversioned', null],
];
Promise.all(
  cases.map(([label, wire]) =>
    route(requestFor('https://x/_zd/greeting', 'POST', ['Ada'], wire), COUNTING, STORE, {}, CONFIG)
      .then((response) => label + ' ' + response.status + ' ' + response.body)
  )
).then((lines) => { globalThis.__out = lines; });
"#,
        "globalThis.__out.join('\\n') + '\\nran ' + globalThis.__ran",
    );
    let lines: Vec<&str> = reported.lines().collect();
    assert_eq!(lines.len(), 4, "three cases and a count: {reported}");

    // Agreeing runs the handler and answers with its value.
    assert!(
        lines[0].starts_with("agreed 200") && lines[0].contains("Hello, Ada."),
        "the matching version did not run: {}",
        lines[0]
    );

    // Naming another version, and naming none, are the same refusal, and
    // each names both numbers so the answer is actionable. Each line is
    // paired with the exact text it must quote rather than checked against
    // either — an `a || b` here would pass if one arm were unreachable.
    for (line, arrived) in [(lines[1], "wire format 2"), (lines[2], "wire format none")] {
        assert!(
            line.contains(" 400 "),
            "a mismatched wire format was not refused: {line}"
        );
        assert!(
            line.contains(arrived),
            "the refusal does not quote `{arrived}`, which is what arrived: {line}"
        );
        assert!(
            line.contains("reads 1"),
            "the refusal does not name the version this server speaks: {line}"
        );
        assert!(
            !line.contains("Hello"),
            "a refused call still produced the handler's answer: {line}"
        );
    }

    // And the handler ran exactly once — for the case that agreed. A
    // refusal that answered 400 *after* running the handler would satisfy
    // every assertion above and would still have committed the work.
    assert_eq!(
        lines[3], "ran 1",
        "the handler ran for a refused call, so the check is after the work: {reported}"
    );
}

/// Every answer names the format it is written in, refusals included.
///
/// It is what lets a browser refuse a server *older* than itself: a build
/// from before #144 stamps nothing, and the absence is what the client
/// notices. A server that stamped only its successes would leave a client
/// unable to tell a refusal from a build that has no opinion.
#[test]
fn every_router_answer_names_the_wire_format() {
    let reported = drive_router(
        r#"
globalThis.__out = [];
const calls = [
  ['ok',        requestFor('https://x/_zd/greeting', 'POST', ['Ada'], '1')],
  ['refused',   requestFor('https://x/_zd/greeting', 'POST', ['Ada'], '2')],
  ['unknown',   requestFor('https://x/_zd/nope',     'POST', [],      '1')],
  ['wrongverb', requestFor('https://x/_zd/greeting', 'GET',  [],      '1')],
];
Promise.all(
  calls.map(([label, request]) =>
    route(request, ENDPOINTS, STORE, {}, CONFIG).then(
      (response) => label + ' ' + String(response.headerBag['zd-wire'])
    )
  )
).then((lines) => { globalThis.__out = lines; });
"#,
        "globalThis.__out.join('\\n')",
    );
    for line in reported.lines() {
        assert!(
            line.ends_with(" 1"),
            "an answer did not name the wire format it is written in: {line}\n{reported}"
        );
    }
    assert_eq!(reported.lines().count(), 4, "four answers: {reported}");
}

/// **The generated router narrows `?keys=` to the keys the program
/// declares**, which `zdc dev` has always done and this side did not.
///
/// `zdc-dev`'s `permitted` says why: the query string arrives from outside,
/// and a key the program never declared would otherwise be a way to read
/// any value in the store by guessing its name. The deployed artefact — the
/// one reachable from the public internet — was the half without the check,
/// and both transports reach it by `GET`, because their branches in `route`
/// return before the `POST` guard that covers ordinary endpoints.
#[test]
fn the_generated_config_carries_the_keys_a_subscriber_may_ask_for() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Cloudflare);
    let config = deployment
        .files
        .iter()
        .find(|file| file.path == "_zd/config.js")
        .expect("the deployment writes a config");

    assert!(
        config.contents.contains("durableKeys: ["),
        "the router has no key list to narrow against:\n{}",
        config.contents
    );
    // `guestbook.zd`'s one durable cell, by name. A list that lost its
    // contents would still carry the field.
    assert!(
        config.contents.contains("'visits'"),
        "the declared key is not in the list:\n{}",
        config.contents
    );

    // And the router consults it rather than splitting the query alone.
    let router = deployment
        .files
        .iter()
        .find(|file| file.path == "_zd/router.js")
        .expect("the deployment writes a router");
    assert!(
        router.contents.contains("config.durableKeys"),
        "the router does not narrow the key set it was given:\n{}",
        router.contents
    );
    assert!(
        router.contents.contains("declared.has(key)"),
        "the narrowing is not applied to each requested key"
    );
}

/// A program with no `durable` state declares no keys, so the list is
/// empty and every subscription is refused — rather than the field being
/// absent and the narrowing silently passing everything.
#[test]
fn a_program_with_no_durable_state_declares_an_empty_key_list() {
    let bundle =
        compile_source("state n is client Whole starting 0\n\nview\n    Text (text of n)\n");
    let deployment = zdc_deploy::generate(
        &Program {
            functions: &bundle.functions,
            linked: &bundle.linked_modules,
            durable: &bundle.durable,
            environment: &bundle.environment,
            // Nothing hashed: this program links no asset stylesheet, and
            // #137's field is the list of names that may be served
            // `immutable`.
            immutable: &[],
        },
        &Options::new(Target::Cloudflare, "empty"),
    )
    .expect("a client-only program deploys");
    let config = deployment
        .files
        .iter()
        .find(|file| file.path == "_zd/config.js")
        .expect("the deployment writes a config");
    assert!(
        config.contents.contains("durableKeys: [],"),
        "an empty list must be written rather than omitted, or the \
         narrowing reads `undefined` and lets everything through:\n{}",
        config.contents
    );
}
