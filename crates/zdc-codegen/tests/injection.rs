//! What a program's own text may and may not become in the output.
//!
//! §16.3.5's escaping argument is about markup: a value that reaches the
//! DOM through `nodeValue` or `setAttribute` is never parsed, so it cannot
//! be markup. The argument holds, and these tests are about the three
//! places a program's text reaches the output by some *other* route, where
//! it never applied:
//!
//! * the base of a generated `class` getter, which is a JavaScript string
//!   literal in the emitted module;
//! * a folded style declaration, which is printed into `styles.css`;
//! * the shape checks that decide what markup is even built.
//!
//! A ZD string literal has no escape sequences at all — the lexer's rule is
//! `"[^"\n]*"` — so it cannot contain `"` or a newline. It can contain
//! everything else, including `'`, `\`, `;`, `{` and `<`, which is exactly
//! the alphabet the first two of those needed.

mod support;

use support::{compile_source, context, refusals, run};

fn assert_refused(source: &str, needle: &str) {
    let messages = refusals(source);
    assert!(
        messages.iter().any(|message| message.contains(needle)),
        "expected a diagnostic mentioning `{needle}`, got:\n{}",
        messages.join("\n")
    );
}

// --- the emitted module ---------------------------------------------------

/// The one place a program's own text was interpolated into JavaScript
/// source rather than into a value.
///
/// `class is <signal>` binds `() => '<base> ' + getter()`, and `<base>` is
/// the element's classes joined. A second `class` argument put the
/// program's own literal into that base, unescaped, so
/// `class is "a'+alert(1)+'b"` closed the string and wrote a call into the
/// module. Two things stop it now: the base goes through `js::string`, and
/// an argument given twice is refused outright.
#[test]
fn a_class_literal_cannot_close_the_getter_it_is_interpolated_into() {
    assert_refused(
        "state c is client Text starting \"red\"\n\
         view\n\
         \x20   Column class is \"a'+alert(1)+'b\", class is c\n\
         \x20       Text \"x\"\n",
        "given `class` twice",
    );
}

/// The escaping half of the same repair, tested where the refusal cannot
/// reach it: one `class` argument, and a base carrying a quote through a
/// path that has nothing to do with duplication.
#[test]
fn the_base_of_a_class_getter_is_a_quoted_string_rather_than_raw_text() {
    let client = compile_source(
        "state c is client Text starting \"red\"\n\
         view\n\
         \x20   Column class is c\n\
         \x20       Text \"x\"\n",
    )
    .client_js;
    assert!(
        client.contains("bindAttr($n0, 'class', () => 'zd-col ' + (c)());"),
        "{client}"
    );
}

/// The same program in the engine: the class the element ends up with is
/// the text the program wrote, and evaluating the module runs nothing the
/// program did not ask for.
#[test]
fn a_quote_in_a_class_reaches_the_dom_as_text_and_not_as_code() {
    let client = compile_source(
        "state c is client Text starting \"it's fine\"\n\
         view\n\
         \x20   Column class is c\n\
         \x20       Text \"x\"\n",
    )
    .client_js;
    let mut context = context(false);
    let found = run(
        &mut context,
        &client,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         html($host);",
    );
    assert_eq!(
        found,
        "<div><div class=\"zd-col it's fine\"><span>x</span></div></div>"
    );
}

// --- the generated stylesheet ---------------------------------------------

/// A folded style declaration is *printed* into `styles.css`, so a value
/// carrying `}` does not make a bad declaration — it ends the rule and
/// opens one for a selector the program never wrote. `bindStyle` goes
/// through `setProperty`, which parses one declaration and drops the rest,
/// which is why only the folded arm needs the check.
#[test]
fn a_style_value_may_not_end_the_rule_it_is_folded_into() {
    assert_refused(
        "view\n\
         \x20   Column weight is \"bold; } body { display: none } x {\"\n\
         \x20       Text \"x\"\n",
        "would end that rule",
    );
}

#[test]
fn a_style_value_may_not_smuggle_a_request_into_the_stylesheet() {
    assert_refused(
        "view\n\
         \x20   Column weight is \"url(https://example.com/x)\"\n\
         \x20       Text \"x\"\n",
        "would end that rule",
    );
}

/// The values a program actually writes still fold.
#[test]
fn an_ordinary_style_value_still_folds_into_a_class() {
    let bundle = compile_source(
        "view\n\
         \x20   Row padding is 8, weight is \"bold\"\n\
         \x20       Text \"a\"\n",
    );
    assert!(
        bundle
            .styles_css
            .contains(".zd-s0 { font-weight: bold; padding: 8px; }"),
        "{}",
        bundle.styles_css
    );
}

// --- the shape checks -----------------------------------------------------

/// `only_children` is about what ends up a DOM child, and `each`, `if`,
/// `when` and a component's own scope all place their contents directly in
/// the parent. Checking only the direct `HirNode::Element` children let
/// every one of them through, so `List / each / Column` emitted a `<div>`
/// inside a `<ul>` — which is what the check exists to prevent.
#[test]
fn a_list_is_checked_through_the_constructs_that_place_nodes_in_it() {
    for (what, source) in [
        (
            "each",
            "state xs is client List of Text starting [\"a\"]\n\
             view\n\
             \x20   List\n\
             \x20       each x in xs\n\
             \x20           Column\n\
             \x20               Text x\n",
        ),
        (
            "if",
            "state open is client Truth starting yes\n\
             view\n\
             \x20   List\n\
             \x20       if open\n\
             \x20           Column\n\
             \x20               Text \"x\"\n",
        ),
        (
            "a component with state of its own",
            "component Bad\n\
             \x20   state n is client Whole starting 0\n\
             \x20   Column\n\
             \x20       Text n\n\
             view\n\
             \x20   List\n\
             \x20       Bad\n",
        ),
    ] {
        let messages = refusals(source);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("takes only `Item`")),
            "`List` accepted a `Column` placed by {what}:\n{}",
            messages.join("\n")
        );
    }
}

/// The same constructs must not turn a legal list into a diagnostic.
#[test]
fn an_item_placed_by_each_is_still_an_item() {
    let bundle = compile_source(
        "state xs is client List of Text starting [\"a\"]\n\
         view\n\
         \x20   List\n\
         \x20       each x in xs\n\
         \x20           Item x\n",
    );
    assert!(bundle.client_js.contains("<ul>"), "{}", bundle.client_js);
}
