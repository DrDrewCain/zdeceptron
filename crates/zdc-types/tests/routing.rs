//! Routes, and the integrity direction routing forces on the language.
//!
//! §14G.7.3: "`address` is the language's **first untrusted-input
//! source**." Every test below is a rule from §14G.2 or §18.1, named by
//! the revision that put it there.

/// Both passes `zdc check` runs, in the order it runs them.
///
/// The integrity direction moved: it used to be a second pass inside
/// `zdc-types`, over a default-open lattice, and it is now the closed
/// lattice in `zdc-graph` that the flow pass runs. The rules below did not
/// move with it — they are properties of the language — so this reads both
/// passes rather than one, and a rule keeps its test wherever the answer
/// comes from.
fn errors(src: &str) -> Vec<String> {
    let program = zdc_parser::parse(src).unwrap_or_else(|e| panic!("parse: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("resolve: {}", errors[0].message));
    // The real placement pass, exactly as `zdc check` runs it (§17.1.2).
    let split = zdc_graph::split(&hir);
    let mut found: Vec<String> = zdc_graph::ifc(&hir, &split)
        .diagnostics
        .into_iter()
        .filter(|d| d.is_error())
        // The code is part of the diagnostic a user sees, and the tests
        // below name codes, so it is carried onto the string here.
        .map(|d| match d.help {
            Some(help) => format!("{} ({})\n{help}", d.message, d.code),
            None => format!("{} ({})", d.message, d.code),
        })
        .collect();
    if let Err(errors) = zdc_types::check(&hir, &split) {
        // The help is part of the diagnostic §7.3 asks for, so a test
        // that read only the message would let the repair rot.
        found.extend(errors.into_iter().map(|e| match e.help {
            Some(help) => format!("{}\n{help}", e.message),
            None => e.message,
        }));
    }
    found
}

fn accepted(src: &str) {
    let found = errors(src);
    assert!(found.is_empty(), "expected this to check:\n{found:#?}");
}

fn rejected(src: &str, needle: &str) -> String {
    let found = errors(src);
    let hit = found
        .iter()
        .find(|message| message.contains(needle))
        .unwrap_or_else(|| panic!("no message contains `{needle}`:\n{found:#?}"));
    hit.clone()
}

/// A route with a parameter, its enumeration, and a view that dispatches
/// on it. The shape every test below varies one thing in.
fn site(extra: &str, arms: &str) -> String {
    format!(
        "state slugs is static List of Text starting [\"a\", \"b\"]\n\
         {extra}\
         route Site\n\
         \x20   Home is \"/\"\n\
         \x20   Post is \"/post\" with slug is Text in slugs\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Home\n\
         \x20                   Text \"home\"\n\
         {arms}"
    )
}

const POST_ARM: &str = "                Post with slug\n                    Text slug\n";

#[test]
fn a_route_with_an_enumerated_parameter_checks() {
    accepted(&site("", POST_ARM));
}

/// §14G.2 revision 3. `Post is "/work" with slug is Text` and
/// `Feed is "/work/feed"` differ in both prefix and arity, so a check on
/// the declaration cannot see that both render `/work/feed`.
#[test]
fn two_routes_rendering_one_url_are_rejected() {
    let message = rejected(
        "state slugs is static List of Text starting [\"feed\"]\n\
         route Site\n\
         \x20   Item is \"/work\" with slug is Text in slugs\n\
         \x20   Feed is \"/work/feed\"\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Item with slug\n\
         \x20                   Text slug\n\
         \x20               Feed\n\
         \x20                   Text \"feed\"\n",
        "both render the URL",
    );
    assert!(message.contains("/work/feed"), "{message}");
}

/// §14G.2 revision 2. The build writes one public file per enumerated
/// value, with the value as the directory name, so a `secret` collection
/// publishes its secrets whether or not a page shows them.
#[test]
fn a_secret_enumeration_is_rejected_because_file_names_are_public() {
    let message = rejected(
        "secret state codes is durable List of Text starting empty\n\
         route Site\n\
         \x20   Invite is \"/invite\" with code is Text in codes\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   Text \"x\"\n",
        "`codes` is `secret`",
    );
    assert!(message.contains("public"), "{message}");
}

/// §14G.7.5: enumerability composes because routing reads the placement
/// and `static` supplies it. There is no route manifest to write.
#[test]
fn an_enumeration_that_is_not_static_is_rejected() {
    rejected(
        "state slugs is client List of Text starting [\"a\"]\n\
         route Site\n\
         \x20   Post is \"/post\" with slug is Text in slugs\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   Text \"x\"\n",
        "must be `static`",
    );
}

/// §14G.2 revision 1, the load-bearing rule: the address signal is
/// immutable, and `Link` is the only navigation.
#[test]
fn writing_the_address_signal_is_rejected() {
    let message = rejected(
        &site(
            "",
            "                Post with slug\n\
             \x20                   Button \"go\"\n\
             \x20                       on click\n\
             \x20                           set page to page\n",
        ),
        "immutable",
    );
    assert!(message.contains("Link"), "{message}");
}

/// §14G.2 revision 4: `in` takes a bare name. An undelimited expression
/// before a comma list is swallowed by the list.
#[test]
fn in_takes_a_name_rather_than_an_expression() {
    let error = zdc_parser::parse(
        "route Site\n    Post is \"/post\" with slug is Text in slugsIn with items is posts\n",
    )
    .expect_err("an expression after `in` must not parse");
    assert!(
        error.message.contains("`with`") || error.message.contains("route"),
        "{}",
        error.message
    );
}

/// A URL is a literal prefix. §6 refuses embedded markup inside a string
/// for the same reason: a second grammar inside a literal is a grammar
/// nothing checks.
#[test]
fn a_parameter_written_inside_the_url_string_is_refused() {
    let error = zdc_parser::parse("route Site\n    Post is \"/post/[slug]\"\n")
        .expect_err("`[slug]` must not parse");
    assert!(error.message.contains("`with`"), "{}", error.message);
}

// --- §18.1, the integrity direction ---

/// **The acceptance test.** A route parameter with no `in` is born
/// untrusted, and using it where a `trusted` value is required is a
/// compile error.
#[test]
fn an_unenumerated_route_parameter_may_not_index_a_trusted_signal() {
    let message = rejected(
        "trusted state owners is durable Map of Text to Text starting empty\n\
         route Site\n\
         \x20   Draft is \"/draft\" with id is Text\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Draft with id\n\
         \x20                   Button \"claim\"\n\
         \x20                       on click\n\
         \x20                           set owners at id to \"me\"\n",
        "E-INT-02",
    );
    assert!(message.contains("chosen by the browser"), "{message}");
    // The old default-open pass printed *why* the key was untrusted — "it
    // is a route parameter with no `in`" — because it carried a reason on
    // every label. The closed lattice has no reasons to carry: a value is
    // Untrusted because no grant covers it. What the help must still do is
    // name a repair rather than restate the rule.
    assert!(
        message.contains("Index by a value the program owns"),
        "the help must say the repair: {message}"
    );
}

/// An `in` clause does not rescue this write, and the reason is not the
/// route parameter at all.
///
/// `feature/routing2` asserted the opposite here, on §18.1 semantics 5:
/// a parameter carrying an `in` over a public `static` collection is
/// **trusted**, because the compiler rendered one document per enumerated
/// value and reaching the document is a proof rather than a check. That
/// stands as far as the *parameter* goes, and `bind_arm` still implements
/// it — but the write below is client-rooted into `durable` state, which
/// §18.1 semantics 4's command rule labels untrusted whatever the index
/// holds, because a browser sends the write and may send any value with
/// it. The merge of the two integrity passes keeps the stricter rule, and
/// spec §21.7.6 (2026-08-03) says the same thing from the other end: it
/// deletes semantics 5 outright and rules every route parameter
/// untrusted, `in` clause or not.
#[test]
fn an_enumerated_route_parameter_does_not_rescue_a_client_rooted_write() {
    let src = "state slugs is static List of Text starting [\"a\", \"b\"]\n\
         trusted state owners is durable Map of Text to Text starting empty\n\
         route Site\n\
         \x20   Post is \"/post\" with slug is Text in slugs\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Post with slug\n\
         \x20                   Button \"claim\"\n\
         \x20                       on click\n\
         \x20                           set owners at slug to \"me\"\n";
    let message = rejected(src, "E-INT-02");
    assert!(message.contains("chosen by the browser"), "{message}");
    // Both obligations fire on this one statement, and the second is the
    // one the doc comment above is about: the *write* is a command
    // whatever the index holds.
    let found = errors(src);
    assert!(
        found
            .iter()
            .any(|m| m.contains("a browser sends this write") && m.contains("E-INT-03")),
        "the command rule must fire on its own account: {found:#?}"
    );
}

/// A3: the value written into a `trusted` place.
#[test]
fn an_unenumerated_route_parameter_may_not_be_written_into_a_trusted_place() {
    rejected(
        "trusted state owner is durable Text starting \"\"\n\
         route Site\n\
         \x20   Draft is \"/draft\" with id is Text\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Draft with id\n\
         \x20                   Button \"claim\"\n\
         \x20                       on click\n\
         \x20                           set owner to id\n",
        "E-INT-03",
    );
}

/// §18.1 semantics 11: an implicit flow is a flow. *Whether* the write
/// happens is the same decision as what it holds.
#[test]
fn a_trusted_write_under_a_browser_chosen_condition_is_rejected() {
    rejected(
        "trusted state owner is durable Text starting \"\"\n\
         route Site\n\
         \x20   Draft is \"/draft\" with id is Text\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Draft with id\n\
         \x20                   Button \"claim\"\n\
         \x20                       on click\n\
         \x20                           if id is \"root\"\n\
         \x20                               set owner to \"yes\"\n",
        "E-INT-04",
    );
}

/// §18.1 semantics 9's `client` half. A browser owns its own memory, so
/// there is no such thing as protecting one from itself.
///
/// **The `static` half was here too and is gone**, and not because it
/// stopped being convenient: §21.7.3 deletes `static`'s blanket grant.
/// *"No browser attached"* is a fact about **when** the code ran, not
/// about **who** chose the data, and a build that ingests a fetched feed
/// through an ungranted `foreign` produces Untrusted `static` state. So
/// `trusted static` is exactly what a declaration is for, and the spec
/// says in as many words that it must no longer be E-INT-01. The half of
/// the old assertion that survives is asserted here; the half that was
/// overturned is asserted below, in the other direction.
#[test]
fn trusted_is_meaningless_on_client() {
    rejected(
        "trusted state a is client Text starting \"x\"\nview\n    Text a\n",
        "cannot be trusted",
    );
}

/// The overturned half, asserted so the deletion is a decision rather than
/// an omission: §21.7.3 makes `trusted static` a declaration the compiler
/// accepts.
#[test]
fn trusted_static_is_accepted_since_static_gets_no_blanket_grant() {
    accepted("trusted state a is static Text starting \"x\"\nview\n    Text a\n");
}

/// A program that never writes `trusted` is checked exactly as it was.
/// That is the difference between an opt-in lattice and Ballerina's
/// mandatory one (§18.1.4).
#[test]
fn a_program_that_never_says_trusted_pays_nothing() {
    accepted(&site("", POST_ARM));
    accepted("state a is client Text starting \"x\"\nview\n    Text a\n");
}

// --- `static` ---

/// §14C.3b: a `static` signal is evaluated on the build host, and §17.4.8
/// evaluates the build root in a real JavaScript engine — so arithmetic,
/// a record construction and a call are all things it computes. A
/// `static` initialiser is therefore *not* required to be a literal, and
/// this checks that it is accepted rather than refused for being one.
#[test]
fn a_static_initialiser_may_be_computed_at_build_time() {
    accepted(
        "state m is static Whole starting 1 + 1\n\
         view\n\
         \x20   Text m\n",
    );
}

/// The narrower rule that survives: a route parameter's `in` is
/// enumerated by *this* pass, which folds literals and lists of them and
/// nothing else. A `static` list it cannot fold is refused at the `in`
/// that needed it, naming the enumeration rather than the declaration —
/// the build host can still compute the value, and everything but the
/// route can still read it.
#[test]
fn an_in_over_a_static_this_pass_cannot_fold_is_refused() {
    let message = rejected(
        "function slugsOf with seed\n\
         \x20   give [seed]\n\
         state slugs is static List of Text from slugsOf with \"a\"\n\
         route Site\n\
         \x20   Post is \"/post\" with slug is Text in slugs\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           Text \"somewhere\"\n",
        "cannot be enumerated over it",
    );
    assert!(
        message.contains("slugs"),
        "the diagnostic must name the signal: {message}"
    );
}

#[test]
fn a_static_signal_may_read_another_static_signal() {
    accepted(
        "state a is static List of Text starting [\"x\"]\n\
         state b is static List of Text from a\n\
         route Site\n\
         \x20   Post is \"/post\" with slug is Text in b\n\
         state page is client Option of Site starting address\n\
         view\n\
         \x20   when page\n\
         \x20       None\n\
         \x20           Text \"nowhere\"\n\
         \x20       Some with here\n\
         \x20           when here\n\
         \x20               Post with slug\n\
         \x20                   Text slug\n",
    );
}

// --- `Link` ---

/// A URL this program serves is written as the route that serves it.
///
/// `Link`'s destination is one slot (§4.1) and two kinds of value sit in
/// it: a route value, whose URL the compiler renders (§14G.2 revision 1),
/// and `Text`, for somewhere outside the program — which `page.zd` needs
/// and which no route can express. They name disjoint things with one
/// overlap, a literal URL this program does serve, and that overlap is
/// what is refused here. The diagnostic names the route to write.
#[test]
fn link_takes_a_route_and_not_a_string_for_a_url_this_program_serves() {
    let message = rejected(
        "route Site\n\
         \x20   Home is \"/\"\n\
         view\n\
         \x20   Link \"/\"\n\
         \x20       Text \"home\"\n",
        "`Link Home`",
    );
    assert!(message.contains("one phrasing"), "{message}");
}

/// The other side of the same rule: a URL this program does *not* serve
/// is a destination outside it, and writing one is how a link leaves the
/// site at all.
#[test]
fn link_takes_a_url_that_leaves_the_site() {
    accepted(
        "route Site\n\
         \x20   Home is \"/\"\n\
         view\n\
         \x20   Link \"https://example.com/feed.xml\"\n\
         \x20       Text \"feed\"\n",
    );
}

/// Without a `route`, `Home` names nothing at all — which is the
/// diagnostic a reader wants, and one the routing pass never has to give.
#[test]
fn link_needs_a_route_declaration() {
    let program =
        zdc_parser::parse("view\n    Link Home\n        Text \"home\"\n").expect("parses");
    let errors = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect_err("`Link Home` with no route must be rejected");
    assert!(
        errors[0].message.contains("`Home`"),
        "{}",
        errors[0].message
    );
}

/// A route is a `choice`, so a missing parameter is a missing field and
/// the existing machinery reports it.
#[test]
fn a_link_missing_a_parameter_is_rejected() {
    let found = errors(
        "state slugs is static List of Text starting [\"a\"]\n\
         route Site\n\
         \x20   Post is \"/post\" with slug is Text in slugs\n\
         view\n\
         \x20   Link Post\n\
         \x20       Text \"post\"\n",
    );
    assert!(!found.is_empty(), "a link with no `slug` must be rejected");
}

/// One `route`, for the same reason there is one `view`: `address` names
/// the URL this document was served at, and two route types would leave
/// that value's type ambiguous.
#[test]
fn two_routes_are_rejected() {
    let program = zdc_parser::parse("route A\n    Home is \"/\"\nroute B\n    Away is \"/away\"\n")
        .expect("parses");
    let errors = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect_err("two routes must be rejected");
    assert!(
        errors.iter().any(|e| e.message.contains("one `route`")),
        "{errors:#?}"
    );
}

/// `address` has a value only once a `route` says which URLs exist.
#[test]
fn address_without_a_route_names_what_is_missing() {
    let program =
        zdc_parser::parse("state page is client Text starting address\nview\n    Text \"x\"\n")
            .expect("parses");
    let errors = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect_err("`address` with no route must be rejected");
    assert!(
        errors[0].message.contains("`route`"),
        "{}",
        errors[0].message
    );
}
