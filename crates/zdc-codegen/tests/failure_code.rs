//! `code`, and why it is public.
//!
//! §14G.1.3(d) puts the join of everything an endpoint reads onto the
//! `Failed` payload, which makes `message` unreadable from exactly the
//! endpoints a developer most wants explained. `code` is the other field
//! of that payload and carries `public` instead — not by assertion, but
//! because `runtime/rpc.js` picks it from the transport outcome and never
//! from a byte the server sent.
//!
//! That claim is a claim about a JavaScript file, so it is tested by
//! running that file. The engine here is the compiler's own embedded
//! `boa_engine`, the same one `zdc build` evaluates a `static` root in.

mod support;

use support::{compile_example, live_context, rpc_context, run_settled};
use zdc_types::FailureCode;

/// The flagship example, driven against a transport that never answers.
///
/// This is what the rule cost and what `code` bought back. Before it, the
/// arm could only render its own words, so the page said the same thing
/// whether the browser was offline or the service had refused — and
/// `guestbook.zd`'s comment said so. Now the page names the transport
/// outcome, from the endpoint that reads `apiKey`, and still cannot say
/// anything the host said.
#[test]
fn guestbook_shows_which_way_the_call_failed() {
    let bundle = compile_example("examples/guestbook.zd");
    let mut context = live_context();
    let rendered = run_settled(
        &mut context,
        // No response at all: the connection failed. `visits` is durable
        // and answers, so the failure on screen is `greeting`'s alone.
        r#"
setTransport((name) =>
  name === 'visits' ? Promise.resolve(0) : Promise.reject(new Error('ECONNREFUSED at 10.0.0.1')));
globalThis.$scripted = () => () => () => {};
"#,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\n",
        "serialize($host)",
    );
    assert!(
        rendered.contains("the greeting service did not answer: Unreachable"),
        "the failure state is not on the page:\n{rendered}"
    );
    assert!(
        !rendered.contains("ECONNREFUSED"),
        "the host's own words reached the page from an endpoint that reads a secret:\n{rendered}"
    );
    assert!(
        !rendered.contains("aria-busy"),
        "the spinner is still up, so this is the \"hangs in Loading\" failure:\n{rendered}"
    );
}

/// Drive `rpc.js` with a shimmed `fetch`, and report one line per crafted
/// response.
fn transport_outcomes(responses: &str) -> Vec<String> {
    let mut context = rpc_context();
    let setup = format!(
        r#"
globalThis.__queue = {responses};
globalThis.__at = 0;
// An index rather than a `shift`: the driver iterates the same array, and
// draining it underneath `map` would silently skip half the cases.
globalThis.fetch = () => Promise.resolve(globalThis.__queue[globalThis.__at++]);
globalThis.__out = [];
"#
    );
    let report = run_settled(
        &mut context,
        &setup,
        "",
        r#"
(() => {
  // One call per crafted response, in order. `defaultTransport` evaluates
  // `fetch(...)` before its first suspension, so the queue stays aligned
  // with the calls.
  const pending = globalThis.__queue.map(() =>
    Promise.resolve(defaultTransport('greeting', [])).then(
      (value) => 'Ready(' + value + ')',
      (error) => {
        const payload = failed(error).fields[0];
        return payload.code + ' | ' + payload.message;
      },
    ),
  );
  Promise.all(pending).then((lines) => { globalThis.__out = lines; });
})();
"#,
        "globalThis.__out.join('\\n')",
    );
    report.lines().map(|line| line.to_string()).collect()
}

/// A server chooses the bytes of its response. It does not choose the
/// code.
///
/// Every body below *names* a code — in the field the runtime uses
/// internally (`zdCode`), in a field with the surface name (`code`), in
/// the error text, and in the `name` an aborted request would have
/// carried. Every one of them arrives as a response, so every one of them
/// is `Rejected`, which is what the status line already said.
///
/// The message assertions are what stop this passing vacuously: they show
/// the crafted bytes really did reach the payload, and moved nothing.
#[test]
fn a_crafted_response_body_cannot_choose_the_code() {
    let responses = r#"[
  { ok: false, status: 500, json: () => Promise.resolve({ error: 'Unreachable' }) },
  { ok: false, status: 503, json: () => Promise.resolve({ error: 'Timeout', code: 'Timeout', zdCode: 'Timeout', name: 'AbortError' }) },
  { ok: true,  status: 200, json: () => Promise.reject(new Error('not json')) },
  { ok: true,  status: 200, json: () => Promise.resolve({ code: 'Unreachable' }) }
]"#;
    let lines = transport_outcomes(responses);
    assert_eq!(
        lines.len(),
        4,
        "four crafted responses, four outcomes: {lines:?}"
    );

    // The body's own word reached `message`, and `code` is the status
    // line's verdict regardless.
    assert_eq!(lines[0], "Rejected | Unreachable");
    assert_eq!(lines[1], "Rejected | Timeout");

    // A 2xx the decoder could not read. Still `Rejected` — deliberately
    // the same code a non-2xx produces, so that choosing what to write
    // into a 200 body distinguishes nothing the status line cannot.
    assert!(
        lines[2].starts_with("Rejected | "),
        "an unreadable body chose its own code: {}",
        lines[2]
    );

    // And a 2xx that *does* decode is not a failure at all, whatever it
    // says about codes.
    assert!(
        lines[3].starts_with("Ready("),
        "a decodable body was reported as a failure: {}",
        lines[3]
    );

    let steered: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("Unreachable") || line.starts_with("Timeout"))
        .collect();
    assert!(
        steered.is_empty(),
        "a response body steered the code: {steered:?}"
    );
}

/// The two codes a response cannot produce come from the client's own
/// control flow, and neither is read out of an error's text.
///
/// The falsifier is in the messages: an error that *says* `Rejected` is
/// `Unreachable`, and one that says `Rejected` while carrying an abort's
/// `name` is `Timeout`. If `codeOf` ever parsed a message, both flip.
#[test]
fn the_code_of_a_rejection_comes_from_its_provenance_and_not_its_text() {
    let mut context = rpc_context();
    let report = run_settled(
        &mut context,
        "",
        "",
        r#"
const abort = new Error('Rejected');
abort.name = 'AbortError';
globalThis.__out = [
  codeOf(new Error('Rejected')),
  codeOf(abort),
  codeOf({ zdCode: 'Timeout', message: 'Unreachable' }),
].join(',');
"#,
        "globalThis.__out",
    );
    // In order: a plain rejection is `Unreachable`, because no answer was
    // obtained; an abort is `Timeout`; and a bare object that merely
    // *claims* the runtime's own internal field is neither — only a
    // `TransportFailure` this file constructed carries a code.
    assert_eq!(
        report, "Unreachable,Timeout,Unreachable",
        "codeOf read something it should not have"
    );
}

/// `rpc.js` and `zdc-types` hold the same set, and neither may drift.
///
/// The Rust side is the one the compiler reasons with; the JavaScript
/// side is the one that writes the field. A fourth variant added to
/// `FailureCode` is a compile error inside `spelling`, and lands here as
/// a missing spelling in `rpc.js`.
#[test]
fn the_runtime_spells_exactly_the_codes_the_compiler_knows() {
    let source = zdc_runtime::RPC_JS;
    let mut checked = 0;
    for code in FailureCode::CLOSED_SET {
        let literal = format!("'{}'", code.spelling());
        assert!(
            source.contains(&literal),
            "`rpc.js` never writes {literal}, so the compiler knows a code the runtime cannot \
             produce"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        FailureCode::CLOSED_SET.len(),
        "the loop skipped a code"
    );

    // The candidate §14G.1.3(d)'s repair specified and this branch
    // dropped: a code selected by the response *body*. Its absence from
    // both sides is the property, so it is asserted rather than left to
    // the reader. As a literal, not as a word — `rpc.js` says in prose
    // why the code is not there, and that sentence is worth keeping.
    assert!(
        !source.contains("'Malformed'"),
        "`rpc.js` produces a body-derived code"
    );
    assert!(FailureCode::from_spelling("Malformed").is_none());
}
