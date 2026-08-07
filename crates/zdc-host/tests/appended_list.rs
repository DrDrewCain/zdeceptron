//! **A durable list built with `append`, from the click to the bytes.**
//!
//! `append` does not build an array. `zdc-codegen` emits a chain of `$Ap`
//! links so that appending is O(1) and the chain is flattened once, which
//! is what keeps a builder in tail position linear, and the class carries
//! a `toJSON` so that serialisation gets the list rather than the links.
//!
//! `runtime/wire.js`'s `encode` walked the value itself, before
//! `JSON.stringify` was ever called, so the `toJSON` never ran: an `$Ap`
//! is not a `Map` and is not an array, so it fell through to the record
//! branch and `base`, `item` and `flat` were copied verbatim. A durable
//! list holding `[1]` reached the store as
//! `{"base":[],"item":1,"flat":null}`, with no error, no diagnostic and a
//! build that exited 0 (#204).
//!
//! # Why this reads the store rather than the value
//!
//! This is the third bug in one family. `STATUS.md` §6 records the first:
//! `JSON.stringify(new Map(...))` is `{}`, so every `durable Map` stored an
//! empty object and nothing noticed, because no example exercised the
//! path. `wire.js` is the codec written to fix that and `examples/tally.zd`
//! is the example written to exercise it.
//!
//! What every one of them survived is a test that looked at the value in
//! memory. `$force(chain)` is `[1]` and `chain.length` is 1, so an
//! assertion about the list the program computed passes with the bug
//! present. The bytes in the store are the only place it shows, so the
//! bytes in the store are what is asserted.
//!
//! Both halves are the real ones: the emitted browser bundle computes the
//! value and encodes it with the runtime's own `wire.js`, and the body
//! that comes out is handed to the emitted endpoint over a real store.

mod support;

use std::sync::Arc;

use boa_engine::{Context, Source};

use support::{bundle, endpoints};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Json};

/// The minimal DOM the runtime is exercised against, shared with
/// `zdc-runtime`'s own tests rather than copied.
const DOM_SHIM: &str = include_str!("../../zdc-runtime/tests/dom-shim.js");

const APPENDING: &str = "\
state items is durable List of Whole starting empty

view
    Column
        Button \"store\"
            on click
                set items to (append 1 to empty)
";

/// A module's source with its `import`/`export` lines flattened away, so
/// several of them can share one engine scope.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The browser, as far as the network: the DOM shim, the runtime, and the
/// aliases a module loader would have bound.
fn browser() -> Context {
    let mut context = Context::default();
    for (what, source) in [
        ("the DOM shim", DOM_SHIM.to_string()),
        ("signal.js", flatten(zdc_runtime::SIGNAL_JS)),
        ("dom.js", flatten(zdc_runtime::DOM_JS)),
        ("markup.js", flatten(zdc_runtime::MARKUP_JS)),
        ("wire.js", flatten(zdc_runtime::WIRE_JS)),
        ("rpc.js", flatten(zdc_runtime::RPC_JS)),
    ] {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("{what} failed to evaluate: {e}"));
    }
    context
        .eval(Source::from_bytes(
            b"const $atomic = atomic, $failed = reportFailure;",
        ))
        .expect("the rpc aliases bind");
    context
}

/// Click the one button in the emitted page and return the request body
/// the runtime would have posted.
///
/// The transport is replaced, and it calls `stringify` itself, because
/// that is what the shipped one does: `rpc.js`'s `defaultTransport` posts
/// `stringify(args)`, never `JSON.stringify(args)`. A stub that recorded
/// the arguments instead of encoding them would be testing around the
/// codec this file is about.
fn posted(client_js: &str) -> String {
    let mut context = browser();
    context
        .eval(Source::from_bytes(
            br#"
let $body = 'the handler posted nothing';
setTransport((name, args) => {
  if (name === '~atomic') $body = stringify(args);
  return Promise.resolve(null);
});
"#,
        ))
        .expect("the transport installs");
    context
        .eval(Source::from_bytes(flatten(client_js).as_bytes()))
        .unwrap_or_else(|e| panic!("the bundle failed to evaluate: {e}\n\n{client_js}"));
    context
        .eval(Source::from_bytes(
            br#"
const $host = document.createElement('div');
main($host);
walk($host).filter((n) => n.tagName === 'button')[0].fire('click');
"#,
        ))
        .unwrap_or_else(|e| panic!("the click failed: {e}"));
    context.run_jobs().expect("the write settles");
    context
        .eval(Source::from_bytes(b"$body"))
        .expect("the body is readable")
        .to_string(&mut context)
        .expect("the body is a string")
        .to_std_string_escaped()
}

/// **The bug, end to end.** One click, and what is in the store afterwards.
#[test]
fn a_list_built_with_append_reaches_the_store_as_a_list() {
    let bundle = bundle(APPENDING, "appending.zd");
    let body = posted(&bundle.client_js);
    assert_eq!(
        body, "[[\"items.set\",[[1]]]]",
        "the browser encoded the append chain instead of the list it stands for"
    );

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = Host::new(
        endpoints(bundle.functions),
        Arc::clone(&store),
        Environment::empty(),
    );
    host.invoke_batch(&body).expect("the write runs");

    assert_eq!(
        store.get("items").expect("get").map(Json::into_string),
        Some("[1]".to_string()),
        "a durable list built with `append` was stored as the chain rather than the list"
    );
}

/// **The same chain, one level down.** `encode` recurses through a record,
/// so a list inside one is a second way to reach the same defect: the
/// field would have held the links rather than the elements, and the
/// record around it would have looked perfectly ordinary.
#[test]
fn an_appended_list_inside_a_record_reaches_the_store_as_a_list() {
    let source = "\
record Bag
    tags is List of Whole
    size is Whole

state bag is durable Bag starting (Bag with tags is empty, size is 0)

view
    Column
        Button \"store\"
            on click
                set bag to (Bag with tags is (append 2 to (append 1 to empty)), size is 2)
";
    let bundle = bundle(source, "bag.zd");
    let body = posted(&bundle.client_js);

    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let host = Host::new(
        endpoints(bundle.functions),
        Arc::clone(&store),
        Environment::empty(),
    );
    host.invoke_batch(&body).expect("the write runs");

    assert_eq!(
        store.get("bag").expect("get").map(Json::into_string),
        Some("{\"tags\":[1,2],\"size\":2}".to_string()),
        "the record's list field was stored as the append chain"
    );
}
