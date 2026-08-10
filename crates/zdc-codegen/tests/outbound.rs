//! What an outbound request actually sends, and what the page is allowed
//! to send it to (#19).
//!
//! `crates/zdc-graph/tests/outbound.rs` establishes that the compiler
//! refuses a secret on every route into a request. This file is the other
//! half of that argument: that the routes it says are *closed* are closed
//! in the bytes, and that the destination the program wrote is the
//! destination the browser is permitted to reach.
//!
//! Every assertion is against the emitted output or against the shipped
//! runtime source, never against a constant in the compiler, for the
//! reason `csp.rs` gives: a policy the emitted program violates is worse
//! than no policy, and it fails only in a browser.

mod support;

use support::compile_source;

use zdc_codegen::CONTENT_SECURITY_POLICY;

/// The directive a policy names, or `None` when it does not name it.
fn directive<'a>(policy: &'a str, name: &str) -> Option<&'a str> {
    policy
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(name).filter(|rest| rest.starts_with(' ')))
        .map(str::trim)
}

fn policy_of(page: &str) -> String {
    let at = page
        .find("Content-Security-Policy\" content=\"")
        .expect("every emitted page carries a policy");
    let rest = &page[at + "Content-Security-Policy\" content=\"".len()..];
    rest[..rest.find('"').expect("an unterminated policy")].to_string()
}

const SAME_ORIGIN: &str = r#"request quote is client
    from  "/quote.txt"
    gives Text

view
    when quote
        Loading show Text "…"
        Failed with error show Text error.message
        Ready with body show Text body
"#;

const CROSS_ORIGIN: &str = r#"request quote is client
    from  "https://api.example.org/v1/quote"
    gives Text

view
    when quote
        Loading show Text "…"
        Failed with error show Text error.message
        Ready with body show Text body
"#;

// --- the policy -----------------------------------------------------------

/// A same-origin request changes nothing. `connect-src 'self'` already
/// permitted it, and a policy that named the origin again would be wider
/// prose saying the same thing.
#[test]
fn a_same_origin_request_leaves_the_policy_exactly_as_it_was() {
    let bundle = compile_source(SAME_ORIGIN);
    let page = bundle.index_html.expect("a page");
    assert_eq!(
        policy_of(&page),
        CONTENT_SECURITY_POLICY,
        "a same-origin request widened the policy"
    );
}

/// **The program's declaration is what widens the policy.** The origin in
/// `connect-src` is the origin on the `from` line, and nothing else moved.
#[test]
fn a_cross_origin_request_names_its_origin_and_widens_nothing_else() {
    let bundle = compile_source(CROSS_ORIGIN);
    let page = bundle.index_html.expect("a page");
    let policy = policy_of(&page);

    assert_eq!(
        directive(&policy, "connect-src"),
        Some("'self' https://api.example.org"),
        "the policy must name the origin the program wrote, and only it"
    );
    // Not `https:`, which would be one character shorter and would permit
    // every host on the web — the blanket loosening this design exists to
    // avoid.
    assert!(
        !policy.contains("connect-src 'self' https:;")
            && !policy.contains("connect-src 'self' http:"),
        "the policy widened to a scheme rather than to an origin: {policy}"
    );
    // Every other directive is the one the constant has. Compared by
    // reconstructing the constant from this policy rather than by listing
    // the directives here, so a directive added to the policy later is
    // covered without anybody remembering to add it.
    assert_eq!(
        policy.replace("'self' https://api.example.org", "'self'"),
        CONTENT_SECURITY_POLICY,
        "a directive other than `connect-src` changed"
    );
}

/// Two declarations naming one host widen the policy once, and two hosts
/// are written in a fixed order — so the emitted document does not depend
/// on the order the declarations were written in.
#[test]
fn origins_are_deduplicated_and_ordered() {
    let source = r#"request one is client
    from  "https://b.example.org/x"
    gives Text

request two is client
    from  "https://a.example.org/y"
    gives Text

request three is client
    from  "https://b.example.org/z"
    gives Text

view
    Column
        when one
            Loading show Text "…"
            Failed with error show Text error.message
            Ready with body show Text body
        when two
            Loading show Text "…"
            Failed with error show Text error.message
            Ready with body show Text body
        when three
            Loading show Text "…"
            Failed with error show Text error.message
            Ready with body show Text body
"#;
    let bundle = compile_source(source);
    let page = bundle.index_html.expect("a page");
    assert_eq!(
        directive(&policy_of(&page), "connect-src"),
        Some("'self' https://a.example.org https://b.example.org")
    );
}

/// A program with no request is byte-for-byte the program it was.
#[test]
fn a_program_with_no_request_pays_nothing() {
    let bundle = compile_source(
        "state greeting is client Text starting \"hi\"\n\nview\n    Text \
                                 greeting\n",
    );
    let page = bundle.index_html.expect("a page");
    assert_eq!(policy_of(&page), CONTENT_SECURITY_POLICY);
    assert!(
        !bundle.runtime.contains("runtime/request.js"),
        "a program with no request shipped the request runtime"
    );
    assert!(
        !bundle.client_js.contains("$request"),
        "{}",
        bundle.client_js
    );
}

/// The manifest records the origins too, for the reader who has the
/// manifest and not the compiler.
///
/// A deploy target that wrote a Content-Security-Policy *header* from
/// `origins` alone — the module origins — would emit one that blocks the
/// program's own requests, which is the compiler emitting the mistake
/// itself. The two sets are separate because they answer different
/// questions: where the page loads modules from, and where it sends
/// requests to.
#[test]
fn the_manifest_records_the_origins_a_request_reaches() {
    let bundle = compile_source(CROSS_ORIGIN);
    assert!(
        bundle
            .manifest_json
            .contains("\"connect\":[\"https://api.example.org\"]"),
        "{}",
        bundle.manifest_json
    );
    let same = compile_source(SAME_ORIGIN);
    assert!(
        same.manifest_json.contains("\"connect\":[]"),
        "a same-origin request names no origin: {}",
        same.manifest_json
    );
}

// --- what the runtime sends -----------------------------------------------

/// The module is linked **only** on use, and it is the whole of what a
/// request costs: no `rpc.js`, no `wire.js`, no `store.js`.
#[test]
fn a_request_links_one_extra_runtime_module() {
    let bundle = compile_source(SAME_ORIGIN);
    let linked: Vec<&str> = bundle.runtime.iter().copied().collect();
    assert!(
        linked.contains(&"runtime/request.js"),
        "a program with a request must ship it: {linked:?}"
    );
    // Shipping the file and importing it are two decisions in two places,
    // and a merge can satisfy one without the other: the bundle then
    // carries `request.js` and the module never names it, so the emitted
    // `$request(…)` call is a `ReferenceError` at first paint rather than
    // a compile error. `document_keys.rs` asserts the same pairing for
    // `keys.js` and caught exactly that.
    assert!(
        bundle.client_js.contains("/request.js"),
        "the import list and the shipped set are one decision: {}",
        bundle.client_js
    );
    for absent in ["runtime/rpc.js", "runtime/wire.js", "runtime/store.js"] {
        assert!(
            !linked.contains(&absent),
            "a request pulled in {absent}, which it does not import: {linked:?}"
        );
    }
}

/// **Route 3, in the bytes.** The only headers a request carries are the
/// module's own frozen constant, and no program value can reach one.
///
/// Two assertions and they are different: that `HEADERS` is the only thing
/// in header position, and that `HEADERS` itself holds no interpolation —
/// a frozen object of literals cannot be given a credential however the
/// caller is written.
#[test]
fn a_request_sends_only_the_runtimes_own_headers() {
    let source = zdc_runtime::REQUEST_JS;
    let uses: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("headers:"))
        .collect();
    assert_eq!(
        uses,
        vec!["headers: HEADERS,"],
        "something other than the frozen constant reached header position"
    );

    let declaration = source
        .lines()
        .find(|line| line.starts_with("const HEADERS"))
        .expect("the constant must be there for the assertion above to mean anything");
    assert!(
        declaration.contains("Object.freeze("),
        "the header set must be frozen: {declaration}"
    );
    assert!(
        !declaration.contains("${") && !declaration.contains('+'),
        "a header value is interpolated: {declaration}"
    );
    // And the set is the one header the module documents. Asserted on the
    // declaration rather than by scanning the file, because the file's own
    // prose names `Authorization` — explaining why there is no way to send
    // one — and a scan for the word would fail on the explanation.
    assert_eq!(
        declaration, "const HEADERS = Object.freeze({ accept: 'text/plain, application/json' });",
        "the header set changed"
    );
}

/// **Route 4, in the bytes.** A request is a `GET` with no body.
#[test]
fn a_request_sends_no_body() {
    let source = zdc_runtime::REQUEST_JS;
    let methods: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("method:"))
        .collect();
    assert_eq!(methods, vec!["method: 'GET',"], "a request is a read");

    let bodies: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("body:"))
        .collect();
    assert_eq!(
        bodies,
        vec!["body: undefined,"],
        "a request must carry nothing in its body"
    );
    // The emitted bundle cannot supply one either: `$request` takes a
    // destination and a list of pairs, and there is no third argument.
    let bundle = compile_source(SAME_ORIGIN);
    assert!(
        bundle.client_js.contains("$request('/quote.txt', [])"),
        "{}",
        bundle.client_js
    );
}

/// A response's failure message is composed by the runtime and never read
/// out of the answer, so an answering host cannot put its own prose on the
/// page. This is the one place this file's rule differs from `rpc.js`'s,
/// which reads `body.error` — right for a body it wrote, wrong for one it
/// did not.
#[test]
fn a_failure_message_is_never_read_out_of_the_response() {
    let source = zdc_runtime::REQUEST_JS;
    for reading in ["response.json()", "body.error", "await response.text()"] {
        // `response.text()` is how the *body* is read on success, so the
        // needle is the failure path's shape rather than the call itself.
        assert!(
            !source.contains(&format!(
                "throw new RequestFailure(CODES.REJECTED, {reading}"
            )),
            "a failure message came out of the response: {reading}"
        );
    }
    assert!(
        source.contains("answered with ${response.status}"),
        "the failure message must be the status line and the runtime's own words"
    );
}

/// Every argument is percent-encoded, so a value stays inside its own
/// parameter: it cannot add a parameter, truncate the query, or reach the
/// path.
#[test]
fn a_query_value_cannot_leave_its_parameter() {
    let mut context = support::context(false);
    support::evaluate_module(&mut context, zdc_runtime::REQUEST_JS);
    let built = support::run(
        &mut context,
        "",
        "requestUrl('https://api.example.org/v1', [['q', 'a&admin=1'], ['p', 'x#y/z']])",
    );
    assert_eq!(
        built, "https://api.example.org/v1?q=a%26admin%3D1&p=x%23y%2Fz",
        "a value escaped its parameter"
    );
}

/// A request with no arguments is the destination unchanged — no trailing
/// `?`, which some hosts treat as a different resource.
#[test]
fn a_request_with_no_arguments_is_the_destination_itself() {
    let mut context = support::context(false);
    support::evaluate_module(&mut context, zdc_runtime::REQUEST_JS);
    assert_eq!(
        support::run(&mut context, "", "requestUrl('/quote.txt', [])"),
        "/quote.txt"
    );
}
