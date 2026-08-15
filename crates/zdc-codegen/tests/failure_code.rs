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
// A crafted response is still a response, and a response names the wire
// format it is written in (#144). Every case below gets the version this
// build speaks unless it states one of its own, so that these tests keep
// asking their question — which code does a *body* choose — rather than
// all becoming the version refusal.
globalThis.__wireHeaders = (version) => ({{
  get: (name) => (name === 'zd-wire' ? version : null),
}});
globalThis.__queue = ({responses}).map((response) =>
  Object.assign({{ headers: globalThis.__wireHeaders('1') }}, response)
);
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
        // `.tag`: `code` is a value of the built-in `choice` `Code`, so
        // it travels as `{ tag, fields }` like every other variant.
        return payload.code.tag + ' | ' + payload.message;
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

/// **The deliberate mismatch (#144), from the client's side.**
///
/// A 200 whose body decodes perfectly, from a server speaking a wire
/// format this page does not. Before the version existed the value went
/// straight into `Ready` — the page rendered whatever the other format's
/// bytes happened to mean, and nothing anywhere said so. Now it is
/// `Failed`, and the message names both numbers so a person reading a red
/// bar knows the answer is "reload" rather than "the server is down".
///
/// A *missing* header is the same outcome and is here for the case that
/// will really happen: a rollback to a build that predates this sends no
/// header at all, and treating silence as agreement would leave the whole
/// rule open in the one situation it was written for.
///
/// The code is `Rejected` and not a fourth `Code`, which is the point of
/// putting this test in *this* file. A version the server picks by
/// writing a header is a bit of channel at a public label — the same
/// argument that dropped `Malformed` — so the mismatch reuses the code a
/// status line already produces and says the rest in `message`.
#[test]
fn an_answer_in_another_wire_format_is_a_named_failure_and_not_a_value() {
    let lines = transport_outcomes(
        r#"[
  { ok: true, status: 200, headers: globalThis.__wireHeaders('2'),
    json: () => Promise.resolve(7) },
  { ok: true, status: 200, headers: globalThis.__wireHeaders(null),
    json: () => Promise.resolve(7) },
  { ok: true, status: 200, headers: globalThis.__wireHeaders('1'),
    json: () => Promise.resolve(7) }
]"#,
    );
    assert_eq!(lines.len(), 3, "three responses, three outcomes: {lines:?}");

    for (line, named) in [(&lines[0], "2"), (&lines[1], "none")] {
        assert!(
            line.starts_with("Rejected | "),
            "a mismatched wire format did not reach the program as Rejected: {line}"
        );
        assert!(
            line.contains(&format!("wire format {named}")) && line.contains("reads 1"),
            "the refusal does not name both versions, so a reader cannot tell \
             what to do about it: {line}"
        );
    }

    // Non-vacuity: the agreeing version still decodes, or the two
    // assertions above would be satisfied by a transport that refused
    // every answer it was given.
    assert_eq!(
        lines[2], "Ready(7)",
        "the matching version stopped decoding, so the check refuses everything: {lines:?}"
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

/// `rpc.js` and the language's `Code` choice hold the same set, and
/// neither may drift.
///
/// **Derived from the choice, not from a second list.** The arms are read
/// off `builtin_choice_of(&Type::Code)` — the same value the checker uses
/// for exhaustiveness and the same one a diagnostic lists — so this test
/// ranges over exactly what a program can write. `code_choice` in turn
/// builds those arms from `FailureCode::CLOSED_SET`, so one edit to the
/// enum moves the surface language, the diagnostics and this test
/// together, and lands here as a missing spelling in `rpc.js`.
///
/// The count is asserted against the choice's own arm list rather than
/// against a number: a `[FailureCode; N]` compared to `N` compares a
/// constant to itself, which is why the loop counts what it visited.
#[test]
fn the_runtime_spells_exactly_the_codes_the_compiler_knows() {
    let source = zdc_runtime::RPC_JS;
    let arms = zdc_types::code_choice().variants;
    assert!(
        !arms.is_empty(),
        "`Code` has no arms, so this ranges over nothing"
    );
    let mut checked = 0;
    for arm in &arms {
        let literal = format!("'{}'", arm.name);
        assert!(
            source.contains(&literal),
            "`rpc.js` never writes {literal}, so `Code` has an arm the runtime cannot produce"
        );
        checked += 1;
    }
    assert_eq!(checked, arms.len(), "the loop skipped an arm");

    // And the enum the arms were built from names the same set, so a
    // spelling cannot be added on one side of that derivation only.
    let spelled: Vec<&str> = FailureCode::CLOSED_SET
        .iter()
        .map(|code| code.spelling())
        .collect();
    let named: Vec<&str> = arms.iter().map(|arm| arm.name.as_str()).collect();
    assert_eq!(named, spelled);

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
