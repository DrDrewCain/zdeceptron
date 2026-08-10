//! The outbound request, and the four ways a secret could reach one (#19).
//!
//! An outbound request is the most direct way to leak a secret that
//! exists: it is the only construct in the language whose whole purpose is
//! to send bytes to a machine the program names. §14G.1.3(c)'s sink 7 —
//! `Sink::OutboundRequest` — already existed for the URL-bearing
//! attributes a browser dereferences, and this file is about its second
//! producing site.
//!
//! # The four routes, and which of them exist
//!
//! A `fetch` has four places a value can ride out on: the URL, a query
//! parameter, a header, and the body. This design leaves **two** of them
//! and closes two, and the difference between "checked" and "closed" is
//! the point of the split below:
//!
//! * **The query parameter is checked.** `with key is …` is the one place
//!   a program value enters a request, and every one of them raises an
//!   obligation against sink 7 separately.
//! * **The URL is checked, twice over.** The destination is a literal, so
//!   there is no expression there to carry anything — and because the
//!   arguments are *appended to the destination* as a query string, an
//!   argument **is** part of the URL. `fetch("https://x/?k=" + apiKey)` is
//!   a leak with no body at all, and in this design it is spelled
//!   `with k is apiKey`, which is the case the first bullet catches.
//! * **The header is closed.** There is no header clause. A request
//!   carries `runtime/request.js`'s frozen `HEADERS` and nothing else, so
//!   `Authorization: Bearer <secret>` — the shortest path from a
//!   credential to a third party — has no syntax.
//! * **The body is closed.** A request is a `GET`. `Remote of T` is a
//!   *read*, and a request that changed something on a third party would
//!   be a command with an outcome cell, which is a different construct.
//!
//! The two closures are asserted here as *refusals of syntax*, and again
//! in `crates/zdc-codegen/tests/outbound.rs` against the emitted bytes,
//! because "the compiler cannot spell it" and "the runtime does not send
//! it" are two claims and a reader should not have to take either on
//! trust.
//!
//! # Why a secret is always behind a `Remote`, and why that is not the
//! whole answer
//!
//! `secret client` and `secret static` are both E0313 — a secret may not
//! live where its reader lives — so every secret a browser can reach is
//! `server` or `durable`, and reading one from the client is a
//! **crossing**. That means the escape is raised at the *read*, not at the
//! argument, and a rule that only inspected an argument's own label would
//! find `⊥` there and pass. `UrlPosition::RequestArgument` is what carries
//! "this read is inside a request argument" down to `Ifc::read`, and it is
//! why these fixtures report sink 7 rather than sink 1.
//!
//! It also means the type checker refuses the same programs independently
//! — `Remote of Text` is not `Shown`, so it cannot go in a query string.
//! That is a second lock and not a reason to drop this one: the type rule
//! is about the shape of a value, and a shape rule that happened to
//! coincide with a security rule today is a shape rule that can stop
//! coinciding with it tomorrow. These tests run the flow pass alone.

mod support;

use support::{codes, verdict};
use zdc_graph::authority::Solution;
use zdc_graph::integrity::{Authority, Integrity, Writers};

/// Reachable from a browser, so a request can name it, and `secret` — the
/// premise every fixture below shares.
const SECRET: &str = "secret state apiKey is durable Text starting \"sk-live\"\n";

fn errors(src: &str) -> Vec<String> {
    let (_, _, verdict) = verdict(src);
    verdict
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{} {}", d.code, d.message))
        .collect()
}

fn flow_codes(src: &str) -> Vec<String> {
    let (_, _, verdict) = verdict(src);
    codes(&verdict.diagnostics)
        .iter()
        .map(|code| code.to_string())
        .collect()
}

/// Whatever a program says after the request, so the declaration is read
/// and the split gives it a client root.
const SPEND: &str = "\nview\n    when feed\n        Loading show Text \"…\"\n        Failed with \
                     error show Text error.message\n        Ready with body show Text body\n";

// --- route 1: the query parameter ----------------------------------------

/// **Route 1.** The secret is the argument, and the argument is the query
/// string. This is the obligation the whole feature turns on.
#[test]
fn a_secret_in_a_query_parameter_is_refused() {
    let src = format!(
        "{SECRET}\nrequest feed is client\n    from  \"https://api.example.org/v1/feed\"\n    \
         with  key is apiKey\n    gives Text\n{SPEND}"
    );
    let found = errors(&src);
    assert!(
        found.iter().any(|error| error.starts_with("E-IFC-11")),
        "a secret reached a request's query string with no diagnostic: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("chooses the host it is sent to")),
        "the diagnostic must say why an outbound request is a sink: {found:?}"
    );
}

/// The same shape with a public value raises nothing, so the test above is
/// about the secret and not about the construct.
#[test]
fn a_public_value_in_a_query_parameter_is_accepted() {
    let src = format!(
        "state topic is client Text starting \"signals\"\n\nrequest feed is client\n    from  \
         \"https://api.example.org/v1/feed\"\n    with  q is topic\n    gives Text\n{SPEND}"
    );
    assert_eq!(flow_codes(&src), Vec::<String>::new(), "{:?}", errors(&src));
}

/// Two arguments are two obligations. Repairing one must not discharge the
/// other, which is what keying the site on the *expression* buys.
#[test]
fn each_argument_is_its_own_obligation() {
    let src = format!(
        "{SECRET}state topic is client Text starting \"x\"\n\nrequest feed is client\n    from  \
         \"https://api.example.org/v1/feed\"\n    with  q is topic, key is apiKey\n    gives \
         Text\n{SPEND}"
    );
    let found = errors(&src);
    let leaks: Vec<&String> = found
        .iter()
        .filter(|error| error.starts_with("E-IFC-11"))
        .collect();
    assert_eq!(
        leaks.len(),
        1,
        "the secret argument and the public one must be told apart: {found:?}"
    );
    assert!(leaks[0].contains("key"), "{leaks:?}");
}

// --- route 2: the URL ----------------------------------------------------

/// **Route 2, first half.** The secret concatenated into an argument.
///
/// This is `fetch("https://x/?k=" + apiKey)` written in this language: the
/// argument is appended to the destination, so the secret is in the URL
/// and in nothing else. It is caught by the join rather than by
/// recognising the reference, which is what makes hiding it in an
/// expression not work.
#[test]
fn a_secret_concatenated_into_a_url_is_refused() {
    let src = format!(
        "{SECRET}\nrequest feed is client\n    from  \"https://api.example.org/v1/feed\"\n    \
         with  key is (\"Bearer \" + apiKey)\n    gives Text\n{SPEND}"
    );
    let found = errors(&src);
    assert!(
        found.iter().any(|error| error.starts_with("E-IFC-11")),
        "a secret concatenated into the URL escaped: {found:?}"
    );
}

/// **Route 2, second half.** The destination cannot be an expression, so
/// there is no way to compute the host at all.
///
/// A parse error and not a flow error, deliberately: a computed
/// destination could not be checked by any pass, could not be named in the
/// emitted `connect-src`, and so is refused where the alternative was
/// never constructed.
#[test]
fn a_computed_destination_does_not_parse() {
    for destination in [
        // A name.
        "apiKey",
        // A concatenation.
        "\"https://api.example.org/?k=\" + apiKey",
        // A call.
        "hostOf with key is apiKey",
    ] {
        let src =
            format!("{SECRET}\nrequest feed is client\n    from  {destination}\n    gives Text\n");
        let error = zdc_parser::parse(&src)
            .err()
            .unwrap_or_else(|| panic!("`from {destination}` must not parse"));
        assert!(
            error.message.contains("written down"),
            "the refusal must say why: {}",
            error.message
        );
    }
}

/// And a destination that is a literal can still be one no browser should
/// be asked to fetch.
#[test]
fn a_destination_the_policy_could_not_name_is_refused() {
    for destination in [
        // The scheme is left to whatever served the page.
        "//api.example.org/v1",
        // Resolves against the document, and a routed program has many.
        "feed.json",
        // A credential in a URL.
        "https://sk-live@api.example.org/v1",
        // Not a scheme that fetches.
        "javascript:alert(1)",
        "data:text/plain,x",
        "mailto:a@example.com",
    ] {
        let src = format!("request feed is client\n    from  \"{destination}\"\n    gives Text\n");
        let program = zdc_parser::parse(&src).expect("the destination is a literal, so it parses");
        let errors = zdc_resolve::Resolver::new(&program)
            .resolve()
            .err()
            .unwrap_or_else(|| panic!("`{destination}` must not be a destination"));
        assert!(
            errors[0].message.contains("is not a destination"),
            "{}",
            errors[0].message
        );
    }
}

// --- routes 3 and 4: the header and the body -----------------------------

/// **Route 3.** There is no header clause, so a credential has no header
/// to ride out in.
///
/// Asserted as a parse failure naming the clauses that do exist, rather
/// than as "some error happened": a program that failed for an unrelated
/// reason would pass a weaker assertion and prove nothing.
#[test]
fn a_request_has_no_header_clause() {
    let src = format!(
        "{SECRET}\nrequest feed is client\n    from   \"https://api.example.org/v1/feed\"\n    \
         header \"authorization\" is apiKey\n    gives  Text\n"
    );
    let error = zdc_parser::parse(&src).expect_err("a header clause must not parse");
    assert!(
        error.message.contains("gives"),
        "the refusal must name the clause that was expected: {}",
        error.message
    );
}

/// **Route 4.** There is no body clause either, for the same reason and a
/// different one: a request is a `GET`, and a `Remote of T` is a read.
#[test]
fn a_request_has_no_body_clause() {
    for clause in ["sends body is apiKey", "body is apiKey", "posts apiKey"] {
        let src = format!(
            "{SECRET}\nrequest feed is client\n    from  \
             \"https://api.example.org/v1/feed\"\n    {clause}\n    gives Text\n"
        );
        assert!(
            zdc_parser::parse(&src).is_err(),
            "`{clause}` must not parse"
        );
    }
}

// --- the response ---------------------------------------------------------

/// **What comes back is Untrusted, always.**
///
/// A response is attacker-controlled data. The integrity lattice is
/// default-closed — a value is Untrusted unless it derives from the closed
/// grant set — so this asserts the absence of a grant rather than the
/// presence of a rule, which is the stronger of the two claims.
#[test]
fn a_response_body_is_untrusted() {
    let src = "request feed is client\n    from  \"https://api.example.org/v1/feed\"\n    gives \
               Text\n";
    let (hir, split) = support::compile(src);
    let feed = support::def_named(&hir, "feed");
    let writers = Writers::of(&hir, &split);
    let solution = Solution::solve(&hir, &writers);
    let integrity = Integrity::new(&hir, &solution);

    // The *initialiser* first, which is where the grant would have been
    // awarded if one applied. `None` is the assertion: no member of the
    // closed set describes an answer a host gave.
    let init = match &hir.defs[feed].kind {
        zdc_hir::DefKind::Signal(signal) => signal.init,
        // Written out for the reason every `DefKind` match in this
        // workspace is: a request lowers to a signal today, and a change
        // that made it something else must be a compile error here.
        zdc_hir::DefKind::Function(_)
        | zdc_hir::DefKind::View(_)
        | zdc_hir::DefKind::Record(_)
        | zdc_hir::DefKind::Choice(_)
        | zdc_hir::DefKind::Component(_)
        | zdc_hir::DefKind::Foreign(_)
        | zdc_hir::DefKind::Release(_) => panic!("a request lowers to a signal"),
    };
    assert_eq!(integrity.of(init), (Authority::Untrusted, None));

    // And a read of the signal, which is what a program actually holds.
    assert_eq!(integrity.of_signal_read(feed), Authority::Untrusted);
}

/// And a `trusted` place refuses one, which is the same fact at the site a
/// program would meet it.
#[test]
fn a_response_body_may_not_be_written_into_a_trusted_place() {
    let src = "trusted state approved is durable Text starting \"\"\n\nrequest feed is \
               client\n    from  \"https://api.example.org/v1/feed\"\n    gives Text\n\nview\n    \
               Column\n        when feed\n            Loading show Text \"…\"\n            Failed \
               with error show Text error.message\n            Ready with body\n                \
               Button \"keep\"\n                    on click\n                        set \
               approved to body\n";
    let (_, _, verdict) = verdict(src);
    let found = codes(&verdict.diagnostics);
    assert!(
        found.contains(&"E-INT-03"),
        "a response body reached a `trusted` place with no diagnostic: {found:?}"
    );
}

// --- the sink itself ------------------------------------------------------

/// Sink 7 has two producing sites and is still one sink.
///
/// The closed list answers "what are the ways a value becomes visible",
/// and both of these are one way: an HTTP request leaves the browser
/// carrying the value to a host the program named. An eighth sink would
/// have said they were two media, which they are not — a mechanism is not
/// a medium. What a `server`-placed request *would* need is exactly that
/// eighth sink, because "a request the deployment sends" is a different
/// medium with a different reader, and this change does not add one.
#[test]
fn both_producers_of_sink_seven_report_the_same_sink() {
    let attribute = "secret state apiKey is durable Text starting \"sk\"\n\nview\n    Image \
                     source is apiKey, description is \"x\"\n";
    let request = format!(
        "{SECRET}\nrequest feed is client\n    from  \"https://api.example.org/v1/feed\"\n    \
         with  key is apiKey\n    gives Text\n{SPEND}"
    );
    for (what, src) in [("an attribute", attribute), ("a request", request.as_str())] {
        let found = flow_codes(src);
        assert!(
            found.contains(&"E-IFC-11".to_string()),
            "{what} did not reach sink 7: {found:?}"
        );
    }
}

/// A request that is refused at all is refused before anything is emitted.
///
/// `Verdict::clearance` is the token `zdc_codegen::Inputs` cannot be built
/// without, so this is the property that makes the refusal a build failure
/// rather than a warning beside a shipped bundle.
#[test]
fn a_refused_request_yields_no_clearance() {
    let src = format!(
        "{SECRET}\nrequest feed is client\n    from  \"https://api.example.org/v1/feed\"\n    \
         with  key is apiKey\n    gives Text\n{SPEND}"
    );
    let (_, _, verdict) = verdict(&src);
    assert!(
        verdict.clearance().is_none(),
        "a program that leaks a secret to a host was cleared for emission"
    );
}
